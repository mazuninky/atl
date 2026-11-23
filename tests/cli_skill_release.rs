//! The automatic update of the agent skill, by the real binary, in a home
//! directory of its own; and the guard that no test of this crate runs the
//! binary in the developer's home directory.
//!
//! Only a release build updates the skill by itself, so the tests of that
//! (`mod release`) are compiled only without debug assertions:
//!
//! ```sh
//! cargo test --release --test cli_skill_release
//! # faster to build, in a directory of its own
//! CARGO_PROFILE_DEV_DEBUG_ASSERTIONS=false CARGO_TARGET_DIR=<tmp>/no-debug \
//!     cargo test --test cli_skill_release
//! ```
//!
//! `atl skill` itself is tested through the binary in `cli_skill.rs`, on the
//! library (`atl::skill`) and through the handler; here only what a process
//! does around it. The home directory is `skill/home.rs`, shared with
//! `cli_skill.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use serde_json::json;

#[path = "skill/home.rs"]
mod home;
use home::*;

impl Home {
    /// The version that is installed, with every file as that version wrote
    /// it.
    fn version(&self) -> String {
        let status = self.status(&[]);
        assert_eq!(status["edited"], json!([]), "{status:#}");
        assert_eq!(status["missing"], json!([]), "{status:#}");
        assert_eq!(status["unlisted"], json!([]), "{status:#}");
        status["version"].as_str().expect("version").to_owned()
    }
}

/// The binary tests run is a debug build: it leaves an older skill as it is.
/// A release build would update it, which `mod release` shows.
#[test]
#[cfg(debug_assertions)]
fn a_debug_build_updates_no_skill_by_itself() {
    let home = Home::with_alias();
    home.install_edition(&OLD);
    let before = tree(&home.skill());
    let (code, _, stderr) = home.finish(home.atl().args(["alias", "list"]));
    assert_eq!((code, stderr.as_str()), (0, ""));
    assert_eq!(tree(&home.skill()), before);
    assert_eq!(home.status(&[])["state"], json!("older"));
    // The home directory is joined to `.agents/skills/atl` by the platform's
    // separator: `\` on Windows.
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(
        home.ok(&["update"]),
        format!(
            "path: ~{sep}.agents/skills/atl\naction: updated\nversion: {VERSION}\n\
             previous: 2020.1.0\nkept: []\nlink: \nlink_action: \n"
        )
    );
    assert_eq!(home.version(), VERSION);
}

/// The whole life of a skill through the binary, on every platform: most
/// tests of the skill are unix-only, and this is what CI on Windows runs of
/// it — every file replaced in place (the manifest twice per write), every
/// file removed, the directory removed after.
#[test]
fn a_skill_is_installed_updated_and_removed_on_every_platform() {
    let home = Home::new();
    let is_skill_dir = |said: &serde_json::Value| {
        let path = said["path"].as_str().expect("path");
        assert!(path.ends_with(".agents/skills/atl"), "{said:#}");
    };
    let said = home.json(&["install"]);
    is_skill_dir(&said);
    assert_eq!(
        (&said["action"], &said["version"], &said["link"]),
        (&json!("installed"), &json!(VERSION), &json!(""))
    );
    let status = home.status(&[]);
    is_skill_dir(&status);
    assert_eq!(
        (
            &status["state"],
            &status["installed"],
            &status["link_state"]
        ),
        (&json!("up_to_date"), &json!(true), &json!("not_wanted"))
    );

    // As an older atl left it.
    fs::remove_dir_all(home.skill()).expect("rm");
    home.install_edition(&OLD);
    let status = home.status(&[]);
    assert_eq!(
        (&status["state"], &status["version"]),
        (&json!("older"), &json!(OLD.version))
    );
    let said = home.json(&["update"]);
    assert_eq!(
        (&said["action"], &said["previous"], &said["version"]),
        (&json!("updated"), &json!(OLD.version), &json!(VERSION))
    );
    assert_eq!(home.version(), VERSION);
    assert!(!home.skill().join("references/gone.md").exists());
    for file in FILES {
        assert!(home.skill().join(file).is_file(), "{file}");
    }
    assert_eq!(home.json(&["update"])["action"], json!("up_to_date"));

    let said = home.json(&["remove"]);
    assert_eq!(
        (&said["action"], &said["version"], &said["dir_kept"]),
        (&json!("removed"), &json!(VERSION), &json!(false))
    );
    assert!(!home.skill().exists());
    assert_eq!(home.status(&[])["state"], json!("not_installed"));
}

/// The automatic update, as a release build does it.
#[cfg(not(debug_assertions))]
mod release {
    use std::process::Stdio;

    use assert_cmd::Command;
    use pretty_assertions::assert_eq;
    use serde_json::Value;

    use super::*;

    /// A home directory with an older skill.
    fn home() -> Home {
        let home = Home::with_alias();
        home.install_edition(&OLD);
        home
    }

    /// Ordinary commands of every kind of outcome, none of which needs the
    /// network: done, done with JSON, done quietly, refused by a handler
    /// (exit 1, exit 3).
    const COMMANDS: [&[&str]; 7] = [
        &["--config", "atl.toml", "alias", "list"],
        &["--config", "atl.toml", "-F", "json", "alias", "list"],
        &["--config", "atl.toml", "-q", "alias", "list"],
        &["completions", "bash"],
        &["--config", "none.toml", "config", "list"],
        &["--config", "none.toml", "jira", "view", "A-1"],
        &[
            "--config",
            "none.toml",
            "api",
            "--service",
            "jira",
            "rest/api/2/myself",
        ],
    ];

    #[test]
    fn an_ordinary_command_updates_an_older_skill_and_is_not_changed_by_it() {
        for command in COMMANDS {
            let run = |home: &Home| home.finish(home.atl().args(command));
            let without = Home::with_alias();
            let expected = run(&without);
            assert!(!without.skill().exists(), "{command:?} made a skill");

            let home = home();
            fs::create_dir(home.root.join(".claude")).expect("mkdir");
            assert_eq!(run(&home), expected, "{command:?}");
            assert_eq!(home.version(), VERSION, "{command:?}");
            assert!(!home.skill().join("references/gone.md").exists());
            assert!(!home.link().exists(), "only `install` makes the link");
            // And the second run, which finds nothing to do.
            let before = tree(&home.skill());
            assert_eq!(run(&home), expected, "{command:?}");
            assert_eq!(tree(&home.skill()), before);
        }
    }

    /// clap ends the process itself; the next run does it.
    #[test]
    fn help_and_usage_errors_update_nothing() {
        for args in [
            &["--help"][..],
            &["--version"],
            &["jira"],
            &["--no-such-flag"],
        ] {
            let home = home();
            home.finish(home.atl().args(args));
            assert_eq!(home.version(), OLD.version, "{args:?}");
        }
    }

    #[test]
    fn the_environment_turns_it_off() {
        let list = ["--config", "atl.toml", "-F", "json", "alias", "list"];
        let listed = |home: &Home| home.finish(home.atl().args(list));
        let expected = listed(&Home::with_alias());
        for (name, value, version) in [
            ("CI", "true", OLD.version),
            ("CI", "", OLD.version),
            ("CI", "false", OLD.version),
            ("ATL_NO_SKILL_UPDATE", "1", OLD.version),
            ("ATL_NO_SKILL_UPDATE", "", OLD.version),
            ("ATL_NO_SKILL_UPDATE", "0", VERSION),
            ("HOME", "", OLD.version),
            ("HOME", "rel", OLD.version),
            ("HOME", "/no/such/home", OLD.version),
        ] {
            let home = home();
            let said = home.finish(home.atl().env(name, value).args(list));
            assert_eq!(said, expected, "{name}={value}");
            assert_eq!(home.version(), version, "{name}={value}");
        }
        let home = home();
        home.atl().env_remove("HOME").args(list).assert().success();
        assert_eq!(home.version(), OLD.version);
    }

    /// `atl skill` and `atl self` see to the skill themselves.
    #[test]
    fn the_commands_about_the_skill_and_atl_itself_update_nothing() {
        for args in [
            &["skill", "status"][..],
            &["skill", "status", "--project"],
            &["skill", "update", "--help"],
            // Refused before it would reach the network.
            &["self", "update", "--to", "2020.1.1"],
        ] {
            let home = home();
            home.finish(home.atl().args(args));
            assert_eq!(home.version(), OLD.version, "{args:?}");
        }
    }

    #[test]
    fn a_skill_that_is_not_atls_to_update_is_left_alone() {
        let mut changes: Vec<fn(&Home)> = vec![
            |home| fs::write(home.skill().join("SKILL.md"), "mine").expect("write"),
            |home| fs::remove_file(home.skill().join("SKILL.md")).expect("rm"),
            |home| {
                fs::write(home.skill().join("references/body-formats.md"), "mine").expect("write")
            },
            |home| home.change_manifest(|value| value["atl_version"] = json!("9999.1.0")),
            |home| home.change_manifest(|value| value["atl_version"] = json!(VERSION)),
            |home| fs::write(home.skill().join(MANIFEST), "{}").expect("write"),
            |home| fs::remove_file(home.skill().join(MANIFEST)).expect("rm"),
        ];
        #[cfg(unix)]
        changes.push(|home| {
            let aside = home.root.join("aside");
            fs::rename(home.skill(), &aside).expect("mv");
            std::os::unix::fs::symlink(aside, home.skill()).expect("symlink");
        });
        let list = ["--config", "atl.toml", "-F", "json", "alias", "list"];
        for (n, change) in changes.into_iter().enumerate() {
            let home = home();
            change(&home);
            let before = tree(&home.root);
            let (code, _, stderr) = home.finish(home.atl().args(list));
            assert_eq!((code, stderr.as_str()), (0, ""), "case {n}");
            assert_eq!(tree(&home.root), before, "case {n}");
        }
        // A file of the user's is in no one's way, and stays.
        let home = home();
        fs::write(home.skill().join("notes.md"), "mine").expect("write");
        home.atl().args(list).assert().success();
        let status = home.status(&[]);
        assert_eq!(status["state"], json!("up_to_date"));
        assert_eq!(status["unlisted"], json!(["notes.md"]));
    }

    /// The project's skill is under version control: the project's decision.
    #[test]
    fn the_skill_of_a_project_is_left_alone() {
        let home = home();
        let project = home.root.join("project");
        fs::create_dir(&project).expect("mkdir");
        let (code, _, stderr) =
            home.finish(
                home.atl()
                    .current_dir(&project)
                    .args(["skill", "install", "--project"]),
            );
        assert_eq!((code, stderr.as_str()), (0, ""));
        let manifest = project.join(".agents/skills/atl").join(MANIFEST);
        let older = fs::read_to_string(&manifest)
            .expect("read")
            .replace(VERSION, OLD.version);
        fs::write(&manifest, older).expect("write");
        let before = tree(&project);
        home.atl()
            .current_dir(&project)
            .args(["completions", "bash"])
            .assert()
            .success();
        assert_eq!(tree(&project), before);
        assert_eq!(home.version(), VERSION, "the home directory's is updated");
    }

    /// A failure is a debug log line; the next run that can, does.
    #[test]
    #[cfg(unix)]
    fn a_skill_that_cannot_be_written_changes_nothing_of_the_command() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = home();
        let chmod = |mode| {
            fs::set_permissions(home.skill(), fs::Permissions::from_mode(mode)).expect("chmod");
        };
        chmod(0o500);
        let is_root = fs::write(home.skill().join("probe"), "").is_ok();
        chmod(0o700);
        if is_root {
            return; // the mode does not bind
        }
        let before = tree(&home.skill());
        let list = |log: &str| {
            chmod(0o500);
            let said = home.finish(
                home.atl()
                    .env("RUST_LOG", log)
                    .args(["--config", "atl.toml", "-F", "json", "alias", "list"]),
            );
            chmod(0o700);
            said
        };
        let (code, stdout, stderr) = list("info");
        assert_eq!((code, stderr.as_str()), (0, ""));
        // The log goes to stderr; what the command printed is as it was.
        let (code, again, stderr) = list("debug");
        assert_eq!((code, again.as_str()), (0, stdout.as_str()));
        assert!(
            stderr.contains("DEBUG") && stderr.contains("the atl skill was not updated: "),
            "{stderr}"
        );
        assert_eq!(tree(&home.skill()), before);
        home.atl().args(["completions", "bash"]).assert().success();
        assert_eq!(home.version(), VERSION);
    }

    /// What is printed is out before the skill is looked at, and all of it.
    #[test]
    fn the_output_is_whole_in_a_pipe_and_in_a_file() {
        let home = home();
        let out = fs::File::create(home.root.join("out.txt")).expect("create");
        let mut atl = std::process::Command::new(assert_cmd::cargo::cargo_bin("atl"));
        atl.env_clear()
            .envs(std::env::vars_os().filter(|(key, _)| key == "LLVM_PROFILE_FILE"))
            .env("HOME", &home.root)
            .current_dir(&home.root)
            .args(["completions", "bash"])
            .stdin(Stdio::null());
        let status = atl.stdout(out).status().expect("run atl");
        assert!(status.success());
        assert_eq!(home.version(), VERSION);
        let in_file = fs::read(home.root.join("out.txt")).expect("read");
        let in_pipe = home
            .atl()
            .args(["completions", "bash"])
            .output()
            .expect("run atl")
            .stdout;
        assert!(!in_file.is_empty());
        assert!(in_file == in_pipe);
    }

    #[test]
    fn many_ordinary_commands_at_once() {
        let home = home();
        let children: Vec<_> = (0..16)
            .map(|_| {
                std::process::Command::new(assert_cmd::cargo::cargo_bin("atl"))
                    .env_clear()
                    .envs(std::env::vars_os().filter(|(key, _)| key == "LLVM_PROFILE_FILE"))
                    .env("HOME", &home.root)
                    .current_dir(&home.root)
                    .args(["--config", "atl.toml", "-F", "json", "alias", "list"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("spawn atl")
            })
            .collect();
        for child in children {
            let output = child.wait_with_output().expect("wait");
            assert!(output.status.success());
            assert_eq!(output.stderr.len(), 0);
            let listed: Value = serde_json::from_slice(&output.stdout).expect("JSON");
            assert_eq!(listed[0]["name"], json!("hi"));
        }
        home.atl().args(["completions", "bash"]).assert().success();
        assert_eq!(home.version(), VERSION);
        assert!(
            !tree(&home.skill())
                .iter()
                .any(|(path, _)| path.contains(".tmp"))
        );
        assert!(home.ok(&["status"]).contains("state: up_to_date"));
    }

    /// A first install that failed midway: the next ordinary command that
    /// can finishes it, and says nothing of it.
    #[test]
    #[cfg(unix)]
    fn an_ordinary_command_finishes_an_install_that_failed_midway() {
        if !modes_bind() {
            return;
        }
        let home = Home::with_alias();
        let references = home.skill().join("references");
        fs::create_dir_all(&references).expect("mkdir");
        chmod(&references, 0o500);
        home.failed(&["install"]);
        let list = ["--config", "atl.toml", "-F", "json", "alias", "list"];
        let other = Home::with_alias();
        let expected = other.finish(other.atl().args(list));
        let listed = || home.finish(home.atl().args(list));
        assert_eq!(listed(), expected);
        assert_eq!(home.status(&[])["state"], json!("differs"));
        chmod(&references, 0o700);
        assert_eq!(listed(), expected);
        assert_eq!(home.version(), VERSION);
        assert_eq!(home.status(&[])["state"], json!("up_to_date"));
    }

    /// A reader that is gone: the process may die of SIGPIPE before it comes
    /// to the skill, or after; the skill is of one version, and the next run
    /// brings it to this one.
    #[test]
    #[cfg(unix)]
    fn a_reader_that_is_gone_leaves_a_whole_skill() {
        let atl = assert_cmd::cargo::cargo_bin("atl");
        for args in ["--version", "completions bash"] {
            let home = home();
            let mut sh = Command::new("/bin/sh");
            sh.env_clear()
                .envs(std::env::vars_os().filter(|(key, _)| key == "LLVM_PROFILE_FILE"))
                .env("HOME", &home.root)
                .env("PATH", "/usr/bin:/bin")
                .env("ATL", &atl)
                .current_dir(&home.root)
                .args(["-c", &format!("{{ \"$ATL\" {args}; echo $? >code; }} | :")]);
            let (code, stdout, stderr) = home.finish(&mut sh);
            assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "", ""));
            let code = fs::read_to_string(home.root.join("code")).expect("read");
            assert!(["0\n", "141\n"].contains(&code.as_str()), "{code}");
            assert!([OLD.version, VERSION].contains(&home.version().as_str()));
            home.atl().args(["completions", "bash"]).assert().success();
            assert_eq!(home.version(), VERSION);
        }
    }

    /// What a write cannot keep of a file's access is a warning of `atl skill
    /// update`, and nothing of an ordinary command. On macOS, where a file of
    /// a group the user is not in is made without being root.
    #[test]
    #[cfg(target_os = "macos")]
    fn an_ordinary_command_says_nothing_of_access_that_is_lost() {
        use std::os::unix::fs::MetadataExt;
        let home_of_wheel = || {
            let home = home();
            let at = home.skill().join("SKILL.md");
            let made = tempfile::NamedTempFile::new_in("/private/tmp").expect("temp file");
            fs::write(made.path(), "old skill\n").expect("write");
            made.persist(&at).expect("mv");
            let group = fs::metadata(&at).expect("stat").gid();
            (home, at, group)
        };
        let (home, at, group) = home_of_wheel();
        let (code, stdout, stderr) = home.run(&["update"]);
        assert_eq!(code, 0, "{stderr}");
        if fs::metadata(&at).expect("stat").gid() == group {
            return; // kept: the user is in that group
        }
        assert!(stderr.contains("WARN"), "{stderr}");
        assert!(!stdout.contains("WARN"), "{stdout}");
        let (home, _, _) = home_of_wheel();
        let list = ["--config", "atl.toml", "-F", "json", "alias", "list"];
        let other = Home::with_alias();
        let expected = other.finish(other.atl().args(list));
        assert_eq!(home.finish(home.atl().args(list)), expected);
        assert_eq!(home.version(), VERSION);
    }
}

/// What a line has that starts the binary. In pieces, so that this file does
/// not find itself.
const STARTS: [&str; 3] = [
    concat!("cargo_", "bin("),
    concat!("cargo_", "bin!"),
    concat!("CARGO_", "BIN_EXE_"),
];

/// The lines of `text` that start the binary, with whether it is given a
/// `HOME` of the test's: an `.env("HOME", …)` whose value is not read from
/// the environment, within 16 lines and before the next start. Anything
/// else, `.env_remove("HOME")` and a comment among it, leaves the binary
/// with the `HOME` the tests run with, or is not seen to change it.
fn starts(text: &str) -> Vec<(usize, bool)> {
    let is_code = |line: &str| !line.trim_start().starts_with("//");
    let lines: Vec<&str> = text.lines().collect();
    let found: Vec<usize> = (0..lines.len())
        .filter(|n| is_code(lines[*n]) && STARTS.iter().any(|name| lines[*n].contains(name)))
        .collect();
    let sets_home = |line: &&str| {
        is_code(line)
            && line
                .split_once(".env(\"HOME\", ")
                .is_some_and(|(_, value)| !value.contains("var") && !value.contains("env!"))
    };
    found
        .iter()
        .enumerate()
        .map(|(i, start)| {
            let next = found.get(i + 1).copied().unwrap_or(lines.len());
            let end = next.min(start + 16);
            (start + 1, lines[*start..end].iter().any(sets_home))
        })
        .collect()
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(at) = todo.pop() {
        for entry in fs::read_dir(&at).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                todo.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    found
}

/// Every place in the tests of this crate that starts the binary gives it a
/// `HOME`: under `cargo test --release` the binary may update the skill of
/// the home directory it has.
#[test]
fn no_test_runs_the_binary_in_the_developers_home() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut seen = 0;
    for path in rust_files(&tests) {
        let text = fs::read_to_string(&path).expect("read");
        for (line, has_home) in starts(&text) {
            seen += 1;
            assert!(
                has_home,
                "{}:{line}: starts atl with the HOME it inherits",
                path.display()
            );
        }
    }
    assert!(seen >= 5, "found {seen}");
}

#[test]
fn a_start_that_inherits_home_is_found() {
    let start = STARTS[0];
    for (rest, has_home) in [
        ("cmd.env_clear();\ncmd.env(\"HOME\", \"/tmp\");", true),
        (".env(\"HOME\", dir.path())", true),
        (".args([\"alias\"])", false),
        (".env(\"HOME\", std::env::var_os(\"HOME\").unwrap())", false),
        (".env(\"HOME\", env!(\"HOME\"))", false),
        (".env_remove(\"HOME\")", false),
        ("// .env(\"HOME\", \"/tmp\")", false),
    ] {
        let text = format!("{start}\"atl\")\n{rest}\n");
        assert_eq!(starts(&text), [(1, has_home)], "{rest}");
    }
    // The HOME of the next start is not this one's.
    let text = format!("{start}\n{start}\n.env(\"HOME\", \"/tmp\")\n");
    assert_eq!(starts(&text), [(1, false), (2, true)]);
    assert_eq!(starts(&format!("// {start}\n")), []);
}
