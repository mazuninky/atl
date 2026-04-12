mod common;

use std::io::Write;
use std::sync::LazyLock;

use common::{AtlRunner, PrismServer, TestConfig, TestConfigBuilder};
use tempfile::NamedTempFile;

static PRISM: LazyLock<PrismServer> =
    LazyLock::new(|| PrismServer::start("tests/contract/specs/confluence.patched.json"));

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

#[test]
#[ignore]
fn conf_search_positive() {
    runner().run_ok(&["conf", "search", "type=page"]);
}

#[test]
#[ignore]
fn conf_search_with_limit() {
    runner().run_ok(&["conf", "search", "space=DEV", "--limit", "5"]);
}

#[test]
#[ignore]
fn conf_search_all() {
    runner().run_ok(&["conf", "search", "type=page", "--all"]);
}

#[test]
#[ignore]
fn conf_attachment_download_positive() {
    let tmp = NamedTempFile::new().unwrap();
    let output_path = tmp.path().to_str().unwrap();
    runner().run_ok(&[
        "conf",
        "attachment",
        "download",
        "12345",
        "--page-id",
        "99999",
        "--output",
        output_path,
    ]);
}

#[test]
#[ignore]
fn conf_attachment_upload_positive() {
    let mut tmp = NamedTempFile::new().unwrap();
    writeln!(tmp, "test content").unwrap();
    let file_path = tmp.path().to_str().unwrap();
    runner().run_ok(&["conf", "attachment", "upload", "12345", "-f", file_path]);
}

#[test]
#[ignore]
fn conf_attachment_upload_read_only() {
    // Read-only check runs entirely in atl before the HTTP call, so this test
    // is safe even though the actual upload body violates the spec.
    let mut tmp = NamedTempFile::new().unwrap();
    writeln!(tmp, "test content").unwrap();
    let file_path = tmp.path().to_str().unwrap();
    runner_ro().run_err(
        &["conf", "attachment", "upload", "12345", "-f", file_path],
        3,
    );
}

#[test]
#[ignore]
fn conf_label_add_positive() {
    runner().run_ok(&["conf", "label", "add", "12345", "tag1", "tag2"]);
}

#[test]
#[ignore]
fn conf_label_add_read_only() {
    runner_ro().run_err(&["conf", "label", "add", "12345", "tag1"], 3);
}

#[test]
#[ignore]
fn conf_label_remove_positive() {
    runner().run_ok(&["conf", "label", "remove", "12345", "tag1"]);
}

#[test]
#[ignore]
fn conf_label_remove_read_only() {
    runner_ro().run_err(&["conf", "label", "remove", "12345", "tag1"], 3);
}
