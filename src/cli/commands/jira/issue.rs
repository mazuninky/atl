//! `atl jira issue …` — nested issue subcommands.
//!
//! Currently only hosts `check`. The flat `view`/`create`/`update`/… surface
//! lives directly under [`crate::cli::args::JiraSubcommand`] for backwards
//! compatibility; new sibling commands should land here so we can grow the
//! subtree without disturbing the existing flat layout.

use std::io::Write;

use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;
use crate::error::Error;
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

/// Curated default warn-list, applied when the user passes neither `--require`
/// nor `--warn`. These are the fields most teams expect to see populated on a
/// well-formed Jira issue.
const DEFAULT_WARN_FIELDS: &[&str] = &[
    "Summary",
    "Description",
    "Assignee",
    "Priority",
    "Labels",
    "Story Points",
    "Sprint",
    "Components",
    "Fix Version/s",
];

/// Flatten a comma-separated list inside each user-supplied flag value.
///
/// Clap already splits on commas via `value_delimiter = ','`, but defensively
/// trim each token and drop empties so trailing commas / extra spaces don't
/// produce phantom field names.
fn split_field_list(items: &[String]) -> Vec<String> {
    items
        .iter()
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Reference to a Jira field as resolved against the project's field schema.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldRef {
    /// Stable Jira field ID (`summary`, `customfield_10035`).
    id: String,
    /// Human-readable display name (`Summary`, `Story Points`).
    display_name: String,
}

/// Resolve a user-supplied field name against the full list of fields.
///
/// Tries, in order:
///
/// 1. Exact match against the field ID (`customfield_10035`, `summary`).
/// 2. Case-insensitive match against the field display name (`Story Points`).
///
/// Returns `None` if neither matches — the caller maps unresolved fields to
/// `SKIPPED` regardless of level. Unknown fields therefore never fail the
/// check, which keeps the command stable across projects whose field schemas
/// differ.
fn resolve_field(input: &str, all_fields: &[Value]) -> Option<FieldRef> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 1. Exact ID match.
    for f in all_fields {
        let id = f.get("id").and_then(Value::as_str).unwrap_or("");
        if id == trimmed {
            let name = f
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_string();
            return Some(FieldRef {
                id: id.to_string(),
                display_name: name,
            });
        }
    }
    // 2. Case-insensitive name match.
    let lower = trimmed.to_ascii_lowercase();
    for f in all_fields {
        let name = f.get("name").and_then(Value::as_str).unwrap_or("");
        if name.to_ascii_lowercase() == lower {
            let id = f
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            return Some(FieldRef {
                id,
                display_name: name.to_string(),
            });
        }
    }
    None
}

/// Returns true when `value` represents a "missing" field on a Jira issue.
///
/// Treats `null`, `""`, `[]`, `{}` as MISSING. Numeric `0`, `false`, and
/// non-empty objects/arrays/strings are OK.
fn is_field_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => m.is_empty(),
        // Number / Bool always count as set.
        Value::Number(_) | Value::Bool(_) => false,
    }
}

/// Status for a single field in the per-field check report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckStatus {
    Ok,
    Missing,
    Skipped,
}

impl CheckStatus {
    fn as_str(self) -> &'static str {
        match self {
            CheckStatus::Ok => "OK",
            CheckStatus::Missing => "MISSING",
            CheckStatus::Skipped => "SKIPPED",
        }
    }
}

/// Severity level for a field in the check report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckLevel {
    Require,
    Warn,
}

impl CheckLevel {
    fn as_str(self) -> &'static str {
        match self {
            CheckLevel::Require => "REQUIRE",
            CheckLevel::Warn => "WARN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckRow {
    field: String,
    id: String,
    level: CheckLevel,
    status: CheckStatus,
}

impl CheckRow {
    fn to_value(&self) -> Value {
        json!({
            "field": &self.field,
            "id": &self.id,
            "level": self.level.as_str(),
            "status": self.status.as_str(),
        })
    }
}

/// Build the per-field report (pure).
///
/// `issue_fields` is the `fields` object from `GET /rest/api/3/issue/{key}`;
/// `all_fields` is the flat array from `GET /rest/api/3/field`.
fn build_report(
    require: &[String],
    warn: &[String],
    issue_fields: &Value,
    all_fields: &[Value],
) -> Vec<CheckRow> {
    let mut rows = Vec::with_capacity(require.len() + warn.len());

    // Track field IDs already covered by REQUIRE so a WARN with the same
    // identifier doesn't produce a duplicate row.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for input in require {
        let row = check_one(input, CheckLevel::Require, issue_fields, all_fields);
        seen.insert(row.id.clone());
        rows.push(row);
    }
    for input in warn {
        let row = check_one(input, CheckLevel::Warn, issue_fields, all_fields);
        if seen.insert(row.id.clone()) {
            rows.push(row);
        }
    }
    rows
}

fn check_one(
    input: &str,
    level: CheckLevel,
    issue_fields: &Value,
    all_fields: &[Value],
) -> CheckRow {
    match resolve_field(input, all_fields) {
        Some(FieldRef { id, display_name }) => {
            // Missing key → SKIPPED (the field isn't on this issue's
            // project/type schema). Explicit `null` / `""` / `[]` / `{}` →
            // MISSING (Jira returned the field but it's unset). Anything
            // else → OK.
            let status = match issue_fields.get(&id) {
                None => CheckStatus::Skipped,
                Some(v) if is_field_empty(v) => CheckStatus::Missing,
                Some(_) => CheckStatus::Ok,
            };
            CheckRow {
                field: display_name,
                id,
                level,
                status,
            }
        }
        None => CheckRow {
            field: input.to_string(),
            // Use the user-supplied label as the id for unknown fields so the
            // JSON report still has a stable identifier.
            id: input.to_string(),
            level,
            status: CheckStatus::Skipped,
        },
    }
}

pub(super) async fn dispatch_issue(
    cmd: &JiraIssueSubcommand,
    client: &JiraClient,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    match cmd {
        JiraIssueSubcommand::Check(args) => check(args, client, format, io, transforms).await,
    }
}

async fn check(
    args: &JiraCheckArgs,
    client: &JiraClient,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let require = split_field_list(&args.require);
    let mut warn = split_field_list(&args.warn);

    // Apply the curated default warn-list only when the user gave neither
    // flag — `--require X` alone narrows the check to X.
    if require.is_empty() && warn.is_empty() {
        warn = DEFAULT_WARN_FIELDS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
    }

    let fields_value = client.list_fields().await?;
    let all_fields: Vec<Value> = fields_value.as_array().cloned().unwrap_or_default();

    let issue = client
        .get_issue(&args.key, &[], crate::client::JiraApiVersion::V2)
        .await?;
    let issue_fields = issue.get("fields").cloned().unwrap_or(Value::Null);

    let rows = build_report(&require, &warn, &issue_fields, &all_fields);

    let json_rows: Vec<Value> = rows.iter().map(CheckRow::to_value).collect();
    let value = Value::Array(json_rows);

    // Emit the JSON report first so CI scripts get the structured output even
    // when the command exits non-zero.
    write_output(value, format, io, transforms)?;

    let missing_required: Vec<String> = rows
        .iter()
        .filter(|r| r.level == CheckLevel::Require && r.status == CheckStatus::Missing)
        .map(|r| r.id.clone())
        .collect();

    if !missing_required.is_empty() {
        let _ = writeln!(
            io.stderr(),
            "missing required fields: {}",
            missing_required.join(", ")
        );
        return Err(Error::CheckFailed(missing_required).into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn fields_fixture() -> Vec<Value> {
        vec![
            json!({"id": "summary", "name": "Summary"}),
            json!({"id": "description", "name": "Description"}),
            json!({"id": "assignee", "name": "Assignee"}),
            json!({"id": "customfield_10035", "name": "Story Points"}),
            json!({"id": "customfield_10020", "name": "Sprint"}),
        ]
    }

    // -- resolve_field --

    #[test]
    fn resolve_field_matches_id_exactly() {
        let f = fields_fixture();
        let r = resolve_field("customfield_10035", &f).unwrap();
        assert_eq!(r.id, "customfield_10035");
        assert_eq!(r.display_name, "Story Points");
    }

    #[test]
    fn resolve_field_matches_short_id_exactly() {
        // Built-in field ID — covers the non-customfield branch.
        let f = fields_fixture();
        let r = resolve_field("summary", &f).unwrap();
        assert_eq!(r.id, "summary");
        assert_eq!(r.display_name, "Summary");
    }

    #[test]
    fn resolve_field_matches_name_case_insensitively() {
        let f = fields_fixture();
        let r = resolve_field("story points", &f).unwrap();
        assert_eq!(r.id, "customfield_10035");
        assert_eq!(r.display_name, "Story Points");

        // Mixed case also works.
        let r = resolve_field("Story Points", &f).unwrap();
        assert_eq!(r.id, "customfield_10035");

        let r = resolve_field("STORY POINTS", &f).unwrap();
        assert_eq!(r.id, "customfield_10035");
    }

    #[test]
    fn resolve_field_trims_surrounding_whitespace() {
        let f = fields_fixture();
        let r = resolve_field("  summary  ", &f).unwrap();
        assert_eq!(r.id, "summary");

        let r = resolve_field("  Story Points  ", &f).unwrap();
        assert_eq!(r.id, "customfield_10035");
    }

    #[test]
    fn resolve_field_empty_input_returns_none() {
        let f = fields_fixture();
        assert!(resolve_field("", &f).is_none());
    }

    #[test]
    fn resolve_field_whitespace_only_input_returns_none() {
        let f = fields_fixture();
        assert!(resolve_field("   ", &f).is_none());
        assert!(resolve_field("\t\n", &f).is_none());
    }

    #[test]
    fn resolve_field_unknown_returns_none() {
        let f = fields_fixture();
        assert!(resolve_field("Magic Field", &f).is_none());
        assert!(resolve_field("customfield_99999", &f).is_none());
    }

    #[test]
    fn resolve_field_prefers_id_match_over_name_match() {
        // Construct a deliberately tricky schema where field A's *name* equals
        // field B's *id*. The ID-match pass must run first so the input
        // resolves to B (the field whose id literally matches), not to A.
        let fields = vec![
            json!({"id": "alpha", "name": "beta"}),
            json!({"id": "beta", "name": "Sometimes Called Alpha"}),
        ];
        let r = resolve_field("beta", &fields).unwrap();
        assert_eq!(
            r.id, "beta",
            "ID-match must win: input 'beta' should resolve to the field \
             whose id is 'beta', not to the field whose name is 'beta'"
        );
        assert_eq!(r.display_name, "Sometimes Called Alpha");
    }

    // -- is_field_empty --

    #[test]
    fn is_field_empty_treats_null_string_array_object_as_missing() {
        assert!(is_field_empty(&Value::Null));
        assert!(is_field_empty(&json!("")));
        assert!(is_field_empty(&json!([])));
        assert!(is_field_empty(&json!({})));
    }

    #[test]
    fn is_field_empty_treats_populated_values_as_ok() {
        assert!(!is_field_empty(&json!("hi")));
        assert!(!is_field_empty(&json!([1])));
        assert!(!is_field_empty(&json!({"k": "v"})));
    }

    #[test]
    fn is_field_empty_treats_zero_and_false_as_set() {
        // Numeric 0 and `false` are valid values, not "missing". Treating
        // them as missing would break Story Points = 0 and any boolean field.
        assert!(!is_field_empty(&json!(0)));
        assert!(!is_field_empty(&json!(0.0)));
        assert!(!is_field_empty(&json!(-1)));
        assert!(!is_field_empty(&json!(false)));
        assert!(!is_field_empty(&json!(true)));
    }

    // -- split_field_list --

    #[test]
    fn split_field_list_flattens_comma_separated_tokens() {
        let v = vec!["a,b".to_string(), "c".to_string()];
        let out = split_field_list(&v);
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn split_field_list_drops_empties_and_trims() {
        let v = vec![" ,a, ,b,".to_string()];
        let out = split_field_list(&v);
        assert_eq!(out, vec!["a", "b"]);
    }

    #[test]
    fn split_field_list_preserves_internal_whitespace_in_names() {
        // "Story Points" contains a space — the splitter must split only on
        // commas, not on whitespace, so multi-word display names survive.
        let v = vec!["Story Points,Sprint".to_string()];
        let out = split_field_list(&v);
        assert_eq!(out, vec!["Story Points", "Sprint"]);
    }

    #[test]
    fn split_field_list_combines_clap_split_and_internal_split() {
        // Clap's value_delimiter=',' already splits on commas, but a user
        // might pass `--require "a, b" --require c` and we have to flatten
        // both layers. Trailing commas / empty whitespace tokens are dropped.
        let v = vec!["a, b".to_string(), "  ".to_string(), "c".to_string()];
        let out = split_field_list(&v);
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    // -- check_one --

    #[test]
    fn check_one_unknown_field_uses_input_as_id() {
        // When the input doesn't resolve, the row still needs a stable id so
        // JSON consumers can reference the typo. The id should equal the
        // user-supplied label, not the empty string.
        let f = fields_fixture();
        let issue = json!({});
        let row = check_one("Made-Up Field", CheckLevel::Require, &issue, &f);
        assert_eq!(row.id, "Made-Up Field");
        assert_eq!(row.field, "Made-Up Field");
        assert_eq!(row.level, CheckLevel::Require);
        assert_eq!(row.status, CheckStatus::Skipped);
    }

    #[test]
    fn check_one_unknown_field_preserves_warn_level() {
        let f = fields_fixture();
        let issue = json!({});
        let row = check_one("Made-Up Field", CheckLevel::Warn, &issue, &f);
        assert_eq!(row.level, CheckLevel::Warn);
        assert_eq!(row.status, CheckStatus::Skipped);
    }

    #[test]
    fn check_one_known_but_absent_field_is_skipped() {
        let f = fields_fixture();
        // `summary` is in the schema, but the issue's `fields` object simply
        // lacks the key. That's SKIPPED (not on this issue's project), not
        // MISSING (present-but-empty).
        let issue = json!({});
        let row = check_one("summary", CheckLevel::Require, &issue, &f);
        assert_eq!(row.id, "summary");
        assert_eq!(row.status, CheckStatus::Skipped);
    }

    #[test]
    fn check_one_explicit_null_is_missing() {
        let f = fields_fixture();
        let issue = json!({"summary": null});
        let row = check_one("summary", CheckLevel::Require, &issue, &f);
        assert_eq!(row.status, CheckStatus::Missing);
    }

    #[test]
    fn check_one_populated_field_is_ok() {
        let f = fields_fixture();
        let issue = json!({"summary": "Hello"});
        let row = check_one("summary", CheckLevel::Require, &issue, &f);
        assert_eq!(row.status, CheckStatus::Ok);
    }

    // -- build_report --

    #[test]
    fn build_report_marks_missing_require_and_ok_warn() {
        let f = fields_fixture();
        // `customfield_10035` is `null` (cleared by Jira) → MISSING.
        // `description` is the empty string → MISSING.
        // `summary` is populated → OK.
        let issue = json!({"summary": "hi", "description": "", "customfield_10035": null});
        let rows = build_report(
            &["Story Points".into()],
            &["Summary".into(), "Description".into()],
            &issue,
            &f,
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "customfield_10035");
        assert_eq!(rows[0].level, CheckLevel::Require);
        assert_eq!(rows[0].status, CheckStatus::Missing);
        assert_eq!(rows[1].id, "summary");
        assert_eq!(rows[1].level, CheckLevel::Warn);
        assert_eq!(rows[1].status, CheckStatus::Ok);
        assert_eq!(rows[2].id, "description");
        assert_eq!(rows[2].level, CheckLevel::Warn);
        assert_eq!(rows[2].status, CheckStatus::Missing);
    }

    #[test]
    fn build_report_require_only_omits_default_warn_list() {
        // Caller passes only REQUIRE — the default warn-list lives in the
        // command handler, not in `build_report`. The pure helper should
        // produce exactly one row: the requested REQUIRE field.
        let f = fields_fixture();
        let issue = json!({"summary": "hi"});
        let rows = build_report(&["Summary".into()], &[], &issue, &f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].level, CheckLevel::Require);
        assert_eq!(rows[0].status, CheckStatus::Ok);
    }

    #[test]
    fn build_report_unknown_field_is_skipped() {
        let f = fields_fixture();
        let issue = json!({});
        let rows = build_report(&[], &["Made-Up Field".into()], &issue, &f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, CheckStatus::Skipped);
        assert_eq!(rows[0].field, "Made-Up Field");
    }

    #[test]
    fn build_report_missing_key_is_skipped_not_missing() {
        let f = fields_fixture();
        // Sprint customfield is on the global schema but not on this issue's
        // project — Jira simply omits it from the response.
        let issue = json!({"summary": "hi"});
        let rows = build_report(&[], &["Sprint".into()], &issue, &f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "customfield_10020");
        assert_eq!(rows[0].status, CheckStatus::Skipped);
    }

    #[test]
    fn build_report_explicit_null_value_is_missing() {
        let f = fields_fixture();
        let issue = json!({"customfield_10035": null});
        let rows = build_report(&["Story Points".into()], &[], &issue, &f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, CheckStatus::Missing);
    }

    #[test]
    fn build_report_populated_value_is_ok() {
        let f = fields_fixture();
        let issue = json!({"customfield_10035": 5});
        let rows = build_report(&["Story Points".into()], &[], &issue, &f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, CheckStatus::Ok);
    }

    #[test]
    fn build_report_dedupes_when_warn_overlaps_require_via_name() {
        let f = fields_fixture();
        let issue = json!({});
        // "Story Points" listed in both REQUIRE and WARN; only the REQUIRE row
        // should appear.
        let rows = build_report(
            &["Story Points".into()],
            &["Story Points".into()],
            &issue,
            &f,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].level, CheckLevel::Require);
    }

    #[test]
    fn build_report_dedupes_when_warn_aliases_require_via_id() {
        // Another flavor of the dedupe rule: REQUIRE uses the display name,
        // WARN uses the ID, but they resolve to the same field. Only one row.
        let f = fields_fixture();
        let issue = json!({});
        let rows = build_report(
            &["Story Points".into()],
            &["customfield_10035".into()],
            &issue,
            &f,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].level, CheckLevel::Require);
        assert_eq!(rows[0].id, "customfield_10035");
    }

    // -- CheckRow::to_value (JSON contract) --

    #[test]
    fn check_row_to_value_emits_contract_shape() {
        // Lock the JSON contract: every consumer of `atl jira issue check
        // -F json` parses these four fields. Any rename here is a breaking
        // change for downstream CI scripts.
        let row = CheckRow {
            field: "Story Points".into(),
            id: "customfield_10035".into(),
            level: CheckLevel::Require,
            status: CheckStatus::Missing,
        };
        let v = row.to_value();
        assert_eq!(
            v,
            json!({
                "field": "Story Points",
                "id": "customfield_10035",
                "level": "REQUIRE",
                "status": "MISSING",
            })
        );
    }

    #[test]
    fn check_row_to_value_serializes_warn_and_ok_and_skipped() {
        // Cover the remaining enum variants so a future `as_str` rename
        // surfaces here.
        let row = CheckRow {
            field: "Summary".into(),
            id: "summary".into(),
            level: CheckLevel::Warn,
            status: CheckStatus::Ok,
        };
        assert_eq!(row.to_value()["level"], "WARN");
        assert_eq!(row.to_value()["status"], "OK");

        let row = CheckRow {
            field: "Sprint".into(),
            id: "customfield_10020".into(),
            level: CheckLevel::Warn,
            status: CheckStatus::Skipped,
        };
        assert_eq!(row.to_value()["status"], "SKIPPED");
    }

    // -- DEFAULT_WARN_FIELDS sanity --

    #[test]
    fn default_warn_fields_is_nonempty_and_contains_summary() {
        // Smoke check: this list ships in the binary. An accidental empty
        // slice would silently make `atl jira issue check KEY` (no flags) a
        // no-op against a populated issue.
        assert!(!DEFAULT_WARN_FIELDS.is_empty());
        assert!(DEFAULT_WARN_FIELDS.contains(&"Summary"));
    }
}
