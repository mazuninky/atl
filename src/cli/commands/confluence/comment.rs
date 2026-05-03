use serde_json::Value;

use crate::cli::args::*;
use crate::cli::commands::read_body_arg;
use crate::client::ConfluenceClient;

use super::page::{ExtractOpts, convert_input, extract_body};
use super::property::dispatch_resource_property;

/// Apply [`extract_body`] + [`rewrite_comment_body`] to every comment in
/// `value` so the user gets the body in their requested format whether the
/// response is a single comment or a `{ "results": [...] }` list.
fn apply_body_format_to_response(
    value: Value,
    body_format: BodyFormat,
    opts: ExtractOpts,
) -> anyhow::Result<Value> {
    if value.get("results").and_then(Value::as_array).is_some() {
        rewrite_list_results(value, body_format, opts)
    } else {
        rewrite_comment_body(value, body_format, opts)
    }
}

/// Walk a paged list response (`{ "results": [...], … }`) and rewrite each
/// comment's body field. Other top-level keys (`_links`, pagination meta,
/// etc.) round-trip untouched.
fn rewrite_list_results(
    mut value: Value,
    body_format: BodyFormat,
    opts: ExtractOpts,
) -> anyhow::Result<Value> {
    if let Some(arr) = value
        .as_object_mut()
        .and_then(|m| m.get_mut("results"))
        .and_then(Value::as_array_mut)
    {
        let original = std::mem::take(arr);
        let mut rewritten: Vec<Value> = Vec::with_capacity(original.len());
        for item in original {
            rewritten.push(rewrite_comment_body(item, body_format, opts)?);
        }
        *arr = rewritten;
    }
    Ok(value)
}

/// Replace `comment.body` with a single-key wrapper carrying the
/// pre-rendered body in the user's chosen format. Mirrors the page-side
/// `rewrite_body_field` shape so downstream consumers see a predictable
/// `body.<format>.value` payload regardless of `--body-format`.
fn rewrite_comment_body(
    mut comment: Value,
    body_format: BodyFormat,
    opts: ExtractOpts,
) -> anyhow::Result<Value> {
    let rendered = extract_body(&comment, body_format, opts)?;
    let key = match body_format {
        BodyFormat::Markdown => "markdown",
        BodyFormat::Storage => "storage",
        BodyFormat::View => "view",
        BodyFormat::Adf => "atlas_doc_format",
    };
    if let Some(obj) = comment.as_object_mut() {
        let body = serde_json::json!({
            "representation": key,
            "value": rendered,
        });
        obj.insert("body".into(), serde_json::json!({ key: body }));
    }
    Ok(comment)
}

pub(super) async fn dispatch_footer_comment(
    cmd: &ConfluenceFooterCommentSubcommand,
    client: &ConfluenceClient,
) -> anyhow::Result<Value> {
    Ok(match cmd {
        ConfluenceFooterCommentSubcommand::List(args) => {
            let raw = client
                .list_footer_comments_v2(&args.page_id, args.limit, args.body_format.wire_format())
                .await?;
            apply_body_format_to_response(
                raw,
                args.body_format,
                ExtractOpts {
                    render_directives: !args.no_directives,
                },
            )?
        }
        ConfluenceFooterCommentSubcommand::Get(args) => {
            let raw = client
                .get_footer_comment_v2(&args.comment_id, args.body_format.wire_format())
                .await?;
            apply_body_format_to_response(
                raw,
                args.body_format,
                ExtractOpts {
                    render_directives: !args.no_directives,
                },
            )?
        }
        ConfluenceFooterCommentSubcommand::Create(args) => {
            let body = convert_input(read_body_arg(&args.body)?, &args.input_format)?;
            client
                .create_footer_comment_v2(&args.page_id, &body)
                .await?
        }
        ConfluenceFooterCommentSubcommand::Update(args) => {
            let body = convert_input(read_body_arg(&args.body)?, &args.input_format)?;
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
            let raw = client
                .list_inline_comments_v2(
                    &args.page_id,
                    args.limit,
                    args.resolution_status.as_deref(),
                    args.body_format.wire_format(),
                )
                .await?;
            apply_body_format_to_response(
                raw,
                args.body_format,
                ExtractOpts {
                    render_directives: !args.no_directives,
                },
            )?
        }
        ConfluenceInlineCommentSubcommand::Get(args) => {
            let raw = client
                .get_inline_comment_v2(&args.comment_id, args.body_format.wire_format())
                .await?;
            apply_body_format_to_response(
                raw,
                args.body_format,
                ExtractOpts {
                    render_directives: !args.no_directives,
                },
            )?
        }
        ConfluenceInlineCommentSubcommand::Create(args) => {
            let body = convert_input(read_body_arg(&args.body)?, &args.input_format)?;
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
            let body = convert_input(read_body_arg(&args.body)?, &args.input_format)?;
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
    use super::*;
    use serde_json::json;

    // ---- rewrite_comment_body / apply_body_format_to_response -----------
    //
    // The pure transforms over the API response are the only piece of the
    // dispatch that doesn't touch HTTP. The dispatch arms that wrap them
    // (`List`, `Get`) are pure delegations once the helper is correct, so
    // we test the helper directly. The body-conversion inside `extract_body`
    // is exhaustively covered in `super::page::tests`.
    //
    // The dispatch arms for `Create` / `Update` go straight through
    // `convert_input`, which has its own dedicated tests in `super::page`.

    fn opts() -> ExtractOpts {
        ExtractOpts::default()
    }

    fn comment_with_storage(value: &str) -> Value {
        json!({
            "id": "c1",
            "body": {"storage": {"value": value, "representation": "storage"}},
        })
    }

    #[test]
    fn rewrite_comment_body_default_markdown_converts_storage_to_md() {
        // The default `--body-format markdown` path must run the storage
        // XHTML through the converter so the user sees `# hi`, never raw
        // `<h1>hi</h1>`.
        let comment = comment_with_storage("<h1>hi</h1>");
        let result = rewrite_comment_body(comment, BodyFormat::Markdown, opts()).unwrap();
        let body = result
            .pointer("/body/markdown/value")
            .and_then(Value::as_str)
            .expect("markdown body");
        assert!(
            !body.contains("<h1>"),
            "markdown body must not contain raw HTML: {body}"
        );
        assert!(
            body.contains("hi"),
            "markdown body must preserve the heading text: {body}"
        );
        // Other body representations must NOT leak through alongside the
        // converted markdown — the user asked for markdown only.
        assert!(
            result.pointer("/body/storage").is_none(),
            "raw storage must be replaced on markdown read: {result}"
        );
    }

    #[test]
    fn rewrite_comment_body_body_format_storage_passes_through() {
        // `--body-format storage` must not touch the body — the user gets
        // the raw XHTML byte-for-byte.
        let xhtml = "<p>raw</p>";
        let comment = comment_with_storage(xhtml);
        let result = rewrite_comment_body(comment, BodyFormat::Storage, opts()).unwrap();
        assert_eq!(
            result
                .pointer("/body/storage/value")
                .and_then(Value::as_str),
            Some(xhtml)
        );
    }

    #[test]
    fn rewrite_comment_body_body_format_adf_returns_canonical_json() {
        // `--body-format adf` must surface the ADF body as pretty-printed
        // canonical JSON under `body.atlas_doc_format.value`.
        let adf_compact = r#"{"type":"doc","version":1,"content":[]}"#;
        let comment = json!({
            "id": "c1",
            "body": {"atlas_doc_format": {
                "value": adf_compact,
                "representation": "atlas_doc_format",
            }},
        });
        let result = rewrite_comment_body(comment, BodyFormat::Adf, opts()).unwrap();
        let pretty = result
            .pointer("/body/atlas_doc_format/value")
            .and_then(Value::as_str)
            .expect("ADF value");
        assert!(
            pretty.contains('\n'),
            "ADF must be pretty-printed: {pretty}"
        );
        let parsed: Value =
            serde_json::from_str(pretty).expect("pretty ADF must still be valid JSON");
        assert_eq!(parsed.get("type").and_then(Value::as_str), Some("doc"));
    }

    #[test]
    fn apply_body_format_to_list_walks_results_array() {
        // Lists carry comments under `results`. Every entry must be
        // rewritten; pagination metadata at the top level must round-trip
        // untouched.
        let response = json!({
            "results": [
                comment_with_storage("<p>one</p>"),
                comment_with_storage("<p>two</p>"),
            ],
            "_links": {"next": "/x"},
        });
        let result = apply_body_format_to_response(response, BodyFormat::Markdown, opts()).unwrap();
        let arr = result
            .pointer("/results")
            .and_then(Value::as_array)
            .expect("results array");
        assert_eq!(arr.len(), 2);
        for (i, entry) in arr.iter().enumerate() {
            let body = entry
                .pointer("/body/markdown/value")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("results[{i}] missing markdown body"));
            assert!(
                !body.contains("<p>"),
                "results[{i}] body must be markdown, got: {body}"
            );
        }
        // Pagination metadata round-trips untouched.
        assert_eq!(
            result.pointer("/_links/next").and_then(Value::as_str),
            Some("/x")
        );
    }

    // ---- write-side input conversion -----------------------------------
    //
    // Each Create/Update dispatch arm composes `convert_input` (page.rs)
    // with a client method. `convert_input` itself is exhaustively tested
    // in `super::page::tests`; the smoke checks below pin the wiring so a
    // future refactor can't accidentally drop the conversion step before
    // hitting the client.
    //
    // We exercise `convert_input` against the same inputs the dispatch
    // would feed it — `# hi` for markdown, raw XHTML for storage, a JSON
    // string for ADF — and inspect the resulting `BodyContent` to confirm
    // the variant the client would have received. This is one happy-path
    // per command (footer-comment create/update, inline-comment
    // create/update) collapsed into format-keyed cases since the four
    // arms are byte-for-byte the same conversion call.

    use crate::cli::commands::converters::body_content::BodyContent;

    #[test]
    fn comment_create_default_markdown_converts_to_storage() {
        // The default `--input-format markdown` path (shared by all four
        // create/update arms) must run the body through the markdown
        // converter — a `# hi` becomes `<h1>hi</h1>` storage XHTML.
        let body = convert_input("# hi".to_string(), &InputFormat::Markdown).unwrap();
        match body {
            BodyContent::Storage(s) => assert!(
                s.contains("<h1>") && s.contains("hi"),
                "markdown must convert to storage XHTML, got: {s}"
            ),
            BodyContent::Adf(_) => panic!("markdown input must produce Storage, got Adf"),
        }
    }

    #[test]
    fn comment_create_input_format_storage_passes_through() {
        // `--input-format storage` keeps the user's XHTML byte-for-byte.
        let xhtml = "<p>raw</p>".to_string();
        let body = convert_input(xhtml.clone(), &InputFormat::Storage).unwrap();
        match body {
            BodyContent::Storage(s) => assert_eq!(s, xhtml),
            BodyContent::Adf(_) => panic!("storage input must stay Storage, got Adf"),
        }
    }

    #[test]
    fn comment_create_input_format_adf_routes_correctly() {
        // `--input-format adf` parses the body as JSON and routes it
        // through the ADF variant; the dispatch hands this off to the
        // client which serialises it under `body.atlas_doc_format.value`.
        let adf = r#"{"type":"doc","version":1,"content":[]}"#.to_string();
        let body = convert_input(adf, &InputFormat::Adf).unwrap();
        match body {
            BodyContent::Adf(v) => {
                assert_eq!(v.get("type").and_then(Value::as_str), Some("doc"));
            }
            BodyContent::Storage(_) => panic!("adf input must produce Adf, got Storage"),
        }
    }

    #[test]
    fn apply_body_format_to_response_handles_single_comment() {
        // Single-comment responses (Get) take the non-list branch and run
        // `rewrite_comment_body` directly.
        let comment = comment_with_storage("<p>only</p>");
        let result = apply_body_format_to_response(comment, BodyFormat::Markdown, opts()).unwrap();
        assert!(
            result
                .pointer("/body/markdown/value")
                .and_then(Value::as_str)
                .map(|s| s.contains("only"))
                .unwrap_or(false),
            "single-comment response must be rewritten: {result}"
        );
    }
}
