use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

/// Build the create payload for a filter.
fn build_filter_create_payload(args: &JiraFilterCreateArgs) -> Value {
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
    payload
}

/// Build the update payload for a filter. Errors if no field is supplied.
fn build_filter_update_payload(args: &JiraFilterUpdateArgs) -> anyhow::Result<Value> {
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
    Ok(Value::Object(payload))
}

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
            client
                .create_filter(&build_filter_create_payload(args))
                .await?
        }
        JiraFilterSubcommand::Update(args) => {
            client
                .update_filter(&args.id, &build_filter_update_payload(args)?)
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
    // All branches are pure HTTP delegation; covered by contract tests in tests/contract_jira_*.rs.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn filter_create(
        name: &str,
        jql: &str,
        description: Option<&str>,
        favourite: bool,
    ) -> JiraFilterCreateArgs {
        JiraFilterCreateArgs {
            name: name.to_string(),
            jql: jql.to_string(),
            description: description.map(str::to_string),
            favourite,
        }
    }

    #[test]
    fn filter_create_minimal_omits_favourite_when_false() {
        let payload =
            build_filter_create_payload(&filter_create("My filter", "project = X", None, false));
        assert_eq!(
            payload,
            json!({ "name": "My filter", "jql": "project = X" })
        );
    }

    #[test]
    fn filter_create_with_description() {
        let payload = build_filter_create_payload(&filter_create(
            "My filter",
            "project = X",
            Some("d"),
            false,
        ));
        assert_eq!(
            payload,
            json!({
                "name": "My filter",
                "jql": "project = X",
                "description": "d",
            })
        );
    }

    #[test]
    fn filter_create_with_favourite_true_includes_field() {
        let payload =
            build_filter_create_payload(&filter_create("My filter", "project = X", None, true));
        assert_eq!(
            payload,
            json!({
                "name": "My filter",
                "jql": "project = X",
                "favourite": true,
            })
        );
    }

    fn filter_update(
        name: Option<&str>,
        jql: Option<&str>,
        description: Option<&str>,
        favourite: Option<bool>,
    ) -> JiraFilterUpdateArgs {
        JiraFilterUpdateArgs {
            id: "100".to_string(),
            name: name.map(str::to_string),
            jql: jql.map(str::to_string),
            description: description.map(str::to_string),
            favourite,
        }
    }

    #[test]
    fn filter_update_with_name_only() {
        let payload =
            build_filter_update_payload(&filter_update(Some("N"), None, None, None)).unwrap();
        assert_eq!(payload, json!({ "name": "N" }));
    }

    #[test]
    fn filter_update_with_jql_only() {
        let payload =
            build_filter_update_payload(&filter_update(None, Some("project = Y"), None, None))
                .unwrap();
        assert_eq!(payload, json!({ "jql": "project = Y" }));
    }

    #[test]
    fn filter_update_with_favourite_false_serializes_false() {
        // `Option<bool>` distinguishes "not specified" (None) from "set to false" (Some(false)).
        // The payload must include `favourite: false` so the server can unfavourite.
        let payload =
            build_filter_update_payload(&filter_update(None, None, None, Some(false))).unwrap();
        assert_eq!(payload, json!({ "favourite": false }));
    }

    #[test]
    fn filter_update_with_favourite_true_serializes_true() {
        let payload =
            build_filter_update_payload(&filter_update(None, None, None, Some(true))).unwrap();
        assert_eq!(payload, json!({ "favourite": true }));
    }

    #[test]
    fn filter_update_with_all_fields() {
        let payload = build_filter_update_payload(&filter_update(
            Some("N"),
            Some("J"),
            Some("D"),
            Some(true),
        ))
        .unwrap();
        assert_eq!(
            payload,
            json!({ "name": "N", "jql": "J", "description": "D", "favourite": true })
        );
    }

    #[test]
    fn filter_update_with_no_fields_errors_and_lists_options() {
        let err = build_filter_update_payload(&filter_update(None, None, None, None)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no fields to update"), "got: {msg}");
        assert!(msg.contains("--name"), "got: {msg}");
        assert!(msg.contains("--jql"), "got: {msg}");
        assert!(msg.contains("--description"), "got: {msg}");
        assert!(msg.contains("--favourite"), "got: {msg}");
    }
}
