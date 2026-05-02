//! `atl jira automation …` — manage Jira Cloud automation rules.
//!
//! All write operations honour `instance.read_only` at the client layer; the
//! retry policy is the same `--retries` / `--retry-all-methods` machinery the
//! rest of the Jira surface uses.

use anyhow::Context;
use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::JiraClient;

/// Parse a JSON body from a literal / `@file` / stdin source. Returns the
/// parsed [`Value`] so a malformed input fails before we send the request.
fn parse_json_body(raw_arg: &str, what: &str) -> anyhow::Result<Value> {
    let raw = read_body_arg(raw_arg).with_context(|| format!("failed to read {what} body"))?;
    serde_json::from_str(&raw).with_context(|| format!("invalid JSON in {what} body"))
}

pub(super) async fn dispatch_automation(
    cmd: &JiraAutomationSubcommand,
    client: &JiraClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        JiraAutomationSubcommand::List(args) => {
            client
                .list_automation_rules(args.cursor.as_deref(), args.limit)
                .await?
        }
        JiraAutomationSubcommand::Get(args) => client.get_automation_rule(&args.uuid).await?,
        JiraAutomationSubcommand::Create(args) => {
            let payload = parse_json_body(&args.body, "rule")?;
            client.create_automation_rule(&payload).await?
        }
        JiraAutomationSubcommand::Update(args) => {
            let payload = parse_json_body(&args.body, "rule")?;
            let value = client.update_automation_rule(&args.uuid, &payload).await?;
            if value.is_null() {
                Value::String(format!("Rule {} updated", args.uuid))
            } else {
                value
            }
        }
        JiraAutomationSubcommand::Enable(args) => {
            let value = client.set_automation_rule_state(&args.uuid, true).await?;
            if value.is_null() {
                Value::String(format!("Rule {} enabled", args.uuid))
            } else {
                value
            }
        }
        JiraAutomationSubcommand::Disable(args) => {
            let value = client.set_automation_rule_state(&args.uuid, false).await?;
            if value.is_null() {
                Value::String(format!("Rule {} disabled", args.uuid))
            } else {
                value
            }
        }
        JiraAutomationSubcommand::Delete(args) => {
            client.delete_automation_rule(&args.uuid).await?;
            Value::String(format!("Rule {} deleted", args.uuid))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_body_literal_round_trips() {
        let v = parse_json_body(r#"{"a": 1}"#, "rule").unwrap();
        assert_eq!(v, serde_json::json!({"a": 1}));
    }

    #[test]
    fn parse_json_body_invalid_errors() {
        let err = parse_json_body("{ not json", "rule").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("invalid JSON"), "got: {msg}");
    }
}
