use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

use super::property::dispatch_resource_property;

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
            let payload: Value = serde_json::from_str(&body)?;
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
            let payload = serde_json::json!({ "name": args.name });
            client
                .create_space_role_v2(&args.space_id, &payload)
                .await?
        }
        ConfluenceSpaceRoleSubcommand::Update(args) => {
            let payload = serde_json::json!({ "name": args.name });
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
