//! Atlassian Document Format (ADF) JSON → markdown (with MyST-style directive
//! extensions).
//!
//! ADF is the JSON tree format used by Confluence Cloud and Jira Cloud for
//! rich text content. A document looks like
//! `{"type": "doc", "version": 1, "content": [/* block nodes */]}`. Block
//! nodes contain inline nodes; inline nodes carry an optional `marks` array
//! for formatting (`strong`, `em`, `code`, `strike`, `link`, `underline`,
//! `subsup`).
//!
//! # Conversion strategy
//!
//! 1. **Normalise the input.** Accept either a top-level `doc` node, a bare
//!    content array, or a single block node, and reduce to a `&[Value]` slice.
//! 2. **Recursive walker.** Each block dispatches on `"type"` and emits
//!    markdown into a buffer. Inline nodes are rendered into a single-line
//!    string and folded into the surrounding block.
//! 3. **Marks.** Text nodes apply marks in a fixed outermost-to-innermost
//!    order (`link → strong → em → strike → underline → code`) so that the
//!    output is stable across runs.
//!
//! # Lossy mappings
//!
//! - **Panel `error`** — ADF supports `panelType: error` but markdown only
//!   has `:::warning`, so error panels collapse to `:::warning`.
//! - **Panel title** — ADF panels have no title attribute; the
//!   `md_to_adf` converter prepends a strong-marked paragraph as a title.
//!   On the way back we detect that exact shape (first child = paragraph
//!   with a single strong-marked text node) and lift it into a
//!   `title="..."` directive parameter. Anything else is treated as body.
//! - **`underline` / `subsup`** — emitted as raw HTML (`<u>`, `<sub>`,
//!   `<sup>`) since CommonMark has no native syntax.
//! - **`inlineCard`** — synthetic `pageId:N` URLs (the marker
//!   `md_to_adf` writes when the user supplied `:link{pageId=N}`) are
//!   reversed into `:link[]{pageId=N}`. Anything else becomes
//!   `:link[]{url=...}`.
//! - **Unknown node types** are surfaced as `<!-- adf:unknown {…} -->`
//!   comments so the caller can see what was dropped.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;
use thiserror::Error;

use super::code_fence::pick_code_fence;
use crate::cli::commands::directives::render_attrs;

// =====================================================================
// Public API
// =====================================================================

/// Errors returned by [`adf_to_markdown`].
#[derive(Debug, Error)]
pub enum AdfToMdError {
    /// The input was not a valid ADF document, content array, or block node.
    #[error("malformed ADF: {0}")]
    Malformed(String),
}

/// Conversion options for [`adf_to_markdown`].
///
/// `render_directives` controls whether ADF panels, expand wrappers and
/// inline status / mention / emoji / inlineCard nodes become MyST-style
/// directives (`true`, the default). When `false`, panel and expand wrappers
/// are flattened to body content; status / emoji / mention / inlineCard
/// inline nodes collapse to their display text only.
#[derive(Debug, Clone, Copy)]
pub struct ConvertOpts {
    /// When `true` (the default), recognised nodes are converted to directive
    /// syntax. When `false`, directives are stripped to their plain bodies.
    pub render_directives: bool,
}

impl Default for ConvertOpts {
    fn default() -> Self {
        Self {
            render_directives: true,
        }
    }
}

/// Convert an ADF JSON document to markdown with MyST-style directives.
///
/// Accepts either a top-level doc node (`{"type":"doc","version":1,"content":[...]}`)
/// or a bare content array. Unknown node types fall through as paragraph
/// passthroughs containing a JSON-comment placeholder so the caller can spot
/// them.
///
/// Returns an error only when the input is not a recognisable ADF shape
/// (e.g. raw `null`).
///
/// # Examples
///
/// ```ignore
/// use atl::cli::commands::converters::adf_to_md::{adf_to_markdown, ConvertOpts};
///
/// let adf = serde_json::json!({
///     "type": "doc",
///     "version": 1,
///     "content": [{
///         "type": "paragraph",
///         "content": [{"type": "text", "text": "hi"}],
///     }],
/// });
/// let md = adf_to_markdown(&adf, ConvertOpts::default()).unwrap();
/// assert_eq!(md.trim(), "hi");
/// ```
pub fn adf_to_markdown(adf: &Value, opts: ConvertOpts) -> Result<String, AdfToMdError> {
    let content = normalise_input(adf)?;
    let mut ctx = Ctx { opts };
    let mut out = String::new();
    render_blocks(&content, &mut out, &mut ctx);
    Ok(normalize_blank_lines(&out))
}

// =====================================================================
// Input normalisation
// =====================================================================

/// Reduce the various accepted input shapes to a single `Vec<&Value>` of
/// block nodes.
fn normalise_input(adf: &Value) -> Result<Vec<Value>, AdfToMdError> {
    match adf {
        Value::Null => Ok(Vec::new()),
        Value::Array(arr) => Ok(arr.clone()),
        Value::Object(map) => {
            // Must have a "type" field for an object to be ADF.
            let Some(t) = map.get("type").and_then(Value::as_str) else {
                return Err(AdfToMdError::Malformed(
                    "object without `type` field".to_string(),
                ));
            };
            if t == "doc" {
                let content = map
                    .get("content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                Ok(content)
            } else {
                // Single bare block node — wrap in a vec.
                Ok(vec![adf.clone()])
            }
        }
        _ => Err(AdfToMdError::Malformed(format!(
            "expected object or array, got {adf}"
        ))),
    }
}

// =====================================================================
// Walker state
// =====================================================================

#[derive(Debug, Clone, Copy)]
struct Ctx {
    opts: ConvertOpts,
}

// =====================================================================
// Block rendering
// =====================================================================

fn render_blocks(content: &[Value], out: &mut String, ctx: &mut Ctx) {
    for node in content {
        render_block(node, out, ctx);
    }
}

fn render_block(node: &Value, out: &mut String, ctx: &mut Ctx) {
    let Some(ty) = node.get("type").and_then(Value::as_str) else {
        return;
    };
    let content_opt = node.get("content").and_then(Value::as_array);
    let content: &[Value] = content_opt.map(Vec::as_slice).unwrap_or(&[]);
    match ty {
        "paragraph" => {
            let inline = render_inline_content(content, ctx);
            push_block(out, &inline);
        }
        "heading" => {
            let level = node
                .get("attrs")
                .and_then(|a| a.get("level"))
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            let inline = render_inline_content(content, ctx);
            ensure_blank_line(out);
            for _ in 0..level {
                out.push('#');
            }
            out.push(' ');
            out.push_str(inline.trim());
            out.push_str("\n\n");
        }
        "bulletList" => {
            ensure_blank_line(out);
            render_list(content, false, 0, out, ctx);
            out.push('\n');
        }
        "orderedList" => {
            ensure_blank_line(out);
            render_list(content, true, 0, out, ctx);
            out.push('\n');
        }
        "codeBlock" => {
            let language = node
                .get("attrs")
                .and_then(|a| a.get("language"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let body = collect_code_text(content);
            // Pick a fence long enough to safely wrap any backtick run in the
            // body — CommonMark §4.5 closes the block on the first run of >=
            // fence-length backticks, so plain ``` would be unsafe when the
            // body contains triple-or-more backticks.
            let fence = pick_code_fence(&body);
            ensure_blank_line(out);
            out.push_str(&fence);
            out.push_str(language);
            out.push('\n');
            out.push_str(&body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&fence);
            out.push_str("\n\n");
        }
        "blockquote" => {
            let mut inner = String::new();
            render_blocks(content, &mut inner, ctx);
            ensure_blank_line(out);
            for line in inner.trim_end_matches('\n').split('\n') {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
        "rule" => {
            ensure_blank_line(out);
            out.push_str("---\n\n");
        }
        "table" => {
            ensure_blank_line(out);
            render_table(content, out, ctx);
            out.push('\n');
        }
        "panel" => render_panel(node, content, out, ctx),
        "expand" => render_expand(node, content, out, ctx),
        "extension" => render_extension(node, out, ctx),
        "mediaSingle" | "mediaGroup" => render_media_block(content, out),
        // Inline-only nodes that occasionally show up at block level — drop or
        // fold into a paragraph so we don't crash.
        "hardBreak" => {
            out.push_str("  \n");
        }
        _ => emit_unknown_block(node, out),
    }
}

// =====================================================================
// Lists
// =====================================================================

/// Render a list. `ordered` controls the marker (`- ` vs `1. `). `depth` is
/// the nesting depth (0 = outermost).
fn render_list(items: &[Value], ordered: bool, depth: usize, out: &mut String, ctx: &mut Ctx) {
    let indent = "  ".repeat(depth);
    let marker = if ordered { "1. " } else { "- " };
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("listItem") {
            continue;
        }
        let kids = item.get("content").and_then(Value::as_array);
        let Some(kids) = kids else {
            out.push_str(&indent);
            out.push_str(marker);
            out.push('\n');
            continue;
        };

        // First block: paragraph rendered inline; or fallback for non-paragraph.
        let mut iter = kids.iter();
        let first_inline = match iter.next() {
            Some(first) if first.get("type").and_then(Value::as_str) == Some("paragraph") => {
                let inner: &[Value] = first
                    .get("content")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                render_inline_content(inner, ctx)
            }
            Some(first) => {
                // Non-paragraph first child — render as block into a buffer
                // and fold it onto the marker line.
                let mut buf = String::new();
                render_block(first, &mut buf, ctx);
                buf.trim().to_string()
            }
            None => String::new(),
        };

        out.push_str(&indent);
        out.push_str(marker);
        out.push_str(first_inline.trim_end());
        out.push('\n');

        // Subsequent blocks: nested lists at depth+1, others indented.
        for child in iter {
            let cty = child.get("type").and_then(Value::as_str).unwrap_or("");
            if cty == "bulletList" || cty == "orderedList" {
                let nested_items: &[Value] = child
                    .get("content")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                render_list(nested_items, cty == "orderedList", depth + 1, out, ctx);
            } else if cty == "paragraph" {
                let inner: &[Value] = child
                    .get("content")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let inline = render_inline_content(inner, ctx);
                if !inline.trim().is_empty() {
                    out.push_str(&indent);
                    out.push_str("  ");
                    out.push_str(inline.trim());
                    out.push('\n');
                }
            } else {
                let mut buf = String::new();
                render_block(child, &mut buf, ctx);
                for line in buf.trim_end_matches('\n').split('\n') {
                    if line.is_empty() {
                        continue;
                    }
                    out.push_str(&indent);
                    out.push_str("  ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
    }
}

// =====================================================================
// Tables
// =====================================================================

fn render_table(rows: &[Value], out: &mut String, ctx: &mut Ctx) {
    if rows.is_empty() {
        return;
    }
    // Detect header row: any row whose cells contain at least one tableHeader.
    let row_is_header = |row: &Value| -> bool {
        row.get("content")
            .and_then(Value::as_array)
            .map(|cells| {
                cells
                    .iter()
                    .any(|c| c.get("type").and_then(Value::as_str) == Some("tableHeader"))
            })
            .unwrap_or(false)
    };

    // Render each row's cells to inline strings.
    let render_row = |row: &Value, ctx: &mut Ctx| -> Vec<String> {
        let Some(cells) = row.get("content").and_then(Value::as_array) else {
            return Vec::new();
        };
        cells
            .iter()
            .map(|c| {
                // Each cell holds block content (typically a paragraph). Render
                // each block into a string and join with `<br/>` for multi-line
                // — but for v1 keep it simple: render the first paragraph's
                // inline content, falling back to all-block render trimmed.
                if let Some(kids) = c.get("content").and_then(Value::as_array) {
                    let mut buf = String::new();
                    for k in kids {
                        if k.get("type").and_then(Value::as_str) == Some("paragraph") {
                            let inner: &[Value] = k
                                .get("content")
                                .and_then(Value::as_array)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]);
                            let inline = render_inline_content(inner, ctx);
                            if !buf.is_empty() {
                                buf.push(' ');
                            }
                            buf.push_str(inline.trim());
                        } else {
                            let mut tmp = String::new();
                            render_block(k, &mut tmp, ctx);
                            let t = tmp.trim();
                            if !t.is_empty() {
                                if !buf.is_empty() {
                                    buf.push(' ');
                                }
                                buf.push_str(t);
                            }
                        }
                    }
                    buf
                } else {
                    String::new()
                }
            })
            .collect()
    };

    let mut rows_iter = rows.iter();
    let first = rows_iter.next();
    let (header_cells, body_rows): (Vec<String>, Vec<Vec<String>>) = match first {
        Some(first_row) if row_is_header(first_row) => {
            let header = render_row(first_row, ctx);
            let body: Vec<Vec<String>> = rows_iter.map(|r| render_row(r, ctx)).collect();
            (header, body)
        }
        Some(first_row) => {
            // No header row — synthesise empty header from first row's column count.
            let body: Vec<Vec<String>> = std::iter::once(first_row)
                .chain(rows_iter)
                .map(|r| render_row(r, ctx))
                .collect();
            let cols = body.first().map(Vec::len).unwrap_or(0);
            (vec![String::new(); cols], body)
        }
        None => return,
    };

    let cols = header_cells
        .len()
        .max(body_rows.iter().map(Vec::len).max().unwrap_or(0));
    if cols == 0 {
        return;
    }

    let mut header = header_cells;
    header.resize(cols, String::new());
    out.push('|');
    for cell in &header {
        out.push(' ');
        out.push_str(&escape_table_cell(cell));
        out.push_str(" |");
    }
    out.push('\n');
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    out.push('\n');

    for mut row in body_rows {
        row.resize(cols, String::new());
        out.push('|');
        for cell in &row {
            out.push(' ');
            out.push_str(&escape_table_cell(cell));
            out.push_str(" |");
        }
        out.push('\n');
    }
    out.push('\n');
}

/// Escape characters that would break a GFM table cell.
///
/// A literal `|` inside a cell would be parsed as a column separator; the GFM
/// convention is to backslash-escape it. We don't touch other characters
/// because the cell content is already inline-rendered markdown, where pipes
/// are the only context-sensitive character at table-cell granularity.
fn escape_table_cell(cell: &str) -> String {
    cell.replace('|', "\\|")
}

// =====================================================================
// Panels / expand / extensions
// =====================================================================

fn render_panel(node: &Value, body: &[Value], out: &mut String, ctx: &mut Ctx) {
    let panel_type = node
        .get("attrs")
        .and_then(|a| a.get("panelType"))
        .and_then(Value::as_str)
        .unwrap_or("info");

    // Map ADF panelType to our directive name. `error` falls back to
    // `warning` because there's no `:::error` directive yet.
    let directive_name = match panel_type {
        "info" => "info",
        "warning" | "error" => "warning",
        "note" => "note",
        "success" => "tip",
        _ => "info",
    };

    if !ctx.opts.render_directives {
        // Strip wrapper; render body as plain blocks.
        render_blocks(body, out, ctx);
        return;
    }

    // Detect title-paragraph: first child is a paragraph whose only inline
    // child is a strong-marked text node.
    let (title, real_body): (Option<String>, &[Value]) = match body.split_first() {
        Some((first, rest)) if is_strong_only_paragraph(first) => {
            let title = first
                .get("content")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|t| t.get("text"))
                .and_then(Value::as_str)
                .map(str::to_string);
            (title, rest)
        }
        _ => (None, body),
    };

    let mut params: BTreeMap<String, String> = BTreeMap::new();
    if let Some(t) = title
        && !t.is_empty()
    {
        params.insert("title".to_string(), t);
    }

    // Render body into its own buffer first.
    let mut body_buf = String::new();
    render_blocks(real_body, &mut body_buf, ctx);
    let body_md = body_buf.trim_end_matches('\n').to_string();

    ensure_blank_line(out);
    out.push_str(":::");
    out.push_str(directive_name);
    if !params.is_empty() {
        out.push(' ');
        out.push_str(&render_attrs(&params));
    }
    out.push('\n');
    if !body_md.is_empty() {
        out.push_str(&body_md);
        out.push('\n');
    }
    out.push_str(":::\n\n");
}

/// Test whether a node is `paragraph` whose only inline child is a `text`
/// node carrying a `strong` mark and nothing else.
fn is_strong_only_paragraph(node: &Value) -> bool {
    if node.get("type").and_then(Value::as_str) != Some("paragraph") {
        return false;
    }
    let Some(content) = node.get("content").and_then(Value::as_array) else {
        return false;
    };
    if content.len() != 1 {
        return false;
    }
    let only = &content[0];
    if only.get("type").and_then(Value::as_str) != Some("text") {
        return false;
    }
    let Some(marks) = only.get("marks").and_then(Value::as_array) else {
        return false;
    };
    if marks.len() != 1 {
        return false;
    }
    marks[0].get("type").and_then(Value::as_str) == Some("strong")
}

fn render_expand(node: &Value, body: &[Value], out: &mut String, ctx: &mut Ctx) {
    let title = node
        .get("attrs")
        .and_then(|a| a.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("");

    if !ctx.opts.render_directives {
        // Strip the wrapper, but preserve the title as a bold paragraph so
        // it isn't silently dropped. ADF panels stash their title as a
        // strong-marked first paragraph inside the panel body — those
        // already survive a `--no-directives` round-trip naturally because
        // we just render the body. ADF `expand`, by contrast, keeps the
        // title on the node's `attrs.title` field (no body paragraph), so
        // we have to materialise it here for parity with the panel path.
        if !title.is_empty() {
            ensure_blank_line(out);
            out.push_str("**");
            out.push_str(title);
            out.push_str("**\n\n");
        }
        render_blocks(body, out, ctx);
        return;
    }
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    if !title.is_empty() {
        params.insert("title".to_string(), title.to_string());
    }

    let mut body_buf = String::new();
    render_blocks(body, &mut body_buf, ctx);
    let body_md = body_buf.trim_end_matches('\n').to_string();

    ensure_blank_line(out);
    out.push_str(":::expand");
    if !params.is_empty() {
        out.push(' ');
        out.push_str(&render_attrs(&params));
    }
    out.push('\n');
    if !body_md.is_empty() {
        out.push_str(&body_md);
        out.push('\n');
    }
    out.push_str(":::\n\n");
}

fn render_extension(node: &Value, out: &mut String, ctx: &mut Ctx) {
    let attrs = node.get("attrs");
    let ext_key = attrs
        .and_then(|a| a.get("extensionKey"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let ext_type = attrs
        .and_then(|a| a.get("extensionType"))
        .and_then(Value::as_str)
        .unwrap_or("");

    if !ctx.opts.render_directives {
        // Without directives, an extension is opaque — emit nothing.
        return;
    }

    if ext_key == "toc" && ext_type == "com.atlassian.confluence.macro.core" {
        let max_level = attrs
            .and_then(|a| a.get("parameters"))
            .and_then(|p| p.get("macroParams"))
            .and_then(|m| m.get("maxLevel"))
            .and_then(|m| m.get("value"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut params: BTreeMap<String, String> = BTreeMap::new();
        if let Some(ml) = max_level {
            params.insert("maxLevel".to_string(), ml);
        }
        ensure_blank_line(out);
        out.push_str(":::toc");
        if !params.is_empty() {
            out.push(' ');
            out.push_str(&render_attrs(&params));
        }
        out.push_str("\n:::\n\n");
        return;
    }

    emit_unknown_block(node, out);
}

// =====================================================================
// Media
// =====================================================================

fn render_media_block(content: &[Value], out: &mut String) {
    for media in content {
        let ty = media.get("type").and_then(Value::as_str).unwrap_or("");
        if ty != "media" {
            continue;
        }
        let attrs = media.get("attrs");
        let url = attrs
            .and_then(|a| a.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let alt = attrs
            .and_then(|a| a.get("alt"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let id = attrs
            .and_then(|a| a.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);

        ensure_blank_line(out);
        match (url, id) {
            (Some(u), _) => {
                let _ = write!(out, "![{alt}]({u})");
            }
            (None, Some(i)) => {
                let _ = write!(out, "![{alt}](attachment:{i})");
            }
            _ => {
                out.push_str("![](");
                out.push(')');
            }
        }
        out.push_str("\n\n");
    }
}

// =====================================================================
// Inline rendering
// =====================================================================

fn render_inline_content(content: &[Value], ctx: &Ctx) -> String {
    let mut buf = String::new();
    for node in content {
        render_inline_node(node, &mut buf, ctx);
    }
    buf
}

fn render_inline_node(node: &Value, out: &mut String, ctx: &Ctx) {
    let Some(ty) = node.get("type").and_then(Value::as_str) else {
        return;
    };
    match ty {
        "text" => {
            let text = node.get("text").and_then(Value::as_str).unwrap_or("");
            let marks = node.get("marks").and_then(Value::as_array);
            let rendered = apply_marks(text, marks);
            out.push_str(&rendered);
        }
        "hardBreak" => out.push_str("  \n"),
        "mention" => render_mention(node, out, ctx),
        "emoji" => render_emoji(node, out, ctx),
        "inlineCard" => render_inline_card(node, out, ctx),
        "status" => render_status(node, out, ctx),
        "media" => {
            let url = node
                .get("attrs")
                .and_then(|a| a.get("url"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let alt = node
                .get("attrs")
                .and_then(|a| a.get("alt"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !url.is_empty() {
                let _ = write!(out, "![{alt}]({url})");
            }
        }
        _ => {
            // Unknown inline — surface as a comment so the user sees what was lost.
            let raw = serde_json::to_string(node).unwrap_or_else(|_| "{}".to_string());
            let _ = write!(out, "<!-- adf:unknown-inline {raw} -->");
        }
    }
}

/// Apply marks to a text run.
///
/// Mark order (outermost → innermost): `link → strong → em → strike →
/// underline → code`. The text is escaped before any non-`code` mark is
/// applied; if the marks include `code`, the inner text is taken verbatim.
fn apply_marks(text: &str, marks: Option<&Vec<Value>>) -> String {
    let marks = marks.cloned().unwrap_or_default();
    let has = |kind: &str| {
        marks
            .iter()
            .any(|m| m.get("type").and_then(Value::as_str) == Some(kind))
    };
    let find = |kind: &str| {
        marks
            .iter()
            .find(|m| m.get("type").and_then(Value::as_str) == Some(kind))
    };

    let is_code = has("code");
    let mut s = if is_code {
        text.to_string()
    } else {
        escape_text(text)
    };

    if is_code {
        s = wrap_in_code_span(&s);
    }
    // When em and strong are both present, use `_` for em so the combined
    // form is `**_x_**` instead of the ambiguous `***x***`.
    if has("em") {
        if has("strong") {
            s = format!("_{s}_");
        } else {
            s = format!("*{s}*");
        }
    }
    if has("strong") {
        s = format!("**{s}**");
    }
    if has("strike") {
        s = format!("~~{s}~~");
    }
    if find("underline").is_some() {
        s = format!("<u>{s}</u>");
    }
    if let Some(sub) = find("subsup") {
        let kind = sub
            .get("attrs")
            .and_then(|a| a.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("sub");
        if kind == "sup" {
            s = format!("<sup>{s}</sup>");
        } else {
            s = format!("<sub>{s}</sub>");
        }
    }
    if let Some(link) = find("link") {
        let href = link
            .get("attrs")
            .and_then(|a| a.get("href"))
            .and_then(Value::as_str)
            .unwrap_or("");
        s = format!("[{s}]({href})");
    }
    s
}

/// Wrap `text` in a CommonMark code span, picking a backtick run long enough
/// to avoid colliding with backticks inside `text`. If `text` starts or ends
/// with a backtick, we pad with a single space (also CommonMark-compliant).
fn wrap_in_code_span(text: &str) -> String {
    // Longest run of consecutive backticks in `text`.
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in text.chars() {
        if ch == '`' {
            current += 1;
            if current > longest {
                longest = current;
            }
        } else {
            current = 0;
        }
    }
    let fence_len = longest + 1;
    let fence: String = "`".repeat(fence_len);
    let needs_pad = text.starts_with('`') || text.ends_with('`');
    let mut out = String::with_capacity(text.len() + 2 * fence_len + if needs_pad { 2 } else { 0 });
    out.push_str(&fence);
    if needs_pad {
        out.push(' ');
    }
    out.push_str(text);
    if needs_pad {
        out.push(' ');
    }
    out.push_str(&fence);
    out
}

fn render_mention(node: &Value, out: &mut String, ctx: &Ctx) {
    let attrs = node.get("attrs");
    let text = attrs
        .and_then(|a| a.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let id = attrs
        .and_then(|a| a.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if !ctx.opts.render_directives {
        if !text.is_empty() {
            out.push_str(&text);
        } else if !id.is_empty() {
            out.push('@');
            out.push_str(&id);
        }
        return;
    }

    let display = if !text.is_empty() {
        text.clone()
    } else if !id.is_empty() {
        format!("@{id}")
    } else {
        String::new()
    };
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    if !id.is_empty() {
        params.insert("accountId".to_string(), id);
    }
    out.push_str(":mention[");
    out.push_str(&display);
    out.push(']');
    if !params.is_empty() {
        out.push('{');
        out.push_str(&render_attrs(&params));
        out.push('}');
    }
}

fn render_emoji(node: &Value, out: &mut String, ctx: &Ctx) {
    let attrs = node.get("attrs");
    let short = attrs
        .and_then(|a| a.get("shortName"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let text = attrs
        .and_then(|a| a.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if !ctx.opts.render_directives {
        // No-directives mode: prefer the rendered glyph (`attrs.text`) when
        // present. If only `attrs.shortName` is set, fall back to the bare
        // `:shortname:` token so the emoji stays visible in the markdown.
        if !text.is_empty() {
            out.push_str(&text);
        } else {
            let bare = short.trim_matches(':');
            if !bare.is_empty() {
                out.push(':');
                out.push_str(bare);
                out.push(':');
            }
        }
        return;
    }

    // Strip leading/trailing `:` from the shortName.
    let bare = short.trim_matches(':').to_string();
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    if !bare.is_empty() {
        params.insert("name".to_string(), bare);
    }
    out.push_str(":emoticon");
    if !params.is_empty() {
        out.push('{');
        out.push_str(&render_attrs(&params));
        out.push('}');
    }
}

fn render_inline_card(node: &Value, out: &mut String, ctx: &Ctx) {
    let url = node
        .get("attrs")
        .and_then(|a| a.get("url"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if !ctx.opts.render_directives {
        if !url.is_empty() {
            out.push('<');
            out.push_str(&url);
            out.push('>');
        }
        return;
    }

    let mut params: BTreeMap<String, String> = BTreeMap::new();
    // Reverse the `pageId:N` synthetic URL used by md_to_adf when the user
    // supplied `:link{pageId=N}` — surface it as `pageId=N` again so the
    // round-trip is lossless.
    if let Some(rest) = url.strip_prefix("pageId:") {
        params.insert("pageId".to_string(), rest.to_string());
    } else if !url.is_empty() {
        params.insert("url".to_string(), url);
    }

    out.push_str(":link[]");
    if !params.is_empty() {
        out.push('{');
        out.push_str(&render_attrs(&params));
        out.push('}');
    }
}

fn render_status(node: &Value, out: &mut String, ctx: &Ctx) {
    let attrs = node.get("attrs");
    let text = attrs
        .and_then(|a| a.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let raw_color = attrs
        .and_then(|a| a.get("color"))
        .and_then(Value::as_str)
        .unwrap_or("neutral");

    if !ctx.opts.render_directives {
        if !text.is_empty() {
            out.push_str(&text);
        }
        return;
    }

    // Map ADF colors back to the same set md_to_adf accepts. md_to_adf's
    // forward map is the identity for {green, red, yellow, blue, purple} and
    // collapses anything else to "neutral", so the inverse is the identity
    // for the same set, with "neutral" passed through.
    let color = match raw_color {
        "green" | "red" | "yellow" | "blue" | "purple" | "neutral" => raw_color,
        _ => "neutral",
    };

    let mut params: BTreeMap<String, String> = BTreeMap::new();
    params.insert("color".to_string(), color.to_string());

    out.push_str(":status[");
    out.push_str(&text);
    out.push(']');
    out.push('{');
    out.push_str(&render_attrs(&params));
    out.push('}');
}

// =====================================================================
// Text helpers
// =====================================================================

/// Escape markdown-special characters in plain text. We escape only those
/// characters that would otherwise be re-interpreted by a markdown parser:
///
/// - `*`, `_`, `[`, `]`, `\` and `` ` `` are always escaped (inline emphasis,
///   links, escapes, code).
/// - `:` is escaped only when followed by an ASCII alphabetic character, to
///   prevent re-triggering the inline-directive parser on round-trip
///   (e.g. `:foo` would otherwise be tokenised as a directive open).
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'*' | b'_' | b'[' | b']' | b'\\' | b'`' => {
                out.push('\\');
                out.push(b as char);
                i += 1;
            }
            b':' => {
                let needs_escape = i + 1 < bytes.len() && bytes[i + 1].is_ascii_alphabetic();
                if needs_escape {
                    out.push('\\');
                }
                out.push(':');
                i += 1;
            }
            _ => {
                let ch_len = utf8_char_len(b);
                let end = (i + ch_len).min(bytes.len());
                if let Ok(s) = std::str::from_utf8(&bytes[i..end]) {
                    out.push_str(s);
                } else {
                    out.push(b as char);
                }
                i = end;
            }
        }
    }
    out
}

fn utf8_char_len(b: u8) -> usize {
    if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

/// Walk a code-block content array and concatenate the `text` field of each
/// `text` node verbatim (no escaping — code blocks are literal).
fn collect_code_text(content: &[Value]) -> String {
    let mut buf = String::new();
    for node in content {
        if node.get("type").and_then(Value::as_str) == Some("text")
            && let Some(s) = node.get("text").and_then(Value::as_str)
        {
            buf.push_str(s);
        }
    }
    buf
}

/// Append `inline` as a paragraph block, ensuring it's preceded by a blank
/// line and followed by `\n\n`. Empty / whitespace-only content is dropped.
fn push_block(out: &mut String, inline: &str) {
    let trimmed = inline.trim();
    if trimmed.is_empty() {
        return;
    }
    ensure_blank_line(out);
    out.push_str(trimmed);
    out.push_str("\n\n");
}

/// Make sure `out` ends with a blank line (or is empty / starts at column 0).
fn ensure_blank_line(out: &mut String) {
    if out.is_empty() {
        return;
    }
    if !out.ends_with("\n\n") {
        if out.ends_with('\n') {
            out.push('\n');
        } else {
            out.push_str("\n\n");
        }
    }
}

/// Collapse 3+ consecutive newlines to 2 to keep output tidy — but never
/// inside a fenced code block. A code block whose body contains two blank
/// lines must keep them; collapsing them silently corrupts the user's code.
///
/// The walker is line-based: a line whose content (after up to three leading
/// spaces) starts with three or more backticks toggles the in-fence state.
/// While in a fence, every line is emitted verbatim. Outside a fence, runs
/// of consecutive blank lines are clamped to a single blank line (i.e. two
/// `\n` in a row).
fn normalize_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_code_fence = false;
    let mut blank_run = 0_usize;

    for (i, line) in s.split('\n').enumerate() {
        let is_fence = is_code_fence_line(line);
        let is_blank = line.is_empty();

        if i > 0 {
            if in_code_fence || is_fence {
                // Inside a fence (or on a fence-toggling line) we always emit
                // the inter-line newline so blank stretches inside code
                // survive intact.
                out.push('\n');
            } else if is_blank {
                blank_run += 1;
                // Only the first blank line in a run gets a newline; further
                // blanks are dropped.
                if blank_run == 1 {
                    out.push('\n');
                }
            } else {
                out.push('\n');
            }
        }

        if !is_blank || in_code_fence || is_fence {
            blank_run = 0;
        }
        // Emit the line content (empty for blank lines is fine — the newline
        // separator is what matters).
        out.push_str(line);

        if is_fence {
            in_code_fence = !in_code_fence;
        }
    }

    out
}

/// Returns true if `line` is a CommonMark fenced-code-block fence (three or
/// more backticks, optionally preceded by up to three spaces and followed by
/// an info string containing no further backticks).
fn is_code_fence_line(line: &str) -> bool {
    let mut leading_spaces = 0_usize;
    let bytes = line.as_bytes();
    while leading_spaces < bytes.len() && bytes[leading_spaces] == b' ' {
        leading_spaces += 1;
    }
    if leading_spaces > 3 {
        return false;
    }
    let mut backticks = 0_usize;
    while leading_spaces + backticks < bytes.len() && bytes[leading_spaces + backticks] == b'`' {
        backticks += 1;
    }
    if backticks < 3 {
        return false;
    }
    let info = &line[leading_spaces + backticks..];
    !info.contains('`')
}

fn emit_unknown_block(node: &Value, out: &mut String) {
    let raw = serde_json::to_string(node).unwrap_or_else(|_| "{}".to_string());
    ensure_blank_line(out);
    let _ = write!(out, "<!-- adf:unknown {raw} -->");
    out.push_str("\n\n");
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn convert(adf: &Value) -> String {
        adf_to_markdown(adf, ConvertOpts::default()).expect("conversion succeeded")
    }

    fn convert_no_directives(adf: &Value) -> String {
        adf_to_markdown(
            adf,
            ConvertOpts {
                render_directives: false,
            },
        )
        .expect("conversion succeeded")
    }

    fn doc(content: Value) -> Value {
        json!({"type": "doc", "version": 1, "content": content})
    }

    // ---- ConvertOpts ----------------------------------------------------

    #[test]
    fn convert_opts_default_renders_directives() {
        assert!(ConvertOpts::default().render_directives);
    }

    // ---- document wrapper -----------------------------------------------

    #[test]
    fn empty_doc_renders_to_empty_string() {
        let adf = doc(json!([]));
        let md = convert(&adf);
        assert!(md.trim().is_empty(), "got: {md:?}");
    }

    #[test]
    fn doc_with_one_paragraph() {
        let adf = doc(json!([
            {"type": "paragraph", "content": [{"type": "text", "text": "hi"}]}
        ]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "hi");
    }

    #[test]
    fn bare_content_array_handled_like_doc() {
        let adf = json!([
            {"type": "paragraph", "content": [{"type": "text", "text": "hi"}]}
        ]);
        let md = convert(&adf);
        assert_eq!(md.trim(), "hi");
    }

    #[test]
    fn bare_block_node_handled() {
        let adf = json!({
            "type": "paragraph",
            "content": [{"type": "text", "text": "lone"}]
        });
        let md = convert(&adf);
        assert_eq!(md.trim(), "lone");
    }

    // ---- block nodes ----------------------------------------------------

    #[test]
    fn heading_levels_one_through_six() {
        for level in 1u64..=6 {
            let adf = doc(json!([
                {
                    "type": "heading",
                    "attrs": {"level": level},
                    "content": [{"type": "text", "text": "h"}]
                }
            ]));
            let md = convert(&adf);
            let prefix = "#".repeat(level as usize);
            assert!(
                md.contains(&format!("{prefix} h")),
                "level {level} got: {md:?}"
            );
        }
    }

    #[test]
    fn heading_default_level_is_one() {
        // Missing level falls back to 1.
        let adf = doc(json!([
            {"type": "heading", "content": [{"type": "text", "text": "x"}]}
        ]));
        let md = convert(&adf);
        assert!(md.contains("# x"), "got: {md:?}");
    }

    #[test]
    fn bullet_list_renders_dash() {
        let adf = doc(json!([
            {
                "type": "bulletList",
                "content": [
                    {"type": "listItem", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "a"}]}
                    ]},
                    {"type": "listItem", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "b"}]}
                    ]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("- a"), "got: {md:?}");
        assert!(md.contains("- b"), "got: {md:?}");
    }

    #[test]
    fn ordered_list_uses_one_dot_prefix() {
        let adf = doc(json!([
            {
                "type": "orderedList",
                "content": [
                    {"type": "listItem", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "one"}]}
                    ]},
                    {"type": "listItem", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "two"}]}
                    ]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("1. one"), "got: {md:?}");
        assert!(md.contains("1. two"), "got: {md:?}");
    }

    #[test]
    fn nested_list_indented() {
        let adf = doc(json!([
            {
                "type": "bulletList",
                "content": [
                    {"type": "listItem", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "outer"}]},
                        {
                            "type": "bulletList",
                            "content": [
                                {"type": "listItem", "content": [
                                    {"type": "paragraph", "content": [{"type": "text", "text": "inner"}]}
                                ]}
                            ]
                        }
                    ]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("- outer"), "got: {md:?}");
        assert!(md.contains("  - inner"), "got: {md:?}");
    }

    #[test]
    fn code_block_with_language() {
        let adf = doc(json!([
            {
                "type": "codeBlock",
                "attrs": {"language": "rust"},
                "content": [{"type": "text", "text": "fn x(){}"}]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("```rust"), "got: {md:?}");
        assert!(md.contains("fn x(){}"), "got: {md:?}");
        assert!(md.contains("```\n"), "got: {md:?}");
    }

    #[test]
    fn code_block_without_language() {
        let adf = doc(json!([
            {
                "type": "codeBlock",
                "content": [{"type": "text", "text": "plain"}]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("```\nplain"), "got: {md:?}");
    }

    #[test]
    fn code_block_preserves_internal_blank_lines() {
        // Regression: `normalize_blank_lines` used to collapse 3+ consecutive
        // newlines globally, including inside fenced code blocks. A code
        // block whose body has two blank lines between two non-empty lines
        // must keep all three newlines verbatim.
        let adf = doc(json!([
            {
                "type": "codeBlock",
                "attrs": {"language": "text"},
                "content": [{"type": "text", "text": "line1\n\n\nline2"}]
            }
        ]));
        let md = convert(&adf);
        assert!(
            md.contains("line1\n\n\nline2"),
            "blank lines inside fenced code block must be preserved, got:\n{md:?}"
        );
    }

    #[test]
    fn adf_code_block_no_backticks_uses_three_tick_fence() {
        let adf = doc(json!([
            {
                "type": "codeBlock",
                "content": [{"type": "text", "text": "hello"}]
            }
        ]));
        let md = convert(&adf);
        assert!(
            md.contains("```\nhello\n```\n"),
            "expected default 3-tick fence, got: {md:?}"
        );
        assert!(
            !md.contains("````"),
            "did not expect a 4-tick fence, got: {md:?}"
        );
    }

    #[test]
    fn adf_code_block_with_double_backticks_still_three_tick_fence() {
        let adf = doc(json!([
            {
                "type": "codeBlock",
                "content": [{"type": "text", "text": "a `` b"}]
            }
        ]));
        let md = convert(&adf);
        assert!(
            md.contains("```\na `` b\n```\n"),
            "expected 3-tick fence, got: {md:?}"
        );
        assert!(
            !md.contains("````"),
            "did not expect a 4-tick fence, got: {md:?}"
        );
    }

    #[test]
    fn adf_code_block_with_triple_backticks_uses_four_tick_fence() {
        // Regression: a triple-backtick run inside the body used to close the
        // 3-tick fence early. Verify the language tag is still adjacent to a
        // 4-tick opening fence.
        let adf = doc(json!([
            {
                "type": "codeBlock",
                "attrs": {"language": "python"},
                "content": [{"type": "text", "text": "```triple```"}]
            }
        ]));
        let md = convert(&adf);
        assert!(
            md.contains("````python\n```triple```\n````\n"),
            "expected 4-tick fence with python language, got: {md:?}"
        );
    }

    #[test]
    fn adf_code_block_with_quadruple_backticks_uses_five_tick_fence() {
        let adf = doc(json!([
            {
                "type": "codeBlock",
                "content": [{"type": "text", "text": "a ```` b"}]
            }
        ]));
        let md = convert(&adf);
        assert!(
            md.contains("`````\na ```` b\n`````\n"),
            "expected 5-tick fence, got: {md:?}"
        );
    }

    #[test]
    fn blockquote_renders_prefix() {
        let adf = doc(json!([
            {
                "type": "blockquote",
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "hi"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("> hi"), "got: {md:?}");
    }

    #[test]
    fn rule_renders_three_dashes() {
        let adf = doc(json!([{"type": "rule"}]));
        let md = convert(&adf);
        assert!(md.contains("---"), "got: {md:?}");
    }

    #[test]
    fn simple_table_with_header() {
        let adf = doc(json!([
            {
                "type": "table",
                "content": [
                    {
                        "type": "tableRow",
                        "content": [
                            {"type": "tableHeader", "content": [
                                {"type": "paragraph", "content": [{"type": "text", "text": "A"}]}
                            ]},
                            {"type": "tableHeader", "content": [
                                {"type": "paragraph", "content": [{"type": "text", "text": "B"}]}
                            ]}
                        ]
                    },
                    {
                        "type": "tableRow",
                        "content": [
                            {"type": "tableCell", "content": [
                                {"type": "paragraph", "content": [{"type": "text", "text": "1"}]}
                            ]},
                            {"type": "tableCell", "content": [
                                {"type": "paragraph", "content": [{"type": "text", "text": "2"}]}
                            ]}
                        ]
                    }
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("| A | B |"), "got: {md:?}");
        assert!(md.contains("| --- | --- |"), "got: {md:?}");
        assert!(md.contains("| 1 | 2 |"), "got: {md:?}");
    }

    #[test]
    fn table_cell_pipe_is_escaped() {
        // Bug 1: a literal `|` inside a cell would split the column. The
        // emitted cell must backslash-escape it (GFM convention).
        let adf = doc(json!([
            {
                "type": "table",
                "content": [
                    {
                        "type": "tableRow",
                        "content": [
                            {"type": "tableHeader", "content": [
                                {"type": "paragraph", "content": [{"type": "text", "text": "h|x"}]}
                            ]},
                        ]
                    },
                    {
                        "type": "tableRow",
                        "content": [
                            {"type": "tableCell", "content": [
                                {"type": "paragraph", "content": [{"type": "text", "text": "a | b"}]}
                            ]},
                        ]
                    }
                ]
            }
        ]));
        let md = convert(&adf);
        // Cell containing `a | b` must have escaped pipes.
        assert!(md.contains(r"a \| b"), "got: {md:?}");
        // Header containing `h|x` must also be escaped.
        assert!(md.contains(r"h\|x"), "got: {md:?}");
    }

    #[test]
    fn code_span_with_backtick_uses_longer_fence() {
        // Bug 2: a single-backtick fence around `a`b` produces malformed
        // markdown; the fence must be longer than any internal run.
        let adf = one_para(json!([
            {"type": "text", "text": "a`b", "marks": [{"type": "code"}]}
        ]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "``a`b``");
    }

    #[test]
    fn code_span_starting_with_backtick_pads_with_space() {
        // Bug 2: when the body starts/ends with `\``, CommonMark requires
        // exactly one padding space between the fence and the body.
        let adf = one_para(json!([
            {"type": "text", "text": "`x", "marks": [{"type": "code"}]}
        ]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "`` `x ``");
    }

    #[test]
    fn media_single_external_url() {
        let adf = doc(json!([
            {
                "type": "mediaSingle",
                "content": [
                    {
                        "type": "media",
                        "attrs": {"type": "external", "url": "http://x.png"}
                    }
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("![](http://x.png)"), "got: {md:?}");
    }

    // ---- inline marks ---------------------------------------------------

    fn one_para(inline: Value) -> Value {
        doc(json!([
            {"type": "paragraph", "content": inline}
        ]))
    }

    #[test]
    fn plain_text() {
        let adf = one_para(json!([{"type": "text", "text": "hello"}]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "hello");
    }

    #[test]
    fn strong_mark_emits_double_asterisk() {
        let adf = one_para(json!([
            {"type": "text", "text": "x", "marks": [{"type": "strong"}]}
        ]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "**x**");
    }

    #[test]
    fn em_mark_emits_asterisk() {
        let adf = one_para(json!([
            {"type": "text", "text": "x", "marks": [{"type": "em"}]}
        ]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "*x*");
    }

    #[test]
    fn code_mark_does_not_escape_inner_text() {
        let adf = one_para(json!([
            {"type": "text", "text": "a*b", "marks": [{"type": "code"}]}
        ]));
        let md = convert(&adf);
        // Inside code, the `*` must NOT be escaped.
        assert_eq!(md.trim(), "`a*b`");
    }

    #[test]
    fn strike_mark_emits_double_tilde() {
        let adf = one_para(json!([
            {"type": "text", "text": "x", "marks": [{"type": "strike"}]}
        ]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "~~x~~");
    }

    #[test]
    fn link_mark_emits_markdown_link() {
        let adf = one_para(json!([
            {
                "type": "text",
                "text": "x",
                "marks": [{"type": "link", "attrs": {"href": "http://x"}}]
            }
        ]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "[x](http://x)");
    }

    #[test]
    fn combined_strong_and_em() {
        let adf = one_para(json!([
            {
                "type": "text",
                "text": "x",
                "marks": [{"type": "strong"}, {"type": "em"}]
            }
        ]));
        let md = convert(&adf);
        // Strong outer, em inner: **_x_**
        assert_eq!(md.trim(), "**_x_**");
    }

    #[test]
    fn hard_break_emits_two_spaces_newline() {
        let adf = one_para(json!([
            {"type": "text", "text": "a"},
            {"type": "hardBreak"},
            {"type": "text", "text": "b"}
        ]));
        let md = convert(&adf);
        assert!(md.contains("a  \nb"), "got: {md:?}");
    }

    #[test]
    fn hard_break_round_trips_as_two_spaces_newline() {
        // ADF paragraph `[text "Foo", hardBreak, text "Bar"]` must render
        // as `Foo  \nBar` so feeding the result back through md_to_adf
        // reproduces the same hardBreak node.
        let adf = one_para(json!([
            {"type": "text", "text": "Foo"},
            {"type": "hardBreak"},
            {"type": "text", "text": "Bar"}
        ]));
        let md = convert(&adf);
        assert!(
            md.contains("Foo  \nBar"),
            "expected 'Foo  \\nBar' (two spaces + newline), got: {md:?}"
        );
    }

    #[test]
    fn hard_break_at_end_of_paragraph_not_emitted() {
        // A trailing hardBreak at the end of paragraph content is redundant —
        // the paragraph's closing `\n\n` already separates blocks, so any
        // stray two-space marker would show up as ugly whitespace in the
        // rendered output. `push_block` trims trailing whitespace from the
        // inline buffer, so the hardBreak collapses away naturally.
        let adf = one_para(json!([
            {"type": "text", "text": "Foo"},
            {"type": "hardBreak"}
        ]));
        let md = convert(&adf);
        // Each non-empty block ends with `\n\n`. The trailing hardBreak
        // means the inline buffer ends with `Foo  \n`, but `push_block`
        // calls `.trim()` so the final output ends in `Foo\n\n`, no stray
        // trailing spaces before the block separator.
        assert!(
            md.ends_with("Foo\n\n") || md.ends_with("Foo\n") || md.trim_end() == "Foo",
            "expected paragraph to end cleanly with 'Foo' (no trailing two-space marker), got: {md:?}"
        );
        assert!(
            !md.contains("Foo  \n\n"),
            "trailing hardBreak must not leave a stray two-space marker before the block separator, got: {md:?}"
        );
    }

    #[test]
    fn underline_emits_html_tag() {
        let adf = one_para(json!([
            {"type": "text", "text": "x", "marks": [{"type": "underline"}]}
        ]));
        let md = convert(&adf);
        assert_eq!(md.trim(), "<u>x</u>");
    }

    // ---- block directives ----------------------------------------------

    #[test]
    fn panel_info() {
        let adf = doc(json!([
            {
                "type": "panel",
                "attrs": {"panelType": "info"},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "body"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::info"), "got: {md:?}");
        assert!(md.contains("body"), "got: {md:?}");
        assert!(md.contains(":::\n"), "got: {md:?}");
    }

    #[test]
    fn panel_with_strong_first_paragraph_as_title() {
        let adf = doc(json!([
            {
                "type": "panel",
                "attrs": {"panelType": "info"},
                "content": [
                    {
                        "type": "paragraph",
                        "content": [
                            {"type": "text", "text": "Heads up", "marks": [{"type": "strong"}]}
                        ]
                    },
                    {"type": "paragraph", "content": [{"type": "text", "text": "body"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::info"), "got: {md:?}");
        assert!(md.contains(r#"title="Heads up""#), "got: {md:?}");
        assert!(md.contains("body"), "got: {md:?}");
        // The title paragraph itself must NOT appear in the body as **Heads up**.
        assert!(!md.contains("**Heads up**"), "got: {md:?}");
    }

    #[test]
    fn panel_warning() {
        let adf = doc(json!([
            {
                "type": "panel",
                "attrs": {"panelType": "warning"},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "w"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::warning"), "got: {md:?}");
    }

    #[test]
    fn panel_note() {
        let adf = doc(json!([
            {
                "type": "panel",
                "attrs": {"panelType": "note"},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "n"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::note"), "got: {md:?}");
    }

    #[test]
    fn panel_success_maps_to_tip() {
        // ADF "success" is the inverse of `:::tip` in md_to_adf.
        let adf = doc(json!([
            {
                "type": "panel",
                "attrs": {"panelType": "success"},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "yes"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::tip"), "got: {md:?}");
    }

    #[test]
    fn panel_error_falls_back_to_warning() {
        // No `:::error` directive yet; collapse to `:::warning`.
        let adf = doc(json!([
            {
                "type": "panel",
                "attrs": {"panelType": "error"},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "boom"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::warning"), "got: {md:?}");
        assert!(md.contains("boom"), "got: {md:?}");
    }

    #[test]
    fn expand_with_title() {
        let adf = doc(json!([
            {
                "type": "expand",
                "attrs": {"title": "Detail"},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "hi"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::expand"), "got: {md:?}");
        assert!(md.contains(r#"title=Detail"#), "got: {md:?}");
        assert!(md.contains("hi"), "got: {md:?}");
    }

    #[test]
    fn expand_without_title_omits_param() {
        let adf = doc(json!([
            {
                "type": "expand",
                "attrs": {"title": ""},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "hi"}]}
                ]
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::expand\n"), "got: {md:?}");
        assert!(!md.contains("title="), "got: {md:?}");
    }

    #[test]
    fn expand_node_no_directives_renders_title_as_bold() {
        // ADF `expand` keeps its title on `attrs.title` (no body paragraph),
        // so a `--no-directives` strip would drop it without explicit
        // handling. Render the title as a bold paragraph so it survives —
        // matches how panel titles flow through as strong-marked first
        // paragraphs.
        let adf = doc(json!([
            {
                "type": "expand",
                "attrs": {"title": "Click to expand"},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "Hidden"}]}
                ]
            }
        ]));
        let md = convert_no_directives(&adf);
        assert!(
            md.contains("**Click to expand**"),
            "expected bold title, got: {md:?}"
        );
        assert!(md.contains("Hidden"), "got: {md:?}");
        assert!(
            !md.contains(":::expand"),
            "directive wrapper must be stripped, got: {md:?}"
        );
        // Title must precede body.
        let title_idx = md.find("**Click to expand**").expect("title present");
        let body_idx = md.find("Hidden").expect("body present");
        assert!(
            title_idx < body_idx,
            "title must come before body, got: {md:?}"
        );
    }

    #[test]
    fn expand_node_no_directives_no_title_emits_only_body() {
        // No title attribute → emit just the body, no stray bold marker.
        let adf = doc(json!([
            {
                "type": "expand",
                "attrs": {"title": ""},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "Hidden"}]}
                ]
            }
        ]));
        let md = convert_no_directives(&adf);
        assert!(md.contains("Hidden"), "got: {md:?}");
        assert!(
            !md.contains("**"),
            "must not emit empty bold marker when no title present, got: {md:?}"
        );
        assert!(
            !md.contains(":::expand"),
            "directive wrapper must be stripped, got: {md:?}"
        );
    }

    #[test]
    fn extension_toc_emits_directive() {
        let adf = doc(json!([
            {
                "type": "extension",
                "attrs": {
                    "extensionType": "com.atlassian.confluence.macro.core",
                    "extensionKey": "toc",
                    "parameters": {
                        "macroParams": {
                            "maxLevel": {"value": "3"}
                        }
                    }
                }
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains(":::toc"), "got: {md:?}");
        assert!(md.contains("maxLevel=3"), "got: {md:?}");
    }

    #[test]
    fn unknown_extension_passthrough_as_comment() {
        let adf = doc(json!([
            {
                "type": "extension",
                "attrs": {
                    "extensionType": "some.other",
                    "extensionKey": "frobinator"
                }
            }
        ]));
        let md = convert(&adf);
        assert!(md.contains("<!-- adf:unknown"), "got: {md:?}");
    }

    // ---- inline directives ---------------------------------------------

    #[test]
    fn inline_status_directive() {
        let adf = one_para(json!([
            {"type": "status", "attrs": {"text": "DONE", "color": "green"}}
        ]));
        let md = convert(&adf);
        assert!(md.contains(":status[DONE]"), "got: {md:?}");
        assert!(md.contains("color=green"), "got: {md:?}");
    }

    #[test]
    fn inline_emoji_directive() {
        let adf = one_para(json!([
            {"type": "emoji", "attrs": {"shortName": ":warning:"}}
        ]));
        let md = convert(&adf);
        assert!(md.contains(":emoticon"), "got: {md:?}");
        assert!(md.contains("name=warning"), "got: {md:?}");
    }

    #[test]
    fn inline_emoji_no_directives_falls_back_to_short_name() {
        // Regression: in `--no-directives` mode, an emoji node with only
        // `attrs.shortName` (no `attrs.text`) used to silently disappear.
        // We now emit `:smile:` so the emoji stays visible in the rendered
        // markdown.
        let adf = one_para(json!([
            {"type": "emoji", "attrs": {"shortName": "smile"}}
        ]));
        let md = convert_no_directives(&adf);
        assert!(
            md.contains(":smile:"),
            "expected `:smile:` short-name fallback, got: {md:?}"
        );
    }

    #[test]
    fn inline_emoji_no_directives_prefers_text_over_short_name() {
        // When both `attrs.text` (the rendered glyph) and `attrs.shortName`
        // are present, the glyph wins — that's the most fluent rendering.
        let adf = one_para(json!([
            {"type": "emoji", "attrs": {"shortName": "smile", "text": "\u{1F604}"}}
        ]));
        let md = convert_no_directives(&adf);
        assert!(
            md.contains('\u{1F604}'),
            "expected glyph `text` to win, got: {md:?}"
        );
        assert!(
            !md.contains(":smile:"),
            "must not also emit short-name fallback when text is present, got: {md:?}"
        );
    }

    #[test]
    fn inline_mention_directive() {
        let adf = one_para(json!([
            {"type": "mention", "attrs": {"id": "abc", "text": "@john"}}
        ]));
        let md = convert(&adf);
        assert!(md.contains(":mention[@john]"), "got: {md:?}");
        assert!(md.contains("accountId=abc"), "got: {md:?}");
    }

    #[test]
    fn inline_card_with_page_id_url_unwraps_to_page_id_param() {
        let adf = one_para(json!([
            {"type": "inlineCard", "attrs": {"url": "pageId:12345"}}
        ]));
        let md = convert(&adf);
        assert!(md.contains(":link[]"), "got: {md:?}");
        assert!(md.contains("pageId=12345"), "got: {md:?}");
        assert!(!md.contains("url="), "got: {md:?}");
    }

    #[test]
    fn inline_card_with_regular_url_uses_url_param() {
        let adf = one_para(json!([
            {"type": "inlineCard", "attrs": {"url": "https://example.com"}}
        ]));
        let md = convert(&adf);
        assert!(md.contains(":link[]"), "got: {md:?}");
        assert!(md.contains("url="), "got: {md:?}");
        assert!(md.contains("https://example.com"), "got: {md:?}");
    }

    // ---- render_directives = false -------------------------------------

    #[test]
    fn no_directives_strips_panel_wrapper() {
        let adf = doc(json!([
            {
                "type": "panel",
                "attrs": {"panelType": "info"},
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "body"}]}
                ]
            }
        ]));
        let md = convert_no_directives(&adf);
        assert!(md.contains("body"), "got: {md:?}");
        assert!(!md.contains(":::info"), "got: {md:?}");
    }

    #[test]
    fn no_directives_strips_status_to_text() {
        let adf = one_para(json!([
            {"type": "text", "text": "before "},
            {"type": "status", "attrs": {"text": "DONE", "color": "green"}},
            {"type": "text", "text": " after"}
        ]));
        let md = convert_no_directives(&adf);
        assert!(md.contains("DONE"), "got: {md:?}");
        assert!(!md.contains(":status"), "got: {md:?}");
    }

    #[test]
    fn no_directives_strips_mention_to_display_text() {
        let adf = one_para(json!([
            {"type": "mention", "attrs": {"id": "abc", "text": "@john"}}
        ]));
        let md = convert_no_directives(&adf);
        assert!(md.contains("@john"), "got: {md:?}");
        assert!(!md.contains(":mention"), "got: {md:?}");
    }

    // ---- edge cases ----------------------------------------------------

    #[test]
    fn null_input_renders_empty() {
        let md = adf_to_markdown(&Value::Null, ConvertOpts::default()).unwrap();
        assert!(md.is_empty());
    }

    #[test]
    fn malformed_object_without_type_returns_err() {
        let bad = json!({"version": 1, "content": []});
        let err = adf_to_markdown(&bad, ConvertOpts::default()).unwrap_err();
        match err {
            AdfToMdError::Malformed(_) => {}
        }
    }

    #[test]
    fn paragraph_missing_content_renders_as_empty() {
        let adf = doc(json!([
            {"type": "paragraph"}
        ]));
        let md = convert(&adf);
        assert!(md.trim().is_empty(), "got: {md:?}");
    }

    #[test]
    fn unknown_block_type_passes_through_as_comment() {
        let adf = doc(json!([
            {"type": "thingamajig", "attrs": {"x": 1}}
        ]));
        let md = convert(&adf);
        assert!(md.contains("<!-- adf:unknown"), "got: {md:?}");
        assert!(md.contains("thingamajig"), "got: {md:?}");
    }

    #[test]
    fn plain_text_special_characters_escaped() {
        let adf = one_para(json!([
            {"type": "text", "text": "a*b_c [d] :foo"}
        ]));
        let md = convert(&adf);
        // Markdown specials must be escaped.
        assert!(md.contains(r"a\*b\_c"), "got: {md:?}");
        assert!(md.contains(r"\[d\]"), "got: {md:?}");
        // `:foo` must be escaped to prevent re-triggering the inline directive parser.
        assert!(md.contains(r"\:foo"), "got: {md:?}");
    }

    #[test]
    fn invalid_input_type_returns_err() {
        // A bare string is not a recognisable ADF shape.
        let bad = json!("just a string");
        let err = adf_to_markdown(&bad, ConvertOpts::default()).unwrap_err();
        match err {
            AdfToMdError::Malformed(_) => {}
        }
    }

    // ---- round-trip soundness ------------------------------------------

    #[test]
    fn roundtrip_heading_via_md_to_adf() {
        use crate::cli::commands::converters::md_to_adf::markdown_to_adf;
        let adf = markdown_to_adf("# Hello").unwrap();
        let md = adf_to_markdown(&adf, ConvertOpts::default()).unwrap();
        assert!(md.starts_with("# Hello"), "got: {md:?}");
    }

    #[test]
    fn roundtrip_info_panel_via_md_to_adf() {
        use crate::cli::commands::converters::md_to_adf::markdown_to_adf;
        let adf = markdown_to_adf(":::info\nbody\n:::").unwrap();
        let md = adf_to_markdown(&adf, ConvertOpts::default()).unwrap();
        assert!(md.contains(":::info"), "got: {md:?}");
        assert!(md.contains("body"), "got: {md:?}");
    }
}
