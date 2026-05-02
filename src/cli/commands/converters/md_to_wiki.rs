//! Markdown (with MyST-style directive extensions) → Jira wiki text.
//!
//! Jira wiki is the legacy text markup used by Jira Server / DC and still
//! accepted by Jira Cloud's REST API for issue descriptions and comments. The
//! syntax uses single-line tokens (`h1.`, `*bold*`, `{code:lang}…{code}`,
//! `[text|url]`) and paired block macros (`{info}…{info}`, `{quote}…{quote}`).
//!
//! # Conversion strategy
//!
//! 1. **Code-fence-aware line walk.** Lines inside a CommonMark fenced code
//!    block (` ``` ` / `~~~`) bypass the directive lexer so a literal
//!    `:::info` inside a code block round-trips as code.
//! 2. **Tree building.** `Open` / `Close` / `Line` events fold into a nested
//!    tree of [`Node::Directive`] / [`Node::Text`].
//! 3. **Recursive render.** Each text chunk runs an inline-directive pre-pass
//!    (substituting `ATLINLPLACEHOLDER{n}` placeholders), feeds the
//!    placeholder-substituted markdown through comrak's AST walker, then
//!    swaps the placeholders for their wiki rendering.
//!
//! # Directive → wiki mapping
//!
//! Block directives:
//!
//! - `:::info` / `:::warning` / `:::note` / `:::tip` → `{info}…{info}` (and
//!   warning/note/tip equivalents). A `title=` attribute becomes a `title=`
//!   parameter on the macro: `{info:title=Heads up}…{info}`. Multiple
//!   parameters are joined with `|`: `{info:title=A|key=val}`.
//! - `:::expand title="X"` → no native Jira wiki expand macro. We fall back
//!   to `*X*\n\nbody` so the title and body still survive (lossy).
//! - `:::toc` → `{toc}`; `:::toc maxLevel=3` → `{toc:maxLevel=3}`.
//! - Unknown block names pass through as the literal `:::name {attrs}` /
//!   body / `:::` markdown.
//!
//! Inline directives:
//!
//! - `:status[DONE]{color=green}` → `{status:colour=Green|title=DONE}`. Color
//!   names are mapped to Jira's expected casing (`Green`, `Red`, …); unknown
//!   colors lowercase-pass through. With no color we omit the parameter and
//!   Jira renders the default grey badge.
//! - `:emoticon{name=N}` → Jira shortcut emoticons. Map: `warning`→`(!)`,
//!   `tick`→`(/)`, `cross`→`(x)`, `info`→`(i)`, `question`→`(?)`. Other names
//!   fall back to the literal `:N:` shortcut.
//! - `:mention[@john]{accountId=abc}` → `[~accountid:abc]` (Cloud format).
//!   With `username=` instead: `[~username]`. With neither: empty `[~]`.
//! - `:link[Title]{url=…}` → `[Title|url]`. With only `pageId=N`: `[Title]`
//!   (Jira tries to resolve the bare title — lossy fallback).
//! - `:image{src=…}` → `!src!`. With an `alt=`: `!src|alt=…!`.
//!
//! All conversions are best-effort and never fail except on directive grammar
//! errors (an unclosed `:::name` block).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use comrak::nodes::{AstNode, ListType, NodeValue};
use comrak::{Arena, Options, parse_document};
use thiserror::Error;

use crate::cli::commands::directives::{
    BlockEvent, BlockLexer, DirectiveError, DirectiveSpec, InlineDirective, InlineToken, lookup,
    parse_inline,
};

/// Placeholder substring used to mark where an inline directive lived in the
/// original markdown. Substituted by [`substitute_inline_directives`] before
/// comrak runs and post-processed back into wiki tokens after rendering.
/// The text is plain ASCII with no markdown specials so comrak passes it
/// through unchanged.
const PH_PREFIX: &str = "ATLINLPLACEHOLDER";

thread_local! {
    /// Buffer of inline directives extracted by [`substitute_inline_directives`]
    /// for the current `render_md_chunk` call. Indexed by the integer that
    /// follows `ATLINLPLACEHOLDER` in placeholder text. Cleared at the start
    /// of every `render_md_chunk` invocation so render calls never see stale
    /// directives.
    static INLINE_DIRECTIVES: RefCell<Vec<InlineDirective>> = const { RefCell::new(Vec::new()) };
}

// =====================================================================
// Errors
// =====================================================================

/// Errors returned by [`markdown_to_wiki`].
#[derive(Debug, Error)]
pub enum MdToWikiError {
    /// A directive grammar error (e.g. unclosed `:::name` block) was found.
    #[error(transparent)]
    Directive(#[from] DirectiveError),
}

// =====================================================================
// Public API
// =====================================================================

/// Convert markdown (with MyST-style directive extensions) to Jira wiki text.
///
/// Block directives (`:::info`, `:::warning`, `:::tip`, …) become Jira wiki
/// block macros (`{info}…{info}` etc.). Inline directives
/// (`:status[DONE]{color=green}`, `:emoticon{name=warning}`, mentions) become
/// their wiki equivalents. Unknown directive names pass through as their
/// original markdown text.
///
/// Returns an error only on unrecoverable directive grammar issues
/// (specifically, an unclosed `:::name` block fence). Conversion of plain
/// markdown is always infallible.
///
/// # Examples
///
/// ```ignore
/// use atl::cli::commands::converters::md_to_wiki::markdown_to_wiki;
///
/// let wiki = markdown_to_wiki(":::info\nHello\n:::").unwrap();
/// assert!(wiki.contains("{info}"));
/// assert!(wiki.contains("Hello"));
/// ```
pub fn markdown_to_wiki(md: &str) -> Result<String, MdToWikiError> {
    let events = lex_with_code_fences(md)?;
    let tree = build_tree(events);
    let mut out = render(&tree);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

// =====================================================================
// Stage 1: code-fence-aware line walk (mirror of md_to_storage / md_to_adf)
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
// Stage 3: tree → wiki rendering
// =====================================================================

fn render(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(md) => out.push_str(&render_md_chunk(md)),
            Node::Directive {
                name,
                spec,
                params,
                children,
            } => {
                let body = render(children);
                out.push_str(&render_directive(name, *spec, params, &body));
            }
        }
    }
    out
}

/// Render a recognised block directive (`:::info`, `:::expand`, `:::toc`, …)
/// as Jira wiki, falling back to the original literal for unknown names.
fn render_directive(
    name: &str,
    spec: Option<&'static DirectiveSpec>,
    params: &BTreeMap<String, String>,
    body: &str,
) -> String {
    match (name, spec) {
        ("info" | "warning" | "note" | "tip", Some(spec)) => render_panel(spec, params, body),
        ("expand", _) => render_expand(params, body),
        ("toc", _) => render_toc(params),
        _ => render_unknown_block(name, params, body),
    }
}

/// `{info}…{info}` (or warning/note/tip) with optional `title=` and other
/// `key=val` parameters.
fn render_panel(spec: &DirectiveSpec, params: &BTreeMap<String, String>, body: &str) -> String {
    let macro_name = spec.jira_wiki_block.unwrap_or("info");
    let mut out = String::new();
    out.push('{');
    out.push_str(macro_name);
    if !params.is_empty() {
        out.push(':');
        out.push_str(&render_wiki_params(params));
    }
    out.push_str("}\n");
    out.push_str(body);
    if !body.is_empty() && !body.ends_with('\n') {
        out.push('\n');
    }
    out.push('{');
    out.push_str(macro_name);
    out.push_str("}\n");
    out
}

/// Jira wiki has no native expand macro. Emit the title (if any) as a bold
/// line followed by the body so neither is silently lost.
fn render_expand(params: &BTreeMap<String, String>, body: &str) -> String {
    let mut out = String::new();
    if let Some(title) = params.get("title")
        && !title.is_empty()
    {
        out.push('*');
        out.push_str(title);
        out.push_str("*\n\n");
    }
    out.push_str(body);
    if !body.is_empty() && !body.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// `:::toc` → `{toc}` (no params) or `{toc:k=v|...}`.
fn render_toc(params: &BTreeMap<String, String>) -> String {
    if params.is_empty() {
        return "{toc}\n".to_string();
    }
    format!("{{toc:{}}}\n", render_wiki_params(params))
}

/// Render a parameter map as `k1=v1|k2=v2`. Keys come out in `BTreeMap` order
/// (alphabetical). Values are emitted verbatim — Jira wiki params don't use
/// quoting; values continue until the next `|` or `}`.
fn render_wiki_params(params: &BTreeMap<String, String>) -> String {
    let mut parts = Vec::with_capacity(params.len());
    for (k, v) in params {
        parts.push(format!("{k}={v}"));
    }
    parts.join("|")
}

/// Unknown block name fallback. Echoes the original `:::name {attrs}` /
/// body / `:::` so the user sees their input survived.
fn render_unknown_block(name: &str, params: &BTreeMap<String, String>, body: &str) -> String {
    let mut out = String::new();
    out.push_str(":::");
    out.push_str(name);
    if !params.is_empty() {
        out.push(' ');
        out.push_str(&crate::cli::commands::directives::render_attrs(params));
    }
    out.push('\n');
    out.push_str(body);
    if !body.is_empty() && !body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(":::\n");
    out
}

// =====================================================================
// Stage 4: markdown chunk → wiki via comrak (with inline-directive pre/post)
// =====================================================================

fn render_md_chunk(md: &str) -> String {
    if md.is_empty() {
        return String::new();
    }
    INLINE_DIRECTIVES.with(|cell| cell.borrow_mut().clear());
    let with_placeholders = substitute_inline_directives(md);

    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;

    let root = parse_document(&arena, &with_placeholders, &options);
    let mut wiki = String::new();
    render_block_children(root, &mut wiki, 0);
    if !wiki.ends_with('\n') {
        wiki.push('\n');
    }
    replace_placeholders(&wiki)
}

/// Walk `md` line by line and replace every inline directive token with a
/// placeholder of the form `ATLINLPLACEHOLDER{n}`. Lines inside a CommonMark
/// fenced code block are passed through verbatim so that `:status[…]` inside
/// a fence stays as code text. The extracted [`InlineDirective`] values are
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

fn substitute_line(line: &str, out: &mut String) {
    let tokens = parse_inline(line);
    if tokens.iter().all(|t| matches!(t, InlineToken::Text(_))) {
        out.push_str(line);
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

/// Walk `wiki` and substitute every `ATLINLPLACEHOLDER{n}` substring with
/// the rendered wiki form of the directive at index `n`.
fn replace_placeholders(wiki: &str) -> String {
    let mut out = String::with_capacity(wiki.len());
    let mut rest = wiki;
    while let Some(pos) = rest.find(PH_PREFIX) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + PH_PREFIX.len()..];
        let digit_end = after
            .as_bytes()
            .iter()
            .take_while(|&&b| b.is_ascii_digit())
            .count();
        if digit_end == 0 {
            // Malformed placeholder — emit prefix as text and continue.
            out.push_str(PH_PREFIX);
            rest = after;
            continue;
        }
        let idx: usize = after[..digit_end].parse().unwrap_or(usize::MAX);
        let directive = INLINE_DIRECTIVES.with(|cell| cell.borrow().get(idx).cloned());
        match directive {
            Some(d) => out.push_str(&render_inline_directive(&d)),
            None => {
                // Index out of range — keep the literal.
                out.push_str(PH_PREFIX);
                out.push_str(&after[..digit_end]);
            }
        }
        rest = &after[digit_end..];
    }
    out.push_str(rest);
    out
}

/// Render an inline directive to its Jira wiki form.
fn render_inline_directive(d: &InlineDirective) -> String {
    match d.name.as_str() {
        "status" => render_inline_status(d),
        "emoticon" => render_inline_emoticon(d),
        "mention" => render_inline_mention(d),
        "link" => render_inline_link(d),
        "image" => render_inline_image(d),
        _ => literal_inline_directive(d),
    }
}

/// `{status:colour=Green|title=DONE}` (Jira uses British "colour"). With no
/// color we omit the parameter and Jira renders the default grey badge.
fn render_inline_status(d: &InlineDirective) -> String {
    let title = d.content.clone().unwrap_or_default();
    let raw_color = d.params.get("color").map(String::as_str).unwrap_or("");
    let color = match raw_color {
        "" => None,
        "green" => Some("Green"),
        "red" => Some("Red"),
        "yellow" => Some("Yellow"),
        "blue" => Some("Blue"),
        "purple" => Some("Purple"),
        "grey" | "gray" => Some("Grey"),
        _ => None,
    };
    let mut params = Vec::new();
    if let Some(c) = color {
        params.push(format!("colour={c}"));
    } else if !raw_color.is_empty() {
        // Unknown color — pass through lowercase so the macro still gets the
        // user's intent without breaking the stricter Jira renderer.
        params.push(format!("colour={}", raw_color.to_lowercase()));
    }
    params.push(format!("title={title}"));
    format!("{{status:{}}}", params.join("|"))
}

/// `(!)`, `(/)`, etc. for the small set Jira recognises; otherwise fall back
/// to `:name:` (a literal shortcut Jira leaves alone).
fn render_inline_emoticon(d: &InlineDirective) -> String {
    let name = d.params.get("name").map(String::as_str).unwrap_or("");
    match name {
        "warning" => "(!)".to_string(),
        "tick" => "(/)".to_string(),
        "cross" => "(x)".to_string(),
        "info" => "(i)".to_string(),
        "question" => "(?)".to_string(),
        other => format!(":{other}:"),
    }
}

/// `[~accountid:abc]` (Cloud) or `[~username]` (Server/DC). Empty `[~]` if
/// neither attribute was given.
fn render_inline_mention(d: &InlineDirective) -> String {
    if let Some(id) = d.params.get("accountId") {
        format!("[~accountid:{id}]")
    } else if let Some(name) = d.params.get("username") {
        format!("[~{name}]")
    } else {
        "[~]".to_string()
    }
}

/// `[Title|url]`, or `[Title]` when only `pageId` is known (lossy — Jira tries
/// to resolve the bare title at render time).
fn render_inline_link(d: &InlineDirective) -> String {
    let title = d.content.clone().unwrap_or_default();
    let safe_title = title.replace('|', "\\|");
    if let Some(url) = d.params.get("url") {
        if safe_title.is_empty() {
            format!("[{url}]")
        } else {
            format!("[{safe_title}|{url}]")
        }
    } else if safe_title.is_empty() {
        // No url, no title — best we can do is empty brackets so the user sees
        // the directive survived.
        "[]".to_string()
    } else {
        format!("[{safe_title}]")
    }
}

/// `!src!` or `!src|alt=…!`.
fn render_inline_image(d: &InlineDirective) -> String {
    let src = d.params.get("src").cloned().unwrap_or_default();
    if let Some(alt) = d.params.get("alt") {
        format!("!{src}|alt={alt}!")
    } else {
        format!("!{src}!")
    }
}

/// Produce a `:name[content]{attrs}` literal for inline directives we can't
/// represent natively in wiki.
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
// comrak AST → Jira wiki (preserved from the previous markdown.rs)
// =====================================================================

/// Render block-level children of `node`. Inserts blank lines between blocks
/// at the top level and inside block quotes; suppresses them inside list items.
fn render_block_children<'a>(node: &'a AstNode<'a>, out: &mut String, list_depth: usize) {
    let mut first = true;
    for child in node.children() {
        if !first {
            ensure_blank_line(out);
        }
        first = false;
        render_block(child, out, list_depth);
    }
}

fn render_block<'a>(node: &'a AstNode<'a>, out: &mut String, list_depth: usize) {
    let value = node.data.borrow().value.clone();
    match value {
        NodeValue::Document => render_block_children(node, out, list_depth),
        NodeValue::Heading(h) => {
            let level = h.level.clamp(1, 6);
            out.push_str(&format!("h{level}. "));
            render_inline_children(node, out);
            ensure_newline(out);
        }
        NodeValue::Paragraph => {
            render_inline_children(node, out);
            ensure_newline(out);
        }
        NodeValue::ThematicBreak => {
            out.push_str("----\n");
        }
        NodeValue::CodeBlock(cb) => {
            render_code_block(&cb.literal, &cb.info, out);
        }
        NodeValue::BlockQuote => {
            out.push_str("{quote}\n");
            let mut inner = String::new();
            render_block_children(node, &mut inner, 0);
            // Trim a single trailing newline so we don't emit a blank line
            // before {quote}.
            if inner.ends_with('\n') {
                inner.pop();
            }
            out.push_str(&inner);
            out.push_str("\n{quote}\n");
        }
        NodeValue::List(list) => {
            render_list(node, &list.list_type, out, list_depth + 1);
        }
        NodeValue::Item(_) | NodeValue::TaskItem(_) => {
            // Items are handled by render_list directly.
        }
        NodeValue::Table(_) => {
            render_table(node, out);
        }
        NodeValue::HtmlBlock(html) => {
            // Keep raw HTML — Jira wiki passes it through.
            out.push_str(&html.literal);
            ensure_newline(out);
        }
        // Inline-ish stragglers that can show up at the top level.
        _ => {
            render_inline(node, out);
        }
    }
}

fn render_list<'a>(node: &'a AstNode<'a>, list_type: &ListType, out: &mut String, depth: usize) {
    let marker_char = match list_type {
        ListType::Bullet => '*',
        ListType::Ordered => '#',
    };
    let marker: String = std::iter::repeat_n(marker_char, depth).collect();

    for item in node.children() {
        let item_value = item.data.borrow().value.clone();
        match item_value {
            NodeValue::Item(_) | NodeValue::TaskItem(_) => {
                out.push_str(&marker);
                out.push(' ');
                render_item_contents(item, out, depth);
                ensure_newline(out);
            }
            _ => {
                // Defensive: a non-item child shouldn't happen, but render it anyway.
                render_block(item, out, depth);
            }
        }
    }
}

/// Render the contents of a list item: typically one paragraph (rendered
/// inline) optionally followed by nested lists or other blocks.
fn render_item_contents<'a>(item: &'a AstNode<'a>, out: &mut String, depth: usize) {
    let mut first = true;
    for child in item.children() {
        let v = child.data.borrow().value.clone();
        match v {
            NodeValue::Paragraph => {
                if !first {
                    // Subsequent paragraphs in the same item — separate with a
                    // wiki line break so they render on a new line.
                    out.push_str("\\\\");
                }
                render_inline_children(child, out);
                first = false;
            }
            NodeValue::List(list) => {
                ensure_newline(out);
                render_list(child, &list.list_type, out, depth + 1);
                first = false;
            }
            _ => {
                // For anything more exotic (code block inside a list item, etc.),
                // emit a newline and render as a block.
                ensure_newline(out);
                render_block(child, out, depth);
                first = false;
            }
        }
    }
}

fn render_table<'a>(node: &'a AstNode<'a>, out: &mut String) {
    for row in node.children() {
        let row_value = row.data.borrow().value.clone();
        let is_header = matches!(row_value, NodeValue::TableRow(true));
        let sep = if is_header { "||" } else { "|" };

        out.push_str(sep);
        for cell in row.children() {
            let mut cell_text = String::new();
            render_inline_children(cell, &mut cell_text);
            // Pipes inside a cell would break the table — escape minimally.
            let escaped = cell_text.replace('|', "\\|");
            out.push_str(&escaped);
            out.push_str(sep);
        }
        out.push('\n');
    }
}

fn render_inline_children<'a>(node: &'a AstNode<'a>, out: &mut String) {
    for child in node.children() {
        render_inline(child, out);
    }
}

fn render_inline<'a>(node: &'a AstNode<'a>, out: &mut String) {
    let value = node.data.borrow().value.clone();
    match value {
        NodeValue::Text(t) => out.push_str(&t),
        NodeValue::SoftBreak => out.push(' '),
        NodeValue::LineBreak => out.push_str("\\\\"),
        NodeValue::Code(code) => render_inline_code(&code.literal, out),
        NodeValue::HtmlInline(s) => out.push_str(&s),
        NodeValue::Strong => {
            out.push('*');
            render_inline_children(node, out);
            out.push('*');
        }
        NodeValue::Emph => {
            out.push('_');
            render_inline_children(node, out);
            out.push('_');
        }
        NodeValue::Strikethrough => {
            out.push('-');
            render_inline_children(node, out);
            out.push('-');
        }
        NodeValue::Link(link) => {
            out.push('[');
            let mut text = String::new();
            render_inline_children(node, &mut text);
            // Escape any embedded `|` in link text — pipe is the wiki separator.
            let safe_text = text.replace('|', "\\|");
            if safe_text.is_empty() {
                out.push_str(&link.url);
            } else {
                out.push_str(&safe_text);
                out.push('|');
                out.push_str(&link.url);
            }
            out.push(']');
        }
        NodeValue::Image(link) => {
            out.push('!');
            out.push_str(&link.url);
            out.push('!');
        }
        NodeValue::Escaped => {
            // Just emit the children; comrak already stripped the escape.
            render_inline_children(node, out);
        }
        // Block nodes that occasionally appear nested via paragraphs in items —
        // recurse into their inline content as a fallback.
        _ => render_inline_children(node, out),
    }
}

fn render_inline_code(literal: &str, out: &mut String) {
    if literal.contains("}}") {
        // {{...}} delimiter would collide — use {noformat} as a fallback.
        out.push_str("{noformat}");
        out.push_str(literal);
        out.push_str("{noformat}");
    } else {
        out.push_str("{{");
        out.push_str(literal);
        out.push_str("}}");
    }
}

fn render_code_block(literal: &str, info: &str, out: &mut String) {
    let lang = info.split_whitespace().next().unwrap_or("");
    // If the body contains the {code} delimiter we'd otherwise emit, fall back
    // to {noformat} which doesn't carry a language but avoids the collision.
    let collides = literal.contains("{code}") || literal.contains("{code:");
    if collides {
        out.push_str("{noformat}\n");
        out.push_str(literal);
        if !literal.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("{noformat}\n");
        return;
    }

    if lang.is_empty() {
        out.push_str("{code}\n");
    } else {
        out.push_str("{code:");
        out.push_str(lang);
        out.push_str("}\n");
    }
    out.push_str(literal);
    if !literal.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("{code}\n");
}

/// Append a newline if the buffer doesn't already end with one.
fn ensure_newline(out: &mut String) {
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

/// Append enough newlines so the buffer ends with exactly one blank line
/// (i.e. `\n\n`) — used between top-level blocks.
fn ensure_blank_line(out: &mut String) {
    if out.is_empty() {
        return;
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(md: &str) -> String {
        markdown_to_wiki(md).expect("conversion succeeded")
    }

    // ===== Existing tests preserved from markdown.rs (22 tests) =====

    #[test]
    fn markdown_to_wiki_heading_h1() {
        assert_eq!(convert("# Hi"), "h1. Hi\n");
    }

    #[test]
    fn markdown_to_wiki_bold() {
        assert_eq!(convert("**x**"), "*x*\n");
    }

    #[test]
    fn markdown_to_wiki_italic_underscore() {
        assert_eq!(convert("_x_"), "_x_\n");
    }

    #[test]
    fn markdown_to_wiki_inline_code() {
        assert_eq!(convert("`x`"), "{{x}}\n");
    }

    #[test]
    fn markdown_to_wiki_fenced_code_with_lang() {
        let input = "```rust\nfn x() {}\n```";
        assert_eq!(convert(input), "{code:rust}\nfn x() {}\n{code}\n");
    }

    #[test]
    fn markdown_to_wiki_link() {
        assert_eq!(convert("[t](u)"), "[t|u]\n");
    }

    #[test]
    fn markdown_to_wiki_bullet_list() {
        assert_eq!(convert("- a\n- b"), "* a\n* b\n");
    }

    #[test]
    fn markdown_to_wiki_pipe_table() {
        let input = "| h1 | h2 |\n|----|----|\n| c1 | c2 |\n";
        let result = convert(input);
        assert!(
            result.contains("||h1||h2||"),
            "expected header row with || separators in:\n{result}"
        );
        assert!(
            result.contains("|c1|c2|"),
            "expected data row with | separators in:\n{result}"
        );
    }

    // ---- headings ----

    #[test]
    fn heading_h2() {
        assert_eq!(convert("## Hi"), "h2. Hi\n");
    }

    #[test]
    fn heading_h6() {
        assert_eq!(convert("###### Hi"), "h6. Hi\n");
    }

    #[test]
    fn heading_with_inline_bold() {
        // Bold inside a heading must be converted to `*...*` wiki syntax.
        assert_eq!(convert("# **Bold** title"), "h1. *Bold* title\n");
    }

    // ---- inline formatting ----

    #[test]
    fn italic_with_asterisk_marker() {
        // `*x*` is parsed as emphasis (Emph) and emits `_x_` per Jira wiki.
        assert_eq!(convert("*x*"), "_x_\n");
    }

    #[test]
    fn triple_star_emits_outer_emph_inner_strong() {
        // `***word***` parses as <em><strong>word</strong></em> in CommonMark.
        // The implementation walks outer-first, so the result is `_*word*_`,
        // not the spec's suggested `*_word_*`. This test locks in the
        // implementation's behavior.
        assert_eq!(convert("***word***"), "_*word*_\n");
    }

    #[test]
    fn strikethrough() {
        assert_eq!(convert("~~x~~"), "-x-\n");
    }

    #[test]
    fn inline_code_with_close_brace_falls_back_to_noformat() {
        // `}}` would prematurely close `{{...}}`, so the converter falls back
        // to `{noformat}` which has no nesting/escape concerns.
        assert_eq!(convert("`a}}b`"), "{noformat}a}}b{noformat}\n");
    }

    #[test]
    fn paragraph_bold_in_text() {
        // Bold inside a paragraph (not standalone) — verifies the default
        // Strong handler runs through the inline pipeline.
        assert_eq!(convert("hello **world**"), "hello *world*\n");
    }

    // ---- code blocks ----

    #[test]
    fn fenced_code_no_lang() {
        assert_eq!(convert("```\nplain\n```"), "{code}\nplain\n{code}\n");
    }

    #[test]
    fn fenced_code_with_marker_in_body_falls_back_to_noformat() {
        // If the body literally contains `{code}`, the `{code}...{code}`
        // delimiter would close prematurely — fall back to `{noformat}`.
        assert_eq!(
            convert("```\n{code}\n```"),
            "{noformat}\n{code}\n{noformat}\n"
        );
    }

    #[test]
    fn fenced_code_with_lang_and_marker_in_body_uses_noformat() {
        // The collision check fires regardless of language.
        let input = "```rust\n// uses {code:foo} delim\n```";
        let result = convert(input);
        assert!(
            result.starts_with("{noformat}\n"),
            "should fall back to {{noformat}} when body contains a code marker, got:\n{result}"
        );
        assert!(
            !result.contains("{code:rust}"),
            "should NOT emit {{code:rust}} when colliding, got:\n{result}"
        );
    }

    #[test]
    fn indented_code_block_treated_as_code() {
        // Comrak parses 4-space indented blocks as code blocks; the converter
        // emits them with a `{code}` fence (no language).
        assert_eq!(
            convert("    line1\n    line2"),
            "{code}\nline1\nline2\n{code}\n"
        );
    }

    // ---- lists ----

    #[test]
    fn bullet_list_asterisk_marker_input() {
        // Same output regardless of whether the source uses `-` or `*`.
        assert_eq!(convert("* a\n* b"), "* a\n* b\n");
    }

    #[test]
    fn nested_bullet_two_levels() {
        assert_eq!(convert("- a\n  - b"), "* a\n** b\n");
    }

    #[test]
    fn nested_bullet_three_levels() {
        assert_eq!(convert("- a\n  - b\n    - c"), "* a\n** b\n*** c\n");
    }

    #[test]
    fn ordered_list_flat() {
        // Each numbered item emits `# ` regardless of source numbering.
        assert_eq!(convert("1. a\n2. b"), "# a\n# b\n");
    }

    #[test]
    fn nested_ordered_list() {
        // Depth-2 ordered list emits `## ` (depth count of `#` markers).
        assert_eq!(convert("1. a\n   1. b"), "# a\n## b\n");
    }

    #[test]
    fn mixed_nested_bullet_then_ordered() {
        // SPEC NOTE: the spec hinted at `*#` for mixed nesting, but the
        // implementation emits `##` because the marker char is chosen by the
        // inner list's type alone, repeated `depth` times. This test locks in
        // the actual behavior — change here means an intentional behavior
        // change.
        assert_eq!(convert("- a\n  1. b"), "* a\n## b\n");
    }

    #[test]
    fn task_list_emitted_as_plain_bullets() {
        // Jira wiki has no native checkbox; the `- [ ] / - [x]` markers are
        // dropped and the items render as ordinary bullets.
        assert_eq!(convert("- [ ] a\n- [x] b"), "* a\n* b\n");
    }

    #[test]
    fn list_item_with_two_paragraphs_joined_by_hard_break() {
        // The second paragraph in the same `<li>` is separated by `\\` so the
        // wiki rendering keeps them visually distinct without breaking the list.
        assert_eq!(convert("- a\n\n  b"), "* a\\\\b\n");
    }

    // ---- links and images ----

    #[test]
    fn link_with_text() {
        assert_eq!(
            convert("[text](https://example.com)"),
            "[text|https://example.com]\n"
        );
    }

    #[test]
    fn link_with_empty_text_emits_url_only() {
        // No text → no `text|` prefix; just `[url]`.
        assert_eq!(
            convert("[](https://example.com)"),
            "[https://example.com]\n"
        );
    }

    #[test]
    fn image_drops_alt_text() {
        // Alt text is intentionally dropped — Jira wiki `!url!` syntax has no
        // alt-text field.
        assert_eq!(
            convert("![alt](https://example.com/img.png)"),
            "!https://example.com/img.png!\n"
        );
    }

    #[test]
    fn autolink_emits_link_with_url_as_text() {
        // SPEC NOTE: comrak's autolink extension expands `<url>` into a Link
        // node with the URL as both the displayed text and the href, so the
        // converter emits `[url|url]` rather than the bare URL or `[url]`.
        assert_eq!(
            convert("<https://example.com>"),
            "[https://example.com|https://example.com]\n"
        );
    }

    // ---- tables ----

    #[test]
    fn table_header_only() {
        // No data rows — separator row is silently dropped, just the header
        // remains.
        assert_eq!(convert("| h1 | h2 |\n|----|----|\n"), "||h1||h2||\n");
    }

    #[test]
    fn table_cell_pipe_is_escaped() {
        // A literal `|` inside a cell must be escaped to `\|` so it doesn't
        // close the cell. Use a markdown-escaped pipe in the input.
        let result = convert("| a \\| b | c |\n|---|---|\n| 1 | 2 |\n");
        assert!(
            result.contains(r"||a \| b||c||"),
            "expected escaped pipe in header cell, got:\n{result}"
        );
        assert!(result.contains("|1|2|"), "expected data row in:\n{result}");
    }

    // ---- blockquotes ----

    #[test]
    fn blockquote_single_line() {
        assert_eq!(convert("> hello"), "{quote}\nhello\n{quote}\n");
    }

    #[test]
    fn blockquote_multi_line_joined_with_space() {
        // A multi-line blockquote is one paragraph with soft breaks; soft
        // breaks become a single space, so the body is `a b` (not `a\nb`).
        assert_eq!(convert("> a\n> b"), "{quote}\na b\n{quote}\n");
    }

    // ---- other blocks ----

    #[test]
    fn horizontal_rule() {
        assert_eq!(convert("---"), "----\n");
    }

    #[test]
    fn hard_line_break_emits_double_backslash() {
        // Two trailing spaces produce a markdown LineBreak → `\\` in wiki.
        assert_eq!(convert("a  \nb"), "a\\\\b\n");
    }

    #[test]
    fn soft_line_break_joins_with_space() {
        // A bare newline in markdown is a soft break; collapses to a space.
        assert_eq!(convert("a\nb"), "a b\n");
    }

    // ---- realistic round-trip ----

    #[test]
    fn realistic_document_round_trip() {
        let input = "\
# Title

A paragraph with **bold** and *italic* and ~~strike~~.

## Section

- item 1
- item 2
  - nested

| name | value |
|------|-------|
| a    | 1     |

```rust
fn main() {}
```

See the [docs](https://example.com) for more.
";
        let result = convert(input);

        // No leftover markdown syntax. Note: a bare `**` substring search
        // would false-positive on wiki nested-list markers (`** nested`), so
        // we look specifically for the markdown-bold pattern `**word**`.
        assert!(
            !result.contains("**bold**"),
            "leftover `**bold**` markdown emphasis in:\n{result}"
        );
        assert!(
            !result.contains("\n## "),
            "leftover `## ` heading prefix in:\n{result}"
        );
        assert!(
            !result.contains("```"),
            "leftover triple-backtick fence in:\n{result}"
        );
        assert!(
            !result.contains("|---"),
            "leftover pipe-separator row in:\n{result}"
        );
        assert!(!result.is_empty(), "result must not be empty");

        // Expected wiki tokens are present.
        assert!(
            result.contains("h1. Title"),
            "expected h1 token in:\n{result}"
        );
        assert!(
            result.contains("h2. Section"),
            "expected h2 token in:\n{result}"
        );
        assert!(
            result.contains("*bold*"),
            "expected bold wiki marker in:\n{result}"
        );
        assert!(
            result.contains("_italic_"),
            "expected italic wiki marker in:\n{result}"
        );
        assert!(
            result.contains("-strike-"),
            "expected strikethrough marker in:\n{result}"
        );
        assert!(
            result.contains("||name||value||"),
            "expected wiki table header in:\n{result}"
        );
        assert!(
            result.contains("|a|1|"),
            "expected wiki table data row in:\n{result}"
        );
        assert!(
            result.contains("{code:rust}"),
            "expected fenced code with rust lang in:\n{result}"
        );
        assert!(
            result.contains("[docs|https://example.com]"),
            "expected wiki link syntax in:\n{result}"
        );
        assert!(
            result.contains("** nested"),
            "expected nested bullet `** ` in:\n{result}"
        );
    }

    // ===== New tests: block directives =====

    #[test]
    fn block_info_emits_paired_macro() {
        let result = convert(":::info\nHello\n:::");
        assert!(result.contains("{info}"), "missing open in:\n{result}");
        // Two `{info}` occurrences (open and close).
        assert_eq!(
            result.matches("{info}").count(),
            2,
            "expected paired open/close in:\n{result}"
        );
        assert!(result.contains("Hello"), "missing body in:\n{result}");
    }

    #[test]
    fn block_info_with_title() {
        let result = convert(":::info title=\"Heads up\"\nbody\n:::");
        assert!(
            result.contains("{info:title=Heads up}"),
            "expected title param in:\n{result}"
        );
        assert!(result.contains("{info}"), "expected close in:\n{result}");
    }

    #[test]
    fn block_info_with_multiple_params() {
        // BTreeMap orders keys alphabetically, so `key` < `title`.
        let result = convert(":::info title=\"Heads up\" key=val\nbody\n:::");
        assert!(
            result.contains("{info:key=val|title=Heads up}"),
            "expected pipe-joined params in:\n{result}"
        );
    }

    #[test]
    fn block_warning_directive() {
        let result = convert(":::warning\nbody\n:::");
        assert_eq!(result.matches("{warning}").count(), 2, "got: {result}");
    }

    #[test]
    fn block_note_directive() {
        let result = convert(":::note\nbody\n:::");
        assert_eq!(result.matches("{note}").count(), 2, "got: {result}");
    }

    #[test]
    fn block_tip_directive() {
        let result = convert(":::tip\nbody\n:::");
        assert_eq!(result.matches("{tip}").count(), 2, "got: {result}");
    }

    #[test]
    fn block_expand_with_title_emits_bold_header() {
        let result = convert(":::expand title=\"Detail\"\nbody\n:::");
        assert!(
            result.contains("*Detail*"),
            "expected bold-marker title in:\n{result}"
        );
        assert!(result.contains("body"), "missing body in:\n{result}");
        // No `{expand}` macro — Jira wiki has no native expand.
        assert!(
            !result.contains("{expand}"),
            "expand has no native wiki macro, got:\n{result}"
        );
    }

    #[test]
    fn block_toc_with_max_level() {
        let result = convert(":::toc maxLevel=3\n:::");
        assert!(
            result.contains("{toc:maxLevel=3}"),
            "expected toc with maxLevel in:\n{result}"
        );
    }

    #[test]
    fn block_toc_no_params() {
        let result = convert(":::toc\n:::");
        assert!(
            result.contains("{toc}") && !result.contains("{toc:"),
            "expected bare `{{toc}}` in:\n{result}"
        );
    }

    #[test]
    fn block_directives_can_nest() {
        // `:::expand` body contains a `:::info`. Expand renders title only,
        // and the inner info is preserved as a paired `{info}` macro.
        let result = convert(":::expand title=\"Outer\"\n:::info\nx\n:::\n:::");
        assert!(
            result.contains("*Outer*"),
            "expected expand title in:\n{result}"
        );
        assert_eq!(
            result.matches("{info}").count(),
            2,
            "expected nested info open+close in:\n{result}"
        );
    }

    #[test]
    fn block_unknown_directive_passes_through_as_literal() {
        // The lexer emits unknown names as Lines. The literal `:::custom`
        // survives into the comrak text and shows up in the output.
        let result = convert(":::custom\nbody\n:::");
        assert!(
            result.contains(":::custom"),
            "expected literal directive in:\n{result}"
        );
    }

    // ===== New tests: inline directives =====

    #[test]
    fn inline_status_with_green_color() {
        let result = convert(":status[DONE]{color=green}");
        assert!(
            result.contains("{status:colour=Green|title=DONE}"),
            "expected mapped status in:\n{result}"
        );
    }

    #[test]
    fn inline_status_with_red() {
        let result = convert(":status[BAD]{color=red}");
        assert!(
            result.contains("{status:colour=Red|title=BAD}"),
            "got: {result}"
        );
    }

    #[test]
    fn inline_status_no_color_omits_colour_param() {
        let result = convert(":status[DONE]");
        // No colour= param; just the title.
        assert!(
            result.contains("{status:title=DONE}"),
            "expected title-only status in:\n{result}"
        );
        assert!(
            !result.contains("colour="),
            "should not emit colour with no input, got:\n{result}"
        );
    }

    #[test]
    fn inline_status_unknown_color_passes_through_lowercase() {
        let result = convert(":status[X]{color=mauve}");
        assert!(
            result.contains("{status:colour=mauve|title=X}"),
            "expected lowercase passthrough in:\n{result}"
        );
    }

    #[test]
    fn inline_emoticon_warning() {
        let result = convert(":emoticon{name=warning}");
        assert!(result.contains("(!)"), "expected `(!)` in:\n{result}");
    }

    #[test]
    fn inline_emoticon_tick() {
        let result = convert(":emoticon{name=tick}");
        assert!(result.contains("(/)"), "expected `(/)` in:\n{result}");
    }

    #[test]
    fn inline_emoticon_cross() {
        let result = convert(":emoticon{name=cross}");
        assert!(result.contains("(x)"), "expected `(x)` in:\n{result}");
    }

    #[test]
    fn inline_emoticon_unknown_name_falls_back_to_shortcut() {
        let result = convert(":emoticon{name=heart}");
        assert!(
            result.contains(":heart:"),
            "expected `:heart:` fallback in:\n{result}"
        );
    }

    #[test]
    fn inline_mention_with_account_id() {
        let result = convert(":mention[@john]{accountId=abc123}");
        assert!(
            result.contains("[~accountid:abc123]"),
            "expected accountid mention in:\n{result}"
        );
    }

    #[test]
    fn inline_mention_with_username() {
        let result = convert(":mention[@john]{username=jdoe}");
        assert!(
            result.contains("[~jdoe]"),
            "expected username mention in:\n{result}"
        );
    }

    #[test]
    fn inline_link_with_url() {
        let result = convert(":link[Title]{url=\"https://example.com\"}");
        assert!(
            result.contains("[Title|https://example.com]"),
            "expected piped link in:\n{result}"
        );
    }

    #[test]
    fn inline_link_with_only_page_id_falls_back_to_bare_title() {
        let result = convert(":link[Title]{pageId=12345}");
        // Lossy: just the title in brackets — Jira tries to resolve at render.
        assert!(
            result.contains("[Title]"),
            "expected bare `[Title]` fallback in:\n{result}"
        );
        assert!(
            !result.contains("12345"),
            "pageId is lossy, should not appear, got:\n{result}"
        );
    }

    #[test]
    fn inline_image_src_only() {
        let result = convert(":image{src=\"http://x.png\"}");
        assert!(
            result.contains("!http://x.png!"),
            "expected wiki image in:\n{result}"
        );
    }

    #[test]
    fn inline_image_with_alt() {
        let result = convert(":image{src=\"http://x.png\" alt=\"a picture\"}");
        assert!(
            result.contains("!http://x.png|alt=a picture!"),
            "expected image with alt in:\n{result}"
        );
    }

    // ===== New tests: code-fence escape =====

    #[test]
    fn directive_inside_backtick_fence_is_literal() {
        // `:::info` inside a fenced code block must NOT become a panel.
        let result = convert("```\n:::info\nbody\n:::\n```");
        // The content should be inside a {code} fence with the literal :::info.
        assert!(
            result.contains(":::info"),
            "expected literal :::info preserved in:\n{result}"
        );
        // No paired panel macro.
        assert!(
            !result.contains("{info}"),
            "should not emit panel inside code fence, got:\n{result}"
        );
    }

    #[test]
    fn directive_inside_tilde_fence_is_literal() {
        let result = convert("~~~\n:::info\nbody\n:::\n~~~");
        assert!(
            result.contains(":::info"),
            "expected literal :::info preserved in:\n{result}"
        );
        assert!(
            !result.contains("{info}"),
            "should not emit panel inside tilde fence, got:\n{result}"
        );
    }

    #[test]
    fn inline_directive_inside_fence_is_literal() {
        let result = convert("```\n:status[X]{color=green}\n```");
        assert!(
            result.contains(":status[X]"),
            "expected literal :status preserved in:\n{result}"
        );
        assert!(
            !result.contains("colour=Green"),
            "should not convert inline directive inside fence, got:\n{result}"
        );
    }

    // ===== New tests: edge cases =====

    #[test]
    fn empty_input_yields_empty_string() {
        assert_eq!(convert(""), "");
    }

    #[test]
    fn whitespace_only_input() {
        // Whitespace-only input rounds through as itself (with trailing
        // newline). The wiki has no concept of "blank document" so we just
        // pass the whitespace.
        let result = convert("   \n\n  \n");
        // Should not error; should not contain any directive markers.
        assert!(!result.contains("{"), "got: {result:?}");
    }

    #[test]
    fn unclosed_directive_returns_err() {
        let err = markdown_to_wiki(":::info\nbody").unwrap_err();
        match err {
            MdToWikiError::Directive(DirectiveError::Unclosed { name, .. }) => {
                assert_eq!(name, "info");
            }
            other => panic!("expected Unclosed, got: {other:?}"),
        }
    }

    #[test]
    fn inline_directive_inside_heading() {
        let result = convert("# Title :status[DONE]{color=green}");
        assert!(
            result.starts_with("h1. "),
            "expected wiki heading in:\n{result}"
        );
        assert!(
            result.contains("{status:colour=Green|title=DONE}"),
            "expected status macro in:\n{result}"
        );
    }

    #[test]
    fn inline_directive_inside_list_item() {
        let result = convert("- item with :emoticon{name=warning}");
        assert!(
            result.starts_with("* item with "),
            "expected wiki bullet in:\n{result}"
        );
        assert!(result.contains("(!)"), "expected emoticon in:\n{result}");
    }

    #[test]
    fn mixed_paragraphs_and_block_directives() {
        let result = convert("Before.\n\n:::info\nIn panel\n:::\n\nAfter.");
        assert!(result.contains("Before."), "got:\n{result}");
        assert_eq!(result.matches("{info}").count(), 2, "got:\n{result}");
        assert!(result.contains("In panel"), "got:\n{result}");
        assert!(result.contains("After."), "got:\n{result}");
    }

    #[test]
    fn multiple_inline_directives_in_one_paragraph() {
        let result = convert(":status[OK]{color=green} and :emoticon{name=tick}");
        assert!(
            result.contains("{status:colour=Green|title=OK}"),
            "expected status in:\n{result}"
        );
        assert!(result.contains("(/)"), "expected tick in:\n{result}");
    }
}
