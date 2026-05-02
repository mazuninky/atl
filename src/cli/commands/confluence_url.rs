//! Shared, validated builder for Confluence browser URLs.
//!
//! The browser URL for a Confluence page is **always** built from the
//! profile's configured `instance.domain`, never from the server-supplied
//! `_links.base`. Trusting `_links.base` would let a compromised or
//! MITM-proxied Confluence instance redirect the user's browser (or any
//! downstream tool that follows the URL) to an arbitrary origin under the
//! guise of "open Confluence page X". The `_links.webui` value is treated
//! as untrusted input and validated to be a clean server-relative path
//! before concatenation.
//!
//! This module centralizes the validation so every code path that emits a
//! Confluence URL (the `atl browse` command, the `confluence read` JSON
//! output, etc.) produces identical URLs and rejects identical inputs.

use anyhow::{Result, anyhow};

/// Builds and validates a Confluence browser URL from a configured
/// `domain` and a server-supplied `_links.webui` path.
///
/// The origin is always taken from `domain` (trusted, comes from the
/// user's local profile). `webui` is treated as untrusted input and must
/// be a clean server-relative path:
///
/// - non-empty
/// - starts with `/` but **not** `//` (rejects scheme-relative URLs like
///   `//attacker.com/x` which `webbrowser::open` would treat as a host)
/// - does not contain `://` (rejects full URL injection)
/// - does not contain `\` (rejects Windows-style path-traversal attempts)
/// - does not contain control characters (rejects newlines, escapes, NUL)
pub fn build_confluence_url(domain: &str, webui: &str) -> Result<String> {
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
    Ok(format!("{scheme}{domain}{webui}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_confluence_url_bare_domain_adds_https() {
        let url = build_confluence_url("example.atlassian.net", "/wiki/spaces/X/pages/123")
            .expect("valid webui path should produce a URL");
        assert_eq!(url, "https://example.atlassian.net/wiki/spaces/X/pages/123");
    }

    #[test]
    fn build_confluence_url_preserves_explicit_scheme() {
        let url = build_confluence_url("https://example.atlassian.net", "/wiki/spaces/X")
            .expect("valid webui path should produce a URL");
        assert_eq!(url, "https://example.atlassian.net/wiki/spaces/X");
    }

    #[test]
    fn build_confluence_url_strips_trailing_slash_on_domain() {
        let url = build_confluence_url("example.atlassian.net/", "/x")
            .expect("valid webui path should produce a URL");
        assert_eq!(url, "https://example.atlassian.net/x");
    }

    #[test]
    fn build_confluence_url_rejects_empty_webui() {
        let err = build_confluence_url("example.atlassian.net", "")
            .expect_err("empty webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_no_leading_slash() {
        let err = build_confluence_url("example.atlassian.net", "no-leading-slash")
            .expect_err("webui without leading slash must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_scheme_relative() {
        let err = build_confluence_url("example.atlassian.net", "//attacker.com/x")
            .expect_err("scheme-relative webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_full_url() {
        let err = build_confluence_url("example.atlassian.net", "https://attacker.com/x")
            .expect_err("full URL in webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_control_character() {
        let err = build_confluence_url("example.atlassian.net", "/path\x1bevil")
            .expect_err("control characters in webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_backslash() {
        let err = build_confluence_url("example.atlassian.net", "/path\\..\\..\\evil")
            .expect_err("backslash in webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }

    #[test]
    fn build_confluence_url_rejects_newline() {
        let err = build_confluence_url("example.atlassian.net", "/path\nnewline")
            .expect_err("newline in webui must be rejected");
        assert!(err.to_string().contains("unsafe webui path"));
    }
}
