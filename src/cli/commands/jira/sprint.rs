use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

pub(super) async fn dispatch_sprint(
    cmd: &JiraSprintSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraSprintSubcommand::List(args) => {
            client
                .get_sprints(args.board_id, args.state.as_ref().map(SprintState::as_str))
                .await?
        }
        JiraSprintSubcommand::Get(args) => client.get_sprint(args.sprint_id).await?,
        JiraSprintSubcommand::Issues(args) => {
            let fields: Vec<&str> = args.fields.split(',').map(str::trim).collect();
            if args.all {
                let url = format!("{}/sprint/{}/issue", client.agile_url(), args.sprint_id);
                let fields_str = fields.join(",");
                client
                    .paginate_offset(&url, args.limit, "issues", &[("fields", &fields_str)])
                    .await?
            } else {
                client
                    .get_sprint_issues(args.sprint_id, args.limit, &fields)
                    .await?
            }
        }
        JiraSprintSubcommand::Create(args) => {
            let mut payload = json!({
                "originBoardId": args.board_id,
                "name": &args.name,
            });
            if let Some(sd) = &args.start_date {
                payload["startDate"] = Value::String(sd.clone());
            }
            if let Some(ed) = &args.end_date {
                payload["endDate"] = Value::String(ed.clone());
            }
            if let Some(goal) = &args.goal {
                payload["goal"] = Value::String(goal.clone());
            }
            client.create_sprint(&payload).await?
        }
        JiraSprintSubcommand::Update(args) => {
            let mut payload = serde_json::Map::new();
            if let Some(name) = &args.name {
                payload.insert("name".into(), Value::String(name.clone()));
            }
            if let Some(sd) = &args.start_date {
                payload.insert("startDate".into(), Value::String(sd.clone()));
            }
            if let Some(ed) = &args.end_date {
                payload.insert("endDate".into(), Value::String(ed.clone()));
            }
            if let Some(goal) = &args.goal {
                payload.insert("goal".into(), Value::String(goal.clone()));
            }
            if let Some(state) = &args.state {
                payload.insert("state".into(), Value::String(state.as_str().to_string()));
            }
            if payload.is_empty() {
                anyhow::bail!(
                    "no fields to update; specify at least one of --name, --start-date, --end-date, --goal, --state"
                );
            }
            client
                .update_sprint(args.sprint_id, &Value::Object(payload))
                .await?
        }
        JiraSprintSubcommand::Delete(args) => {
            client.delete_sprint(args.sprint_id).await?;
            Value::String(format!("Sprint {} deleted", args.sprint_id))
        }
        JiraSprintSubcommand::Move(args) => {
            client
                .move_issues_to_sprint(args.sprint_id, &args.issues)
                .await?;
            Value::String(format!("Issues moved to sprint {}", args.sprint_id))
        }
    })
}

pub(super) async fn dispatch_epic(
    cmd: &JiraEpicSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraEpicSubcommand::List(args) => client.get_epics(args.board_id).await?,
        JiraEpicSubcommand::Get(args) => client.get_epic(&args.epic_id_or_key).await?,
        JiraEpicSubcommand::Issues(args) => {
            if args.all {
                let url = format!("{}/epic/{}/issue", client.agile_url(), args.epic_id_or_key);
                client
                    .paginate_offset(&url, args.limit, "issues", &[])
                    .await?
            } else {
                client
                    .get_epic_issues(&args.epic_id_or_key, args.limit)
                    .await?
            }
        }
        JiraEpicSubcommand::Add(args) => {
            let val = client
                .add_issues_to_epic(&args.epic_key, &args.issues)
                .await?;
            if val.is_null() {
                Value::String(format!("Issues added to epic {}", args.epic_key))
            } else {
                val
            }
        }
        JiraEpicSubcommand::Remove(args) => {
            let val = client.remove_issues_from_epic(&args.issues).await?;
            if val.is_null() {
                Value::String("Issues removed from epic".to_string())
            } else {
                val
            }
        }
    })
}
