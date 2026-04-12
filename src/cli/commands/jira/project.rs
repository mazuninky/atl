use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

pub(super) async fn dispatch_project(
    cmd: &JiraProjectSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraProjectSubcommand::List => client.get_projects().await?,
        JiraProjectSubcommand::Get(args) => client.get_project(&args.project_key).await?,
        JiraProjectSubcommand::Create(args) => {
            let mut payload = json!({
                "key": &args.key,
                "name": &args.name,
                "projectTypeKey": &args.project_type_key,
                "leadAccountId": &args.lead,
            });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            if let Some(tmpl) = &args.template {
                payload["projectTemplateKey"] = Value::String(tmpl.clone());
            }
            client.create_project(&payload).await?
        }
        JiraProjectSubcommand::Update(args) => {
            let mut fields = serde_json::Map::new();
            if let Some(name) = &args.name {
                fields.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(lead) = &args.lead {
                fields.insert("leadAccountId".into(), Value::String(lead.clone()));
            }
            if let Some(desc) = &args.description {
                fields.insert("description".into(), Value::String(desc.clone()));
            }
            if fields.is_empty() {
                anyhow::bail!(
                    "no fields to update; specify at least one of --name, --lead, --description"
                );
            }
            client
                .update_project(&args.key, &Value::Object(fields))
                .await?
        }
        JiraProjectSubcommand::Delete(args) => {
            client.delete_project(&args.project_key).await?;
            Value::String(format!("Project {} deleted", args.project_key))
        }
        JiraProjectSubcommand::Statuses(args) => {
            client.get_project_statuses(&args.project_key).await?
        }
        JiraProjectSubcommand::Roles(args) => client.get_project_roles(&args.project_key).await?,
        JiraProjectSubcommand::Archive(args) => {
            client.archive_project(&args.project_key).await?;
            Value::String(format!("Project {} archived", args.project_key))
        }
        JiraProjectSubcommand::Restore(args) => {
            client.restore_project(&args.project_key).await?;
            Value::String(format!("Project {} restored", args.project_key))
        }
        JiraProjectSubcommand::Features(args) => {
            client.get_project_features(&args.project_key).await?
        }
    })
}
