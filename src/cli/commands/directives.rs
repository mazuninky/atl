//! Foundation for the markdown converters that bridge user-facing markdown to
//! Confluence storage XHTML, ADF JSON, and Jira wiki text.
//!
//! Markdown alone cannot represent Confluence macros (info / warning / expand /
//! TOC) or Jira panels, so this module defines a parser for MyST-style
//! directives that the converters use as the lossless intermediate
//! representation. Block directives use `:::name` fences; inline directives
//! use `:name[content]{attrs}` tokens. The directive registry records the
//! per-target metadata each converter needs to render a directive back out as
//! storage XML, ADF JSON, or Jira wiki.
//!
//! This module is parsing-only: it returns events / tokens and metadata. It
//! does **not** render to any target format and it performs no IO. Rendering
//! lives inside each converter under `src/cli/commands/converters/*` (added
//! later). The parser is also unaware of CommonMark fenced code blocks
//! (` ``` `): callers must skip lines inside code fences themselves before
//! feeding lines to [`BlockLexer`], otherwise a literal `:::info` inside a
//! code block will be parsed as a directive open. See
//! `lexer_does_not_know_about_code_fences` for an illustration of that
//! contract.

use std::collections::BTreeMap;

use thiserror::Error;

// =====================================================================
// Errors
// =====================================================================

/// Errors that can be produced while lexing block directives or parsing
/// attribute lists.
#[derive(Debug, Error)]
pub enum DirectiveError {
    /// A `:::name` fence was opened but never closed before EOF.
    #[error("unclosed directive `{name}` (opened at depth {depth})")]
    Unclosed {
        /// Name of the directive that was left open.
        name: String,
        /// Stack depth at which the directive was opened (1 = outermost).
        depth: usize,
    },

    /// An attribute list (`key=val key2="val 2"`) failed to parse.
    #[error("malformed attribute list: {0}")]
    BadAttrs(String),

    /// A `:::` close fence appeared while the lexer's stack was empty. Today
    /// this is reported by callers that opt into strict checking; the default
    /// [`BlockLexer`] passes the stray fence through as a [`BlockEvent::Line`].
    #[error("close fence at depth 0 (no open directive)")]
    UnexpectedClose,
}

// =====================================================================
// Spec table (the directive registry)
// =====================================================================

/// Whether a directive lives at block scope (`:::name … :::`) or inline scope
/// (`:name[…]{…}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveKind {
    /// Block-level directive — must occupy whole lines.
    Block,
    /// Inline directive — appears within a paragraph or other inline run.
    Inline,
}

/// Static metadata for one known directive.
///
/// Each converter consults the relevant target field
/// (`conf_storage_macro`, `conf_adf_node_type`, etc.) to render the
/// directive back to its native representation. A `None` field means the
/// directive has no equivalent in that target, and the converter must
/// degrade gracefully (typically by emitting the directive's body as plain
/// content with a warning).
#[derive(Debug, Clone, Copy)]
pub struct DirectiveSpec {
    /// Directive name as it appears in markdown (e.g. `"info"`, `"status"`).
    pub name: &'static str,

    /// Block or inline scope.
    pub kind: DirectiveKind,

    /// Whether the directive can have a body. Self-closing directives
    /// (`toc`, `emoticon`, `image`) set this to `false`.
    pub allows_body: bool,

    /// Confluence storage XML macro name, e.g. `"info"`, `"warning"`,
    /// `"expand"`, `"toc"`. `None` means no storage-XML equivalent.
    pub conf_storage_macro: Option<&'static str>,

    /// Confluence ADF JSON top-level `"type"` for the node, e.g.
    /// `"panel"`, `"expand"`, `"status"`, `"mention"`, `"emoji"`,
    /// `"inlineCard"`, `"mediaSingle"`.
    pub conf_adf_node_type: Option<&'static str>,

    /// For ADF `"panel"` nodes only — value of `attrs.panelType`, e.g.
    /// `"info"`, `"warning"`, `"note"`, `"success"` (used for `tip`).
    pub conf_adf_panel_type: Option<&'static str>,

    /// Jira wiki block name, e.g. `"info"`, `"warning"`. `None` means
    /// Jira wiki has no equivalent (e.g. `expand`, `toc`).
    pub jira_wiki_block: Option<&'static str>,

    /// Jira ADF JSON node `"type"`. Same set as
    /// [`Self::conf_adf_node_type`] but kept separate because some
    /// directives behave differently on Jira (e.g. `expand` panels).
    pub jira_adf_node_type: Option<&'static str>,

    /// For Jira ADF `"panel"` nodes — value of `attrs.panelType`.
    pub jira_adf_panel_type: Option<&'static str>,
}

const SPECS: &[DirectiveSpec] = &[
    // ---- block ----
    DirectiveSpec {
        name: "info",
        kind: DirectiveKind::Block,
        allows_body: true,
        conf_storage_macro: Some("info"),
        conf_adf_node_type: Some("panel"),
        conf_adf_panel_type: Some("info"),
        jira_wiki_block: Some("info"),
        jira_adf_node_type: Some("panel"),
        jira_adf_panel_type: Some("info"),
    },
    DirectiveSpec {
        name: "warning",
        kind: DirectiveKind::Block,
        allows_body: true,
        conf_storage_macro: Some("warning"),
        conf_adf_node_type: Some("panel"),
        conf_adf_panel_type: Some("warning"),
        jira_wiki_block: Some("warning"),
        jira_adf_node_type: Some("panel"),
        jira_adf_panel_type: Some("warning"),
    },
    DirectiveSpec {
        name: "note",
        kind: DirectiveKind::Block,
        allows_body: true,
        conf_storage_macro: Some("note"),
        conf_adf_node_type: Some("panel"),
        conf_adf_panel_type: Some("note"),
        jira_wiki_block: Some("note"),
        jira_adf_node_type: Some("panel"),
        jira_adf_panel_type: Some("note"),
    },
    DirectiveSpec {
        name: "tip",
        kind: DirectiveKind::Block,
        allows_body: true,
        conf_storage_macro: Some("tip"),
        conf_adf_node_type: Some("panel"),
        conf_adf_panel_type: Some("success"),
        jira_wiki_block: Some("tip"),
        jira_adf_node_type: Some("panel"),
        jira_adf_panel_type: Some("success"),
    },
    DirectiveSpec {
        name: "expand",
        kind: DirectiveKind::Block,
        allows_body: true,
        conf_storage_macro: Some("expand"),
        conf_adf_node_type: Some("expand"),
        conf_adf_panel_type: None,
        jira_wiki_block: None,
        jira_adf_node_type: Some("expand"),
        jira_adf_panel_type: None,
    },
    DirectiveSpec {
        name: "toc",
        kind: DirectiveKind::Block,
        allows_body: false,
        conf_storage_macro: Some("toc"),
        conf_adf_node_type: None,
        conf_adf_panel_type: None,
        jira_wiki_block: None,
        jira_adf_node_type: None,
        jira_adf_panel_type: None,
    },
    // ---- inline ----
    DirectiveSpec {
        name: "status",
        kind: DirectiveKind::Inline,
        allows_body: true,
        conf_storage_macro: Some("status"),
        conf_adf_node_type: Some("status"),
        conf_adf_panel_type: None,
        jira_wiki_block: None,
        jira_adf_node_type: Some("status"),
        jira_adf_panel_type: None,
    },
    DirectiveSpec {
        name: "emoticon",
        kind: DirectiveKind::Inline,
        allows_body: false,
        conf_storage_macro: None,
        conf_adf_node_type: Some("emoji"),
        conf_adf_panel_type: None,
        jira_wiki_block: None,
        jira_adf_node_type: Some("emoji"),
        jira_adf_panel_type: None,
    },
    DirectiveSpec {
        name: "mention",
        kind: DirectiveKind::Inline,
        allows_body: true,
        conf_storage_macro: None,
        conf_adf_node_type: Some("mention"),
        conf_adf_panel_type: None,
        jira_wiki_block: None,
        jira_adf_node_type: Some("mention"),
        jira_adf_panel_type: None,
    },
    DirectiveSpec {
        name: "link",
        kind: DirectiveKind::Inline,
        allows_body: true,
        conf_storage_macro: None,
        conf_adf_node_type: Some("inlineCard"),
        conf_adf_panel_type: None,
        jira_wiki_block: None,
        jira_adf_node_type: Some("inlineCard"),
        jira_adf_panel_type: None,
    },
    DirectiveSpec {
        name: "image",
        kind: DirectiveKind::Inline,
        allows_body: false,
        conf_storage_macro: None,
        conf_adf_node_type: Some("mediaSingle"),
        conf_adf_panel_type: None,
        jira_wiki_block: None,
        jira_adf_node_type: Some("mediaSingle"),
        jira_adf_panel_type: None,
    },
];

/// Look up a directive spec by name.
///
/// Returns `None` if `name` is not a registered directive.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static DirectiveSpec> {
    SPECS.iter().find(|s| s.name == name)
}

/// All registered block-scope directive specs.
#[must_use]
pub fn block_specs() -> Vec<&'static DirectiveSpec> {
    SPECS
        .iter()
        .filter(|s| matches!(s.kind, DirectiveKind::Block))
        .collect()
}

/// All registered inline-scope directive specs.
#[must_use]
pub fn inline_specs() -> Vec<&'static DirectiveSpec> {
    SPECS
        .iter()
        .filter(|s| matches!(s.kind, DirectiveKind::Inline))
        .collect()
}

// =====================================================================
// Attribute grammar
// =====================================================================

/// Parse an attribute list of the form `key=value key2="value with spaces"`.
///
/// Grammar:
///
/// - `key=value` — value is unquoted (no whitespace, no `=`, no `"`).
/// - `key="value"` — value is double-quoted; `\"` escapes a literal `"`,
///   `\\` escapes a literal `\`.
/// - `key=` — accepted as an empty-string value.
/// - `key` (no `=`) — rejected as malformed.
/// - Pairs are separated by one or more whitespace characters. Leading and
///   trailing whitespace is ignored. The empty input string parses as an
///   empty map (not an error).
/// - Keys must match `[A-Za-z][A-Za-z0-9_-]*`.
///
/// # Examples
///
/// ```ignore
/// let m = parse_attrs("title=\"Heads up\" mode=collapsed").unwrap();
/// assert_eq!(m["title"], "Heads up");
/// assert_eq!(m["mode"], "collapsed");
/// ```
pub fn parse_attrs(s: &str) -> Result<BTreeMap<String, String>, DirectiveError> {
    let mut out = BTreeMap::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    let n = bytes.len();

    loop {
        // skip whitespace
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }

        // parse key
        let key_start = i;
        if !is_key_start(bytes[i]) {
            return Err(DirectiveError::BadAttrs(format!(
                "unexpected character `{}` at position {i}",
                bytes[i] as char
            )));
        }
        i += 1;
        while i < n && is_key_cont(bytes[i]) {
            i += 1;
        }
        let key = std::str::from_utf8(&bytes[key_start..i])
            .map_err(|e| DirectiveError::BadAttrs(format!("invalid utf-8 in key: {e}")))?
            .to_string();

        // expect '='
        if i >= n || bytes[i] != b'=' {
            return Err(DirectiveError::BadAttrs(format!(
                "key `{key}` must be followed by `=`"
            )));
        }
        i += 1;

        // parse value
        //
        // We collect raw bytes (not chars) so that multi-byte UTF-8 sequences
        // round-trip correctly. The escape sequences (`\"`, `\\`) are pure
        // ASCII so byte-level handling of the escape ITSELF is fine, but the
        // surrounding content must be preserved verbatim. After the loop we
        // decode the collected bytes once via `String::from_utf8`.
        let value = if i < n && bytes[i] == b'"' {
            i += 1;
            let mut buf: Vec<u8> = Vec::new();
            loop {
                if i >= n {
                    return Err(DirectiveError::BadAttrs(format!(
                        "unterminated quoted value for `{key}`"
                    )));
                }
                let b = bytes[i];
                if b == b'\\' && i + 1 < n {
                    let next = bytes[i + 1];
                    if next == b'"' || next == b'\\' {
                        buf.push(next);
                        i += 2;
                        continue;
                    }
                    buf.push(b'\\');
                    i += 1;
                    continue;
                }
                if b == b'"' {
                    i += 1;
                    break;
                }
                buf.push(b);
                i += 1;
            }
            String::from_utf8(buf).map_err(|e| {
                DirectiveError::BadAttrs(format!("invalid utf-8 in quoted value for `{key}`: {e}"))
            })?
        } else {
            let value_start = i;
            while i < n {
                let b = bytes[i];
                if b.is_ascii_whitespace() || b == b'=' || b == b'"' {
                    break;
                }
                i += 1;
            }
            std::str::from_utf8(&bytes[value_start..i])
                .map_err(|e| DirectiveError::BadAttrs(format!("invalid utf-8 in value: {e}")))?
                .to_string()
        };

        out.insert(key, value);
    }

    Ok(out)
}

/// Render an attribute map as a canonical string.
///
/// - Keys are emitted in alphabetical order (the `BTreeMap` ordering is
///   already alphabetical, but this is documented as part of the contract).
/// - Values are quoted only when they contain whitespace, `=`, `"`, or `\`.
/// - `"` and `\` inside quoted values are escaped with `\`.
/// - Pairs are separated by a single space.
///
/// # Examples
///
/// ```ignore
/// let mut m = BTreeMap::new();
/// m.insert("title".into(), "Heads up".into());
/// m.insert("mode".into(), "collapsed".into());
/// assert_eq!(render_attrs(&m), r#"mode=collapsed title="Heads up""#);
/// ```
#[must_use]
pub fn render_attrs(params: &BTreeMap<String, String>) -> String {
    let mut parts = Vec::with_capacity(params.len());
    for (k, v) in params {
        parts.push(format!("{k}={}", quote_value_if_needed(v)));
    }
    parts.join(" ")
}

fn quote_value_if_needed(v: &str) -> String {
    let needs_quote = v.is_empty()
        || v.chars()
            .any(|c| c.is_whitespace() || c == '=' || c == '"' || c == '\\');
    if !needs_quote {
        return v.to_string();
    }
    let mut buf = String::with_capacity(v.len() + 2);
    buf.push('"');
    for c in v.chars() {
        match c {
            '"' | '\\' => {
                buf.push('\\');
                buf.push(c);
            }
            other => buf.push(other),
        }
    }
    buf.push('"');
    buf
}

fn is_key_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_key_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}

fn is_name_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

// =====================================================================
// Block lexer
// =====================================================================

/// One event in the block-directive event stream.
///
/// The stream consists of `Line` events for non-directive content and
/// `Open` / `Close` events for matched `:::` fences. `Line` payloads do not
/// include a trailing newline; the caller is responsible for joining them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockEvent {
    /// A `:::name` fence opened a new directive. `depth` is the new stack
    /// depth (1 means the directive is at the top level).
    Open {
        /// Directive name (registered in [`SPECS`]).
        name: String,
        /// Parsed attributes from the opening fence.
        params: BTreeMap<String, String>,
        /// Stack depth after opening this directive (>= 1).
        depth: usize,
    },

    /// A `:::` fence closed the topmost open directive. `depth` is the stack
    /// depth *before* this close, so `depth == 1` means the outermost
    /// directive just closed.
    Close {
        /// Directive name that was opened (and is now closed).
        name: String,
        /// Stack depth before this close (>= 1).
        depth: usize,
    },

    /// A non-directive line, returned verbatim **without** the trailing
    /// newline. Indented `:::` fences and unrecognized directive names also
    /// pass through as `Line`.
    Line(String),
}

/// Stateful, line-by-line lexer that recognises block-level directives.
///
/// Feed lines (without their trailing `\n`) to [`Self::lex_line`] in order.
/// Call [`Self::finalize`] when the input is exhausted to detect unclosed
/// directives.
///
/// The lexer does **not** know about CommonMark fenced code blocks. If the
/// caller wants `:::info` inside a ` ``` ` fence to round-trip as plain text,
/// the caller must skip those lines (i.e. emit them as `Line` themselves)
/// rather than feeding them to [`Self::lex_line`].
#[derive(Debug, Default)]
pub struct BlockLexer {
    stack: Vec<String>,
}

impl BlockLexer {
    /// Build a fresh lexer with an empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// Current stack depth (number of currently-open directives).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Feed one logical line (without the trailing `\n`) and produce an
    /// event.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut lex = BlockLexer::new();
    /// matches!(lex.lex_line(":::info"), BlockEvent::Open { .. });
    /// matches!(lex.lex_line("hello"),  BlockEvent::Line(_));
    /// matches!(lex.lex_line(":::"),    BlockEvent::Close { .. });
    /// ```
    pub fn lex_line(&mut self, line: &str) -> BlockEvent {
        // Closing fence: a line whose trim equals `:::`.
        // The fence must start at column 0; we additionally require that
        // there is no leading whitespace before the `:::`.
        if !line.starts_with(':') {
            return BlockEvent::Line(line.to_string());
        }

        // The opening fence must be at column 0. An indented `:::info` is
        // a `Line`. (`starts_with(':')` already guarantees no leading space,
        // but we check explicitly for clarity.)
        if line.starts_with(' ') || line.starts_with('\t') {
            return BlockEvent::Line(line.to_string());
        }

        // Detect close fence: "::: " or ":::" with optional trailing
        // whitespace, but NOT followed by a directive name.
        if let Some(rest) = line.strip_prefix(":::") {
            // If `rest` starts with name-start char it's a (potential) open
            // fence. Otherwise (empty or whitespace), it's a close fence.
            if rest.is_empty() || rest.chars().all(char::is_whitespace) {
                if let Some(name) = self.stack.pop() {
                    let depth_before = self.stack.len() + 1;
                    return BlockEvent::Close {
                        name,
                        depth: depth_before,
                    };
                }
                // Stray close at depth 0 — pass through as a Line.
                return BlockEvent::Line(line.to_string());
            }

            // Open fence candidate. Parse name + optional attrs.
            // After `:::`, a space then attrs OR directly `name`.
            let after_fence = rest;
            // Walk the name. The first char must be a name-start; subsequent
            // chars must be name-cont. `end` is the byte index just past the
            // name.
            let mut end = 0;
            for (i, c) in after_fence.char_indices() {
                if i == 0 {
                    if !is_name_start(c) {
                        return BlockEvent::Line(line.to_string());
                    }
                    end = c.len_utf8();
                } else if is_name_cont(c) {
                    end = i + c.len_utf8();
                } else {
                    break;
                }
            }
            if end == 0 {
                return BlockEvent::Line(line.to_string());
            }
            let name = &after_fence[..end];
            let attrs_part = after_fence[end..].trim();

            // Unknown directive name OR a name registered as inline-only →
            // passthrough. The `:::name` syntax is reserved for block-scope
            // directives; an inline-only name (e.g. `mention`) appearing
            // between `:::` fences is treated as plain text so the inline
            // form remains the single canonical way to invoke it.
            if !matches!(lookup(name), Some(spec) if matches!(spec.kind, DirectiveKind::Block)) {
                return BlockEvent::Line(line.to_string());
            }

            let params = match parse_attrs(attrs_part) {
                Ok(p) => p,
                Err(_) => {
                    // Treat malformed attrs as a literal line so users can
                    // see the offending text in the output. Strict mode
                    // would error; the lexer stays permissive.
                    return BlockEvent::Line(line.to_string());
                }
            };

            self.stack.push(name.to_string());
            return BlockEvent::Open {
                name: name.to_string(),
                params,
                depth: self.stack.len(),
            };
        }

        BlockEvent::Line(line.to_string())
    }

    /// Consume the lexer; succeeds only if the stack is empty.
    pub fn finalize(self) -> Result<(), DirectiveError> {
        if let Some(name) = self.stack.last() {
            return Err(DirectiveError::Unclosed {
                name: name.clone(),
                depth: self.stack.len(),
            });
        }
        Ok(())
    }
}

/// Parse an entire markdown string into a flat block-event stream.
///
/// Newlines are split on `\n`. The final event sequence ends as soon as
/// the input is exhausted; if any directive is still open, [`Self::finalize`]
/// returns an [`DirectiveError::Unclosed`] error.
pub fn lex_blocks(md: &str) -> Result<Vec<BlockEvent>, DirectiveError> {
    let mut lex = BlockLexer::new();
    let mut events = Vec::new();
    for line in md.split('\n') {
        events.push(lex.lex_line(line));
    }
    lex.finalize()?;
    Ok(events)
}

// =====================================================================
// Inline parser
// =====================================================================

/// One token in the inline event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineToken {
    /// Plain text (anything that is not a recognised directive token).
    Text(String),
    /// A parsed `:name[content]{attrs}` token.
    Directive(InlineDirective),
}

/// A parsed inline directive token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineDirective {
    /// Directive name.
    pub name: String,
    /// Parsed attributes (may be empty).
    pub params: BTreeMap<String, String>,
    /// Body content from `[...]`. `None` if no body was present.
    pub content: Option<String>,
}

/// Parse an inline string into a sequence of text + directive tokens.
///
/// Unknown directive names (and false-positive `:` characters such as
/// `https://`) round-trip as [`InlineToken::Text`], preserving the original
/// substring verbatim.
///
/// # Examples
///
/// ```ignore
/// let tokens = parse_inline("hi :status[Done]{color=green} bye");
/// assert_eq!(tokens.len(), 3);
/// ```
pub fn parse_inline(text: &str) -> Vec<InlineToken> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut out: Vec<InlineToken> = Vec::new();
    let mut text_start = 0_usize;
    let mut i = 0_usize;

    let push_text = |out: &mut Vec<InlineToken>, s: &str| {
        if s.is_empty() {
            return;
        }
        if let Some(InlineToken::Text(prev)) = out.last_mut() {
            prev.push_str(s);
        } else {
            out.push(InlineToken::Text(s.to_string()));
        }
    };

    while i < n {
        if bytes[i] != b':' {
            i += 1;
            continue;
        }

        // The character before `:` must be either start-of-string or NOT
        // alphanumeric (so `https:` doesn't trigger). We look at the byte
        // because the names are ASCII; a multi-byte char that happens to end
        // in an ASCII alphanumeric byte is impossible for valid utf-8.
        if i > 0 {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() {
                i += 1;
                continue;
            }
        }

        // Try to parse a directive name after the colon.
        let after_colon = i + 1;
        if after_colon >= n {
            break;
        }
        let first = bytes[after_colon] as char;
        if !is_name_start(first) {
            i += 1;
            continue;
        }

        // Walk the name.
        let mut name_end = after_colon;
        while name_end < n && is_name_cont(bytes[name_end] as char) {
            name_end += 1;
        }
        let name = &text[after_colon..name_end];

        // Unknown name OR a block-only name? leave as text and advance past
        // `:`. The `:name[…]` syntax is reserved for inline-scope directives;
        // a block-only name (e.g. `info`) appearing inline is treated as
        // plain text so the block form remains the single canonical way to
        // invoke it.
        let spec = match lookup(name) {
            Some(s) if matches!(s.kind, DirectiveKind::Inline) => s,
            _ => {
                i += 1;
                continue;
            }
        };

        // Optional `[content]` — only consumed when the directive's spec
        // declares `allows_body == true`. Self-closing directives like
        // `:emoticon` and `:image` must not swallow a following `[…]`; the
        // bracketed text remains in the surrounding text stream.
        let mut cursor = name_end;
        let mut content: Option<String> = None;
        if spec.allows_body && cursor < n && bytes[cursor] == b'[' {
            // Find matching `]` (no nesting).
            let body_start = cursor + 1;
            let body_end = (body_start..n).find(|j| bytes[*j] == b']');
            match body_end {
                Some(end) => {
                    content = Some(text[body_start..end].to_string());
                    cursor = end + 1;
                }
                None => {
                    // Unbalanced `[` — leave as plain text.
                    i += 1;
                    continue;
                }
            }
        }

        // Optional `{attrs}`.
        let mut params = BTreeMap::new();
        if cursor < n && bytes[cursor] == b'{' {
            let attrs_start = cursor + 1;
            let attrs_end = (attrs_start..n).find(|j| bytes[*j] == b'}');
            match attrs_end {
                Some(end) => {
                    let attrs_text = &text[attrs_start..end];
                    match parse_attrs(attrs_text) {
                        Ok(p) => {
                            params = p;
                            cursor = end + 1;
                        }
                        Err(_) => {
                            // Malformed attrs — leave entire token as text.
                            i += 1;
                            continue;
                        }
                    }
                }
                None => {
                    i += 1;
                    continue;
                }
            }
        }

        // Flush any preceding text and emit the directive token.
        if text_start < i {
            push_text(&mut out, &text[text_start..i]);
        }
        out.push(InlineToken::Directive(InlineDirective {
            name: name.to_string(),
            params,
            content,
        }));
        text_start = cursor;
        i = cursor;
    }

    if text_start < n {
        push_text(&mut out, &text[text_start..]);
    }

    out
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_attrs ------------------------------------------------------

    #[test]
    fn parse_attrs_empty() {
        let m = parse_attrs("").unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn parse_attrs_single_unquoted() {
        let m = parse_attrs("key=value").unwrap();
        assert_eq!(m["key"], "value");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn parse_attrs_single_quoted_with_spaces() {
        let m = parse_attrs(r#"title="Heads up""#).unwrap();
        assert_eq!(m["title"], "Heads up");
    }

    #[test]
    fn parse_attrs_multiple() {
        let m = parse_attrs(r#"title="Heads up" mode=collapsed"#).unwrap();
        assert_eq!(m["title"], "Heads up");
        assert_eq!(m["mode"], "collapsed");
    }

    #[test]
    fn parse_attrs_escaped_quote() {
        let m = parse_attrs(r#"label="say \"hi\"""#).unwrap();
        assert_eq!(m["label"], r#"say "hi""#);
    }

    #[test]
    fn parse_attrs_escaped_backslash() {
        let m = parse_attrs(r#"path="C:\\foo""#).unwrap();
        assert_eq!(m["path"], r"C:\foo");
    }

    #[test]
    fn parse_attrs_unbalanced_quote_errors() {
        let err = parse_attrs(r#"title="oops"#).unwrap_err();
        assert!(matches!(err, DirectiveError::BadAttrs(_)));
    }

    #[test]
    fn parse_attrs_empty_value_accepted() {
        // Document the decision: `key=` is accepted as an empty string.
        let m = parse_attrs("key=").unwrap();
        assert_eq!(m["key"], "");
    }

    #[test]
    fn parse_attrs_bare_key_is_error() {
        // Document the decision: `key` alone (no `=`) is rejected.
        let err = parse_attrs("key").unwrap_err();
        assert!(matches!(err, DirectiveError::BadAttrs(_)));
    }

    #[test]
    fn parse_attrs_extra_whitespace() {
        let m = parse_attrs("   a=1    b=2   ").unwrap();
        assert_eq!(m["a"], "1");
        assert_eq!(m["b"], "2");
    }

    #[test]
    fn parse_attrs_invalid_key_start() {
        let err = parse_attrs("1key=val").unwrap_err();
        assert!(matches!(err, DirectiveError::BadAttrs(_)));
    }

    #[test]
    fn parse_attrs_key_with_dash_and_underscore() {
        let m = parse_attrs("a-b_c=1").unwrap();
        assert_eq!(m["a-b_c"], "1");
    }

    // ---- render_attrs -----------------------------------------------------

    #[test]
    fn render_attrs_empty() {
        assert_eq!(render_attrs(&BTreeMap::new()), "");
    }

    #[test]
    fn render_attrs_simple_unquoted() {
        let mut m = BTreeMap::new();
        m.insert("mode".to_string(), "collapsed".to_string());
        assert_eq!(render_attrs(&m), "mode=collapsed");
    }

    #[test]
    fn render_attrs_quotes_when_needed() {
        let mut m = BTreeMap::new();
        m.insert("title".to_string(), "Heads up".to_string());
        assert_eq!(render_attrs(&m), r#"title="Heads up""#);
    }

    #[test]
    fn render_attrs_sorted_keys() {
        let mut m = BTreeMap::new();
        m.insert("z".to_string(), "1".to_string());
        m.insert("a".to_string(), "2".to_string());
        m.insert("m".to_string(), "3".to_string());
        assert_eq!(render_attrs(&m), "a=2 m=3 z=1");
    }

    #[test]
    fn render_attrs_escapes_quote_and_backslash() {
        let mut m = BTreeMap::new();
        m.insert("v".to_string(), r#"a"b\c"#.to_string());
        assert_eq!(render_attrs(&m), r#"v="a\"b\\c""#);
    }

    #[test]
    fn render_attrs_quotes_empty_value() {
        let mut m = BTreeMap::new();
        m.insert("v".to_string(), String::new());
        assert_eq!(render_attrs(&m), r#"v="""#);
    }

    #[test]
    fn parse_render_roundtrip_simple() {
        let original = r#"a=1 b="two words" c=three"#;
        let parsed = parse_attrs(original).unwrap();
        let rendered = render_attrs(&parsed);
        let reparsed = parse_attrs(&rendered).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn parse_render_roundtrip_with_escapes() {
        let original = r#"k="a\"b\\c""#;
        let parsed = parse_attrs(original).unwrap();
        let rendered = render_attrs(&parsed);
        let reparsed = parse_attrs(&rendered).unwrap();
        assert_eq!(parsed, reparsed);
    }

    // ---- BlockLexer -------------------------------------------------------

    #[test]
    fn block_open_and_close() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line(":::info");
        assert!(matches!(
            e,
            BlockEvent::Open {
                ref name,
                depth: 1,
                ..
            } if name == "info"
        ));
        let e = lex.lex_line("hello");
        assert!(matches!(e, BlockEvent::Line(ref s) if s == "hello"));
        let e = lex.lex_line(":::");
        assert!(matches!(
            e,
            BlockEvent::Close {
                ref name,
                depth: 1,
            } if name == "info"
        ));
        lex.finalize().unwrap();
    }

    #[test]
    fn block_open_with_attrs() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line(r#":::warning title="Heads up""#);
        match e {
            BlockEvent::Open {
                name,
                params,
                depth,
            } => {
                assert_eq!(name, "warning");
                assert_eq!(depth, 1);
                assert_eq!(params["title"], "Heads up");
            }
            other => panic!("expected Open, got {other:?}"),
        }
    }

    #[test]
    fn block_nested_open_and_close() {
        let mut lex = BlockLexer::new();
        assert!(matches!(
            lex.lex_line(":::info"),
            BlockEvent::Open { depth: 1, .. }
        ));
        assert!(matches!(
            lex.lex_line(":::expand title=Detail"),
            BlockEvent::Open { depth: 2, .. }
        ));
        assert!(matches!(
            lex.lex_line(":::"),
            BlockEvent::Close {
                depth: 2,
                ref name,
            } if name == "expand"
        ));
        assert!(matches!(
            lex.lex_line(":::"),
            BlockEvent::Close {
                depth: 1,
                ref name,
            } if name == "info"
        ));
        lex.finalize().unwrap();
    }

    #[test]
    fn block_unclosed_directive_errors_on_finalize() {
        let mut lex = BlockLexer::new();
        let _ = lex.lex_line(":::info");
        let err = lex.finalize().unwrap_err();
        assert!(matches!(
            err,
            DirectiveError::Unclosed { ref name, depth: 1 } if name == "info"
        ));
    }

    #[test]
    fn block_stray_close_passthrough() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line(":::");
        assert!(matches!(e, BlockEvent::Line(ref s) if s == ":::"));
        // Finalize succeeds — nothing was opened.
        lex.finalize().unwrap();
    }

    #[test]
    fn block_unknown_directive_passthrough() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line(":::custom-thing");
        assert!(matches!(e, BlockEvent::Line(ref s) if s == ":::custom-thing"));
        // Stack must remain empty.
        assert_eq!(lex.depth(), 0);
        lex.finalize().unwrap();
    }

    #[test]
    fn block_indented_fence_is_line() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line("   :::info");
        assert!(matches!(e, BlockEvent::Line(ref s) if s == "   :::info"));
        assert_eq!(lex.depth(), 0);
        lex.finalize().unwrap();
    }

    #[test]
    fn block_tab_indented_fence_is_line() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line("\t:::info");
        assert!(matches!(e, BlockEvent::Line(_)));
        assert_eq!(lex.depth(), 0);
        lex.finalize().unwrap();
    }

    #[test]
    fn block_close_with_trailing_whitespace() {
        let mut lex = BlockLexer::new();
        let _ = lex.lex_line(":::info");
        let e = lex.lex_line("::: ");
        assert!(matches!(e, BlockEvent::Close { depth: 1, .. }));
        lex.finalize().unwrap();
    }

    #[test]
    fn block_open_no_attrs_no_trailing_whitespace() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line(":::toc");
        assert!(matches!(e, BlockEvent::Open { ref name, .. } if name == "toc"));
        // toc allows no body but the lexer doesn't enforce that — converters do.
    }

    #[test]
    fn block_open_with_multiple_attrs() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line(":::expand title=Detail mode=collapsed");
        match e {
            BlockEvent::Open { name, params, .. } => {
                assert_eq!(name, "expand");
                assert_eq!(params["title"], "Detail");
                assert_eq!(params["mode"], "collapsed");
            }
            other => panic!("expected Open, got {other:?}"),
        }
    }

    #[test]
    fn block_plain_line_passes_through() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line("just plain text");
        assert!(matches!(e, BlockEvent::Line(ref s) if s == "just plain text"));
    }

    #[test]
    fn block_line_with_colon_in_middle() {
        let mut lex = BlockLexer::new();
        let e = lex.lex_line("see also: foo");
        assert!(matches!(e, BlockEvent::Line(_)));
    }

    #[test]
    fn lexer_does_not_know_about_code_fences() {
        // The caller is responsible for skipping lines inside ``` fences.
        // Demonstrate that without that filter, a `:::info` inside a code
        // block IS parsed as a directive open.
        let md = "```\n:::info\nhi\n:::\n```";
        let events = lex_blocks(md).unwrap();
        let opened = events
            .iter()
            .any(|e| matches!(e, BlockEvent::Open { name, .. } if name == "info"));
        assert!(
            opened,
            "lexer is intentionally code-fence-unaware; converters must filter"
        );
    }

    // ---- lex_blocks -------------------------------------------------------

    #[test]
    fn lex_blocks_two_top_level_directives() {
        let md = ":::info\nhello\n:::\n\n:::warning\nworld\n:::";
        let events = lex_blocks(md).unwrap();
        let opens = events
            .iter()
            .filter(|e| matches!(e, BlockEvent::Open { .. }))
            .count();
        let closes = events
            .iter()
            .filter(|e| matches!(e, BlockEvent::Close { .. }))
            .count();
        assert_eq!(opens, 2);
        assert_eq!(closes, 2);
    }

    #[test]
    fn lex_blocks_nested() {
        let md = ":::info\n:::expand\nbody\n:::\n:::";
        let events = lex_blocks(md).unwrap();
        // Two opens, two closes.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, BlockEvent::Open { .. }))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, BlockEvent::Close { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn lex_blocks_unclosed_errors() {
        let md = ":::info\nbody";
        let err = lex_blocks(md).unwrap_err();
        assert!(matches!(err, DirectiveError::Unclosed { .. }));
    }

    // ---- parse_inline -----------------------------------------------------

    #[test]
    fn inline_directive_with_content_and_attrs() {
        let tokens = parse_inline(":status[Done]{color=green}");
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            InlineToken::Directive(d) => {
                assert_eq!(d.name, "status");
                assert_eq!(d.content.as_deref(), Some("Done"));
                assert_eq!(d.params["color"], "green");
            }
            other => panic!("expected Directive, got {other:?}"),
        }
    }

    #[test]
    fn inline_self_closing_with_attrs_only() {
        let tokens = parse_inline(":emoticon{name=warning}");
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            InlineToken::Directive(d) => {
                assert_eq!(d.name, "emoticon");
                assert!(d.content.is_none());
                assert_eq!(d.params["name"], "warning");
            }
            other => panic!("expected Directive, got {other:?}"),
        }
    }

    #[test]
    fn inline_with_content_only() {
        let tokens = parse_inline(":status[Done]");
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            InlineToken::Directive(d) => {
                assert_eq!(d.name, "status");
                assert_eq!(d.content.as_deref(), Some("Done"));
                assert!(d.params.is_empty());
            }
            other => panic!("expected Directive, got {other:?}"),
        }
    }

    #[test]
    fn inline_unknown_name_is_text() {
        let tokens = parse_inline(":notathing[foo]");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], InlineToken::Text(s) if s == ":notathing[foo]"));
    }

    #[test]
    fn inline_multiple_directives_in_one_string() {
        let tokens = parse_inline("hi :status[Done] and :emoticon{name=ok} bye");
        assert_eq!(tokens.len(), 5);
        assert!(matches!(&tokens[0], InlineToken::Text(s) if s == "hi "));
        assert!(matches!(&tokens[1], InlineToken::Directive(d) if d.name == "status"));
        assert!(matches!(&tokens[2], InlineToken::Text(s) if s == " and "));
        assert!(matches!(&tokens[3], InlineToken::Directive(d) if d.name == "emoticon"));
        assert!(matches!(&tokens[4], InlineToken::Text(s) if s == " bye"));
    }

    #[test]
    fn inline_directive_at_start() {
        let tokens = parse_inline(":status[Done] more");
        assert!(matches!(&tokens[0], InlineToken::Directive(d) if d.name == "status"));
        assert!(matches!(&tokens[1], InlineToken::Text(s) if s == " more"));
    }

    #[test]
    fn inline_directive_at_end() {
        let tokens = parse_inline("more :status[Done]");
        assert!(matches!(&tokens[0], InlineToken::Text(s) if s == "more "));
        assert!(matches!(&tokens[1], InlineToken::Directive(d) if d.name == "status"));
    }

    #[test]
    fn inline_https_does_not_trigger() {
        // The `:` after `https` is preceded by alphabetic char; do not parse.
        let tokens = parse_inline("see https://example.com today");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], InlineToken::Text(_)));
    }

    #[test]
    fn inline_text_colon_space_does_not_trigger() {
        // `:` not followed by name char (a space) does not parse.
        let tokens = parse_inline("text: foo");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], InlineToken::Text(_)));
    }

    #[test]
    fn inline_numeric_colons_do_not_trigger() {
        let tokens = parse_inline("1:2:3");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], InlineToken::Text(s) if s == "1:2:3"));
    }

    #[test]
    fn inline_unbalanced_brackets_passthrough() {
        let tokens = parse_inline(":status[unclosed");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], InlineToken::Text(_)));
    }

    #[test]
    fn inline_unbalanced_braces_passthrough() {
        let tokens = parse_inline(":status[Done]{color=green");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], InlineToken::Text(_)));
    }

    #[test]
    fn inline_image_self_closing_with_attrs() {
        let tokens = parse_inline(r#":image{src="a.png" alt="x"}"#);
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            InlineToken::Directive(d) => {
                assert_eq!(d.name, "image");
                assert_eq!(d.params["src"], "a.png");
                assert_eq!(d.params["alt"], "x");
                assert!(d.content.is_none());
            }
            other => panic!("expected Directive, got {other:?}"),
        }
    }

    #[test]
    fn inline_self_closing_does_not_swallow_brackets() {
        // Regression: `:emoticon` is a self-closing directive (allows_body =
        // false). It must NOT consume a following `[hi]` as content; that
        // bracketed text should remain in the surrounding text stream.
        let tokens = parse_inline(":emoticon[hi]{name=warning}");
        // Expected: Directive(:emoticon, no content, no params)
        // followed by Text("[hi]{name=warning}").
        assert!(
            tokens.len() >= 2,
            "expected at least 2 tokens, got {tokens:?}"
        );
        match &tokens[0] {
            InlineToken::Directive(d) => {
                assert_eq!(d.name, "emoticon");
                assert!(
                    d.content.is_none(),
                    "self-closing directive must not have a body"
                );
            }
            other => panic!("expected Directive at index 0, got {other:?}"),
        }
        // The remaining text `[hi]{name=warning}` should round-trip verbatim.
        let trailing: String = tokens[1..]
            .iter()
            .map(|t| match t {
                InlineToken::Text(s) => s.clone(),
                other => panic!("expected only Text after directive, got {other:?}"),
            })
            .collect();
        assert_eq!(trailing, "[hi]{name=warning}");
    }

    #[test]
    fn inline_status_with_body_still_parses() {
        // Regression check: `:status` has allows_body == true, so the body
        // branch must still work after the spec-aware fix.
        let tokens = parse_inline(":status[DONE]{color=green}");
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            InlineToken::Directive(d) => {
                assert_eq!(d.name, "status");
                assert_eq!(d.content.as_deref(), Some("DONE"));
                assert_eq!(d.params["color"], "green");
            }
            other => panic!("expected Directive, got {other:?}"),
        }
    }

    #[test]
    fn inline_only_text() {
        let tokens = parse_inline("just plain text");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], InlineToken::Text(s) if s == "just plain text"));
    }

    #[test]
    fn inline_empty_input() {
        let tokens = parse_inline("");
        assert!(tokens.is_empty());
    }

    // ---- lookup / spec table ----------------------------------------------

    #[test]
    fn lookup_info() {
        let s = lookup("info").unwrap();
        assert_eq!(s.name, "info");
        assert_eq!(s.kind, DirectiveKind::Block);
        assert!(s.allows_body);
        assert_eq!(s.conf_storage_macro, Some("info"));
        assert_eq!(s.conf_adf_node_type, Some("panel"));
        assert_eq!(s.conf_adf_panel_type, Some("info"));
        assert_eq!(s.jira_wiki_block, Some("info"));
        assert_eq!(s.jira_adf_panel_type, Some("info"));
    }

    #[test]
    fn lookup_warning() {
        let s = lookup("warning").unwrap();
        assert_eq!(s.conf_adf_panel_type, Some("warning"));
        assert_eq!(s.jira_wiki_block, Some("warning"));
    }

    #[test]
    fn lookup_note() {
        let s = lookup("note").unwrap();
        assert_eq!(s.conf_adf_panel_type, Some("note"));
    }

    #[test]
    fn lookup_tip_maps_to_success_panel() {
        // "tip" markdown directive must map to ADF panelType "success".
        let s = lookup("tip").unwrap();
        assert_eq!(s.conf_adf_panel_type, Some("success"));
        assert_eq!(s.jira_adf_panel_type, Some("success"));
        assert_eq!(s.jira_wiki_block, Some("tip"));
    }

    #[test]
    fn lookup_expand_no_jira_wiki() {
        let s = lookup("expand").unwrap();
        assert_eq!(s.conf_storage_macro, Some("expand"));
        assert_eq!(s.conf_adf_node_type, Some("expand"));
        assert!(s.jira_wiki_block.is_none());
    }

    #[test]
    fn lookup_toc_self_closing() {
        let s = lookup("toc").unwrap();
        assert!(!s.allows_body);
        assert_eq!(s.conf_storage_macro, Some("toc"));
        assert!(s.conf_adf_node_type.is_none());
        assert!(s.jira_wiki_block.is_none());
    }

    #[test]
    fn lookup_status_inline() {
        let s = lookup("status").unwrap();
        assert_eq!(s.kind, DirectiveKind::Inline);
        assert!(s.allows_body);
        assert_eq!(s.conf_storage_macro, Some("status"));
        assert_eq!(s.conf_adf_node_type, Some("status"));
    }

    #[test]
    fn lookup_emoticon_self_closing_inline() {
        let s = lookup("emoticon").unwrap();
        assert_eq!(s.kind, DirectiveKind::Inline);
        assert!(!s.allows_body);
        assert_eq!(s.conf_adf_node_type, Some("emoji"));
    }

    #[test]
    fn lookup_mention() {
        let s = lookup("mention").unwrap();
        assert_eq!(s.conf_adf_node_type, Some("mention"));
        assert_eq!(s.jira_adf_node_type, Some("mention"));
    }

    #[test]
    fn lookup_link() {
        let s = lookup("link").unwrap();
        assert_eq!(s.conf_adf_node_type, Some("inlineCard"));
    }

    #[test]
    fn lookup_image() {
        let s = lookup("image").unwrap();
        assert!(!s.allows_body);
        assert_eq!(s.conf_adf_node_type, Some("mediaSingle"));
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("nope").is_none());
    }

    #[test]
    fn block_specs_count() {
        let bs = block_specs();
        assert_eq!(bs.len(), 6);
        assert!(bs.iter().all(|s| matches!(s.kind, DirectiveKind::Block)));
    }

    #[test]
    fn inline_specs_count() {
        let is = inline_specs();
        assert_eq!(is.len(), 5);
        assert!(is.iter().all(|s| matches!(s.kind, DirectiveKind::Inline)));
    }

    // ---- UTF-8 in attribute values (regression) --------------------------

    #[test]
    fn parse_attrs_preserves_utf8_in_quoted_value() {
        let result = parse_attrs(r#"title="café""#).unwrap();
        assert_eq!(result.get("title"), Some(&"café".to_string()));
    }

    #[test]
    fn parse_attrs_preserves_utf8_with_escape() {
        let result = parse_attrs(r#"title="\"café\"""#).unwrap();
        assert_eq!(result.get("title"), Some(&"\"café\"".to_string()));
    }

    #[test]
    fn parse_attrs_preserves_utf8_in_unquoted_value() {
        // Unquoted values stop at whitespace, but multi-byte UTF-8 runs
        // (Cyrillic, emoji) should round-trip.
        let result = parse_attrs("name=привет").unwrap();
        assert_eq!(result.get("name"), Some(&"привет".to_string()));
    }

    #[test]
    fn parse_attrs_preserves_emoji_in_quoted_value() {
        let result = parse_attrs(r#"emoji="hi 🎉 there""#).unwrap();
        assert_eq!(result.get("emoji"), Some(&"hi 🎉 there".to_string()));
    }

    // ---- DirectiveKind enforcement (regression) --------------------------

    #[test]
    fn block_lexer_rejects_inline_name_as_block_directive() {
        // `:::mention` — `mention` is an Inline-only directive; block fence
        // should fall through as Line.
        let mut lex = BlockLexer::new();
        let event = lex.lex_line(":::mention");
        assert!(matches!(event, BlockEvent::Line(_)), "got {event:?}");
    }

    #[test]
    fn inline_parser_rejects_block_name_as_inline_directive() {
        // `:info[x]` — `info` is a Block-only directive; inline syntax
        // should pass through as Text.
        let tokens = parse_inline(":info[content]");
        assert!(
            matches!(tokens.as_slice(), [InlineToken::Text(s)] if s == ":info[content]"),
            "got {tokens:?}"
        );
    }

    #[test]
    fn block_lexer_accepts_block_name() {
        // `:::info` — `info` is Block; should open a directive.
        let mut lex = BlockLexer::new();
        let event = lex.lex_line(":::info");
        assert!(
            matches!(event, BlockEvent::Open { ref name, .. } if name == "info"),
            "got {event:?}"
        );
    }

    #[test]
    fn inline_parser_accepts_inline_name() {
        // `:status[DONE]` — `status` is Inline; should produce a Directive
        // token.
        let tokens = parse_inline(":status[DONE]");
        assert!(
            matches!(tokens.first(), Some(InlineToken::Directive(d)) if d.name == "status"),
            "got {tokens:?}"
        );
    }

    #[test]
    fn block_lexer_rejects_all_inline_only_names() {
        // Sweep every Inline-only spec and confirm `:::name` falls through.
        for spec in inline_specs() {
            let mut lex = BlockLexer::new();
            let line = format!(":::{}", spec.name);
            let event = lex.lex_line(&line);
            assert!(
                matches!(event, BlockEvent::Line(ref s) if s == &line),
                "inline-only `{}` must not open as block, got {event:?}",
                spec.name
            );
            assert_eq!(
                lex.depth(),
                0,
                "inline-only `{}` must not be pushed onto block stack",
                spec.name
            );
        }
    }

    #[test]
    fn inline_parser_rejects_all_block_only_names() {
        // Sweep every Block-only spec and confirm `:name[…]` falls through
        // as text.
        for spec in block_specs() {
            let input = format!(":{}[body]", spec.name);
            let tokens = parse_inline(&input);
            assert!(
                matches!(tokens.as_slice(), [InlineToken::Text(s)] if s == &input),
                "block-only `{}` must not parse inline, got {tokens:?}",
                spec.name
            );
        }
    }
}
