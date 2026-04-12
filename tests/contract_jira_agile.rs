mod common;

use std::sync::LazyLock;

use common::{AtlRunner, PrismServer, TestConfig, TestConfigBuilder};

static PRISM: LazyLock<PrismServer> =
    LazyLock::new(|| PrismServer::start("tests/contract/specs/jira-software.patched.json"));

static SETUP: LazyLock<(TestConfig, AtlRunner)> = LazyLock::new(|| {
    let config = TestConfigBuilder::new().jira(PRISM.base_url()).build();
    let runner = AtlRunner::new(&config.config_path);
    (config, runner)
});

fn runner() -> &'static AtlRunner {
    &SETUP.1
}

static SETUP_RO: LazyLock<(TestConfig, AtlRunner)> = LazyLock::new(|| {
    let config = TestConfigBuilder::new()
        .jira(PRISM.base_url())
        .read_only(true)
        .build();
    let runner = AtlRunner::new(&config.config_path);
    (config, runner)
});

fn runner_ro() -> &'static AtlRunner {
    &SETUP_RO.1
}

// -- Boards --

#[test]
#[ignore]
fn board_list() {
    runner().run_ok(&["jira", "board", "list"]);
}

#[test]
#[ignore]
fn board_list_by_project() {
    runner().run_ok(&["jira", "board", "list", "--project", "TEST"]);
}

#[test]
#[ignore]
fn board_get() {
    runner().run_ok(&["jira", "board", "get", "1"]);
}

#[test]
#[ignore]
fn board_config() {
    runner().run_ok(&["jira", "board", "config", "1"]);
}

#[test]
#[ignore]
fn board_issues() {
    runner().run_ok(&["jira", "board", "issues", "1"]);
}

#[test]
#[ignore]
fn board_backlog() {
    runner().run_ok(&["jira", "board", "backlog", "1"]);
}

// -- Sprints --

#[test]
#[ignore]
fn sprint_list() {
    runner().run_ok(&["jira", "sprint", "list", "1"]);
}

#[test]
#[ignore]
fn sprint_list_active() {
    runner().run_ok(&["jira", "sprint", "list", "1", "--state", "active"]);
}

#[test]
#[ignore]
fn sprint_get() {
    runner().run_ok(&["jira", "sprint", "get", "1"]);
}

#[test]
#[ignore]
fn sprint_issues() {
    runner().run_ok(&["jira", "sprint", "issues", "1"]);
}

#[test]
#[ignore]
fn sprint_create_positive() {
    runner().run_ok(&[
        "jira",
        "sprint",
        "create",
        "--board-id",
        "1",
        "--name",
        "S1",
    ]);
}

#[test]
#[ignore]
fn sprint_create_read_only() {
    runner_ro().run_err(
        &[
            "jira",
            "sprint",
            "create",
            "--board-id",
            "1",
            "--name",
            "S1",
        ],
        3,
    );
}

#[test]
#[ignore]
fn sprint_update_positive() {
    runner().run_ok(&["jira", "sprint", "update", "1", "--name", "S2"]);
}

#[test]
#[ignore]
fn sprint_update_read_only() {
    runner_ro().run_err(&["jira", "sprint", "update", "1", "--name", "S2"], 3);
}

#[test]
#[ignore]
fn sprint_delete_positive() {
    runner().run_ok(&["jira", "sprint", "delete", "1"]);
}

#[test]
#[ignore]
fn sprint_delete_read_only() {
    runner_ro().run_err(&["jira", "sprint", "delete", "1"], 3);
}

#[test]
#[ignore]
fn sprint_move_positive() {
    runner().run_ok(&["jira", "sprint", "move", "1", "TEST-1", "TEST-2"]);
}

#[test]
#[ignore]
fn sprint_move_read_only() {
    runner_ro().run_err(&["jira", "sprint", "move", "1", "TEST-1", "TEST-2"], 3);
}

#[test]
#[ignore]
fn backlog_move_positive() {
    runner().run_ok(&["jira", "backlog-move", "TEST-1", "TEST-2"]);
}

#[test]
#[ignore]
fn backlog_move_read_only() {
    runner_ro().run_err(&["jira", "backlog-move", "TEST-1", "TEST-2"], 3);
}

// -- Epics --

#[test]
#[ignore]
fn epic_list() {
    runner().run_ok(&["jira", "epic", "list", "1"]);
}

#[test]
#[ignore]
fn epic_get() {
    runner().run_ok(&["jira", "epic", "get", "TEST-100"]);
}

#[test]
#[ignore]
fn epic_issues() {
    runner().run_ok(&["jira", "epic", "issues", "TEST-100"]);
}

#[test]
#[ignore]
fn epic_add_positive() {
    runner().run_ok(&["jira", "epic", "add", "TEST-100", "TEST-1", "TEST-2"]);
}

#[test]
#[ignore]
fn epic_add_read_only() {
    runner_ro().run_err(&["jira", "epic", "add", "TEST-100", "TEST-1", "TEST-2"], 3);
}

#[test]
#[ignore]
fn epic_remove_positive() {
    runner().run_ok(&["jira", "epic", "remove", "TEST-1"]);
}

#[test]
#[ignore]
fn epic_remove_read_only() {
    runner_ro().run_err(&["jira", "epic", "remove", "TEST-1"], 3);
}
