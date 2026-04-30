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

// -- Component --

fn build_component_create_payload(args: &JiraComponentCreateArgs) -> Value {
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
    payload
}

fn build_component_update_payload(args: &JiraComponentUpdateArgs) -> anyhow::Result<Value> {
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
    Ok(Value::Object(fields))
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
            client
                .create_component(&build_component_create_payload(args))
                .await?
        }
        JiraComponentSubcommand::Update(args) => {
            client
                .update_component(&args.id, &build_component_update_payload(args)?)
                .await?
        }
        JiraComponentSubcommand::Delete(args) => {
            client.delete_component(&args.id).await?;
            Value::String(format!("Component {} deleted", args.id))
        }
    })
}

// -- Version --

fn build_version_create_payload(args: &JiraVersionCreateArgs) -> Value {
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
    payload
}

fn build_version_update_payload(args: &JiraVersionUpdateArgs) -> anyhow::Result<Value> {
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
    Ok(Value::Object(fields))
}

pub(super) async fn dispatch_version(
    cmd: &JiraVersionSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraVersionSubcommand::List(args) => client.get_project_versions(&args.project_key).await?,
        JiraVersionSubcommand::Get(args) => client.get_version(&args.id).await?,
        JiraVersionSubcommand::Create(args) => {
            client
                .create_version(&build_version_create_payload(args))
                .await?
        }
        JiraVersionSubcommand::Update(args) => {
            client
                .update_version(&args.id, &build_version_update_payload(args)?)
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

// -- Dashboard --

fn build_dashboard_create_payload(args: &JiraDashboardCreateArgs) -> Value {
    let mut payload = json!({ "name": &args.name });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

fn build_dashboard_update_payload(args: &JiraDashboardUpdateArgs) -> anyhow::Result<Value> {
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
    Ok(Value::Object(fields))
}

fn build_dashboard_copy_payload(args: &JiraDashboardCopyArgs) -> Value {
    let mut payload = serde_json::Map::new();
    if let Some(name) = &args.name {
        payload.insert("name".into(), Value::String(name.clone()));
    }
    Value::Object(payload)
}

fn build_dashboard_add_gadget_payload(args: &JiraDashboardAddGadgetArgs) -> anyhow::Result<Value> {
    let mut payload = json!({ "uri": &args.uri });
    if let Some(color) = &args.color {
        payload["color"] = Value::String(color.clone());
    }
    if let Some(pos) = &args.position {
        payload["position"] = parse_gadget_position(pos)?;
    }
    Ok(payload)
}

fn build_dashboard_update_gadget_payload(
    args: &JiraDashboardUpdateGadgetArgs,
) -> anyhow::Result<Value> {
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
    Ok(payload)
}

pub(super) async fn dispatch_dashboard(
    cmd: &JiraDashboardSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraDashboardSubcommand::List => client.list_dashboards().await?,
        JiraDashboardSubcommand::Get(args) => client.get_dashboard(&args.id).await?,
        JiraDashboardSubcommand::Create(args) => {
            client
                .create_dashboard(&build_dashboard_create_payload(args))
                .await?
        }
        JiraDashboardSubcommand::Update(args) => {
            client
                .update_dashboard(&args.id, &build_dashboard_update_payload(args)?)
                .await?
        }
        JiraDashboardSubcommand::Delete(args) => {
            client.delete_dashboard(&args.id).await?;
            Value::String(format!("Dashboard {} deleted", args.id))
        }
        JiraDashboardSubcommand::Copy(args) => {
            client
                .copy_dashboard(&args.id, &build_dashboard_copy_payload(args))
                .await?
        }
        JiraDashboardSubcommand::Gadgets(args) => client.list_dashboard_gadgets(&args.id).await?,
        JiraDashboardSubcommand::AddGadget(args) => {
            client
                .add_dashboard_gadget(
                    &args.dashboard_id,
                    &build_dashboard_add_gadget_payload(args)?,
                )
                .await?
        }
        JiraDashboardSubcommand::UpdateGadget(args) => {
            client
                .update_dashboard_gadget(
                    &args.dashboard_id,
                    &args.gadget_id,
                    &build_dashboard_update_gadget_payload(args)?,
                )
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

// -- Link Type --

fn build_link_type_create_payload(args: &JiraLinkTypeCreateArgs) -> Value {
    json!({
        "name": &args.name,
        "inward": &args.inward,
        "outward": &args.outward,
    })
}

fn build_link_type_update_payload(args: &JiraLinkTypeUpdateArgs) -> anyhow::Result<Value> {
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
            "no fields to update; specify at least one of --name, --inward, --outward".into(),
        )
        .into());
    }
    Ok(Value::Object(fields))
}

pub(super) async fn dispatch_link_type(
    cmd: &JiraLinkTypeSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraLinkTypeSubcommand::List => client.get_issue_link_types().await?,
        JiraLinkTypeSubcommand::Get(args) => client.get_issue_link_type(&args.id).await?,
        JiraLinkTypeSubcommand::Create(args) => {
            client
                .create_issue_link_type(&build_link_type_create_payload(args))
                .await?
        }
        JiraLinkTypeSubcommand::Update(args) => {
            client
                .update_issue_link_type(&args.id, &build_link_type_update_payload(args)?)
                .await?
        }
        JiraLinkTypeSubcommand::Delete(args) => {
            client.delete_issue_link_type(&args.id).await?;
            Value::String(format!("Issue link type {} deleted", args.id))
        }
    })
}

// -- Role --

fn build_role_create_payload(args: &JiraRoleCreateArgs) -> Value {
    let mut payload = json!({ "name": &args.name });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

pub(super) async fn dispatch_role(
    cmd: &JiraRoleSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraRoleSubcommand::List => client.list_roles().await?,
        JiraRoleSubcommand::Get(args) => client.get_role(&args.id).await?,
        JiraRoleSubcommand::Create(args) => {
            client.create_role(&build_role_create_payload(args)).await?
        }
        JiraRoleSubcommand::Delete(args) => {
            client.delete_role(&args.id).await?;
            Value::String(format!("Role {} deleted", args.id))
        }
    })
}

// -- Banner --

fn build_banner_set_payload(args: &JiraBannerSetArgs) -> Value {
    let mut payload = json!({ "message": &args.message });
    if let Some(enabled) = args.is_enabled {
        payload["isEnabled"] = Value::Bool(enabled);
    }
    if let Some(vis) = &args.visibility {
        payload["visibility"] = Value::String(vis.clone());
    }
    payload
}

pub(super) async fn dispatch_banner(
    cmd: &JiraBannerSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraBannerSubcommand::Get => client.get_banner().await?,
        JiraBannerSubcommand::Set(args) => {
            client.set_banner(&build_banner_set_payload(args)).await?
        }
    })
}

pub(super) async fn dispatch_task(
    cmd: &JiraTaskSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    // All branches are pure HTTP delegation; covered by contract tests in tests/contract_jira_*.rs.
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
    // All branches are pure HTTP delegation; covered by contract tests in tests/contract_jira_*.rs.
    Ok(match cmd {
        JiraAttachmentAdminSubcommand::Get(args) => client.get_attachment(&args.id).await?,
        JiraAttachmentAdminSubcommand::Delete(args) => {
            client.delete_attachment(&args.id).await?;
            Value::String(format!("Attachment {} deleted", args.id))
        }
        JiraAttachmentAdminSubcommand::Meta => client.get_attachment_meta().await?,
    })
}

// -- Project Category --

fn build_project_category_create_payload(args: &JiraProjectCategoryCreateArgs) -> Value {
    let mut payload = json!({ "name": &args.name });
    if let Some(desc) = &args.description {
        payload["description"] = Value::String(desc.clone());
    }
    payload
}

fn build_project_category_update_payload(
    args: &JiraProjectCategoryUpdateArgs,
) -> anyhow::Result<Value> {
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
    Ok(Value::Object(fields))
}

pub(super) async fn dispatch_project_category(
    cmd: &JiraProjectCategorySubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraProjectCategorySubcommand::List => client.list_project_categories().await?,
        JiraProjectCategorySubcommand::Get(args) => client.get_project_category(&args.id).await?,
        JiraProjectCategorySubcommand::Create(args) => {
            client
                .create_project_category(&build_project_category_create_payload(args))
                .await?
        }
        JiraProjectCategorySubcommand::Update(args) => {
            client
                .update_project_category(&args.id, &build_project_category_update_payload(args)?)
                .await?
        }
        JiraProjectCategorySubcommand::Delete(args) => {
            client.delete_project_category(&args.id).await?;
            Value::String(format!("Project category {} deleted", args.id))
        }
    })
}

// -- Webhook --

/// Parse a comma-separated `--events` list into a vector of trimmed event names. Returns an
/// `InvalidInput` error if any segment (after trim) is empty.
fn parse_webhook_events(raw: &str) -> anyhow::Result<Vec<String>> {
    let events: Vec<&str> = raw.split(',').map(str::trim).collect();
    if events.iter().any(|e| e.is_empty()) {
        return Err(Error::InvalidInput(
            "invalid --events; expected a comma-separated list of non-empty event names".into(),
        )
        .into());
    }
    Ok(events.into_iter().map(str::to_owned).collect())
}

fn build_webhook_create_payload(args: &JiraWebhookCreateArgs) -> anyhow::Result<Value> {
    let events = parse_webhook_events(&args.events)?;
    let events_value: Vec<Value> = events.into_iter().map(Value::String).collect();
    let mut payload = json!({
        "name": &args.name,
        "url": &args.url,
        "events": events_value,
    });
    if let Some(jql) = &args.jql {
        payload["filters"] = json!({ "issue-related-events-section": jql });
    }
    Ok(payload)
}

pub(super) async fn dispatch_webhook(
    cmd: &JiraWebhookSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraWebhookSubcommand::List => client.list_webhooks().await?,
        JiraWebhookSubcommand::Get(args) => client.get_webhook(&args.id).await?,
        JiraWebhookSubcommand::Create(args) => {
            client
                .create_webhook(&build_webhook_create_payload(args)?)
                .await?
        }
        JiraWebhookSubcommand::Delete(args) => {
            client.delete_webhook(&args.id).await?;
            Value::String(format!("Webhook {} deleted", args.id))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- parse_gadget_position -----------------------------------------------

    #[test]
    fn parse_valid_position() {
        let result = parse_gadget_position("0:1").unwrap();
        assert_eq!(result, json!({"row": 0, "column": 1}));
    }

    #[test]
    fn parse_large_numbers() {
        let result = parse_gadget_position("100:200").unwrap();
        assert_eq!(result, json!({"row": 100, "column": 200}));
    }

    #[test]
    fn parse_missing_colon_errors() {
        let err = parse_gadget_position("123").unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    #[test]
    fn parse_non_numeric_errors() {
        let err = parse_gadget_position("a:b").unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    // --- build_component_create_payload --------------------------------------

    fn component_create(
        name: &str,
        description: Option<&str>,
        lead: Option<&str>,
    ) -> JiraComponentCreateArgs {
        JiraComponentCreateArgs {
            project: "PROJ".to_string(),
            name: name.to_string(),
            description: description.map(str::to_string),
            lead: lead.map(str::to_string),
        }
    }

    #[test]
    fn component_create_minimal() {
        let payload = build_component_create_payload(&component_create("UI", None, None));
        assert_eq!(payload, json!({ "project": "PROJ", "name": "UI" }));
    }

    #[test]
    fn component_create_with_description_and_lead() {
        let payload =
            build_component_create_payload(&component_create("UI", Some("frontend"), Some("acct")));
        assert_eq!(
            payload,
            json!({
                "project": "PROJ",
                "name": "UI",
                "description": "frontend",
                "leadAccountId": "acct",
            })
        );
    }

    // --- build_component_update_payload --------------------------------------

    fn component_update(
        name: Option<&str>,
        description: Option<&str>,
        lead: Option<&str>,
        assignee_type: Option<&str>,
    ) -> JiraComponentUpdateArgs {
        JiraComponentUpdateArgs {
            id: "1".to_string(),
            name: name.map(str::to_string),
            description: description.map(str::to_string),
            lead: lead.map(str::to_string),
            assignee_type: assignee_type.map(str::to_string),
        }
    }

    #[test]
    fn component_update_with_assignee_type_only() {
        let payload = build_component_update_payload(&component_update(
            None,
            None,
            None,
            Some("PROJECT_LEAD"),
        ))
        .unwrap();
        assert_eq!(payload, json!({ "assigneeType": "PROJECT_LEAD" }));
    }

    #[test]
    fn component_update_with_all_fields() {
        let payload = build_component_update_payload(&component_update(
            Some("N"),
            Some("D"),
            Some("L"),
            Some("UNASSIGNED"),
        ))
        .unwrap();
        assert_eq!(
            payload,
            json!({
                "name": "N",
                "description": "D",
                "leadAccountId": "L",
                "assigneeType": "UNASSIGNED",
            })
        );
    }

    #[test]
    fn component_update_with_no_fields_errors_with_invalid_input() {
        let err =
            build_component_update_payload(&component_update(None, None, None, None)).unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
        assert!(err.to_string().contains("--assignee-type"), "got: {err}");
    }

    // --- build_version_create_payload ----------------------------------------

    fn version_create(
        name: &str,
        description: Option<&str>,
        release_date: Option<&str>,
    ) -> JiraVersionCreateArgs {
        JiraVersionCreateArgs {
            project: "PROJ".to_string(),
            name: name.to_string(),
            description: description.map(str::to_string),
            release_date: release_date.map(str::to_string),
        }
    }

    #[test]
    fn version_create_minimal() {
        let payload = build_version_create_payload(&version_create("v1", None, None));
        assert_eq!(payload, json!({ "project": "PROJ", "name": "v1" }));
    }

    #[test]
    fn version_create_with_description_and_release_date() {
        let payload =
            build_version_create_payload(&version_create("v1", Some("first"), Some("2024-01-31")));
        assert_eq!(
            payload,
            json!({
                "project": "PROJ",
                "name": "v1",
                "description": "first",
                "releaseDate": "2024-01-31",
            })
        );
    }

    // --- build_version_update_payload ----------------------------------------

    fn version_update(
        name: Option<&str>,
        description: Option<&str>,
        start_date: Option<&str>,
        release_date: Option<&str>,
        released: Option<bool>,
        archived: Option<bool>,
    ) -> JiraVersionUpdateArgs {
        JiraVersionUpdateArgs {
            id: "1".to_string(),
            name: name.map(str::to_string),
            description: description.map(str::to_string),
            start_date: start_date.map(str::to_string),
            release_date: release_date.map(str::to_string),
            released,
            archived,
        }
    }

    #[test]
    fn version_update_with_released_false() {
        // Distinguish "not set" from explicit `false` — must serialize the bool.
        let payload = build_version_update_payload(&version_update(
            None,
            None,
            None,
            None,
            Some(false),
            None,
        ))
        .unwrap();
        assert_eq!(payload, json!({ "released": false }));
    }

    #[test]
    fn version_update_with_archived_true() {
        let payload =
            build_version_update_payload(&version_update(None, None, None, None, None, Some(true)))
                .unwrap();
        assert_eq!(payload, json!({ "archived": true }));
    }

    #[test]
    fn version_update_with_all_fields() {
        let payload = build_version_update_payload(&version_update(
            Some("N"),
            Some("D"),
            Some("2024-01-01"),
            Some("2024-12-31"),
            Some(true),
            Some(false),
        ))
        .unwrap();
        assert_eq!(
            payload,
            json!({
                "name": "N",
                "description": "D",
                "startDate": "2024-01-01",
                "releaseDate": "2024-12-31",
                "released": true,
                "archived": false,
            })
        );
    }

    #[test]
    fn version_update_with_no_fields_errors_with_invalid_input() {
        let err = build_version_update_payload(&version_update(None, None, None, None, None, None))
            .unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("--released"), "got: {msg}");
        assert!(msg.contains("--archived"), "got: {msg}");
    }

    // --- build_dashboard_create_payload --------------------------------------

    #[test]
    fn dashboard_create_minimal() {
        let args = JiraDashboardCreateArgs {
            name: "Dash".to_string(),
            description: None,
        };
        assert_eq!(
            build_dashboard_create_payload(&args),
            json!({ "name": "Dash" })
        );
    }

    #[test]
    fn dashboard_create_with_description() {
        let args = JiraDashboardCreateArgs {
            name: "Dash".to_string(),
            description: Some("d".to_string()),
        };
        assert_eq!(
            build_dashboard_create_payload(&args),
            json!({ "name": "Dash", "description": "d" })
        );
    }

    // --- build_dashboard_update_payload --------------------------------------

    #[test]
    fn dashboard_update_with_name_only() {
        let args = JiraDashboardUpdateArgs {
            id: "1".into(),
            name: Some("N".into()),
            description: None,
        };
        assert_eq!(
            build_dashboard_update_payload(&args).unwrap(),
            json!({ "name": "N" })
        );
    }

    #[test]
    fn dashboard_update_with_no_fields_errors_with_invalid_input() {
        let args = JiraDashboardUpdateArgs {
            id: "1".into(),
            name: None,
            description: None,
        };
        let err = build_dashboard_update_payload(&args).unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    // --- build_dashboard_copy_payload ----------------------------------------

    #[test]
    fn dashboard_copy_with_no_name_yields_empty_object() {
        // Server-side: omitting `name` keeps the original (with " (copy)" suffix). The empty
        // object is the documented contract — keep it explicit.
        let args = JiraDashboardCopyArgs {
            id: "1".into(),
            name: None,
        };
        assert_eq!(build_dashboard_copy_payload(&args), json!({}));
    }

    #[test]
    fn dashboard_copy_with_name_includes_field() {
        let args = JiraDashboardCopyArgs {
            id: "1".into(),
            name: Some("My copy".into()),
        };
        assert_eq!(
            build_dashboard_copy_payload(&args),
            json!({ "name": "My copy" })
        );
    }

    // --- build_dashboard_add_gadget_payload ----------------------------------

    #[test]
    fn dashboard_add_gadget_minimal_only_uri() {
        let args = JiraDashboardAddGadgetArgs {
            dashboard_id: "1".into(),
            uri: "rest://x".into(),
            color: None,
            position: None,
        };
        assert_eq!(
            build_dashboard_add_gadget_payload(&args).unwrap(),
            json!({ "uri": "rest://x" })
        );
    }

    #[test]
    fn dashboard_add_gadget_with_color_and_position() {
        let args = JiraDashboardAddGadgetArgs {
            dashboard_id: "1".into(),
            uri: "rest://x".into(),
            color: Some("blue".into()),
            position: Some("2:3".into()),
        };
        assert_eq!(
            build_dashboard_add_gadget_payload(&args).unwrap(),
            json!({
                "uri": "rest://x",
                "color": "blue",
                "position": { "row": 2, "column": 3 },
            })
        );
    }

    #[test]
    fn dashboard_add_gadget_with_invalid_position_errors() {
        let args = JiraDashboardAddGadgetArgs {
            dashboard_id: "1".into(),
            uri: "rest://x".into(),
            color: None,
            position: Some("not-a-position".into()),
        };
        let err = build_dashboard_add_gadget_payload(&args).unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    // --- build_dashboard_update_gadget_payload -------------------------------

    #[test]
    fn dashboard_update_gadget_with_color_only() {
        let args = JiraDashboardUpdateGadgetArgs {
            dashboard_id: "1".into(),
            gadget_id: "g1".into(),
            color: Some("red".into()),
            position: None,
        };
        assert_eq!(
            build_dashboard_update_gadget_payload(&args).unwrap(),
            json!({ "color": "red" })
        );
    }

    #[test]
    fn dashboard_update_gadget_with_position_only() {
        let args = JiraDashboardUpdateGadgetArgs {
            dashboard_id: "1".into(),
            gadget_id: "g1".into(),
            color: None,
            position: Some("0:0".into()),
        };
        assert_eq!(
            build_dashboard_update_gadget_payload(&args).unwrap(),
            json!({ "position": { "row": 0, "column": 0 } })
        );
    }

    #[test]
    fn dashboard_update_gadget_with_no_fields_errors_with_invalid_input() {
        let args = JiraDashboardUpdateGadgetArgs {
            dashboard_id: "1".into(),
            gadget_id: "g1".into(),
            color: None,
            position: None,
        };
        let err = build_dashboard_update_gadget_payload(&args).unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
        assert!(
            err.to_string().contains("--color"),
            "error should mention --color, got: {err}"
        );
    }

    // --- build_link_type_create_payload --------------------------------------

    #[test]
    fn link_type_create_serializes_all_required_fields() {
        let args = JiraLinkTypeCreateArgs {
            name: "Blocks".into(),
            inward: "is blocked by".into(),
            outward: "blocks".into(),
        };
        assert_eq!(
            build_link_type_create_payload(&args),
            json!({
                "name": "Blocks",
                "inward": "is blocked by",
                "outward": "blocks",
            })
        );
    }

    // --- build_link_type_update_payload --------------------------------------

    fn link_type_update(
        name: Option<&str>,
        inward: Option<&str>,
        outward: Option<&str>,
    ) -> JiraLinkTypeUpdateArgs {
        JiraLinkTypeUpdateArgs {
            id: "10000".into(),
            name: name.map(str::to_string),
            inward: inward.map(str::to_string),
            outward: outward.map(str::to_string),
        }
    }

    #[test]
    fn link_type_update_with_inward_only() {
        let payload =
            build_link_type_update_payload(&link_type_update(None, Some("blocked"), None)).unwrap();
        assert_eq!(payload, json!({ "inward": "blocked" }));
    }

    #[test]
    fn link_type_update_with_all_fields() {
        let payload =
            build_link_type_update_payload(&link_type_update(Some("N"), Some("I"), Some("O")))
                .unwrap();
        assert_eq!(
            payload,
            json!({ "name": "N", "inward": "I", "outward": "O" })
        );
    }

    #[test]
    fn link_type_update_with_no_fields_errors_with_invalid_input() {
        let err = build_link_type_update_payload(&link_type_update(None, None, None)).unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    // --- build_role_create_payload -------------------------------------------

    #[test]
    fn role_create_minimal() {
        let args = JiraRoleCreateArgs {
            name: "Admin".into(),
            description: None,
        };
        assert_eq!(build_role_create_payload(&args), json!({ "name": "Admin" }));
    }

    #[test]
    fn role_create_with_description() {
        let args = JiraRoleCreateArgs {
            name: "Admin".into(),
            description: Some("d".into()),
        };
        assert_eq!(
            build_role_create_payload(&args),
            json!({ "name": "Admin", "description": "d" })
        );
    }

    // --- build_banner_set_payload --------------------------------------------

    #[test]
    fn banner_set_message_only() {
        let args = JiraBannerSetArgs {
            message: "Hi".into(),
            is_enabled: None,
            visibility: None,
        };
        assert_eq!(build_banner_set_payload(&args), json!({ "message": "Hi" }));
    }

    #[test]
    fn banner_set_with_is_enabled_false() {
        let args = JiraBannerSetArgs {
            message: "Hi".into(),
            is_enabled: Some(false),
            visibility: None,
        };
        assert_eq!(
            build_banner_set_payload(&args),
            json!({ "message": "Hi", "isEnabled": false })
        );
    }

    #[test]
    fn banner_set_with_all_fields() {
        let args = JiraBannerSetArgs {
            message: "Hi".into(),
            is_enabled: Some(true),
            visibility: Some("public".into()),
        };
        assert_eq!(
            build_banner_set_payload(&args),
            json!({ "message": "Hi", "isEnabled": true, "visibility": "public" })
        );
    }

    // --- build_project_category_create_payload ------------------------------

    #[test]
    fn project_category_create_minimal() {
        let args = JiraProjectCategoryCreateArgs {
            name: "Cat".into(),
            description: None,
        };
        assert_eq!(
            build_project_category_create_payload(&args),
            json!({ "name": "Cat" })
        );
    }

    #[test]
    fn project_category_create_with_description() {
        let args = JiraProjectCategoryCreateArgs {
            name: "Cat".into(),
            description: Some("d".into()),
        };
        assert_eq!(
            build_project_category_create_payload(&args),
            json!({ "name": "Cat", "description": "d" })
        );
    }

    // --- build_project_category_update_payload ------------------------------

    #[test]
    fn project_category_update_with_name_only() {
        let args = JiraProjectCategoryUpdateArgs {
            id: "1".into(),
            name: Some("N".into()),
            description: None,
        };
        assert_eq!(
            build_project_category_update_payload(&args).unwrap(),
            json!({ "name": "N" })
        );
    }

    #[test]
    fn project_category_update_with_no_fields_errors_with_invalid_input() {
        let args = JiraProjectCategoryUpdateArgs {
            id: "1".into(),
            name: None,
            description: None,
        };
        let err = build_project_category_update_payload(&args).unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    // --- parse_webhook_events ------------------------------------------------

    #[test]
    fn parse_webhook_events_simple() {
        let events = parse_webhook_events("jira:issue_created,jira:issue_updated").unwrap();
        assert_eq!(events, vec!["jira:issue_created", "jira:issue_updated"]);
    }

    #[test]
    fn parse_webhook_events_trims_whitespace() {
        let events = parse_webhook_events(" jira:issue_created , jira:issue_updated ").unwrap();
        assert_eq!(events, vec!["jira:issue_created", "jira:issue_updated"]);
    }

    #[test]
    fn parse_webhook_events_empty_string_errors() {
        let err = parse_webhook_events("").unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    #[test]
    fn parse_webhook_events_trailing_comma_errors() {
        let err = parse_webhook_events("jira:issue_created,").unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    #[test]
    fn parse_webhook_events_double_comma_errors() {
        let err = parse_webhook_events("a,,b").unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }

    // --- build_webhook_create_payload ---------------------------------------

    fn webhook_create(
        name: &str,
        url: &str,
        events: &str,
        jql: Option<&str>,
    ) -> JiraWebhookCreateArgs {
        JiraWebhookCreateArgs {
            name: name.to_string(),
            url: url.to_string(),
            events: events.to_string(),
            jql: jql.map(str::to_string),
        }
    }

    #[test]
    fn webhook_create_minimal_no_jql() {
        let payload = build_webhook_create_payload(&webhook_create(
            "hook",
            "https://example.com/hook",
            "jira:issue_created",
            None,
        ))
        .unwrap();
        assert_eq!(
            payload,
            json!({
                "name": "hook",
                "url": "https://example.com/hook",
                "events": ["jira:issue_created"],
            })
        );
    }

    #[test]
    fn webhook_create_with_jql_filter() {
        let payload = build_webhook_create_payload(&webhook_create(
            "hook",
            "https://example.com/hook",
            "jira:issue_created,jira:issue_updated",
            Some("project = PROJ"),
        ))
        .unwrap();
        assert_eq!(
            payload,
            json!({
                "name": "hook",
                "url": "https://example.com/hook",
                "events": ["jira:issue_created", "jira:issue_updated"],
                "filters": { "issue-related-events-section": "project = PROJ" },
            })
        );
    }

    #[test]
    fn webhook_create_with_invalid_events_errors() {
        let err = build_webhook_create_payload(&webhook_create(
            "hook",
            "https://example.com/hook",
            "jira:issue_created,,",
            None,
        ))
        .unwrap_err();
        let domain = err.downcast_ref::<Error>();
        assert!(
            matches!(domain, Some(Error::InvalidInput(_))),
            "expected Error::InvalidInput, got: {err:?}"
        );
    }
}
