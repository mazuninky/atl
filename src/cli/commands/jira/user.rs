use serde_json::{Value, json};

use crate::cli::args::*;
use crate::client::JiraClient;

/// Build the create payload for a user. Splits the comma-separated `--products` list into a
/// JSON array of trimmed product keys.
fn build_user_create_payload(args: &JiraUserCreateArgs) -> Value {
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
    payload
}

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
            client.create_user(&build_user_create_payload(args)).await?
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
    // All branches are pure HTTP delegation; covered by contract tests in tests/contract_jira_*.rs.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn user_create(
        email: &str,
        display_name: Option<&str>,
        products: Option<&str>,
    ) -> JiraUserCreateArgs {
        JiraUserCreateArgs {
            email: email.to_string(),
            display_name: display_name.map(str::to_string),
            products: products.map(str::to_string),
        }
    }

    #[test]
    fn user_create_minimal() {
        let payload = build_user_create_payload(&user_create("a@b.com", None, None));
        assert_eq!(payload, json!({ "emailAddress": "a@b.com" }));
    }

    #[test]
    fn user_create_with_display_name() {
        let payload = build_user_create_payload(&user_create("a@b.com", Some("Alice"), None));
        assert_eq!(
            payload,
            json!({ "emailAddress": "a@b.com", "displayName": "Alice" })
        );
    }

    #[test]
    fn user_create_with_single_product() {
        let payload =
            build_user_create_payload(&user_create("a@b.com", None, Some("jira-software")));
        assert_eq!(
            payload,
            json!({ "emailAddress": "a@b.com", "products": ["jira-software"] })
        );
    }

    #[test]
    fn user_create_with_multiple_products_trimmed() {
        let payload = build_user_create_payload(&user_create(
            "a@b.com",
            None,
            Some(" jira-software , confluence "),
        ));
        assert_eq!(
            payload,
            json!({
                "emailAddress": "a@b.com",
                "products": ["jira-software", "confluence"],
            })
        );
    }

    #[test]
    fn user_create_with_empty_products_string_yields_single_empty_entry() {
        // Document current behaviour: an empty `--products ""` produces `[""]`. We keep the
        // round-trip explicit so any future change to drop empty entries is intentional.
        let payload = build_user_create_payload(&user_create("a@b.com", None, Some("")));
        assert_eq!(
            payload,
            json!({ "emailAddress": "a@b.com", "products": [""] })
        );
    }

    #[test]
    fn user_create_with_all_fields() {
        let payload = build_user_create_payload(&user_create(
            "a@b.com",
            Some("Alice"),
            Some("jira-software"),
        ));
        assert_eq!(
            payload,
            json!({
                "emailAddress": "a@b.com",
                "displayName": "Alice",
                "products": ["jira-software"],
            })
        );
    }
}
