//! The agent skill the binary carries: writing it to the disk, keeping it at
//! the version of the binary, removing it.
//!
//! What atl wrote is known by a manifest in the skill directory
//! ([`manifest`]). Nothing here overwrites or deletes a file the manifest
//! does not vouch for, unless `--force` says so; nothing is written through a
//! symbolic link in the place of the skill directory or inside it.
//!
//! The command is `atl skill` ([`crate::cli::commands::skill`]); the update
//! after any other command is [`FirstRun`], called from `main`.

mod atomic;
mod content;
mod disk;
mod error;
mod first_run;
mod link;
mod manifest;
mod ops;
mod place;
mod survey;

use std::ffi::OsString;
use std::fmt::Write as _;

use camino::{Utf8Path, Utf8PathBuf};

pub use content::{Edition, SkillFile};
pub use error::{FileError, Refusal, SkillError, listed, listed_up_to};
pub use first_run::{FirstRun, Outcome, Skipped};
pub use link::{ClaudeLink, LinkState, Made};
pub use manifest::{MANIFEST, Manifest, ManifestError, SCHEMA, sha256_hex};
pub use ops::{
    Freshness, RemoveReport, State, Status, WriteReport, install, installed_note,
    installed_version, remove, status, update,
};

/// The skill directory, relative to the home directory or a project's.
const SKILL_DIR: &str = ".agents/skills/atl";

/// The link to it, relative to Claude Code's directory.
const CLAUDE_LINK: &str = "skills/atl";

/// Claude Code's directory, relative to the home directory or a project's.
const CLAUDE_DIR: &str = ".claude";

/// What the process environment says about where the skill is and whether it
/// may be updated without being asked. Filled in once, at the edge, so that
/// everything else can be tested on a directory of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    /// `HOME`; on Windows `USERPROFILE` when `HOME` is unset or empty.
    pub home: Option<OsString>,
    /// `CLAUDE_CONFIG_DIR`.
    pub claude_config_dir: Option<OsString>,
    /// `CI`.
    pub ci: Option<OsString>,
    /// `ATL_NO_SKILL_UPDATE`.
    pub no_skill_update: Option<OsString>,
    /// The effective user id; `None` where there is none (Windows).
    pub euid: Option<u32>,
    /// The current directory, for `--project`; `None` when there is none
    /// atl can use, or nobody asked ([`Environment::with_current_dir`]).
    pub current_dir: Option<Utf8PathBuf>,
}

impl Environment {
    /// The environment of this process, without its current directory.
    #[must_use]
    pub fn from_process() -> Self {
        let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
        #[cfg(windows)]
        let home = home.or_else(|| std::env::var_os("USERPROFILE"));
        Self {
            home,
            claude_config_dir: std::env::var_os("CLAUDE_CONFIG_DIR"),
            ci: std::env::var_os("CI"),
            no_skill_update: std::env::var_os("ATL_NO_SKILL_UPDATE"),
            euid: effective_user(),
            current_dir: None,
        }
    }

    /// With the current directory of this process.
    #[must_use]
    pub fn with_current_dir(mut self) -> Self {
        self.current_dir = std::env::current_dir()
            .ok()
            .and_then(|dir| Utf8PathBuf::from_path_buf(dir).ok());
        self
    }

    /// The home directory, as `HOME` names it.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoHome`] when it is unset or empty, [`Refusal::BadHome`]
    /// when it is no absolute UTF-8 path.
    pub fn home(&self) -> Result<&Utf8Path, Refusal> {
        let home = self
            .home
            .as_deref()
            .filter(|home| !home.is_empty())
            .ok_or(Refusal::NoHome)?;
        Utf8Path::from_path(home.as_ref())
            .filter(|home| home.is_absolute())
            .ok_or_else(|| Refusal::BadHome {
                home: home.to_string_lossy().into_owned(),
            })
    }

    /// Whether `ATL_NO_SKILL_UPDATE` asks atl to leave the skill alone: set
    /// to anything but `0`.
    #[must_use]
    pub fn opted_out(&self) -> bool {
        self.no_skill_update
            .as_deref()
            .is_some_and(|value| value != "0")
    }

    /// The user `dir` belongs to, when atl runs as root and that is someone
    /// else (`sudo`): what atl wrote there would be root's.
    fn foreign_owner(&self, dir: &Utf8Path) -> Option<u32> {
        if self.euid != Some(0) {
            return None;
        }
        place::owner(dir).filter(|owner| *owner != 0)
    }

    /// Whether atl runs as root while the home directory is another user's.
    #[must_use]
    pub fn is_root_in_foreign_home(&self) -> bool {
        self.home()
            .is_ok_and(|home| self.foreign_owner(home).is_some())
    }

    /// The skill of the home directory.
    ///
    /// The link is wanted when Claude Code is there — the directory
    /// `CLAUDE_CONFIG_DIR` names, or `~/.claude` when it is unset, exists —
    /// and the platform has symbolic links (not Windows).
    ///
    /// # Errors
    ///
    /// A [`Refusal`] when `HOME` is no directory or `CLAUDE_CONFIG_DIR` no
    /// absolute path.
    pub fn global(&self) -> Result<Target, Refusal> {
        let home = self.home()?;
        if !home.is_dir() {
            return Err(Refusal::BadHome {
                home: home.to_string(),
            });
        }
        let claude = match self.claude_config_dir.as_deref() {
            Some(dir) if !dir.is_empty() => Utf8Path::from_path(dir.as_ref())
                .filter(|dir| dir.is_absolute())
                .ok_or_else(|| Refusal::BadClaudeDir {
                    dir: dir.to_string_lossy().into_owned(),
                })?
                .to_path_buf(),
            _ => home.join(CLAUDE_DIR),
        };
        Ok(Target {
            foreign_owner: self.foreign_owner(home),
            user: self.euid,
            ..Target::new(home, &claude)
        })
    }

    /// The skill of the project in the current directory. Whom that belongs
    /// to does not matter: root in a checkout of another user's is what a
    /// container or a CI job is.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoCurrentDir`] when there is none.
    pub fn project(&self) -> Result<Target, Refusal> {
        let dir = self.current_dir.as_deref().ok_or(Refusal::NoCurrentDir)?;
        Ok(Target::new(dir, &dir.join(CLAUDE_DIR)))
    }
}

/// Where one skill is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The home directory, or the project's.
    pub root: Utf8PathBuf,
    /// The skill directory.
    pub dir: Utf8PathBuf,
    /// The link to it; `None` where Claude Code is not, or links are not.
    pub link: Option<ClaudeLink>,
    /// Whom the home directory belongs to, when this is its skill, atl runs
    /// as root and that is someone else: nothing is written or deleted then.
    pub foreign_owner: Option<u32>,
    /// Whom the skill directory has to belong to, when this is the skill of
    /// the home directory: the user atl runs as, where there are users.
    /// Nothing is written or deleted in anyone else's.
    pub user: Option<u32>,
}

impl Target {
    /// The skill of `root`, where Claude Code keeps its files in `claude`.
    fn new(root: &Utf8Path, claude: &Utf8Path) -> Self {
        let link = (cfg!(unix) && claude.is_dir()).then(|| ClaudeLink {
            place: claude.join(CLAUDE_LINK),
            root: root.to_path_buf(),
            inside: SKILL_DIR.into(),
        });
        Self {
            root: root.to_path_buf(),
            dir: root.join(SKILL_DIR),
            link,
            foreign_owner: None,
            user: None,
        }
    }
}

/// `text` as one line that a terminal shows as it is: control characters,
/// line and paragraph separators and the characters that turn text around
/// (`U+202E`) are written as escapes: `\x` and two hex digits, or `\u` and four. Names on a disk
/// may hold any of them, and every message names some.
#[must_use]
pub fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let code = u32::from(c);
        if c.is_control() || is_invisible_control(code) {
            if code < 0x100 {
                let _ = write!(out, "\\x{code:02x}");
            } else {
                let _ = write!(out, "\\u{code:04x}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether `code` is a line or paragraph separator or a bidirectional
/// formatting character.
fn is_invisible_control(code: u32) -> bool {
    matches!(
        code,
        0x061C | 0x200E | 0x200F | 0x2028 | 0x2029 | 0x202A..=0x202E | 0x2066..=0x2069
    )
}

#[cfg(unix)]
fn effective_user() -> Option<u32> {
    // SAFETY: `geteuid(2)` takes nothing, touches no memory and cannot fail.
    Some(unsafe { libc::geteuid() })
}

#[cfg(not(unix))]
fn effective_user() -> Option<u32> {
    None
}

/// Shared by the tests of the skill.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::Path;

    /// Make sure that `dir`, a directory of the test's, is not root's, also
    /// when root runs the tests. The id of its owner.
    #[cfg(unix)]
    pub(crate) fn not_roots(dir: &Path) -> u32 {
        use std::os::unix::fs::MetadataExt;
        let owner = || std::fs::metadata(dir).expect("metadata").uid();
        if owner() == 0 {
            std::os::unix::fs::chown(dir, Some(12345), None).expect("chown");
        }
        owner()
    }

    /// Every entry under `dir`, directories too, as `/`-separated paths,
    /// sorted; links are not followed.
    pub(crate) fn entries_under(dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut todo = vec![(dir.to_path_buf(), String::new())];
        while let Some((at, prefix)) = todo.pop() {
            for entry in std::fs::read_dir(&at).expect("read dir") {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_line_is_one_line_that_shows_as_it_is() {
        assert_eq!(one_line("plain/path ok"), "plain/path ok");
        assert_eq!(
            one_line("a\nb\x1b[31m\u{202e}c\u{2028}\u{7f}é"),
            "a\\x0ab\\x1b[31m\\u202ec\\u2028\\x7fé"
        );
    }

    #[test]
    fn home_is_an_absolute_path() {
        let env = |home: Option<&str>| Environment {
            home: home.map(Into::into),
            ..Environment::default()
        };
        assert_eq!(env(None).home(), Err(Refusal::NoHome));
        assert_eq!(env(Some("")).home(), Err(Refusal::NoHome));
        assert_eq!(
            env(Some("rel")).home(),
            Err(Refusal::BadHome {
                home: "rel".to_owned()
            })
        );
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = Utf8Path::from_path(tmp.path()).expect("UTF-8");
        let found = env(Some(root.as_str()));
        assert_eq!(found.home(), Ok(root));
        let target = found.global().expect("global");
        assert_eq!(target.dir, root.join(".agents/skills/atl"));
        assert_eq!(target.link, None, "no Claude Code");
        assert!(matches!(
            env(Some(root.join("none").as_str())).global(),
            Err(Refusal::BadHome { .. })
        ));
    }

    #[test]
    fn the_link_is_wanted_where_claude_code_is() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = Utf8Path::from_path(tmp.path()).expect("UTF-8");
        std::fs::create_dir(root.join(".claude")).expect("mkdir");
        let mut env = Environment {
            home: Some(root.as_str().into()),
            ..Environment::default()
        };
        let link = env.global().expect("global").link;
        assert_eq!(
            link.map(|link| link.place),
            cfg!(unix).then(|| root.join(".claude/skills/atl"))
        );
        env.claude_config_dir = Some("relative".into());
        assert!(matches!(env.global(), Err(Refusal::BadClaudeDir { .. })));
        env.claude_config_dir = Some(root.join("none").as_str().into());
        assert_eq!(env.global().expect("global").link, None);
        assert_eq!(env.project(), Err(Refusal::NoCurrentDir));
        env.current_dir = Some(root.to_path_buf());
        assert_eq!(
            env.project().expect("project").dir,
            root.join(".agents/skills/atl")
        );
    }

    #[test]
    fn only_zero_does_not_opt_out() {
        for (value, opted_out) in [
            (None, false),
            (Some("0"), false),
            (Some(""), true),
            (Some("1"), true),
            (Some("false"), true),
            (Some(" 0"), true),
        ] {
            let env = Environment {
                no_skill_update: value.map(Into::into),
                ..Environment::default()
            };
            assert_eq!(env.opted_out(), opted_out, "{value:?}");
        }
    }
}
