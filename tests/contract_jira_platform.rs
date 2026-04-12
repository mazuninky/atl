mod common;

use std::sync::LazyLock;

use common::{AtlRunner, PrismServer, TestConfig, TestConfigBuilder};

static PRISM: LazyLock<PrismServer> =
    LazyLock::new(|| PrismServer::start("tests/contract/specs/jira-platform.patched.json"));

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

// -- Issues Core --

#[test]
#[ignore]
fn search_basic() {
    runner().run_ok(&["jira", "search", "project=TEST"]);
}

#[test]
#[ignore]
fn search_with_filters() {
    runner().run_ok(&[
        "jira",
        "search",
        "--status",
        "Open",
        "--assignee",
        "currentUser()",
    ]);
}

#[test]
#[ignore]
fn search_with_limit() {
    runner().run_ok(&["jira", "search", "project=TEST", "--limit", "10"]);
}

#[test]
#[ignore]
fn view_issue() {
    runner().run_ok(&["jira", "view", "TEST-1"]);
}

#[test]
#[ignore]
fn create_issue_positive() {
    runner().run_ok(&[
        "jira",
        "create",
        "--project",
        "TEST",
        "-t",
        "Task",
        "-s",
        "Title",
    ]);
}

#[test]
#[ignore]
fn create_issue_read_only() {
    runner_ro().run_err(
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
fn update_issue_positive() {
    runner().run_ok(&["jira", "update", "TEST-1", "--summary", "New"]);
}

#[test]
#[ignore]
fn update_issue_read_only() {
    runner_ro().run_err(&["jira", "update", "TEST-1", "--summary", "New"], 3);
}

#[test]
#[ignore]
fn delete_issue_positive() {
    runner().run_ok(&["jira", "delete", "TEST-1"]);
}

#[test]
#[ignore]
fn delete_issue_read_only() {
    runner_ro().run_err(&["jira", "delete", "TEST-1"], 3);
}

#[test]
#[ignore]
fn transition_issue_positive() {
    runner().run_ok(&["jira", "move", "TEST-1", "-t", "31"]);
}

#[test]
#[ignore]
fn transition_issue_read_only() {
    runner_ro().run_err(&["jira", "move", "TEST-1", "-t", "31"], 3);
}

#[test]
#[ignore]
fn transitions_list() {
    runner().run_ok(&["jira", "transitions", "TEST-1"]);
}

#[test]
#[ignore]
fn assign_positive() {
    runner().run_ok(&["jira", "assign", "TEST-1", "account123"]);
}

#[test]
#[ignore]
fn assign_read_only() {
    runner_ro().run_err(&["jira", "assign", "TEST-1", "account123"], 3);
}

#[test]
#[ignore]
fn comment_positive() {
    runner().run_ok(&["jira", "comment", "TEST-1", "text"]);
}

#[test]
#[ignore]
fn comment_read_only() {
    runner_ro().run_err(&["jira", "comment", "TEST-1", "text"], 3);
}

#[test]
#[ignore]
fn comments_list() {
    runner().run_ok(&["jira", "comments", "TEST-1"]);
}

#[test]
#[ignore]
fn create_meta() {
    runner().run_ok(&["jira", "create-meta", "--project", "TEST"]);
}

#[test]
#[ignore]
fn edit_meta() {
    runner().run_ok(&["jira", "edit-meta", "TEST-1"]);
}

// -- Issues Extras --

#[test]
#[ignore]
fn worklog_list() {
    runner().run_ok(&["jira", "worklog", "list", "TEST-1"]);
}

#[test]
#[ignore]
fn worklog_add_positive() {
    runner().run_ok(&["jira", "worklog", "add", "TEST-1", "--time-spent", "2h"]);
}

#[test]
#[ignore]
fn worklog_add_read_only() {
    runner_ro().run_err(
        &["jira", "worklog", "add", "TEST-1", "--time-spent", "2h"],
        3,
    );
}

#[test]
#[ignore]
fn watchers_list() {
    runner().run_ok(&["jira", "watchers", "TEST-1"]);
}

#[test]
#[ignore]
fn watch_positive() {
    runner().run_ok(&["jira", "watch", "TEST-1"]);
}

#[test]
#[ignore]
fn unwatch_positive() {
    runner().run_ok(&["jira", "unwatch", "TEST-1"]);
}

#[test]
#[ignore]
fn vote_positive() {
    runner().run_ok(&["jira", "vote", "TEST-1"]);
}

#[test]
#[ignore]
fn unvote_positive() {
    runner().run_ok(&["jira", "unvote", "TEST-1"]);
}

#[test]
#[ignore]
fn changelog_basic() {
    runner().run_ok(&["jira", "changelog", "TEST-1"]);
}

#[test]
#[ignore]
fn changelog_paginated() {
    runner().run_ok(&["jira", "changelog", "TEST-1", "--limit", "10"]);
}

#[test]
#[ignore]
fn attach_positive() {
    let file = tempfile::NamedTempFile::new().expect("create tempfile");
    let path = file.path().to_str().expect("utf-8 path");
    runner().run_ok(&["jira", "attach", "TEST-1", "-f", path]);
}

#[test]
#[ignore]
fn link_positive() {
    runner().run_ok(&["jira", "link", "-t", "Blocks", "TEST-1", "TEST-2"]);
}

#[test]
#[ignore]
fn link_type_list() {
    runner().run_ok(&["jira", "link-type", "list"]);
}

#[test]
#[ignore]
fn link_type_get() {
    runner().run_ok(&["jira", "link-type", "get", "10001"]);
}

#[test]
#[ignore]
fn link_type_create_positive() {
    runner().run_ok(&[
        "jira",
        "link-type",
        "create",
        "--name",
        "T",
        "--inward",
        "I",
        "--outward",
        "O",
    ]);
}

#[test]
#[ignore]
fn issue_link_get() {
    runner().run_ok(&["jira", "issue-link-get", "10001"]);
}

#[test]
#[ignore]
fn remote_link_positive() {
    runner().run_ok(&["jira", "remote-link", "TEST-1", "https://x.com"]);
}

#[test]
#[ignore]
fn remote_links_list() {
    runner().run_ok(&["jira", "remote-links", "TEST-1"]);
}

#[test]
#[ignore]
fn notify_positive() {
    runner().run_ok(&["jira", "notify", "TEST-1", "-s", "Subj", "-b", "Body"]);
}

#[test]
#[ignore]
fn clone_positive() {
    runner().run_ok(&["jira", "clone", "TEST-1"]);
}

// -- Projects --

#[test]
#[ignore]
fn project_list() {
    runner().run_ok(&["jira", "project", "list"]);
}

#[test]
#[ignore]
fn project_get() {
    runner().run_ok(&["jira", "project", "get", "TEST"]);
}

#[test]
#[ignore]
fn project_create_positive() {
    runner().run_ok(&[
        "jira",
        "project",
        "create",
        "--key",
        "T",
        "--name",
        "Test",
        "--project-type-key",
        "software",
        "--lead",
        "acc1",
    ]);
}

#[test]
#[ignore]
fn project_create_read_only() {
    runner_ro().run_err(
        &[
            "jira",
            "project",
            "create",
            "--key",
            "T",
            "--name",
            "Test",
            "--project-type-key",
            "software",
            "--lead",
            "acc1",
        ],
        3,
    );
}

#[test]
#[ignore]
fn project_update_positive() {
    runner().run_ok(&["jira", "project", "update", "TEST", "--name", "New"]);
}

#[test]
#[ignore]
fn project_delete_positive() {
    runner().run_ok(&["jira", "project", "delete", "TEST"]);
}

#[test]
#[ignore]
fn project_statuses() {
    runner().run_ok(&["jira", "project", "statuses", "TEST"]);
}

#[test]
#[ignore]
fn project_roles() {
    runner().run_ok(&["jira", "project", "roles", "TEST"]);
}

#[test]
#[ignore]
fn project_features() {
    runner().run_ok(&["jira", "project", "features", "TEST"]);
}

// -- Users --

#[test]
#[ignore]
fn user_me() {
    runner().run_ok(&["jira", "me"]);
}

#[test]
#[ignore]
fn user_get() {
    runner().run_ok(&["jira", "user", "get", "acc123"]);
}

#[test]
#[ignore]
fn user_search() {
    runner().run_ok(&["jira", "user", "search", "john"]);
}

#[test]
#[ignore]
fn user_list() {
    runner().run_ok(&["jira", "user", "list"]);
}

// -- Groups --

#[test]
#[ignore]
fn group_list() {
    runner().run_ok(&["jira", "group", "list"]);
}

#[test]
#[ignore]
fn group_search() {
    runner().run_ok(&["jira", "group", "search", "dev"]);
}

#[test]
#[ignore]
fn group_members() {
    runner().run_ok(&["jira", "group", "members", "devs"]);
}

// -- Filters --

#[test]
#[ignore]
fn filter_list_favourites() {
    runner().run_ok(&["jira", "filter", "list", "--favourites"]);
}

#[test]
#[ignore]
fn filter_list_mine() {
    runner().run_ok(&["jira", "filter", "list", "--mine"]);
}

#[test]
#[ignore]
fn filter_get() {
    runner().run_ok(&["jira", "filter", "get", "10001"]);
}

#[test]
#[ignore]
fn filter_create_positive() {
    runner().run_ok(&[
        "jira",
        "filter",
        "create",
        "--name",
        "F",
        "--jql",
        "project=TEST",
    ]);
}

// -- Dashboards --

#[test]
#[ignore]
fn dashboard_list() {
    runner().run_ok(&["jira", "dashboard", "list"]);
}

#[test]
#[ignore]
fn dashboard_get() {
    runner().run_ok(&["jira", "dashboard", "get", "10001"]);
}

#[test]
#[ignore]
fn dashboard_create_positive() {
    runner().run_ok(&["jira", "dashboard", "create", "--name", "D"]);
}

// -- Versions --

#[test]
#[ignore]
fn version_list() {
    runner().run_ok(&["jira", "version", "list", "TEST"]);
}

#[test]
#[ignore]
fn version_get() {
    runner().run_ok(&["jira", "version", "get", "10001"]);
}

#[test]
#[ignore]
fn version_create_positive() {
    runner().run_ok(&[
        "jira",
        "version",
        "create",
        "--project",
        "TEST",
        "--name",
        "1.0",
    ]);
}

// -- Components --

#[test]
#[ignore]
fn component_list() {
    runner().run_ok(&["jira", "component", "list", "TEST"]);
}

#[test]
#[ignore]
fn component_create_positive() {
    runner().run_ok(&[
        "jira",
        "component",
        "create",
        "--project",
        "TEST",
        "--name",
        "BE",
    ]);
}

// -- Fields --

#[test]
#[ignore]
fn field_list() {
    runner().run_ok(&["jira", "field", "list"]);
}

// -- Admin read-only --

#[test]
#[ignore]
fn issue_type_list() {
    runner().run_ok(&["jira", "issue-type", "list"]);
}

#[test]
#[ignore]
fn issue_type_get() {
    runner().run_ok(&["jira", "issue-type", "get", "10001"]);
}

#[test]
#[ignore]
fn priority_list() {
    runner().run_ok(&["jira", "priority", "list"]);
}

#[test]
#[ignore]
fn resolution_list() {
    runner().run_ok(&["jira", "resolution", "list"]);
}

#[test]
#[ignore]
fn status_list() {
    runner().run_ok(&["jira", "status", "list"]);
}

#[test]
#[ignore]
fn status_categories() {
    runner().run_ok(&["jira", "status", "categories"]);
}

#[test]
#[ignore]
fn screen_list() {
    runner().run_ok(&["jira", "screen", "list"]);
}

#[test]
#[ignore]
fn workflow_scheme_list() {
    runner().run_ok(&["jira", "workflow-scheme", "list"]);
}

#[test]
#[ignore]
fn permission_scheme_list() {
    runner().run_ok(&["jira", "permission-scheme", "list"]);
}

#[test]
#[ignore]
fn notification_scheme_list() {
    runner().run_ok(&["jira", "notification-scheme", "list"]);
}

#[test]
#[ignore]
fn field_config_list() {
    runner().run_ok(&["jira", "field-config", "list"]);
}

#[test]
#[ignore]
fn role_list() {
    runner().run_ok(&["jira", "role", "list"]);
}

// -- Misc --

#[test]
#[ignore]
fn labels_list() {
    runner().run_ok(&["jira", "labels"]);
}

#[test]
#[ignore]
fn server_info() {
    runner().run_ok(&["jira", "server-info"]);
}

#[test]
#[ignore]
fn configuration() {
    runner().run_ok(&["jira", "configuration"]);
}

#[test]
#[ignore]
fn permissions() {
    runner().run_ok(&["jira", "permissions"]);
}

#[test]
#[ignore]
fn my_permissions() {
    runner().run_ok(&["jira", "my-permissions"]);
}

#[test]
#[ignore]
fn banner_get() {
    runner().run_ok(&["jira", "banner", "get"]);
}

#[test]
#[ignore]
fn webhook_list() {
    runner().run_ok(&["jira", "webhook", "list"]);
}

#[test]
#[ignore]
fn audit_records() {
    runner().run_ok(&["jira", "audit-records"]);
}

// -- Project categories + Attachment admin --

#[test]
#[ignore]
fn project_category_list() {
    runner().run_ok(&["jira", "project-category", "list"]);
}

#[test]
#[ignore]
fn attachment_meta() {
    runner().run_ok(&["jira", "attachment", "meta"]);
}
