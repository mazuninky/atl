use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

use super::property::dispatch_resource_property;

/// Build the JSON payload for creating a space role: a single `name` field.
pub(super) fn build_space_role_create_payload(name: &str) -> Value {
    serde_json::json!({ "name": name })
}

/// Build the JSON payload for updating a space role: a single `name` field
/// (same shape as create — it is intentionally factored together so any future
/// schema additions touch one place).
pub(super) fn build_space_role_update_payload(name: &str) -> Value {
    serde_json::json!({ "name": name })
}

/// Parse a JSON role-assignments body. Wraps `serde_json::from_str` so a
/// downstream context can distinguish parse failures from network errors.
pub(super) fn parse_role_assignments_body(body: &str) -> anyhow::Result<Value> {
    serde_json::from_str(body)
        .map_err(|e| anyhow::anyhow!("failed to parse role assignments body as JSON: {e}"))
}

pub(super) async fn dispatch_space(
    cmd: &ConfluenceSpaceSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceSpaceSubcommand::List(args) => {
            if args.all {
                client.get_spaces_all(args.limit).await?
            } else {
                client.get_spaces(args.limit).await?
            }
        }
        ConfluenceSpaceSubcommand::Get(args) => client.get_space_v2(&args.space_id).await?,
        ConfluenceSpaceSubcommand::Create(args) => {
            client
                .create_space_v2(
                    &args.key,
                    &args.name,
                    args.description.as_deref(),
                    args.private,
                    args.alias.as_deref(),
                    args.template_key.as_deref(),
                )
                .await?
        }
        ConfluenceSpaceSubcommand::Delete(args) => {
            client.delete_space_v2(&args.space_id).await?;
            Value::String(format!("Space {} deleted", args.space_id))
        }
        ConfluenceSpaceSubcommand::Pages(args) => {
            client
                .get_space_pages_v2(&args.space_id, args.limit)
                .await?
        }
        ConfluenceSpaceSubcommand::Blogposts(args) => {
            client
                .get_space_blogposts_v2(&args.space_id, args.limit)
                .await?
        }
        ConfluenceSpaceSubcommand::Labels(args) => {
            client
                .get_space_labels_v2(&args.space_id, args.limit)
                .await?
        }
        ConfluenceSpaceSubcommand::Permissions(args) => {
            client
                .get_space_permissions_v2(&args.space_id, args.limit)
                .await?
        }
        ConfluenceSpaceSubcommand::PermissionsAvailable => {
            client.get_space_permissions_available_v2().await?
        }
        ConfluenceSpaceSubcommand::ContentLabels(args) => {
            client
                .get_space_content_labels_v2(&args.space_id, args.limit)
                .await?
        }
        ConfluenceSpaceSubcommand::CustomContent(args) => {
            client
                .get_space_custom_content_v2(&args.space_id, &args.content_type, args.limit)
                .await?
        }
        ConfluenceSpaceSubcommand::Operations(args) => {
            client.get_space_operations_v2(&args.space_id).await?
        }
        ConfluenceSpaceSubcommand::RoleAssignments(args) => {
            client
                .get_space_role_assignments_v2(&args.space_id, args.limit)
                .await?
        }
        ConfluenceSpaceSubcommand::SetRoleAssignments(args) => {
            let body = read_body_arg(&args.body)?;
            let payload = parse_role_assignments_body(&body)?;
            client
                .set_space_role_assignments_v2(&args.space_id, &payload)
                .await?
        }
        ConfluenceSpaceSubcommand::Property(cmd) => {
            dispatch_resource_property("spaces", &cmd.command, client).await?
        }
        ConfluenceSpaceSubcommand::Role(cmd) => dispatch_space_role(&cmd.command, client).await?,
    })
}

async fn dispatch_space_role(
    cmd: &ConfluenceSpaceRoleSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceSpaceRoleSubcommand::List(args) => {
            client
                .list_space_roles_v2(&args.space_id, args.limit)
                .await?
        }
        ConfluenceSpaceRoleSubcommand::Get(args) => {
            client
                .get_space_role_v2(&args.space_id, &args.role_id)
                .await?
        }
        ConfluenceSpaceRoleSubcommand::Create(args) => {
            let payload = build_space_role_create_payload(&args.name);
            client
                .create_space_role_v2(&args.space_id, &payload)
                .await?
        }
        ConfluenceSpaceRoleSubcommand::Update(args) => {
            let payload = build_space_role_update_payload(&args.name);
            client
                .update_space_role_v2(&args.space_id, &args.role_id, &payload)
                .await?
        }
        ConfluenceSpaceRoleSubcommand::Delete(args) => {
            client
                .delete_space_role_v2(&args.space_id, &args.role_id)
                .await?;
            Value::String(format!("Role {} deleted", args.role_id))
        }
        ConfluenceSpaceRoleSubcommand::Mode(args) => {
            client.get_space_roles_mode_v2(&args.space_id).await?
        }
    })
}

#[cfg(test)]
mod tests {
    // Most arms are pure HTTP delegation (List/Get/Delete/Pages/etc.) and are
    // covered by contract tests in tests/contract_confluence_v*.rs. The
    // helpers below are the local pure logic worth unit-testing.

    use super::*;
    use serde_json::json;

    // ---- build_space_role_create_payload ----

    #[test]
    fn create_role_payload_has_only_name_field() {
        let payload = build_space_role_create_payload("Reviewer");
        assert_eq!(payload, json!({ "name": "Reviewer" }));
    }

    #[test]
    fn create_role_payload_handles_empty_name() {
        // The CLI does not enforce non-empty names locally; the server is the
        // authority. We just round-trip whatever we got.
        let payload = build_space_role_create_payload("");
        assert_eq!(payload, json!({ "name": "" }));
    }

    #[test]
    fn create_role_payload_preserves_unicode_and_quotes() {
        let payload = build_space_role_create_payload(r#"日本語 "with quotes""#);
        assert_eq!(payload["name"].as_str(), Some(r#"日本語 "with quotes""#));
    }

    // ---- build_space_role_update_payload ----

    #[test]
    fn update_role_payload_has_only_name_field() {
        let payload = build_space_role_update_payload("Editor");
        assert_eq!(payload, json!({ "name": "Editor" }));
    }

    #[test]
    fn update_payload_matches_create_payload_shape() {
        // Both helpers currently produce identical payloads. If they diverge
        // (e.g. update gains a `description` field), this test is the canary.
        assert_eq!(
            build_space_role_create_payload("X"),
            build_space_role_update_payload("X")
        );
    }

    // ---- parse_role_assignments_body ----

    #[test]
    fn parse_role_assignments_accepts_object() {
        let body = r#"{"principals": [{"id": "alice", "type": "user"}]}"#;
        let value = parse_role_assignments_body(body).unwrap();
        assert!(value.is_object(), "expected object, got {value}");
        let principals = value["principals"].as_array().expect("principals array");
        assert_eq!(principals.len(), 1);
        assert_eq!(principals[0]["id"].as_str(), Some("alice"));
    }

    #[test]
    fn parse_role_assignments_accepts_array() {
        // The Confluence API allows top-level arrays for some role-assignment
        // payloads. We don't validate shape here — we just parse JSON.
        let body = r#"[{"id": "1"}, {"id": "2"}]"#;
        let value = parse_role_assignments_body(body).unwrap();
        let arr = value.as_array().expect("expected top-level array");
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn parse_role_assignments_rejects_invalid_json() {
        let err = parse_role_assignments_body("{not-json").unwrap_err();
        assert!(
            err.to_string()
                .contains("failed to parse role assignments body as JSON"),
            "error should be wrapped with context, got: {err}"
        );
    }

    #[test]
    fn parse_role_assignments_rejects_empty_body() {
        // Empty string is not valid JSON.
        let err = parse_role_assignments_body("").unwrap_err();
        assert!(
            err.to_string()
                .contains("failed to parse role assignments body as JSON"),
            "error should be wrapped with context, got: {err}"
        );
    }
}
