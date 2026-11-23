//! `.atl-skill.json`: what atl wrote into a skill directory.
//!
//! ```json
//! {
//!   "schema": 1,
//!   "atl_version": "2026.48.1",
//!   "files": {
//!     "SKILL.md": "<SHA-256, lowercase hex>",
//!     "references/jira-commands.md": "…"
//!   }
//! }
//! ```
//!
//! While a run is at work, and after one that was cut short, `unfinished`
//! lists the same of the files it writes: each of them may be there or not
//! yet, and those only `files` has may be gone already.
//!
//! The file is data from a disk anyone may have written to: the paths in it
//! are what `remove` deletes and `update` replaces, so a manifest with a path
//! that is not a plain relative one is refused whole ([`Manifest::parse`]).

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::content::Edition;

/// Name of the manifest in the skill directory.
pub const MANIFEST: &str = ".atl-skill.json";

/// The manifest schema this atl reads and writes.
pub const SCHEMA: u64 = 1;

/// The largest manifest. One within the limits below is never larger, as atl
/// writes it.
pub(super) const MAX_BYTES: u64 = 160 * 1024;

/// How many files a manifest lists at most, `files` and `unfinished` together.
const MAX_FILES: usize = 256;

/// The longest version of a manifest, in bytes.
const MAX_VERSION_BYTES: usize = 64;

/// The longest path of a manifest, in bytes: what every file system takes as
/// a name.
const MAX_PATH_BYTES: usize = 255;

/// Length of a SHA-256 digest in hex digits.
const SHA256_HEX_LEN: usize = 64;

/// Why a manifest cannot be used.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ManifestError {
    /// Written by a newer atl in a schema this one does not know.
    #[error("schema {schema} is newer than the {SCHEMA} this atl knows")]
    NewerSchema {
        /// The schema number in the file.
        schema: u64,
    },
    /// Not a manifest atl could have written.
    #[error("{reason}")]
    Malformed {
        /// What is wrong.
        reason: String,
    },
}

/// Which files atl wrote into a skill directory, and from which version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The atl version that wrote the files.
    pub atl_version: String,
    /// SHA-256 (lowercase hex) of every file, by its path in the directory.
    pub files: BTreeMap<String, String>,
    /// The same of the edition that a run was writing over them, which has
    /// not come to its end. Empty when there is none.
    pub unfinished: BTreeMap<String, String>,
}

impl Manifest {
    /// The manifest of a directory that holds `edition`.
    #[must_use]
    pub fn of(edition: &Edition) -> Self {
        Self {
            atl_version: edition.version.to_owned(),
            files: edition
                .files
                .iter()
                .map(|file| (file.path.to_owned(), sha256_hex(file.text.as_bytes())))
                .collect(),
            unfinished: BTreeMap::new(),
        }
    }

    /// The manifest of a directory over whose `files`, of `atl_version`,
    /// `edition` is being written.
    #[must_use]
    pub fn unfinished(
        atl_version: &str,
        files: BTreeMap<String, String>,
        edition: &Edition,
    ) -> Self {
        Self {
            atl_version: atl_version.to_owned(),
            files,
            unfinished: Self::of(edition).files,
        }
    }

    /// Whether it lists no more files than a manifest may. Of one that was
    /// read that is so; what atl makes of one need not be.
    pub(super) fn is_within_limits(&self) -> bool {
        self.files.len() + self.unfinished.len() <= MAX_FILES
    }

    /// The path of every file it lists, sorted, each once.
    pub(super) fn paths(&self) -> impl Iterator<Item = &str> {
        let paths: std::collections::BTreeSet<&str> = self
            .files
            .keys()
            .chain(self.unfinished.keys())
            .map(String::as_str)
            .collect();
        paths.into_iter()
    }

    /// Every SHA-256 the file at `path` may have without having been edited.
    pub(super) fn digests(&self, path: &str) -> impl Iterator<Item = &str> {
        [self.files.get(path), self.unfinished.get(path)]
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    /// Whether the file at `path`, which it lists, may be gone without having
    /// been removed by a user.
    pub(super) fn may_lack(&self, path: &str) -> bool {
        // Of one that both list, the run wrote this edition over that one.
        let in_both = self.files.contains_key(path) && self.unfinished.contains_key(path);
        !self.unfinished.is_empty() && !in_both
    }

    /// Read a manifest.
    ///
    /// # Errors
    ///
    /// [`ManifestError::NewerSchema`] for a schema above [`SCHEMA`], whatever
    /// else the file holds; [`ManifestError::Malformed`] for anything that is
    /// not a manifest: no JSON object, a field missing or of another type, a
    /// version that is none, a digest that is none, a path that is not a
    /// plain relative one or names the manifest itself.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        let malformed = |reason: &str| ManifestError::Malformed {
            reason: reason.to_owned(),
        };
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|err| malformed(&format!("it is not JSON: {err}")))?;
        match value.get("schema").and_then(serde_json::Value::as_u64) {
            Some(SCHEMA) => {}
            Some(schema) if schema > SCHEMA => {
                return Err(ManifestError::NewerSchema { schema });
            }
            _ => return Err(malformed("it has no schema number")),
        }
        let atl_version = value
            .get("atl_version")
            .and_then(serde_json::Value::as_str)
            .filter(|version| is_version(version))
            .ok_or_else(|| malformed("it names no atl version"))?
            .to_owned();
        let files = value
            .get("files")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| malformed("it lists no files"))?;
        let unfinished = match value.get("unfinished") {
            None => BTreeMap::new(),
            Some(value) => value
                .as_object()
                .ok_or_else(|| malformed("it lists no unfinished files"))
                .and_then(listed)?,
        };
        let manifest = Self {
            atl_version,
            files: listed(files)?,
            unfinished,
        };
        if !manifest.is_within_limits() {
            return Err(malformed(&format!("it lists more than {MAX_FILES} files")));
        }
        Ok(manifest)
    }

    /// The bytes of the manifest file.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut value = serde_json::json!({
            "schema": SCHEMA,
            "atl_version": self.atl_version,
            "files": self.files,
        });
        if !self.unfinished.is_empty() {
            value["unfinished"] = serde_json::json!(self.unfinished);
        }
        format!("{value:#}\n").into_bytes()
    }
}

/// The SHA-256 of `bytes`, in lowercase hex.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(SHA256_HEX_LEN);
    for byte in digest.iter() {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// Whether `installed` is newer than `current`: their components compared as
/// numbers, one by one, a missing one being 0. A component reads as the
/// digits it starts with (`1-rc1` as 1), so a version from a disk is
/// [`is_version`]-checked before: anything else would read as 0, the oldest
/// there is.
pub(super) fn is_newer(installed: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|c| {
                c.split(|ch: char| !ch.is_ascii_digit())
                    .next()
                    .unwrap_or("")
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    };
    let (a, b) = (parse(installed), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// The files of a manifest, by path.
fn listed(
    files: &serde_json::Map<String, serde_json::Value>,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let malformed = |reason: String| Err(ManifestError::Malformed { reason });
    if files.len() > MAX_FILES {
        return malformed(format!("it lists more than {MAX_FILES} files"));
    }
    let mut listed = BTreeMap::new();
    for (path, digest) in files {
        if !is_plain_path(path) {
            // Cut, for it may be of any length.
            let mut shown: String = path.chars().take(MAX_PATH_BYTES).collect();
            if shown.len() < path.len() {
                shown.push('\u{2026}');
            }
            return malformed(format!(
                "it lists '{shown}', which is no path inside the skill"
            ));
        }
        let Some(digest) = digest.as_str().filter(|digest| is_digest(digest)) else {
            return malformed(format!("it has no SHA-256 for '{path}'"));
        };
        listed.insert(path.clone(), digest.to_owned());
    }
    Ok(listed)
}

/// Whether `path` is a relative path that stays inside the directory it is
/// joined to, on every platform: `/`-separated components, none of them
/// empty, `.` or `..`, with no backslash, colon or control character, and no
/// longer than a name may be.
fn is_plain_path(path: &str) -> bool {
    path != MANIFEST
        && path.len() <= MAX_PATH_BYTES
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|component| !matches!(component, "" | "." | ".."))
}

/// Whether `version` is one [`is_newer`] can order: every component starts
/// with a number that fits.
fn is_version(version: &str) -> bool {
    version.len() <= MAX_VERSION_BYTES
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        && version.split('.').all(|component| {
            let digits = component
                .find(|c: char| !c.is_ascii_digit())
                .map_or(component, |end| &component[..end]);
            digits.parse::<u64>().is_ok()
        })
}

/// Whether `digest` is a SHA-256 as [`sha256_hex`] spells it.
fn is_digest(digest: &str) -> bool {
    digest.len() == SHA256_HEX_LEN
        && digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::super::content::SkillFile;
    use super::*;
    use pretty_assertions::assert_eq;

    fn malformed(text: &str) -> String {
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestError::Malformed { reason }) => reason,
            other => panic!("expected a malformed manifest, got {other:?}"),
        }
    }

    fn with_path(path: &str) -> String {
        serde_json::json!({
            "schema": 1,
            "atl_version": "2026.1.0",
            "files": { path: "0".repeat(64) },
        })
        .to_string()
    }

    #[test]
    fn a_digest_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn versions_are_ordered_as_numbers() {
        for (installed, current, newer) in [
            ("2026.10.0", "2026.9.0", true),
            ("2026.9.0", "2026.10.0", false),
            ("2026.10.0", "2026.10.0", false),
            ("2026.10", "2026.10.0", false),
            ("2026.10.0.1", "2026.10.0", true),
            ("2026.10.0-rc1", "2026.10.0", false),
            ("2027.1.0", "2026.52.9", true),
        ] {
            assert_eq!(is_newer(installed, current), newer, "{installed} {current}");
        }
    }

    #[test]
    fn what_is_written_reads_back() {
        let manifest = Manifest::of(&Edition::embedded());
        assert_eq!(Manifest::parse(&manifest.to_bytes()), Ok(manifest));
    }

    #[test]
    fn the_file_starts_with_schema_and_version_and_ends_with_a_line_break() {
        let text = String::from_utf8(Manifest::of(&Edition::embedded()).to_bytes()).expect("UTF-8");
        let head = format!(
            "{{\n  \"schema\": 1,\n  \"atl_version\": \"{}\",\n  \"files\": {{\n    \"SKILL.md\": \"",
            env!("CARGO_PKG_VERSION")
        );
        assert!(text.starts_with(&head), "{text}");
        assert!(text.ends_with("\"\n  }\n}\n"), "{text}");
    }

    #[test]
    fn a_newer_schema_is_its_own_error_whatever_else_is_there() {
        assert_eq!(
            Manifest::parse(br#"{"schema": 2, "files": 7}"#),
            Err(ManifestError::NewerSchema { schema: 2 })
        );
    }

    #[test]
    fn what_is_no_manifest_is_malformed() {
        assert!(malformed("not json").starts_with("it is not JSON: "));
        assert_eq!(malformed("[]"), "it has no schema number");
        assert_eq!(malformed(r#"{"schema": 0}"#), "it has no schema number");
        assert_eq!(malformed(r#"{"schema": "1"}"#), "it has no schema number");
        assert_eq!(malformed(r#"{"schema": 1}"#), "it names no atl version");
        // The key of the other CLI is not this one's.
        assert_eq!(
            malformed(r#"{"schema": 1, "dq_version": "2026.1.0", "files": {}}"#),
            "it names no atl version"
        );
        for version in ["", "new", "2026.x.1", "99999999999999999999.1.0"] {
            assert_eq!(
                malformed(&format!(r#"{{"schema": 1, "atl_version": "{version}"}}"#)),
                "it names no atl version",
                "{version:?}"
            );
        }
        assert_eq!(
            malformed(r#"{"schema": 1, "atl_version": "2026.1.0"}"#),
            "it lists no files"
        );
        for digest in [
            "\"abc\"",
            "1",
            "\"G000000000000000000000000000000000000000000000000000000000000000\"",
        ] {
            assert_eq!(
                malformed(&format!(
                    r#"{{"schema": 1, "atl_version": "2026.1.0", "files": {{"a": {digest}}}}}"#
                )),
                "it has no SHA-256 for 'a'",
                "{digest}"
            );
        }
    }

    /// The two maps share one limit, and the largest manifest there is within
    /// it, every character of every path escaped, is read again.
    #[test]
    fn a_manifest_lists_so_many_files_of_so_long_a_path() {
        let of = |files: usize, unfinished: usize| {
            let some = |from: usize, count: usize| -> BTreeMap<String, String> {
                (from..from + count)
                    .map(|n| (format!("{n:\">255}"), "0".repeat(64)))
                    .collect()
            };
            Manifest {
                atl_version: "1".repeat(MAX_VERSION_BYTES),
                files: some(0, files),
                unfinished: some(files, unfinished),
            }
        };
        assert_eq!(of(1, 0).files.keys().next().expect("a path").len(), 255);
        for (files, unfinished) in [(256, 0), (0, 256), (128, 128), (255, 1)] {
            let mut most = of(files, unfinished);
            assert!(most.is_within_limits());
            let bytes = most.to_bytes();
            assert!(bytes.len() as u64 <= MAX_BYTES, "{}", bytes.len());
            // A version of 64 digits is none that can be ordered.
            most.atl_version = format!("1{}", ".1".repeat(31));
            let bytes = most.to_bytes();
            assert_eq!(Manifest::parse(&bytes), Ok(most));
        }
        for (files, unfinished) in [(257, 0), (0, 257), (256, 1), (1, 256), (200, 200)] {
            let mut more = of(files, unfinished);
            more.atl_version = "1".to_owned();
            assert!(!more.is_within_limits());
            assert_eq!(
                malformed(&String::from_utf8(more.to_bytes()).expect("UTF-8")),
                "it lists more than 256 files"
            );
        }
        assert_eq!(
            malformed(&with_path(&"a".repeat(256))),
            format!(
                "it lists '{}\u{2026}', which is no path inside the skill",
                "a".repeat(255)
            )
        );
        let version = format!("1{}", ".1".repeat(32));
        assert_eq!(
            malformed(&format!(
                r#"{{"schema": 1, "atl_version": "{version}", "files": {{}}}}"#
            )),
            "it names no atl version"
        );
    }

    /// What may be there while `unfinished` lists something, and after.
    #[test]
    fn an_unfinished_run_has_two_texts_of_a_file() {
        const NEW: Edition = Edition {
            version: "2026.2.0",
            files: &[
                SkillFile {
                    path: "both",
                    text: "new",
                },
                SkillFile {
                    path: "new",
                    text: "new",
                },
            ],
        };
        let digest = |text: &str| sha256_hex(text.as_bytes());
        let old = ["both", "old"].map(|path| (path.to_owned(), digest("old")));
        let manifest = Manifest::unfinished("2026.1.0", old.into(), &NEW);
        assert_eq!(Manifest::parse(&manifest.to_bytes()), Ok(manifest.clone()));
        let of = |path: &str| {
            manifest
                .digests(path)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(of("both"), [digest("old"), digest("new")]);
        assert_eq!(of("old"), [digest("old")]);
        assert_eq!(of("new"), [digest("new")]);
        assert_eq!(of("none"), Vec::<String>::new());
        assert_eq!(
            ["both", "old", "new"].map(|path| manifest.may_lack(path)),
            [false, true, true]
        );
        assert_eq!(manifest.paths().collect::<Vec<_>>(), ["both", "new", "old"]);
        let finished = Manifest::of(&NEW);
        assert!(!finished.may_lack("new"));
        assert_eq!(
            malformed(r#"{"schema": 1, "atl_version": "1", "files": {}, "unfinished": []}"#),
            "it lists no unfinished files"
        );
    }

    #[test]
    fn a_path_that_leaves_the_directory_is_refused() {
        for path in [
            "",
            "/etc/passwd",
            "..",
            "../x",
            "a/../../x",
            "a//b",
            "a/",
            "./a",
            ".",
            "a\\..\\b",
            "C:/x",
            "a\0b",
            "a\nb",
            MANIFEST,
        ] {
            assert!(
                malformed(&with_path(path)).ends_with("which is no path inside the skill"),
                "{path:?}"
            );
        }
        for path in ["SKILL.md", "references/a b.md", "..a", "a/...", "a/.hidden"] {
            assert!(
                Manifest::parse(with_path(path).as_bytes()).is_ok(),
                "{path:?}"
            );
        }
    }
}
