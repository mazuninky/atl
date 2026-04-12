use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

pub(super) async fn dispatch_filter(
    cmd: &JiraFilterSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraFilterSubcommand::List(args) => {
            client
                .search_filters(args.name.as_deref(), args.favourites, args.mine)
                .await?
        }
        JiraFilterSubcommand::Get(args) => client.get_filter(&args.id).await?,
        JiraFilterSubcommand::Create(args) => {
            let mut payload = json!({
                "name": &args.name,
                "jql": &args.jql,
            });
            if let Some(desc) = &args.description {
                payload["description"] = Value::String(desc.clone());
            }
            if args.favourite {
                payload["favourite"] = Value::Bool(true);
            }
            client.create_filter(&payload).await?
        }
        JiraFilterSubcommand::Update(args) => {
            let mut payload = serde_json::Map::new();
            if let Some(name) = &args.name {
                payload.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(jql) = &args.jql {
                payload.insert("jql".into(), Value::String(jql.clone()));
            }
            if let Some(desc) = &args.description {
                payload.insert("description".into(), Value::String(desc.clone()));
            }
            if let Some(fav) = args.favourite {
                payload.insert("favourite".into(), Value::Bool(fav));
            }
            if payload.is_empty() {
                anyhow::bail!(
                    "no fields to update; specify at least one of --name, --jql, --description, --favourite"
                );
            }
            client
                .update_filter(&args.id, &Value::Object(payload))
                .await?
        }
        JiraFilterSubcommand::Delete(args) => {
            client.delete_filter(&args.id).await?;
            Value::String(format!("Filter {} deleted", args.id))
        }
    })
}

pub(super) async fn dispatch_worklog(
    cmd: &JiraWorklogSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraWorklogSubcommand::List(args) => client.list_worklogs(&args.key).await?,
        JiraWorklogSubcommand::Add(args) => {
            client
                .add_worklog(
                    &args.key,
                    &args.time_spent,
                    args.comment.as_deref(),
                    args.started.as_deref(),
                )
                .await?
        }
        JiraWorklogSubcommand::Delete(args) => {
            client.delete_worklog(&args.key, &args.worklog_id).await?;
            Value::String(format!(
                "Worklog {} deleted from {}",
                args.worklog_id, args.key
            ))
        }
    })
}
