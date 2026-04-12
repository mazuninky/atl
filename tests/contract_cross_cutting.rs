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

#[test]
#[ignore]
fn no_config_file() {
    let runner = AtlRunner::new(std::path::Path::new("/tmp/nonexistent_atl_config_xxx.toml"));
    runner.run_err(&["jira", "me"], 3);
}

#[test]
#[ignore]
fn bad_profile() {
    let _ = &*SETUP;
    runner().run_err(&["--profile", "nonexistent", "jira", "me"], 3);
}

#[test]
#[ignore]
fn output_format_json() {
    let out = runner().run_ok(&["-F", "json", "jira", "me"]);
    let trimmed = out.trim();
    assert!(
        trimmed.starts_with('{') || trimmed.starts_with('['),
        "expected JSON output, got: {out}"
    );
}

#[test]
#[ignore]
fn output_format_csv() {
    let out = runner().run_ok(&["-F", "csv", "jira", "me"]);
    assert!(!out.is_empty());
}

#[test]
#[ignore]
fn output_format_toon() {
    let out = runner().run_ok(&["-F", "toon", "jira", "me"]);
    assert!(!out.is_empty());
}

#[test]
#[ignore]
fn output_format_toml() {
    let out = runner().run_ok(&["-F", "toml", "jira", "me"]);
    assert!(!out.is_empty());
}

#[test]
#[ignore]
fn quiet_mode() {
    runner().run_ok(&["-q", "jira", "me"]);
}

#[test]
#[ignore]
fn verbose_mode() {
    runner().run_ok(&["-vvv", "jira", "me"]);
}

#[test]
#[ignore]
fn read_only_blocks_jira_create() {
    let config = TestConfigBuilder::new()
        .jira(JIRA_PRISM.base_url())
        .read_only(true)
        .build();
    let runner = AtlRunner::new(&config.config_path);
    runner.run_err(
        &[
            "jira",
            "create",
            "--project",
            "TEST",
            "-t",
            "Task",
            "-s",
            "Title",
        ],
        3,
    );
}

#[test]
#[ignore]
fn read_only_blocks_confluence_create() {
    let config = TestConfigBuilder::new()
        .confluence(CONFLUENCE_PRISM.base_url())
        .confluence_api_path("/wiki/rest/api")
        .read_only(true)
        .build();
    let runner = AtlRunner::new(&config.config_path);
    runner.run_err(
        &["conf", "create", "-s", "TEST", "-t", "Title", "-b", "body"],
        3,
    );
}

// --------------------------------------------------------------------------
// `atl self` — help output and clap validation.
//
// These tests are deliberately offline-only: they exercise clap argument
// parsing and help rendering, nothing that would hit GitHub's API. A dummy
// AtlRunner is used so we don't depend on the Prism fixture.
// --------------------------------------------------------------------------

fn self_runner() -> AtlRunner {
    // `--help` short-circuits config loading, so any path works here.
    AtlRunner::new(std::path::Path::new("/tmp/atl-self-help-dummy.toml"))
}

#[test]
fn self_help_lists_subcommands() {
    let runner = self_runner();
    let result = runner.run(&["self", "--help"]);
    assert_eq!(
        result.exit_code, 0,
        "self --help exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result.stdout.contains("check"),
        "expected 'check' in self --help output:\n{}",
        result.stdout
    );
    assert!(
        result.stdout.contains("update"),
        "expected 'update' in self --help output:\n{}",
        result.stdout
    );
}

#[test]
fn self_check_help_runs() {
    let runner = self_runner();
    let result = runner.run(&["self", "check", "--help"]);
    assert_eq!(
        result.exit_code, 0,
        "self check --help exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
}

#[test]
fn self_update_help_lists_flags() {
    let runner = self_runner();
    let result = runner.run(&["self", "update", "--help"]);
    assert_eq!(
        result.exit_code, 0,
        "self update --help exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result.stdout.contains("--to"),
        "expected '--to' in self update --help output:\n{}",
        result.stdout
    );
    assert!(
        result.stdout.contains("--allow-downgrade"),
        "expected '--allow-downgrade' in self update --help output:\n{}",
        result.stdout
    );
}

#[test]
fn self_update_allow_downgrade_requires_to() {
    let runner = self_runner();
    let result = runner.run(&["self", "update", "--allow-downgrade"]);
    assert_ne!(
        result.exit_code, 0,
        "clap should reject --allow-downgrade without --to; stdout: {} stderr: {}",
        result.stdout, result.stderr
    );
}

// --------------------------------------------------------------------------
// P3 — `--jq` / `--template` global flags surface on every command.
//
// We exercise only the help text so these tests stay offline; the pipeline
// itself has exhaustive unit-test coverage in `src/output/transform.rs`.
// --------------------------------------------------------------------------

#[test]
fn help_lists_jq_and_template_flags() {
    // `--help` short-circuits before any config / IO work, so any dummy
    // path is safe here.
    let runner = AtlRunner::new(std::path::Path::new("/tmp/atl-help-dummy.toml"));
    let result = runner.run(&["--help"]);
    assert_eq!(
        result.exit_code, 0,
        "`atl --help` exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result.stdout.contains("--jq"),
        "expected `--jq` in top-level help:\n{}",
        result.stdout
    );
    assert!(
        result.stdout.contains("--template"),
        "expected `--template` in top-level help:\n{}",
        result.stdout
    );
}

// --------------------------------------------------------------------------
// P7 — `atl browse` + `--retries` global flag.
//
// These tests are offline-only: the Jira case constructs its URL purely from
// the configured domain (no HTTP required), and help-text assertions exercise
// clap wiring without hitting any server.
// --------------------------------------------------------------------------

#[test]
fn browse_help_in_top_level_help() {
    let runner = AtlRunner::new(std::path::Path::new("/tmp/atl-help-dummy.toml"));
    let result = runner.run(&["--help"]);
    assert_eq!(
        result.exit_code, 0,
        "`atl --help` exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result.stdout.contains("browse"),
        "expected `browse` in top-level help:\n{}",
        result.stdout
    );
}

#[test]
fn browse_help_lists_service_flag() {
    let runner = AtlRunner::new(std::path::Path::new("/tmp/atl-browse-help-dummy.toml"));
    let result = runner.run(&["browse", "--help"]);
    assert_eq!(
        result.exit_code, 0,
        "`atl browse --help` exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result.stdout.contains("--service"),
        "expected `--service` in browse help:\n{}",
        result.stdout
    );
}

#[test]
fn retries_flag_in_top_level_help() {
    let runner = AtlRunner::new(std::path::Path::new("/tmp/atl-retries-help-dummy.toml"));
    let result = runner.run(&["--help"]);
    assert_eq!(
        result.exit_code, 0,
        "`atl --help` exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result.stdout.contains("--retries"),
        "expected `--retries` in top-level help:\n{}",
        result.stdout
    );
}

// --------------------------------------------------------------------------
// P6 — `atl auth` subcommand wiring.
// --------------------------------------------------------------------------

#[test]
fn auth_command_in_top_level_help() {
    let runner = AtlRunner::new(std::path::Path::new("/tmp/atl-auth-top-help-dummy.toml"));
    let result = runner.run(&["--help"]);
    assert_eq!(
        result.exit_code, 0,
        "`atl --help` exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result.stdout.contains("auth"),
        "expected `auth` in top-level help:\n{}",
        result.stdout
    );
}

#[test]
fn browse_jira_key_prints_url_in_non_tty() {
    // Test process stdout is not a TTY, so browse must print the URL
    // instead of launching a browser. The Jira URL is derived entirely from
    // the configured domain — no API call, no Prism required.
    let config = TestConfigBuilder::new()
        .jira("https://example.atlassian.net")
        .build();
    let runner = AtlRunner::new(&config.config_path);
    let result = runner.run(&["browse", "TEST-1"]);
    assert_eq!(
        result.exit_code, 0,
        "`atl browse TEST-1` exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result
            .stdout
            .contains("https://example.atlassian.net/browse/TEST-1"),
        "expected Jira URL in stdout:\n{}",
        result.stdout
    );
}

#[test]
fn browse_jira_service_override_prints_url() {
    let config = TestConfigBuilder::new()
        .jira("https://example.atlassian.net")
        .build();
    let runner = AtlRunner::new(&config.config_path);
    // Pass a numeric ID but force Jira service; URL should still be built.
    let result = runner.run(&["browse", "--service", "jira", "12345"]);
    assert_eq!(
        result.exit_code, 0,
        "`atl browse --service jira 12345` exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result
            .stdout
            .contains("https://example.atlassian.net/browse/12345"),
        "expected Jira URL in stdout:\n{}",
        result.stdout
    );
}

#[test]
fn browse_auto_detects_jira_key_over_confluence() {
    // Profile has both services; auto-detect must pick Jira for a key-shape.
    let config = TestConfigBuilder::new()
        .jira("https://jira.example.com")
        .confluence("https://conf.example.com")
        .confluence_api_path("/wiki/rest/api")
        .build();
    let runner = AtlRunner::new(&config.config_path);
    let result = runner.run(&["browse", "PROJ-42"]);
    assert_eq!(
        result.exit_code, 0,
        "`atl browse PROJ-42` exited {}; stderr: {}",
        result.exit_code, result.stderr
    );
    assert!(
        result
            .stdout
            .contains("https://jira.example.com/browse/PROJ-42"),
        "expected Jira URL in stdout:\n{}",
        result.stdout
    );
}

#[test]
fn browse_missing_profile_errors() {
    let runner = AtlRunner::new(std::path::Path::new(
        "/tmp/atl-browse-no-profile-dummy.toml",
    ));
    let result = runner.run(&["browse", "PROJ-1"]);
    // Config load failure should surface as a non-zero exit.
    assert_ne!(
        result.exit_code, 0,
        "expected browse to fail without a profile\nstdout:\n{}\nstderr:\n{}",
        result.stdout, result.stderr
    );
}
