//! Atomic replacement of the skill's files: the minimum the skill needs, and
//! the guarantees it needs of it.
//!
//! 1. **No link is followed or replaced at the file.** A file is read as no
//!    link ([`read_no_link`]); one that is a link by the time it would be
//!    written or removed is refused as changed. The directories on the way
//!    are the caller's to look at (`disk::ensure_plain_way`).
//! 2. **Bound to what was read.** [`rewrite`] and [`remove`] are given the
//!    [`Original`] that [`read_no_link`] returned: the path must still hold
//!    that very file — identity (device and inode, where there are such),
//!    time of creation, size, time of modification — when the temporary file
//!    is made and once more right before the rename. Anything else is
//!    refused as [`is_changed`]: someone's work, left alone. The caller binds
//!    the digest (`disk::change`).
//! 3. **The temporary file is in the same directory** (a rename across file
//!    systems is no rename), and named as `tempfile` names it: `.tmp` and six
//!    letters or digits, which the survey leaves out.
//! 4. **Its mode is never wider than the final one, from the start.** A new
//!    file is created with `0666` less the umask, which the system applies
//!    (`tempfile` alone would make it `0600`). One that replaces a file is
//!    created open to its owner alone, as far as that file was, and gets the
//!    file's permission bits, and its group and owner as far as the process
//!    may set them, once written. Of a group that cannot be kept, both the
//!    group and the others get what both had (`0664` becomes `0644`).
//! 5. **A new file takes no one's place**: the rename is one that replaces
//!    nothing ([`create`]); where the file system has none, an empty file
//!    made with `O_EXCL` takes the name first.
//! 6. **Deletion the system forbids is asked first** — a file whose ACL or
//!    flag forbids it, or a directory that does (macOS ACLs and `uchg`,
//!    Linux `chattr +i/+a`) — so that nothing is made in vain.
//!
//! Not kept: ACLs, extended attributes, flags, timestamps, other hard links.
//! On Windows there are no modes to keep, and no identity but times and size.

use std::fs::{self, File, Metadata};
use std::io::{self, Read as _, Seek as _, Write as _};
use std::time::SystemTime;

use camino::Utf8Path;
use tempfile::NamedTempFile;

use super::error::FileError;

/// The file a path held when it was read: what [`rewrite`] and [`remove`]
/// are bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Original {
    entry: Entry,
}

/// What a file is, whatever it is called, and what tells that it is as it
/// was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    /// The device and the inode.
    #[cfg(unix)]
    id: (u64, u64),
    /// The time of creation, where there is one: the inode of a file that
    /// was deleted may be given to the next one.
    created: Option<SystemTime>,
    /// A file rewritten in place is the same file, but not what was read.
    size: u64,
    modified: Option<SystemTime>,
}

impl Entry {
    fn of(found: &Metadata) -> Self {
        Self {
            #[cfg(unix)]
            id: {
                use std::os::unix::fs::MetadataExt as _;
                (found.dev(), found.ino())
            },
            created: found.created().ok(),
            size: found.len(),
            modified: found.modified().ok(),
        }
    }
}

/// What is at a path, a link not followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum There {
    Nothing,
    File(Entry),
    /// A link, a directory, a device.
    Other,
}

fn look(path: &Utf8Path) -> io::Result<There> {
    match fs::symlink_metadata(path) {
        Ok(found) if found.is_file() => Ok(There::File(Entry::of(&found))),
        Ok(_) => Ok(There::Other),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(There::Nothing),
        Err(err) => Err(err),
    }
}

/// Read the regular file at `path`, which is no symbolic link, and note which
/// file it is.
///
/// # Errors
///
/// [`FileError::Read`] as for any read: missing, not permitted, a directory;
/// also when `path` is a link, or no regular file.
pub(super) fn read_no_link(path: &Utf8Path) -> Result<(Vec<u8>, Original), FileError> {
    let read = || -> io::Result<(Vec<u8>, Original)> {
        let mut file = open_no_follow(path)?;
        // Before the contents: a change while they are read shows, too.
        let found = file.metadata()?;
        if !found.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "it is no regular file",
            ));
        }
        let entry = Entry::of(&found);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok((bytes, Original { entry }))
    };
    read().map_err(|cause| FileError::Read {
        path: path.to_path_buf(),
        cause,
    })
}

/// Atomically put `content` at `path`, in place of the file `original` was
/// read from. `false` when that holds exactly `content`: it is left alone.
///
/// # Errors
///
/// [`FileError::Write`] when `path` does not hold that file any more
/// ([`is_changed`]), may not be deleted, or the temporary file cannot be
/// made, written, given the file's mode or renamed.
pub(super) fn rewrite(
    path: &Utf8Path,
    original: &Original,
    content: &[u8],
) -> Result<bool, FileError> {
    let rewrite = || -> io::Result<bool> {
        let existing = original.find(path)?;
        if existing.holds(content)? {
            return Ok(false);
        }
        // Closed before the rename: Windows refuses to replace a file that
        // is open. What binds the rename is the check right before it.
        let found = existing.close();
        check_deletable(path, "replaced")?;
        let temp = stage(path, content, Some(&found))?;
        original.check(path)?;
        temp.persist(path).map_err(|err| err.error)?;
        Ok(true)
    };
    rewrite().map_err(write_error(path))
}

/// Atomically put `content` at `path`, where nothing is: a new file, `0666`
/// less the umask.
///
/// # Errors
///
/// [`FileError::Write`]; [`is_changed`] when anything, a link among it, is
/// at `path` by the time the file would get its name.
pub(super) fn create(path: &Utf8Path, content: &[u8]) -> Result<(), FileError> {
    let create = || -> io::Result<()> {
        if look(path)? != There::Nothing {
            return Err(appeared());
        }
        if !deletable::may_delete_in(dir_of(path)?) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "its directory forbids deleting the files in it, and so renaming them",
            ));
        }
        let temp = stage(path, content, None)?;
        put_new(temp, path).map_err(|err| match err.kind() {
            io::ErrorKind::AlreadyExists => appeared(),
            _ => err,
        })
    };
    create().map_err(write_error(path))
}

/// Check, before anything is written, that the system will let [`remove`]
/// take `path` away, as far as it tells beforehand.
///
/// # Errors
///
/// [`FileError::Write`] when it says that it will not.
pub(super) fn check_remove(path: &Utf8Path) -> Result<(), FileError> {
    check_deletable(path, "removed").map_err(write_error(path))
}

/// Remove `path`, from which `original` was read.
///
/// # Errors
///
/// [`FileError::Write`] when `path` does not hold that file any more
/// ([`is_changed`]), or cannot be removed.
pub(super) fn remove(path: &Utf8Path, original: &Original) -> Result<(), FileError> {
    original
        .find(path)
        // Closed before it is removed: on Windows an open file is only marked
        // for deletion, and its directory is not empty until it is closed.
        .map(Existing::close)
        .and_then(|_| fs::remove_file(path))
        .map_err(write_error(path))
}

/// Whether `err` is the refusal of a write or a removal because the file is
/// not the one that was read any more, or one is there where none was.
#[must_use]
pub(super) fn is_changed(err: &FileError) -> bool {
    matches!(err, FileError::Write { cause, .. }
        if cause.get_ref().is_some_and(|cause| cause.is::<Changed>()))
}

/// What [`is_changed`] tells from other errors.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Changed(&'static str);

fn changed() -> io::Error {
    io::Error::other(Changed(
        "it changed while atl was running: it is not the file that was read any more",
    ))
}

fn appeared() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        Changed("something appeared there while atl was running; it is not replaced"),
    )
}

fn write_error(path: &Utf8Path) -> impl FnOnce(io::Error) -> FileError {
    let path = path.to_path_buf();
    move |cause| FileError::Write { path, cause }
}

impl Original {
    /// Check that `path` holds the file this was read from.
    fn check(&self, path: &Utf8Path) -> io::Result<()> {
        if look(path)? == There::File(self.entry) {
            Ok(())
        } else {
            Err(changed())
        }
    }

    /// Find the file again, and open it. What was opened is the gate: its
    /// identity must be the one that was read. A path that cannot be opened
    /// because it is gone or a link now is changed too.
    fn find(&self, path: &Utf8Path) -> io::Result<Existing> {
        let file = match open_no_follow(path) {
            Ok(file) => file,
            // Gone, or a link now.
            Err(_) if self.check(path).is_err() => return Err(changed()),
            Err(err) => return Err(err),
        };
        let found = file.metadata()?;
        if Entry::of(&found) != self.entry {
            return Err(changed());
        }
        Ok(Existing { file, found })
    }
}

/// A file that is there already, open.
struct Existing {
    file: File,
    found: Metadata,
}

impl Existing {
    /// Close the file; what was found of it stays.
    fn close(self) -> Metadata {
        let Self { file, found } = self;
        drop(file);
        found
    }

    /// Whether the file holds exactly `content`.
    fn holds(&self, content: &[u8]) -> io::Result<bool> {
        if usize::try_from(self.found.len()) != Ok(content.len()) {
            return Ok(false);
        }
        let mut bytes = Vec::with_capacity(content.len());
        let mut file = &self.file;
        file.rewind()?;
        file.read_to_end(&mut bytes)?;
        Ok(bytes == content)
    }
}

/// Open `path` to read it, not through a link at its last component, and
/// without waiting for the other end of a pipe.
fn open_no_follow(path: &Utf8Path) -> io::Result<File> {
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        /// `FILE_FLAG_OPEN_REPARSE_POINT`: the link itself is opened, which
        /// is then no regular file.
        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn dir_of(path: &Utf8Path) -> io::Result<&Utf8Path> {
    match path.parent() {
        Some(dir) if dir.as_str().is_empty() => Ok(Utf8Path::new(".")),
        Some(dir) => Ok(dir),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a file of the skill has no directory",
        )),
    }
}

/// Fail when the system forbids deleting what is at `path`, which a rename
/// over it is too.
fn check_deletable(path: &Utf8Path, done: &str) -> io::Result<()> {
    if deletable::may_delete(path) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("it cannot be {done}: its ACL or flags, or its directory, forbid deleting it"),
    ))
}

/// A temporary file holding `content`, ready to be renamed to `path`: in its
/// directory, with the access of `like` or a new file's.
fn stage(path: &Utf8Path, content: &[u8], like: Option<&Metadata>) -> io::Result<NamedTempFile> {
    let mut temp = create_temp(dir_of(path)?, like)?;
    temp.write_all(content)?;
    // After the content: writing clears set-user-ID and set-group-ID bits.
    if let Some(like) = like {
        for lost in keep_access(temp.as_file(), like)? {
            tracing::warn!("{}: {lost}", super::one_line(path.as_str()));
        }
    }
    Ok(temp)
}

/// Rename `temp` to `path`, where nothing may be.
fn put_new(temp: NamedTempFile, path: &Utf8Path) -> io::Result<()> {
    let temp = match temp.persist_noclobber(path) {
        Ok(_) => return Ok(()),
        Err(failed) if lacks_rename_to_new(&failed.error) => failed.file,
        Err(failed) => return Err(failed.error),
    };
    // No such rename here, and no hard links either: an empty file, which is
    // only made where nothing is, takes the name first.
    let claim = Entry::of(&File::create_new(path)?.metadata()?);
    temp.persist(path).map(|_| ()).map_err(|failed| {
        if look(path).is_ok_and(|there| there == There::File(claim)) {
            let _ = fs::remove_file(path);
        }
        failed.error
    })
}

/// Whether `err` says that the file system has no rename that replaces
/// nothing, and no hard link to stand in for one.
#[cfg(unix)]
fn lacks_rename_to_new(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(code) if code == libc::ENOTSUP
            || code == libc::EOPNOTSUPP
            || code == libc::ENOSYS
            || code == libc::EINVAL
            // Said of hard links on file systems that have none. Of a
            // directory whose files may not be renamed it is said too, but
            // `create` asked that before; the second way fails as well then.
            || code == libc::EPERM
    )
}

#[cfg(not(unix))]
fn lacks_rename_to_new(_err: &io::Error) -> bool {
    false
}

/// An empty temporary file in `dir`. One to stand in for `like` is open to
/// its owner alone, and to no more than the owner of `like` had.
#[cfg(unix)]
fn create_temp(dir: &Utf8Path, like: Option<&Metadata>) -> io::Result<NamedTempFile> {
    use std::os::unix::fs::PermissionsExt as _;
    // The mode of `open(2)`, from which the system takes the umask off. The
    // file is open for writing whatever its mode is.
    let mode = like.map_or(0o666, |like| like.permissions().mode() & 0o600);
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(mode))
        .tempfile_in(dir)
}

#[cfg(not(unix))]
fn create_temp(dir: &Utf8Path, _like: Option<&Metadata>) -> io::Result<NamedTempFile> {
    NamedTempFile::new_in(dir)
}

/// Give `file` the group, permission bits and owner of `like`, in an order
/// that never opens it wider than `like` was; what could not be kept, to say.
#[cfg(unix)]
fn keep_access(file: &File, like: &Metadata) -> io::Result<Vec<&'static str>> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, fchown};
    let mut lost = Vec::new();
    let mut bits = like.mode() & 0o777;
    if file.metadata()?.gid() != like.gid() {
        let _ = fchown(file, None, Some(like.gid()));
        if file.metadata()?.gid() != like.gid() {
            // The old group were "others" of this file, the old others are
            // now in its group: both get what both had.
            let both = (bits >> 3) & bits & 0o7;
            bits = (bits & 0o700) | (both << 3) | both;
            lost.push("its group could not be kept");
        }
    }
    file.set_permissions(fs::Permissions::from_mode(bits))?;
    if file.metadata()?.uid() != like.uid() {
        let _ = fchown(file, Some(like.uid()), None);
        if file.metadata()?.uid() != like.uid() {
            lost.push("its owner could not be kept");
        }
    }
    Ok(lost)
}

#[cfg(not(unix))]
fn keep_access(_file: &File, _like: &Metadata) -> io::Result<Vec<&'static str>> {
    Ok(Vec::new())
}

/// What the system lets be deleted, asked before anything is made.
///
/// - macOS: an ACL can forbid it, of a file (`deny delete`) or of what is in
///   a directory (`deny delete_child`), and so can a flag (`uchg`). The
///   system is asked.
/// - Linux: the flags `i` and `a` of a file, and `a` of a directory
///   (`chattr`), forbid it to root too. Those are looked at, where the file
///   system tells them.
/// - Elsewhere, and of what else forbids it, the rename or the removal says
///   so.
mod deletable {
    #[cfg(target_os = "linux")]
    pub(super) use linux::{may_delete, may_delete_in};
    #[cfg(target_os = "macos")]
    pub(super) use macos::{may_delete, may_delete_in};
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(super) use other::{may_delete, may_delete_in};

    #[cfg(target_os = "macos")]
    mod macos {
        use std::ffi::CString;
        use std::io;

        use camino::Utf8Path;

        use super::super::dir_of;

        /// `_WRITE_OK` of `sys/unistd.h`: of a directory, to add files to it.
        const WRITE_OK: libc::c_int = 1 << 10;
        /// `_DELETE_OK`: to delete what is at the path.
        const DELETE_OK: libc::c_int = 1 << 12;
        /// `_RMFILE_OK`: to delete the files in the directory at the path.
        const RMFILE_OK: libc::c_int = 1 << 14;

        /// Whether what is at `path`, be it a link, may be deleted. `true`
        /// when that cannot be told.
        pub(in super::super) fn may_delete(path: &Utf8Path) -> bool {
            may(path, DELETE_OK, libc::AT_SYMLINK_NOFOLLOW)
                || dir_of(path).is_ok_and(|dir| !may(dir, WRITE_OK, 0))
        }

        /// Whether the files in `dir` may be deleted. `true` when that cannot
        /// be told.
        pub(in super::super) fn may_delete_in(dir: &Utf8Path) -> bool {
            may(dir, RMFILE_OK, 0) || !may(dir, WRITE_OK, 0)
        }

        fn may(path: &Utf8Path, access: libc::c_int, flags: libc::c_int) -> bool {
            let Ok(path) = CString::new(path.as_str()) else {
                return true;
            };
            let flags = flags | libc::AT_EACCESS;
            // SAFETY: the path is a C string that lives for the call.
            let found = unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), access, flags) };
            found == 0
                || !matches!(
                    io::Error::last_os_error().raw_os_error(),
                    Some(libc::EACCES | libc::EPERM)
                )
        }
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        use camino::Utf8Path;

        use super::super::dir_of;

        /// Whether what is at `path`, be it a link, may be deleted. `true`
        /// when that cannot be told.
        pub(in super::super) fn may_delete(path: &Utf8Path) -> bool {
            !has(path, libc::STATX_ATTR_IMMUTABLE | libc::STATX_ATTR_APPEND)
                && dir_of(path).ok().is_none_or(may_delete_in)
        }

        /// Whether the files in `dir` may be deleted. `true` when that cannot
        /// be told.
        pub(in super::super) fn may_delete_in(dir: &Utf8Path) -> bool {
            // In an immutable one the temporary file cannot even be made.
            !has(&dir.join("."), libc::STATX_ATTR_APPEND)
        }

        /// Whether what is at `path` has one of the `flags`.
        fn has(path: &Utf8Path, flags: libc::c_int) -> bool {
            let Ok(path) = CString::new(path.as_str()) else {
                return false;
            };
            let mut found = MaybeUninit::<libc::statx>::zeroed();
            // SAFETY: the path is a C string, and there is room for the answer.
            let done = unsafe {
                libc::statx(
                    libc::AT_FDCWD,
                    path.as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                    0,
                    found.as_mut_ptr(),
                )
            };
            // SAFETY: zeroes are a valid answer, whether it was filled in or not.
            let found = unsafe { found.assume_init() };
            done == 0 && found.stx_attributes & u64::from(flags.unsigned_abs()) != 0
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    mod other {
        use camino::Utf8Path;

        pub(in super::super) fn may_delete(_path: &Utf8Path) -> bool {
            true
        }

        pub(in super::super) fn may_delete_in(_dir: &Utf8Path) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use pretty_assertions::assert_eq;

    fn dir() -> (tempfile::TempDir, Utf8PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("UTF-8");
        (tmp, dir)
    }

    /// What a rewrite does to the file that was read, and to what came in
    /// its place since.
    #[test]
    fn a_rewrite_is_bound_to_the_file_that_was_read() {
        let (_tmp, dir) = dir();
        let at = dir.join("a.md");
        fs::write(&at, "old").expect("write");
        let (bytes, original) = read_no_link(&at).expect("read");
        assert_eq!(bytes, b"old");
        assert!(!rewrite(&at, &original, b"old").expect("same"), "untouched");
        assert!(rewrite(&at, &original, b"new").expect("written"));
        assert_eq!(fs::read(&at).expect("read"), b"new");
        // Bound to what was read: the file that is there now is another one.
        let err = rewrite(&at, &original, b"newer").expect_err("changed");
        assert!(is_changed(&err), "{err}");
        let err = remove(&at, &original).expect_err("changed");
        assert!(is_changed(&err), "{err}");
        assert_eq!(fs::read(&at).expect("read"), b"new");
        // Rewritten in place, to another size.
        let (_, original) = read_no_link(&at).expect("read");
        fs::write(&at, "mine, longer").expect("write");
        assert!(is_changed(
            &rewrite(&at, &original, b"x").expect_err("changed")
        ));
        let (_, original) = read_no_link(&at).expect("read");
        remove(&at, &original).expect("removed");
        assert!(!at.exists());
        assert!(is_changed(
            &rewrite(&at, &original, b"x").expect_err("gone")
        ));
        let leftovers = fs::read_dir(&dir).expect("dir").count();
        assert_eq!(leftovers, 0, "no temporary file stays");
    }

    #[test]
    fn a_new_file_takes_no_ones_place() {
        let (_tmp, dir) = dir();
        let at = dir.join("a.md");
        create(&at, b"new").expect("created");
        assert_eq!(fs::read(&at).expect("read"), b"new");
        let err = create(&at, b"newer").expect_err("there");
        assert!(is_changed(&err), "{err}");
        assert_eq!(fs::read(&at).expect("read"), b"new");
        let err = create(&dir.join("none/a.md"), b"x").expect_err("no directory");
        assert!(!is_changed(&err), "{err}");
        assert!(matches!(err, FileError::Write { .. }));
    }

    #[test]
    fn what_is_read_is_no_link_and_a_regular_file() {
        let (_tmp, dir) = dir();
        fs::create_dir(dir.join("d")).expect("mkdir");
        assert!(matches!(
            read_no_link(&dir.join("d")),
            Err(FileError::Read { .. })
        ));
        assert!(matches!(
            read_no_link(&dir.join("none")),
            Err(FileError::Read { cause, .. }) if cause.kind() == io::ErrorKind::NotFound
        ));
        #[cfg(unix)]
        {
            fs::write(dir.join("a"), "old").expect("write");
            std::os::unix::fs::symlink(dir.join("a"), dir.join("link")).expect("symlink");
            assert!(read_no_link(&dir.join("link")).is_err());
            // A link that comes in the place of a file that was read.
            let (_, original) = read_no_link(&dir.join("a")).expect("read");
            fs::rename(dir.join("a"), dir.join("aside")).expect("mv");
            std::os::unix::fs::symlink(dir.join("aside"), dir.join("a")).expect("symlink");
            let err = rewrite(&dir.join("a"), &original, b"new").expect_err("a link");
            assert!(is_changed(&err), "{err}");
            assert!(is_changed(
                &create(&dir.join("a"), b"new").expect_err("a link")
            ));
            assert_eq!(fs::read(dir.join("aside")).expect("read"), b"old");
            assert!(dir.join("a").is_symlink());
        }
    }

    /// A new file has the mode of any new file; one that replaces another has
    /// that one's, and is never more open while it is written.
    #[test]
    #[cfg(unix)]
    fn a_file_keeps_its_mode_and_a_new_one_gets_a_new_files() {
        use std::os::unix::fs::PermissionsExt as _;
        let (_tmp, dir) = dir();
        let mode =
            |path: &Utf8Path| fs::metadata(path).expect("stat").permissions().mode() & 0o7777;
        let at = dir.join("new.md");
        create(&at, b"new").expect("created");
        fs::write(dir.join("plain"), "").expect("write");
        assert_eq!(mode(&at), mode(&dir.join("plain")), "as any new file");
        for kept in [0o644, 0o600, 0o640, 0o444, 0o751] {
            fs::set_permissions(&at, fs::Permissions::from_mode(kept)).expect("chmod");
            let (_, original) = read_no_link(&at).expect("read");
            let staged = stage(&at, b"x", Some(&fs::metadata(&at).expect("stat"))).expect("stage");
            assert_eq!(
                mode(Utf8Path::from_path(staged.path()).expect("UTF-8")),
                kept
            );
            drop(staged);
            assert!(rewrite(&at, &original, format!("{kept}").as_bytes()).expect("written"));
            assert_eq!(mode(&at), kept, "{kept:o}");
        }
        let created = create_temp(&dir, Some(&fs::metadata(&at).expect("stat"))).expect("temp");
        let staged = Utf8Path::from_path(created.path()).expect("UTF-8");
        assert_eq!(mode(staged) & !0o600, 0, "the owner's alone until written");
    }

    #[test]
    fn deletion_is_asked_for_first() {
        let (_tmp, dir) = dir();
        fs::write(dir.join("a"), "").expect("write");
        check_remove(&dir.join("a")).expect("may be removed");
        check_remove(&dir.join("none")).expect("nothing forbids it");
    }

    /// A save that comes while the new text is staged — after the file was
    /// found again and opened, before the rename — is the user's, and stays.
    ///
    /// The one moment a test can step in there is the warning a stage gives
    /// of a group it cannot keep: a subscriber that saves the file when it
    /// hears it. On macOS, where a file of a group the user is not in
    /// (`wheel`, of `/private/tmp`) is made without being root.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_save_while_the_new_text_is_staged_is_not_lost() {
        use std::os::unix::fs::MetadataExt as _;
        use std::sync::OnceLock;
        use tracing_subscriber::layer::SubscriberExt as _;

        /// Saves `text` at `path`, as an editor does (a rename over it), when
        /// a warning is given.
        struct SaveOnWarning {
            path: Utf8PathBuf,
            text: &'static str,
        }
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for SaveOnWarning {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                if *event.metadata().level() == tracing::Level::WARN {
                    let saved = self.path.with_extension("saved");
                    fs::write(&saved, self.text).expect("write");
                    fs::rename(&saved, &self.path).expect("save");
                }
            }
        }
        // With one subscriber in the process, `tracing` takes what another
        // thread's test says of a log line, whose thread has none, for all.
        static OTHERS: OnceLock<[tracing::Dispatch; 2]> = OnceLock::new();
        OTHERS.get_or_init(|| {
            [(), ()].map(|()| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()))
        });

        let (_tmp, dir) = dir();
        let at = dir.join("SKILL.md");
        let wheel = tempfile::NamedTempFile::new_in("/private/tmp").expect("temp file");
        fs::write(wheel.path(), "old").expect("write");
        wheel.persist(&at).expect("mv");
        let group = fs::metadata(&at).expect("stat").gid();
        let (_, original) = read_no_link(&at).expect("read");
        let subscriber = tracing_subscriber::registry().with(SaveOnWarning {
            path: at.clone(),
            text: "mine",
        });
        let result =
            tracing::subscriber::with_default(subscriber, || rewrite(&at, &original, b"new"));
        if fs::read(&at).expect("read") == b"new" && fs::metadata(&at).expect("stat").gid() == group
        {
            return; // kept: the user is in that group, and no warning came
        }
        let err = result.expect_err("saved meanwhile");
        assert!(is_changed(&err), "{err}");
        assert_eq!(fs::read(&at).expect("read"), b"mine", "the save is kept");
        let left: Vec<_> = fs::read_dir(&dir).expect("dir").collect();
        assert_eq!(left.len(), 1, "no temporary file stays");
    }
}
