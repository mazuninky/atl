//! `atl self` command handlers — check for and install atl updates from
//! GitHub Releases.
//!
//! Uses the `self_update` crate with a rustls-only HTTP stack and a blocking
//! reqwest client. Both handlers are synchronous and must not be wrapped in a
//! tokio runtime — `self_update` manages its own HTTP client.
//!
//! ## Integrity verification
//!
//! [`run_update`] does **not** trust the binary served from the GitHub
//! release URL on its own. The release pipeline publishes a `.sha256`
//! sidecar alongside every archive (see `.github/workflows/release.yml`); we
//! download the sidecar first, parse it, then stream-hash the archive while
//! writing it to a temp file. If the digests disagree we bail **before**
//! touching the running binary. Without this guard, anyone with momentary
//! write access to the GitHub release assets could ship a backdoored `atl`
//! to every user who runs `atl self update`.

use std::io::Write;

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tracing::{debug, info};

use crate::cli::args::SelfUpdateArgs;
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

pub(super) const REPO_OWNER: &str = "mazuninky";
pub(super) const REPO_NAME: &str = "atl";

/// Returns the platform-specific binary name (`atl` / `atl.exe`).
fn bin_name() -> &'static str {
    if cfg!(windows) { "atl.exe" } else { "atl" }
}

/// Builds the canonical GitHub release page URL for a given version.
fn release_url(version: &str) -> String {
    format!("https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/tag/v{version}")
}

/// Returns the archive extension for the current target.
///
/// Linux / macOS targets are packaged as `.tar.gz`; Windows is packaged as
/// `.zip`. This must stay in sync with the matrix in
/// `.github/workflows/release.yml`.
fn archive_ext_for_target(target: &str) -> &'static str {
    if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    }
}

/// Builds the GitHub asset filename for a given version + target.
///
/// Mirrors the naming used by the release workflow (`atl-{version}-{target}.{ext}`).
fn asset_name(version: &str, target: &str) -> String {
    format!(
        "atl-{version}-{target}.{ext}",
        ext = archive_ext_for_target(target)
    )
}

/// Builds the direct download URL for a release asset.
fn asset_url(version: &str, target: &str) -> String {
    let name = asset_name(version, target);
    format!("https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/download/v{version}/{name}")
}

/// Verifies that the current target triple has prebuilt release assets.
///
/// Returns the detected target on success; bails with an actionable message
/// pointing at `cargo install` otherwise.
fn ensure_supported_target() -> anyhow::Result<&'static str> {
    let t = self_update::get_target();
    match t {
        "aarch64-apple-darwin" | "x86_64-unknown-linux-gnu" | "x86_64-pc-windows-msvc" => Ok(t),
        other => anyhow::bail!(
            "no prebuilt atl binary for target '{other}'. \
             Build from source: `cargo install --git https://github.com/{REPO_OWNER}/{REPO_NAME}`"
        ),
    }
}

/// Refuses to overwrite binaries managed by a package manager, then probes
/// writability of the parent directory so we fail fast with a clear error
/// instead of letting a later `rename` call stumble.
fn preflight_exe_location() -> anyhow::Result<()> {
    let exe_std = std::env::current_exe()?.canonicalize()?;
    let exe: Utf8PathBuf = Utf8PathBuf::from_path_buf(exe_std)
        .map_err(|p| anyhow::anyhow!("binary path is not valid UTF-8: {}", p.display()))?;
    preflight_for_path(&exe)?;

    // Writability probe — a side-effect that can't be covered by unit tests.
    let parent = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("binary has no parent directory: {exe}"))?;
    // Unique per-process probe name so concurrent `atl self update` runs can't
    // collide, and `create_new` so we never truncate a pre-existing file.
    let probe = parent.join(format!(".atl-update-probe-{}", std::process::id()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| {
            anyhow::anyhow!(
                "cannot write to {parent}: {e}. Try running with elevated permissions or reinstalling atl in a writable location."
            )
        })?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Pure path classifier used by [`preflight_exe_location`] so unit tests can
/// feed in synthetic paths without touching the filesystem.
fn preflight_for_path(exe: &Utf8Path) -> anyhow::Result<()> {
    let exe_str = exe.as_str();

    // Homebrew (Intel `/usr/local/Cellar` + Apple Silicon `/opt/homebrew/Cellar`).
    // Match only the real Cellar prefixes so paths like `/srv/Cellar/atl` or
    // `/Users/me/homebrew-tools/atl` aren't misclassified.
    if exe_str.starts_with("/usr/local/Cellar/") || exe_str.starts_with("/opt/homebrew/Cellar/") {
        anyhow::bail!("atl is managed by Homebrew at {exe_str}. Run `brew upgrade atl` instead.");
    }

    // System / distro package managers and immutable stores.
    for prefix in ["/usr/bin/", "/usr/sbin/", "/opt/", "/nix/store/"] {
        if exe_str.starts_with(prefix) {
            anyhow::bail!(
                "atl is installed at {exe_str} (managed by a package manager). \
                 Use your package manager to update."
            );
        }
    }

    Ok(())
}

/// Parses a `YYYY.WW.BUILD` version string into a tuple for ordering.
pub(super) fn parse_version(v: &str) -> anyhow::Result<(u64, u64, u64)> {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        anyhow::bail!("version '{v}' does not match YYYY.WW.BUILD format");
    }
    let year: u64 = parts[0]
        .parse()
        .map_err(|_| anyhow::anyhow!("version '{v}': year component is not a number"))?;
    let week: u64 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("version '{v}': week component is not a number"))?;
    let build: u64 = parts[2]
        .parse()
        .map_err(|_| anyhow::anyhow!("version '{v}': build component is not a number"))?;
    Ok((year, week, build))
}

/// Dispatches a single `serde_json::Value` through the existing reporter
/// stack, honouring any active `--jq` / `--template` transforms.
fn emit(
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
    value: Value,
) -> anyhow::Result<()> {
    write_output(value, format, io, transforms)
}

/// Handles `atl self check` — queries GitHub for the latest release and
/// reports whether an update is available. Always exits 0.
pub fn run_check(
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    ensure_supported_target()?;

    debug!("fetching release list from github.com/{REPO_OWNER}/{REPO_NAME}");
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .map_err(|e| anyhow::anyhow!("self-update config failed: {e}"))?
        .fetch()
        .map_err(|e| anyhow::anyhow!("failed to fetch releases: {e}"))?;

    let current = env!("CARGO_PKG_VERSION");
    let latest = releases
        .first()
        .ok_or_else(|| anyhow::anyhow!("no releases found for {REPO_OWNER}/{REPO_NAME}"))?;
    let latest_version = latest.version.trim_start_matches('v').to_string();

    let value = build_check_value(current, &latest_version);

    emit(format, io, transforms, value)
}

/// Pure builder for the `atl self check` payload — used by [`run_check`] and
/// directly from unit tests without going through the GitHub API.
fn build_check_value(current: &str, latest: &str) -> Value {
    let update_available = match (parse_version(current), parse_version(latest)) {
        (Ok(c), Ok(l)) => l > c,
        // If either version fails to parse, fall back to string inequality so
        // we still produce a useful signal rather than bailing on the user.
        _ => latest != current,
    };
    json!({
        "current": current,
        "latest": latest,
        "update_available": update_available,
        "release_url": release_url(latest),
    })
}

/// Pure downgrade guard: refuses when `pinned` is strictly older than
/// `current` and `allow_downgrade` is `false`. Bails when either version
/// cannot be parsed so the user gets a clear error instead of an unguarded
/// install.
fn check_downgrade_guard(current: &str, pinned: &str, allow_downgrade: bool) -> anyhow::Result<()> {
    if allow_downgrade {
        return Ok(());
    }
    let (Ok(current_v), Ok(pinned_v)) = (parse_version(current), parse_version(pinned)) else {
        anyhow::bail!(
            "cannot validate downgrade: failed to parse version (current='{current}', pinned='{pinned}')"
        );
    };
    if pinned_v < current_v {
        anyhow::bail!(
            "refusing to downgrade from {current} to {pinned}; pass --allow-downgrade to override"
        );
    }
    Ok(())
}

/// Parses the GitHub `.sha256` sidecar text and returns the expected hex
/// digest.
///
/// Sidecar format (as produced by `shasum -a 256` / PowerShell):
/// `<64-hex>  <filename>` (or just `<64-hex>` with no filename, on a single
/// line). We accept any whitespace separator, ignore the filename column,
/// tolerate trailing newlines / `\r\n`, and validate that the digest is
/// exactly 64 lowercase-hex characters before returning it.
fn parse_sha256_sidecar(text: &str) -> anyhow::Result<String> {
    // `lines()` would happily split on any internal `\n` and silently drop
    // payloads on later lines; we want to be defensive against multi-line
    // sidecars (which would be invalid). Take the first non-empty line.
    let first_line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("sha256 sidecar is empty"))?;

    let digest = first_line
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("sha256 sidecar has no digest token"))?;

    if digest.len() != 64 {
        anyhow::bail!(
            "sha256 sidecar digest has wrong length: expected 64 hex chars, got {} ('{digest}')",
            digest.len()
        );
    }
    if !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("sha256 sidecar digest is not valid hex: '{digest}'");
    }
    if digest.chars().any(|c| c.is_ascii_uppercase()) {
        // Normalize to lowercase so the comparison in `verify_archive_bytes`
        // is a plain `==`. Uppercase-hex sidecars happen on Windows
        // (PowerShell `Get-FileHash` defaults), and our release pipeline
        // already lowercases them — so this is mostly defensive.
        return Ok(digest.to_ascii_lowercase());
    }
    Ok(digest.to_string())
}

/// Verifies that `bytes` hash to `expected_hex` under SHA-256.
///
/// `asset_name` is only used to make the error message human-readable.
/// Returns `Ok(())` on match, otherwise an error containing the literal
/// substring `"checksum mismatch"` plus both digests.
///
/// Production code uses [`HashingWriter`] + [`compare_digests`] to avoid
/// double-buffering the archive; this in-memory variant exists so the
/// integrity gate has direct unit-test coverage with no streaming setup.
#[cfg(test)]
fn verify_archive_bytes(bytes: &[u8], expected_hex: &str, asset_name: &str) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex_lower(&hasher.finalize());
    compare_digests(&actual, expected_hex, asset_name)
}

/// Compares two SHA-256 hex digests case-insensitively. Used by both the
/// in-memory verifier (`verify_archive_bytes`) and the streaming production
/// path (`run_update`) so they fail with identical error messages.
fn compare_digests(actual: &str, expected: &str, asset_name: &str) -> anyhow::Result<()> {
    let actual_lc = actual.to_ascii_lowercase();
    let expected_lc = expected.to_ascii_lowercase();
    if actual_lc != expected_lc {
        anyhow::bail!(
            "checksum mismatch for {asset_name}: expected {expected_lc}, got {actual_lc}"
        );
    }
    Ok(())
}

/// Lowercase-hex encoder for a SHA-256 digest. We avoid pulling in a
/// dependency for a 32-byte conversion.
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// `Write` adapter that streams every byte through both an inner writer
/// (typically a `NamedTempFile`) and a `Sha256` hasher. Avoids
/// double-buffering the archive in memory while we download.
struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha256,
}

impl<W: Write> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    fn into_digest_hex(self) -> String {
        hex_lower(&self.hasher.finalize())
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Resolves the version we should install: either the explicit `--to` value
/// or the latest GitHub release.
fn resolve_target_version(args: &SelfUpdateArgs) -> anyhow::Result<String> {
    if let Some(v) = &args.to {
        return Ok(v.clone());
    }

    debug!("fetching latest release tag from github.com/{REPO_OWNER}/{REPO_NAME}");
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .map_err(|e| anyhow::anyhow!("self-update config failed: {e}"))?
        .fetch()
        .map_err(|e| anyhow::anyhow!("failed to fetch releases: {e}"))?;
    let latest = releases
        .first()
        .ok_or_else(|| anyhow::anyhow!("no releases found for {REPO_OWNER}/{REPO_NAME}"))?;
    Ok(latest.version.trim_start_matches('v').to_string())
}

/// Downloads the release archive into `dest` while computing its SHA-256.
/// Returns the lowercase-hex digest on success.
fn download_archive_to<W: Write>(url: &str, dest: W) -> anyhow::Result<String> {
    let mut writer = HashingWriter::new(dest);
    self_update::Download::from_url(url)
        .show_progress(false)
        .download_to(&mut writer)
        .map_err(|e| anyhow::anyhow!("failed to download {url}: {e}"))?;
    writer.flush().ok();
    Ok(writer.into_digest_hex())
}

/// Downloads the `.sha256` sidecar into memory and returns the parsed digest.
fn fetch_expected_digest(url: &str) -> anyhow::Result<String> {
    let mut buf: Vec<u8> = Vec::with_capacity(128);
    self_update::Download::from_url(url)
        .show_progress(false)
        .download_to(&mut buf)
        .map_err(|e| anyhow::anyhow!("failed to download {url}: {e}"))?;
    let text = std::str::from_utf8(&buf)
        .map_err(|e| anyhow::anyhow!("sha256 sidecar at {url} is not valid UTF-8: {e}"))?;
    parse_sha256_sidecar(text)
}

/// Locates the unpacked `atl` (or `atl.exe`) binary in `dir`. The release
/// archives place the binary at `atl-{version}-{target}/{bin}` so we recurse
/// one level. Returns an error if the binary can't be found.
fn locate_extracted_binary(dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let bin = bin_name();

    // Direct hit (defensive — unlikely with the current release layout).
    let direct = dir.join(bin);
    if direct.is_file() {
        return Ok(direct);
    }

    // The release archive contains a single top-level `atl-{version}-{target}/`
    // directory holding the binary.
    let entries = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("cannot read extraction dir {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let candidate = path.join(bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    anyhow::bail!(
        "could not locate '{bin}' in extracted archive at {}",
        dir.display()
    )
}

/// Handles `atl self update` — downloads the target release archive,
/// verifies its SHA-256 against the published sidecar, and atomically
/// replaces the current binary. Refuses to run from package-manager paths
/// and refuses to downgrade unless `--allow-downgrade` is set.
pub fn run_update(
    args: &SelfUpdateArgs,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let target = ensure_supported_target()?;
    preflight_exe_location()?;

    let current = env!("CARGO_PKG_VERSION");

    if let Some(pinned) = &args.to {
        check_downgrade_guard(current, pinned, args.allow_downgrade)?;
    }

    info!("preparing self-update (current version {current})");

    let target_version = resolve_target_version(args)?;
    let target_version = target_version.trim_start_matches('v').to_string();

    if target_version == current {
        let value = json!({
            "previous": current,
            "current": current,
            "release_url": release_url(current),
            "already_up_to_date": true,
        });
        return emit(format, io, transforms, value);
    }

    let asset = asset_name(&target_version, target);
    let archive_url = asset_url(&target_version, target);
    let sidecar_url = format!("{archive_url}.sha256");

    info!("downloading sha256 sidecar from {sidecar_url}");
    let expected_digest = fetch_expected_digest(&sidecar_url)?;
    debug!("expected sha256 for {asset}: {expected_digest}");

    info!("downloading {asset} from {archive_url}");
    let archive_tmp = tempfile::NamedTempFile::new()
        .map_err(|e| anyhow::anyhow!("cannot create temp file for archive: {e}"))?;

    // Stream-hash while downloading; persist after a successful checksum match.
    let archive_path = archive_tmp.path().to_path_buf();
    let actual_digest = {
        let file = archive_tmp.reopen().map_err(|e| {
            anyhow::anyhow!("cannot reopen temp file {}: {e}", archive_path.display())
        })?;
        download_archive_to(&archive_url, file)?
    };

    compare_digests(&actual_digest, &expected_digest, &asset)?;
    info!("sha256 verified for {asset}");

    // Extract into a temp dir so a partially-extracted archive never leaks
    // into the final install location.
    let extract_dir = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("cannot create temp dir for extraction: {e}"))?;
    let archive_kind = if asset.ends_with(".zip") {
        self_update::ArchiveKind::Zip
    } else {
        self_update::ArchiveKind::Tar(Some(self_update::Compression::Gz))
    };
    self_update::Extract::from_source(&archive_path)
        .archive(archive_kind)
        .extract_into(extract_dir.path())
        .map_err(|e| anyhow::anyhow!("failed to extract {asset}: {e}"))?;
    drop(archive_tmp);

    let new_bin = locate_extracted_binary(extract_dir.path())?;
    debug!("located new binary at {}", new_bin.display());

    // Atomic in-place swap.
    self_replace::self_replace(&new_bin)
        .map_err(|e| anyhow::anyhow!("failed to replace running binary: {e}"))?;

    // Ad-hoc codesign on macOS so the updated binary retains keychain
    // access without prompting for the login keychain password.
    #[cfg(target_os = "macos")]
    if let Ok(exe) = std::env::current_exe() {
        let result = std::process::Command::new("codesign")
            .args(["-s", "-", "-f"])
            .arg(&exe)
            .output();
        match result {
            Ok(output) if output.status.success() => {
                debug!("ad-hoc codesign applied to {}", exe.display());
            }
            Ok(output) => {
                debug!(
                    "codesign failed (exit {}): {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                debug!("codesign command not found or failed to run: {e}");
            }
        }
    }

    let value = json!({
        "previous": current,
        "current": target_version,
        "release_url": release_url(&target_version),
    });

    emit(format, io, transforms, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    #[test]
    fn preflight_refuses_homebrew_apple_silicon() {
        let path = Utf8PathBuf::from("/opt/homebrew/Cellar/atl/2026.15.1/bin/atl");
        let err = preflight_for_path(&path).unwrap_err();
        assert!(
            err.to_string().contains("Homebrew"),
            "expected Homebrew refusal, got: {err}"
        );
    }

    #[test]
    fn preflight_refuses_homebrew_intel() {
        let path = Utf8PathBuf::from("/usr/local/Cellar/atl/2026.15.1/bin/atl");
        let err = preflight_for_path(&path).unwrap_err();
        assert!(
            err.to_string().contains("Homebrew"),
            "expected Homebrew refusal, got: {err}"
        );
    }

    #[test]
    fn preflight_refuses_usr_bin() {
        let path = Utf8PathBuf::from("/usr/bin/atl");
        let err = preflight_for_path(&path).unwrap_err();
        assert!(
            err.to_string().contains("package manager"),
            "expected package-manager refusal, got: {err}"
        );
    }

    #[test]
    fn preflight_refuses_nix_store() {
        let path = Utf8PathBuf::from("/nix/store/abc-atl/bin/atl");
        let err = preflight_for_path(&path).unwrap_err();
        assert!(
            err.to_string().contains("package manager"),
            "expected package-manager refusal, got: {err}"
        );
    }

    #[test]
    fn preflight_accepts_cargo_bin() {
        let path = Utf8PathBuf::from("/home/user/.cargo/bin/atl");
        preflight_for_path(&path).expect("cargo-bin install should be writable");
    }

    #[test]
    fn preflight_accepts_local_bin() {
        let path = Utf8PathBuf::from("/Users/me/.local/bin/atl");
        preflight_for_path(&path).expect("user-local install should be writable");
    }

    #[test]
    fn preflight_accepts_homebrew_lookalike() {
        // A user directory that merely contains "homebrew" in its name must
        // not be misclassified as a Homebrew-managed install.
        let path = Utf8PathBuf::from("/Users/me/homebrew-tools/atl");
        preflight_for_path(&path).expect("homebrew-lookalike path should be accepted");
    }

    #[test]
    fn preflight_accepts_non_cellar_path_with_cellar_segment() {
        // `/Cellar/` as a middle segment is not a real Homebrew Cellar prefix.
        let path = Utf8PathBuf::from("/srv/Cellar/atl");
        preflight_for_path(&path).expect("non-Homebrew /Cellar/ path should be accepted");
    }

    #[test]
    fn parse_version_basic() {
        assert_eq!(parse_version("2026.15.1").unwrap(), (2026, 15, 1));
        assert_eq!(parse_version("2026.16.2").unwrap(), (2026, 16, 2));
    }

    #[test]
    fn parse_version_ordering() {
        assert!(parse_version("2026.16.2").unwrap() > parse_version("2026.15.1").unwrap());
        assert!(parse_version("2026.15.2").unwrap() > parse_version("2026.15.1").unwrap());
        assert!(parse_version("2027.1.0").unwrap() > parse_version("2026.52.99").unwrap());
    }

    #[test]
    fn parse_version_rejects_bad_input() {
        assert!(parse_version("not-a-version").is_err());
        assert!(parse_version("1.2").is_err());
        assert!(parse_version("1.2.3.4").is_err());
        assert!(parse_version("a.b.c").is_err());
    }

    #[test]
    fn release_url_format() {
        assert_eq!(
            release_url("2026.16.2"),
            "https://github.com/mazuninky/atl/releases/tag/v2026.16.2"
        );
    }

    #[test]
    fn bin_name_platform() {
        let name = bin_name();
        if cfg!(windows) {
            assert_eq!(name, "atl.exe");
        } else {
            assert_eq!(name, "atl");
        }
    }

    // -------------------------------------------------------------------
    // preflight_for_path: additional negative paths so a regression in the
    // prefix list (e.g. accidentally removing /usr/sbin/ or /opt/) trips a
    // test rather than reaching production.
    // -------------------------------------------------------------------

    #[test]
    fn preflight_refuses_usr_sbin() {
        let err =
            preflight_for_path(&Utf8PathBuf::from("/usr/sbin/atl")).expect_err("should refuse");
        assert!(
            err.to_string().contains("package manager"),
            "expected package-manager refusal, got: {err}"
        );
    }

    #[test]
    fn preflight_refuses_opt_install() {
        let err = preflight_for_path(&Utf8PathBuf::from("/opt/atl/bin/atl"))
            .expect_err("should refuse /opt/");
        assert!(
            err.to_string().contains("package manager"),
            "expected package-manager refusal, got: {err}"
        );
    }

    #[test]
    fn preflight_homebrew_error_mentions_brew_upgrade() {
        let err = preflight_for_path(&Utf8PathBuf::from(
            "/opt/homebrew/Cellar/atl/2026.15.1/bin/atl",
        ))
        .expect_err("should refuse Homebrew");
        let msg = err.to_string();
        assert!(
            msg.contains("brew upgrade atl"),
            "Homebrew error must point at brew upgrade, got: {msg}"
        );
    }

    // -------------------------------------------------------------------
    // parse_version: more edge cases.
    // -------------------------------------------------------------------

    #[test]
    fn parse_version_rejects_negative_components() {
        // u64 parse rejects leading minus.
        assert!(parse_version("-1.2.3").is_err());
        assert!(parse_version("1.-2.3").is_err());
        assert!(parse_version("1.2.-3").is_err());
    }

    #[test]
    fn parse_version_rejects_empty_components() {
        assert!(parse_version("..").is_err());
        assert!(parse_version("1..3").is_err());
        assert!(parse_version("1.2.").is_err());
        assert!(parse_version(".2.3").is_err());
    }

    #[test]
    fn parse_version_accepts_zero_components() {
        // Calendar-version uses non-zero week and build in practice but the
        // parser itself must not reject zeros — only non-numeric input.
        assert_eq!(parse_version("0.0.0").unwrap(), (0, 0, 0));
        assert_eq!(parse_version("2026.0.0").unwrap(), (2026, 0, 0));
    }

    #[test]
    fn parse_version_error_mentions_input() {
        // The error message names the bad input so users can debug.
        let err = parse_version("nope.x.y").unwrap_err().to_string();
        assert!(
            err.contains("nope.x.y"),
            "error must echo input, got: {err}"
        );
    }

    #[test]
    fn parse_version_rejects_empty_string() {
        assert!(parse_version("").is_err());
    }

    // -------------------------------------------------------------------
    // release_url: canonical format, including special-character versions.
    // -------------------------------------------------------------------

    #[test]
    fn release_url_passes_version_through_verbatim() {
        // release_url does no validation — it just builds the URL. Verify the
        // template doesn't accidentally double-prefix the `v`.
        assert_eq!(
            release_url("0.1.0"),
            "https://github.com/mazuninky/atl/releases/tag/v0.1.0"
        );
    }

    // -------------------------------------------------------------------
    // build_check_value: pure builder, exercises the update_available
    // decision tree without touching the network.
    // -------------------------------------------------------------------

    #[test]
    fn build_check_value_no_update_available() {
        let v = build_check_value("2026.18.1", "2026.18.1");
        assert_eq!(v["current"], "2026.18.1");
        assert_eq!(v["latest"], "2026.18.1");
        assert_eq!(v["update_available"], false);
        assert_eq!(
            v["release_url"],
            "https://github.com/mazuninky/atl/releases/tag/v2026.18.1"
        );
    }

    #[test]
    fn build_check_value_update_available_when_latest_newer() {
        let v = build_check_value("2026.15.1", "2026.18.2");
        assert_eq!(v["current"], "2026.15.1");
        assert_eq!(v["latest"], "2026.18.2");
        assert_eq!(v["update_available"], true);
    }

    #[test]
    fn build_check_value_no_update_when_local_is_ahead() {
        // A locally-built binary may be newer than what GitHub publishes —
        // the notifier must NOT claim an update is available.
        let v = build_check_value("2099.1.99", "2026.18.1");
        assert_eq!(v["update_available"], false);
    }

    #[test]
    fn build_check_value_string_inequality_fallback_when_unparseable() {
        // If the upstream tag isn't `YYYY.WW.BUILD` we still produce a
        // signal: "different from current" → update_available = true.
        let v = build_check_value("2026.18.1", "main-snapshot-abc123");
        assert_eq!(v["update_available"], true);
    }

    #[test]
    fn build_check_value_string_equality_fallback_when_unparseable() {
        // Same unparseable tag on both sides → not different → no update.
        let v = build_check_value("dirty-build", "dirty-build");
        assert_eq!(v["update_available"], false);
    }

    // -------------------------------------------------------------------
    // check_downgrade_guard: every branch.
    // -------------------------------------------------------------------

    #[test]
    fn downgrade_guard_allows_when_pinned_is_newer() {
        check_downgrade_guard("2026.15.1", "2026.18.2", false).expect("upgrade is allowed");
    }

    #[test]
    fn downgrade_guard_allows_when_pinned_is_same() {
        // Same version isn't a downgrade — `self_update` will report
        // "already up to date" downstream.
        check_downgrade_guard("2026.18.1", "2026.18.1", false).expect("same version is allowed");
    }

    #[test]
    fn downgrade_guard_refuses_strict_downgrade() {
        let err = check_downgrade_guard("2026.18.2", "2026.15.1", false)
            .expect_err("downgrade must be refused");
        let msg = err.to_string();
        assert!(
            msg.contains("refusing to downgrade"),
            "error must mention refusal, got: {msg}"
        );
        assert!(
            msg.contains("--allow-downgrade"),
            "error must point at the override flag, got: {msg}"
        );
    }

    #[test]
    fn downgrade_guard_allows_when_explicitly_overridden() {
        check_downgrade_guard("2026.18.2", "2026.15.1", true).expect("override should succeed");
    }

    #[test]
    fn downgrade_guard_bails_on_unparseable_current() {
        let err = check_downgrade_guard("dirty", "2026.18.1", false)
            .expect_err("must bail on unparseable current");
        assert!(
            err.to_string().contains("cannot validate downgrade"),
            "got: {err}"
        );
    }

    #[test]
    fn downgrade_guard_bails_on_unparseable_pinned() {
        let err = check_downgrade_guard("2026.18.1", "main-snapshot", false)
            .expect_err("must bail on unparseable pinned");
        assert!(
            err.to_string().contains("cannot validate downgrade"),
            "got: {err}"
        );
    }

    #[test]
    fn downgrade_guard_with_allow_downgrade_skips_parse_check() {
        // Invariant: the override is the user's escape hatch — it must work
        // even when versions can't be compared.
        check_downgrade_guard("dirty-current", "anything-pinned", true)
            .expect("--allow-downgrade should bypass version parsing too");
    }

    // -------------------------------------------------------------------
    // asset URL / name layout — verify the strings we send to GitHub match
    // what the release workflow actually produces.
    // -------------------------------------------------------------------

    #[test]
    fn asset_name_linux() {
        assert_eq!(
            asset_name("2026.18.3", "x86_64-unknown-linux-gnu"),
            "atl-2026.18.3-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn asset_name_macos_apple_silicon() {
        assert_eq!(
            asset_name("2026.18.3", "aarch64-apple-darwin"),
            "atl-2026.18.3-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn asset_name_windows_uses_zip() {
        assert_eq!(
            asset_name("2026.18.3", "x86_64-pc-windows-msvc"),
            "atl-2026.18.3-x86_64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn asset_url_format() {
        assert_eq!(
            asset_url("2026.18.3", "x86_64-unknown-linux-gnu"),
            "https://github.com/mazuninky/atl/releases/download/v2026.18.3/atl-2026.18.3-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn asset_url_windows_uses_zip() {
        assert_eq!(
            asset_url("2026.18.3", "x86_64-pc-windows-msvc"),
            "https://github.com/mazuninky/atl/releases/download/v2026.18.3/atl-2026.18.3-x86_64-pc-windows-msvc.zip"
        );
    }

    // -------------------------------------------------------------------
    // SHA-256 sidecar parsing — defensive tolerance + strict validation.
    // -------------------------------------------------------------------

    /// SHA-256 of the empty string — handy fixed test fixture.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn parse_sha256_sidecar_canonical_two_space_form() {
        // Exact form `shasum -a 256` produces.
        let text = format!("{EMPTY_SHA256}  atl-2026.18.3-x86_64-unknown-linux-gnu.tar.gz\n");
        assert_eq!(parse_sha256_sidecar(&text).unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn parse_sha256_sidecar_handles_crlf() {
        let text = format!("{EMPTY_SHA256}  atl.tar.gz\r\n");
        assert_eq!(parse_sha256_sidecar(&text).unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn parse_sha256_sidecar_handles_trailing_newline_only() {
        let text = format!("{EMPTY_SHA256}\n");
        assert_eq!(parse_sha256_sidecar(&text).unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn parse_sha256_sidecar_handles_no_trailing_newline() {
        // PowerShell `-NoNewline` writes just the digest + filename.
        let text = format!("{EMPTY_SHA256}  atl.zip");
        assert_eq!(parse_sha256_sidecar(&text).unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn parse_sha256_sidecar_handles_extra_whitespace() {
        // Tabs and runs of spaces between digest and filename are valid.
        let text = format!("{EMPTY_SHA256}\t\tatl.tar.gz\n");
        assert_eq!(parse_sha256_sidecar(&text).unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn parse_sha256_sidecar_normalizes_uppercase_hex() {
        // Some Windows toolchains emit uppercase hex; we normalize so the
        // comparison in `verify_archive_bytes` stays a plain `==`.
        let upper = EMPTY_SHA256.to_ascii_uppercase();
        let text = format!("{upper}  atl.zip\n");
        assert_eq!(parse_sha256_sidecar(&text).unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn parse_sha256_sidecar_rejects_empty() {
        let err = parse_sha256_sidecar("").unwrap_err().to_string();
        assert!(err.contains("empty"), "got: {err}");
    }

    #[test]
    fn parse_sha256_sidecar_rejects_whitespace_only() {
        let err = parse_sha256_sidecar("   \n\t\n").unwrap_err().to_string();
        assert!(err.contains("empty"), "got: {err}");
    }

    #[test]
    fn parse_sha256_sidecar_rejects_short_digest() {
        let err = parse_sha256_sidecar("abc123  atl.tar.gz\n")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("wrong length"),
            "must mention length, got: {err}"
        );
    }

    #[test]
    fn parse_sha256_sidecar_rejects_long_digest() {
        // 65 hex chars.
        let bad = format!("{EMPTY_SHA256}f");
        let err = parse_sha256_sidecar(&format!("{bad}  atl.tar.gz\n"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("wrong length"),
            "must mention length, got: {err}"
        );
    }

    #[test]
    fn parse_sha256_sidecar_rejects_non_hex_digest() {
        // Right length, wrong alphabet.
        let bad = "z".repeat(64);
        let err = parse_sha256_sidecar(&format!("{bad}  atl.tar.gz\n"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not valid hex"), "got: {err}");
    }

    // -------------------------------------------------------------------
    // verify_archive_bytes — the actual integrity gate.
    // -------------------------------------------------------------------

    #[test]
    fn verify_archive_bytes_accepts_matching_digest() {
        // SHA-256 of "abc".
        let bytes = b"abc";
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        verify_archive_bytes(bytes, expected, "test.tar.gz").expect("matching digest should pass");
    }

    #[test]
    fn verify_archive_bytes_accepts_uppercase_expected() {
        let bytes = b"abc";
        let expected = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        verify_archive_bytes(bytes, expected, "test.tar.gz")
            .expect("uppercase expected digest should be normalized");
    }

    #[test]
    fn verify_archive_bytes_rejects_mismatch() {
        // Wrong digest for "abc".
        let err = verify_archive_bytes(b"abc", EMPTY_SHA256, "atl-2026.18.3-test.tar.gz")
            .expect_err("mismatch must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("checksum mismatch"),
            "error must contain 'checksum mismatch', got: {msg}"
        );
        assert!(
            msg.contains("atl-2026.18.3-test.tar.gz"),
            "error must name the asset, got: {msg}"
        );
        assert!(
            msg.contains(EMPTY_SHA256),
            "error must include the expected digest, got: {msg}"
        );
    }

    #[test]
    fn verify_archive_bytes_rejects_empty_with_nonempty_expected() {
        // Defensive: an attacker can't pass "" past us by claiming a digest.
        let err = verify_archive_bytes(
            b"",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "atl.tar.gz",
        )
        .expect_err("empty bytes vs nonempty expected must mismatch");
        assert!(err.to_string().contains("checksum mismatch"), "got: {err}");
    }

    // -------------------------------------------------------------------
    // HashingWriter — round-trip the digest through the streaming wrapper
    // we use during the actual download. Confirms that streaming and
    // non-streaming hashing agree.
    // -------------------------------------------------------------------

    #[test]
    fn hashing_writer_streaming_digest_matches_one_shot() {
        let payload = b"the quick brown fox jumps over the lazy dog";
        let one_shot = {
            let mut h = Sha256::new();
            h.update(payload);
            hex_lower(&h.finalize())
        };

        // Stream the payload in 7-byte chunks to exercise partial writes.
        let mut sink: Vec<u8> = Vec::new();
        let mut writer = HashingWriter::new(&mut sink);
        for chunk in payload.chunks(7) {
            writer.write_all(chunk).unwrap();
        }
        writer.flush().unwrap();
        let streamed = writer.into_digest_hex();
        assert_eq!(streamed, one_shot);

        // The inner writer received every byte verbatim.
        assert_eq!(sink.as_slice(), payload);
    }

    // -------------------------------------------------------------------
    // compare_digests — case-insensitive comparison shared by streaming
    // and in-memory paths.
    // -------------------------------------------------------------------

    #[test]
    fn compare_digests_lowercase_match() {
        compare_digests(EMPTY_SHA256, EMPTY_SHA256, "atl.tar.gz").expect("identical match");
    }

    #[test]
    fn compare_digests_mixed_case_match() {
        let upper = EMPTY_SHA256.to_ascii_uppercase();
        compare_digests(EMPTY_SHA256, &upper, "atl.tar.gz")
            .expect("case-insensitive comparison must accept mixed case");
    }

    #[test]
    fn compare_digests_mismatch_includes_both_digests() {
        let other = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let err = compare_digests(EMPTY_SHA256, other, "atl-2026.18.3-test.tar.gz")
            .expect_err("different digests must mismatch");
        let msg = err.to_string();
        assert!(msg.contains("checksum mismatch"), "got: {msg}");
        assert!(
            msg.contains(EMPTY_SHA256),
            "must mention actual, got: {msg}"
        );
        assert!(msg.contains(other), "must mention expected, got: {msg}");
        assert!(
            msg.contains("atl-2026.18.3-test.tar.gz"),
            "must name asset, got: {msg}"
        );
    }
}
