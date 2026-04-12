use serde_json::Value;

use crate::cli::args::*;
use crate::client::ConfluenceClient;

use super::property::dispatch_resource_property;

pub(super) async fn dispatch_attachment(
    cmd: &ConfluenceAttachmentSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceAttachmentSubcommand::List(args) => {
            let mut value = client
                .get_attachments(
                    &args.page_id,
                    args.limit,
                    args.media_type.as_deref(),
                    args.filename.as_deref(),
                )
                .await?;
            if let Some(pattern_str) = &args.pattern {
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
            }
            value
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
                Value::String(format!("Downloaded to {output}"))
            } else {
                Value::String(format!("Downloaded {} bytes", bytes.len()))
            }
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
