use std::io::Write;

use serde_json::Value;
use tracing::info;

use crate::cli::args::*;
use crate::cli::commands::markdown;
use crate::client::ConfluenceClient;

/// Extract the body content string out of a Confluence page response, given
/// the requested body format. Returns `""` when the body is missing instead
/// of erroring — the caller writes the result verbatim, and an empty body is
/// a legal page state.
fn extract_body_content(page: &Value, body_format: BodyFormat) -> &str {
    let body_key = match body_format {
        BodyFormat::Storage => "storage",
        BodyFormat::View => "view",
    };
    page.pointer(&format!("/body/{body_key}/value"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Build the JSON summary returned by the `export` command. Pure so the test
/// can assert on the exact shape without spinning up the HTTP layer.
fn build_export_summary(
    page_id: &str,
    title: &str,
    attachments_downloaded: u32,
    output_dir: &str,
) -> Value {
    serde_json::json!({
        "page_id": page_id,
        "title": title,
        "attachments_downloaded": attachments_downloaded,
        "output_dir": output_dir,
    })
}

/// Pick a safe, collision-free filename for a downloaded attachment.
///
/// Sanitises the attachment title and, if a file with that name already
/// exists in `att_dir`, prepends the attachment ID to disambiguate.
/// `exists` is injected so tests can drive both branches without touching
/// the filesystem (production callers pass `|p| p.exists()`).
fn pick_attachment_filename(
    att_title: &str,
    att_id: &str,
    att_dir: &camino::Utf8Path,
    exists: impl Fn(&camino::Utf8Path) -> bool,
) -> String {
    let safe_name = sanitize_filename(att_title);
    let candidate = att_dir.join(&safe_name);
    if exists(&candidate) {
        format!("{att_id}_{safe_name}")
    } else {
        safe_name
    }
}

pub(super) async fn export_page(
    client: &ConfluenceClient,
    args: &ConfluenceExportArgs,
) -> anyhow::Result<Value> {
    let page = client
        .get_page(&args.page_id, args.body_format.as_str(), &[])
        .await?;
    let title = page
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("untitled");

    std::fs::create_dir_all(args.output_dir.as_std_path())?;

    let body_content = extract_body_content(&page, args.body_format);
    let page_file = args
        .output_dir
        .join(format!("{}.html", sanitize_filename(title)));
    std::fs::write(page_file.as_std_path(), body_content)?;
    info!("Wrote page content to {page_file}");

    let attachments = client.get_attachments_all(&args.page_id, 200).await?;
    let mut count = 0u32;
    if let Some(results) = attachments.get("results").and_then(Value::as_array)
        && !results.is_empty()
    {
        let att_dir = args.output_dir.join("attachments");
        std::fs::create_dir_all(att_dir.as_std_path())?;
        for att in results {
            let att_id = match att.get("id").and_then(Value::as_str) {
                Some(id) => id,
                None => continue,
            };
            let att_title = att
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let safe_name =
                pick_attachment_filename(att_title, att_id, &att_dir, |p| p.as_std_path().exists());
            let target = att_dir.join(&safe_name);
            info!("Downloading attachment: {att_title}");
            let bytes = client.download_attachment(&args.page_id, att_id).await?;
            std::fs::write(target.as_std_path(), &bytes)?;
            count += 1;
        }
    }

    Ok(build_export_summary(
        &args.page_id,
        title,
        count,
        args.output_dir.as_str(),
    ))
}

/// Build the per-page record emitted by `copy_tree`. Pure so we can verify
/// the exact shape (including `dry_run`/`new_id` branches) in unit tests.
fn build_copy_record(
    dry_run: bool,
    source_page_id: &str,
    title: &str,
    depth: u32,
    new_id: Option<&str>,
) -> Value {
    if dry_run {
        serde_json::json!({
            "action": "copy",
            "source_id": source_page_id,
            "title": title,
            "depth": depth,
            "dry_run": true,
        })
    } else {
        serde_json::json!({
            "action": "copied",
            "source_id": source_page_id,
            "new_id": new_id.unwrap_or(""),
            "title": title,
            "depth": depth,
        })
    }
}

/// Returns true if the given title matches the user's exclude glob. Centralised
/// so the recursion body stays readable and the rule has a single test point.
fn should_exclude(title: &str, exclude: Option<&glob::Pattern>) -> bool {
    exclude.is_some_and(|pattern| pattern.matches(title))
}

pub(super) async fn copy_tree(
    client: &ConfluenceClient,
    args: &ConfluenceCopyTreeArgs,
) -> anyhow::Result<Value> {
    let exclude_pattern = args
        .exclude
        .as_deref()
        .map(glob::Pattern::new)
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid exclude pattern: {e}"))?;

    let target_space = args
        .target_space
        .as_deref()
        .or(args.target_space_id.as_deref())
        .expect("clap enforces required_unless_present=target_space_id on ConfluenceCopyTreeArgs");
    let results = copy_tree_recursive(
        client,
        &args.source_page_id,
        target_space,
        args.target_parent.as_deref(),
        args.depth,
        args.dry_run,
        exclude_pattern.as_ref(),
        0,
    )
    .await?;

    Ok(Value::Array(results))
}

#[allow(clippy::too_many_arguments)]
fn copy_tree_recursive<'a>(
    client: &'a ConfluenceClient,
    source_page_id: &'a str,
    target_space: &'a str,
    target_parent: Option<&'a str>,
    depth: u32,
    dry_run: bool,
    exclude: Option<&'a glob::Pattern>,
    level: u32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<Value>>> + Send + 'a>> {
    Box::pin(async move {
        let page = client.get_page(source_page_id, "storage", &[]).await?;
        let title = page
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled");

        if should_exclude(title, exclude) {
            info!("Excluding page '{title}'");
            return Ok(vec![]);
        }

        let body = extract_body_content(&page, BodyFormat::Storage);

        let mut results = Vec::new();

        if dry_run {
            info!("[dry-run] Would copy page '{title}' (depth {level})");
            results.push(build_copy_record(true, source_page_id, title, level, None));
        } else {
            info!("Copying page '{title}' (depth {level})");
            let created = client
                .create_page(target_space, title, body, target_parent, false)
                .await?;
            let new_id = created.get("id").and_then(Value::as_str).unwrap_or("");
            results.push(build_copy_record(
                false,
                source_page_id,
                title,
                level,
                Some(new_id),
            ));
        }

        if depth > 0 {
            let children_resp = client.get_children(source_page_id, 200).await?;
            if let Some(children) = children_resp.get("results").and_then(Value::as_array) {
                let new_parent_id: Option<String> = if !dry_run {
                    results
                        .last()
                        .and_then(|r| r.get("new_id"))
                        .and_then(Value::as_str)
                        .map(String::from)
                } else {
                    None
                };
                for child in children {
                    let child_id = match child.get("id").and_then(Value::as_str) {
                        Some(id) => id,
                        None => continue,
                    };
                    let mut child_results = copy_tree_recursive(
                        client,
                        child_id,
                        target_space,
                        new_parent_id.as_deref(),
                        depth - 1,
                        dry_run,
                        exclude,
                        level + 1,
                    )
                    .await?;
                    results.append(&mut child_results);
                }
            }
        }

        Ok(results)
    })
}

pub(super) fn maybe_convert_markdown(body: String, input_format: &InputFormat) -> String {
    match input_format {
        InputFormat::Markdown => markdown::markdown_to_storage(&body),
        InputFormat::Storage => body,
    }
}

/// Windows reserved device names that cannot be used as filenames.
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn sanitize_filename(name: &str) -> String {
    // Use only the filename component (strip any path separators)
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let sanitized: String = base
        .chars()
        .map(|c| match c {
            '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect();

    // Prevent names like "." or ".." that could escape the directory
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return "_".to_string();
    }

    // Strip trailing dots and spaces (Windows cannot create such files)
    let sanitized = sanitized.trim_end_matches(['.', ' ']);
    if sanitized.is_empty() {
        return "_".to_string();
    }

    // Check if the stem (name without extension) is a Windows reserved name
    let stem = match sanitized.find('.') {
        Some(pos) => &sanitized[..pos],
        None => sanitized,
    };
    if WINDOWS_RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(stem))
    {
        return format!("_{sanitized}");
    }

    sanitized.to_string()
}

pub(super) fn render_tree(
    node: &Value,
    indent: usize,
    is_last: bool,
    writer: &mut dyn Write,
) -> anyhow::Result<()> {
    let title = node.get("title").and_then(Value::as_str).unwrap_or("?");
    let id = node.get("id").and_then(Value::as_str).unwrap_or("");

    if indent == 0 {
        writeln!(writer, "{title} ({id})")?;
    } else {
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let prefix = "    ".repeat(indent.saturating_sub(1));
        writeln!(writer, "{prefix}{connector}{title} ({id})")?;
    }

    if let Some(children) = node.get("_children").and_then(Value::as_array) {
        for (i, child) in children.iter().enumerate() {
            let child_is_last = i == children.len() - 1;
            render_tree(child, indent + 1, child_is_last, writer)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_normal_name() {
        assert_eq!(sanitize_filename("hello.txt"), "hello.txt");
    }

    #[test]
    fn sanitize_replaces_illegal_chars() {
        assert_eq!(sanitize_filename("a<b>c:d"), "a_b_c_d");
    }

    #[test]
    fn sanitize_empty_and_dots() {
        assert_eq!(sanitize_filename(""), "_");
        assert_eq!(sanitize_filename("."), "_");
        assert_eq!(sanitize_filename(".."), "_");
    }

    #[test]
    fn sanitize_strips_trailing_dots_and_spaces() {
        assert_eq!(sanitize_filename("file. ."), "file");
        assert_eq!(sanitize_filename("test..."), "test");
        assert_eq!(sanitize_filename("doc   "), "doc");
    }

    #[test]
    fn sanitize_windows_reserved_names() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("con"), "_con");
        assert_eq!(sanitize_filename("NUL.txt"), "_NUL.txt");
        assert_eq!(sanitize_filename("COM1"), "_COM1");
        assert_eq!(sanitize_filename("lpt3.log"), "_lpt3.log");
    }

    #[test]
    fn sanitize_non_reserved_with_reserved_substring() {
        // "CONNECT" starts with "CON" but the stem is "CONNECT", not "CON"
        assert_eq!(sanitize_filename("CONNECT.txt"), "CONNECT.txt");
    }

    #[test]
    fn sanitize_path_separator_stripping() {
        assert_eq!(sanitize_filename("foo/bar.txt"), "bar.txt");
        assert_eq!(sanitize_filename("foo\\bar.txt"), "bar.txt");
    }

    // ---- render_tree ----

    #[test]
    fn render_tree_single_root() {
        let node = serde_json::json!({"title": "Root", "id": "1"});
        let mut buf = Vec::new();
        render_tree(&node, 0, true, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output, "Root (1)\n",
            "single root should render title and id on one line"
        );
    }

    #[test]
    fn render_tree_with_children() {
        let node = serde_json::json!({
            "title": "Parent", "id": "1",
            "_children": [
                {"title": "Child A", "id": "2"},
                {"title": "Child B", "id": "3"}
            ]
        });
        let mut buf = Vec::new();
        render_tree(&node, 0, true, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 lines (parent + 2 children)");
        assert_eq!(lines[0], "Parent (1)");
        assert!(
            lines[1].contains('\u{251c}'),
            "first child should use branch connector, got: {:?}",
            lines[1]
        );
        assert!(
            lines[1].contains("Child A (2)"),
            "first child line should contain title and id, got: {:?}",
            lines[1]
        );
        assert!(
            lines[2].contains('\u{2514}'),
            "last child should use corner connector, got: {:?}",
            lines[2]
        );
        assert!(
            lines[2].contains("Child B (3)"),
            "last child line should contain title and id, got: {:?}",
            lines[2]
        );
    }

    // ---- maybe_convert_markdown ----

    #[test]
    fn maybe_convert_markdown_storage_passthrough() {
        // Storage-format input must be returned byte-for-byte so the user's
        // hand-written XHTML reaches Confluence as-is.
        let body = "<p>already storage</p>".to_string();
        let result = maybe_convert_markdown(body.clone(), &InputFormat::Storage);
        assert_eq!(
            result, body,
            "storage input must pass through unchanged, got: {result:?}"
        );
    }

    #[test]
    fn maybe_convert_markdown_markdown_converts_heading() {
        // Markdown input must run through the converter — the cheapest signal
        // that conversion happened is the presence of the storage `<h1>` tag.
        let result = maybe_convert_markdown("# Hi".to_string(), &InputFormat::Markdown);
        assert!(
            result.contains("<h1>"),
            "expected <h1> after markdown conversion, got: {result:?}"
        );
        assert!(
            !result.starts_with("# "),
            "markdown heading prefix must be replaced, got: {result:?}"
        );
    }

    #[test]
    fn maybe_convert_markdown_empty_body_is_safe() {
        // Empty body is legal (e.g. user passes `--body ""` with `--input-format markdown`).
        // Must not panic on either path.
        let storage = maybe_convert_markdown(String::new(), &InputFormat::Storage);
        assert_eq!(storage, "", "empty storage body should pass through");

        // Markdown converter may emit nothing or trivial whitespace; just
        // require it doesn't panic.
        let _ = maybe_convert_markdown(String::new(), &InputFormat::Markdown);
    }

    // ---- extract_body_content ----

    #[test]
    fn extract_body_storage_path() {
        let page = serde_json::json!({
            "body": {"storage": {"value": "<p>x</p>", "representation": "storage"}}
        });
        assert_eq!(extract_body_content(&page, BodyFormat::Storage), "<p>x</p>");
    }

    #[test]
    fn extract_body_view_path() {
        let page = serde_json::json!({
            "body": {"view": {"value": "<p>rendered</p>", "representation": "view"}}
        });
        assert_eq!(
            extract_body_content(&page, BodyFormat::View),
            "<p>rendered</p>"
        );
    }

    #[test]
    fn extract_body_missing_key_yields_empty_string() {
        let page = serde_json::json!({"body": {"view": {"value": "x"}}});
        // Asking for storage when only view exists must return "" rather than
        // panicking; the caller writes the empty string to disk.
        assert_eq!(extract_body_content(&page, BodyFormat::Storage), "");
    }

    #[test]
    fn extract_body_missing_body_object_yields_empty_string() {
        let page = serde_json::json!({"id": "1"});
        assert_eq!(extract_body_content(&page, BodyFormat::Storage), "");
    }

    // ---- build_export_summary ----

    #[test]
    fn build_export_summary_shape() {
        let v = build_export_summary("123", "My Page", 4, "/tmp/out");
        assert_eq!(
            v,
            serde_json::json!({
                "page_id": "123",
                "title": "My Page",
                "attachments_downloaded": 4,
                "output_dir": "/tmp/out"
            }),
            "summary JSON must match the documented shape"
        );
    }

    #[test]
    fn build_export_summary_zero_attachments() {
        let v = build_export_summary("1", "T", 0, ".");
        assert_eq!(v["attachments_downloaded"], 0);
    }

    // ---- build_copy_record ----

    #[test]
    fn build_copy_record_dry_run_shape() {
        let v = build_copy_record(true, "src1", "Title", 2, None);
        assert_eq!(v["action"], "copy");
        assert_eq!(v["source_id"], "src1");
        assert_eq!(v["title"], "Title");
        assert_eq!(v["depth"], 2);
        assert_eq!(v["dry_run"], true);
        assert!(
            v.get("new_id").is_none(),
            "dry-run record must not include new_id"
        );
    }

    #[test]
    fn build_copy_record_real_shape() {
        let v = build_copy_record(false, "src1", "Title", 0, Some("dest42"));
        assert_eq!(v["action"], "copied");
        assert_eq!(v["new_id"], "dest42");
        assert!(
            v.get("dry_run").is_none(),
            "real copy must not include dry_run flag"
        );
    }

    #[test]
    fn build_copy_record_real_with_missing_new_id_uses_empty() {
        // The copy_tree caller passes Some("") rather than None when the
        // server omits id; the helper still produces a well-formed record.
        let v = build_copy_record(false, "src", "T", 1, None);
        assert_eq!(v["new_id"], "");
    }

    // ---- should_exclude ----

    #[test]
    fn should_exclude_no_pattern_never_excludes() {
        assert!(!should_exclude("anything", None));
    }

    #[test]
    fn should_exclude_pattern_matches() {
        let p = glob::Pattern::new("Archive*").unwrap();
        assert!(should_exclude("Archive 2023", Some(&p)));
    }

    #[test]
    fn should_exclude_pattern_does_not_match() {
        let p = glob::Pattern::new("Archive*").unwrap();
        assert!(!should_exclude("Active Project", Some(&p)));
    }

    #[test]
    fn should_exclude_glob_question_mark() {
        // Confirms standard glob semantics — `?` matches exactly one char.
        let p = glob::Pattern::new("page-?").unwrap();
        assert!(should_exclude("page-a", Some(&p)));
        assert!(!should_exclude("page-ab", Some(&p)));
    }

    // ---- pick_attachment_filename ----

    #[test]
    fn pick_attachment_filename_no_collision_uses_sanitised_title() {
        // Inject a probe that says nothing exists — the helper must return the
        // bare sanitised title.
        let dir = camino::Utf8PathBuf::from("/tmp/att");
        let name = pick_attachment_filename("hello.png", "9999", &dir, |_| false);
        assert_eq!(name, "hello.png");
    }

    #[test]
    fn pick_attachment_filename_collision_prepends_id() {
        // When the candidate already exists, the helper must prefix the
        // attachment ID so the second download does not overwrite the first.
        let dir = camino::Utf8PathBuf::from("/tmp/att");
        let name = pick_attachment_filename("hello.png", "9999", &dir, |_| true);
        assert_eq!(name, "9999_hello.png");
    }

    #[test]
    fn pick_attachment_filename_sanitises_illegal_chars_first() {
        // Sanitisation must happen before the existence check so the probe
        // sees a path the OS could actually create.
        let dir = camino::Utf8PathBuf::from("/tmp/att");
        let name = pick_attachment_filename("a:b/c.png", "1", &dir, |_| false);
        assert_eq!(name, "c.png", "path separator should reduce to basename");
    }

    #[test]
    fn pick_attachment_filename_collision_check_uses_sanitised_path() {
        // Confirm the probe is called with the sanitised candidate, not the
        // raw title — otherwise the existence check would never fire on
        // titles that contain illegal characters.
        let dir = camino::Utf8PathBuf::from("/tmp/att");
        let probed = std::cell::RefCell::new(Vec::new());
        let _ = pick_attachment_filename("file<x>.png", "42", &dir, |p| {
            probed.borrow_mut().push(p.to_string());
            false
        });
        assert_eq!(probed.borrow().len(), 1);
        // The path must contain the sanitised (`_` replacement) form.
        assert!(
            probed.borrow()[0].contains("file_x_.png"),
            "probe path should be sanitised, got: {:?}",
            probed.borrow()
        );
    }

    #[test]
    fn pick_attachment_filename_collision_with_real_filesystem() {
        // End-to-end: create a real file in a tempdir and confirm the
        // production-style probe (`p.as_std_path().exists()`) triggers the
        // collision branch. Guards against drift between the helper and the
        // closure the caller actually passes.
        let td = tempfile::tempdir().expect("create tempdir");
        let dir = camino::Utf8PathBuf::try_from(td.path().to_path_buf()).expect("UTF-8 temp path");
        let existing = dir.join("doc.txt");
        std::fs::write(existing.as_std_path(), "x").expect("seed file");

        let name = pick_attachment_filename("doc.txt", "id7", &dir, |p| p.as_std_path().exists());
        assert_eq!(name, "id7_doc.txt");

        // And when the file is absent, it stays as just the sanitised name.
        let name = pick_attachment_filename("other.txt", "id8", &dir, |p| p.as_std_path().exists());
        assert_eq!(name, "other.txt");
    }

    #[test]
    fn render_tree_nested() {
        let node = serde_json::json!({
            "title": "Root", "id": "1",
            "_children": [{
                "title": "Level1", "id": "2",
                "_children": [{
                    "title": "Level2", "id": "3"
                }]
            }]
        });
        let mut buf = Vec::new();
        render_tree(&node, 0, true, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 lines (root + level1 + level2)");
        assert_eq!(lines[0], "Root (1)");
        assert!(
            lines[1].contains("Level1 (2)"),
            "level-1 node should appear, got: {:?}",
            lines[1]
        );
        assert!(
            lines[2].contains("Level2 (3)"),
            "level-2 node should appear, got: {:?}",
            lines[2]
        );
        // Level 2 should be indented further than level 1.
        let indent_l1 = lines[1].find('L').unwrap_or(0);
        let indent_l2 = lines[2].find('L').unwrap_or(0);
        assert!(
            indent_l2 > indent_l1,
            "level-2 indent ({indent_l2}) should exceed level-1 indent ({indent_l1})"
        );
    }
}
