use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

/// Build the create payload for a project.
fn build_project_create_payload(args: &JiraProjectCreateArgs) -> Value {
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
    payload
}

/// Build the update payload for a project. Errors if no field is supplied.
fn build_project_update_payload(args: &JiraProjectUpdateArgs) -> anyhow::Result<Value> {
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
        anyhow::bail!("no fields to update; specify at least one of --name, --lead, --description");
    }
    Ok(Value::Object(fields))
}

pub(super) async fn dispatch_project(
    cmd: &JiraProjectSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraProjectSubcommand::List => client.get_projects().await?,
        JiraProjectSubcommand::Get(args) => client.get_project(&args.project_key).await?,
        JiraProjectSubcommand::Create(args) => {
            client
                .create_project(&build_project_create_payload(args))
                .await?
        }
        JiraProjectSubcommand::Update(args) => {
            client
                .update_project(&args.key, &build_project_update_payload(args)?)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn project_create(
        key: &str,
        name: &str,
        project_type_key: &str,
        lead: &str,
        description: Option<&str>,
        template: Option<&str>,
    ) -> JiraProjectCreateArgs {
        JiraProjectCreateArgs {
            key: key.to_string(),
            name: name.to_string(),
            project_type_key: project_type_key.to_string(),
            lead: lead.to_string(),
            description: description.map(str::to_string),
            template: template.map(str::to_string),
        }
    }

    #[test]
    fn project_create_minimal() {
        let payload = build_project_create_payload(&project_create(
            "PROJ", "Project", "software", "lead-id", None, None,
        ));
        assert_eq!(
            payload,
            json!({
                "key": "PROJ",
                "name": "Project",
                "projectTypeKey": "software",
                "leadAccountId": "lead-id",
            })
        );
    }

    #[test]
    fn project_create_with_description_only() {
        let payload = build_project_create_payload(&project_create(
            "PROJ",
            "Project",
            "software",
            "lead-id",
            Some("d"),
            None,
        ));
        assert_eq!(
            payload,
            json!({
                "key": "PROJ",
                "name": "Project",
                "projectTypeKey": "software",
                "leadAccountId": "lead-id",
                "description": "d",
            })
        );
    }

    #[test]
    fn project_create_with_template_only() {
        let payload = build_project_create_payload(&project_create(
            "PROJ",
            "Project",
            "software",
            "lead-id",
            None,
            Some("scrum-template"),
        ));
        assert_eq!(
            payload,
            json!({
                "key": "PROJ",
                "name": "Project",
                "projectTypeKey": "software",
                "leadAccountId": "lead-id",
                "projectTemplateKey": "scrum-template",
            })
        );
    }

    #[test]
    fn project_create_with_all_fields() {
        let payload = build_project_create_payload(&project_create(
            "PROJ",
            "Project",
            "software",
            "lead-id",
            Some("d"),
            Some("scrum-template"),
        ));
        assert_eq!(
            payload,
            json!({
                "key": "PROJ",
                "name": "Project",
                "projectTypeKey": "software",
                "leadAccountId": "lead-id",
                "description": "d",
                "projectTemplateKey": "scrum-template",
            })
        );
    }

    fn project_update(
        name: Option<&str>,
        lead: Option<&str>,
        description: Option<&str>,
    ) -> JiraProjectUpdateArgs {
        JiraProjectUpdateArgs {
            key: "PROJ".to_string(),
            name: name.map(str::to_string),
            lead: lead.map(str::to_string),
            description: description.map(str::to_string),
        }
    }

    #[test]
    fn project_update_with_name_only() {
        let payload =
            build_project_update_payload(&project_update(Some("New"), None, None)).unwrap();
        assert_eq!(payload, json!({ "name": "New" }));
    }

    #[test]
    fn project_update_with_lead_only() {
        let payload =
            build_project_update_payload(&project_update(None, Some("acct"), None)).unwrap();
        assert_eq!(payload, json!({ "leadAccountId": "acct" }));
    }

    #[test]
    fn project_update_with_description_only() {
        let payload = build_project_update_payload(&project_update(None, None, Some("d"))).unwrap();
        assert_eq!(payload, json!({ "description": "d" }));
    }

    #[test]
    fn project_update_with_all_fields() {
        let payload =
            build_project_update_payload(&project_update(Some("N"), Some("L"), Some("D"))).unwrap();
        assert_eq!(
            payload,
            json!({ "name": "N", "leadAccountId": "L", "description": "D" })
        );
    }

    #[test]
    fn project_update_with_no_fields_errors_and_lists_options() {
        let err = build_project_update_payload(&project_update(None, None, None)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no fields to update"), "got: {msg}");
        assert!(msg.contains("--name"), "got: {msg}");
        assert!(msg.contains("--lead"), "got: {msg}");
        assert!(msg.contains("--description"), "got: {msg}");
    }
}
