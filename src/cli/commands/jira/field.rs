use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

/// Build the create payload for a custom field context.
fn build_context_create_payload(args: &JiraFieldContextCreateArgs) -> Value {
    let mut payload = json!({
        "name": &args.name,
        "issueTypeIds": &args.issue_type_ids,
        "projectIds": &args.project_ids,
    });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

/// Build the update payload for a custom field context.
fn build_context_update_payload(args: &JiraFieldContextUpdateArgs) -> anyhow::Result<Value> {
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

/// Build the batch-create payload for select-list options.
fn build_options_create_payload(args: &JiraFieldOptionsAddArgs) -> Value {
    let opts: Vec<Value> = args
        .values
        .iter()
        .map(|v| {
            let mut o = json!({ "value": v });
            if args.disabled {
                o["disabled"] = Value::Bool(true);
            }
            o
        })
        .collect();
    json!({ "options": opts })
}

/// Build the batch-update payload for a single select-list option.
fn build_options_update_payload(args: &JiraFieldOptionsUpdateArgs) -> anyhow::Result<Value> {
    let mut entry = serde_json::Map::new();
    entry.insert("id".into(), Value::String(args.option_id.clone()));
    let mut had_change = false;
    if let Some(value) = &args.value {
        entry.insert("value".into(), Value::String(value.clone()));
        had_change = true;
    }
    if let Some(disabled) = args.disabled {
        entry.insert("disabled".into(), Value::Bool(disabled));
        had_change = true;
    }
    if !had_change {
        anyhow::bail!("no fields to update; specify at least one of --value, --disabled");
    }
    Ok(json!({ "options": [Value::Object(entry)] }))
}

/// Build the reorder payload for select-list options.
fn build_options_reorder_payload(args: &JiraFieldOptionsReorderArgs) -> Value {
    let mut payload = json!({ "customFieldOptionIds": &args.option_ids });
    if let Some(after) = &args.after {
        payload["after"] = Value::String(after.clone());
    } else if let Some(position) = args.position {
        let label = match position {
            JiraFieldOptionsPosition::First => "First",
            JiraFieldOptionsPosition::Last => "Last",
        };
        payload["position"] = Value::String(label.to_string());
    }
    payload
}

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
        JiraFieldSubcommand::Context(cmd) => {
            dispatch_field_context(&cmd.field_id, &cmd.command, client).await?
        }
        JiraFieldSubcommand::Options(cmd) => {
            dispatch_field_options(&cmd.field_id, &cmd.context_id, &cmd.command, client).await?
        }
    })
}

async fn dispatch_field_context(
    field_id: &str,
    cmd: &JiraFieldContextSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraFieldContextSubcommand::List(args) => {
            if args.all {
                client.field_contexts_list_all(field_id).await?
            } else {
                client.field_contexts_list(field_id, args.limit, 0).await?
            }
        }
        JiraFieldContextSubcommand::Create(args) => {
            client
                .create_field_context(field_id, &build_context_create_payload(args))
                .await?
        }
        JiraFieldContextSubcommand::Update(args) => {
            client
                .update_field_context(
                    field_id,
                    &args.context_id,
                    &build_context_update_payload(args)?,
                )
                .await?
        }
        JiraFieldContextSubcommand::Delete(args) => {
            client
                .delete_field_context(field_id, &args.context_id)
                .await?;
            Value::String(format!("Context {} deleted", args.context_id))
        }
        JiraFieldContextSubcommand::Projects(args) => {
            if args.all {
                client
                    .field_context_project_mappings_all(field_id, &args.context_id)
                    .await?
            } else {
                client
                    .field_context_project_mappings(field_id, &args.context_id, args.limit, 0)
                    .await?
            }
        }
        JiraFieldContextSubcommand::AddProjects(args) => {
            let res = client
                .field_context_assign_projects(field_id, &args.context_id, &args.project_ids)
                .await?;
            if res.is_null() {
                Value::String(format!(
                    "Added {} project(s) to context {}",
                    args.project_ids.len(),
                    args.context_id
                ))
            } else {
                res
            }
        }
        JiraFieldContextSubcommand::RemoveProjects(args) => {
            let res = client
                .field_context_remove_projects(field_id, &args.context_id, &args.project_ids)
                .await?;
            if res.is_null() {
                Value::String(format!(
                    "Removed {} project(s) from context {}",
                    args.project_ids.len(),
                    args.context_id
                ))
            } else {
                res
            }
        }
        JiraFieldContextSubcommand::IssueTypes(args) => {
            if args.all {
                client
                    .field_context_issue_type_mappings_all(field_id, &args.context_id)
                    .await?
            } else {
                client
                    .field_context_issue_type_mappings(field_id, &args.context_id, args.limit, 0)
                    .await?
            }
        }
        JiraFieldContextSubcommand::AddIssueTypes(args) => {
            let res = client
                .field_context_assign_issue_types(field_id, &args.context_id, &args.issue_type_ids)
                .await?;
            if res.is_null() {
                Value::String(format!(
                    "Added {} issue type(s) to context {}",
                    args.issue_type_ids.len(),
                    args.context_id
                ))
            } else {
                res
            }
        }
        JiraFieldContextSubcommand::RemoveIssueTypes(args) => {
            let res = client
                .field_context_remove_issue_types(field_id, &args.context_id, &args.issue_type_ids)
                .await?;
            if res.is_null() {
                Value::String(format!(
                    "Removed {} issue type(s) from context {}",
                    args.issue_type_ids.len(),
                    args.context_id
                ))
            } else {
                res
            }
        }
    })
}

async fn dispatch_field_options(
    field_id: &str,
    context_id: &str,
    cmd: &JiraFieldOptionsSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraFieldOptionsSubcommand::List(args) => {
            if args.all {
                client.field_options_list_all(field_id, context_id).await?
            } else {
                client
                    .field_options_list(field_id, context_id, args.limit, 0)
                    .await?
            }
        }
        JiraFieldOptionsSubcommand::Add(args) => {
            client
                .field_options_create(field_id, context_id, &build_options_create_payload(args))
                .await?
        }
        JiraFieldOptionsSubcommand::Update(args) => {
            client
                .field_options_update(field_id, context_id, &build_options_update_payload(args)?)
                .await?
        }
        JiraFieldOptionsSubcommand::Delete(args) => {
            client
                .field_option_delete(field_id, context_id, &args.option_id)
                .await?;
            Value::String(format!("Option {} deleted", args.option_id))
        }
        JiraFieldOptionsSubcommand::Reorder(args) => {
            let res = client
                .field_options_reorder(field_id, context_id, &build_options_reorder_payload(args))
                .await?;
            if res.is_null() {
                Value::String(format!("Reordered {} option(s)", args.option_ids.len()))
            } else {
                res
            }
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

    // --- build_context_create_payload ----------------------------------------

    #[test]
    fn context_create_minimal_emits_empty_id_arrays() {
        let args = JiraFieldContextCreateArgs {
            name: "Default".to_string(),
            description: None,
            issue_type_ids: vec![],
            project_ids: vec![],
        };
        assert_eq!(
            build_context_create_payload(&args),
            json!({
                "name": "Default",
                "issueTypeIds": [],
                "projectIds": [],
            })
        );
    }

    #[test]
    fn context_create_with_description_and_ids() {
        let args = JiraFieldContextCreateArgs {
            name: "Bugs".to_string(),
            description: Some("for bug-only projects".to_string()),
            issue_type_ids: vec!["10001".to_string(), "10002".to_string()],
            project_ids: vec!["10010".to_string()],
        };
        assert_eq!(
            build_context_create_payload(&args),
            json!({
                "name": "Bugs",
                "description": "for bug-only projects",
                "issueTypeIds": ["10001", "10002"],
                "projectIds": ["10010"],
            })
        );
    }

    // --- build_context_update_payload ----------------------------------------

    #[test]
    fn context_update_with_name_only() {
        let args = JiraFieldContextUpdateArgs {
            context_id: "1".to_string(),
            name: Some("New".to_string()),
            description: None,
        };
        assert_eq!(
            build_context_update_payload(&args).unwrap(),
            json!({ "name": "New" })
        );
    }

    #[test]
    fn context_update_with_no_fields_errors() {
        let args = JiraFieldContextUpdateArgs {
            context_id: "1".to_string(),
            name: None,
            description: None,
        };
        let err = build_context_update_payload(&args).unwrap_err();
        assert!(
            err.to_string().contains("no fields to update"),
            "got: {err}"
        );
    }

    // --- build_options_create_payload ----------------------------------------

    #[test]
    fn options_create_single_value() {
        let args = JiraFieldOptionsAddArgs {
            values: vec!["foo".to_string()],
            disabled: false,
        };
        assert_eq!(
            build_options_create_payload(&args),
            json!({ "options": [ { "value": "foo" } ] })
        );
    }

    #[test]
    fn options_create_multiple_values_with_disabled_flag() {
        let args = JiraFieldOptionsAddArgs {
            values: vec!["foo".to_string(), "bar".to_string()],
            disabled: true,
        };
        assert_eq!(
            build_options_create_payload(&args),
            json!({
                "options": [
                    { "value": "foo", "disabled": true },
                    { "value": "bar", "disabled": true },
                ]
            })
        );
    }

    // --- build_options_update_payload ----------------------------------------

    #[test]
    fn options_update_with_value_only() {
        let args = JiraFieldOptionsUpdateArgs {
            option_id: "10000".to_string(),
            value: Some("new".to_string()),
            disabled: None,
        };
        assert_eq!(
            build_options_update_payload(&args).unwrap(),
            json!({ "options": [ { "id": "10000", "value": "new" } ] })
        );
    }

    #[test]
    fn options_update_with_disabled_only() {
        let args = JiraFieldOptionsUpdateArgs {
            option_id: "10000".to_string(),
            value: None,
            disabled: Some(true),
        };
        assert_eq!(
            build_options_update_payload(&args).unwrap(),
            json!({ "options": [ { "id": "10000", "disabled": true } ] })
        );
    }

    #[test]
    fn options_update_with_no_fields_errors() {
        let args = JiraFieldOptionsUpdateArgs {
            option_id: "10000".to_string(),
            value: None,
            disabled: None,
        };
        let err = build_options_update_payload(&args).unwrap_err();
        assert!(
            err.to_string().contains("no fields to update"),
            "got: {err}"
        );
    }

    // --- build_options_reorder_payload ---------------------------------------

    #[test]
    fn options_reorder_with_after() {
        let args = JiraFieldOptionsReorderArgs {
            option_ids: vec!["1".to_string(), "2".to_string()],
            after: Some("3".to_string()),
            position: None,
        };
        assert_eq!(
            build_options_reorder_payload(&args),
            json!({ "customFieldOptionIds": ["1", "2"], "after": "3" })
        );
    }

    #[test]
    fn options_reorder_with_position_first() {
        let args = JiraFieldOptionsReorderArgs {
            option_ids: vec!["1".to_string()],
            after: None,
            position: Some(JiraFieldOptionsPosition::First),
        };
        assert_eq!(
            build_options_reorder_payload(&args),
            json!({ "customFieldOptionIds": ["1"], "position": "First" })
        );
    }

    #[test]
    fn options_reorder_with_position_last() {
        let args = JiraFieldOptionsReorderArgs {
            option_ids: vec!["1".to_string()],
            after: None,
            position: Some(JiraFieldOptionsPosition::Last),
        };
        assert_eq!(
            build_options_reorder_payload(&args),
            json!({ "customFieldOptionIds": ["1"], "position": "Last" })
        );
    }

    // --- clap parse smoke tests ----------------------------------------------
    //
    // Boot a tiny CLI tree just to confirm the new args parse and that the
    // mutual-exclusion / required guards on `reorder` actually trigger.

    use clap::Parser;

    #[derive(Debug, Parser)]
    struct TestField {
        #[command(subcommand)]
        command: JiraFieldSubcommand,
    }

    fn parse_field(argv: &[&str]) -> Result<TestField, clap::Error> {
        let mut full = vec!["test"];
        full.extend_from_slice(argv);
        TestField::try_parse_from(full)
    }

    #[test]
    fn context_list_parses() {
        let parsed = parse_field(&["context", "customfield_10001", "list"]).unwrap();
        match parsed.command {
            JiraFieldSubcommand::Context(c) => {
                assert_eq!(c.field_id, "customfield_10001");
                assert!(matches!(c.command, JiraFieldContextSubcommand::List(_)));
            }
            other => panic!("expected Context, got {other:?}"),
        }
    }

    #[test]
    fn context_create_parses_with_minimum_args() {
        let parsed =
            parse_field(&["context", "customfield_10001", "create", "--name", "X"]).unwrap();
        match parsed.command {
            JiraFieldSubcommand::Context(c) => match c.command {
                JiraFieldContextSubcommand::Create(args) => {
                    assert_eq!(args.name, "X");
                    assert!(args.issue_type_ids.is_empty());
                    assert!(args.project_ids.is_empty());
                }
                other => panic!("expected Create, got {other:?}"),
            },
            other => panic!("expected Context, got {other:?}"),
        }
    }

    #[test]
    fn context_update_parses_positional_id_and_name() {
        let parsed = parse_field(&[
            "context",
            "customfield_10001",
            "update",
            "10100",
            "--name",
            "Y",
        ])
        .unwrap();
        match parsed.command {
            JiraFieldSubcommand::Context(c) => match c.command {
                JiraFieldContextSubcommand::Update(args) => {
                    assert_eq!(args.context_id, "10100");
                    assert_eq!(args.name.as_deref(), Some("Y"));
                }
                other => panic!("expected Update, got {other:?}"),
            },
            other => panic!("expected Context, got {other:?}"),
        }
    }

    #[test]
    fn context_add_projects_collects_repeats() {
        let parsed = parse_field(&[
            "context",
            "customfield_10001",
            "add-projects",
            "10100",
            "--project-id",
            "10000",
            "--project-id",
            "10001",
        ])
        .unwrap();
        match parsed.command {
            JiraFieldSubcommand::Context(c) => match c.command {
                JiraFieldContextSubcommand::AddProjects(args) => {
                    assert_eq!(args.context_id, "10100");
                    assert_eq!(args.project_ids, vec!["10000", "10001"]);
                }
                other => panic!("expected AddProjects, got {other:?}"),
            },
            other => panic!("expected Context, got {other:?}"),
        }
    }

    #[test]
    fn context_projects_parses_with_defaults() {
        let parsed = parse_field(&["context", "customfield_10001", "projects", "10100"]).unwrap();
        match parsed.command {
            JiraFieldSubcommand::Context(c) => match c.command {
                JiraFieldContextSubcommand::Projects(args) => {
                    assert_eq!(args.context_id, "10100");
                    assert_eq!(args.limit, 50);
                    assert!(!args.all);
                }
                other => panic!("expected Projects, got {other:?}"),
            },
            other => panic!("expected Context, got {other:?}"),
        }
    }

    #[test]
    fn context_projects_parses_with_all_flag() {
        let parsed =
            parse_field(&["context", "customfield_10001", "projects", "10100", "--all"]).unwrap();
        match parsed.command {
            JiraFieldSubcommand::Context(c) => match c.command {
                JiraFieldContextSubcommand::Projects(args) => {
                    assert_eq!(args.context_id, "10100");
                    assert!(args.all);
                }
                other => panic!("expected Projects, got {other:?}"),
            },
            other => panic!("expected Context, got {other:?}"),
        }
    }

    #[test]
    fn context_issue_types_parses_with_explicit_limit() {
        let parsed = parse_field(&[
            "context",
            "customfield_10001",
            "issue-types",
            "10100",
            "--limit",
            "100",
        ])
        .unwrap();
        match parsed.command {
            JiraFieldSubcommand::Context(c) => match c.command {
                JiraFieldContextSubcommand::IssueTypes(args) => {
                    assert_eq!(args.context_id, "10100");
                    assert_eq!(args.limit, 100);
                    assert!(!args.all);
                }
                other => panic!("expected IssueTypes, got {other:?}"),
            },
            other => panic!("expected Context, got {other:?}"),
        }
    }

    #[test]
    fn options_add_parses_repeated_value() {
        let parsed = parse_field(&[
            "options",
            "customfield_10001",
            "10100",
            "add",
            "--value",
            "foo",
            "--value",
            "bar",
        ])
        .unwrap();
        match parsed.command {
            JiraFieldSubcommand::Options(o) => {
                assert_eq!(o.field_id, "customfield_10001");
                assert_eq!(o.context_id, "10100");
                match o.command {
                    JiraFieldOptionsSubcommand::Add(args) => {
                        assert_eq!(args.values, vec!["foo", "bar"]);
                        assert!(!args.disabled);
                    }
                    other => panic!("expected Add, got {other:?}"),
                }
            }
            other => panic!("expected Options, got {other:?}"),
        }
    }

    #[test]
    fn options_update_parses() {
        let parsed = parse_field(&[
            "options",
            "customfield_10001",
            "10100",
            "update",
            "20000",
            "--value",
            "foo",
        ])
        .unwrap();
        match parsed.command {
            JiraFieldSubcommand::Options(o) => match o.command {
                JiraFieldOptionsSubcommand::Update(args) => {
                    assert_eq!(args.option_id, "20000");
                    assert_eq!(args.value.as_deref(), Some("foo"));
                    assert_eq!(args.disabled, None);
                }
                other => panic!("expected Update, got {other:?}"),
            },
            other => panic!("expected Options, got {other:?}"),
        }
    }

    #[test]
    fn options_reorder_with_after_parses() {
        let parsed = parse_field(&[
            "options",
            "customfield_10001",
            "10100",
            "reorder",
            "1",
            "2",
            "--after",
            "3",
        ])
        .unwrap();
        match parsed.command {
            JiraFieldSubcommand::Options(o) => match o.command {
                JiraFieldOptionsSubcommand::Reorder(args) => {
                    assert_eq!(args.option_ids, vec!["1", "2"]);
                    assert_eq!(args.after.as_deref(), Some("3"));
                    assert!(args.position.is_none());
                }
                other => panic!("expected Reorder, got {other:?}"),
            },
            other => panic!("expected Options, got {other:?}"),
        }
    }

    #[test]
    fn options_reorder_without_target_errors() {
        let err =
            parse_field(&["options", "customfield_10001", "10100", "reorder", "1"]).unwrap_err();
        // The ArgGroup is `required = true`, so clap rejects a reorder call
        // that supplies neither `--after` nor `--position`.
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::MissingRequiredArgument,
            "expected MissingRequiredArgument, got: {err}"
        );
    }

    #[test]
    fn options_reorder_with_both_after_and_position_errors() {
        let err = parse_field(&[
            "options",
            "customfield_10001",
            "10100",
            "reorder",
            "1",
            "--after",
            "3",
            "--position",
            "first",
        ])
        .unwrap_err();
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "expected ArgumentConflict, got: {err}"
        );
    }
}
