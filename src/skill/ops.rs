//! `install`, `update`, `remove`, `status` on one skill directory.
//!
//! Each command first decides, from a [`survey`] of the directory, whether it
//! may go on, and only then writes: a refused command has changed nothing.
//!
//! **Concurrency.** The directory itself is locked (an advisory `flock`)
//! while a command looks at it and writes to it, so two versions of atl do not
//! leave a mixture. `remove` deletes what it has locked: whoever waited for
//! that lock has one on a directory that is gone, and starts over with what
//! is at the path by then ([`lock`]).
//!
//! **What others do meanwhile.** A user's editor takes no lock. Every write
//! and deletion is bound to what the survey saw ([`super::disk`]): a file
//! that was saved since is left as it is, and the command stops there.
//!
//! **A run that is cut short.** Every file is replaced atomically. The
//! manifest is written twice: before the files, with what they are and what
//! they are to be, and after them. So whatever comes next, of this version or
//! of another, knows atl's files from edited ones.

use std::collections::BTreeMap;
use std::time::Instant;

use camino::{Utf8Path, Utf8PathBuf};

use super::content::Edition;
use super::disk::{Done, Seen, delete, remove_if_empty, respell, write};
use super::error::{FileError, Refusal, SkillError, listed};
use super::link::{LinkState, Made};
use super::manifest::is_newer;
use super::manifest::{MANIFEST, Manifest, ManifestError};
use super::place::{
    ATTEMPTS, LOCK_WAIT, Locked, Place, ensure_directory_or_absent, lock, make_and_lock,
    other_owner, place,
};
use super::survey::{
    FileState, Installed, Survey, Verdict, files_under, left_under, read_manifest, survey,
};
use super::{Environment, Target};

/// What `--force` adds to `install` and `update` over changed files.
const FORCE_WRITES: &str = "--force writes atl's files in their place";

/// What `--force` adds to `remove` over edited files.
const FORCE_REMOVES: &str = "--force removes the edited files too";

/// How the version of an installed skill compares with an edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// The same version.
    Same,
    /// The installed skill is older.
    Older,
    /// The installed skill is newer.
    Newer,
}

impl Freshness {
    /// How `installed` compares with `current`, in atl's order of versions.
    #[must_use]
    pub fn of(installed: &str, current: &str) -> Self {
        if is_newer(current, installed) {
            Self::Older
        } else if is_newer(installed, current) {
            Self::Newer
        } else {
            Self::Same
        }
    }
}

/// What [`install`] or [`update`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReport {
    /// The skill directory.
    pub dir: Utf8PathBuf,
    /// The version that was installed before, `None` when atl had installed
    /// none.
    pub previous: Option<String>,
    /// The version that is installed now.
    pub version: String,
    /// Whether anything in the directory changed.
    pub changed: bool,
    /// Files of an older atl that this one does not have, kept because they
    /// were edited. They are nobody's from here on.
    pub kept: Vec<String>,
    /// The link and what was done about it; `None` when none is wanted.
    pub link: Option<(Utf8PathBuf, Made)>,
}

/// What [`remove`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveReport {
    /// The skill directory.
    pub dir: Utf8PathBuf,
    /// The version that was removed, `None` when none was installed.
    pub version: Option<String>,
    /// The files that stay in the directory: not atl's.
    pub left: Vec<String>,
    /// When there are none of those, whatever else it is that stays.
    pub other: Vec<String>,
    /// Whether the directory stays, because it is not empty.
    pub dir_kept: bool,
    /// The link that was removed.
    pub link: Option<Utf8PathBuf>,
}

/// What [`status`] found in the place of the skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing.
    NotInstalled,
    /// Installed, and what this atl would write.
    UpToDate,
    /// Installed by an older atl.
    Older,
    /// Installed by a newer atl.
    Newer,
    /// Installed by a atl of the same version with other files (a build
    /// between two releases).
    Differs,
    /// Not installed by atl.
    Foreign,
    /// Installed by atl, with a manifest that cannot be used.
    Damaged,
    /// Installed by a newer atl in a schema this one does not know.
    UnknownSchema,
    /// The place of the skill directory is a link, or no directory.
    Blocked,
    /// The skill directory is another user's. What is in it is not looked at.
    OtherOwner,
}

impl State {
    /// The name in the output of `atl skill status`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::UpToDate => "up_to_date",
            Self::Older => "older",
            Self::Newer => "newer",
            Self::Differs => "differs",
            Self::Foreign => "foreign",
            Self::Damaged => "damaged",
            Self::UnknownSchema => "unknown_schema",
            Self::Blocked => "blocked",
            Self::OtherOwner => "other_owner",
        }
    }

    /// Whether this is a skill atl installed.
    #[must_use]
    pub fn is_installed(self) -> bool {
        match self {
            Self::UpToDate
            | Self::Older
            | Self::Newer
            | Self::Differs
            | Self::Damaged
            | Self::UnknownSchema => true,
            Self::NotInstalled | Self::Foreign | Self::Blocked | Self::OtherOwner => false,
        }
    }
}

/// The skill at a [`Target`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The skill directory.
    pub dir: Utf8PathBuf,
    /// What is there.
    pub state: State,
    /// The atl version that installed it, where the manifest says so.
    pub version: Option<String>,
    /// Files of atl's that are not as it wrote them.
    pub edited: Vec<String>,
    /// Files of atl's that are gone.
    pub missing: Vec<String>,
    /// Files that are not atl's.
    pub unlisted: Vec<String>,
    /// Files and links that are not atl's, where this atl has a file to write.
    pub colliding: Vec<String>,
    /// Links and directories in the way of atl's files, which atl neither goes
    /// through nor replaces.
    pub in_the_way: Vec<String>,
    /// The link and what is in its place; `None` when none is wanted.
    pub link: Option<(Utf8PathBuf, LinkState)>,
}

/// Write `edition` to `target` and make the link.
///
/// # Errors
///
/// [`SkillError::Refused`] for what is not atl's to change (nothing is
/// written then), [`SkillError::File`] when a file cannot be read or
/// written.
pub fn install(target: &Target, edition: &Edition, force: bool) -> Result<WriteReport, SkillError> {
    ensure_owner(target)?;
    ensure_directory_or_absent(&target.dir)?;
    if let Some(link) = &target.link {
        link.ensure_free(force)?;
    }
    let _lock = make_and_lock(&target.dir, Instant::now() + LOCK_WAIT)?;
    let mut report = write_skill(target, edition, force)?;
    if let Some(link) = &target.link {
        report.link = Some((link.place.clone(), link.make(force)?));
    }
    Ok(report)
}

/// Bring the skill at `target` to `edition`; `None` when atl installed none
/// there. The link is left as it is: `install` makes it.
///
/// # Errors
///
/// As [`install`].
pub fn update(
    target: &Target,
    edition: &Edition,
    force: bool,
) -> Result<Option<WriteReport>, SkillError> {
    ensure_owner(target)?;
    let Locked::Directory(_lock) = lock(&target.dir, Instant::now() + LOCK_WAIT)? else {
        return Ok(None);
    };
    let found = survey(&target.dir, edition)?;
    if found.installed == Installed::No && found.unlisted.is_empty() {
        return Ok(None);
    }
    write_skill(target, edition, force).map(Some)
}

/// Remove the skill at `target`: the files atl wrote, the manifest, the
/// directories that leaves empty, and the link.
///
/// # Errors
///
/// As [`install`]; a directory without a usable manifest is refused with
/// `force` too, because nothing says which of its files are atl's.
pub fn remove(target: &Target, edition: &Edition, force: bool) -> Result<RemoveReport, SkillError> {
    ensure_owner(target)?;
    let mut report = RemoveReport {
        dir: target.dir.clone(),
        version: None,
        left: Vec::new(),
        other: Vec::new(),
        dir_kept: false,
        link: None,
    };
    let deadline = Instant::now() + LOCK_WAIT;
    for _ in 0..ATTEMPTS {
        match lock(&target.dir, deadline)? {
            Locked::Directory(_lock) => {
                remove_locked(target, edition, force, &mut report)?;
                break;
            }
            Locked::Absent => {
                // A link left by a remove that was cut short.
                let Some(place) = remove_link(target)? else {
                    break;
                };
                report.link = Some(place);
                // Or by an install that has come since: that one goes whole.
                if ensure_directory_or_absent(&target.dir)? == Place::Absent {
                    break;
                }
            }
        }
    }
    Ok(report)
}

/// [`remove`] from a directory that is there and locked; what was done goes
/// into `report`.
fn remove_locked(
    target: &Target,
    edition: &Edition,
    force: bool,
    report: &mut RemoveReport,
) -> Result<(), SkillError> {
    ensure_owner(target)?; // once more, now that it is locked
    let found = survey(&target.dir, edition)?;
    let manifest = match &found.installed {
        Installed::Yes(manifest) => manifest,
        Installed::No if found.unlisted.is_empty() => return Ok(()),
        Installed::No => return Err(not_ours(&target.dir, format!("it has no {MANIFEST}"))),
        Installed::Unusable(ManifestError::NewerSchema { schema }) => {
            return Err(newer_schema(&target.dir, *schema));
        }
        Installed::Unusable(ManifestError::Malformed { reason }) => {
            return Err(not_ours(
                &target.dir,
                format!("its {MANIFEST} cannot be used: {reason}"),
            ));
        }
    };
    ensure_unblocked(&target.dir, found.blocked())?;
    // Only what the manifest lists is atl's to remove: a file that is in
    // the way of another edition's is none of this command's business.
    ensure_unchanged(
        &target.dir,
        found.edited(),
        Vec::new(),
        force,
        FORCE_REMOVES,
    )?;
    if let Some(place) = remove_link(target)? {
        report.link = Some(place);
    }
    delete_files(&target.dir, &found.files, force)?;
    report.dir_kept = !remove_if_empty(&target.dir)?;
    report.version = Some(manifest.atl_version.clone());
    if report.dir_kept {
        report.left = files_under(&target.dir)?;
        if report.left.is_empty() {
            report.other = left_under(&target.dir);
        }
    }
    Ok(())
}

/// Delete those of `files`, what was seen of them, that atl wrote, then the
/// manifest.
fn delete_files(dir: &Utf8Path, files: &[FileState], force: bool) -> Result<(), SkillError> {
    for file in files {
        let seen = (!force).then_some(&file.seen);
        for path in &file.listed {
            if delete(dir, path, seen)? == Done::Changed {
                return Err(changed_file(dir, file, FORCE_REMOVES));
            }
        }
    }
    delete(dir, MANIFEST, None)?;
    Ok(())
}

/// What is at `target`, as `edition` sees it.
///
/// # Errors
///
/// [`SkillError::File`] when it cannot be read.
pub fn status(target: &Target, edition: &Edition) -> Result<Status, SkillError> {
    let mut status = Status {
        dir: target.dir.clone(),
        state: State::NotInstalled,
        version: None,
        edited: Vec::new(),
        missing: Vec::new(),
        unlisted: Vec::new(),
        colliding: Vec::new(),
        in_the_way: Vec::new(),
        link: None,
    };
    if let Some(link) = &target.link {
        status.link = Some((link.place.clone(), link.state()?));
    }
    match place(&target.dir)? {
        Place::Absent => return Ok(status),
        Place::Link | Place::Other(_) => {
            status.state = State::Blocked;
            return Ok(status);
        }
        Place::Directory => {}
    }
    if ensure_owner(target).is_err() {
        status.state = State::OtherOwner;
        return Ok(status);
    }
    let found = survey(&target.dir, edition)?;
    status.state = match &found.installed {
        Installed::No if found.unlisted.is_empty() => State::NotInstalled,
        Installed::No => State::Foreign,
        Installed::Unusable(ManifestError::NewerSchema { .. }) => State::UnknownSchema,
        Installed::Unusable(ManifestError::Malformed { .. }) => State::Damaged,
        Installed::Yes(manifest) => match Freshness::of(&manifest.atl_version, edition.version) {
            Freshness::Older => State::Older,
            Freshness::Newer => State::Newer,
            // An edited or missing file is said to be one; the version is
            // this atl's all the same.
            Freshness::Same if *manifest == Manifest::of(edition) => State::UpToDate,
            Freshness::Same => State::Differs,
        },
    };
    status.edited = found.edited();
    status.missing = found.missing();
    status.colliding = found.colliding();
    status.in_the_way = found.in_the_way();
    status.unlisted = found.unlisted;
    if let Installed::Yes(manifest) = found.installed {
        status.version = Some(manifest.atl_version);
    }
    Ok(status)
}

/// The version of the skill atl installed in the home directory, `None` when
/// it installed none, when that cannot be told, or when it is another user's.
#[must_use]
pub fn installed_version(env: &Environment) -> Option<(Utf8PathBuf, String)> {
    let target = env.global().ok()?;
    ensure_owner(&target).ok()?;
    let dir = target.dir;
    if place(&dir).ok()? != Place::Directory {
        return None;
    }
    match read_manifest(&dir).ok()? {
        Installed::Yes(manifest) => Some((dir, manifest.atl_version)),
        Installed::No | Installed::Unusable(_) => None,
    }
}

/// The line `atl self check` adds about the skill of the home directory, when
/// it was changed or is older than `edition`.
#[must_use]
pub fn installed_note(env: &Environment, edition: &Edition) -> Option<String> {
    let target = env.global().ok()?;
    ensure_owner(&target).ok()?;
    let status = status(&target, edition)
        .map_err(|err| tracing::debug!("the skill was not checked: {err}"))
        .ok()?;
    status.version.as_ref()?;
    let mut said = Vec::new();
    if status.state == State::Older {
        let version = status.version.as_deref().unwrap_or_default();
        said.push(format!("is older (v{version})"));
    }
    if !status.missing.is_empty() {
        said.push(format!("lacks {}", listed(&status.missing)));
    }
    let changed = [status.edited, status.colliding].concat();
    if !changed.is_empty() {
        said.push(format!("was changed ({})", listed(&changed)));
    }
    if !status.in_the_way.is_empty() {
        let in_the_way = listed(&status.in_the_way);
        said.push(format!("has {in_the_way} in the way of atl's files"));
    }
    // What is in the way is not replaced by force.
    let advice = match (changed.is_empty(), status.in_the_way.is_empty()) {
        (true, true) => "run `atl skill update`",
        (false, true) => {
            "atl leaves it alone; `atl skill update --force` writes atl's files in their place"
        }
        (true, false) => "move that away, then run `atl skill update`",
        (false, false) => {
            "atl leaves it alone; move what is in the way away, then `atl skill update --force` \
             writes atl's files in place of the changed ones"
        }
    };
    if said.is_empty() {
        return None;
    }
    Some(format!(
        "atl skill in {} {}: {advice}",
        target.dir,
        said.join(" and ")
    ))
}

/// Write `edition` into the directory of `target`, which is there and locked,
/// when what is in it allows that.
fn write_skill(target: &Target, edition: &Edition, force: bool) -> Result<WriteReport, SkillError> {
    let dir = &target.dir;
    // Again, now that it is locked, or for where nothing is.
    ensure_owner(target)?;
    ensure_directory_or_absent(dir)?;
    let found = survey(dir, edition)?;
    ensure_unblocked(dir, found.in_the_way())?;
    let Survey {
        installed,
        files,
        unlisted,
        ..
    } = &found;
    match installed {
        Installed::Yes(_) => {
            ensure_unchanged(dir, found.edited(), found.colliding(), force, FORCE_WRITES)?;
        }
        Installed::No => {
            if !force && !unlisted.is_empty() {
                return Err(Refusal::Foreign {
                    dir: dir.clone(),
                    files: unlisted.clone(),
                }
                .into());
            }
        }
        Installed::Unusable(ManifestError::NewerSchema { schema }) => {
            return Err(newer_schema(dir, *schema));
        }
        Installed::Unusable(ManifestError::Malformed { reason }) => {
            if !force {
                return Err(Refusal::DamagedManifest {
                    dir: dir.clone(),
                    reason: reason.clone(),
                }
                .into());
            }
        }
    }
    let previous = installed.manifest();
    let (changed, kept) = write_files(dir, edition, previous, files, force)?;
    Ok(WriteReport {
        dir: dir.clone(),
        previous: previous.map(|manifest| manifest.atl_version.clone()),
        version: edition.version.to_owned(),
        changed,
        kept,
        link: None,
    })
}

/// Write the files of `edition` into `dir`, then take out those of `previous`
/// it does not have, with the manifest before and after. `files` is what was
/// seen of all of them. A file that is there already, byte for byte, is not
/// touched, unless the directory has it under another spelling.
///
/// Whether anything changed, and which files of `previous` were kept because
/// they were edited.
///
/// # Errors
///
/// [`Refusal::Changed`] when a file is not what was seen any more and `force`
/// is not given: the files before it are the new ones then, and the next run
/// knows them. [`SkillError::File`] when a file cannot be read or written.
pub(super) fn write_files(
    dir: &Utf8Path,
    edition: &Edition,
    previous: Option<&Manifest>,
    files: &[FileState],
    force: bool,
) -> Result<(bool, Vec<String>), SkillError> {
    let stale: Vec<&FileState> = files
        .iter()
        .filter(|file| match file.in_edition {
            Some(_) => file.verdict != Verdict::Identical,
            None => !file.listed.is_empty() && file.seen != Seen::Missing,
        })
        .collect();
    let finished = Manifest::of(edition);
    if !stale.is_empty() {
        let version = previous.map_or(edition.version, |manifest| &manifest.atl_version);
        let before = Manifest::unfinished(version, written_files(previous, files), edition);
        // Of a list no atl makes: the old one stays, rather than one no atl
        // reads. A run that is cut short is then finished by this version.
        if before.is_within_limits() {
            write_manifest(dir, &before)?;
        }
    }
    let ours: Vec<&str> = files
        .iter()
        .flat_map(|file| {
            file.listed
                .iter()
                .map(String::as_str)
                .chain(file.in_edition)
        })
        .collect();
    let mut changed = false;
    for (file, path) in files
        .iter()
        .filter_map(|file| Some((file, file.in_edition?)))
    {
        let text = edition.text(path).unwrap_or_default();
        changed |= respell(dir, path, &ours)?;
        let seen = (!force).then_some(&file.seen);
        match write(dir, path, text.as_bytes(), seen)? {
            Done::Yes => changed = true,
            Done::No => {}
            Done::Changed => return Err(changed_file(dir, file, FORCE_WRITES)),
        }
    }
    let mut kept = Vec::new();
    for file in stale.into_iter().filter(|file| file.in_edition.is_none()) {
        if file.verdict == Verdict::Edited {
            kept.push(file.path.clone());
            continue;
        }
        match delete(dir, &file.path, Some(&file.seen))? {
            Done::Yes => changed = true,
            Done::No => {}
            Done::Changed if force => kept.push(file.path.clone()),
            Done::Changed => return Err(changed_file(dir, file, FORCE_WRITES)),
        }
    }
    let wrote = write_manifest(dir, &finished)?;
    changed |= wrote && previous != Some(&finished);
    Ok((changed, kept))
}

/// Write the manifest of `dir`, over whatever is there. Whether it is new.
fn write_manifest(dir: &Utf8Path, manifest: &Manifest) -> Result<bool, SkillError> {
    match write(dir, MANIFEST, &manifest.to_bytes(), None)? {
        Done::Yes => Ok(true),
        Done::No => Ok(false),
        Done::Changed => Err(changed_since(
            dir,
            vec![MANIFEST.to_owned()],
            vec![],
            FORCE_WRITES,
        )),
    }
}

/// The files that are there of those `previous` lists, as a manifest has
/// them: with what each of them is, or, of an edited one, with what it was.
fn written_files(previous: Option<&Manifest>, files: &[FileState]) -> BTreeMap<String, String> {
    let mut written = BTreeMap::new();
    for file in files {
        for path in &file.listed {
            let was = previous.and_then(|manifest| manifest.files.get(path));
            let digest = match &file.seen {
                Seen::Missing => continue,
                Seen::File(digest) if file.is_atls() => Some(digest),
                Seen::File(_) | Seen::Other => was,
            };
            if let Some(digest) = digest {
                written.insert(path.clone(), digest.clone());
            }
        }
    }
    written
}

/// The refusal of a change to a file that is not what was seen any more.
fn changed_file(dir: &Utf8Path, file: &FileState, hint: &'static str) -> SkillError {
    let path = vec![file.path.clone()];
    if file.listed.is_empty() {
        changed_since(dir, Vec::new(), path, hint)
    } else {
        changed_since(dir, path, Vec::new(), hint)
    }
}

/// [`Refusal::Changed`].
fn changed_since(
    dir: &Utf8Path,
    edited: Vec<String>,
    colliding: Vec<String>,
    hint: &'static str,
) -> SkillError {
    Refusal::Changed {
        dir: dir.into(),
        edited,
        colliding,
        hint,
    }
    .into()
}

/// Remove the link of `target`, when it is the one atl writes.
fn remove_link(target: &Target) -> Result<Option<Utf8PathBuf>, FileError> {
    match &target.link {
        Some(link) if link.remove()? => Ok(Some(link.place.clone())),
        _ => Ok(None),
    }
}

/// Refuse, as root, the home directory of another user, and as anyone the
/// skill directory of another.
fn ensure_owner(target: &Target) -> Result<(), Refusal> {
    if let Some(owner) = target.foreign_owner {
        return Err(Refusal::ForeignOwner {
            home: target.root.clone(),
            owner,
        });
    }
    match target
        .user
        .and_then(|user| Some((user, other_owner(&target.dir, user)?)))
    {
        None => Ok(()),
        Some((user, (path, owner))) => Err(Refusal::OtherOwner { path, owner, user }),
    }
}

/// Refuse a directory in which a path of the skill cannot hold a file.
fn ensure_unblocked(dir: &Utf8Path, in_the_way: Vec<String>) -> Result<(), Refusal> {
    if in_the_way.is_empty() {
        return Ok(());
    }
    Err(Refusal::Blocked {
        dir: dir.into(),
        paths: in_the_way,
    })
}

/// Refuse a directory atl wrote in which files of its were edited, or others
/// collide with those it has to write.
fn ensure_unchanged(
    dir: &Utf8Path,
    edited: Vec<String>,
    colliding: Vec<String>,
    force: bool,
    hint: &'static str,
) -> Result<(), Refusal> {
    if force || (edited.is_empty() && colliding.is_empty()) {
        return Ok(());
    }
    Err(Refusal::Changed {
        dir: dir.into(),
        edited,
        colliding,
        hint,
    })
}

fn not_ours(dir: &Utf8Path, why: String) -> SkillError {
    Refusal::NotOurs {
        dir: dir.into(),
        why,
    }
    .into()
}

fn newer_schema(dir: &Utf8Path, schema: u64) -> SkillError {
    Refusal::NewerSchema {
        dir: dir.into(),
        schema,
    }
    .into()
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::content::SkillFile;
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;

    const A1: SkillFile = SkillFile {
        path: "SKILL.md",
        text: "one",
    };
    const A2: SkillFile = SkillFile {
        path: "SKILL.md",
        text: "two",
    };
    const OLD_REF: SkillFile = SkillFile {
        path: "references/old.md",
        text: "old",
    };
    const NEW_REF: SkillFile = SkillFile {
        path: "references/new.md",
        text: "new",
    };
    /// Two versions of a skill; the second has another reference file.
    const V1: Edition = Edition {
        version: "2026.1.0",
        files: &[A1, OLD_REF],
    };
    const V2: Edition = Edition {
        version: "2026.2.0",
        files: &[A2, NEW_REF],
    };

    /// A skill target in a directory of its own, without a link.
    fn target() -> (tempfile::TempDir, Target) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("UTF-8");
        let target = Target {
            dir: root.join(".agents/skills/atl"),
            root,
            link: None,
            foreign_owner: None,
            user: None,
        };
        (tmp, target)
    }

    fn names(target: &Target) -> Vec<String> {
        super::super::testing::entries_under(target.dir.as_std_path())
    }

    fn refusal<T: std::fmt::Debug>(result: Result<T, SkillError>) -> Refusal {
        match result {
            Err(SkillError::Refused(refusal)) => refusal,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_update_replaces_adds_and_takes_out_files() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        assert_eq!(status(&target, &V2).expect("status").state, State::Older);
        let report = update(&target, &V2, false)
            .expect("update")
            .expect("installed");
        assert_eq!(report.previous.as_deref(), Some("2026.1.0"));
        assert!(report.changed && report.kept.is_empty());
        assert_eq!(
            names(&target),
            [
                ".atl-skill.json",
                "SKILL.md",
                "references",
                "references/new.md"
            ]
        );
        assert_eq!(status(&target, &V2).expect("status").state, State::UpToDate);
        assert_eq!(status(&target, &V1).expect("status").state, State::Newer);
    }

    #[test]
    fn an_explicit_update_goes_back_too() {
        let (_tmp, target) = target();
        install(&target, &V2, false).expect("install");
        update(&target, &V1, false).expect("update");
        assert_eq!(status(&target, &V1).expect("status").state, State::UpToDate);
    }

    #[test]
    fn an_edited_file_the_new_version_does_not_have_is_kept() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        fs::write(target.dir.join(OLD_REF.path), "mine").expect("write");
        assert!(matches!(
            refusal(update(&target, &V2, false)),
            Refusal::Changed { .. }
        ));
        let report = update(&target, &V2, true)
            .expect("update")
            .expect("installed");
        assert_eq!(report.kept, [OLD_REF.path]);
        assert_eq!(
            status(&target, &V2).expect("status").unlisted,
            [OLD_REF.path]
        );
        // Nobody's, and in nobody's way.
        let report = update(&target, &V2, false)
            .expect("update")
            .expect("installed");
        assert!(!report.changed);
        let report = remove(&target, &V2, false).expect("remove");
        assert_eq!(report.left, [OLD_REF.path]);
        assert!(report.dir_kept);
    }

    #[test]
    fn files_that_are_not_atls_stop_nothing_and_stay() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        let notes = target.dir.join("references/notes.md");
        fs::write(&notes, "mine").expect("write");
        std::os::unix::fs::symlink("/nowhere", target.dir.join("link")).expect("symlink");
        install(&target, &V1, false).expect("install again");
        update(&target, &V2, false).expect("update");
        let status = status(&target, &V2).expect("status");
        assert_eq!(status.state, State::UpToDate);
        assert_eq!(status.unlisted, ["link", "references/notes.md"]);
        let report = remove(&target, &V2, false).expect("remove");
        assert!(report.dir_kept);
        assert_eq!(report.left, ["link", "references/notes.md"]);
        assert_eq!(
            names(&target),
            ["link", "references", "references/notes.md"]
        );
        assert_eq!(fs::read_to_string(&notes).expect("read"), "mine");
    }

    #[test]
    fn a_file_where_the_new_version_has_one_stops_the_update() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        let at = target.dir.join(NEW_REF.path);
        fs::write(&at, "mine").expect("write");
        assert_eq!(
            refusal(update(&target, &V2, false)),
            Refusal::Changed {
                dir: target.dir.clone(),
                edited: Vec::new(),
                colliding: vec![NEW_REF.path.to_owned()],
                hint: FORCE_WRITES,
            }
        );
        // V1 has no such file, and neither has the manifest.
        update(&target, &V1, false).expect("update");
        update(&target, &V2, true).expect("forced update");
        assert_eq!(fs::read_to_string(&at).expect("read"), NEW_REF.text);
    }

    /// The files of the new version are there, the manifest is the old one's:
    /// what a run leaves that was cut short.
    #[test]
    fn a_run_that_was_cut_short_is_repaired_without_force() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        fs::write(target.dir.join(A2.path), A2.text).expect("write");
        fs::remove_file(target.dir.join(OLD_REF.path)).expect("rm");
        fs::write(target.dir.join(".tmpAb12Cd"), "half").expect("write");
        assert!(update(&target, &V2, false).expect("update").is_some());
        assert_eq!(status(&target, &V2).expect("status").state, State::UpToDate);

        // Of a first install: no manifest yet.
        fs::remove_file(target.dir.join(MANIFEST)).expect("rm");
        assert!(install(&target, &V2, false).expect("install").changed);
    }

    #[test]
    fn an_unusable_manifest_stops_everything_but_a_forced_write() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        let manifest = target.dir.join(MANIFEST);
        fs::write(&manifest, "{}").expect("write");
        assert_eq!(status(&target, &V1).expect("status").state, State::Damaged);
        assert!(matches!(
            refusal(update(&target, &V1, false)),
            Refusal::DamagedManifest { .. }
        ));
        assert!(matches!(
            refusal(remove(&target, &V1, true)),
            Refusal::NotOurs { .. }
        ));
        install(&target, &V1, true).expect("forced install");

        fs::write(&manifest, r#"{"schema": 2}"#).expect("write");
        for refused in [
            refusal(install(&target, &V1, true)),
            refusal(update(&target, &V1, true)),
            refusal(remove(&target, &V1, true)),
        ] {
            assert!(matches!(refused, Refusal::NewerSchema { schema: 2, .. }));
        }
    }

    #[test]
    fn a_forced_write_does_not_list_the_files_it_kept() {
        let (_tmp, target) = target();
        fs::create_dir_all(&target.dir).expect("mkdir");
        fs::write(target.dir.join("notes.md"), "theirs").expect("write");
        install(&target, &V1, true).expect("install");
        let Installed::Yes(manifest) =
            super::super::survey::read_manifest(&target.dir).expect("manifest")
        else {
            panic!("no manifest");
        };
        assert_eq!(manifest, Manifest::of(&V1));
    }

    /// The line of `atl self check` for each thing there is to say.
    #[test]
    fn a_note_says_what_is_there_and_nothing_else() {
        type Change = fn(&Utf8Path);
        let write = |dir: &Utf8Path, path: &str| fs::write(dir.join(path), "mine").expect("write");
        let cases: [(Change, Option<&str>); 13] = [
            (|_| {}, Some("is older (v2026.1.0): run `atl skill update`")),
            (
                |dir| fs::remove_file(dir.join("SKILL.md")).expect("rm"),
                Some("is older (v2026.1.0) and lacks SKILL.md: run `atl skill update`"),
            ),
            (
                |dir| fs::write(dir.join(OLD_REF.path), "mine").expect("write"),
                Some("is older (v2026.1.0) and was changed (references/old.md): FORCE"),
            ),
            (
                |dir| fs::write(dir.join(NEW_REF.path), "mine").expect("write"),
                Some("is older (v2026.1.0) and was changed (references/new.md): FORCE"),
            ),
            (
                |dir| fs::create_dir(dir.join(NEW_REF.path)).expect("mkdir"),
                Some(
                    "is older (v2026.1.0) and has references/new.md in the way of atl's files: \
                     move that away, then run `atl skill update`",
                ),
            ),
            (
                |dir| {
                    fs::create_dir(dir.join(NEW_REF.path)).expect("mkdir");
                    fs::write(dir.join("SKILL.md"), "mine").expect("write");
                },
                Some(
                    "is older (v2026.1.0) and was changed (SKILL.md) and has references/new.md in \
                     the way of atl's files: atl leaves it alone; move what is in the way away, then \
                     `atl skill update --force` writes atl's files in place of the changed ones",
                ),
            ),
            (
                |dir| {
                    fs::rename(dir.join("references"), dir.join("aside")).expect("mv");
                    fs::write(dir.join("references"), "mine").expect("write");
                },
                Some(
                    "is older (v2026.1.0) and has references in the way of atl's files: \
                     move that away, then run `atl skill update`",
                ),
            ),
            (
                |dir| {
                    fs::remove_file(dir.join("SKILL.md")).expect("rm");
                    fs::write(dir.join(OLD_REF.path), "mine").expect("write");
                },
                Some(
                    "is older (v2026.1.0) and lacks SKILL.md and was changed (references/old.md): \
                     FORCE",
                ),
            ),
            (
                |dir| fs::write(dir.join("notes.md"), "mine").expect("write"),
                Some("is older (v2026.1.0): run `atl skill update`"),
            ),
            (
                |dir| fs::write(dir.join(MANIFEST), "{}").expect("write"),
                None,
            ),
            (
                |dir| fs::write(dir.join(MANIFEST), r#"{"schema": 2}"#).expect("write"),
                None,
            ),
            (|dir| fs::remove_file(dir.join(MANIFEST)).expect("rm"), None),
            (
                |dir| {
                    let aside = dir.with_file_name("aside");
                    fs::rename(dir, &aside).expect("mv");
                    std::os::unix::fs::symlink(&aside, dir).expect("symlink");
                },
                None,
            ),
        ];
        for (n, (change, expected)) in cases.into_iter().enumerate() {
            let (_tmp, target) = target();
            let env = Environment {
                home: Some(target.root.as_str().into()),
                ..Environment::default()
            };
            install(&target, &V1, false).expect("install");
            change(&target.dir);
            let expected = expected.map(|text| {
                let text = text.replace(
                    "FORCE",
                    "atl leaves it alone; `atl skill update --force` writes atl's files in their place",
                );
                format!("atl skill in {} {text}", target.dir)
            });
            assert_eq!(installed_note(&env, &V2), expected, "case {n}");
        }

        // Of this version, or of a newer one: only what was changed.
        let (_tmp, target) = target();
        let env = Environment {
            home: Some(target.root.as_str().into()),
            ..Environment::default()
        };
        install(&target, &V2, false).expect("install");
        assert_eq!(installed_note(&env, &V2), None);
        assert_eq!(installed_note(&env, &V1), None);
        write(&target.dir, "SKILL.md");
        for edition in [&V2, &V1] {
            let note = installed_note(&env, edition).expect("a note");
            assert!(
                note.contains(" was changed (SKILL.md): atl leaves it alone"),
                "{note}"
            );
            assert!(!note.contains("is older"), "{note}");
        }
        assert_eq!(installed_note(&Environment::default(), &V1), None);
    }

    /// What is looked at again right before a write: a link that came into
    /// the place of a directory after the survey is not gone through.
    #[test]
    fn a_link_that_appears_after_the_survey_is_not_written_through() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        let seen = survey(&target.dir, &V2).expect("survey").files;
        let outside = target.root.join("outside");
        fs::rename(target.dir.join("references"), &outside).expect("mv");
        std::os::unix::fs::symlink(&outside, target.dir.join("references")).expect("symlink");
        let err = write_files(&target.dir, &V2, Some(&Manifest::of(&V1)), &seen, false)
            .expect_err("blocked");
        assert!(
            matches!(
                &err,
                SkillError::File(FileError::Write { path, .. })
                    if path.ends_with(NEW_REF.path)
            ),
            "{err}"
        );
        assert!(!outside.join("new.md").exists());
        assert!(outside.join("old.md").exists());

        // In the place of a file: the link goes, what it leads to stays.
        fs::remove_file(target.dir.join("references")).expect("rm");
        let theirs = target.root.join("theirs.md");
        fs::write(&theirs, "theirs").expect("write");
        fs::remove_file(target.dir.join("SKILL.md")).expect("rm");
        std::os::unix::fs::symlink(&theirs, target.dir.join("SKILL.md")).expect("symlink");
        write_files(&target.dir, &V2, None, &seen, true).expect("written");
        assert_eq!(fs::read_to_string(&theirs).expect("read"), "theirs");
        assert!(!target.dir.join("SKILL.md").is_symlink());
    }

    /// What a user saves after the survey, before atl comes to the file: the
    /// command stops there, and the next one says why.
    #[test]
    fn what_is_saved_after_the_survey_is_not_lost() {
        type Check = fn(&Target, &Survey) -> Refusal;
        let update: Check = |target, seen| {
            let previous = seen.installed.manifest();
            refusal(write_files(&target.dir, &V2, previous, &seen.files, false))
        };
        let remove: Check = |target, seen| refusal(delete_files(&target.dir, &seen.files, false));
        for (path, check, edited, hint) in [
            (A2.path, update, true, FORCE_WRITES),
            (OLD_REF.path, update, true, FORCE_WRITES), // to be taken out
            (NEW_REF.path, update, false, FORCE_WRITES), // not there yet
            (A1.path, remove, true, FORCE_REMOVES),
            (OLD_REF.path, remove, true, FORCE_REMOVES),
        ] {
            let (_tmp, target) = target();
            install(&target, &V1, false).expect("install");
            let seen = survey(&target.dir, &V2).expect("survey");
            let at = target.dir.join(path);
            fs::write(&at, "mine").expect("write");
            let named = |is: bool| {
                if is {
                    vec![path.to_owned()]
                } else {
                    Vec::new()
                }
            };
            let expected = || Refusal::Changed {
                dir: target.dir.clone(),
                edited: named(edited),
                colliding: named(!edited),
                hint,
            };
            assert_eq!(check(&target, &seen), expected(), "{path}");
            assert_eq!(fs::read_to_string(&at).expect("read"), "mine", "{path}");
            // What was written before it is no edit of anyone's, and a file
            // atl did not come to write is none of its.
            let seen = survey(&target.dir, &V2).expect("survey");
            assert_eq!(seen.edited(), named(edited), "{path}");
            assert_eq!(seen.colliding(), named(!edited), "{path}");
            fs::remove_file(&at).expect("rm");
            super::update(&target, &V2, false).expect("update");
            assert_eq!(
                names(&target),
                [MANIFEST, A2.path, "references", NEW_REF.path]
            );
        }
    }

    /// With `--force`, what is there is replaced, but for a file this version
    /// does not have.
    #[test]
    fn what_is_saved_after_the_survey_gives_way_to_force() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        let seen = survey(&target.dir, &V2).expect("survey");
        for path in [A2.path, NEW_REF.path, OLD_REF.path] {
            fs::write(target.dir.join(path), "mine").expect("write");
        }
        let previous = seen.installed.manifest();
        let (changed, kept) =
            write_files(&target.dir, &V2, previous, &seen.files, true).expect("written");
        assert!(changed);
        assert_eq!(kept, [OLD_REF.path]);
        for (path, text) in [
            (A2.path, A2.text),
            (NEW_REF.path, NEW_REF.text),
            (OLD_REF.path, "mine"),
        ] {
            assert_eq!(
                fs::read_to_string(target.dir.join(path)).expect("read"),
                text
            );
        }
        let seen = survey(&target.dir, &V2).expect("survey");
        fs::write(target.dir.join(A2.path), "mine").expect("write");
        delete_files(&target.dir, &seen.files, true).expect("deleted");
        assert!(!target.dir.join(A2.path).exists());
    }

    /// Version after version that does not come to its end, cut short at each
    /// of its steps: the files of each are atl's to all that follow, and a
    /// file it took out, or had not made yet, was not removed by a user.
    #[test]
    fn a_run_that_was_cut_short_is_finished_by_another_version() {
        /// V2, newer, with the file it has not.
        const V3: Edition = Edition {
            version: "2026.3.0",
            files: &[
                SkillFile {
                    path: "SKILL.md",
                    text: "three",
                },
                OLD_REF,
            ],
        };
        // The files in the order they are written and taken out.
        let cut_short = |target: &Target, edition: &Edition, steps: usize| {
            let seen = survey(&target.dir, edition).expect("survey");
            assert_eq!(seen.edited(), Vec::<String>::new());
            let manifest = seen.installed.manifest().expect("a manifest");
            let written = written_files(Some(manifest), &seen.files);
            let unfinished = Manifest::unfinished(&manifest.atl_version, written, edition);
            let mut done = 0;
            let mut step = |act: &mut dyn FnMut()| {
                if done < steps {
                    act();
                }
                done += 1;
            };
            step(&mut || {
                let bytes = unfinished.to_bytes();
                write(&target.dir, MANIFEST, &bytes, None).expect("manifest");
            });
            for file in edition.files {
                step(&mut || {
                    write(&target.dir, file.path, file.text.as_bytes(), None).expect("write");
                });
            }
            for file in seen.files.iter().filter(|file| file.in_edition.is_none()) {
                step(&mut || {
                    delete(&target.dir, &file.path, None).expect("delete");
                });
            }
        };
        for first in 0..=4 {
            for second in 0..=4 {
                let (_tmp, target) = target();
                install(&target, &V1, false).expect("install");
                cut_short(&target, &V2, first);
                cut_short(&target, &V3, second);
                let at = format!("{first}, {second}");
                // Any version sees nothing to ask about.
                let Installed::Yes(manifest) = read_manifest(&target.dir).expect("manifest") else {
                    panic!("no manifest");
                };
                for edition in [&V1, &V2, &V3] {
                    let found = survey(&target.dir, edition).expect("survey");
                    assert!(
                        !found.files.iter().any(FileState::is_change),
                        "{at}: {found:?}"
                    );
                    let found = status(&target, edition).expect("status");
                    // What was yet to be written, and this edition has too.
                    let is_unwritten = |path: &&str| {
                        manifest.unfinished.contains_key(*path)
                            && !manifest.files.contains_key(*path)
                            && !target.dir.join(path).exists()
                    };
                    let mut unwritten: Vec<&str> = edition.files.iter().map(|f| f.path).collect();
                    unwritten.retain(is_unwritten);
                    assert_eq!(found.missing, unwritten, "{at}");
                    assert_eq!(found.unlisted, Vec::<String>::new(), "{at}");
                    let is_whole = found.state == State::UpToDate;
                    assert_eq!(
                        is_whole,
                        at == "0, 0" && edition.version == V1.version,
                        "{at}"
                    );
                }
                if second % 2 == 0 {
                    super::update(&target, &V2, false).expect("update");
                    let expected = [MANIFEST, A2.path, "references", NEW_REF.path];
                    assert_eq!(names(&target), expected, "{at}");
                    let state = status(&target, &V2).expect("status").state;
                    assert_eq!(state, State::UpToDate, "{at}");
                } else {
                    remove(&target, &V2, false).expect("remove");
                    assert!(!target.dir.exists(), "{at}");
                }
            }
        }
    }

    /// Of a first install, too, whose files are another version's then.
    #[test]
    fn a_first_install_that_was_cut_short_is_finished_by_another_version() {
        let (_tmp, target) = target();
        fs::create_dir_all(&target.dir).expect("mkdir");
        let seen = survey(&target.dir, &V1).expect("survey").files;
        let unfinished = Manifest::unfinished(V1.version, written_files(None, &seen), &V1);
        fs::write(target.dir.join(MANIFEST), unfinished.to_bytes()).expect("write");
        fs::write(target.dir.join(A1.path), A1.text).expect("write");
        let status = status(&target, &V2).expect("status");
        assert_eq!(
            (status.state, status.edited, status.missing),
            (State::Older, vec![], vec![])
        );
        install(&target, &V2, false).expect("install");
        assert_eq!(
            names(&target),
            [MANIFEST, A2.path, "references", NEW_REF.path]
        );
    }

    /// A file at a path a run was yet to write is the user's: no command takes
    /// it for atl's, and none deletes it, `--force` or not.
    #[test]
    fn what_a_run_did_not_come_to_write_is_not_atls() {
        // The user's own directory, taken over by a run that got as far as
        // the manifest and, or not, a file of its.
        for written in [false, true] {
            for path in [A1.path, OLD_REF.path] {
                for edition in [&V1, &V2] {
                    for force in [false, true] {
                        let (_tmp, target) = target();
                        fs::create_dir_all(target.dir.join("references")).expect("mkdir");
                        fs::write(target.dir.join(path), "mine").expect("write");
                        let seen = survey(&target.dir, &V1).expect("survey").files;
                        let written_before = written_files(None, &seen);
                        let manifest = Manifest::unfinished(V1.version, written_before, &V1);
                        fs::write(target.dir.join(MANIFEST), manifest.to_bytes()).expect("write");
                        let other = if path == A1.path { OLD_REF } else { A1 };
                        if written {
                            fs::write(target.dir.join(other.path), other.text).expect("write");
                        }
                        let at = format!("{path}, {written}, {}, {force}", edition.version);

                        let found = status(&target, edition).expect("status");
                        assert_eq!(found.edited, Vec::<String>::new(), "{at}");
                        assert_eq!(found.unlisted, [path], "{at}");
                        let has = edition.text(path).is_some();
                        let expected: &[&str] = if has { &[path] } else { &[] };
                        assert_eq!(found.colliding, expected, "{at}");

                        let report = remove(&target, edition, force).expect("remove");
                        assert_eq!(report.left, [path], "{at}");
                        assert!(!target.dir.join(other.path).exists(), "{at}");
                        let kept = fs::read_to_string(target.dir.join(path)).expect("read");
                        assert_eq!(kept, "mine", "{at}");
                    }
                }
            }
        }
    }

    /// Nor by an update: where this version has the file, it is asked about;
    /// where it has not, the file is in no one's way and stays.
    #[test]
    fn an_update_keeps_what_a_run_did_not_come_to_write() {
        let cut_short = || {
            let (tmp, target) = target();
            fs::create_dir_all(target.dir.join("references")).expect("mkdir");
            fs::write(target.dir.join(OLD_REF.path), "mine").expect("write");
            let manifest = Manifest::unfinished(V1.version, BTreeMap::new(), &V1);
            fs::write(target.dir.join(MANIFEST), manifest.to_bytes()).expect("write");
            (tmp, target)
        };
        let (_tmp, target) = cut_short();
        assert_eq!(
            refusal(super::update(&target, &V1, false)),
            Refusal::Changed {
                dir: target.dir.clone(),
                edited: Vec::new(),
                colliding: vec![OLD_REF.path.to_owned()],
                hint: FORCE_WRITES,
            }
        );
        for force in [false, true] {
            let (_tmp, target) = cut_short();
            let report = super::update(&target, &V2, force)
                .expect("update")
                .expect("installed");
            assert_eq!(report.kept, Vec::<String>::new());
            let status = status(&target, &V2).expect("status");
            assert_eq!(status.state, State::UpToDate);
            assert_eq!(status.unlisted, [OLD_REF.path]);
            let kept = fs::read_to_string(target.dir.join(OLD_REF.path)).expect("read");
            assert_eq!(kept, "mine");
        }
        // A directory or a link there is in no one's way either.
        type Make = fn(&Utf8Path);
        let cases: [Make; 3] = [
            |at| fs::create_dir(at).expect("mkdir"),
            |at| std::os::unix::fs::symlink("/nowhere", at).expect("symlink"),
            |at| {
                fs::create_dir(at).expect("mkdir");
                fs::write(at.join("mine"), "mine").expect("write");
            },
        ];
        for make in cases {
            let (_tmp, target) = cut_short();
            let at = target.dir.join(OLD_REF.path);
            fs::remove_file(&at).expect("rm");
            make(&at);
            super::update(&target, &V2, false).expect("update");
            assert!(fs::symlink_metadata(&at).is_ok());
            remove(&target, &V2, true).expect("remove");
            assert!(fs::symlink_metadata(&at).is_ok());
        }
    }

    /// With `--force`, a file the user makes where a run was yet to write one
    /// is not deleted by a `remove` that had seen nothing there.
    #[test]
    fn what_comes_where_nothing_was_written_yet_is_not_deleted() {
        let (_tmp, target) = target();
        fs::create_dir_all(&target.dir).expect("mkdir");
        let manifest = Manifest::unfinished(V1.version, BTreeMap::new(), &V1);
        fs::write(target.dir.join(MANIFEST), manifest.to_bytes()).expect("write");
        let seen = survey(&target.dir, &V1).expect("survey");
        assert_eq!(seen.missing(), [A1.path, OLD_REF.path]);
        let at = target.dir.join(A1.path);
        fs::write(&at, "mine").expect("write");
        let error = refusal(write_files(
            &target.dir,
            &V1,
            Some(&manifest),
            &seen.files,
            false,
        ));
        assert!(
            matches!(&error, Refusal::Changed { edited, colliding, .. }
                if edited.is_empty() && *colliding == [A1.path]),
            "{error:?}"
        );
        delete_files(&target.dir, &seen.files, true).expect("deleted");
        assert_eq!(fs::read_to_string(&at).expect("read"), "mine");
    }

    /// A list no atl makes is not made longer than one atl reads: a run over it
    /// that stops half-way leaves a manifest, and the next one goes on.
    #[test]
    fn a_manifest_is_never_written_that_is_not_read_again() {
        for count in [254, 255, 256] {
            let (_tmp, target) = target();
            fs::create_dir_all(&target.dir).expect("mkdir");
            let mut files = BTreeMap::new();
            for n in 0..count {
                let path = format!("f{n}");
                fs::write(target.dir.join(&path), &path).expect("write");
                files.insert(
                    path.clone(),
                    super::super::manifest::sha256_hex(path.as_bytes()),
                );
            }
            let previous = Manifest {
                atl_version: V1.version.to_owned(),
                files,
                unfinished: BTreeMap::new(),
            };
            fs::write(target.dir.join(MANIFEST), previous.to_bytes()).expect("write");
            let seen = survey(&target.dir, &V2).expect("survey");
            assert_eq!(seen.installed, Installed::Yes(previous.clone()));
            // In the way of the second file.
            fs::write(target.dir.join("references"), "mine").expect("write");
            write_files(&target.dir, &V2, Some(&previous), &seen.files, false)
                .expect_err("in the way");
            assert_eq!(
                fs::read_to_string(target.dir.join(A2.path)).expect("read"),
                A2.text
            );
            let left = read_manifest(&target.dir).expect("read");
            assert!(matches!(left, Installed::Yes(_)), "{count}: {left:?}");
            fs::remove_file(target.dir.join("references")).expect("rm");
            super::update(&target, &V2, false).expect("update");
            assert_eq!(
                names(&target),
                [MANIFEST, A2.path, "references", NEW_REF.path]
            );
        }
    }

    /// A user's edit is one before, during and after a forced run that did
    /// not come to its end.
    #[test]
    fn an_edit_is_one_after_a_forced_run_that_was_cut_short() {
        let (_tmp, target) = target();
        install(&target, &V1, false).expect("install");
        for path in [A1.path, OLD_REF.path] {
            fs::write(target.dir.join(path), "mine").expect("write");
        }
        let seen = survey(&target.dir, &V2).expect("survey");
        let previous = seen.installed.manifest();
        let written = written_files(previous, &seen.files);
        assert_eq!(written, Manifest::of(&V1).files);
        let unfinished = Manifest::unfinished(V1.version, written, &V2);
        fs::write(target.dir.join(MANIFEST), unfinished.to_bytes()).expect("write");
        let seen = survey(&target.dir, &V2).expect("survey");
        assert_eq!(seen.edited(), [A1.path, OLD_REF.path]);
    }

    #[test]
    fn a_skill_directory_of_another_user_is_refused() {
        let (_tmp, mut target) = target();
        install(&target, &V1, false).expect("install");
        let owner = super::super::testing::not_roots(target.dir.as_std_path());
        target.user = Some(owner + 1);
        let before = names(&target);
        let expected = Refusal::OtherOwner {
            path: target.dir.clone(),
            owner,
            user: owner + 1,
        };
        for force in [false, true] {
            assert_eq!(refusal(install(&target, &V2, force)), expected);
            assert_eq!(refusal(super::update(&target, &V2, force)), expected);
            assert_eq!(refusal(remove(&target, &V2, force)), expected);
            // And after the lock, for what came into the place before it.
            assert_eq!(refusal(write_skill(&target, &V2, force)), expected);
            let mut report = RemoveReport {
                dir: target.dir.clone(),
                version: None,
                left: Vec::new(),
                other: Vec::new(),
                dir_kept: false,
                link: None,
            };
            let removed = remove_locked(&target, &V2, force, &mut report);
            assert_eq!(refusal(removed), expected);
        }
        assert_eq!(names(&target), before);
        assert_eq!(
            expected.to_string(),
            format!(
                "{} belongs to the user with id {owner}, and atl runs as the user with id {}: \
                 atl does not touch the skill there",
                target.dir,
                owner + 1
            )
        );
        // Nothing in it is looked at.
        let found = status(&target, &V2).expect("status");
        assert_eq!((found.state, found.version), (State::OtherOwner, None));
        fs::write(target.dir.join(MANIFEST), "{}").expect("write");
        let found = status(&target, &V2).expect("status");
        assert_eq!(found.state, State::OtherOwner);
        fs::write(target.dir.join(MANIFEST), Manifest::of(&V1).to_bytes()).expect("write");
        let env = Environment {
            home: Some(target.root.as_str().into()),
            euid: target.user,
            ..Environment::default()
        };
        assert_eq!(installed_version(&env), None);
        assert_eq!(installed_note(&env, &V2), None);
        target.user = Some(owner);
        assert!(super::update(&target, &V2, false).is_ok());
    }

    /// As root, where a directory would be made in another user's, none is.
    #[test]
    fn no_skill_directory_is_made_in_another_users() {
        let (_tmp, mut target) = target();
        let owner = super::super::testing::not_roots(target.root.as_std_path());
        target.user = Some(0);
        for force in [false, true] {
            assert_eq!(
                refusal(install(&target, &V1, force)),
                Refusal::OtherOwner {
                    path: target.root.clone(),
                    owner,
                    user: 0,
                }
            );
        }
        assert!(!target.root.join(".agents").exists());
        let found = status(&target, &V1).expect("status");
        assert_eq!(found.state, State::NotInstalled, "there is no directory");
    }

    #[test]
    fn a_note_is_made_of_a_skill_that_is_older_or_changed() {
        let home = tempfile::tempdir().expect("tempdir");
        let env = Environment {
            home: Some(home.path().into()),
            ..Environment::default()
        };
        let target = env.global().expect("target");
        assert_eq!(installed_note(&env, &V1), None);
        install(&target, &V1, false).expect("install");
        assert_eq!(installed_note(&env, &V1), None);
        let dir = &target.dir;
        assert_eq!(
            installed_note(&env, &V2),
            Some(format!(
                "atl skill in {dir} is older (v2026.1.0): run `atl skill update`"
            ))
        );
        fs::write(dir.join("SKILL.md"), "mine").expect("write");
        assert_eq!(
            installed_note(&env, &V2),
            Some(format!(
                "atl skill in {dir} is older (v2026.1.0) and was changed (SKILL.md): atl leaves it \
                 alone; `atl skill update --force` writes atl's files in their place"
            ))
        );
        assert_eq!(
            installed_version(&env),
            Some((dir.clone(), "2026.1.0".to_owned()))
        );
    }
}
