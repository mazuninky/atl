//! Confluence storage XHTML → markdown (with MyST-style directive extensions).
//!
//! # Conversion strategy
//!
//! The converter runs in three stages:
//!
//! 1. **Parse to a tree.** The input is wrapped in a synthetic `<root>` element
//!    so that bare top-level fragments parse cleanly, then [`quick_xml`]'s
//!    event-streaming reader is folded into a small [`XNode`] tree of
//!    `Element` / `Text` / `Cdata` / `Comment` nodes.
//!
//! 2. **Recursive emit.** Each [`XNode`] is rendered to a [`String`] buffer
//!    according to a small element-to-markdown table. Block elements (`<p>`,
//!    `<h1>`..`<h6>`, `<ul>`, `<ol>`, `<table>`, `<blockquote>`, `<pre>`,
//!    `<hr/>`) emit standard markdown; inline elements (`<strong>`, `<em>`,
//!    `<s>`, `<code>`, `<a>`, `<img/>`, `<br/>`) emit their inline markdown
//!    forms.
//!
//! 3. **Confluence-specific elements** in the `ac:` and `ri:` namespaces are
//!    matched against the directive registry from
//!    [`crate::cli::commands::directives`] and emitted as MyST-style
//!    directives (`:::info … :::` blocks, `:status[…]{color=…}` inline,
//!    `:emoticon{name=…}`, `:mention[]{accountId=…}`, `:link[…]{pageId=…}`,
//!    `:image{src=…}`). Unknown macros and unknown XHTML elements pass
//!    through as raw HTML so the round-trip is lossless.
//!
//! Special inverse mappings: `colour` → `color` for the `status` macro
//! (Confluence uses British spelling), and the `title` parameter of `status`
//! becomes the directive's `[content]` slot.
//!
//! Set [`ConvertOpts::render_directives`] to `false` to strip macros and
//! emit just their body text — useful for "clean text" extraction modes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use thiserror::Error;

use super::code_fence::pick_code_fence;
use crate::cli::commands::directives::{DirectiveKind, lookup, render_attrs};

// =====================================================================
// Public API
// =====================================================================

/// Errors returned by [`storage_to_markdown`].
#[derive(Debug, Error)]
pub enum StorageToMdError {
    /// The input was not well-formed XML / XHTML and could not be parsed.
    #[error("malformed XML: {0}")]
    Xml(String),
}

/// Conversion options for [`storage_to_markdown`].
///
/// `render_directives` controls whether Confluence macros become MyST-style
/// directives (`true`, the default) or are stripped to their plain body text
/// (`false`).
#[derive(Debug, Clone, Copy)]
pub struct ConvertOpts {
    /// When `true` (the default), recognised macros are converted to
    /// directive syntax (`:::info`, `:status[…]`, etc.). When `false`,
    /// directives are stripped: block macros emit only their body content,
    /// inline self-closing macros (`emoticon`, `image`) are dropped, and
    /// `mention` / `link` collapse to their display text.
    pub render_directives: bool,
}

impl Default for ConvertOpts {
    fn default() -> Self {
        Self {
            render_directives: true,
        }
    }
}

/// Convert Confluence storage-format XHTML to markdown with MyST-style
/// directives.
///
/// Macros (`<ac:structured-macro>`, `<ac:emoticon>`, `<ac:link>`, `<ac:image>`)
/// are converted to fenced/inline directives where the directive registry has
/// a mapping; unknown macros and unknown XHTML elements pass through as raw
/// HTML so the round-trip is lossless.
///
/// Returns an error only if the input is not well-formed XML.
///
/// # Examples
///
/// ```ignore
/// use atl::cli::commands::converters::storage_to_md::{
///     storage_to_markdown, ConvertOpts,
/// };
///
/// let md = storage_to_markdown(
///     r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>Hi</p></ac:rich-text-body></ac:structured-macro>"#,
///     ConvertOpts::default(),
/// )
/// .unwrap();
/// assert!(md.contains(":::info"));
/// assert!(md.contains("Hi"));
/// ```
pub fn storage_to_markdown(xhtml: &str, opts: ConvertOpts) -> Result<String, StorageToMdError> {
    // Confluence Cloud serializes content with HTML named entities like
    // `&mdash;`, `&nbsp;`, `&hellip;`, etc. quick-xml only knows the five
    // XML predefined entities (`&lt;` `&gt;` `&amp;` `&quot;` `&apos;`) and
    // rejects everything else as `unrecognized entity`. We pre-pass the
    // input and rewrite known HTML named entities to their literal Unicode
    // characters before parsing. CDATA sections are preserved verbatim
    // because entities inside them are literal text, not entities.
    let preprocessed = replace_html_entities(xhtml);
    let nodes = parse(&preprocessed)?;
    let mut ctx = Context {
        opts,
        list_depth: 0,
    };
    let mut out = String::new();
    emit_nodes(&nodes, &mut out, &mut ctx);
    Ok(normalize_blank_lines(&out))
}

/// Table of HTML named entities Confluence is known to emit, mapped to their
/// literal Unicode characters. The five XML-predefined entities (`amp`, `lt`,
/// `gt`, `quot`, `apos`) are intentionally **omitted** — quick-xml decodes
/// those itself and we must not double-decode `&amp;`.
///
/// Numeric entities (`&#65;` and `&#x41;`) are also handled by quick-xml and
/// are skipped in the pre-pass.
///
/// This list covers the common HTML5 named character references that
/// Confluence Cloud serializes (em/en dashes, smart quotes, accented Latin
/// letters, common symbols). It is not exhaustive — if a user hits a missing
/// entity the parser will surface the original `unrecognized entity` error
/// and a new entry can be added here.
static HTML_ENTITIES: &[(&str, &str)] = &[
    // Whitespace and punctuation
    ("nbsp", "\u{00A0}"),
    ("ensp", "\u{2002}"),
    ("emsp", "\u{2003}"),
    ("thinsp", "\u{2009}"),
    ("zwnj", "\u{200C}"),
    ("zwj", "\u{200D}"),
    ("shy", "\u{00AD}"),
    // Dashes and ellipsis
    ("mdash", "\u{2014}"),
    ("ndash", "\u{2013}"),
    ("hellip", "\u{2026}"),
    ("bull", "\u{2022}"),
    ("middot", "\u{00B7}"),
    // Quotes
    ("lsquo", "\u{2018}"),
    ("rsquo", "\u{2019}"),
    ("ldquo", "\u{201C}"),
    ("rdquo", "\u{201D}"),
    ("sbquo", "\u{201A}"),
    ("bdquo", "\u{201E}"),
    ("laquo", "\u{00AB}"),
    ("raquo", "\u{00BB}"),
    ("lsaquo", "\u{2039}"),
    ("rsaquo", "\u{203A}"),
    ("prime", "\u{2032}"),
    ("Prime", "\u{2033}"),
    // Symbols
    ("copy", "\u{00A9}"),
    ("reg", "\u{00AE}"),
    ("trade", "\u{2122}"),
    ("times", "\u{00D7}"),
    ("divide", "\u{00F7}"),
    ("dagger", "\u{2020}"),
    ("Dagger", "\u{2021}"),
    ("permil", "\u{2030}"),
    ("oline", "\u{203E}"),
    ("sect", "\u{00A7}"),
    ("para", "\u{00B6}"),
    ("deg", "\u{00B0}"),
    ("plusmn", "\u{00B1}"),
    ("micro", "\u{00B5}"),
    ("not", "\u{00AC}"),
    ("iquest", "\u{00BF}"),
    ("iexcl", "\u{00A1}"),
    ("cent", "\u{00A2}"),
    ("pound", "\u{00A3}"),
    ("yen", "\u{00A5}"),
    ("euro", "\u{20AC}"),
    ("curren", "\u{00A4}"),
    ("brvbar", "\u{00A6}"),
    ("uml", "\u{00A8}"),
    ("ordf", "\u{00AA}"),
    ("ordm", "\u{00BA}"),
    ("acute", "\u{00B4}"),
    ("cedil", "\u{00B8}"),
    ("macr", "\u{00AF}"),
    // Superscripts and fractions
    ("sup1", "\u{00B9}"),
    ("sup2", "\u{00B2}"),
    ("sup3", "\u{00B3}"),
    ("frac12", "\u{00BD}"),
    ("frac14", "\u{00BC}"),
    ("frac34", "\u{00BE}"),
    // Math
    ("infin", "\u{221E}"),
    ("ne", "\u{2260}"),
    ("le", "\u{2264}"),
    ("ge", "\u{2265}"),
    ("plus", "\u{002B}"),
    ("minus", "\u{2212}"),
    ("sum", "\u{2211}"),
    ("prod", "\u{220F}"),
    ("radic", "\u{221A}"),
    ("part", "\u{2202}"),
    ("int", "\u{222B}"),
    // Arrows
    ("larr", "\u{2190}"),
    ("rarr", "\u{2192}"),
    ("uarr", "\u{2191}"),
    ("darr", "\u{2193}"),
    ("harr", "\u{2194}"),
    ("lArr", "\u{21D0}"),
    ("rArr", "\u{21D2}"),
    ("uArr", "\u{21D1}"),
    ("dArr", "\u{21D3}"),
    ("hArr", "\u{21D4}"),
    // Latin lowercase accented
    ("agrave", "\u{00E0}"),
    ("aacute", "\u{00E1}"),
    ("acirc", "\u{00E2}"),
    ("atilde", "\u{00E3}"),
    ("auml", "\u{00E4}"),
    ("aring", "\u{00E5}"),
    ("aelig", "\u{00E6}"),
    ("ccedil", "\u{00E7}"),
    ("egrave", "\u{00E8}"),
    ("eacute", "\u{00E9}"),
    ("ecirc", "\u{00EA}"),
    ("euml", "\u{00EB}"),
    ("igrave", "\u{00EC}"),
    ("iacute", "\u{00ED}"),
    ("icirc", "\u{00EE}"),
    ("iuml", "\u{00EF}"),
    ("ntilde", "\u{00F1}"),
    ("ograve", "\u{00F2}"),
    ("oacute", "\u{00F3}"),
    ("ocirc", "\u{00F4}"),
    ("otilde", "\u{00F5}"),
    ("ouml", "\u{00F6}"),
    ("oslash", "\u{00F8}"),
    ("ugrave", "\u{00F9}"),
    ("uacute", "\u{00FA}"),
    ("ucirc", "\u{00FB}"),
    ("uuml", "\u{00FC}"),
    ("yacute", "\u{00FD}"),
    ("yuml", "\u{00FF}"),
    ("szlig", "\u{00DF}"),
    ("eth", "\u{00F0}"),
    ("thorn", "\u{00FE}"),
    // Latin uppercase accented
    ("Agrave", "\u{00C0}"),
    ("Aacute", "\u{00C1}"),
    ("Acirc", "\u{00C2}"),
    ("Atilde", "\u{00C3}"),
    ("Auml", "\u{00C4}"),
    ("Aring", "\u{00C5}"),
    ("AElig", "\u{00C6}"),
    ("Ccedil", "\u{00C7}"),
    ("Egrave", "\u{00C8}"),
    ("Eacute", "\u{00C9}"),
    ("Ecirc", "\u{00CA}"),
    ("Euml", "\u{00CB}"),
    ("Igrave", "\u{00CC}"),
    ("Iacute", "\u{00CD}"),
    ("Icirc", "\u{00CE}"),
    ("Iuml", "\u{00CF}"),
    ("Ntilde", "\u{00D1}"),
    ("Ograve", "\u{00D2}"),
    ("Oacute", "\u{00D3}"),
    ("Ocirc", "\u{00D4}"),
    ("Otilde", "\u{00D5}"),
    ("Ouml", "\u{00D6}"),
    ("Oslash", "\u{00D8}"),
    ("Ugrave", "\u{00D9}"),
    ("Uacute", "\u{00DA}"),
    ("Ucirc", "\u{00DB}"),
    ("Uuml", "\u{00DC}"),
    ("Yacute", "\u{00DD}"),
    ("ETH", "\u{00D0}"),
    ("THORN", "\u{00DE}"),
    // Greek (common)
    ("alpha", "\u{03B1}"),
    ("beta", "\u{03B2}"),
    ("gamma", "\u{03B3}"),
    ("delta", "\u{03B4}"),
    ("epsilon", "\u{03B5}"),
    ("pi", "\u{03C0}"),
    ("sigma", "\u{03C3}"),
    ("Alpha", "\u{0391}"),
    ("Beta", "\u{0392}"),
    ("Gamma", "\u{0393}"),
    ("Delta", "\u{0394}"),
    ("Pi", "\u{03A0}"),
    ("Sigma", "\u{03A3}"),
    ("Omega", "\u{03A9}"),
];

/// Replace HTML named entities (e.g. `&mdash;`, `&nbsp;`) with their literal
/// Unicode characters so that [`quick_xml::Reader`] — which only knows the
/// five XML predefined entities — does not error on `unrecognized entity`.
///
/// CDATA sections (`<![CDATA[ … ]]>`) are passed through unchanged because
/// the bytes inside them are literal text, not entities.
///
/// XML predefined entities (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`) are
/// left alone for quick-xml to decode — replacing `&amp;` here would let a
/// subsequent `&mdash;` from the same input be wrongly decoded twice.
///
/// Numeric entities (`&#65;`, `&#x41;`) are also passed through; quick-xml
/// handles those natively.
///
/// Unknown named entities (anything not in [`HTML_ENTITIES`]) are passed
/// through unchanged so that legitimate `&someUnknownThing;` text is not
/// corrupted, and so that a future Confluence-emitted entity not yet in the
/// table still surfaces as the original "unrecognized entity" XML error
/// rather than silently disappearing.
fn replace_html_entities(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        // Detect CDATA start — copy verbatim through `]]>`.
        if bytes[i..].starts_with(b"<![CDATA[") {
            // Find the closing `]]>`. If absent (malformed input), copy the
            // rest of the buffer and let quick-xml report the parse error.
            let start = i;
            let mut end = start + b"<![CDATA[".len();
            let mut closed = false;
            while end + 2 < bytes.len() {
                if &bytes[end..end + 3] == b"]]>" {
                    end += 3;
                    closed = true;
                    break;
                }
                end += 1;
            }
            if !closed {
                end = bytes.len();
            }
            // Safe: CDATA content is bytes from the original UTF-8 input,
            // sliced on byte boundaries that don't split codepoints (the
            // CDATA delimiters are pure ASCII).
            out.push_str(std::str::from_utf8(&bytes[start..end]).unwrap_or(""));
            i = end;
            continue;
        }

        if bytes[i] != b'&' {
            // Push the (possibly multi-byte) char.
            let ch_len = utf8_char_len(bytes[i]);
            let end = (i + ch_len).min(bytes.len());
            if let Ok(s) = std::str::from_utf8(&bytes[i..end]) {
                out.push_str(s);
            }
            i = end;
            continue;
        }

        // We're at `&`. Look ahead for a `;` within a reasonable bound to
        // identify the entity name. HTML5 entity names top out around 30
        // characters; cap the search to avoid pathological scans.
        let mut j = i + 1;
        let max = (i + 1 + 32).min(bytes.len());
        while j < max && bytes[j] != b';' {
            j += 1;
        }
        if j >= max || bytes[j] != b';' {
            // No terminator — emit the `&` and continue.
            out.push('&');
            i += 1;
            continue;
        }

        let name = &bytes[i + 1..j];

        // Skip XML-predefined entities — quick-xml handles these. Crucially,
        // we must NOT decode `&amp;` here, otherwise a subsequent `&mdash;`
        // in the same input could be decoded twice (`&amp;mdash;` should
        // round-trip as the literal text `&mdash;`, not as an em-dash).
        if matches!(name, b"amp" | b"lt" | b"gt" | b"quot" | b"apos") {
            out.push('&');
            // Safe: ASCII bytes only.
            out.push_str(std::str::from_utf8(name).unwrap_or(""));
            out.push(';');
            i = j + 1;
            continue;
        }

        // Skip numeric entities — quick-xml handles `&#NN;` and `&#xNN;`.
        if name.first() == Some(&b'#') {
            out.push('&');
            out.push_str(std::str::from_utf8(name).unwrap_or(""));
            out.push(';');
            i = j + 1;
            continue;
        }

        // Look up the named entity in the table. The table is small (~150
        // entries) so a linear scan is fine.
        let mut replaced = false;
        if let Ok(name_str) = std::str::from_utf8(name) {
            for (key, value) in HTML_ENTITIES {
                if *key == name_str {
                    out.push_str(value);
                    replaced = true;
                    break;
                }
            }
        }
        if replaced {
            i = j + 1;
        } else {
            // Unknown entity — pass through so we don't corrupt legitimate
            // `&unknownThing;` text. quick-xml will surface this as
            // `unrecognized entity` if it really is meant to be an entity.
            out.push('&');
            out.push_str(std::str::from_utf8(name).unwrap_or(""));
            out.push(';');
            i = j + 1;
        }
    }
    out
}

// =====================================================================
// Stage 1: tree construction
// =====================================================================

/// One node in the parsed input tree.
#[derive(Debug)]
enum XNode {
    /// Text content (already entity-decoded).
    Text(String),
    /// CDATA section content (verbatim).
    Cdata(String),
    /// XML element with children.
    Element {
        /// Qualified name as it appears in the source, e.g. `"p"`,
        /// `"ac:structured-macro"`, `"ri:user"`.
        name: String,
        /// Attributes by qualified name.
        attrs: BTreeMap<String, String>,
        /// Child nodes.
        children: Vec<XNode>,
        /// True if the element was self-closing (`<br/>`).
        self_closing: bool,
    },
    /// Comment content (without the `<!--` / `-->` delimiters).
    Comment(String),
}

fn parse(xhtml: &str) -> Result<Vec<XNode>, StorageToMdError> {
    // Wrap in a synthetic root so a sequence of top-level fragments parses
    // cleanly without requiring the input to have a single root element.
    let wrapped = format!("<atl_root>{xhtml}</atl_root>");
    let mut reader = Reader::from_str(&wrapped);
    let cfg = reader.config_mut();
    cfg.trim_text(false);
    cfg.expand_empty_elements = false;
    // `check_end_names = true` (the quick-xml default) enforces matched
    // start/end tag names. The module's documented contract is "well-formed
    // XML required", so a mismatched close like `<p></div>` should surface as
    // `Error::Xml`, not silently succeed.

    // Stack of (element-name, attrs, accumulated-children) for currently-open
    // elements. The synthetic `<atl_root>` is the first frame pushed when its
    // Start event is seen, and is stripped on return.
    let mut stack: Vec<(String, BTreeMap<String, String>, Vec<XNode>)> = Vec::new();
    let mut root_children: Option<Vec<XNode>> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let (name, attrs) = element_meta(e)?;
                stack.push((name, attrs, Vec::new()));
            }
            Ok(Event::Empty(ref e)) => {
                let (name, attrs) = element_meta(e)?;
                let node = XNode::Element {
                    name,
                    attrs,
                    children: Vec::new(),
                    self_closing: true,
                };
                if let Some(top) = stack.last_mut() {
                    top.2.push(node);
                }
            }
            Ok(Event::End(_)) => {
                let Some((name, attrs, children)) = stack.pop() else {
                    // Stray close — ignore.
                    continue;
                };
                if name == "atl_root" {
                    // Strip the synthetic root; children become the result.
                    root_children = Some(children);
                    continue;
                }
                let node = XNode::Element {
                    name,
                    attrs,
                    children,
                    self_closing: false,
                };
                if let Some(top) = stack.last_mut() {
                    top.2.push(node);
                }
            }
            Ok(Event::Text(t)) => {
                let s = t
                    .unescape()
                    .map_err(|e| StorageToMdError::Xml(e.to_string()))?
                    .into_owned();
                if !s.is_empty()
                    && let Some(top) = stack.last_mut()
                {
                    top.2.push(XNode::Text(s));
                }
            }
            Ok(Event::CData(c)) => {
                let bytes = c.into_inner();
                let s = std::str::from_utf8(&bytes)
                    .map_err(|e| StorageToMdError::Xml(e.to_string()))?
                    .to_string();
                if let Some(top) = stack.last_mut() {
                    top.2.push(XNode::Cdata(s));
                }
            }
            Ok(Event::Comment(c)) => {
                let bytes = c.into_inner();
                let s = std::str::from_utf8(&bytes)
                    .map_err(|e| StorageToMdError::Xml(e.to_string()))?
                    .to_string();
                if let Some(top) = stack.last_mut() {
                    top.2.push(XNode::Comment(s));
                }
            }
            Ok(Event::Eof) => break,
            // Decl, PI, DocType — drop silently; these are unusual in
            // Confluence storage XML and have no markdown equivalent.
            Ok(_) => {}
            Err(e) => return Err(StorageToMdError::Xml(e.to_string())),
        }
    }

    if !stack.is_empty() {
        return Err(StorageToMdError::Xml(format!(
            "unclosed element `{}`",
            stack.last().map(|f| f.0.as_str()).unwrap_or("?"),
        )));
    }

    Ok(root_children.unwrap_or_default())
}

fn element_meta(
    e: &BytesStart<'_>,
) -> Result<(String, BTreeMap<String, String>), StorageToMdError> {
    let name_bytes = e.name().into_inner().to_vec();
    let name =
        String::from_utf8(name_bytes).map_err(|err| StorageToMdError::Xml(err.to_string()))?;

    let mut attrs = BTreeMap::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|err| StorageToMdError::Xml(err.to_string()))?;
        let key_bytes = attr.key.into_inner().to_vec();
        let key =
            String::from_utf8(key_bytes).map_err(|err| StorageToMdError::Xml(err.to_string()))?;
        let value = attr
            .unescape_value()
            .map_err(|err| StorageToMdError::Xml(err.to_string()))?
            .into_owned();
        attrs.insert(key, value);
    }
    Ok((name, attrs))
}

// =====================================================================
// Stage 2: render
// =====================================================================

/// Mutable state carried while emitting markdown.
struct Context {
    opts: ConvertOpts,
    /// Nesting depth of the currently-active list (`0` = not in a list).
    list_depth: usize,
}

fn emit_nodes(nodes: &[XNode], out: &mut String, ctx: &mut Context) {
    for n in nodes {
        emit_node(n, out, ctx);
    }
}

fn emit_node(node: &XNode, out: &mut String, ctx: &mut Context) {
    match node {
        XNode::Text(t) => out.push_str(&escape_text(t)),
        XNode::Cdata(c) => out.push_str(c),
        XNode::Comment(c) => {
            out.push_str("<!--");
            out.push_str(c);
            out.push_str("-->");
        }
        XNode::Element {
            name,
            attrs,
            children,
            self_closing,
        } => emit_element(name, attrs, children, *self_closing, out, ctx),
    }
}

fn emit_element(
    name: &str,
    attrs: &BTreeMap<String, String>,
    children: &[XNode],
    self_closing: bool,
    out: &mut String,
    ctx: &mut Context,
) {
    match name {
        // ---- block ----
        "p" => {
            let inner = render_inline(children, ctx);
            push_block(out, &inner);
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level: usize = name[1..].parse().unwrap_or(1);
            let inner = render_inline(children, ctx);
            ensure_blank_line(out);
            for _ in 0..level {
                out.push('#');
            }
            out.push(' ');
            out.push_str(&inner);
            out.push_str("\n\n");
        }
        "hr" => {
            ensure_blank_line(out);
            out.push_str("---\n\n");
        }
        "br" => {
            out.push_str("  \n");
        }
        "blockquote" => {
            let mut inner = String::new();
            emit_nodes(children, &mut inner, ctx);
            ensure_blank_line(out);
            for line in inner.trim_end_matches('\n').split('\n') {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
        "ul" | "ol" => {
            let ordered = name == "ol";
            ensure_blank_line(out);
            ctx.list_depth += 1;
            for child in children {
                if let XNode::Element {
                    name: cname,
                    children: cchildren,
                    ..
                } = child
                    && cname == "li"
                {
                    emit_list_item(cchildren, ordered, ctx, out);
                }
            }
            ctx.list_depth -= 1;
            if ctx.list_depth == 0 {
                out.push('\n');
            }
        }
        "li" => {
            // Top-level <li> outside of <ul>/<ol> — just emit content.
            let inner = render_inline(children, ctx);
            out.push_str(&inner);
        }
        "pre" => {
            // Look for a single <code> child; if present, use its language.
            let (lang, body) = extract_code_block(children);
            // Pick a fence long enough to safely wrap any backtick run in the
            // body — CommonMark §4.5 closes the block on the first run of >=
            // fence-length backticks, so plain ``` would be unsafe when the
            // body contains triple-or-more backticks.
            let fence = pick_code_fence(&body);
            ensure_blank_line(out);
            out.push_str(&fence);
            out.push_str(&lang);
            out.push('\n');
            out.push_str(&body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&fence);
            out.push_str("\n\n");
        }
        "code" => {
            // Inline code (when not inside <pre>).
            let body = collect_text(children);
            out.push('`');
            out.push_str(&body);
            out.push('`');
        }
        // ---- inline ----
        "strong" | "b" => {
            out.push_str("**");
            out.push_str(&render_inline(children, ctx));
            out.push_str("**");
        }
        "em" | "i" => {
            out.push('*');
            out.push_str(&render_inline(children, ctx));
            out.push('*');
        }
        "s" | "del" | "strike" => {
            out.push_str("~~");
            out.push_str(&render_inline(children, ctx));
            out.push_str("~~");
        }
        "a" => {
            let href = attrs.get("href").cloned().unwrap_or_default();
            let text = render_inline(children, ctx);
            if href.is_empty() {
                out.push_str(&text);
            } else {
                out.push('[');
                out.push_str(&text);
                out.push_str("](");
                out.push_str(&href);
                out.push(')');
            }
        }
        "img" => {
            let src = attrs.get("src").cloned().unwrap_or_default();
            let alt = attrs.get("alt").cloned().unwrap_or_default();
            out.push_str("![");
            out.push_str(&alt);
            out.push_str("](");
            out.push_str(&src);
            out.push(')');
        }
        // ---- tables ----
        "table" => {
            ensure_blank_line(out);
            emit_table(children, out, ctx);
            out.push('\n');
        }
        "tr" | "th" | "td" | "thead" | "tbody" | "tfoot" => {
            // These should only be reached via emit_table; if encountered in
            // isolation, render their inline content.
            out.push_str(&render_inline(children, ctx));
        }
        // ---- Confluence ----
        "ac:structured-macro" => emit_structured_macro(attrs, children, out, ctx),
        "ac:emoticon" => emit_emoticon(attrs, out, ctx),
        "ac:link" => emit_link(children, out, ctx),
        "ac:image" => emit_image(attrs, children, out, ctx),
        "ac:plain-text-body" | "ac:rich-text-body" | "ac:parameter" | "ac:link-body" => {
            // These are normally handled by their parent macro's emitter.
            // If we hit them at top level, just emit children.
            emit_nodes(children, out, ctx);
        }
        // ---- fallback: raw HTML passthrough ----
        _ => {
            emit_raw_passthrough(name, attrs, children, self_closing, out);
        }
    }
}

/// Extract `(language, body)` from the children of a `<pre>` element. If the
/// children are a single `<code class="language-X">…</code>`, the language is
/// `X` and the body is the code text. Otherwise the language is empty and
/// the body is the concatenated text content.
fn extract_code_block(children: &[XNode]) -> (String, String) {
    // Find the first <code> child (allow whitespace text nodes around it).
    for child in children {
        if let XNode::Element {
            name,
            attrs,
            children: code_kids,
            ..
        } = child
            && name == "code"
        {
            let mut lang = String::new();
            if let Some(class) = attrs.get("class") {
                for token in class.split_ascii_whitespace() {
                    if let Some(rest) = token.strip_prefix("language-") {
                        lang = rest.to_string();
                        break;
                    }
                }
            }
            let body = collect_text(code_kids);
            return (lang, body);
        }
    }
    (String::new(), collect_text(children))
}

/// Concatenate plain text from a node tree (used for code blocks where we
/// want raw, unescaped content).
fn collect_text(nodes: &[XNode]) -> String {
    let mut buf = String::new();
    walk_text(nodes, &mut buf);
    buf
}

fn walk_text(nodes: &[XNode], out: &mut String) {
    for n in nodes {
        match n {
            XNode::Text(t) => out.push_str(t),
            XNode::Cdata(c) => out.push_str(c),
            XNode::Element { children, .. } => walk_text(children, out),
            XNode::Comment(_) => {}
        }
    }
}

/// Emit a single `<li>` payload to `out`. Children are inline-rendered; if
/// the list item contains nested lists, those are emitted with deeper
/// indent.
fn emit_list_item(children: &[XNode], ordered: bool, ctx: &mut Context, out: &mut String) {
    let indent_units = ctx.list_depth.saturating_sub(1);
    let indent = "  ".repeat(indent_units);

    // Split children into inline content and nested-list content.
    let mut inline_chunks: Vec<&XNode> = Vec::new();
    let mut nested_lists: Vec<&XNode> = Vec::new();
    for c in children {
        match c {
            XNode::Element { name, .. } if name == "ul" || name == "ol" => nested_lists.push(c),
            other => inline_chunks.push(other),
        }
    }

    let inline_text = render_inline_filtered(&inline_chunks, ctx);
    let inline_text = inline_text.trim_end_matches('\n').to_string();

    out.push_str(&indent);
    out.push_str(if ordered { "1. " } else { "- " });
    out.push_str(&inline_text);
    out.push('\n');

    // Emit nested lists; emit_element pushes its own ensure_blank_line, so
    // pre-trim trailing blank lines so nested lists don't introduce extra
    // blanks in the middle of a list.
    let mut tmp = String::new();
    for nested in nested_lists {
        emit_node(nested, &mut tmp, ctx);
    }
    let nested = tmp.trim_matches('\n');
    if !nested.is_empty() {
        out.push_str(nested);
        out.push('\n');
    }
}

/// Render a sequence of nodes inline (no block-level newlines) into a string.
fn render_inline(children: &[XNode], ctx: &mut Context) -> String {
    let mut buf = String::new();
    for n in children {
        emit_node(n, &mut buf, ctx);
    }
    // Trim trailing/leading hard newlines that would interrupt inline flow.
    buf.trim().to_string()
}

fn render_inline_filtered(children: &[&XNode], ctx: &mut Context) -> String {
    let mut buf = String::new();
    for n in children {
        emit_node(n, &mut buf, ctx);
    }
    buf.trim().to_string()
}

/// Append `inline` as a paragraph block, ensuring it's preceded by a blank
/// line and followed by `\n\n`.
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

/// Collapse 3+ consecutive newlines to 2 to keep output tidy.
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

// =====================================================================
// Tables
// =====================================================================

fn emit_table(children: &[XNode], out: &mut String, ctx: &mut Context) {
    // Flatten <thead>/<tbody>/<tfoot> wrappers to find <tr> children.
    let mut rows: Vec<&[XNode]> = Vec::new();
    let mut header_count: Option<usize> = None;
    fn collect_rows<'a>(nodes: &'a [XNode], rows: &mut Vec<&'a [XNode]>) {
        for n in nodes {
            if let XNode::Element {
                name,
                children: kids,
                ..
            } = n
            {
                match name.as_str() {
                    "tr" => rows.push(kids),
                    "thead" | "tbody" | "tfoot" => collect_rows(kids, rows),
                    _ => {}
                }
            }
        }
    }
    collect_rows(children, &mut rows);

    if rows.is_empty() {
        return;
    }

    // Determine which row(s) are headers: a row containing any <th> is a
    // header row. If the FIRST row is a header, treat as GFM header row.
    let mut first_is_header = false;
    for cell in rows[0] {
        if let XNode::Element { name, .. } = cell
            && name == "th"
        {
            first_is_header = true;
            break;
        }
    }
    if first_is_header {
        header_count = Some(cell_count(rows[0]));
    }

    // If there's no explicit header, GFM still requires one. Synthesize an
    // empty header from the column count of the first row.
    let cols = header_count.unwrap_or_else(|| cell_count(rows[0]));
    if cols == 0 {
        return;
    }

    // Render header row.
    let header_cells = if first_is_header {
        rows.remove(0)
    } else {
        // Empty headers.
        &[]
    };
    out.push('|');
    if first_is_header {
        for cell in header_cells {
            if let XNode::Element { children: kids, .. } = cell {
                out.push(' ');
                out.push_str(&render_inline(kids, ctx));
                out.push_str(" |");
            }
        }
    } else {
        for _ in 0..cols {
            out.push_str("  |");
        }
    }
    out.push('\n');

    // Separator row.
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    out.push('\n');

    // Data rows.
    for row in rows {
        out.push('|');
        let mut emitted = 0_usize;
        for cell in row {
            if let XNode::Element {
                name,
                children: kids,
                ..
            } = cell
                && (name == "td" || name == "th")
            {
                out.push(' ');
                out.push_str(&render_inline(kids, ctx));
                out.push_str(" |");
                emitted += 1;
            }
        }
        // Pad short rows.
        while emitted < cols {
            out.push_str("  |");
            emitted += 1;
        }
        out.push('\n');
    }
    out.push('\n');
}

fn cell_count(row: &[XNode]) -> usize {
    row.iter()
        .filter(|c| {
            matches!(
                c,
                XNode::Element { name, .. } if name == "td" || name == "th"
            )
        })
        .count()
}

// =====================================================================
// Confluence-specific elements
// =====================================================================

fn emit_structured_macro(
    attrs: &BTreeMap<String, String>,
    children: &[XNode],
    out: &mut String,
    ctx: &mut Context,
) {
    let macro_name = attrs.get("ac:name").cloned().unwrap_or_default();

    // Find which directive (if any) matches this macro name. We can't just
    // call lookup() with the macro name because, e.g., `tip` vs `success`,
    // but the storage layer uses `conf_storage_macro` field which matches
    // macro_name on the directive registry.
    let spec = registered_spec_for_macro(&macro_name);

    // Collect parameters (text content of <ac:parameter ac:name="X">).
    let mut params = collect_macro_params(children);
    // Find the rich-text body (recursively rendered) and the plain-text body
    // (CDATA preserved verbatim).
    let mut rich_body_children: Option<&[XNode]> = None;
    let mut plain_body: Option<String> = None;
    for c in children {
        if let XNode::Element {
            name,
            children: kids,
            ..
        } = c
        {
            match name.as_str() {
                "ac:rich-text-body" => rich_body_children = Some(kids),
                "ac:plain-text-body" => plain_body = Some(collect_text(kids)),
                _ => {}
            }
        }
    }

    // Render the rich-text body first into a string (even if we won't use it,
    // we need the content for the directive emit).
    let body_md = if let Some(kids) = rich_body_children {
        let mut buf = String::new();
        emit_nodes(kids, &mut buf, ctx);
        buf.trim_end_matches('\n').to_string()
    } else {
        plain_body.clone().unwrap_or_default()
    };

    if !ctx.opts.render_directives {
        // Strip wrapper — emit the body content. The Confluence `status`
        // macro is a special case: its visible text lives in the `title`
        // parameter rather than the body, so falling back to `body_md` would
        // drop the badge text entirely. Prefer `title` when present.
        if macro_name == "status" {
            let title = params.get("title").cloned().unwrap_or_default();
            let text = if title.is_empty() { body_md } else { title };
            if !text.is_empty() {
                out.push_str(&text);
            }
            return;
        }
        // The `expand` macro keeps its title in an `<ac:parameter>` element
        // rather than as a bold paragraph at the top of its rich-text body,
        // so a naive `--no-directives` strip would drop it. Materialise the
        // title as a bold paragraph so users still see what the expand was
        // labelled as. Mirrors the equivalent fix in adf_to_md::render_expand.
        if macro_name == "expand" {
            let title = params.get("title").cloned().unwrap_or_default();
            if !title.is_empty() {
                ensure_blank_line(out);
                out.push_str("**");
                out.push_str(&title);
                out.push_str("**\n\n");
            }
            if !body_md.is_empty() {
                out.push_str(&body_md);
                out.push_str("\n\n");
            }
            return;
        }
        if !body_md.is_empty() {
            ensure_blank_line(out);
            out.push_str(&body_md);
            out.push_str("\n\n");
        }
        return;
    }

    let Some(spec) = spec else {
        // Unknown macro — pass through as raw HTML so the round-trip is
        // lossless (rare external macros like jira-issue, gallery, etc.).
        let node = XNode::Element {
            name: "ac:structured-macro".to_string(),
            attrs: attrs.clone(),
            children: clone_nodes(children),
            self_closing: false,
        };
        out.push_str(&emit_raw(&node));
        return;
    };

    // ---- inline directives (status, …) ----
    if spec.kind == DirectiveKind::Inline {
        emit_inline_macro_directive(spec.name, &mut params, out);
        return;
    }

    // ---- block directives ----
    let dname = spec.name;
    ensure_blank_line(out);
    out.push_str(":::");
    out.push_str(dname);
    if !params.is_empty() {
        out.push(' ');
        out.push_str(&render_attrs(&params));
    }
    out.push('\n');

    if spec.allows_body && !body_md.is_empty() {
        out.push_str(&body_md);
        out.push('\n');
    }
    out.push_str(":::\n\n");
}

/// Look up a directive spec by Confluence storage macro name (the macro name
/// used inside `<ac:structured-macro ac:name="…">`). This walks the registered
/// specs and finds the one whose `conf_storage_macro` field equals `macro_name`.
fn registered_spec_for_macro(
    macro_name: &str,
) -> Option<&'static crate::cli::commands::directives::DirectiveSpec> {
    if macro_name.is_empty() {
        return None;
    }
    // Try direct lookup by directive name first (covers info/warning/note/
    // tip/expand/toc/status which all share the macro name with the directive
    // name).
    if let Some(s) = lookup(macro_name)
        && s.conf_storage_macro == Some(macro_name)
    {
        return Some(s);
    }
    // Fallback: scan all specs for a matching conf_storage_macro.
    for &name in &[
        "info", "warning", "note", "tip", "expand", "toc", "status", "emoticon", "mention", "link",
        "image",
    ] {
        if let Some(s) = lookup(name)
            && s.conf_storage_macro == Some(macro_name)
        {
            return Some(s);
        }
    }
    None
}

fn collect_macro_params(children: &[XNode]) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    for c in children {
        if let XNode::Element {
            name,
            attrs,
            children: kids,
            ..
        } = c
            && name == "ac:parameter"
            && let Some(pname) = attrs.get("ac:name")
        {
            let value = collect_text(kids);
            params.insert(pname.clone(), value);
        }
    }
    params
}

fn emit_inline_macro_directive(
    dname: &str,
    params: &mut BTreeMap<String, String>,
    out: &mut String,
) {
    if dname == "status" {
        // `colour` → `color`; `title` becomes the directive content.
        if let Some(col) = params.remove("colour") {
            params.insert("color".to_string(), col);
        }
        let title = params.remove("title").unwrap_or_default();
        out.push(':');
        out.push_str(dname);
        out.push('[');
        out.push_str(&title);
        out.push(']');
        if !params.is_empty() {
            out.push('{');
            out.push_str(&render_attrs(params));
            out.push('}');
        }
        return;
    }

    // Generic inline macro fallback (other inline-kind macros — currently
    // none, but keep the path).
    out.push(':');
    out.push_str(dname);
    if !params.is_empty() {
        out.push('{');
        out.push_str(&render_attrs(params));
        out.push('}');
    }
}

fn emit_emoticon(attrs: &BTreeMap<String, String>, out: &mut String, ctx: &mut Context) {
    if !ctx.opts.render_directives {
        return;
    }
    let name = attrs.get("ac:name").cloned().unwrap_or_default();
    let mut params = BTreeMap::new();
    if !name.is_empty() {
        params.insert("name".to_string(), name);
    }
    out.push_str(":emoticon");
    if !params.is_empty() {
        out.push('{');
        out.push_str(&render_attrs(&params));
        out.push('}');
    }
}

fn emit_link(children: &[XNode], out: &mut String, ctx: &mut Context) {
    // Find the resource identifier child (`<ri:user/>`, `<ri:page/>`, …) and
    // the optional link-body text. Prefer `<ac:plain-text-link-body>` over
    // `<ac:link-body>` when both are present — the plain-text variant is
    // the authoritative display text written by the storage-format
    // serializer; the rich-text `<ac:link-body>` form is older and may
    // contain additional formatting we don't want to fold into the
    // bracketed directive content.
    let mut ri: Option<(&str, &BTreeMap<String, String>)> = None;
    let mut plain_body: Option<String> = None;
    let mut rich_body: Option<String> = None;
    for c in children {
        if let XNode::Element {
            name,
            attrs,
            children: kids,
            ..
        } = c
        {
            if name.starts_with("ri:") {
                ri = Some((name.as_str(), attrs));
            } else if name == "ac:plain-text-link-body" {
                plain_body = Some(collect_text(kids).trim().to_string());
            } else if name == "ac:link-body" {
                rich_body = Some(collect_text(kids).trim().to_string());
            }
        }
    }
    let body_text = plain_body.or(rich_body);

    match ri {
        Some(("ri:user", a)) => {
            if !ctx.opts.render_directives {
                if let Some(t) = body_text {
                    out.push_str(&t);
                } else if let Some(id) = a.get("ri:account-id").or_else(|| a.get("ri:userkey")) {
                    // Storage `<ac:link><ri:user ri:account-id="…"/></ac:link>`
                    // commonly omits any inner text — Confluence renders
                    // the display name from the user lookup. With
                    // `--no-directives`, fall back to a stable
                    // `@<account-id>` (Cloud) or `@<userkey>` (DC) token
                    // so the mention does not silently disappear.
                    out.push('@');
                    out.push_str(id);
                }
                return;
            }
            let account_id = a
                .get("ri:account-id")
                .or_else(|| a.get("ri:userkey"))
                .cloned()
                .unwrap_or_default();
            out.push_str(":mention[");
            out.push_str(body_text.as_deref().unwrap_or(""));
            out.push(']');
            if !account_id.is_empty() {
                out.push_str("{accountId=");
                out.push_str(&account_id);
                out.push('}');
            }
        }
        Some(("ri:page", a)) => {
            if !ctx.opts.render_directives {
                if let Some(t) = body_text {
                    out.push_str(&t);
                } else if let Some(title) = a.get("ri:content-title") {
                    out.push_str(title);
                }
                return;
            }
            let mut params: BTreeMap<String, String> = BTreeMap::new();
            // Prefer `ri:page-id` (the supported attribute on Data Center)
            // but also accept `ri:content-id` for backwards compatibility
            // with content stored by the previous, broken serializer. When
            // a numeric id is present we keep just `pageId=` in the
            // directive — the title is implicit in the bracketed display
            // content, so duplicating it as `title=` would only add noise.
            // We still emit both `ri:page-id` and `ri:content-title` on the
            // forward path (md→storage) so the page works on Cloud, but the
            // reverse direction collapses them back to a single `pageId=`.
            let page_id = a.get("ri:page-id").or_else(|| a.get("ri:content-id"));
            if let Some(id) = page_id {
                params.insert("pageId".to_string(), id.clone());
            } else if let Some(title) = a.get("ri:content-title") {
                params.insert("title".to_string(), title.clone());
            }
            if let Some(space) = a.get("ri:space-key") {
                params.insert("spaceKey".to_string(), space.clone());
            }
            // Use the link body when present; otherwise fall back to the
            // page title from `ri:content-title` so the bracketed
            // directive content matches what the user sees on the page.
            let display = body_text
                .or_else(|| a.get("ri:content-title").cloned())
                .unwrap_or_default();
            out.push_str(":link[");
            out.push_str(&display);
            out.push(']');
            if !params.is_empty() {
                out.push('{');
                out.push_str(&render_attrs(&params));
                out.push('}');
            }
        }
        // Other ri:* (attachment, blogpost, shortcut) → raw HTML passthrough.
        _ => {
            // Re-emit verbatim.
            let node = XNode::Element {
                name: "ac:link".to_string(),
                attrs: BTreeMap::new(),
                children: clone_nodes(children),
                self_closing: false,
            };
            out.push_str(&emit_raw(&node));
        }
    }
}

fn emit_image(
    attrs: &BTreeMap<String, String>,
    children: &[XNode],
    out: &mut String,
    ctx: &mut Context,
) {
    if !ctx.opts.render_directives {
        return;
    }
    let alt = attrs.get("ac:alt").cloned();

    // The resource is a `<ri:url ri:value="…"/>` or `<ri:attachment ri:filename="…"/>`.
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    for c in children {
        if let XNode::Element { name, attrs: a, .. } = c {
            match name.as_str() {
                "ri:url" => {
                    if let Some(v) = a.get("ri:value") {
                        params.insert("src".to_string(), v.clone());
                    }
                }
                "ri:attachment" => {
                    if let Some(v) = a.get("ri:filename") {
                        params.insert("attachment".to_string(), v.clone());
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(alt) = alt {
        params.insert("alt".to_string(), alt);
    }
    out.push_str(":image");
    if !params.is_empty() {
        out.push('{');
        out.push_str(&render_attrs(&params));
        out.push('}');
    }
}

/// Re-emit an `XNode` as raw XML, used as the unknown-element passthrough.
fn emit_raw(node: &XNode) -> String {
    let mut out = String::new();
    write_raw(node, &mut out);
    out
}

fn write_raw(node: &XNode, out: &mut String) {
    match node {
        XNode::Text(t) => out.push_str(&xml_escape_text(t)),
        XNode::Cdata(c) => {
            out.push_str("<![CDATA[");
            out.push_str(c);
            out.push_str("]]>");
        }
        XNode::Comment(c) => {
            out.push_str("<!--");
            out.push_str(c);
            out.push_str("-->");
        }
        XNode::Element {
            name,
            attrs,
            children,
            self_closing,
        } => {
            out.push('<');
            out.push_str(name);
            for (k, v) in attrs {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                out.push_str(&xml_escape_attr(v));
                out.push('"');
            }
            if *self_closing && children.is_empty() {
                out.push_str("/>");
                return;
            }
            out.push('>');
            for c in children {
                write_raw(c, out);
            }
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
    }
}

fn emit_raw_passthrough(
    name: &str,
    attrs: &BTreeMap<String, String>,
    children: &[XNode],
    self_closing: bool,
    out: &mut String,
) {
    out.push('<');
    out.push_str(name);
    for (k, v) in attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        out.push_str(&xml_escape_attr(v));
        out.push('"');
    }
    if self_closing && children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for c in children {
        write_raw(c, out);
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

fn xml_escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            other => out.push(other),
        }
    }
    out
}

fn xml_escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// Clone a slice of `XNode`s. Required by paths that need owned children for
/// raw-HTML re-emission (we can't move out of borrowed references).
fn clone_nodes(nodes: &[XNode]) -> Vec<XNode> {
    nodes.iter().map(clone_node).collect()
}

fn clone_node(node: &XNode) -> XNode {
    match node {
        XNode::Text(t) => XNode::Text(t.clone()),
        XNode::Cdata(c) => XNode::Cdata(c.clone()),
        XNode::Comment(c) => XNode::Comment(c.clone()),
        XNode::Element {
            name,
            attrs,
            children,
            self_closing,
        } => XNode::Element {
            name: name.clone(),
            attrs: attrs.clone(),
            children: clone_nodes(children),
            self_closing: *self_closing,
        },
    }
}

// =====================================================================
// Text escaping
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
                // Escape `:` only if the next byte is ASCII alphabetic.
                let needs_escape = i + 1 < bytes.len() && bytes[i + 1].is_ascii_alphabetic();
                if needs_escape {
                    out.push('\\');
                }
                out.push(':');
                i += 1;
            }
            _ => {
                // Push the (possibly multi-byte) char.
                let ch_len = utf8_char_len(b);
                let end = (i + ch_len).min(bytes.len());
                if let Ok(s) = std::str::from_utf8(&bytes[i..end]) {
                    out.push_str(s);
                } else {
                    // Fallback: push as-is byte (lossy).
                    let _ = write!(out, "{}", b as char);
                }
                i = end;
            }
        }
    }
    out
}

fn utf8_char_len(b: u8) -> usize {
    // ASCII (0x00..0x80) and UTF-8 continuation bytes (0x80..0xC0) advance
    // by 1; lead bytes 0xC0..0xE0 / 0xE0..0xF0 / 0xF0+ start 2-/3-/4-byte
    // sequences respectively.
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

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(xhtml: &str) -> String {
        storage_to_markdown(xhtml, ConvertOpts::default()).expect("conversion succeeded")
    }

    fn convert_no_directives(xhtml: &str) -> String {
        storage_to_markdown(
            xhtml,
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

    // ---- basic XHTML elements --------------------------------------------

    #[test]
    fn paragraph_to_markdown() {
        let out = convert("<p>Hello</p>");
        assert!(out.contains("Hello"), "got: {out:?}");
    }

    #[test]
    fn h1_to_markdown() {
        let out = convert("<h1>Title</h1>");
        assert!(out.contains("# Title"), "got: {out:?}");
    }

    #[test]
    fn h3_to_markdown() {
        let out = convert("<h3>Sub</h3>");
        assert!(out.contains("### Sub"), "got: {out:?}");
    }

    #[test]
    fn strong_to_markdown() {
        let out = convert("<p><strong>bold</strong></p>");
        assert!(out.contains("**bold**"), "got: {out:?}");
    }

    #[test]
    fn em_to_markdown() {
        let out = convert("<p><em>x</em></p>");
        assert!(out.contains("*x*"), "got: {out:?}");
    }

    #[test]
    fn strike_to_markdown() {
        let out = convert("<p><s>gone</s></p>");
        assert!(out.contains("~~gone~~"), "got: {out:?}");
    }

    #[test]
    fn link_to_markdown() {
        let out = convert(r#"<p><a href="http://x">link</a></p>"#);
        assert!(out.contains("[link](http://x)"), "got: {out:?}");
    }

    #[test]
    fn link_without_href_emits_text() {
        let out = convert("<p><a>nohref</a></p>");
        assert!(out.contains("nohref"), "got: {out:?}");
        assert!(!out.contains("]("), "got: {out:?}");
    }

    #[test]
    fn img_to_markdown() {
        let out = convert(r#"<p><img src="x.png" alt="alt"/></p>"#);
        assert!(out.contains("![alt](x.png)"), "got: {out:?}");
    }

    #[test]
    fn unordered_list() {
        let out = convert("<ul><li>a</li><li>b</li></ul>");
        assert!(out.contains("- a"), "got: {out:?}");
        assert!(out.contains("- b"), "got: {out:?}");
    }

    #[test]
    fn ordered_list_renumbers_to_one() {
        let out = convert("<ol><li>a</li><li>b</li></ol>");
        // CommonMark renumbers, so emitting 1. for every item is fine.
        assert!(out.contains("1. a"), "got: {out:?}");
        assert!(out.contains("1. b"), "got: {out:?}");
    }

    #[test]
    fn pre_code_with_language() {
        let out = convert(r#"<pre><code class="language-rust">fn main(){}</code></pre>"#);
        assert!(out.contains("```rust"), "got: {out:?}");
        assert!(out.contains("fn main(){}"), "got: {out:?}");
        assert!(out.contains("```\n"), "got: {out:?}");
    }

    #[test]
    fn pre_code_no_language() {
        let out = convert("<pre><code>plain code</code></pre>");
        assert!(out.contains("```\n"), "got: {out:?}");
        assert!(out.contains("plain code"), "got: {out:?}");
    }

    #[test]
    fn code_block_no_backticks_uses_three_tick_fence() {
        // Body has zero backticks, so the default 3-backtick fence is safe.
        let out = convert("<pre><code>hello</code></pre>");
        assert!(
            out.contains("```\nhello\n```\n"),
            "expected default 3-tick fence, got: {out:?}"
        );
        assert!(
            !out.contains("````"),
            "did not expect a 4-tick fence, got: {out:?}"
        );
    }

    #[test]
    fn code_block_with_double_backticks_still_three_tick_fence() {
        // Two consecutive backticks fit inside a 3-backtick fence.
        let out = convert("<pre><code>a `` b</code></pre>");
        assert!(
            out.contains("```\na `` b\n```\n"),
            "expected 3-tick fence, got: {out:?}"
        );
        assert!(
            !out.contains("````"),
            "did not expect a 4-tick fence, got: {out:?}"
        );
    }

    #[test]
    fn code_block_with_triple_backticks_uses_four_tick_fence() {
        // A run of 3 backticks would close a 3-tick fence prematurely; we must
        // emit at least a 4-tick fence.
        let out = convert("<pre><code>a ``` b</code></pre>");
        assert!(
            out.contains("````\na ``` b\n````\n"),
            "expected 4-tick fence, got: {out:?}"
        );
    }

    #[test]
    fn code_block_with_quadruple_backticks_uses_five_tick_fence() {
        let out = convert("<pre><code>a ```` b</code></pre>");
        assert!(
            out.contains("`````\na ```` b\n`````\n"),
            "expected 5-tick fence, got: {out:?}"
        );
    }

    #[test]
    fn code_block_with_language_and_long_run() {
        // Regression for the language case: a `<code class="language-rust">`
        // body with a quadruple-backtick run must emit a 5-tick fence and keep
        // the `rust` language tag adjacent to the opening fence.
        let out = convert(r#"<pre><code class="language-rust">a ```` b</code></pre>"#);
        assert!(
            out.contains("`````rust\na ```` b\n`````\n"),
            "expected 5-tick fence with rust language, got: {out:?}"
        );
    }

    #[test]
    fn inline_code() {
        let out = convert("<p>x <code>y</code> z</p>");
        assert!(out.contains("`y`"), "got: {out:?}");
    }

    #[test]
    fn blockquote_emits_prefix() {
        let out = convert("<blockquote><p>x</p></blockquote>");
        assert!(out.contains("> x"), "got: {out:?}");
    }

    #[test]
    fn hr_emits_three_dashes() {
        let out = convert("<hr/>");
        assert!(out.contains("---"), "got: {out:?}");
    }

    #[test]
    fn br_emits_hard_break() {
        let out = convert("<p>a<br/>b</p>");
        assert!(out.contains("  \n"), "got: {out:?}");
    }

    #[test]
    fn br_tag_becomes_two_spaces_newline() {
        // Storage `<p>Foo<br/>Bar</p>` must render as markdown `Foo  \nBar`
        // — the two-space marker is the CommonMark "hard line break" that a
        // round-trip through md_to_storage reproduces as `<br/>`.
        let out = convert("<p>Foo<br/>Bar</p>");
        assert!(
            out.contains("Foo  \nBar"),
            "expected 'Foo  \\nBar' (two spaces + newline), got: {out:?}"
        );
    }

    // ---- tables -----------------------------------------------------------

    #[test]
    fn simple_table_with_header() {
        let xhtml = "<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>";
        let out = convert(xhtml);
        assert!(out.contains("| A | B |"), "got: {out:?}");
        assert!(out.contains("| --- | --- |"), "got: {out:?}");
        assert!(out.contains("| 1 | 2 |"), "got: {out:?}");
    }

    #[test]
    fn table_cell_with_inline_formatting() {
        let xhtml = "<table><tr><th>X</th></tr><tr><td><strong>bold</strong></td></tr></table>";
        let out = convert(xhtml);
        assert!(out.contains("**bold**"), "got: {out:?}");
    }

    #[test]
    fn empty_table_is_safe() {
        // An empty <table> should not panic and should produce no output.
        let out = convert("<table></table>");
        assert!(!out.contains("|"), "got: {out:?}");
    }

    // ---- macros (block) ---------------------------------------------------

    #[test]
    fn info_macro_to_directive() {
        let xhtml = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>Hi</p></ac:rich-text-body></ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains("Hi"), "got: {out:?}");
        assert!(out.contains(":::"), "got: {out:?}");
    }

    #[test]
    fn info_with_title_parameter() {
        let xhtml = r#"<ac:structured-macro ac:name="info"><ac:parameter ac:name="title">Heads up</ac:parameter><ac:rich-text-body><p>Body</p></ac:rich-text-body></ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains(r#"title="Heads up""#), "got: {out:?}");
        assert!(out.contains("Body"), "got: {out:?}");
    }

    #[test]
    fn warning_macro_to_directive() {
        let xhtml = r#"<ac:structured-macro ac:name="warning"><ac:rich-text-body><p>w</p></ac:rich-text-body></ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(out.contains(":::warning"), "got: {out:?}");
    }

    #[test]
    fn note_macro_to_directive() {
        let xhtml = r#"<ac:structured-macro ac:name="note"><ac:rich-text-body><p>n</p></ac:rich-text-body></ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(out.contains(":::note"), "got: {out:?}");
    }

    #[test]
    fn tip_macro_to_directive() {
        let xhtml = r#"<ac:structured-macro ac:name="tip"><ac:rich-text-body><p>t</p></ac:rich-text-body></ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(out.contains(":::tip"), "got: {out:?}");
    }

    #[test]
    fn toc_self_closing_directive() {
        let xhtml = r#"<ac:structured-macro ac:name="toc"><ac:parameter ac:name="maxLevel">3</ac:parameter></ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(out.contains(":::toc"), "got: {out:?}");
        assert!(out.contains("maxLevel=3"), "got: {out:?}");
    }

    #[test]
    fn nested_expand_with_info_inside() {
        let xhtml = r#"<ac:structured-macro ac:name="expand">
<ac:parameter ac:name="title">Outer</ac:parameter>
<ac:rich-text-body>
<ac:structured-macro ac:name="info">
<ac:rich-text-body><p>inner</p></ac:rich-text-body>
</ac:structured-macro>
</ac:rich-text-body>
</ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(out.contains(":::expand"), "got: {out:?}");
        assert!(out.contains(":::info"), "got: {out:?}");
        assert!(out.contains("inner"), "got: {out:?}");
    }

    #[test]
    fn unknown_macro_passes_through_as_raw_html() {
        let xhtml = r#"<ac:structured-macro ac:name="jira-issue"><ac:parameter ac:name="key">FOO-1</ac:parameter></ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(
            out.contains("ac:structured-macro"),
            "must preserve raw XML, got: {out:?}"
        );
        assert!(
            !out.contains(":::jira-issue"),
            "must NOT directive-ize, got: {out:?}"
        );
    }

    #[test]
    fn render_directives_false_strips_wrappers() {
        let xhtml = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>Just the body</p></ac:rich-text-body></ac:structured-macro>"#;
        let out = convert_no_directives(xhtml);
        assert!(out.contains("Just the body"), "got: {out:?}");
        assert!(!out.contains(":::info"), "got: {out:?}");
    }

    #[test]
    fn expand_no_directives_renders_title_as_bold() {
        // `:::expand title="Click me"` → storage stashes the title in an
        // `<ac:parameter ac:name="title">` element. With render_directives
        // off, a naive strip would drop the title entirely; we render it as
        // a bold paragraph so the user still sees what the expand was
        // labelled as (mirrors how the warning panel preserves its title
        // through a strong-marked first paragraph in its body).
        let xhtml = r#"<ac:structured-macro ac:name="expand"><ac:parameter ac:name="title">Click me</ac:parameter><ac:rich-text-body><p>Hidden</p></ac:rich-text-body></ac:structured-macro>"#;
        let out = convert_no_directives(xhtml);
        assert!(
            out.contains("**Click me**"),
            "expected bold title, got: {out:?}"
        );
        assert!(out.contains("Hidden"), "got: {out:?}");
        assert!(
            !out.contains(":::expand"),
            "directive wrapper must be stripped, got: {out:?}"
        );
        // Title must precede body.
        let title_idx = out.find("**Click me**").expect("title present");
        let body_idx = out.find("Hidden").expect("body present");
        assert!(
            title_idx < body_idx,
            "title must come before body, got: {out:?}"
        );
    }

    #[test]
    fn expand_no_directives_no_title_emits_only_body() {
        // No `<ac:parameter ac:name="title">` element → we emit just the
        // body, no stray empty bold paragraph.
        let xhtml = r#"<ac:structured-macro ac:name="expand"><ac:rich-text-body><p>Hidden</p></ac:rich-text-body></ac:structured-macro>"#;
        let out = convert_no_directives(xhtml);
        assert!(out.contains("Hidden"), "got: {out:?}");
        assert!(
            !out.contains("**"),
            "must not emit empty bold marker when no title present, got: {out:?}"
        );
        assert!(
            !out.contains(":::expand"),
            "directive wrapper must be stripped, got: {out:?}"
        );
    }

    // ---- inline macros ----------------------------------------------------

    #[test]
    fn emoticon_to_directive() {
        let out = convert(r#"<p><ac:emoticon ac:name="warning"/></p>"#);
        assert!(out.contains(":emoticon"), "got: {out:?}");
        assert!(out.contains("name=warning"), "got: {out:?}");
    }

    #[test]
    fn mention_to_directive() {
        let out = convert(r#"<p><ac:link><ri:user ri:account-id="abc"/></ac:link></p>"#);
        assert!(out.contains(":mention[]"), "got: {out:?}");
        assert!(out.contains("accountId=abc"), "got: {out:?}");
    }

    #[test]
    fn mention_no_directives_falls_back_to_at_account_id() {
        // Storage `<ac:link><ri:user ri:account-id="…"/></ac:link>` with
        // no inner text. With `--no-directives`, the mention must not
        // disappear silently — fall back to `@<account-id>` so the
        // reference survives in plain text.
        let out =
            convert_no_directives(r#"<p><ac:link><ri:user ri:account-id="abc123"/></ac:link></p>"#);
        assert!(
            out.contains("@abc123"),
            "expected @<account-id> fallback, got: {out:?}"
        );
    }

    #[test]
    fn mention_no_directives_falls_back_to_userkey() {
        // Older Confluence DC uses `ri:userkey` instead of `ri:account-id`.
        // The fallback must prefer account-id but accept userkey when
        // account-id is absent.
        let out =
            convert_no_directives(r#"<p><ac:link><ri:user ri:userkey="legacy123"/></ac:link></p>"#);
        assert!(
            out.contains("@legacy123"),
            "expected @<userkey> fallback, got: {out:?}"
        );
    }

    #[test]
    fn mention_no_directives_with_body_text_uses_body() {
        // When an explicit `<ac:plain-text-link-body>` is present,
        // `--no-directives` must prefer the body text over the
        // account-id fallback.
        let xhtml = r#"<p><ac:link><ri:user ri:account-id="abc123"/><ac:plain-text-link-body><![CDATA[Jane Doe]]></ac:plain-text-link-body></ac:link></p>"#;
        let out = convert_no_directives(xhtml);
        assert!(
            out.contains("Jane Doe"),
            "expected the link body to render, got: {out:?}"
        );
        assert!(
            !out.contains("@abc123"),
            "must not emit the @<account-id> fallback when body text exists, got: {out:?}"
        );
    }

    #[test]
    fn link_to_page_with_title() {
        let out = convert(r#"<p><ac:link><ri:page ri:content-title="Page X"/></ac:link></p>"#);
        // No `<ac:plain-text-link-body>` is present, so we fall back to
        // the page title for the bracketed display content.
        assert!(out.contains(":link[Page X]"), "got: {out:?}");
        assert!(out.contains(r#"title="Page X""#), "got: {out:?}");
    }

    #[test]
    fn link_with_page_id_attribute_round_trips() {
        // `ri:page-id` (the new authoritative attribute) is parsed and the
        // `<ac:plain-text-link-body>` becomes the bracketed display text.
        let xhtml = r#"<p><ac:link><ri:page ri:page-id="98420"/><ac:plain-text-link-body><![CDATA[Parent]]></ac:plain-text-link-body></ac:link></p>"#;
        let out = convert(xhtml);
        assert!(out.contains(":link[Parent]"), "got: {out:?}");
        assert!(out.contains("pageId=98420"), "got: {out:?}");
    }

    #[test]
    fn link_with_both_page_id_and_content_title_prefers_page_id_in_directive() {
        // The forward (md→storage) path emits both `ri:page-id` and
        // `ri:content-title` so the link works on both Confluence Cloud and
        // Data Center. On the reverse path we keep only `pageId=` in the
        // directive: the title is already implicit in the bracketed
        // display content, so a redundant `title=` attribute would be
        // noise. PageId wins because the numeric reference survives a
        // page rename.
        let xhtml = r#"<p><ac:link><ri:page ri:page-id="42" ri:content-title="Foo"/><ac:plain-text-link-body><![CDATA[Foo]]></ac:plain-text-link-body></ac:link></p>"#;
        let out = convert(xhtml);
        assert!(out.contains(":link[Foo]"), "got: {out:?}");
        assert!(out.contains("pageId=42"), "got: {out:?}");
        assert!(
            !out.contains("title="),
            "title= attribute must be omitted when pageId is present (title is implicit in the bracketed content): {out:?}"
        );
    }

    #[test]
    fn link_with_legacy_content_id_attribute_still_parses() {
        // Pages stored by the previous, broken serializer used
        // `ri:content-id`. We must still surface the id as `pageId=` so
        // those documents remain readable.
        let xhtml = r#"<p><ac:link><ri:page ri:content-id="98420"/></ac:link></p>"#;
        let out = convert(xhtml);
        assert!(out.contains("pageId=98420"), "got: {out:?}");
    }

    #[test]
    fn link_extracts_plain_text_link_body() {
        // The text inside `<ac:plain-text-link-body><![CDATA[…]]>` becomes
        // the bracketed directive content, beating the `ri:content-title`
        // fallback when both are present.
        let xhtml = r#"<p><ac:link><ri:page ri:content-title="Stale"/><ac:plain-text-link-body><![CDATA[Fresh]]></ac:plain-text-link-body></ac:link></p>"#;
        let out = convert(xhtml);
        assert!(out.contains(":link[Fresh]"), "got: {out:?}");
    }

    #[test]
    fn link_prefers_plain_text_body_over_link_body() {
        // When both `<ac:plain-text-link-body>` and `<ac:link-body>` are
        // present, the plain-text variant wins.
        let xhtml = r#"<p><ac:link><ri:page ri:page-id="42"/><ac:link-body>RichText</ac:link-body><ac:plain-text-link-body><![CDATA[PlainText]]></ac:plain-text-link-body></ac:link></p>"#;
        let out = convert(xhtml);
        assert!(out.contains(":link[PlainText]"), "got: {out:?}");
    }

    #[test]
    fn link_to_page_with_id_and_space() {
        let out = convert(
            r#"<p><ac:link><ri:page ri:content-id="123" ri:space-key="DEV"/></ac:link></p>"#,
        );
        assert!(out.contains(":link"), "got: {out:?}");
        assert!(out.contains("pageId=123"), "got: {out:?}");
        assert!(out.contains("spaceKey=DEV"), "got: {out:?}");
    }

    #[test]
    fn image_with_url() {
        let out = convert(r#"<p><ac:image><ri:url ri:value="http://x.png"/></ac:image></p>"#);
        assert!(out.contains(":image"), "got: {out:?}");
        assert!(out.contains("src=http://x.png"), "got: {out:?}");
    }

    #[test]
    fn image_with_attachment() {
        let out =
            convert(r#"<p><ac:image><ri:attachment ri:filename="diagram.png"/></ac:image></p>"#);
        assert!(out.contains(":image"), "got: {out:?}");
        assert!(out.contains("attachment=diagram.png"), "got: {out:?}");
    }

    #[test]
    fn status_inline_with_colour_to_color() {
        let xhtml = r#"<p><ac:structured-macro ac:name="status"><ac:parameter ac:name="title">DONE</ac:parameter><ac:parameter ac:name="colour">green</ac:parameter></ac:structured-macro></p>"#;
        let out = convert(xhtml);
        assert!(out.contains(":status[DONE]"), "got: {out:?}");
        assert!(out.contains("color=green"), "got: {out:?}");
        assert!(
            !out.contains("colour="),
            "must reverse-map colour→color, got: {out:?}"
        );
    }

    // ---- unknown / raw passthrough ---------------------------------------

    #[test]
    fn div_passes_through() {
        let out = convert(r#"<div class="x"><p>y</p></div>"#);
        assert!(out.contains("<div"), "got: {out:?}");
        assert!(out.contains("</div>"), "got: {out:?}");
        assert!(out.contains("y"), "got: {out:?}");
    }

    #[test]
    fn span_passes_through() {
        let out = convert(r#"<p><span style="color:red">x</span></p>"#);
        assert!(out.contains("<span"), "got: {out:?}");
        assert!(out.contains("</span>"), "got: {out:?}");
    }

    #[test]
    fn comment_passes_through() {
        let out = convert("<!-- foo --><p>bar</p>");
        assert!(out.contains("<!-- foo -->"), "got: {out:?}");
    }

    // ---- edge cases ------------------------------------------------------

    #[test]
    fn empty_input() {
        let out = convert("");
        assert!(out.trim().is_empty(), "got: {out:?}");
    }

    #[test]
    fn whitespace_only_input() {
        let out = convert("   \n  ");
        // Whitespace should not produce any XHTML wrappers.
        assert!(!out.contains("<"), "got: {out:?}");
    }

    #[test]
    fn malformed_xml_returns_err() {
        let err = storage_to_markdown("<p>unclosed", ConvertOpts::default()).unwrap_err();
        match err {
            StorageToMdError::Xml(_) => {}
        }
    }

    #[test]
    fn entities_decoded_in_text() {
        let out = convert("<p>a &amp; b &lt;c&gt; &quot;d&quot;</p>");
        assert!(out.contains("a & b"), "got: {out:?}");
        assert!(out.contains("<c>"), "got: {out:?}");
        assert!(out.contains(r#""d""#), "got: {out:?}");
    }

    #[test]
    fn cdata_in_plain_text_body_preserved() {
        let xhtml = r#"<ac:structured-macro ac:name="info"><ac:plain-text-body><![CDATA[raw <stuff> here]]></ac:plain-text-body></ac:structured-macro>"#;
        let out = convert(xhtml);
        assert!(out.contains("raw <stuff> here"), "got: {out:?}");
    }

    #[test]
    fn mixed_content_text_element_text() {
        let out = convert("<p>before <strong>middle</strong> after</p>");
        assert!(out.contains("before"), "got: {out:?}");
        assert!(out.contains("**middle**"), "got: {out:?}");
        assert!(out.contains("after"), "got: {out:?}");
    }

    #[test]
    fn markdown_special_chars_escaped_in_text() {
        let out = convert("<p>a*b*c</p>");
        assert!(out.contains(r"a\*b\*c"), "got: {out:?}");
    }

    #[test]
    fn underscore_escaped_in_text() {
        let out = convert("<p>a_b_c</p>");
        assert!(out.contains(r"a\_b\_c"), "got: {out:?}");
    }

    #[test]
    fn brackets_escaped_in_text() {
        let out = convert("<p>foo [bar]</p>");
        assert!(out.contains(r"\["), "got: {out:?}");
        assert!(out.contains(r"\]"), "got: {out:?}");
    }

    #[test]
    fn colon_followed_by_alpha_escaped() {
        // `:foo` in text would otherwise re-trigger inline directive parsing
        // when this output is fed back through md→storage.
        let out = convert("<p>see :foo here</p>");
        assert!(out.contains(r"\:foo"), "got: {out:?}");
    }

    #[test]
    fn colon_followed_by_space_not_escaped() {
        let out = convert("<p>note: text</p>");
        // Colon followed by space (or non-alpha) should NOT be escaped.
        assert!(!out.contains(r"\: "), "got: {out:?}");
        assert!(out.contains("note:"), "got: {out:?}");
    }

    #[test]
    fn https_url_in_text_not_escaped() {
        // https: is followed by `/`, not alpha, so colon stays unescaped.
        let out = convert("<p>see https://example.com today</p>");
        assert!(out.contains("https://example.com"), "got: {out:?}");
        assert!(!out.contains(r"https\:"), "got: {out:?}");
    }

    #[test]
    fn backtick_escaped_in_text() {
        let out = convert("<p>see `bar`</p>");
        assert!(out.contains(r"\`bar\`"), "got: {out:?}");
    }

    #[test]
    fn backslash_escaped_in_text() {
        let out = convert(r#"<p>a\b</p>"#);
        assert!(out.contains(r"a\\b"), "got: {out:?}");
    }

    // ---- round-trip sanity (md → storage → md) ----------------------------

    #[test]
    fn roundtrip_info_directive_sanity() {
        // Apply md → storage, then storage → md, and check key tokens.
        use crate::cli::commands::converters::md_to_storage::markdown_to_storage;
        let xml = markdown_to_storage(":::info\nHello\n:::").unwrap();
        let md = storage_to_markdown(&xml, ConvertOpts::default()).unwrap();
        assert!(md.contains(":::info"), "round-trip lost directive: {md:?}");
        assert!(md.contains("Hello"), "round-trip lost body: {md:?}");
    }

    #[test]
    fn roundtrip_warning_with_title_sanity() {
        use crate::cli::commands::converters::md_to_storage::markdown_to_storage;
        let xml = markdown_to_storage(":::warning title=\"Heads up\"\nbody\n:::").unwrap();
        let md = storage_to_markdown(&xml, ConvertOpts::default()).unwrap();
        assert!(md.contains(":::warning"), "got: {md:?}");
        assert!(md.contains(r#"title="Heads up""#), "got: {md:?}");
        assert!(md.contains("body"), "got: {md:?}");
    }

    #[test]
    fn roundtrip_status_inline_sanity() {
        use crate::cli::commands::converters::md_to_storage::markdown_to_storage;
        let xml = markdown_to_storage(":status[DONE]{color=green}").unwrap();
        let md = storage_to_markdown(&xml, ConvertOpts::default()).unwrap();
        assert!(md.contains(":status[DONE]"), "got: {md:?}");
        assert!(md.contains("color=green"), "got: {md:?}");
    }

    #[test]
    fn mismatched_end_tag_returns_err() {
        // Bug 7: with `check_end_names = true` (the quick-xml default),
        // mismatched end tags must surface as an XML error rather than parsing
        // silently. The module's documented contract is "well-formed XML
        // required".
        let result = storage_to_markdown("<p>oops</div>", ConvertOpts::default());
        match result {
            Err(StorageToMdError::Xml(_)) => {}
            other => panic!("expected XML error, got: {other:?}"),
        }
    }

    #[test]
    fn no_directives_status_emits_title_text() {
        // Bug 8: in the strip path, the `status` macro stores its visible
        // text in the `title` parameter, not the body. Falling back to body
        // content drops the badge text — title must be preferred.
        let xhtml = r#"<p><ac:structured-macro ac:name="status"><ac:parameter ac:name="title">DONE</ac:parameter><ac:parameter ac:name="colour">green</ac:parameter></ac:structured-macro></p>"#;
        let out = convert_no_directives(xhtml);
        assert!(out.contains("DONE"), "expected title text, got: {out:?}");
        // The directive syntax must NOT be emitted in strip mode.
        assert!(
            !out.contains(":status["),
            "directive form leaked into strip mode: {out:?}"
        );
    }

    // ---- HTML named-entity pre-pass ---------------------------------------

    #[test]
    fn replace_html_entities_handles_mdash() {
        // Confluence Cloud serializes `—` as `&mdash;`. quick-xml only
        // knows the five XML predefined entities, so without the pre-pass
        // we'd error out on `unrecognized entity mdash`.
        assert_eq!(replace_html_entities("a &mdash; b"), "a \u{2014} b");
    }

    #[test]
    fn replace_html_entities_handles_nbsp() {
        assert_eq!(replace_html_entities("a&nbsp;b"), "a\u{00A0}b");
    }

    #[test]
    fn replace_html_entities_handles_hellip() {
        assert_eq!(replace_html_entities("wait&hellip;"), "wait\u{2026}");
    }

    #[test]
    fn replace_html_entities_passes_through_unknown() {
        // An unknown named entity must be preserved verbatim. We don't drop
        // it (which would corrupt legitimate text) and we don't error
        // ourselves — quick-xml will surface "unrecognized entity" if it
        // really is meant as XML.
        assert_eq!(replace_html_entities("&unknownEntity;"), "&unknownEntity;");
    }

    #[test]
    fn replace_html_entities_does_not_double_decode_amp() {
        // `&amp;mdash;` must stay literal `&mdash;` text — that is, the
        // `&amp;` is preserved as-is so quick-xml can later decode it to
        // `&`. If we expanded `&amp;` first and then expanded the resulting
        // `&mdash;` to an em-dash, the round-trip would be wrong.
        assert_eq!(replace_html_entities("&amp;mdash;"), "&amp;mdash;");
    }

    #[test]
    fn replace_html_entities_skips_numeric_entities() {
        // quick-xml handles `&#NN;` and `&#xNN;` natively; we leave them
        // alone in the pre-pass.
        assert_eq!(replace_html_entities("&#65; &#x41;"), "&#65; &#x41;");
    }

    #[test]
    fn replace_html_entities_preserves_xml_predefined() {
        // The five XML predefined entities are passed through verbatim so
        // quick-xml decodes them once and only once.
        assert_eq!(
            replace_html_entities("&amp; &lt; &gt; &quot; &apos;"),
            "&amp; &lt; &gt; &quot; &apos;",
        );
    }

    #[test]
    fn replace_html_entities_skips_inside_cdata() {
        // Entities inside CDATA are literal text, not entities. Replacing
        // `&mdash;` inside a CDATA block would corrupt user data.
        let input = "<p><![CDATA[raw &mdash; literal]]></p>";
        assert_eq!(replace_html_entities(input), input);
    }

    #[test]
    fn replace_html_entities_handles_entity_outside_cdata_after_cdata() {
        // After a CDATA section closes, regular entity replacement resumes.
        let input = "<p><![CDATA[raw &mdash; literal]]> then &mdash; here</p>";
        let expected = "<p><![CDATA[raw &mdash; literal]]> then \u{2014} here</p>";
        assert_eq!(replace_html_entities(input), expected);
    }

    #[test]
    fn replace_html_entities_handles_lone_ampersand() {
        // A bare `&` that doesn't form an entity is left as-is; quick-xml
        // will still error if it really isn't escapable, but we must not
        // turn it into garbage here.
        assert_eq!(replace_html_entities("foo & bar"), "foo & bar");
    }

    #[test]
    fn replace_html_entities_empty_input() {
        assert_eq!(replace_html_entities(""), "");
    }

    #[test]
    fn storage_to_markdown_em_dash_round_trip() {
        // Regression test: a Confluence storage page containing `&mdash;`
        // used to error with `unrecognized entity 'mdash'`. It must now
        // convert successfully and contain the literal em-dash.
        let out = convert("<p>hello &mdash; world</p>");
        assert!(out.contains("hello \u{2014} world"), "got: {out:?}");
    }

    #[test]
    fn storage_to_markdown_non_breaking_space() {
        let out = convert("<p>a&nbsp;b</p>");
        assert!(out.contains("a\u{00A0}b"), "got: {out:?}");
    }

    #[test]
    fn storage_to_markdown_smart_quotes() {
        let out = convert("<p>&ldquo;hi&rdquo; she said</p>");
        assert!(out.contains("\u{201C}hi\u{201D}"), "got: {out:?}");
    }

    #[test]
    fn storage_to_markdown_accented_latin_in_attribute() {
        // Named entities in attribute values are also covered by the
        // pre-pass.
        let out = convert(r#"<p><a href="http://x?q=caf&eacute;">link</a></p>"#);
        assert!(out.contains("caf\u{00E9}"), "got: {out:?}");
    }

    #[test]
    fn storage_to_markdown_unknown_entity_still_errors() {
        // If a Confluence-emitted entity is not in our table, the original
        // `unrecognized entity` XML error must still surface (rather than
        // silently disappearing) so we can add it.
        let result =
            storage_to_markdown("<p>&someBogusUnknownThingyXyz;</p>", ConvertOpts::default());
        match result {
            Err(StorageToMdError::Xml(_)) => {}
            other => panic!("expected XML error for unknown entity, got: {other:?}"),
        }
    }
}
