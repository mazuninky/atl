//! Jira wiki text → markdown (with MyST-style directive extensions).
//!
//! Jira wiki is the legacy line-oriented markup used by Jira Server / DC and
//! still accepted by Jira Cloud's REST API. This converter is the inverse of
//! [`crate::cli::commands::converters::md_to_wiki`]: anything `md_to_wiki` can
//! emit should round-trip back through this converter to a markdown form that
//! preserves the meaning, even if not byte-for-byte identical.
//!
//! # Conversion strategy
//!
//! A pragmatic two-stage state machine — Jira wiki is messy and edge-case
//! laden, so the converter aims for correctness on the productive subset
//! rather than full grammar coverage.
//!
//! 1. **Block walker.** Walk the input line by line, tracking a small state
//!    enum (`Normal`, `InCodeBlock`, `InNoformat`, `InQuote`, `InMacro`).
//!    Open / close tokens (`{code}`, `{quote}`, `{info}`, …) flip state. In
//!    `Normal` state the walker recognises headings, lists, tables, the
//!    horizontal rule, single-line `bq.`, and self-closing `{toc}` macros.
//!    Block macros nest via a stack so `{info}` inside `{warning}` works.
//!
//! 2. **Inline rendering.** For each accumulated paragraph / list item /
//!    table cell / blockquote line, run a character-by-character scanner
//!    that recognises bold (`*x*`), italic (`_x_`), strikethrough (`-x-`
//!    with word boundaries), underline (`+x+`), inline code (`{{x}}`),
//!    links (`[text|url]` and variants), images (`!url!`), mentions
//!    (`[~user]`), the inline status macro, and the canonical emoticon set.
//!    Backslash escapes `\X` produce literal `X`.
//!
//! # Lossy fallbacks
//!
//! - **Citation** `??text??` — emitted as `<cite>text</cite>` HTML.
//! - **Subscript / superscript** `~x~` / `^x^` — emitted as `<sub>` / `<sup>`
//!   HTML so the round-trip preserves the visual intent.
//! - **Underline** `+x+` — emitted as `<u>x</u>` HTML; markdown has no native
//!   underline syntax.
//! - **Unknown emoticons** — `(blah)` is left as literal parenthesised text.
//!   Only the canonical set (`(!)`, `(?)`, `(/)`, `(x)`, `(i)`, `(*)`,
//!   `(y)`, `(n)`, `(on)`, `(off)`) is recognised.
//! - **Unclosed macro / code block** — the body up to EOF is emitted verbatim.
//!   No panic, no error: the caller still gets useful output.
//! - **Unknown block macros** — pass through as a fenced directive so the
//!   user sees the original intent, but no semantic conversion happens.
//!
//! # `render_directives = false`
//!
//! With [`ConvertOpts::render_directives`] set to `false`, block macros
//! (`info` / `warning` / `note` / `tip`) are flattened to their body
//! content; status macros become plain text; emoticons and `{toc}` are
//! dropped; mentions collapse to their display name. This is useful for
//! "clean text" extraction modes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use thiserror::Error;

use super::code_fence::pick_code_fence;
use crate::cli::commands::directives::render_attrs;

// =====================================================================
// Public API
// =====================================================================

/// Errors returned by [`wiki_to_markdown`].
#[derive(Debug, Error)]
pub enum WikiToMdError {
    /// The input contained a structural error that the converter could not
    /// recover from (currently unused — every path degrades gracefully).
    #[error("malformed wiki: {0}")]
    Malformed(String),
}

/// Conversion options for [`wiki_to_markdown`].
///
/// `render_directives` controls whether Jira wiki macros become MyST-style
/// directives (`true`, the default) or are stripped to their plain body text
/// (`false`).
#[derive(Debug, Clone, Copy)]
pub struct ConvertOpts {
    /// When `true` (the default), recognised block macros are converted to
    /// `:::name` directive fences and inline macros (`{status}`, emoticons,
    /// mentions) become `:status[…]` / `:emoticon{…}` / `:mention[…]`. When
    /// `false`, block macros emit only their body content, status macros
    /// emit only their title, emoticons / `{toc}` are dropped, and mentions
    /// collapse to their display name.
    pub render_directives: bool,
}

impl Default for ConvertOpts {
    fn default() -> Self {
        Self {
            render_directives: true,
        }
    }
}

/// Convert Jira wiki syntax to markdown with MyST-style directives.
///
/// Block macros (`{info}…{info}`) become fenced directives (`:::info`).
/// Inline constructs (status macro, emoticons, mentions) become inline
/// directives or markdown equivalents. Unknown macros pass through as fenced
/// directive blocks so the round-trip is non-destructive.
///
/// Returns an error only on unrecoverable structural failures; today every
/// productive code path degrades gracefully and never errors.
///
/// # Examples
///
/// ```ignore
/// use atl::cli::commands::converters::wiki_to_md::{wiki_to_markdown, ConvertOpts};
///
/// let md = wiki_to_markdown("h1. Title\n\nbody", ConvertOpts::default()).unwrap();
/// assert!(md.contains("# Title"));
/// assert!(md.contains("body"));
/// ```
pub fn wiki_to_markdown(wiki: &str, opts: ConvertOpts) -> Result<String, WikiToMdError> {
    let blocks = parse_blocks(wiki);
    let mut out = String::new();
    render_blocks(&blocks, &mut out, &opts);
    Ok(normalize_blank_lines(&out))
}

// =====================================================================
// Stage 1: block-level parsing
// =====================================================================

/// One parsed block-level element.
#[derive(Debug, Clone)]
enum Block {
    /// Paragraph: a contiguous run of text lines, joined with markdown soft
    /// breaks (`\n`). Hard breaks (`\\` at end of line) become `  \n`.
    Paragraph(Vec<String>),
    /// Heading `h1.`–`h6.`.
    Heading { level: u8, text: String },
    /// `----` thematic break.
    HorizontalRule,
    /// `bq.` single-line blockquote or `{quote}…{quote}` multi-line.
    Quote(Vec<String>),
    /// `{code}` / `{noformat}` block. `lang` is `None` for noformat or
    /// language-less code blocks.
    Code { lang: Option<String>, body: String },
    /// `*` / `#` list lines, contiguous run.
    List(Vec<ListItem>),
    /// `||` / `|` table.
    Table(Vec<TableRow>),
    /// Self-closing block macro (`{toc}` / `{toc:maxLevel=3}`).
    SelfClosingMacro {
        name: String,
        params: BTreeMap<String, String>,
    },
    /// Paired block macro (`{info}…{info}`, etc.) with a body of nested
    /// blocks.
    Macro {
        name: String,
        params: BTreeMap<String, String>,
        body: Vec<Block>,
    },
}

#[derive(Debug, Clone)]
struct ListItem {
    /// `b` for bullet, `n` for numbered.
    kind: char,
    /// Number of marker characters before the space (1-based).
    depth: usize,
    /// Inline content of the item.
    text: String,
}

#[derive(Debug, Clone)]
struct TableRow {
    is_header: bool,
    cells: Vec<String>,
}

/// Parse a chunk of wiki text into a flat sequence of [`Block`]s.
///
/// Block macros (info / warning / note / tip / quote / code / noformat) are
/// gathered into [`Block::Macro`] / [`Block::Quote`] / [`Block::Code`] frames
/// with their bodies recursively re-parsed.
fn parse_blocks(wiki: &str) -> Vec<Block> {
    let lines: Vec<&str> = wiki.split('\n').collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        i = consume_one_block(&lines, i, &mut blocks);
    }
    blocks
}

/// Consume one or more lines starting at `i`, push zero or more blocks to
/// `out`, and return the new position.
fn consume_one_block(lines: &[&str], i: usize, out: &mut Vec<Block>) -> usize {
    let line = lines[i];

    // Blank line → paragraph separator (no block emitted).
    if line.trim().is_empty() {
        return i + 1;
    }

    // Inline single-line `{code[:lang]}content{code}` form (Cloud sometimes
    // emits this when downgrading ADF code blocks to wiki). Must be checked
    // before `parse_code_open` because the inline form has trailing content
    // on the same line as the opening token.
    if let Some((lang, body)) = parse_inline_code_block(line) {
        out.push(Block::Code { lang, body });
        return i + 1;
    }
    // {code} / {code:lang}
    if let Some(lang) = parse_code_open(line) {
        return consume_code_block(lines, i + 1, lang, out);
    }
    // {noformat}
    if line.trim() == "{noformat}" {
        return consume_noformat_block(lines, i + 1, out);
    }
    // {quote}
    if line.trim() == "{quote}" {
        return consume_quote_block(lines, i + 1, out);
    }
    // {name} / {name:params} for paired block macros (info/warning/note/tip/panel)
    if let Some((name, params, _)) = parse_macro_open(line) {
        if matches!(name.as_str(), "info" | "warning" | "note" | "tip" | "panel") {
            return consume_macro_block(lines, i + 1, name, params, out);
        }
        if name == "toc" {
            out.push(Block::SelfClosingMacro { name, params });
            return i + 1;
        }
    }
    // Heading
    if let Some((level, text)) = parse_heading(line) {
        out.push(Block::Heading { level, text });
        return i + 1;
    }
    // Horizontal rule
    if is_horizontal_rule(line) {
        out.push(Block::HorizontalRule);
        return i + 1;
    }
    // bq. single-line blockquote
    if let Some(rest) = line.strip_prefix("bq. ") {
        out.push(Block::Quote(vec![rest.to_string()]));
        return i + 1;
    }
    // Table
    if is_table_line(line) {
        return consume_table(lines, i, out);
    }
    // List
    if parse_list_marker(line).is_some() {
        return consume_list(lines, i, out);
    }
    // Paragraph (default)
    consume_paragraph(lines, i, out)
}

fn parse_heading(line: &str) -> Option<(u8, String)> {
    if line.len() < 4 {
        return None;
    }
    let bytes = line.as_bytes();
    if bytes[0] != b'h' {
        return None;
    }
    let digit = bytes[1];
    if !(b'1'..=b'6').contains(&digit) {
        return None;
    }
    if bytes[2] != b'.' || bytes[3] != b' ' {
        return None;
    }
    let level = digit - b'0';
    let text = line[4..].to_string();
    Some((level, text))
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 4 && trimmed.bytes().all(|b| b == b'-')
}

fn parse_code_open(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim();
    if trimmed == "{code}" {
        return Some(None);
    }
    if let Some(rest) = trimmed.strip_prefix("{code:")
        && let Some(end) = rest.strip_suffix('}')
    {
        let lang = end.split('|').next().unwrap_or("").trim().to_string();
        if lang.is_empty() {
            return Some(None);
        }
        return Some(Some(lang));
    }
    None
}

/// Detect a one-line `{code[:lang]}body{code}` macro on the same line.
///
/// Cloud occasionally emits this single-line form when downgrading an ADF
/// `codeBlock` to wiki text — instead of the conventional multi-line block:
///
/// ```text
/// {code:python}
/// print('x')
/// {code}
/// ```
///
/// it produces `{code:python}print('x'){code}` on one line. Returns the
/// language (`None` when absent or `{code}body{code}`) and the trimmed
/// body. Multi-line forms (where the close `{code}` is on a different line,
/// or where the open token has trailing content but no close) return `None`
/// so the multi-line walker handles them.
fn parse_inline_code_block(line: &str) -> Option<(Option<String>, String)> {
    let trimmed = line.trim();

    // Find the open token boundary: either `{code}` immediately or
    // `{code:...}` where `...` does not contain `}`.
    let (lang, after_open_idx) = if let Some(rest) = trimmed.strip_prefix("{code}") {
        (None, trimmed.len() - rest.len())
    } else if let Some(rest) = trimmed.strip_prefix("{code:") {
        let close = rest.find('}')?;
        let raw_lang = &rest[..close];
        let lang_str = raw_lang.split('|').next().unwrap_or("").trim().to_string();
        let lang = if lang_str.is_empty() {
            None
        } else {
            Some(lang_str)
        };
        // `{code:` is 6 bytes; +close+1 for the `}`.
        (lang, 6 + close + 1)
    } else {
        return None;
    };

    let after_open = &trimmed[after_open_idx..];

    // The body must end at the first `{code}` and the rest of the line must
    // be empty / whitespace. If `{code}` is not on this line, we're in the
    // multi-line case — return None and let `consume_code_block` handle it.
    let close_idx = after_open.find("{code}")?;
    let body = &after_open[..close_idx];
    let after_close = &after_open[close_idx + "{code}".len()..];
    if !after_close.trim().is_empty() {
        return None;
    }

    // Reject the empty-body form `{code}{code}` — there's nothing to render
    // and the multi-line walker handles `{code}\n{code}` more usefully.
    if body.is_empty() {
        return None;
    }

    Some((lang, body.trim().to_string()))
}

/// Map a Jira Cloud panel hex `bgColor` to the closest MyST directive name.
///
/// When Cloud Server-side downgrades an ADF `panel` node to wiki text it
/// emits `{panel:bgColor=#XXXXXX}…{panel}` and loses the original
/// `panelType`. We recover the intent by mapping the hex back to the
/// directive name. Hex codes are case-insensitive — the caller does not
/// need to lowercase before passing in.
///
/// Returns `None` for unknown colors so the caller can fall back to the
/// raw `:::panel{bgColor="#…"}` form, which preserves the round-trip.
fn panel_bgcolor_to_directive(hex: &str) -> Option<&'static str> {
    match hex.to_ascii_lowercase().as_str() {
        "#deebff" => Some("info"),
        "#fffae6" | "#ffebe6" => Some("warning"),
        "#e3fcef" => Some("tip"),
        "#eae6ff" => Some("note"),
        _ => None,
    }
}

/// Parse a paired-macro open line `{name}` or `{name:k=v|k=v}`. Returns the
/// macro name, parsed parameters, and the unparsed parameter string for
/// debugging. Returns `None` if the line is not a recognised macro open.
fn parse_macro_open(line: &str) -> Option<(String, BTreeMap<String, String>, String)> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?;
    if inner.is_empty() {
        return None;
    }
    // Split on first ':' (params follow).
    if let Some((name, params_str)) = inner.split_once(':') {
        if !is_valid_macro_name(name) {
            return None;
        }
        let params = parse_pipe_params(params_str);
        return Some((name.to_string(), params, params_str.to_string()));
    }
    if !is_valid_macro_name(inner) {
        return None;
    }
    Some((inner.to_string(), BTreeMap::new(), String::new()))
}

fn is_valid_macro_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Parse pipe-separated `key=value` pairs (e.g. `title=Heads up|key=val`).
/// Splits on unescaped `|` only — `\|` inside a value is preserved as a
/// literal pipe. The shared [`split_unescaped_pipe`] splitter also decodes
/// `\\` → `\` and `\}` → `}` while walking, so values come back already
/// unescaped.
fn parse_pipe_params(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for piece in split_unescaped_pipe(s) {
        if let Some((k, v)) = piece.split_once('=') {
            let key = k.trim();
            if !key.is_empty() && is_valid_macro_name(key) {
                out.insert(key.to_string(), v.to_string());
            }
        }
    }
    out
}

fn consume_code_block(
    lines: &[&str],
    start: usize,
    lang: Option<String>,
    out: &mut Vec<Block>,
) -> usize {
    let mut body = String::new();
    let mut i = start;
    while i < lines.len() {
        if lines[i].trim() == "{code}" {
            // Trim a trailing newline if body is non-empty.
            if body.ends_with('\n') {
                body.pop();
            }
            out.push(Block::Code { lang, body });
            return i + 1;
        }
        body.push_str(lines[i]);
        body.push('\n');
        i += 1;
    }
    // Unclosed — emit body as-is, no panic.
    if body.ends_with('\n') {
        body.pop();
    }
    out.push(Block::Code { lang, body });
    i
}

fn consume_noformat_block(lines: &[&str], start: usize, out: &mut Vec<Block>) -> usize {
    let mut body = String::new();
    let mut i = start;
    while i < lines.len() {
        if lines[i].trim() == "{noformat}" {
            if body.ends_with('\n') {
                body.pop();
            }
            out.push(Block::Code { lang: None, body });
            return i + 1;
        }
        body.push_str(lines[i]);
        body.push('\n');
        i += 1;
    }
    if body.ends_with('\n') {
        body.pop();
    }
    out.push(Block::Code { lang: None, body });
    i
}

fn consume_quote_block(lines: &[&str], start: usize, out: &mut Vec<Block>) -> usize {
    let mut body = Vec::new();
    let mut i = start;
    while i < lines.len() {
        if lines[i].trim() == "{quote}" {
            out.push(Block::Quote(body));
            return i + 1;
        }
        body.push(lines[i].to_string());
        i += 1;
    }
    out.push(Block::Quote(body));
    i
}

fn consume_macro_block(
    lines: &[&str],
    start: usize,
    name: String,
    params: BTreeMap<String, String>,
    out: &mut Vec<Block>,
) -> usize {
    // Collect lines until the matching `{name}` close, accounting for nested
    // macros of the same name by tracking depth.
    let mut depth: usize = 1;
    let mut body_lines = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        // Same-name close (no params).
        if trimmed == format!("{{{name}}}") {
            depth -= 1;
            if depth == 0 {
                let body = parse_blocks(&body_lines.join("\n"));
                out.push(Block::Macro { name, params, body });
                return i + 1;
            }
            body_lines.push(line.to_string());
            i += 1;
            continue;
        }
        // Nested same-name open (with or without params).
        if let Some((open_name, _, _)) = parse_macro_open(line)
            && open_name == name
        {
            depth += 1;
        }
        body_lines.push(line.to_string());
        i += 1;
    }
    // Unclosed macro → emit body anyway, gracefully.
    let body = parse_blocks(&body_lines.join("\n"));
    out.push(Block::Macro { name, params, body });
    i
}

fn is_table_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("||") || trimmed.starts_with('|')
}

fn consume_table(lines: &[&str], start: usize, out: &mut Vec<Block>) -> usize {
    let mut rows = Vec::new();
    let mut i = start;
    while i < lines.len() && is_table_line(lines[i]) {
        rows.push(parse_table_row(lines[i]));
        i += 1;
    }
    out.push(Block::Table(rows));
    i
}

fn parse_table_row(line: &str) -> TableRow {
    let trimmed = line.trim();
    let is_header = trimmed.starts_with("||");
    let sep = if is_header { "||" } else { "|" };

    // Split using the chosen separator. Strip leading/trailing separators
    // first so we don't get empty leading/trailing cells.
    let stripped = trimmed
        .strip_prefix(sep)
        .unwrap_or(trimmed)
        .strip_suffix(sep)
        .unwrap_or_else(|| trimmed.strip_prefix(sep).unwrap_or(trimmed));

    let cells: Vec<String> = if sep == "||" {
        stripped.split("||").map(|s| s.to_string()).collect()
    } else {
        // Split on `|` but respect `\|` escapes.
        split_unescaped_pipe(stripped)
    };
    TableRow { is_header, cells }
}

fn split_unescaped_pipe(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
        {
            buf.push(next);
            chars.next();
            continue;
        }
        if c == '|' {
            out.push(buf.clone());
            buf.clear();
            continue;
        }
        buf.push(c);
    }
    out.push(buf);
    out
}

fn parse_list_marker(line: &str) -> Option<(char, usize)> {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let first = bytes[0];
    let kind = match first {
        b'*' => 'b',
        b'#' => 'n',
        _ => return None,
    };
    let mut depth = 0usize;
    while depth < bytes.len() && bytes[depth] == first {
        depth += 1;
    }
    if depth == bytes.len() || bytes[depth] != b' ' {
        return None;
    }
    Some((kind, depth))
}

fn consume_list(lines: &[&str], start: usize, out: &mut Vec<Block>) -> usize {
    let mut items = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let Some((kind, depth)) = parse_list_marker(lines[i]) else {
            break;
        };
        // Skip the marker prefix (depth chars + 1 space).
        let text = &lines[i][depth + 1..];
        items.push(ListItem {
            kind,
            depth,
            text: text.to_string(),
        });
        i += 1;
    }
    out.push(Block::List(items));
    i
}

fn consume_paragraph(lines: &[&str], start: usize, out: &mut Vec<Block>) -> usize {
    let mut buf = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            break;
        }
        // Stop if the next line begins another construct.
        if i > start
            && (parse_heading(line).is_some()
                || is_horizontal_rule(line)
                || parse_code_open(line).is_some()
                || parse_inline_code_block(line).is_some()
                || line.trim() == "{noformat}"
                || line.trim() == "{quote}"
                || is_table_line(line)
                || parse_list_marker(line).is_some()
                || line.starts_with("bq. ")
                || is_block_macro_open(line))
        {
            break;
        }
        buf.push(line.to_string());
        i += 1;
    }
    if !buf.is_empty() {
        out.push(Block::Paragraph(buf));
    }
    i
}

fn is_block_macro_open(line: &str) -> bool {
    if let Some((name, _, _)) = parse_macro_open(line) {
        matches!(
            name.as_str(),
            "info" | "warning" | "note" | "tip" | "panel" | "toc"
        )
    } else {
        false
    }
}

// =====================================================================
// Stage 2: rendering
// =====================================================================

fn render_blocks(blocks: &[Block], out: &mut String, opts: &ConvertOpts) {
    let mut first = true;
    for block in blocks {
        if !first {
            ensure_blank_line(out);
        }
        first = false;
        render_block(block, out, opts);
    }
}

fn render_block(block: &Block, out: &mut String, opts: &ConvertOpts) {
    match block {
        Block::Paragraph(lines) => {
            let body = render_paragraph_lines(lines, opts);
            push_paragraph(out, &body);
        }
        Block::Heading { level, text } => {
            ensure_blank_line(out);
            for _ in 0..*level {
                out.push('#');
            }
            out.push(' ');
            out.push_str(&render_inline(text, opts));
            out.push('\n');
        }
        Block::HorizontalRule => {
            ensure_blank_line(out);
            out.push_str("---\n");
        }
        Block::Quote(lines) => {
            ensure_blank_line(out);
            for line in lines {
                out.push_str("> ");
                out.push_str(&render_inline(line, opts));
                out.push('\n');
            }
        }
        Block::Code { lang, body } => {
            ensure_blank_line(out);
            // Pick a fence at least one backtick longer than any run inside
            // the body so inner ``` strings cannot prematurely close the
            // block (CommonMark §4.5).
            let fence = pick_code_fence(body);
            out.push_str(&fence);
            if let Some(l) = lang {
                out.push_str(l);
            }
            out.push('\n');
            out.push_str(body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&fence);
            out.push('\n');
        }
        Block::List(items) => {
            ensure_blank_line(out);
            render_list(items, out, opts);
        }
        Block::Table(rows) => {
            ensure_blank_line(out);
            render_table(rows, out, opts);
        }
        Block::SelfClosingMacro { name, params } => {
            render_self_closing_macro(name, params, out, opts);
        }
        Block::Macro { name, params, body } => {
            render_macro_block(name, params, body, out, opts);
        }
    }
}

fn render_paragraph_lines(lines: &[String], opts: &ConvertOpts) -> String {
    let mut out = String::new();
    for (i, raw) in lines.iter().enumerate() {
        // Hard break: line ends with `\\`.
        let (clean, hard) = if let Some(stripped) = raw.strip_suffix("\\\\") {
            (stripped, true)
        } else {
            (raw.as_str(), false)
        };
        let rendered = render_inline(clean, opts);
        out.push_str(&rendered);
        if i + 1 < lines.len() {
            if hard {
                out.push_str("  \n");
            } else {
                out.push('\n');
            }
        }
    }
    out
}

fn push_paragraph(out: &mut String, body: &str) {
    let trimmed = body.trim_end();
    if trimmed.is_empty() {
        return;
    }
    ensure_blank_line(out);
    out.push_str(trimmed);
    out.push('\n');
}

fn render_list(items: &[ListItem], out: &mut String, opts: &ConvertOpts) {
    for item in items {
        let indent_units = item.depth.saturating_sub(1);
        let indent = "  ".repeat(indent_units);
        out.push_str(&indent);
        if item.kind == 'b' {
            out.push_str("- ");
        } else {
            out.push_str("1. ");
        }
        out.push_str(&render_inline(&item.text, opts));
        out.push('\n');
    }
}

fn render_table(rows: &[TableRow], out: &mut String, opts: &ConvertOpts) {
    if rows.is_empty() {
        return;
    }
    let cols = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }

    // GFM requires a header row. If first row is a header, use it; otherwise
    // synthesize an empty header from the column count of the first row.
    let (header_idx, header_cells) = if rows[0].is_header {
        (Some(0), &rows[0].cells)
    } else {
        (None, &rows[0].cells)
    };

    out.push('|');
    if header_idx.is_some() {
        for cell in header_cells {
            out.push(' ');
            out.push_str(&render_inline(cell.trim(), opts));
            out.push_str(" |");
        }
        // Pad short header rows.
        for _ in header_cells.len()..cols {
            out.push_str("  |");
        }
    } else {
        for _ in 0..cols {
            out.push_str("  |");
        }
    }
    out.push('\n');

    // Separator
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    out.push('\n');

    // Data rows
    let data_start = if header_idx.is_some() { 1 } else { 0 };
    for row in &rows[data_start..] {
        out.push('|');
        for cell in &row.cells {
            out.push(' ');
            out.push_str(&render_inline(cell.trim(), opts));
            out.push_str(" |");
        }
        for _ in row.cells.len()..cols {
            out.push_str("  |");
        }
        out.push('\n');
    }
}

fn render_self_closing_macro(
    name: &str,
    params: &BTreeMap<String, String>,
    out: &mut String,
    opts: &ConvertOpts,
) {
    if !opts.render_directives {
        // Drop self-closing macros entirely in stripped mode.
        return;
    }
    ensure_blank_line(out);
    out.push_str(":::");
    out.push_str(name);
    if !params.is_empty() {
        out.push(' ');
        out.push_str(&render_attrs(params));
    }
    out.push('\n');
    out.push_str(":::\n");
}

fn render_macro_block(
    name: &str,
    params: &BTreeMap<String, String>,
    body: &[Block],
    out: &mut String,
    opts: &ConvertOpts,
) {
    if !opts.render_directives {
        // Strip wrapper — emit body inline.
        let mut inner = String::new();
        render_blocks(body, &mut inner, opts);
        ensure_blank_line(out);
        out.push_str(inner.trim_end_matches('\n'));
        out.push('\n');
        return;
    }

    // Cloud-after-ADF panel recovery: when we see a `panel` macro with a
    // recognised `bgColor`, map it to the closest directive name and drop
    // the now-redundant attribute. Unknown bgColors / no-attr panels fall
    // through to the verbatim `:::panel{bgColor="…"}` form so a downstream
    // md→storage/adf converter can still round-trip the raw color.
    let (rendered_name, rendered_params): (&str, BTreeMap<String, String>) = if name == "panel" {
        match params.get("bgColor").and_then(|h| {
            // Only consider `bgColor` values that look like a hex code; an
            // unrelated string value should not be mapped.
            if h.starts_with('#') {
                panel_bgcolor_to_directive(h)
            } else {
                None
            }
        }) {
            Some(directive) => {
                let mut filtered = params.clone();
                filtered.remove("bgColor");
                (directive, filtered)
            }
            None => ("panel", params.clone()),
        }
    } else {
        (name, params.clone())
    };

    let mut inner = String::new();
    render_blocks(body, &mut inner, opts);
    ensure_blank_line(out);
    out.push_str(":::");
    out.push_str(rendered_name);
    if !rendered_params.is_empty() {
        out.push(' ');
        out.push_str(&render_attrs(&rendered_params));
    }
    out.push('\n');
    out.push_str(inner.trim_end_matches('\n'));
    if !inner.is_empty() {
        out.push('\n');
    }
    out.push_str(":::\n");
}

// =====================================================================
// Inline rendering
// =====================================================================

/// Render a single line of wiki inline content into markdown, handling all
/// recognised inline tokens.
fn render_inline(text: &str, opts: &ConvertOpts) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];

        // Backslash escape.
        if b == b'\\' && i + 1 < bytes.len() {
            // `\\` at end of line is a hard break, but inside a paragraph the
            // hard break is handled by the paragraph renderer; here it's a
            // literal escape.
            let next = bytes[i + 1];
            // Emit the next byte literally (escaped where markdown would
            // interpret it).
            push_literal_byte(&mut out, next);
            i += 2;
            continue;
        }

        // Inline code {{...}}
        if b == b'{'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'{'
            && let Some(end) = find_inline_code_end(text, i + 2)
        {
            let body = &text[i + 2..end];
            out.push_str(&wrap_in_code_span(body));
            i = end + 2;
            continue;
        }

        // Cloud-after-ADF status heuristic: `{color:#HHHHHH}*[ TEXT ]*{color}`
        // — Cloud's ADF→wiki downgrade emits this for an ADF status node.
        // Match the whole pattern before falling through to the regular
        // `{color:…}` color macro (which we don't otherwise translate).
        if b == b'{'
            && let Some((rendered, consumed)) = try_parse_color_status_heuristic(&text[i..], opts)
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }

        // Status macro {status:colour=X|title=Y}
        if b == b'{'
            && let Some((rendered, consumed)) = try_parse_inline_macro(&text[i..], opts)
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }

        // Image !url! or !url|alt=...!
        if b == b'!'
            && let Some((rendered, consumed)) = try_parse_image(&text[i..])
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }

        // Link [text|url] / [text|url|tip] / [url] / [~user] / [~accountid:id]
        if b == b'['
            && let Some((rendered, consumed)) = try_parse_link(&text[i..], opts)
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }

        // Bold *...*
        if b == b'*'
            && let Some((rendered, consumed)) = try_parse_paired(text, i, b'*', "**", "**", opts)
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }
        // Italic _..._
        if b == b'_'
            && let Some((rendered, consumed)) = try_parse_paired(text, i, b'_', "*", "*", opts)
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }
        // Underline +...+
        if b == b'+'
            && let Some((rendered, consumed)) = try_parse_paired(text, i, b'+', "<u>", "</u>", opts)
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }
        // Strikethrough -...- (with word boundaries)
        if b == b'-'
            && is_strike_start(text, i)
            && let Some((rendered, consumed)) =
                try_parse_paired_with_boundary(text, i, b'-', "~~", "~~", opts)
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }
        // Citation ??...??
        if b == b'?'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'?'
            && let Some(end) = find_double_marker(text, i + 2, '?')
        {
            let inner = &text[i + 2..end];
            out.push_str("<cite>");
            out.push_str(&render_inline(inner, opts));
            out.push_str("</cite>");
            i = end + 2;
            continue;
        }
        // Subscript ~...~
        if b == b'~'
            && let Some(end) = find_single_marker(text, i + 1, '~')
        {
            let inner = &text[i + 1..end];
            out.push_str("<sub>");
            out.push_str(&render_inline(inner, opts));
            out.push_str("</sub>");
            i = end + 1;
            continue;
        }
        // Superscript ^...^
        if b == b'^'
            && let Some(end) = find_single_marker(text, i + 1, '^')
        {
            let inner = &text[i + 1..end];
            out.push_str("<sup>");
            out.push_str(&render_inline(inner, opts));
            out.push_str("</sup>");
            i = end + 1;
            continue;
        }
        // Emoticon (!), (?), (/), (x), (i), (*), (y), (n), (on), (off)
        if b == b'('
            && let Some((rendered, consumed)) = try_parse_emoticon(&text[i..], opts)
        {
            out.push_str(&rendered);
            i += consumed;
            continue;
        }

        // Default: escape markdown-significant characters and emit.
        push_escaped_byte(&mut out, b, &text[i..]);
        i += utf8_char_len(b);
    }
    out
}

/// Emit a byte as-is when the user explicitly escaped it with `\`. Markdown
/// significant chars still need escaping if we want to round-trip through the
/// markdown parser.
fn push_literal_byte(out: &mut String, b: u8) {
    match b {
        b'*' | b'_' | b'[' | b']' | b'\\' | b'`' => {
            out.push('\\');
            out.push(b as char);
        }
        _ => out.push(b as char),
    }
}

/// Default text-escaping: same rules as storage_to_md / adf_to_md.
fn push_escaped_byte(out: &mut String, b: u8, remaining: &str) {
    match b {
        b'*' | b'_' | b'[' | b']' | b'\\' | b'`' => {
            out.push('\\');
            out.push(b as char);
        }
        b':' => {
            // Escape `:` only when followed by ASCII alphabetic.
            let bytes = remaining.as_bytes();
            let needs_escape = bytes.len() > 1 && bytes[1].is_ascii_alphabetic();
            if needs_escape {
                out.push('\\');
            }
            out.push(':');
        }
        _ => {
            // Push the (possibly multi-byte) char verbatim.
            let ch_len = utf8_char_len(b);
            let end = ch_len.min(remaining.len());
            if let Ok(s) = std::str::from_utf8(&remaining.as_bytes()[..end]) {
                out.push_str(s);
            } else {
                out.push(b as char);
            }
        }
    }
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

fn find_inline_code_end(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Attempt to match a `*…*` / `_…_` / `+…+` paired marker starting at `start`.
/// Returns `Some((rendered, consumed))` with byte-length consumed including
/// both delimiters.
fn try_parse_paired(
    text: &str,
    start: usize,
    marker: u8,
    open_md: &str,
    close_md: &str,
    opts: &ConvertOpts,
) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let inner_start = start + 1;
    if inner_start >= bytes.len() {
        return None;
    }
    // Don't allow whitespace immediately after the opening marker (e.g. `* x`
    // is a list bullet, not bold).
    if bytes[inner_start].is_ascii_whitespace() {
        return None;
    }
    let mut i = inner_start;
    while i < bytes.len() {
        if bytes[i] == marker {
            // Don't accept if the byte before the close marker is whitespace.
            if i > inner_start && !bytes[i - 1].is_ascii_whitespace() {
                let inner = &text[inner_start..i];
                if !inner.is_empty() {
                    let rendered = format!("{}{}{}", open_md, render_inline(inner, opts), close_md);
                    return Some((rendered, i - start + 1));
                }
            }
            return None;
        }
        // Don't span newlines.
        if bytes[i] == b'\n' {
            return None;
        }
        i += 1;
    }
    None
}

fn is_strike_start(text: &str, i: usize) -> bool {
    let bytes = text.as_bytes();
    if i > 0 {
        let prev = bytes[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return false;
        }
    }
    if i + 1 >= bytes.len() {
        return false;
    }
    let next = bytes[i + 1];
    if next.is_ascii_whitespace() || next == b'-' {
        return false;
    }
    true
}

fn try_parse_paired_with_boundary(
    text: &str,
    start: usize,
    marker: u8,
    open_md: &str,
    close_md: &str,
    opts: &ConvertOpts,
) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let inner_start = start + 1;
    if inner_start >= bytes.len() {
        return None;
    }
    let mut i = inner_start;
    while i < bytes.len() {
        if bytes[i] == marker {
            // Boundary: byte after `-` must be non-alphanumeric / EOL.
            let after_ok = i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_alphanumeric();
            let prev_ok = !bytes[i - 1].is_ascii_whitespace();
            if after_ok && prev_ok {
                let inner = &text[inner_start..i];
                if !inner.is_empty() {
                    let rendered = format!("{}{}{}", open_md, render_inline(inner, opts), close_md);
                    return Some((rendered, i - start + 1));
                }
            }
            return None;
        }
        if bytes[i] == b'\n' {
            return None;
        }
        i += 1;
    }
    None
}

fn find_double_marker(text: &str, from: usize, c: char) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == c as u8 && bytes[i + 1] == c as u8 {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single_marker(text: &str, from: usize, c: char) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == c as u8 {
            return Some(i);
        }
        if bytes[i] == b'\n' {
            return None;
        }
        i += 1;
    }
    None
}

/// Try to parse `{status:...}` etc. starting at `text[0]`. Returns the
/// rendered output and how many bytes were consumed (including both braces).
fn try_parse_inline_macro(text: &str, opts: &ConvertOpts) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes[0] != b'{' {
        return None;
    }
    // Find closing `}` (no nesting in inline macros).
    let mut i = 1;
    while i < bytes.len() && bytes[i] != b'}' {
        if bytes[i] == b'\n' {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let inner = &text[1..i];
    let consumed = i + 1;

    // {status:colour=X|title=Y} → :status[Y]{color=x}
    if let Some(rest) = inner.strip_prefix("status") {
        let params_str = rest.strip_prefix(':').unwrap_or("");
        let params = parse_pipe_params(params_str);
        return Some((render_status(&params, opts), consumed));
    }

    None
}

/// Map a status-lozenge hex color to its conventional name.
///
/// Cloud's ADF→wiki downgrade emits a small set of fixed hex codes when it
/// rasterises a status node. We map both the "primary" and "darker" variants
/// of each Atlassian palette swatch back to the same name so the output is
/// stable. Unknown hex codes return `None` and the caller preserves the raw
/// hex as a quoted attr to keep the round-trip lossless.
fn status_hex_to_color_name(hex: &str) -> Option<&'static str> {
    match hex.to_ascii_lowercase().as_str() {
        "#36b37e" | "#00875a" => Some("green"),
        "#de350b" | "#bf2600" => Some("red"),
        "#ff991f" | "#ff8b00" => Some("yellow"),
        "#0052cc" | "#0747a6" => Some("blue"),
        "#42526e" | "#7a869a" => Some("neutral"),
        "#5243aa" | "#403294" => Some("purple"),
        _ => None,
    }
}

/// Recognise the literal `{color:#HHHHHH}*[ TEXT ]*{color}` pattern produced
/// by Cloud's ADF→wiki downgrade for an ADF status node.
///
/// `text` must start with `{` (caller has already checked). The match has to
/// be exact:
///
/// - Open: `{color:#HHHHHH}` where `HHHHHH` is exactly 6 hex digits.
/// - Body: `*[ TEXT ]*` — bold around a bracketed label. Whitespace inside
///   the brackets is allowed (the canonical Cloud emission has
///   `[ DONE ]`); the brackets themselves are not optional.
/// - Close: `{color}` immediately after the closing `*`.
///
/// Anything else returns `None` so a regular `{color:#xxx}plain text{color}`
/// passage is left unchanged.
fn try_parse_color_status_heuristic(text: &str, opts: &ConvertOpts) -> Option<(String, usize)> {
    // Open: `{color:#HHHHHH}` — keep it strict so partial matches don't
    // accidentally trigger.
    let after_open_prefix = text.strip_prefix("{color:#")?;
    let close_brace = after_open_prefix.find('}')?;
    let hex_digits = &after_open_prefix[..close_brace];
    if hex_digits.len() != 6 || !hex_digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let raw_hex = format!("#{hex_digits}");

    // `{color:#xxxxxx}` — `{color:#` is 8 bytes plus the 6 digits and the
    // closing `}`. The `?` prefix-strip already advanced past `{color:#`.
    let open_len = "{color:#".len() + close_brace + 1;
    let after_open = &text[open_len..];

    // Body: `*[ ... ]*` with optional whitespace inside the brackets.
    let after_bold_open = after_open.strip_prefix("*[")?;

    // Close `]*{color}` — search for the closing `]*` followed by `{color}`.
    let close_marker = "]*{color}";
    // Find the first occurrence of `]*` that is followed by `{color}`. We
    // scan rather than naively use `find("]*{color}")` so that an inner `]`
    // that doesn't lead into `*{color}` doesn't break the match.
    let mut search_from = 0;
    let close_idx = loop {
        let candidate = after_bold_open[search_from..].find("]*")?;
        let abs = search_from + candidate;
        if after_bold_open[abs..].starts_with(close_marker) {
            break abs;
        }
        search_from = abs + 2;
    };
    let body = &after_bold_open[..close_idx];
    // Body must NOT contain `\n` — keep the heuristic local to a single
    // logical line so it doesn't accidentally swallow a paragraph.
    if body.contains('\n') {
        return None;
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    let consumed = open_len + "*[".len() + close_idx + close_marker.len();

    if !opts.render_directives {
        return Some((trimmed.to_string(), consumed));
    }

    let mut rendered = String::new();
    rendered.push_str(":status[");
    rendered.push_str(trimmed);
    rendered.push(']');
    if let Some(name) = status_hex_to_color_name(&raw_hex) {
        let _ = write!(rendered, "{{color={name}}}");
    } else {
        // Preserve raw hex so a downstream md→storage/adf pipeline can
        // still attempt a faithful round-trip.
        let _ = write!(rendered, "{{color=\"{raw_hex}\"}}");
    }
    Some((rendered, consumed))
}

fn render_status(params: &BTreeMap<String, String>, opts: &ConvertOpts) -> String {
    let title = params.get("title").cloned().unwrap_or_default();
    let raw_color = params
        .get("colour")
        .or_else(|| params.get("color"))
        .cloned()
        .unwrap_or_default();

    if !opts.render_directives {
        return title;
    }

    let mut out = String::new();
    out.push_str(":status[");
    out.push_str(&title);
    out.push(']');
    if !raw_color.is_empty() {
        let lc = raw_color.to_lowercase();
        let _ = write!(out, "{{color={lc}}}");
    }
    out
}

/// Parse `!url!` or `!url|alt=...|width=...!` starting at `text[0]`.
fn try_parse_image(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes[0] != b'!' {
        return None;
    }
    // Find the closing `!`. The image cannot span newlines.
    let mut i = 1;
    while i < bytes.len() && bytes[i] != b'!' {
        if bytes[i] == b'\n' {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() || i == 1 {
        return None;
    }
    let inner = &text[1..i];
    // Reject if inner doesn't look like a URL (must contain something other
    // than digits/spaces — otherwise `! 5 ! 7 !` would be misparsed).
    if inner.trim().is_empty() {
        return None;
    }
    let mut parts = inner.split('|');
    let url = parts.next().unwrap_or("").trim();
    if url.is_empty() {
        return None;
    }
    let mut alt = String::new();
    for p in parts {
        if let Some(v) = p.strip_prefix("alt=") {
            alt = v.to_string();
        }
        // Other params (width=, height=, etc.) intentionally dropped.
    }
    Some((format!("![{alt}]({url})"), i + 1))
}

/// Parse `[...]`-prefixed link forms. Returns the rendered output and bytes
/// consumed (including both brackets).
fn try_parse_link(text: &str, opts: &ConvertOpts) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes[0] != b'[' {
        return None;
    }
    // Find the matching `]` respecting `\]` escapes.
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b']' {
            break;
        }
        if bytes[i] == b'\n' {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let inner = &text[1..i];
    let consumed = i + 1;

    // Mention: `[~accountid:abc]` or `[~user]`.
    if let Some(rest) = inner.strip_prefix('~') {
        return Some((render_mention(rest, opts), consumed));
    }

    if inner.is_empty() {
        return Some((String::new(), consumed));
    }

    // Split on `|`. First field may be the URL (autolink) or the text.
    let pipe_count = inner.bytes().filter(|&c| c == b'|').count();
    if pipe_count == 0 {
        // Autolink `[url]`. If it looks like a URL, emit `<url>`; else
        // emit literal brackets.
        if looks_like_url(inner) {
            return Some((format!("<{inner}>"), consumed));
        }
        // Plain `[text]` — emit as a text-only link. Markdown has no native
        // bare-bracket form; we emit literal text in brackets escaped to
        // avoid re-parsing.
        return Some((format!("\\[{}\\]", render_inline(inner, opts)), consumed));
    }

    // Split into at most 3 parts: text|url[|tip].
    let parts: Vec<&str> = inner.splitn(3, '|').collect();
    let (text_part, url, tip) = match parts.len() {
        2 => (parts[0], parts[1], None),
        3 => (parts[0], parts[1], Some(parts[2])),
        _ => unreachable!(),
    };
    let display = render_inline(text_part, opts);
    let url_clean = url.trim();
    if let Some(t) = tip {
        let tip_escaped = t.replace('"', "\\\"");
        return Some((
            format!("[{display}]({url_clean} \"{tip_escaped}\")"),
            consumed,
        ));
    }
    Some((format!("[{display}]({url_clean})"), consumed))
}

fn render_mention(rest: &str, opts: &ConvertOpts) -> String {
    if let Some(id) = rest.strip_prefix("accountid:") {
        if !opts.render_directives {
            return format!("@{id}");
        }
        return format!(":mention[]{{accountId={id}}}");
    }
    if !opts.render_directives {
        return format!("@{rest}");
    }
    format!(":mention[{rest}]{{username={rest}}}")
}

fn looks_like_url(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("mailto:")
        || lower.starts_with("file:")
}

fn try_parse_emoticon(text: &str, opts: &ConvertOpts) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'(') {
        return None;
    }
    // The canonical set + length lookup (longest first to avoid prefix issues).
    const EMOTICONS: &[(&str, &str)] = &[
        ("(off)", "off"),
        ("(on)", "on"),
        ("(!)", "warning"),
        ("(?)", "question"),
        ("(/)", "tick"),
        ("(x)", "cross"),
        ("(i)", "info"),
        ("(*)", "star"),
        ("(y)", "thumbs-up"),
        ("(n)", "thumbs-down"),
    ];
    for (token, name) in EMOTICONS {
        if text.starts_with(token) {
            if !opts.render_directives {
                return Some((String::new(), token.len()));
            }
            return Some((format!(":emoticon{{name={name}}}"), token.len()));
        }
    }
    None
}

// =====================================================================
// Output normalization
// =====================================================================

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

fn normalize_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newline_run = 0_usize;
    for c in s.chars() {
        if c == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                out.push('\n');
            }
        } else {
            newline_run = 0;
            out.push(c);
        }
    }
    out
}

/// Wrap `text` in a CommonMark code span, picking a backtick fence long
/// enough to avoid colliding with backticks inside `text`. If `text` starts
/// or ends with a backtick we pad with a single space (also CommonMark-compliant).
fn wrap_in_code_span(text: &str) -> String {
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

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(wiki: &str) -> String {
        wiki_to_markdown(wiki, ConvertOpts::default()).expect("conversion succeeded")
    }

    fn convert_no_directives(wiki: &str) -> String {
        wiki_to_markdown(
            wiki,
            ConvertOpts {
                render_directives: false,
            },
        )
        .expect("conversion succeeded")
    }

    // ---- ConvertOpts ------------------------------------------------------

    #[test]
    fn convert_opts_default_renders_directives() {
        assert!(ConvertOpts::default().render_directives);
    }

    // ---- parse_pipe_params ------------------------------------------------

    #[test]
    fn parse_pipe_params_handles_escaped_pipe() {
        // `\|` inside a value must NOT split the parameter list. The
        // value's `\|` decodes to a literal `|`.
        let params = parse_pipe_params(r"title=A\|B|color=red");
        assert_eq!(params.get("title").map(String::as_str), Some("A|B"));
        assert_eq!(params.get("color").map(String::as_str), Some("red"));
    }

    #[test]
    fn parse_pipe_params_handles_escaped_brace() {
        // `\}` inside a value decodes to a literal `}`.
        let params = parse_pipe_params(r"title=A\}B");
        assert_eq!(params.get("title").map(String::as_str), Some("A}B"));
    }

    #[test]
    fn parse_pipe_params_handles_escaped_backslash() {
        // `\\` decodes to a single literal `\`. The trailing `foo`
        // following the escaped backslash must not be re-escaped.
        let params = parse_pipe_params(r"path=C:\\foo");
        assert_eq!(params.get("path").map(String::as_str), Some(r"C:\foo"));
    }

    // ---- code-block fence picking -----------------------------------------

    #[test]
    fn wiki_to_md_code_block_no_backticks_uses_three_tick_fence() {
        // No backticks in the body — three-tick fence is sufficient.
        let out = convert("{code}\nhello\n{code}");
        assert!(
            out.contains("```\nhello\n```"),
            "expected a 3-tick fence around plain text, got: {out:?}"
        );
    }

    #[test]
    fn wiki_to_md_code_block_with_triple_backticks_uses_four_tick_fence() {
        // Body contains a 3-backtick run — fence must be at least 4 ticks
        // so the body's run cannot close the block prematurely.
        let out = convert("{code}\na ``` b\n{code}");
        assert!(
            out.contains("````\na ``` b\n````"),
            "expected a 4-tick fence around code containing ```, got: {out:?}"
        );
    }

    // ---- headings ---------------------------------------------------------

    #[test]
    fn heading_h1() {
        let out = convert("h1. Title");
        assert!(out.contains("# Title"), "got: {out:?}");
    }

    #[test]
    fn heading_h2() {
        let out = convert("h2. Sub");
        assert!(out.contains("## Sub"), "got: {out:?}");
    }

    #[test]
    fn heading_h6() {
        let out = convert("h6. Deep");
        assert!(out.contains("###### Deep"), "got: {out:?}");
    }

    #[test]
    fn heading_levels_3_to_5() {
        assert!(convert("h3. A").contains("### A"));
        assert!(convert("h4. B").contains("#### B"));
        assert!(convert("h5. C").contains("##### C"));
    }

    #[test]
    fn heading_with_inline_bold() {
        let out = convert("h1. *Bold* title");
        assert!(out.contains("**Bold**"), "got: {out:?}");
    }

    // ---- inline formatting ------------------------------------------------

    #[test]
    fn bold_to_double_asterisk() {
        let out = convert("*bold*");
        assert!(out.contains("**bold**"), "got: {out:?}");
    }

    #[test]
    fn italic_underscore_to_single_asterisk() {
        let out = convert("_italic_");
        assert!(out.contains("*italic*"), "got: {out:?}");
        assert!(!out.contains("_italic_"), "got: {out:?}");
    }

    #[test]
    fn strikethrough_word_boundaries() {
        let out = convert("-gone-");
        assert!(out.contains("~~gone~~"), "got: {out:?}");
    }

    #[test]
    fn strikethrough_inside_word_not_converted() {
        // `text-with-dash` should NOT be converted — dashes are surrounded by
        // alphanumerics.
        let out = convert("text-with-dash");
        assert!(!out.contains("~~"), "got: {out:?}");
    }

    #[test]
    fn underline_to_html() {
        let out = convert("+underlined+");
        assert!(out.contains("<u>underlined</u>"), "got: {out:?}");
    }

    #[test]
    fn inline_code_double_braces() {
        let out = convert("{{code}}");
        assert!(out.contains("`code`"), "got: {out:?}");
    }

    #[test]
    fn inline_code_with_embedded_backtick_uses_longer_fence() {
        // Bug 9: a single-backtick fence around `a`b` is malformed. The fence
        // must be longer than any internal run of backticks (CommonMark).
        let out = convert("{{a`b}}");
        assert!(out.contains("``a`b``"), "got: {out:?}");
    }

    #[test]
    fn inline_code_starting_with_backtick_pads_with_space() {
        // Bug 9 sibling: when the body starts with `\``, CommonMark requires
        // a single space pad between the fence and the body.
        let out = convert("{{`x}}");
        assert!(out.contains("`` `x ``"), "got: {out:?}");
    }

    #[test]
    fn multiple_inline_in_one_line() {
        let out = convert("*bold* and _italic_ and {{code}}");
        assert!(out.contains("**bold**"), "got: {out:?}");
        assert!(out.contains("*italic*"), "got: {out:?}");
        assert!(out.contains("`code`"), "got: {out:?}");
    }

    #[test]
    fn escape_asterisk_with_backslash() {
        let out = convert(r"\*not bold\*");
        // Escaped asterisks should be literal — markdown will see them as
        // escaped specials too, so `\*` stays.
        assert!(out.contains(r"\*not bold\*"), "got: {out:?}");
    }

    #[test]
    fn citation_to_cite_html() {
        let out = convert("??cited??");
        assert!(out.contains("<cite>cited</cite>"), "got: {out:?}");
    }

    #[test]
    fn subscript_to_sub_html() {
        let out = convert("H~2~O");
        assert!(out.contains("<sub>2</sub>"), "got: {out:?}");
    }

    #[test]
    fn superscript_to_sup_html() {
        let out = convert("E=mc^2^");
        assert!(out.contains("<sup>2</sup>"), "got: {out:?}");
    }

    // ---- links ------------------------------------------------------------

    #[test]
    fn autolink_url() {
        let out = convert("[https://example.com]");
        assert!(out.contains("<https://example.com>"), "got: {out:?}");
    }

    #[test]
    fn link_with_text_and_url() {
        let out = convert("[click|https://example.com]");
        assert!(out.contains("[click](https://example.com)"), "got: {out:?}");
    }

    #[test]
    fn link_with_tooltip() {
        let out = convert("[click|https://example.com|tooltip]");
        assert!(
            out.contains(r#"[click](https://example.com "tooltip")"#),
            "got: {out:?}"
        );
    }

    #[test]
    fn empty_link_brackets() {
        // Documented choice: empty brackets pass through escaped so the text
        // survives but doesn't become a markdown link.
        let out = convert("[]");
        // No URL → not a link; we emit the consumed bracket pair as nothing.
        assert!(!out.contains("[]("), "got: {out:?}");
    }

    #[test]
    fn link_with_special_text_chars() {
        let out = convert("[a*b|https://x]");
        // Text part is rendered through inline so `*` is escaped.
        assert!(out.contains("(https://x)"), "got: {out:?}");
    }

    // ---- mentions ---------------------------------------------------------

    #[test]
    fn mention_account_id_directives_on() {
        let out = convert("[~accountid:abc123]");
        assert!(out.contains(":mention[]{accountId=abc123}"), "got: {out:?}");
    }

    #[test]
    fn mention_username_directives_on() {
        let out = convert("[~jdoe]");
        assert!(out.contains(":mention[jdoe]"), "got: {out:?}");
        assert!(out.contains("username=jdoe"), "got: {out:?}");
    }

    #[test]
    fn mention_directives_off_emits_at_handle() {
        let out = convert_no_directives("[~jdoe]");
        assert!(out.contains("@jdoe"), "got: {out:?}");
        assert!(!out.contains(":mention"), "got: {out:?}");
    }

    #[test]
    fn mention_account_id_directives_off() {
        let out = convert_no_directives("[~accountid:abc]");
        assert!(out.contains("@abc"), "got: {out:?}");
    }

    // ---- images -----------------------------------------------------------

    #[test]
    fn image_url_only() {
        let out = convert("!http://x.png!");
        assert!(out.contains("![](http://x.png)"), "got: {out:?}");
    }

    #[test]
    fn image_with_alt() {
        let out = convert("!http://x.png|alt=picture!");
        assert!(out.contains("![picture](http://x.png)"), "got: {out:?}");
    }

    #[test]
    fn image_extra_params_dropped() {
        let out = convert("!http://x.png|width=300|alt=foo!");
        assert!(out.contains("![foo](http://x.png)"), "got: {out:?}");
        // width should be dropped.
        assert!(!out.contains("width="), "got: {out:?}");
    }

    // ---- lists ------------------------------------------------------------

    #[test]
    fn bullet_list_simple() {
        let out = convert("* a\n* b");
        assert!(out.contains("- a"), "got: {out:?}");
        assert!(out.contains("- b"), "got: {out:?}");
    }

    #[test]
    fn bullet_list_nested_two_levels() {
        let out = convert("* a\n** b");
        assert!(out.contains("- a"), "got: {out:?}");
        assert!(out.contains("  - b"), "got: {out:?}");
    }

    #[test]
    fn ordered_list_simple() {
        let out = convert("# a\n# b");
        assert!(out.contains("1. a"), "got: {out:?}");
        assert!(out.contains("1. b"), "got: {out:?}");
    }

    #[test]
    fn mixed_bullet_with_ordered_nested() {
        let out = convert("* a\n## b");
        assert!(out.contains("- a"), "got: {out:?}");
        assert!(out.contains("  1. b"), "got: {out:?}");
    }

    #[test]
    fn list_with_inline_formatting() {
        let out = convert("* *bold* item");
        assert!(out.contains("- **bold** item"), "got: {out:?}");
    }

    // ---- tables -----------------------------------------------------------

    #[test]
    fn table_with_header_and_data() {
        let out = convert("||h1||h2||\n|c1|c2|");
        assert!(out.contains("| h1 | h2 |"), "got: {out:?}");
        assert!(out.contains("| --- | --- |"), "got: {out:?}");
        assert!(out.contains("| c1 | c2 |"), "got: {out:?}");
    }

    #[test]
    fn table_header_only() {
        let out = convert("||h1||h2||");
        assert!(out.contains("| h1 | h2 |"), "got: {out:?}");
    }

    #[test]
    fn table_cell_with_inline_formatting() {
        let out = convert("||head||\n|*bold* cell|");
        assert!(out.contains("**bold**"), "got: {out:?}");
    }

    // ---- code blocks ------------------------------------------------------

    #[test]
    fn code_block_no_lang() {
        let out = convert("{code}\nplain\n{code}");
        assert!(out.contains("```\nplain\n```"), "got: {out:?}");
    }

    #[test]
    fn code_block_with_lang() {
        let out = convert("{code:rust}\nfn x(){}\n{code}");
        assert!(out.contains("```rust"), "got: {out:?}");
        assert!(out.contains("fn x(){}"), "got: {out:?}");
        assert!(out.contains("```\n"), "got: {out:?}");
    }

    #[test]
    fn noformat_block() {
        let out = convert("{noformat}\nstuff\n{noformat}");
        assert!(out.contains("```\nstuff\n```"), "got: {out:?}");
    }

    #[test]
    fn code_block_preserves_wiki_syntax_inside() {
        // A `*not bold*` inside a code block should stay literal — the code
        // body must not run through the inline parser.
        let out = convert("{code}\n*not bold*\n{code}");
        assert!(out.contains("*not bold*"), "got: {out:?}");
        // No markdown bold conversion should leak in.
        assert!(!out.contains("**not"), "got: {out:?}");
    }

    // ---- block quotes -----------------------------------------------------

    #[test]
    fn blockquote_single_line() {
        let out = convert("bq. quoted line");
        assert!(out.contains("> quoted line"), "got: {out:?}");
    }

    #[test]
    fn blockquote_multi_line() {
        let out = convert("{quote}\nline1\nline2\n{quote}");
        assert!(out.contains("> line1"), "got: {out:?}");
        assert!(out.contains("> line2"), "got: {out:?}");
    }

    // ---- block macros -----------------------------------------------------

    #[test]
    fn info_macro_directive() {
        let out = convert("{info}\nbody\n{info}");
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains("body"), "got: {out:?}");
    }

    #[test]
    fn info_macro_with_title() {
        let out = convert("{info:title=Heads up}\nbody\n{info}");
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains(r#"title="Heads up""#), "got: {out:?}");
        assert!(out.contains("body"), "got: {out:?}");
    }

    #[test]
    fn info_macro_with_multiple_params() {
        let out = convert("{info:title=A|key=val}\nbody\n{info}");
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains("key=val"), "got: {out:?}");
        assert!(out.contains("title=A"), "got: {out:?}");
    }

    #[test]
    fn warning_macro_directive() {
        let out = convert("{warning}\nbody\n{warning}");
        assert!(out.contains(":::warning"), "got: {out:?}");
    }

    #[test]
    fn note_macro_directive() {
        let out = convert("{note}\nbody\n{note}");
        assert!(out.contains(":::note"), "got: {out:?}");
    }

    #[test]
    fn tip_macro_directive() {
        let out = convert("{tip}\nbody\n{tip}");
        assert!(out.contains(":::tip"), "got: {out:?}");
    }

    #[test]
    fn toc_self_closing() {
        let out = convert("{toc}");
        assert!(out.contains(":::toc"), "got: {out:?}");
    }

    #[test]
    fn toc_self_closing_with_params() {
        let out = convert("{toc:maxLevel=3}");
        assert!(out.contains(":::toc"), "got: {out:?}");
        assert!(out.contains("maxLevel=3"), "got: {out:?}");
    }

    #[test]
    fn nested_block_macros() {
        let out = convert("{info}\n{warning}\nx\n{warning}\n{info}");
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains(":::warning"), "got: {out:?}");
        assert!(out.contains('x'), "got: {out:?}");
    }

    #[test]
    fn render_directives_false_strips_macro() {
        let out = convert_no_directives("{info}\nbody\n{info}");
        assert!(out.contains("body"), "got: {out:?}");
        assert!(!out.contains(":::info"), "got: {out:?}");
    }

    #[test]
    fn render_directives_false_strips_toc() {
        let out = convert_no_directives("{toc}");
        assert!(!out.contains(":::toc"), "got: {out:?}");
    }

    // ---- inline macros ----------------------------------------------------

    #[test]
    fn inline_status_with_color() {
        let out = convert("{status:colour=Green|title=DONE}");
        assert!(out.contains(":status[DONE]"), "got: {out:?}");
        assert!(out.contains("color=green"), "got: {out:?}");
    }

    #[test]
    fn inline_status_no_color() {
        let out = convert("{status:title=ONLY}");
        assert!(out.contains(":status[ONLY]"), "got: {out:?}");
        assert!(!out.contains("color="), "got: {out:?}");
    }

    #[test]
    fn inline_status_directives_off() {
        let out = convert_no_directives("{status:colour=Green|title=DONE}");
        assert!(out.contains("DONE"), "got: {out:?}");
        assert!(!out.contains(":status"), "got: {out:?}");
    }

    #[test]
    fn emoticon_warning() {
        let out = convert("(!)");
        assert!(out.contains(":emoticon"), "got: {out:?}");
        assert!(out.contains("name=warning"), "got: {out:?}");
    }

    #[test]
    fn emoticon_question() {
        let out = convert("(?)");
        assert!(out.contains("name=question"), "got: {out:?}");
    }

    #[test]
    fn emoticon_tick() {
        let out = convert("(/)");
        assert!(out.contains("name=tick"), "got: {out:?}");
    }

    #[test]
    fn emoticon_cross() {
        let out = convert("(x)");
        assert!(out.contains("name=cross"), "got: {out:?}");
    }

    #[test]
    fn emoticon_info() {
        let out = convert("(i)");
        assert!(out.contains("name=info"), "got: {out:?}");
    }

    #[test]
    fn emoticon_unknown_paren_text_stays_literal() {
        // `(blah)` is NOT a canonical emoticon — must stay as literal text.
        let out = convert("(blah)");
        assert!(!out.contains(":emoticon"), "got: {out:?}");
        assert!(out.contains("(blah)"), "got: {out:?}");
    }

    #[test]
    fn emoticon_directives_off_drops() {
        let out = convert_no_directives("(!)");
        assert!(!out.contains(":emoticon"), "got: {out:?}");
        assert!(!out.contains("(!)"), "got: {out:?}");
    }

    // ---- edge cases -------------------------------------------------------

    #[test]
    fn empty_input() {
        let out = convert("");
        assert!(out.is_empty(), "got: {out:?}");
    }

    #[test]
    fn whitespace_only_input() {
        let out = convert("   \n\n  \n");
        assert!(out.trim().is_empty(), "got: {out:?}");
    }

    #[test]
    fn unclosed_code_block_does_not_panic() {
        let out = convert("{code}\nstuff\nmore");
        assert!(out.contains("stuff"), "got: {out:?}");
        // Should still emit a code block.
        assert!(out.contains("```"), "got: {out:?}");
    }

    #[test]
    fn unclosed_macro_does_not_panic() {
        let out = convert("{info}\nbody");
        // Body must survive even though macro never closes.
        assert!(out.contains("body"), "got: {out:?}");
    }

    #[test]
    fn horizontal_rule() {
        let out = convert("----");
        assert!(out.contains("---"), "got: {out:?}");
    }

    #[test]
    fn hard_break_in_paragraph() {
        let out = convert("text\\\\\nmore");
        // `\\` at end of line becomes `  \n` (markdown hard break).
        assert!(out.contains("  \n"), "got: {out:?}");
    }

    #[test]
    fn paragraph_starting_with_directive_marker_is_escaped() {
        // A literal `:::info` line in plain paragraph text would otherwise
        // re-trigger directive parsing on round-trip. The colon-alpha escape
        // (`:i` → `\:i`) breaks the `:::name` token so a downstream markdown
        // directive parser won't recognise it as an open fence.
        let out = convert(":::info");
        assert!(out.contains(r"\:"), "got: {out:?}");
        assert!(!out.contains(":::info"), "got: {out:?}");
    }

    #[test]
    fn colon_alpha_in_text_is_escaped() {
        let out = convert("see :foo here");
        assert!(out.contains(r"\:foo"), "got: {out:?}");
    }

    #[test]
    fn https_url_colon_not_escaped() {
        let out = convert("see https://example.com today");
        // `:` followed by `/` — not alpha, no escape.
        assert!(!out.contains(r"https\:"), "got: {out:?}");
    }

    #[test]
    fn note_colon_space_not_escaped() {
        let out = convert("note: text");
        assert!(!out.contains(r"\: "), "got: {out:?}");
    }

    // ---- round-trip sanity ------------------------------------------------

    #[test]
    fn roundtrip_info_directive() {
        use crate::cli::commands::converters::md_to_wiki::markdown_to_wiki;
        let wiki = markdown_to_wiki(":::info\nHello\n:::").unwrap();
        let md = wiki_to_markdown(&wiki, ConvertOpts::default()).unwrap();
        assert!(md.contains(":::info"), "round-trip lost directive: {md:?}");
        assert!(md.contains("Hello"), "round-trip lost body: {md:?}");
    }

    #[test]
    fn roundtrip_heading_and_list() {
        use crate::cli::commands::converters::md_to_wiki::markdown_to_wiki;
        let wiki = markdown_to_wiki("# Title\n\n- a\n- b").unwrap();
        let md = wiki_to_markdown(&wiki, ConvertOpts::default()).unwrap();
        assert!(md.contains("# Title"), "got: {md:?}");
        assert!(md.contains("- a"), "got: {md:?}");
        assert!(md.contains("- b"), "got: {md:?}");
    }

    #[test]
    fn roundtrip_status_inline() {
        use crate::cli::commands::converters::md_to_wiki::markdown_to_wiki;
        let wiki = markdown_to_wiki(":status[DONE]{color=green}").unwrap();
        let md = wiki_to_markdown(&wiki, ConvertOpts::default()).unwrap();
        assert!(md.contains(":status[DONE]"), "got: {md:?}");
        assert!(md.contains("color=green"), "got: {md:?}");
    }

    // ---- realistic doc ----------------------------------------------------

    #[test]
    fn realistic_document() {
        let input = "\
h1. Title

A paragraph with *bold* and _italic_ and {{code}}.

h2. Section

* item 1
* item 2
** nested

||name||value||
|a|1|

{code:rust}
fn main() {}
{code}

See the [docs|https://example.com] for more.

{info}
This is informational.
{info}
";
        let out = convert(input);

        // Original wiki syntax should not leak through.
        assert!(!out.contains("h1. "), "got: {out:?}");
        assert!(!out.contains("{code:"), "got: {out:?}");
        assert!(!out.contains("{info}"), "got: {out:?}");
        // Markdown tokens present.
        assert!(out.contains("# Title"), "got: {out:?}");
        assert!(out.contains("## Section"), "got: {out:?}");
        assert!(out.contains("**bold**"), "got: {out:?}");
        assert!(out.contains("*italic*"), "got: {out:?}");
        assert!(out.contains("`code`"), "got: {out:?}");
        assert!(out.contains("- item 1"), "got: {out:?}");
        assert!(out.contains("  - nested"), "got: {out:?}");
        assert!(out.contains("```rust"), "got: {out:?}");
        assert!(out.contains("[docs](https://example.com)"), "got: {out:?}");
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains("This is informational."), "got: {out:?}");
    }

    // ---- Cloud-after-ADF panel recovery (Fix 1A) -------------------------

    #[test]
    fn panel_bgcolor_to_directive_known_hex_maps_to_directive() {
        assert_eq!(panel_bgcolor_to_directive("#deebff"), Some("info"));
        assert_eq!(panel_bgcolor_to_directive("#fffae6"), Some("warning"));
        assert_eq!(panel_bgcolor_to_directive("#ffebe6"), Some("warning"));
        assert_eq!(panel_bgcolor_to_directive("#e3fcef"), Some("tip"));
        assert_eq!(panel_bgcolor_to_directive("#eae6ff"), Some("note"));
    }

    #[test]
    fn panel_bgcolor_to_directive_is_case_insensitive() {
        assert_eq!(panel_bgcolor_to_directive("#DEEBFF"), Some("info"));
        assert_eq!(panel_bgcolor_to_directive("#FfFaE6"), Some("warning"));
    }

    #[test]
    fn panel_bgcolor_to_directive_unknown_returns_none() {
        assert!(panel_bgcolor_to_directive("#123456").is_none());
        assert!(panel_bgcolor_to_directive("not-a-hex").is_none());
    }

    #[test]
    fn panel_bgcolor_info_maps_to_info_directive() {
        let out = convert("{panel:bgColor=#deebff}\nbody\n{panel}");
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains("body"), "got: {out:?}");
        // bgColor should be dropped once we've remapped the directive name.
        assert!(!out.contains("bgColor"), "got: {out:?}");
        assert!(!out.contains(":::panel"), "got: {out:?}");
    }

    #[test]
    fn panel_bgcolor_warning_maps_to_warning_directive() {
        let out = convert("{panel:bgColor=#fffae6}\ncareful\n{panel}");
        assert!(out.contains(":::warning"), "got: {out:?}");
        assert!(out.contains("careful"), "got: {out:?}");
        assert!(!out.contains("bgColor"), "got: {out:?}");
    }

    #[test]
    fn panel_bgcolor_unknown_falls_back_to_unknown_panel_directive() {
        let out = convert("{panel:bgColor=#abcdef}\nbody\n{panel}");
        // Unknown color → keep the raw `bgColor` attr on a `:::panel`
        // directive so a downstream md→storage/adf pipeline can still
        // round-trip the original color.
        assert!(out.contains(":::panel"), "got: {out:?}");
        assert!(out.contains("body"), "got: {out:?}");
        assert!(out.contains("bgColor=#abcdef"), "got: {out:?}");
    }

    #[test]
    fn panel_without_bgcolor_falls_back_to_unknown_panel_directive() {
        let out = convert("{panel}\nbody\n{panel}");
        assert!(out.contains(":::panel"), "got: {out:?}");
        assert!(out.contains("body"), "got: {out:?}");
        assert!(!out.contains("bgColor"), "got: {out:?}");
    }

    #[test]
    fn panel_with_unknown_attr_only_falls_back_to_panel_directive() {
        // `{panel:title=Heads up}` — no bgColor, so we keep `:::panel` and
        // pass the title through. This shape is rare but should not crash.
        let out = convert("{panel:title=Heads up}\nbody\n{panel}");
        assert!(out.contains(":::panel"), "got: {out:?}");
        assert!(out.contains("body"), "got: {out:?}");
        assert!(out.contains(r#"title="Heads up""#), "got: {out:?}");
    }

    // ---- Cloud-after-ADF inline code recovery (Fix 1B) -------------------

    #[test]
    fn inline_code_macro_single_line_emits_fenced_block() {
        let out = convert("{code}print('x'){code}");
        assert!(out.contains("```\nprint('x')\n```"), "got: {out:?}");
    }

    #[test]
    fn inline_code_macro_with_language_emits_lang_fence() {
        let out = convert("{code:python}print('x'){code}");
        assert!(out.contains("```python"), "got: {out:?}");
        assert!(out.contains("print('x')"), "got: {out:?}");
        assert!(out.contains("```\n"), "got: {out:?}");
    }

    #[test]
    fn inline_code_macro_trims_inner_whitespace() {
        let out = convert("{code:python}   print('x')   {code}");
        // Whitespace immediately around the body should be stripped before
        // emitting the fenced block.
        assert!(out.contains("```python\nprint('x')\n```"), "got: {out:?}");
    }

    #[test]
    fn multiline_code_macro_still_works() {
        // Regression: the multi-line form must still parse correctly.
        let out = convert("{code:rust}\nfn main() {}\n{code}");
        assert!(out.contains("```rust"), "got: {out:?}");
        assert!(out.contains("fn main() {}"), "got: {out:?}");
        assert!(out.contains("```\n"), "got: {out:?}");
    }

    #[test]
    fn inline_code_macro_unclosed_falls_through() {
        // No `{code}` close on the line → treat as multi-line opener.
        let out = convert("{code:python}\nprint('x')\n{code}");
        assert!(out.contains("```python\nprint('x')\n```"), "got: {out:?}");
    }

    // ---- Cloud-after-ADF status heuristic (Fix 1C) -----------------------

    #[test]
    fn status_hex_to_color_name_known_palette() {
        assert_eq!(status_hex_to_color_name("#36b37e"), Some("green"));
        assert_eq!(status_hex_to_color_name("#00875a"), Some("green"));
        assert_eq!(status_hex_to_color_name("#de350b"), Some("red"));
        assert_eq!(status_hex_to_color_name("#bf2600"), Some("red"));
        assert_eq!(status_hex_to_color_name("#ff991f"), Some("yellow"));
        assert_eq!(status_hex_to_color_name("#0052cc"), Some("blue"));
        assert_eq!(status_hex_to_color_name("#42526e"), Some("neutral"));
        assert_eq!(status_hex_to_color_name("#5243aa"), Some("purple"));
    }

    #[test]
    fn status_hex_to_color_name_unknown_returns_none() {
        assert!(status_hex_to_color_name("#123456").is_none());
    }

    #[test]
    fn status_heuristic_canonical_green_maps_to_status_directive() {
        let out = convert("{color:#36B37E}*[ DONE ]*{color}");
        assert!(out.contains(":status[DONE]"), "got: {out:?}");
        assert!(out.contains("color=green"), "got: {out:?}");
    }

    #[test]
    fn status_heuristic_red_maps_to_status_directive() {
        let out = convert("{color:#de350b}*[ BLOCKED ]*{color}");
        assert!(out.contains(":status[BLOCKED]"), "got: {out:?}");
        assert!(out.contains("color=red"), "got: {out:?}");
    }

    #[test]
    fn status_heuristic_unknown_hex_keeps_raw_color_attr() {
        let out = convert("{color:#abcdef}*[ FOO ]*{color}");
        assert!(out.contains(":status[FOO]"), "got: {out:?}");
        // Unknown hex → preserve as quoted raw attr.
        assert!(out.contains(r##"color="#abcdef""##), "got: {out:?}");
    }

    #[test]
    fn status_heuristic_partial_pattern_left_as_is() {
        // No `*[ ]*` wrapping — this is a regular colored text run, not a
        // status lozenge. Must not be transformed.
        let out = convert("{color:#36B37E}plain text{color}");
        assert!(!out.contains(":status"), "got: {out:?}");
    }

    #[test]
    fn status_heuristic_partial_pattern_brackets_only_left_as_is() {
        // Brackets but no bold wrapping — also not a status pattern.
        let out = convert("{color:#36B37E}[ DONE ]{color}");
        assert!(!out.contains(":status"), "got: {out:?}");
    }

    #[test]
    fn status_heuristic_directives_off_emits_text_only() {
        let out = convert_no_directives("{color:#36B37E}*[ DONE ]*{color}");
        assert!(out.contains("DONE"), "got: {out:?}");
        assert!(!out.contains(":status"), "got: {out:?}");
    }
}
