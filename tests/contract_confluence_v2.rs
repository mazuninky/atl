mod common;

use std::sync::LazyLock;

use common::{AtlRunner, PrismServer, TestConfig, TestConfigBuilder};

static PRISM: LazyLock<PrismServer> =
    LazyLock::new(|| PrismServer::start("tests/contract/specs/confluence-v2.patched.json"));

static SETUP: LazyLock<(TestConfig, AtlRunner)> = LazyLock::new(|| {
    let config = TestConfigBuilder::new()
        .confluence(PRISM.base_url())
        .confluence_api_path("/wiki/rest/api")
        .build();
    let runner = AtlRunner::new(&config.config_path);
    (config, runner)
});

fn runner() -> &'static AtlRunner {
    &SETUP.1
}

static SETUP_RO: LazyLock<(TestConfig, AtlRunner)> = LazyLock::new(|| {
    let config = TestConfigBuilder::new()
        .confluence(PRISM.base_url())
        .confluence_api_path("/wiki/rest/api")
        .read_only(true)
        .build();
    let runner = AtlRunner::new(&config.config_path);
    (config, runner)
});

fn runner_ro() -> &'static AtlRunner {
    &SETUP_RO.1
}

// -- Pages --

#[test]
#[ignore]
fn page_read() {
    runner().run_ok(&["conf", "read", "12345"]);
}

#[test]
#[ignore]
fn page_read_view_format() {
    runner().run_ok(&["conf", "read", "12345", "--body-format", "view"]);
}

#[test]
#[ignore]
fn page_info() {
    runner().run_ok(&["conf", "info", "12345"]);
}

#[test]
#[ignore]
fn page_create_positive() {
    runner().run_ok(&["conf", "create", "-s", "TEST", "-t", "Title", "-b", "body"]);
}

#[test]
#[ignore]
fn page_create_read_only() {
    runner_ro().run_err(
        &["conf", "create", "-s", "TEST", "-t", "Title", "-b", "body"],
        3,
    );
}

#[test]
#[ignore]
fn page_create_markdown() {
    runner().run_ok(&[
        "conf",
        "create",
        "-s",
        "TEST",
        "-t",
        "T",
        "--input-format",
        "markdown",
        "-b",
        "# H",
    ]);
}

#[test]
#[ignore]
fn page_create_with_parent() {
    runner().run_ok(&[
        "conf", "create", "-s", "TEST", "-t", "T", "-b", "b", "--parent", "111",
    ]);
}

#[test]
#[ignore]
fn page_update_positive() {
    runner().run_ok(&[
        "conf",
        "update",
        "12345",
        "-t",
        "T",
        "-b",
        "B",
        "--version",
        "2",
    ]);
}

#[test]
#[ignore]
fn page_update_read_only() {
    runner_ro().run_err(
        &[
            "conf",
            "update",
            "12345",
            "-t",
            "T",
            "-b",
            "B",
            "--version",
            "2",
        ],
        3,
    );
}

#[test]
#[ignore]
fn page_delete_positive() {
    runner().run_ok(&["conf", "delete", "12345"]);
}

#[test]
#[ignore]
fn page_delete_read_only() {
    runner_ro().run_err(&["conf", "delete", "12345"], 3);
}

#[test]
#[ignore]
fn page_delete_purge() {
    runner().run_ok(&["conf", "delete", "12345", "--purge"]);
}

#[test]
#[ignore]
fn page_children() {
    runner().run_ok(&["conf", "children", "12345"]);
}

#[test]
#[ignore]
fn page_children_depth() {
    runner().run_ok(&["conf", "children", "12345", "--depth", "2"]);
}

#[test]
#[ignore]
fn page_list() {
    runner().run_ok(&["conf", "page-list"]);
}

// `conf find` uses the v1 search endpoint (/content/search) and is covered by
// contract_confluence_v1.rs.

#[test]
#[ignore]
fn page_versions() {
    runner().run_ok(&["conf", "versions", "12345"]);
}

#[test]
#[ignore]
fn page_version_detail() {
    runner().run_ok(&["conf", "version-detail", "12345", "2"]);
}

// `conf likes 12345` was removed — the v2 spec has no `/pages/{id}/likes`
// endpoint, only `/likes/count` and `/likes/users`. Use `likes-count` or
// `likes-users` instead.

#[test]
#[ignore]
fn page_likes_count() {
    runner().run_ok(&["conf", "likes-count", "12345"]);
}

#[test]
#[ignore]
fn page_likes_users() {
    runner().run_ok(&["conf", "likes-users", "12345"]);
}

#[test]
#[ignore]
fn page_operations() {
    runner().run_ok(&["conf", "operations", "12345"]);
}

#[test]
#[ignore]
fn page_ancestors() {
    runner().run_ok(&["conf", "ancestors", "12345"]);
}

#[test]
#[ignore]
fn page_descendants() {
    runner().run_ok(&["conf", "descendants", "12345"]);
}

#[test]
#[ignore]
fn page_custom_content() {
    runner().run_ok(&["conf", "page-custom-content", "12345", "-t", "mytype"]);
}

#[test]
#[ignore]
fn page_update_title_positive() {
    runner().run_ok(&[
        "conf",
        "update-title",
        "12345",
        "-t",
        "New",
        "--version",
        "2",
    ]);
}

#[test]
#[ignore]
fn page_redact_positive() {
    runner().run_ok(&["conf", "redact", "12345"]);
}

// -- Properties --

#[test]
#[ignore]
fn property_list() {
    runner().run_ok(&["conf", "property", "list", "12345"]);
}

#[test]
#[ignore]
fn property_get() {
    runner().run_ok(&["conf", "property", "get", "12345", "mykey"]);
}

#[test]
#[ignore]
fn property_set_positive() {
    runner().run_ok(&[
        "conf",
        "property",
        "set",
        "12345",
        "mykey",
        "--value",
        "{\"v\":1}",
    ]);
}

#[test]
#[ignore]
fn property_set_read_only() {
    runner_ro().run_err(
        &[
            "conf",
            "property",
            "set",
            "12345",
            "mykey",
            "--value",
            "{\"v\":1}",
        ],
        3,
    );
}

#[test]
#[ignore]
fn property_delete_positive() {
    runner().run_ok(&["conf", "property", "delete", "12345", "mykey"]);
}

#[test]
#[ignore]
fn property_delete_read_only() {
    runner_ro().run_err(&["conf", "property", "delete", "12345", "mykey"], 3);
}

// -- Attachments --

#[test]
#[ignore]
fn attachment_list() {
    runner().run_ok(&["conf", "attachment", "list", "12345"]);
}

#[test]
#[ignore]
fn attachment_list_filtered() {
    runner().run_ok(&[
        "conf",
        "attachment",
        "list",
        "12345",
        "--media-type",
        "image/png",
    ]);
}

#[test]
#[ignore]
fn attachment_delete_positive() {
    runner().run_ok(&["conf", "attachment", "delete", "67890"]);
}

#[test]
#[ignore]
fn attachment_delete_read_only() {
    runner_ro().run_err(&["conf", "attachment", "delete", "67890"], 3);
}

// -- Footer Comments --

#[test]
#[ignore]
fn footer_comment_list_on_page() {
    runner().run_ok(&["conf", "footer-comment", "list", "12345"]);
}

#[test]
#[ignore]
fn footer_comment_get() {
    runner().run_ok(&["conf", "footer-comment", "get", "999"]);
}

#[test]
#[ignore]
fn footer_comment_create_positive() {
    runner().run_ok(&["conf", "footer-comment", "create", "12345", "-b", "text"]);
}

#[test]
#[ignore]
fn footer_comment_create_read_only() {
    runner_ro().run_err(
        &["conf", "footer-comment", "create", "12345", "-b", "text"],
        3,
    );
}

#[test]
#[ignore]
fn footer_comment_update_positive() {
    runner().run_ok(&[
        "conf",
        "footer-comment",
        "update",
        "999",
        "--body",
        "new",
        "--version",
        "2",
    ]);
}

#[test]
#[ignore]
fn footer_comment_delete_positive() {
    runner().run_ok(&["conf", "footer-comment", "delete", "999"]);
}

#[test]
#[ignore]
fn footer_comment_delete_read_only() {
    runner_ro().run_err(&["conf", "footer-comment", "delete", "999"], 3);
}

#[test]
#[ignore]
fn footer_comment_children() {
    runner().run_ok(&["conf", "footer-comment", "children", "999"]);
}

#[test]
#[ignore]
fn footer_comment_versions() {
    runner().run_ok(&["conf", "footer-comment", "versions", "999"]);
}

// -- Inline Comments --

#[test]
#[ignore]
fn inline_comment_list_on_page() {
    runner().run_ok(&["conf", "inline-comment", "list", "12345"]);
}

#[test]
#[ignore]
fn inline_comment_get() {
    runner().run_ok(&["conf", "inline-comment", "get", "999"]);
}

#[test]
#[ignore]
fn inline_comment_create_positive() {
    runner().run_ok(&[
        "conf",
        "inline-comment",
        "create",
        "12345",
        "--body",
        "text",
        "--inline-marker-ref",
        "marker-1",
    ]);
}

#[test]
#[ignore]
fn inline_comment_create_read_only() {
    runner_ro().run_err(
        &[
            "conf",
            "inline-comment",
            "create",
            "12345",
            "--body",
            "text",
            "--inline-marker-ref",
            "marker-1",
        ],
        3,
    );
}

#[test]
#[ignore]
fn inline_comment_delete_positive() {
    runner().run_ok(&["conf", "inline-comment", "delete", "999"]);
}

#[test]
#[ignore]
fn inline_comment_children() {
    runner().run_ok(&["conf", "inline-comment", "children", "999"]);
}

#[test]
#[ignore]
fn inline_comment_versions() {
    runner().run_ok(&["conf", "inline-comment", "versions", "999"]);
}

// -- Blog Posts --

#[test]
#[ignore]
fn blog_list() {
    runner().run_ok(&["conf", "blog", "list"]);
}

#[test]
#[ignore]
fn blog_list_by_space() {
    runner().run_ok(&["conf", "blog", "list", "--space", "TEST"]);
}

#[test]
#[ignore]
fn blog_read() {
    runner().run_ok(&["conf", "blog", "read", "12345"]);
}

#[test]
#[ignore]
fn blog_create_positive() {
    runner().run_ok(&["conf", "blog", "create", "-s", "TEST", "-t", "T", "-b", "B"]);
}

#[test]
#[ignore]
fn blog_create_read_only() {
    runner_ro().run_err(
        &["conf", "blog", "create", "-s", "TEST", "-t", "T", "-b", "B"],
        3,
    );
}

#[test]
#[ignore]
fn blog_update_positive() {
    runner().run_ok(&[
        "conf",
        "blog",
        "update",
        "12345",
        "-t",
        "T",
        "-b",
        "B",
        "--version",
        "2",
    ]);
}

#[test]
#[ignore]
fn blog_delete_positive() {
    runner().run_ok(&["conf", "blog", "delete", "12345"]);
}

#[test]
#[ignore]
fn blog_attachments() {
    runner().run_ok(&["conf", "blog", "attachments", "12345"]);
}

#[test]
#[ignore]
fn blog_labels() {
    runner().run_ok(&["conf", "blog", "labels", "12345"]);
}

#[test]
#[ignore]
fn blog_footer_comments() {
    runner().run_ok(&["conf", "blog", "footer-comments", "12345"]);
}

#[test]
#[ignore]
fn blog_versions() {
    runner().run_ok(&["conf", "blog", "versions", "12345"]);
}

// -- Spaces --

#[test]
#[ignore]
fn space_list() {
    runner().run_ok(&["conf", "space", "list"]);
}

#[test]
#[ignore]
fn space_get() {
    runner().run_ok(&["conf", "space", "get", "12345"]);
}

#[test]
#[ignore]
fn space_create_positive() {
    runner().run_ok(&["conf", "space", "create", "-k", "TEST", "-n", "Test"]);
}

#[test]
#[ignore]
fn space_create_read_only() {
    runner_ro().run_err(&["conf", "space", "create", "-k", "TEST", "-n", "Test"], 3);
}

// `conf space delete` was removed — Confluence v2 spec has GET only for
// `/spaces/{id}`; space deletion is not supported via v2.

#[test]
#[ignore]
fn space_pages() {
    runner().run_ok(&["conf", "space", "pages", "12345"]);
}

#[test]
#[ignore]
fn space_blogposts() {
    runner().run_ok(&["conf", "space", "blogposts", "12345"]);
}

#[test]
#[ignore]
fn space_labels() {
    runner().run_ok(&["conf", "space", "labels", "12345"]);
}

#[test]
#[ignore]
fn space_permissions() {
    runner().run_ok(&["conf", "space", "permissions", "12345"]);
}

#[test]
#[ignore]
fn space_content_labels() {
    runner().run_ok(&["conf", "space", "content-labels", "12345"]);
}

#[test]
#[ignore]
fn space_custom_content() {
    runner().run_ok(&["conf", "space", "custom-content", "12345", "-t", "mytype"]);
}

#[test]
#[ignore]
fn space_operations() {
    runner().run_ok(&["conf", "space", "operations", "12345"]);
}

// -- Whiteboards --

#[test]
#[ignore]
fn whiteboard_create_positive() {
    runner().run_ok(&[
        "conf",
        "whiteboard",
        "create",
        "--space-id",
        "12345",
        "--title",
        "W",
    ]);
}

#[test]
#[ignore]
fn whiteboard_get() {
    runner().run_ok(&["conf", "whiteboard", "get", "123"]);
}

#[test]
#[ignore]
fn whiteboard_delete_positive() {
    runner().run_ok(&["conf", "whiteboard", "delete", "123"]);
}

// -- Databases --

#[test]
#[ignore]
fn database_create_positive() {
    runner().run_ok(&[
        "conf",
        "database",
        "create",
        "--space-id",
        "12345",
        "--title",
        "W",
    ]);
}

#[test]
#[ignore]
fn database_get() {
    runner().run_ok(&["conf", "database", "get", "123"]);
}

#[test]
#[ignore]
fn database_delete_positive() {
    runner().run_ok(&["conf", "database", "delete", "123"]);
}

// -- Folders --

#[test]
#[ignore]
fn folder_create_positive() {
    runner().run_ok(&[
        "conf",
        "folder",
        "create",
        "--space-id",
        "12345",
        "--title",
        "W",
    ]);
}

#[test]
#[ignore]
fn folder_get() {
    runner().run_ok(&["conf", "folder", "get", "123"]);
}

#[test]
#[ignore]
fn folder_delete_positive() {
    runner().run_ok(&["conf", "folder", "delete", "123"]);
}

// -- Smart Links --

// SmartLink feature removed from atl — not defined in Confluence v2 spec.

// -- Custom Content --

#[test]
#[ignore]
fn custom_content_list() {
    runner().run_ok(&["conf", "custom-content", "list"]);
}

#[test]
#[ignore]
fn custom_content_get() {
    runner().run_ok(&["conf", "custom-content", "get", "123"]);
}

#[test]
#[ignore]
fn custom_content_create_positive() {
    runner().run_ok(&[
        "conf",
        "custom-content",
        "create",
        "--content-type",
        "mytype",
        "--space-id",
        "12345",
        "--title",
        "T",
        "--body",
        "B",
    ]);
}

#[test]
#[ignore]
fn custom_content_update_positive() {
    runner().run_ok(&[
        "conf",
        "custom-content",
        "update",
        "123",
        "--title",
        "T",
        "--body",
        "new",
        "--version",
        "2",
    ]);
}

#[test]
#[ignore]
fn custom_content_delete_positive() {
    runner().run_ok(&["conf", "custom-content", "delete", "123"]);
}

// -- Tasks --

#[test]
#[ignore]
fn task_list() {
    runner().run_ok(&["conf", "task", "list"]);
}

#[test]
#[ignore]
fn task_get() {
    runner().run_ok(&["conf", "task", "get", "123"]);
}

// -- Admin --

#[test]
#[ignore]
fn admin_key_get() {
    runner().run_ok(&["conf", "admin-key", "get"]);
}

// data-policy endpoints removed — not in v2 spec.

#[test]
#[ignore]
fn classification_list() {
    runner().run_ok(&["conf", "classification", "list"]);
}

#[test]
#[ignore]
fn classification_get_page() {
    runner().run_ok(&["conf", "classification", "get-page", "12345"]);
}

// -- Users --

#[test]
#[ignore]
fn user_bulk() {
    runner().run_ok(&["conf", "user", "bulk", "acc1", "acc2"]);
}

#[test]
#[ignore]
fn user_check_access() {
    runner().run_ok(&["conf", "user", "check-access", "test@example.com"]);
}

// -- Misc --

#[test]
#[ignore]
fn convert_ids() {
    runner().run_ok(&["conf", "convert-ids", "123", "456"]);
}

#[test]
#[ignore]
fn app_property_list() {
    runner().run_ok(&["conf", "app-property", "list"]);
}
