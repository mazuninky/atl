use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

use super::page::maybe_convert_markdown;
use super::property::dispatch_resource_property;

pub(super) async fn dispatch_blog(
    cmd: &ConfluenceBlogSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceBlogSubcommand::List(args) => {
            client
                .list_blog_posts(args.space.as_deref(), args.limit)
                .await?
        }
        ConfluenceBlogSubcommand::Read(args) => {
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
            client
                .get_blog_post(&args.blog_id, args.body_format.as_str(), &expand)
                .await?
        }
        ConfluenceBlogSubcommand::Create(args) => {
            let body = maybe_convert_markdown(read_body_arg(&args.body)?, &args.input_format);
            let space = args.space.as_deref().or(args.space_id.as_deref()).expect(
                "clap enforces required_unless_present=space_id on ConfluenceBlogCreateArgs",
            );
            client
                .create_blog_post(space, &args.title, &body, args.private)
                .await?
        }
        ConfluenceBlogSubcommand::Update(args) => {
            let body = maybe_convert_markdown(read_body_arg(&args.body)?, &args.input_format);
            client
                .update_blog_post(
                    &args.blog_id,
                    &args.title,
                    &body,
                    args.version,
                    args.version_message.as_deref(),
                )
                .await?
        }
        ConfluenceBlogSubcommand::Delete(args) => {
            client
                .delete_blog_post(&args.blog_id, args.purge, args.draft)
                .await?;
            Value::String(format!("Blog post {} deleted", args.blog_id))
        }
        // v2 sub-resources
        ConfluenceBlogSubcommand::Attachments(args) => {
            client
                .get_blogpost_attachments_v2(&args.blog_id, args.limit)
                .await?
        }
        ConfluenceBlogSubcommand::Labels(args) => {
            client.get_blogpost_labels_v2(&args.blog_id).await?
        }
        ConfluenceBlogSubcommand::FooterComments(args) => {
            client
                .get_blogpost_footer_comments_v2(&args.blog_id, args.limit)
                .await?
        }
        ConfluenceBlogSubcommand::InlineComments(args) => {
            client
                .get_blogpost_inline_comments_v2(&args.blog_id, args.limit)
                .await?
        }
        ConfluenceBlogSubcommand::Versions(args) => {
            client
                .get_blogpost_versions_v2(&args.blog_id, args.limit)
                .await?
        }
        ConfluenceBlogSubcommand::Likes(args) => {
            client.get_blogpost_likes_v2(&args.blog_id).await?
        }
        ConfluenceBlogSubcommand::Operations(args) => {
            client.get_blogpost_operations_v2(&args.blog_id).await?
        }
        ConfluenceBlogSubcommand::VersionDetails(args) => {
            client
                .get_blogpost_version_v2(&args.blog_id, args.version)
                .await?
        }
        ConfluenceBlogSubcommand::LikesCount(args) => {
            client.get_blogpost_likes_count_v2(&args.blog_id).await?
        }
        ConfluenceBlogSubcommand::LikesUsers(args) => {
            client.get_blogpost_likes_users_v2(&args.blog_id).await?
        }
        ConfluenceBlogSubcommand::CustomContent(args) => {
            client
                .get_blogpost_custom_content_v2(&args.blog_id, &args.content_type, args.limit)
                .await?
        }
        ConfluenceBlogSubcommand::Redact(args) => client.redact_blogpost_v2(&args.blog_id).await?,
        ConfluenceBlogSubcommand::Property(cmd) => {
            dispatch_resource_property("blogposts", &cmd.command, client).await?
        }
    })
}
