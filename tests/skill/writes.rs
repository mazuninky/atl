//! How `atl skill` writes: what fails midway, what a run that was cut short
//! leaves, and what the next one makes of it.

use pretty_assertions::assert_eq;

use super::*;

/// The state `status` reports, and that no file of atl's differs or is gone.
fn whole(home: &Home) -> String {
    let status = home.status(&[]);
    assert_eq!(status["edited"], json!([]), "{status:#}");
    assert_eq!(status["missing"], json!([]), "{status:#}");
    status["state"].as_str().expect("state").to_owned()
}

/// The files in the skill directory, without what they hold.
fn names(home: &Home) -> Vec<String> {
    tree(&home.skill())
        .into_iter()
        .map(|(path, _)| path)
        .collect()
}

/// `references/` cannot be written to: `SKILL.md` is new, the rest is the
/// old version's, and so is the manifest. The next run goes on from there,
/// without `--force`.
#[test]
fn an_update_that_fails_midway_is_finished_by_the_next() {
    if !modes_bind() {
        return;
    }
    for command in ["update", "install"] {
        let home = Home::new();
        home.install_edition(&OLD);
        let references = home.skill().join("references");
        chmod(&references, 0o500);
        let said = home.failed(&[command]);
        chmod(&references, 0o700);
        assert!(said.contains("~/.agents/skills/atl/references/"), "{said}");
        assert_eq!(
            fs::read_to_string(home.skill().join("SKILL.md")).expect("read"),
            source("SKILL.md")
        );
        let status = home.status(&[]);
        assert_eq!(
            (&status["state"], &status["edited"], &status["missing"]),
            (&json!("older"), &json!([]), &json!(&FILES[2..4])),
            "what was yet to be written is missing"
        );
        // Failing once more changes nothing about that.
        chmod(&references, 0o500);
        home.failed(&[command]);
        chmod(&references, 0o700);
        assert_eq!(
            home.json(&[command])["action"],
            json!("updated"),
            "{command}"
        );
        assert_eq!(whole(&home), "up_to_date");
        assert_eq!(names(&home), FILES, "nothing left over");
    }
}

/// The skill directory cannot be written to: nothing in it changes.
#[test]
fn a_skill_directory_that_cannot_be_written_to_is_a_write_failure() {
    if !modes_bind() {
        return;
    }
    let home = Home::new();
    home.install_edition(&OLD);
    let before = tree(&home.skill());
    chmod(&home.skill(), 0o500);
    for args in [&["update"][..], &["update", "--force"], &["install"]] {
        home.failed(args);
    }
    chmod(&home.skill(), 0o700);
    assert_eq!(tree(&home.skill()), before);
    home.ok(&["update"]);
    assert_eq!(whole(&home), "up_to_date");
}

/// `remove` that fails midway says so, and the next one finishes.
#[test]
fn a_remove_that_fails_midway_is_finished_by_the_next() {
    if !modes_bind() {
        return;
    }
    let home = Home::with_claude();
    home.ok(&["install"]);
    let references = home.skill().join("references");
    chmod(&references, 0o500);
    home.failed(&["remove"]);
    chmod(&references, 0o700);
    assert!(home.skill().join(MANIFEST).is_file(), "the last to go");
    home.ok(&["remove"]);
    assert_eq!(tree(&home.root), []);
    assert!(!home.skill().exists());
}

/// The link cannot be made: the skill is there, and the next run makes it.
#[test]
fn a_link_that_cannot_be_made_is_a_write_failure() {
    if !modes_bind() {
        return;
    }
    let home = Home::with_claude();
    chmod(&home.root.join(".claude"), 0o500);
    let said = home.failed(&["install"]);
    chmod(&home.root.join(".claude"), 0o700);
    assert!(said.contains("~/.claude/skills"), "{said}");
    assert_eq!(home.status(&[])["state"], json!("up_to_date"));
    assert_eq!(
        home.json(&["install"]),
        linked(wrote("up_to_date", VERSION, VERSION), "created")
    );
}

/// A first install that was cut short leaves files with no manifest, as this
/// version has them, perhaps with a temporary file: not a foreign directory.
#[test]
fn an_install_that_was_cut_short_is_finished_by_the_next() {
    let home = Home::new();
    for left in [&FILES[1..2], &FILES[1..3], &FILES[1..]] {
        for path in left {
            let at = home.skill().join(path);
            fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
            fs::write(at, source(path)).expect("write");
        }
        fs::write(home.skill().join(".tmpAb12Cd"), "half").expect("write");
        assert_eq!(home.status(&[])["state"], json!("not_installed"));
        assert_eq!(
            home.json(&["install"]),
            wrote("installed", VERSION, ""),
            "{left:?}"
        );
        assert_eq!(whole(&home), "up_to_date");
        home.ok(&["remove"]);
    }
    // One of another version in their midst is someone else's.
    fs::create_dir_all(home.skill()).expect("mkdir");
    fs::write(home.skill().join("SKILL.md"), "another").expect("write");
    let said = home.refused(&["install"]);
    assert!(
        said.contains("was not installed by atl (SKILL.md)"),
        "{said}"
    );
}

/// A first install that fails midway says what is not there, by any next
/// command.
#[test]
fn an_install_that_fails_midway_says_what_is_missing() {
    if !modes_bind() {
        return;
    }
    let home = Home::new();
    let references = home.skill().join("references");
    fs::create_dir_all(&references).expect("mkdir");
    chmod(&references, 0o500);
    home.failed(&["install"]);
    let status = home.status(&[]);
    assert_eq!(
        (&status["state"], &status["installed"], &status["missing"]),
        (&json!("differs"), &json!(true), &json!(&FILES[2..]),)
    );
    for list in ["edited", "colliding", "in_the_way", "unlisted"] {
        assert_eq!(status[list], json!([]), "{list}");
    }
    chmod(&references, 0o700);
    home.ok(&["update"]);
    assert_eq!(whole(&home), "up_to_date");
    assert_eq!(names(&home), FILES);
}

/// A forced install over the user's own files that fails midway: those it did
/// not come to are still theirs, and `remove` leaves them, `--force` or not.
#[test]
fn a_forced_install_that_fails_midway_does_not_make_files_atls() {
    if !modes_bind() {
        return;
    }
    for force in [&["remove"][..], &["remove", "--force"]] {
        let home = Home::new();
        let references = home.skill().join("references");
        fs::create_dir_all(&references).expect("mkdir");
        fs::write(references.join("jira-commands.md"), "mine").expect("write");
        chmod(&references, 0o500);
        home.failed(&["install", "--force"]);
        chmod(&references, 0o700);
        let status = home.status(&[]);
        assert_eq!(
            (&status["edited"], &status["colliding"], &status["unlisted"]),
            (
                &json!([]),
                &json!(["references/jira-commands.md"]),
                &json!(["references/jira-commands.md"])
            )
        );
        assert_eq!(
            home.refused(&["install"]),
            "~/.agents/skills/atl was changed since atl wrote it: \
             not atl's: references/jira-commands.md (--force writes atl's files in their place)"
        );
        assert_eq!(
            home.json(force),
            kept(removed(VERSION), &["references/jira-commands.md"], &[])
        );
        assert_eq!(
            tree(&home.skill()),
            [("references/jira-commands.md".to_owned(), "mine".to_owned())]
        );
    }
}

/// An update that was cut short anywhere is finished by the next, `--force`
/// or not: the files written so far are this version's, the manifest is not.
#[test]
fn an_update_that_was_cut_short_is_finished_by_the_next() {
    for written in 0..=4 {
        let home = Home::new();
        home.install_edition(&OLD);
        for path in &FILES[1..=written] {
            fs::write(home.skill().join(path), source(path)).expect("write");
        }
        if written == 4 {
            fs::remove_file(home.skill().join("references/gone.md")).expect("rm");
        }
        let status = home.status(&[]);
        assert_eq!(
            (&status["state"], &status["edited"], &status["unlisted"]),
            (&json!("older"), &json!([]), &json!([])),
            "{written}"
        );
        home.ok(&["update"]);
        assert_eq!(whole(&home), "up_to_date", "{written}");
        assert_eq!(names(&home), FILES);
    }
}

/// A file in the place of a directory, a directory in the place of a file:
/// refused before anything is written, `--force` or not.
#[test]
fn what_is_in_the_way_of_the_files_is_refused_before_anything_is_written() {
    type Make = fn(&Path) -> std::io::Result<()>;
    let cases: [(&str, Make); 4] = [
        ("SKILL.md", |at| fs::create_dir(at)),
        ("references/jira-commands.md", |at| {
            fs::create_dir(at).and_then(|()| fs::write(at.join("x"), "theirs"))
        }),
        ("references", |at| fs::write(at, "theirs")),
        ("references", |at| symlink("/nowhere", at)),
    ];
    for (path, make) in cases {
        for installed in [false, true] {
            let home = Home::new();
            let at = home.skill().join(path);
            if installed {
                home.install_edition(&OLD);
                if at.is_dir() {
                    fs::rename(&at, home.root.join("aside")).expect("mv");
                } else {
                    fs::remove_file(&at).expect("rm");
                }
            } else {
                fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
            }
            make(&at).expect("make");
            for args in [&["install", "--force"][..], &["update", "--force"]] {
                // Nothing atl could have installed is there: nothing to update.
                if path == "SKILL.md" && !installed && args[0] == "update" {
                    continue;
                }
                assert_eq!(
                    home.refused(args),
                    format!(
                        "~/.agents/skills/atl cannot hold the files of the skill ({path}): \
                         atl neither goes through a link nor replaces a directory"
                    ),
                    "installed: {installed}"
                );
            }
        }
    }
}

/// A link or a file of the user's inside the skill directory, where atl has
/// nothing to write, stays what it is through everything.
#[test]
fn links_where_atl_writes_nothing_stay() {
    let home = Home::new();
    home.install_edition(&OLD);
    let outside = home.root.join("outside");
    fs::create_dir(&outside).expect("mkdir");
    fs::write(outside.join("file.md"), "theirs").expect("write");
    symlink(&outside, home.skill().join("mine")).expect("symlink");
    symlink("/nowhere", home.skill().join("references/mine.md")).expect("symlink");
    home.ok(&["update"]);
    assert_eq!(whole(&home), "up_to_date");
    assert_eq!(
        home.status(&[])["unlisted"],
        json!(["mine", "references/mine.md"]),
        "the link, not what it leads to"
    );
    assert_eq!(
        home.json(&["remove"]),
        kept(removed(VERSION), &["mine", "references/mine.md"], &[])
    );
    assert!(home.skill().join("mine").is_symlink());
    assert_eq!(
        fs::read_to_string(outside.join("file.md")).expect("read"),
        "theirs"
    );
}

/// The link of an older version's file that this one has not: `--force`
/// keeps it, as any edited file of that kind, and nothing goes through it.
#[test]
fn a_link_in_the_place_of_a_file_this_version_has_not_is_kept() {
    let home = Home::new();
    home.install_edition(&OLD);
    let outside = home.root.join("outside.md");
    fs::write(&outside, "old gone\n").expect("write"); // byte for byte atl's
    let gone = home.skill().join("references/gone.md");
    fs::remove_file(&gone).expect("rm");
    symlink(&outside, &gone).expect("symlink");
    let said = home.refused(&["update"]);
    assert!(said.contains("edited references/gone.md"), "{said}");
    assert_eq!(
        home.json(&["update", "--force"])["kept"],
        json!(["references/gone.md"])
    );
    assert!(gone.is_symlink());
    assert_eq!(fs::read_to_string(&outside).expect("read"), "old gone\n");
}

/// What a system puts into a directory it shows is nobody's file, and atl
/// leaves it.
#[test]
fn a_finder_file_is_in_no_ones_way_and_stays() {
    let home = Home::new();
    fs::create_dir_all(home.skill()).expect("mkdir");
    fs::write(home.skill().join(".DS_Store"), "finder").expect("write");
    home.ok(&["install"]); // not a foreign directory
    fs::write(home.skill().join("references/desktop.ini"), "finder").expect("write");
    assert_eq!(home.status(&[])["unlisted"], json!([]));
    home.ok(&["update"]);
    assert_eq!(
        home.json(&["remove"]),
        kept(
            removed(VERSION),
            &[],
            &[".DS_Store", "references/desktop.ini"]
        )
    );
    assert_eq!(names(&home), [".DS_Store", "references/desktop.ini"]);
    assert_eq!(home.status(&[])["state"], json!("not_installed"));
    home.ok(&["install"]);
    assert_eq!(whole(&home), "up_to_date");
}

/// What nobody counts as a file of theirs is named, up to three of it.
#[test]
fn what_keeps_the_directory_from_being_empty_is_named() {
    let home = Home::new();
    home.ok(&["install"]);
    for name in [".tmpAb12Cd", "references/.tmpZz9zZz", ".DS_Store"] {
        fs::write(home.skill().join(name), "left").expect("write");
    }
    assert_eq!(
        home.json(&["remove"]),
        kept(
            removed(VERSION),
            &[],
            &[".DS_Store", ".tmpAb12Cd", "references/.tmpZz9zZz"]
        )
    );
    home.ok(&["install"]);
    fs::create_dir(home.skill().join("empty")).expect("mkdir");
    assert_eq!(
        home.json(&["remove"])["other"],
        json!([".DS_Store", ".tmpAb12Cd", "empty", "references/.tmpZz9zZz"])
    );
}

/// A directory of the user's that cannot be read is theirs, whatever is in
/// it, and in no one's way.
#[test]
fn a_directory_that_cannot_be_read_is_not_atls() {
    if !modes_bind() {
        return;
    }
    let home = Home::new();
    home.install_edition(&OLD);
    let mine = home.skill().join("references/mine");
    fs::create_dir(&mine).expect("mkdir");
    fs::write(mine.join("mine.md"), "mine").expect("write");
    chmod(&mine, 0o000);
    let unlisted = |home: &Home| home.status(&[])["unlisted"].clone();
    assert_eq!(unlisted(&home), json!(["references/mine"]));
    for command in ["update", "install"] {
        home.ok(&[command]);
        assert_eq!(whole(&home), "up_to_date");
        assert_eq!(unlisted(&home), json!(["references/mine"]));
    }
    let said = home.json(&["remove"]);
    let refused = home.run(&["install"]);
    chmod(&mine, 0o700);
    assert_eq!(
        refused,
        (
            5,
            String::new(),
            "Error: invalid input: ~/.agents/skills/atl was not installed by atl \
             (references/mine): --force writes atl's files into it and keeps the rest\n"
                .to_owned()
        )
    );
    assert_eq!(said, kept(removed(VERSION), &["references/mine"], &[]));
    assert_eq!(
        tree(&home.skill()),
        [("references/mine/mine.md".to_owned(), "mine".to_owned())]
    );
}

/// A directory whose name differs in case is the user's as soon as a file of
/// theirs is in it: it keeps its name, and the files in it are atl's there.
#[test]
fn a_directory_with_a_file_of_the_users_keeps_its_name() {
    let home = Home::new();
    home.install_edition(&cased());
    fs::write(home.skill().join("References/mine.md"), "mine").expect("write");
    home.ok(&["update"]);
    assert_eq!(whole(&home), "up_to_date");
    let expected: &[&str] = if ignores_case(&home) {
        &[
            MANIFEST,
            "References",
            "References/body-formats.md",
            "References/confluence-commands.md",
            "References/jira-commands.md",
            "References/mine.md",
            "SKILL.md",
        ]
    } else {
        &[
            MANIFEST,
            "References",
            "References/mine.md",
            "SKILL.md",
            "references",
            "references/body-formats.md",
            "references/confluence-commands.md",
            "references/jira-commands.md",
        ]
    };
    assert_eq!(entries(&home), expected);
    assert_eq!(home.status(&[])["unlisted"], json!(["References/mine.md"]));
    // Nothing more to do, and no more than atl's goes.
    assert_eq!(
        home.json(&["update"]),
        wrote("up_to_date", VERSION, VERSION)
    );
    home.ok(&["remove"]);
    assert_eq!(entries(&home), ["References", "References/mine.md"]);
}

/// A new file has the mode of any new file, and one that is replaced keeps
/// its own.
#[test]
fn modes_are_those_of_new_files_and_stay() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let atl = assert_cmd::cargo::cargo_bin("atl");
    let run = |umask: &str, command: &str| {
        let mut sh = Command::new("/bin/sh");
        sh.env_clear().env("HOME", &home.root).env("ATL", &atl);
        let script = format!("umask {umask} && exec \"$ATL\" skill {command}");
        sh.args(["-c", &script]).assert().success();
    };
    let mode = |path: &str| {
        let meta = fs::metadata(home.skill().join(path)).expect("stat");
        meta.permissions().mode() & 0o7777
    };
    run("027", "install");
    assert_eq!(FILES.map(mode), [0o640; 5]);
    assert_eq!((mode(""), mode("references")), (0o750, 0o750));

    home.install_edition(&OLD);
    chmod(&home.skill().join("SKILL.md"), 0o755);
    chmod(&home.skill().join(MANIFEST), 0o600);
    chmod(&home.skill().join("references/jira-commands.md"), 0o444);
    for new in [
        "references/body-formats.md",
        "references/confluence-commands.md",
    ] {
        fs::remove_file(home.skill().join(new)).expect("rm");
    }
    run("002", "update");
    assert_eq!(whole(&home), "up_to_date");
    assert_eq!(FILES.map(mode), [0o600, 0o755, 0o664, 0o664, 0o444]);
}

/// The mode of a file does not stand in the way of replacing or removing
/// it: it is atl's, and the directory can be written to.
#[test]
fn read_only_files_of_atls_are_replaced_and_removed() {
    let home = Home::new();
    let lock = |home: &Home| {
        for path in names(home) {
            chmod(&home.skill().join(path), 0o444);
        }
    };
    home.install_edition(&OLD);
    lock(&home);
    home.ok(&["update"]);
    assert_eq!(whole(&home), "up_to_date");
    assert_eq!(names(&home), FILES);
    lock(&home);
    fs::remove_file(home.skill().join("SKILL.md")).expect("rm");
    home.ok(&["update"]);
    lock(&home);
    home.ok(&["remove"]);
    assert_eq!(tree(&home.root), []);
}

/// An older skill with files whose names this version spells in another case:
/// one with other text, one in a directory, and one with the text it has now.
fn cased() -> Edition {
    static FILES: std::sync::OnceLock<Vec<SkillFile>> = std::sync::OnceLock::new();
    let files = FILES.get_or_init(|| {
        let file = |path, text| SkillFile { path, text };
        let confluence = Edition::embedded()
            .text("references/confluence-commands.md")
            .expect("confluence-commands.md");
        vec![
            file("Skill.md", "old skill\n"),
            file("References/BODY-FORMATS.md", "old formats\n"),
            file("References/CONFLUENCE-COMMANDS.md", confluence),
            file("References/gone.md", "old gone\n"),
        ]
    });
    Edition {
        version: "2020.1.0",
        files,
    }
}

/// Whether the file system finds a file by its name in another case.
fn ignores_case(home: &Home) -> bool {
    fs::write(home.root.join("probe"), "").expect("write");
    home.root.join("PROBE").exists()
}

/// Every file and directory in the skill directory.
fn entries(home: &Home) -> Vec<String> {
    let mut found = Vec::new();
    let mut todo = vec![(home.skill(), String::new())];
    while let Some((at, prefix)) = todo.pop() {
        for entry in fs::read_dir(&at).expect("read dir") {
            let entry = entry.expect("entry");
            let path = format!("{prefix}{}", entry.file_name().to_str().expect("UTF-8"));
            if entry.file_type().expect("type").is_dir() {
                todo.push((entry.path(), format!("{path}/")));
            }
            found.push(path);
        }
    }
    found.sort();
    found
}

/// The skill is whole, of this version, with the names it has now and no
/// other file or directory.
fn assert_respelled(home: &Home) {
    assert_eq!(whole(home), "up_to_date");
    assert_eq!(home.status(&[])["unlisted"], json!([]));
    assert_eq!(
        entries(home),
        [
            MANIFEST,
            "SKILL.md",
            "references",
            "references/body-formats.md",
            "references/confluence-commands.md",
            "references/jira-commands.md",
        ]
    );
}

/// A name that changed only in case is one file under its new name, on a
/// file system that ignores case and on one that does not.
#[test]
fn names_that_differ_in_case_only_are_the_new_ones_after_an_update() {
    for args in [&["update"][..], &["install"], &["update", "--force"]] {
        let home = Home::new();
        home.install_edition(&cased());
        assert_eq!(whole(&home), "older");
        assert_eq!(
            home.json(args),
            wrote("updated", VERSION, "2020.1.0"),
            "{args:?}"
        );
        assert_respelled(&home);
        home.ok(&["remove"]);
        assert_eq!(tree(&home.root), []);
        assert!(!home.skill().exists());
    }
}

/// What a run leaves that was cut short: names that are the new ones
/// already, over text that is not, and some that is.
#[test]
fn a_change_of_case_that_was_cut_short_is_finished_by_the_next() {
    for written in [false, true] {
        let home = Home::new();
        home.install_edition(&cased());
        if ignores_case(&home) {
            for (old, new) in [
                ("Skill.md", "SKILL.md"),
                ("References", "references"),
                ("references/BODY-FORMATS.md", "references/body-formats.md"),
            ] {
                fs::rename(home.skill().join(old), home.skill().join(new)).expect("mv");
            }
        }
        if written {
            for path in ["SKILL.md", "references/body-formats.md"] {
                let at = home.skill().join(path);
                fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
                fs::write(at, source(path)).expect("write");
            }
        }
        assert_eq!(whole(&home), "older", "{written}");
        assert_eq!(home.status(&[])["unlisted"], json!([]), "{written}");
        home.ok(&["update"]);
        assert_respelled(&home);
    }
}

/// Edited under the old name: refused, named once, and with `--force`
/// replaced where it is the file of the new name, kept where it is not.
#[test]
fn an_edited_file_whose_name_differs_in_case_is_not_lost() {
    let home = Home::new();
    home.install_edition(&cased());
    let edited = home.skill().join("References/BODY-FORMATS.md");
    fs::write(&edited, "mine").expect("write");
    for command in ["update", "install"] {
        assert_eq!(
            home.refused(&[command]),
            "~/.agents/skills/atl was changed since atl wrote it: edited \
             References/BODY-FORMATS.md (--force writes atl's files in their place)"
        );
    }
    let said = home.json(&["update", "--force"]);
    if ignores_case(&home) {
        assert_eq!(said, wrote("updated", VERSION, "2020.1.0"));
        assert_respelled(&home);
    } else {
        let mut expected = wrote("updated", VERSION, "2020.1.0");
        expected["kept"] = json!(["References/BODY-FORMATS.md"]);
        assert_eq!(said, expected);
        assert_eq!(whole(&home), "up_to_date");
        assert_eq!(fs::read_to_string(&edited).expect("read"), "mine");
    }
}

/// A file of the user's under a name that differs in case is in the way
/// where it is the file, and in no one's way where it is not.
#[test]
fn a_file_of_the_users_whose_name_differs_in_case_is_replaced_only_by_force() {
    let home = Home::new();
    let ignores_case = ignores_case(&home);
    fs::create_dir_all(home.skill()).expect("mkdir");
    let theirs = home.skill().join("skill.md");
    fs::write(&theirs, "theirs").expect("write");
    let said = home.refused(&["install"]);
    assert!(said.contains("not installed by atl (skill.md)"), "{said}");
    home.ok(&["install", "--force"]);
    if ignores_case {
        assert_respelled(&home);
    } else {
        assert_eq!(whole(&home), "up_to_date");
        home.ok(&["remove"]);
        assert_eq!(
            tree(&home.skill()),
            [("skill.md".to_owned(), "theirs".to_owned())]
        );
    }

    // In the place of a file only the new version has.
    let home = Home::new();
    home.install_edition(&OLD);
    fs::write(home.skill().join("references/BODY-FORMATS.md"), "theirs").expect("write");
    if ignores_case {
        assert_eq!(
            home.refused(&["update"]),
            "~/.agents/skills/atl was changed since atl wrote it: \
             not atl's: references/body-formats.md (--force writes atl's files in their place)"
        );
        home.ok(&["update", "--force"]);
        assert_respelled(&home);
    } else {
        home.ok(&["update"]);
        assert_eq!(
            home.status(&[])["unlisted"],
            json!(["references/BODY-FORMATS.md"])
        );
    }
}

/// The user renamed a file of atl's: where that is the same file, it gets its
/// name back and is no file of the user's; where it is not, one is missing
/// and the other the user's.
#[test]
fn a_file_renamed_in_case_only_gets_its_name_back() {
    let home = Home::new();
    home.ok(&["install"]);
    fs::rename(home.skill().join("SKILL.md"), home.skill().join("skill.md")).expect("mv");
    let status = home.status(&[]);
    let expected = if ignores_case(&home) {
        (json!([]), json!([]))
    } else {
        (json!(["SKILL.md"]), json!(["skill.md"]))
    };
    assert_eq!(
        (&status["missing"], &status["unlisted"]),
        (&expected.0, &expected.1)
    );
    assert_eq!(home.json(&["update"]), wrote("restored", VERSION, VERSION));
    if ignores_case(&home) {
        assert_respelled(&home);
    } else {
        assert_eq!(whole(&home), "up_to_date");
        assert_eq!(home.status(&[])["unlisted"], json!(["skill.md"]));
    }
    let before = tree(&home.skill());
    assert_eq!(
        home.json(&["update"]),
        wrote("up_to_date", VERSION, VERSION)
    );
    assert_eq!(tree(&home.skill()), before);
}

/// Two names of one file (a hard link) are two entries, so one of them is
/// not atl's, whoever the file itself belongs to.
#[test]
fn a_hard_link_to_a_file_of_atls_is_not_atls() {
    let home = Home::new();
    home.install_edition(&OLD);
    let gone = home.skill().join("references/gone.md");
    let formats = home.skill().join("references/body-formats.md");
    fs::hard_link(&gone, &formats).expect("ln");
    let backup = home.skill().join("references/backup.md");
    fs::hard_link(&gone, &backup).expect("ln");
    assert_eq!(
        home.refused(&["update"]),
        "~/.agents/skills/atl was changed since atl wrote it: \
         not atl's: references/body-formats.md (--force writes atl's files in their place)"
    );
    home.ok(&["update", "--force"]);
    assert_eq!(whole(&home), "up_to_date");
    assert!(!gone.exists(), "atl's, and this version has it not");
    assert_eq!(fs::read_to_string(&backup).expect("read"), "old gone\n");
    assert_eq!(
        home.status(&[])["unlisted"],
        json!(["references/backup.md"])
    );
}

/// A file that is atl's already, byte for byte, is not written again.
#[test]
fn a_file_that_is_up_to_date_is_not_touched() {
    use std::os::unix::fs::MetadataExt;
    let home = Home::new();
    home.ok(&["install"]);
    let inodes = || FILES.map(|path| fs::metadata(home.skill().join(path)).expect("stat").ino());
    let before = inodes();
    home.ok(&["install"]);
    home.ok(&["update", "--force"]);
    assert_eq!(inodes(), before);
    fs::write(home.skill().join("SKILL.md"), "mine").expect("write");
    home.ok(&["update", "--force"]);
    let after = inodes();
    assert_eq!(
        &after[2..],
        &before[2..],
        "but for the manifest and the file"
    );
}
