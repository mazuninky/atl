use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

use super::property::dispatch_resource_property;

pub(super) async fn dispatch_content_type(
    type_name: &str,
    cmd: &ConfluenceContentTypeSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceContentTypeSubcommand::Create(args) => {
            let mut payload = serde_json::json!({ "spaceId": args.space_id });
            if let Some(t) = &args.title {
                payload["title"] = Value::String(t.clone());
            }
            if let Some(p) = &args.parent_id {
                payload["parentId"] = Value::String(p.clone());
            }
            if let Some(tk) = &args.template_key {
                payload["templateKey"] = Value::String(tk.clone());
            }
            client.create_content_type_v2(type_name, &payload).await?
        }
        ConfluenceContentTypeSubcommand::Get(args) => {
            client.get_content_type_v2(type_name, &args.id).await?
        }
        ConfluenceContentTypeSubcommand::Delete(args) => {
            client.delete_content_type_v2(type_name, &args.id).await?;
            Value::String(format!("{type_name} {} deleted", args.id))
        }
        ConfluenceContentTypeSubcommand::Ancestors(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "ancestors", 200)
                .await?
        }
        ConfluenceContentTypeSubcommand::Descendants(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "descendants", args.limit)
                .await?
        }
        ConfluenceContentTypeSubcommand::Children(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "children", args.limit)
                .await?
        }
        ConfluenceContentTypeSubcommand::Operations(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "operations", 200)
                .await?
        }
        ConfluenceContentTypeSubcommand::Property(cmd) => match &cmd.command {
            ConfluenceContentTypePropertySubcommand::List(args) => {
                client
                    .get_content_type_sub_v2(type_name, &args.id, "properties", 200)
                    .await?
            }
            ConfluenceContentTypePropertySubcommand::Get(args) => {
                client
                    .get_content_type_property_v2(type_name, &args.id, &args.key)
                    .await?
            }
            ConfluenceContentTypePropertySubcommand::Set(args) => {
                let value_str = read_body_arg(&args.value)?;
                let value: Value =
                    serde_json::from_str(&value_str).unwrap_or(Value::String(value_str));
                client
                    .set_content_type_property_v2(type_name, &args.id, &args.key, &value)
                    .await?
            }
            ConfluenceContentTypePropertySubcommand::Delete(args) => {
                client
                    .delete_content_type_property_v2(type_name, &args.id, &args.key)
                    .await?;
                Value::String(format!("Property '{}' deleted", args.key))
            }
        },
    })
}

pub(super) async fn dispatch_custom_content(
    cmd: &ConfluenceCustomContentSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceCustomContentSubcommand::List(args) => {
            client
                .list_custom_content_v2(
                    args.content_type.as_deref(),
                    args.space_id.as_deref(),
                    args.limit,
                )
                .await?
        }
        ConfluenceCustomContentSubcommand::Get(args) => {
            client.get_custom_content_v2(&args.id).await?
        }
        ConfluenceCustomContentSubcommand::Create(args) => {
            let body = read_body_arg(&args.body)?;
            let payload = serde_json::json!({
                "type": args.content_type,
                "spaceId": args.space_id,
                "title": args.title,
                "body": { "storage": { "value": body, "representation": "storage" } }
            });
            client.create_custom_content_v2(&payload).await?
        }
        ConfluenceCustomContentSubcommand::Update(args) => {
            let mut payload = serde_json::json!({ "version": { "number": args.version } });
            if let Some(t) = &args.title {
                payload["title"] = Value::String(t.clone());
            }
            if let Some(b) = &args.body {
                let body = read_body_arg(b)?;
                payload["body"] = serde_json::json!({ "storage": { "value": body, "representation": "storage" } });
            }
            client.update_custom_content_v2(&args.id, &payload).await?
        }
        ConfluenceCustomContentSubcommand::Delete(args) => {
            client.delete_custom_content_v2(&args.id).await?;
            Value::String(format!("Custom content {} deleted", args.id))
        }
        ConfluenceCustomContentSubcommand::Attachments(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "attachments", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::Children(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "children", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::Labels(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "labels", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::Comments(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "comments", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::Operations(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "operations", 200)
                .await?
        }
        ConfluenceCustomContentSubcommand::Versions(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "versions", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::VersionDetails(args) => {
            client
                .get_custom_content_version_v2(&args.id, args.version)
                .await?
        }
        ConfluenceCustomContentSubcommand::Property(cmd) => {
            dispatch_resource_property("custom-content", &cmd.command, client).await?
        }
    })
}
