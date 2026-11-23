//! `atl skill status`: every state, in every format, writing nothing.

use pretty_assertions::assert_eq;

use super::*;

/// A change to a home directory.
type Change = fn(&Home);

/// `state`, `installed` and `version` of a status.
fn state(status: &Value) -> (&str, bool, &str) {
    (
        status["state"].as_str().expect("state"),
        status["installed"].as_bool().expect("installed"),
        status["version"].as_str().expect("version"),
    )
}

#[test]
fn every_state() {
    let cases: [(Change, (&str, bool, &str)); 15] = [
        (|_| {}, ("not_installed", false, "")),
        (
            |home| fs::create_dir_all(home.skill()).expect("mkdir"),
            ("not_installed", false, ""),
        ),
        (
            |home| {
                fs::create_dir_all(home.skill().join("references")).expect("mkdir");
                fs::write(home.skill().join(".DS_Store"), "x").expect("write");
                fs::write(home.skill().join("references/desktop.ini"), "x").expect("write");
            },
            ("not_installed", false, ""),
        ),
        (
            |home| drop(home.ok(&["install"])),
            ("up_to_date", true, VERSION),
        ),
        (
            |home| home.install_edition(&OLD),
            ("older", true, "2020.1.0"),
        ),
        (
            |home| {
                home.ok(&["install"]);
                home.change_manifest(|value| value["atl_version"] = json!("9999.1.0"));
            },
            ("newer", true, "9999.1.0"),
        ),
        // A component more is a later version; a suffix does not count.
        (
            |home| {
                home.ok(&["install"]);
                home.change_manifest(|value| value["atl_version"] = json!(format!("{VERSION}.1")));
            },
            ("newer", true, concat!(env!("CARGO_PKG_VERSION"), ".1")),
        ),
        (
            |home| {
                home.ok(&["install"]);
                home.change_manifest(|value| {
                    value["files"]
                        .as_object_mut()
                        .expect("object")
                        .remove("SKILL.md");
                });
            },
            ("differs", true, VERSION),
        ),
        (
            |home| {
                home.ok(&["install"]);
                home.change_manifest(|value| value["files"]["gone.md"] = json!("0".repeat(64)));
            },
            ("differs", true, VERSION),
        ),
        (
            |home| {
                fs::create_dir_all(home.skill()).expect("mkdir");
                fs::write(home.skill().join("SKILL.md"), "theirs").expect("write");
            },
            ("foreign", false, ""),
        ),
        (
            |home| {
                home.ok(&["install"]);
                home.write_manifest("{}");
            },
            ("damaged", true, ""),
        ),
        (
            |home| {
                home.ok(&["install"]);
                home.write_manifest(r#"{"schema": 2}"#);
            },
            ("unknown_schema", true, ""),
        ),
        (
            |home| {
                home.ok(&["install"]);
                fs::rename(home.skill(), home.root.join("checkout")).expect("mv");
                symlink(home.root.join("checkout"), home.skill()).expect("symlink");
            },
            ("blocked", false, ""),
        ),
        (
            |home| {
                fs::create_dir_all(home.root.join(".agents/skills")).expect("mkdir");
                symlink("/nowhere", home.skill()).expect("symlink");
            },
            ("blocked", false, ""),
        ),
        (
            |home| {
                fs::create_dir_all(home.root.join(".agents/skills")).expect("mkdir");
                fs::write(home.skill(), "theirs").expect("write");
            },
            ("blocked", false, ""),
        ),
    ];
    for (n, (change, expected)) in cases.into_iter().enumerate() {
        let home = Home::new();
        change(&home);
        let before = tree(&home.root);
        assert_eq!(state(&home.status(&[])), expected, "case {n}");
        assert_eq!(tree(&home.root), before, "case {n}: status wrote something");
    }
}

/// The files of an older skill, as the new version sees them: its own text
/// counts as unchanged, a file only it has does not exist for the user.
#[test]
fn an_older_skill_is_compared_with_what_wrote_it() {
    let home = Home::new();
    home.install_edition(&OLD);
    let lists = |home: &Home| {
        let status = home.status(&[]);
        [
            status["edited"].clone(),
            status["missing"].clone(),
            status["unlisted"].clone(),
        ]
    };
    assert_eq!(lists(&home), [json!([]), json!([]), json!([])]);
    let at = |path: &str| home.skill().join(path);
    fs::write(at("SKILL.md"), source("SKILL.md")).expect("write");
    assert_eq!(lists(&home), [json!([]), json!([]), json!([])]);
    fs::write(at("references/gone.md"), "mine").expect("write");
    fs::remove_file(at("references/jira-commands.md")).expect("rm");
    fs::write(at("references/body-formats.md"), "mine").expect("write");
    fs::write(
        at("references/confluence-commands.md"),
        source("references/confluence-commands.md"),
    )
    .expect("write");
    assert_eq!(
        lists(&home),
        [
            json!(["references/gone.md"]),
            json!(["references/jira-commands.md"]),
            json!(["references/body-formats.md"]),
        ]
    );
}

/// What stops an update is in one list or another: what `--force` replaces
/// in `edited` and `colliding`, what it does not in `in_the_way`.
#[test]
fn what_stops_an_update_is_said() {
    let lists = |home: &Home| {
        let status = home.status(&[]);
        ["edited", "colliding", "in_the_way"].map(|key| status[key].clone())
    };
    let cases: [(Change, [Value; 3]); 6] = [
        (
            |home| fs::create_dir(home.skill().join("references/body-formats.md")).expect("mkdir"),
            [json!([]), json!([]), json!(["references/body-formats.md"])],
        ),
        (
            |home| {
                fs::write(home.skill().join("references/body-formats.md"), "mine").expect("write");
            },
            [json!([]), json!(["references/body-formats.md"]), json!([])],
        ),
        (
            |home| {
                let at = home.skill().join("references/body-formats.md");
                symlink("/nowhere", at).expect("symlink");
            },
            [json!([]), json!(["references/body-formats.md"]), json!([])],
        ),
        (
            |home| fs::write(home.skill().join("SKILL.md"), "mine").expect("write"),
            [json!(["SKILL.md"]), json!([]), json!([])],
        ),
        (
            |home| {
                let references = home.skill().join("references");
                fs::rename(&references, home.root.join("aside")).expect("mv");
                fs::write(&references, "mine").expect("write");
            },
            [json!([]), json!([]), json!(["references"])],
        ),
        (
            |home| {
                fs::remove_file(home.skill().join("SKILL.md")).expect("rm");
                fs::create_dir(home.skill().join("SKILL.md")).expect("mkdir");
            },
            [json!([]), json!([]), json!(["SKILL.md"])],
        ),
    ];
    for (n, (change, expected)) in cases.into_iter().enumerate() {
        let home = Home::new();
        home.install_edition(&OLD);
        change(&home);
        assert_eq!(lists(&home), expected, "case {n}");
        assert_eq!(home.status(&[])["state"], json!("older"));
        let said = home.refused(&["update"]);
        let forced = home.run(&["update", "--force"]).0;
        assert_eq!(forced == 5, expected[2] != json!([]), "case {n}: {said}");
        let console = home.ok(&["status"]);
        for key in ["colliding", "in_the_way"] {
            assert!(console.contains(&format!("\n{key}: ")), "{console}");
        }
    }
    let home = Home::new();
    home.install_edition(&OLD);
    fs::remove_file(home.skill().join(MANIFEST)).expect("rm");
    fs::create_dir(home.skill().join(MANIFEST)).expect("mkdir");
    assert_eq!(home.status(&[])["in_the_way"], json!([MANIFEST]));
}

/// The skill directory with the link, and what happens to be in its place.
#[test]
fn every_state_of_the_link() {
    let cases: [(Change, &str); 12] = [
        (|home| drop(home.ok(&["install"])), "ok"),
        // Another spelling of the same place, which atl did not write.
        (
            |home| {
                fs::create_dir_all(home.skill()).expect("mkdir");
                fs::create_dir(home.root.join(".claude/skills")).expect("mkdir");
                symlink(home.skill(), home.link()).expect("symlink");
            },
            "ok",
        ),
        // Claude Code reads the shared directory itself.
        (
            |home| {
                fs::create_dir_all(home.root.join(".agents/skills")).expect("mkdir");
                symlink("../.agents/skills", home.root.join(".claude/skills")).expect("symlink");
            },
            "ok",
        ),
        (|_| {}, "missing"),
        (
            |home| fs::create_dir(home.root.join(".claude/skills")).expect("mkdir"),
            "missing",
        ),
        (
            |home| {
                fs::create_dir(home.root.join(".claude/skills")).expect("mkdir");
                symlink("/nowhere", home.link()).expect("symlink");
            },
            "elsewhere",
        ),
        (
            |home| {
                fs::create_dir_all(home.root.join("theirs")).expect("mkdir");
                fs::create_dir(home.root.join(".claude/skills")).expect("mkdir");
                symlink("../../theirs", home.link()).expect("symlink");
            },
            "elsewhere",
        ),
        (
            |home| fs::create_dir_all(home.link()).expect("mkdir"),
            "taken",
        ),
        (
            |home| {
                fs::create_dir(home.root.join(".claude/skills")).expect("mkdir");
                fs::write(home.link(), "theirs").expect("write");
            },
            "taken",
        ),
        (
            |home| fs::write(home.root.join(".claude/skills"), "theirs").expect("write"),
            "taken",
        ),
        (
            |home| fs::remove_dir(home.root.join(".claude")).expect("rmdir"),
            "not_wanted",
        ),
        (
            |home| {
                fs::remove_dir(home.root.join(".claude")).expect("rmdir");
                fs::write(home.root.join(".claude"), "theirs").expect("write");
            },
            "not_wanted",
        ),
    ];
    for (n, (change, expected)) in cases.into_iter().enumerate() {
        let home = Home::with_claude();
        change(&home);
        let before = tree(&home.root);
        let status = home.status(&[]);
        assert_eq!(status["link_state"], json!(expected), "case {n}");
        let place = if expected == "not_wanted" {
            ""
        } else {
            "~/.claude/skills/atl"
        };
        assert_eq!(status["link"], json!(place), "case {n}");
        assert_eq!(tree(&home.root), before, "case {n}: status wrote something");
        // `remove` takes the link atl writes, and no other.
        if n > 0 {
            home.run(&["remove"]);
            assert_eq!(tree(&home.root), before, "case {n}");
        }
    }
}

/// Every key, in its order, never null, in every state; and every format
/// that can be read back holds the same object.
#[test]
fn every_key_is_there_in_every_state() {
    let keys = [
        "path",
        "installed",
        "state",
        "version",
        "atl_version",
        "edited",
        "missing",
        "colliding",
        "in_the_way",
        "unlisted",
        "link",
        "link_state",
    ];
    let homes: [Change; 3] = [
        |_| {},
        |home| drop(home.ok(&["install"])),
        |home| home.write_manifest("{}"),
    ];
    let home = Home::with_claude();
    for change in homes {
        change(&home);
        // JSON, so that it is the order of the keys too.
        let text = home.ok(&["status", "-F", "json"]);
        let at: Vec<usize> = keys
            .iter()
            .map(|key| text.find(&format!("\n  \"{key}\": ")).expect(key))
            .collect();
        assert!(at.is_sorted(), "{text}");
        let status: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(status.as_object().expect("object").len(), keys.len());
        assert!(!text.contains("null"), "{text}");

        let toml: Value = toml::from_str(&home.ok(&["status", "-F", "toml"])).expect("TOML");
        assert_eq!(toml, status, "toml");

        let csv = home.ok(&["status", "-F", "csv"]);
        let mut rows = csv::Reader::from_reader(csv.as_bytes());
        let header: Vec<String> = rows
            .headers()
            .expect("header")
            .iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(header, keys);
        let records: Vec<csv::StringRecord> = rows.records().map(|row| row.expect("row")).collect();
        assert_eq!(records.len(), 1, "{csv}");
        for (key, cell) in keys.iter().zip(records[0].iter()) {
            let expected = match &status[key] {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            assert_eq!(cell, expected, "csv {key}");
        }

        for format in ["console", "toon"] {
            let text = home.ok(&["status", "-F", format]);
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), keys.len(), "{format}: {text}");
            for (line, key) in lines.iter().zip(keys) {
                assert!(line.starts_with(&format!("{key}: ")), "{format}: {line}");
            }
        }
    }
}

/// `-q` quiets the log, not the report.
#[test]
fn quiet_changes_nothing_of_a_status() {
    let home = Home::with_claude();
    home.ok(&["install"]);
    for format in ["console", "json", "toml", "toon", "csv"] {
        assert_eq!(
            home.ok(&["status", "-F", format, "-q"]),
            home.ok(&["status", "-F", format])
        );
    }
}

#[test]
fn the_status_of_a_project_is_that_of_the_current_directory() {
    let home = Home::with_claude();
    home.ok(&["install"]);
    let project = home.root.join("project");
    fs::create_dir_all(project.join(".claude")).expect("mkdir");
    let status = |args: &[&str]| -> Value {
        let mut atl = home.atl();
        atl.current_dir(&project)
            .args(["-F", "json", "skill", "status"])
            .args(args);
        let (code, stdout, stderr) = home.finish(&mut atl);
        assert_eq!((code, stderr.as_str()), (0, ""));
        serde_json::from_str(&stdout).expect("JSON")
    };
    let of_project = status(&["--project"]);
    assert_eq!(state(&of_project), ("not_installed", false, ""));
    assert_eq!(of_project["path"], json!("~/project/.agents/skills/atl"));
    assert_eq!(of_project["link"], json!("~/project/.claude/skills/atl"));
    assert_eq!(of_project["link_state"], json!("missing"));
    let global = status(&[]);
    assert_eq!(state(&global), ("up_to_date", true, VERSION));
    assert_eq!(global["path"], json!("~/.agents/skills/atl"));
}

/// What is there is seen by someone who cannot write to it, as `status`
/// writes nothing and takes no lock.
#[test]
fn a_status_needs_no_right_to_write() {
    let home = Home::with_claude();
    home.ok(&["install"]);
    let skill = home.skill();
    chmod(&skill.join("references"), 0o500);
    chmod(&skill, 0o500);
    chmod(&home.root, 0o500);
    let status = home.status(&[]);
    chmod(&home.root, 0o700);
    chmod(&skill, 0o700);
    chmod(&skill.join("references"), 0o700);
    assert_eq!(state(&status), ("up_to_date", true, VERSION));
}
