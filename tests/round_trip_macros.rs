//! Round-trip integration tests for Confluence storage ↔ markdown conversion
//! of `<ac:structured-macro>` blocks.
//!
//! These tests exercise the public API directly (no Prism mock, no HTTP).
//! They guard against the historical regression where:
//!
//! - The read side (`storage_to_markdown`) emitted unknown macros — including
//!   the very common `<ac:structured-macro ac:name="code">` — as raw XHTML
//!   inline in the markdown output.
//! - The write side (`markdown_to_storage`) then ran that markdown through
//!   comrak, which (by CommonMark §6.6) escaped `<` to `&lt;` because the
//!   tag name contains a colon — destroying the macro structure.
//!
//! After the fix, `code` macros become CommonMark fenced code blocks on the
//! read side and rebuild back into `<ac:structured-macro ac:name="code">` on
//! the write side. Other unknown macros (jira-issue, gallery, panel, …) pass
//! through verbatim via the raw-XHTML extractor.

use atl::cli::commands::converters::md_to_storage::markdown_to_storage;
use atl::cli::commands::converters::storage_to_md::{ConvertOpts, storage_to_markdown};

/// The exact failing fixture from the original bug report: a Confluence-emitted
/// `code` macro with breakout parameters, language, and a multi-line body.
#[test]
fn code_macro_round_trip_from_bug_report() {
    let storage = concat!(
        r#"<ac:structured-macro ac:local-id="abc123" ac:macro-id="def456" "#,
        r#"ac:name="code" ac:schema-version="1">"#,
        r#"<ac:parameter ac:name="breakoutMode">wide</ac:parameter>"#,
        r#"<ac:parameter ac:name="breakoutWidth">760</ac:parameter>"#,
        r#"<ac:parameter ac:name="language">bash</ac:parameter>"#,
        r#"<ac:plain-text-body><![CDATA[#!/bin/bash"#,
        "\necho \"hello\"\necho \"world\"]]></ac:plain-text-body>",
        r#"</ac:structured-macro>"#,
    );

    // Step 1: storage → markdown. The macro must become a fenced code block.
    let md = storage_to_markdown(storage, ConvertOpts::default()).expect("storage→md");
    assert!(
        md.contains("```"),
        "intermediate markdown must contain a backtick fence, got: {md:?}"
    );
    assert!(
        md.contains("bash"),
        "intermediate markdown must carry the bash language token, got: {md:?}"
    );
    assert!(
        md.contains("#!/bin/bash"),
        "shebang line must survive, got: {md:?}"
    );
    assert!(
        md.contains("echo \"hello\""),
        "first echo must survive, got: {md:?}"
    );
    assert!(
        md.contains("echo \"world\""),
        "second echo must survive, got: {md:?}"
    );
    assert!(
        !md.contains("<ac:structured-macro"),
        "raw macro tag must NOT survive in intermediate markdown, got: {md:?}"
    );

    // Step 2: markdown → storage. The fence must rebuild as a `code` macro.
    let storage2 = markdown_to_storage(&md).expect("md→storage");
    assert!(
        storage2.contains(r#"<ac:structured-macro ac:name="code">"#),
        "rebuilt storage must contain the code macro (literal <), got: {storage2:?}"
    );
    assert!(
        storage2.contains(r#"<ac:parameter ac:name="language">bash</ac:parameter>"#),
        "rebuilt storage must carry the language parameter, got: {storage2:?}"
    );
    assert!(
        storage2.contains("<![CDATA[#!/bin/bash\necho \"hello\"\necho \"world\"]]>"),
        "rebuilt storage must preserve the original body inside CDATA, got: {storage2:?}"
    );

    // Lossy-on-purpose: breakout-* parameters and macro-ids are dropped.
    assert!(
        !storage2.contains("breakoutMode"),
        "breakoutMode parameter must NOT appear after round-trip, got: {storage2:?}"
    );
    assert!(
        !storage2.contains("breakoutWidth"),
        "breakoutWidth parameter must NOT appear after round-trip, got: {storage2:?}"
    );
    assert!(
        !storage2.contains("local-id"),
        "ac:local-id must NOT appear after round-trip, got: {storage2:?}"
    );
    assert!(
        !storage2.contains("macro-id"),
        "ac:macro-id must NOT appear after round-trip, got: {storage2:?}"
    );
    assert!(
        !storage2.contains("&lt;ac:structured-macro"),
        "macro must NOT be escaped to &lt;, got: {storage2:?}"
    );

    // Step 3: a second round-trip on the rebuilt storage must be idempotent.
    let md2 = storage_to_markdown(&storage2, ConvertOpts::default()).expect("storage→md (2)");
    let storage3 = markdown_to_storage(&md2).expect("md→storage (2)");
    assert_eq!(
        storage2, storage3,
        "second round-trip must be byte-identical (idempotence after the first lossy pass)"
    );
}

/// An unknown macro (jira-issue) must survive the round-trip verbatim through
/// the raw-XHTML extractor — no `<` → `&lt;` escaping anywhere.
#[test]
fn unknown_macro_byte_identical_first_round_trip() {
    let storage = concat!(
        "<p>Before.</p>",
        r#"<ac:structured-macro ac:name="jira-issue" ac:schema-version="1">"#,
        r#"<ac:parameter ac:name="key">FOO-123</ac:parameter>"#,
        r#"<ac:parameter ac:name="showSummary">true</ac:parameter>"#,
        r#"</ac:structured-macro>"#,
        "<p>After.</p>",
    );

    // Step 1: storage → markdown. The macro is unknown, so it falls through to
    // the raw-XHTML passthrough and appears literally in the intermediate
    // markdown.
    let md = storage_to_markdown(storage, ConvertOpts::default()).expect("storage→md");
    assert!(
        md.contains(r#"<ac:structured-macro ac:name="jira-issue""#),
        "intermediate markdown must contain raw macro tag, got: {md:?}"
    );

    // Step 2: markdown → storage. The raw extractor pre-substitutes the macro
    // with a placeholder so comrak doesn't escape it, then restores it
    // verbatim in the rendered HTML.
    let storage2 = markdown_to_storage(&md).expect("md→storage");
    assert!(
        storage2.contains(r#"<ac:structured-macro ac:name="jira-issue""#),
        "rebuilt storage must contain literal `<ac:structured-macro ac:name=\"jira-issue\"`, \
         got: {storage2:?}"
    );
    assert!(
        storage2.contains("FOO-123"),
        "issue key must survive, got: {storage2:?}"
    );
    assert!(
        storage2.contains("showSummary"),
        "showSummary parameter name must survive, got: {storage2:?}"
    );
    assert!(
        storage2.contains("true"),
        "showSummary value must survive, got: {storage2:?}"
    );
    assert!(
        !storage2.contains("&lt;ac:"),
        "macro must NOT be escaped to &lt;, got: {storage2:?}"
    );
    assert!(
        !storage2.contains("&lt;/ac:"),
        "close tag must NOT be escaped to &lt;/, got: {storage2:?}"
    );
}
