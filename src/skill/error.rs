//! Why `atl skill` did not do what it was asked.
//!
//! A [`Refusal`] is atl declining: nothing was written, and the command maps
//! it to `Error::InvalidInput` (exit 5). A [`FileError`] is a file that could
//! not be read or written (exit 1).

use camino::Utf8PathBuf;
use thiserror::Error;

use super::manifest::{MANIFEST, SCHEMA};
use super::one_line;

/// How many file names a message lists before `and N more`.
const SHOWN_FILES: usize = 8;

/// An error of an `atl skill` command.
#[derive(Debug, Error)]
pub enum SkillError {
    /// atl will not do it: nothing written.
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// Reading or writing a file failed.
    #[error(transparent)]
    File(#[from] FileError),
}

/// A file that could not be read or written. The message is one line, with
/// what the path holds of control characters escaped.
#[derive(Debug, Error)]
pub enum FileError {
    /// Reading `path` failed.
    #[error("cannot read {}: {}", one_line(path.as_str()), one_line(&cause.to_string()))]
    Read {
        /// What could not be read.
        path: Utf8PathBuf,
        /// Why. Not a `source`: the message says it already.
        cause: std::io::Error,
    },
    /// Writing, replacing or deleting `path` failed.
    #[error("cannot write {}: {}", one_line(path.as_str()), one_line(&cause.to_string()))]
    Write {
        /// What could not be written.
        path: Utf8PathBuf,
        /// Why. Not a `source`: the message says it already.
        cause: std::io::Error,
    },
}

/// A refusal: what is on the disk is not atl's to change.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Refusal {
    /// `HOME` is not set, or empty.
    #[error(
        "HOME is not set, so there is no home directory to look for the skill in \
         (--project works in the current directory)"
    )]
    NoHome,
    /// `HOME` is not an absolute path of a directory.
    #[error("HOME is '{home}', which is no absolute path of a directory")]
    BadHome {
        /// What the variable holds.
        home: String,
    },
    /// `CLAUDE_CONFIG_DIR` is not an absolute path.
    #[error("CLAUDE_CONFIG_DIR is '{dir}', which is no absolute path")]
    BadClaudeDir {
        /// What the variable holds.
        dir: String,
    },
    /// The current directory has no usable path.
    #[error("the current directory has no path atl can use (it is gone, or not UTF-8)")]
    NoCurrentDir,
    /// Root, in another user's home directory.
    #[error(
        "{home} belongs to the user with id {owner}, and atl runs as root: run it without sudo, \
         or what it writes there would be root's"
    )]
    ForeignOwner {
        /// The home directory.
        home: Utf8PathBuf,
        /// The user it belongs to.
        owner: u32,
    },
    /// The skill directory is another user's.
    #[error(
        "{path} belongs to the user with id {owner}, and atl runs as the user with id {user}: \
         atl does not touch the skill there"
    )]
    OtherOwner {
        /// The skill directory, or the one it would be made in.
        path: Utf8PathBuf,
        /// The user it belongs to.
        owner: u32,
        /// The user atl runs as.
        user: u32,
    },
    /// The skill directory is a symbolic link.
    #[error("{dir} is a symbolic link: atl neither writes through it nor replaces it")]
    DirectoryIsLink {
        /// The skill directory.
        dir: Utf8PathBuf,
    },
    /// Something that is no directory is in the place of the skill directory.
    #[error("{path} is not a directory: move it away first")]
    NotADirectory {
        /// What is in the way.
        path: Utf8PathBuf,
    },
    /// A directory with no manifest: atl did not install it.
    #[error(
        "{dir} was not installed by atl ({}): --force writes atl's files into it and keeps the rest",
        listed(files)
    )]
    Foreign {
        /// The skill directory.
        dir: Utf8PathBuf,
        /// Files in it.
        files: Vec<String>,
    },
    /// `remove` of a directory with no usable manifest.
    #[error("{dir} was not installed by atl ({why}): remove it yourself")]
    NotOurs {
        /// The skill directory.
        dir: Utf8PathBuf,
        /// What is wrong with the manifest.
        why: String,
    },
    /// The manifest is there and is none.
    #[error(
        "atl cannot tell which files of {dir} it wrote: its {MANIFEST} cannot be used, {reason} \
         (--force writes atl's files anew and keeps the rest)"
    )]
    DamagedManifest {
        /// The skill directory.
        dir: Utf8PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The manifest is of a schema this atl does not know.
    #[error(
        "the skill in {dir} was installed by a newer atl ({MANIFEST} has schema {schema}, \
         this atl knows {SCHEMA}): use that atl"
    )]
    NewerSchema {
        /// The skill directory.
        dir: Utf8PathBuf,
        /// The schema number in the file.
        schema: u64,
    },
    /// Files of atl's were edited, or others are where atl has one to write.
    #[error(
        "{dir} was changed since atl wrote it: {} ({hint})",
        changes(edited, colliding)
    )]
    Changed {
        /// The skill directory.
        dir: Utf8PathBuf,
        /// Files of atl's that differ from what it wrote.
        edited: Vec<String>,
        /// Files and links atl did not write, at a path it has a file for.
        colliding: Vec<String>,
        /// What `--force` would do.
        hint: &'static str,
    },
    /// A path of the skill goes through a link, or is a directory.
    #[error(
        "{dir} cannot hold the files of the skill ({}): atl neither goes through a link \
         nor replaces a directory",
        listed(paths)
    )]
    Blocked {
        /// The skill directory.
        dir: Utf8PathBuf,
        /// What is in the way, relative to it.
        paths: Vec<String>,
    },
    /// Another symbolic link is in the place of the link to the skill.
    #[error("{place} is a link to '{target}': --force replaces it")]
    OtherLink {
        /// Where the link goes.
        place: Utf8PathBuf,
        /// Where the link that is there leads.
        target: String,
    },
    /// A file or a directory is in the place of the link to the skill.
    #[error("{place} is not a link, and atl did not write it: move it away first")]
    LinkPlaceTaken {
        /// Where the link goes.
        place: Utf8PathBuf,
    },
}

/// `names` for a message: `a, b and 3 more`.
#[must_use]
pub fn listed(names: &[String]) -> String {
    listed_up_to(names, SHOWN_FILES)
}

/// [`listed`], with at most `max` names.
#[must_use]
pub fn listed_up_to(names: &[String], max: usize) -> String {
    let shown = names.len().min(max);
    let mut text = names[..shown].join(", ");
    if names.len() > shown {
        text.push_str(&format!(" and {} more", names.len() - shown));
    }
    text
}

/// `edited SKILL.md; not atl's: notes.md`.
fn changes(edited: &[String], colliding: &[String]) -> String {
    let mut parts = Vec::new();
    if !edited.is_empty() {
        parts.push(format!("edited {}", listed(edited)));
    }
    if !colliding.is_empty() {
        parts.push(format!("not atl's: {}", listed(colliding)));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn names(count: usize) -> Vec<String> {
        (0..count).map(|n| format!("f{n}")).collect()
    }

    #[test]
    fn a_long_list_is_cut() {
        assert_eq!(listed(&names(2)), "f0, f1");
        assert_eq!(listed(&names(8)), "f0, f1, f2, f3, f4, f5, f6, f7");
        assert_eq!(
            listed(&names(10)),
            "f0, f1, f2, f3, f4, f5, f6, f7 and 2 more"
        );
        assert_eq!(listed_up_to(&names(4), 3), "f0, f1, f2 and 1 more");
    }

    #[test]
    fn a_change_names_both_kinds_of_files() {
        let err = Refusal::Changed {
            dir: "/h/atl".into(),
            edited: vec!["SKILL.md".to_owned()],
            colliding: vec!["notes.md".to_owned()],
            hint: "why",
        };
        assert_eq!(
            err.to_string(),
            "/h/atl was changed since atl wrote it: edited SKILL.md; not atl's: notes.md (why)"
        );
    }

    #[test]
    fn a_file_error_is_one_line() {
        let err = FileError::Write {
            path: "/h/a\nb".into(),
            cause: std::io::Error::other("no\nway"),
        };
        assert_eq!(err.to_string(), "cannot write /h/a\\x0ab: no\\x0away");
        let err = FileError::Read {
            path: "/h/a".into(),
            cause: std::io::ErrorKind::NotFound.into(),
        };
        assert!(err.to_string().starts_with("cannot read /h/a: "), "{err}");
        assert!(std::error::Error::source(&err).is_none(), "said once");
    }
}
