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

/// Returns a platform-appropriate path to a null/empty file that clap can
/// accept as `--config` without error.
fn null_config_path() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// Returns a platform-appropriate path that is guaranteed not to exist, for
/// testing error handling on missing config files.
fn nonexistent_config_path() -> String {
    let dir = std::env::temp_dir();
    dir.join("atl-test-nonexistent-config-4f9a1c.toml")
        .to_string_lossy()
        .into_owned()
}

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
        .args(["--config", null_config_path(), "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn top_level_version_exits_zero() {
    atl()
        .args(["--config", null_config_path(), "--version"])
        .assert()
        .success()
        .stdout(predicate::str::contains("atl"));
}

#[test]
fn jira_help_exits_zero() {
    atl()
        .args(["--config", null_config_path(), "jira", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn confluence_help_exits_zero() {
    atl()
        .args(["--config", null_config_path(), "confluence", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn auth_help_exits_zero() {
    atl()
        .args(["--config", null_config_path(), "auth", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn api_help_exits_zero() {
    atl()
        .args(["--config", null_config_path(), "api", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

#[test]
fn alias_help_exits_zero() {
    atl()
        .args(["--config", null_config_path(), "alias", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage"));
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
fn missing_config_exits_with_error() {
    let bad_path = nonexistent_config_path();
    atl()
        .args([
            "--config",
            &bad_path,
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
        .args(["--config", null_config_path(), "nonexistent"])
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
        .args(["--config", null_config_path(), "--no-color", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\x1b[").not());
}

// ---------------------------------------------------------------------------
// Jira `--input-format` flag presence
//
// Locks in that the markdown-conversion flag is wired on every command that
// accepts a body. A help dump is the cheapest contract check — no network or
// auth required — and catches accidental removal of the `#[arg]` attribute.
// ---------------------------------------------------------------------------

#[test]
fn jira_create_help_advertises_input_format_flag() {
    atl()
        .args(["--config", null_config_path(), "jira", "create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--input-format"));
}

#[test]
fn jira_update_help_advertises_input_format_flag() {
    atl()
        .args(["--config", null_config_path(), "jira", "update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--input-format"));
}

#[test]
fn jira_comment_help_advertises_input_format_flag() {
    atl()
        .args(["--config", null_config_path(), "jira", "comment", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--input-format"));
}

#[test]
fn jira_create_rejects_invalid_input_format() {
    // Clap should reject an unknown enum value with exit code 2 (usage error).
    // This guards the `value_enum` constraint on `--input-format` so removing
    // it would fail this test.
    atl()
        .args([
            "--config",
            null_config_path(),
            "jira",
            "create",
            "--project",
            "X",
            "-t",
            "Task",
            "-s",
            "summary",
            "--input-format",
            "bogus",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid value")
                .or(predicate::str::contains("possible values")),
        );
}
