//! The symbolic link through which Claude Code finds the skill: from its
//! `skills/` directory to the shared one.
//!
//! atl replaces only symbolic links there. It never writes a file or a
//! directory in that place, so one that is there is not atl's, `--force` or
//! not. Where there are no symbolic links (Windows) no link is wanted
//! ([`super::Target::link`] is `None`), and nothing here is reached.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};

use super::disk::{read_error, write_error};
use super::error::{FileError, Refusal, SkillError};
use super::place::{leads_nowhere, no_directory_above};

/// How many links on one path are followed: what systems follow at most.
const MAX_LINKS: u32 = 40;

/// Where the link goes, and where it leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeLink {
    /// The path of the link: `<claude>/skills/atl`.
    pub place: Utf8PathBuf,
    /// The directory [`Self::inside`] starts at: the home directory, or the
    /// project's.
    pub root: Utf8PathBuf,
    /// The skill directory, relative to [`Self::root`].
    pub inside: Utf8PathBuf,
}

/// What is in the place of the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    /// The link atl writes.
    Ours,
    /// Another link to the skill directory.
    Alike,
    /// The place *is* the skill directory: Claude Code's `skills/` and the
    /// shared one are the same directory (one is a link to the other).
    Shared,
    /// Nothing.
    Missing,
    /// A link that leads somewhere else, with where.
    Elsewhere(String),
    /// A file or a directory.
    Taken,
    /// Something that is no directory, above it: in the place of `skills/`.
    Blocked(Utf8PathBuf),
}

/// What [`ClaudeLink::make`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
    /// The skill was reachable already.
    Nothing,
    /// The link is new.
    Created,
    /// The link replaced another one.
    Replaced,
}

impl ClaudeLink {
    /// What is in the place of the link.
    ///
    /// # Errors
    ///
    /// [`FileError::Read`] when it cannot be told.
    pub fn state(&self) -> Result<LinkState, FileError> {
        if self.is_skill_directory() {
            return Ok(LinkState::Shared);
        }
        // One look: another atl may remove the link between two.
        let leads_to = match fs::read_link(&self.place) {
            Ok(leads_to) => leads_to,
            Err(err) => {
                return if leads_nowhere(&err) {
                    let above = no_directory_above(&self.place);
                    Ok(above.map_or(LinkState::Missing, LinkState::Blocked))
                } else if err.kind() == ErrorKind::InvalidInput {
                    Ok(LinkState::Taken) // no link
                } else {
                    Err(read_error(&self.place, err))
                };
            }
        };
        if leads_to == self.target()?.as_std_path() {
            return Ok(LinkState::Ours);
        }
        if self.leads_to_skill(&leads_to) {
            return Ok(LinkState::Alike);
        }
        Ok(LinkState::Elsewhere(
            leads_to.to_string_lossy().into_owned(),
        ))
    }

    /// Refuse what [`Self::make`] would refuse, before anything is written.
    ///
    /// # Errors
    ///
    /// [`Refusal::OtherLink`] without `force`, [`Refusal::LinkPlaceTaken`]
    /// and [`Refusal::NotADirectory`] with or without; [`FileError::Read`]
    /// when the place cannot be read.
    pub fn ensure_free(&self, force: bool) -> Result<(), SkillError> {
        match self.state()? {
            LinkState::Elsewhere(target) if !force => Err(Refusal::OtherLink {
                place: self.place.clone(),
                target,
            }
            .into()),
            LinkState::Taken => Err(Refusal::LinkPlaceTaken {
                place: self.place.clone(),
            }
            .into()),
            LinkState::Blocked(path) => Err(Refusal::NotADirectory { path }.into()),
            _ => Ok(()),
        }
    }

    /// Make the link, unless the skill is reachable already.
    ///
    /// # Errors
    ///
    /// As [`Self::ensure_free`]; [`FileError::Write`] when the link or its
    /// directory cannot be made.
    pub fn make(&self, force: bool) -> Result<Made, SkillError> {
        self.ensure_free(force)?;
        if let Some(parent) = self.place.parent() {
            fs::create_dir_all(parent).map_err(|err| write_error(parent, err))?;
        }
        match self.state()? {
            LinkState::Missing => {
                match symlink(&self.target()?, &self.place) {
                    // Another atl made it in the meantime.
                    Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                        self.ensure_free(false)?;
                        Ok(Made::Nothing)
                    }
                    Err(err) => Err(write_error(&self.place, err).into()),
                    Ok(()) => Ok(Made::Created),
                }
            }
            LinkState::Elsewhere(_) => {
                self.replace()
                    .map_err(|err| write_error(&self.place, err))?;
                Ok(Made::Replaced)
            }
            LinkState::Ours
            | LinkState::Alike
            | LinkState::Shared
            | LinkState::Taken
            | LinkState::Blocked(_) => Ok(Made::Nothing),
        }
    }

    /// Remove the link, when it is the one atl writes. `true` when it was.
    ///
    /// # Errors
    ///
    /// [`FileError`] when the place cannot be read or the link cannot be
    /// removed.
    pub fn remove(&self) -> Result<bool, FileError> {
        if self.state()? != LinkState::Ours {
            return Ok(false);
        }
        match fs::remove_file(&self.place) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(write_error(&self.place, err)),
        }
    }

    /// Whether the place of the link is the skill directory itself.
    fn is_skill_directory(&self) -> bool {
        is_one_place(
            self.place.as_std_path(),
            self.root.join(&self.inside).as_std_path(),
        )
    }

    /// Whether a link in the place of the link that holds `target` leads to
    /// the skill directory: to that very place, so that it does while there
    /// is none, too, or through other links to what is there.
    fn leads_to_skill(&self, target: &Path) -> bool {
        let skill = self.root.join(&self.inside);
        let from = self.place.parent().unwrap_or(&self.place);
        if fs::canonicalize(from)
            .is_ok_and(|from| is_one_place(&from.join(target), skill.as_std_path()))
        {
            return true;
        }
        match (fs::canonicalize(&self.place), fs::canonicalize(&skill)) {
            (Ok(there), Ok(skill)) => there == skill,
            _ => false,
        }
    }

    /// What the link holds: the skill directory, relative to the directory
    /// the link is in. That directory is resolved first — a system resolves
    /// `..` from where a link really is, not from the path it was reached
    /// by — and so is the root; the rest is as written, so a link above the
    /// skill directory can be pointed elsewhere.
    fn target(&self) -> Result<Utf8PathBuf, FileError> {
        let parent = self.place.parent().unwrap_or(&self.place);
        let from = parent
            .canonicalize_utf8()
            .map_err(|err| read_error(parent, err))?;
        let root = self
            .root
            .canonicalize_utf8()
            .map_err(|err| read_error(&self.root, err))?;
        Ok(relative(&from, &root.join(&self.inside)))
    }

    /// Put the link in the place of another one, atomically.
    fn replace(&self) -> std::io::Result<()> {
        let target = self.target().map_err(std::io::Error::other)?;
        let parent = self.place.parent().unwrap_or(&self.place);
        let made = tempfile::Builder::new().make_in(parent, |path| {
            let path = Utf8Path::from_path(path)
                .ok_or_else(|| std::io::Error::other("the path is not UTF-8"))?;
            symlink(&target, path)
        })?;
        made.into_temp_path().persist(&self.place)?;
        Ok(())
    }
}

/// Whether two paths are the same entry of the same directory, whether or not
/// there is one.
fn is_one_place(a: &Path, b: &Path) -> bool {
    match (a.parent(), b.parent()) {
        (Some(x), Some(y)) => a.file_name() == b.file_name() && is_one_directory(x, y),
        _ => false,
    }
}

/// Whether two absolute paths lead to the same directory, whether or not it
/// has been made yet.
fn is_one_directory(a: &Path, b: &Path) -> bool {
    if let (Ok(a), Ok(b)) = (fs::canonicalize(a), fs::canonicalize(b)) {
        return a == b;
    }
    leads_to(a, &mut { MAX_LINKS }) == leads_to(b, &mut { MAX_LINKS })
}

/// Where `path`, an absolute one, leads by the links there are, be there
/// anything or not. `links` is how many of them may still be followed.
fn leads_to(path: &Path, links: &mut u32) -> PathBuf {
    let mut at = PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir {
            at.pop();
            continue;
        }
        at.push(component);
        if *links > 0
            && let Ok(target) = fs::read_link(&at)
        {
            *links -= 1;
            at.pop();
            at = leads_to(&at.join(target), links);
        }
    }
    at
}

/// `to` as seen from the directory `from`; both absolute and normal.
fn relative(from: &Utf8Path, to: &Utf8Path) -> Utf8PathBuf {
    let from: Vec<Utf8Component<'_>> = from.components().collect();
    let to: Vec<Utf8Component<'_>> = to.components().collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    std::iter::repeat_n(Utf8Component::ParentDir, from.len() - shared)
        .chain(to[shared..].iter().copied())
        .collect()
}

#[cfg(unix)]
fn symlink(target: &Utf8Path, place: &Utf8Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, place)
}

#[cfg(not(unix))]
fn symlink(_target: &Utf8Path, _place: &Utf8Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atl makes no symbolic links on this platform",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_relative_path_goes_up_to_what_both_share() {
        for (from, to, expected) in [
            (
                "/h/.claude/skills",
                "/h/.agents/skills/atl",
                "../../.agents/skills/atl",
            ),
            ("/h", "/h/.agents", ".agents"),
            ("/a/b", "/c", "../../c"),
            ("/", "/c/d", "c/d"),
        ] {
            assert_eq!(relative(from.into(), to.into()), expected, "{from} -> {to}");
        }
    }

    /// The link atl makes, what it says of others, and what it removes.
    #[test]
    fn the_link_is_made_where_nothing_is_and_removed_when_it_is_atls() {
        use std::os::unix::fs::symlink as ln;
        let tmp = tempfile::tempdir().expect("tempdir");
        let root =
            Utf8PathBuf::from_path_buf(tmp.path().canonicalize().expect("real")).expect("UTF-8");
        fs::create_dir_all(root.join(".agents/skills/atl")).expect("mkdir");
        fs::create_dir(root.join(".claude")).expect("mkdir");
        let link = ClaudeLink {
            place: root.join(".claude/skills/atl"),
            root: root.clone(),
            inside: ".agents/skills/atl".into(),
        };
        assert_eq!(link.state().expect("state"), LinkState::Missing);
        assert_eq!(link.make(false).expect("made"), Made::Created);
        assert_eq!(
            fs::read_link(&link.place).expect("link"),
            Path::new("../../.agents/skills/atl")
        );
        assert_eq!(link.state().expect("state"), LinkState::Ours);
        assert_eq!(link.make(false).expect("made"), Made::Nothing);
        fs::remove_file(&link.place).expect("rm");
        ln(root.join(".agents/skills/atl"), &link.place).expect("ln");
        assert_eq!(link.state().expect("state"), LinkState::Alike);
        assert!(!link.remove().expect("removed"), "not atl's");
        fs::remove_file(&link.place).expect("rm");
        ln("/elsewhere", &link.place).expect("ln");
        assert_eq!(
            link.state().expect("state"),
            LinkState::Elsewhere("/elsewhere".to_owned())
        );
        assert!(matches!(
            link.make(false),
            Err(SkillError::Refused(Refusal::OtherLink { .. }))
        ));
        assert_eq!(link.make(true).expect("made"), Made::Replaced);
        assert!(link.remove().expect("removed"));
        fs::create_dir(&link.place).expect("mkdir");
        assert_eq!(link.state().expect("state"), LinkState::Taken);
        for force in [false, true] {
            assert!(matches!(
                link.make(force),
                Err(SkillError::Refused(Refusal::LinkPlaceTaken { .. }))
            ));
        }
        fs::remove_dir(&link.place).expect("rmdir");
        fs::remove_dir(root.join(".claude/skills")).expect("rmdir");
        fs::write(root.join(".claude/skills"), "").expect("write");
        assert_eq!(
            link.state().expect("state"),
            LinkState::Blocked(root.join(".claude/skills"))
        );
        // Claude Code's `skills/` is the shared one.
        fs::remove_file(root.join(".claude/skills")).expect("rm");
        ln(root.join(".agents/skills"), root.join(".claude/skills")).expect("ln");
        assert_eq!(link.state().expect("state"), LinkState::Shared);
    }
}
