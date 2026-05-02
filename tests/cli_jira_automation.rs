//! CLI integration tests for `atl jira automation`.
//!
//! Coverage scope: only behaviours that need the **whole binary** and
//! cannot be covered at the component layer in `src/client/jira.rs`.
//! HTTP-shape assertions for the six automation methods (URL, method,
//! body, headers) live in
//! `src/client/jira.rs::automation_component_tests` — they use the
//! `JiraClient::with_automation_base_url` test seam to redirect the
//! cross-host calls to a local `httpmock` server. This file therefore
//! covers:
//!
//! * Help / arg parsing (clap wiring).
//! * Required positional / flag enforcement (clap-level errors).
//! * `read_only` rejection (smoke — the full coverage is at the
//!   component layer).
//! * Cloud-id resolution failure surfaces as exit code 3 (smoke — the
//!   full mapping is asserted in component tests on `JiraClient`).
//! * Body-input parsing (`@file`, malformed JSON) — exercises the CLI
//!   handler's integration with `read_body_arg`.

mod common;

use common::{AtlRunner, TestConfigBuilder};
use httpmock::Method::GET;
use httpmock::MockServer;

/// Stand up a fake Jira at `127.0.0.1:port` and return a runner pointed at it.
fn setup() -> (MockServer, AtlRunner, common::TestConfig) {
    let server = MockServer::start();
    let config = TestConfigBuilder::new().jira(&server.base_url()).build();
    let runner = AtlRunner::new(&config.config_path);
    (server, runner, config)
}

/// As [`setup`] but with `read_only = true` on the profile.
fn setup_readonly() -> (MockServer, AtlRunner, common::TestConfig) {
    let server = MockServer::start();
    let config = TestConfigBuilder::new()
        .jira(&server.base_url())
        .read_only(true)
        .build();
    let runner = AtlRunner::new(&config.config_path);
    (server, runner, config)
}

// ---------------------------------------------------------------------------
// Help wiring — pinned so any rename of a subcommand or flag breaks here.
// These can only be tested at the CLI level (clap derive output).
// ---------------------------------------------------------------------------

#[test]
fn automation_help_lists_all_subcommands() {
    let (_server, runner, _config) = setup();
    let result = runner.run(&["jira", "automation", "--help"]);
    assert_eq!(result.exit_code, 0, "stderr:\n{}", result.stderr);
    for needle in [
        "list", "get", "create", "update", "enable", "disable", "delete",
    ] {
        assert!(
            result.stdout.contains(needle),
            "expected `{needle}` in `atl jira automation --help`:\n{}",
            result.stdout
        );
    }
}

#[test]
fn automation_list_help_advertises_pagination_flags() {
    let (_server, runner, _config) = setup();
    let result = runner.run(&["jira", "automation", "list", "--help"]);
    assert_eq!(result.exit_code, 0, "stderr:\n{}", result.stderr);
    for needle in ["--limit", "--cursor"] {
        assert!(
            result.stdout.contains(needle),
            "expected `{needle}` in list --help:\n{}",
            result.stdout
        );
    }
}

// ---------------------------------------------------------------------------
// Missing positional / flag args (clap usage errors). These guard against
// accidentally removing `required = true` and only manifest at the CLI tier.
// ---------------------------------------------------------------------------

#[test]
fn automation_get_requires_uuid() {
    let (_server, runner, _config) = setup();
    let result = runner.run(&["jira", "automation", "get"]);
    assert_ne!(result.exit_code, 0);
    assert!(
        result.stderr.contains("required") || result.stderr.contains("UUID"),
        "expected clap to mention the missing UUID:\nstderr:\n{}",
        result.stderr
    );
}

#[test]
fn automation_create_requires_body_flag() {
    let (_server, runner, _config) = setup();
    // Body is `#[arg(long, short)]` without `required = true` — clap defaults
    // make it required because the field type is `String`. Confirm.
    let result = runner.run(&["jira", "automation", "create"]);
    assert_ne!(result.exit_code, 0);
    assert!(
        result.stderr.contains("required") || result.stderr.contains("--body"),
        "expected clap to mention --body:\nstderr:\n{}",
        result.stderr
    );
}

// ---------------------------------------------------------------------------
// Cloud-id failure surfaces as exit code 3 (smoke).
//
// Full Error variant mapping is covered at the component layer
// (`automation_component_tests::*`); here we just confirm the
// CLI-level `Error::Config -> exit 3` contract is wired up end-to-end.
// ---------------------------------------------------------------------------

#[test]
fn list_fails_with_config_error_on_non_cloud_site() {
    let (server, runner, _config) = setup();
    server.mock(|when, then| {
        when.method(GET).path("/_edge/tenant_info");
        then.status(404);
    });

    let result = runner.run(&["jira", "automation", "list"]);
    assert_eq!(
        result.exit_code, 3,
        "Config error must map to exit code 3 (CONFIG_ERROR).\nstderr:\n{}\nstdout:\n{}",
        result.stderr, result.stdout
    );
    assert!(
        result.stderr.contains("Cloud") || result.stderr.contains("tenant_info"),
        "stderr should explain the Cloud/tenant_info issue:\n{}",
        result.stderr
    );
}

// ---------------------------------------------------------------------------
// Read-only profile blocks mutating subcommands (smoke).
//
// All four mutators (`create`, `update`, `enable`/`disable`, `delete`) go
// through the same `assert_writable` short-circuit, fully covered at the
// component layer. We keep one CLI-level smoke per HTTP-method shape:
// `create` (POST + body) and `delete` (DELETE + flag).
// ---------------------------------------------------------------------------

#[test]
fn create_blocked_on_read_only_profile() {
    let (_server, runner, _config) = setup_readonly();
    let result = runner.run(&[
        "jira",
        "automation",
        "create",
        "--body",
        r#"{"name": "rule"}"#,
    ]);
    assert_eq!(
        result.exit_code, 3,
        "read-only profile must produce Config error (exit 3).\nstderr:\n{}\nstdout:\n{}",
        result.stderr, result.stdout
    );
    assert!(
        result.stderr.contains("read-only"),
        "stderr should mention read-only:\n{}",
        result.stderr
    );
}

#[test]
fn delete_blocked_on_read_only_profile() {
    let (_server, runner, _config) = setup_readonly();
    let result = runner.run(&["jira", "automation", "delete", "uuid", "--force"]);
    assert_eq!(result.exit_code, 3, "stderr:\n{}", result.stderr);
}

// ---------------------------------------------------------------------------
// Body-flag input handling — `@file` and malformed JSON exercise the CLI
// handler's integration with `read_body_arg`. Cannot be moved to the
// component layer because `read_body_arg` lives in the CLI command layer.
// ---------------------------------------------------------------------------

#[test]
fn create_body_at_file_is_parsed_before_readonly_block() {
    use std::fs::write;
    use tempfile::NamedTempFile;

    let tmp = NamedTempFile::new().expect("tempfile");
    write(tmp.path(), r#"{"name": "rule from file"}"#).expect("write json");
    let body_arg = format!("@{}", tmp.path().to_string_lossy());

    let (_server, runner, _config) = setup_readonly();
    let result = runner.run(&["jira", "automation", "create", "--body", &body_arg]);
    // The handler reads the file, parses it, THEN tries to send — and the
    // read-only check kicks in. Exit 3 confirms the body was parsed
    // successfully (no exit 5 InvalidInput, no exit 1 RUNTIME_ERROR for a
    // JSON parse failure).
    assert_eq!(
        result.exit_code, 3,
        "@file body should parse and then trip read-only.\nstderr:\n{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("read-only"),
        "stderr should mention read-only:\n{}",
        result.stderr
    );
}

#[test]
fn create_body_invalid_json_fails_with_runtime_error() {
    let (_server, runner, _config) = setup();
    // Send a body that is not valid JSON. The client never gets called —
    // the handler returns an anyhow error wrapped from serde_json. That
    // path is not a domain Error variant, so it falls through to exit 1
    // (RUNTIME_ERROR).
    let result = runner.run(&["jira", "automation", "create", "--body", "{not valid json"]);
    assert_eq!(
        result.exit_code, 1,
        "malformed JSON body should exit 1.\nstderr:\n{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("invalid JSON") || result.stderr.contains("JSON"),
        "stderr should mention the JSON parse error:\n{}",
        result.stderr
    );
}
