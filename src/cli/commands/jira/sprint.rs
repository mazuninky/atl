use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

/// Split a comma-separated `--fields` argument into trimmed slices, dropping empty entries.
fn parse_fields_list(raw: &str) -> Vec<&str> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Build the create payload for a sprint.
fn build_sprint_create_payload(args: &JiraSprintCreateArgs) -> Value {
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
    payload
}

/// Build the update payload for a sprint. Returns an error if no fields were supplied.
fn build_sprint_update_payload(args: &JiraSprintUpdateArgs) -> anyhow::Result<Value> {
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
    Ok(Value::Object(payload))
}

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
            let fields = parse_fields_list(&args.fields);
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
            client
                .create_sprint(&build_sprint_create_payload(args))
                .await?
        }
        JiraSprintSubcommand::Update(args) => {
            client
                .update_sprint(args.sprint_id, &build_sprint_update_payload(args)?)
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_fields_list ----------------------------------------------------

    #[test]
    fn parse_fields_simple() {
        assert_eq!(
            parse_fields_list("key,summary,status"),
            vec!["key", "summary", "status"]
        );
    }

    #[test]
    fn parse_fields_trims_whitespace() {
        assert_eq!(
            parse_fields_list("  key , summary,status "),
            vec!["key", "summary", "status"]
        );
    }

    #[test]
    fn parse_fields_drops_empty_segments() {
        // Trailing comma and double comma should not produce empty entries.
        assert_eq!(parse_fields_list("key,,summary,"), vec!["key", "summary"]);
    }

    #[test]
    fn parse_fields_single_value() {
        assert_eq!(parse_fields_list("key"), vec!["key"]);
    }

    // --- build_sprint_create_payload -----------------------------------------

    fn sprint_create(
        board_id: u64,
        name: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        goal: Option<&str>,
    ) -> JiraSprintCreateArgs {
        JiraSprintCreateArgs {
            board_id,
            name: name.to_string(),
            start_date: start_date.map(str::to_string),
            end_date: end_date.map(str::to_string),
            goal: goal.map(str::to_string),
        }
    }

    #[test]
    fn sprint_create_minimal() {
        let payload = build_sprint_create_payload(&sprint_create(42, "Sprint 1", None, None, None));
        assert_eq!(payload, json!({ "originBoardId": 42, "name": "Sprint 1" }));
    }

    #[test]
    fn sprint_create_with_dates_and_goal() {
        let payload = build_sprint_create_payload(&sprint_create(
            42,
            "Sprint 1",
            Some("2024-01-15T09:00:00.000Z"),
            Some("2024-01-29T09:00:00.000Z"),
            Some("Ship it"),
        ));
        assert_eq!(
            payload,
            json!({
                "originBoardId": 42,
                "name": "Sprint 1",
                "startDate": "2024-01-15T09:00:00.000Z",
                "endDate": "2024-01-29T09:00:00.000Z",
                "goal": "Ship it",
            })
        );
    }

    // --- build_sprint_update_payload -----------------------------------------

    fn sprint_update(
        name: Option<&str>,
        start_date: Option<&str>,
        end_date: Option<&str>,
        goal: Option<&str>,
        state: Option<SprintState>,
    ) -> JiraSprintUpdateArgs {
        JiraSprintUpdateArgs {
            sprint_id: 7,
            name: name.map(str::to_string),
            start_date: start_date.map(str::to_string),
            end_date: end_date.map(str::to_string),
            goal: goal.map(str::to_string),
            state,
        }
    }

    #[test]
    fn sprint_update_with_name_only() {
        let payload =
            build_sprint_update_payload(&sprint_update(Some("New"), None, None, None, None))
                .unwrap();
        assert_eq!(payload, json!({ "name": "New" }));
    }

    #[test]
    fn sprint_update_with_state_serializes_enum_as_lower_case() {
        let payload = build_sprint_update_payload(&sprint_update(
            None,
            None,
            None,
            None,
            Some(SprintState::Active),
        ))
        .unwrap();
        assert_eq!(payload, json!({ "state": "active" }));
    }

    #[test]
    fn sprint_update_with_all_fields() {
        let payload = build_sprint_update_payload(&sprint_update(
            Some("N"),
            Some("S"),
            Some("E"),
            Some("G"),
            Some(SprintState::Closed),
        ))
        .unwrap();
        assert_eq!(
            payload,
            json!({
                "name": "N",
                "startDate": "S",
                "endDate": "E",
                "goal": "G",
                "state": "closed",
            })
        );
    }

    #[test]
    fn sprint_update_with_no_fields_errors_and_lists_options() {
        let err =
            build_sprint_update_payload(&sprint_update(None, None, None, None, None)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no fields to update"), "got: {msg}");
        assert!(msg.contains("--name"), "got: {msg}");
        assert!(msg.contains("--start-date"), "got: {msg}");
        assert!(msg.contains("--end-date"), "got: {msg}");
        assert!(msg.contains("--goal"), "got: {msg}");
        assert!(msg.contains("--state"), "got: {msg}");
    }
}
