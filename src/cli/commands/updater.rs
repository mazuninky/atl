//! `atl self` command handlers — check for and install atl updates from
//! GitHub Releases.
//!
//! Uses the `self_update` crate with a rustls-only HTTP stack and a blocking
//! reqwest client. Both handlers are synchronous and must not be wrapped in a
//! tokio runtime — `self_update` manages its own HTTP client.

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Value, json};
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

    // Homebrew (Intel `/usr/local/Cellar`, Apple Silicon `/opt/homebrew/Cellar`,
    // Linuxbrew `/home/linuxbrew/.linuxbrew/Cellar`). Match only the real Cellar
    // prefixes so paths like `/srv/Cellar/atl` or `/Users/me/homebrew-tools/atl`
    // aren't misclassified.
    if exe_str.starts_with("/usr/local/Cellar/")
        || exe_str.starts_with("/opt/homebrew/Cellar/")
        || exe_str.starts_with("/home/linuxbrew/.linuxbrew/Cellar/")
    {
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

/// Handles `atl self update` — downloads the target release archive and
/// atomically replaces the current binary. Refuses to run from package-manager
/// paths and refuses to downgrade unless `--allow-downgrade` is set.
pub fn run_update(
    args: &SelfUpdateArgs,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    ensure_supported_target()?;
    preflight_exe_location()?;

    let current = env!("CARGO_PKG_VERSION");

    if let Some(pinned) = &args.to {
        check_downgrade_guard(current, pinned, args.allow_downgrade)?;
    }

    info!("preparing self-update (current version {current})");

    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(bin_name())
        .current_version(current)
        .bin_path_in_archive("atl-{{ version }}-{{ target }}/{{ bin }}")
        .show_download_progress(false)
        .show_output(false)
        .no_confirm(true);
    if let Some(v) = &args.to {
        builder.target_version_tag(&format!("v{v}"));
    }
    let updater = builder
        .build()
        .map_err(|e| anyhow::anyhow!("self-update config failed: {e}"))?;

    // TODO(self-update): verify sidecar .sha256 before replacement.
    let status = updater
        .update()
        .map_err(|e| anyhow::anyhow!("self-update failed: {e}"))?;

    // Ad-hoc codesign on macOS so the updated binary retains keychain
    // access without prompting for the login keychain password.
    #[cfg(target_os = "macos")]
    if status.updated()
        && let Ok(exe) = std::env::current_exe()
    {
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

    let new_version = status.version().trim_start_matches('v').to_string();
    let value = if status.updated() {
        json!({
            "previous": current,
            "current": new_version,
            "release_url": release_url(&new_version),
        })
    } else {
        json!({
            "previous": current,
            "current": new_version,
            "release_url": release_url(&new_version),
            "already_up_to_date": true,
        })
    };

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
    fn preflight_refuses_homebrew_linux() {
        let path = Utf8PathBuf::from("/home/linuxbrew/.linuxbrew/Cellar/atl/2026.18.3/bin/atl");
        let err = preflight_for_path(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Homebrew"),
            "expected Homebrew refusal, got: {msg}"
        );
        assert!(
            msg.contains("brew upgrade atl"),
            "Linuxbrew error must point at brew upgrade, got: {msg}"
        );
    }

    #[test]
    fn preflight_accepts_linuxbrew_lookalike() {
        // A user directory that merely contains "linuxbrew" in its name must
        // not be misclassified as a Linuxbrew-managed install.
        let path = Utf8PathBuf::from("/home/me/linuxbrew-tools/atl");
        preflight_for_path(&path).expect("linuxbrew-lookalike path should be accepted");
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
}
