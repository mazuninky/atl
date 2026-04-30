//! `atl api` — generic REST passthrough against Confluence or Jira.
//!
//! The user supplies an endpoint and (optionally) a method, headers, query
//! parameters, and a JSON body (composed from `--field` / `--raw-field`
//! pairs or read verbatim from `--input`). The response is always printed
//! as JSON unless the user explicitly asked for a non-console format with
//! `-F`.

use std::io::Write;

use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8Path;
use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value};
use tracing::debug;

use crate::auth::{SecretStore, SystemKeyring};
use crate::cli::args::{ApiArgs, ApiService};
use crate::client::raw_request;
use crate::config::{AtlassianInstance, ConfigLoader};
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

/// Entry point for `atl api`.
pub async fn run(
    args: &ApiArgs,
    config_path: Option<&Utf8Path>,
    profile_name: Option<&str>,
    retries: u32,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> Result<()> {
    let config = ConfigLoader::load(config_path)?;
    let resolved_profile_name = profile_name
        .or(config.as_ref().map(|c| c.default_profile.as_str()))
        .unwrap_or("default");
    let profile = config
        .as_ref()
        .and_then(|c| c.resolve_profile(Some(resolved_profile_name)))
        .ok_or_else(|| anyhow!("no profile found; run `atl init` first"))?;

    let kind = match args.service {
        ApiService::Confluence => "confluence",
        ApiService::Jira => "jira",
    };
    let instance = match args.service {
        ApiService::Confluence => profile
            .confluence
            .as_ref()
            .ok_or_else(|| anyhow!("no Confluence instance configured in profile"))?,
        ApiService::Jira => profile
            .jira
            .as_ref()
            .ok_or_else(|| anyhow!("no Jira instance configured in profile"))?,
    };
    let store = SystemKeyring;

    let method = parse_method(&args.method)?;
    let headers = build_headers(&args.headers)?;
    let query = parse_queries(&args.queries)?;
    let body = build_body(&args.fields, &args.raw_fields, args.input.as_deref(), io)?;
    let endpoint = normalize_endpoint(&args.endpoint);

    // Writes to the profile must be blocked when the instance is marked
    // read-only, even when going through the raw passthrough. Mirror the
    // behaviour of the typed clients.
    if instance.read_only && !is_safe_method(&method) {
        return Err(anyhow::Error::from(crate::error::Error::Config(
            "profile is read-only; write operations are blocked".into(),
        )));
    }

    if args.preview {
        print_preview(io, instance, &method, &endpoint, &headers, &query, &body)?;
        return Ok(());
    }

    let value = if args.paginate {
        paginate(
            instance,
            resolved_profile_name,
            kind,
            &store,
            &method,
            &endpoint,
            &headers,
            &query,
            body.as_ref(),
            retries,
        )
        .await?
    } else {
        raw_request(
            instance,
            resolved_profile_name,
            kind,
            &store,
            method,
            &endpoint,
            headers,
            &query,
            body,
            retries,
        )
        .await?
    };

    // `atl api` defaults to JSON output regardless of the global -F setting,
    // because it is a raw passthrough; the only time we honour the user's
    // format is when they asked for something other than console.
    let effective = if matches!(format, OutputFormat::Console) {
        OutputFormat::Json
    } else {
        *format
    };
    write_output(value, &effective, io, transforms)?;

    Ok(())
}

// -------------------------------------------------------------------------
// Parsing helpers
// -------------------------------------------------------------------------

/// Detected pagination style for a response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaginationStyle {
    /// Jira v2 search: `{startAt, maxResults, total, issues: [...]}`.
    JiraSearch,
    /// Jira Agile: `{values: [...], startAt, isLast}`.
    JiraAgile,
    /// Confluence: `{results: [...], _links: {next: "..."}}`.
    ConfluenceLinksNext,
    /// Not a recognised paginated shape.
    None,
}

/// Inspects a JSON response and returns the pagination style it matches.
///
/// Used by `--paginate` to decide how to walk subsequent pages. Returns
/// [`PaginationStyle::None`] when the shape is not recognised.
pub(crate) fn detect_pagination_style(value: &Value) -> PaginationStyle {
    let Some(obj) = value.as_object() else {
        return PaginationStyle::None;
    };

    if obj.get("issues").is_some_and(Value::is_array)
        && obj.get("startAt").is_some_and(Value::is_number)
        && obj.get("total").is_some_and(Value::is_number)
    {
        return PaginationStyle::JiraSearch;
    }

    if obj.get("values").is_some_and(Value::is_array)
        && obj.get("isLast").is_some_and(Value::is_boolean)
    {
        return PaginationStyle::JiraAgile;
    }

    if obj.get("results").is_some_and(Value::is_array)
        && obj
            .get("_links")
            .and_then(Value::as_object)
            .is_some_and(|l| l.contains_key("next"))
    {
        return PaginationStyle::ConfluenceLinksNext;
    }

    PaginationStyle::None
}

/// Parses an HTTP method string into a [`reqwest::Method`], upper-casing it
/// for convenience so callers can write `--method get`.
pub(crate) fn parse_method(method: &str) -> Result<Method> {
    let upper = method.to_ascii_uppercase();
    Method::from_bytes(upper.as_bytes()).with_context(|| format!("invalid HTTP method: {method}"))
}

/// Returns true when `method` does not modify server state.
fn is_safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Ensures the endpoint starts with a leading `/`.
pub(crate) fn normalize_endpoint(endpoint: &str) -> String {
    if endpoint.starts_with('/') {
        endpoint.to_string()
    } else {
        format!("/{endpoint}")
    }
}

fn build_headers(raw: &[String]) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for entry in raw {
        let (key, value) = entry
            .split_once(':')
            .ok_or_else(|| anyhow!("header must be in KEY:VALUE form: {entry}"))?;
        let name = HeaderName::from_bytes(key.trim().as_bytes())
            .with_context(|| format!("invalid header name: {key}"))?;
        let value = HeaderValue::from_str(value.trim())
            .with_context(|| format!("invalid header value for {key}"))?;
        map.append(name, value);
    }
    Ok(map)
}

fn parse_queries(raw: &[String]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::with_capacity(raw.len());
    for entry in raw {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow!("query param must be in KEY=VALUE form: {entry}"))?;
        out.push((key.to_string(), value.to_string()));
    }
    Ok(out)
}

/// Builds the request body from `--field` / `--raw-field` / `--input`.
///
/// Returns `None` when no body source was provided. When `--input` is set,
/// the file / stdin contents are parsed as JSON so the resulting value fits
/// the same `Option<Value>` slot that reqwest's `json(&body)` expects. If the
/// input is not valid JSON we pass it through as a string — this matches the
/// behaviour of `gh api --input` which accepts arbitrary bodies.
fn build_body(
    fields: &[String],
    raw_fields: &[String],
    input: Option<&str>,
    io: &mut IoStreams,
) -> Result<Option<Value>> {
    if let Some(src) = input {
        let content = read_body_source(src, io)?;
        // Try JSON first; fall back to a string if that fails.
        return Ok(Some(
            serde_json::from_str::<Value>(&content).unwrap_or(Value::String(content)),
        ));
    }

    if fields.is_empty() && raw_fields.is_empty() {
        return Ok(None);
    }

    let mut object = Map::new();
    for entry in fields {
        let (key, value) = parse_field(entry, io)?;
        object.insert(key, Value::String(value));
    }
    for entry in raw_fields {
        let (key, value) = parse_raw_field(entry)?;
        object.insert(key, value);
    }
    Ok(Some(Value::Object(object)))
}

/// Parses a `-f key=value` pair into a `(key, value)` tuple.
///
/// `value` accepts three forms:
/// - `@path` — the contents of the file at `path`
/// - `-`     — the contents of stdin (read via `io`)
/// - otherwise — a literal string
pub(crate) fn parse_field(entry: &str, io: &mut IoStreams) -> Result<(String, String)> {
    let (key, value) = entry
        .split_once('=')
        .ok_or_else(|| anyhow!("field must be in KEY=VALUE form: {entry}"))?;
    let resolved = read_body_source(value, io)?;
    Ok((key.to_string(), resolved))
}

/// Parses a `--raw-field key=<json>` pair into a `(key, Value)` tuple.
///
/// The value must be valid JSON; an error is returned otherwise.
pub(crate) fn parse_raw_field(entry: &str) -> Result<(String, Value)> {
    let (key, value) = entry
        .split_once('=')
        .ok_or_else(|| anyhow!("raw-field must be in KEY=VALUE form: {entry}"))?;
    let parsed: Value = serde_json::from_str(value)
        .with_context(|| format!("raw-field value for `{key}` is not valid JSON: {value}"))?;
    Ok((key.to_string(), parsed))
}

/// Resolves a body source spec: literal, `@file`, or `-` for stdin.
fn read_body_source(spec: &str, io: &mut IoStreams) -> Result<String> {
    if spec == "-" {
        let mut buf = String::new();
        io.stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else if let Some(path) = spec.strip_prefix('@') {
        if path.is_empty() {
            bail!("file path after '@' cannot be empty");
        }
        std::fs::read_to_string(path).with_context(|| format!("failed to read {path}"))
    } else {
        Ok(spec.to_string())
    }
}

// -------------------------------------------------------------------------
// Pagination
// -------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn paginate(
    instance: &AtlassianInstance,
    profile: &str,
    kind: &str,
    store: &dyn SecretStore,
    method: &Method,
    endpoint: &str,
    headers: &HeaderMap,
    query: &[(String, String)],
    body: Option<&Value>,
    retries: u32,
) -> Result<Value> {
    // First page — shape drives the rest of the walk.
    let first = raw_request(
        instance,
        profile,
        kind,
        store,
        method.clone(),
        endpoint,
        headers.clone(),
        query,
        body.cloned(),
        retries,
    )
    .await?;

    match detect_pagination_style(&first) {
        PaginationStyle::JiraSearch => {
            paginate_jira_search(
                instance, profile, kind, store, method, endpoint, headers, query, body, first,
                retries,
            )
            .await
        }
        PaginationStyle::JiraAgile => {
            paginate_jira_agile(
                instance, profile, kind, store, method, endpoint, headers, query, body, first,
                retries,
            )
            .await
        }
        PaginationStyle::ConfluenceLinksNext => {
            paginate_confluence_links(
                instance, profile, kind, store, method, headers, body, first, retries,
            )
            .await
        }
        PaginationStyle::None => Err(anyhow!(
            "pagination style not detected for this endpoint; use manual pagination"
        )),
    }
}

#[allow(clippy::too_many_arguments)]
async fn paginate_jira_search(
    instance: &AtlassianInstance,
    profile: &str,
    kind: &str,
    store: &dyn SecretStore,
    method: &Method,
    endpoint: &str,
    headers: &HeaderMap,
    query: &[(String, String)],
    body: Option<&Value>,
    first: Value,
    retries: u32,
) -> Result<Value> {
    let mut merged = first;
    let mut accumulated: Vec<Value> = merged
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    loop {
        let total = merged
            .get("total")
            .and_then(Value::as_u64)
            .unwrap_or(accumulated.len() as u64);
        if accumulated.len() as u64 >= total {
            break;
        }

        let mut next_query: Vec<(String, String)> = query
            .iter()
            .filter(|(k, _)| k != "startAt")
            .cloned()
            .collect();
        next_query.push(("startAt".into(), accumulated.len().to_string()));

        let page = raw_request(
            instance,
            profile,
            kind,
            store,
            method.clone(),
            endpoint,
            headers.clone(),
            &next_query,
            body.cloned(),
            retries,
        )
        .await?;

        let issues = page
            .get("issues")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if issues.is_empty() {
            break;
        }
        accumulated.extend(issues);
    }

    if let Some(obj) = merged.as_object_mut() {
        obj.insert("issues".into(), Value::Array(accumulated));
    }
    Ok(merged)
}

#[allow(clippy::too_many_arguments)]
async fn paginate_jira_agile(
    instance: &AtlassianInstance,
    profile: &str,
    kind: &str,
    store: &dyn SecretStore,
    method: &Method,
    endpoint: &str,
    headers: &HeaderMap,
    query: &[(String, String)],
    body: Option<&Value>,
    first: Value,
    retries: u32,
) -> Result<Value> {
    let mut merged = first;
    let mut accumulated: Vec<Value> = merged
        .get("values")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut is_last = merged
        .get("isLast")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    while !is_last {
        let mut next_query: Vec<(String, String)> = query
            .iter()
            .filter(|(k, _)| k != "startAt")
            .cloned()
            .collect();
        next_query.push(("startAt".into(), accumulated.len().to_string()));

        let page = raw_request(
            instance,
            profile,
            kind,
            store,
            method.clone(),
            endpoint,
            headers.clone(),
            &next_query,
            body.cloned(),
            retries,
        )
        .await?;

        let values = page
            .get("values")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        is_last = page.get("isLast").and_then(Value::as_bool).unwrap_or(true);
        if values.is_empty() {
            break;
        }
        accumulated.extend(values);
    }

    if let Some(obj) = merged.as_object_mut() {
        obj.insert("values".into(), Value::Array(accumulated));
        obj.insert("isLast".into(), Value::Bool(true));
    }
    Ok(merged)
}

#[allow(clippy::too_many_arguments)]
async fn paginate_confluence_links(
    instance: &AtlassianInstance,
    profile: &str,
    kind: &str,
    store: &dyn SecretStore,
    method: &Method,
    headers: &HeaderMap,
    body: Option<&Value>,
    first: Value,
    retries: u32,
) -> Result<Value> {
    let mut merged = first;
    let mut accumulated: Vec<Value> = merged
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    while let Some(next) = merged
        .get("_links")
        .and_then(Value::as_object)
        .and_then(|l| l.get("next"))
        .and_then(Value::as_str)
    {
        let (next_endpoint, next_query) = split_next_url(next, &instance.domain);
        debug!(
            "Confluence _links.next → endpoint={next_endpoint} query_pairs={}",
            next_query.len()
        );

        let page = raw_request(
            instance,
            profile,
            kind,
            store,
            method.clone(),
            &next_endpoint,
            headers.clone(),
            &next_query,
            body.cloned(),
            retries,
        )
        .await?;

        let results = page
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if results.is_empty() {
            // Replace merged so the next-link iteration moves forward even
            // when the last page carries no results.
            merged = page;
            break;
        }
        accumulated.extend(results);
        merged = page;
    }

    if let Some(obj) = merged.as_object_mut() {
        obj.insert("results".into(), Value::Array(accumulated));
        // Strip the trailing `_links.next` so downstream consumers don't
        // mistake the merged blob for a mid-walk page.
        if let Some(links) = obj.get_mut("_links").and_then(Value::as_object_mut) {
            links.remove("next");
        }
    }
    Ok(merged)
}

/// Splits a Confluence `_links.next` URL into an `(endpoint, query)` pair
/// suitable for another `raw_request` call. The next URL may be absolute or
/// relative; the `domain` is used only to strip the host portion of an
/// absolute URL.
fn split_next_url(next: &str, domain: &str) -> (String, Vec<(String, String)>) {
    // Strip an absolute prefix if present so we retain just the path+query.
    let stripped = if next.starts_with("http://") || next.starts_with("https://") {
        // Strip scheme://host so what remains is /path?query.
        next.split_once("://")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once('/').map(|(_, tail)| tail))
            .map(|path| format!("/{path}"))
            .unwrap_or_else(|| next.to_string())
    } else {
        // For something like `example.atlassian.net/wiki/...` or a true path
        // start like `/wiki/...`, attempt to strip the domain prefix as a
        // convenience even when the scheme is absent.
        let d = domain.trim_end_matches('/');
        let d_bare = d
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        if let Some(rest) = next.strip_prefix(d_bare) {
            rest.to_string()
        } else {
            next.to_string()
        }
    };

    let (path, query_part) = match stripped.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (stripped, String::new()),
    };
    let endpoint = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    let query: Vec<(String, String)> = if query_part.is_empty() {
        Vec::new()
    } else {
        query_part
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| match pair.split_once('=') {
                Some((k, v)) => (
                    urldecode(k).unwrap_or_else(|| k.to_string()),
                    urldecode(v).unwrap_or_else(|| v.to_string()),
                ),
                None => (pair.to_string(), String::new()),
            })
            .collect()
    };
    (endpoint, query)
}

/// Tiny percent-decoder so `split_next_url` doesn't need a dependency on
/// `url` / `percent-encoding`. Only handles UTF-8 and the common case; returns
/// `None` if decoding runs off the end of a `%` escape.
fn urldecode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = hex_nibble(bytes[i + 1])?;
                let lo = hex_nibble(bytes[i + 2])?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// -------------------------------------------------------------------------
// Preview rendering
// -------------------------------------------------------------------------

fn print_preview(
    io: &mut IoStreams,
    instance: &AtlassianInstance,
    method: &Method,
    endpoint: &str,
    headers: &HeaderMap,
    query: &[(String, String)],
    body: &Option<Value>,
) -> Result<()> {
    let domain = instance.domain.trim_end_matches('/');
    let scheme = if domain.starts_with("http://") || domain.starts_with("https://") {
        ""
    } else {
        "https://"
    };
    let url = format!("{scheme}{domain}{endpoint}");

    let mut err = io.stderr();
    writeln!(err, "HTTP Request Preview")?;
    writeln!(err, "  Method:   {method}")?;
    writeln!(err, "  URL:      {url}")?;

    // Always show Authorization redacted + Content-Type so the preview is a
    // faithful approximation of what reqwest will ultimately send.
    writeln!(err, "  Headers:")?;
    writeln!(err, "    Accept: application/json")?;
    let auth_kind = match instance.auth_type {
        crate::config::AuthType::Basic => "Basic",
        crate::config::AuthType::Bearer => "Bearer",
    };
    writeln!(err, "    Authorization: {auth_kind} <redacted>")?;
    for (name, value) in headers {
        let name_str = name.as_str();
        if name_str.eq_ignore_ascii_case("authorization") {
            let raw = value.to_str().unwrap_or("");
            let kind = raw.split_whitespace().next().unwrap_or("");
            writeln!(err, "    {name_str}: {kind} <redacted>")?;
        } else {
            let raw = value.to_str().unwrap_or("<binary>");
            writeln!(err, "    {name_str}: {raw}")?;
        }
    }

    if query.is_empty() {
        writeln!(err, "  Query:    <none>")?;
    } else {
        writeln!(err, "  Query:")?;
        for (k, v) in query {
            writeln!(err, "    {k}={v}")?;
        }
    }

    match body {
        None => writeln!(err, "  Body:     <none>")?,
        Some(v) => {
            let pretty =
                serde_json::to_string_pretty(v).unwrap_or_else(|_| "<unserializable>".to_string());
            writeln!(err, "  Body:")?;
            for line in pretty.lines() {
                writeln!(err, "    {line}")?;
            }
        }
    }

    err.flush()?;
    Ok(())
}

// -------------------------------------------------------------------------
// Unit tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_endpoint_adds_leading_slash() {
        assert_eq!(
            normalize_endpoint("rest/api/2/myself"),
            "/rest/api/2/myself"
        );
    }

    #[test]
    fn normalize_endpoint_preserves_leading_slash() {
        assert_eq!(
            normalize_endpoint("/wiki/rest/api/content/1"),
            "/wiki/rest/api/content/1"
        );
    }

    #[test]
    fn detect_pagination_jira_search_shape() {
        let v = json!({
            "startAt": 0,
            "maxResults": 50,
            "total": 100,
            "issues": [],
        });
        assert_eq!(detect_pagination_style(&v), PaginationStyle::JiraSearch);
    }

    #[test]
    fn detect_pagination_jira_agile_shape() {
        let v = json!({
            "values": [{"id": 1}],
            "startAt": 0,
            "isLast": false,
        });
        assert_eq!(detect_pagination_style(&v), PaginationStyle::JiraAgile);
    }

    #[test]
    fn detect_pagination_confluence_links_next() {
        let v = json!({
            "results": [{"id": 1}],
            "_links": { "next": "/wiki/api/v2/pages?cursor=abc" },
        });
        assert_eq!(
            detect_pagination_style(&v),
            PaginationStyle::ConfluenceLinksNext
        );
    }

    #[test]
    fn detect_pagination_none_on_scalar() {
        assert_eq!(detect_pagination_style(&json!(42)), PaginationStyle::None);
    }

    #[test]
    fn detect_pagination_none_on_bare_array() {
        assert_eq!(detect_pagination_style(&json!([])), PaginationStyle::None);
    }

    #[test]
    fn detect_pagination_none_on_unrelated_object() {
        let v = json!({"emailAddress": "me@example.com"});
        assert_eq!(detect_pagination_style(&v), PaginationStyle::None);
    }

    #[test]
    fn parse_field_literal_string() {
        let mut io = IoStreams::test();
        let (k, v) = parse_field("name=widget", &mut io).unwrap();
        assert_eq!(k, "name");
        assert_eq!(v, "widget");
    }

    #[test]
    fn parse_field_from_file() {
        let mut io = IoStreams::test();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.txt");
        std::fs::write(&path, "contents-from-file").unwrap();
        let spec = format!("body=@{}", path.display());
        let (k, v) = parse_field(&spec, &mut io).unwrap();
        assert_eq!(k, "body");
        assert_eq!(v, "contents-from-file");
    }

    #[test]
    fn parse_field_missing_separator_errors() {
        let mut io = IoStreams::test();
        let err = parse_field("no-equals-sign", &mut io).unwrap_err();
        assert!(err.to_string().contains("KEY=VALUE"), "got: {err}");
    }

    #[test]
    fn parse_raw_field_number() {
        let (k, v) = parse_raw_field("count=42").unwrap();
        assert_eq!(k, "count");
        assert_eq!(v, json!(42));
    }

    #[test]
    fn parse_raw_field_array() {
        let (k, v) = parse_raw_field("tags=[1,2,3]").unwrap();
        assert_eq!(k, "tags");
        assert_eq!(v, json!([1, 2, 3]));
    }

    #[test]
    fn parse_raw_field_nested_object() {
        let (k, v) = parse_raw_field(r#"fields={"project":{"key":"TEST"}}"#).unwrap();
        assert_eq!(k, "fields");
        assert_eq!(v, json!({"project": {"key": "TEST"}}));
    }

    #[test]
    fn parse_raw_field_invalid_json_errors() {
        let err = parse_raw_field("count=not-json").unwrap_err();
        assert!(err.to_string().contains("not valid JSON"), "got: {err}");
    }

    #[test]
    fn parse_method_case_insensitive() {
        assert_eq!(parse_method("get").unwrap(), Method::GET);
        assert_eq!(parse_method("Post").unwrap(), Method::POST);
        assert_eq!(parse_method("DELETE").unwrap(), Method::DELETE);
    }

    #[test]
    fn parse_method_rejects_invalid() {
        assert!(parse_method("not a method").is_err());
    }

    #[test]
    fn build_headers_parses_colon_form() {
        let h = build_headers(&["X-Test:hello".to_string(), "X-Foo: bar".to_string()]).unwrap();
        assert_eq!(h.get("X-Test").unwrap(), "hello");
        assert_eq!(h.get("X-Foo").unwrap(), "bar");
    }

    #[test]
    fn build_headers_rejects_missing_colon() {
        assert!(build_headers(&["no-colon".to_string()]).is_err());
    }

    #[test]
    fn parse_queries_ok() {
        let q =
            parse_queries(&["jql=project=TEST".to_string(), "fields=*all".to_string()]).unwrap();
        // The first `=` is the separator; the remainder is the literal value.
        assert_eq!(q[0], ("jql".to_string(), "project=TEST".to_string()));
        assert_eq!(q[1], ("fields".to_string(), "*all".to_string()));
    }

    #[test]
    fn parse_queries_rejects_missing_equals() {
        assert!(parse_queries(&["no-equals".to_string()]).is_err());
    }

    #[test]
    fn build_body_none_without_fields() {
        let mut io = IoStreams::test();
        assert!(build_body(&[], &[], None, &mut io).unwrap().is_none());
    }

    #[test]
    fn build_body_merges_fields_and_raw_fields_last_wins() {
        let mut io = IoStreams::test();
        let fields = vec!["a=one".to_string(), "b=two".to_string()];
        let raw = vec!["b=99".to_string(), "c=[1,2]".to_string()];
        let body = build_body(&fields, &raw, None, &mut io).unwrap().unwrap();
        assert_eq!(body["a"], json!("one"));
        // Raw field for `b` overrides the string field of the same name.
        assert_eq!(body["b"], json!(99));
        assert_eq!(body["c"], json!([1, 2]));
    }

    #[test]
    fn split_next_url_absolute() {
        let (endpoint, query) = split_next_url(
            "https://example.atlassian.net/wiki/api/v2/pages?cursor=abc&limit=25",
            "example.atlassian.net",
        );
        assert_eq!(endpoint, "/wiki/api/v2/pages");
        assert_eq!(
            query,
            vec![
                ("cursor".to_string(), "abc".to_string()),
                ("limit".to_string(), "25".to_string()),
            ]
        );
    }

    #[test]
    fn split_next_url_relative_path() {
        let (endpoint, query) = split_next_url(
            "/wiki/rest/api/content/search?cql=type%3Dpage&start=25",
            "example.atlassian.net",
        );
        assert_eq!(endpoint, "/wiki/rest/api/content/search");
        assert_eq!(
            query,
            vec![
                ("cql".to_string(), "type=page".to_string()),
                ("start".to_string(), "25".to_string()),
            ]
        );
    }

    #[test]
    fn split_next_url_no_query() {
        let (endpoint, query) = split_next_url("/rest/api/3/search/jql", "example.atlassian.net");
        assert_eq!(endpoint, "/rest/api/3/search/jql");
        assert!(query.is_empty());
    }

    #[test]
    fn is_safe_method_classification() {
        assert!(is_safe_method(&Method::GET));
        assert!(is_safe_method(&Method::HEAD));
        assert!(is_safe_method(&Method::OPTIONS));
        assert!(!is_safe_method(&Method::POST));
        assert!(!is_safe_method(&Method::PUT));
        assert!(!is_safe_method(&Method::DELETE));
    }

    // -------------------------------------------------------------------
    // read_body_source
    // -------------------------------------------------------------------

    #[test]
    fn read_body_source_literal_string() {
        let mut io = IoStreams::test();
        let got = read_body_source("just-a-literal", &mut io).unwrap();
        assert_eq!(got, "just-a-literal");
    }

    #[test]
    fn read_body_source_at_file() {
        let mut io = IoStreams::test();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.txt");
        std::fs::write(&path, "from-disk").unwrap();
        let spec = format!("@{}", path.display());
        let got = read_body_source(&spec, &mut io).unwrap();
        assert_eq!(got, "from-disk");
    }

    #[test]
    fn read_body_source_at_empty_path_errors() {
        let mut io = IoStreams::test();
        let err = read_body_source("@", &mut io).unwrap_err();
        assert!(err.to_string().contains("cannot be empty"), "got: {err}");
    }

    #[test]
    fn read_body_source_at_missing_file_errors() {
        let mut io = IoStreams::test();
        let err = read_body_source("@/this/path/does/not/exist/atl-test", &mut io).unwrap_err();
        assert!(
            err.to_string().contains("failed to read"),
            "expected 'failed to read' in error, got: {err}"
        );
    }

    // -------------------------------------------------------------------
    // build_body input resolution
    // -------------------------------------------------------------------

    #[test]
    fn build_body_input_parses_valid_json() {
        let mut io = IoStreams::test();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.json");
        std::fs::write(&path, r#"{"key":"value","n":7}"#).unwrap();
        let spec = format!("@{}", path.display());
        let body = build_body(&[], &[], Some(&spec), &mut io).unwrap().unwrap();
        assert_eq!(body, json!({"key": "value", "n": 7}));
    }

    #[test]
    fn build_body_input_falls_back_to_string_for_non_json() {
        let mut io = IoStreams::test();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.txt");
        std::fs::write(&path, "this is not json").unwrap();
        let spec = format!("@{}", path.display());
        let body = build_body(&[], &[], Some(&spec), &mut io).unwrap().unwrap();
        assert_eq!(body, json!("this is not json"));
    }

    #[test]
    fn build_body_input_literal_json_string() {
        let mut io = IoStreams::test();
        // A literal value goes through `read_body_source` -> string fallback
        // since `[1,2,3]` is valid JSON, it's parsed as such.
        let body = build_body(&[], &[], Some("[1,2,3]"), &mut io)
            .unwrap()
            .unwrap();
        assert_eq!(body, json!([1, 2, 3]));
    }

    #[test]
    fn build_body_only_raw_fields() {
        let mut io = IoStreams::test();
        let raw = vec![r#"obj={"a":1}"#.to_string()];
        let body = build_body(&[], &raw, None, &mut io).unwrap().unwrap();
        assert_eq!(body, json!({"obj": {"a": 1}}));
    }

    #[test]
    fn build_body_only_string_fields() {
        let mut io = IoStreams::test();
        let fields = vec!["name=widget".to_string(), "color=red".to_string()];
        let body = build_body(&fields, &[], None, &mut io).unwrap().unwrap();
        assert_eq!(body, json!({"name": "widget", "color": "red"}));
    }

    #[test]
    fn build_body_propagates_field_parse_error() {
        let mut io = IoStreams::test();
        let fields = vec!["bad-no-equals".to_string()];
        let err = build_body(&fields, &[], None, &mut io).unwrap_err();
        assert!(err.to_string().contains("KEY=VALUE"), "got: {err}");
    }

    #[test]
    fn build_body_propagates_raw_field_parse_error() {
        let mut io = IoStreams::test();
        let raw = vec!["bad-no-equals".to_string()];
        let err = build_body(&[], &raw, None, &mut io).unwrap_err();
        assert!(err.to_string().contains("KEY=VALUE"), "got: {err}");
    }

    // -------------------------------------------------------------------
    // build_headers — invalid input
    // -------------------------------------------------------------------

    #[test]
    fn build_headers_appends_repeated_header_name() {
        // HeaderMap permits multiple values for the same name; verify both
        // are preserved (order is not asserted because get() returns first).
        let h = build_headers(&["X-Multi: one".to_string(), "X-Multi: two".to_string()]).unwrap();
        let values: Vec<_> = h.get_all("X-Multi").iter().collect();
        assert_eq!(values.len(), 2, "expected two appended values");
    }

    #[test]
    fn build_headers_rejects_invalid_name() {
        // Spaces in the header name are illegal per RFC 7230.
        let err = build_headers(&["bad name: value".to_string()]).unwrap_err();
        assert!(
            err.to_string().contains("invalid header name"),
            "got: {err}"
        );
    }

    #[test]
    fn build_headers_rejects_invalid_value() {
        // CR/LF in a header value are forbidden.
        let err = build_headers(&["X-OK: value\nwith-newline".to_string()]).unwrap_err();
        assert!(
            err.to_string().contains("invalid header value"),
            "got: {err}"
        );
    }

    #[test]
    fn build_headers_empty_value_is_allowed() {
        // An empty value is legal and trimmed away.
        let h = build_headers(&["X-Empty:".to_string()]).unwrap();
        assert_eq!(h.get("X-Empty").unwrap(), "");
    }

    // -------------------------------------------------------------------
    // parse_queries
    // -------------------------------------------------------------------

    #[test]
    fn parse_queries_empty_input_is_empty_output() {
        let q = parse_queries(&[]).unwrap();
        assert!(q.is_empty());
    }

    #[test]
    fn parse_queries_allows_empty_value() {
        let q = parse_queries(&["empty=".to_string()]).unwrap();
        assert_eq!(q, vec![("empty".to_string(), String::new())]);
    }

    #[test]
    fn parse_queries_allows_empty_key() {
        // The split is on the first '=', so an empty key technically parses.
        // We document the current behaviour here so any future change is
        // intentional.
        let q = parse_queries(&["=value".to_string()]).unwrap();
        assert_eq!(q, vec![(String::new(), "value".to_string())]);
    }

    // -------------------------------------------------------------------
    // parse_method
    // -------------------------------------------------------------------

    #[test]
    fn parse_method_uppercases_mixed_case() {
        assert_eq!(parse_method("PaTcH").unwrap(), Method::PATCH);
    }

    #[test]
    fn parse_method_rejects_empty() {
        assert!(parse_method("").is_err());
    }

    // -------------------------------------------------------------------
    // split_next_url — additional edge cases
    // -------------------------------------------------------------------

    #[test]
    fn split_next_url_strips_bare_domain_prefix() {
        // The relative path branch strips the bare domain prefix when given
        // a hostnameless reference like `example.atlassian.net/wiki/...`.
        let (endpoint, query) = split_next_url(
            "example.atlassian.net/wiki/api/v2/pages?cursor=abc",
            "example.atlassian.net",
        );
        assert_eq!(endpoint, "/wiki/api/v2/pages");
        assert_eq!(query, vec![("cursor".to_string(), "abc".to_string())]);
    }

    #[test]
    fn split_next_url_handles_https_prefixed_domain_arg() {
        // The domain may itself be passed with a scheme — the bare-domain
        // strip still matches.
        let (endpoint, _query) = split_next_url(
            "example.atlassian.net/wiki/api/v2/pages",
            "https://example.atlassian.net",
        );
        assert_eq!(endpoint, "/wiki/api/v2/pages");
    }

    #[test]
    fn split_next_url_skips_empty_pairs() {
        // A trailing `&` produces an empty pair which must be filtered out.
        let (_endpoint, query) = split_next_url("/wiki/api?cursor=abc&", "example.atlassian.net");
        assert_eq!(query, vec![("cursor".to_string(), "abc".to_string())]);
    }

    #[test]
    fn split_next_url_pair_without_equals() {
        // A bare key without `=` produces (key, empty).
        let (_endpoint, query) =
            split_next_url("/wiki/api?flag&cursor=abc", "example.atlassian.net");
        assert_eq!(
            query,
            vec![
                ("flag".to_string(), String::new()),
                ("cursor".to_string(), "abc".to_string()),
            ]
        );
    }

    #[test]
    fn split_next_url_path_without_leading_slash_gets_one() {
        // When the bare-domain strip removes the leading `/`, we still
        // return a path that begins with `/`.
        let (endpoint, _query) =
            split_next_url("example.atlassian.net?cursor=abc", "example.atlassian.net");
        // Stripping the domain leaves `?cursor=abc`; the `?` triggers the
        // split, leaving the path empty — the leading-slash branch then
        // promotes it to `/`.
        assert_eq!(endpoint, "/");
    }

    // -------------------------------------------------------------------
    // urldecode / hex_nibble
    // -------------------------------------------------------------------

    #[test]
    fn urldecode_passes_through_plain_ascii() {
        assert_eq!(urldecode("hello").as_deref(), Some("hello"));
    }

    #[test]
    fn urldecode_handles_plus_as_space() {
        assert_eq!(urldecode("a+b+c").as_deref(), Some("a b c"));
    }

    #[test]
    fn urldecode_handles_percent_escapes() {
        assert_eq!(urldecode("%2F%3D%26").as_deref(), Some("/=&"));
    }

    #[test]
    fn urldecode_lowercase_hex() {
        assert_eq!(urldecode("%2f").as_deref(), Some("/"));
    }

    #[test]
    fn urldecode_returns_none_for_truncated_escape() {
        // Only one hex digit after `%` — must fail rather than silently
        // dropping bytes.
        assert!(urldecode("%2").is_none());
        assert!(urldecode("%").is_none());
    }

    #[test]
    fn urldecode_returns_none_for_invalid_hex() {
        assert!(urldecode("%ZZ").is_none());
    }

    #[test]
    fn urldecode_returns_none_for_invalid_utf8() {
        // %FF on its own is not valid UTF-8.
        assert!(urldecode("%FF").is_none());
    }

    #[test]
    fn hex_nibble_recognises_all_classes() {
        assert_eq!(hex_nibble(b'0'), Some(0));
        assert_eq!(hex_nibble(b'9'), Some(9));
        assert_eq!(hex_nibble(b'a'), Some(10));
        assert_eq!(hex_nibble(b'f'), Some(15));
        assert_eq!(hex_nibble(b'A'), Some(10));
        assert_eq!(hex_nibble(b'F'), Some(15));
        assert!(hex_nibble(b'g').is_none());
        assert!(hex_nibble(b'/').is_none());
    }

    // -------------------------------------------------------------------
    // print_preview
    // -------------------------------------------------------------------

    fn mk_basic_instance(domain: &str) -> AtlassianInstance {
        AtlassianInstance {
            domain: domain.to_string(),
            email: Some("alice@example.com".into()),
            api_token: None,
            auth_type: crate::config::AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: None,
        }
    }

    #[test]
    fn print_preview_renders_method_and_url_to_stderr() {
        let mut io = IoStreams::test();
        let instance = mk_basic_instance("example.atlassian.net");
        let headers = HeaderMap::new();
        print_preview(
            &mut io,
            &instance,
            &Method::GET,
            "/rest/api/2/myself",
            &headers,
            &[],
            &None,
        )
        .unwrap();
        let err = io.stderr_as_string();
        assert!(err.contains("HTTP Request Preview"), "stderr={err}");
        assert!(err.contains("Method:   GET"), "stderr={err}");
        assert!(
            err.contains("URL:      https://example.atlassian.net/rest/api/2/myself"),
            "stderr={err}"
        );
        assert!(err.contains("Body:     <none>"), "stderr={err}");
        assert!(err.contains("Query:    <none>"), "stderr={err}");
    }

    #[test]
    fn print_preview_keeps_existing_scheme() {
        let mut io = IoStreams::test();
        let instance = mk_basic_instance("https://example.atlassian.net");
        let headers = HeaderMap::new();
        print_preview(
            &mut io,
            &instance,
            &Method::POST,
            "/x",
            &headers,
            &[],
            &None,
        )
        .unwrap();
        let err = io.stderr_as_string();
        assert!(
            err.contains("URL:      https://example.atlassian.net/x"),
            "stderr={err}"
        );
        // Negative: must not double up the scheme.
        assert!(!err.contains("https://https://"), "stderr={err}");
    }

    #[test]
    fn print_preview_renders_query_and_body() {
        let mut io = IoStreams::test();
        let instance = mk_basic_instance("example.atlassian.net");
        let mut headers = HeaderMap::new();
        headers.append(
            HeaderName::from_static("x-custom"),
            HeaderValue::from_static("hello"),
        );
        let query = vec![
            ("jql".to_string(), "project=TEST".to_string()),
            ("limit".to_string(), "25".to_string()),
        ];
        let body = Some(json!({"summary": "demo"}));
        print_preview(
            &mut io,
            &instance,
            &Method::PUT,
            "/rest/api/2/issue",
            &headers,
            &query,
            &body,
        )
        .unwrap();
        let err = io.stderr_as_string();
        assert!(err.contains("    x-custom: hello"), "stderr={err}");
        assert!(err.contains("jql=project=TEST"), "stderr={err}");
        assert!(err.contains("limit=25"), "stderr={err}");
        // Body is pretty-printed, indented two spaces past the "  Body:" label.
        assert!(err.contains(r#""summary""#), "stderr={err}");
        assert!(err.contains(r#""demo""#), "stderr={err}");
    }

    #[test]
    fn print_preview_redacts_authorization_header() {
        let mut io = IoStreams::test();
        let instance = mk_basic_instance("example.atlassian.net");
        let mut headers = HeaderMap::new();
        // A user-supplied Authorization header — must be redacted while
        // keeping the scheme prefix visible.
        headers.append(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer abcd1234secret"),
        );
        print_preview(&mut io, &instance, &Method::GET, "/x", &headers, &[], &None).unwrap();
        let err = io.stderr_as_string();
        assert!(
            err.contains("authorization: Bearer <redacted>"),
            "expected redacted Authorization header, got: {err}"
        );
        assert!(
            !err.contains("abcd1234secret"),
            "secret token must not appear in preview, got: {err}"
        );
    }

    #[test]
    fn print_preview_includes_default_basic_authorization_label() {
        let mut io = IoStreams::test();
        let instance = mk_basic_instance("example.atlassian.net");
        let headers = HeaderMap::new();
        print_preview(&mut io, &instance, &Method::GET, "/x", &headers, &[], &None).unwrap();
        let err = io.stderr_as_string();
        assert!(
            err.contains("Authorization: Basic <redacted>"),
            "stderr={err}"
        );
    }

    #[test]
    fn print_preview_includes_bearer_label_for_bearer_auth() {
        let mut io = IoStreams::test();
        let mut instance = mk_basic_instance("example.atlassian.net");
        instance.auth_type = crate::config::AuthType::Bearer;
        let headers = HeaderMap::new();
        print_preview(&mut io, &instance, &Method::GET, "/x", &headers, &[], &None).unwrap();
        let err = io.stderr_as_string();
        assert!(
            err.contains("Authorization: Bearer <redacted>"),
            "stderr={err}"
        );
    }

    #[test]
    fn print_preview_strips_trailing_slash_from_domain() {
        let mut io = IoStreams::test();
        let instance = mk_basic_instance("example.atlassian.net/");
        let headers = HeaderMap::new();
        print_preview(&mut io, &instance, &Method::GET, "/x", &headers, &[], &None).unwrap();
        let err = io.stderr_as_string();
        assert!(
            err.contains("URL:      https://example.atlassian.net/x"),
            "stderr={err}"
        );
        assert!(!err.contains("net//x"), "stderr={err}");
    }
}
