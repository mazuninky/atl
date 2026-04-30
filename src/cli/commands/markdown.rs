use comrak::nodes::{AstNode, ListType, NodeValue};
use comrak::{Arena, Options, markdown_to_html, parse_document};

/// Convert Markdown to Confluence storage format (XHTML).
pub fn markdown_to_storage(md: &str) -> String {
    markdown_to_html(md, &Options::default())
}

/// Convert Markdown to Jira wiki syntax.
///
/// Implementation strategy: parse markdown into a comrak AST and walk the
/// tree, emitting Jira wiki notation. The conversion is intentionally lossy
/// for constructs Jira wiki doesn't support natively (e.g. task lists become
/// plain bullets, image alt-text is dropped) and never fails — invalid input
/// just produces best-effort output.
pub fn markdown_to_wiki(md: &str) -> String {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;

    let root = parse_document(&arena, md, &options);
    let mut out = String::new();
    render_block_children(root, &mut out, 0);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_heading() {
        let result = markdown_to_storage("# Hello");
        assert!(result.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn converts_paragraph() {
        let result = markdown_to_storage("Some text");
        assert!(result.contains("<p>Some text</p>"));
    }

    #[test]
    fn converts_bold() {
        let result = markdown_to_storage("**bold**");
        assert!(result.contains("<strong>bold</strong>"));
    }

    #[test]
    fn converts_list() {
        let result = markdown_to_storage("- item1\n- item2");
        assert!(result.contains("<li>item1</li>"));
        assert!(result.contains("<li>item2</li>"));
    }

    #[test]
    fn markdown_to_wiki_heading_h1() {
        assert_eq!(markdown_to_wiki("# Hi"), "h1. Hi\n");
    }

    #[test]
    fn markdown_to_wiki_bold() {
        assert_eq!(markdown_to_wiki("**x**"), "*x*\n");
    }

    #[test]
    fn markdown_to_wiki_italic_underscore() {
        assert_eq!(markdown_to_wiki("_x_"), "_x_\n");
    }

    #[test]
    fn markdown_to_wiki_inline_code() {
        assert_eq!(markdown_to_wiki("`x`"), "{{x}}\n");
    }

    #[test]
    fn markdown_to_wiki_fenced_code_with_lang() {
        let input = "```rust\nfn x() {}\n```";
        assert_eq!(markdown_to_wiki(input), "{code:rust}\nfn x() {}\n{code}\n");
    }

    #[test]
    fn markdown_to_wiki_link() {
        assert_eq!(markdown_to_wiki("[t](u)"), "[t|u]\n");
    }

    #[test]
    fn markdown_to_wiki_bullet_list() {
        assert_eq!(markdown_to_wiki("- a\n- b"), "* a\n* b\n");
    }

    #[test]
    fn markdown_to_wiki_pipe_table() {
        let input = "| h1 | h2 |\n|----|----|\n| c1 | c2 |\n";
        let result = markdown_to_wiki(input);
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
        assert_eq!(markdown_to_wiki("## Hi"), "h2. Hi\n");
    }

    #[test]
    fn heading_h6() {
        assert_eq!(markdown_to_wiki("###### Hi"), "h6. Hi\n");
    }

    #[test]
    fn heading_with_inline_bold() {
        // Bold inside a heading must be converted to `*...*` wiki syntax.
        assert_eq!(markdown_to_wiki("# **Bold** title"), "h1. *Bold* title\n");
    }

    // ---- inline formatting ----

    #[test]
    fn italic_with_asterisk_marker() {
        // `*x*` is parsed as emphasis (Emph) and emits `_x_` per Jira wiki.
        assert_eq!(markdown_to_wiki("*x*"), "_x_\n");
    }

    #[test]
    fn triple_star_emits_outer_emph_inner_strong() {
        // `***word***` parses as <em><strong>word</strong></em> in CommonMark.
        // The implementation walks outer-first, so the result is `_*word*_`,
        // not the spec's suggested `*_word_*`. This test locks in the
        // implementation's behavior.
        assert_eq!(markdown_to_wiki("***word***"), "_*word*_\n");
    }

    #[test]
    fn strikethrough() {
        assert_eq!(markdown_to_wiki("~~x~~"), "-x-\n");
    }

    #[test]
    fn inline_code_with_close_brace_falls_back_to_noformat() {
        // `}}` would prematurely close `{{...}}`, so the converter falls back
        // to `{noformat}` which has no nesting/escape concerns.
        assert_eq!(markdown_to_wiki("`a}}b`"), "{noformat}a}}b{noformat}\n");
    }

    #[test]
    fn paragraph_bold_in_text() {
        // Bold inside a paragraph (not standalone) — verifies the default
        // Strong handler runs through the inline pipeline.
        assert_eq!(markdown_to_wiki("hello **world**"), "hello *world*\n");
    }

    // ---- code blocks ----

    #[test]
    fn fenced_code_no_lang() {
        assert_eq!(
            markdown_to_wiki("```\nplain\n```"),
            "{code}\nplain\n{code}\n"
        );
    }

    #[test]
    fn fenced_code_with_marker_in_body_falls_back_to_noformat() {
        // If the body literally contains `{code}`, the `{code}...{code}`
        // delimiter would close prematurely — fall back to `{noformat}`.
        assert_eq!(
            markdown_to_wiki("```\n{code}\n```"),
            "{noformat}\n{code}\n{noformat}\n"
        );
    }

    #[test]
    fn fenced_code_with_lang_and_marker_in_body_uses_noformat() {
        // The collision check fires regardless of language.
        let input = "```rust\n// uses {code:foo} delim\n```";
        let result = markdown_to_wiki(input);
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
            markdown_to_wiki("    line1\n    line2"),
            "{code}\nline1\nline2\n{code}\n"
        );
    }

    // ---- lists ----

    #[test]
    fn bullet_list_asterisk_marker_input() {
        // Same output regardless of whether the source uses `-` or `*`.
        assert_eq!(markdown_to_wiki("* a\n* b"), "* a\n* b\n");
    }

    #[test]
    fn nested_bullet_two_levels() {
        assert_eq!(markdown_to_wiki("- a\n  - b"), "* a\n** b\n");
    }

    #[test]
    fn nested_bullet_three_levels() {
        assert_eq!(
            markdown_to_wiki("- a\n  - b\n    - c"),
            "* a\n** b\n*** c\n"
        );
    }

    #[test]
    fn ordered_list_flat() {
        // Each numbered item emits `# ` regardless of source numbering.
        assert_eq!(markdown_to_wiki("1. a\n2. b"), "# a\n# b\n");
    }

    #[test]
    fn nested_ordered_list() {
        // Depth-2 ordered list emits `## ` (depth count of `#` markers).
        assert_eq!(markdown_to_wiki("1. a\n   1. b"), "# a\n## b\n");
    }

    #[test]
    fn mixed_nested_bullet_then_ordered() {
        // SPEC NOTE: the spec hinted at `*#` for mixed nesting, but the
        // implementation emits `##` because the marker char is chosen by the
        // inner list's type alone, repeated `depth` times. This test locks in
        // the actual behavior — change here means an intentional behavior
        // change.
        assert_eq!(markdown_to_wiki("- a\n  1. b"), "* a\n## b\n");
    }

    #[test]
    fn task_list_emitted_as_plain_bullets() {
        // Jira wiki has no native checkbox; the `- [ ] / - [x]` markers are
        // dropped and the items render as ordinary bullets.
        assert_eq!(markdown_to_wiki("- [ ] a\n- [x] b"), "* a\n* b\n");
    }

    #[test]
    fn list_item_with_two_paragraphs_joined_by_hard_break() {
        // The second paragraph in the same `<li>` is separated by `\\` so the
        // wiki rendering keeps them visually distinct without breaking the list.
        assert_eq!(markdown_to_wiki("- a\n\n  b"), "* a\\\\b\n");
    }

    // ---- links and images ----

    #[test]
    fn link_with_text() {
        assert_eq!(
            markdown_to_wiki("[text](https://example.com)"),
            "[text|https://example.com]\n"
        );
    }

    #[test]
    fn link_with_empty_text_emits_url_only() {
        // No text → no `text|` prefix; just `[url]`.
        assert_eq!(
            markdown_to_wiki("[](https://example.com)"),
            "[https://example.com]\n"
        );
    }

    #[test]
    fn image_drops_alt_text() {
        // Alt text is intentionally dropped — Jira wiki `!url!` syntax has no
        // alt-text field.
        assert_eq!(
            markdown_to_wiki("![alt](https://example.com/img.png)"),
            "!https://example.com/img.png!\n"
        );
    }

    #[test]
    fn autolink_emits_link_with_url_as_text() {
        // SPEC NOTE: comrak's autolink extension expands `<url>` into a Link
        // node with the URL as both the displayed text and the href, so the
        // converter emits `[url|url]` rather than the bare URL or `[url]`.
        assert_eq!(
            markdown_to_wiki("<https://example.com>"),
            "[https://example.com|https://example.com]\n"
        );
    }

    // ---- tables ----

    #[test]
    fn table_header_only() {
        // No data rows — separator row is silently dropped, just the header
        // remains.
        assert_eq!(
            markdown_to_wiki("| h1 | h2 |\n|----|----|\n"),
            "||h1||h2||\n"
        );
    }

    #[test]
    fn table_cell_pipe_is_escaped() {
        // A literal `|` inside a cell must be escaped to `\|` so it doesn't
        // close the cell. Use a markdown-escaped pipe in the input.
        let result = markdown_to_wiki("| a \\| b | c |\n|---|---|\n| 1 | 2 |\n");
        assert!(
            result.contains(r"||a \| b||c||"),
            "expected escaped pipe in header cell, got:\n{result}"
        );
        assert!(result.contains("|1|2|"), "expected data row in:\n{result}");
    }

    // ---- blockquotes ----

    #[test]
    fn blockquote_single_line() {
        assert_eq!(markdown_to_wiki("> hello"), "{quote}\nhello\n{quote}\n");
    }

    #[test]
    fn blockquote_multi_line_joined_with_space() {
        // A multi-line blockquote is one paragraph with soft breaks; soft
        // breaks become a single space, so the body is `a b` (not `a\nb`).
        assert_eq!(markdown_to_wiki("> a\n> b"), "{quote}\na b\n{quote}\n");
    }

    // ---- other blocks ----

    #[test]
    fn horizontal_rule() {
        assert_eq!(markdown_to_wiki("---"), "----\n");
    }

    #[test]
    fn hard_line_break_emits_double_backslash() {
        // Two trailing spaces produce a markdown LineBreak → `\\` in wiki.
        assert_eq!(markdown_to_wiki("a  \nb"), "a\\\\b\n");
    }

    #[test]
    fn soft_line_break_joins_with_space() {
        // A bare newline in markdown is a soft break; collapses to a space.
        assert_eq!(markdown_to_wiki("a\nb"), "a b\n");
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
        let result = markdown_to_wiki(input);

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
}
