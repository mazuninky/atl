//! CLI integration tests for `atl jira issue check`.
//!
//! These tests stand up an [`httpmock`] server bound to `127.0.0.1:port` and
//! point an `atl` config at it. The check command hits two same-host endpoints
//! — `/rest/api/2/field` and `/rest/api/2/issue/{key}` — both of which are
//! happy on a non-Cloud domain (Prism v2 routes), so we don't need any
//! `flavor = "cloud"` override.
//!
//! Contract pinned by these tests:
//!
//! 1. Stdout is the JSON-shaped per-field report (a JSON array when `-F json`).
//! 2. Exit code 1 (`RUNTIME_ERROR`, mapped from `Error::CheckFailed`) when at
//!    least one REQUIRE field is MISSING. The report still goes to stdout —
//!    CI scripts depend on it.
//! 3. Exit code 0 when all REQUIRE fields are populated.
//! 4. `--help` is wired correctly and surfaces the documented flags.

mod common;

use common::{AtlRunner, TestConfigBuilder};
use httpmock::Method::GET;
use httpmock::MockServer;
use serde_json::Value;

/// Parse a `Value` from stdout. Some flag combinations interleave warnings on
/// stderr, so the JSON sits on stdout cleanly.
fn parse_json_stdout(stdout: &str) -> Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("stdout was not valid JSON ({e}): {stdout}"))
}

/// Stand up a fake Jira at `127.0.0.1:port` and return a runner pointed at it.
fn setup() -> (MockServer, AtlRunner, common::TestConfig) {
    let server = MockServer::start();
    let config = TestConfigBuilder::new().jira(&server.base_url()).build();
    let runner = AtlRunner::new(&config.config_path);
    (server, runner, config)
}

/// The `/field` endpoint with a small but realistic field schema.
fn mock_fields(server: &MockServer) {
    server.mock(|when, then| {
        when.method(GET).path("/rest/api/2/field");
        then.status(200).json_body(serde_json::json!([
            {"id": "summary", "name": "Summary"},
            {"id": "description", "name": "Description"},
            {"id": "customfield_10035", "name": "Story Points"},
        ]));
    });
}

// ---------------------------------------------------------------------------
// Help wiring
// ---------------------------------------------------------------------------

#[test]
fn check_help_advertises_require_and_warn_flags() {
    let server = MockServer::start();
    let config = TestConfigBuilder::new().jira(&server.base_url()).build();
    let runner = AtlRunner::new(&config.config_path);

    let result = runner.run(&["jira", "issue", "check", "--help"]);
    assert_eq!(
        result.exit_code, 0,
        "stderr:\n{}\nstdout:\n{}",
        result.stderr, result.stdout
    );
    for needle in ["--require", "--warn", "<KEY>"] {
        assert!(
            result.stdout.contains(needle),
            "expected `{needle}` in `atl jira issue check --help`:\n{}",
            result.stdout
        );
    }
}

// ---------------------------------------------------------------------------
// Happy path: required field populated → exit 0
// ---------------------------------------------------------------------------

#[test]
fn require_populated_exits_zero_with_ok_status() {
    let (server, runner, _config) = setup();
    mock_fields(&server);
    server.mock(|when, then| {
        when.method(GET).path("/rest/api/2/issue/PROJ-1");
        then.status(200).json_body(serde_json::json!({
            "key": "PROJ-1",
            "fields": {
                "summary": "Hello",
                "customfield_10035": 5,
            }
        }));
    });

    let result = runner.run(&[
        "jira",
        "issue",
        "check",
        "PROJ-1",
        "--require",
        "Story Points",
        "-F",
        "json",
    ]);

    assert_eq!(
        result.exit_code, 0,
        "expected exit 0 (story points populated), got {}\nstderr:\n{}\nstdout:\n{}",
        result.exit_code, result.stderr, result.stdout
    );

    let v = parse_json_stdout(&result.stdout);
    let arr = v.as_array().expect("stdout JSON should be an array");
    assert_eq!(arr.len(), 1, "expected one row, got: {v:#}");
    assert_eq!(arr[0]["id"], "customfield_10035");
    assert_eq!(arr[0]["level"], "REQUIRE");
    assert_eq!(arr[0]["status"], "OK");
    assert_eq!(arr[0]["field"], "Story Points");
}

// ---------------------------------------------------------------------------
// Negative path: required field is null → exit 1, report still on stdout
// ---------------------------------------------------------------------------

#[test]
fn require_null_exits_one_and_emits_report_to_stdout() {
    let (server, runner, _config) = setup();
    mock_fields(&server);
    server.mock(|when, then| {
        when.method(GET).path("/rest/api/2/issue/PROJ-1");
        then.status(200).json_body(serde_json::json!({
            "key": "PROJ-1",
            "fields": {
                "summary": "Hello",
                "customfield_10035": null,
            }
        }));
    });

    let result = runner.run(&[
        "jira",
        "issue",
        "check",
        "PROJ-1",
        "--require",
        "Story Points",
        "-F",
        "json",
    ]);

    // The contract: exit 1 (CheckFailed → RUNTIME_ERROR), stdout still
    // carries the structured report so CI scripts can read it on failure.
    assert_eq!(
        result.exit_code, 1,
        "expected exit 1 (CheckFailed → RUNTIME_ERROR), got {}\nstderr:\n{}\nstdout:\n{}",
        result.exit_code, result.stderr, result.stdout
    );

    let v = parse_json_stdout(&result.stdout);
    let arr = v.as_array().expect("stdout JSON should be an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "customfield_10035");
    assert_eq!(arr[0]["status"], "MISSING");
    assert_eq!(arr[0]["level"], "REQUIRE");

    // The hint with the missing field IDs should also appear on stderr so
    // an interactive operator sees what failed.
    assert!(
        result.stderr.contains("customfield_10035"),
        "stderr should mention the missing field ID:\n{}",
        result.stderr
    );
}

// ---------------------------------------------------------------------------
// Mixed levels: REQUIRE missing + WARN missing → exit 1, both rows present
// ---------------------------------------------------------------------------

#[test]
fn mixed_require_and_warn_both_appear_in_report() {
    let (server, runner, _config) = setup();
    mock_fields(&server);
    server.mock(|when, then| {
        when.method(GET).path("/rest/api/2/issue/PROJ-1");
        then.status(200).json_body(serde_json::json!({
            "key": "PROJ-1",
            // summary is the empty string (MISSING per is_field_empty)
            // customfield_10035 is missing entirely (SKIPPED → not MISSING)
            "fields": { "summary": "" }
        }));
    });

    let result = runner.run(&[
        "jira",
        "issue",
        "check",
        "PROJ-1",
        "--require",
        "summary",
        "--warn",
        "Story Points",
        "-F",
        "json",
    ]);

    assert_eq!(
        result.exit_code, 1,
        "summary is empty → REQUIRE failed → exit 1\nstdout:\n{}\nstderr:\n{}",
        result.stdout, result.stderr
    );

    let v = parse_json_stdout(&result.stdout);
    let arr = v.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["id"], "summary");
    assert_eq!(arr[0]["level"], "REQUIRE");
    assert_eq!(arr[0]["status"], "MISSING");
    assert_eq!(arr[1]["id"], "customfield_10035");
    assert_eq!(arr[1]["level"], "WARN");
    // SKIPPED, not MISSING — the field is absent from the issue, not null.
    assert_eq!(arr[1]["status"], "SKIPPED");
}

// ---------------------------------------------------------------------------
// Warn-only missing → exit 0
// ---------------------------------------------------------------------------

#[test]
fn warn_only_missing_does_not_fail_command() {
    let (server, runner, _config) = setup();
    mock_fields(&server);
    server.mock(|when, then| {
        when.method(GET).path("/rest/api/2/issue/PROJ-1");
        then.status(200).json_body(serde_json::json!({
            "key": "PROJ-1",
            "fields": { "summary": "Hello", "description": null }
        }));
    });

    let result = runner.run(&[
        "jira",
        "issue",
        "check",
        "PROJ-1",
        "--warn",
        "description",
        "-F",
        "json",
    ]);

    assert_eq!(
        result.exit_code, 0,
        "WARN-only missing must not fail the command\nstdout:\n{}\nstderr:\n{}",
        result.stdout, result.stderr
    );

    let v = parse_json_stdout(&result.stdout);
    assert_eq!(v[0]["status"], "MISSING");
    assert_eq!(v[0]["level"], "WARN");
}

// ---------------------------------------------------------------------------
// `--require` with multiple fields (comma-separated AND repeated)
// ---------------------------------------------------------------------------

#[test]
fn require_accepts_comma_lists_and_repetition() {
    let (server, runner, _config) = setup();
    mock_fields(&server);
    server.mock(|when, then| {
        when.method(GET).path("/rest/api/2/issue/PROJ-1");
        then.status(200).json_body(serde_json::json!({
            "key": "PROJ-1",
            "fields": {
                "summary": "S",
                "description": "D",
                "customfield_10035": 8,
            }
        }));
    });

    // `summary,description` arrives as one token via clap's value_delimiter,
    // and `Story Points` arrives via a second `--require`. Together they
    // must produce exactly three OK rows.
    let result = runner.run(&[
        "jira",
        "issue",
        "check",
        "PROJ-1",
        "--require",
        "summary,description",
        "--require",
        "Story Points",
        "-F",
        "json",
    ]);

    assert_eq!(result.exit_code, 0, "stderr:\n{}", result.stderr);
    let v = parse_json_stdout(&result.stdout);
    let arr = v.as_array().expect("array");
    assert_eq!(arr.len(), 3, "got: {v:#}");
    for row in arr {
        assert_eq!(row["status"], "OK", "row: {row}");
        assert_eq!(row["level"], "REQUIRE", "row: {row}");
    }
}

// ---------------------------------------------------------------------------
// Missing positional argument
// ---------------------------------------------------------------------------

#[test]
fn missing_key_argument_is_clap_usage_error() {
    let server = MockServer::start();
    let config = TestConfigBuilder::new().jira(&server.base_url()).build();
    let runner = AtlRunner::new(&config.config_path);

    let result = runner.run(&["jira", "issue", "check"]);
    // clap exit code is 2 for usage errors. Don't pin the exact byte —
    // assert it's nonzero and the stderr explains.
    assert_ne!(result.exit_code, 0, "missing <KEY> should fail");
    assert!(
        result.stderr.contains("required") || result.stderr.contains("<KEY>"),
        "expected clap to mention the missing positional argument:\nstderr:\n{}",
        result.stderr
    );
}
