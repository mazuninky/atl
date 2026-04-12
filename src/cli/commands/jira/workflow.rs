use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

pub(super) async fn dispatch_workflow(
    cmd: &JiraListGetSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraListGetSubcommand::List => client.list_workflows().await?,
        JiraListGetSubcommand::Get(args) => client.get_workflow(&args.id).await?,
    })
}

pub(super) async fn dispatch_workflow_scheme(
    cmd: &JiraCrudSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraCrudSubcommand::List => client.list_workflow_schemes().await?,
        JiraCrudSubcommand::Get(args) => client.get_workflow_scheme(&args.id).await?,
        JiraCrudSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_workflow_scheme(&payload).await?
        }
        JiraCrudSubcommand::Update(args) => {
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
                .update_workflow_scheme(&args.id, &Value::Object(fields))
                .await?
        }
        JiraCrudSubcommand::Delete(args) => {
            client.delete_workflow_scheme(&args.id).await?;
            Value::String(format!("Workflow scheme {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_screen(
    cmd: &JiraScreenSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraScreenSubcommand::List => client.list_screens().await?,
        JiraScreenSubcommand::Get(args) => client.get_screen(&args.id).await?,
        JiraScreenSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_screen(&payload).await?
        }
        JiraScreenSubcommand::Delete(args) => {
            client.delete_screen(&args.id).await?;
            Value::String(format!("Screen {} deleted", args.id))
        }
        JiraScreenSubcommand::Tabs(args) => client.get_screen_tabs(&args.id).await?,
        JiraScreenSubcommand::Fields(args) => {
            client
                .get_screen_tab_fields(&args.screen_id, &args.tab_id)
                .await?
        }
    })
}

pub(super) async fn dispatch_permission_scheme(
    cmd: &JiraCrudSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraCrudSubcommand::List => client.list_permission_schemes().await?,
        JiraCrudSubcommand::Get(args) => client.get_permission_scheme(&args.id).await?,
        JiraCrudSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_permission_scheme(&payload).await?
        }
        JiraCrudSubcommand::Update(args) => {
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
                .update_permission_scheme(&args.id, &Value::Object(fields))
                .await?
        }
        JiraCrudSubcommand::Delete(args) => {
            client.delete_permission_scheme(&args.id).await?;
            Value::String(format!("Permission scheme {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_notification_scheme(
    cmd: &JiraCrudSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraCrudSubcommand::List => client.list_notification_schemes().await?,
        JiraCrudSubcommand::Get(args) => client.get_notification_scheme(&args.id).await?,
        JiraCrudSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_notification_scheme(&payload).await?
        }
        JiraCrudSubcommand::Update(args) => {
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
                .update_notification_scheme(&args.id, &Value::Object(fields))
                .await?
        }
        JiraCrudSubcommand::Delete(args) => {
            client.delete_notification_scheme(&args.id).await?;
            Value::String(format!("Notification scheme {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_issue_security_scheme(
    cmd: &JiraCrudSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraCrudSubcommand::List => client.list_issue_security_schemes().await?,
        JiraCrudSubcommand::Get(args) => client.get_issue_security_scheme(&args.id).await?,
        JiraCrudSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_issue_security_scheme(&payload).await?
        }
        JiraCrudSubcommand::Update(args) => {
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
                .update_issue_security_scheme(&args.id, &Value::Object(fields))
                .await?
        }
        JiraCrudSubcommand::Delete(args) => {
            client.delete_issue_security_scheme(&args.id).await?;
            Value::String(format!("Issue security scheme {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_field_config(
    cmd: &JiraFieldConfigSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraFieldConfigSubcommand::List => client.list_field_configurations().await?,
        JiraFieldConfigSubcommand::Get(args) => client.get_field_configuration(&args.id).await?,
        JiraFieldConfigSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            client.create_field_configuration(&payload).await?
        }
        JiraFieldConfigSubcommand::Delete(args) => {
            client.delete_field_configuration(&args.id).await?;
            Value::String(format!("Field configuration {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_issue_type_scheme(
    cmd: &JiraIssueTypeSchemeSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraIssueTypeSchemeSubcommand::List => client.list_issue_type_schemes().await?,
        JiraIssueTypeSchemeSubcommand::Get(args) => client.get_issue_type_scheme(&args.id).await?,
        JiraIssueTypeSchemeSubcommand::Create(args) => {
            let mut payload = json!({ "name": &args.name });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            if let Some(dit) = &args.default_issue_type_id {
                payload["defaultIssueTypeId"] = Value::String(dit.clone());
            }
            client.create_issue_type_scheme(&payload).await?
        }
        JiraIssueTypeSchemeSubcommand::Update(args) => {
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
                .update_issue_type_scheme(&args.id, &Value::Object(fields))
                .await?
        }
        JiraIssueTypeSchemeSubcommand::Delete(args) => {
            client.delete_issue_type_scheme(&args.id).await?;
            Value::String(format!("Issue type scheme {} deleted", args.id))
        }
    })
}
