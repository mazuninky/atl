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

/// Local event type used by [`lex_with_code_fences`].
///
/// Wraps the shared [`BlockEvent`] (for directive parsing) and adds a
/// `CodeBlock` variant for complete fenced code blocks. Capturing a code block
/// as a single atomic event (rather than a sequence of `Line`s) lets the
/// renderer emit `<ac:structured-macro ac:name="code">` directly, sidestepping
/// comrak entirely — comrak would otherwise wrap the body in `<pre><code>` and
/// we'd lose the round-trip with the Confluence-native `code` macro.
#[derive(Debug)]
enum LocalEvent {
    /// Pass-through for the directive lexer's events.
    Block(BlockEvent),
    /// A complete fenced code block (from opening fence to matching close).
    /// `lang` is the optional info-string language token; `body` is the raw
    /// content between the fences, joined with `\n`.
    CodeBlock { lang: Option<String>, body: String },
}

/// Walk `md` line by line, tracking CommonMark fenced code-block state.
///
/// Lines outside a code fence go through the directive lexer and become
/// `LocalEvent::Block(BlockEvent::…)`. A complete fenced code block (from its
/// opening fence to its matching close, exclusive of both fence lines) is
/// captured as a single `LocalEvent::CodeBlock` so the renderer can emit it as
/// a native Confluence `code` macro instead of letting it round-trip through
/// comrak's `<pre><code>` output.
fn lex_with_code_fences(md: &str) -> Result<Vec<LocalEvent>, DirectiveError> {
    let mut lex = BlockLexer::new();
    let mut events = Vec::new();
    let mut code_fence: Option<CodeFence> = None;
    // When inside a fence, accumulate the language token and body lines so the
    // close-fence can emit a single CodeBlock event.
    let mut current_lang: Option<String> = None;
    let mut current_body: Vec<String> = Vec::new();

    for line in md.split('\n') {
        match &code_fence {
            Some(open) => {
                if is_matching_close_fence(line, open) {
                    // Close fence — flush the captured body as a single
                    // CodeBlock event and exit fence state.
                    let body = current_body.join("\n");
                    events.push(LocalEvent::CodeBlock {
                        lang: current_lang.take(),
                        body,
                    });
                    current_body.clear();
                    code_fence = None;
                } else {
                    current_body.push(line.to_string());
                }
            }
            None => {
                if let Some(open) = parse_fence_open(line) {
                    current_lang = extract_fence_language(line, open.fence_char);
                    current_body.clear();
                    code_fence = Some(open);
                } else {
                    events.push(LocalEvent::Block(lex.lex_line(line)));
                }
            }
        }
    }

    // Unterminated code fence: flush whatever we have so content isn't lost.
    // CommonMark's behaviour is to treat the rest of the document as code, so
    // mirror that.
    if code_fence.is_some() {
        let body = current_body.join("\n");
        events.push(LocalEvent::CodeBlock {
            lang: current_lang.take(),
            body,
        });
    }

    lex.finalize()?;
    Ok(events)
}

/// Extract the language token (first whitespace-delimited word of the info
/// string) from a fence-open line, lowercased. Returns `None` when the info
/// string is empty.
fn extract_fence_language(line: &str, fence_char: char) -> Option<String> {
    let trimmed = line.trim_start_matches(' ');
    let after_fence = trimmed.trim_start_matches(fence_char);
    let info = after_fence.trim();
    if info.is_empty() {
        return None;
    }
    info.split_whitespace()
        .next()
        .map(|t| t.to_ascii_lowercase())
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
    /// A fenced code block captured atomically by [`lex_with_code_fences`].
    /// Emitted directly as a Confluence `<ac:structured-macro ac:name="code">`
    /// without going through comrak.
    CodeBlock { lang: Option<String>, body: String },
}

/// Frame on the build stack: a directive that is currently open and the
/// children we've collected so far for it.
struct Frame {
    name: String,
    spec: Option<&'static DirectiveSpec>,
    params: BTreeMap<String, String>,
    children: Vec<Node>,
}

fn build_tree(events: Vec<LocalEvent>) -> Vec<Node> {
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
            LocalEvent::Block(BlockEvent::Open {
                name,
                params,
                depth: _,
            }) => {
                let spec = lookup(&name);
                stack.push(Frame {
                    name,
                    spec,
                    params,
                    children: Vec::new(),
                });
            }
            LocalEvent::Block(BlockEvent::Close { .. }) => {
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
            LocalEvent::Block(BlockEvent::Line(line)) => {
                let target = stack
                    .last_mut()
                    .map_or(&mut root, |frame| &mut frame.children);
                push_line(target, line);
            }
            LocalEvent::CodeBlock { lang, body } => {
                let target = stack
                    .last_mut()
                    .map_or(&mut root, |frame| &mut frame.children);
                target.push(Node::CodeBlock { lang, body });
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
            Node::CodeBlock { lang, body } => {
                out.push_str(&render_code_macro(lang.as_deref(), body));
            }
        }
    }
    out
}

/// Render a fenced code block as a Confluence `<ac:structured-macro
/// ac:name="code">`. The body goes inside `<ac:plain-text-body>` wrapped in
/// CDATA so XHTML escaping is bypassed; the optional language becomes the
/// `language` parameter. We deliberately do not round-trip `breakoutMode`,
/// `theme`, line-number flags, etc. — the read side drops them, and the write
/// side rebuilds just the structural minimum, which is enough for Confluence
/// to render the block correctly.
fn render_code_macro(lang: Option<&str>, body: &str) -> String {
    let mut out = String::new();
    out.push_str(r#"<ac:structured-macro ac:name="code">"#);
    if let Some(l) = lang
        && !l.is_empty()
    {
        push_parameter(&mut out, "language", l);
    }
    out.push_str("<ac:plain-text-body><![CDATA[");
    out.push_str(&cdata_escape(body));
    out.push_str("]]></ac:plain-text-body>");
    out.push_str("</ac:structured-macro>");
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
    } else if !body.trim().is_empty() {
        // Spec forbids a body, but the user wrote one anyway. Emitting the
        // self-closing macro would silently drop their content, which is
        // worse than degrading visibly — pass the original literal through
        // so the user sees their input survived.
        render_unknown_block(name, params, body)
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
///
/// All three pieces (`name`, the rendered `params` string, and `body`) are
/// XML-escaped before concatenation so a name like `weird<x>` or a body that
/// contains `&`/`<`/`>` cannot break the surrounding storage XHTML. The
/// `:::` fences, spaces, and newlines remain unescaped (they are pure ASCII
/// and never carry XML metacharacters).
fn render_unknown_block(name: &str, params: &BTreeMap<String, String>, body: &str) -> String {
    let mut out = String::new();
    out.push_str(":::");
    out.push_str(&xml_escape(name));
    if !params.is_empty() {
        out.push(' ');
        let rendered = crate::cli::commands::directives::render_attrs(params);
        out.push_str(&xml_escape(&rendered));
    }
    out.push('\n');
    out.push_str(&xml_escape(body));
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
    let with_inline = substitute_inline_directives(md, &mut placeholders);

    // Pre-extract raw Confluence-storage tag blocks (`<ac:…>` / `<ri:…>`) so
    // comrak doesn't mangle them. CommonMark restricts raw-HTML tag names to
    // `[A-Za-z][A-Za-z0-9-]*`; the colon in `ac:structured-macro` fails that
    // regex and comrak escapes the angle brackets, destroying the macro.
    // Replacing each block with an HTML-comment placeholder shields it through
    // the comrak pass and we restore the original bytes verbatim afterwards.
    let mut raw_xhtml: Vec<String> = Vec::new();
    let with_both = substitute_raw_xhtml(&with_inline, &mut raw_xhtml);

    let html = markdown_to_html(&with_both, &gfm_options());

    let html = if placeholders.is_empty() {
        html
    } else {
        restore_inline_directives(&html, &placeholders)
    };
    let html = if raw_xhtml.is_empty() {
        html
    } else {
        restore_raw_xhtml(&html, &raw_xhtml)
    };

    rewrite_stripped_html_tags(&html)
}

/// Atlassian's storage-format sanitiser silently drops the keyboard-input
/// (`<kbd>`), sample-output (`<samp>`), and variable (`<var>`) tags, so
/// after a round-trip the user's `Press <kbd>Ctrl</kbd>` shows up as plain
/// `Press Ctrl`. Storage *does* preserve `<code>`, so the cheapest survival
/// trick is to rewrite each of these tags as `<code>` before the body
/// reaches the API. The visual presentation differs slightly (monospace vs
/// keyboard-key chrome) but the structural information survives.
///
/// The substitution is purely lexical — the input here is comrak-rendered
/// HTML, so we only have to recognise the literal opening / closing tags.
/// We do NOT touch attributes (the affected tags are bare in any realistic
/// markdown source) and we do NOT recurse into nested forms; back-to-back
/// `<kbd>` / `<samp>` / `<var>` runs each get their own pair of `<code>`
/// tags, mirroring the original structure.
fn rewrite_stripped_html_tags(html: &str) -> String {
    const STRIPPED_TAGS: &[&str] = &["kbd", "samp", "var"];
    let mut out = html.to_string();
    for tag in STRIPPED_TAGS {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        out = out.replace(&open, "<code>").replace(&close, "</code>");
    }
    out
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

/// Walk `md` line by line and pull out blocks that begin with a literal
/// Confluence-storage tag (`<ac:…>` or `<ri:…>`). Each captured block is
/// pushed to `out_raw` and replaced with an `<!--ATL_RAW_{n}-->` placeholder
/// so it survives comrak intact.
///
/// Why: the read-side converter ([`super::storage_to_md`]) emits unknown
/// Confluence macros (jira-issue, gallery, panel, …) as their raw `<ac:…>`
/// XHTML inline in markdown. Comrak then sees an opening tag with a `:` in
/// the name; CommonMark §6.6 requires raw-HTML tag names to match
/// `[A-Za-z][A-Za-z0-9-]*` (no colon), so comrak escapes the angle brackets
/// and the macro structure is destroyed. Shielding these blocks behind an
/// HTML-comment placeholder bypasses comrak entirely.
///
/// The function is conservative: anything it can't cleanly parse is left
/// untouched, so the worst-case outcome is the previous (already-broken)
/// behaviour rather than a partial extraction that corrupts surrounding text.
fn substitute_raw_xhtml(md: &str, out_raw: &mut Vec<String>) -> String {
    let lines: Vec<&str> = md.split('\n').collect();
    let mut out = String::with_capacity(md.len());
    let mut code_fence: Option<CodeFence> = None;
    let mut i = 0;
    let mut first = true;

    while i < lines.len() {
        let line = lines[i];
        if !first {
            out.push('\n');
        }
        first = false;

        // Inside a fenced code block — pass everything through verbatim.
        if let Some(open) = code_fence {
            out.push_str(line);
            if is_matching_close_fence(line, &open) {
                code_fence = None;
            }
            i += 1;
            continue;
        }

        // New opening fence on this line — pass through and enter fence state.
        if let Some(open) = parse_fence_open(line) {
            out.push_str(line);
            code_fence = Some(open);
            i += 1;
            continue;
        }

        // Look for an opening `<ac:NAME …` / `<ri:NAME …` at the start of the
        // line (allow up to 3 leading spaces, matching CommonMark indented
        // tolerance for block-level HTML).
        if let Some((consumed, raw)) = try_extract_raw_block(&lines, i) {
            let idx = out_raw.len();
            out_raw.push(raw);
            let _ = write!(out, "<!--ATL_RAW_{idx}-->");
            // Subsequent lines are already encoded inside `raw`; advance past
            // them, but the trailing newlines between them were not emitted to
            // `out` (we only emitted the placeholder), so update `first` state
            // and skip lines.
            i += consumed;
            continue;
        }

        out.push_str(line);
        i += 1;
    }

    out
}

/// Try to identify a single `<ac:NAME …>…</ac:NAME>` or `<ri:NAME …/>` block
/// starting at `lines[start]`. On success, return `(lines_consumed,
/// raw_xhtml_string)`. On failure, return `None`.
///
/// Recognised forms:
/// - `<ac:NAME …/>` — self-closing on one line.
/// - `<ri:NAME …/>` — resource identifier, always self-closing.
/// - `<ac:NAME …>` — opening tag; consume lines until the matching
///   `</ac:NAME>` close is found, tracking opens/closes of the SAME local
///   name for depth.
/// - `<ri:NAME …>` — same as above, treated as a block tag.
///
/// Returns `None` if the start line doesn't begin with such a tag, if the tag
/// is malformed, or if no matching close is found before EOF.
fn try_extract_raw_block(lines: &[&str], start: usize) -> Option<(usize, String)> {
    let first_line = lines[start];
    let trimmed = first_line.trim_start_matches(' ');
    let indent = first_line.len() - trimmed.len();
    if indent > 3 {
        return None;
    }
    let bytes = trimmed.as_bytes();
    if bytes.first()? != &b'<' {
        return None;
    }
    // Read the tag name: must start with `ac:` or `ri:`, followed by a
    // CommonMark-style name character run.
    let rest = &trimmed[1..];
    let (ns, after_ns) = if let Some(r) = rest.strip_prefix("ac:") {
        ("ac", r)
    } else if let Some(r) = rest.strip_prefix("ri:") {
        ("ri", r)
    } else {
        return None;
    };
    let name_end = after_ns
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .unwrap_or(after_ns.len());
    if name_end == 0 {
        return None;
    }
    let local_name = &after_ns[..name_end];
    let full_name = format!("{ns}:{local_name}");

    // Parse the opening tag span (could span multiple lines if attributes
    // wrap). Collect raw bytes from `first_line`'s `<` onward, walking lines
    // until we see the closing `>` of the opening tag. Track whether it's
    // self-closing (ends with `/>`).
    //
    // To keep behaviour conservative, only accept openers that close on the
    // same line — multi-line opening tags are rare in Confluence storage and
    // any false negative is harmless (the original line is left intact).
    let tag_close = bytes.iter().position(|&b| b == b'>')?;
    let open_tag = &trimmed[..=tag_close];
    let self_closing = open_tag.ends_with("/>");
    // Pre-fix: capture the slice of `first_line` starting at the `<` so we
    // include the original characters (no leading-space trimming on output).
    let tag_offset_in_line = indent;
    let leading = &first_line[..tag_offset_in_line];
    // Any text on the first line BEFORE the tag must be only whitespace —
    // we already trimmed leading spaces and indent <= 3 — that's enforced.
    // If leading is non-empty whitespace, that's fine; if it's non-whitespace,
    // bail (we can't extract a partial line cleanly).
    if !leading.chars().all(|c| c == ' ') {
        return None;
    }

    // Self-closing: the entire block fits on the start line, but require
    // there's no trailing non-whitespace AFTER the tag (otherwise the original
    // line mixes raw XHTML with markdown and we shouldn't extract).
    if self_closing {
        let after_tag = &trimmed[tag_close + 1..];
        if !after_tag.trim().is_empty() {
            return None;
        }
        return Some((1, open_tag.to_string()));
    }

    // Non-self-closing: track depth of `<NAME` opens vs `</NAME>` closes
    // across subsequent lines until depth returns to 0.
    let after_open = &trimmed[tag_close + 1..];
    let mut raw = String::new();
    raw.push_str(open_tag);
    let mut depth: i32 = 1;

    // Helper: count occurrences of `<NAME ` (open) and `</NAME>` (close) in a
    // string. Returns (opens, closes).
    fn count_tags(haystack: &str, full_name: &str) -> (i32, i32) {
        let open_self = format!("<{full_name}/>");
        let close = format!("</{full_name}>");
        // First, count strict self-closing occurrences of the same name: these
        // are balanced (no contribution to depth). We don't subtract them
        // because we only count open tags that aren't self-closing; the open
        // match below uses `<NAME>` or `<NAME ` (with trailing space or `>`).
        let open_with_space = format!("<{full_name} ");
        let open_bare = format!("<{full_name}>");
        let mut opens = 0i32;
        let mut closes = 0i32;
        // Scan once linearly, since these substrings may overlap (closes
        // contain `</…>` which is distinct from `<…`).
        let bytes = haystack.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'<' {
                i += 1;
                continue;
            }
            // `</NAME>` — close.
            if haystack[i..].starts_with(&close) {
                closes += 1;
                i += close.len();
                continue;
            }
            // `<NAME/>` — self-closing of the SAME name: balanced, skip.
            if haystack[i..].starts_with(&open_self) {
                i += open_self.len();
                continue;
            }
            // `<NAME ` or `<NAME>` — opening tag.
            if haystack[i..].starts_with(&open_with_space) || haystack[i..].starts_with(&open_bare)
            {
                opens += 1;
                i += 1;
                continue;
            }
            i += 1;
        }
        (opens, closes)
    }

    // Apply tag counting to the rest of the start line (after the opener).
    let (o, c) = count_tags(after_open, &full_name);
    depth = depth.saturating_add(o).saturating_sub(c);
    raw.push_str(after_open);

    let mut consumed = 1usize;

    while depth > 0 {
        let idx = start + consumed;
        if idx >= lines.len() {
            // Unterminated — bail.
            return None;
        }
        let line = lines[idx];
        raw.push('\n');
        let (o, c) = count_tags(line, &full_name);
        depth = depth.saturating_add(o).saturating_sub(c);
        raw.push_str(line);
        consumed += 1;
    }

    // After balancing, ensure the line that closed the block has no
    // additional non-whitespace content AFTER the final `</NAME>` — otherwise
    // we'd be mixing raw XHTML with markdown on the same line and the
    // extraction is unsafe.
    let last_line_idx = start + consumed - 1;
    let last_line = lines[last_line_idx];
    let close_tag = format!("</{full_name}>");
    if let Some(pos) = last_line.rfind(&close_tag) {
        let after = &last_line[pos + close_tag.len()..];
        if !after.trim().is_empty() {
            return None;
        }
    }

    Some((consumed, raw))
}

/// Replace `<!--ATL_RAW_{n}-->` placeholders with the original raw XHTML
/// captured during [`substitute_raw_xhtml`]. Restored verbatim — no escaping.
fn restore_raw_xhtml(html: &str, raw: &[String]) -> String {
    const PREFIX: &str = "<!--ATL_RAW_";
    const SUFFIX: &str = "-->";

    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(pos) = rest.find(PREFIX) {
        out.push_str(&rest[..pos]);
        let after_prefix = &rest[pos + PREFIX.len()..];
        let end = match after_prefix.find(SUFFIX) {
            Some(e) => e,
            None => {
                out.push_str(&rest[pos..]);
                rest = "";
                break;
            }
        };
        let digits = &after_prefix[..end];
        let idx: usize = match digits.parse() {
            Ok(n) => n,
            Err(_) => {
                out.push_str(&rest[pos..pos + PREFIX.len() + end + SUFFIX.len()]);
                rest = &rest[pos + PREFIX.len() + end + SUFFIX.len()..];
                continue;
            }
        };
        match raw.get(idx) {
            Some(s) => out.push_str(s),
            None => {
                out.push_str(&rest[pos..pos + PREFIX.len() + end + SUFFIX.len()]);
            }
        }
        rest = &rest[pos + PREFIX.len() + end + SUFFIX.len()..];
    }
    out.push_str(rest);
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
/// The opening and closing backtick runs are included in the `CodeSpan`
/// segments so the original byte sequence is preserved on rejoin.
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
        // Count opener length.
        let opener_start = i;
        while i < bytes.len() && bytes[i] == b'`' {
            i += 1;
        }
        let opener_len = i - opener_start;

        // Search for a matching close run of exactly `opener_len` backticks.
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
            // Wrong-length run inside the span — keep scanning.
        };

        if let Some(close_end) = close_end {
            // Flush "outside" prefix up to opener_start.
            if opener_start > outside_start {
                segments.push(LineSegment::Outside(&line[outside_start..opener_start]));
            }
            segments.push(LineSegment::CodeSpan(&line[opener_start..close_end]));
            outside_start = close_end;
            i = close_end;
        }
        // No close found — the opener is just literal text. Continue scanning
        // from where we are; outside_start stays put.
    }

    if outside_start < line.len() {
        segments.push(LineSegment::Outside(&line[outside_start..]));
    }
    segments
}

/// Run the inline parser against one line and append the result (with
/// placeholders for directives) to `out`.
///
/// The line is split into "outside" and "inside" segments by inline code
/// spans (`` `…` ``, `` ``…`` ``, etc.). Only the "outside" segments are sent
/// through [`parse_inline`] — anything inside a code span is passed through
/// verbatim so directives like `` `:status[Done]` `` survive into comrak as
/// literal code-span content.
///
/// Limitation: indented (4-space) code blocks are NOT recognised here. They
/// would require a markdown AST walk; the line-oriented pre-pass cannot tell
/// an indented code line from a regular paragraph by itself.
fn substitute_line(line: &str, out: &mut String, out_placeholders: &mut Vec<InlineDirective>) {
    for segment in split_code_span_segments(line) {
        match segment {
            LineSegment::Outside(s) => substitute_outside_segment(s, out, out_placeholders),
            LineSegment::CodeSpan(s) => out.push_str(s),
        }
    }
}

fn substitute_outside_segment(
    segment: &str,
    out: &mut String,
    out_placeholders: &mut Vec<InlineDirective>,
) {
    let tokens = parse_inline(segment);
    if tokens.iter().all(|t| matches!(t, InlineToken::Text(_))) {
        out.push_str(segment);
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
/// - `link` becomes `<ac:link><ri:page …/></ac:link>`. When `pageId` is set
///   we emit `ri:page-id` (DC/Server resolves this, Cloud strips it) and,
///   if the user bracketed display content, also `ri:content-title` (Cloud
///   resolves this) so the same storage works on both flavours.
///   `spaceKey=` becomes `ri:space-key` for cross-space references.
///   `title=` overrides the bracketed text as the `ri:content-title` source,
///   so the visible link text and the resolved page title can differ.
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
    // External URL takes priority over Confluence page references — if the
    // user wrote `:link[Docs]{url="https://example.com"}` we emit a plain
    // HTML anchor so the URL isn't silently dropped. Without this branch the
    // `<ac:link><ri:page/>` fallback would render as a broken page link.
    if let Some(url) = d.params.get("url") {
        let label = d
            .content
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(url);
        let mut out = String::new();
        out.push_str(r#"<a href=""#);
        out.push_str(&xml_escape(url));
        out.push_str(r#"">"#);
        out.push_str(&xml_escape(label));
        out.push_str("</a>");
        return out;
    }

    let mut out = String::new();
    out.push_str("<ac:link>");
    // Emit BOTH `ri:page-id` and `ri:content-title` when we have them so the
    // same storage XML renders correctly on Cloud and Data Center:
    //
    // - Cloud silently strips `ri:page-id` (and the legacy `ri:content-id`)
    //   from `<ri:page>` references, leaving an empty self-link unless
    //   `ri:content-title` is also present. So Cloud needs the title.
    // - DC/Server uses `ri:page-id` directly and ignores the title attribute,
    //   which means it works even when the page has been retitled.
    //
    // Emitting both gives a working link in both flavours. When the user
    // didn't bracket any content (so no title to use), we fall back to
    // `ri:page-id` alone — DC will resolve it; Cloud will render an empty
    // link, which is the existing behaviour and can't be fixed without a
    // title to emit.
    //
    // `spaceKey=` (when present) becomes `ri:space-key` so cross-space links
    // resolve against the named space instead of the current one.
    // An explicit `title=` attribute takes precedence over the bracketed
    // content as the `ri:content-title` source — the visible link text
    // (bracketed content) and the resolved page title can legitimately differ.
    let explicit_title = d
        .params
        .get("title")
        .map(String::as_str)
        .filter(|s| !s.is_empty());
    let bracketed = d.content.as_deref().filter(|s| !s.is_empty());
    let title_for_page = explicit_title.or(bracketed);
    let space_key = d
        .params
        .get("spaceKey")
        .map(String::as_str)
        .filter(|s| !s.is_empty());
    let page_id = d
        .params
        .get("pageId")
        .map(String::as_str)
        .filter(|s| !s.is_empty());

    // Attribute order on `<ri:page …/>`: `ri:page-id`, `ri:space-key`,
    // `ri:content-title` — matches the order Confluence itself emits.
    if page_id.is_none() && space_key.is_none() && title_for_page.is_none() {
        out.push_str("<ri:page/>");
    } else {
        out.push_str("<ri:page");
        if let Some(pid) = page_id {
            out.push_str(r#" ri:page-id=""#);
            out.push_str(&xml_escape(pid));
            out.push('"');
        }
        if let Some(sk) = space_key {
            out.push_str(r#" ri:space-key=""#);
            out.push_str(&xml_escape(sk));
            out.push('"');
        }
        if let Some(title) = title_for_page {
            out.push_str(r#" ri:content-title=""#);
            out.push_str(&xml_escape(title));
            out.push('"');
        }
        out.push_str("/>");
    }
    // Emit `<ac:plain-text-link-body>` so the user's bracketed display text
    // is preserved on the page. Without it, Confluence falls back to
    // rendering the page title — which is empty when the resource ref is
    // unresolved. Wrap in CDATA, splitting any literal `]]>` sequence
    // across two CDATA sections so it doesn't terminate the body early.
    if let Some(content) = d.content.as_ref().filter(|s| !s.is_empty()) {
        out.push_str("<ac:plain-text-link-body><![CDATA[");
        out.push_str(&cdata_escape(content));
        out.push_str("]]></ac:plain-text-link-body>");
    }
    out.push_str("</ac:link>");
    out
}

/// Escape literal `]]>` inside CDATA content by splitting it across two
/// CDATA sections — the only sequence that can prematurely terminate a
/// CDATA block. Returns a borrowed slice when the content is already safe.
fn cdata_escape(s: &str) -> Cow<'_, str> {
    if !s.contains("]]>") {
        return Cow::Borrowed(s);
    }
    Cow::Owned(s.replace("]]>", "]]]]><![CDATA[>"))
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
    fn hard_break_emits_br_tag() {
        // CommonMark hard break: two trailing spaces before a newline. The
        // storage XML must preserve the break as a `<br/>` element so a
        // round-trip through storage_to_markdown reproduces the two-space
        // marker. Comrak emits `<br />` (with a space and self-closing
        // slash) — both are valid in Confluence storage XHTML.
        let out = convert("Foo  \nBar");
        assert!(
            out.contains("<br />") || out.contains("<br/>"),
            "expected <br/> between Foo and Bar, got: {out}"
        );
        assert!(out.contains("Foo"), "got: {out}");
        assert!(out.contains("Bar"), "got: {out}");
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
    fn code_fence_emits_structured_macro() {
        // Fenced code blocks now emit the native Confluence `code` macro instead
        // of `<pre><code>`. The macro round-trips cleanly through
        // `storage_to_md::storage_to_markdown` back into a fenced block.
        let out = convert("```\nplain\n```");
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="code">"#),
            "got: {out}"
        );
        assert!(
            out.contains("<ac:plain-text-body><![CDATA[plain]]></ac:plain-text-body>"),
            "got: {out}"
        );
        assert!(out.contains("</ac:structured-macro>"), "got: {out}");
        assert!(
            !out.contains("<pre>"),
            "code macro must not render as <pre>, got: {out}"
        );
    }

    #[test]
    fn code_fence_with_language_emits_language_parameter() {
        let out = convert("```python\nprint(\"hi\")\n```");
        assert!(
            out.contains(r#"<ac:parameter ac:name="language">python</ac:parameter>"#),
            "expected language parameter, got: {out}"
        );
        assert!(
            out.contains(r#"<![CDATA[print("hi")]]>"#),
            "expected body inside CDATA, got: {out}"
        );
    }

    #[test]
    fn code_fence_without_language_omits_language_parameter() {
        let out = convert("```\nplain\n```");
        // `<ac:structured-macro ac:name="code">` must be followed immediately by
        // `<ac:plain-text-body>` (no `<ac:parameter>` in between).
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="code"><ac:plain-text-body>"#),
            "expected no language parameter between macro and body, got: {out}"
        );
    }

    #[test]
    fn code_fence_with_cdata_terminator_in_body_splits_cdata() {
        // `cdata_escape` rewrites `]]>` as `]]]]><![CDATA[>` so the CDATA section
        // remains valid in the rendered macro.
        let out = convert("```\nfoo ]]> bar\n```");
        assert!(
            out.contains("]]]]><![CDATA[>"),
            "expected CDATA terminator to be split, got: {out}"
        );
    }

    #[test]
    fn code_fence_with_multiple_lines_preserves_newlines() {
        let out = convert("```\nline1\nline2\nline3\n```");
        assert!(
            out.contains("<![CDATA[line1\nline2\nline3]]>"),
            "expected newline-separated lines inside CDATA, got: {out}"
        );
    }

    #[test]
    fn code_fence_with_tilde_fence_also_emits_macro() {
        let out = convert("~~~python\ncode\n~~~");
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="code">"#),
            "tilde fence must produce code macro, got: {out}"
        );
        assert!(
            out.contains(r#"<ac:parameter ac:name="language">python</ac:parameter>"#),
            "tilde fence must keep language parameter, got: {out}"
        );
        assert!(
            out.contains("<![CDATA[code]]>"),
            "tilde fence must keep body, got: {out}"
        );
    }

    // ---- raw `<ac:…>` / `<ri:…>` survives comrak ---------------------------

    #[test]
    fn raw_ac_structured_macro_block_survives_comrak() {
        // Without the pre-extraction pass, comrak would see the `:` in
        // `<ac:structured-macro>` (rejected by CommonMark §6.6) and escape
        // every `<` to `&lt;`, destroying the macro.
        let md = concat!(
            "Some prose.\n",
            "\n",
            "<ac:structured-macro ac:name=\"jira-issue\" ac:schema-version=\"1\">",
            "<ac:parameter ac:name=\"key\">FOO-123</ac:parameter>",
            "</ac:structured-macro>\n",
            "\n",
            "More prose.",
        );
        let out = convert(md);
        assert!(
            out.contains("<ac:structured-macro ac:name=\"jira-issue\""),
            "raw macro tag must survive verbatim, got: {out}"
        );
        assert!(
            out.contains("</ac:structured-macro>"),
            "raw macro must keep its close tag, got: {out}"
        );
        assert!(
            out.contains("FOO-123"),
            "issue key must survive, got: {out}"
        );
        assert!(
            !out.contains("&lt;ac:"),
            "macro must NOT be escaped to &lt;, got: {out}"
        );
    }

    #[test]
    fn raw_ac_with_multiline_body_survives_comrak() {
        // A multi-line macro with a `<ac:rich-text-body>` child must be captured
        // as a single block and replayed verbatim.
        let md = concat!(
            "<ac:structured-macro ac:name=\"info\">\n",
            "<ac:rich-text-body>\n",
            "<p>Hello</p>\n",
            "</ac:rich-text-body>\n",
            "</ac:structured-macro>",
        );
        let out = convert(md);
        assert!(
            out.contains("<ac:rich-text-body>"),
            "rich-text-body must survive verbatim, got: {out}"
        );
        assert!(
            out.contains("<p>Hello</p>"),
            "inner content must survive, got: {out}"
        );
        assert!(
            !out.contains("&lt;ac:"),
            "macro must not be escaped, got: {out}"
        );
    }

    #[test]
    fn raw_ri_self_closing_block_survives_comrak() {
        let md = r#"<ri:user ri:userkey="abc"/>"#;
        let out = convert(md);
        assert!(
            out.contains(r#"<ri:user ri:userkey="abc"/>"#),
            "self-closing ri: tag must survive verbatim, got: {out}"
        );
        assert!(
            !out.contains("&lt;ri:"),
            "ri: tag must not be escaped, got: {out}"
        );
    }

    #[test]
    fn raw_ac_inside_fenced_code_block_is_not_extracted() {
        // The extractor must respect fenced code regions — content inside a
        // ```...``` block is documentation, not actual storage XHTML.
        let md = "```\n<ac:structured-macro ac:name=\"x\"/>\n```";
        let out = convert(md);
        // The whole block is now the new code-macro shape with the raw text
        // preserved inside CDATA.
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="code">"#),
            "outer block must be a code macro, got: {out}"
        );
        assert!(
            out.contains("<![CDATA[<ac:structured-macro ac:name=\"x\"/>]]>"),
            "inner tag must appear as literal text inside CDATA, got: {out}"
        );
    }

    #[test]
    fn raw_ac_followed_by_more_markdown_is_extracted_cleanly() {
        // The extractor must not consume trailing markdown.
        let md = "<ac:structured-macro ac:name=\"x\"/>\n\n# Heading after";
        let out = convert(md);
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="x"/>"#),
            "raw self-closing tag must survive, got: {out}"
        );
        assert!(
            out.contains("<h1>Heading after</h1>"),
            "trailing markdown heading must still render, got: {out}"
        );
    }

    #[test]
    fn malformed_ac_tag_falls_back_to_comrak_escaping() {
        // `try_extract_raw_block` returns `None` on malformed input, so the
        // line falls back to comrak's previous (escaping) behaviour. The test
        // documents the no-regression contract: we accept either the escaped
        // or the literal form, but the conversion must NOT panic.
        let out = convert("<ac:not-closed without close tag");
        assert!(
            out.contains("&lt;ac:") || out.contains("<ac:not-closed"),
            "expected either escaped or literal form (no panic, no regression), got: {out}"
        );
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
    fn block_fence_with_inline_only_name_does_not_emit_structured_macro() {
        // `mention` is registered as INLINE; using it in block-fence form
        // (`:::mention`) is a kind mismatch. The block lexer must fall
        // through (the `:::mention` line is treated as plain text), and the
        // overall conversion must NOT produce a Confluence structured-macro
        // node — `mention` has no storage-macro mapping.
        let out = convert(":::mention\nx\n:::");
        assert!(
            !out.contains(r#"ac:name="mention""#),
            "kind-mismatched block fence must not become structured-macro, got: {out}"
        );
        // The literal `x` body must still be emitted somewhere in the
        // output (it's not a directive body, it's just a paragraph line).
        assert!(out.contains('x'), "body line must round-trip, got: {out}");
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
        // Both `ri:page-id` (for DC/Server) and `ri:content-title` (for Cloud,
        // which strips `ri:page-id`) must be emitted on the same `<ri:page>`.
        assert!(
            out.contains(r#"ri:page-id="12345""#),
            "pageId should produce ri:page-id, got: {out}"
        );
        assert!(
            out.contains(r#"ri:content-title="Page Title""#),
            "bracketed content should also be emitted as ri:content-title for Cloud, got: {out}"
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
    fn link_with_pageid_emits_page_id_attribute_and_link_body() {
        let out = convert(":link[Parent]{pageId=98420}");
        // The single `<ri:page>` element must carry both `ri:page-id` (which
        // DC honours) and `ri:content-title` (which Cloud needs because it
        // silently strips `ri:page-id`).
        assert!(
            out.contains(r#"ri:page-id="98420""#),
            "pageId must emit ri:page-id (not ri:content-id), got: {out}"
        );
        assert!(
            out.contains(r#"ri:content-title="Parent""#),
            "bracketed content must also be emitted as ri:content-title so Cloud can resolve the link, got: {out}"
        );
        assert!(
            out.contains(
                r#"<ac:plain-text-link-body><![CDATA[Parent]]></ac:plain-text-link-body>"#
            ),
            "display text must be wrapped in plain-text-link-body, got: {out}"
        );
    }

    #[test]
    fn link_with_pageid_and_content_emits_both_attrs_for_cloud_and_dc() {
        // Confluence Cloud silently strips `ri:page-id` from `<ri:page>` but
        // honours `ri:content-title`; DC honours `ri:page-id` and ignores the
        // title. Emitting both means the same storage XML renders correctly
        // on both flavours.
        let out = convert(":link[Parent]{pageId=98420}");
        assert!(
            out.contains(r#"ri:page-id="98420""#),
            "ri:page-id must be present for DC/Server: {out}"
        );
        assert!(
            out.contains(r#"ri:content-title="Parent""#),
            "ri:content-title must be present for Cloud: {out}"
        );
        // Both attributes must live on the same `<ri:page>` element, not in
        // sibling resource references.
        let opens: Vec<_> = out.match_indices("<ri:page").collect();
        assert_eq!(
            opens.len(),
            1,
            "expected exactly one <ri:page> element, got: {out}"
        );
    }

    #[test]
    fn link_with_pageid_only_no_content_omits_content_title() {
        // No bracketed content means we have nothing meaningful to put in
        // `ri:content-title`. Emit just `ri:page-id` so DC still resolves
        // the link; on Cloud the link will render empty (same as before —
        // can't be fixed without a title).
        let out = convert(":link[]{pageId=99}");
        assert!(
            out.contains(r#"<ri:page ri:page-id="99"/>"#),
            "no content means only ri:page-id, no ri:content-title, got: {out}"
        );
        assert!(
            !out.contains("ri:content-title"),
            "no content means no ri:content-title attribute, got: {out}"
        );
    }

    #[test]
    fn link_with_title_only_emits_content_title_and_link_body() {
        let out = convert(":link[Foo]");
        assert!(
            out.contains(r#"<ri:page ri:content-title="Foo"/>"#),
            "title-only link must use ri:content-title, got: {out}"
        );
        assert!(
            out.contains(r#"<ac:plain-text-link-body><![CDATA[Foo]]></ac:plain-text-link-body>"#),
            "display text must also appear in plain-text-link-body, got: {out}"
        );
    }

    #[test]
    fn link_with_pageid_and_no_content_omits_link_body() {
        let out = convert(":link[]{pageId=99}");
        assert!(
            out.contains(r#"<ri:page ri:page-id="99"/>"#),
            "pageId must still emit ri:page-id, got: {out}"
        );
        assert!(
            !out.contains("ac:plain-text-link-body"),
            "no content means no link body, got: {out}"
        );
    }

    #[test]
    fn link_with_space_key_emits_ri_space_key_for_cross_space_reference() {
        // Bug 1: `spaceKey=` was silently dropped, breaking cross-space
        // links unless the current space happened to contain a page with
        // the same title.
        let out = convert(r#":link[Some page]{title="Some page" spaceKey=OTHERSPACE}"#);
        assert!(
            out.contains(r#"ri:space-key="OTHERSPACE""#),
            "spaceKey must be emitted as ri:space-key, got: {out}"
        );
        assert!(
            out.contains(r#"ri:content-title="Some page""#),
            "title must be emitted as ri:content-title, got: {out}"
        );
        assert!(
            out.contains(
                r#"<ac:plain-text-link-body><![CDATA[Some page]]></ac:plain-text-link-body>"#
            ),
            "bracketed content must wrap the link body, got: {out}"
        );
    }

    #[test]
    fn link_with_title_overrides_bracketed_content_as_content_title() {
        // Bug 2: `title=` was ignored when it differed from the bracketed
        // text, so the link pointed at a page with the visible label as
        // its title rather than the actual target page.
        let out = convert(r#":link[clickable text]{title="Real Target Page Title"}"#);
        assert!(
            out.contains(r#"ri:content-title="Real Target Page Title""#),
            "title attr must override bracketed content as ri:content-title, got: {out}"
        );
        assert!(
            out.contains(
                r#"<ac:plain-text-link-body><![CDATA[clickable text]]></ac:plain-text-link-body>"#
            ),
            "bracketed content (not title) must wrap the link body, got: {out}"
        );
    }

    #[test]
    fn link_body_cdata_escapes_close_marker() {
        // The CDATA body must split a literal `]]>` so it doesn't terminate
        // the CDATA section early. The standard escape is `]]]]><![CDATA[>`.
        // Test the helper directly because the inline directive parser
        // stops bracket content at the first `]`, so `]]>` cannot reach
        // the renderer through normal markdown source.
        assert_eq!(cdata_escape("safe"), "safe");
        assert_eq!(cdata_escape("a]]>b"), "a]]]]><![CDATA[>b");
        assert_eq!(cdata_escape("]]>x]]>y"), "]]]]><![CDATA[>x]]]]><![CDATA[>y");
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

    #[test]
    fn self_closing_directive_with_body_passes_through_literal() {
        // Bug 4: `:::toc` is self-closing (allows_body == false). When the
        // user wrote a body anyway, silently dropping it is worse than
        // emitting the literal markdown so the user sees their content.
        let out = convert(":::toc\nstray text\n:::");
        // The self-closing structured-macro must NOT be emitted because we
        // chose to degrade visibly instead.
        assert!(
            !out.contains(r#"<ac:structured-macro ac:name="toc">"#),
            "expected literal passthrough, got: {out}"
        );
        // The original body content must still be visible somewhere in the output.
        assert!(out.contains("stray text"), "got: {out}");
    }

    #[test]
    fn self_closing_directive_without_body_still_emits_macro() {
        // Bug 4 sibling: when there's no body content, the self-closing
        // structured macro is still the right thing to emit.
        let out = convert(":::toc\n:::");
        assert!(
            out.contains(r#"<ac:structured-macro ac:name="toc">"#),
            "got: {out}"
        );
        assert!(
            !out.contains("<ac:rich-text-body>"),
            "self-closing macro must not have a body, got: {out}"
        );
    }

    #[test]
    fn inline_directive_inside_code_span_is_not_rewritten() {
        // Bug 5: a `:status[…]` inside an inline code span must NOT be
        // rewritten by the pre-pass. comrak should see the original literal
        // as code-span content.
        let out = convert("Run `:status[DONE]` to set status.");
        // No `status` macro should be emitted — the directive is inside a code span.
        assert!(
            !out.contains(r#"ac:name="status""#),
            "directive inside `…` was rewritten: {out}"
        );
        // The literal `:status[DONE]` should appear inside `<code>…</code>`.
        assert!(
            out.contains("<code>:status[DONE]</code>"),
            "expected literal inside code span, got: {out}"
        );
    }

    #[test]
    fn inline_directive_outside_code_span_on_same_line_still_works() {
        // Bug 5 follow-up: only the code-span content is skipped; directives
        // before/after the span are still rewritten.
        let out = convert("Run `:status[DONE]` then :status[OK]{color=green} ok.");
        // The first one (inside code) is preserved as literal code text.
        assert!(out.contains("<code>:status[DONE]</code>"), "got: {out}");
        // The second one (outside) becomes a real status macro.
        assert!(
            out.contains(r#"ac:name="status""#),
            "outside-span directive should still be emitted: {out}"
        );
    }

    // ---- render_unknown_block escapes XML metacharacters ------------------

    #[test]
    fn render_unknown_block_escapes_name_params_and_body() {
        // Regression: every piece of `render_unknown_block`'s output must be
        // XML-escaped so a malicious or malformed unknown directive cannot
        // produce non-well-formed storage XHTML.
        let mut params = BTreeMap::new();
        params.insert("k".to_string(), r#"a&b"#.to_string());
        let out = render_unknown_block("name<x>", &params, "body & <tag>");

        assert!(
            out.contains("&lt;x&gt;"),
            "name's `<` and `>` must be escaped: {out}"
        );
        assert!(
            out.contains("&amp;"),
            "ampersands in body and params must be escaped: {out}"
        );
        // The body's `<tag>` must be escaped, not preserved verbatim.
        assert!(
            !out.contains("<tag>"),
            "raw `<tag>` must not appear in output: {out}"
        );
        assert!(
            out.contains("&lt;tag&gt;"),
            "body `<tag>` must be escaped: {out}"
        );

        // Sanity: the produced fragment should be well-formed XML when wrapped
        // in a single root element. We use `quick_xml` since it's already a
        // dependency for the reverse converter.
        let wrapped = format!("<root>{out}</root>");
        let mut reader = quick_xml::reader::Reader::from_str(&wrapped);
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("unknown-block output is not well-formed XML: {e}\noutput: {out}"),
            }
        }
    }

    // ---- render_link respects external URL --------------------------------

    #[test]
    fn link_directive_with_url_emits_anchor() {
        // Regression: `:link[Docs]{url="…"}` must emit a plain HTML anchor —
        // not a Confluence page link — so the URL isn't silently dropped.
        let out = convert(r#":link[Docs]{url="https://example.com/?a=1&b=2"}"#);
        assert!(
            out.contains(r#"<a href="https://example.com/?a=1&amp;b=2">Docs</a>"#),
            "expected escaped anchor in output: {out}"
        );
        // And it must NOT fall through to the page-link branch.
        assert!(
            !out.contains("<ac:link>"),
            "url-bearing link must not emit <ac:link>: {out}"
        );
    }

    #[test]
    fn link_directive_with_url_only_uses_url_as_label() {
        // When `:link{url=…}` has no body, the URL itself should be the
        // visible label so the link isn't blank.
        let out = convert(r#":link{url="https://example.com/x"}"#);
        assert!(
            out.contains(r#"<a href="https://example.com/x">https://example.com/x</a>"#),
            "expected URL-as-label, got: {out}"
        );
    }

    #[test]
    fn link_directive_without_url_falls_back_to_page_link() {
        // Regression check: when neither `url` nor `pageId` is set, but
        // content is, fall back to the existing `ri:content-title`
        // behaviour. The display text is also emitted as
        // `<ac:plain-text-link-body>` so Confluence renders the user's
        // chosen label rather than guessing from the page title.
        let out = convert(":link[Some Page]");
        assert!(
            out.contains(r#"<ri:page ri:content-title="Some Page"/>"#),
            "expected page-title fallback: {out}"
        );
        assert!(
            out.contains(
                r#"<ac:plain-text-link-body><![CDATA[Some Page]]></ac:plain-text-link-body>"#
            ),
            "expected plain-text link body: {out}"
        );
    }

    // ---- spaceKey + pageId/title combinations -----------------------------

    #[test]
    fn link_with_space_key_and_page_id_emits_all_three_attrs_in_order() {
        // Coverage gap A.1: spaceKey + pageId + bracketed content (no
        // explicit title=). All three `<ri:page>` attributes must be
        // present, AND the attribute order must match what Confluence
        // itself emits: `ri:page-id` → `ri:space-key` → `ri:content-title`.
        // A reordering would still be technically valid XML but would
        // diff noisily against any tool that round-trips Confluence-
        // authored storage XML.
        let out = convert(":link[Page]{spaceKey=DOCS pageId=42}");
        assert!(
            out.contains(
                r#"<ri:page ri:page-id="42" ri:space-key="DOCS" ri:content-title="Page"/>"#
            ),
            "expected page-id, space-key, content-title in that exact order: {out}"
        );
        assert!(
            out.contains(r#"<ac:plain-text-link-body><![CDATA[Page]]></ac:plain-text-link-body>"#),
            "bracketed content must wrap the link body: {out}"
        );
    }

    #[test]
    fn link_with_space_key_page_id_and_explicit_title_uses_title_for_content_title() {
        // Coverage gap A.2: all three params present. Explicit `title=`
        // wins over bracketed content for the `ri:content-title` slot,
        // but the visible link body still wraps the bracketed text.
        let out = convert(r#":link[Page]{spaceKey=DOCS pageId=42 title="Real Title"}"#);
        assert!(
            out.contains(r#"ri:page-id="42""#),
            "ri:page-id must be present: {out}"
        );
        assert!(
            out.contains(r#"ri:space-key="DOCS""#),
            "ri:space-key must be present: {out}"
        );
        assert!(
            out.contains(r#"ri:content-title="Real Title""#),
            "explicit title must win over bracketed content for ri:content-title: {out}"
        );
        assert!(
            out.contains(r#"<ac:plain-text-link-body><![CDATA[Page]]></ac:plain-text-link-body>"#),
            "link body must wrap bracketed content (Page), not the title attr: {out}"
        );
    }

    #[test]
    fn link_with_space_key_and_page_id_empty_brackets_omits_content_title_and_body() {
        // Coverage gap A.3: empty bracketed content with both spaceKey
        // and pageId. The `<ri:page>` element must carry both attrs,
        // but no `ri:content-title` (nothing to put there) and no
        // `<ac:plain-text-link-body>` (nothing to wrap). This guards
        // the existing empty-brackets behaviour against a regression
        // where adding spaceKey support might accidentally inject an
        // empty body.
        let out = convert(":link[]{spaceKey=DOCS pageId=42}");
        assert!(
            out.contains(r#"<ri:page ri:page-id="42" ri:space-key="DOCS"/>"#),
            "expected page-id + space-key only: {out}"
        );
        assert!(
            !out.contains("ri:content-title"),
            "empty brackets must not produce ri:content-title: {out}"
        );
        assert!(
            !out.contains("ac:plain-text-link-body"),
            "empty brackets must not produce a link body: {out}"
        );
    }

    #[test]
    fn link_with_only_space_key_keeps_ri_page_non_empty() {
        // Coverage gap A.4: spaceKey alone (no pageId, no title, empty
        // brackets) must still produce a non-empty `<ri:page>` element
        // — `<ri:page ri:space-key="DOCS"/>`, NOT the bare `<ri:page/>`
        // self-closer fallback. This proves spaceKey alone is enough
        // to keep the page reference meaningful.
        let out = convert(":link[]{spaceKey=DOCS}");
        assert!(
            out.contains(r#"<ri:page ri:space-key="DOCS"/>"#),
            "spaceKey alone must keep <ri:page> non-empty: {out}"
        );
        assert!(
            !out.contains("<ri:page/>"),
            "must not fall through to the empty-page fallback: {out}"
        );
    }

    // ---- title= precedence in more configurations -------------------------

    #[test]
    fn link_with_title_and_space_key_overrides_bracketed_content() {
        // Coverage gap B.5: title= still wins over bracketed content
        // for ri:content-title even when spaceKey is also present (a
        // variant of the Bug 2 regression test, but with a third param
        // in the mix to make sure the title resolution doesn't get
        // perturbed by sibling attributes).
        let out = convert(r#":link[label]{title="Different Title" spaceKey=DOCS}"#);
        assert!(
            out.contains(r#"ri:content-title="Different Title""#),
            "explicit title must win over bracketed `label`: {out}"
        );
        assert!(
            out.contains(r#"ri:space-key="DOCS""#),
            "spaceKey must still be emitted alongside title: {out}"
        );
        assert!(
            out.contains(r#"<ac:plain-text-link-body><![CDATA[label]]></ac:plain-text-link-body>"#),
            "link body must wrap bracketed `label`, not the title attr: {out}"
        );
    }

    #[test]
    fn link_with_explicit_empty_title_falls_back_to_bracketed_content() {
        // Coverage gap B.6: an explicit empty `title=""` must be
        // treated the same as "title absent" — i.e. the bracketed
        // content takes over as `ri:content-title`. Empty strings
        // are filtered out of the attribute lookup so they don't
        // suppress the natural fallback chain or emit an empty
        // `ri:content-title=""` attribute.
        let out = convert(r#":link[label]{title=""}"#);
        assert!(
            out.contains(r#"<ri:page ri:content-title="label"/>"#),
            "empty title= must fall back to bracketed `label`: {out}"
        );
        assert!(
            !out.contains(r#"ri:content-title="""#),
            "empty `ri:content-title` attr must not be emitted: {out}"
        );
        // The visible body must still come from the bracketed content.
        assert!(
            out.contains(r#"<ac:plain-text-link-body><![CDATA[label]]></ac:plain-text-link-body>"#),
            "link body must always wrap bracketed `label`: {out}"
        );
    }

    #[test]
    fn link_with_explicit_empty_space_key_falls_back_to_bracketed_content() {
        // Symmetric to the `title=""` fix: an explicit empty
        // `spaceKey=""` must be treated as "spaceKey absent" so we
        // don't emit `ri:space-key=""`. With no pageId and no real
        // spaceKey, the bracketed content provides the title.
        let out = convert(r#":link[label]{spaceKey=""}"#);
        assert!(
            out.contains(r#"<ri:page ri:content-title="label"/>"#),
            "empty spaceKey= must not suppress bracketed-content fallback: {out}"
        );
        assert!(
            !out.contains(r#"ri:space-key="""#),
            "empty `ri:space-key` attr must not be emitted: {out}"
        );
    }

    #[test]
    fn link_with_explicit_empty_page_id_falls_back_to_bracketed_content() {
        // Symmetric to the `title=""` fix: an explicit empty
        // `pageId=""` must be treated as "pageId absent" so we
        // don't emit `ri:page-id=""`. With no real pageId and no
        // spaceKey, the bracketed content provides the title.
        let out = convert(r#":link[label]{pageId=""}"#);
        assert!(
            out.contains(r#"<ri:page ri:content-title="label"/>"#),
            "empty pageId= must not suppress bracketed-content fallback: {out}"
        );
        assert!(
            !out.contains(r#"ri:page-id="""#),
            "empty `ri:page-id` attr must not be emitted: {out}"
        );
    }

    // ---- XML escaping with the new attributes -----------------------------

    #[test]
    fn link_space_key_with_ampersand_is_xml_escaped() {
        // Coverage gap C.7: `spaceKey` values flow through `xml_escape`
        // in the new code. A space key like `A&B` must appear as
        // `ri:space-key="A&amp;B"`, not as raw `&` (which would break
        // XML well-formedness and could be an XSS vector against any
        // downstream renderer that trusts the storage XML).
        let out = convert(r#":link[t]{spaceKey="A&B"}"#);
        assert!(
            out.contains(r#"ri:space-key="A&amp;B""#),
            "ampersand in spaceKey must be escaped to &amp;: {out}"
        );
        assert!(
            !out.contains(r#"ri:space-key="A&B""#),
            "raw `&` must not appear in attribute value: {out}"
        );
    }

    #[test]
    fn link_title_with_double_quote_is_xml_escaped() {
        // Coverage gap C.8: the explicit-title path also flows through
        // `xml_escape` — a `"` inside `title=` must become `&quot;`
        // so the attribute value's enclosing quotes don't terminate
        // early.
        let out = convert(r#":link[t]{title="\"quoted\""}"#);
        assert!(
            out.contains(r#"ri:content-title="&quot;quoted&quot;""#),
            "double quotes in title= must be escaped to &quot;: {out}"
        );
    }

    // ---- url= branch is undisturbed ---------------------------------------

    #[test]
    fn link_with_url_wins_over_space_key_and_title() {
        // Coverage gap D.9: the external-URL early-return must take
        // priority over every Confluence page reference (spaceKey,
        // pageId, title) — even when all of them are present. The
        // fix did not touch this branch, but a future refactor that
        // moved the URL check below the param resolution would
        // silently regress: the user would get a broken `<ac:link>`
        // pointing at a page that may not exist instead of the
        // intended external anchor.
        let out =
            convert(r#":link[Docs]{url="https://example.com" spaceKey=IGNORED title="ignored"}"#);
        assert!(
            out.contains(r#"<a href="https://example.com">Docs</a>"#),
            "url= must produce a plain anchor: {out}"
        );
        assert!(
            !out.contains("<ri:page"),
            "url= must NOT fall through to <ri:page>: {out}"
        );
        assert!(
            !out.contains("<ac:link>"),
            "url= must NOT fall through to <ac:link>: {out}"
        );
    }

    // ---- stripped HTML tag preservation -----------------------------------
    //
    // Atlassian's storage sanitiser drops `<kbd>`, `<samp>`, and `<var>`
    // server-side. We rewrite each as `<code>` so the content survives the
    // round-trip — the user's keystroke / sample-output / variable text is
    // more useful as monospace than vanishing into plain prose.

    #[test]
    fn kbd_inline_html_becomes_code_for_confluence_storage() {
        // Plain `<kbd>Ctrl</kbd>` must reach the API as `<code>Ctrl</code>`
        // — never as the literal `<kbd>` tag (which Confluence drops) and
        // never as plain text `Ctrl` (which loses the visual distinction).
        let out = convert("<kbd>Ctrl</kbd>");
        assert!(
            out.contains("<code>Ctrl</code>"),
            "kbd must be rewritten as code, got: {out}"
        );
        assert!(
            !out.contains("<kbd>"),
            "kbd opening tag must not survive, got: {out}"
        );
        assert!(
            !out.contains("</kbd>"),
            "kbd closing tag must not survive, got: {out}"
        );
    }

    #[test]
    fn samp_inline_html_becomes_code() {
        // `<samp>` is the sample-program-output tag — same treatment as
        // `<kbd>` since Confluence storage strips it the same way.
        let out = convert("<samp>output</samp>");
        assert!(
            out.contains("<code>output</code>"),
            "samp must be rewritten as code, got: {out}"
        );
        assert!(
            !out.contains("<samp>") && !out.contains("</samp>"),
            "samp tag must not survive, got: {out}"
        );
    }

    #[test]
    fn var_inline_html_becomes_code() {
        // `<var>` is the variable / placeholder tag — third member of the
        // keyboard / sample / variable trio that Confluence drops.
        let out = convert("<var>x</var>");
        assert!(
            out.contains("<code>x</code>"),
            "var must be rewritten as code, got: {out}"
        );
        assert!(
            !out.contains("<var>") && !out.contains("</var>"),
            "var tag must not survive, got: {out}"
        );
    }

    #[test]
    fn nested_kbd_with_other_content_preserves_surrounding() {
        // The replacement is per-tag and must leave the surrounding markup
        // alone — both `<kbd>` runs reach the API as `<code>` while the
        // intervening literal text and the wrapping paragraph are kept as
        // they would be without any rewrite.
        let out = convert("Press <kbd>Ctrl</kbd> + <kbd>C</kbd> to copy");
        assert!(
            out.contains("Press <code>Ctrl</code> + <code>C</code> to copy"),
            "back-to-back kbd runs must each become code with surrounding text intact, got: {out}"
        );
        assert!(
            out.contains("<p>"),
            "wrapping paragraph must still be emitted, got: {out}"
        );
        assert!(
            !out.contains("<kbd>"),
            "no kbd tags should remain, got: {out}"
        );
    }
}
