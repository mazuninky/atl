//! The files in a skill directory: what is at a path, and putting another
//! file there or deleting it.
//!
//! A write or a deletion is bound to what was seen when the command decided
//! ([`Seen`]): what is there is read once more right before, and if it is
//! something else, it is someone's work and stays. From that read on
//! [`super::atomic`] binds it to the very file, up to the instant before the
//! rename.
//!
//! No link is followed below the skill directory, at a file or above it. A
//! file is read as no link, and what was read so is never written through
//! one; a new one takes no one's place; and the directories on the way are
//! looked at last of all.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;

use camino::{Utf8Path, Utf8PathBuf};

use super::atomic::{self, Original};
use super::error::FileError;
use super::manifest::sha256_hex;
use super::place::identity;
use super::survey::{files_under, spelling};

/// The largest file read. atl's own files are far smaller; a larger one in
/// their place was not written by atl.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// How often a change is tried that is overtaken by another each time.
const ATTEMPTS: usize = 3;

/// What a path of the skill holds.
#[derive(Debug)]
pub(super) enum Found {
    /// Nothing.
    Missing,
    /// A regular file, with what is in it.
    File(Vec<u8>, Original),
    /// A regular file larger than was asked for.
    Large,
    /// A link, a device.
    Other,
    /// A directory, or a path that goes through something that is none.
    Blocked(String),
}

/// What a path held when it was looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Seen {
    /// Nothing.
    Missing,
    /// A regular file with this SHA-256.
    File(String),
    /// Anything else.
    Other,
}

impl Found {
    /// What is kept of it.
    pub(super) fn seen(&self) -> Seen {
        match self {
            Self::Missing => Seen::Missing,
            Self::File(bytes, _) => Seen::File(sha256_hex(bytes)),
            Self::Large | Self::Other | Self::Blocked(_) => Seen::Other,
        }
    }

    /// What a change to the file that was found is bound to.
    fn original(&self) -> Option<&Original> {
        match self {
            Self::File(_, original) => Some(original),
            Self::Missing | Self::Large | Self::Other | Self::Blocked(_) => None,
        }
    }
}

/// What a write or a deletion came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Done {
    /// It was done.
    Yes,
    /// There was nothing to do.
    No,
    /// It was not done: what is there is not what was seen.
    Changed,
}

/// What is at `path`, a plain relative path, in `dir`. No link is followed
/// below `dir`.
///
/// # Errors
///
/// [`FileError::Read`] when it cannot be told, or not be read.
pub(super) fn find(dir: &Utf8Path, path: &str) -> Result<Found, FileError> {
    find_up_to(dir, path, MAX_FILE_BYTES)
}

/// [`find`], of a file of at most `max` bytes.
///
/// # Errors
///
/// As [`find`].
pub(super) fn find_up_to(dir: &Utf8Path, path: &str, max: u64) -> Result<Found, FileError> {
    let mut at = Utf8PathBuf::new();
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        at.push(component);
        let here = dir.join(&at);
        let meta = match fs::symlink_metadata(&here) {
            Ok(meta) => meta,
            // Where no file can have such a name, there is none.
            Err(err) if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::InvalidFilename) => {
                return Ok(Found::Missing);
            }
            Err(err) => return Err(read_error(&here, err)),
        };
        let last = components.peek().is_none();
        if meta.is_dir() == last {
            return Ok(Found::Blocked(components_of(&at)));
        }
        if last {
            if !meta.is_file() {
                return Ok(Found::Other);
            }
            if meta.len() > max {
                return Ok(Found::Large);
            }
            return content(&here, max);
        }
    }
    Ok(Found::Missing)
}

/// `at`, a relative path, with `/` between its components on any platform.
fn components_of(at: &Utf8Path) -> String {
    at.components()
        .map(|component| component.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// The file at `path`, which was a regular one of at most `max` bytes a moment
/// ago.
fn content(path: &Utf8Path, max: u64) -> Result<Found, FileError> {
    match atomic::read_no_link(path) {
        Ok((bytes, _)) if bytes.len() as u64 > max => Ok(Found::Large),
        Ok((bytes, original)) => Ok(Found::File(bytes, original)),
        Err(FileError::Read { cause, .. }) if cause.kind() == ErrorKind::NotFound => {
            Ok(Found::Missing)
        }
        // A link, or something else that is no file, by now.
        Err(_) if fs::symlink_metadata(path).is_ok_and(|meta| !meta.is_file()) => Ok(Found::Other),
        Err(err) => Err(err),
    }
}

/// Put `bytes` at `path` in `dir`. A file that holds them already is not
/// touched. With `seen`, only over what was seen; without, over anything but
/// a directory.
///
/// A file keeps its access, a new one has that of any new file. What is no
/// regular file, or too large to be read, is taken out first.
///
/// # Errors
///
/// [`FileError`] when what is there cannot be read, or the file not be
/// written, or a link or a directory is in the way.
pub(super) fn write(
    dir: &Utf8Path,
    path: &str,
    bytes: &[u8],
    seen: Option<&Seen>,
) -> Result<Done, FileError> {
    let is_done = |found: &Found| matches!(found, Found::File(there, _) if there == bytes);
    change(dir, path, seen, is_done, |found| {
        let at = dir.join(path);
        if let Found::Large | Found::Other = found {
            unlink(dir, path, None)?;
        }
        if let Some(parent) = at.parent() {
            fs::create_dir_all(parent).map_err(|err| write_error(parent, err))?;
        }
        put(dir, path, bytes, found.original())
    })
}

/// Delete the file (or link) at `path` in `dir`, and the directories above
/// it, up to `dir`, that this leaves empty. With `seen`, only what was seen;
/// without, anything but a directory.
///
/// # Errors
///
/// [`FileError`] when what is there cannot be read, or not be deleted, or a
/// link or a directory is in the way.
pub(super) fn delete(dir: &Utf8Path, path: &str, seen: Option<&Seen>) -> Result<Done, FileError> {
    let is_done = |found: &Found| matches!(found, Found::Missing);
    let done = change(dir, path, seen, is_done, |found| {
        unlink(dir, path, found.original())
    })?;
    let mut parent = Utf8Path::new(path).parent();
    while done == Done::Yes
        && let Some(relative) = parent.filter(|p| !p.as_str().is_empty())
    {
        if !remove_empty(dir, relative.as_str())? {
            break;
        }
        parent = relative.parent();
    }
    Ok(done)
}

/// Delete the regular file at `path` in `dir`, what a write of atl's that was
/// cut short left there, and the directories above it, up to `dir`, that
/// this leaves empty. Whether it was deleted: a link, a directory, a file
/// larger than any of atl's, or one that changes meanwhile stays.
///
/// # Errors
///
/// [`FileError`] when it cannot be read or deleted.
pub(super) fn delete_left_over(dir: &Utf8Path, path: &str) -> Result<bool, FileError> {
    let Found::File(_, original) = find(dir, path)? else {
        return Ok(false);
    };
    match unlink(dir, path, Some(&original)) {
        Ok(()) => {}
        Err(err) if atomic::is_changed(&err) => return Ok(false),
        Err(err) => return Err(err),
    }
    let mut parent = Utf8Path::new(path).parent();
    while let Some(relative) = parent.filter(|p| !p.as_str().is_empty()) {
        if !remove_empty(dir, relative.as_str())? {
            break;
        }
        parent = relative.parent();
    }
    Ok(true)
}

/// Make a change to what is at `path` in `dir`, unless it `is_done` already:
/// only to what was `seen`, if that is given. When another overtakes `act`,
/// what is there by then is looked at once more.
fn change(
    dir: &Utf8Path,
    path: &str,
    seen: Option<&Seen>,
    is_done: impl Fn(&Found) -> bool,
    act: impl Fn(&Found) -> Result<(), FileError>,
) -> Result<Done, FileError> {
    for _ in 0..ATTEMPTS {
        let found = find(dir, path)?;
        if is_done(&found) {
            return Ok(Done::No);
        }
        if matches!(found, Found::Blocked(_)) {
            return Err(in_the_way(&dir.join(path)));
        }
        let was = found.seen();
        if seen.is_some_and(|seen| *seen != was) {
            return Ok(Done::Changed);
        }
        match act(&found) {
            Ok(()) => return Ok(Done::Yes),
            Err(err) if atomic::is_changed(&err) => {}
            // As good as that: the error is of what has come there since.
            Err(_) if find(dir, path).is_ok_and(|now| now.seen() != was) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(Done::Changed)
}

/// Write `bytes` to `path` in `dir`, over the file that `over` was read from,
/// or where nothing is; not through a link on the way there.
fn put(dir: &Utf8Path, path: &str, bytes: &[u8], over: Option<&Original>) -> Result<(), FileError> {
    ensure_plain_way(dir, path)?;
    let at = dir.join(path);
    match over {
        Some(over) => atomic::rewrite(&at, over, bytes).map(|_| ()),
        None => atomic::create(&at, bytes),
    }
}

/// Delete what is at `path` in `dir`: the file that `original` was read from,
/// or what is none (of a link, the link); not through a link on the way there.
fn unlink(dir: &Utf8Path, path: &str, original: Option<&Original>) -> Result<(), FileError> {
    ensure_plain_way(dir, path)?;
    let at = dir.join(path);
    atomic::check_remove(&at)?;
    match original {
        Some(original) => atomic::remove(&at, original),
        None => fs::remove_file(&at).map_err(|err| write_error(&at, err)),
    }
}

/// Remove the directory at `path` in `dir`, if it is empty and no link is on
/// the way there. Whether it is gone.
fn remove_empty(dir: &Utf8Path, path: &str) -> Result<bool, FileError> {
    ensure_plain_way(dir, path)?;
    remove_if_empty(&dir.join(path))
}

/// Remove a directory, if it is empty. Whether it is gone.
pub(super) fn remove_if_empty(dir: &Utf8Path) -> Result<bool, FileError> {
    match fs::remove_dir(dir) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(true),
        Err(err) if err.kind() == ErrorKind::DirectoryNotEmpty => Ok(false),
        Err(err) => Err(write_error(dir, err)),
    }
}

/// Give what is found at `path` in `dir` under another spelling the one of
/// `path`: each directory on the way, then the file. Whether that was done.
/// A directory keeps its spelling when there is a file in it that is none of
/// `ours`, the paths of atl's files.
///
/// Before the file is written, so that a run that is cut short leaves the
/// file under the old name, or under the new one as it was: either is
/// recognised by the next.
///
/// # Errors
///
/// [`FileError`] when a directory cannot be read, or not be renamed.
pub(super) fn respell(dir: &Utf8Path, path: &str, ours: &[&str]) -> Result<bool, FileError> {
    let there = spelling(dir, path)?;
    if there == path {
        return Ok(false);
    }
    let mut parent = dir.to_path_buf();
    let mut names = there.split('/').zip(path.split('/')).peekable();
    while let Some((name, wanted)) = names.next() {
        let (from, to) = (parent.join(name), parent.join(wanted));
        let is_file = names.peek().is_none();
        // Anything else is the same file by a name that is not its own.
        if name != wanted
            && name.eq_ignore_ascii_case(wanted)
            && (is_file || holds_only(&from, dir, ours)?)
        {
            rename_case(&from, &to).map_err(|err| write_error(&to, err))?;
        }
        parent = to;
    }
    // Some file systems take that for a rename of a file to itself.
    Ok(spelling(dir, path)? != there)
}

/// Rename `from` to `to`, which differs from it in case only.
#[cfg(not(windows))]
fn rename_case(from: &Utf8Path, to: &Utf8Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

/// Rename `from` to `to`, which differs from it in case only, by a name of
/// its own in between: a rename between two spellings of one name may be
/// taken for one to itself, and do nothing. Back to `from` when the second
/// step fails.
#[cfg(windows)]
fn rename_case(from: &Utf8Path, to: &Utf8Path) -> std::io::Result<()> {
    let mut between = from.as_str().to_owned();
    between.push_str(".atl-respell");
    fs::rename(from, &between)?;
    fs::rename(&between, to).inspect_err(|_| {
        let _ = fs::rename(&between, from);
    })
}

/// Whether every file under `parent` is one of `ours`, paths in `dir`.
fn holds_only(parent: &Utf8Path, dir: &Utf8Path, ours: &[&str]) -> Result<bool, FileError> {
    let ours: BTreeSet<_> = ours
        .iter()
        .filter_map(|path| identity(&dir.join(path)))
        .collect();
    Ok(files_under(parent)?
        .iter()
        .all(|file| identity(&parent.join(file)).is_some_and(|id| ours.contains(&id))))
}

/// That every directory on the way to `path` in `dir` is one, and no link.
fn ensure_plain_way(dir: &Utf8Path, path: &str) -> Result<(), FileError> {
    let at = dir.join(path);
    for above in at.ancestors().skip(1) {
        if !fs::symlink_metadata(above).is_ok_and(|meta| meta.is_dir()) {
            return Err(in_the_way(&at));
        }
        if above == dir {
            break;
        }
    }
    Ok(())
}

/// [`FileError::Write`] for a path that cannot be written to.
fn in_the_way(at: &Utf8Path) -> FileError {
    let err = std::io::Error::other("a link or a directory is in the way");
    write_error(at, err)
}

/// [`FileError::Read`] for `path`.
pub(super) fn read_error(path: &Utf8Path, cause: std::io::Error) -> FileError {
    FileError::Read {
        path: path.to_path_buf(),
        cause,
    }
}

/// [`FileError::Write`] for `path`.
pub(super) fn write_error(path: &Utf8Path, cause: std::io::Error) -> FileError {
    FileError::Write {
        path: path.to_path_buf(),
        cause,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::symlink;

    fn dir() -> (tempfile::TempDir, Utf8PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("UTF-8");
        let dir = root.join("atl");
        fs::create_dir_all(dir.join("references")).expect("mkdir");
        (tmp, dir)
    }

    fn file(text: &str) -> Seen {
        Seen::File(sha256_hex(text.as_bytes()))
    }

    /// What was there when the command decided, what is there now, and
    /// whether that is replaced and deleted.
    #[test]
    fn a_change_is_bound_to_what_was_seen() {
        let (_tmp, dir) = dir();
        let path = "references/a.md";
        let at = dir.join(path);
        let set = |text: Option<&str>| {
            let _ = fs::remove_file(&at);
            fs::create_dir_all(dir.join("references")).expect("mkdir");
            if let Some(text) = text {
                fs::write(&at, text).expect("write");
            }
        };
        let read = || fs::read_to_string(&at).ok();
        for (seen, now, written, deleted) in [
            (Some(file("old")), Some("old"), Done::Yes, Done::Yes),
            (
                Some(file("old")),
                Some("mine"),
                Done::Changed,
                Done::Changed,
            ),
            (Some(file("old")), Some("new"), Done::No, Done::Changed),
            (Some(file("old")), None, Done::Changed, Done::No),
            (Some(Seen::Missing), None, Done::Yes, Done::No),
            (
                Some(Seen::Missing),
                Some("mine"),
                Done::Changed,
                Done::Changed,
            ),
            (Some(Seen::Missing), Some("new"), Done::No, Done::Changed),
            (
                Some(Seen::Other),
                Some("mine"),
                Done::Changed,
                Done::Changed,
            ),
            (None, Some("mine"), Done::Yes, Done::Yes),
            (None, None, Done::Yes, Done::No),
        ] {
            set(now);
            let done = write(&dir, path, b"new", seen.as_ref()).expect("write");
            assert_eq!(done, written, "{seen:?}, {now:?}");
            let expected = if done == Done::Yes { Some("new") } else { now };
            assert_eq!(read().as_deref(), expected, "{seen:?}, {now:?}");
            set(now);
            let done = delete(&dir, path, seen.as_ref()).expect("delete");
            assert_eq!(done, deleted, "{seen:?}, {now:?}");
            let expected = if done == Done::Yes { None } else { now };
            assert_eq!(read().as_deref(), expected, "{seen:?}, {now:?}");
        }
    }

    /// A link in the place of a directory, put there after the command
    /// decided: nothing is written, deleted, renamed or removed behind it.
    #[test]
    fn nothing_is_changed_through_a_link() {
        let (_tmp, dir) = dir();
        let outside = dir.with_file_name("outside");
        fs::create_dir(&outside).expect("mkdir");
        fs::write(outside.join("a.md"), "old").expect("write");
        fs::remove_dir(dir.join("references")).expect("rmdir");
        symlink(&outside, dir.join("references")).expect("symlink");
        let is_in_the_way = |result: Result<(), FileError>| {
            let err = result.expect_err("in the way");
            assert!(
                matches!(&err, FileError::Write { .. })
                    && err
                        .to_string()
                        .contains("a link or a directory is in the way"),
                "{err}"
            );
        };
        for seen in [Some(file("old")), None] {
            is_in_the_way(write(&dir, "references/a.md", b"new", seen.as_ref()).map(|_| ()));
            is_in_the_way(write(&dir, "references/b.md", b"new", seen.as_ref()).map(|_| ()));
            is_in_the_way(delete(&dir, "references/a.md", seen.as_ref()).map(|_| ()));
        }
        // The last look of all, which only the change itself comes after.
        fs::create_dir(outside.join("empty")).expect("mkdir");
        let (_, original) = atomic::read_no_link(&outside.join("a.md")).expect("read");
        for original in [None, Some(&original)] {
            is_in_the_way(put(&dir, "references/a.md", b"new", original));
            is_in_the_way(unlink(&dir, "references/a.md", original));
        }
        is_in_the_way(remove_empty(&dir, "references/empty").map(|_| ()));
        assert!(outside.join("empty").is_dir());
        is_in_the_way(put(&dir, "references/empty/a.md", b"new", None));
        assert!(!outside.join("empty/a.md").exists(), "further up the way");
        ensure_plain_way(&dir, "SKILL.md").expect("no link on the way");
        let moved = dir.with_file_name("moved");
        fs::rename(&dir, &moved).expect("mv");
        symlink(&moved, &dir).expect("symlink");
        is_in_the_way(ensure_plain_way(&dir, "SKILL.md"));
        assert_eq!(
            fs::read_to_string(outside.join("a.md")).expect("read"),
            "old"
        );
        assert!(!outside.join("b.md").exists());
    }

    /// What others do to a file between atl's last look at it and its
    /// change: what is there by then stays, and no link is followed. `act` is
    /// overtaken by `other`.
    #[test]
    fn a_change_that_is_overtaken_is_none() {
        type Other = fn(&Utf8Path, &Utf8Path);
        let save: Other = |at, _| fs::write(at, "mine, of another size").expect("write");
        let replace: Other = |at, outside| {
            fs::write(outside.join("new"), "mine").expect("write");
            fs::rename(outside.join("new"), at).expect("mv");
        };
        let link: Other = |at, outside| {
            let _ = fs::remove_file(at);
            symlink(outside.join("a.md"), at).expect("symlink");
        };
        let link_to_itself_moved: Other = |at, outside| {
            fs::rename(at, outside.join("a.md")).expect("mv");
            symlink(outside.join("a.md"), at).expect("symlink");
        };
        let dangling: Other = |at, outside| {
            let _ = fs::remove_file(at);
            symlink(outside.join("none"), at).expect("symlink");
        };
        let remove: Other = |at, _| fs::remove_file(at).expect("rm");
        for (n, (was, other, written, deleted)) in [
            (Some("old"), save, Done::Changed, Done::Changed),
            (Some("old"), replace, Done::Changed, Done::Changed),
            (Some("old"), link, Done::Changed, Done::Changed),
            (
                Some("old"),
                link_to_itself_moved,
                Done::Changed,
                Done::Changed,
            ),
            (Some("old"), dangling, Done::Changed, Done::Changed),
            (Some("old"), remove, Done::Changed, Done::No),
            (None, save, Done::Changed, Done::Changed),
            (None, replace, Done::Changed, Done::Changed),
            (None, link, Done::Changed, Done::Changed),
            (None, dangling, Done::Changed, Done::Changed),
        ]
        .into_iter()
        .enumerate()
        {
            for delete in [false, true] {
                if delete && was.is_none() {
                    continue;
                }
                let (_tmp, dir) = dir();
                let outside = dir.with_file_name("outside");
                fs::create_dir(&outside).expect("mkdir");
                fs::write(outside.join("a.md"), "old").expect("write");
                let at = dir.join("a.md");
                if let Some(was) = was {
                    fs::write(&at, was).expect("write");
                }
                let is_done = |found: &Found| delete && matches!(found, Found::Missing);
                let seen = was.map_or(Seen::Missing, file);
                let done = change(&dir, "a.md", Some(&seen), is_done, |found| {
                    other(&at, &outside);
                    if delete {
                        unlink(&dir, "a.md", found.original())
                    } else {
                        put(&dir, "a.md", b"new", found.original())
                    }
                });
                let expected = if delete { deleted } else { written };
                assert_eq!(done.expect("no error"), expected, "{n}, {delete}");
                let there = fs::symlink_metadata(&at).is_ok();
                assert_eq!(there, deleted != Done::No, "{n}, {delete}");
                assert_ne!(fs::read(&at).ok().as_deref(), Some(&b"new"[..]), "{n}");
                let kept = fs::read_to_string(outside.join("a.md")).expect("read");
                assert_eq!(kept, "old", "{n}, {delete}: changed behind a link");
                assert!(!outside.join("none").exists(), "{n}");
            }
        }
    }

    /// An error that has nothing to do with another is one, and one that is
    /// overtaken each time gives up.
    #[test]
    fn a_change_is_tried_again_only_when_it_was_overtaken() {
        let (_tmp, dir) = dir();
        let at = dir.join("a.md");
        fs::write(&at, "old").expect("write");
        let tries = std::cell::Cell::new(0);
        let count = || tries.replace(tries.get() + 1);
        let failed = change(
            &dir,
            "a.md",
            None,
            |_| false,
            |_| {
                count();
                Err(in_the_way(&at))
            },
        );
        assert!(failed.is_err());
        assert_eq!(tries.replace(0), 1);
        let overtaken = change(
            &dir,
            "a.md",
            None,
            |_| false,
            |found| {
                let n = count();
                fs::write(&at, "mine".repeat(n + 1)).expect("write");
                put(&dir, "a.md", b"new", found.original())
            },
        );
        assert_eq!(overtaken.expect("no error"), Done::Changed);
        assert_eq!(tries.replace(0), ATTEMPTS);
        // By the same text: it is what was seen.
        let seen = file("old");
        fs::write(&at, "old").expect("write");
        let overtaken = change(
            &dir,
            "a.md",
            Some(&seen),
            |_| false,
            |found| {
                if count() == 0 {
                    fs::write(dir.join("saved"), "old").expect("write");
                    fs::rename(dir.join("saved"), &at).expect("mv");
                }
                put(&dir, "a.md", b"new", found.original())
            },
        );
        assert_eq!(overtaken.expect("no error"), Done::Yes);
        assert_eq!(tries.replace(0), 2);
        // Once only: with nothing seen, what is there by then is replaced.
        let overtaken = change(
            &dir,
            "a.md",
            None,
            |_| false,
            |found| {
                if count() == 0 {
                    fs::write(&at, "theirs").expect("write");
                }
                put(&dir, "a.md", b"new", found.original())
            },
        );
        assert_eq!(overtaken.expect("no error"), Done::Yes);
        assert_eq!(fs::read(&at).expect("read"), b"new");
    }

    /// A link is never read as the file it leads to, whatever is in that.
    #[test]
    fn a_link_is_no_file() {
        let (_tmp, dir) = dir();
        fs::write(dir.join("a.md"), "old").expect("write");
        symlink("a.md", dir.join("link.md")).expect("symlink");
        assert!(matches!(find(&dir, "link.md"), Ok(Found::Other)));
        // As if it had become one after the look at what it is.
        assert!(matches!(
            content(&dir.join("link.md"), 10),
            Ok(Found::Other)
        ));
        assert!(matches!(
            content(&dir.join("none.md"), 10),
            Ok(Found::Missing)
        ));
        assert_eq!(
            write(&dir, "link.md", b"new", None).expect("write"),
            Done::Yes
        );
        assert!(!dir.join("link.md").is_symlink());
        assert_eq!(fs::read(dir.join("a.md")).expect("read"), b"old");
        assert_eq!(delete(&dir, "link.md", None).expect("delete"), Done::Yes);
    }

    #[test]
    fn a_name_no_file_can_have_is_that_of_none() {
        let (_tmp, dir) = dir();
        let long = "a".repeat(300);
        for path in [long.clone(), format!("{long}/a.md")] {
            assert!(matches!(find(&dir, &path), Ok(Found::Missing)), "{path}");
        }
    }

    #[test]
    fn a_file_larger_than_any_of_atls_is_not_read() {
        let (_tmp, dir) = dir();
        fs::write(dir.join("a"), "1234").expect("write");
        let found = |max| find_up_to(&dir, "a", max).expect("found");
        assert!(matches!(found(4), Found::File(bytes, _) if bytes == b"1234"));
        assert!(matches!(found(3), Found::Large));
        // Grown since it was looked at.
        assert!(matches!(content(&dir.join("a"), 3), Ok(Found::Large)));
        let file = fs::File::create(dir.join("b")).expect("create");
        file.set_len(MAX_FILE_BYTES + 1).expect("grow");
        assert!(matches!(find(&dir, "b"), Ok(Found::Large)));
        // Taken out, then: what replaces it is a new file.
        assert_eq!(write(&dir, "b", b"new", None).expect("write"), Done::Yes);
        assert_eq!(fs::read(dir.join("b")).expect("read"), b"new");
    }

    #[test]
    fn what_blocks_a_path_is_named_with_slashes() {
        let (_tmp, dir) = dir();
        fs::write(dir.join("references/x"), "").expect("write");
        assert!(matches!(
            find(&dir, "references/x/a.md"),
            Ok(Found::Blocked(at)) if at == "references/x"
        ));
        assert!(matches!(
            find(&dir, "references"),
            Ok(Found::Blocked(at)) if at == "references"
        ));
    }
}
