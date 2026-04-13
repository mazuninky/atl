mod admin;
mod attachment;
mod blog;
mod comment;
mod content;
mod page;
mod property;
mod space;

use std::io::Write;

use camino::Utf8Path;
use serde_json::Value;

use crate::auth::SystemKeyring;
use crate::cli::args::*;
use crate::client::ConfluenceClient;
use crate::config::ConfigLoader;
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

use super::read_body_arg;
use page::{copy_tree, export_page, maybe_convert_markdown, render_tree};

pub async fn run(
    cmd: &ConfluenceSubcommand,
    config_path: Option<&Utf8Path>,
    profile_name: Option<&str>,
    retries: u32,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    // `--web` short-circuits before we build a typed client so the browse
    // helper can decide whether it needs to hit the API (Confluence) or can
    // construct the URL locally (Jira). Handled here rather than inside the
    // dispatch so the error message for missing config is consistent.
    if let ConfluenceSubcommand::Read(args) = cmd
        && args.web
    {
        let browse_args = crate::cli::args::BrowseArgs {
            target: args.page_id.clone(),
            service: crate::cli::args::BrowseService::Confluence,
        };
        return super::browse::run(&browse_args, config_path, profile_name, retries, io).await;
    }

    let config = ConfigLoader::load(config_path)?;
    let resolved_profile_name = profile_name
        .or(config.as_ref().map(|c| c.default_profile.as_str()))
        .unwrap_or("default");
    let profile = config
        .as_ref()
        .and_then(|c| c.resolve_profile(Some(resolved_profile_name)))
        .ok_or_else(|| anyhow::anyhow!("no profile found; run `atl init` first"))?;
    let instance = profile
        .confluence
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no Confluence instance configured in profile"))?;
    let store = SystemKeyring;
    let client =
        ConfluenceClient::connect(instance, resolved_profile_name, &store, retries).await?;

    dispatch(cmd, &client, format, io, transforms).await
}

/// Returns true when the long-form output of `cmd` would benefit from a
/// pager. Only the read-heavy "view" commands qualify; mutating or
/// short-output commands are excluded so user-facing prompts and progress
/// remain inline.
fn cmd_uses_pager(cmd: &ConfluenceSubcommand) -> bool {
    matches!(
        cmd,
        ConfluenceSubcommand::Read(_) | ConfluenceSubcommand::Search(_)
    )
}

/// Flattens a Confluence search response for human-readable console display.
///
/// Extracts the `results` array and maps each item to a flat object with
/// only `id`, `title`, `type`, `status`, and `url` fields, dropping noisy
/// metadata like `childTypes`, `macroRenderedOutput`, `restrictions`,
/// `_expandable`, and `_links`.
fn flatten_confluence_search(value: Value) -> Value {
    let results = match value.get("results").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return value,
    };
    let flat: Vec<Value> = results
        .iter()
        .map(|item| {
            let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let r#type = item.get("type").and_then(Value::as_str).unwrap_or_default();
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let url = item
                .get("_links")
                .and_then(|l| l.get("webui"))
                .and_then(Value::as_str)
                .unwrap_or_default();

            let mut map = serde_json::Map::new();
            map.insert("id".into(), Value::String(id.into()));
            map.insert("title".into(), Value::String(title.into()));
            map.insert("type".into(), Value::String(r#type.into()));
            map.insert("status".into(), Value::String(status.into()));
            map.insert("url".into(), Value::String(url.into()));
            Value::Object(map)
        })
        .collect();
    Value::Array(flat)
}

/// Flattens a single Confluence page for human-readable console display.
///
/// Extracts key fields from the nested API response and produces a flat
/// key-value object that the console reporter renders as a readable list
/// instead of a giant JSON blob.
fn flatten_confluence_page(value: Value) -> Value {
    let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let space_id = value
        .get("spaceId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let created = value
        .get("createdAt")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version_num = value
        .get("version")
        .and_then(|v| v.get("number"))
        .and_then(Value::as_u64)
        .map(|n| n.to_string())
        .unwrap_or_default();
    let updated = value
        .get("version")
        .and_then(|v| v.get("createdAt"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    // Build full URL from _links
    let base = value
        .get("_links")
        .and_then(|l| l.get("base"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let webui = value
        .get("_links")
        .and_then(|l| l.get("webui"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = if !base.is_empty() && !webui.is_empty() {
        format!("{base}{webui}")
    } else {
        webui.to_string()
    };

    // Extract body content from whatever representation is available
    let body = value
        .get("body")
        .and_then(Value::as_object)
        .and_then(|obj| obj.values().next())
        .and_then(|repr| repr.get("value"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    let mut map = serde_json::Map::new();
    map.insert("id".into(), Value::String(id.into()));
    map.insert("title".into(), Value::String(title.into()));
    map.insert("status".into(), Value::String(status.into()));
    map.insert("spaceId".into(), Value::String(space_id.into()));
    map.insert("version".into(), Value::String(version_num));
    map.insert("created".into(), Value::String(created.into()));
    map.insert("updated".into(), Value::String(updated.into()));
    if !url.is_empty() {
        map.insert("url".into(), Value::String(url));
    }
    if !body.is_empty() {
        map.insert("body".into(), Value::String(body.into()));
    }
    Value::Object(map)
}

async fn dispatch(
    cmd: &ConfluenceSubcommand,
    client: &ConfluenceClient,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let value = match cmd {
        ConfluenceSubcommand::Read(args) => {
            let mut expand = Vec::new();
            if args.include_labels {
                expand.push("metadata.labels");
            }
            if args.include_properties {
                expand.push("metadata.properties");
            }
            if args.include_operations {
                expand.push("operations");
            }
            if args.include_versions {
                expand.push("version");
            }
            if args.include_collaborators {
                expand.push("collaborators");
            }
            if args.include_favorited_by {
                expand.push("metadata.currentuser.favourited");
            }
            let value = client
                .get_page(&args.page_id, args.body_format.as_str(), &expand)
                .await?;
            if matches!(format, OutputFormat::Console) {
                flatten_confluence_page(value)
            } else {
                value
            }
        }
        ConfluenceSubcommand::Info(args) => client.get_page_info(&args.page_id).await?,
        ConfluenceSubcommand::Search(args) => {
            let value = if args.all {
                client.search_all(&args.cql, args.limit).await?
            } else {
                client.search(&args.cql, args.limit).await?
            };
            if matches!(format, OutputFormat::Console) {
                flatten_confluence_search(value)
            } else {
                value
            }
        }
        ConfluenceSubcommand::Space(cmd) => space::dispatch_space(&cmd.command, client).await?,
        ConfluenceSubcommand::Children(args) => {
            if args.depth > 1 || args.tree {
                let tree_value = client
                    .get_children_recursive(&args.page_id, args.depth, args.limit)
                    .await?;
                if args.tree && matches!(format, OutputFormat::Console) {
                    let mut stdout = io.stdout();
                    render_tree(&tree_value, 0, true, &mut stdout)?;
                    stdout.flush()?;
                    return Ok(());
                }
                tree_value
            } else {
                client.get_children(&args.page_id, args.limit).await?
            }
        }
        ConfluenceSubcommand::Create(args) => {
            let body = maybe_convert_markdown(read_body_arg(&args.body)?, &args.input_format);
            let space =
                args.space.as_deref().or(args.space_id.as_deref()).expect(
                    "clap enforces required_unless_present=space_id on ConfluenceCreateArgs",
                );
            client
                .create_page(
                    space,
                    &args.title,
                    &body,
                    args.parent.as_deref(),
                    args.private,
                )
                .await?
        }
        ConfluenceSubcommand::Update(args) => {
            let body = maybe_convert_markdown(read_body_arg(&args.body)?, &args.input_format);
            client
                .update_page(
                    &args.page_id,
                    &args.title,
                    &body,
                    args.version,
                    args.version_message.as_deref(),
                )
                .await?
        }
        ConfluenceSubcommand::Delete(args) => {
            client
                .delete_page(&args.page_id, args.purge, args.draft)
                .await?;
            Value::String(format!("Page {} deleted", args.page_id))
        }
        ConfluenceSubcommand::Attachment(cmd) => {
            attachment::dispatch_attachment(&cmd.command, client).await?
        }
        // Legacy hidden aliases (v1)
        ConfluenceSubcommand::Comments(args) => {
            client.get_comments(&args.page_id, args.limit).await?
        }
        ConfluenceSubcommand::Find(args) => {
            let escaped_title = args.title.replace('\\', "\\\\").replace('"', "\\\"");
            let mut cql = format!("title=\"{escaped_title}\" AND type=page");
            if let Some(space) = &args.space {
                let escaped_space = space.replace('\\', "\\\\").replace('"', "\\\"");
                cql.push_str(&format!(" AND space=\"{escaped_space}\""));
            }
            let value = client.search(&cql, args.limit).await?;
            if matches!(format, OutputFormat::Console) {
                flatten_confluence_search(value)
            } else {
                value
            }
        }
        ConfluenceSubcommand::CreateComment(args) => {
            let body = read_body_arg(&args.body)?;
            client
                .create_comment(&args.page_id, &body, args.parent.as_deref())
                .await?
        }
        ConfluenceSubcommand::DeleteComment(args) => {
            client.delete_comment(&args.comment_id).await?;
            Value::String(format!("Comment {} deleted", args.comment_id))
        }
        ConfluenceSubcommand::DeleteAttachment(args) => {
            client.delete_attachment(&args.attachment_id).await?;
            Value::String(format!("Attachment {} deleted", args.attachment_id))
        }
        ConfluenceSubcommand::UploadAttachment(args) => {
            client
                .upload_attachment(&args.page_id, args.file.as_path())
                .await?
        }
        ConfluenceSubcommand::Export(args) => export_page(client, args).await?,
        ConfluenceSubcommand::CopyTree(args) => copy_tree(client, args).await?,
        ConfluenceSubcommand::Property(cmd) => match &cmd.command {
            ConfluencePropertySubcommand::List(args) => {
                client.get_properties(&args.page_id).await?
            }
            ConfluencePropertySubcommand::Get(args) => {
                client.get_property(&args.page_id, &args.key).await?
            }
            ConfluencePropertySubcommand::Set(args) => {
                let value_str = read_body_arg(&args.value)?;
                let value: Value =
                    serde_json::from_str(&value_str).unwrap_or(Value::String(value_str));
                client
                    .set_property(&args.page_id, &args.key, &value)
                    .await?
            }
            ConfluencePropertySubcommand::Delete(args) => {
                client.delete_property(&args.page_id, &args.key).await?;
                Value::String(format!(
                    "Property '{}' deleted from page {}",
                    args.key, args.page_id
                ))
            }
        },
        ConfluenceSubcommand::Blog(cmd) => blog::dispatch_blog(&cmd.command, client).await?,
        ConfluenceSubcommand::Label(cmd) => match &cmd.command {
            ConfluenceLabelSubcommand::List(args) => {
                client
                    .get_labels(&args.page_id, args.prefix.as_deref())
                    .await?
            }
            ConfluenceLabelSubcommand::Add(args) => {
                client.add_labels(&args.page_id, &args.labels).await?
            }
            ConfluenceLabelSubcommand::Remove(args) => {
                client.remove_label(&args.page_id, &args.label).await?;
                Value::String(format!(
                    "Label '{}' removed from page {}",
                    args.label, args.page_id
                ))
            }
            ConfluenceLabelSubcommand::Pages(args) => {
                client
                    .get_label_pages_v2(&args.label_id, args.limit)
                    .await?
            }
            ConfluenceLabelSubcommand::Blogposts(args) => {
                client
                    .get_label_blogposts_v2(&args.label_id, args.limit)
                    .await?
            }
            ConfluenceLabelSubcommand::Attachments(args) => {
                client
                    .get_label_attachments_v2(&args.label_id, args.limit)
                    .await?
            }
        },

        // -- Page extras (v2) --
        ConfluenceSubcommand::Versions(args) => {
            client
                .get_page_versions_v2(&args.page_id, args.limit)
                .await?
        }
        ConfluenceSubcommand::VersionDetail(args) => {
            client
                .get_page_version_v2(&args.page_id, args.version)
                .await?
        }
        ConfluenceSubcommand::Likes(args) => client.get_page_likes_v2(&args.page_id).await?,
        ConfluenceSubcommand::Operations(args) => {
            client.get_page_operations_v2(&args.page_id).await?
        }
        ConfluenceSubcommand::Ancestors(args) => {
            client.get_page_ancestors_v2(&args.page_id).await?
        }
        ConfluenceSubcommand::Descendants(args) => {
            client
                .get_page_descendants_v2(&args.page_id, args.limit)
                .await?
        }

        // -- v2 comment resources --
        ConfluenceSubcommand::FooterComment(cmd) => {
            comment::dispatch_footer_comment(&cmd.command, client).await?
        }
        ConfluenceSubcommand::InlineComment(cmd) => {
            comment::dispatch_inline_comment(&cmd.command, client).await?
        }

        // -- New content types (v2) --
        ConfluenceSubcommand::Whiteboard(cmd) => {
            content::dispatch_content_type("whiteboards", &cmd.command, client).await?
        }
        ConfluenceSubcommand::Database(cmd) => {
            content::dispatch_content_type("databases", &cmd.command, client).await?
        }
        ConfluenceSubcommand::Folder(cmd) => {
            content::dispatch_content_type("folders", &cmd.command, client).await?
        }

        // -- Custom content (v2) --
        ConfluenceSubcommand::CustomContent(cmd) => {
            content::dispatch_custom_content(&cmd.command, client).await?
        }

        // -- Tasks (v2) --
        ConfluenceSubcommand::Task(cmd) => admin::dispatch_task(&cmd.command, client).await?,

        // -- Admin (v2) --
        ConfluenceSubcommand::AdminKey(cmd) => match &cmd.command {
            ConfluenceAdminKeySubcommand::Get => client.get_admin_key_v2().await?,
            ConfluenceAdminKeySubcommand::Enable => client.enable_admin_key_v2().await?,
            ConfluenceAdminKeySubcommand::Disable => client.disable_admin_key_v2().await?,
        },
        ConfluenceSubcommand::Classification(cmd) => {
            admin::dispatch_classification(&cmd.command, client).await?
        }

        // -- Users & misc (v2) --
        ConfluenceSubcommand::User(cmd) => match &cmd.command {
            ConfluenceUserSubcommand::Bulk(args) => {
                client.bulk_lookup_users_v2(&args.account_ids).await?
            }
            ConfluenceUserSubcommand::CheckAccess(args) => {
                client.check_user_access_by_email_v2(&args.email).await?
            }
            ConfluenceUserSubcommand::Invite(args) => client.invite_users_v2(&args.emails).await?,
        },
        ConfluenceSubcommand::ConvertIds(args) => client.convert_content_ids_v2(&args.ids).await?,
        ConfluenceSubcommand::AppProperty(cmd) => match &cmd.command {
            ConfluenceAppPropertySubcommand::List => client.list_app_properties_v2().await?,
            ConfluenceAppPropertySubcommand::Get(args) => {
                client.get_app_property_v2(&args.key).await?
            }
            ConfluenceAppPropertySubcommand::Set(args) => {
                let value_str = read_body_arg(&args.value)?;
                let value: Value =
                    serde_json::from_str(&value_str).unwrap_or(Value::String(value_str));
                client.set_app_property_v2(&args.key, &value).await?
            }
            ConfluenceAppPropertySubcommand::Delete(args) => {
                client.delete_app_property_v2(&args.key).await?;
                Value::String(format!("App property '{}' deleted", args.key))
            }
        },

        // -- Page extras (v2) --
        ConfluenceSubcommand::PageList(args) => {
            client
                .list_pages_v2(
                    args.space_id.as_deref(),
                    args.title.as_deref(),
                    args.status.as_deref(),
                    args.sort.as_deref(),
                    args.limit,
                )
                .await?
        }
        ConfluenceSubcommand::UpdateTitle(args) => {
            client
                .update_page_title_v2(&args.page_id, &args.title, args.version)
                .await?
        }
        ConfluenceSubcommand::LikesCount(args) => {
            client.get_page_likes_count_v2(&args.page_id).await?
        }
        ConfluenceSubcommand::LikesUsers(args) => {
            client.get_page_likes_users_v2(&args.page_id).await?
        }
        ConfluenceSubcommand::PageCustomContent(args) => {
            client
                .get_page_custom_content_v2(&args.page_id, &args.content_type, args.limit)
                .await?
        }
        ConfluenceSubcommand::Redact(args) => client.redact_page_v2(&args.page_id).await?,
    };

    // Start the pager before writing the (potentially long) response so the
    // user can scroll. The pager only engages on Console output to a TTY when
    // the command was a long-form view; everything else stays inline.
    let use_pager = matches!(format, OutputFormat::Console)
        && io.is_stdout_tty()
        && !io.pager_disabled()
        && cmd_uses_pager(cmd);
    if use_pager {
        io.start_pager()?;
    }

    let write_res = write_output(value, format, io, transforms);
    let stop_res = if use_pager { io.stop_pager() } else { Ok(()) };
    write_res?;
    stop_res?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- flatten_confluence_search ----

    #[test]
    fn flatten_search_extracts_fields() {
        let input = json!({
            "results": [
                {
                    "id": "98486",
                    "type": "page",
                    "status": "current",
                    "title": "Template - Meeting notes",
                    "childTypes": {},
                    "macroRenderedOutput": {},
                    "restrictions": {},
                    "_expandable": { "container": "/rest/api/space/TEAM" },
                    "_links": {
                        "webui": "/spaces/TEAM/pages/98486/Template+-+Meeting+notes",
                        "self": "https://example.atlassian.net/wiki/rest/api/content/98486",
                        "tinyui": "/x/toAB"
                    }
                }
            ],
            "start": 0,
            "limit": 3,
            "size": 1,
            "_links": {}
        });
        let result = flatten_confluence_search(input);
        let items = result.as_array().expect("should be an array");
        assert_eq!(items.len(), 1);

        let item = &items[0];
        assert_eq!(item.get("id").and_then(Value::as_str), Some("98486"));
        assert_eq!(
            item.get("title").and_then(Value::as_str),
            Some("Template - Meeting notes")
        );
        assert_eq!(item.get("type").and_then(Value::as_str), Some("page"));
        assert_eq!(item.get("status").and_then(Value::as_str), Some("current"));
        assert_eq!(
            item.get("url").and_then(Value::as_str),
            Some("/spaces/TEAM/pages/98486/Template+-+Meeting+notes")
        );

        // Metadata fields should be absent.
        assert!(item.get("childTypes").is_none());
        assert!(item.get("macroRenderedOutput").is_none());
        assert!(item.get("restrictions").is_none());
        assert!(item.get("_expandable").is_none());
        assert!(item.get("_links").is_none());
    }

    #[test]
    fn flatten_search_no_results_key_passthrough() {
        let input = json!({"total": 0});
        let result = flatten_confluence_search(input.clone());
        assert_eq!(
            result, input,
            "input without results key should pass through unchanged"
        );
    }

    #[test]
    fn flatten_search_preserves_column_order() {
        let input = json!({
            "results": [{
                "id": "1",
                "type": "page",
                "status": "current",
                "title": "T",
                "_links": { "webui": "/x" }
            }],
            "size": 1
        });
        let result = flatten_confluence_search(input);
        let item = result.as_array().unwrap()[0].as_object().unwrap();
        let keys: Vec<&String> = item.keys().collect();
        assert_eq!(
            keys,
            vec!["id", "title", "type", "status", "url"],
            "columns should appear in insertion order"
        );
    }

    // ---- flatten_confluence_page ----

    #[test]
    fn flatten_page_extracts_fields() {
        let input = json!({
            "id": "98420",
            "title": "Page title",
            "status": "current",
            "spaceId": "98309",
            "parentType": null,
            "parentId": null,
            "createdAt": "2025-10-29T12:00:00Z",
            "version": { "number": 4, "createdAt": "2025-11-01T08:30:00Z" },
            "body": {
                "storage": {
                    "value": "<p>Hello world</p>",
                    "representation": "storage"
                }
            },
            "_links": {
                "webui": "/spaces/inno/overview",
                "base": "https://example.atlassian.net/wiki"
            },
            "ownerId": "abc123",
            "authorId": "def456",
            "lastOwnerId": null,
            "position": 195
        });
        let result = flatten_confluence_page(input);
        let obj = result.as_object().expect("should be an object");

        assert_eq!(obj.get("id").and_then(Value::as_str), Some("98420"));
        assert_eq!(obj.get("title").and_then(Value::as_str), Some("Page title"));
        assert_eq!(obj.get("status").and_then(Value::as_str), Some("current"));
        assert_eq!(obj.get("spaceId").and_then(Value::as_str), Some("98309"));
        assert_eq!(obj.get("version").and_then(Value::as_str), Some("4"));
        assert_eq!(
            obj.get("created").and_then(Value::as_str),
            Some("2025-10-29T12:00:00Z")
        );
        assert_eq!(
            obj.get("updated").and_then(Value::as_str),
            Some("2025-11-01T08:30:00Z")
        );
        assert_eq!(
            obj.get("url").and_then(Value::as_str),
            Some("https://example.atlassian.net/wiki/spaces/inno/overview")
        );
        assert_eq!(
            obj.get("body").and_then(Value::as_str),
            Some("<p>Hello world</p>")
        );

        // Dropped fields should be absent.
        assert!(obj.get("parentType").is_none());
        assert!(obj.get("parentId").is_none());
        assert!(obj.get("ownerId").is_none());
        assert!(obj.get("authorId").is_none());
        assert!(obj.get("lastOwnerId").is_none());
        assert!(obj.get("position").is_none());
        assert!(obj.get("_links").is_none());
    }

    #[test]
    fn flatten_page_url_without_base() {
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "_links": { "webui": "/spaces/X/overview" }
        });
        let result = flatten_confluence_page(input);
        assert_eq!(
            result.get("url").and_then(Value::as_str),
            Some("/spaces/X/overview"),
            "url should be just webui when base is absent"
        );
    }

    #[test]
    fn flatten_page_omits_empty_body() {
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current"
        });
        let result = flatten_confluence_page(input);
        assert!(
            result.get("body").is_none(),
            "body key should be absent when no body content exists"
        );
    }

    #[test]
    fn flatten_page_omits_empty_url() {
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current"
        });
        let result = flatten_confluence_page(input);
        assert!(
            result.get("url").is_none(),
            "url key should be absent when _links is missing"
        );
    }

    #[test]
    fn flatten_page_preserves_column_order() {
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "spaceId": "S",
            "createdAt": "2025-01-01T00:00:00Z",
            "version": { "number": 1, "createdAt": "2025-01-01T00:00:00Z" },
            "_links": { "webui": "/x", "base": "https://example.com" },
            "body": { "storage": { "value": "content" } }
        });
        let result = flatten_confluence_page(input);
        let obj = result.as_object().unwrap();
        let keys: Vec<&String> = obj.keys().collect();
        assert_eq!(
            keys,
            vec![
                "id", "title", "status", "spaceId", "version", "created", "updated", "url", "body"
            ],
            "columns should appear in insertion order"
        );
    }
}
