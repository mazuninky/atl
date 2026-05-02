//! `atl browse` — open a Confluence page or Jira issue in a browser.
//!
//! When stdout is a TTY, delegates to `webbrowser::open` to launch the
//! user's default browser. When stdout is piped or running in CI, prints
//! the resolved URL to stdout so it can be piped into `xargs open`.
//!
//! Service auto-detection: a target matching the Jira key shape
//! (`^[A-Z][A-Z0-9_]*-\d+$`) is routed to Jira; anything else is treated as
//! a Confluence page ID.

use std::io::Write;

use anyhow::{Result, anyhow};
use camino::Utf8Path;

use crate::auth::{SecretStore, SystemKeyring};
use crate::cli::args::{BrowseArgs, BrowseService};
use crate::cli::commands::confluence_url::build_confluence_url;
use crate::client::{RetryConfig, raw_request};
use crate::config::{AtlassianInstance, ConfigLoader};
use crate::io::IoStreams;

/// Entry point for `atl browse`.
pub async fn run(
    args: &BrowseArgs,
    config_path: Option<&Utf8Path>,
    profile_name: Option<&str>,
    retry_cfg: RetryConfig,
    io: &mut IoStreams,
) -> Result<()> {
    let service = resolve_service(args.service, &args.target);

    let config = ConfigLoader::load(config_path)?;
    let resolved_profile_name = profile_name
        .or(config.as_ref().map(|c| c.default_profile.as_str()))
        .unwrap_or("default");
    let profile = config
        .as_ref()
        .and_then(|c| c.resolve_profile(Some(resolved_profile_name)))
        .ok_or_else(|| anyhow!("no profile found; run `atl init` first"))?;
    let store = SystemKeyring;

    let url = match service {
        BrowseService::Jira => {
            let instance = profile
                .jira
                .as_ref()
                .ok_or_else(|| anyhow!("no Jira instance configured in profile"))?;
            jira_url(instance, &args.target)
        }
        BrowseService::Confluence => {
            let instance = profile
                .confluence
                .as_ref()
                .ok_or_else(|| anyhow!("no Confluence instance configured in profile"))?;
            confluence_url(
                instance,
                resolved_profile_name,
                &store,
                &args.target,
                retry_cfg,
            )
            .await?
        }
        // `resolve_service` never returns `Auto`.
        BrowseService::Auto => unreachable!("Auto resolved above"),
    };

    if io.is_stdout_tty() {
        // Best-effort: if launching the browser fails (e.g. no DISPLAY, no
        // `open`/`xdg-open` on PATH) we fall back to printing the URL so the
        // user can still act on it.
        if webbrowser::open(&url).is_err() {
            let mut stdout = io.stdout();
            writeln!(stdout, "{url}")?;
            stdout.flush()?;
        }
    } else {
        let mut stdout = io.stdout();
        writeln!(stdout, "{url}")?;
        stdout.flush()?;
    }

    Ok(())
}

/// Resolves [`BrowseService::Auto`] to a concrete service based on the
/// target's shape. Non-auto values are returned unchanged.
fn resolve_service(service: BrowseService, target: &str) -> BrowseService {
    match service {
        BrowseService::Auto => detect_service(target),
        other => other,
    }
}

/// Detects which service a bare `target` refers to. Returns
/// [`BrowseService::Jira`] for strings matching the Jira key pattern,
/// [`BrowseService::Confluence`] otherwise.
fn detect_service(target: &str) -> BrowseService {
    if looks_like_jira_key(target) {
        BrowseService::Jira
    } else {
        BrowseService::Confluence
    }
}

/// Returns `true` iff `s` matches the Jira issue key shape
/// `^[A-Z][A-Z0-9_]*-\d+$`. Kept as a hand-rolled predicate to avoid
/// pulling in `regex` for a single pattern.
fn looks_like_jira_key(s: &str) -> bool {
    let mut parts = s.splitn(2, '-');
    let project = parts.next().unwrap_or("");
    let number = parts.next().unwrap_or("");

    if project.is_empty() || number.is_empty() {
        return false;
    }
    if !project
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
    {
        return false;
    }
    if !project
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return false;
    }
    number.chars().all(|c| c.is_ascii_digit())
}

/// Builds the browser URL for a Jira issue directly from the profile's
/// configured domain — no network round trip required.
fn jira_url(instance: &AtlassianInstance, key: &str) -> String {
    let domain = instance.domain.trim_end_matches('/');
    let scheme = if domain.starts_with("http://") || domain.starts_with("https://") {
        ""
    } else {
        "https://"
    };
    format!("{scheme}{domain}/browse/{key}")
}

/// Resolves a Confluence page ID to its human-facing web URL.
///
/// The browser URL is **always** anchored to the profile's configured
/// `instance.domain` — never to the host of the server-supplied
/// `_links.base`. Trusting `_links.base`'s host would let a compromised
/// or MITM-proxied Confluence instance redirect the user's browser to an
/// arbitrary origin under the guise of "open Confluence page X". The
/// path component of `_links.base` is used as a context prefix only
/// after its host is validated against the configured domain (so the
/// canonical Confluence Cloud `/wiki` prefix is preserved). The
/// `_links.webui` value is validated to be a clean server-relative path
/// before concatenation (see [`build_confluence_url`]).
async fn confluence_url(
    instance: &AtlassianInstance,
    profile: &str,
    store: &dyn SecretStore,
    page_id: &str,
    retry_cfg: RetryConfig,
) -> Result<String> {
    // Prefer the v2 endpoint since the rest of the code base probes and
    // upgrades to it. The v2 shape is `{ "_links": { "webui": ..., "base": ... } }`
    // which happens to match the v1 shape for this particular field, so the
    // same extraction works either way.
    let endpoint = format!("/wiki/api/v2/pages/{page_id}");
    let page = raw_request(
        instance,
        profile,
        "confluence",
        store,
        reqwest::Method::GET,
        &endpoint,
        reqwest::header::HeaderMap::new(),
        &[],
        None,
        retry_cfg,
    )
    .await?;

    let webui = page
        .pointer("/_links/webui")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Confluence response missing _links.webui for page {page_id}"))?;
    let base = page.pointer("/_links/base").and_then(|v| v.as_str());

    build_confluence_url(&instance.domain, base, webui)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AtlassianInstance, AuthType};

    fn instance(domain: &str) -> AtlassianInstance {
        AtlassianInstance {
            domain: domain.to_string(),
            email: None,
            api_token: None,
            auth_type: AuthType::default(),
            api_path: None,
            read_only: false,
            flavor: None,
        }
    }

    #[test]
    fn looks_like_jira_key_accepts_basic() {
        assert!(looks_like_jira_key("PROJ-123"));
    }

    #[test]
    fn looks_like_jira_key_accepts_underscore() {
        assert!(looks_like_jira_key("MY_PROJ-1"));
    }

    #[test]
    fn looks_like_jira_key_accepts_digits_in_project() {
        assert!(looks_like_jira_key("PROJ2-99"));
    }

    #[test]
    fn looks_like_jira_key_rejects_lowercase() {
        assert!(!looks_like_jira_key("proj-123"));
    }

    #[test]
    fn looks_like_jira_key_rejects_pure_number() {
        assert!(!looks_like_jira_key("123"));
    }

    #[test]
    fn looks_like_jira_key_rejects_non_numeric_suffix() {
        assert!(!looks_like_jira_key("PROJ-abc"));
    }

    #[test]
    fn looks_like_jira_key_rejects_missing_dash() {
        assert!(!looks_like_jira_key("PROJ"));
    }

    #[test]
    fn looks_like_jira_key_rejects_leading_digit() {
        assert!(!looks_like_jira_key("1PROJ-1"));
    }

    #[test]
    fn looks_like_jira_key_rejects_empty_number() {
        assert!(!looks_like_jira_key("PROJ-"));
    }

    #[test]
    fn detect_service_routes_jira_key() {
        assert!(matches!(detect_service("PROJ-123"), BrowseService::Jira));
    }

    #[test]
    fn detect_service_routes_numeric_id_to_confluence() {
        assert!(matches!(detect_service("12345"), BrowseService::Confluence));
    }

    #[test]
    fn detect_service_routes_slug_to_confluence() {
        assert!(matches!(
            detect_service("random-slug"),
            BrowseService::Confluence
        ));
    }

    #[test]
    fn resolve_service_respects_explicit_choice() {
        assert!(matches!(
            resolve_service(BrowseService::Confluence, "PROJ-123"),
            BrowseService::Confluence
        ));
        assert!(matches!(
            resolve_service(BrowseService::Jira, "12345"),
            BrowseService::Jira
        ));
    }

    #[test]
    fn resolve_service_auto_delegates_to_detect() {
        assert!(matches!(
            resolve_service(BrowseService::Auto, "PROJ-7"),
            BrowseService::Jira
        ));
        assert!(matches!(
            resolve_service(BrowseService::Auto, "42"),
            BrowseService::Confluence
        ));
    }

    #[test]
    fn jira_url_bare_domain_adds_https() {
        let inst = instance("example.atlassian.net");
        assert_eq!(
            jira_url(&inst, "PROJ-1"),
            "https://example.atlassian.net/browse/PROJ-1"
        );
    }

    #[test]
    fn jira_url_preserves_explicit_scheme() {
        let inst = instance("https://jira.example.com/");
        assert_eq!(
            jira_url(&inst, "FOO-42"),
            "https://jira.example.com/browse/FOO-42"
        );
    }

    #[test]
    fn jira_url_preserves_http_scheme() {
        let inst = instance("http://localhost:8080");
        assert_eq!(
            jira_url(&inst, "DEV-9"),
            "http://localhost:8080/browse/DEV-9"
        );
    }
}
