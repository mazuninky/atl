//! CLI smoke tests for core `atl` commands.
//!
//! These are Layer 3 (integration) tests that verify the public contract:
//! exit codes, help text, version output, and color suppression.
//! They do NOT test business logic — that belongs in unit/component tests.
//!
//! Every test uses `--config /dev/null` to prevent reading the user's real
//! config and `ATL_NO_UPDATE_NOTIFIER=1` to suppress network calls.

use assert_cmd::Command;
use predicates::prelude::*;

/// Build an isolated `atl` command that won't touch real config or network.
fn atl() -> Command {
    let mut cmd = Command::cargo_bin("atl").unwrap();
    cmd.env("ATL_NO_UPDATE_NOTIFIER", "1");
    // Prevent tracing subscriber init from interfering across parallel tests.
    cmd.env_remove("RUST_LOG");
    // Don't let the developer's shell color settings leak into assertions.
    cmd.env_remove("NO_COLOR");
    cmd.env_remove("CLICOLOR_FORCE");
    cmd
}

// ---------------------------------------------------------------------------
// Help / version
// ---------------------------------------------------------------------------

#[test]
fn top_level_help_exits_zero() {
    atl()
        .args(["--config", "/dev/null", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn top_level_version_exits_zero() {
    atl()
        .args(["--config", "/dev/null", "--version"])
        .assert()
        .success()
        .stdout(predicate::str::contains("atl"));
}

#[test]
fn jira_help_exits_zero() {
    atl()
        .args(["--config", "/dev/null", "jira", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn confluence_help_exits_zero() {
    atl()
        .args(["--config", "/dev/null", "confluence", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn auth_help_exits_zero() {
    atl()
        .args(["--config", "/dev/null", "auth", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn api_help_exits_zero() {
    atl()
        .args(["--config", "/dev/null", "api", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn alias_help_exits_zero() {
    atl()
        .args(["--config", "/dev/null", "alias", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
fn missing_config_exits_with_error() {
    atl()
        .args([
            "--config",
            "/nonexistent/path.toml",
            "jira",
            "search",
            "--jql",
            "project = TEST",
        ])
        .assert()
        .failure();
}

#[test]
fn unknown_subcommand_exits_nonzero() {
    atl()
        .args(["--config", "/dev/null", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

// ---------------------------------------------------------------------------
// Color suppression
// ---------------------------------------------------------------------------

#[test]
fn no_color_flag_suppresses_ansi() {
    atl()
        .args(["--config", "/dev/null", "--no-color", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\x1b[").not());
}
