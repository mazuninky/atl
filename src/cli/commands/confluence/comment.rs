use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

use super::property::dispatch_resource_property;

pub(super) async fn dispatch_footer_comment(
    cmd: &ConfluenceFooterCommentSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceFooterCommentSubcommand::List(args) => {
            client
                .list_footer_comments_v2(&args.page_id, args.limit)
                .await?
        }
        ConfluenceFooterCommentSubcommand::Get(args) => {
            client.get_footer_comment_v2(&args.comment_id).await?
        }
        ConfluenceFooterCommentSubcommand::Create(args) => {
            let body = read_body_arg(&args.body)?;
            client
                .create_footer_comment_v2(&args.page_id, &body)
                .await?
        }
        ConfluenceFooterCommentSubcommand::Update(args) => {
            let body = read_body_arg(&args.body)?;
            client
                .update_footer_comment_v2(&args.comment_id, &body, args.version)
                .await?
        }
        ConfluenceFooterCommentSubcommand::Delete(args) => {
            client.delete_footer_comment_v2(&args.comment_id).await?;
            Value::String(format!("Footer comment {} deleted", args.comment_id))
        }
        ConfluenceFooterCommentSubcommand::Children(args) => {
            client
                .get_footer_comment_children_v2(&args.comment_id, args.limit)
                .await?
        }
        ConfluenceFooterCommentSubcommand::Versions(args) => {
            client
                .get_footer_comment_versions_v2(&args.comment_id, args.limit)
                .await?
        }
        ConfluenceFooterCommentSubcommand::Likes(args) => {
            client.get_footer_comment_likes_v2(&args.comment_id).await?
        }
        ConfluenceFooterCommentSubcommand::Operations(args) => {
            client
                .get_footer_comment_operations_v2(&args.comment_id)
                .await?
        }
        ConfluenceFooterCommentSubcommand::LikesCount(args) => {
            client
                .get_footer_comment_likes_count_v2(&args.comment_id)
                .await?
        }
        ConfluenceFooterCommentSubcommand::LikesUsers(args) => {
            client
                .get_footer_comment_likes_users_v2(&args.comment_id)
                .await?
        }
        ConfluenceFooterCommentSubcommand::VersionDetails(args) => {
            client
                .get_footer_comment_version_v2(&args.comment_id, args.version)
                .await?
        }
        ConfluenceFooterCommentSubcommand::Property(cmd) => {
            dispatch_resource_property("footer-comments", &cmd.command, client).await?
        }
    })
}

pub(super) async fn dispatch_inline_comment(
    cmd: &ConfluenceInlineCommentSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceInlineCommentSubcommand::List(args) => {
            client
                .list_inline_comments_v2(
                    &args.page_id,
                    args.limit,
                    args.resolution_status.as_deref(),
                )
                .await?
        }
        ConfluenceInlineCommentSubcommand::Get(args) => {
            client.get_inline_comment_v2(&args.comment_id).await?
        }
        ConfluenceInlineCommentSubcommand::Create(args) => {
            let body = read_body_arg(&args.body)?;
            client
                .create_inline_comment_v2(
                    &args.page_id,
                    &body,
                    &args.inline_marker_ref,
                    args.text_selection.as_deref(),
                )
                .await?
        }
        ConfluenceInlineCommentSubcommand::Update(args) => {
            let body = read_body_arg(&args.body)?;
            client
                .update_inline_comment_v2(&args.comment_id, &body, args.version, args.resolved)
                .await?
        }
        ConfluenceInlineCommentSubcommand::Delete(args) => {
            client.delete_inline_comment_v2(&args.comment_id).await?;
            Value::String(format!("Inline comment {} deleted", args.comment_id))
        }
        ConfluenceInlineCommentSubcommand::Children(args) => {
            client
                .get_inline_comment_children_v2(&args.comment_id, args.limit)
                .await?
        }
        ConfluenceInlineCommentSubcommand::Versions(args) => {
            client
                .get_inline_comment_versions_v2(&args.comment_id, args.limit)
                .await?
        }
        ConfluenceInlineCommentSubcommand::Likes(args) => {
            client.get_inline_comment_likes_v2(&args.comment_id).await?
        }
        ConfluenceInlineCommentSubcommand::Operations(args) => {
            client
                .get_inline_comment_operations_v2(&args.comment_id)
                .await?
        }
        ConfluenceInlineCommentSubcommand::LikesCount(args) => {
            client
                .get_inline_comment_likes_count_v2(&args.comment_id)
                .await?
        }
        ConfluenceInlineCommentSubcommand::LikesUsers(args) => {
            client
                .get_inline_comment_likes_users_v2(&args.comment_id)
                .await?
        }
        ConfluenceInlineCommentSubcommand::VersionDetails(args) => {
            client
                .get_inline_comment_version_v2(&args.comment_id, args.version)
                .await?
        }
        ConfluenceInlineCommentSubcommand::Property(cmd) => {
            dispatch_resource_property("inline-comments", &cmd.command, client).await?
        }
    })
}

#[cfg(test)]
mod tests {
    // Both `dispatch_footer_comment` and `dispatch_inline_comment` are pure
    // HTTP delegation — every arm either calls a typed client method or
    // returns a constant `"<X> deleted"` string. The body-resolution that
    // lives in front of Create/Update is `read_body_arg` from
    // src/cli/commands/mod.rs, which has its own tests.
    //
    // All branches here are covered by contract tests in
    // tests/contract_confluence_v*.rs.
}
