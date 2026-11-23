//! Several atl at once: the directory is locked while one works on it, and
//! whoever comes last leaves a whole skill of one version.

use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use atl::skill::{FirstRun, Outcome, Skipped};
use pretty_assertions::assert_eq;

use super::*;

/// Start `atl skill ARGS` in `home`.
fn spawn(home: &Home, args: &[&str]) -> Child {
    std::process::Command::new(assert_cmd::cargo::cargo_bin("atl"))
        .env_clear()
        .env("HOME", &home.root)
        .current_dir(&home.root)
        .args(["--no-color", "-F", "json", "skill"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn atl")
}

/// The exit code and stderr of `child`.
fn finish(child: Child) -> (i32, String) {
    let output = child.wait_with_output().expect("wait");
    (
        output.status.code().expect("exit code"),
        String::from_utf8(output.stderr).expect("UTF-8"),
    )
}

/// Whether `child` is done within `time`.
fn ends_within(child: &mut Child, time: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < time {
        if child.try_wait().expect("try_wait").is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// What a release build does after an ordinary command, in `home`.
fn first_run(home: &Home) -> FirstRun {
    FirstRun {
        release_build: true,
        own_command: false,
        env: environment(home),
    }
}

/// The skill is whole, of this version, with no file left over.
fn assert_whole(home: &Home) {
    let status = home.status(&[]);
    for (key, expected) in [
        ("state", json!("up_to_date")),
        ("edited", json!([])),
        ("missing", json!([])),
        ("unlisted", json!([])),
    ] {
        assert_eq!(status[key], expected, "{status:#}");
    }
    let names: Vec<String> = tree(&home.skill())
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(names, FILES, "and no temporary file");
}

/// A command that writes waits for the one that holds the lock; one that
/// only looks does not, and the automatic update does without.
#[test]
fn a_command_waits_for_the_lock_of_another() {
    let home = Home::new();
    home.install_edition(&OLD);
    let before = tree(&home.root);
    for (args, done) in [
        (&["update"][..], "up_to_date"),
        (&["install"], "up_to_date"),
        (&["remove"], "not_installed"),
    ] {
        let lock = fs::File::open(home.skill()).expect("open");
        lock.lock().expect("lock");
        let mut child = spawn(&home, args);
        assert!(
            !ends_within(&mut child, Duration::from_millis(300)),
            "{args:?} did not wait"
        );
        assert_eq!(
            first_run(&home).update_skill(&Edition::embedded()),
            Outcome::Skipped(Skipped::Busy),
            "{args:?}"
        );
        assert_eq!(home.status(&[])["state"], json!("older"));
        assert_eq!(tree(&home.root), before, "{args:?} wrote before its turn");
        drop(lock);
        assert_eq!(finish(child), (0, String::new()), "{args:?}");
        assert_eq!(home.status(&[])["state"], json!(done), "{args:?}");
        home.ok(&["remove"]);
        home.install_edition(&OLD);
    }
}

#[test]
fn many_installs_at_once() {
    let home = Home::with_claude();
    let children: Vec<Child> = (0..12).map(|_| spawn(&home, &["install"])).collect();
    for child in children {
        assert_eq!(finish(child), (0, String::new()));
    }
    assert_whole(&home);
    assert_eq!(home.status(&[])["link_state"], json!("ok"));
    let in_claude: Vec<String> = fs::read_dir(home.root.join(".claude/skills"))
        .expect("dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .collect();
    assert_eq!(in_claude, ["atl"], "and no temporary link");
}

/// Forced installs replace the link of another at the same time.
#[test]
fn many_forced_installs_replace_one_link() {
    let home = Home::with_claude();
    fs::create_dir(home.root.join(".claude/skills")).expect("mkdir");
    symlink("/nowhere", home.link()).expect("symlink");
    let children: Vec<Child> = (0..8)
        .map(|_| spawn(&home, &["install", "--force"]))
        .collect();
    for child in children {
        assert_eq!(finish(child), (0, String::new()));
    }
    assert_whole(&home);
    assert_eq!(
        fs::read_link(home.link()).expect("a link"),
        Path::new("../../.agents/skills/atl")
    );
}

/// Explicit updates of this version, and automatic ones of it and of an
/// older one, on an older skill.
#[test]
fn updates_of_every_kind_at_once() {
    /// Newer than the one that is there, older than the one that comes.
    const BETWEEN: Edition = Edition {
        version: "2021.1.0",
        files: OLD.files,
    };
    let home = Home::new();
    home.install_edition(&OLD);
    let children: Vec<Child> = (0..6).map(|_| spawn(&home, &["update"])).collect();
    let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..12)
            .map(|n| {
                let run = first_run(&home);
                let edition = [Edition::embedded(), BETWEEN][n % 2];
                scope.spawn(move || run.update_skill(&edition))
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().expect("no panic"))
            .collect()
    });
    for child in children {
        assert_eq!(finish(child), (0, String::new()));
    }
    for outcome in outcomes {
        assert!(
            matches!(
                outcome,
                // Changed: what another is in the middle of.
                Outcome::Updated
                    | Outcome::Skipped(Skipped::Busy | Skipped::NotOlder | Skipped::Changed)
            ),
            "{outcome:?}"
        );
    }
    assert_whole(&home);
}

/// A reader that comes in between sees a whole file of one version, never
/// none and never half of one.
#[test]
fn a_reader_never_sees_half_a_file() {
    let home = Home::new();
    home.install_edition(&OLD);
    let at = home.skill().join("SKILL.md");
    let new = source("SKILL.md");
    let seen = std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            let mut seen = Vec::new();
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(30) {
                let text = fs::read_to_string(&at).expect("the file is there");
                let done = text == new;
                seen.push(text);
                if done {
                    break;
                }
            }
            seen
        });
        home.ok(&["update"]);
        reader.join().expect("no panic")
    });
    assert_eq!(seen.last(), Some(&new), "the update was seen");
    assert!(
        seen.iter()
            .all(|text| *text == "old skill\n" || *text == new)
    );
}

/// The lock is the directory's, and a `remove` deletes that. Who waited for
/// it waits for the one that is at the path by then, and has not seen what
/// the atl that holds that one is writing.
#[test]
fn a_lock_on_a_directory_that_is_gone_is_none() {
    for (args, done) in [
        (&["install"][..], "up_to_date"),
        (&["update"], "up_to_date"),
        (&["remove"], "not_installed"),
    ] {
        let home = Home::new();
        home.install_edition(&OLD);
        let gone = fs::File::open(home.skill()).expect("open");
        gone.lock().expect("lock");
        let mut child = spawn(&home, args);
        assert!(
            !ends_within(&mut child, Duration::from_millis(300)),
            "{args:?} did not wait"
        );
        // Another atl removes it; a third begins to install, and holds that.
        fs::rename(home.skill(), home.root.join("gone")).expect("mv");
        fs::create_dir(home.skill()).expect("mkdir");
        let lock = fs::File::open(home.skill()).expect("open");
        lock.lock().expect("lock");
        fs::write(home.skill().join("SKILL.md"), "half").expect("write");
        let before = tree(&home.root);
        drop(gone);
        assert!(
            !ends_within(&mut child, Duration::from_millis(300)),
            "{args:?} did not wait for the second"
        );
        assert_eq!(tree(&home.root), before, "{args:?} wrote before its turn");
        home.install_edition(&OLD); // the third is done
        drop(lock);
        assert_eq!(finish(child), (0, String::new()), "{args:?}");
        assert_eq!(home.status(&[])["state"], json!(done), "{args:?}");
    }
}

/// The directory goes while a command waits for its lock, and none comes:
/// there is nothing to update or to remove, and `install` makes one.
#[test]
fn a_skill_that_is_removed_meanwhile_is_not_installed() {
    for command in ["update", "remove", "install"] {
        let home = Home::new();
        home.install_edition(&OLD);
        let gone = fs::File::open(home.skill()).expect("open");
        gone.lock().expect("lock");
        let mut child = spawn(&home, &[command]);
        assert!(!ends_within(&mut child, Duration::from_millis(300)));
        fs::rename(home.skill(), home.root.join("gone")).expect("mv");
        drop(gone);
        let output = child.wait_with_output().expect("wait");
        assert_eq!(output.status.code(), Some(0), "{command}");
        let stdout = String::from_utf8(output.stdout).expect("UTF-8");
        let said: Value =
            serde_json::from_str(&stdout.replace(home.root.to_str().expect("UTF-8"), "~"))
                .expect("JSON");
        let expected = match command {
            "install" => wrote("installed", VERSION, ""),
            _ => none_installed(command),
        };
        assert_eq!(said, expected, "{command}");
        assert_eq!(home.skill().exists(), command == "install");
    }
}

/// Installing, updating and removing at the same time, by explicit commands
/// and by the automatic update of a newer atl: none fails because of another,
/// and one more install makes the skill whole.
#[test]
fn installs_updates_and_removes_at_once_all_succeed() {
    let newer = Edition {
        version: "9999.1.0",
        files: Edition::embedded().files,
    };
    for round in 0..8 {
        let home = Home::with_claude();
        if round % 2 == 0 {
            home.ok(&["install"]);
        }
        let done = std::sync::atomic::AtomicBool::new(false);
        let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
            let automatic: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        let run = first_run(&home);
                        let mut outcomes = Vec::new();
                        let start = Instant::now();
                        while !done.load(std::sync::atomic::Ordering::Relaxed)
                            && start.elapsed() < Duration::from_secs(60)
                        {
                            outcomes.push(run.update_skill(&newer));
                        }
                        outcomes
                    })
                })
                .collect();
            let children: Vec<Child> = (0..24)
                .map(|n| spawn(&home, &[["install", "remove", "update", "status"][n % 4]]))
                .collect();
            let ends: Vec<(i32, String)> = children.into_iter().map(finish).collect();
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            for end in ends {
                assert_eq!(end, (0, String::new()), "round {round}");
            }
            automatic
                .into_iter()
                .flat_map(|thread| thread.join().expect("no panic"))
                .collect()
        });
        for outcome in outcomes {
            assert!(
                matches!(
                    outcome,
                    Outcome::Updated
                        | Outcome::Skipped(
                            Skipped::Busy
                                | Skipped::NotOlder
                                | Skipped::NotInstalled
                                | Skipped::Changed // what another is doing
                        )
                ),
                "round {round}: {outcome:?}"
            );
        }
        home.ok(&["install"]);
        assert_whole(&home);
        assert_eq!(home.status(&[])["link_state"], json!("ok"));
    }
}

/// The same by threads, which get in each other's way far more often than
/// processes do; after each round there is a skill with its link or neither.
#[test]
fn an_install_and_a_remove_at_once_leave_both_or_neither() {
    let home = Home::with_claude();
    let target = environment(&home).global().expect("target");
    let newer = Edition {
        version: "9999.1.0",
        files: Edition::embedded().files,
    };
    let edition = Edition::embedded();
    for round in 0..400 {
        let start = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                atl::skill::install(&target, &edition, false).expect("install");
            });
            scope.spawn(|| {
                start.wait();
                atl::skill::update(&target, &edition, false).expect("update");
                atl::skill::status(&target, &edition).expect("status");
            });
            scope.spawn(|| {
                start.wait();
                let outcome = first_run(&home).update_skill(&newer);
                assert!(
                    matches!(
                        outcome,
                        Outcome::Updated
                            | Outcome::Skipped(
                                Skipped::Busy
                                    | Skipped::NotOlder
                                    | Skipped::NotInstalled
                                    | Skipped::Changed // what another is doing
                            )
                    ),
                    "{outcome:?}"
                );
            });
            start.wait();
            if round % 4 == 0 {
                std::thread::yield_now();
            }
            atl::skill::remove(&target, &edition, false).expect("remove");
        });
        let status = atl::skill::status(&target, &edition).expect("status");
        assert_eq!(
            (
                status.edited.len(),
                status.missing.len(),
                status.unlisted.len()
            ),
            (0, 0, 0),
            "round {round}: {status:?}"
        );
        assert_eq!(
            status.state.is_installed(),
            home.link().is_symlink(),
            "round {round}: {status:?}"
        );
    }
}
