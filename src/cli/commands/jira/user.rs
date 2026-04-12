use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

pub(super) async fn dispatch_user(
    cmd: &JiraUserSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraUserSubcommand::Search(args) => client.search_users(&args.query, args.limit).await?,
        JiraUserSubcommand::Get(args) => client.get_user(&args.account_id).await?,
        JiraUserSubcommand::List(args) => {
            if args.all {
                let url = format!("{}/users/search", client.base_url());
                client
                    .paginate_offset(&url, args.limit, "values", &[])
                    .await?
            } else {
                client.list_users(args.limit).await?
            }
        }
        JiraUserSubcommand::Create(args) => {
            let mut payload = json!({ "emailAddress": &args.email });
            if let Some(dn) = &args.display_name {
                payload["displayName"] = Value::String(dn.clone());
            }
            if let Some(prods) = &args.products {
                let products: Vec<Value> = prods
                    .split(',')
                    .map(|p| Value::String(p.trim().to_string()))
                    .collect();
                payload["products"] = Value::Array(products);
            }
            client.create_user(&payload).await?
        }
        JiraUserSubcommand::Delete(args) => {
            client.delete_user(&args.account_id).await?;
            Value::String(format!("User {} deleted", args.account_id))
        }
        JiraUserSubcommand::Assignable(args) => {
            client
                .get_assignable_users(&args.issue_key, args.limit)
                .await?
        }
    })
}

pub(super) async fn dispatch_group(
    cmd: &JiraGroupSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraGroupSubcommand::List => client.list_groups().await?,
        JiraGroupSubcommand::Get(args) => client.get_group(&args.name).await?,
        JiraGroupSubcommand::Create(args) => client.create_group(&args.name).await?,
        JiraGroupSubcommand::Delete(args) => {
            client.delete_group(&args.name).await?;
            Value::String(format!("Group '{}' deleted", args.name))
        }
        JiraGroupSubcommand::Members(args) => {
            client.get_group_members(&args.name, args.limit).await?
        }
        JiraGroupSubcommand::AddUser(args) => {
            client
                .add_user_to_group(&args.name, &args.account_id)
                .await?
        }
        JiraGroupSubcommand::RemoveUser(args) => {
            client
                .remove_user_from_group(&args.name, &args.account_id)
                .await?;
            Value::String(format!(
                "User {} removed from group '{}'",
                args.account_id, args.name
            ))
        }
        JiraGroupSubcommand::Search(args) => client.search_groups(&args.query, args.limit).await?,
    })
}
