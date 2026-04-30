use camino::Utf8Path;
use serde_json::Value;

use crate::cli::args::*;
use crate::client::ConfluenceClient;

use super::property::dispatch_resource_property;

/// Filter the `results` array of an attachment-list payload by glob pattern,
/// matched against the per-item `title` field.
///
/// - Items without a string `title` are dropped.
/// - If the value has no `results` array, it is returned unchanged.
/// - Returns an error if `pattern_str` is not a valid glob.
pub(super) fn filter_attachments_by_pattern(
    mut value: Value,
    pattern_str: &str,
) -> anyhow::Result<Value> {
    let pattern = glob::Pattern::new(pattern_str)
        .map_err(|e| anyhow::anyhow!("invalid glob pattern: {e}"))?;
    if let Some(results) = value.get_mut("results").and_then(Value::as_array_mut) {
        results.retain(|item| {
            item.get("title")
                .and_then(Value::as_str)
                .map(|title| pattern.matches(title))
                .unwrap_or(false)
        });
    }
    Ok(value)
}

/// Build the success message returned to the user after a download.
///
/// If `output` is `Some`, the message reports the path the bytes were written
/// to; otherwise it reports the byte count of the in-memory payload.
pub(super) fn download_result_message(output: Option<&Utf8Path>, byte_len: usize) -> String {
    match output {
        Some(path) => format!("Downloaded to {path}"),
        None => format!("Downloaded {byte_len} bytes"),
    }
}

pub(super) async fn dispatch_attachment(
    cmd: &ConfluenceAttachmentSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceAttachmentSubcommand::List(args) => {
            let value = client
                .get_attachments(
                    &args.page_id,
                    args.limit,
                    args.media_type.as_deref(),
                    args.filename.as_deref(),
                )
                .await?;
            if let Some(pattern_str) = &args.pattern {
                filter_attachments_by_pattern(value, pattern_str)?
            } else {
                value
            }
        }
        ConfluenceAttachmentSubcommand::Get(args) => {
            client.get_attachment_v2(&args.attachment_id).await?
        }
        ConfluenceAttachmentSubcommand::Upload(args) => {
            client
                .upload_attachment(&args.page_id, args.file.as_path())
                .await?
        }
        ConfluenceAttachmentSubcommand::Delete(args) => {
            client.delete_attachment(&args.attachment_id).await?;
            Value::String(format!("Attachment {} deleted", args.attachment_id))
        }
        ConfluenceAttachmentSubcommand::Download(args) => {
            let bytes = client
                .download_attachment(&args.page_id, &args.attachment_id)
                .await?;
            if let Some(output) = &args.output {
                std::fs::write(output.as_std_path(), &bytes)?;
            }
            Value::String(download_result_message(args.output.as_deref(), bytes.len()))
        }
        ConfluenceAttachmentSubcommand::Labels(args) => {
            client
                .get_attachment_labels_v2(&args.attachment_id, args.limit)
                .await?
        }
        ConfluenceAttachmentSubcommand::Comments(args) => {
            client
                .get_attachment_comments_v2(&args.attachment_id, args.limit)
                .await?
        }
        ConfluenceAttachmentSubcommand::Operations(args) => {
            client
                .get_attachment_operations_v2(&args.attachment_id)
                .await?
        }
        ConfluenceAttachmentSubcommand::Versions(args) => {
            client
                .get_attachment_versions_v2(&args.attachment_id, args.limit)
                .await?
        }
        ConfluenceAttachmentSubcommand::VersionDetails(args) => {
            client
                .get_attachment_version_v2(&args.attachment_id, args.version)
                .await?
        }
        ConfluenceAttachmentSubcommand::Property(cmd) => {
            dispatch_resource_property("attachments", &cmd.command, client).await?
        }
    })
}

#[cfg(test)]
mod tests {
    // Most arms are pure HTTP delegation and are covered by contract tests in
    // tests/contract_confluence_v*.rs. Only the small pure helpers below
    // (`filter_attachments_by_pattern` and `download_result_message`) live in
    // process and are unit-tested here.

    use camino::Utf8PathBuf;
    use serde_json::json;

    use super::*;

    // ---- filter_attachments_by_pattern ----

    #[test]
    fn filter_keeps_matching_titles() {
        let value = json!({
            "results": [
                { "title": "report.pdf" },
                { "title": "diagram.png" },
                { "title": "summary.pdf" },
            ]
        });
        let filtered = filter_attachments_by_pattern(value, "*.pdf").unwrap();
        let results = filtered["results"].as_array().expect("results array");
        assert_eq!(
            results.len(),
            2,
            "two .pdf items should remain: {results:?}"
        );
        let titles: Vec<&str> = results
            .iter()
            .map(|v| v["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, vec!["report.pdf", "summary.pdf"]);
    }

    #[test]
    fn filter_drops_items_without_string_title() {
        let value = json!({
            "results": [
                { "title": "ok.pdf" },
                { "title": 42 },         // not a string -> dropped
                {},                       // no title -> dropped
            ]
        });
        let filtered = filter_attachments_by_pattern(value, "*.pdf").unwrap();
        let results = filtered["results"].as_array().expect("results array");
        assert_eq!(
            results.len(),
            1,
            "items without string title should be dropped: {results:?}"
        );
        assert_eq!(results[0]["title"].as_str(), Some("ok.pdf"));
    }

    #[test]
    fn filter_passes_through_when_no_results_array() {
        // No `results` key at all -> value returned unchanged.
        let value = json!({ "other": "data" });
        let filtered = filter_attachments_by_pattern(value.clone(), "*.pdf").unwrap();
        assert_eq!(filtered, value);
    }

    #[test]
    fn filter_returns_error_for_invalid_glob() {
        let value = json!({ "results": [] });
        let err = filter_attachments_by_pattern(value, "[invalid").unwrap_err();
        assert!(
            err.to_string().contains("invalid glob pattern"),
            "error should mention invalid glob, got: {err}"
        );
    }

    #[test]
    fn filter_empty_pattern_matches_only_empty_titles() {
        // Empty glob pattern matches only the empty string.
        let value = json!({
            "results": [
                { "title": "" },
                { "title": "x" },
            ]
        });
        let filtered = filter_attachments_by_pattern(value, "").unwrap();
        let results = filtered["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["title"].as_str(), Some(""));
    }

    // ---- download_result_message ----

    #[test]
    fn download_message_with_output_path() {
        let path = Utf8PathBuf::from("/tmp/file.pdf");
        let msg = download_result_message(Some(path.as_path()), 1024);
        assert_eq!(msg, "Downloaded to /tmp/file.pdf");
    }

    #[test]
    fn download_message_without_output_reports_bytes() {
        let msg = download_result_message(None, 4096);
        assert_eq!(msg, "Downloaded 4096 bytes");
    }

    #[test]
    fn download_message_zero_bytes() {
        let msg = download_result_message(None, 0);
        assert_eq!(msg, "Downloaded 0 bytes");
    }
}
