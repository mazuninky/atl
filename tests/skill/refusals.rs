//! What `atl skill` refuses, and what it says: exit 5, one line, nothing
//! written.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use pretty_assertions::assert_eq;

use super::*;

/// The four commands, with the forms that write.
const EVERY: [&[&str]; 7] = [
    &["install"],
    &["install", "--force"],
    &["update"],
    &["update", "--force"],
    &["remove"],
    &["remove", "--force"],
    &["status"],
];

/// A digest that is none of a file of these tests.
const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// A manifest of this version that lists `files`.
fn manifest(files: Value) -> String {
    json!({"schema": 1, "atl_version": VERSION, "files": files}).to_string()
}

/// `atl skill ARGS` with `name` set to `value`, which must be refused.
fn refused_with_env(home: &Home, name: &str, value: &OsStr, args: &[&str]) -> String {
    let (code, stdout, message) =
        home.refused_with(home.atl().env(name, value).arg("skill").args(args));
    assert_eq!((code, stdout.as_str()), (5, ""), "{name}: {args:?}");
    message
}

#[test]
fn a_claude_config_dir_that_is_no_absolute_path_is_refused() {
    let home = Home::with_claude();
    for dir in ["claude", "./claude", "../claude", "~/.claude"] {
        for args in EVERY {
            assert_eq!(
                refused_with_env(&home, "CLAUDE_CONFIG_DIR", OsStr::new(dir), args),
                format!("CLAUDE_CONFIG_DIR is '{dir}', which is no absolute path"),
                "{args:?}"
            );
        }
        assert_eq!(tree(&home.root), [], "{dir}");
    }
    // A project's link does not depend on it.
    home.atl()
        .env("CLAUDE_CONFIG_DIR", "claude")
        .args(["skill", "status", "--project"])
        .assert()
        .success();
}

/// Set but empty is unset: the link goes to `~/.claude`.
#[test]
fn an_empty_claude_config_dir_is_none() {
    let home = Home::with_claude();
    home.atl()
        .env("CLAUDE_CONFIG_DIR", "")
        .args(["skill", "install"])
        .assert()
        .success();
    assert_eq!(home.status(&[])["link_state"], json!("ok"));
}

#[test]
fn a_home_that_is_empty_a_file_or_no_text_is_refused() {
    let home = Home::new();
    let file = home.root.join("file");
    fs::write(&file, "").expect("write");
    let no_text = OsStr::from_bytes(b"/tmp/\xff");
    for (value, message) in [
        (
            OsStr::new(""),
            "HOME is not set, so there is no home directory to look for the skill in \
             (--project works in the current directory)",
        ),
        (
            file.as_os_str(),
            "HOME is '~/file', which is no absolute path of a directory",
        ),
        (
            no_text,
            "HOME is '/tmp/\u{fffd}', which is no absolute path of a directory",
        ),
    ] {
        for args in EVERY {
            assert_eq!(
                refused_with_env(&home, "HOME", value, args),
                message,
                "{args:?}"
            );
        }
    }
}

/// A directory that is gone has no path; only `--project` needs one.
#[test]
fn a_current_directory_that_is_gone_is_refused() {
    let home = Home::new();
    let atl = assert_cmd::cargo::cargo_bin("atl");
    let run = |args: &str| {
        let mut sh = Command::new("/bin/sh");
        sh.env_clear().env("HOME", &home.root).env("ATL", &atl);
        sh.current_dir(&home.root).arg("-c").arg(format!(
            "mkdir gone && cd gone && rmdir ../gone && exec \"$ATL\" --no-color skill {args}"
        ));
        sh
    };
    for args in EVERY {
        let said = home.refused_with(&mut run(&format!("{} --project", args.join(" "))));
        assert_eq!(
            said,
            (
                5,
                String::new(),
                "the current directory has no path atl can use (it is gone, or not UTF-8)"
                    .to_owned()
            ),
            "{args:?}"
        );
    }
    let (code, stdout, stderr) = home.finish(&mut run("install"));
    assert_eq!((code, stderr.as_str()), (0, ""), "{stdout}");
    assert_eq!(home.status(&[])["state"], json!("up_to_date"));
}

#[test]
fn a_skill_directory_that_is_no_directory_is_refused_always() {
    let home = Home::new();
    fs::create_dir_all(home.root.join(".agents/skills")).expect("mkdir");
    fs::write(home.skill(), "theirs").expect("write");
    for args in &EVERY[..6] {
        assert_eq!(
            home.refused(args),
            "~/.agents/skills/atl is not a directory: move it away first"
        );
    }
    let status = home.status(&[]);
    assert_eq!(
        (&status["state"], &status["installed"]),
        (&json!("blocked"), &json!(false))
    );
}

/// A file in the place of `.agents`, or of `skills` in it, or a link there
/// that leads to no directory: named, and it stays.
#[test]
fn what_is_no_directory_above_the_skill_directory_is_refused_always() {
    type Make = fn(&Path) -> std::io::Result<()>;
    let cases: [Make; 5] = [
        |at| fs::write(at, "theirs"),
        |at| symlink("/nowhere", at),
        |at| symlink(at, at),
        |at| {
            let file = at.with_file_name("file");
            fs::write(&file, "theirs").and_then(|()| symlink(file.join("x"), at))
        },
        |at| {
            let file = at.with_file_name("file");
            fs::write(&file, "theirs").and_then(|()| symlink(&file, at))
        },
    ];
    for place in [".agents", ".agents/skills"] {
        for (n, make) in cases.into_iter().enumerate() {
            let home = Home::with_claude();
            let at = home.root.join(place);
            fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
            make(&at).expect("make");
            for args in &EVERY[..6] {
                assert_eq!(
                    home.refused(args),
                    format!("~/{place} is not a directory: move it away first"),
                    "case {n}, {args:?}"
                );
            }
            let status = home.status(&[]);
            assert_eq!(
                (
                    &status["state"],
                    &status["installed"],
                    &status["link_state"]
                ),
                (&json!("blocked"), &json!(false), &json!("missing")),
                "case {n}"
            );
            // In a project, the same.
            let said = home.refused_with(home.atl().args(["skill", "install", "--project"]));
            assert_eq!(
                said,
                (
                    5,
                    String::new(),
                    format!("~/{place} is not a directory: move it away first")
                )
            );
        }
    }
}

/// A link that holds the path of the skill directory is no other link,
/// whether or not there is a skill at the time.
#[test]
fn another_spelling_of_the_link_is_no_other_link() {
    let home = Home::with_claude();
    fs::create_dir_all(home.root.join(".agents/skills")).expect("mkdir");
    fs::create_dir(home.root.join(".claude/skills")).expect("mkdir");
    symlink(home.skill(), home.link()).expect("symlink");
    for _ in 0..2 {
        assert_eq!(home.status(&[])["link_state"], json!("ok"));
        assert_eq!(
            home.json(&["install"]),
            linked(wrote("installed", VERSION, ""), "unchanged")
        );
        assert!(home.link().join("SKILL.md").is_file());
        assert_eq!(home.json(&["remove"]), removed(VERSION));
        assert!(home.link().is_symlink(), "not the one atl writes");
    }
}

/// A link to nothing in the place of Claude Code's `skills/`, or a file:
/// `install` says so before it writes anything, and `status` says it too.
/// The others make no link, so it is in no one's way.
#[test]
fn what_is_no_directory_in_the_place_of_claude_codes_skills_is_refused() {
    type Make = fn(&Path) -> std::io::Result<()>;
    let cases: [Make; 3] = [
        |at| symlink("/nowhere", at),
        |at| symlink("skills", at), // to itself
        |at| fs::write(at, "theirs"),
    ];
    for make in cases {
        let home = Home::with_claude();
        make(&home.root.join(".claude/skills")).expect("make");
        for args in [&["install"][..], &["install", "--force"]] {
            assert_eq!(
                home.refused(args),
                "~/.claude/skills is not a directory: move it away first"
            );
        }
        assert_eq!(home.status(&[])["link_state"], json!("taken"));
        home.install_edition(&OLD);
        home.ok(&["update"]);
        home.ok(&["remove"]);
        assert!(!home.skill().exists());
    }
}

/// A link there that leads to the shared directory, which has not been made
/// yet, is one to it.
#[test]
fn claude_codes_skills_may_lead_to_what_install_makes() {
    for (agents, target) in [
        (".agents", "../.agents/skills"),
        ("dotfiles", "../.agents/skills"),
        ("dotfiles", "../dotfiles/skills"),
    ] {
        let home = Home::with_claude();
        if agents != ".agents" {
            fs::create_dir(home.root.join(agents)).expect("mkdir");
            symlink(agents, home.root.join(".agents")).expect("symlink");
        }
        symlink(target, home.root.join(".claude/skills")).expect("symlink");
        assert_eq!(home.status(&[])["link_state"], json!("ok"), "{target}");
        home.ok(&["install"]);
        assert!(home.link().join("SKILL.md").is_file(), "{target}");
        home.ok(&["remove"]);
    }
}

/// Run by root only, as in a container: the skill directory is another
/// user's, the home directory is not.
#[test]
fn a_skill_directory_of_another_user_is_refused() {
    if modes_bind() {
        return; // not root
    }
    let home = Home::new();
    home.install_edition(&OLD);
    std::os::unix::fs::chown(home.skill(), Some(12345), None).expect("chown");
    for args in &EVERY[..6] {
        assert_eq!(
            home.refused(args),
            "~/.agents/skills/atl belongs to the user with id 12345, and atl runs as the user \
             with id 0: atl does not touch the skill there"
        );
    }
    let status = home.status(&[]);
    assert_eq!(
        (&status["state"], &status["installed"], &status["version"]),
        (&json!("other_owner"), &json!(false), &json!(""))
    );
    assert!(home.ok(&["status"]).contains("\nstate: other_owner\n"));
    // Not there yet, but in a directory of another user's.
    let home = Home::new();
    let theirs = home.root.join("theirs");
    fs::create_dir(&theirs).expect("mkdir");
    std::os::unix::fs::chown(&theirs, Some(12345), None).expect("chown");
    symlink("theirs", home.root.join(".agents")).expect("symlink");
    assert!(
        home.refused(&["install"])
            .starts_with("~/.agents belongs to the user with id 12345, ")
    );
}

/// A file that may not be deleted may not be replaced either: one line says
/// so, exit 1, and the file stays. What was done before it is not asked about
/// again.
#[test]
#[cfg(target_os = "macos")]
fn a_file_that_may_not_be_deleted_is_a_write_failure() {
    let system = |program: &str, args: &[&str], at: &Path| {
        let mut cmd = std::process::Command::new(program);
        assert!(cmd.args(args).arg(at).status().expect(program).success());
    };
    type Change<'a> = &'a dyn Fn(&Path);
    let cases: [(Change, Change); 2] = [
        (&|at| system("/usr/bin/chflags", &["uchg"], at), &|at| {
            system("/usr/bin/chflags", &["nouchg"], at);
        }),
        (
            &|at| system("/bin/chmod", &["+a", "everyone deny delete"], at),
            &|at| system("/bin/chmod", &["-N"], at),
        ),
    ];
    for (forbid, allow) in cases {
        let home = Home::new();
        home.install_edition(&OLD);
        let at = home.skill().join("references/jira-commands.md");
        forbid(&at);
        let updated = home.run(&["update"]);
        let removed = home.run(&["remove"]);
        allow(&at);
        let failed = |done: &str| {
            format!(
                "Error: cannot write ~/.agents/skills/atl/references/jira-commands.md: it cannot \
                 be {done}: its ACL or flags, or its directory, forbid deleting it\n"
            )
        };
        assert_eq!(updated, (1, String::new(), failed("replaced")));
        assert_eq!(removed, (1, String::new(), failed("removed")));
        assert_eq!(fs::read_to_string(&at).expect("read"), "old jira\n");
        home.ok(&["update", "--force"]);
        assert_eq!(home.status(&[])["state"], json!("up_to_date"));
    }
}

/// Every way a manifest can be none, through every command: `install` and
/// `update` say so and take the directory over with `--force`; `remove`
/// deletes nothing, `--force` or not.
#[test]
fn a_damaged_manifest_is_refused() {
    let listing = |path: &str| manifest(json!({ path: ZEROS }));
    let no_path = |path: &str| format!("it lists '{path}', which is no path inside the skill");
    for (text, reason) in [
        (
            String::new(),
            "it is not JSON: EOF while parsing a value at line 1 column 0".to_owned(),
        ),
        (
            "not json".to_owned(),
            "it is not JSON: expected ident at line 1 column 2".to_owned(),
        ),
        ("[]".to_owned(), "it has no schema number".to_owned()),
        ("null".to_owned(), "it has no schema number".to_owned()),
        (
            json!({"schema": -1}).to_string(),
            "it has no schema number".to_owned(),
        ),
        (
            json!({"schema": 1.0, "atl_version": VERSION, "files": {}}).to_string(),
            "it has no schema number".to_owned(),
        ),
        (
            json!({"schema": 1, "atl_version": 2026, "files": {}}).to_string(),
            "it names no atl version".to_owned(),
        ),
        (
            json!({"schema": 1, "atl_version": "2026.1.0\u{1b}[31m", "files": {}}).to_string(),
            "it names no atl version".to_owned(),
        ),
        // The key of another tool's manifest is not atl's.
        (
            json!({"schema": 1, "dq_version": VERSION, "files": {}}).to_string(),
            "it names no atl version".to_owned(),
        ),
        (
            json!({"schema": 1, "atl_version": VERSION}).to_string(),
            "it lists no files".to_owned(),
        ),
        (
            json!({"schema": 1, "atl_version": VERSION, "files": ["SKILL.md"]}).to_string(),
            "it lists no files".to_owned(),
        ),
        (
            manifest(json!({"SKILL.md": null})),
            "it has no SHA-256 for 'SKILL.md'".to_owned(),
        ),
        (
            manifest(json!({"SKILL.md": ZEROS.to_owned() + "0"})),
            "it has no SHA-256 for 'SKILL.md'".to_owned(),
        ),
        (listing("../outside.md"), no_path("../outside.md")),
        (
            listing("references/../../outside.md"),
            no_path("references/../../outside.md"),
        ),
        (listing("/tmp/outside.md"), no_path("/tmp/outside.md")),
        (listing(".."), no_path("..")),
        (listing(MANIFEST), no_path(MANIFEST)),
        (listing("a\\b.md"), no_path("a\\b.md")),
        (listing("c:/b.md"), no_path("c:/b.md")),
        // No file system has such a name.
        (
            listing(&"a".repeat(300)),
            no_path(&("a".repeat(255) + "\u{2026}")),
        ),
        (
            manifest((0..300).map(|n| (n.to_string(), json!(ZEROS))).collect()),
            "it lists more than 256 files".to_owned(),
        ),
        (
            json!({"schema": 1, "atl_version": VERSION, "files": {}, "unfinished": {"..": ZEROS}})
                .to_string(),
            no_path(".."),
        ),
        (
            json!({"schema": 1, "atl_version": VERSION, "files": {}, "unfinished": []}).to_string(),
            "it lists no unfinished files".to_owned(),
        ),
        (
            format!("{}{}", manifest(json!({})), " ".repeat(160 * 1024)),
            "it is larger than 163840 bytes".to_owned(),
        ),
        // Escaped where it is printed; the line stays one line.
        (listing("a\nb\u{1b}[31m"), no_path("a\\x0ab\\x1b[31m")),
    ] {
        let home = Home::new();
        home.ok(&["install"]);
        fs::write(home.skill().join("notes.md"), "theirs").expect("write");
        home.write_manifest(&text);
        for command in ["install", "update"] {
            assert_eq!(
                home.refused(&[command]),
                format!(
                    "atl cannot tell which files of ~/.agents/skills/atl it wrote: \
                     its .atl-skill.json cannot be used, {reason} \
                     (--force writes atl's files anew and keeps the rest)"
                )
            );
        }
        for args in [&["remove"][..], &["remove", "--force"]] {
            assert_eq!(
                home.refused(args),
                format!(
                    "~/.agents/skills/atl was not installed by atl \
                     (its .atl-skill.json cannot be used: {reason}): remove it yourself"
                )
            );
        }
        let status = home.status(&[]);
        assert_eq!(
            (&status["state"], &status["installed"], &status["version"]),
            (&json!("damaged"), &json!(true), &json!("")),
            "{text}"
        );
        assert_eq!(
            home.json(&["install", "--force"]),
            wrote("installed", VERSION, ""),
            "{text}"
        );
        let status = home.status(&[]);
        assert_eq!(
            (&status["state"], &status["edited"], &status["unlisted"]),
            (&json!("up_to_date"), &json!([]), &json!(["notes.md"])),
            "{text}"
        );
    }
}

/// The paths of a manifest are what `remove` deletes: none of them reaches
/// what is outside, with the right digest and with `--force` too.
#[test]
fn a_manifest_does_not_reach_out_of_the_directory() {
    let home = Home::new();
    let outside = home.root.join("outside");
    fs::create_dir(&outside).expect("mkdir");
    let victim = outside.join("victim.md");
    fs::write(&victim, "theirs").expect("write");
    let digest = atl::skill::sha256_hex(b"theirs");
    fs::create_dir_all(home.skill().join("a")).expect("mkdir");
    symlink(&outside, home.skill().join("out")).expect("symlink");
    symlink(&outside, home.skill().join("a/out")).expect("symlink");

    // Listed outright: no manifest.
    for path in [
        "../../../outside/victim.md".to_owned(),
        victim.to_str().expect("UTF-8").to_owned(),
    ] {
        home.write_manifest(&manifest(json!({ path: digest })));
        for args in &EVERY[..6] {
            if *args == ["install", "--force"] || *args == ["update", "--force"] {
                continue;
            }
            home.refused(args);
        }
        assert_eq!(home.status(&[])["state"], json!("damaged"));
    }
    // Through a link in the place of a directory.
    for (path, at) in [("out/victim.md", "out"), ("a/out/victim.md", "a/out")] {
        home.write_manifest(&manifest(json!({ path: digest })));
        for args in &EVERY[..6] {
            assert_eq!(
                home.refused(args),
                format!(
                    "~/.agents/skills/atl cannot hold the files of the skill ({at}): \
                     atl neither goes through a link nor replaces a directory"
                )
            );
        }
        let status = home.status(&[]);
        assert_eq!(
            (&status["edited"], &status["in_the_way"]),
            (&json!([]), &json!([at]))
        );
    }
    assert_eq!(fs::read_to_string(&victim).expect("read"), "theirs");

    // A directory, listed as a file; a file, as a directory.
    fs::write(home.skill().join("file"), "theirs").expect("write");
    for (path, at) in [("a", "a"), ("file/x.md", "file")] {
        home.write_manifest(&manifest(json!({ path: digest })));
        for args in &EVERY[..6] {
            let said = home.refused(args);
            assert!(said.contains(&format!("skill ({at}): ")), "{said}");
        }
    }

    // A link listed as a file: `--force` takes the link, not what it leads to.
    symlink(&victim, home.skill().join("link.md")).expect("symlink");
    home.write_manifest(&manifest(json!({ "link.md": digest })));
    assert!(home.refused(&["remove"]).contains("edited link.md"));
    home.ok(&["remove", "--force"]);
    assert!(!home.skill().join("link.md").is_symlink());
    assert_eq!(fs::read_to_string(&victim).expect("read"), "theirs");
}

/// `remove` deletes what the manifest lists and nothing else, whatever this
/// edition has.
#[test]
fn remove_leaves_a_file_the_manifest_does_not_list() {
    let home = Home::new();
    home.ok(&["install"]);
    home.change_manifest(|value| {
        value["files"]
            .as_object_mut()
            .expect("object")
            .remove("SKILL.md");
    });
    assert_eq!(
        home.json(&["remove"]),
        kept(removed(VERSION), &["SKILL.md"], &[])
    );
    assert_eq!(
        tree(&home.skill()),
        [("SKILL.md".to_owned(), source("SKILL.md"))]
    );
}

#[test]
fn a_manifest_of_a_newer_schema_is_refused_always() {
    let home = Home::with_claude();
    home.ok(&["install"]);
    // Whatever else it holds is the newer atl's business.
    home.write_manifest(r#"{"schema": 18446744073709551615, "files": ["../x"]}"#);
    for args in &EVERY[..6] {
        assert_eq!(
            home.refused(args),
            "the skill in ~/.agents/skills/atl was installed by a newer atl \
             (.atl-skill.json has schema 18446744073709551615, this atl knows 1): use that atl"
        );
    }
    let status = home.status(&[]);
    assert_eq!(
        (&status["state"], &status["installed"], &status["version"]),
        (&json!("unknown_schema"), &json!(true), &json!(""))
    );
}

#[test]
fn a_manifest_that_is_a_directory_is_refused_always() {
    let home = Home::new();
    home.ok(&["install"]);
    let at = home.skill().join(MANIFEST);
    fs::remove_file(&at).expect("rm");
    fs::create_dir(&at).expect("mkdir");
    fs::write(at.join("theirs"), "theirs").expect("write");
    for args in &EVERY[..4] {
        assert_eq!(
            home.refused(args),
            "~/.agents/skills/atl cannot hold the files of the skill (.atl-skill.json): \
             atl neither goes through a link nor replaces a directory"
        );
    }
    for args in &EVERY[4..6] {
        assert_eq!(
            home.refused(args),
            "~/.agents/skills/atl was not installed by atl (its .atl-skill.json cannot be used: \
             it is no regular file): remove it yourself"
        );
    }
    assert_eq!(home.status(&[])["state"], json!("damaged"));
}

/// Reading a FIFO would wait for a writer forever.
#[test]
fn a_fifo_in_the_place_of_a_file_is_not_read() {
    let home = Home::new();
    for path in ["SKILL.md", MANIFEST] {
        home.ok(&["install", "--force"]);
        let at = home.skill().join(path);
        fs::remove_file(&at).expect("rm");
        let name = std::ffi::CString::new(at.as_os_str().as_bytes()).expect("no NUL");
        // SAFETY: a NUL-terminated path, which the call only reads.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        for command in ["install", "update", "remove"] {
            home.refused(&[command]);
        }
        home.status(&[]);
        home.ok(&["install", "--force"]);
        assert!(at.is_file(), "{path}");
    }
}

/// Names with control and direction characters: escaped in a refusal, which
/// is one line on stderr, and raw in JSON.
#[test]
fn names_with_controls_are_escaped_in_a_refusal_and_raw_in_json() {
    const NAME: &str = "a\nb\u{1b}[31m\u{202e}\u{9b}x";
    const SHOWN: &str = "a\\x0ab\\x1b[31m\\u202e\\x9bx";

    // In the path of the home directory, so of everything.
    let mut home = Home::with_claude();
    home.root.push(NAME);
    fs::create_dir_all(home.root.join(".claude")).expect("mkdir");
    let root = home.root.to_str().expect("UTF-8").to_owned();
    let shown = root.replace(NAME, SHOWN);
    let raw = |args: &[&str]| -> Value {
        let output = home
            .atl()
            .args(["-F", "json", "skill"])
            .args(args)
            .output()
            .expect("run atl");
        assert!(output.status.success(), "{args:?}");
        serde_json::from_slice(&output.stdout).expect("JSON")
    };
    let report = raw(&["install"]);
    assert_eq!(report["path"], json!(format!("{root}/.agents/skills/atl")));
    assert_eq!(report["link"], json!(format!("{root}/.claude/skills/atl")));
    let status = raw(&["status"]);
    assert_eq!(status["path"], json!(format!("{root}/.agents/skills/atl")));
    assert_eq!(status["link"], json!(format!("{root}/.claude/skills/atl")));
    fs::write(home.skill().join("SKILL.md"), "mine").expect("write");
    let output = home
        .atl()
        .args(["skill", "update"])
        .output()
        .expect("run atl");
    assert_eq!(output.status.code(), Some(5));
    assert_eq!(
        String::from_utf8(output.stderr).expect("UTF-8"),
        format!(
            "Error: invalid input: {shown}/.agents/skills/atl was changed since atl wrote it: \
             edited SKILL.md (--force writes atl's files in their place)\n"
        )
    );
    assert_eq!(raw(&["remove", "--force"])["action"], json!("removed"));

    // In the name of a file that is not atl's.
    let home = Home::with_claude();
    fs::create_dir_all(home.skill()).expect("mkdir");
    fs::write(home.skill().join(NAME), "theirs").expect("write");
    assert_eq!(
        home.refused(&["install"]),
        format!(
            "~/.agents/skills/atl was not installed by atl ({SHOWN}): \
             --force writes atl's files into it and keeps the rest"
        )
    );
    assert_eq!(home.status(&[])["unlisted"], json!([NAME]));
    home.ok(&["install", "--force"]);
    assert_eq!(home.json(&["remove"])["left"], json!([NAME]));

    // Of one the manifest lists; a direction control is in no one's way.
    let home = Home::new();
    home.ok(&["install"]);
    home.change_manifest(|value| {
        value["files"]["x\u{202e}.md"] = json!(ZEROS);
        value["atl_version"] = json!("2020.1.0");
    });
    fs::write(home.skill().join("x\u{202e}.md"), "mine").expect("write");
    assert_eq!(
        home.refused(&["update"]),
        "~/.agents/skills/atl was changed since atl wrote it: edited x\\u202e.md \
         (--force writes atl's files in their place)"
    );
    let mut updated = wrote("updated", VERSION, "2020.1.0");
    updated["kept"] = json!(["x\u{202e}.md"]);
    assert_eq!(home.json(&["update", "--force"]), updated);

    // Of where another link leads.
    let home = Home::with_claude();
    fs::create_dir(home.root.join(".claude/skills")).expect("mkdir");
    symlink(NAME, home.link()).expect("symlink");
    assert_eq!(
        home.refused(&["install"]),
        format!("~/.claude/skills/atl is a link to '{SHOWN}': --force replaces it")
    );
}
