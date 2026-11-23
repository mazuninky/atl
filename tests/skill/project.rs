//! `--project`: the skill of the current directory, and nothing else.

use pretty_assertions::assert_eq;

use super::*;

/// A home directory with a skill and Claude Code, and a project in it.
fn project() -> (Home, PathBuf) {
    let home = Home::with_claude();
    home.ok(&["install"]);
    let dir = home.root.join("project");
    fs::create_dir(&dir).expect("mkdir");
    (home, dir)
}

/// `atl -F json skill ARGS --project` in `dir`, which must succeed: the
/// object it reports.
fn ok(home: &Home, dir: &Path, args: &[&str]) -> Value {
    let mut atl = home.atl();
    atl.current_dir(dir)
        .args(["-F", "json", "skill"])
        .args(args)
        .arg("--project");
    let (code, stdout, stderr) = home.finish(&mut atl);
    assert_eq!((code, stderr.as_str()), (0, ""), "{args:?}");
    serde_json::from_str(&stdout).expect("JSON")
}

/// Every directory under `dir`, sorted.
fn directories(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut todo = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, prefix)) = todo.pop() {
        for entry in fs::read_dir(&at).expect("read dir") {
            let entry = entry.expect("entry");
            if entry.file_type().expect("type").is_dir() {
                let path = format!("{prefix}{}", entry.file_name().to_str().expect("UTF-8"));
                todo.push((entry.path(), format!("{path}/")));
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// `report` of the project in `~/project`, not of the home directory.
fn of_project(mut report: Value) -> Value {
    report["path"] = json!("~/project/.agents/skills/atl");
    if report["link"] != json!("") {
        report["link"] = json!("~/project/.claude/skills/atl");
    }
    report
}

/// Claude Code is not there: no link, and no directory of its.
#[test]
fn a_project_without_claude_code_gets_no_link() {
    for taken in [false, true] {
        let (home, dir) = project();
        if taken {
            fs::write(dir.join(".claude"), "theirs").expect("write");
        }
        let before = tree(&dir);
        assert_eq!(
            ok(&home, &dir, &["install"]),
            of_project(wrote("installed", VERSION, ""))
        );
        assert_eq!(directories(&dir)[0], ".agents", "taken: {taken}");
        assert!(!directories(&dir).contains(&".claude".to_owned()));
        let status = ok(&home, &dir, &["status"]);
        assert_eq!(status["link_state"], json!("not_wanted"));
        assert_eq!(ok(&home, &dir, &["remove"]), of_project(removed(VERSION)));
        assert_eq!(tree(&dir), before, "taken: {taken}");
    }
}

/// Installing and removing leaves no file of atl's and changes none of the
/// project's; what stays is empty directories, which git does not see.
#[test]
fn removing_leaves_the_project_as_it_was() {
    let (home, dir) = project();
    for (path, text) in [
        (".git/HEAD", "ref: refs/heads/main\n"),
        (".git/index", "index"),
        (".claude/settings.json", "{}"),
        (".claude/skills/other/SKILL.md", "theirs"),
        (".agents/skills/other/SKILL.md", "theirs"),
        ("README.md", "# project\n"),
    ] {
        let at = dir.join(path);
        fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
        fs::write(at, text).expect("write");
    }
    let before = (tree(&dir), directories(&dir));
    assert_eq!(
        ok(&home, &dir, &["install"]),
        of_project(linked(wrote("installed", VERSION, ""), "created"))
    );
    assert_eq!(
        fs::read_link(dir.join(".claude/skills/atl")).expect("a link"),
        Path::new("../../.agents/skills/atl"),
        "relative, so that it is the same in every checkout"
    );
    let mut removed = of_project(removed(VERSION));
    removed["link"] = json!("~/project/.claude/skills/atl");
    assert_eq!(ok(&home, &dir, &["remove"]), removed);
    assert_eq!((tree(&dir), directories(&dir)), before);
}

/// The project is the current directory, not the repository it is in.
#[test]
fn a_directory_inside_a_repository_is_a_project_of_its_own() {
    let (home, dir) = project();
    fs::create_dir_all(dir.join(".git")).expect("mkdir");
    fs::create_dir_all(dir.join(".claude")).expect("mkdir");
    let inside = dir.join("crates/one");
    fs::create_dir_all(&inside).expect("mkdir");
    ok(&home, &inside, &["install"]);
    assert!(inside.join(".agents/skills/atl/SKILL.md").is_file());
    assert_eq!(directories(&dir)[..2], [".claude", ".git"]);
    assert!(!dir.join(".agents").exists());
    ok(&home, &inside, &["remove"]);
    assert_eq!(tree(&dir), []);
}

/// The two skills do not know of each other, and neither has a link to the
/// other's.
#[test]
fn the_skill_of_the_home_directory_is_left_alone() {
    let (home, dir) = project();
    fs::create_dir(dir.join(".claude")).expect("mkdir");
    let outside = |home: &Home| {
        let mut entries = tree(&home.root);
        entries.retain(|(path, _)| !path.starts_with("project/"));
        entries
    };
    let before = outside(&home);
    let config = home.root.join("config");
    fs::create_dir(&config).expect("mkdir");
    for args in [
        &["install"][..],
        &["update"],
        &["install", "--force"],
        &["remove"],
    ] {
        home.atl()
            .current_dir(&dir)
            .env("CLAUDE_CONFIG_DIR", &config)
            .arg("skill")
            .args(args)
            .arg("--project")
            .assert()
            .success();
        assert_eq!(outside(&home), before, "{args:?}");
    }
    ok(&home, &dir, &["install"]);
    home.ok(&["remove"]);
    assert!(dir.join(".claude/skills/atl/SKILL.md").is_file());
}

/// A link to the skill directory of the home directory is another link: the
/// project's would lead to its own.
#[test]
fn a_link_to_the_skill_of_the_home_directory_is_another_link() {
    let (home, dir) = project();
    fs::create_dir_all(dir.join(".claude/skills")).expect("mkdir");
    symlink(home.skill(), dir.join(".claude/skills/atl")).expect("symlink");
    let said =
        home.refused_with(
            home.atl()
                .current_dir(&dir)
                .args(["skill", "install", "--project"]),
        );
    assert_eq!(
        said,
        (
            5,
            String::new(),
            "~/project/.claude/skills/atl is a link to '~/.agents/skills/atl': \
             --force replaces it"
                .to_owned()
        )
    );
    assert!(!dir.join(".agents").exists());
    assert_eq!(
        ok(&home, &dir, &["install", "--force"]),
        of_project(linked(wrote("installed", VERSION, ""), "replaced"))
    );
    assert_eq!(
        fs::read_link(dir.join(".claude/skills/atl")).expect("a link"),
        Path::new("../../.agents/skills/atl")
    );
    assert!(home.skill().join("SKILL.md").is_file());
}
