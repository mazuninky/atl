use std::io::Write;

use serde_json::Value;
use tracing::info;

use crate::cli::args::*;
use crate::cli::commands::markdown;
use crate::client::ConfluenceClient;

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

    let body_key = match args.body_format {
        BodyFormat::Storage => "storage",
        BodyFormat::View => "view",
    };
    let body_content = page
        .pointer(&format!("/body/{body_key}/value"))
        .and_then(Value::as_str)
        .unwrap_or("");
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
            let mut safe_name = sanitize_filename(att_title);
            let mut target = att_dir.join(&safe_name);
            if target.as_std_path().exists() {
                safe_name = format!("{att_id}_{safe_name}");
                target = att_dir.join(&safe_name);
            }
            info!("Downloading attachment: {att_title}");
            let bytes = client.download_attachment(&args.page_id, att_id).await?;
            std::fs::write(target.as_std_path(), &bytes)?;
            count += 1;
        }
    }

    Ok(serde_json::json!({
        "page_id": args.page_id,
        "title": title,
        "attachments_downloaded": count,
        "output_dir": args.output_dir.as_str()
    }))
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

        if let Some(pattern) = exclude
            && pattern.matches(title)
        {
            info!("Excluding page '{title}'");
            return Ok(vec![]);
        }

        let body = page
            .pointer("/body/storage/value")
            .and_then(Value::as_str)
            .unwrap_or("");

        let mut results = Vec::new();

        if dry_run {
            info!("[dry-run] Would copy page '{title}' (depth {level})");
            results.push(serde_json::json!({
                "action": "copy",
                "source_id": source_page_id,
                "title": title,
                "depth": level,
                "dry_run": true,
            }));
        } else {
            info!("Copying page '{title}' (depth {level})");
            let created = client
                .create_page(target_space, title, body, target_parent, false)
                .await?;
            let new_id = created
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            results.push(serde_json::json!({
                "action": "copied",
                "source_id": source_page_id,
                "new_id": new_id,
                "title": title,
                "depth": level,
            }));
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
}
