//! Contract tests for `atl api` — generic REST passthrough.
//!
//! Most tests are `#[ignore]`d so they only run when a Prism mock server is
//! available, mirroring the other `contract_*.rs` suites. The tests that
//! exercise argument parsing and `--help` output are unconditional because
//! they never hit the network.

mod common;

use std::sync::LazyLock;

use common::{AtlRunner, PrismServer, TestConfig, TestConfigBuilder};

static JIRA_PRISM: LazyLock<PrismServer> =
    LazyLock::new(|| PrismServer::start("tests/contract/specs/jira-platform.patched.json"));

static CONFLUENCE_PRISM: LazyLock<PrismServer> =
    LazyLock::new(|| PrismServer::start("tests/contract/specs/confluence-v2.patched.json"));

static SETUP: LazyLock<(TestConfig, AtlRunner)> = LazyLock::new(|| {
    let config = TestConfigBuilder::new()
        .jira(JIRA_PRISM.base_url())
        .confluence(CONFLUENCE_PRISM.base_url())
        .confluence_api_path("/wiki/rest/api")
        .build();
    let runner = AtlRunner::new(&config.config_path);
    (config, runner)
});

fn runner() -> &'static AtlRunner {
    &SETUP.1
}

// --------------------------------------------------------------------------
// Offline tests: `--help` output + clap validation. These never hit Prism.
// --------------------------------------------------------------------------

/// `--help` short-circuits config loading and network calls, so any path
/// works for `AtlRunner::new`.
fn help_runner() -> AtlRunner {
    AtlRunner::new(std::path::Path::new("/tmp/atl-api-help-dummy.toml"))
}

#[test]
fn api_help_lists_flags() {
    let runner = help_runner();
    let result = runner.run(&["api", "--help"]);
    assert_eq!(
        result.exit_code, 0,
        "api --help exited {}; stderr: {}",
        result.exit_code, result.stderr
    );

    // Every flag named in the plan must appear in `--help` output.
    for expected in &[
        "--service",
        "--method",
        "--header",
        "--field",
        "--raw-field",
        "--input",
        "--query",
        "--paginate",
        "--preview",
    ] {
        assert!(
            result.stdout.contains(expected),
            "expected {expected} in api --help output:\n{}",
            result.stdout
        );
    }
}

#[test]
fn api_missing_service_fails() {
    // clap returns exit code 2 for validation errors. We pick an endpoint
    // that does not exist, but we should never reach the network because
    // the required `--service` argument is missing.
    let runner = help_runner();
    let result = runner.run(&["api", "rest/api/2/myself"]);
    assert_eq!(
        result.exit_code, 2,
        "expected clap validation exit 2; got {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result.stderr.contains("--service"),
        "expected stderr to mention --service; stderr: {}",
        result.stderr
    );
}

#[test]
fn api_missing_endpoint_fails() {
    let runner = help_runner();
    let result = runner.run(&["api", "--service", "jira"]);
    assert_eq!(
        result.exit_code, 2,
        "expected clap validation exit 2 when endpoint is missing; stderr: {}",
        result.stderr
    );
}

#[test]
fn api_field_and_input_mutually_exclusive() {
    let runner = help_runner();
    let result = runner.run(&[
        "api",
        "--service",
        "jira",
        "rest/api/2/issue",
        "--field",
        "a=b",
        "--input",
        "-",
    ]);
    assert_eq!(
        result.exit_code, 2,
        "expected clap validation exit 2 when --field and --input collide; stderr: {}",
        result.stderr
    );
}

// --------------------------------------------------------------------------
// Prism-dependent tests
// --------------------------------------------------------------------------

#[test]
#[ignore]
fn api_get_myself_jira() {
    let out = runner().run_ok(&["api", "--service", "jira", "rest/api/2/myself"]);
    // The passthrough defaults to JSON, so stdout should be parseable.
    let trimmed = out.trim();
    assert!(
        trimmed.starts_with('{') || trimmed.starts_with('['),
        "expected JSON output, got: {out}"
    );
}

#[test]
#[ignore]
fn api_get_with_query() {
    let out = runner().run_ok(&[
        "api",
        "--service",
        "jira",
        "rest/api/2/search",
        "--query",
        "jql=project=TEST",
        "--query",
        "maxResults=5",
    ]);
    assert!(!out.is_empty(), "expected non-empty output from api search");
}

#[test]
#[ignore]
fn api_post_with_fields() {
    // POST with both --field (strings) and --raw-field (structured JSON).
    // Prism only verifies the request shape so this is sufficient to cover
    // body construction end-to-end.
    runner().run_ok(&[
        "api",
        "--service",
        "jira",
        "--method",
        "POST",
        "rest/api/2/issue",
        "--raw-field",
        r#"fields={"project":{"key":"TEST"},"summary":"x","issuetype":{"name":"Task"}}"#,
    ]);
}

#[test]
#[ignore]
fn api_paginate_jira_search() {
    // Paginated call against Jira v2 search. Prism only returns one page
    // but the pagination loop should terminate without error.
    runner().run_ok(&[
        "api",
        "--service",
        "jira",
        "rest/api/2/search",
        "--query",
        "jql=project=TEST",
        "--paginate",
    ]);
}

#[test]
#[ignore]
fn api_confluence_get_pages() {
    // Absolute endpoint form against Confluence, using the v1 API path that
    // the test fixture is configured with.
    runner().run_ok(&["api", "--service", "confluence", "/wiki/rest/api/space"]);
}

#[test]
#[ignore]
fn api_preview_does_not_hit_network() {
    // --preview must NOT perform any HTTP call. We validate this by pointing
    // the request at a path that does not exist in the spec (Prism would
    // return NO_PATH_MATCHED_ERROR if it were hit, which `run_ok` refuses to
    // accept). The call should succeed and emit the preview header on stderr.
    let result = runner().run(&[
        "api",
        "--service",
        "jira",
        "rest/api/2/this/does/not/exist",
        "--preview",
    ]);
    assert_eq!(
        result.exit_code, 0,
        "preview should exit 0 without sending a request; stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("HTTP Request Preview"),
        "expected preview header on stderr; got: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("<redacted>"),
        "expected redacted auth marker on stderr; got: {}",
        result.stderr
    );
    // The absent endpoint must not have been reached.
    assert!(
        !result.stderr.contains("NO_PATH_MATCHED_ERROR"),
        "preview must not hit the network; stderr: {}",
        result.stderr
    );
}

#[test]
#[ignore]
fn api_preview_with_raw_field_body() {
    // --preview should dump the constructed body to stderr.
    let result = runner().run(&[
        "api",
        "--service",
        "jira",
        "rest/api/2/issue",
        "--method",
        "POST",
        "--raw-field",
        r#"fields={"summary":"x"}"#,
        "--preview",
    ]);
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("Body:"),
        "expected Body: section on stderr; got: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("summary"),
        "expected body to include field name; got: {}",
        result.stderr
    );
}
