//! A home directory of a test's own, and the real binary run in it: shared
//! by `cli_skill.rs` and `cli_skill_release.rs`, each of which includes this
//! file with `#[path]`. Not every helper is used by both.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use atl::skill::{Edition, Manifest, SkillFile};
use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The skill of an atl older than any: other text in two files this one has,
/// and a file it has not.
pub const OLD: Edition = Edition {
    version: "2020.1.0",
    files: &[
        SkillFile {
            path: "SKILL.md",
            text: "old skill\n",
        },
        SkillFile {
            path: "references/jira-commands.md",
            text: "old jira\n",
        },
        SkillFile {
            path: "references/gone.md",
            text: "old gone\n",
        },
    ],
};

/// The manifest, relative to the skill directory.
pub const MANIFEST: &str = ".atl-skill.json";

/// What an installed skill of this version has, sorted.
pub const FILES: [&str; 5] = [
    MANIFEST,
    "SKILL.md",
    "references/body-formats.md",
    "references/confluence-commands.md",
    "references/jira-commands.md",
];

/// What every refusal says first on stderr: `main`'s prefix and
/// `Error::InvalidInput`'s.
const REFUSED: &str = "Error: invalid input: ";

/// A home directory.
pub struct Home {
    /// Held for its `Drop`.
    _tmp: TempDir,
    pub root: PathBuf,
}

impl Home {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().canonicalize().expect("canonical tempdir");
        Self { _tmp: tmp, root }
    }

    /// With Claude Code in it.
    pub fn with_claude() -> Self {
        let home = Self::new();
        fs::create_dir(home.root.join(".claude")).expect("mkdir");
        home
    }

    /// With a config file that has an alias in it, so that `alias list` has
    /// something to say without a network.
    pub fn with_alias() -> Self {
        let home = Self::new();
        fs::write(
            home.root.join("atl.toml"),
            "[aliases]\nhi = \"jira view\"\n",
        )
        .expect("write");
        home
    }

    /// `atl`, with this home directory and nothing else of the environment
    /// the tests run in.
    pub fn atl(&self) -> Command {
        let mut cmd = Command::cargo_bin("atl").expect("atl binary built");
        cmd.env_clear();
        cmd.env("HOME", &self.root);
        cmd.current_dir(&self.root);
        cmd.arg("--no-color");
        cmd
    }

    pub fn skill(&self) -> PathBuf {
        self.root.join(".agents/skills/atl")
    }

    pub fn link(&self) -> PathBuf {
        self.root.join(".claude/skills/atl")
    }

    /// Write what the atl `edition` belongs to would. Not by `atl::skill`: a
    /// process that another test forks has the locks of this one until it
    /// execs, and the automatic update would find the skill busy.
    pub fn install_edition(&self, edition: &Edition) {
        let manifest = Manifest::of(edition).to_bytes();
        let files = edition.files.iter();
        for (path, bytes) in files
            .map(|file| (file.path, file.text.as_bytes()))
            .chain([(MANIFEST, manifest.as_slice())])
        {
            let at = self.skill().join(path);
            fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
            fs::write(at, bytes).expect("write");
        }
    }

    /// Put `text` where the manifest is.
    pub fn write_manifest(&self, text: &str) {
        fs::write(self.skill().join(MANIFEST), text).expect("write manifest");
    }

    /// The manifest, changed by `change`.
    pub fn change_manifest(&self, change: impl FnOnce(&mut Value)) {
        let bytes = fs::read(self.skill().join(MANIFEST)).expect("read manifest");
        let mut value = serde_json::from_slice(&bytes).expect("JSON");
        change(&mut value);
        self.write_manifest(&value.to_string());
    }

    /// Run `cmd`, which must end by itself: a minute is far more than any
    /// command takes. Exit code, stdout, stderr, with the home directory
    /// spelled `~`.
    pub fn finish(&self, cmd: &mut Command) -> (i32, String, String) {
        let output = cmd
            .timeout(std::time::Duration::from_secs(60))
            .output()
            .expect("run atl");
        let text = |bytes: Vec<u8>| {
            String::from_utf8(bytes)
                .expect("UTF-8")
                .replace(self.root.to_str().expect("UTF-8 tempdir"), "~")
        };
        (
            output.status.code().expect("exit code"),
            text(output.stdout),
            text(output.stderr),
        )
    }

    /// `atl skill ARGS`: exit code, stdout, stderr.
    pub fn run(&self, args: &[&str]) -> (i32, String, String) {
        self.finish(self.atl().arg("skill").args(args))
    }

    /// `atl skill ARGS`, which must succeed; what it prints.
    pub fn ok(&self, args: &[&str]) -> String {
        let (code, stdout, stderr) = self.run(args);
        assert_eq!((code, stderr.as_str()), (0, ""), "atl skill {args:?}");
        stdout
    }

    /// `atl -F json skill ARGS`, which must succeed: the object it reports.
    pub fn json(&self, args: &[&str]) -> Value {
        let stdout = self.ok(&[&["-F", "json"], args].concat());
        serde_json::from_str(&stdout).expect("JSON")
    }

    /// `atl skill ARGS`, which must be refused: exit 5, nothing on stdout,
    /// nothing written. The message, without the prefix every refusal has.
    pub fn refused(&self, args: &[&str]) -> String {
        let (code, stdout, stderr) = self.refused_with(self.atl().arg("skill").args(args));
        assert_eq!((code, stdout.as_str()), (5, ""), "atl skill {args:?}");
        stderr
    }

    /// Run `cmd`, which must write nothing: exit code, stdout, and stderr
    /// without the prefix of a refusal, which it must have, in one line.
    pub fn refused_with(&self, cmd: &mut Command) -> (i32, String, String) {
        let before = tree(&self.root);
        let (code, stdout, stderr) = self.finish(cmd);
        assert_eq!(
            tree(&self.root),
            before,
            "a refused command wrote something"
        );
        let message = stderr
            .strip_prefix(REFUSED)
            .unwrap_or_else(|| panic!("no refusal: {stderr:?}"));
        let message = message
            .strip_suffix('\n')
            .unwrap_or_else(|| panic!("no line: {stderr:?}"));
        assert!(!message.contains('\n'), "more than one line: {stderr:?}");
        (code, stdout, message.to_owned())
    }

    /// `atl skill ARGS`, which must fail as a read or a write fails: exit 1,
    /// nothing on stdout. What it says on stderr.
    pub fn failed(&self, args: &[&str]) -> String {
        let (code, stdout, stderr) = self.run(args);
        assert_eq!(
            (code, stdout.as_str()),
            (1, ""),
            "atl skill {args:?}: {stderr}"
        );
        assert!(stderr.starts_with("Error: cannot "), "{stderr}");
        assert!(!stderr.trim_end().contains('\n'), "{stderr}");
        stderr
    }

    /// `atl -F json skill status ARGS`.
    pub fn status(&self, args: &[&str]) -> Value {
        self.json(&[&["status"], args].concat())
    }
}

/// Every file and link under `dir` with what is in it, sorted by path.
pub fn tree(dir: &Path) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut todo = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, prefix)) = todo.pop() {
        let Ok(read) = fs::read_dir(&at) else {
            // One the test made unreadable: what is in it is not seen.
            entries.push((prefix, "unreadable".to_owned()));
            continue;
        };
        for entry in read {
            let entry = entry.expect("entry");
            let path = format!("{prefix}{}", entry.file_name().to_str().expect("UTF-8"));
            let kind = entry.file_type().expect("type");
            if kind.is_dir() {
                todo.push((entry.path(), format!("{path}/")));
                continue;
            }
            let holds = if kind.is_symlink() {
                let to = fs::read_link(entry.path()).expect("link");
                format!("-> {}", to.display())
            } else if kind.is_file() {
                fs::read_to_string(entry.path()).unwrap_or_else(|_| "unreadable".to_owned())
            } else {
                "special".to_owned() // a FIFO: reading it would wait
            };
            entries.push((path, holds));
        }
    }
    entries.sort();
    entries
}

/// The skill in this repository.
pub fn source(path: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("skill")
        .join(path);
    fs::read_to_string(file).expect("skill file")
}

/// Set the mode of `path`.
#[cfg(unix)]
pub fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}

/// Whether a mode binds whoever runs the tests; root's does not.
#[cfg(unix)]
pub fn modes_bind() -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    chmod(dir.path(), 0o500);
    let bound = fs::write(dir.path().join("probe"), "").is_err();
    chmod(dir.path(), 0o700);
    bound
}
