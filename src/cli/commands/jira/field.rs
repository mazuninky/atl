use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

pub(super) async fn dispatch_field(
    cmd: &JiraFieldSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraFieldSubcommand::List(args) => {
            let all = client.get_fields().await?;
            if args.custom {
                if let Some(arr) = all.as_array() {
                    let custom: Vec<&Value> = arr
                        .iter()
                        .filter(|f| f.get("custom").and_then(|v| v.as_bool()).unwrap_or(false))
                        .collect();
                    serde_json::to_value(custom)?
                } else {
                    all
                }
            } else {
                all
            }
        }
        JiraFieldSubcommand::Create(args) => {
            let mut payload = json!({
                "name": &args.name,
                "type": &args.r#type,
            });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            if let Some(sk) = &args.search_key {
                payload["searcherKey"] = Value::String(sk.clone());
            }
            client.create_field(&payload).await?
        }
        JiraFieldSubcommand::Delete(args) => {
            client.delete_field(&args.id).await?;
            Value::String(format!("Field {} deleted", args.id))
        }
        JiraFieldSubcommand::Trash(args) => {
            client.trash_field(&args.id).await?;
            Value::String(format!("Field {} moved to trash", args.id))
        }
        JiraFieldSubcommand::Restore(args) => {
            client.restore_field(&args.id).await?;
            Value::String(format!("Field {} restored", args.id))
        }
    })
}

pub(super) async fn dispatch_issue_type(
    cmd: &JiraIssueTypeSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraIssueTypeSubcommand::List => client.list_issue_types().await?,
        JiraIssueTypeSubcommand::Get(args) => client.get_issue_type(&args.id).await?,
        JiraIssueTypeSubcommand::Create(args) => {
            let mut payload = json!({
                "name": &args.name,
                "type": &args.r#type,
            });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_issue_type(&payload).await?
        }
        JiraIssueTypeSubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(desc) = &args.description {
                fields.insert("description".into(), Value::String(desc.clone()));
            }
            if fields.is_empty() {
                anyhow::bail!("no fields to update; specify at least one of --name, --description");
            }
            client
                .update_issue_type(&args.id, &Value::Object(fields))
                .await?
        }
        JiraIssueTypeSubcommand::Delete(args) => {
            client.delete_issue_type(&args.id).await?;
            Value::String(format!("Issue type {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_priority(
    cmd: &JiraPrioritySubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraPrioritySubcommand::List => client.list_priorities().await?,
        JiraPrioritySubcommand::Get(args) => client.get_priority(&args.id).await?,
        JiraPrioritySubcommand::Create(args) => {
            let mut payload = json!({
                "name": &args.name,
                "statusColor": &args.status_color,
            });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_priority(&payload).await?
        }
        JiraPrioritySubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(desc) = &args.description {
                fields.insert("description".into(), Value::String(desc.clone()));
            }
            if let Some(color) = &args.status_color {
                fields.insert("statusColor".into(), Value::String(color.clone()));
            }
            if fields.is_empty() {
                anyhow::bail!(
                    "no fields to update; specify at least one of --name, --description, --status-color"
                );
            }
            client
                .update_priority(&args.id, &Value::Object(fields))
                .await?
        }
        JiraPrioritySubcommand::Delete(args) => {
            client.delete_priority(&args.id).await?;
            Value::String(format!("Priority {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_resolution(
    cmd: &JiraResolutionSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraResolutionSubcommand::List => client.list_resolutions().await?,
        JiraResolutionSubcommand::Get(args) => client.get_resolution(&args.id).await?,
        JiraResolutionSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_resolution(&payload).await?
        }
        JiraResolutionSubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(desc) = &args.description {
                fields.insert("description".into(), Value::String(desc.clone()));
            }
            if fields.is_empty() {
                anyhow::bail!("no fields to update; specify at least one of --name, --description");
            }
            client
                .update_resolution(&args.id, &Value::Object(fields))
                .await?
        }
        JiraResolutionSubcommand::Delete(args) => {
            client.delete_resolution(&args.id).await?;
            Value::String(format!("Resolution {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_status(
    cmd: &JiraStatusSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraStatusSubcommand::List => client.list_statuses().await?,
        JiraStatusSubcommand::Get(args) => client.get_status(&args.id).await?,
        JiraStatusSubcommand::Categories => client.list_status_categories().await?,
    })
}
