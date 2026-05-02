//! Markdown (with MyST-style directive extensions) → Confluence storage XHTML.
//!
//! # Conversion strategy
//!
//! The converter runs in three stages:
//!
//! 1. **Code-fence-aware line walk.** Lines inside a CommonMark fenced code
//!    block (` ``` ` / `~~~`) bypass the directive lexer entirely so a literal
//!    `:::info` inside a code block round-trips as text. Lines outside go
//!    through [`crate::cli::commands::directives::BlockLexer`].
//!
//! 2. **Tree building.** `Open` / `Close` / `Line` events are folded into a
//!    nested tree of `Node::Directive { … }` and `Node::Text(String)`. Text
//!    accumulates inside whichever directive (or the root) is currently
//!    on top of the stack.
//!
//! 3. **Recursive render.** Each `Node::Text` runs an inline-directive pre-pass
//!    (substituting placeholder HTML comments), then comrak with GFM, then a
//!    post-pass that swaps the placeholders for storage XML. Each
//!    `Node::Directive` emits a `<ac:structured-macro>` wrapper around the
//!    rendered children (or a self-closing tag for `toc` etc.). Special inline
//!    forms (`<ac:emoticon>`, `<ac:link>`, `<ac:image>`) are case-by-case.
//!
//! XML escaping is hand-written; we don't pull in `quick-xml` for this step.
//! Output is single-line by default — comrak controls block-level whitespace
//! inside text chunks; the directive wrappers are emitted with no surrounding
//! whitespace.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use comrak::{Options, markdown_to_html};
use thiserror::Error;

use crate::cli::commands::directives::{
    BlockEvent, BlockLexer, DirectiveError, DirectiveSpec, InlineDirective, InlineToken, lookup,
    parse_inline,
};

// =====================================================================
// Errors
// =====================================================================

/// Errors returned by [`markdown_to_storage`].
#[derive(Debug, Error)]
pub enum MdToStorageError {
    /// A directive grammar error (e.g. unclosed `:::name` block) was found.
    #[error(transparent)]
    Directive(#[from] DirectiveError),
}

// =====================================================================
// Public API
// =====================================================================

/// Convert markdown (with MyST-style directive extensions) to Confluence
/// storage XML.
///
/// Returns an error only on unrecoverable directive grammar issues
/// (specifically, an unclosed `:::name` block fence). Unknown directive names
/// pass through as their original markdown text — they don't fail conversion.
///
/// # Examples
///
/// ```ignore
/// use atl::cli::commands::converters::md_to_storage::markdown_to_storage;
///
/// let xml = markdown_to_storage(":::info\nHello\n:::").unwrap();
/// assert!(xml.contains("<ac:structured-macro ac:name=\"info\">"));
/// assert!(xml.contains("<ac:rich-text-body>"));
/// ```
pub fn markdown_to_storage(md: &str) -> Result<String, MdToStorageError> {
    let events = lex_with_code_fences(md)?;
    let tree = build_tree(events);
    Ok(render_nodes(&tree))
}

// =====================================================================
// Stage 1: code-fence-aware line walk
// =====================================================================

/// Walk `md` line by line, tracking CommonMark fenced code-block state, and
/// produce the same kind of [`BlockEvent`] stream that
/// [`crate::cli::commands::directives::lex_blocks`] produces — but with lines
/// inside a code fence forced to [`BlockEvent::Line`] instead of being
/// re-tokenised by the directive lexer.
fn lex_with_code_fences(md: &str) -> Result<Vec<BlockEvent>, DirectiveError> {
    let mut lex = BlockLexer::new();
    let mut events = Vec::new();
    let mut code_fence: Option<CodeFence> = None;

    for line in md.split('\n') {
        match &code_fence {
            Some(open) => {
                // Inside a fenced code block. The only thing that matters is
                // whether THIS line is the matching close fence.
                events.push(BlockEvent::Line(line.to_string()));
                if is_matching_close_fence(line, open) {
                    code_fence = None;
                }
            }
            None => {
                if let Some(open) = parse_fence_open(line) {
                    // A new fenced code block is opening — emit the fence line
                    // verbatim and mark code-fence state.
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

/// State for a currently-open fenced code block.
#[derive(Debug, Clone, Copy)]
struct CodeFence {
    /// `'`'` (backtick) or `'~'` (tilde).
    fence_char: char,
    /// Number of fence chars (>= 3 per CommonMark).
    fence_len: usize,
}

/// Recognise an opening code-fence line.
///
/// A valid opener has 0–3 leading spaces (the directive lexer rejects indented
/// `:::` fences but CommonMark allows up to 3 spaces of indent for code
/// fences) followed by 3+ identical `'`'` or `'~'` characters and an optional
/// info string (which we don't care about for state tracking).
fn parse_fence_open(line: &str) -> Option<CodeFence> {
    let trimmed = line.trim_start_matches(' ');
    // CommonMark: at most 3 leading spaces.
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
    // For backtick fences the info string must not contain a backtick (per
    // CommonMark). We don't enforce — being permissive here just means the
    // close fence still has to match length+char and that's what gates state.
    Some(CodeFence {
        fence_char: first,
        fence_len: count,
    })
}

/// True if `line` is a valid close fence for the given open fence.
fn is_matching_close_fence(line: &str, open: &CodeFence) -> bool {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return false;
    }
    // The close fence must use the same char as the open and be at least as
    // long. After the fence there may be only whitespace.
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

/// One node in the converter's intermediate tree.
#[derive(Debug)]
enum Node {
    /// A run of plain markdown lines, joined with `\n`.
    Text(String),
    /// A `:::name … :::` block directive.
    Directive {
        /// The original directive name as it appeared in markdown.
        name: String,
        /// Spec lookup result. `None` only when the directive name is unknown
        /// (which the lexer turns into `Line`, so we never actually build an
        /// `Open` for an unknown name — but plumb it through defensively).
        spec: Option<&'static DirectiveSpec>,
        params: BTreeMap<String, String>,
        children: Vec<Node>,
    },
}

/// Frame on the build stack: a directive that is currently open and the
/// children we've collected so far for it.
struct Frame {
    name: String,
    spec: Option<&'static DirectiveSpec>,
    params: BTreeMap<String, String>,
    children: Vec<Node>,
}

fn build_tree(events: Vec<BlockEvent>) -> Vec<Node> {
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Vec<Node> = Vec::new();

    // Helper: append a Line to the topmost frame's children (or `root`).
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
            BlockEvent::Open {
                name,
                params,
                depth: _,
            } => {
                let spec = lookup(&name);
                stack.push(Frame {
                    name,
                    spec,
                    params,
                    children: Vec::new(),
                });
            }
            BlockEvent::Close { .. } => {
                // Pop the topmost frame and attach as a Directive node to its
                // parent (the new top of stack, or root if the stack is now
                // empty).
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
                // Stray closes can't happen here because the lexer only emits
                // Close when the stack is non-empty; if for some reason one
                // does slip through, it's silently dropped.
            }
            BlockEvent::Line(line) => {
                let target = stack
                    .last_mut()
                    .map_or(&mut root, |frame| &mut frame.children);
                push_line(target, line);
            }
        }
    }

    // If the stack isn't empty here, the lexer's `finalize` would have already
    // returned an error before we got here; stack should be empty.
    root
}

// =====================================================================
// Stage 3: rendering
// =====================================================================

fn render_nodes(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(md) => out.push_str(&render_text(md)),
            Node::Directive {
                name,
                spec,
                params,
                children,
            } => {
                let body = render_nodes(children);
                match spec {
                    Some(spec) => {
                        out.push_str(&render_block_directive(name, spec, params, &body));
                    }
                    None => {
                        // Unknown directive — emit the literal markdown back so
                        // the user sees it. The lexer should have routed this
                        // as Line events, so reaching here is unusual.
                        out.push_str(&render_unknown_block(name, params, &body));
                    }
                }
            }
        }
    }
    out
}

/// Render a recognised block directive.
fn render_block_directive(
    name: &str,
    spec: &DirectiveSpec,
    params: &BTreeMap<String, String>,
    body: &str,
) -> String {
    let Some(macro_name) = spec.conf_storage_macro else {
        // Known directive but no Confluence storage equivalent. Pass through
        // as the original markdown so the converter degrades visibly.
        return render_unknown_block(name, params, body);
    };

    if spec.allows_body {
        render_macro_with_body(macro_name, params, body)
    } else {
        render_macro_self_closing(macro_name, params)
    }
}

/// `<ac:structured-macro ac:name="…"> <ac:parameter …/> <ac:rich-text-body>…</ac:rich-text-body> </ac:structured-macro>`
fn render_macro_with_body(
    macro_name: &str,
    params: &BTreeMap<String, String>,
    body: &str,
) -> String {
    let mut out = String::new();
    out.push_str(r#"<ac:structured-macro ac:name=""#);
    out.push_str(&xml_escape(macro_name));
    out.push_str(r#"">"#);
    for (k, v) in params {
        push_parameter(&mut out, k, v);
    }
    out.push_str("<ac:rich-text-body>");
    out.push_str(body);
    out.push_str("</ac:rich-text-body></ac:structured-macro>");
    out
}

/// `<ac:structured-macro ac:name="…"> <ac:parameter …/> </ac:structured-macro>`
fn render_macro_self_closing(macro_name: &str, params: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    out.push_str(r#"<ac:structured-macro ac:name=""#);
    out.push_str(&xml_escape(macro_name));
    out.push_str(r#"">"#);
    for (k, v) in params {
        push_parameter(&mut out, k, v);
    }
    out.push_str("</ac:structured-macro>");
    out
}

fn push_parameter(out: &mut String, key: &str, value: &str) {
    out.push_str(r#"<ac:parameter ac:name=""#);
    out.push_str(&xml_escape(key));
    out.push_str(r#"">"#);
    out.push_str(&xml_escape(value));
    out.push_str("</ac:parameter>");
}

/// Fallback for directives that are known but have no storage equivalent, or
/// for unknown names that somehow reached the renderer. Emits the original
/// `:::name {…} … :::` literal.
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
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(":::\n");
    out
}

// =====================================================================
// Inline / text rendering
// =====================================================================

/// Render a chunk of markdown text (possibly multi-line) to storage XHTML.
///
/// Inline directives are pulled out before comrak runs so comrak doesn't
/// rewrite them; placeholders survive as HTML comments and are swapped back
/// for storage XML after comrak finishes.
fn render_text(md: &str) -> String {
    let mut placeholders: Vec<InlineDirective> = Vec::new();
    let with_placeholders = substitute_inline_directives(md, &mut placeholders);

    let html = markdown_to_html(&with_placeholders, &gfm_options());

    if placeholders.is_empty() {
        return html;
    }
    restore_inline_directives(&html, &placeholders)
}

fn gfm_options() -> Options<'static> {
    let mut opts = Options::default();
    opts.extension.table = true;
    opts.extension.strikethrough = true;
    opts.extension.autolink = true;
    opts.extension.tasklist = true;
    // Enable raw HTML rendering so our placeholder comments
    // (`<!--ATL_INL_{n}-->`) survive the comrak pass instead of being
    // replaced with `<!-- raw HTML omitted -->`.
    opts.render.r#unsafe = true;
    opts
}

/// Walk `md` line by line and replace inline directive tokens with HTML
/// comment placeholders. Returns the rewritten string. Each placeholder's
/// directive is appended to `out_placeholders` at the index that matches the
/// `{n}` value in `<!--ATL_INL_{n}-->`.
///
/// Lines inside a CommonMark fenced code block are passed through verbatim:
/// a literal `:status[…]` inside ` ``` ` must NOT be rewritten as a
/// placeholder, otherwise comrak would emit a comment inside `<pre><code>`
/// and the directive registry would interpret a literal as an inline
/// directive.
fn substitute_inline_directives(md: &str, out_placeholders: &mut Vec<InlineDirective>) -> String {
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
                // Inside a code fence — never run the inline parser.
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
                    substitute_line(line, &mut out, out_placeholders);
                }
            }
        }
    }

    out
}

/// Run the inline parser against one line and append the result (with
/// placeholders for directives) to `out`.
fn substitute_line(line: &str, out: &mut String, out_placeholders: &mut Vec<InlineDirective>) {
    let tokens = parse_inline(line);
    if tokens.iter().all(|t| matches!(t, InlineToken::Text(_))) {
        out.push_str(line);
        return;
    }
    for token in tokens {
        match token {
            InlineToken::Text(s) => out.push_str(&s),
            InlineToken::Directive(d) => {
                let idx = out_placeholders.len();
                out_placeholders.push(d);
                let _ = write!(out, "<!--ATL_INL_{idx}-->");
            }
        }
    }
}

/// Replace `<!--ATL_INL_{n}-->` placeholders with rendered storage XML.
fn restore_inline_directives(html: &str, placeholders: &[InlineDirective]) -> String {
    // Single linear scan. We look for the literal `<!--ATL_INL_` prefix,
    // parse the index, find `-->`, and substitute.
    const PREFIX: &str = "<!--ATL_INL_";
    const SUFFIX: &str = "-->";

    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(pos) = rest.find(PREFIX) {
        out.push_str(&rest[..pos]);
        let after_prefix = &rest[pos + PREFIX.len()..];
        // Parse digits up to "-->"
        let end = match after_prefix.find(SUFFIX) {
            Some(e) => e,
            None => {
                // Malformed — leave as-is.
                out.push_str(&rest[pos..]);
                rest = "";
                break;
            }
        };
        let digits = &after_prefix[..end];
        let idx: usize = match digits.parse() {
            Ok(n) => n,
            Err(_) => {
                // Not actually one of ours — leave verbatim.
                out.push_str(&rest[pos..pos + PREFIX.len() + end + SUFFIX.len()]);
                rest = &rest[pos + PREFIX.len() + end + SUFFIX.len()..];
                continue;
            }
        };
        match placeholders.get(idx) {
            Some(d) => out.push_str(&render_inline_storage(d)),
            None => {
                // Index out of bounds — leave the placeholder verbatim.
                out.push_str(&rest[pos..pos + PREFIX.len() + end + SUFFIX.len()]);
            }
        }
        rest = &rest[pos + PREFIX.len() + end + SUFFIX.len()..];
    }
    out.push_str(rest);
    out
}

/// Render one inline directive to its storage XML representation.
///
/// Each directive name has its own special-case mapping because Confluence
/// does not use `<ac:structured-macro>` uniformly for inline elements:
///
/// - `status` is a structured-macro with `title` + `colour` parameters.
///   Note Confluence uses British spelling **`colour`**, so this renderer
///   maps the user-facing `color` attribute to `colour` in the output.
/// - `emoticon` becomes `<ac:emoticon ac:name="…"/>` — NOT a structured macro.
/// - `mention` becomes `<ac:link><ri:user ri:account-id="…"/></ac:link>`.
/// - `link` becomes `<ac:link><ri:page …/></ac:link>` — `pageId` (resource id)
///   wins over `title` (content title) when both are present.
/// - `image` becomes `<ac:image><ri:url ri:value="…"/></ac:image>`.
fn render_inline_storage(d: &InlineDirective) -> String {
    match d.name.as_str() {
        "status" => render_status(d),
        "emoticon" => render_emoticon(d),
        "mention" => render_mention(d),
        "link" => render_link(d),
        "image" => render_image(d),
        _ => render_unknown_inline(d),
    }
}

fn render_status(d: &InlineDirective) -> String {
    let mut out = String::new();
    out.push_str(r#"<ac:structured-macro ac:name="status">"#);
    if let Some(title) = d.content.as_ref() {
        push_parameter(&mut out, "title", title);
    }
    // Map the user-facing `color` attribute to Confluence's `colour`.
    for (k, v) in &d.params {
        let mapped_key: &str = if k == "color" { "colour" } else { k.as_str() };
        push_parameter(&mut out, mapped_key, v);
    }
    out.push_str("</ac:structured-macro>");
    out
}

fn render_emoticon(d: &InlineDirective) -> String {
    let name = d.params.get("name").map(String::as_str).unwrap_or_default();
    let mut out = String::new();
    out.push_str(r#"<ac:emoticon ac:name=""#);
    out.push_str(&xml_escape(name));
    out.push_str(r#""/>"#);
    out
}

fn render_mention(d: &InlineDirective) -> String {
    let account_id = d
        .params
        .get("accountId")
        .map(String::as_str)
        .unwrap_or_default();
    let mut out = String::new();
    out.push_str(r#"<ac:link><ri:user ri:account-id=""#);
    out.push_str(&xml_escape(account_id));
    out.push_str(r#""/></ac:link>"#);
    out
}

fn render_link(d: &InlineDirective) -> String {
    let mut out = String::new();
    out.push_str("<ac:link>");
    // Prefer pageId when present (numeric content id), otherwise fall back to
    // the bracketed display title as `ri:content-title`. If neither is given
    // we still emit a `<ri:page/>` self-closing element with no attribute.
    if let Some(page_id) = d.params.get("pageId") {
        out.push_str(r#"<ri:page ri:content-id=""#);
        out.push_str(&xml_escape(page_id));
        out.push_str(r#""/>"#);
    } else if let Some(title) = d.content.as_ref() {
        out.push_str(r#"<ri:page ri:content-title=""#);
        out.push_str(&xml_escape(title));
        out.push_str(r#""/>"#);
    } else {
        out.push_str("<ri:page/>");
    }
    out.push_str("</ac:link>");
    out
}

fn render_image(d: &InlineDirective) -> String {
    let src = d.params.get("src").map(String::as_str).unwrap_or_default();
    let alt = d.params.get("alt").map(String::as_str);

    let mut out = String::new();
    out.push_str("<ac:image");
    if let Some(alt) = alt {
        out.push_str(r#" ac:alt=""#);
        out.push_str(&xml_escape(alt));
        out.push('"');
    }
    out.push('>');
    out.push_str(r#"<ri:url ri:value=""#);
    out.push_str(&xml_escape(src));
    out.push_str(r#""/></ac:image>"#);
    out
}

/// Pass-through for inline directives the renderer doesn't recognise — emit
/// the literal `:name[content]{attrs}` so the user sees something useful.
fn render_unknown_inline(d: &InlineDirective) -> String {
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
// XML escaping
// =====================================================================

/// Escape a string for use in XML element text or `"…"`-quoted attribute
/// values. Conservative: escapes the five core XML entities.
fn xml_escape(s: &str) -> Cow<'_, str> {
    let needs = s
        .bytes()
        .any(|b| matches!(b, b'<' | b'>' | b'&' | b'"' | b'\''));
    if !needs {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    Cow::Owned(out)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(md: &str) -> String {
        markdown_to_storage(md).expect("conversion succeeded")
    }

    // ---- basic markdown passthrough ---------------------------------------

    #[test]
    fn heading_h1_passes_through() {
        let out = convert("# heading");
        assert!(out.contains("<h1>heading</h1>"), "got: {out}");
    }

    #[test]
    fn bold_passes_through() {
        let out = convert("**bold**");
        assert!(out.contains("<strong>bold</strong>"), "got: {out}");
    }

    #[test]
    fn unordered_list_passes_through() {
        let out = convert("- a\n- b");
        assert!(out.contains("<li>a</li>"), "got: {out}");
        assert!(out.contains("<li>b</li>"), "got: {out}");
    }

    #[test]
    fn ordered_list_passes_through() {
        let out = convert("1. one\n2. two");
        assert!(out.contains("<ol"), "got: {out}");
        assert!(out.contains("<li>one</li>"), "got: {out}");
    }

    #[test]
    fn paragraph_passes_through() {
        let out = convert("Hello world.");
        assert!(out.contains("<p>Hello world.</p>"), "got: {out}");
    }

    #[test]
    fn link_passes_through() {
        let out = convert("[t](https://e.com)");
        assert!(out.contains(r#"href="https://e.com""#), "got: {out}");
    }

    #[test]
    fn image_passes_through() {
        let out = convert("![alt](u)");
        assert!(out.contains(r#"src="u""#), "got: {out}");
    }

    #[test]
    fn code_block_passes_through() {
        let out = convert("```\nplain\n```");
        assert!(out.contains("<pre>"), "got: {out}");
        assert!(out.contains("plain"), "got: {out}");
    }

    // ---- block directives -------------------------------------------------

    #[test]
    fn block_info_directive_emits_macro() {
        let out = convert(":::info\nHello\n:::");
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="info">"#),
            "got: {out}"
        );
        assert!(out.contains("<ac:rich-text-body>"), "got: {out}");
        assert!(out.contains("</ac:structured-macro>"), "got: {out}");
    }

    #[test]
    fn block_directive_with_title_param() {
        let out = convert(":::warning title=\"Heads up\"\nText.\n:::");
        assert!(
            out.contains(r#"<ac:parameter ac:name="title">Heads up</ac:parameter>"#),
            "got: {out}"
        );
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="warning">"#),
            "got: {out}"
        );
    }

    #[test]
    fn block_directive_nested() {
        let out = convert(":::expand title=\"Outer\"\n:::info\nInner.\n:::\n:::");
        // Outer expand wraps inner info macro.
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="expand">"#),
            "got: {out}"
        );
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="info">"#),
            "got: {out}"
        );
        // The inner info must appear inside the outer's rich-text-body, not
        // outside it. A simple check: the outer's opening tag appears before
        // the inner's, and the outer's closing structured-macro appears after.
        let outer_open = out
            .find(r#"<ac:structured-macro ac:name="expand">"#)
            .expect("expand open");
        let inner_open = out
            .find(r#"<ac:structured-macro ac:name="info">"#)
            .expect("info open");
        let outer_close_idx = out
            .rfind("</ac:structured-macro>")
            .expect("outermost close");
        assert!(outer_open < inner_open, "got: {out}");
        assert!(inner_open < outer_close_idx, "got: {out}");
    }

    #[test]
    fn block_self_closing_toc() {
        let out = convert(":::toc maxLevel=3\n:::");
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="toc">"#),
            "got: {out}"
        );
        assert!(
            out.contains(r#"<ac:parameter ac:name="maxLevel">3</ac:parameter>"#),
            "got: {out}"
        );
        assert!(
            !out.contains("<ac:rich-text-body>"),
            "self-closing macro must not emit rich-text-body, got: {out}"
        );
    }

    #[test]
    fn block_directive_processes_inner_markdown() {
        let out = convert(":::info\n# Title\n:::");
        assert!(out.contains("<h1>Title</h1>"), "got: {out}");
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="info">"#),
            "got: {out}"
        );
    }

    #[test]
    fn multiple_sibling_directives_with_text_between() {
        let out = convert(":::info\nA\n:::\n\nMiddle\n\n:::warning\nB\n:::");
        assert!(
            out.contains(r#"ac:name="info""#),
            "info macro present, got: {out}"
        );
        assert!(
            out.contains(r#"ac:name="warning""#),
            "warning macro present, got: {out}"
        );
        assert!(out.contains("Middle"), "plain text preserved, got: {out}");
    }

    #[test]
    fn directive_at_start_of_doc() {
        let out = convert(":::info\nHi\n:::\n\nAfter");
        assert!(
            out.contains(r#"ac:name="info""#),
            "info macro present, got: {out}"
        );
        assert!(out.contains("After"), "trailing text preserved, got: {out}");
    }

    #[test]
    fn directive_at_end_of_doc() {
        let out = convert("Before\n\n:::info\nHi\n:::");
        assert!(out.contains("Before"), "leading text preserved, got: {out}");
        assert!(
            out.contains(r#"ac:name="info""#),
            "info macro present, got: {out}"
        );
    }

    #[test]
    fn only_directive_no_surrounding_text() {
        let out = convert(":::tip\nyay\n:::");
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="tip">"#),
            "got: {out}"
        );
    }

    #[test]
    fn unknown_block_directive_passes_through() {
        // The lexer routes unknown names as Lines, so the literal `:::custom`
        // text shows up in the output as plain markdown.
        let out = convert(":::custom\nbody\n:::");
        // Either as literal text or paragraph — at minimum, no structured
        // macro called "custom" should appear.
        assert!(
            !out.contains(r#"ac:name="custom""#),
            "unknown directive must NOT produce a structured-macro, got: {out}"
        );
        assert!(
            out.contains(":::custom"),
            "literal text preserved, got: {out}"
        );
    }

    #[test]
    fn known_directive_without_storage_macro_passes_through() {
        // The `mention` directive is registered but has `conf_storage_macro:
        // None`. Block-form mention isn't valid (it's an inline directive),
        // but we test the fallback path explicitly via the renderer.
        // Since `mention` is registered as INLINE, the block lexer would
        // accept `:::mention` as a known name with no storage macro.
        let out = convert(":::mention\nx\n:::");
        // Should render the original literal `:::mention` form, NOT a
        // structured-macro.
        assert!(
            !out.contains(r#"ac:name="mention""#),
            "directive with no storage macro must not become structured-macro, got: {out}"
        );
        assert!(
            out.contains(":::mention"),
            "fallback literal present, got: {out}"
        );
    }

    // ---- code-fence escape ------------------------------------------------

    #[test]
    fn directive_inside_backtick_code_fence_is_literal() {
        let md = "```\n:::info\nx\n:::\n```";
        let out = convert(md);
        // No info macro should appear — the directive lines were inside a
        // code fence and so should be passed through as code.
        assert!(
            !out.contains(r#"ac:name="info""#),
            "directive inside ``` must NOT become a macro, got: {out}"
        );
        // The literal `:::info` should appear in the code block.
        assert!(
            out.contains(":::info"),
            "literal :::info in code, got: {out}"
        );
    }

    #[test]
    fn directive_inside_code_fence_with_language_is_literal() {
        let md = "```rust\n:::info\nx\n:::\n```";
        let out = convert(md);
        assert!(
            !out.contains(r#"ac:name="info""#),
            "directive inside ```rust must not become a macro, got: {out}"
        );
        assert!(out.contains(":::info"), "got: {out}");
    }

    #[test]
    fn directive_inside_tilde_fence_is_literal() {
        let md = "~~~\n:::info\nx\n:::\n~~~";
        let out = convert(md);
        assert!(
            !out.contains(r#"ac:name="info""#),
            "directive inside ~~~ must not become a macro, got: {out}"
        );
        assert!(out.contains(":::info"), "got: {out}");
    }

    #[test]
    fn directive_outside_code_fence_still_works_after_one() {
        let md = "```\nplain\n```\n\n:::info\nhi\n:::";
        let out = convert(md);
        assert!(
            out.contains(r#"ac:name="info""#),
            "directive after a closed fence still becomes a macro, got: {out}"
        );
    }

    // ---- inline directives ------------------------------------------------

    #[test]
    fn inline_status_with_color() {
        let out = convert(":status[DONE]{color=green}");
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="status">"#),
            "got: {out}"
        );
        assert!(
            out.contains(r#"<ac:parameter ac:name="title">DONE</ac:parameter>"#),
            "got: {out}"
        );
        assert!(
            out.contains(r#"<ac:parameter ac:name="colour">green</ac:parameter>"#),
            "user-facing color must map to British 'colour', got: {out}"
        );
    }

    #[test]
    fn inline_emoticon_emits_self_closing_emoticon_tag() {
        let out = convert(":emoticon{name=warning}");
        assert!(
            out.contains(r#"<ac:emoticon ac:name="warning"/>"#),
            "got: {out}"
        );
        assert!(
            !out.contains(r#"ac:name="emoticon""#),
            "emoticon must NOT render as a structured-macro, got: {out}"
        );
    }

    #[test]
    fn inline_mention_emits_link_with_user_ri() {
        let out = convert(":mention[@john]{accountId=abc123}");
        assert!(
            out.contains(r#"<ac:link><ri:user ri:account-id="abc123"/></ac:link>"#),
            "got: {out}"
        );
    }

    #[test]
    fn inline_link_with_page_id() {
        let out = convert(":link[Page Title]{pageId=12345}");
        assert!(
            out.contains(r#"<ri:page ri:content-id="12345"/>"#),
            "pageId should produce ri:content-id, got: {out}"
        );
        assert!(out.contains("<ac:link>"), "got: {out}");
    }

    #[test]
    fn inline_link_with_only_title_falls_back_to_content_title() {
        let out = convert(":link[Page Title]");
        assert!(
            out.contains(r#"<ri:page ri:content-title="Page Title"/>"#),
            "got: {out}"
        );
    }

    #[test]
    fn inline_image_emits_image_tag() {
        let out = convert(r#":image{src="https://e.com/x.png" alt="diagram"}"#);
        assert!(out.contains("<ac:image"), "got: {out}");
        assert!(
            out.contains(r#"ri:value="https://e.com/x.png""#),
            "got: {out}"
        );
        assert!(out.contains(r#"ac:alt="diagram""#), "got: {out}");
    }

    #[test]
    fn inline_directive_in_paragraph_middle() {
        let out = convert("Hello :emoticon{name=warning} world");
        assert!(
            out.contains(r#"<ac:emoticon ac:name="warning"/>"#),
            "got: {out}"
        );
        // Surrounding text should still be present in the paragraph.
        assert!(out.contains("Hello"), "got: {out}");
        assert!(out.contains("world"), "got: {out}");
    }

    #[test]
    fn inline_directive_in_heading() {
        let out = convert("# Title :status[DONE]{color=green}");
        assert!(out.contains("<h1>"), "got: {out}");
        assert!(
            out.contains(r#"ac:name="status""#),
            "status macro inside heading, got: {out}"
        );
    }

    #[test]
    fn unknown_inline_directive_passes_through_as_literal() {
        // `custom` is not a registered name; the inline parser leaves it as
        // plain text, so it round-trips through comrak as literal text.
        let out = convert(":custom[x]");
        assert!(out.contains(":custom[x]"), "got: {out}");
        assert!(!out.contains(r#"ac:name="custom""#), "got: {out}");
    }

    #[test]
    fn multiple_inline_directives_on_same_line() {
        let out = convert(":status[DONE]{color=green} and :emoticon{name=ok}");
        assert!(out.contains(r#"ac:name="status""#), "got: {out}");
        assert!(out.contains(r#"<ac:emoticon ac:name="ok"/>"#), "got: {out}");
    }

    // ---- XML escaping -----------------------------------------------------

    #[test]
    fn parameter_value_with_ampersand_is_escaped() {
        let out = convert(":::info title=\"A & B\"\nbody\n:::");
        assert!(
            out.contains(r#"<ac:parameter ac:name="title">A &amp; B</ac:parameter>"#),
            "got: {out}"
        );
    }

    #[test]
    fn parameter_value_with_angle_brackets_is_escaped() {
        let out = convert(":::info title=\"<x>\"\n_\n:::");
        assert!(
            out.contains(r#"<ac:parameter ac:name="title">&lt;x&gt;</ac:parameter>"#),
            "got: {out}"
        );
    }

    #[test]
    fn parameter_value_with_embedded_quote_is_escaped() {
        // The directive grammar lets the user escape `"` inside a quoted value
        // with `\"`. The resulting unescaped value is `say "hi"`. The
        // converter must then XML-escape the embedded `"` for safe XHTML.
        let out = convert(
            r#":::info title="say \"hi\""
body
:::"#,
        );
        assert!(
            out.contains(r#"<ac:parameter ac:name="title">say &quot;hi&quot;</ac:parameter>"#),
            "got: {out}"
        );
    }

    // ---- edge cases -------------------------------------------------------

    #[test]
    fn empty_input_produces_empty_output() {
        let out = convert("");
        // Comrak emits "" for an empty input — anything more is fine, but
        // there must be no macro tags.
        assert!(
            !out.contains("<ac:"),
            "empty input must not produce any macros, got: {out}"
        );
    }

    #[test]
    fn whitespace_only_input_is_safe() {
        let out = convert("   \n\n  \n");
        assert!(
            !out.contains("<ac:"),
            "whitespace-only input must not produce any macros, got: {out}"
        );
    }

    #[test]
    fn unclosed_directive_returns_err() {
        let err = markdown_to_storage(":::info\nbody").unwrap_err();
        match err {
            MdToStorageError::Directive(DirectiveError::Unclosed { name, .. }) => {
                assert_eq!(name, "info");
            }
            other => panic!("expected Unclosed, got: {other:?}"),
        }
    }

    #[test]
    fn inline_directive_at_start_of_line() {
        let out = convert(":status[DONE]{color=green} after");
        assert!(out.contains(r#"ac:name="status""#), "got: {out}");
        assert!(out.contains("after"), "got: {out}");
    }

    #[test]
    fn inline_directive_at_end_of_line() {
        let out = convert("before :status[DONE]");
        assert!(out.contains(r#"ac:name="status""#), "got: {out}");
        assert!(out.contains("before"), "got: {out}");
    }
}
