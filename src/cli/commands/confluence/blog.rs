use serde_json::{Value, json};

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

use super::page::{ExtractOpts, convert_input, extract_body};
use super::property::dispatch_resource_property;

/// Build the `expand` query-parameter list for a blog post `Read` request from
/// the user's `--include-*` boolean flags. Order matches the source ordering of
/// the booleans so output is deterministic for snapshot/log inspection.
pub(super) fn build_blog_read_expand(args: &ConfluenceBlogReadArgs) -> Vec<&'static str> {
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
    expand
}

/// Resolve the space target for a blog Create from the `--space` (key) or
/// `--space-id` (numeric) flag, in that order. Clap guarantees at least one of
/// the two is set via `required_unless_present`, so we panic if both are
/// missing — that is a programmer error in the arg definitions, not user
/// input.
pub(super) fn resolve_blog_create_space(args: &ConfluenceBlogCreateArgs) -> &str {
    args.space
        .as_deref()
        .or(args.space_id.as_deref())
        .expect("clap enforces required_unless_present=space_id on ConfluenceBlogCreateArgs")
}

/// Rewrite the body value of a blog post response to the requested
/// `body_format`, converting markdown / pretty-printing ADF as needed.
///
/// Operates in place on the JSON returned by the Confluence API: writes
/// the converted output produced by [`extract_body`] to
/// `body.{repr}.value`, where `repr` is the user-facing key for the
/// requested `body_format` (e.g. `markdown` for [`BodyFormat::Markdown`],
/// matching the shape produced by `rewrite_body_field`).
///
/// The path is created on demand: if `body` or `body.{repr}` is missing
/// (e.g. the API returned ADF when storage was requested, or vice versa,
/// and our converter recovered via the alternative representation), the
/// converted result is still surfaced rather than silently dropped.
///
/// Mirrors the page export flow so blog Read returns the same shape page
/// readers expect — without this step, `--body-format markdown` would
/// return raw storage XHTML, which is the bug this code fixes.
pub(super) fn convert_blog_body(
    response: &mut Value,
    body_format: BodyFormat,
    render_directives: bool,
) -> anyhow::Result<()> {
    let converted = extract_body(response, body_format, ExtractOpts { render_directives })?;
    let repr = match body_format {
        BodyFormat::Markdown => "markdown",
        BodyFormat::Storage => "storage",
        BodyFormat::View => "view",
        BodyFormat::Adf => "atlas_doc_format",
    };
    let Some(obj) = response.as_object_mut() else {
        return Ok(());
    };
    let body = obj
        .entry("body")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("body must be an object after or_insert_with(json!({}))");
    let repr_obj = body
        .entry(repr.to_string())
        .or_insert_with(|| json!({"representation": repr}))
        .as_object_mut()
        .expect("body.{repr} must be an object after or_insert_with(json!({}))");
    repr_obj.insert("value".into(), Value::String(converted));
    repr_obj
        .entry("representation".to_string())
        .or_insert_with(|| Value::String(repr.to_string()));
    Ok(())
}

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
            let expand = build_blog_read_expand(args);
            let mut value = client
                .get_blog_post(&args.blog_id, args.body_format.wire_format(), &expand)
                .await?;
            convert_blog_body(&mut value, args.body_format, !args.no_directives)?;
            value
        }
        ConfluenceBlogSubcommand::Create(args) => {
            let body = convert_input(read_body_arg(&args.body)?, &args.input_format)?;
            let space = resolve_blog_create_space(args);
            client
                .create_blog_post(space, &args.title, &body, args.private)
                .await?
        }
        ConfluenceBlogSubcommand::Update(args) => {
            let body = convert_input(read_body_arg(&args.body)?, &args.input_format)?;
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

#[cfg(test)]
mod tests {
    // Most arms are pure HTTP delegation and are covered by contract tests in
    // tests/contract_confluence_v*.rs. Only the small pure helpers
    // (`build_blog_read_expand` and `resolve_blog_create_space`) are unit-tested
    // here.

    use super::*;

    fn read_args(
        labels: bool,
        properties: bool,
        operations: bool,
        versions: bool,
        collaborators: bool,
    ) -> ConfluenceBlogReadArgs {
        ConfluenceBlogReadArgs {
            blog_id: "1".into(),
            body_format: BodyFormat::Storage,
            no_directives: false,
            include_labels: labels,
            include_properties: properties,
            include_operations: operations,
            include_versions: versions,
            include_collaborators: collaborators,
        }
    }

    fn create_args(space: Option<&str>, space_id: Option<&str>) -> ConfluenceBlogCreateArgs {
        ConfluenceBlogCreateArgs {
            space: space.map(String::from),
            space_id: space_id.map(String::from),
            title: "title".into(),
            body: "body".into(),
            input_format: InputFormat::Storage,
            private: false,
        }
    }

    // ---- build_blog_read_expand ----

    #[test]
    fn expand_empty_when_no_flags_set() {
        let expand = build_blog_read_expand(&read_args(false, false, false, false, false));
        assert!(
            expand.is_empty(),
            "expected no expand entries, got {expand:?}"
        );
    }

    #[test]
    fn expand_all_flags_in_source_order() {
        let expand = build_blog_read_expand(&read_args(true, true, true, true, true));
        assert_eq!(
            expand,
            vec![
                "metadata.labels",
                "metadata.properties",
                "operations",
                "version",
                "collaborators",
            ],
            "expand entries should appear in source order"
        );
    }

    #[test]
    fn expand_only_labels_set() {
        let expand = build_blog_read_expand(&read_args(true, false, false, false, false));
        assert_eq!(expand, vec!["metadata.labels"]);
    }

    #[test]
    fn expand_only_properties_set() {
        let expand = build_blog_read_expand(&read_args(false, true, false, false, false));
        assert_eq!(expand, vec!["metadata.properties"]);
    }

    #[test]
    fn expand_only_operations_set() {
        let expand = build_blog_read_expand(&read_args(false, false, true, false, false));
        assert_eq!(expand, vec!["operations"]);
    }

    #[test]
    fn expand_only_versions_set() {
        let expand = build_blog_read_expand(&read_args(false, false, false, true, false));
        assert_eq!(expand, vec!["version"]);
    }

    #[test]
    fn expand_only_collaborators_set() {
        let expand = build_blog_read_expand(&read_args(false, false, false, false, true));
        assert_eq!(expand, vec!["collaborators"]);
    }

    #[test]
    fn expand_subset_preserves_order() {
        // Labels + operations + collaborators (skipping properties + versions)
        let expand = build_blog_read_expand(&read_args(true, false, true, false, true));
        assert_eq!(
            expand,
            vec!["metadata.labels", "operations", "collaborators"]
        );
    }

    // ---- resolve_blog_create_space ----

    #[test]
    fn resolve_space_prefers_key_when_both_set() {
        // Clap's `conflicts_with` makes both-set impossible at the CLI layer,
        // but if a future change relaxes that, we still pick `space` first.
        let args = create_args(Some("KEY"), Some("12345"));
        assert_eq!(resolve_blog_create_space(&args), "KEY");
    }

    #[test]
    fn resolve_space_falls_back_to_space_id() {
        let args = create_args(None, Some("12345"));
        assert_eq!(resolve_blog_create_space(&args), "12345");
    }

    #[test]
    fn resolve_space_uses_key_when_only_key_set() {
        let args = create_args(Some("KEY"), None);
        assert_eq!(resolve_blog_create_space(&args), "KEY");
    }

    #[test]
    #[should_panic(expected = "clap enforces required_unless_present=space_id")]
    fn resolve_space_panics_when_both_none() {
        // This case is impossible in practice (clap rejects it), but we
        // document the contract: if both are unset we panic with a clear
        // message rather than silently picking an empty string.
        let args = create_args(None, None);
        let _ = resolve_blog_create_space(&args);
    }

    // ---- convert_blog_body ----

    #[test]
    fn convert_blog_body_markdown_replaces_storage_xhtml() {
        // The default `--body-format markdown` requested markdown but the
        // wire format is still `storage`. Conversion must write the
        // converted markdown to `body.markdown.value` so the consumer
        // never has to inspect the wire-format key.
        let mut response = serde_json::json!({
            "id": "1",
            "title": "post",
            "body": {"storage": {"value": "<p>Hello</p>", "representation": "storage"}}
        });
        convert_blog_body(&mut response, BodyFormat::Markdown, true).unwrap();
        let value = response
            .pointer("/body/markdown/value")
            .and_then(Value::as_str)
            .expect("body.markdown.value should be populated");
        assert!(
            value.contains("Hello"),
            "expected markdown to contain the paragraph text, got: {value:?}"
        );
        assert!(
            !value.contains("<p>"),
            "expected XHTML tags to be stripped during markdown conversion, got: {value:?}"
        );
    }

    #[test]
    fn convert_blog_body_markdown_no_directives_strips_directive_syntax() {
        // With `render_directives: false`, info macros must collapse to
        // plain body text — the `:::info` fence must not appear in the
        // resulting markdown.
        let storage = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>note</p></ac:rich-text-body></ac:structured-macro>"#;
        let mut response = serde_json::json!({
            "body": {"storage": {"value": storage, "representation": "storage"}}
        });
        convert_blog_body(&mut response, BodyFormat::Markdown, false).unwrap();
        let value = response
            .pointer("/body/markdown/value")
            .and_then(Value::as_str)
            .expect("body.markdown.value should be populated");
        assert!(
            !value.contains(":::info"),
            "no_directives must strip the :::info fence, got: {value:?}"
        );
        assert!(
            value.contains("note"),
            "directive body text must be preserved, got: {value:?}"
        );
    }

    #[test]
    fn convert_blog_body_storage_format_passthrough() {
        // Storage body_format must leave the raw XHTML untouched so users
        // who explicitly opt out of conversion still see the canonical
        // wire format.
        let mut response = serde_json::json!({
            "body": {"storage": {"value": "<p>raw</p>", "representation": "storage"}}
        });
        convert_blog_body(&mut response, BodyFormat::Storage, true).unwrap();
        assert_eq!(
            response
                .pointer("/body/storage/value")
                .and_then(Value::as_str),
            Some("<p>raw</p>"),
            "storage body_format must be a passthrough"
        );
    }

    #[test]
    fn convert_blog_body_missing_body_creates_path() {
        // A response without a `body` object must surface the converted
        // value rather than dropping it. The path is created on demand
        // so the consumer always sees `body.{repr}.value`.
        let mut response = serde_json::json!({"id": "1", "title": "post"});
        convert_blog_body(&mut response, BodyFormat::Markdown, true).unwrap();
        assert_eq!(
            response
                .pointer("/body/markdown/value")
                .and_then(Value::as_str),
            Some(""),
            "body.markdown.value must be created even when the input has no body"
        );
        assert_eq!(
            response
                .pointer("/body/markdown/representation")
                .and_then(Value::as_str),
            Some("markdown"),
            "body.markdown.representation must be set when the path is created"
        );
    }

    #[test]
    fn convert_blog_body_writes_value_when_body_repr_missing() {
        // When `body` exists but the requested representation key is
        // absent (e.g. the API returned ADF when storage was requested
        // and our converter recovered via the alternative form), the
        // converted result must be upserted into `body.{repr}.value`
        // rather than silently dropped.
        let mut response = serde_json::json!({
            "id": "1",
            "title": "post",
            "body": {}
        });
        convert_blog_body(&mut response, BodyFormat::Markdown, true).unwrap();
        let value = response
            .pointer("/body/markdown/value")
            .and_then(Value::as_str)
            .expect("body.markdown.value must be populated when the repr key was missing");
        // No source body was present, so `extract_body` returned an empty
        // string; the upsert path is what we care about here.
        assert_eq!(value, "");
        assert_eq!(
            response
                .pointer("/body/markdown/representation")
                .and_then(Value::as_str),
            Some("markdown"),
            "representation tag must be set when the repr key is created on demand"
        );
    }
}
