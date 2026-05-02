//! Shared, validated builder for Confluence browser URLs.
//!
//! The browser URL for a Confluence page is **always** anchored to the
//! profile's configured `instance.domain` — never to the host portion of
//! the server-supplied `_links.base`. Trusting `_links.base`'s host would
//! let a compromised or MITM-proxied Confluence instance redirect the
//! user's browser (or any downstream tool that follows the URL) to an
//! arbitrary origin under the guise of "open Confluence page X".
//!
//! However, on Confluence Cloud the canonical browser URL takes the form
//! `https://<host>/wiki/<webui-path>`: the `/wiki` *context path* lives
//! inside `_links.base`, not in `_links.webui`. Dropping the context path
//! produces a 404. So we accept `_links.base` for its **path component
//! only**, after validating that its host matches the configured domain.
//! A host mismatch logs a `warn!` and falls back to no context prefix —
//! preserving the security goal while staying functional in environments
//! without a context path.
//!
//! `_links.webui` is treated as untrusted input and validated to be a
//! clean server-relative path before concatenation.
//!
//! This module centralizes the validation so every code path that emits a
//! Confluence URL (the `atl browse` command, the `confluence read` JSON
//! output, etc.) produces identical URLs and rejects identical inputs.

use anyhow::{Result, anyhow};
use tracing::warn;

/// Builds and validates a Confluence browser URL.
///
/// - `domain` is the locally configured host (always honored).
/// - `base` is the optional `_links.base` from the server response. When
///   present and its host matches `domain` (case-insensitive,
///   scheme-insensitive), its path component is used as a context prefix
///   (e.g. `/wiki` on Confluence Cloud). When absent or its host differs,
///   no context prefix is used.
/// - `webui` is the server-supplied path component, treated as untrusted
///   input. It must be:
///   - non-empty
///   - starts with `/` but **not** `//` (rejects scheme-relative URLs like
///     `//attacker.com/x` which `webbrowser::open` would treat as a host)
///   - does not contain `://` (rejects full URL injection)
///   - does not contain `\` (rejects Windows-style path-traversal attempts)
///   - does not contain control characters (rejects newlines, escapes, NUL)
pub fn build_confluence_url(domain: &str, base: Option<&str>, webui: &str) -> Result<String> {
    if webui.is_empty()
        || !webui.starts_with('/')
        || webui.starts_with("//")
        || webui.contains("://")
        || webui.contains('\\')
        || webui.chars().any(|c| c.is_control())
    {
        return Err(anyhow!(
            "Confluence response returned an unsafe webui path: {webui:?}"
        ));
    }

    let domain = domain.trim_end_matches('/');
    let scheme = if domain.starts_with("http://") || domain.starts_with("https://") {
        ""
    } else {
        "https://"
    };
    let domain_host = strip_scheme(domain);

    let context_prefix = base
        .and_then(|b| context_prefix_from_base(b, domain_host))
        .unwrap_or_default();

    Ok(format!("{scheme}{domain}{context_prefix}{webui}"))
}

/// Returns the path component of `base` if its host matches
/// `expected_host` case-insensitively. On host mismatch, logs a warning
/// and returns `None`. On parse failure, returns `None` silently —
/// callers fall back to no prefix.
fn context_prefix_from_base(base: &str, expected_host: &str) -> Option<String> {
    let base = base.trim_end_matches('/');
    let after_scheme = base.split_once("://").map(|(_, rest)| rest).unwrap_or(base);

    // Split the authority (host[:port]) from the path on the first `/`.
    let (authority, path) = match after_scheme.split_once('/') {
        Some((auth, p)) => (auth, format!("/{p}")),
        None => (after_scheme, String::new()),
    };

    // Strip port if present.
    let host = authority.split_once(':').map_or(authority, |(h, _)| h);

    if host.eq_ignore_ascii_case(expected_host) {
        if path.is_empty() { None } else { Some(path) }
    } else {
        warn!(
            "Confluence _links.base host mismatch: configured={expected_host}, server={host}; \
             ignoring server base"
        );
        None
    }
}

/// Strips a leading `http://` or `https://` from a string, returning the
/// authority + path portion. Used to compare hosts against
/// scheme-prefixed configured domains.
fn strip_scheme(s: &str) -> &str {
    s.strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_confluence_url_uses_base_path_as_context_prefix() {
        // The canonical Confluence Cloud case: `_links.base` carries the
        // `/wiki` context path, `_links.webui` is the path after `/wiki`.
        let url = build_confluence_url(
            "example.atlassian.net",
            Some("https://example.atlassian.net/wiki"),
            "/spaces/X/pages/123/Title",
        )
        .expect("valid inputs should produce a URL");
        assert_eq!(
            url,
            "https://example.atlassian.net/wiki/spaces/X/pages/123/Title"
        );
    }

    #[test]
    fn build_confluence_url_no_context_path_when_base_has_no_path() {
        // Server-managed Confluence without a context path.
        let url = build_confluence_url(
            "example.atlassian.net",
            Some("https://example.atlassian.net"),
            "/x",
        )
        .expect("valid inputs should produce a URL");
        assert_eq!(url, "https://example.atlassian.net/x");
    }

    #[test]
    fn build_confluence_url_ignores_attacker_base_host() {
        // Host mismatch: the server's `_links.base` points at an attacker
        // origin. We must ignore the entire base (including its path) and
        // fall back to no context prefix — never let the attacker's path
        // shape the URL either.
        let url = build_confluence_url(
            "example.atlassian.net",
            Some("https://attacker.example/wiki"),
            "/x",
        )
        .expect("valid inputs should produce a URL");
        assert_eq!(url, "https://example.atlassian.net/x");
    }

    #[test]
    fn build_confluence_url_no_base_uses_no_context_prefix() {
        let url = build_confluence_url("example.atlassian.net", None, "/x")
            .expect("valid inputs should produce a URL");
        assert_eq!(url, "https://example.atlassian.net/x");
    }

    #[test]
    fn build_confluence_url_host_match_is_case_insensitive() {
        let url = build_confluence_url(
            "example.atlassian.net",
            Some("https://EXAMPLE.atlassian.net/wiki"),
            "/x",
        )
        .expect("case-insensitive host match should succeed");
        assert_eq!(url, "https://example.atlassian.net/wiki/x");
    }

    #[test]
    fn build_confluence_url_host_match_with_port_in_base() {
        // Defensive: a base with an explicit port should compare on host
        // alone. (We don't currently honor a port from base, but at minimum
        // the host comparison must not be confused by a `:port` suffix.)
        let url = build_confluence_url(
            "example.atlassian.net",
            Some("https://example.atlassian.net:8443/wiki"),
            "/x",
        )
        .expect("port in base should not derail host match");
        assert_eq!(url, "https://example.atlassian.net/wiki/x");
    }

    #[test]
    fn build_confluence_url_preserves_explicit_scheme_on_domain() {
        let url = build_confluence_url(
            "https://example.atlassian.net",
            Some("https://example.atlassian.net/wiki"),
            "/x",
        )
        .expect("explicit scheme on domain should pass through");
        assert_eq!(url, "https://example.atlassian.net/wiki/x");
    }

    #[test]
    fn build_confluence_url_strips_trailing_slash_on_domain() {
        let url = build_confluence_url("example.atlassian.net/", None, "/x")
            .expect("valid inputs should produce a URL");
        assert_eq!(url, "https://example.atlassian.net/x");
    }

    #[test]
    fn build_confluence_url_strips_trailing_slash_on_base_path() {
        let url = build_confluence_url(
            "example.atlassian.net",
            Some("https://example.atlassian.net/wiki/"),
            "/x",
        )
        .expect("trailing slash on base should not produce double-slash");
        assert_eq!(url, "https://example.atlassian.net/wiki/x");
    }

    #[test]
    fn build_confluence_url_rejects_empty_webui() {
        let err = build_confluence_url("example.atlassian.net", None, "")
            .expect_err("empty webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_no_leading_slash() {
        let err = build_confluence_url("example.atlassian.net", None, "no-leading-slash")
            .expect_err("webui without leading slash must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_scheme_relative() {
        let err = build_confluence_url("example.atlassian.net", None, "//attacker.com/x")
            .expect_err("scheme-relative webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_full_url() {
        let err = build_confluence_url("example.atlassian.net", None, "https://attacker.com/x")
            .expect_err("full URL in webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_control_character() {
        let err = build_confluence_url("example.atlassian.net", None, "/path\x1bevil")
            .expect_err("control characters in webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_backslash() {
        let err = build_confluence_url("example.atlassian.net", None, "/path\\..\\..\\evil")
            .expect_err("backslash in webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_newline() {
        let err = build_confluence_url("example.atlassian.net", None, "/path\nnewline")
            .expect_err("newline in webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }
}
