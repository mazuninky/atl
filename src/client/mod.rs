mod confluence;
mod jira;

pub use confluence::ConfluenceClient;
pub use jira::JiraClient;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest_middleware::{ClientBuilder as MwClientBuilder, ClientWithMiddleware};
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};
use tracing::debug;

use crate::auth::SecretStore;
use crate::config::{AtlassianInstance, AuthType};
use crate::error::Error;

/// Crate-internal alias for the middleware-wrapped reqwest client. All
/// Atlassian HTTP traffic flows through this type so the retry layer is
/// uniformly applied.
pub(crate) type HttpClient = ClientWithMiddleware;

/// Builds an authenticated HTTP client for an Atlassian instance with an
/// optional retry layer.
///
/// When `retries > 0`, wraps the underlying [`reqwest::Client`] with
/// [`RetryTransientMiddleware`] which retries transient failures (5xx,
/// 429, connection errors) using exponential backoff. When `retries == 0`
/// the client is returned with no retry layer, still wrapped in a
/// [`ClientWithMiddleware`] so the call sites have a uniform type.
///
/// Note on idempotency: the default
/// [`RetryTransientMiddleware`] retries on transient *status codes*
/// regardless of HTTP method. That means a POST returning 503 will be
/// retried, which could in theory double-submit a write. Callers who are
/// concerned about double-submission should run with `--retries 0`.
///
/// Token resolution uses the full `env → TOML → keyring` chain via
/// [`AtlassianInstance::resolved_token`], so callers do not need to
/// pre-resolve the token before constructing a client.
pub(crate) fn build_http_client(
    instance: &AtlassianInstance,
    profile: &str,
    kind: &str,
    store: &dyn SecretStore,
    retries: u32,
) -> Result<HttpClient, Error> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let token = instance
        .resolved_token(profile, kind, store)
        .ok_or_else(|| {
            Error::Auth(
                "no API token configured; run `atl auth login` or set ATL_API_TOKEN env var".into(),
            )
        })?;

    let auth_value = match instance.auth_type {
        AuthType::Basic => {
            use base64::Engine;
            let email = instance.email.as_deref().ok_or_else(|| {
                Error::Auth("email is required for Basic auth; set email in config".into())
            })?;
            let credentials = format!("{email}:{token}");
            let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
            format!("Basic {encoded}")
        }
        AuthType::Bearer => format!("Bearer {token}"),
    };

    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&auth_value).map_err(|e| Error::Auth(e.to_string()))?,
    );

    debug!(
        "Building HTTP client for {} (retries={retries})",
        instance.domain
    );

    let base = reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(Error::Http)?;

    let client = if retries == 0 {
        MwClientBuilder::new(base).build()
    } else {
        let policy = ExponentialBackoff::builder()
            .retry_bounds(
                std::time::Duration::from_millis(200),
                std::time::Duration::from_secs(10),
            )
            .build_with_max_retries(retries);
        MwClientBuilder::new(base)
            .with(RetryTransientMiddleware::new_with_policy(policy))
            .build()
    };

    Ok(client)
}

pub(crate) fn build_base_url(instance: &AtlassianInstance, default_api_path: &str) -> String {
    let domain = instance.domain.trim_end_matches('/');
    let api_path = instance.api_path.as_deref().unwrap_or(default_api_path);
    let scheme = if domain.starts_with("http://") || domain.starts_with("https://") {
        ""
    } else {
        "https://"
    };
    format!("{scheme}{domain}{api_path}")
}

async fn handle_response(response: reqwest::Response) -> Result<serde_json::Value, Error> {
    let status = response.status();
    if status.is_success() {
        let body = response.json().await?;
        Ok(body)
    } else {
        handle_error_status(status.as_u16(), response).await
    }
}

async fn handle_response_maybe_empty(
    response: reqwest::Response,
) -> Result<serde_json::Value, Error> {
    let status = response.status();
    if status.is_success() {
        let body = response.text().await?;
        if body.is_empty() {
            Ok(serde_json::Value::Null)
        } else {
            Ok(serde_json::from_str(&body)?)
        }
    } else {
        handle_error_status(status.as_u16(), response).await
    }
}

pub(crate) async fn detect_confluence_api_path(
    http: &HttpClient,
    domain: &str,
) -> Result<String, Error> {
    let domain = domain.trim_end_matches('/');
    let scheme = if domain.starts_with("http://") || domain.starts_with("https://") {
        ""
    } else {
        "https://"
    };

    let mut last_error: Option<String> = None;
    for path in ["/wiki/rest/api", "/rest/api"] {
        let url = format!("{scheme}{domain}{path}/space?limit=1");
        debug!("Probing {url}");
        match http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                debug!("Auto-detected API path: {path}");
                return Ok(path.to_string());
            }
            Ok(resp) => {
                let status = resp.status();
                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    return Err(Error::Auth(format!(
                        "authentication failed while probing {url} ({status})"
                    )));
                }
                last_error = Some(format!("{url} returned {status}"));
            }
            Err(e) => {
                last_error = Some(format!("{url}: {e}"));
            }
        }
    }

    Err(Error::Config(format!(
        "cannot auto-detect Confluence API path; set api_path in config (last probe: {})",
        last_error.unwrap_or_default()
    )))
}

/// Issue a single authenticated HTTP request against an Atlassian instance,
/// returning the parsed JSON response.
///
/// Unlike [`build_base_url`], this helper does **not** prepend any API path —
/// the caller provides the full path in `endpoint` (e.g.
/// `/rest/api/2/myself`). The URL is constructed as `{scheme}{domain}{endpoint}`
/// where `scheme` is inferred from `instance.domain` (defaults to `https://`).
///
/// Used by the generic `atl api` passthrough command.
#[allow(clippy::too_many_arguments)]
pub async fn raw_request(
    instance: &AtlassianInstance,
    profile: &str,
    kind: &str,
    store: &dyn SecretStore,
    method: reqwest::Method,
    endpoint: &str,
    headers: HeaderMap,
    query: &[(String, String)],
    body: Option<serde_json::Value>,
    retries: u32,
) -> Result<serde_json::Value, Error> {
    if instance.read_only
        && !matches!(
            method,
            reqwest::Method::GET | reqwest::Method::HEAD | reqwest::Method::OPTIONS
        )
    {
        return Err(Error::Config(
            "profile is read_only; refusing write request".into(),
        ));
    }

    let http = build_http_client(instance, profile, kind, store, retries)?;

    let domain = instance.domain.trim_end_matches('/');
    let scheme = if domain.starts_with("http://") || domain.starts_with("https://") {
        ""
    } else {
        "https://"
    };
    let path = if endpoint.starts_with('/') {
        endpoint.to_string()
    } else {
        format!("/{endpoint}")
    };
    let url = format!("{scheme}{domain}{path}");

    debug!("{method} {url}");
    let mut req = http.request(method, &url).headers(headers);
    if !query.is_empty() {
        req = req.query(query);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await?;
    handle_response_maybe_empty(resp).await
}

pub(super) async fn handle_error_status(
    status_code: u16,
    response: reqwest::Response,
) -> Result<serde_json::Value, Error> {
    let body = read_sanitized_error_body(response).await;
    match status_code {
        401 | 403 => Err(Error::Auth(format!("{status_code}: {body}"))),
        404 => Err(Error::NotFound(body)),
        _ => Err(Error::Api {
            status: status_code,
            message: body,
        }),
    }
}

/// Read the response body as text, log the full untruncated body at debug
/// level for `--verbose` debugging, and return a sanitized form safe for
/// inclusion in user-visible error messages.
///
/// The full body is emitted on the `atl::client::error` target so it can
/// be filtered with `RUST_LOG=atl::client::error=debug`.
pub(super) async fn read_sanitized_error_body(response: reqwest::Response) -> String {
    let raw = response.text().await.unwrap_or_default();
    debug!(target: "atl::client::error", "upstream error body: {raw}");
    sanitize_error_body(&raw)
}

/// Sanitize an upstream response body for embedding in a user-facing error
/// message.
///
/// Truncates to at most 512 bytes on a UTF-8 char boundary (appending `…`
/// when truncation occurs), and replaces ASCII / Unicode "Cc" control
/// characters — anything matched by [`char::is_control`] except `\n` and
/// `\t` — with `?`. This neutralizes ANSI escape sequences (ESC, 0x1B),
/// terminal bells (0x07), NUL bytes, and similar terminal-manipulation
/// vectors that a malicious or MITM-proxied upstream could inject.
///
/// Note: Unicode bidi-override format characters (e.g. U+202E) are in the
/// `Cf` (format) category, not `Cc` (control), so [`char::is_control`]
/// does not match them and they pass through. Hardening that surface is
/// tracked separately.
fn sanitize_error_body(body: &str) -> String {
    const MAX_BYTES: usize = 512;

    // Truncate at the largest UTF-8 char boundary <= MAX_BYTES.
    let (truncated, was_truncated) = if body.len() <= MAX_BYTES {
        (body, false)
    } else {
        let cut = body
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|&idx| idx <= MAX_BYTES)
            .last()
            .unwrap_or(0);
        (&body[..cut], true)
    };

    let mut out = String::with_capacity(truncated.len() + if was_truncated { 4 } else { 0 });
    for c in truncated.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            out.push('?');
        } else {
            out.push(c);
        }
    }
    if was_truncated {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::InMemoryStore;
    use crate::config::AtlassianInstance;
    use crate::test_util::env_lock;

    fn make_instance(domain: &str, api_path: Option<&str>) -> AtlassianInstance {
        AtlassianInstance {
            domain: domain.to_string(),
            email: None,
            api_token: None,
            auth_type: AuthType::default(),
            api_path: api_path.map(String::from),
            read_only: false,
            flavor: None,
        }
    }

    /// Instance with enough credentials for `build_http_client` to succeed
    /// (Basic auth needs both email and api_token).
    fn make_authed_instance() -> AtlassianInstance {
        AtlassianInstance {
            domain: "https://example.atlassian.net".to_string(),
            email: Some("test@example.com".into()),
            api_token: Some("test-token".into()),
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: None,
        }
    }

    #[test]
    fn build_base_url_adds_https_scheme() {
        let inst = make_instance("example.atlassian.net", None);
        let url = build_base_url(&inst, "/rest/api/2");
        assert_eq!(url, "https://example.atlassian.net/rest/api/2");
    }

    #[test]
    fn build_base_url_preserves_explicit_scheme() {
        let inst = make_instance("https://my.server.com", None);
        let url = build_base_url(&inst, "/wiki/rest/api");
        assert_eq!(url, "https://my.server.com/wiki/rest/api");
    }

    #[test]
    fn build_base_url_preserves_http_scheme() {
        let inst = make_instance("http://localhost:8080", None);
        let url = build_base_url(&inst, "/rest/api/2");
        assert_eq!(url, "http://localhost:8080/rest/api/2");
    }

    #[test]
    fn build_base_url_uses_custom_api_path() {
        let inst = make_instance("example.com", Some("/custom/api"));
        let url = build_base_url(&inst, "/rest/api/2");
        assert_eq!(url, "https://example.com/custom/api");
    }

    #[test]
    fn build_base_url_strips_trailing_slash() {
        let inst = make_instance("example.com/", None);
        let url = build_base_url(&inst, "/rest/api/2");
        assert_eq!(url, "https://example.com/rest/api/2");
    }

    #[test]
    fn build_http_client_with_retries_zero_succeeds() {
        // retries == 0 must still return a functioning middleware client,
        // just without the retry layer attached.
        let inst = make_authed_instance();
        let store = InMemoryStore::new();
        let client = build_http_client(&inst, "default", "jira", &store, 0);
        assert!(
            client.is_ok(),
            "build_http_client(_, 0) should succeed: {:?}",
            client.err()
        );
    }

    #[test]
    fn build_http_client_with_retries_nonzero_succeeds() {
        // Non-zero retries attaches the RetryTransientMiddleware. We can't
        // inspect the middleware chain directly (it's opaque), so we just
        // verify the client was constructed.
        let inst = make_authed_instance();
        let store = InMemoryStore::new();
        let client = build_http_client(&inst, "default", "jira", &store, 5);
        assert!(
            client.is_ok(),
            "build_http_client(_, 5) should succeed: {:?}",
            client.err()
        );
    }

    #[test]
    fn build_http_client_without_token_errors() {
        // No api_token set and no ATL_API_TOKEN env var means the builder
        // cannot attach Authorization — retries setting must not change that.
        let _g = env_lock();
        let inst = make_instance("example.atlassian.net", None);
        let store = InMemoryStore::new();
        // Avoid env var contamination from the developer shell.
        // SAFETY: serialized via env_lock() so no concurrent thread in
        // this crate can read or write ATL_API_TOKEN while we are here.
        unsafe { std::env::remove_var("ATL_API_TOKEN") };
        let err = build_http_client(&inst, "default", "jira", &store, 0).unwrap_err();
        assert!(
            matches!(err, Error::Auth(_)),
            "expected Error::Auth, got {err:?}"
        );
    }

    #[test]
    fn build_http_client_resolves_keyring_token() {
        // Verify the full resolution chain: when no env var or TOML token
        // is set, the builder should find the token in the keyring.
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let inst = AtlassianInstance {
            domain: "https://example.atlassian.net".to_string(),
            email: Some("test@example.com".into()),
            api_token: None,
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: None,
        };
        let store = InMemoryStore::new();
        store
            .set("atl:default:jira", "test@example.com", "keyring-token")
            .unwrap();

        let client = build_http_client(&inst, "default", "jira", &store, 0);
        assert!(
            client.is_ok(),
            "build_http_client should resolve keyring token: {:?}",
            client.err()
        );
    }

    // -------------------------------------------------------------------
    // Wiring tests: keyring token resolution through client constructors
    //
    // These verify the full path from client constructor → build_http_client
    // → resolved_token → keyring. The unit test above
    // (build_http_client_resolves_keyring_token) proves the builder itself
    // calls resolved_token, but these catch a different class of bug:
    // a client constructor passing the wrong `kind`, wrong `profile`,
    // or forgetting to forward the `store` parameter.
    // -------------------------------------------------------------------

    /// Helper: instance with email but NO TOML token, forcing the resolution
    /// chain to fall through to the keyring.
    fn make_keyring_only_instance() -> AtlassianInstance {
        AtlassianInstance {
            domain: "https://example.atlassian.net".to_string(),
            email: Some("alice@acme.com".into()),
            api_token: None,
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: None,
        }
    }

    #[test]
    fn jira_client_new_resolves_keyring_token() {
        // JiraClient::new must forward the store to build_http_client with
        // kind="jira", so a keyring entry under "atl:<profile>:jira" is found.
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let inst = make_keyring_only_instance();
        let store = InMemoryStore::new();
        store
            .set("atl:default:jira", "alice@acme.com", "jira-keyring-token")
            .unwrap();

        let client = JiraClient::new(&inst, "default", &store, 0);
        assert!(
            client.is_ok(),
            "JiraClient::new should resolve keyring token, got: {:?}",
            client.err()
        );
    }

    #[test]
    fn jira_client_new_fails_without_any_token() {
        // Negative case: no env, no TOML, no keyring entry → Error::Auth.
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let inst = make_keyring_only_instance();
        let store = InMemoryStore::new();

        match JiraClient::new(&inst, "default", &store, 0) {
            Err(Error::Auth(_)) => {} // expected
            Err(other) => panic!("expected Error::Auth when no token exists, got: {other:?}"),
            Ok(_) => panic!("expected Error::Auth when no token exists, got Ok"),
        }
    }

    #[test]
    fn jira_client_new_uses_correct_profile_scope() {
        // A keyring entry for profile "staging" must NOT be found when
        // constructing with profile "default" — proves the profile parameter
        // is forwarded, not hard-coded.
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let inst = make_keyring_only_instance();
        let store = InMemoryStore::new();
        store
            .set("atl:staging:jira", "alice@acme.com", "wrong-profile-token")
            .unwrap();

        match JiraClient::new(&inst, "default", &store, 0) {
            Err(Error::Auth(_)) => {} // expected
            Err(other) => panic!(
                "expected Error::Auth when keyring entry is under wrong profile, got: {other:?}"
            ),
            Ok(_) => {
                panic!("expected Error::Auth when keyring entry is under wrong profile, got Ok")
            }
        }
    }

    #[test]
    fn confluence_client_new_resolves_keyring_token() {
        // ConfluenceClient::new must forward the store with kind="confluence".
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let inst = make_keyring_only_instance();
        let store = InMemoryStore::new();
        store
            .set(
                "atl:default:confluence",
                "alice@acme.com",
                "confluence-keyring-token",
            )
            .unwrap();

        let client = ConfluenceClient::new(&inst, "default", &store, 0);
        assert!(
            client.is_ok(),
            "ConfluenceClient::new should resolve keyring token, got: {:?}",
            client.err()
        );
    }

    #[test]
    fn confluence_client_new_fails_without_any_token() {
        // Negative case: no env, no TOML, no keyring → Error::Auth.
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let inst = make_keyring_only_instance();
        let store = InMemoryStore::new();

        match ConfluenceClient::new(&inst, "default", &store, 0) {
            Err(Error::Auth(_)) => {} // expected
            Err(other) => panic!("expected Error::Auth when no token exists, got: {other:?}"),
            Ok(_) => panic!("expected Error::Auth when no token exists, got Ok"),
        }
    }

    #[test]
    fn confluence_client_kind_is_not_jira() {
        // A keyring entry for kind="jira" must NOT satisfy ConfluenceClient::new.
        // Catches the bug where a client constructor hard-codes the wrong kind.
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let inst = make_keyring_only_instance();
        let store = InMemoryStore::new();
        store
            .set("atl:default:jira", "alice@acme.com", "jira-only-token")
            .unwrap();

        match ConfluenceClient::new(&inst, "default", &store, 0) {
            Err(Error::Auth(_)) => {} // expected
            Err(other) => panic!(
                "ConfluenceClient must use kind=\"confluence\", not \"jira\"; got: {other:?}"
            ),
            Ok(_) => panic!("ConfluenceClient must use kind=\"confluence\", not \"jira\"; got Ok"),
        }
    }

    #[tokio::test]
    async fn raw_request_fails_without_any_token() {
        // Negative case: raw_request with no token anywhere → Error::Auth
        // at the client-construction stage (never reaches the network).
        {
            let _g = env_lock();
            // SAFETY: serialized via env_lock().
            unsafe { std::env::remove_var("ATL_API_TOKEN") };
        } // lock released before any await

        let inst = make_keyring_only_instance();
        let store = InMemoryStore::new();

        let result = raw_request(
            &inst,
            "default",
            "jira",
            &store,
            reqwest::Method::GET,
            "/rest/api/2/myself",
            HeaderMap::new(),
            &[],
            None,
            0,
        )
        .await;

        match result {
            Err(Error::Auth(msg)) if msg.contains("no API token configured") => {} // expected
            Err(Error::Auth(msg)) => panic!(
                "expected 'no API token configured' Auth error, got different Auth error: {msg}"
            ),
            Err(other) => panic!("expected Error::Auth, got: {other:?}"),
            Ok(_) => panic!("expected Error::Auth when no token exists, got Ok"),
        }
    }

    #[tokio::test]
    async fn raw_request_refuses_writes_on_read_only_instance() {
        // Construct a read_only instance with just enough plumbing that, if
        // the early guard did not fire, `build_http_client` would otherwise
        // succeed. The guard must short-circuit before we ever hit the
        // network.
        let inst = AtlassianInstance {
            domain: "https://example.atlassian.net".to_string(),
            email: Some("test@example.com".into()),
            api_token: Some("test-token".into()),
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: true,
            flavor: None,
        };
        let store = InMemoryStore::new();

        let err = raw_request(
            &inst,
            "default",
            "jira",
            &store,
            reqwest::Method::POST,
            "/rest/api/2/issue",
            HeaderMap::new(),
            &[],
            Some(serde_json::json!({"fields": {}})),
            0,
        )
        .await
        .expect_err("POST on a read_only instance must error");

        match err {
            Error::Config(msg) => assert!(
                msg.contains("read_only"),
                "expected read_only in error message, got: {msg}"
            ),
            other => panic!("expected Error::Config, got: {other:?}"),
        }
    }

    // ----------------------------------------------------------------------
    // sanitize_error_body / handle_error_status — security regression tests
    //
    // These cover audit finding M1: the upstream 4xx body must not be
    // interpolated raw into user-visible error messages, since a malicious
    // or MITM-proxied Atlassian server could inject ANSI escape sequences
    // or forged instructions.
    // ----------------------------------------------------------------------

    #[test]
    fn sanitize_error_body_passes_through_short_clean_input() {
        let out = sanitize_error_body("plain ascii");
        assert_eq!(out, "plain ascii");
    }

    #[test]
    fn sanitize_error_body_preserves_newlines_and_tabs() {
        let out = sanitize_error_body("line1\nline2\twith-tab");
        assert_eq!(out, "line1\nline2\twith-tab");
    }

    #[test]
    fn sanitize_error_body_strips_esc_bell_and_nul() {
        // ESC (0x1B), bell (0x07), NUL (0x00) all become '?'.
        let input = "ansi:\x1b[2Jbell:\x07nul:\x00end";
        let out = sanitize_error_body(input);
        assert_eq!(out, "ansi:?[2Jbell:?nul:?end");
        assert!(
            !out.contains('\x1b'),
            "ESC must not survive sanitization: {out:?}"
        );
        assert!(!out.contains('\x07'));
        assert!(!out.contains('\x00'));
    }

    #[test]
    fn sanitize_error_body_truncates_at_512_bytes_with_ellipsis() {
        let input = "a".repeat(1024);
        let out = sanitize_error_body(&input);
        // 512 'a' bytes + '…' (3-byte UTF-8) = 515 bytes.
        assert_eq!(out.len(), 512 + '…'.len_utf8());
        assert!(out.ends_with('…'));
        assert!(out.starts_with(&"a".repeat(512)));
    }

    #[test]
    fn sanitize_error_body_truncates_on_utf8_char_boundary() {
        // Build input where a multi-byte char straddles the 512-byte mark.
        // 510 ASCII bytes, then '€' (3 bytes at positions 510, 511, 512),
        // then more ASCII. Cutting at 512 would split the '€' mid-codepoint.
        // The helper must instead cut at index 510 (the start of '€').
        let mut input = "a".repeat(510);
        input.push('€');
        input.push_str(&"b".repeat(100));

        let out = sanitize_error_body(&input);
        // Must be valid UTF-8 (round-trips through &str without panic).
        let _: &str = out.as_str();
        // First 510 'a's preserved, then '…' (no '€', no 'b's, no truncation
        // mid-codepoint).
        assert!(out.starts_with(&"a".repeat(510)));
        assert!(out.ends_with('…'));
        assert!(
            !out.contains('€'),
            "'€' must be dropped because it would straddle the 512-byte cut"
        );
        assert!(!out.contains('b'));
    }

    #[test]
    fn sanitize_error_body_handles_exactly_512_bytes_without_truncation() {
        let input = "a".repeat(512);
        let out = sanitize_error_body(&input);
        assert_eq!(out, input);
        assert!(!out.ends_with('…'));
    }

    #[test]
    fn sanitize_error_body_empty_input() {
        assert_eq!(sanitize_error_body(""), "");
    }

    #[test]
    fn sanitize_error_body_bidi_override_passes_through() {
        // Documents the actual boundary: U+202E is in the Unicode "Cf"
        // (format) category, not "Cc", so char::is_control() does not match.
        // The helper does NOT strip it. If we ever broaden the filter,
        // update this test.
        let input = "before\u{202e}after";
        let out = sanitize_error_body(input);
        assert!(out.contains('\u{202e}'));
    }

    /// Spawn a one-shot HTTP/1.1 mock that responds with the given status
    /// and raw body, then return the URL to send a request to.
    ///
    /// Uses a `tokio::net::TcpListener` rather than `mockito` to avoid a
    /// new dev-dependency. The listener is bound to `127.0.0.1:0` so it
    /// gets an ephemeral free port. The spawned task handles exactly one
    /// connection and exits.
    async fn spawn_mock_with_body(status_line: &'static str, body: &'static [u8]) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Drain the request (read until we see the end of headers).
            let mut buf = [0u8; 4096];
            loop {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let mut response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            response.extend_from_slice(body);
            sock.write_all(&response).await.unwrap();
            sock.shutdown().await.ok();
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn handle_error_status_sanitizes_ansi_in_auth_error() {
        // End-to-end: a mock 401 with an ANSI-laden body is fetched through
        // reqwest; the resulting Error::Auth message must not contain raw
        // ESC bytes.
        let url = spawn_mock_with_body("401 Unauthorized", b"\x1b[2J\x1b[Hattacker says hi").await;
        let resp = reqwest::Client::new().get(&url).send().await.unwrap();
        let status_code = resp.status().as_u16();
        let err = handle_error_status(status_code, resp).await.unwrap_err();
        match err {
            Error::Auth(msg) => {
                assert!(
                    !msg.contains('\x1b'),
                    "Error::Auth must not contain ESC bytes, got: {msg:?}"
                );
                assert!(
                    msg.contains("attacker says hi"),
                    "non-control text should still surface: {msg:?}"
                );
                assert!(msg.starts_with("401:"), "expected status prefix: {msg:?}");
            }
            other => panic!("expected Error::Auth, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_error_status_sanitizes_not_found_body() {
        let url = spawn_mock_with_body("404 Not Found", b"page not\x07 here\x00").await;
        let resp = reqwest::Client::new().get(&url).send().await.unwrap();
        let status_code = resp.status().as_u16();
        let err = handle_error_status(status_code, resp).await.unwrap_err();
        match err {
            Error::NotFound(msg) => {
                assert!(!msg.contains('\x07'));
                assert!(!msg.contains('\x00'));
                assert!(msg.contains("page not"));
            }
            other => panic!("expected Error::NotFound, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_error_status_sanitizes_generic_api_error() {
        let url =
            spawn_mock_with_body("500 Internal Server Error", b"oops\x1b[31mred\x1b[0m").await;
        let resp = reqwest::Client::new().get(&url).send().await.unwrap();
        let status_code = resp.status().as_u16();
        let err = handle_error_status(status_code, resp).await.unwrap_err();
        match err {
            Error::Api { status, message } => {
                assert_eq!(status, 500);
                assert!(!message.contains('\x1b'));
                assert!(message.contains("oops"));
            }
            other => panic!("expected Error::Api, got: {other:?}"),
        }
    }
}
