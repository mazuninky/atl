use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

/// Build the create payload for a generic CRUD scheme resource (workflow / permission /
/// notification / issue-security scheme).
fn build_scheme_create_payload(args: &JiraSchemeCreateArgs) -> Value {
    let mut payload = json!({ "name": &args.name });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

/// Build the update payload for a generic CRUD scheme resource. Returns an error if no
/// fields were supplied — callers must specify at least one of `--name` or `--description`.
fn build_scheme_update_payload(args: &JiraSchemeUpdateArgs) -> anyhow::Result<Value> {
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

/// Build the create payload for a screen.
fn build_screen_create_payload(args: &JiraScreenCreateArgs) -> Value {
    let mut payload = json!({ "name": &args.name });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

/// Build the create payload for an issue type scheme. Includes optional default issue type.
fn build_issue_type_scheme_create_payload(args: &JiraIssueTypeSchemeCreateArgs) -> Value {
    let mut payload = json!({ "name": &args.name });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    if let Some(dit) = &args.default_issue_type_id {
        payload["defaultIssueTypeId"] = Value::String(dit.clone());
    }
    payload
}

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
            client
                .create_workflow_scheme(&build_scheme_create_payload(args))
                .await?
        }
        JiraCrudSubcommand::Update(args) => {
            client
                .update_workflow_scheme(&args.id, &build_scheme_update_payload(args)?)
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
            client
                .create_screen(&build_screen_create_payload(args))
                .await?
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
            client
                .create_permission_scheme(&build_scheme_create_payload(args))
                .await?
        }
        JiraCrudSubcommand::Update(args) => {
            client
                .update_permission_scheme(&args.id, &build_scheme_update_payload(args)?)
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
            client
                .create_notification_scheme(&build_scheme_create_payload(args))
                .await?
        }
        JiraCrudSubcommand::Update(args) => {
            client
                .update_notification_scheme(&args.id, &build_scheme_update_payload(args)?)
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
            client
                .create_issue_security_scheme(&build_scheme_create_payload(args))
                .await?
        }
        JiraCrudSubcommand::Update(args) => {
            client
                .update_issue_security_scheme(&args.id, &build_scheme_update_payload(args)?)
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
            client
                .create_field_configuration(&build_scheme_create_payload(args))
                .await?
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
            client
                .create_issue_type_scheme(&build_issue_type_scheme_create_payload(args))
                .await?
        }
        JiraIssueTypeSchemeSubcommand::Update(args) => {
            client
                .update_issue_type_scheme(&args.id, &build_scheme_update_payload(args)?)
                .await?
        }
        JiraIssueTypeSchemeSubcommand::Delete(args) => {
            client.delete_issue_type_scheme(&args.id).await?;
            Value::String(format!("Issue type scheme {} deleted", args.id))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheme_create(name: &str, description: Option<&str>) -> JiraSchemeCreateArgs {
        JiraSchemeCreateArgs {
            name: name.to_string(),
            description: description.map(str::to_string),
        }
    }

    fn scheme_update(
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> JiraSchemeUpdateArgs {
        JiraSchemeUpdateArgs {
            id: id.to_string(),
            name: name.map(str::to_string),
            description: description.map(str::to_string),
        }
    }

    // --- build_scheme_create_payload ------------------------------------------

    #[test]
    fn scheme_create_with_name_only() {
        let payload = build_scheme_create_payload(&scheme_create("My scheme", None));
        assert_eq!(payload, json!({ "name": "My scheme" }));
    }

    #[test]
    fn scheme_create_with_description() {
        let payload = build_scheme_create_payload(&scheme_create("My scheme", Some("Desc")));
        assert_eq!(
            payload,
            json!({ "name": "My scheme", "description": "Desc" })
        );
    }

    // --- build_scheme_update_payload ------------------------------------------

    #[test]
    fn scheme_update_with_name() {
        let payload = build_scheme_update_payload(&scheme_update("42", Some("New"), None)).unwrap();
        assert_eq!(payload, json!({ "name": "New" }));
    }

    #[test]
    fn scheme_update_with_description() {
        let payload =
            build_scheme_update_payload(&scheme_update("42", None, Some("New desc"))).unwrap();
        assert_eq!(payload, json!({ "description": "New desc" }));
    }

    #[test]
    fn scheme_update_with_both_fields() {
        let payload =
            build_scheme_update_payload(&scheme_update("42", Some("N"), Some("D"))).unwrap();
        assert_eq!(payload, json!({ "name": "N", "description": "D" }));
    }

    #[test]
    fn scheme_update_with_no_fields_errors() {
        let err = build_scheme_update_payload(&scheme_update("42", None, None)).unwrap_err();
        assert!(
            err.to_string().contains("no fields to update"),
            "expected guard message, got: {err}"
        );
        assert!(
            err.to_string().contains("--name"),
            "error should mention --name, got: {err}"
        );
        assert!(
            err.to_string().contains("--description"),
            "error should mention --description, got: {err}"
        );
    }

    // --- build_screen_create_payload ------------------------------------------

    #[test]
    fn screen_create_with_name_only() {
        let args = JiraScreenCreateArgs {
            name: "Screen 1".to_string(),
            description: None,
        };
        assert_eq!(
            build_screen_create_payload(&args),
            json!({ "name": "Screen 1" })
        );
    }

    #[test]
    fn screen_create_with_description() {
        let args = JiraScreenCreateArgs {
            name: "Screen 1".to_string(),
            description: Some("d".to_string()),
        };
        assert_eq!(
            build_screen_create_payload(&args),
            json!({ "name": "Screen 1", "description": "d" })
        );
    }

    // --- build_issue_type_scheme_create_payload -------------------------------

    #[test]
    fn issue_type_scheme_create_minimal() {
        let args = JiraIssueTypeSchemeCreateArgs {
            name: "Scheme".to_string(),
            description: None,
            default_issue_type_id: None,
        };
        assert_eq!(
            build_issue_type_scheme_create_payload(&args),
            json!({ "name": "Scheme" })
        );
    }

    #[test]
    fn issue_type_scheme_create_with_description_only() {
        let args = JiraIssueTypeSchemeCreateArgs {
            name: "Scheme".to_string(),
            description: Some("d".to_string()),
            default_issue_type_id: None,
        };
        assert_eq!(
            build_issue_type_scheme_create_payload(&args),
            json!({ "name": "Scheme", "description": "d" })
        );
    }

    #[test]
    fn issue_type_scheme_create_with_default_issue_type_only() {
        let args = JiraIssueTypeSchemeCreateArgs {
            name: "Scheme".to_string(),
            description: None,
            default_issue_type_id: Some("10001".to_string()),
        };
        assert_eq!(
            build_issue_type_scheme_create_payload(&args),
            json!({ "name": "Scheme", "defaultIssueTypeId": "10001" })
        );
    }

    #[test]
    fn issue_type_scheme_create_with_all_fields() {
        let args = JiraIssueTypeSchemeCreateArgs {
            name: "Scheme".to_string(),
            description: Some("d".to_string()),
            default_issue_type_id: Some("10001".to_string()),
        };
        assert_eq!(
            build_issue_type_scheme_create_payload(&args),
            json!({ "name": "Scheme", "description": "d", "defaultIssueTypeId": "10001" })
        );
    }
}
