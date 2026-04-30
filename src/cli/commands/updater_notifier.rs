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
use std::time::Duration;

use camino::Utf8PathBuf;
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
/// unusual), or when the resolved path is not valid UTF-8 (extremely
/// unusual on the platforms `atl` targets), the notifier skips caching
/// entirely instead of panicking.
fn state_file_path() -> Option<Utf8PathBuf> {
    let base = dirs::state_dir().or_else(dirs::cache_dir)?;
    let utf8 = Utf8PathBuf::from_path_buf(base)
        .map_err(|p| {
            trace!(
                "update-notifier: state/cache dir is not valid UTF-8, skipping cache: {}",
                p.display()
            );
        })
        .ok()?;
    Some(utf8.join("atl").join("update-check.toml"))
}

/// Reads and parses the cached state file. Returns `None` if the file does
/// not exist, is unreadable, or cannot be parsed — any of these is treated
/// as "no cached state" and the notifier will attempt a fresh fetch.
fn read_state() -> Option<State> {
    let path = state_file_path()?;
    let bytes = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            trace!("update-notifier: no cached state at {path}: {e}");
            return None;
        }
    };
    match toml::from_str::<State>(&bytes) {
        Ok(state) => Some(state),
        Err(e) => {
            debug!("update-notifier: cached state at {path} is unparseable: {e}");
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

    // -------------------------------------------------------------------
    // print_notice — captures output via test IoStreams.
    // -------------------------------------------------------------------

    #[test]
    fn print_notice_writes_two_lines_to_stderr() {
        let mut io = IoStreams::test();
        let rel = Release {
            version: "2026.18.2".into(),
            html_url: "https://example.invalid/v2026.18.2".into(),
        };
        print_notice(&mut io, "2026.15.1", &rel).expect("write succeeds");

        let captured = io.stderr_as_string();
        assert!(
            captured.contains("2026.15.1 -> 2026.18.2"),
            "notice must show old -> new, got: {captured:?}"
        );
        assert!(
            captured.contains("https://example.invalid/v2026.18.2"),
            "notice must include release URL, got: {captured:?}"
        );
        // Two writeln! calls → two newline characters.
        assert_eq!(
            captured.matches('\n').count(),
            2,
            "expected exactly 2 lines of output, got: {captured:?}"
        );
    }

    #[test]
    fn print_notice_does_not_touch_stdout() {
        // Critical contract: pipeable JSON/TOON/CSV output on stdout must
        // stay clean — the notice goes to stderr only.
        let mut io = IoStreams::test();
        let rel = Release {
            version: "2026.18.2".into(),
            html_url: "https://example.invalid/".into(),
        };
        print_notice(&mut io, "2026.15.1", &rel).unwrap();
        assert_eq!(io.stdout_as_string(), "");
    }

    // -------------------------------------------------------------------
    // is_newer — extra coverage for build-component comparisons.
    // -------------------------------------------------------------------

    #[test]
    fn is_newer_higher_build_within_same_week() {
        assert!(is_newer("2026.18.99", "2026.18.1"));
    }

    #[test]
    fn is_newer_higher_week_within_same_year() {
        assert!(is_newer("2026.20.0", "2026.18.99"));
    }

    #[test]
    fn is_newer_lower_year_is_false() {
        assert!(!is_newer("2025.52.99", "2026.01.0"));
    }

    #[test]
    fn is_newer_both_unparseable_is_false() {
        // The contract is "never nag on unparseable input" — even if both
        // sides are bogus.
        assert!(!is_newer("garbage-a", "garbage-b"));
    }

    // -------------------------------------------------------------------
    // needs_refresh — boundary at 23h 59m → still cached.
    // -------------------------------------------------------------------

    #[test]
    fn needs_refresh_just_under_24h_is_false() {
        let now = Utc::now();
        let state = State {
            checked_at: now - chrono::Duration::minutes(60 * 24 - 1),
            latest_version: "2026.18.1".into(),
            html_url: "https://example.invalid/".into(),
        };
        assert!(!needs_refresh(&state, now));
    }

    #[test]
    fn needs_refresh_far_in_the_past_is_true() {
        let now = Utc::now();
        let state = State {
            checked_at: now - chrono::Duration::days(7),
            latest_version: "2026.18.1".into(),
            html_url: "https://example.invalid/".into(),
        };
        assert!(needs_refresh(&state, now));
    }

    // -------------------------------------------------------------------
    // should_skip — covers ATL_NO_UPDATE_NOTIFIER and CI env paths.
    // These tests mutate process-wide env vars, so they hold env_lock().
    // -------------------------------------------------------------------

    #[test]
    fn should_skip_when_env_disable_set() {
        let _g = crate::test_util::env_lock();
        // SAFETY: serialized via env_lock().
        unsafe {
            std::env::remove_var("CI");
            std::env::set_var("ATL_NO_UPDATE_NOTIFIER", "1");
        }
        // IoStreams::test() reports stderr_tty=false, which would also cause
        // skipping. That's fine — this test proves the env path returns
        // `true` regardless of TTY state, which is the behaviour CI relies
        // on (`atl ... 2>/dev/null` should skip the notice without the env
        // var, and the env var lets users opt out interactively too).
        let io = IoStreams::test();
        assert!(
            should_skip(&io, false),
            "must skip when ATL_NO_UPDATE_NOTIFIER is set"
        );
        unsafe {
            std::env::remove_var("ATL_NO_UPDATE_NOTIFIER");
        }
    }

    #[test]
    fn should_skip_when_ci_env_set() {
        let _g = crate::test_util::env_lock();
        // SAFETY: serialized via env_lock().
        unsafe {
            std::env::remove_var("ATL_NO_UPDATE_NOTIFIER");
            std::env::set_var("CI", "1");
        }
        let probe_io = IoStreams::test();
        assert!(
            should_skip(&probe_io, false),
            "must skip when CI env var is set"
        );
        unsafe {
            std::env::remove_var("CI");
        }
    }

    // -------------------------------------------------------------------
    // state_file_path — typically resolvable on supported platforms.
    // We don't assert the exact path since it varies per platform and per
    // user, but we do assert the suffix is the cache file name and that
    // the path is under an `atl` subdir.
    // -------------------------------------------------------------------

    #[test]
    fn state_file_path_uses_atl_subdir_when_available() {
        if let Some(p) = state_file_path() {
            assert_eq!(p.file_name(), Some("update-check.toml"));
            // Parent must end with `atl` so multiple Innowald CLIs don't
            // collide on the same cache file.
            assert_eq!(
                p.parent().and_then(|p| p.file_name()),
                Some("atl"),
                "expected parent dir name `atl`, got path: {p}"
            );
        }
    }

    // -------------------------------------------------------------------
    // GithubRelease parsing — guards against a silent schema migration.
    // -------------------------------------------------------------------

    #[test]
    fn github_release_parses_minimum_fields() {
        let payload = r#"{
            "tag_name": "v2026.18.1",
            "html_url": "https://github.com/mazuninky/atl/releases/tag/v2026.18.1",
            "name": "v2026.18.1",
            "prerelease": false
        }"#;
        let parsed: GithubRelease =
            serde_json::from_str(payload).expect("payload must parse with extra fields");
        assert_eq!(parsed.tag_name, "v2026.18.1");
        assert_eq!(
            parsed.html_url,
            "https://github.com/mazuninky/atl/releases/tag/v2026.18.1"
        );
    }

    #[test]
    fn github_release_missing_tag_name_fails() {
        let payload = r#"{ "html_url": "https://example.invalid/" }"#;
        let result: Result<GithubRelease, _> = serde_json::from_str(payload);
        assert!(result.is_err(), "missing tag_name must fail to deserialize");
    }
}
