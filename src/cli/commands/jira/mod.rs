mod admin;
mod board;
mod field;
mod filter;
mod project;
mod sprint;
mod user;
mod workflow;

use anyhow::Context;
use camino::Utf8Path;
use serde_json::{Value, json};

use crate::auth::SystemKeyring;
use crate::cli::args::*;
use crate::cli::commands::converters::adf_to_md::{ConvertOpts as AdfConvertOpts, adf_to_markdown};
use crate::cli::commands::converters::body_content::JiraBodyContent;
use crate::cli::commands::converters::wiki_to_md::{
    ConvertOpts as WikiConvertOpts, wiki_to_markdown,
};
use crate::client::{JiraApiVersion, JiraClient, RetryConfig};
use crate::config::{ConfigLoader, JiraFlavor};
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

use super::read_body_arg;

/// Normalise a user-supplied body into the [`JiraBodyContent`] enum the
/// Jira client understands.
///
/// - [`JiraInputFormat::Wiki`] is a passthrough — the body reaches the v2 API
///   byte-for-byte as the `description` / comment `body` field.
/// - [`JiraInputFormat::Markdown`] runs the body through
///   [`super::converters::md_to_wiki::markdown_to_wiki`] and wraps the result
///   in [`JiraBodyContent::Wiki`].
/// - [`JiraInputFormat::Adf`] parses the body as JSON and wraps it in
///   [`JiraBodyContent::Adf`]. Invalid JSON surfaces as an error. Cloud-flavor
///   validation lives in [`assert_adf_supported`] at the call site so the
///   message can mention the specific operation that failed.
fn convert_input(body: String, fmt: &JiraInputFormat) -> anyhow::Result<JiraBodyContent> {
    Ok(match fmt {
        JiraInputFormat::Wiki => JiraBodyContent::Wiki(body),
        JiraInputFormat::Markdown => {
            let wiki = super::converters::md_to_wiki::markdown_to_wiki(&body)
                .context("failed to convert markdown to Jira wiki")?;
            JiraBodyContent::Wiki(wiki)
        }
        JiraInputFormat::Adf => {
            let parsed: Value =
                serde_json::from_str(&body).with_context(|| "ADF input is not valid JSON")?;
            JiraBodyContent::Adf(parsed)
        }
    })
}

/// Map a [`JiraBodyContent`] variant to the API version the Jira client must
/// call. ADF bodies require the v3 API; wiki bodies use v2.
fn api_version_for(body: &JiraBodyContent) -> JiraApiVersion {
    match body {
        JiraBodyContent::Wiki(_) => JiraApiVersion::V2,
        JiraBodyContent::Adf(_) => JiraApiVersion::V3,
    }
}

/// Render a [`JiraBodyContent`] as the JSON value that goes into the
/// `description` (or any other body-shaped) field. v2 expects a string; v3
/// expects the ADF object directly.
fn body_field_value(body: JiraBodyContent) -> Value {
    match body {
        JiraBodyContent::Wiki(text) => Value::String(text),
        JiraBodyContent::Adf(adf) => adf,
    }
}

/// Reject ADF input on Data Center / Server with a typed `Error::Config` so
/// the exit-code mapping returns 3 instead of the generic 1.
///
/// Cloud-only because the Jira v3 REST API — which carries the ADF body —
/// is not available on self-hosted instances. Callers pass the deployment
/// flavor (from [`JiraClient::flavor`]) and the resolved body content; the
/// check is a no-op when the body is wiki text.
fn assert_adf_supported(flavor: JiraFlavor, body: &JiraBodyContent) -> anyhow::Result<()> {
    if matches!(body, JiraBodyContent::Adf(_)) && flavor != JiraFlavor::Cloud {
        return Err(crate::error::Error::Config(
            "ADF input is not supported on Data Center / Server (v3 API not available)".into(),
        )
        .into());
    }
    Ok(())
}

/// Map a read-side [`JiraBodyFormat`] to the [`JiraApiVersion`] the client
/// must call to fetch a body in that format.
///
/// `Adf` requires v3 (Cloud only); `Wiki` and `Markdown` both fetch wiki text
/// from v2 — `Markdown` then runs the result through `wiki_to_markdown` (or
/// `adf_to_markdown` when the description field comes back as an ADF object,
/// which can happen on Cloud regardless of API version).
fn read_api_version_for(fmt: JiraBodyFormat) -> JiraApiVersion {
    match fmt {
        JiraBodyFormat::Adf => JiraApiVersion::V3,
        JiraBodyFormat::Wiki | JiraBodyFormat::Markdown => JiraApiVersion::V2,
    }
}

/// Reject `--body-format adf` on Data Center / Server with a typed
/// `Error::Config`. Mirror of [`assert_adf_supported`] for read paths.
fn assert_adf_read_supported(flavor: JiraFlavor, fmt: JiraBodyFormat) -> anyhow::Result<()> {
    if fmt == JiraBodyFormat::Adf && flavor != JiraFlavor::Cloud {
        return Err(crate::error::Error::Config(
            "--body-format adf is not supported on Data Center / Server (v3 API not available)"
                .into(),
        )
        .into());
    }
    Ok(())
}

/// Convert a Jira body field (`description` / comment `body`) to markdown if
/// requested.
///
/// Auto-detects the body shape: a JSON string is treated as Jira wiki text
/// and run through `wiki_to_markdown`; a JSON object with `"type": "doc"` is
/// treated as an ADF document and run through `adf_to_markdown`. `null` is
/// preserved. Anything else passes through unchanged so unexpected shapes
/// don't get silently corrupted. The auto-detect makes the same handler work
/// transparently across v2 (wiki strings) and v3 (ADF objects) Cloud
/// responses.
fn convert_body_field(value: &Value, render_directives: bool) -> anyhow::Result<Value> {
    if let Some(text) = value.as_str() {
        let opts = WikiConvertOpts { render_directives };
        let md = wiki_to_markdown(text, opts)
            .with_context(|| "failed to convert Jira wiki body to markdown")?;
        Ok(Value::String(md))
    } else if value.is_object() && value.get("type").and_then(Value::as_str) == Some("doc") {
        let opts = AdfConvertOpts { render_directives };
        let md = adf_to_markdown(value, opts)
            .with_context(|| "failed to convert ADF body to markdown")?;
        Ok(Value::String(md))
    } else {
        Ok(value.clone())
    }
}

/// Walk an issue JSON and rewrite description + comment bodies into markdown
/// when [`JiraBodyFormat::Markdown`] is requested. No-op for `Wiki` and
/// `Adf` (raw passthrough).
fn convert_issue_bodies(
    issue: &mut Value,
    fmt: JiraBodyFormat,
    render_directives: bool,
) -> anyhow::Result<()> {
    if fmt != JiraBodyFormat::Markdown {
        return Ok(());
    }
    if let Some(desc) = issue.pointer_mut("/fields/description") {
        *desc = convert_body_field(desc, render_directives)?;
    }
    if let Some(comments) = issue.pointer_mut("/fields/comment/comments")
        && let Some(arr) = comments.as_array_mut()
    {
        for c in arr.iter_mut() {
            if let Some(body) = c.get_mut("body") {
                *body = convert_body_field(body, render_directives)?;
            }
        }
    }
    Ok(())
}

/// Walk a comments-list response and rewrite each comment body into markdown
/// when [`JiraBodyFormat::Markdown`] is requested. No-op for `Wiki` and `Adf`.
fn convert_comments_bodies(
    value: &mut Value,
    fmt: JiraBodyFormat,
    render_directives: bool,
) -> anyhow::Result<()> {
    if fmt != JiraBodyFormat::Markdown {
        return Ok(());
    }
    if let Some(arr) = value.pointer_mut("/comments").and_then(Value::as_array_mut) {
        for c in arr.iter_mut() {
            if let Some(body) = c.get_mut("body") {
                *body = convert_body_field(body, render_directives)?;
            }
        }
    }
    Ok(())
}

/// Walk a single-comment response and rewrite its `body` field into markdown
/// when [`JiraBodyFormat::Markdown`] is requested. No-op for `Wiki` and `Adf`.
fn convert_comment_body(
    value: &mut Value,
    fmt: JiraBodyFormat,
    render_directives: bool,
) -> anyhow::Result<()> {
    if fmt != JiraBodyFormat::Markdown {
        return Ok(());
    }
    if let Some(body) = value.get_mut("body") {
        *body = convert_body_field(body, render_directives)?;
    }
    Ok(())
}

pub(super) fn today_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = now / 86400;
    // Algorithm to convert days since epoch to YYYY-MM-DD
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Howard Hinnant's algorithm
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub(super) fn insert_extra_fields(
    map: &mut serde_json::Map<String, Value>,
    fix_version: &Option<String>,
    component: &Option<String>,
    custom_fields: &[String],
) -> anyhow::Result<()> {
    if let Some(fv) = fix_version {
        let versions: Vec<Value> = fv.split(',').map(|v| json!({ "name": v.trim() })).collect();
        map.insert("fixVersions".into(), Value::Array(versions));
    }
    if let Some(comp) = component {
        let components: Vec<Value> = comp
            .split(',')
            .map(|c| json!({ "name": c.trim() }))
            .collect();
        map.insert("components".into(), Value::Array(components));
    }
    for entry in custom_fields {
        let (key, raw_val) = entry.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid --custom format: expected KEY=VALUE, got '{entry}'")
        })?;
        let val = serde_json::from_str(raw_val).unwrap_or(Value::String(raw_val.to_string()));
        map.insert(key.to_string(), val);
    }
    Ok(())
}

/// Escape a value for safe interpolation into a JQL quoted string.
fn escape_jql(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Build the `fields` JSON object for an issue create request.
///
/// `description_value` is the already-resolved description payload — a JSON
/// string when the user picked wiki/markdown input, a JSON object when they
/// picked ADF input. HTTP I/O and stdin/file resolution happen at the caller;
/// this helper is pure so it can be unit-tested.
fn build_create_fields(
    args: &JiraCreateArgs,
    description_value: Option<Value>,
) -> anyhow::Result<Value> {
    let mut fields = json!({
        "project": { "key": &args.project },
        "issuetype": { "name": &args.issue_type },
        "summary": &args.summary,
    });
    if let Some(map) = fields.as_object_mut() {
        if let Some(body) = description_value {
            map.insert("description".into(), body);
        }
        if let Some(assignee) = &args.assignee {
            map.insert("assignee".into(), json!({ "accountId": assignee }));
        }
        if let Some(priority) = &args.priority {
            map.insert("priority".into(), json!({ "name": priority }));
        }
        if let Some(labels) = &args.labels {
            let label_list: Vec<Value> = labels
                .split(',')
                .map(|l| Value::String(l.trim().to_string()))
                .collect();
            map.insert("labels".into(), Value::Array(label_list));
        }
        if let Some(parent) = &args.parent {
            map.insert("parent".into(), json!({ "key": parent }));
        }
        insert_extra_fields(map, &args.fix_version, &args.component, &args.custom_fields)?;
    }
    Ok(fields)
}

/// Build the `fields` map for an issue update request.
///
/// `description_value` is the already-resolved description payload (string
/// for wiki/markdown input, JSON object for ADF input); pass `None` to leave
/// description unchanged. Returns an error if the resulting map is empty
/// (the user passed `update` with no field flags).
fn build_update_fields(
    args: &JiraUpdateArgs,
    description_value: Option<Value>,
) -> anyhow::Result<serde_json::Map<String, Value>> {
    let mut fields = serde_json::Map::new();
    if let Some(summary) = &args.summary {
        fields.insert("summary".into(), Value::String(summary.clone()));
    }
    if let Some(body) = description_value {
        fields.insert("description".into(), body);
    }
    if let Some(assignee) = &args.assignee {
        fields.insert("assignee".into(), json!({ "accountId": assignee }));
    }
    if let Some(priority) = &args.priority {
        fields.insert("priority".into(), json!({ "name": priority }));
    }
    if let Some(labels) = &args.labels {
        let label_list: Vec<Value> = labels
            .split(',')
            .map(|l| Value::String(l.trim().to_string()))
            .collect();
        fields.insert("labels".into(), Value::Array(label_list));
    }
    insert_extra_fields(
        &mut fields,
        &args.fix_version,
        &args.component,
        &args.custom_fields,
    )?;
    if fields.is_empty() {
        anyhow::bail!(
            "no fields to update; specify at least one of --summary, --description, --assignee, --priority, --labels, --fix-version, --component, --custom"
        );
    }
    Ok(fields)
}

/// Build a clone payload by copying selected fields from a source issue and
/// overriding the summary.
///
/// Pure: takes the source issue JSON (as returned by `client.get_issue`) and
/// optionally an override summary, returns the new `fields` map ready to wrap
/// in `{"fields": ...}`.
fn build_clone_fields(
    source: &Value,
    override_summary: Option<&str>,
) -> anyhow::Result<serde_json::Map<String, Value>> {
    let source_fields = source
        .get("fields")
        .and_then(|f| f.as_object())
        .ok_or_else(|| anyhow::anyhow!("could not read fields from source issue"))?;

    let mut new_fields = serde_json::Map::new();

    for key in [
        "project",
        "issuetype",
        "description",
        "priority",
        "labels",
        "components",
        "fixVersions",
    ] {
        if let Some(val) = source_fields.get(key)
            && !val.is_null()
        {
            new_fields.insert(key.to_string(), val.clone());
        }
    }

    let summary = if let Some(s) = override_summary {
        s.to_string()
    } else {
        let original = source_fields
            .get("summary")
            .and_then(|s| s.as_str())
            .unwrap_or("(no summary)");
        format!("[Clone] {original}")
    };
    new_fields.insert("summary".into(), Value::String(summary));

    Ok(new_fields)
}

/// Build the JSON payload for the `notify` endpoint.
///
/// `body` is already-resolved (no @file / `-` handling here).
fn build_notify_payload(subject: &str, body: &str, to: &[String]) -> Value {
    let mut payload = json!({
        "subject": subject,
        "textBody": body,
    });
    if !to.is_empty() {
        let users: Vec<Value> = to.iter().map(|id| json!({ "accountId": id })).collect();
        payload["to"] = json!({ "users": users });
    }
    payload
}

/// Parse the user-supplied JSON body for `bulk-create`. Accepts either a raw
/// array of field objects or the full `{"issueUpdates": [...]}` envelope and
/// always returns the envelope shape that the API expects.
fn parse_bulk_create_payload(raw: &str) -> anyhow::Result<Value> {
    let parsed: Value =
        serde_json::from_str(raw).map_err(|e| anyhow::anyhow!("invalid JSON input: {e}"))?;
    if parsed.is_array() {
        let updates: Vec<Value> = parsed
            .as_array()
            .unwrap()
            .iter()
            .map(|fields| json!({ "fields": fields }))
            .collect();
        Ok(json!({ "issueUpdates": updates }))
    } else if parsed.get("issueUpdates").is_some() {
        Ok(parsed)
    } else {
        anyhow::bail!("expected a JSON array of field objects or an object with 'issueUpdates' key")
    }
}

fn build_jql(args: &JiraSearchArgs) -> anyhow::Result<String> {
    let mut clauses = Vec::new();
    let mut raw_order_by: Option<String> = None;

    if let Some(jql) = &args.jql {
        // Split off ORDER BY clause so it doesn't get wrapped in parentheses.
        if let Some(pos) = jql.to_ascii_uppercase().find(" ORDER BY ") {
            let (filter_part, order_part) = jql.split_at(pos);
            clauses.push(format!("({filter_part})"));
            raw_order_by = Some(order_part.to_string());
        } else {
            clauses.push(format!("({jql})"));
        }
    }
    if let Some(v) = &args.status {
        clauses.push(format!("status = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.priority {
        clauses.push(format!("priority = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.assignee {
        if v == "currentUser()" {
            clauses.push("assignee = currentUser()".to_string());
        } else {
            clauses.push(format!("assignee = \"{}\"", escape_jql(v)));
        }
    }
    if let Some(v) = &args.reporter {
        if v == "currentUser()" {
            clauses.push("reporter = currentUser()".to_string());
        } else {
            clauses.push(format!("reporter = \"{}\"", escape_jql(v)));
        }
    }
    if let Some(v) = &args.r#type {
        clauses.push(format!("type = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.label {
        clauses.push(format!("labels = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.component {
        clauses.push(format!("component = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.resolution {
        clauses.push(format!("resolution = \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.created {
        clauses.push(format!("created >= \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.created_after {
        clauses.push(format!("created > \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.updated {
        clauses.push(format!("updated >= \"{}\"", escape_jql(v)));
    }
    if let Some(v) = &args.updated_after {
        clauses.push(format!("updated > \"{}\"", escape_jql(v)));
    }
    if args.watching {
        clauses.push("watcher = currentUser()".to_string());
    }

    if clauses.is_empty() {
        anyhow::bail!("provide a JQL query or at least one filter flag");
    }

    let mut jql = clauses.join(" AND ");

    // --order-by flag takes precedence over ORDER BY in raw JQL
    if let Some(field) = &args.order_by {
        let dir = if args.reverse { "DESC" } else { "ASC" };
        jql.push_str(&format!(" ORDER BY {field} {dir}"));
    } else if let Some(order) = &raw_order_by {
        jql.push_str(order);
    }

    Ok(jql)
}

pub async fn run(
    cmd: &JiraSubcommand,
    config_path: Option<&Utf8Path>,
    profile_name: Option<&str>,
    retry_cfg: RetryConfig,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    // `--web` short-circuits to the browse helper so we don't hit the Jira
    // API at all — the URL is derived from the configured domain + key.
    if let JiraSubcommand::View(args) = cmd
        && args.web
    {
        let browse_args = crate::cli::args::BrowseArgs {
            target: args.key.clone(),
            service: crate::cli::args::BrowseService::Jira,
        };
        return super::browse::run(&browse_args, config_path, profile_name, retry_cfg, io).await;
    }

    let config = ConfigLoader::load(config_path)?;
    let resolved_profile_name = profile_name
        .or(config.as_ref().map(|c| c.default_profile.as_str()))
        .unwrap_or("default");
    let profile = config
        .as_ref()
        .and_then(|c| c.resolve_profile(Some(resolved_profile_name)))
        .ok_or_else(|| {
            crate::error::Error::Config("no profile found; run `atl init` first".into())
        })?;
    let instance = profile.jira.as_ref().ok_or_else(|| {
        crate::error::Error::Config("no Jira instance configured in profile".into())
    })?;
    let store = SystemKeyring;

    let client = JiraClient::new(instance, resolved_profile_name, &store, retry_cfg)?;

    dispatch(cmd, &client, format, io, transforms).await
}

/// Returns true when the long-form output of `cmd` would benefit from a
/// pager. Only the read-heavy "view" commands qualify.
fn cmd_uses_pager(cmd: &JiraSubcommand) -> bool {
    matches!(cmd, JiraSubcommand::View(_) | JiraSubcommand::Search(_))
}

/// Flattens Jira issue objects for human-readable console table display.
///
/// Extracts key fields from the nested `fields` object and drops metadata
/// like `expand`, `id`, and `self` that clutter the table.
fn flatten_issues(value: Value) -> Value {
    let Value::Array(issues) = value else {
        return value;
    };
    let flat: Vec<Value> = issues
        .into_iter()
        .map(|issue| {
            let key = issue.get("key").and_then(Value::as_str).unwrap_or_default();
            let fields = issue.get("fields").unwrap_or(&Value::Null);
            let summary = fields
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let status = fields
                .get("status")
                .and_then(|s| s.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let priority = fields
                .get("priority")
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let assignee = fields
                .get("assignee")
                .and_then(|a| a.get("displayName"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let issue_type = fields
                .get("issuetype")
                .and_then(|t| t.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();

            let mut map = serde_json::Map::new();
            map.insert("key".into(), Value::String(key.into()));
            map.insert("summary".into(), Value::String(summary.into()));
            map.insert("status".into(), Value::String(status.into()));
            map.insert("priority".into(), Value::String(priority.into()));
            map.insert("assignee".into(), Value::String(assignee.into()));
            if !issue_type.is_empty() {
                map.insert("type".into(), Value::String(issue_type.into()));
            }
            Value::Object(map)
        })
        .collect();
    Value::Array(flat)
}

/// Flattens a single Jira issue for human-readable console display.
///
/// Extracts key fields from the nested `fields` object and produces a flat
/// key-value object that the console reporter renders as a readable list
/// instead of a giant JSON blob.
fn flatten_issue(value: Value) -> Value {
    let fields = value.get("fields").unwrap_or(&Value::Null);
    let key = value.get("key").and_then(Value::as_str).unwrap_or_default();

    let summary = fields
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let status = fields
        .get("status")
        .and_then(|s| s.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let priority = fields
        .get("priority")
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let issue_type = fields
        .get("issuetype")
        .and_then(|t| t.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let project = fields
        .get("project")
        .and_then(|p| p.get("key"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let assignee = fields
        .get("assignee")
        .and_then(|a| a.get("displayName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let reporter = fields
        .get("reporter")
        .and_then(|r| r.get("displayName"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let created = fields
        .get("created")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let updated = fields
        .get("updated")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let description = fields
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let resolution = fields
        .get("resolution")
        .and_then(|r| r.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let labels = fields
        .get("labels")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let components = fields
        .get("components")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let mut map = serde_json::Map::new();
    map.insert("key".into(), Value::String(key.into()));
    map.insert("summary".into(), Value::String(summary.into()));
    map.insert("type".into(), Value::String(issue_type.into()));
    map.insert("status".into(), Value::String(status.into()));
    map.insert("priority".into(), Value::String(priority.into()));
    map.insert("assignee".into(), Value::String(assignee.into()));
    map.insert("reporter".into(), Value::String(reporter.into()));
    map.insert("project".into(), Value::String(project.into()));
    if !labels.is_empty() {
        map.insert("labels".into(), Value::String(labels));
    }
    if !components.is_empty() {
        map.insert("components".into(), Value::String(components));
    }
    if !resolution.is_empty() {
        map.insert("resolution".into(), Value::String(resolution.into()));
    }
    map.insert("created".into(), Value::String(created.into()));
    map.insert("updated".into(), Value::String(updated.into()));
    if !description.is_empty() {
        map.insert("description".into(), Value::String(description.into()));
    }

    Value::Object(map)
}

async fn dispatch(
    cmd: &JiraSubcommand,
    client: &JiraClient,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let value = match cmd {
        JiraSubcommand::Search(args) => {
            let jql = build_jql(args)?;
            let fields: Vec<&str> = args.fields.split(',').map(str::trim).collect();
            let value = if args.all {
                client.search_issues_all(&jql, args.limit, &fields).await?
            } else {
                client.search_issues(&jql, args.limit, &fields).await?
            };
            // For human-readable output, extract the issues array and flatten
            // nested fields so the console reporter renders a clean table
            // instead of a raw JSON blob. Skip flattening when the user
            // requested custom fields — flatten would drop them.
            let is_default_fields = args.fields == "key,summary,status,assignee,priority";
            if matches!(format, OutputFormat::Console) {
                let issues = value.get("issues").cloned().unwrap_or(value);
                if is_default_fields {
                    flatten_issues(issues)
                } else {
                    issues
                }
            } else {
                value
            }
        }
        JiraSubcommand::View(args) => {
            assert_adf_read_supported(client.flavor(), args.body_format)?;
            let api_version = read_api_version_for(args.body_format);
            let mut value = client.get_issue(&args.key, &[], api_version).await?;
            convert_issue_bodies(&mut value, args.body_format, !args.no_directives)?;
            if matches!(format, OutputFormat::Console) {
                flatten_issue(value)
            } else {
                value
            }
        }
        JiraSubcommand::Create(args) => {
            let description_value = if let Some(desc) = &args.description {
                let raw = read_body_arg(desc).context("failed to read --description body")?;
                let body = convert_input(raw, &args.input_format)?;
                assert_adf_supported(client.flavor(), &body)?;
                let version = api_version_for(&body);
                Some((body_field_value(body), version))
            } else {
                None
            };
            let api_version = description_value
                .as_ref()
                .map_or(JiraApiVersion::V2, |(_, v)| *v);
            let description = description_value.map(|(v, _)| v);
            let fields = build_create_fields(args, description)?;
            client
                .create_issue(&json!({ "fields": fields }), api_version)
                .await?
        }
        JiraSubcommand::Update(args) => {
            let description_value = if let Some(desc) = &args.description {
                let raw = read_body_arg(desc).context("failed to read --description body")?;
                let body = convert_input(raw, &args.input_format)?;
                assert_adf_supported(client.flavor(), &body)?;
                let version = api_version_for(&body);
                Some((body_field_value(body), version))
            } else {
                None
            };
            let api_version = description_value
                .as_ref()
                .map_or(JiraApiVersion::V2, |(_, v)| *v);
            let description = description_value.map(|(v, _)| v);
            let fields = build_update_fields(args, description)?;
            client
                .update_issue(&args.key, &json!({ "fields": fields }), api_version)
                .await?;
            Value::String(format!("Issue {} updated", args.key))
        }
        JiraSubcommand::Delete(args) => {
            client.delete_issue(&args.key, args.delete_subtasks).await?;
            Value::String(format!("Issue {} deleted", args.key))
        }
        JiraSubcommand::Move(args) => {
            client.transition_issue(&args.key, &args.transition).await?;
            Value::String(format!("Issue {} transitioned", args.key))
        }
        JiraSubcommand::Assign(args) => {
            client.assign_issue(&args.key, &args.account_id).await?;
            Value::String(format!("Issue {} assigned", args.key))
        }
        JiraSubcommand::Comment(args) => {
            let raw = read_body_arg(&args.body).context("failed to read comment body argument")?;
            let body = convert_input(raw, &args.input_format)?;
            assert_adf_supported(client.flavor(), &body)?;
            client.add_comment(&args.key, &body).await?
        }
        JiraSubcommand::Comments(args) => {
            assert_adf_read_supported(client.flavor(), args.body_format)?;
            let api_version = read_api_version_for(args.body_format);
            let mut value = client.list_comments(&args.key, api_version).await?;
            convert_comments_bodies(&mut value, args.body_format, !args.no_directives)?;
            value
        }
        JiraSubcommand::CommentGet(args) => {
            assert_adf_read_supported(client.flavor(), args.body_format)?;
            let api_version = read_api_version_for(args.body_format);
            let mut value = client
                .get_comment(&args.key, &args.comment_id, api_version)
                .await?;
            convert_comment_body(&mut value, args.body_format, !args.no_directives)?;
            value
        }
        JiraSubcommand::CommentDelete(args) => {
            client.delete_comment(&args.key, &args.comment_id).await?;
            Value::String(format!(
                "Comment {} deleted from {}",
                args.comment_id, args.key
            ))
        }
        JiraSubcommand::Transitions(args) => client.get_transitions(&args.key).await?,
        JiraSubcommand::Project(cmd) => project::dispatch_project(&cmd.command, client).await?,
        JiraSubcommand::Board(cmd) => board::dispatch_board(&cmd.command, client).await?,
        JiraSubcommand::Sprint(cmd) => sprint::dispatch_sprint(&cmd.command, client).await?,
        JiraSubcommand::BacklogMove(args) => {
            client.move_issues_to_backlog(&args.issues).await?;
            Value::String("Issues moved to backlog".to_string())
        }
        JiraSubcommand::Me => client.get_myself().await?,
        JiraSubcommand::Epic(epic_cmd) => sprint::dispatch_epic(&epic_cmd.command, client).await?,
        JiraSubcommand::Link(args) => {
            client
                .create_issue_link(&args.link_type, &args.inward_key, &args.outward_key)
                .await?;
            Value::String(format!(
                "Linked {} -> {} ({})",
                args.inward_key, args.outward_key, args.link_type
            ))
        }
        JiraSubcommand::LinkType(cmd) => admin::dispatch_link_type(&cmd.command, client).await?,
        JiraSubcommand::IssueLinkGet(args) => client.get_issue_link(&args.id).await?,
        JiraSubcommand::IssueLinkDelete(args) => {
            client.delete_issue_link(&args.id).await?;
            Value::String(format!("Issue link {} deleted", args.id))
        }
        JiraSubcommand::RemoteLink(args) => {
            let title = args.title.as_deref().unwrap_or(&args.url);
            client.add_remote_link(&args.key, &args.url, title).await?
        }
        JiraSubcommand::RemoteLinks(args) => client.get_remote_links(&args.key).await?,
        JiraSubcommand::RemoteLinkDelete(args) => {
            client.delete_remote_link(&args.key, &args.link_id).await?;
            Value::String(format!(
                "Remote link {} deleted from {}",
                args.link_id, args.key
            ))
        }
        JiraSubcommand::Clone(args) => {
            // Clone preserves the source description shape — read and write
            // through v2 since `get_issue(.., V2)` returns wiki-text
            // descriptions and the v2 create endpoint accepts the same
            // shape unchanged.
            let source = client.get_issue(&args.key, &[], JiraApiVersion::V2).await?;
            let new_fields = build_clone_fields(&source, args.summary.as_deref())?;
            client
                .create_issue(&json!({ "fields": new_fields }), JiraApiVersion::V2)
                .await?
        }
        JiraSubcommand::Worklog(wl_cmd) => {
            filter::dispatch_worklog(&wl_cmd.command, client).await?
        }
        JiraSubcommand::Filter(f_cmd) => filter::dispatch_filter(&f_cmd.command, client).await?,
        JiraSubcommand::Attach(args) => client.attach_file(&args.key, &args.file).await?,
        JiraSubcommand::Dashboard(cmd) => admin::dispatch_dashboard(&cmd.command, client).await?,
        JiraSubcommand::Field(cmd) => field::dispatch_field(&cmd.command, client).await?,
        JiraSubcommand::User(cmd) => user::dispatch_user(&cmd.command, client).await?,
        JiraSubcommand::Group(cmd) => user::dispatch_group(&cmd.command, client).await?,
        JiraSubcommand::Version(ver_cmd) => {
            admin::dispatch_version(&ver_cmd.command, client).await?
        }
        JiraSubcommand::Component(comp_cmd) => {
            admin::dispatch_component(&comp_cmd.command, client).await?
        }
        JiraSubcommand::Vote(args) => {
            client.vote_issue(&args.key).await?;
            Value::String(format!("Voted for {}", args.key))
        }
        JiraSubcommand::Unvote(args) => {
            client.unvote_issue(&args.key).await?;
            Value::String(format!("Vote removed from {}", args.key))
        }
        JiraSubcommand::Changelog(args) => {
            if args.all {
                let url = format!("{}/issue/{}/changelog", client.base_url(), args.key);
                client
                    .paginate_offset(&url, args.limit, "values", &[])
                    .await?
            } else {
                client
                    .get_changelog(&args.key, args.limit, args.start_at)
                    .await?
            }
        }
        JiraSubcommand::Watch(args) => {
            client.watch_issue(&args.key).await?;
            Value::String(format!("Now watching {}", args.key))
        }
        JiraSubcommand::Unwatch(args) => {
            client.unwatch_issue(&args.key).await?;
            Value::String(format!("Stopped watching {}", args.key))
        }
        JiraSubcommand::Watchers(args) => client.get_watchers(&args.key).await?,
        JiraSubcommand::Notify(args) => {
            let body = read_body_arg(&args.body)?;
            let payload = build_notify_payload(&args.subject, &body, &args.to);
            client.notify_issue(&args.key, &payload).await?;
            Value::String(format!("Notification sent for {}", args.key))
        }
        JiraSubcommand::CreateMeta(args) => {
            client
                .get_create_meta(args.project.as_deref(), args.issue_type.as_deref())
                .await?
        }
        JiraSubcommand::EditMeta(args) => client.get_edit_meta(&args.key).await?,
        JiraSubcommand::IssueType(cmd) => field::dispatch_issue_type(&cmd.command, client).await?,
        JiraSubcommand::Priority(cmd) => field::dispatch_priority(&cmd.command, client).await?,
        JiraSubcommand::Resolution(cmd) => field::dispatch_resolution(&cmd.command, client).await?,
        JiraSubcommand::Status(cmd) => field::dispatch_status(&cmd.command, client).await?,
        JiraSubcommand::Screen(cmd) => workflow::dispatch_screen(&cmd.command, client).await?,
        JiraSubcommand::Workflow(cmd) => workflow::dispatch_workflow(&cmd.command, client).await?,
        JiraSubcommand::WorkflowScheme(cmd) => {
            workflow::dispatch_workflow_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::PermissionScheme(cmd) => {
            workflow::dispatch_permission_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::NotificationScheme(cmd) => {
            workflow::dispatch_notification_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::IssueSecurityScheme(cmd) => {
            workflow::dispatch_issue_security_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::FieldConfig(cmd) => {
            workflow::dispatch_field_config(&cmd.command, client).await?
        }
        JiraSubcommand::ProjectCategory(cmd) => {
            admin::dispatch_project_category(&cmd.command, client).await?
        }
        JiraSubcommand::IssueTypeScheme(cmd) => {
            workflow::dispatch_issue_type_scheme(&cmd.command, client).await?
        }
        JiraSubcommand::Role(cmd) => admin::dispatch_role(&cmd.command, client).await?,
        JiraSubcommand::Banner(cmd) => admin::dispatch_banner(&cmd.command, client).await?,
        JiraSubcommand::Configuration => client.get_configuration().await?,
        JiraSubcommand::Task(cmd) => admin::dispatch_task(&cmd.command, client).await?,
        JiraSubcommand::Attachment(cmd) => {
            admin::dispatch_attachment_admin(&cmd.command, client).await?
        }
        JiraSubcommand::ServerInfo => client.get_server_info().await?,
        JiraSubcommand::Webhook(cmd) => admin::dispatch_webhook(&cmd.command, client).await?,
        JiraSubcommand::AuditRecords(args) => {
            client
                .get_audit_records(
                    args.limit,
                    args.offset,
                    args.filter.as_deref(),
                    args.from.as_deref(),
                    args.to.as_deref(),
                )
                .await?
        }
        JiraSubcommand::Permissions => client.get_all_permissions().await?,
        JiraSubcommand::MyPermissions => client.get_my_permissions().await?,
        JiraSubcommand::Labels(args) => {
            if args.all {
                let url = format!("{}/label", client.base_url());
                client
                    .paginate_offset(&url, args.limit, "values", &[])
                    .await?
            } else {
                client.list_labels(args.limit).await?
            }
        }
        JiraSubcommand::BulkCreate(args) => {
            let raw = read_body_arg(&args.input)?;
            let payload = parse_bulk_create_payload(&raw)?;
            client.bulk_create_issues(&payload).await?
        }
        JiraSubcommand::Archive(args) => {
            if args.keys.len() == 1 {
                client.archive_issue(&args.keys[0]).await?;
                Value::String(format!("Issue {} archived", args.keys[0]))
            } else {
                client.archive_issues_bulk(&args.keys).await?
            }
        }
        JiraSubcommand::Unarchive(args) => client.unarchive_issues_bulk(&args.keys).await?,
    };

    // Start the pager before writing the (potentially long) response so the
    // user can scroll. The pager only engages on Console output to a TTY when
    // the command was a long-form view; everything else stays inline.
    let use_pager = matches!(format, OutputFormat::Console)
        && io.is_stdout_tty()
        && !io.pager_disabled()
        && cmd_uses_pager(cmd);
    if use_pager {
        io.start_pager()?;
    }

    let write_res = write_output(value, format, io, transforms);
    let stop_res = if use_pager { io.stop_pager() } else { Ok(()) };
    write_res?;
    stop_res?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a `JiraSearchArgs` with all fields defaulted.
    fn default_search_args() -> JiraSearchArgs {
        JiraSearchArgs {
            jql: None,
            limit: 50,
            all: false,
            fields: "key,summary".to_string(),
            status: None,
            priority: None,
            assignee: None,
            reporter: None,
            r#type: None,
            label: None,
            component: None,
            resolution: None,
            created: None,
            created_after: None,
            updated: None,
            updated_after: None,
            watching: false,
            order_by: None,
            reverse: false,
        }
    }

    #[test]
    fn build_jql_raw_only() {
        let mut args = default_search_args();
        args.jql = Some("project = FOO".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "(project = FOO)");
    }

    #[test]
    fn build_jql_raw_parenthesized_with_filter() {
        let mut args = default_search_args();
        args.jql = Some("status = Done OR assignee = me".to_string());
        args.status = Some("Open".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(
            result,
            "(status = Done OR assignee = me) AND status = \"Open\""
        );
    }

    #[test]
    fn build_jql_raw_with_order_by() {
        let mut args = default_search_args();
        args.jql = Some("project = FOO ORDER BY created DESC".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "(project = FOO) ORDER BY created DESC");
    }

    #[test]
    fn build_jql_raw_order_by_with_filter() {
        let mut args = default_search_args();
        args.jql = Some("project = FOO ORDER BY created".to_string());
        args.status = Some("Open".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(
            result,
            "(project = FOO) AND status = \"Open\" ORDER BY created"
        );
    }

    #[test]
    fn build_jql_raw_order_by_overridden_by_flag() {
        let mut args = default_search_args();
        args.jql = Some("project = FOO ORDER BY created".to_string());
        args.order_by = Some("updated".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "(project = FOO) ORDER BY updated ASC");
    }

    #[test]
    fn build_jql_raw_order_by_case_insensitive() {
        let mut args = default_search_args();
        args.jql = Some("project = FOO order by created DESC".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "(project = FOO) order by created DESC");
    }

    #[test]
    fn build_jql_status_only() {
        let mut args = default_search_args();
        args.status = Some("In Progress".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "status = \"In Progress\"");
    }

    #[test]
    fn build_jql_empty_returns_error() {
        let args = default_search_args();
        assert!(build_jql(&args).is_err());
    }

    // ---- civil_from_days ----

    #[test]
    fn civil_from_days_unix_epoch() {
        assert_eq!(
            civil_from_days(0),
            (1970, 1, 1),
            "day 0 should be the Unix epoch"
        );
    }

    #[test]
    fn civil_from_days_known_date() {
        // 2023-03-15 is 19431 days after the Unix epoch.
        assert_eq!(civil_from_days(19431), (2023, 3, 15), "expected 2023-03-15");
    }

    #[test]
    fn civil_from_days_leap_year() {
        // 2024-02-29 is 19782 days after the Unix epoch.
        assert_eq!(
            civil_from_days(19782),
            (2024, 2, 29),
            "expected leap day 2024-02-29"
        );
    }

    #[test]
    fn civil_from_days_negative() {
        assert_eq!(
            civil_from_days(-1),
            (1969, 12, 31),
            "day -1 should be 1969-12-31"
        );
    }

    // ---- escape_jql ----

    #[test]
    fn escape_jql_no_special_chars() {
        assert_eq!(escape_jql("hello"), "hello");
    }

    #[test]
    fn escape_jql_escapes_backslash_and_quote() {
        assert_eq!(
            escape_jql(r#"back\slash and "quote""#),
            r#"back\\slash and \"quote\""#,
            "both backslash and double-quote must be escaped"
        );
    }

    #[test]
    fn escape_jql_empty_string() {
        assert_eq!(escape_jql(""), "");
    }

    // ---- flatten_issues ----

    #[test]
    fn flatten_issues_extracts_fields() {
        let input = json!([
            {
                "expand": "renderedFields",
                "id": "12294",
                "self": "https://example.atlassian.net/rest/api/3/issue/12294",
                "key": "ORB-196",
                "fields": {
                    "summary": "Some summary",
                    "status": { "name": "To Do" },
                    "priority": { "name": "High" },
                    "assignee": { "displayName": "laskin.sergey" },
                    "issuetype": { "name": "Task" }
                }
            }
        ]);
        let result = flatten_issues(input);
        let issues = result.as_array().expect("should be an array");
        assert_eq!(issues.len(), 1);

        let issue = &issues[0];
        assert_eq!(issue.get("key").and_then(Value::as_str), Some("ORB-196"));
        assert_eq!(
            issue.get("summary").and_then(Value::as_str),
            Some("Some summary")
        );
        assert_eq!(issue.get("status").and_then(Value::as_str), Some("To Do"));
        assert_eq!(issue.get("priority").and_then(Value::as_str), Some("High"));
        assert_eq!(
            issue.get("assignee").and_then(Value::as_str),
            Some("laskin.sergey")
        );
        assert_eq!(issue.get("type").and_then(Value::as_str), Some("Task"));

        // Metadata fields should be absent.
        assert!(issue.get("expand").is_none());
        assert!(issue.get("id").is_none());
        assert!(issue.get("self").is_none());
        assert!(issue.get("fields").is_none());
    }

    #[test]
    fn flatten_issues_null_assignee() {
        let input = json!([
            {
                "key": "ORB-10",
                "fields": {
                    "summary": "No assignee",
                    "status": { "name": "Open" },
                    "priority": { "name": "Low" },
                    "assignee": null
                }
            }
        ]);
        let result = flatten_issues(input);
        let issue = &result.as_array().unwrap()[0];
        assert_eq!(issue.get("assignee").and_then(Value::as_str), Some(""));
    }

    #[test]
    fn flatten_issues_no_issuetype_omits_type() {
        let input = json!([
            {
                "key": "ORB-11",
                "fields": {
                    "summary": "No type",
                    "status": { "name": "Done" },
                    "priority": { "name": "Medium" }
                }
            }
        ]);
        let result = flatten_issues(input);
        let issue = &result.as_array().unwrap()[0];
        assert!(
            issue.get("type").is_none(),
            "type column should be absent when issuetype is missing"
        );
    }

    #[test]
    fn flatten_issues_non_array_passthrough() {
        let input = json!({"total": 0, "issues": []});
        let result = flatten_issues(input.clone());
        assert_eq!(
            result, input,
            "non-array input should pass through unchanged"
        );
    }

    #[test]
    fn flatten_issues_preserves_column_order() {
        let input = json!([
            {
                "key": "X-1",
                "fields": {
                    "summary": "s",
                    "status": { "name": "st" },
                    "priority": { "name": "p" },
                    "assignee": { "displayName": "a" },
                    "issuetype": { "name": "t" }
                }
            }
        ]);
        let result = flatten_issues(input);
        let issue = result.as_array().unwrap()[0].as_object().unwrap();
        let keys: Vec<&String> = issue.keys().collect();
        assert_eq!(
            keys,
            vec!["key", "summary", "status", "priority", "assignee", "type"],
            "columns should appear in insertion order with preserve_order enabled"
        );
    }

    // ---- insert_extra_fields ----

    #[test]
    fn insert_fix_version_comma_split() {
        let mut map = serde_json::Map::new();
        insert_extra_fields(&mut map, &Some("v1, v2".to_string()), &None, &[]).unwrap();
        let versions = map.get("fixVersions").expect("fixVersions key missing");
        assert_eq!(
            *versions,
            json!([{"name": "v1"}, {"name": "v2"}]),
            "comma-separated versions should split and trim into name objects"
        );
    }

    #[test]
    fn insert_component_comma_split() {
        let mut map = serde_json::Map::new();
        insert_extra_fields(&mut map, &None, &Some("frontend, backend".to_string()), &[]).unwrap();
        let components = map.get("components").expect("components key missing");
        assert_eq!(
            *components,
            json!([{"name": "frontend"}, {"name": "backend"}]),
            "comma-separated components should split and trim into name objects"
        );
    }

    #[test]
    fn insert_custom_field_json_and_string() {
        let mut map = serde_json::Map::new();
        let custom = vec![
            r#"cf_json={"obj":true}"#.to_string(),
            "cf_plain=plain".to_string(),
        ];
        insert_extra_fields(&mut map, &None, &None, &custom).unwrap();

        assert_eq!(
            map.get("cf_json").expect("cf_json missing"),
            &json!({"obj": true}),
            "JSON value should be parsed as a JSON object"
        );
        assert_eq!(
            map.get("cf_plain").expect("cf_plain missing"),
            &json!("plain"),
            "non-JSON value should become a JSON string"
        );
    }

    // ---- convert_input ----

    /// Helper: assert the converted body is the Wiki variant and return the
    /// inner string. Panics with a clear message for ADF results.
    fn assert_wiki(body: JiraBodyContent) -> String {
        match body {
            JiraBodyContent::Wiki(s) => s,
            JiraBodyContent::Adf(adf) => panic!("expected Wiki, got Adf: {adf:?}"),
        }
    }

    #[test]
    fn convert_input_wiki_passthrough() {
        // Wiki-format input must be returned byte-for-byte unchanged so the
        // user's hand-written wiki syntax (which contains characters like `*`
        // and `{` that the markdown converter would interpret) reaches Jira
        // as-is.
        let body = "h1. Hello\n\n*already bold*".to_string();
        let result = assert_wiki(convert_input(body.clone(), &JiraInputFormat::Wiki).unwrap());
        assert_eq!(
            result, body,
            "Wiki input must pass through unchanged, got: {result:?}"
        );
    }

    #[test]
    fn convert_input_markdown_converts_heading() {
        // Markdown input must run through the converter — the cheapest signal
        // that conversion happened is the presence of the wiki heading token.
        let result =
            assert_wiki(convert_input("# Hi".to_string(), &JiraInputFormat::Markdown).unwrap());
        assert!(
            result.contains("h1. Hi"),
            "expected wiki heading `h1. Hi` after markdown conversion, got: {result:?}"
        );
        assert!(
            !result.starts_with("# "),
            "markdown heading prefix must be replaced, got: {result:?}"
        );
    }

    #[test]
    fn convert_input_markdown_converts_bold() {
        // Locks in that bold conversion runs (`**x**` → `*x*`) when the input
        // format is Markdown. The wiki path would leave `**x**` literally.
        let result =
            assert_wiki(convert_input("**x**".to_string(), &JiraInputFormat::Markdown).unwrap());
        assert!(
            result.contains("*x*") && !result.contains("**x**"),
            "expected `**x**` to convert to `*x*`, got: {result:?}"
        );
    }

    #[test]
    fn convert_input_empty_body_does_not_panic() {
        // Edge case: empty body is legal (e.g. user passes `--description ""`).
        // Must not panic on either path.
        let wiki = assert_wiki(convert_input(String::new(), &JiraInputFormat::Wiki).unwrap());
        assert_eq!(wiki, "", "empty wiki body should pass through");

        let md = assert_wiki(convert_input(String::new(), &JiraInputFormat::Markdown).unwrap());
        // Markdown converter may emit a trailing newline for an empty doc;
        // accept either to keep the test resilient to converter trims.
        assert!(
            md.is_empty() || md == "\n",
            "empty markdown body should produce empty or single newline, got: {md:?}"
        );
    }

    #[test]
    fn convert_input_adf_valid_json_yields_adf_variant() {
        // Valid ADF JSON must be parsed and wrapped in JiraBodyContent::Adf so
        // the caller knows to route through v3.
        let adf = json!({"type": "doc", "version": 1, "content": []});
        let result = convert_input(adf.to_string(), &JiraInputFormat::Adf).unwrap();
        match result {
            JiraBodyContent::Adf(v) => {
                assert_eq!(v["type"], "doc");
                assert_eq!(v["version"], 1);
            }
            JiraBodyContent::Wiki(_) => panic!("expected Adf variant, got Wiki"),
        }
    }

    #[test]
    fn convert_input_adf_invalid_json_returns_error() {
        let err = convert_input("not json".to_string(), &JiraInputFormat::Adf).unwrap_err();
        assert!(
            err.to_string().contains("ADF input is not valid JSON"),
            "expected ADF JSON parse error, got: {err}"
        );
    }

    // ---- assert_adf_supported ----

    #[test]
    fn assert_adf_supported_wiki_on_data_center_ok() {
        // Wiki input must work on every flavor — this guard only fires for
        // the ADF variant.
        let body = JiraBodyContent::Wiki("text".into());
        assert!(assert_adf_supported(JiraFlavor::DataCenter, &body).is_ok());
    }

    #[test]
    fn assert_adf_supported_adf_on_cloud_ok() {
        let body = JiraBodyContent::Adf(json!({"type": "doc"}));
        assert!(assert_adf_supported(JiraFlavor::Cloud, &body).is_ok());
    }

    #[test]
    fn assert_adf_supported_adf_on_data_center_errors_with_config() {
        let body = JiraBodyContent::Adf(json!({"type": "doc"}));
        let err = assert_adf_supported(JiraFlavor::DataCenter, &body).unwrap_err();
        // Must downcast to Error::Config so the exit code maps to 3.
        let downcast = err.downcast_ref::<crate::error::Error>();
        assert!(
            matches!(downcast, Some(crate::error::Error::Config(_))),
            "ADF on Data Center must return Error::Config, got: {downcast:?}"
        );
        assert!(
            err.to_string().contains("ADF input is not supported"),
            "error must mention ADF unsupported, got: {err}"
        );
    }

    // ---- api_version_for / read_api_version_for ----

    #[test]
    fn api_version_for_wiki_is_v2() {
        assert_eq!(
            api_version_for(&JiraBodyContent::Wiki("x".into())),
            JiraApiVersion::V2
        );
    }

    #[test]
    fn api_version_for_adf_is_v3() {
        assert_eq!(
            api_version_for(&JiraBodyContent::Adf(json!({"type": "doc"}))),
            JiraApiVersion::V3
        );
    }

    #[test]
    fn read_api_version_wiki_and_markdown_use_v2() {
        assert_eq!(
            read_api_version_for(JiraBodyFormat::Wiki),
            JiraApiVersion::V2
        );
        assert_eq!(
            read_api_version_for(JiraBodyFormat::Markdown),
            JiraApiVersion::V2
        );
    }

    #[test]
    fn read_api_version_adf_uses_v3() {
        assert_eq!(
            read_api_version_for(JiraBodyFormat::Adf),
            JiraApiVersion::V3
        );
    }

    // ---- body_field_value ----

    #[test]
    fn body_field_value_wiki_becomes_string() {
        let v = body_field_value(JiraBodyContent::Wiki("hello".into()));
        assert_eq!(v, json!("hello"));
    }

    #[test]
    fn body_field_value_adf_passes_object_through() {
        let adf = json!({"type": "doc", "version": 1});
        let v = body_field_value(JiraBodyContent::Adf(adf.clone()));
        assert_eq!(v, adf);
    }

    // ---- convert_body_field ----

    #[test]
    fn convert_body_field_wiki_string_becomes_markdown() {
        let v = json!("h1. Hi");
        let out = convert_body_field(&v, true).unwrap();
        let s = out.as_str().expect("markdown should be a string");
        assert!(
            s.contains("# Hi"),
            "wiki h1 must convert to md heading, got: {s:?}"
        );
    }

    #[test]
    fn convert_body_field_adf_doc_becomes_markdown() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "hello"}]}
            ]
        });
        let out = convert_body_field(&adf, true).unwrap();
        let s = out.as_str().expect("markdown should be a string");
        assert!(s.contains("hello"), "ADF doc must convert; got: {s:?}");
    }

    #[test]
    fn convert_body_field_null_passes_through() {
        let v = Value::Null;
        let out = convert_body_field(&v, true).unwrap();
        assert!(
            out.is_null(),
            "null body must remain null after conversion, got: {out:?}"
        );
    }

    #[test]
    fn convert_body_field_unknown_object_passes_through() {
        // An object that is not an ADF doc (no `type: "doc"`) should pass
        // through verbatim so we don't silently corrupt unexpected shapes.
        let v = json!({"foo": "bar"});
        let out = convert_body_field(&v, true).unwrap();
        assert_eq!(out, v);
    }

    // ---- convert_issue_bodies ----

    #[test]
    fn convert_issue_bodies_wiki_format_is_noop() {
        // Wiki / Adf both pass through untouched; only Markdown rewrites.
        let mut issue = json!({
            "fields": {
                "description": "h1. Hi",
                "comment": {"comments": [{"body": "h2. there"}]}
            }
        });
        let before = issue.clone();
        convert_issue_bodies(&mut issue, JiraBodyFormat::Wiki, true).unwrap();
        assert_eq!(issue, before, "Wiki body_format must be a no-op");
    }

    #[test]
    fn convert_issue_bodies_markdown_walks_description_and_comments() {
        let mut issue = json!({
            "fields": {
                "description": "h1. Title",
                "comment": {
                    "comments": [
                        {"id": "1", "body": "h2. Sub"},
                        {"id": "2", "body": "plain"}
                    ]
                }
            }
        });
        convert_issue_bodies(&mut issue, JiraBodyFormat::Markdown, true).unwrap();
        let desc = issue
            .pointer("/fields/description")
            .and_then(Value::as_str)
            .unwrap();
        assert!(desc.contains("# Title"), "description must convert");
        let c0 = issue
            .pointer("/fields/comment/comments/0/body")
            .and_then(Value::as_str)
            .unwrap();
        assert!(c0.contains("## Sub"), "first comment must convert");
        let c1 = issue
            .pointer("/fields/comment/comments/1/body")
            .and_then(Value::as_str)
            .unwrap();
        assert!(c1.contains("plain"), "second comment must convert");
    }

    #[test]
    fn convert_comments_bodies_markdown_walks_array() {
        let mut value = json!({
            "comments": [
                {"id": "1", "body": "h1. one"},
                {"id": "2", "body": "h1. two"}
            ]
        });
        convert_comments_bodies(&mut value, JiraBodyFormat::Markdown, true).unwrap();
        let b0 = value
            .pointer("/comments/0/body")
            .and_then(Value::as_str)
            .unwrap();
        let b1 = value
            .pointer("/comments/1/body")
            .and_then(Value::as_str)
            .unwrap();
        assert!(b0.contains("# one"), "first comment converts: {b0:?}");
        assert!(b1.contains("# two"), "second comment converts: {b1:?}");
    }

    #[test]
    fn convert_comment_body_markdown_rewrites_body() {
        let mut value = json!({"id": "1", "body": "h1. solo"});
        convert_comment_body(&mut value, JiraBodyFormat::Markdown, true).unwrap();
        let s = value.get("body").and_then(Value::as_str).unwrap();
        assert!(s.contains("# solo"), "body must convert, got: {s:?}");
    }

    #[test]
    fn convert_comment_body_wiki_format_is_noop() {
        let before = json!({"id": "1", "body": "h1. solo"});
        let mut value = before.clone();
        convert_comment_body(&mut value, JiraBodyFormat::Wiki, true).unwrap();
        assert_eq!(value, before);
    }

    // ---- build_jql: filter flag coverage ----

    #[test]
    fn build_jql_assignee_current_user_unquoted() {
        let mut args = default_search_args();
        args.assignee = Some("currentUser()".to_string());
        let result = build_jql(&args).unwrap();
        // currentUser() is a JQL function — must not be wrapped in quotes
        // or the server treats it as a literal user name.
        assert_eq!(result, "assignee = currentUser()");
    }

    #[test]
    fn build_jql_assignee_account_id_quoted() {
        let mut args = default_search_args();
        args.assignee = Some("712020:abc".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "assignee = \"712020:abc\"");
    }

    #[test]
    fn build_jql_reporter_current_user_unquoted() {
        let mut args = default_search_args();
        args.reporter = Some("currentUser()".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "reporter = currentUser()");
    }

    #[test]
    fn build_jql_reporter_account_id_quoted() {
        let mut args = default_search_args();
        args.reporter = Some("alice".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "reporter = \"alice\"");
    }

    #[test]
    fn build_jql_priority_filter() {
        let mut args = default_search_args();
        args.priority = Some("High".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "priority = \"High\"");
    }

    #[test]
    fn build_jql_type_filter() {
        let mut args = default_search_args();
        args.r#type = Some("Bug".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "type = \"Bug\"");
    }

    #[test]
    fn build_jql_label_component_resolution_filters() {
        let mut args = default_search_args();
        args.label = Some("hot".to_string());
        args.component = Some("backend".to_string());
        args.resolution = Some("Done".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(
            result,
            "labels = \"hot\" AND component = \"backend\" AND resolution = \"Done\""
        );
    }

    #[test]
    fn build_jql_date_filters_use_correct_operators() {
        let mut args = default_search_args();
        args.created = Some("2025-01-01".to_string());
        args.created_after = Some("2025-01-02".to_string());
        args.updated = Some("2025-01-03".to_string());
        args.updated_after = Some("2025-01-04".to_string());
        let result = build_jql(&args).unwrap();
        // `created`/`updated` are inclusive (>=), `*_after` are exclusive (>)
        assert!(
            result.contains("created >= \"2025-01-01\""),
            "missing inclusive created clause, got: {result}"
        );
        assert!(
            result.contains("created > \"2025-01-02\""),
            "missing exclusive created clause, got: {result}"
        );
        assert!(
            result.contains("updated >= \"2025-01-03\""),
            "missing inclusive updated clause, got: {result}"
        );
        assert!(
            result.contains("updated > \"2025-01-04\""),
            "missing exclusive updated clause, got: {result}"
        );
    }

    #[test]
    fn build_jql_watching_flag() {
        let mut args = default_search_args();
        args.watching = true;
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "watcher = currentUser()");
    }

    #[test]
    fn build_jql_order_by_flag_default_ascending() {
        let mut args = default_search_args();
        args.status = Some("Open".to_string());
        args.order_by = Some("created".to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "status = \"Open\" ORDER BY created ASC");
    }

    #[test]
    fn build_jql_order_by_flag_descending_with_reverse() {
        let mut args = default_search_args();
        args.status = Some("Open".to_string());
        args.order_by = Some("created".to_string());
        args.reverse = true;
        let result = build_jql(&args).unwrap();
        assert_eq!(result, "status = \"Open\" ORDER BY created DESC");
    }

    #[test]
    fn build_jql_escapes_user_input_in_quoted_fields() {
        let mut args = default_search_args();
        // Status values can contain backslashes/quotes if a user quotes them.
        args.status = Some(r#"weird"name"#.to_string());
        let result = build_jql(&args).unwrap();
        assert_eq!(
            result, r#"status = "weird\"name""#,
            "status value must be escaped before interpolation"
        );
    }

    // ---- cmd_uses_pager ----

    #[test]
    fn cmd_uses_pager_view_qualifies() {
        let cmd = JiraSubcommand::View(JiraViewArgs {
            key: "X-1".into(),
            web: false,
            body_format: JiraBodyFormat::Wiki,
            no_directives: false,
        });
        assert!(cmd_uses_pager(&cmd));
    }

    #[test]
    fn cmd_uses_pager_search_qualifies() {
        let cmd = JiraSubcommand::Search(default_search_args());
        assert!(cmd_uses_pager(&cmd));
    }

    #[test]
    fn cmd_uses_pager_short_output_does_not_qualify() {
        // Sanity: a mutating command should never engage the pager so error
        // messages stay inline.
        let cmd = JiraSubcommand::Move(JiraMoveArgs {
            key: "X-1".into(),
            transition: "31".into(),
        });
        assert!(!cmd_uses_pager(&cmd));
    }

    // ---- today_date ----

    #[test]
    fn today_date_has_iso_shape() {
        let s = today_date();
        // Must be exactly YYYY-MM-DD (10 chars, dashes at index 4 and 7).
        assert_eq!(s.len(), 10, "expected 10-char date string, got {s:?}");
        assert_eq!(s.as_bytes()[4], b'-', "dash missing at index 4: {s:?}");
        assert_eq!(s.as_bytes()[7], b'-', "dash missing at index 7: {s:?}");
        // All other positions must be ASCII digits.
        for (i, b) in s.bytes().enumerate() {
            if i == 4 || i == 7 {
                continue;
            }
            assert!(b.is_ascii_digit(), "non-digit at index {i}: {s:?}");
        }
    }

    // ---- flatten_issue (single) ----

    #[test]
    fn flatten_issue_extracts_all_known_fields() {
        let input = json!({
            "key": "ORB-9",
            "fields": {
                "summary": "Do thing",
                "status": { "name": "In Progress" },
                "priority": { "name": "Medium" },
                "issuetype": { "name": "Story" },
                "project": { "key": "ORB" },
                "assignee": { "displayName": "Alice" },
                "reporter": { "displayName": "Bob" },
                "created": "2025-01-01T00:00:00Z",
                "updated": "2025-01-02T00:00:00Z",
                "description": "hello",
                "resolution": { "name": "Fixed" },
                "labels": ["a", "b"],
                "components": [{"name": "be"}, {"name": "fe"}]
            }
        });
        let out = flatten_issue(input);
        let obj = out.as_object().expect("object");
        assert_eq!(obj.get("key").and_then(Value::as_str), Some("ORB-9"));
        assert_eq!(obj.get("summary").and_then(Value::as_str), Some("Do thing"));
        assert_eq!(obj.get("type").and_then(Value::as_str), Some("Story"));
        assert_eq!(
            obj.get("status").and_then(Value::as_str),
            Some("In Progress")
        );
        assert_eq!(obj.get("priority").and_then(Value::as_str), Some("Medium"));
        assert_eq!(obj.get("assignee").and_then(Value::as_str), Some("Alice"));
        assert_eq!(obj.get("reporter").and_then(Value::as_str), Some("Bob"));
        assert_eq!(obj.get("project").and_then(Value::as_str), Some("ORB"));
        assert_eq!(obj.get("labels").and_then(Value::as_str), Some("a, b"));
        assert_eq!(
            obj.get("components").and_then(Value::as_str),
            Some("be, fe")
        );
        assert_eq!(obj.get("resolution").and_then(Value::as_str), Some("Fixed"));
        assert_eq!(
            obj.get("created").and_then(Value::as_str),
            Some("2025-01-01T00:00:00Z")
        );
        assert_eq!(
            obj.get("updated").and_then(Value::as_str),
            Some("2025-01-02T00:00:00Z")
        );
        assert_eq!(
            obj.get("description").and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn flatten_issue_omits_empty_optional_fields() {
        // labels/components/resolution/description are conditionally inserted.
        let input = json!({
            "key": "X-1",
            "fields": {
                "summary": "s",
                "status": { "name": "Open" },
                "priority": { "name": "Low" },
                "issuetype": { "name": "Task" },
                "project": { "key": "X" },
                "assignee": null,
                "reporter": { "displayName": "R" },
                "created": "2025-01-01T00:00:00Z",
                "updated": "2025-01-01T00:00:00Z"
            }
        });
        let out = flatten_issue(input);
        let obj = out.as_object().expect("object");
        assert!(obj.get("labels").is_none(), "labels should be omitted");
        assert!(
            obj.get("components").is_none(),
            "components should be omitted"
        );
        assert!(
            obj.get("resolution").is_none(),
            "resolution should be omitted"
        );
        assert!(
            obj.get("description").is_none(),
            "description should be omitted"
        );
        assert_eq!(
            obj.get("assignee").and_then(Value::as_str),
            Some(""),
            "missing assignee should map to empty string"
        );
    }

    // ---- build_create_fields ----

    fn default_create_args() -> JiraCreateArgs {
        JiraCreateArgs {
            project: "PROJ".into(),
            issue_type: "Task".into(),
            summary: "S".into(),
            description: None,
            assignee: None,
            priority: None,
            labels: None,
            parent: None,
            fix_version: None,
            component: None,
            custom_fields: vec![],
            input_format: JiraInputFormat::Wiki,
        }
    }

    #[test]
    fn build_create_fields_minimum_required() {
        let args = default_create_args();
        let v = build_create_fields(&args, None).unwrap();
        assert_eq!(v["project"]["key"], "PROJ");
        assert_eq!(v["issuetype"]["name"], "Task");
        assert_eq!(v["summary"], "S");
        // Optional fields must NOT be present when not provided.
        assert!(v.get("description").is_none(), "description must be absent");
        assert!(v.get("assignee").is_none(), "assignee must be absent");
        assert!(v.get("labels").is_none(), "labels must be absent");
        assert!(v.get("parent").is_none(), "parent must be absent");
    }

    #[test]
    fn build_create_fields_with_description_uses_passed_body() {
        // The caller is responsible for resolving --description (literal/file/stdin)
        // and converting markdown if requested. The builder takes the result.
        let args = default_create_args();
        let v = build_create_fields(&args, Some(json!("the body"))).unwrap();
        assert_eq!(v["description"], "the body");
    }

    #[test]
    fn build_create_fields_with_adf_description_keeps_object_shape() {
        // ADF input passes a JSON object through verbatim — the v3 API
        // expects the description field to be an ADF doc object, not a
        // string.
        let args = default_create_args();
        let adf = json!({"type": "doc", "version": 1, "content": []});
        let v = build_create_fields(&args, Some(adf.clone())).unwrap();
        assert_eq!(v["description"], adf);
    }

    #[test]
    fn build_create_fields_with_full_optionals() {
        let mut args = default_create_args();
        args.assignee = Some("acct123".into());
        args.priority = Some("High".into());
        args.labels = Some("a, b , c".into());
        args.parent = Some("PROJ-1".into());
        let v = build_create_fields(&args, None).unwrap();
        assert_eq!(v["assignee"], json!({"accountId": "acct123"}));
        assert_eq!(v["priority"], json!({"name": "High"}));
        // Comma-separated labels must be split AND trimmed.
        assert_eq!(v["labels"], json!(["a", "b", "c"]));
        assert_eq!(v["parent"], json!({"key": "PROJ-1"}));
    }

    #[test]
    fn build_create_fields_propagates_extra_field_error() {
        let mut args = default_create_args();
        // Custom field without `=` must surface as an error.
        args.custom_fields = vec!["malformed".into()];
        let err = build_create_fields(&args, None).unwrap_err();
        assert!(
            err.to_string().contains("invalid --custom format"),
            "expected --custom format error, got: {err}"
        );
    }

    // ---- build_update_fields ----

    fn default_update_args() -> JiraUpdateArgs {
        JiraUpdateArgs {
            key: "PROJ-1".into(),
            summary: None,
            description: None,
            assignee: None,
            priority: None,
            labels: None,
            fix_version: None,
            component: None,
            custom_fields: vec![],
            input_format: JiraInputFormat::Wiki,
        }
    }

    #[test]
    fn build_update_fields_empty_returns_error() {
        let args = default_update_args();
        let err = build_update_fields(&args, None).unwrap_err();
        assert!(
            err.to_string().contains("no fields to update"),
            "expected helpful 'no fields' message, got: {err}"
        );
    }

    #[test]
    fn build_update_fields_summary_only() {
        let mut args = default_update_args();
        args.summary = Some("new".into());
        let map = build_update_fields(&args, None).unwrap();
        assert_eq!(map.len(), 1, "only summary should be set: {map:?}");
        assert_eq!(map.get("summary").and_then(Value::as_str), Some("new"));
    }

    #[test]
    fn build_update_fields_description_uses_passed_body() {
        let args = default_update_args();
        let map = build_update_fields(&args, Some(json!("body"))).unwrap();
        assert_eq!(map.get("description").and_then(Value::as_str), Some("body"));
    }

    #[test]
    fn build_update_fields_with_adf_description_keeps_object_shape() {
        // ADF on update mirrors create: the description field stays as the
        // ADF doc object so the v3 endpoint accepts it.
        let args = default_update_args();
        let adf = json!({"type": "doc", "version": 1, "content": []});
        let map = build_update_fields(&args, Some(adf.clone())).unwrap();
        assert_eq!(map.get("description").unwrap(), &adf);
    }

    #[test]
    fn build_update_fields_full_optionals() {
        let mut args = default_update_args();
        args.assignee = Some("a".into());
        args.priority = Some("Low".into());
        args.labels = Some("x,y".into());
        let map = build_update_fields(&args, Some(json!("body"))).unwrap();
        assert_eq!(map.get("assignee").unwrap(), &json!({"accountId": "a"}));
        assert_eq!(map.get("priority").unwrap(), &json!({"name": "Low"}));
        assert_eq!(map.get("labels").unwrap(), &json!(["x", "y"]));
    }

    // ---- build_clone_fields ----

    #[test]
    fn build_clone_fields_default_summary_prefix() {
        let source = json!({
            "fields": {
                "project": { "key": "P" },
                "issuetype": { "name": "Bug" },
                "summary": "Original",
                "priority": { "name": "Low" }
            }
        });
        let map = build_clone_fields(&source, None).unwrap();
        // Default summary must prepend `[Clone] ` so the cloned issue is
        // visually distinguishable in lists.
        assert_eq!(
            map.get("summary").and_then(Value::as_str),
            Some("[Clone] Original")
        );
        assert_eq!(map.get("project").unwrap(), &json!({"key": "P"}));
        assert_eq!(map.get("issuetype").unwrap(), &json!({"name": "Bug"}));
        assert_eq!(map.get("priority").unwrap(), &json!({"name": "Low"}));
    }

    #[test]
    fn build_clone_fields_summary_override() {
        let source = json!({"fields": {"summary": "Original", "project": {"key": "P"}}});
        let map = build_clone_fields(&source, Some("Custom")).unwrap();
        assert_eq!(map.get("summary").and_then(Value::as_str), Some("Custom"));
    }

    #[test]
    fn build_clone_fields_skips_null_fields() {
        // Confirms the `&& !val.is_null()` guard — Jira issues frequently
        // include explicit nulls for unset fields and copying them through
        // produces invalid create payloads.
        let source = json!({
            "fields": {
                "project": { "key": "P" },
                "issuetype": { "name": "Task" },
                "summary": "S",
                "priority": null,
                "labels": null
            }
        });
        let map = build_clone_fields(&source, None).unwrap();
        assert!(
            map.get("priority").is_none(),
            "null priority must be dropped"
        );
        assert!(map.get("labels").is_none(), "null labels must be dropped");
    }

    #[test]
    fn build_clone_fields_only_clones_known_keys() {
        // Internal/server-managed fields like `created`, `creator`, `status`
        // must not be carried over — they'd be rejected on create.
        let source = json!({
            "fields": {
                "project": { "key": "P" },
                "issuetype": { "name": "Task" },
                "summary": "S",
                "created": "2025-01-01",
                "creator": { "accountId": "x" },
                "status": { "name": "Done" }
            }
        });
        let map = build_clone_fields(&source, None).unwrap();
        assert!(map.get("created").is_none());
        assert!(map.get("creator").is_none());
        assert!(map.get("status").is_none());
    }

    #[test]
    fn build_clone_fields_missing_summary_uses_placeholder() {
        let source = json!({"fields": {"project": {"key": "P"}}});
        let map = build_clone_fields(&source, None).unwrap();
        assert_eq!(
            map.get("summary").and_then(Value::as_str),
            Some("[Clone] (no summary)")
        );
    }

    #[test]
    fn build_clone_fields_no_fields_returns_error() {
        let source = json!({"key": "X-1"});
        let err = build_clone_fields(&source, None).unwrap_err();
        assert!(
            err.to_string().contains("could not read fields"),
            "expected fields-missing error, got: {err}"
        );
    }

    // ---- build_notify_payload ----

    #[test]
    fn build_notify_payload_no_recipients() {
        let v = build_notify_payload("hi", "body", &[]);
        assert_eq!(v["subject"], "hi");
        assert_eq!(v["textBody"], "body");
        assert!(
            v.get("to").is_none(),
            "to key must be absent when there are no recipients"
        );
    }

    #[test]
    fn build_notify_payload_with_recipients() {
        let v = build_notify_payload("hi", "body", &["a".into(), "b".into()]);
        assert_eq!(
            v["to"],
            json!({"users": [{"accountId": "a"}, {"accountId": "b"}]})
        );
    }

    // ---- parse_bulk_create_payload ----

    #[test]
    fn parse_bulk_create_array_input_wraps_into_envelope() {
        let raw = r#"[{"summary":"a"},{"summary":"b"}]"#;
        let v = parse_bulk_create_payload(raw).unwrap();
        assert_eq!(
            v,
            json!({
                "issueUpdates": [
                    {"fields": {"summary": "a"}},
                    {"fields": {"summary": "b"}}
                ]
            })
        );
    }

    #[test]
    fn parse_bulk_create_envelope_input_passthrough() {
        let raw = r#"{"issueUpdates":[{"fields":{"summary":"a"}}]}"#;
        let v = parse_bulk_create_payload(raw).unwrap();
        let parsed: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(v, parsed, "envelope input must pass through unchanged");
    }

    #[test]
    fn parse_bulk_create_invalid_json_errors() {
        let err = parse_bulk_create_payload("not json").unwrap_err();
        assert!(
            err.to_string().contains("invalid JSON input"),
            "expected JSON parse error, got: {err}"
        );
    }

    #[test]
    fn parse_bulk_create_unknown_object_shape_errors() {
        // Object that is neither the envelope nor an array must surface an
        // error so the user knows their input shape is wrong.
        let err = parse_bulk_create_payload(r#"{"foo": []}"#).unwrap_err();
        assert!(
            err.to_string().contains("expected a JSON array"),
            "expected shape error, got: {err}"
        );
    }

    // ---- run() error paths ----
    //
    // The `run` entry point loads config, resolves the profile, and pulls the
    // Jira instance before constructing any HTTP client. Each of those steps
    // can fail with a distinct user-facing message — we cover all three pre-HTTP
    // branches here so the messages stay stable and the exit-code mapping in
    // `error.rs` continues to match `Error::Config`.

    use crate::output::{OutputFormat, Transforms};
    use camino::Utf8PathBuf;
    use std::io::Write as _;

    fn write_config(dir: &tempfile::TempDir, body: &str) -> Utf8PathBuf {
        let path = dir.path().join("atl.toml");
        let mut f = std::fs::File::create(&path).expect("create config file");
        f.write_all(body.as_bytes()).expect("write config body");
        Utf8PathBuf::try_from(path).expect("UTF-8 temp path")
    }

    #[tokio::test]
    async fn run_errors_when_config_path_does_not_exist() {
        // Explicit config path that doesn't resolve to a file is a hard
        // error: it tells the user their --config flag is wrong rather than
        // silently falling back to defaults.
        let mut io = IoStreams::test();
        let cmd = JiraSubcommand::Search(default_search_args());
        let bogus = Utf8PathBuf::from("/definitely/does/not/exist/atl.toml");
        let err = run(
            &cmd,
            Some(&bogus),
            None,
            RetryConfig::off(),
            &OutputFormat::Json,
            &mut io,
            &Transforms::none(),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("config file not found"),
            "expected 'config file not found' error, got: {err}"
        );
    }

    #[tokio::test]
    async fn run_errors_when_no_profile_in_config() {
        // Config exists but is empty — no profiles at all. Must be rejected
        // with the actionable "run `atl init` first" hint.
        let dir = tempfile::tempdir().expect("create tempdir");
        let cfg = write_config(&dir, "default_profile = \"work\"\n");
        let mut io = IoStreams::test();
        let cmd = JiraSubcommand::Search(default_search_args());
        let err = run(
            &cmd,
            Some(&cfg),
            None,
            RetryConfig::off(),
            &OutputFormat::Json,
            &mut io,
            &Transforms::none(),
        )
        .await
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no profile found"),
            "expected 'no profile found' message, got: {msg}"
        );
        // The hint must mention `atl init` so the user knows the recovery path.
        assert!(
            msg.contains("atl init"),
            "error must mention `atl init` recovery path, got: {msg}"
        );
    }

    #[tokio::test]
    async fn run_view_web_short_circuits_to_browse() {
        // `atl jira view PROJ-1 --web` must NOT touch the Jira REST API; it
        // hands off to `browse::run` which constructs the URL from the
        // configured domain. In test mode (non-TTY) browse prints the URL to
        // stdout instead of launching a browser, so we can assert on it.
        let dir = tempfile::tempdir().expect("create tempdir");
        let cfg = write_config(
            &dir,
            r#"default_profile = "work"

[profiles.work.jira]
domain = "example.atlassian.net"
"#,
        );
        let mut io = IoStreams::test();
        let cmd = JiraSubcommand::View(JiraViewArgs {
            key: "PROJ-1".into(),
            web: true,
            body_format: JiraBodyFormat::Wiki,
            no_directives: false,
        });
        run(
            &cmd,
            Some(&cfg),
            None,
            RetryConfig::off(),
            &OutputFormat::Console,
            &mut io,
            &Transforms::none(),
        )
        .await
        .expect("--web path must succeed without HTTP");
        let out = io.stdout_as_string();
        assert!(
            out.contains("example.atlassian.net"),
            "expected the configured domain in the printed URL, got: {out:?}"
        );
        assert!(
            out.contains("PROJ-1"),
            "expected the issue key in the printed URL, got: {out:?}"
        );
    }

    #[tokio::test]
    async fn run_errors_when_profile_has_no_jira_instance() {
        // Profile exists but has only a Confluence instance — calling jira
        // commands on a Confluence-only profile must return a typed error so
        // the exit code maps to Config (3), not the generic 1.
        let dir = tempfile::tempdir().expect("create tempdir");
        let cfg = write_config(
            &dir,
            r#"default_profile = "work"

[profiles.work.confluence]
domain = "x.atlassian.net"
"#,
        );
        let mut io = IoStreams::test();
        let cmd = JiraSubcommand::Search(default_search_args());
        let err = run(
            &cmd,
            Some(&cfg),
            None,
            RetryConfig::off(),
            &OutputFormat::Json,
            &mut io,
            &Transforms::none(),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("no Jira instance configured"),
            "expected 'no Jira instance configured', got: {err}"
        );
        // The error must downcast to Error::Config so the exit code lookup
        // returns 3 instead of falling through to the generic 1.
        let downcast = err.downcast_ref::<crate::error::Error>();
        assert!(
            matches!(downcast, Some(crate::error::Error::Config(_))),
            "error must be Error::Config so exit code maps to 3, got: {downcast:?}"
        );
    }
}
