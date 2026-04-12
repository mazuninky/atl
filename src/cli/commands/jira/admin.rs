use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;
use crate::error::Error;

use super::today_date;

pub(super) fn parse_gadget_position(raw: &str) -> anyhow::Result<Value> {
    let (row, col) = raw
        .split_once(':')
        .ok_or_else(|| Error::InvalidInput("invalid --position; expected ROW:COL".into()))?;
    let row: u32 = row
        .parse()
        .map_err(|_| Error::InvalidInput("invalid --position row; expected u32".into()))?;
    let column: u32 = col
        .parse()
        .map_err(|_| Error::InvalidInput("invalid --position column; expected u32".into()))?;
    Ok(json!({ "row": row, "column": column }))
}

pub(super) async fn dispatch_component(
    cmd: &JiraComponentSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraComponentSubcommand::List(args) => {
            client.get_project_components(&args.project_key).await?
        }
        JiraComponentSubcommand::Get(args) => client.get_component(&args.id).await?,
        JiraComponentSubcommand::Create(args) => {
            let mut payload = json!({
                "project": &args.project,
                "name": &args.name,
            });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            if let Some(lead) = &args.lead {
                payload["leadAccountId"] = Value::String(lead.clone());
            }
            client.create_component(&payload).await?
        }
        JiraComponentSubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(desc) = &args.description {
                fields.insert("description".into(), Value::String(desc.clone()));
            }
            if let Some(lead) = &args.lead {
                fields.insert("leadAccountId".into(), Value::String(lead.clone()));
            }
            if let Some(at) = &args.assignee_type {
                fields.insert("assigneeType".into(), Value::String(at.clone()));
            }
            if fields.is_empty() {
                return Err(Error::InvalidInput(
                    "no fields to update; specify at least one of --name, --description, --lead, --assignee-type"
                        .into(),
                )
                .into());
            }
            client
                .update_component(&args.id, &Value::Object(fields))
                .await?
        }
        JiraComponentSubcommand::Delete(args) => {
            client.delete_component(&args.id).await?;
            Value::String(format!("Component {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_version(
    cmd: &JiraVersionSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraVersionSubcommand::List(args) => client.get_project_versions(&args.project_key).await?,
        JiraVersionSubcommand::Get(args) => client.get_version(&args.id).await?,
        JiraVersionSubcommand::Create(args) => {
            let mut payload = json!({
                "project": &args.project,
                "name": &args.name,
            });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            if let Some(date) = &args.release_date {
                payload["releaseDate"] = Value::String(date.clone());
            }
            client.create_version(&payload).await?
        }
        JiraVersionSubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(desc) = &args.description {
                fields.insert("description".into(), Value::String(desc.clone()));
            }
            if let Some(sd) = &args.start_date {
                fields.insert("startDate".into(), Value::String(sd.clone()));
            }
            if let Some(rd) = &args.release_date {
                fields.insert("releaseDate".into(), Value::String(rd.clone()));
            }
            if let Some(r) = args.released {
                fields.insert("released".into(), Value::Bool(r));
            }
            if let Some(a) = args.archived {
                fields.insert("archived".into(), Value::Bool(a));
            }
            if fields.is_empty() {
                return Err(Error::InvalidInput(
                    "no fields to update; specify at least one of --name, --description, --start-date, --release-date, --released, --archived"
                        .into(),
                )
                .into());
            }
            client
                .update_version(&args.id, &Value::Object(fields))
                .await?
        }
        JiraVersionSubcommand::Delete(args) => {
            client.delete_version(&args.id).await?;
            Value::String(format!("Version {} deleted", args.id))
        }
        JiraVersionSubcommand::Release(args) => {
            let date = args.date.clone().unwrap_or_else(today_date);
            client.release_version(&args.id, &date).await?
        }
    })
}

pub(super) async fn dispatch_dashboard(
    cmd: &JiraDashboardSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraDashboardSubcommand::List => client.list_dashboards().await?,
        JiraDashboardSubcommand::Get(args) => client.get_dashboard(&args.id).await?,
        JiraDashboardSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_dashboard(&payload).await?
        }
        JiraDashboardSubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(desc) = &args.description {
                fields.insert("description".into(), Value::String(desc.clone()));
            }
            if fields.is_empty() {
                return Err(Error::InvalidInput(
                    "no fields to update; specify at least one of --name, --description".into(),
                )
                .into());
            }
            client
                .update_dashboard(&args.id, &Value::Object(fields))
                .await?
        }
        JiraDashboardSubcommand::Delete(args) => {
            client.delete_dashboard(&args.id).await?;
            Value::String(format!("Dashboard {} deleted", args.id))
        }
        JiraDashboardSubcommand::Copy(args) => {
            let mut payload = serde_json::Map::new();
            if let Some(name) = &args.name {
                payload.insert("name".into(), Value::String(name.clone()));
            }
            client
                .copy_dashboard(&args.id, &Value::Object(payload))
                .await?
        }
        JiraDashboardSubcommand::Gadgets(args) => client.list_dashboard_gadgets(&args.id).await?,
        JiraDashboardSubcommand::AddGadget(args) => {
            let mut payload = json!({ "uri": &args.uri });
            if let Some(color) = &args.color {
                payload["color"] = Value::String(color.clone());
            }
            if let Some(pos) = &args.position {
                payload["position"] = parse_gadget_position(pos)?;
            }
            client
                .add_dashboard_gadget(&args.dashboard_id, &payload)
                .await?
        }
        JiraDashboardSubcommand::UpdateGadget(args) => {
            let mut payload = json!({});
            if let Some(color) = &args.color {
                payload["color"] = Value::String(color.clone());
            }
            if let Some(pos) = &args.position {
                payload["position"] = parse_gadget_position(pos)?;
            }
            if payload.as_object().map(|m| m.is_empty()).unwrap_or(true) {
                return Err(Error::InvalidInput(
                    "update-gadget requires at least one of --color or --position".into(),
                )
                .into());
            }
            client
                .update_dashboard_gadget(&args.dashboard_id, &args.gadget_id, &payload)
                .await?
        }
        JiraDashboardSubcommand::RemoveGadget(args) => {
            client
                .remove_dashboard_gadget(&args.dashboard_id, &args.gadget_id)
                .await?;
            Value::String(format!(
                "Gadget {} removed from dashboard {}",
                args.gadget_id, args.dashboard_id
            ))
        }
    })
}

pub(super) async fn dispatch_link_type(
    cmd: &JiraLinkTypeSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraLinkTypeSubcommand::List => client.get_issue_link_types().await?,
        JiraLinkTypeSubcommand::Get(args) => client.get_issue_link_type(&args.id).await?,
        JiraLinkTypeSubcommand::Create(args) => {
            let payload = json!({
                "name": &args.name,
                "inward": &args.inward,
                "outward": &args.outward,
            });
            client.create_issue_link_type(&payload).await?
        }
        JiraLinkTypeSubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(inward) = &args.inward {
                fields.insert("inward".into(), Value::String(inward.clone()));
            }
            if let Some(outward) = &args.outward {
                fields.insert("outward".into(), Value::String(outward.clone()));
            }
            if fields.is_empty() {
                return Err(Error::InvalidInput(
                    "no fields to update; specify at least one of --name, --inward, --outward"
                        .into(),
                )
                .into());
            }
            client
                .update_issue_link_type(&args.id, &Value::Object(fields))
                .await?
        }
        JiraLinkTypeSubcommand::Delete(args) => {
            client.delete_issue_link_type(&args.id).await?;
            Value::String(format!("Issue link type {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_role(
    cmd: &JiraRoleSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraRoleSubcommand::List => client.list_roles().await?,
        JiraRoleSubcommand::Get(args) => client.get_role(&args.id).await?,
        JiraRoleSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_role(&payload).await?
        }
        JiraRoleSubcommand::Delete(args) => {
            client.delete_role(&args.id).await?;
            Value::String(format!("Role {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_banner(
    cmd: &JiraBannerSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraBannerSubcommand::Get => client.get_banner().await?,
        JiraBannerSubcommand::Set(args) => {
            let mut payload = json!({ "message": &args.message });
            if let Some(enabled) = args.is_enabled {
                payload["isEnabled"] = Value::Bool(enabled);
            }
            if let Some(vis) = &args.visibility {
                payload["visibility"] = Value::String(vis.clone());
            }
            client.set_banner(&payload).await?
        }
    })
}

pub(super) async fn dispatch_task(
    cmd: &JiraTaskSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraTaskSubcommand::Get(args) => client.get_task(&args.id).await?,
        JiraTaskSubcommand::Cancel(args) => {
            client.cancel_task(&args.id).await?;
            Value::String(format!("Task {} cancelled", args.id))
        }
    })
}

pub(super) async fn dispatch_attachment_admin(
    cmd: &JiraAttachmentAdminSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraAttachmentAdminSubcommand::Get(args) => client.get_attachment(&args.id).await?,
        JiraAttachmentAdminSubcommand::Delete(args) => {
            client.delete_attachment(&args.id).await?;
            Value::String(format!("Attachment {} deleted", args.id))
        }
        JiraAttachmentAdminSubcommand::Meta => client.get_attachment_meta().await?,
    })
}

pub(super) async fn dispatch_project_category(
    cmd: &JiraProjectCategorySubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraProjectCategorySubcommand::List => client.list_project_categories().await?,
        JiraProjectCategorySubcommand::Get(args) => client.get_project_category(&args.id).await?,
        JiraProjectCategorySubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_project_category(&payload).await?
        }
        JiraProjectCategorySubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(desc) = &args.description {
                fields.insert("description".into(), Value::String(desc.clone()));
            }
            if fields.is_empty() {
                return Err(Error::InvalidInput(
                    "no fields to update; specify at least one of --name, --description".into(),
                )
                .into());
            }
            client
                .update_project_category(&args.id, &Value::Object(fields))
                .await?
        }
        JiraProjectCategorySubcommand::Delete(args) => {
            client.delete_project_category(&args.id).await?;
            Value::String(format!("Project category {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_webhook(
    cmd: &JiraWebhookSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraWebhookSubcommand::List => client.list_webhooks().await?,
        JiraWebhookSubcommand::Get(args) => client.get_webhook(&args.id).await?,
        JiraWebhookSubcommand::Create(args) => {
            let raw_events: Vec<&str> = args.events.split(',').map(str::trim).collect();
            if raw_events.iter().any(|e| e.is_empty()) {
                return Err(Error::InvalidInput(
                    "invalid --events; expected a comma-separated list of non-empty event names"
                        .into(),
                )
                .into());
            }
            let events: Vec<Value> = raw_events
                .into_iter()
                .map(|e| Value::String(e.to_owned()))
                .collect();
            let mut payload = json!({
                "name": &args.name,
                "url": &args.url,
                "events": events,
            });
            if let Some(jql) = &args.jql {
                payload["filters"] = json!({ "issue-related-events-section": jql });
            }
            client.create_webhook(&payload).await?
        }
        JiraWebhookSubcommand::Delete(args) => {
            client.delete_webhook(&args.id).await?;
            Value::String(format!("Webhook {} deleted", args.id))
        }
    })
}
