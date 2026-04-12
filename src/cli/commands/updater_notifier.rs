//! Update notifier — prints a one-line notice to stderr after a successful
//! command if a newer GitHub release of atl is available.
//!
//! This is purely a notification. The actual upgrade path is
//! [`crate::cli::commands::updater::run_update`] (`atl self update`).
//!
//! Design goals:
//!
//! * **Best-effort**: every failure path is swallowed and logged via
//!   `tracing::debug!` / `tracing::trace!`. The notifier must never panic,
//!   return an error, or block command exit meaningfully.
//! * **Cached**: GitHub is queried at most once per 24 hours. The last check
//!   is persisted to a small TOML state file under the platform state dir.
//! * **Quiet in CI / pipes**: the notice only appears on an interactive
//!   stderr. Non-TTY stderr, `CI=1`, and `ATL_NO_UPDATE_NOTIFIER=1` all
//!   disable it.
//! * **Doesn't pollute stdout**: the notice goes to stderr so piped JSON /
//!   TOON / CSV output stays clean.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::io::IoStreams;

use super::updater::{REPO_NAME, REPO_OWNER, parse_version};

/// On-disk state cached between invocations so we hit GitHub at most once
/// per 24 hours.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    checked_at: DateTime<Utc>,
    latest_version: String,
    html_url: String,
}

/// A GitHub release as we care about it — version tag and HTML URL.
#[derive(Debug, Clone)]
struct Release {
    version: String,
    html_url: String,
}

/// Subset of the GitHub release JSON payload we actually read.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

/// Check for a newer release and print a notice to stderr if one is found.
///
/// This is best-effort: any failure (missing state dir, network error,
/// GitHub downtime, parse error) is logged at `debug`/`trace` level and
/// swallowed. The notifier never returns an error and never panics.
///
/// Pass `skip_if_update_command = true` when the invoking command was
/// `atl self check` or `atl self update`, so we don't double-print during
/// an explicit update workflow.
pub fn maybe_print_notice(io: &mut IoStreams, skip_if_update_command: bool) {
    if should_skip(io, skip_if_update_command) {
        return;
    }

    let now = Utc::now();
    let cached = read_state();

    let refresh = cached.as_ref().is_none_or(|s| needs_refresh(s, now));

    let release = if refresh {
        match fetch_latest_release() {
            Ok(rel) => {
                let state = State {
                    checked_at: now,
                    latest_version: rel.version.clone(),
                    html_url: rel.html_url.clone(),
                };
                if let Err(e) = write_state(&state) {
                    debug!("update-notifier: failed to write state file: {e}");
                }
                Some(rel)
            }
            Err(e) => {
                debug!("update-notifier: fetch_latest_release failed: {e}");
                // If we failed to fetch but have a cached entry, reuse it
                // so the notice still shows until the next successful check.
                cached.as_ref().map(|s| Release {
                    version: s.latest_version.clone(),
                    html_url: s.html_url.clone(),
                })
            }
        }
    } else {
        cached.as_ref().map(|s| Release {
            version: s.latest_version.clone(),
            html_url: s.html_url.clone(),
        })
    };

    let Some(rel) = release else {
        return;
    };

    let current = env!("CARGO_PKG_VERSION");
    if !is_newer(&rel.version, current) {
        trace!(
            "update-notifier: current={current} latest={} — no notice",
            rel.version
        );
        return;
    }

    if let Err(e) = print_notice(io, current, &rel) {
        debug!("update-notifier: failed to write notice: {e}");
    }
}

/// Returns `true` when we should suppress the notifier entirely for this
/// invocation.
fn should_skip(io: &IoStreams, skip_if_update_command: bool) -> bool {
    if skip_if_update_command {
        trace!("update-notifier: skipping — command was `atl self`");
        return true;
    }
    if !io.is_stderr_tty() {
        trace!("update-notifier: skipping — stderr is not a TTY");
        return true;
    }
    if std::env::var_os("ATL_NO_UPDATE_NOTIFIER").is_some() {
        trace!("update-notifier: skipping — ATL_NO_UPDATE_NOTIFIER is set");
        return true;
    }
    if std::env::var_os("CI").is_some() {
        trace!("update-notifier: skipping — CI is set");
        return true;
    }
    false
}

/// Returns the path to the cached state file, or `None` if neither the
/// state dir nor the cache dir is available. When both are `None` (very
/// unusual), the notifier skips caching entirely.
fn state_file_path() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::cache_dir)?;
    Some(base.join("atl").join("update-check.toml"))
}

/// Reads and parses the cached state file. Returns `None` if the file does
/// not exist, is unreadable, or cannot be parsed — any of these is treated
/// as "no cached state" and the notifier will attempt a fresh fetch.
fn read_state() -> Option<State> {
    let path = state_file_path()?;
    let bytes = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            trace!(
                "update-notifier: no cached state at {}: {e}",
                path.display()
            );
            return None;
        }
    };
    match toml::from_str::<State>(&bytes) {
        Ok(state) => Some(state),
        Err(e) => {
            debug!(
                "update-notifier: cached state at {} is unparseable: {e}",
                path.display()
            );
            None
        }
    }
}

/// Serializes and writes `state` to the cache file, creating parent dirs as
/// needed.
fn write_state(state: &State) -> std::io::Result<()> {
    let path = state_file_path()
        .ok_or_else(|| std::io::Error::other("no state/cache directory available"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let serialized = toml::to_string(state)
        .map_err(|e| std::io::Error::other(format!("toml serialize: {e}")))?;
    std::fs::write(&path, serialized)
}

/// Returns `true` when the cached state is older than 24 hours and a fresh
/// check should be performed.
fn needs_refresh(state: &State, now: DateTime<Utc>) -> bool {
    (now - state.checked_at) >= chrono::Duration::hours(24)
}

/// Fetches the latest release from the GitHub REST API with a tight 2 second
/// timeout. Uses a one-shot single-threaded tokio runtime because the main
/// runtime is already torn down by the time the notifier runs (and the
/// notifier is called from the sync `run()` path anyway).
fn fetch_latest_release() -> anyhow::Result<Release> {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let rel: GithubRelease = rt.block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        let resp = client
            .get(&url)
            .header("User-Agent", "atl-update-check")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()?;
        resp.json::<GithubRelease>().await
    })?;

    let version = rel.tag_name.trim_start_matches('v').to_string();
    Ok(Release {
        version,
        html_url: rel.html_url,
    })
}

/// Compares two `YYYY.WW.BUILD` calendar versions and returns whether
/// `latest` is strictly newer than `current`. If either version fails to
/// parse the result is `false` — we never want to nag the user based on an
/// unparseable tag.
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

/// Writes the two-line update notice to stderr.
fn print_notice(io: &mut IoStreams, current: &str, rel: &Release) -> std::io::Result<()> {
    let mut err = io.stderr();
    writeln!(
        err,
        "A new release of atl is available: {current} -> {latest}",
        latest = rel.version
    )?;
    writeln!(err, "{}", rel.html_url)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_strict_greater() {
        assert!(is_newer("2026.18.1", "2026.15.1"));
    }

    #[test]
    fn is_newer_same_version_is_false() {
        assert!(!is_newer("2026.15.1", "2026.15.1"));
    }

    #[test]
    fn is_newer_older_is_false() {
        assert!(!is_newer("2026.15.0", "2026.15.1"));
    }

    #[test]
    fn is_newer_handles_year_rollover() {
        assert!(is_newer("2027.01.1", "2026.52.3"));
    }

    #[test]
    fn is_newer_unparseable_latest_is_false() {
        assert!(!is_newer("not-a-version", "2026.15.1"));
    }

    #[test]
    fn is_newer_unparseable_current_is_false() {
        assert!(!is_newer("2026.18.1", "not-a-version"));
    }

    #[test]
    fn is_newer_strips_v_prefix() {
        // parse_version itself strips `v` in the notifier path via the
        // shared implementation on `updater::parse_version`, which does not
        // strip `v`. The notifier passes pre-stripped strings from
        // GithubRelease.tag_name. Verify plain numeric comparison here.
        assert!(is_newer("2026.18.1", "2026.18.0"));
    }

    #[test]
    fn needs_refresh_within_24h_is_false() {
        let now = Utc::now();
        let state = State {
            checked_at: now - chrono::Duration::hours(23),
            latest_version: "2026.18.1".into(),
            html_url: "https://example.invalid/".into(),
        };
        assert!(!needs_refresh(&state, now));
    }

    #[test]
    fn needs_refresh_after_24h_is_true() {
        let now = Utc::now();
        let state = State {
            checked_at: now - chrono::Duration::hours(25),
            latest_version: "2026.18.1".into(),
            html_url: "https://example.invalid/".into(),
        };
        assert!(needs_refresh(&state, now));
    }

    #[test]
    fn needs_refresh_exactly_24h_is_true() {
        let now = Utc::now();
        let state = State {
            checked_at: now - chrono::Duration::hours(24),
            latest_version: "2026.18.1".into(),
            html_url: "https://example.invalid/".into(),
        };
        assert!(needs_refresh(&state, now));
    }

    #[test]
    fn should_skip_when_skip_flag_set() {
        let io = IoStreams::test();
        assert!(should_skip(&io, true));
    }

    #[test]
    fn should_skip_when_stderr_not_tty() {
        // Test IoStreams has is_stderr_tty = false by construction.
        let io = IoStreams::test();
        assert!(should_skip(&io, false));
    }

    #[test]
    fn state_toml_roundtrip() {
        let original = State {
            checked_at: DateTime::parse_from_rfc3339("2026-04-11T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            latest_version: "2026.18.1".into(),
            html_url: "https://github.com/mazuninky/atl/releases/tag/v2026.18.1".into(),
        };

        let serialized = toml::to_string(&original).expect("serialize");
        let roundtripped: State = toml::from_str(&serialized).expect("deserialize");

        assert_eq!(roundtripped.checked_at, original.checked_at);
        assert_eq!(roundtripped.latest_version, original.latest_version);
        assert_eq!(roundtripped.html_url, original.html_url);
    }
}
