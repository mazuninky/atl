use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

use super::property::dispatch_resource_property;

/// Build the nested `body.storage` sub-object for custom-content payloads
/// from a storage XHTML string. The custom-content v2 endpoint uses a
/// different envelope shape than `pages`/`blogposts`
/// (`body.storage.{value,representation}` instead of
/// `body.{representation,value}`), so this helper is local to the file.
fn storage_body(value: &str) -> Value {
    serde_json::json!({
        "storage": { "value": value, "representation": "storage" }
    })
}

/// Build the create-payload for a content-type (whiteboard / database /
/// folder / etc.). `space_id` is required; `title`, `parent_id`, and
/// `template_key` are appended only when present.
pub(super) fn build_content_type_create_payload(args: &ConfluenceContentTypeCreateArgs) -> Value {
    let mut payload = serde_json::json!({ "spaceId": args.space_id });
    if let Some(t) = &args.title {
        payload["title"] = Value::String(t.clone());
    }
    if let Some(p) = &args.parent_id {
        payload["parentId"] = Value::String(p.clone());
    }
    if let Some(tk) = &args.template_key {
        payload["templateKey"] = Value::String(tk.clone());
    }
    payload
}

/// Build the create-payload for v2 custom content. `body` is the
/// already-resolved storage-format string (i.e. `read_body_arg` was already
/// called on the user input).
pub(super) fn build_custom_content_create_payload(
    args: &ConfluenceCustomContentCreateArgs,
    body: &str,
) -> Value {
    serde_json::json!({
        "type": args.content_type,
        "spaceId": args.space_id,
        "title": args.title,
        "body": storage_body(body)
    })
}

/// Build the update-payload for v2 custom content. Always includes the
/// `version.number` field (clap requires it). `title` and `body` are appended
/// only when present. `body` is the already-resolved storage-format string.
pub(super) fn build_custom_content_update_payload(
    args: &ConfluenceCustomContentUpdateArgs,
    body: Option<&str>,
) -> Value {
    let mut payload = serde_json::json!({ "version": { "number": args.version } });
    if let Some(t) = &args.title {
        payload["title"] = Value::String(t.clone());
    }
    if let Some(b) = body {
        payload["body"] = storage_body(b);
    }
    payload
}

pub(super) async fn dispatch_content_type(
    type_name: &str,
    cmd: &ConfluenceContentTypeSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceContentTypeSubcommand::Create(args) => {
            let payload = build_content_type_create_payload(args);
            client.create_content_type_v2(type_name, &payload).await?
        }
        ConfluenceContentTypeSubcommand::Get(args) => {
            client.get_content_type_v2(type_name, &args.id).await?
        }
        ConfluenceContentTypeSubcommand::Delete(args) => {
            client.delete_content_type_v2(type_name, &args.id).await?;
            Value::String(format!("{type_name} {} deleted", args.id))
        }
        ConfluenceContentTypeSubcommand::Ancestors(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "ancestors", 200)
                .await?
        }
        ConfluenceContentTypeSubcommand::Descendants(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "descendants", args.limit)
                .await?
        }
        ConfluenceContentTypeSubcommand::Children(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "children", args.limit)
                .await?
        }
        ConfluenceContentTypeSubcommand::Operations(args) => {
            client
                .get_content_type_sub_v2(type_name, &args.id, "operations", 200)
                .await?
        }
        ConfluenceContentTypeSubcommand::Property(cmd) => match &cmd.command {
            ConfluenceContentTypePropertySubcommand::List(args) => {
                client
                    .get_content_type_sub_v2(type_name, &args.id, "properties", 200)
                    .await?
            }
            ConfluenceContentTypePropertySubcommand::Get(args) => {
                client
                    .get_content_type_property_v2(type_name, &args.id, &args.key)
                    .await?
            }
            ConfluenceContentTypePropertySubcommand::Set(args) => {
                let value_str = read_body_arg(&args.value)?;
                let value: Value =
                    serde_json::from_str(&value_str).unwrap_or(Value::String(value_str));
                client
                    .set_content_type_property_v2(type_name, &args.id, &args.key, &value)
                    .await?
            }
            ConfluenceContentTypePropertySubcommand::Delete(args) => {
                client
                    .delete_content_type_property_v2(type_name, &args.id, &args.key)
                    .await?;
                Value::String(format!("Property '{}' deleted", args.key))
            }
        },
    })
}

pub(super) async fn dispatch_custom_content(
    cmd: &ConfluenceCustomContentSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceCustomContentSubcommand::List(args) => {
            client
                .list_custom_content_v2(
                    args.content_type.as_deref(),
                    args.space_id.as_deref(),
                    args.limit,
                )
                .await?
        }
        ConfluenceCustomContentSubcommand::Get(args) => {
            client.get_custom_content_v2(&args.id).await?
        }
        ConfluenceCustomContentSubcommand::Create(args) => {
            let body = read_body_arg(&args.body)?;
            let payload = build_custom_content_create_payload(args, &body);
            client.create_custom_content_v2(&payload).await?
        }
        ConfluenceCustomContentSubcommand::Update(args) => {
            let body = match &args.body {
                Some(b) => Some(read_body_arg(b)?),
                None => None,
            };
            let payload = build_custom_content_update_payload(args, body.as_deref());
            client.update_custom_content_v2(&args.id, &payload).await?
        }
        ConfluenceCustomContentSubcommand::Delete(args) => {
            client.delete_custom_content_v2(&args.id).await?;
            Value::String(format!("Custom content {} deleted", args.id))
        }
        ConfluenceCustomContentSubcommand::Attachments(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "attachments", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::Children(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "children", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::Labels(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "labels", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::Comments(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "comments", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::Operations(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "operations", 200)
                .await?
        }
        ConfluenceCustomContentSubcommand::Versions(args) => {
            client
                .get_content_type_sub_v2("custom-content", &args.id, "versions", args.limit)
                .await?
        }
        ConfluenceCustomContentSubcommand::VersionDetails(args) => {
            client
                .get_custom_content_version_v2(&args.id, args.version)
                .await?
        }
        ConfluenceCustomContentSubcommand::Property(cmd) => {
            dispatch_resource_property("custom-content", &cmd.command, client).await?
        }
    })
}

#[cfg(test)]
mod tests {
    // Most arms in `dispatch_content_type` and `dispatch_custom_content` are
    // pure HTTP delegation, covered by contract tests in
    // tests/contract_confluence_v*.rs. The pure payload-builders below are
    // unit-tested here.

    use super::*;
    use serde_json::json;

    fn ct_create_args(
        space_id: &str,
        title: Option<&str>,
        parent_id: Option<&str>,
        template_key: Option<&str>,
    ) -> ConfluenceContentTypeCreateArgs {
        ConfluenceContentTypeCreateArgs {
            space_id: space_id.into(),
            title: title.map(String::from),
            template_key: template_key.map(String::from),
            parent_id: parent_id.map(String::from),
        }
    }

    fn cc_create_args(
        content_type: &str,
        space_id: &str,
        title: &str,
        body: &str,
    ) -> ConfluenceCustomContentCreateArgs {
        ConfluenceCustomContentCreateArgs {
            content_type: content_type.into(),
            space_id: space_id.into(),
            title: title.into(),
            body: body.into(),
        }
    }

    fn cc_update_args(
        id: &str,
        title: Option<&str>,
        body: Option<&str>,
        version: u32,
    ) -> ConfluenceCustomContentUpdateArgs {
        ConfluenceCustomContentUpdateArgs {
            id: id.into(),
            title: title.map(String::from),
            body: body.map(String::from),
            version,
        }
    }

    // ---- build_content_type_create_payload ----

    #[test]
    fn ct_create_required_only_yields_just_space_id() {
        let payload = build_content_type_create_payload(&ct_create_args("100", None, None, None));
        assert_eq!(payload, json!({ "spaceId": "100" }));
    }

    #[test]
    fn ct_create_with_title_only() {
        let payload = build_content_type_create_payload(&ct_create_args(
            "100",
            Some("My Whiteboard"),
            None,
            None,
        ));
        assert_eq!(
            payload,
            json!({ "spaceId": "100", "title": "My Whiteboard" })
        );
    }

    #[test]
    fn ct_create_with_parent_only() {
        let payload =
            build_content_type_create_payload(&ct_create_args("100", None, Some("999"), None));
        assert_eq!(payload, json!({ "spaceId": "100", "parentId": "999" }));
    }

    #[test]
    fn ct_create_with_template_key_only() {
        let payload = build_content_type_create_payload(&ct_create_args(
            "100",
            None,
            None,
            Some("brainstorm"),
        ));
        assert_eq!(
            payload,
            json!({ "spaceId": "100", "templateKey": "brainstorm" })
        );
    }

    #[test]
    fn ct_create_with_all_fields_set() {
        let payload = build_content_type_create_payload(&ct_create_args(
            "100",
            Some("T"),
            Some("999"),
            Some("brainstorm"),
        ));
        assert_eq!(
            payload,
            json!({
                "spaceId": "100",
                "title": "T",
                "parentId": "999",
                "templateKey": "brainstorm",
            })
        );
    }

    #[test]
    fn ct_create_omits_none_fields_entirely() {
        // Verify that `null` is never serialized — fields are simply absent.
        // This matters because some Atlassian endpoints distinguish "field not
        // present" from "field set to null".
        let payload =
            build_content_type_create_payload(&ct_create_args("100", Some("T"), None, None));
        let obj = payload.as_object().expect("payload is an object");
        assert!(
            !obj.contains_key("parentId"),
            "parentId must not be present"
        );
        assert!(
            !obj.contains_key("templateKey"),
            "templateKey must not be present"
        );
    }

    // ---- build_custom_content_create_payload ----

    #[test]
    fn cc_create_includes_all_required_fields() {
        let args = cc_create_args("acme:custom", "100", "My Title", "ignored");
        let payload = build_custom_content_create_payload(&args, "<p>hi</p>");
        assert_eq!(
            payload,
            json!({
                "type": "acme:custom",
                "spaceId": "100",
                "title": "My Title",
                "body": {
                    "storage": { "value": "<p>hi</p>", "representation": "storage" }
                }
            })
        );
    }

    #[test]
    fn cc_create_uses_resolved_body_not_raw_args_body() {
        // The dispatcher resolves `args.body` via `read_body_arg` (which can
        // strip a `@file`/`-` prefix) and passes the result here. The helper
        // must use that resolved string, not the raw value on `args`.
        let args = cc_create_args("acme:custom", "100", "T", "@should-be-ignored");
        let payload = build_custom_content_create_payload(&args, "RESOLVED");
        assert_eq!(
            payload["body"]["storage"]["value"].as_str(),
            Some("RESOLVED"),
            "body should come from the resolved string, not args.body"
        );
    }

    // ---- build_custom_content_update_payload ----

    #[test]
    fn cc_update_only_version_when_no_optional_fields() {
        let args = cc_update_args("123", None, None, 7);
        let payload = build_custom_content_update_payload(&args, None);
        assert_eq!(payload, json!({ "version": { "number": 7 } }));
    }

    #[test]
    fn cc_update_with_title_only() {
        let args = cc_update_args("123", Some("New Title"), None, 7);
        let payload = build_custom_content_update_payload(&args, None);
        assert_eq!(
            payload,
            json!({ "version": { "number": 7 }, "title": "New Title" })
        );
    }

    #[test]
    fn cc_update_with_body_only() {
        let args = cc_update_args("123", None, Some("set"), 7);
        // Body resolution lives in the dispatcher; we just receive the
        // resolved string.
        let payload = build_custom_content_update_payload(&args, Some("RESOLVED"));
        assert_eq!(
            payload,
            json!({
                "version": { "number": 7 },
                "body": { "storage": { "value": "RESOLVED", "representation": "storage" } }
            })
        );
    }

    #[test]
    fn cc_update_with_title_and_body() {
        let args = cc_update_args("123", Some("T"), Some("set"), 7);
        let payload = build_custom_content_update_payload(&args, Some("RESOLVED"));
        assert_eq!(
            payload,
            json!({
                "version": { "number": 7 },
                "title": "T",
                "body": { "storage": { "value": "RESOLVED", "representation": "storage" } }
            })
        );
    }

    #[test]
    fn cc_update_body_arg_is_used_only_when_resolved_body_is_some() {
        // Even if `args.body` is Some(...), we obey the explicit `body` arg.
        // The dispatcher contract is: "if you didn't resolve it, pass None".
        let args = cc_update_args("123", None, Some("@file"), 7);
        let payload = build_custom_content_update_payload(&args, None);
        let obj = payload.as_object().unwrap();
        assert!(
            !obj.contains_key("body"),
            "body must be absent when the resolved-body arg is None"
        );
    }
}
