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
    let profile = config
        .as_ref()
        .and_then(|c| c.resolve_profile(profile_name))
        .ok_or_else(|| anyhow::anyhow!("no profile found; run `atl init` first"))?;
    let instance = profile
        .confluence
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no Confluence instance configured in profile"))?;
    let client = ConfluenceClient::connect(instance, retries).await?;

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
            client
                .get_page(&args.page_id, args.body_format.as_str(), &expand)
                .await?
        }
        ConfluenceSubcommand::Info(args) => client.get_page_info(&args.page_id).await?,
        ConfluenceSubcommand::Search(args) => {
            if args.all {
                client.search_all(&args.cql, args.limit).await?
            } else {
                client.search(&args.cql, args.limit).await?
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
            client.search(&cql, args.limit).await?
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
