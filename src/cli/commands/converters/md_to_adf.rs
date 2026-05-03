//! Markdown (with MyST-style directive extensions) → Atlassian Document Format
//! (ADF) JSON.
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
//! 1. **Code-fence-aware line walk.** Lines inside a CommonMark fenced code
//!    block (` ``` ` / `~~~`) bypass the directive lexer entirely so a literal
//!    `:::info` inside a code block round-trips as code, not a panel.
//! 2. **Tree building.** `Open` / `Close` / `Line` events fold into a nested
//!    tree of [`Node::Directive`] and [`Node::Text`].
//! 3. **Recursive render.** Each text chunk is parsed by comrak with GFM, and
//!    the resulting AST is walked to emit ADF nodes; each directive becomes
//!    the matching ADF block (`panel`, `expand`, …) or extension. Inline text
//!    runs are first split through [`crate::cli::commands::directives::parse_inline`]
//!    so inline directives become structured ADF nodes
//!    (`status`, `mention`, `emoji`, `inlineCard`).
//!
//! # Lossy mappings
//!
//! ADF doesn't have a 1:1 equivalent for every markdown construct, so a few
//! choices are documented inline:
//!
//! - **Images** become `mediaSingle { media { type: "external", url } }`. Real
//!   ADF media uses an `id` + `collection` from Confluence's media service —
//!   we cannot synthesise those here, so the `external` form is the best
//!   approximation.
//! - **`:link{pageId=N}`** lacks the API resource URL we'd need for a real
//!   `inlineCard`. We fall back to `pageId:N` so the lossy mapping is visible
//!   to humans inspecting the output.
//! - **`:::toc`** maps to a Confluence macro extension node — ADF has no
//!   native TOC node.
//! - **`:image{}` inline** is dropped (emits an empty text run). `mediaSingle`
//!   is a block node and would break inline flow if injected mid-paragraph.
//!   Use a standalone `:::image` block (TODO once supported) or a markdown
//!   `![alt](url)` reference at block level instead.
//! - **HTML inline / blocks** are emitted as text-mode passthrough — ADF has
//!   no raw-HTML node, so we keep the literal as plain text.
//! - **Soft breaks** become a single space (matches Confluence's rendering).
//!
//! The inverse converter (ADF → markdown) is not yet implemented.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use comrak::nodes::{AstNode, ListType, NodeCode, NodeValue};
use comrak::{Arena, Options, parse_document};
use serde_json::{Value, json};
use thiserror::Error;

use crate::cli::commands::directives::{
    BlockEvent, BlockLexer, DirectiveError, DirectiveSpec, InlineDirective, InlineToken, lookup,
    parse_inline,
};

/// Placeholder substring used to mark where an inline directive lived in the
/// original markdown. Wrapped in a thread-local pre-pass before comrak runs so
/// markdown's own inline grammar (e.g. `[Title]`) doesn't eat the directive's
/// `[content]` brackets.
const PH_PREFIX: &str = "ATLINLPLACEHOLDER";

thread_local! {
    /// Buffer of inline directives extracted by [`substitute_inline_directives`]
    /// for the current `render_md_block` call. Indexed by the integer that
    /// follows `ATLINLPLACEHOLDER` in placeholder text. Cleared at the start of
    /// every `render_md_block` invocation so render calls never see stale
    /// directives.
    static INLINE_DIRECTIVES: RefCell<Vec<InlineDirective>> = const { RefCell::new(Vec::new()) };
}

// =====================================================================
// Errors
// =====================================================================

/// Errors returned by [`markdown_to_adf`].
#[derive(Debug, Error)]
pub enum MdToAdfError {
    /// A directive grammar error (e.g. unclosed `:::name` block) was found.
    #[error(transparent)]
    Directive(#[from] DirectiveError),
}

// =====================================================================
// Public API
// =====================================================================

/// Convert markdown (with MyST-style directive extensions) to an Atlassian
/// Document Format JSON document.
///
/// Returns the full `{"type": "doc", "version": 1, "content": [...]}` value.
/// Callers can either embed the value into a Confluence/Jira API payload or
/// serialise it to a string with `serde_json::to_string`.
///
/// Returns an error only on unrecoverable directive grammar issues
/// (specifically, an unclosed `:::name` block fence). Unknown directive names
/// pass through as plain text — they don't fail conversion.
///
/// # Examples
///
/// ```ignore
/// use atl::cli::commands::converters::md_to_adf::markdown_to_adf;
///
/// let doc = markdown_to_adf(":::info\nHello\n:::").unwrap();
/// assert_eq!(doc["type"], "doc");
/// assert_eq!(doc["content"][0]["type"], "panel");
/// assert_eq!(doc["content"][0]["attrs"]["panelType"], "info");
/// ```
pub fn markdown_to_adf(md: &str) -> Result<Value, MdToAdfError> {
    let events = lex_with_code_fences(md)?;
    let tree = build_tree(events);
    let content = render(&tree);
    Ok(json!({
        "type": "doc",
        "version": 1,
        "content": content,
    }))
}

// =====================================================================
// Stage 1: code-fence-aware line walk (mirror of md_to_storage)
// =====================================================================

fn lex_with_code_fences(md: &str) -> Result<Vec<BlockEvent>, DirectiveError> {
    let mut lex = BlockLexer::new();
    let mut events = Vec::new();
    let mut code_fence: Option<CodeFence> = None;

    for line in md.split('\n') {
        match &code_fence {
            Some(open) => {
                events.push(BlockEvent::Line(line.to_string()));
                if is_matching_close_fence(line, open) {
                    code_fence = None;
                }
            }
            None => {
                if let Some(open) = parse_fence_open(line) {
                    events.push(BlockEvent::Line(line.to_string()));
                    code_fence = Some(open);
                } else {
                    events.push(lex.lex_line(line));
                }
            }
        }
    }

    lex.finalize()?;
    Ok(events)
}

#[derive(Debug, Clone, Copy)]
struct CodeFence {
    fence_char: char,
    fence_len: usize,
}

fn parse_fence_open(line: &str) -> Option<CodeFence> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return None;
    }
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let mut count = 1;
    for c in chars.by_ref() {
        if c == first {
            count += 1;
        } else {
            break;
        }
    }
    if count < 3 {
        return None;
    }
    Some(CodeFence {
        fence_char: first,
        fence_len: count,
    })
}

fn is_matching_close_fence(line: &str, open: &CodeFence) -> bool {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return false;
    }
    let mut iter = trimmed.chars();
    let mut count = 0;
    while let Some(c) = iter.clone().next() {
        if c == open.fence_char {
            count += 1;
            iter.next();
        } else {
            break;
        }
    }
    if count < open.fence_len {
        return false;
    }
    iter.all(char::is_whitespace)
}

// =====================================================================
// Stage 2: tree building
// =====================================================================

#[derive(Debug)]
enum Node {
    /// A run of plain markdown lines, joined with `\n`.
    Text(String),
    /// A `:::name … :::` block directive.
    Directive {
        name: String,
        spec: Option<&'static DirectiveSpec>,
        params: BTreeMap<String, String>,
        children: Vec<Node>,
    },
}

struct Frame {
    name: String,
    spec: Option<&'static DirectiveSpec>,
    params: BTreeMap<String, String>,
    children: Vec<Node>,
}

fn build_tree(events: Vec<BlockEvent>) -> Vec<Node> {
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Vec<Node> = Vec::new();

    fn push_line(target: &mut Vec<Node>, line: String) {
        if let Some(Node::Text(prev)) = target.last_mut() {
            prev.push('\n');
            prev.push_str(&line);
        } else {
            target.push(Node::Text(line));
        }
    }

    for ev in events {
        match ev {
            BlockEvent::Open { name, params, .. } => {
                let spec = lookup(&name);
                stack.push(Frame {
                    name,
                    spec,
                    params,
                    children: Vec::new(),
                });
            }
            BlockEvent::Close { .. } => {
                if let Some(frame) = stack.pop() {
                    let node = Node::Directive {
                        name: frame.name,
                        spec: frame.spec,
                        params: frame.params,
                        children: frame.children,
                    };
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    } else {
                        root.push(node);
                    }
                }
            }
            BlockEvent::Line(line) => {
                let target = stack
                    .last_mut()
                    .map_or(&mut root, |frame| &mut frame.children);
                push_line(target, line);
            }
        }
    }

    root
}

// =====================================================================
// Stage 3: rendering — tree → ADF content array
// =====================================================================

fn render(nodes: &[Node]) -> Vec<Value> {
    let mut content: Vec<Value> = Vec::new();
    for node in nodes {
        match node {
            Node::Text(md) => content.extend(render_md_block(md)),
            Node::Directive {
                name,
                spec,
                params,
                children,
            } => {
                let body_content = render(children);
                content.push(directive_to_adf_block(name, *spec, params, body_content));
            }
        }
    }
    content
}

/// Render a recognised block directive (`:::info`, `:::expand`, …) to its ADF
/// node. Falls back to a plain paragraph for unknown names.
fn directive_to_adf_block(
    name: &str,
    spec: Option<&'static DirectiveSpec>,
    params: &BTreeMap<String, String>,
    body: Vec<Value>,
) -> Value {
    match (name, spec) {
        ("info" | "warning" | "note" | "tip", Some(spec)) => panel_node(spec, params, body),
        ("expand", _) => expand_node(params, body),
        ("toc", _) => toc_extension(params),
        _ => fallback_block(name, params, body),
    }
}

/// `{"type": "panel", "attrs": {"panelType": "..."}, "content": [...]}`
fn panel_node(
    spec: &'static DirectiveSpec,
    params: &BTreeMap<String, String>,
    body: Vec<Value>,
) -> Value {
    let panel_type = spec.conf_adf_panel_type.unwrap_or("info");
    let mut content = body;
    if let Some(title) = params.get("title")
        && !title.is_empty()
    {
        // Prepend a strong-marked paragraph so the panel renders with a
        // visible title even though ADF panels don't have a dedicated title
        // attr. Documented in the module-level rustdoc.
        let title_paragraph = json!({
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": title,
                "marks": [{"type": "strong"}],
            }],
        });
        content.insert(0, title_paragraph);
    }
    json!({
        "type": "panel",
        "attrs": {"panelType": panel_type},
        "content": content,
    })
}

/// `{"type": "expand", "attrs": {"title": "..."}, "content": [...]}`
fn expand_node(params: &BTreeMap<String, String>, body: Vec<Value>) -> Value {
    let title = params.get("title").cloned().unwrap_or_default();
    json!({
        "type": "expand",
        "attrs": {"title": title},
        "content": body,
    })
}

/// `:::toc` has no native ADF node — emit a Confluence macro extension.
fn toc_extension(params: &BTreeMap<String, String>) -> Value {
    let max_level = params
        .get("maxLevel")
        .cloned()
        .unwrap_or_else(|| "3".to_string());
    json!({
        "type": "extension",
        "attrs": {
            "extensionType": "com.atlassian.confluence.macro.core",
            "extensionKey": "toc",
            "parameters": {
                "macroParams": {
                    "maxLevel": {"value": max_level},
                },
            },
        },
    })
}

/// Unknown directive (or known but with no ADF mapping). Emit a paragraph
/// echoing the literal `:::name {attrs}` so the user sees their input wasn't
/// silently dropped, followed by the rendered body.
fn fallback_block(name: &str, params: &BTreeMap<String, String>, body: Vec<Value>) -> Value {
    let mut header = format!(":::{name}");
    if !params.is_empty() {
        header.push(' ');
        header.push_str(&crate::cli::commands::directives::render_attrs(params));
    }
    let mut content = vec![json!({
        "type": "paragraph",
        "content": [{"type": "text", "text": header}],
    })];
    content.extend(body);
    // Wrap in a no-op blockquote so the fallback survives as a single block.
    json!({
        "type": "blockquote",
        "content": content,
    })
}

// =====================================================================
// Stage 4: rendering — markdown chunk → ADF blocks via comrak
// =====================================================================

fn gfm_options() -> Options<'static> {
    let mut opts = Options::default();
    opts.extension.table = true;
    opts.extension.strikethrough = true;
    opts.extension.autolink = true;
    opts.extension.tasklist = true;
    // ADF has no raw HTML node; we surface inline HTML as plain text.
    opts.render.r#unsafe = false;
    opts
}

fn render_md_block(md: &str) -> Vec<Value> {
    if md.trim().is_empty() {
        return Vec::new();
    }
    INLINE_DIRECTIVES.with(|cell| cell.borrow_mut().clear());
    let with_placeholders = substitute_inline_directives(md);
    let arena = Arena::new();
    let opts = gfm_options();
    let root = parse_document(&arena, &with_placeholders, &opts);
    let mut content = Vec::new();
    for child in root.children() {
        if let Some(node) = render_block(child) {
            content.push(node);
        }
    }
    content
}

/// Walk `md` line by line and replace every inline directive token with a
/// placeholder of the form `ATLINLPLACEHOLDER{n}`. Lines inside a CommonMark
/// fenced code block are passed through verbatim so that `:status[…]` inside a
/// fence stays as code text. The extracted [`InlineDirective`] values are
/// pushed into the thread-local [`INLINE_DIRECTIVES`] buffer.
fn substitute_inline_directives(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut code_fence: Option<CodeFence> = None;
    let mut first = true;

    for line in md.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;

        match &code_fence {
            Some(open) => {
                out.push_str(line);
                if is_matching_close_fence(line, open) {
                    code_fence = None;
                }
            }
            None => {
                if let Some(open) = parse_fence_open(line) {
                    out.push_str(line);
                    code_fence = Some(open);
                } else {
                    substitute_line(line, &mut out);
                }
            }
        }
    }

    out
}

/// One slice of a line, classified by whether it lives inside a CommonMark
/// inline code span. The borrowed strings sum back to the original line.
enum LineSegment<'a> {
    Outside(&'a str),
    CodeSpan(&'a str),
}

/// Split `line` into alternating "outside" and "code-span" segments using the
/// CommonMark rule for inline code: a span opens with N consecutive backticks
/// and closes with the next run of *exactly* N backticks. Unmatched openers
/// are returned verbatim as `Outside`.
///
/// Limitation: indented (4-space) code blocks are NOT recognised here. They
/// would require a markdown AST walk; the line-oriented pre-pass cannot tell
/// an indented code line from a regular paragraph by itself.
fn split_code_span_segments(line: &str) -> Vec<LineSegment<'_>> {
    let bytes = line.as_bytes();
    let mut segments: Vec<LineSegment<'_>> = Vec::new();
    let mut i = 0usize;
    let mut outside_start = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let opener_start = i;
        while i < bytes.len() && bytes[i] == b'`' {
            i += 1;
        }
        let opener_len = i - opener_start;

        let mut j = i;
        let close_end: Option<usize> = loop {
            if j >= bytes.len() {
                break None;
            }
            if bytes[j] != b'`' {
                j += 1;
                continue;
            }
            let run_start = j;
            while j < bytes.len() && bytes[j] == b'`' {
                j += 1;
            }
            if j - run_start == opener_len {
                break Some(j);
            }
        };

        if let Some(close_end) = close_end {
            if opener_start > outside_start {
                segments.push(LineSegment::Outside(&line[outside_start..opener_start]));
            }
            segments.push(LineSegment::CodeSpan(&line[opener_start..close_end]));
            outside_start = close_end;
            i = close_end;
        }
    }

    if outside_start < line.len() {
        segments.push(LineSegment::Outside(&line[outside_start..]));
    }
    segments
}

fn substitute_line(line: &str, out: &mut String) {
    for segment in split_code_span_segments(line) {
        match segment {
            LineSegment::Outside(s) => substitute_outside_segment(s, out),
            LineSegment::CodeSpan(s) => out.push_str(s),
        }
    }
}

fn substitute_outside_segment(segment: &str, out: &mut String) {
    let tokens = parse_inline(segment);
    if tokens.iter().all(|t| matches!(t, InlineToken::Text(_))) {
        out.push_str(segment);
        return;
    }
    for token in tokens {
        match token {
            InlineToken::Text(s) => out.push_str(&s),
            InlineToken::Directive(d) => {
                let idx = INLINE_DIRECTIVES.with(|cell| {
                    let mut v = cell.borrow_mut();
                    let n = v.len();
                    v.push(d);
                    n
                });
                let _ = write!(out, "{PH_PREFIX}{idx}");
            }
        }
    }
}

/// Render one comrak block-level node to its ADF equivalent. Returns `None`
/// for nodes that should be silently dropped (e.g. empty paragraphs).
fn render_block<'a>(node: &'a AstNode<'a>) -> Option<Value> {
    let value = node.data.borrow().value.clone();
    match value {
        NodeValue::Paragraph => {
            let inline = render_inlines(node);
            if inline.is_empty() {
                None
            } else {
                Some(json!({"type": "paragraph", "content": inline}))
            }
        }
        NodeValue::Heading(h) => {
            let inline = render_inlines(node);
            Some(json!({
                "type": "heading",
                "attrs": {"level": h.level},
                "content": inline,
            }))
        }
        NodeValue::List(list) => match list.list_type {
            ListType::Bullet => {
                let items = render_list_items(node);
                Some(json!({"type": "bulletList", "content": items}))
            }
            ListType::Ordered => {
                let items = render_list_items(node);
                let start = list.start.max(1);
                Some(json!({
                    "type": "orderedList",
                    "attrs": {"order": start},
                    "content": items,
                }))
            }
        },
        NodeValue::Item(_) => Some(render_list_item(node)),
        NodeValue::TaskItem(task) => {
            // No first-class taskItem in ADF panels; fold to a listItem with
            // a `[ ]` / `[x]` text prefix on the first paragraph.
            let prefix = if task.symbol.is_some() {
                "[x] "
            } else {
                "[ ] "
            };
            Some(render_list_item_with_prefix(node, prefix))
        }
        NodeValue::CodeBlock(cb) => {
            let mut attrs = serde_json::Map::new();
            let info = cb.info.trim();
            if !info.is_empty() {
                attrs.insert("language".to_string(), Value::String(info.to_string()));
            }
            // Strip exactly one trailing newline that comrak appends to fenced
            // code blocks; preserve the rest of the literal verbatim.
            let mut literal = cb.literal.clone();
            if literal.ends_with('\n') {
                literal.pop();
            }
            let text_node = json!({"type": "text", "text": literal});
            let mut block = serde_json::Map::new();
            block.insert("type".to_string(), Value::String("codeBlock".to_string()));
            if !attrs.is_empty() {
                block.insert("attrs".to_string(), Value::Object(attrs));
            }
            block.insert("content".to_string(), Value::Array(vec![text_node]));
            Some(Value::Object(block))
        }
        NodeValue::BlockQuote => {
            let mut content = Vec::new();
            for child in node.children() {
                if let Some(child_node) = render_block(child) {
                    content.push(child_node);
                }
            }
            Some(json!({"type": "blockquote", "content": content}))
        }
        NodeValue::ThematicBreak => Some(json!({"type": "rule"})),
        NodeValue::Table(_) => Some(render_table(node)),
        NodeValue::TableRow(header) => Some(render_table_row(node, header)),
        NodeValue::TableCell => Some(render_table_cell(node, false)),
        NodeValue::HtmlBlock(html) => {
            // ADF has no raw HTML — preserve the literal as plain paragraph
            // text so the user can see the content survived.
            let literal = html.literal.trim_end_matches('\n').to_string();
            if literal.is_empty() {
                None
            } else {
                Some(json!({
                    "type": "paragraph",
                    "content": [{"type": "text", "text": literal}],
                }))
            }
        }
        _ => None,
    }
}

/// Walk the children of a List node and emit a `listItem` for each. Skips any
/// non-Item children defensively.
fn render_list_items<'a>(list_node: &'a AstNode<'a>) -> Vec<Value> {
    let mut items = Vec::new();
    for child in list_node.children() {
        let v = child.data.borrow().value.clone();
        match v {
            NodeValue::Item(_) => items.push(render_list_item(child)),
            NodeValue::TaskItem(task) => {
                let prefix = if task.symbol.is_some() {
                    "[x] "
                } else {
                    "[ ] "
                };
                items.push(render_list_item_with_prefix(child, prefix));
            }
            _ => {}
        }
    }
    items
}

fn render_list_item<'a>(item: &'a AstNode<'a>) -> Value {
    let mut content = Vec::new();
    for child in item.children() {
        if let Some(block) = render_block(child) {
            content.push(block);
        }
    }
    if content.is_empty() {
        // Every listItem must have at least one paragraph child.
        content.push(json!({"type": "paragraph", "content": []}));
    }
    json!({"type": "listItem", "content": content})
}

/// Like [`render_list_item`] but injects a `[ ]` / `[x]` prefix on the first
/// inline run of the first paragraph child.
fn render_list_item_with_prefix<'a>(item: &'a AstNode<'a>, prefix: &str) -> Value {
    let mut content = Vec::new();
    let mut prefixed = false;
    for child in item.children() {
        if let Some(mut block) = render_block(child) {
            if !prefixed && block["type"] == "paragraph" {
                if let Some(arr) = block["content"].as_array_mut() {
                    let prefix_node = json!({"type": "text", "text": prefix});
                    arr.insert(0, prefix_node);
                }
                prefixed = true;
            }
            content.push(block);
        }
    }
    if content.is_empty() {
        content.push(json!({
            "type": "paragraph",
            "content": [{"type": "text", "text": prefix}],
        }));
    }
    json!({"type": "listItem", "content": content})
}

fn render_table<'a>(table: &'a AstNode<'a>) -> Value {
    let mut rows = Vec::new();
    for child in table.children() {
        let v = child.data.borrow().value.clone();
        if let NodeValue::TableRow(is_header) = v {
            rows.push(render_table_row(child, is_header));
        }
    }
    json!({
        "type": "table",
        "attrs": {"layout": "default"},
        "content": rows,
    })
}

fn render_table_row<'a>(row: &'a AstNode<'a>, is_header: bool) -> Value {
    let mut cells = Vec::new();
    for child in row.children() {
        let v = child.data.borrow().value.clone();
        if matches!(v, NodeValue::TableCell) {
            cells.push(render_table_cell(child, is_header));
        }
    }
    json!({"type": "tableRow", "content": cells})
}

fn render_table_cell<'a>(cell: &'a AstNode<'a>, is_header: bool) -> Value {
    let inline = render_inlines(cell);
    let kind = if is_header {
        "tableHeader"
    } else {
        "tableCell"
    };
    json!({
        "type": kind,
        "content": [{
            "type": "paragraph",
            "content": inline,
        }],
    })
}

// =====================================================================
// Inline rendering
// =====================================================================

/// Render the inline children of a block-level comrak node into an ADF inline
/// array. Empty marks/attrs are omitted.
fn render_inlines<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
    let mut out = Vec::new();
    for child in node.children() {
        render_inline(child, &[], &mut out);
    }
    out
}

/// Recursive walker. `marks` is the accumulated list of marks from enclosing
/// `Strong` / `Emph` / `Strikethrough` / `Link` / `Underline` nodes that
/// should be applied to leaf `text` nodes.
fn render_inline<'a>(node: &'a AstNode<'a>, marks: &[Value], out: &mut Vec<Value>) {
    let value = node.data.borrow().value.clone();
    match value {
        NodeValue::Text(s) => emit_text_with_directives(&s, marks, out),
        NodeValue::SoftBreak => emit_text_with_directives(" ", marks, out),
        NodeValue::LineBreak => out.push(json!({"type": "hardBreak"})),
        NodeValue::Code(c) => {
            let mut new_marks = marks.to_vec();
            new_marks.push(json!({"type": "code"}));
            out.push(text_node(&c.literal, &new_marks));
        }
        NodeValue::Strong => {
            let mut new_marks = marks.to_vec();
            new_marks.push(json!({"type": "strong"}));
            for child in node.children() {
                render_inline(child, &new_marks, out);
            }
        }
        NodeValue::Emph => {
            let mut new_marks = marks.to_vec();
            new_marks.push(json!({"type": "em"}));
            for child in node.children() {
                render_inline(child, &new_marks, out);
            }
        }
        NodeValue::Strikethrough => {
            let mut new_marks = marks.to_vec();
            new_marks.push(json!({"type": "strike"}));
            for child in node.children() {
                render_inline(child, &new_marks, out);
            }
        }
        NodeValue::Underline => {
            let mut new_marks = marks.to_vec();
            new_marks.push(json!({"type": "underline"}));
            for child in node.children() {
                render_inline(child, &new_marks, out);
            }
        }
        NodeValue::Superscript => {
            let mut new_marks = marks.to_vec();
            new_marks.push(json!({"type": "subsup", "attrs": {"type": "sup"}}));
            for child in node.children() {
                render_inline(child, &new_marks, out);
            }
        }
        NodeValue::Subscript => {
            let mut new_marks = marks.to_vec();
            new_marks.push(json!({"type": "subsup", "attrs": {"type": "sub"}}));
            for child in node.children() {
                render_inline(child, &new_marks, out);
            }
        }
        NodeValue::Link(link) => {
            let mut new_marks = marks.to_vec();
            // Comrak parses `[text](url "title")` and reference defs of the
            // form `[ref]: url "title"` into the same NodeLink, exposing the
            // title via the `title` field. Attach it to the ADF link mark when
            // present so the round-trip preserves it.
            let mut attrs = serde_json::Map::new();
            attrs.insert("href".to_string(), Value::String(link.url));
            if !link.title.is_empty() {
                attrs.insert("title".to_string(), Value::String(link.title));
            }
            new_marks.push(json!({
                "type": "link",
                "attrs": attrs,
            }));
            for child in node.children() {
                render_inline(child, &new_marks, out);
            }
        }
        NodeValue::Image(img) => {
            // ADF's `mediaSingle` is a *block* node — emitting it inside
            // `paragraph.content` produces invalid ADF. Until block-level
            // image promotion is implemented, degrade inline images to a
            // text node containing the original markdown literal so the URL
            // and alt text survive. This is intentionally lossy.
            let alt = collect_inline_text(node);
            let literal = if alt.is_empty() {
                format!("![]({})", img.url)
            } else {
                format!("![{}]({})", alt, img.url)
            };
            push_text(&literal, marks, out);
        }
        NodeValue::HtmlInline(s) => {
            // ADF has no raw HTML inline; keep as text.
            emit_text_with_directives(&s, marks, out);
        }
        NodeValue::FootnoteReference(_) | NodeValue::WikiLink(_) => {
            // Recurse so leaf text still surfaces.
            for child in node.children() {
                render_inline(child, marks, out);
            }
        }
        _ => {
            for child in node.children() {
                render_inline(child, marks, out);
            }
        }
    }
}

/// Emit a `text` node, attaching `marks` only when non-empty.
fn text_node(text: &str, marks: &[Value]) -> Value {
    if marks.is_empty() {
        json!({"type": "text", "text": text})
    } else {
        json!({"type": "text", "text": text, "marks": marks})
    }
}

/// Emit a text run that may contain placeholder substrings left behind by
/// [`substitute_inline_directives`]. Each placeholder becomes the corresponding
/// inline-directive ADF node; the surrounding text becomes `text` nodes
/// carrying the accumulated marks.
fn emit_text_with_directives(text: &str, marks: &[Value], out: &mut Vec<Value>) {
    if text.is_empty() {
        return;
    }
    let mut rest = text;
    while let Some(pos) = rest.find(PH_PREFIX) {
        if pos > 0 {
            push_text(&rest[..pos], marks, out);
        }
        let after = &rest[pos + PH_PREFIX.len()..];
        // Read consecutive ASCII digits as the index.
        let digit_end = after
            .as_bytes()
            .iter()
            .take_while(|&&b| b.is_ascii_digit())
            .count();
        if digit_end == 0 {
            // Malformed placeholder — emit the prefix as text and continue.
            push_text(PH_PREFIX, marks, out);
            rest = after;
            continue;
        }
        let idx: usize = after[..digit_end].parse().unwrap_or(usize::MAX);
        let directive = INLINE_DIRECTIVES.with(|cell| cell.borrow().get(idx).cloned());
        match directive {
            Some(d) => match render_inline_directive(&d) {
                Some(node) => out.push(node),
                None => {
                    let literal = literal_inline_directive(&d);
                    if !literal.is_empty() {
                        push_text(&literal, marks, out);
                    }
                }
            },
            None => {
                // Index out of range — leave verbatim.
                let literal = format!("{PH_PREFIX}{}", &after[..digit_end]);
                push_text(&literal, marks, out);
            }
        }
        rest = &after[digit_end..];
    }
    if !rest.is_empty() {
        push_text(rest, marks, out);
    }
}

/// Recursively collect plain-text content from an inline AST node's children.
///
/// Used to recover the alt text from `NodeValue::Image` when degrading inline
/// images to literal markdown. Only text-bearing leaves contribute; marks and
/// inline structure are dropped because the alt text is purely descriptive.
fn collect_inline_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut buf = String::new();
    for child in node.children() {
        append_inline_text(child, &mut buf);
    }
    buf
}

fn append_inline_text<'a>(node: &'a AstNode<'a>, out: &mut String) {
    let value = node.data.borrow().value.clone();
    match value {
        NodeValue::Text(s) => out.push_str(&s),
        NodeValue::Code(NodeCode { literal, .. }) => out.push_str(&literal),
        NodeValue::SoftBreak | NodeValue::LineBreak => out.push(' '),
        _ => {
            for child in node.children() {
                append_inline_text(child, out);
            }
        }
    }
}

/// Append a text run with the given marks to `out`, coalescing with the
/// previous text node when its marks match.
fn push_text(s: &str, marks: &[Value], out: &mut Vec<Value>) {
    if s.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last["type"] == "text"
        && last.get("marks") == Some(&marks_value(marks))
        && let Some(prev) = last["text"].as_str()
    {
        let combined = format!("{prev}{s}");
        last["text"] = Value::String(combined);
        return;
    }
    out.push(text_node(s, marks));
}

/// Help [`emit_text_with_directives`] compare the marks slice against the
/// `marks` field of an existing JSON text node. Returns `Value::Null` when
/// `marks` is empty so the comparison still works (the existing text node
/// will not have a `marks` field, so its `.get("marks")` is `None`, not
/// `Some(Null)` — emit `Null` to mean "no marks").
fn marks_value(marks: &[Value]) -> Value {
    if marks.is_empty() {
        Value::Null
    } else {
        Value::Array(marks.to_vec())
    }
}

/// Render an inline directive to its ADF node. Returns `None` when the
/// directive has no inline ADF representation (e.g. `image` — see module-level
/// notes).
fn render_inline_directive(d: &InlineDirective) -> Option<Value> {
    match d.name.as_str() {
        "status" => Some(render_status_node(d)),
        "emoticon" => Some(render_emoji_node(d)),
        "mention" => Some(render_mention_node(d)),
        "link" => Some(render_inline_card_node(d)),
        "image" => None,
        _ => None,
    }
}

fn render_status_node(d: &InlineDirective) -> Value {
    let text = d.content.clone().unwrap_or_default();
    let raw_color = d.params.get("color").map(String::as_str).unwrap_or("");
    let color = match raw_color {
        "green" => "green",
        "red" => "red",
        "yellow" => "yellow",
        "blue" => "blue",
        "purple" => "purple",
        _ => "neutral",
    };
    json!({
        "type": "status",
        "attrs": {
            "text": text,
            "color": color,
        },
    })
}

fn render_emoji_node(d: &InlineDirective) -> Value {
    let name = d.params.get("name").map(String::as_str).unwrap_or("");
    let short = format!(":{name}:");
    json!({
        "type": "emoji",
        "attrs": {"shortName": short},
    })
}

fn render_mention_node(d: &InlineDirective) -> Value {
    let id = d
        .params
        .get("accountId")
        .cloned()
        .unwrap_or_else(String::new);
    let mut attrs = serde_json::Map::new();
    attrs.insert("id".to_string(), Value::String(id));
    if let Some(text) = d.content.as_ref() {
        attrs.insert("text".to_string(), Value::String(text.clone()));
    }
    json!({
        "type": "mention",
        "attrs": attrs,
    })
}

fn render_inline_card_node(d: &InlineDirective) -> Value {
    // Prefer an explicit url, otherwise synthesise `pageId:N` so the lossy
    // mapping is visible to humans inspecting the JSON.
    let url = d
        .params
        .get("url")
        .cloned()
        .or_else(|| d.params.get("pageId").map(|n| format!("pageId:{n}")));

    match url {
        Some(url) => json!({
            "type": "inlineCard",
            "attrs": {"url": url},
        }),
        None => {
            // Neither `url` nor `pageId` is set — emitting an inlineCard with
            // an empty URL is invalid ADF, so we degrade gracefully to a plain
            // text node carrying whatever label the user supplied. If the
            // body is empty too we still produce a (zero-length) text node;
            // the upstream renderer is responsible for filtering it out if
            // necessary.
            let text = d.content.clone().unwrap_or_default();
            json!({
                "type": "text",
                "text": text,
            })
        }
    }
}

/// Produce a `:name[content]{attrs}` literal for inline directives we can't
/// represent natively in ADF.
fn literal_inline_directive(d: &InlineDirective) -> String {
    let mut out = String::new();
    out.push(':');
    out.push_str(&d.name);
    if let Some(c) = d.content.as_ref() {
        out.push('[');
        out.push_str(c);
        out.push(']');
    }
    if !d.params.is_empty() {
        out.push('{');
        out.push_str(&crate::cli::commands::directives::render_attrs(&d.params));
        out.push('}');
    }
    out
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(md: &str) -> Value {
        markdown_to_adf(md).expect("conversion succeeded")
    }

    fn first_block(doc: &Value) -> &Value {
        &doc["content"][0]
    }

    // ---- document wrapper -------------------------------------------------

    #[test]
    fn empty_input_produces_empty_doc() {
        let doc = convert("");
        assert_eq!(doc["type"], "doc");
        assert_eq!(doc["version"], 1);
        assert!(doc["content"].as_array().unwrap().is_empty());
    }

    #[test]
    fn single_paragraph_doc() {
        let doc = convert("Hello world.");
        assert_eq!(doc["type"], "doc");
        assert_eq!(doc["version"], 1);
        let blocks = doc["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "paragraph");
    }

    // ---- block markdown ---------------------------------------------------

    #[test]
    fn heading_levels_one_through_six() {
        for level in 1..=6 {
            let md = format!("{} h", "#".repeat(level));
            let doc = convert(&md);
            let h = first_block(&doc);
            assert_eq!(h["type"], "heading", "level {level}: {h:?}");
            assert_eq!(h["attrs"]["level"], level);
            assert_eq!(h["content"][0]["type"], "text");
            assert_eq!(h["content"][0]["text"], "h");
        }
    }

    #[test]
    fn paragraph_with_text() {
        let doc = convert("Hello world.");
        let p = first_block(&doc);
        assert_eq!(p["type"], "paragraph");
        assert_eq!(p["content"][0]["type"], "text");
        assert_eq!(p["content"][0]["text"], "Hello world.");
    }

    #[test]
    fn bullet_list() {
        let doc = convert("- a\n- b");
        let list = first_block(&doc);
        assert_eq!(list["type"], "bulletList");
        let items = list["content"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        for item in items {
            assert_eq!(item["type"], "listItem");
        }
    }

    #[test]
    fn ordered_list() {
        let doc = convert("1. one\n2. two");
        let list = first_block(&doc);
        assert_eq!(list["type"], "orderedList");
        assert_eq!(list["attrs"]["order"], 1);
        let items = list["content"].as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn nested_lists() {
        let md = "- outer\n  - inner";
        let doc = convert(md);
        let outer = first_block(&doc);
        assert_eq!(outer["type"], "bulletList");
        let outer_items = outer["content"].as_array().unwrap();
        // The first listItem must contain a paragraph then a nested bulletList.
        let inner_blocks = outer_items[0]["content"].as_array().unwrap();
        let has_nested_list = inner_blocks
            .iter()
            .any(|b| b["type"] == "bulletList" || b["type"] == "orderedList");
        assert!(has_nested_list, "nested list missing in: {outer_items:#?}");
    }

    #[test]
    fn code_block_with_language() {
        let doc = convert("```rust\nfn main() {}\n```");
        let cb = first_block(&doc);
        assert_eq!(cb["type"], "codeBlock");
        assert_eq!(cb["attrs"]["language"], "rust");
        assert_eq!(cb["content"][0]["type"], "text");
        assert_eq!(cb["content"][0]["text"], "fn main() {}");
    }

    #[test]
    fn code_block_without_language_omits_attrs() {
        let doc = convert("```\nplain\n```");
        let cb = first_block(&doc);
        assert_eq!(cb["type"], "codeBlock");
        assert!(
            cb.get("attrs").is_none() || cb["attrs"].as_object().is_none_or(|m| m.is_empty()),
            "expected no attrs, got: {cb:?}"
        );
        assert_eq!(cb["content"][0]["text"], "plain");
    }

    #[test]
    fn blockquote_with_paragraph() {
        let doc = convert("> hi");
        let bq = first_block(&doc);
        assert_eq!(bq["type"], "blockquote");
        let inner = &bq["content"][0];
        assert_eq!(inner["type"], "paragraph");
    }

    #[test]
    fn horizontal_rule() {
        let doc = convert("---");
        let hr = first_block(&doc);
        assert_eq!(hr["type"], "rule");
    }

    #[test]
    fn simple_table() {
        let md = "| a | b |\n|---|---|\n| 1 | 2 |";
        let doc = convert(md);
        let table = first_block(&doc);
        assert_eq!(table["type"], "table");
        assert_eq!(table["attrs"]["layout"], "default");
        let rows = table["content"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["type"], "tableRow");
        // First row is header.
        let header_cells = rows[0]["content"].as_array().unwrap();
        assert_eq!(header_cells[0]["type"], "tableHeader");
        // Second row is body.
        let body_cells = rows[1]["content"].as_array().unwrap();
        assert_eq!(body_cells[0]["type"], "tableCell");
    }

    // ---- inline marks -----------------------------------------------------

    fn text_node_at(doc: &Value, idx: usize) -> Value {
        doc["content"][0]["content"][idx].clone()
    }

    #[test]
    fn bold_emits_strong_mark() {
        let doc = convert("**bold**");
        let t = text_node_at(&doc, 0);
        assert_eq!(t["type"], "text");
        assert_eq!(t["text"], "bold");
        let marks = t["marks"].as_array().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0]["type"], "strong");
    }

    #[test]
    fn italic_emits_em_mark() {
        let doc = convert("*italic*");
        let t = text_node_at(&doc, 0);
        assert_eq!(t["text"], "italic");
        assert_eq!(t["marks"][0]["type"], "em");
    }

    #[test]
    fn inline_code_emits_code_mark() {
        let doc = convert("`code`");
        let t = text_node_at(&doc, 0);
        assert_eq!(t["text"], "code");
        assert_eq!(t["marks"][0]["type"], "code");
    }

    #[test]
    fn strikethrough_emits_strike_mark() {
        let doc = convert("~~bye~~");
        let t = text_node_at(&doc, 0);
        assert_eq!(t["text"], "bye");
        assert_eq!(t["marks"][0]["type"], "strike");
    }

    #[test]
    fn link_emits_link_mark() {
        let doc = convert("[t](https://example.com)");
        let t = text_node_at(&doc, 0);
        assert_eq!(t["type"], "text");
        assert_eq!(t["text"], "t");
        let marks = t["marks"].as_array().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0]["type"], "link");
        assert_eq!(marks[0]["attrs"]["href"], "https://example.com");
    }

    #[test]
    fn combined_bold_italic() {
        let doc = convert("**_both_**");
        let t = text_node_at(&doc, 0);
        assert_eq!(t["text"], "both");
        let marks: Vec<&str> = t["marks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["type"].as_str().unwrap())
            .collect();
        assert!(marks.contains(&"strong"));
        assert!(marks.contains(&"em"));
    }

    #[test]
    fn hard_break_emits_hard_break() {
        // Two trailing spaces produce a hard break in CommonMark.
        let doc = convert("a  \nb");
        let inline = doc["content"][0]["content"].as_array().unwrap();
        let has_hard_break = inline.iter().any(|n| n["type"] == "hardBreak");
        assert!(has_hard_break, "hard break missing in inline: {inline:#?}");
    }

    #[test]
    fn plain_text_omits_marks_field() {
        let doc = convert("plain");
        let t = text_node_at(&doc, 0);
        assert!(
            t.get("marks").is_none(),
            "plain text must not carry marks: {t:?}"
        );
    }

    // ---- block directives -------------------------------------------------

    #[test]
    fn block_info_panel() {
        let doc = convert(":::info\nbody\n:::");
        let panel = first_block(&doc);
        assert_eq!(panel["type"], "panel");
        assert_eq!(panel["attrs"]["panelType"], "info");
        let body = panel["content"].as_array().unwrap();
        assert_eq!(body.len(), 1);
        assert_eq!(body[0]["type"], "paragraph");
    }

    #[test]
    fn block_warning_panel_with_title_prepends_strong_paragraph() {
        let doc = convert(":::warning title=\"Heads up\"\nbody\n:::");
        let panel = first_block(&doc);
        assert_eq!(panel["attrs"]["panelType"], "warning");
        let body = panel["content"].as_array().unwrap();
        // First child is a paragraph with a strong-marked text "Heads up".
        let title_para = &body[0];
        assert_eq!(title_para["type"], "paragraph");
        let title_text = &title_para["content"][0];
        assert_eq!(title_text["text"], "Heads up");
        let marks = title_text["marks"].as_array().unwrap();
        assert_eq!(marks[0]["type"], "strong");
        // Body is preserved.
        assert!(body.len() >= 2);
    }

    #[test]
    fn block_note_panel() {
        let doc = convert(":::note\nbody\n:::");
        let panel = first_block(&doc);
        assert_eq!(panel["attrs"]["panelType"], "note");
    }

    #[test]
    fn block_tip_maps_to_success_panel() {
        let doc = convert(":::tip\nbody\n:::");
        let panel = first_block(&doc);
        assert_eq!(panel["type"], "panel");
        assert_eq!(panel["attrs"]["panelType"], "success");
    }

    #[test]
    fn nested_expand_with_panel_inside() {
        let doc = convert(":::expand title=\"Outer\"\n:::info\nInner.\n:::\n:::");
        let outer = first_block(&doc);
        assert_eq!(outer["type"], "expand");
        assert_eq!(outer["attrs"]["title"], "Outer");
        let inner = &outer["content"][0];
        assert_eq!(inner["type"], "panel");
        assert_eq!(inner["attrs"]["panelType"], "info");
    }

    #[test]
    fn block_toc_emits_extension_node() {
        let doc = convert(":::toc maxLevel=3\n:::");
        let ext = first_block(&doc);
        assert_eq!(ext["type"], "extension");
        assert_eq!(ext["attrs"]["extensionKey"], "toc");
        assert_eq!(
            ext["attrs"]["extensionType"],
            "com.atlassian.confluence.macro.core"
        );
        assert_eq!(
            ext["attrs"]["parameters"]["macroParams"]["maxLevel"]["value"],
            "3"
        );
    }

    #[test]
    fn block_toc_default_max_level() {
        let doc = convert(":::toc\n:::");
        let ext = first_block(&doc);
        assert_eq!(
            ext["attrs"]["parameters"]["macroParams"]["maxLevel"]["value"],
            "3"
        );
    }

    #[test]
    fn unknown_block_directive_passes_through_as_blockquote_fallback() {
        // The block lexer routes unknown names as Lines, so the literal
        // `:::custom` shows up in the AST as plain markdown — it never reaches
        // the directive renderer at all. Verify it doesn't become a panel.
        let doc = convert(":::custom\nbody\n:::");
        let blocks = doc["content"].as_array().unwrap();
        let any_panel = blocks.iter().any(|b| b["type"] == "panel");
        assert!(
            !any_panel,
            "unknown directive must NOT become a panel: {blocks:#?}"
        );
    }

    #[test]
    fn known_directive_with_no_adf_mapping_falls_back() {
        // `mention` is registered as inline; if it ever appears as a block
        // (unusual), the renderer must not crash and must not produce a panel.
        let doc = convert(":::mention\nx\n:::");
        let blocks = doc["content"].as_array().unwrap();
        // Either fallback blockquote or paragraph — but not panel.
        assert!(blocks.iter().all(|b| b["type"] != "panel"));
    }

    // ---- inline directives -----------------------------------------------

    #[test]
    fn inline_status_directive() {
        let doc = convert(":status[DONE]{color=green}");
        let inline = &doc["content"][0]["content"][0];
        assert_eq!(inline["type"], "status");
        assert_eq!(inline["attrs"]["text"], "DONE");
        assert_eq!(inline["attrs"]["color"], "green");
    }

    #[test]
    fn inline_status_unknown_color_maps_to_neutral() {
        let doc = convert(":status[X]{color=mauve}");
        let inline = &doc["content"][0]["content"][0];
        assert_eq!(inline["attrs"]["color"], "neutral");
    }

    #[test]
    fn inline_emoticon_emits_emoji_node() {
        let doc = convert(":emoticon{name=warning}");
        let inline = &doc["content"][0]["content"][0];
        assert_eq!(inline["type"], "emoji");
        assert_eq!(inline["attrs"]["shortName"], ":warning:");
    }

    #[test]
    fn inline_mention_emits_mention_node() {
        let doc = convert(":mention[@john]{accountId=abc123}");
        let inline = &doc["content"][0]["content"][0];
        assert_eq!(inline["type"], "mention");
        assert_eq!(inline["attrs"]["id"], "abc123");
        assert_eq!(inline["attrs"]["text"], "@john");
    }

    #[test]
    fn inline_link_with_page_id_emits_inline_card_with_synthetic_url() {
        let doc = convert(":link[Title]{pageId=12345}");
        let inline = &doc["content"][0]["content"][0];
        assert_eq!(inline["type"], "inlineCard");
        assert_eq!(inline["attrs"]["url"], "pageId:12345");
    }

    #[test]
    fn inline_link_with_explicit_url_wins() {
        let doc = convert(":link[Title]{url=\"https://example.com\"}");
        let inline = &doc["content"][0]["content"][0];
        assert_eq!(inline["type"], "inlineCard");
        assert_eq!(inline["attrs"]["url"], "https://example.com");
    }

    #[test]
    fn inline_link_without_url_or_page_id_falls_back_to_text() {
        // Regression: `:link[fallback]{}` (or any link directive without
        // `url` / `pageId`) used to emit `{"type":"inlineCard","attrs":
        // {"url":""}}`, which ADF consumers reject. We now degrade to a plain
        // text node so the label isn't lost and the document remains valid.
        let doc = convert(":link[fallback]");
        let inline = &doc["content"][0]["content"][0];
        assert_eq!(inline["type"], "text", "expected text fallback: {inline:?}");
        assert_eq!(inline["text"], "fallback");
        // Nothing in the doc should be an inlineCard with empty url.
        let blocks = doc["content"].as_array().unwrap();
        for block in blocks {
            if let Some(content) = block.get("content").and_then(|v| v.as_array()) {
                for node in content {
                    if node["type"] == "inlineCard" {
                        let url = node["attrs"]["url"].as_str().unwrap_or_default();
                        assert!(!url.is_empty(), "must not emit empty-url inlineCard");
                    }
                }
            }
        }
    }

    #[test]
    fn unknown_inline_directive_round_trips_as_text() {
        // The inline parser leaves unknown names as text, so the literal
        // `:custom[x]` survives to the comrak pass and ends up as plain text
        // in the paragraph content.
        let doc = convert(":custom[x] tail");
        let inline = doc["content"][0]["content"].as_array().unwrap();
        // No structured `custom` node; first text node contains the literal.
        let any_custom = inline.iter().any(|n| n["type"] == "custom");
        assert!(!any_custom);
        let any_text_with_literal = inline
            .iter()
            .any(|n| n["type"] == "text" && n["text"].as_str().unwrap_or("").contains(":custom"));
        assert!(
            any_text_with_literal,
            "literal :custom[x] should survive: {inline:#?}"
        );
    }

    // ---- code-fence escape ------------------------------------------------

    #[test]
    fn directive_inside_backtick_fence_is_literal_code() {
        let doc = convert("```\n:::info\n```");
        let cb = first_block(&doc);
        assert_eq!(cb["type"], "codeBlock");
        assert!(
            cb["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(":::info"),
            "got: {cb:?}"
        );
        // No panel anywhere in the doc.
        let any_panel = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["type"] == "panel");
        assert!(!any_panel);
    }

    #[test]
    fn directive_inside_tilde_fence_is_literal_code() {
        let doc = convert("~~~\n:::info\n~~~");
        let cb = first_block(&doc);
        assert_eq!(cb["type"], "codeBlock");
        assert!(
            cb["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(":::info"),
            "got: {cb:?}"
        );
    }

    // ---- edge cases -------------------------------------------------------

    #[test]
    fn whitespace_only_input_yields_empty_content() {
        let doc = convert("   \n\n  \n");
        assert!(doc["content"].as_array().unwrap().is_empty());
    }

    #[test]
    fn just_a_directive_no_surrounding_text() {
        let doc = convert(":::info\nbody\n:::");
        let blocks = doc["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "panel");
    }

    #[test]
    fn unclosed_directive_returns_err() {
        let err = markdown_to_adf(":::info\nbody").unwrap_err();
        match err {
            MdToAdfError::Directive(DirectiveError::Unclosed { name, .. }) => {
                assert_eq!(name, "info");
            }
            other => panic!("expected Unclosed, got: {other:?}"),
        }
    }

    #[test]
    fn mixed_paragraphs_and_directives() {
        let md = "Before.\n\n:::info\nIn panel\n:::\n\nAfter.";
        let doc = convert(md);
        let blocks = doc["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0]["type"], "paragraph");
        assert_eq!(blocks[1]["type"], "panel");
        assert_eq!(blocks[2]["type"], "paragraph");
    }

    #[test]
    fn inline_status_inside_paragraph_with_text_around_it() {
        let doc = convert("Hi :status[OK]{color=green} bye");
        let inline = doc["content"][0]["content"].as_array().unwrap();
        // Expect: text "Hi ", status, text " bye"
        assert!(inline.iter().any(|n| n["type"] == "status"));
        assert!(
            inline
                .iter()
                .any(|n| n["type"] == "text" && n["text"] == "Hi ")
        );
        assert!(
            inline
                .iter()
                .any(|n| n["type"] == "text" && n["text"] == " bye")
        );
    }

    #[test]
    fn multiple_inline_directives_in_one_paragraph() {
        let doc = convert(":status[OK]{color=green} and :emoticon{name=warning}");
        let inline = doc["content"][0]["content"].as_array().unwrap();
        let has_status = inline.iter().any(|n| n["type"] == "status");
        let has_emoji = inline.iter().any(|n| n["type"] == "emoji");
        assert!(has_status);
        assert!(has_emoji);
    }

    #[test]
    fn directive_after_closed_code_fence_still_works() {
        let md = "```\nplain\n```\n\n:::info\nhi\n:::";
        let doc = convert(md);
        let blocks = doc["content"].as_array().unwrap();
        assert!(blocks.iter().any(|b| b["type"] == "codeBlock"));
        assert!(blocks.iter().any(|b| b["type"] == "panel"));
    }

    #[test]
    fn doc_wrapper_has_correct_keys() {
        let doc = convert("hi");
        let obj = doc.as_object().unwrap();
        assert!(obj.contains_key("type"));
        assert!(obj.contains_key("version"));
        assert!(obj.contains_key("content"));
    }

    #[test]
    fn inline_image_degrades_to_text_not_media_single() {
        // Bug 3: ADF's `mediaSingle` is block-level. Emitting it inside
        // `paragraph.content` produces invalid ADF, so we degrade inline
        // images to a text node carrying the original markdown literal.
        let doc = convert("Hello ![alt](http://x/y.png) world");
        let para = first_block(&doc);
        assert_eq!(para["type"], "paragraph");
        let inline = para["content"].as_array().unwrap();
        // No node in the paragraph may be `mediaSingle` or `media`.
        for node in inline {
            assert_ne!(node["type"], "mediaSingle", "found mediaSingle: {node:?}");
            assert_ne!(node["type"], "media", "found media: {node:?}");
        }
        // The original markdown literal should be preserved as text.
        let joined: String = inline
            .iter()
            .filter(|n| n["type"] == "text")
            .filter_map(|n| n["text"].as_str())
            .collect();
        assert!(
            joined.contains("![alt](http://x/y.png)"),
            "joined inline text was: {joined:?}",
        );
    }

    // ---- reference-style links -------------------------------------------

    #[test]
    fn reference_style_link_resolves_to_link_mark() {
        let md = "See [docs][d].\n\n[d]: https://example.com/docs";
        let doc = convert(md);
        // No paragraph in the document should echo the literal `[d]:` def.
        for block in doc["content"].as_array().unwrap() {
            if block["type"] != "paragraph" {
                continue;
            }
            let joined: String = block["content"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|n| n["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            assert!(
                !joined.contains("[d]: https://example.com/docs"),
                "reference def must not appear as text: {joined:?}"
            );
        }
        let inline = doc["content"][0]["content"].as_array().unwrap();
        // Expect: text "See ", text "docs" with link mark, text "."
        let leading = inline
            .iter()
            .find(|n| n["type"] == "text" && n["text"] == "See ")
            .unwrap_or_else(|| panic!("missing 'See ' text run: {inline:#?}"));
        assert!(leading.get("marks").is_none());
        let linked = inline
            .iter()
            .find(|n| n["type"] == "text" && n["text"] == "docs")
            .unwrap_or_else(|| panic!("missing linked 'docs' text run: {inline:#?}"));
        let marks = linked["marks"].as_array().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0]["type"], "link");
        assert_eq!(marks[0]["attrs"]["href"], "https://example.com/docs");
        let trailing = inline
            .iter()
            .find(|n| n["type"] == "text" && n["text"] == ".")
            .unwrap_or_else(|| panic!("missing trailing '.' text run: {inline:#?}"));
        assert!(trailing.get("marks").is_none());
    }

    #[test]
    fn reference_style_link_with_title_attr_resolves() {
        let md = "See [docs][d].\n\n[d]: https://example.com/docs \"Doc title\"";
        let doc = convert(md);
        let inline = doc["content"][0]["content"].as_array().unwrap();
        let linked = inline
            .iter()
            .find(|n| n["type"] == "text" && n["text"] == "docs")
            .unwrap_or_else(|| panic!("missing linked 'docs': {inline:#?}"));
        let mark = &linked["marks"][0];
        assert_eq!(mark["type"], "link");
        assert_eq!(mark["attrs"]["href"], "https://example.com/docs");
        assert_eq!(
            mark["attrs"]["title"], "Doc title",
            "title from ref def must be preserved on the link mark: {mark:?}"
        );
    }

    #[test]
    fn shortcut_reference_link_resolves() {
        // `[docs]` with a matching `[docs]:` def — the shortcut form (no
        // second bracket pair) must still resolve to a link mark.
        let md = "[docs]\n\n[docs]: https://example.com/docs";
        let doc = convert(md);
        let inline = doc["content"][0]["content"].as_array().unwrap();
        assert_eq!(inline.len(), 1, "expected single linked run: {inline:#?}");
        let linked = &inline[0];
        assert_eq!(linked["type"], "text");
        assert_eq!(linked["text"], "docs");
        let marks = linked["marks"].as_array().unwrap();
        assert_eq!(marks[0]["type"], "link");
        assert_eq!(marks[0]["attrs"]["href"], "https://example.com/docs");
    }

    #[test]
    fn reference_style_image_resolves() {
        // ADF's mediaSingle is a block node; emitting it inline produces
        // invalid ADF. Inline images therefore degrade to a literal markdown
        // text run. The reference-style form must still resolve through
        // comrak's ref map so the alt + src survive inside that literal —
        // they must not surface as the unresolved `![alt][img]` form.
        let md = "![alt][img]\n\n[img]: img.png";
        let doc = convert(md);
        let para = first_block(&doc);
        assert_eq!(para["type"], "paragraph");
        let inline = para["content"].as_array().unwrap();
        // No mediaSingle / media node may appear inside the paragraph.
        for node in inline {
            assert_ne!(node["type"], "mediaSingle");
            assert_ne!(node["type"], "media");
        }
        let joined: String = inline
            .iter()
            .filter(|n| n["type"] == "text")
            .filter_map(|n| n["text"].as_str())
            .collect();
        assert!(
            joined.contains("![alt](img.png)"),
            "alt + src must survive as resolved literal markdown: {joined:?}"
        );
        // The unresolved ref form must not leak through.
        assert!(
            !joined.contains("![alt][img]"),
            "unresolved reference form must not appear: {joined:?}"
        );
    }

    #[test]
    fn unresolved_reference_falls_back_to_text() {
        // No `[missing]:` def — must not crash, must not emit a link mark,
        // and must surface the literal `[unresolved][missing]` so the user
        // sees their input wasn't silently dropped.
        let md = "[unresolved][missing]";
        let doc = convert(md);
        let para = first_block(&doc);
        assert_eq!(para["type"], "paragraph");
        let inline = para["content"].as_array().unwrap();
        // No link mark anywhere in the inline content.
        for node in inline {
            if let Some(marks) = node.get("marks").and_then(|v| v.as_array()) {
                for mk in marks {
                    assert_ne!(
                        mk["type"], "link",
                        "unresolved ref must not produce a link mark: {node:?}"
                    );
                }
            }
        }
        let joined: String = inline
            .iter()
            .filter(|n| n["type"] == "text")
            .filter_map(|n| n["text"].as_str())
            .collect();
        assert!(
            joined.contains("[unresolved][missing]"),
            "literal ref text should survive: {joined:?}"
        );
    }

    #[test]
    fn reference_def_does_not_appear_as_paragraph() {
        // Given a doc with a ref def line, no paragraph block in the output
        // should contain the def text (`[d]: ...`). Comrak consumes the def
        // out of the AST during reference resolution.
        let md = "See [docs][d].\n\n[d]: https://example.com/docs";
        let doc = convert(md);
        for block in doc["content"].as_array().unwrap() {
            if block["type"] != "paragraph" {
                continue;
            }
            let joined: String = block["content"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|n| n["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            assert!(
                !joined.contains("[d]:"),
                "ref def `[d]: ...` must not appear as a paragraph: {joined:?}"
            );
        }
    }

    #[test]
    fn inline_directive_inside_code_span_is_preserved_verbatim() {
        // Bug 5: pre-pass must skip inline code spans so `:status[…]` inside
        // them stays as code-span content rather than being rewritten into
        // ADF storage XML.
        let doc = convert("Run `:status[DONE]` to mark done.");
        let para = first_block(&doc);
        assert_eq!(para["type"], "paragraph");
        let inline = para["content"].as_array().unwrap();
        // No `status` node should appear — the directive lives inside a code span.
        for node in inline {
            assert_ne!(node["type"], "status", "found status node: {node:?}");
        }
        // A text run with the `code` mark should carry the literal `:status[DONE]`.
        let has_code_directive = inline.iter().any(|n| {
            n["type"] == "text"
                && n["text"].as_str() == Some(":status[DONE]")
                && n["marks"]
                    .as_array()
                    .map(|m| m.iter().any(|mk| mk["type"] == "code"))
                    .unwrap_or(false)
        });
        assert!(has_code_directive, "got: {inline:?}");
    }
}
