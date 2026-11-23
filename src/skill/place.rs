//! The skill directory itself: what is in its place, whose it is, and the
//! lock on it.

use std::fs::{self, File, TryLockError};
use std::io::{self, ErrorKind};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};

use super::disk::{read_error, write_error};
use super::error::{FileError, Refusal, SkillError};

/// How long a command waits for another that holds the lock. Far longer than
/// any of them takes.
pub(super) const LOCK_WAIT: Duration = Duration::from_secs(10);

/// The longest pause between two looks at a lock another atl holds.
const LOCK_PAUSE: Duration = Duration::from_millis(50);

/// How often a command starts over because another atl removed or made the
/// directory under its hands. Every time is a command of another that went
/// through, so this is far more than there ever are.
pub(super) const ATTEMPTS: usize = 1000;

/// What is at the path of a skill directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// Nothing.
    Absent,
    /// A symbolic link, wherever it leads.
    Link,
    /// Something that is neither, here: in this place, or above it, where a
    /// directory would have to be.
    Other(Utf8PathBuf),
    /// A directory.
    Directory,
}

/// What is at `dir` itself; a link is not followed.
///
/// # Errors
///
/// [`FileError::Read`] when it cannot be told.
pub fn place(dir: &Utf8Path) -> Result<Place, FileError> {
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => Ok(Place::Link),
        Ok(meta) if meta.is_dir() => Ok(Place::Directory),
        Ok(_) => Ok(Place::Other(dir.to_path_buf())),
        Err(err) if leads_nowhere(&err) => {
            Ok(no_directory_above(dir).map_or(Place::Absent, Place::Other))
        }
        Err(err) => Err(read_error(dir, err)),
    }
}

/// The nearest path above `path` at which there is something, when that is no
/// directory and leads to none.
pub(super) fn no_directory_above(path: &Utf8Path) -> Option<Utf8PathBuf> {
    let nearest = path
        .ancestors()
        .skip(1)
        .find(|at| fs::symlink_metadata(at).is_ok())?;
    let in_the_way = match fs::metadata(nearest) {
        Ok(meta) => !meta.is_dir(),
        Err(err) => leads_nowhere(&err), // a link to nothing
    };
    in_the_way.then(|| nearest.to_path_buf())
}

/// Whether `err` says that there is nothing at a path, for something on the
/// way there is missing, no directory, or a link to itself.
pub(super) fn leads_nowhere(err: &io::Error) -> bool {
    #[cfg(unix)]
    if err.raw_os_error() == Some(libc::ELOOP) {
        return true;
    }
    matches!(err.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

/// Refuse a skill directory that is a link or no directory, or cannot be
/// one; say which of the other two it is.
pub(super) fn ensure_directory_or_absent(dir: &Utf8Path) -> Result<Place, SkillError> {
    match place(dir)? {
        Place::Link => Err(Refusal::DirectoryIsLink { dir: dir.into() }.into()),
        Place::Other(path) => Err(Refusal::NotADirectory { path }.into()),
        found => Ok(found),
    }
}

/// What tells one file or directory from another; a link is not followed.
#[cfg(unix)]
pub(super) fn identity(path: &Utf8Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(path).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// What tells one file or directory from another: the path the system has
/// for it, which is spelled as the directories have it.
#[cfg(not(unix))]
pub(super) fn identity(path: &Utf8Path) -> Option<std::path::PathBuf> {
    fs::canonicalize(path).ok()
}

/// Whether the directory `dir` was opened from is the one that is at `path`
/// now: not removed, moved or replaced since.
#[cfg(unix)]
fn is_at(dir: &File, path: &Utf8Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    dir.metadata()
        .is_ok_and(|meta| identity(path) == Some((meta.dev(), meta.ino())))
}

#[cfg(not(unix))]
fn is_at(_dir: &File, path: &Utf8Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

/// Open the directory at `path`, to lock it.
fn open_dir(path: &Utf8Path) -> io::Result<File> {
    let mut options = File::options();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        /// `FILE_FLAG_BACKUP_SEMANTICS`: without it a directory is no file
        /// that can be opened.
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        options.custom_flags(BACKUP_SEMANTICS);
    }
    options.open(path)
}

/// The user a file belongs to, or what a link leads to.
#[cfg(unix)]
pub(super) fn owner(path: &Utf8Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|meta| meta.uid())
}

#[cfg(not(unix))]
pub(super) fn owner(_path: &Utf8Path) -> Option<u32> {
    None
}

/// Whom the skill directory `dir` belongs to, when that is not `user`, with
/// the path that does. Where there is none yet, what matters is the directory
/// it would be made in, and that only as root: whoever else may write there
/// makes a directory of their own.
pub(super) fn other_owner(dir: &Utf8Path, user: u32) -> Option<(Utf8PathBuf, u32)> {
    let at = if owner(dir).is_some() {
        dir
    } else if user == 0 {
        dir.ancestors().skip(1).find(|at| owner(at).is_some())?
    } else {
        return None;
    };
    owner(at)
        .filter(|owner| *owner != user)
        .map(|owner| (at.to_path_buf(), owner))
}

/// What [`lock`] came to.
#[derive(Debug)]
pub(super) enum Locked {
    /// The directory is there, and locked until this is dropped. `None`
    /// where the file system (or the platform) has no locks on directories:
    /// the writes are atomic file by file all the same.
    Directory(Option<File>),
    /// There is none.
    Absent,
}

/// Lock the skill directory, waiting for the atl that holds it until
/// `deadline`. One that was removed or replaced while this waited is not the
/// directory any more: the one that is there by then is.
///
/// # Errors
///
/// As [`ensure_directory_or_absent`]; [`FileError::Write`] when another atl
/// holds the lock still.
pub(super) fn lock(dir: &Utf8Path, deadline: Instant) -> Result<Locked, SkillError> {
    let unlocked = |why: &dyn std::fmt::Display| {
        tracing::debug!("{dir} is not locked: {why}");
        Ok(Locked::Directory(None))
    };
    for _ in 0..ATTEMPTS {
        if ensure_directory_or_absent(dir)? == Place::Absent {
            return Ok(Locked::Absent);
        }
        match lock_once(dir, |file| take(file, deadline)) {
            Ok(Some(file)) => return Ok(Locked::Directory(Some(file))),
            Ok(None) => {}
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                let busy = io::Error::new(err.kind(), "another atl is writing the skill");
                return Err(write_error(dir, busy).into());
            }
            Err(err) => return unlocked(&err),
        }
    }
    unlocked(&"it is another directory each time")
}

/// Take the lock on `file`, waiting until `deadline` for who holds it.
fn take(file: &File, deadline: Instant) -> io::Result<()> {
    let mut pause = Duration::from_millis(1);
    loop {
        match file.try_lock() {
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(LOCK_PAUSE);
            }
            taken => return Ok(taken?),
        }
    }
}

/// Lock, by `take`, the directory that is at `dir` now. `None` when there is
/// none, or not that one any more once the lock was got.
fn lock_once(
    dir: &Utf8Path,
    take: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<Option<File>> {
    let file = match open_dir(dir) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    take(&file)?;
    Ok(is_at(&file, dir).then_some(file))
}

/// Make the skill directory, if there is none, and [`lock`] it.
///
/// # Errors
///
/// As [`lock`]; [`FileError::Write`] when it cannot be made.
pub(super) fn make_and_lock(dir: &Utf8Path, deadline: Instant) -> Result<Option<File>, SkillError> {
    let mut why = io::Error::other("it is removed each time it is made");
    for _ in 0..ATTEMPTS {
        match fs::create_dir_all(dir) {
            Ok(()) => {}
            // There, then gone: another atl removed it in between. Or there
            // and no directory, which `lock` refuses.
            Err(err) if matches!(err.kind(), ErrorKind::AlreadyExists | ErrorKind::NotFound) => {
                why = err;
            }
            Err(err) => return Err(write_error(dir, err).into()),
        }
        if let Locked::Directory(lock) = lock(dir, deadline)? {
            return Ok(lock);
        }
    }
    Err(write_error(dir, why).into())
}

/// Lock the skill directory without waiting: `None` when another atl holds
/// it or has just removed it. Where directories cannot be locked, the lock
/// is no file, and the writes go on without.
///
/// # Errors
///
/// [`FileError::Read`] when the directory cannot be opened.
pub(super) fn try_lock(dir: &Utf8Path) -> Result<Option<Locked>, FileError> {
    match lock_once(dir, |file| Ok(file.try_lock()?)) {
        Ok(Some(file)) => Ok(Some(Locked::Directory(Some(file)))),
        Ok(None) => Ok(None),
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(None),
        // Opened, and not lockable: a platform or file system without locks.
        Err(err) if open_dir(dir).is_ok() => {
            tracing::debug!("{dir} is not locked: {err}");
            Ok(Some(Locked::Directory(None)))
        }
        Err(err) => Err(read_error(dir, err)),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn root() -> (tempfile::TempDir, Utf8PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("UTF-8");
        (tmp, root)
    }

    fn in_a_minute() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    #[test]
    fn a_directory_that_was_moved_or_replaced_is_not_at_its_path() {
        let (_tmp, root) = root();
        let (at, aside) = (root.join("atl"), root.join("aside"));
        fs::create_dir(&at).expect("mkdir");
        let dir = File::open(&at).expect("open");
        assert!(is_at(&dir, &at));
        fs::rename(&at, &aside).expect("mv");
        assert!(!is_at(&dir, &at), "nothing is");
        fs::create_dir(&at).expect("mkdir");
        assert!(!is_at(&dir, &at), "another is");
        fs::remove_dir(&at).expect("rmdir");
        std::os::unix::fs::symlink(&aside, &at).expect("symlink");
        assert!(!is_at(&dir, &at), "a link to it is");
        assert!(is_at(&dir, &aside));
    }

    /// What comes into the place, or goes from it, between the look at it
    /// and the lock.
    #[test]
    fn a_lock_is_of_what_is_there_by_then() {
        let (_tmp, root) = root();
        let dir = root.join(".agents/skills/atl");
        assert!(matches!(lock(&dir, in_a_minute()), Ok(Locked::Absent)));
        assert!(matches!(lock_once(&dir, File::lock), Ok(None)));
        assert!(matches!(try_lock(&dir), Ok(None)));
        fs::create_dir_all(dir.parent().expect("parent")).expect("mkdir");
        fs::write(&dir, "theirs").expect("write");
        assert!(matches!(
            make_and_lock(&dir, in_a_minute()),
            Err(SkillError::Refused(Refusal::NotADirectory { path })) if path == dir
        ));
        fs::remove_file(&dir).expect("rm");
        let held = make_and_lock(&dir, in_a_minute()).expect("made");
        assert!(held.is_some() && dir.is_dir());
        assert!(matches!(try_lock(&dir), Ok(None)), "held");
        // Gone, and another in its place, while this one is waited for.
        let replace = |file: &File| {
            fs::rename(&dir, root.join("gone"))?;
            fs::create_dir(&dir)?;
            drop(held);
            file.lock()
        };
        assert!(matches!(lock_once(&dir, replace), Ok(None)));
        assert!(matches!(
            try_lock(&dir),
            Ok(Some(Locked::Directory(Some(_))))
        ));
    }

    /// A lock that is held for longer than a command waits is a write
    /// failure, not a wait without an end.
    #[test]
    fn a_lock_is_not_waited_for_for_ever() {
        let (_tmp, root) = root();
        let held = make_and_lock(&root.join("atl"), in_a_minute()).expect("locked");
        let dir = root.join("atl");
        let start = Instant::now();
        for result in [
            lock(&dir, start + Duration::from_millis(100)).map(|_| ()),
            make_and_lock(&dir, start).map(|_| ()),
        ] {
            let err = result.expect_err("held");
            assert!(
                matches!(err, SkillError::File(FileError::Write { .. })),
                "{err:?}"
            );
            assert_eq!(
                err.to_string(),
                format!("cannot write {dir}: another atl is writing the skill")
            );
        }
        assert!(start.elapsed() >= Duration::from_millis(100));
        drop(held);
        assert!(matches!(
            lock(&dir, in_a_minute()),
            Ok(Locked::Directory(Some(_)))
        ));
    }

    #[test]
    fn whose_a_skill_directory_is() {
        let (tmp, root) = root();
        let user = super::super::testing::not_roots(tmp.path());
        let dir = root.join(".agents/skills/atl");
        // Not there: only root minds.
        assert_eq!(other_owner(&dir, user), None);
        assert_eq!(other_owner(&dir, user + 1), None);
        assert_eq!(other_owner(&dir, 0), Some((root.clone(), user)));
        fs::create_dir_all(&dir).expect("mkdir");
        super::super::testing::not_roots(dir.as_std_path());
        assert_eq!(other_owner(&dir, user), None);
        assert_eq!(other_owner(&dir, user + 1), Some((dir.clone(), user)));
        assert_eq!(other_owner(&dir, 0), Some((dir, user)));
        // Not there, and above it a link into another's directory.
        let home = root.join("home");
        fs::create_dir(&home).expect("mkdir");
        std::os::unix::fs::symlink(root.join(".agents/skills/atl"), home.join(".agents"))
            .expect("symlink");
        assert_eq!(
            other_owner(&home.join(".agents/skills/atl"), 0),
            Some((home.join(".agents"), user))
        );
    }

    #[test]
    fn what_is_in_the_place_of_the_directory() {
        let (_tmp, root) = root();
        let at = |path: &str| place(&root.join(path)).expect("place");
        assert_eq!(at("none"), Place::Absent);
        fs::write(root.join("file"), "").expect("write");
        assert_eq!(at("file/atl"), Place::Other(root.join("file")));
        assert_eq!(at("file"), Place::Other(root.join("file")));
        std::os::unix::fs::symlink(root.join("none"), root.join("dangling")).expect("symlink");
        assert_eq!(at("dangling"), Place::Link);
        assert_eq!(at("dangling/atl"), Place::Other(root.join("dangling")));
        assert_eq!(place(&root).expect("place"), Place::Directory);
    }
}
