use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

/// Filter the raw field list down to custom fields. Pass-through if the value isn't an array
/// (the upstream API is supposed to return an array but we don't enforce a schema here).
fn filter_custom_fields(all: Value) -> serde_json::Result<Value> {
    if let Some(arr) = all.as_array() {
        let custom: Vec<&Value> = arr
            .iter()
            .filter(|f| f.get("custom").and_then(|v| v.as_bool()).unwrap_or(false))
            .collect();
        serde_json::to_value(custom)
    } else {
        Ok(all)
    }
}

/// Build the create payload for a custom field.
fn build_field_create_payload(args: &JiraFieldCreateArgs) -> Value {
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
    payload
}

/// Build the create payload for an issue type.
fn build_issue_type_create_payload(args: &JiraIssueTypeCreateArgs) -> Value {
    let mut payload = json!({
        "name": &args.name,
        "type": &args.r#type,
    });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

/// Build the update payload for an issue type.
fn build_issue_type_update_payload(args: &JiraIssueTypeUpdateArgs) -> anyhow::Result<Value> {
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
    Ok(Value::Object(fields))
}

/// Build the create payload for a priority.
fn build_priority_create_payload(args: &JiraPriorityCreateArgs) -> Value {
    let mut payload = json!({
        "name": &args.name,
        "statusColor": &args.status_color,
    });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

/// Build the update payload for a priority. Errors if no field is specified.
fn build_priority_update_payload(args: &JiraPriorityUpdateArgs) -> anyhow::Result<Value> {
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
    Ok(Value::Object(fields))
}

/// Build the create payload for a resolution.
fn build_resolution_create_payload(args: &JiraResolutionCreateArgs) -> Value {
    let mut payload = json!({ "name": &args.name });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

/// Build the update payload for a resolution.
fn build_resolution_update_payload(args: &JiraResolutionUpdateArgs) -> anyhow::Result<Value> {
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
    Ok(Value::Object(fields))
}

pub(super) async fn dispatch_field(
    cmd: &JiraFieldSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraFieldSubcommand::List(args) => {
            let all = client.get_fields().await?;
            if args.custom {
                filter_custom_fields(all)?
            } else {
                all
            }
        }
        JiraFieldSubcommand::Create(args) => {
            client
                .create_field(&build_field_create_payload(args))
                .await?
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
            client
                .create_issue_type(&build_issue_type_create_payload(args))
                .await?
        }
        JiraIssueTypeSubcommand::Update(args) => {
            client
                .update_issue_type(&args.id, &build_issue_type_update_payload(args)?)
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
            client
                .create_priority(&build_priority_create_payload(args))
                .await?
        }
        JiraPrioritySubcommand::Update(args) => {
            client
                .update_priority(&args.id, &build_priority_update_payload(args)?)
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
            client
                .create_resolution(&build_resolution_create_payload(args))
                .await?
        }
        JiraResolutionSubcommand::Update(args) => {
            client
                .update_resolution(&args.id, &build_resolution_update_payload(args)?)
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
    // All branches are pure HTTP delegation; covered by contract tests in tests/contract_jira_*.rs.
    Ok(match cmd {
        JiraStatusSubcommand::List => client.list_statuses().await?,
        JiraStatusSubcommand::Get(args) => client.get_status(&args.id).await?,
        JiraStatusSubcommand::Categories => client.list_status_categories().await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- filter_custom_fields -------------------------------------------------

    #[test]
    fn filter_custom_fields_keeps_only_custom() {
        let input = json!([
            { "id": "1", "custom": true,  "name": "Foo" },
            { "id": "2", "custom": false, "name": "Bar" },
            { "id": "3", "custom": true,  "name": "Baz" },
        ]);
        let out = filter_custom_fields(input).unwrap();
        assert_eq!(
            out,
            json!([
                { "id": "1", "custom": true, "name": "Foo" },
                { "id": "3", "custom": true, "name": "Baz" },
            ])
        );
    }

    #[test]
    fn filter_custom_fields_treats_missing_custom_as_false() {
        let input = json!([
            { "id": "1", "name": "Foo" },
            { "id": "2", "custom": true, "name": "Bar" },
        ]);
        let out = filter_custom_fields(input).unwrap();
        assert_eq!(out, json!([{ "id": "2", "custom": true, "name": "Bar" }]));
    }

    #[test]
    fn filter_custom_fields_returns_non_array_unchanged() {
        // Defensive: if the upstream returns an object instead of an array, pass through.
        let input = json!({ "error": "unexpected shape" });
        let out = filter_custom_fields(input.clone()).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn filter_custom_fields_empty_array() {
        let out = filter_custom_fields(json!([])).unwrap();
        assert_eq!(out, json!([]));
    }

    // --- build_field_create_payload ------------------------------------------

    fn field_create(
        name: &str,
        ty: &str,
        description: Option<&str>,
        search_key: Option<&str>,
    ) -> JiraFieldCreateArgs {
        JiraFieldCreateArgs {
            name: name.to_string(),
            r#type: ty.to_string(),
            description: description.map(str::to_string),
            search_key: search_key.map(str::to_string),
        }
    }

    #[test]
    fn field_create_minimal() {
        let payload = build_field_create_payload(&field_create("My", "text", None, None));
        assert_eq!(payload, json!({ "name": "My", "type": "text" }));
    }

    #[test]
    fn field_create_with_description_only() {
        let payload = build_field_create_payload(&field_create("My", "text", Some("d"), None));
        assert_eq!(
            payload,
            json!({ "name": "My", "type": "text", "description": "d" })
        );
    }

    #[test]
    fn field_create_with_search_key_only() {
        let payload = build_field_create_payload(&field_create("My", "text", None, Some("sk")));
        assert_eq!(
            payload,
            json!({ "name": "My", "type": "text", "searcherKey": "sk" })
        );
    }

    #[test]
    fn field_create_with_all_optional_fields() {
        let payload =
            build_field_create_payload(&field_create("My", "text", Some("d"), Some("sk")));
        assert_eq!(
            payload,
            json!({
                "name": "My",
                "type": "text",
                "description": "d",
                "searcherKey": "sk"
            })
        );
    }

    // --- build_issue_type_create_payload -------------------------------------

    fn issue_type_create(
        name: &str,
        ty: &str,
        description: Option<&str>,
    ) -> JiraIssueTypeCreateArgs {
        JiraIssueTypeCreateArgs {
            name: name.to_string(),
            description: description.map(str::to_string),
            r#type: ty.to_string(),
        }
    }

    #[test]
    fn issue_type_create_minimal() {
        let payload = build_issue_type_create_payload(&issue_type_create("Bug", "standard", None));
        assert_eq!(payload, json!({ "name": "Bug", "type": "standard" }));
    }

    #[test]
    fn issue_type_create_with_description() {
        let payload =
            build_issue_type_create_payload(&issue_type_create("Bug", "standard", Some("desc")));
        assert_eq!(
            payload,
            json!({ "name": "Bug", "type": "standard", "description": "desc" })
        );
    }

    // --- build_issue_type_update_payload -------------------------------------

    fn issue_type_update(name: Option<&str>, description: Option<&str>) -> JiraIssueTypeUpdateArgs {
        JiraIssueTypeUpdateArgs {
            id: "10000".to_string(),
            name: name.map(str::to_string),
            description: description.map(str::to_string),
        }
    }

    #[test]
    fn issue_type_update_with_name() {
        let payload =
            build_issue_type_update_payload(&issue_type_update(Some("New"), None)).unwrap();
        assert_eq!(payload, json!({ "name": "New" }));
    }

    #[test]
    fn issue_type_update_with_description() {
        let payload =
            build_issue_type_update_payload(&issue_type_update(None, Some("nd"))).unwrap();
        assert_eq!(payload, json!({ "description": "nd" }));
    }

    #[test]
    fn issue_type_update_with_no_fields_errors() {
        let err = build_issue_type_update_payload(&issue_type_update(None, None)).unwrap_err();
        assert!(
            err.to_string().contains("no fields to update"),
            "expected guard message, got: {err}"
        );
    }

    // --- build_priority_create_payload ---------------------------------------

    #[test]
    fn priority_create_minimal_uses_status_color() {
        let args = JiraPriorityCreateArgs {
            name: "High".to_string(),
            description: None,
            status_color: "#ff0000".to_string(),
        };
        let payload = build_priority_create_payload(&args);
        assert_eq!(payload, json!({ "name": "High", "statusColor": "#ff0000" }));
    }

    #[test]
    fn priority_create_with_description() {
        let args = JiraPriorityCreateArgs {
            name: "High".to_string(),
            description: Some("hot".to_string()),
            status_color: "#ff0000".to_string(),
        };
        let payload = build_priority_create_payload(&args);
        assert_eq!(
            payload,
            json!({ "name": "High", "statusColor": "#ff0000", "description": "hot" })
        );
    }

    // --- build_priority_update_payload ---------------------------------------

    fn priority_update(
        name: Option<&str>,
        description: Option<&str>,
        status_color: Option<&str>,
    ) -> JiraPriorityUpdateArgs {
        JiraPriorityUpdateArgs {
            id: "1".to_string(),
            name: name.map(str::to_string),
            description: description.map(str::to_string),
            status_color: status_color.map(str::to_string),
        }
    }

    #[test]
    fn priority_update_with_only_status_color() {
        let payload =
            build_priority_update_payload(&priority_update(None, None, Some("#abc"))).unwrap();
        assert_eq!(payload, json!({ "statusColor": "#abc" }));
    }

    #[test]
    fn priority_update_with_all_fields() {
        let payload =
            build_priority_update_payload(&priority_update(Some("N"), Some("D"), Some("#fff")))
                .unwrap();
        assert_eq!(
            payload,
            json!({ "name": "N", "description": "D", "statusColor": "#fff" })
        );
    }

    #[test]
    fn priority_update_with_no_fields_errors_and_mentions_status_color() {
        let err = build_priority_update_payload(&priority_update(None, None, None)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no fields to update"), "got: {msg}");
        assert!(
            msg.contains("--status-color"),
            "error should mention --status-color, got: {msg}"
        );
    }

    // --- build_resolution_create_payload -------------------------------------

    #[test]
    fn resolution_create_minimal() {
        let args = JiraResolutionCreateArgs {
            name: "Done".to_string(),
            description: None,
        };
        assert_eq!(
            build_resolution_create_payload(&args),
            json!({ "name": "Done" })
        );
    }

    #[test]
    fn resolution_create_with_description() {
        let args = JiraResolutionCreateArgs {
            name: "Done".to_string(),
            description: Some("d".to_string()),
        };
        assert_eq!(
            build_resolution_create_payload(&args),
            json!({ "name": "Done", "description": "d" })
        );
    }

    // --- build_resolution_update_payload -------------------------------------

    #[test]
    fn resolution_update_with_name() {
        let args = JiraResolutionUpdateArgs {
            id: "1".to_string(),
            name: Some("N".to_string()),
            description: None,
        };
        let payload = build_resolution_update_payload(&args).unwrap();
        assert_eq!(payload, json!({ "name": "N" }));
    }

    #[test]
    fn resolution_update_with_no_fields_errors() {
        let args = JiraResolutionUpdateArgs {
            id: "1".to_string(),
            name: None,
            description: None,
        };
        let err = build_resolution_update_payload(&args).unwrap_err();
        assert!(
            err.to_string().contains("no fields to update"),
            "got: {err}"
        );
    }
}
