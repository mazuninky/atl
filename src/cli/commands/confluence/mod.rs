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
use crate::client::{ConfluenceClient, RetryConfig};
use crate::config::{AtlassianInstance, ConfigLoader};
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

use super::confluence_url::build_confluence_url;
use super::read_body_arg;
use page::{ExtractOpts, convert_input, copy_tree, export_page, extract_body, render_tree};

pub async fn run(
    cmd: &ConfluenceSubcommand,
    config_path: Option<&Utf8Path>,
    profile_name: Option<&str>,
    retry_cfg: RetryConfig,
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
        return super::browse::run(&browse_args, config_path, profile_name, retry_cfg, io).await;
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
        ConfluenceClient::connect(instance, resolved_profile_name, &store, retry_cfg).await?;

    dispatch(cmd, &client, instance, format, io, transforms).await
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

/// Escape a value for safe interpolation into a CQL quoted string.
///
/// Mirrors the JQL-side escape used by the Jira dispatcher: backslashes first,
/// then double quotes — order matters or the doubled backslashes from the
/// quote replacement would themselves get re-escaped.
fn escape_cql(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Build the CQL string used by the legacy `confluence find` command.
fn build_find_cql(title: &str, space: Option<&str>) -> String {
    let mut cql = format!("title=\"{}\" AND type=page", escape_cql(title));
    if let Some(sp) = space {
        cql.push_str(&format!(" AND space=\"{}\"", escape_cql(sp)));
    }
    cql
}

/// Compute the list of `expand` parameters from the user's `--include-*` flags
/// for `confluence read`. Returns `'static` slice references so the caller can
/// own the resulting `Vec<&str>` without lifetime headaches.
fn compute_read_expand(args: &ConfluenceReadArgs) -> Vec<&'static str> {
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
    expand
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
///
/// `body` is the pre-rendered body string in the user's requested
/// `--body-format`. The caller is responsible for running the page
/// through `extract_body` so that the markdown / ADF conversion happens
/// before the value reaches the console reporter — this helper does not
/// re-extract from the raw `body.<repr>.value` field.
///
/// The emitted `url` field is anchored to the locally configured
/// `instance.domain` — never to the host of `_links.base`. The path
/// component of `_links.base` is used as a context prefix only after its
/// host is validated to match the configured domain (so the canonical
/// Confluence Cloud `/wiki` prefix is preserved). See
/// [`build_confluence_url`] for the rationale. A compromised or
/// MITM-proxied Confluence instance would otherwise be able to inject
/// an attacker-controlled origin into output that downstream tools or
/// copy-paste would treat as trusted.
fn flatten_confluence_page(value: Value, instance: &AtlassianInstance, body: &str) -> Value {
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

    // Build the full URL from the locally configured domain and the
    // server-supplied `_links.webui`. The host portion of `_links.base`
    // is never trusted; only its path component is used as a context
    // prefix (e.g. `/wiki` on Confluence Cloud) after its host is
    // validated to match the configured domain. See the function-level
    // doc comment on `build_confluence_url`.
    let links = value.get("_links");
    let webui = links
        .and_then(|l| l.get("webui"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let base = links.and_then(|l| l.get("base")).and_then(Value::as_str);
    let url = if webui.is_empty() {
        String::new()
    } else {
        match build_confluence_url(&instance.domain, base, webui) {
            Ok(u) => u,
            Err(e) => {
                // Output formatting must never fail the command — fall back
                // to an empty url field. A debug-level log lets developers
                // notice when a server-supplied path was rejected.
                tracing::warn!(
                    "dropping unsafe Confluence webui path from output: {e}; webui={webui:?}"
                );
                String::new()
            }
        }
    };

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

/// Replace the page's `body` object with a single-key wrapper carrying
/// the pre-rendered body string under the user's requested format.
///
/// The wrapper mirrors the shape of the API's existing `body.storage`
/// / `body.atlas_doc_format` payload (`{"representation": "<key>",
/// "value": "<rendered>"}`) so downstream consumers see a predictable
/// structure regardless of `--body-format`. Crucially it removes the
/// other representations from the response — when the user asks for
/// `markdown`, the raw storage XHTML must not leak through alongside
/// the converted markdown.
///
/// For `Storage` / `View` / `Adf`, `rendered` is the same string the
/// API already returned in `body.<key>.value`, so the rewrite is
/// effectively a normalisation step.
fn rewrite_body_field(mut value: Value, body_format: BodyFormat, rendered: String) -> Value {
    let key = match body_format {
        BodyFormat::Markdown => "markdown",
        BodyFormat::Storage => "storage",
        BodyFormat::View => "view",
        BodyFormat::Adf => "atlas_doc_format",
    };
    if let Some(obj) = value.as_object_mut() {
        let body = serde_json::json!({
            "representation": key,
            "value": rendered,
        });
        obj.insert("body".into(), serde_json::json!({ key: body }));
    }
    value
}

async fn dispatch(
    cmd: &ConfluenceSubcommand,
    client: &ConfluenceClient,
    instance: &AtlassianInstance,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let value = match cmd {
        ConfluenceSubcommand::Read(args) => {
            let expand = compute_read_expand(args);
            let value = client
                .get_page(&args.page_id, args.body_format.wire_format(), &expand)
                .await?;
            // Convert the raw API body into the user's requested format
            // *before* flattening or shaping the response. Without this,
            // `--body-format markdown` (the default) returned raw storage
            // XHTML because the conversion was wired only into `export`.
            let rendered = extract_body(
                &value,
                args.body_format,
                ExtractOpts {
                    render_directives: !args.no_directives,
                },
            )?;
            if matches!(format, OutputFormat::Console) && expand.is_empty() {
                flatten_confluence_page(value, instance, &rendered)
            } else {
                rewrite_body_field(value, args.body_format, rendered)
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
            let body = convert_input(read_body_arg(&args.body)?, &args.input_format)?;
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
            let body = convert_input(read_body_arg(&args.body)?, &args.input_format)?;
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
            let cql = build_find_cql(&args.title, args.space.as_deref());
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
    use crate::config::AuthType;
    use serde_json::json;

    /// Builds a minimal `AtlassianInstance` for tests that exercise URL
    /// formatting. The domain is the only field that affects the
    /// `flatten_confluence_page` output.
    fn test_instance(domain: &str) -> AtlassianInstance {
        AtlassianInstance {
            domain: domain.to_string(),
            email: None,
            api_token: None,
            auth_type: AuthType::default(),
            api_path: None,
            read_only: false,
            flavor: None,
        }
    }

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
        let result = flatten_confluence_page(
            input,
            &test_instance("example.atlassian.net"),
            "<p>Hello world</p>",
        );
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
        // The URL is anchored to the locally configured domain. The path
        // component of `_links.base` (here `/wiki`) is used as a context
        // prefix because its host matches the configured domain — this
        // is the canonical Confluence Cloud shape.
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
    fn flatten_page_uses_configured_domain_when_base_absent() {
        // Even with no `_links.base`, the URL is built from the configured
        // domain — never as a bare server-relative path that downstream
        // tools would have to guess at.
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "_links": { "webui": "/spaces/X/overview" }
        });
        let result = flatten_confluence_page(input, &test_instance("example.atlassian.net"), "");
        assert_eq!(
            result.get("url").and_then(Value::as_str),
            Some("https://example.atlassian.net/spaces/X/overview"),
            "url must be qualified with the configured domain"
        );
    }

    #[test]
    fn flatten_page_omits_empty_body() {
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current"
        });
        let result = flatten_confluence_page(input, &test_instance("example.atlassian.net"), "");
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
        let result = flatten_confluence_page(input, &test_instance("example.atlassian.net"), "");
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
        let result =
            flatten_confluence_page(input, &test_instance("example.atlassian.net"), "content");
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

    #[test]
    fn flatten_page_ignores_attacker_controlled_base() {
        // A compromised / MITM-proxied Confluence server can return a
        // hostile `_links.base`. When its host doesn't match the
        // configured domain, both its host AND its path are discarded —
        // the URL stays anchored to the configured domain with no
        // context prefix.
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "_links": {
                "webui": "/x",
                "base": "https://attacker.example/wiki"
            }
        });
        let result = flatten_confluence_page(input, &test_instance("example.atlassian.net"), "");
        assert_eq!(
            result.get("url").and_then(Value::as_str),
            Some("https://example.atlassian.net/x"),
            "url must use configured domain and ignore attacker-controlled base entirely"
        );
    }

    #[test]
    fn flatten_page_preserves_legitimate_wiki_context_path() {
        // Counterpart to `flatten_page_ignores_attacker_controlled_base`:
        // when `_links.base` is from the same host as the configured
        // domain, its path component IS preserved so Confluence Cloud
        // URLs include the `/wiki` context. Without this, every URL we
        // emit for Cloud 404s.
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "_links": {
                "webui": "/spaces/X/pages/123",
                "base": "https://example.atlassian.net/wiki"
            }
        });
        let result = flatten_confluence_page(input, &test_instance("example.atlassian.net"), "");
        assert_eq!(
            result.get("url").and_then(Value::as_str),
            Some("https://example.atlassian.net/wiki/spaces/X/pages/123"),
            "legitimate same-host base path must be used as context prefix"
        );
    }

    #[test]
    fn flatten_page_drops_url_when_webui_unsafe() {
        // If the server returns a webui that fails validation (e.g. a
        // scheme-relative URL pointing at an attacker), we drop the url
        // field entirely rather than emit something dangerous. Output
        // formatting must never bail — the rest of the page still flattens.
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "_links": { "webui": "//attacker.example/x" }
        });
        let result = flatten_confluence_page(input, &test_instance("example.atlassian.net"), "");
        assert!(
            result.get("url").is_none(),
            "url must be dropped when webui fails validation"
        );
        // Other fields still present.
        assert_eq!(result.get("id").and_then(Value::as_str), Some("1"));
        assert_eq!(result.get("title").and_then(Value::as_str), Some("T"));
    }

    // ---- escape_cql ----

    #[test]
    fn escape_cql_no_special_chars_passthrough() {
        assert_eq!(escape_cql("hello"), "hello");
    }

    #[test]
    fn escape_cql_escapes_backslash_and_quote() {
        assert_eq!(
            escape_cql(r#"a\b"c"#),
            r#"a\\b\"c"#,
            "both backslash and double-quote must be escaped"
        );
    }

    #[test]
    fn escape_cql_empty_string() {
        assert_eq!(escape_cql(""), "");
    }

    // ---- build_find_cql ----

    #[test]
    fn build_find_cql_title_only() {
        let cql = build_find_cql("My Page", None);
        assert_eq!(cql, "title=\"My Page\" AND type=page");
    }

    #[test]
    fn build_find_cql_with_space_appends_clause() {
        let cql = build_find_cql("My Page", Some("TEAM"));
        assert_eq!(cql, "title=\"My Page\" AND type=page AND space=\"TEAM\"");
    }

    #[test]
    fn build_find_cql_escapes_quote_in_title_and_space() {
        // Hostile title containing a quote must be escaped to avoid CQL
        // injection. The same applies to the space clause.
        let cql = build_find_cql(r#"weird"name"#, Some(r#"space"x"#));
        assert_eq!(
            cql,
            r#"title="weird\"name" AND type=page AND space="space\"x""#
        );
    }

    #[test]
    fn build_find_cql_escapes_backslash() {
        let cql = build_find_cql(r"back\slash", None);
        assert_eq!(cql, r#"title="back\\slash" AND type=page"#);
    }

    // ---- compute_read_expand ----

    fn default_read_args() -> ConfluenceReadArgs {
        ConfluenceReadArgs {
            page_id: "1".into(),
            body_format: BodyFormat::Storage,
            no_directives: false,
            include_labels: false,
            include_properties: false,
            include_operations: false,
            include_versions: false,
            include_collaborators: false,
            include_favorited_by: false,
            web: false,
        }
    }

    #[test]
    fn compute_read_expand_no_flags_yields_empty() {
        let args = default_read_args();
        assert!(
            compute_read_expand(&args).is_empty(),
            "expand list must start empty when no --include-* flag is set"
        );
    }

    #[test]
    fn compute_read_expand_each_flag_maps_to_correct_token() {
        // One test per flag would balloon the file; this table-driven test
        // pairs each flag with its expected expand token in a single pass.
        let cases = [
            (
                "labels",
                "metadata.labels",
                ConfluenceReadArgs {
                    include_labels: true,
                    ..default_read_args()
                },
            ),
            (
                "properties",
                "metadata.properties",
                ConfluenceReadArgs {
                    include_properties: true,
                    ..default_read_args()
                },
            ),
            (
                "operations",
                "operations",
                ConfluenceReadArgs {
                    include_operations: true,
                    ..default_read_args()
                },
            ),
            (
                "versions",
                "version",
                ConfluenceReadArgs {
                    include_versions: true,
                    ..default_read_args()
                },
            ),
            (
                "collaborators",
                "collaborators",
                ConfluenceReadArgs {
                    include_collaborators: true,
                    ..default_read_args()
                },
            ),
            (
                "favorited_by",
                "metadata.currentuser.favourited",
                ConfluenceReadArgs {
                    include_favorited_by: true,
                    ..default_read_args()
                },
            ),
        ];
        for (label, expected_token, args) in cases {
            let expand = compute_read_expand(&args);
            assert_eq!(
                expand,
                vec![expected_token],
                "case {label}: expected just [{expected_token:?}], got {expand:?}"
            );
        }
    }

    #[test]
    fn compute_read_expand_all_flags_in_documented_order() {
        // The order matters: the same token order is what reaches the API,
        // and we want it stable so the resulting URL is reproducible.
        let args = ConfluenceReadArgs {
            include_labels: true,
            include_properties: true,
            include_operations: true,
            include_versions: true,
            include_collaborators: true,
            include_favorited_by: true,
            ..default_read_args()
        };
        let expand = compute_read_expand(&args);
        assert_eq!(
            expand,
            vec![
                "metadata.labels",
                "metadata.properties",
                "operations",
                "version",
                "collaborators",
                "metadata.currentuser.favourited",
            ]
        );
    }

    // ---- cmd_uses_pager ----

    #[test]
    fn cmd_uses_pager_read_qualifies() {
        let cmd = ConfluenceSubcommand::Read(default_read_args());
        assert!(cmd_uses_pager(&cmd));
    }

    #[test]
    fn cmd_uses_pager_search_qualifies() {
        let cmd = ConfluenceSubcommand::Search(ConfluenceSearchArgs {
            cql: "type=page".into(),
            limit: 25,
            all: false,
        });
        assert!(cmd_uses_pager(&cmd));
    }

    #[test]
    fn cmd_uses_pager_short_command_does_not_qualify() {
        // A delete command produces single-line output and must not engage
        // the pager — that would obscure the success/failure message.
        let cmd = ConfluenceSubcommand::Delete(ConfluenceDeleteArgs {
            page_id: "1".into(),
            purge: false,
            draft: false,
        });
        assert!(!cmd_uses_pager(&cmd));
    }

    // ---- flatten_confluence_search additional cases ----

    #[test]
    fn flatten_search_empty_results_array() {
        // Empty results array must still flatten to an empty array, not
        // pass through as the original wrapper object.
        let input = json!({"results": [], "size": 0});
        let result = flatten_confluence_search(input);
        assert_eq!(
            result,
            json!([]),
            "empty results should flatten to an empty array"
        );
    }

    #[test]
    fn flatten_search_results_not_array_passthrough() {
        // If `results` is not an array (server schema drift), pass through
        // the entire value rather than panicking.
        let input = json!({"results": "broken"});
        let result = flatten_confluence_search(input.clone());
        assert_eq!(result, input);
    }

    #[test]
    fn flatten_search_handles_missing_optional_fields() {
        // status / _links may be absent on certain item kinds (e.g. attachments).
        let input = json!({
            "results": [{"id": "1", "title": "T", "type": "page"}]
        });
        let result = flatten_confluence_search(input);
        let item = &result.as_array().unwrap()[0];
        assert_eq!(item.get("status").and_then(Value::as_str), Some(""));
        assert_eq!(item.get("url").and_then(Value::as_str), Some(""));
    }

    // ---- flatten_confluence_page additional cases ----

    #[test]
    fn flatten_page_uses_provided_body_verbatim() {
        // The helper must surface the body string passed by the caller —
        // never re-extract from the raw `body.<repr>.value` field. If the
        // caller already converted storage XHTML to markdown, the helper
        // must trust that string and not re-render the original storage.
        let input = json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "body": {
                "storage": {"value": "<p>raw storage</p>", "representation": "storage"}
            }
        });
        let result = flatten_confluence_page(
            input,
            &test_instance("example.atlassian.net"),
            "converted markdown",
        );
        assert_eq!(
            result.get("body").and_then(Value::as_str),
            Some("converted markdown"),
            "body field must come from the parameter, not the raw API value"
        );
    }

    #[test]
    fn flatten_page_missing_version_yields_empty_string() {
        // The console reporter renders `version: ""` rather than dropping
        // the field — confirms `unwrap_or_default` on the version path.
        let input = json!({"id": "1", "title": "T", "status": "current"});
        let result = flatten_confluence_page(input, &test_instance("example.atlassian.net"), "");
        assert_eq!(result.get("version").and_then(Value::as_str), Some(""));
    }

    // ---- rewrite_body_field ----

    #[test]
    fn rewrite_body_field_markdown_replaces_raw_storage() {
        // The user asked for `--body-format markdown`. The non-Console
        // dispatcher must replace the API's raw `body.storage` payload with
        // a markdown-shaped wrapper carrying the converted text — the raw
        // storage XHTML must NOT leak through alongside the markdown.
        let api_value = json!({
            "id": "1",
            "title": "T",
            "body": {
                "storage": {"value": "<h1>Hi</h1>", "representation": "storage"}
            }
        });
        let result = rewrite_body_field(api_value, BodyFormat::Markdown, "# Hi".into());
        let body = result.get("body").expect("body must exist");
        assert!(
            body.get("storage").is_none(),
            "raw storage representation must be replaced, got: {body:?}"
        );
        let md = body.get("markdown").expect("markdown wrapper must exist");
        assert_eq!(md.get("value").and_then(Value::as_str), Some("# Hi"));
        assert_eq!(
            md.get("representation").and_then(Value::as_str),
            Some("markdown")
        );
    }

    #[test]
    fn rewrite_body_field_storage_normalises_shape() {
        // Even when the user keeps `--body-format storage`, the helper
        // normalises the shape to a single-key `body.storage` wrapper so
        // downstream consumers always see a predictable structure.
        let api_value = json!({
            "id": "1",
            "body": {
                "storage": {"value": "<p>x</p>", "representation": "storage"}
            }
        });
        let result = rewrite_body_field(api_value, BodyFormat::Storage, "<p>x</p>".into());
        let storage = result
            .pointer("/body/storage")
            .expect("storage wrapper must exist");
        assert_eq!(
            storage.get("value").and_then(Value::as_str),
            Some("<p>x</p>")
        );
        assert_eq!(
            storage.get("representation").and_then(Value::as_str),
            Some("storage")
        );
    }

    #[test]
    fn rewrite_body_field_adf_uses_atlas_doc_format_key() {
        // ADF must be wrapped under `atlas_doc_format` to match the API's
        // own naming, so existing JSON consumers don't have to special-case
        // a new key for `--body-format adf`.
        let api_value = json!({"id": "1"});
        let result = rewrite_body_field(api_value, BodyFormat::Adf, "{}".into());
        assert!(
            result.pointer("/body/atlas_doc_format").is_some(),
            "ADF wrapper must use atlas_doc_format key, got: {result:?}"
        );
    }

    // ---- Read-pipeline behaviour (extract_body + rewrite/flatten) ----
    //
    // The `Read` arm of `dispatch` runs the API page value through
    // `extract_body` and then passes the result to either
    // `flatten_confluence_page` (Console) or `rewrite_body_field` (other
    // formats). These tests pin down the end-to-end shape for each
    // `--body-format` choice without spinning up the HTTP layer.

    fn fake_storage_page(storage: &str) -> Value {
        json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "body": {"storage": {"value": storage, "representation": "storage"}},
        })
    }

    #[test]
    fn read_default_markdown_returns_markdown_in_body_field() {
        // The default `--body-format markdown` path must run storage XHTML
        // through the converter. Before the fix, the raw `<p>hi</p>` was
        // echoed back as-is on the read path.
        let page = fake_storage_page("<p>hi</p>");
        let rendered = extract_body(&page, BodyFormat::Markdown, ExtractOpts::default()).unwrap();
        // Console flattening shape — what the user sees by default.
        let flat = flatten_confluence_page(
            page.clone(),
            &test_instance("example.atlassian.net"),
            &rendered,
        );
        let body = flat
            .get("body")
            .and_then(Value::as_str)
            .expect("body field");
        assert!(
            !body.contains("<p>"),
            "markdown body must not contain raw HTML, got: {body:?}"
        );
        assert!(
            body.contains("hi"),
            "markdown body must preserve the paragraph text, got: {body:?}"
        );
        // JSON shape — what `--format json` consumers see.
        let json = rewrite_body_field(page, BodyFormat::Markdown, rendered.clone());
        assert_eq!(
            json.pointer("/body/markdown/value").and_then(Value::as_str),
            Some(rendered.as_str())
        );
        assert!(
            json.pointer("/body/storage").is_none(),
            "raw storage representation must not leak through on markdown read"
        );
    }

    #[test]
    fn read_storage_returns_raw_storage_unchanged() {
        // `--body-format storage` must not touch the body — the user gets
        // the raw XHTML byte-for-byte.
        let xhtml = "<h1>raw</h1>";
        let page = fake_storage_page(xhtml);
        let rendered = extract_body(&page, BodyFormat::Storage, ExtractOpts::default()).unwrap();
        assert_eq!(rendered, xhtml);
        let json = rewrite_body_field(page, BodyFormat::Storage, rendered);
        assert_eq!(
            json.pointer("/body/storage/value").and_then(Value::as_str),
            Some(xhtml)
        );
    }

    #[test]
    fn read_adf_returns_pretty_adf_json() {
        // `--body-format adf` must emit pretty-printed canonical ADF JSON,
        // delivered through both Console (flat body string) and JSON
        // (`body.atlas_doc_format.value`) paths.
        let adf_compact = r#"{"type":"doc","version":1,"content":[]}"#;
        let page = json!({
            "id": "1",
            "title": "T",
            "status": "current",
            "body": {"atlas_doc_format": {"value": adf_compact, "representation": "atlas_doc_format"}},
        });
        let rendered = extract_body(&page, BodyFormat::Adf, ExtractOpts::default()).unwrap();
        assert!(
            rendered.contains('\n'),
            "ADF body must be pretty-printed, got: {rendered:?}"
        );
        let parsed: Value =
            serde_json::from_str(&rendered).expect("pretty ADF must still be valid JSON");
        assert_eq!(parsed.get("type").and_then(Value::as_str), Some("doc"));
        let json = rewrite_body_field(page, BodyFormat::Adf, rendered);
        assert!(
            json.pointer("/body/atlas_doc_format/value").is_some(),
            "ADF JSON path must use atlas_doc_format key"
        );
    }

    #[test]
    fn read_no_directives_strips_directive_fence() {
        // `--body-format markdown --no-directives` must flatten Confluence
        // info macros into plain text — the `:::info` fence must not appear
        // in the output.
        let storage = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>note</p></ac:rich-text-body></ac:structured-macro>"#;
        let page = fake_storage_page(storage);
        let rendered = extract_body(
            &page,
            BodyFormat::Markdown,
            ExtractOpts {
                render_directives: false,
            },
        )
        .unwrap();
        assert!(
            !rendered.contains(":::info"),
            "no_directives must strip the :::info fence, got: {rendered:?}"
        );
        assert!(
            rendered.contains("note"),
            "macro body text must be preserved, got: {rendered:?}"
        );
    }

    #[test]
    fn rewrite_body_field_preserves_sibling_fields() {
        // Only the `body` key is replaced — `id`, `title`, `_links`, etc.
        // must round-trip untouched so JSON consumers still see the page
        // metadata they expect.
        let api_value = json!({
            "id": "1",
            "title": "T",
            "_links": {"webui": "/x"},
            "body": {"storage": {"value": "old"}}
        });
        let result = rewrite_body_field(api_value, BodyFormat::Markdown, "new".into());
        assert_eq!(result.get("id").and_then(Value::as_str), Some("1"));
        assert_eq!(result.get("title").and_then(Value::as_str), Some("T"));
        assert!(
            result.get("_links").is_some(),
            "_links must round-trip untouched"
        );
    }

    // ---- run() error paths ----
    //
    // Mirrors the Jira run() error coverage: each pre-HTTP branch has a
    // dedicated test so the user-facing error message and the exit-code
    // mapping (Error::Config → 3 in error.rs) stay locked in.

    use crate::output::{OutputFormat, Transforms};
    use camino::Utf8PathBuf;

    fn write_config(dir: &tempfile::TempDir, body: &str) -> Utf8PathBuf {
        let path = dir.path().join("atl.toml");
        let mut f = std::fs::File::create(&path).expect("create config file");
        // `Write` is already brought into scope at the top of the module.
        f.write_all(body.as_bytes()).expect("write config body");
        Utf8PathBuf::try_from(path).expect("UTF-8 temp path")
    }

    fn search_cmd() -> ConfluenceSubcommand {
        ConfluenceSubcommand::Search(ConfluenceSearchArgs {
            cql: "type=page".into(),
            limit: 25,
            all: false,
        })
    }

    #[tokio::test]
    async fn run_errors_when_config_path_does_not_exist() {
        let mut io = IoStreams::test();
        let cmd = search_cmd();
        let bogus = Utf8PathBuf::from("/definitely/does/not/exist/atl.toml");
        let err = run(
            &cmd,
            Some(&bogus),
            None,
            RetryConfig::off(),
            &OutputFormat::Json,
            &mut io,
            &Transforms::none(),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("config file not found"),
            "expected 'config file not found' error, got: {err}"
        );
    }

    #[tokio::test]
    async fn run_errors_when_no_profile_in_config() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let cfg = write_config(&dir, "default_profile = \"work\"\n");
        let mut io = IoStreams::test();
        let cmd = search_cmd();
        let err = run(
            &cmd,
            Some(&cfg),
            None,
            RetryConfig::off(),
            &OutputFormat::Json,
            &mut io,
            &Transforms::none(),
        )
        .await
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no profile found"),
            "expected 'no profile found' message, got: {msg}"
        );
        assert!(
            msg.contains("atl init"),
            "error must mention `atl init` recovery path, got: {msg}"
        );
    }

    #[tokio::test]
    async fn run_errors_when_profile_has_no_confluence_instance() {
        // Profile has only a Jira instance; a `confluence` subcommand must
        // surface a clear error so the user knows the profile is incomplete.
        let dir = tempfile::tempdir().expect("create tempdir");
        let cfg = write_config(
            &dir,
            r#"default_profile = "work"

[profiles.work.jira]
domain = "x.atlassian.net"
"#,
        );
        let mut io = IoStreams::test();
        let cmd = search_cmd();
        let err = run(
            &cmd,
            Some(&cfg),
            None,
            RetryConfig::off(),
            &OutputFormat::Json,
            &mut io,
            &Transforms::none(),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("no Confluence instance configured"),
            "expected 'no Confluence instance configured', got: {err}"
        );
    }
}
