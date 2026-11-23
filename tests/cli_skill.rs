//! `atl skill`, driven through the real binary in a home directory of its own.
//!
//! Every subcommand reports one object through `write_output`; most tests
//! read it as JSON (`-F json`), a few pin what the console shows. Every
//! refusal exits 5 (`Error::InvalidInput`) with one line on stderr and
//! nothing written; a file that cannot be read or written, and a lock held by
//! another atl for longer than ten seconds, exit 1.
//!
//! The binary under `cargo test` is a debug build, which never updates a
//! skill by itself: that is `cli_skill_release.rs`, with the guard that no
//! test of this crate runs the binary in the developer's home directory.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use atl::skill::{Edition, Environment, SkillFile};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

#[path = "skill/home.rs"]
mod home;
use home::*;

#[path = "skill/concurrent.rs"]
mod concurrent;
#[path = "skill/project.rs"]
mod project;
#[path = "skill/refusals.rs"]
mod refusals;
#[path = "skill/status.rs"]
mod status;
#[path = "skill/writes.rs"]
mod writes;

/// What `install` or `update` of the skill of the home directory reports:
/// nothing kept, no link.
fn wrote(action: &str, version: &str, previous: &str) -> Value {
    json!({
        "path": "~/.agents/skills/atl",
        "action": action,
        "version": version,
        "previous": previous,
        "kept": [],
        "link": "",
        "link_action": "",
    })
}

/// `report` with the link of the home directory, and what was done of it.
fn linked(mut report: Value, link_action: &str) -> Value {
    report["link"] = json!("~/.claude/skills/atl");
    report["link_action"] = json!(link_action);
    report
}

/// What `remove` of the skill of the home directory reports when it took the
/// directory, and no link.
fn removed(version: &str) -> Value {
    json!({
        "path": "~/.agents/skills/atl",
        "action": "removed",
        "version": version,
        "link": "",
        "dir_kept": false,
        "left": [],
        "other": [],
    })
}

/// `report` of a `remove` that kept the directory, with `left` and `other`.
fn kept(mut report: Value, left: &[&str], other: &[&str]) -> Value {
    report["dir_kept"] = json!(true);
    report["left"] = json!(left);
    report["other"] = json!(other);
    report
}

/// What `update` and `remove` report where atl installed nothing.
fn none_installed(command: &str) -> Value {
    match command {
        "remove" => json!({
            "path": "~/.agents/skills/atl",
            "action": "not_installed",
            "version": "",
            "link": "",
            "dir_kept": false,
            "left": [],
            "other": [],
        }),
        _ => wrote("not_installed", "", ""),
    }
}

#[test]
fn install_writes_the_skill_its_manifest_and_the_link() {
    let home = Home::with_claude();
    assert_eq!(
        home.ok(&["install"]),
        format!(
            "path: ~/.agents/skills/atl\naction: installed\nversion: {VERSION}\nprevious: \n\
             kept: []\nlink: ~/.claude/skills/atl\nlink_action: created\n"
        )
    );
    let files: Vec<String> = tree(&home.skill())
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(files, FILES);
    for path in &files[1..] {
        let written = fs::read_to_string(home.skill().join(path)).expect("read");
        assert_eq!(written, source(path), "{path}");
    }
    let manifest: Value =
        serde_json::from_slice(&fs::read(home.skill().join(MANIFEST)).expect("read"))
            .expect("JSON");
    assert_eq!(manifest["schema"], json!(1));
    assert_eq!(manifest["atl_version"], json!(VERSION));
    assert_eq!(manifest["files"].as_object().expect("object").len(), 4);
    assert_eq!(
        fs::read_link(home.link()).expect("a link"),
        Path::new("../../.agents/skills/atl")
    );
    assert!(
        home.link().join("SKILL.md").is_file(),
        "the link leads there"
    );
    assert_eq!(
        home.ok(&["install"]),
        format!(
            "path: ~/.agents/skills/atl\naction: up_to_date\nversion: {VERSION}\n\
             previous: {VERSION}\nkept: []\nlink: ~/.claude/skills/atl\nlink_action: unchanged\n"
        )
    );
}

/// `-q` only quiets the log: what a command reports is what it reports.
#[test]
fn quiet_changes_nothing_of_the_output() {
    let (home, quiet) = (Home::with_claude(), Home::with_claude());
    for args in [
        &["update"][..],
        &["remove"],
        &["install"],
        &["install"],
        &["status"],
        &["update"],
        &["remove"],
    ] {
        let said = home.ok(args);
        assert_eq!(quiet.ok(&[args, &["-q"]].concat()), said, "{args:?}");
        assert_eq!(tree(&quiet.root), tree(&home.root), "{args:?}");
    }
    fs::create_dir_all(home.link()).expect("mkdir");
    assert_eq!(
        home.refused(&["install", "-q"]),
        "~/.claude/skills/atl is not a link, and atl did not write it: move it away first"
    );
}

#[test]
fn status_reports_an_object_of_a_fixed_shape() {
    let home = Home::new();
    assert_eq!(
        home.status(&[]),
        json!({
            "path": "~/.agents/skills/atl",
            "installed": false,
            "state": "not_installed",
            "version": "",
            "atl_version": VERSION,
            "edited": [],
            "missing": [],
            "colliding": [],
            "in_the_way": [],
            "unlisted": [],
            "link": "",
            "link_state": "not_wanted",
        })
    );
    fs::create_dir(home.root.join(".claude")).expect("mkdir");
    home.ok(&["install"]);
    fs::write(home.skill().join("SKILL.md"), "mine").expect("write");
    fs::write(home.skill().join("notes.md"), "mine").expect("write");
    fs::remove_file(home.skill().join("references/body-formats.md")).expect("rm");
    assert_eq!(
        home.status(&[]),
        json!({
            "path": "~/.agents/skills/atl",
            "installed": true,
            "state": "up_to_date",
            "version": VERSION,
            "atl_version": VERSION,
            "edited": ["SKILL.md"],
            "missing": ["references/body-formats.md"],
            "colliding": [],
            "in_the_way": [],
            "unlisted": ["notes.md"],
            "link": "~/.claude/skills/atl",
            "link_state": "ok",
        })
    );
    assert_eq!(
        home.ok(&["status"]),
        format!(
            "path: ~/.agents/skills/atl\ninstalled: true\nstate: up_to_date\nversion: {VERSION}\n\
             atl_version: {VERSION}\nedited: [\"SKILL.md\"]\n\
             missing: [\"references/body-formats.md\"]\ncolliding: []\nin_the_way: []\n\
             unlisted: [\"notes.md\"]\nlink: ~/.claude/skills/atl\nlink_state: ok\n"
        )
    );
    for format in ["toml", "toon", "csv", "console"] {
        assert!(
            home.ok(&["status", "-F", format]).contains("SKILL.md"),
            "{format}"
        );
    }
}

/// `--jq` takes from the object, `--template` renders it; a refusal is one
/// whatever the output would have been.
#[test]
fn the_object_goes_through_jq_and_templates() {
    let home = Home::with_claude();
    let (code, stdout, stderr) = home.finish(
        home.atl()
            .args(["-F", "json", "--jq", "{action, link_action}", "skill"])
            .arg("install"),
    );
    assert_eq!((code, stderr.as_str()), (0, ""));
    assert_eq!(
        serde_json::from_str::<Value>(&stdout).expect("JSON"),
        json!({"action": "installed", "link_action": "created"})
    );
    let jq = |args: &[&str]| {
        let mut atl = home.atl();
        atl.args(["-F", "json", "--jq"]).args(args);
        let (code, stdout, stderr) = home.finish(&mut atl);
        assert_eq!((code, stderr.as_str()), (0, ""), "{args:?}");
        serde_json::from_str::<Value>(&stdout).expect("JSON")
    };
    assert_eq!(jq(&[".state", "skill", "status"]), json!("up_to_date"));
    assert_eq!(jq(&[".kept", "skill", "update"]), json!([]));
    assert_eq!(
        jq(&["[.action, .dir_kept, .link]", "skill", "remove"]),
        json!(["removed", false, "~/.claude/skills/atl"])
    );
    let (code, stdout, stderr) = home.finish(home.atl().args([
        "--template",
        "{{ action }} v{{ version }} in {{ path }}",
        "skill",
        "install",
    ]));
    assert_eq!(
        (code, stdout.as_str(), stderr.as_str()),
        (
            0,
            format!("installed v{VERSION} in ~/.agents/skills/atl\n").as_str(),
            ""
        )
    );
    fs::write(home.skill().join("SKILL.md"), "mine").expect("write");
    for transform in [&["--jq", ".action"][..], &["--template", "{{ action }}"]] {
        let (code, stdout, message) =
            home.refused_with(home.atl().args(transform).args(["skill", "update"]));
        assert_eq!((code, stdout.as_str()), (5, ""), "{transform:?}");
        assert!(message.contains("edited SKILL.md"), "{message}");
    }
}

#[test]
fn update_and_remove_with_nothing_installed_say_so() {
    let home = Home::new();
    for command in ["update", "remove"] {
        assert_eq!(home.json(&[command]), none_installed(command));
    }
    assert_eq!(
        home.ok(&["update"]),
        "path: ~/.agents/skills/atl\naction: not_installed\nversion: \nprevious: \nkept: []\n\
         link: \nlink_action: \n"
    );
    assert_eq!(
        home.ok(&["remove"]),
        "path: ~/.agents/skills/atl\naction: not_installed\nversion: \nlink: \n\
         dir_kept: false\nleft: []\nother: []\n"
    );
    assert_eq!(tree(&home.root), []);
}

#[test]
fn update_restores_a_missing_file_and_refuses_an_edited_one() {
    let home = Home::new();
    home.ok(&["install"]);
    let skill_md = home.skill().join("SKILL.md");
    fs::remove_file(&skill_md).expect("rm");
    assert_eq!(home.json(&["update"]), wrote("restored", VERSION, VERSION));
    fs::write(&skill_md, "mine").expect("write");
    for command in ["install", "update"] {
        assert_eq!(
            home.refused(&[command]),
            "~/.agents/skills/atl was changed since atl wrote it: edited SKILL.md \
             (--force writes atl's files in their place)"
        );
    }
    assert_eq!(
        home.refused(&["remove"]),
        "~/.agents/skills/atl was changed since atl wrote it: edited SKILL.md \
         (--force removes the edited files too)"
    );
    assert_eq!(
        home.json(&["update", "--force"]),
        wrote("restored", VERSION, VERSION)
    );
    assert_eq!(
        fs::read_to_string(&skill_md).expect("read"),
        source("SKILL.md")
    );
}

#[test]
fn a_foreign_directory_is_taken_over_only_by_force_and_loses_only_atls_files() {
    let home = Home::new();
    fs::create_dir_all(home.skill()).expect("mkdir");
    fs::write(home.skill().join("SKILL.md"), "theirs").expect("write");
    fs::write(home.skill().join("notes.md"), "theirs").expect("write");
    for command in ["install", "update"] {
        assert_eq!(
            home.refused(&[command]),
            "~/.agents/skills/atl was not installed by atl (SKILL.md, notes.md): \
             --force writes atl's files into it and keeps the rest"
        );
    }
    for args in [&["remove"][..], &["remove", "--force"]] {
        assert_eq!(
            home.refused(args),
            "~/.agents/skills/atl was not installed by atl (it has no .atl-skill.json): \
             remove it yourself"
        );
    }
    assert_eq!(
        home.json(&["install", "--force"]),
        wrote("installed", VERSION, "")
    );
    assert_eq!(home.status(&[])["unlisted"], json!(["notes.md"]));
    // Atl's from here on: what is not atl's stops nothing.
    let up_to_date = wrote("up_to_date", VERSION, VERSION);
    assert_eq!(home.json(&["update"]), up_to_date);
    assert_eq!(home.json(&["install"]), up_to_date);
    assert_eq!(
        home.json(&["remove"]),
        kept(removed(VERSION), &["notes.md"], &[])
    );
    assert_eq!(
        tree(&home.skill()),
        [("notes.md".to_owned(), "theirs".to_owned())]
    );
}

/// A file atl did not write, where this version has one: the manifest is one
/// that does not list it, as an older atl's.
#[test]
fn a_file_in_the_place_of_a_new_one_is_replaced_only_by_force() {
    let home = Home::new();
    home.ok(&["install"]);
    let manifest = home.skill().join(MANIFEST);
    let mut value: Value =
        serde_json::from_slice(&fs::read(&manifest).expect("read")).expect("JSON");
    value["files"]
        .as_object_mut()
        .expect("object")
        .remove("references/body-formats.md");
    let older = value.to_string();
    let formats = home.skill().join("references/body-formats.md");
    let outside = home.root.join("outside.md");
    fs::write(&outside, "theirs").expect("write");
    for link in [false, true] {
        fs::write(&manifest, &older).expect("write");
        fs::remove_file(&formats).expect("rm");
        if link {
            symlink(&outside, &formats).expect("symlink");
        } else {
            fs::write(&formats, "theirs").expect("write");
        }
        for command in ["install", "update"] {
            assert_eq!(
                home.refused(&[command]),
                "~/.agents/skills/atl was changed since atl wrote it: \
                 not atl's: references/body-formats.md (--force writes atl's files in their place)"
            );
        }
        home.ok(&["update", "--force"]);
        assert!(!formats.is_symlink());
        assert_eq!(
            fs::read_to_string(&formats).expect("read"),
            source("references/body-formats.md")
        );
        assert_eq!(fs::read_to_string(&outside).expect("read"), "theirs");
    }

    fs::write(&manifest, &older).expect("write");
    fs::remove_file(&formats).expect("rm");
    fs::create_dir(&formats).expect("mkdir");
    for args in [&["update"][..], &["update", "--force"]] {
        assert_eq!(
            home.refused(args),
            "~/.agents/skills/atl cannot hold the files of the skill \
             (references/body-formats.md): atl neither goes through a link nor replaces a \
             directory"
        );
    }
    // Removing writes nothing there, so nothing is in its way.
    assert_eq!(
        home.json(&["remove"]),
        kept(removed(VERSION), &[], &["references/body-formats.md"])
    );
}

#[test]
fn remove_takes_the_files_the_directory_and_the_link() {
    let home = Home::with_claude();
    home.ok(&["install"]);
    let mut expected = removed(VERSION);
    expected["link"] = json!("~/.claude/skills/atl");
    assert_eq!(home.json(&["remove"]), expected);
    assert_eq!(tree(&home.root), []);
    assert!(!home.skill().exists());
    assert!(
        home.root.join(".agents/skills").is_dir(),
        "not atl's to remove"
    );
}

#[test]
fn a_skill_directory_that_is_a_link_is_refused_always() {
    let home = Home::new();
    let checkout = home.root.join("checkout");
    fs::create_dir_all(&checkout).expect("mkdir");
    fs::create_dir_all(home.root.join(".agents/skills")).expect("mkdir");
    symlink(&checkout, home.skill()).expect("symlink");
    for command in ["install", "update", "remove"] {
        for args in [&[command][..], &[command, "--force"]] {
            assert_eq!(
                home.refused(args),
                "~/.agents/skills/atl is a symbolic link: \
                 atl neither writes through it nor replaces it"
            );
        }
    }
    assert_eq!(home.status(&[])["state"], json!("blocked"));
}

#[test]
fn a_link_inside_the_skill_is_never_written_or_deleted_through() {
    let home = Home::new();
    home.ok(&["install"]);
    let outside = home.root.join("outside");
    fs::rename(home.skill().join("references"), &outside).expect("mv");
    symlink(&outside, home.skill().join("references")).expect("symlink");
    for command in ["install", "update", "remove"] {
        assert_eq!(
            home.refused(&[command, "--force"]),
            "~/.agents/skills/atl cannot hold the files of the skill (references): \
             atl neither goes through a link nor replaces a directory"
        );
    }
}

/// A link in the place of a file of atl's, or of the manifest: what it leads
/// to is neither written nor deleted, and `--force` replaces the link itself.
#[test]
fn a_link_in_the_place_of_a_file_is_replaced_not_followed() {
    let home = Home::new();
    home.ok(&["install"]);
    let outside = home.root.join("outside");
    for (path, refusal) in [
        (
            "SKILL.md",
            "was changed since atl wrote it: edited SKILL.md",
        ),
        (
            "references/jira-commands.md",
            "was changed since atl wrote it: edited references/jira-commands.md",
        ),
        (MANIFEST, "its .atl-skill.json cannot be used"),
    ] {
        let at = home.skill().join(path);
        for command in [&["update", "--force"][..], &["install", "--force"]] {
            // To a file that is there, then to one that is not.
            for text in [Some("theirs"), None] {
                let _ = fs::remove_file(&outside);
                if let Some(text) = text {
                    fs::write(&outside, text).expect("write");
                }
                fs::remove_file(&at).expect("rm");
                symlink(&outside, &at).expect("symlink");
                let said = home.refused(&["update"]);
                assert!(said.contains(refusal), "{said}");
                home.ok(command);
                assert!(at.is_file() && !at.is_symlink(), "{path}");
                assert_eq!(fs::read_to_string(&outside).ok().as_deref(), text, "{path}");
                assert_eq!(home.status(&[])["edited"], json!([]));
            }
        }
    }
    let skill_md = home.skill().join("SKILL.md");
    fs::write(&outside, "theirs").expect("write");
    fs::remove_file(&skill_md).expect("rm");
    symlink(&outside, &skill_md).expect("symlink");
    home.ok(&["remove", "--force"]);
    assert!(!home.skill().exists());
    assert_eq!(fs::read_to_string(&outside).expect("read"), "theirs");
}

#[test]
fn a_link_above_the_skill_directory_is_allowed() {
    let home = Home::with_claude();
    fs::create_dir_all(home.root.join("dotfiles/agents")).expect("mkdir");
    fs::create_dir_all(home.root.join("dotfiles/claude")).expect("mkdir");
    fs::remove_dir(home.root.join(".claude")).expect("rmdir");
    symlink("dotfiles/agents", home.root.join(".agents")).expect("symlink");
    symlink("dotfiles/claude", home.root.join(".claude")).expect("symlink");
    assert_eq!(
        home.json(&["install"]),
        linked(wrote("installed", VERSION, ""), "created")
    );
    assert!(
        home.root
            .join("dotfiles/agents/skills/atl/SKILL.md")
            .is_file()
    );
    assert!(
        home.link().join("SKILL.md").is_file(),
        "the link leads there"
    );
    assert_eq!(home.status(&[])["link_state"], json!("ok"));
}

#[test]
fn claude_code_reading_the_shared_directory_needs_no_link() {
    let home = Home::with_claude();
    symlink("../.agents/skills", home.root.join(".claude/skills")).expect("symlink");
    assert_eq!(
        home.json(&["install"]),
        linked(wrote("installed", VERSION, ""), "unchanged")
    );
    assert_eq!(home.status(&[])["link_state"], json!("ok"));
    assert!(home.skill().join("SKILL.md").is_file());
    assert_eq!(home.json(&["remove"]), removed(VERSION), "no link removed");
    assert!(home.root.join(".claude/skills").is_symlink());
}

#[test]
fn only_another_link_in_the_place_of_the_link_is_replaced_and_only_by_force() {
    let home = Home::with_claude();
    fs::create_dir(home.root.join(".claude/skills")).expect("mkdir");
    symlink("/nowhere", home.link()).expect("symlink");
    assert_eq!(
        home.refused(&["install"]),
        "~/.claude/skills/atl is a link to '/nowhere': --force replaces it"
    );
    assert_eq!(home.status(&[])["link_state"], json!("elsewhere"));
    assert_eq!(
        home.json(&["install", "--force"]),
        linked(wrote("installed", VERSION, ""), "replaced")
    );
    assert!(home.link().join("SKILL.md").is_file());

    let home = Home::with_claude();
    fs::create_dir_all(home.link()).expect("mkdir");
    fs::write(home.link().join("SKILL.md"), "theirs").expect("write");
    for args in [&["install"][..], &["install", "--force"]] {
        assert_eq!(
            home.refused(args),
            "~/.claude/skills/atl is not a link, and atl did not write it: move it away first"
        );
    }
}

/// `update` leaves the link alone: it reports none, whatever is there.
#[test]
fn update_makes_no_link() {
    let home = Home::new();
    home.ok(&["install"]);
    fs::create_dir(home.root.join(".claude")).expect("mkdir");
    fs::remove_file(home.skill().join("SKILL.md")).expect("rm");
    assert_eq!(home.json(&["update"]), wrote("restored", VERSION, VERSION));
    assert!(!home.link().exists());
    assert_eq!(home.status(&[])["link_state"], json!("missing"));
    assert_eq!(
        home.json(&["install"]),
        linked(wrote("up_to_date", VERSION, VERSION), "created")
    );
}

#[test]
fn claude_config_dir_is_where_the_link_goes() {
    let home = Home::with_claude();
    let claude = home.root.join("config/claude");
    let atl = |args: &[&str]| {
        home.atl()
            .env("CLAUDE_CONFIG_DIR", &claude)
            .arg("skill")
            .args(args)
            .assert()
            .success();
    };
    atl(&["install"]);
    assert!(
        !home.link().exists(),
        "no ~/.claude when the variable is set"
    );
    fs::create_dir_all(&claude).expect("mkdir");
    atl(&["install"]);
    assert_eq!(
        fs::read_link(claude.join("skills/atl")).expect("a link"),
        Path::new("../../../.agents/skills/atl")
    );
}

#[test]
fn project_is_the_current_directory_and_needs_no_home() {
    let home = Home::new();
    let project = home.root.join("project");
    fs::create_dir_all(project.join(".claude")).expect("mkdir");
    let atl = |args: &[&str]| {
        home.atl()
            .env_remove("HOME")
            .current_dir(&project)
            .args(["skill"])
            .args(args)
            .assert()
            .success();
    };
    atl(&["install", "--project"]);
    assert!(project.join(".agents/skills/atl/SKILL.md").is_file());
    assert!(project.join(".claude/skills/atl/SKILL.md").is_file());
    atl(&["update", "--project"]);
    atl(&["status", "--project"]);
    atl(&["remove", "--project"]);
    assert_eq!(tree(&home.root), []);
}

#[test]
fn a_home_that_is_none_is_refused() {
    let home = Home::new();
    for (value, message) in [
        (
            None,
            "HOME is not set, so there is no home directory to look for the skill in \
             (--project works in the current directory)",
        ),
        (
            Some("rel"),
            "HOME is 'rel', which is no absolute path of a directory",
        ),
        (
            Some("/no/such/home"),
            "HOME is '/no/such/home', which is no absolute path of a directory",
        ),
    ] {
        for command in ["install", "update", "status", "remove"] {
            let mut atl = home.atl();
            match value {
                Some(value) => atl.env("HOME", value),
                None => atl.env_remove("HOME"),
            };
            let said = home.refused_with(atl.args(["skill", command]));
            assert_eq!(said, (5, String::new(), message.to_owned()), "{command}");
        }
    }
}

/// An unwritable directory is a write failure, an unreadable file a read
/// failure: both exit 1, and say which.
#[test]
fn file_errors_exit_1() {
    let home = Home::new();
    if !modes_bind() {
        return;
    }
    chmod(&home.root, 0o500);
    let said = home.failed(&["install"]);
    chmod(&home.root, 0o700);
    assert!(said.starts_with("Error: cannot write ~/.agents"), "{said}");
    home.ok(&["install"]);
    let skill_md = home.skill().join("SKILL.md");
    chmod(&skill_md, 0o000);
    let said = home.failed(&["update"]);
    let status = home.failed(&["status"]);
    chmod(&skill_md, 0o600);
    for said in [said, status] {
        assert!(
            said.starts_with("Error: cannot read ~/.agents/skills/atl/SKILL.md: "),
            "{said}"
        );
    }
}

/// Another atl holds the lock for longer than ten seconds: exit 1, nothing
/// written; a command that only looks does not wait.
#[test]
fn a_lock_held_for_too_long_is_a_write_failure() {
    let home = Home::new();
    home.install_edition(&OLD);
    let before = tree(&home.root);
    let lock = fs::File::open(home.skill()).expect("open");
    lock.lock().expect("lock");
    assert_eq!(home.status(&[])["state"], json!("older"));
    let start = std::time::Instant::now();
    let said = home.failed(&["update"]);
    assert!(
        start.elapsed() >= std::time::Duration::from_secs(9),
        "did not wait"
    );
    assert_eq!(
        said,
        "Error: cannot write ~/.agents/skills/atl: another atl is writing the skill\n"
    );
    assert_eq!(tree(&home.root), before);
    drop(lock);
    assert_eq!(
        home.json(&["update"]),
        wrote("updated", VERSION, OLD.version)
    );
}

/// Every kind of refusal (`atl::skill::Refusal`), through the binary: exit
/// 5, one line, nothing written. The two that need root through the binary,
/// `ForeignOwner` and `OtherOwner`, are `root_is_refused_in_another_users_home`
/// and `refusals::a_skill_directory_of_another_user_is_refused` (the root lane
/// of the container), and the handler's tests of `atl::cli::commands::skill`
/// for anyone.
#[test]
fn every_refusal_exits_5() {
    type Case = (&'static str, fn(&Home) -> Command, &'static str);
    fn skill(home: &Home, args: &[&str]) -> Command {
        let mut atl = home.atl();
        atl.arg("skill").args(args);
        atl
    }
    let cases: [Case; 14] = [
        (
            "NoHome",
            |home| {
                let mut atl = skill(home, &["install"]);
                atl.env_remove("HOME");
                atl
            },
            "HOME is not set, ",
        ),
        (
            "BadHome",
            |home| {
                let mut atl = skill(home, &["install"]);
                atl.env("HOME", "rel");
                atl
            },
            "HOME is 'rel', ",
        ),
        (
            "BadClaudeDir",
            |home| {
                let mut atl = skill(home, &["install"]);
                atl.env("CLAUDE_CONFIG_DIR", "rel");
                atl
            },
            "CLAUDE_CONFIG_DIR is 'rel', ",
        ),
        (
            "NoCurrentDir",
            |home| {
                let mut sh = Command::new("/bin/sh");
                let atl = assert_cmd::cargo::cargo_bin("atl");
                sh.env_clear().env("HOME", &home.root).env("ATL", atl);
                sh.current_dir(&home.root).args([
                    "-c",
                    "mkdir gone && cd gone && rmdir ../gone && \
                     exec \"$ATL\" skill install --project",
                ]);
                sh
            },
            "the current directory has no path atl can use ",
        ),
        (
            "DirectoryIsLink",
            |home| {
                fs::create_dir_all(home.root.join(".agents/skills")).expect("mkdir");
                symlink(&home.root, home.skill()).expect("symlink");
                skill(home, &["install"])
            },
            "~/.agents/skills/atl is a symbolic link: ",
        ),
        (
            "NotADirectory",
            |home| {
                fs::create_dir_all(home.root.join(".agents/skills")).expect("mkdir");
                fs::write(home.skill(), "theirs").expect("write");
                skill(home, &["install"])
            },
            "~/.agents/skills/atl is not a directory: ",
        ),
        (
            "Foreign",
            |home| {
                fs::create_dir_all(home.skill()).expect("mkdir");
                fs::write(home.skill().join("notes.md"), "theirs").expect("write");
                skill(home, &["install"])
            },
            "~/.agents/skills/atl was not installed by atl (notes.md): ",
        ),
        (
            "NotOurs",
            |home| {
                fs::create_dir_all(home.skill()).expect("mkdir");
                fs::write(home.skill().join("notes.md"), "theirs").expect("write");
                skill(home, &["remove"])
            },
            "~/.agents/skills/atl was not installed by atl (it has no .atl-skill.json): ",
        ),
        (
            "DamagedManifest",
            |home| {
                home.install_edition(&OLD);
                home.write_manifest("{}");
                skill(home, &["update"])
            },
            "atl cannot tell which files of ~/.agents/skills/atl it wrote: ",
        ),
        (
            "NewerSchema",
            |home| {
                home.install_edition(&OLD);
                home.write_manifest(r#"{"schema": 2}"#);
                skill(home, &["update", "--force"])
            },
            "the skill in ~/.agents/skills/atl was installed by a newer atl ",
        ),
        (
            "Changed",
            |home| {
                home.install_edition(&OLD);
                fs::write(home.skill().join("SKILL.md"), "mine").expect("write");
                skill(home, &["update"])
            },
            "~/.agents/skills/atl was changed since atl wrote it: edited SKILL.md ",
        ),
        (
            "Blocked",
            |home| {
                home.install_edition(&OLD);
                fs::remove_file(home.skill().join("SKILL.md")).expect("rm");
                fs::create_dir(home.skill().join("SKILL.md")).expect("mkdir");
                skill(home, &["update", "--force"])
            },
            "~/.agents/skills/atl cannot hold the files of the skill (SKILL.md): ",
        ),
        (
            "OtherLink",
            |home| {
                fs::create_dir_all(home.root.join(".claude/skills")).expect("mkdir");
                symlink("/nowhere", home.link()).expect("symlink");
                skill(home, &["install"])
            },
            "~/.claude/skills/atl is a link to '/nowhere': ",
        ),
        (
            "LinkPlaceTaken",
            |home| {
                fs::create_dir_all(home.link()).expect("mkdir");
                skill(home, &["install", "--force"])
            },
            "~/.claude/skills/atl is not a link, and atl did not write it: ",
        ),
    ];
    for (name, make, says) in cases {
        let home = Home::with_claude();
        let mut cmd = make(&home);
        let (code, stdout, message) = home.refused_with(&mut cmd);
        assert_eq!((code, stdout.as_str()), (5, ""), "{name}: {message}");
        assert!(message.starts_with(says), "{name}: {message}");
    }
}

/// Run by root only, as in a container: the home directory is another
/// user's (`sudo atl skill …`). Nothing is written, `--force` or not, and
/// `status` says whose it is.
#[test]
fn root_is_refused_in_another_users_home() {
    if modes_bind() {
        return; // not root
    }
    let home = Home::new();
    home.install_edition(&OLD);
    std::os::unix::fs::chown(&home.root, Some(12345), None).expect("chown");
    for command in ["install", "update", "remove"] {
        for args in [&[command][..], &[command, "--force"]] {
            assert_eq!(
                home.refused(args),
                "~ belongs to the user with id 12345, and atl runs as root: run it without \
                 sudo, or what it writes there would be root's",
                "{args:?}"
            );
        }
    }
    assert_eq!(home.status(&[])["state"], json!("other_owner"));
    // A project of that user's is root's to work on.
    home.atl()
        .args(["skill", "install", "--project"])
        .assert()
        .success();
}

/// The library's view of the skill of `home`, as the binary sees it.
fn environment(home: &Home) -> Environment {
    Environment {
        home: Some(home.root.as_os_str().to_owned()),
        ..Environment::default()
    }
}
