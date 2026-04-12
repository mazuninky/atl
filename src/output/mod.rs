mod console;
mod csv_out;
mod json;
mod toml_out;
mod toon;
pub mod transform;

pub use console::ConsoleReporter;
pub use csv_out::CsvReporter;
pub use json::JsonReporter;
pub use toml_out::TomlReporter;
pub use toon::ToonReporter;
pub use transform::{Transformed, Transforms};

use std::io::Write;

use crate::io::IoStreams;

pub trait Reporter {
    fn report(&self, value: &serde_json::Value, writer: &mut dyn Write) -> anyhow::Result<()>;
}

/// Builds a reporter for the given format.
///
/// Takes `use_color` directly (rather than borrowing an `IoStreams`) so the
/// caller can hold a mutable borrow on `io` in parallel — required by the
/// `write_output` helper, which needs both the reporter factory and
/// `io.stdout()` at the same time.
pub fn reporter_for_format(format: &OutputFormat, use_color: bool) -> Box<dyn Reporter> {
    match format {
        OutputFormat::Console => Box::new(ConsoleReporter::new(use_color)),
        OutputFormat::Json => Box::new(JsonReporter),
        OutputFormat::Toon => Box::new(ToonReporter),
        OutputFormat::Toml => Box::new(TomlReporter),
        OutputFormat::Csv => Box::new(CsvReporter),
    }
}

/// High-level writer that runs the P3 transform pipeline and hands the
/// result to either the selected reporter (JSON path) or writes the rendered
/// text verbatim (template path).
///
/// Command handlers call this in place of the old
/// `reporter_for_format(...).report(...)` pair so the `--jq` and
/// `--template` flags are honoured uniformly.
pub fn write_output(
    value: serde_json::Value,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let out = transform::apply(value, transforms)?;
    match out {
        Transformed::Text(s) => {
            let mut stdout = io.stdout();
            stdout.write_all(s.as_bytes())?;
            // Ensure a trailing newline so the terminal prompt lands on a
            // fresh line. Templates that already end in "\n" don't get a
            // doubled newline.
            if !s.ends_with('\n') {
                stdout.write_all(b"\n")?;
            }
            stdout.flush()?;
            Ok(())
        }
        Transformed::Json(v) => {
            // Empty jq result → nothing to print.
            if v.is_null() && transforms.jq.is_some() {
                return Ok(());
            }
            // Snapshot color now so we can hand the reporter out while
            // still holding a mutable borrow on io.stdout().
            let use_color = io.color_enabled();
            let reporter = reporter_for_format(format, use_color);
            let mut stdout = io.stdout();
            reporter.report(&v, &mut stdout)?;
            stdout.flush()?;
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Console,
    Json,
    Toon,
    Toml,
    Csv,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---------------------------------------------------------------
    // Format routing — write_output with Transforms::none()
    // ---------------------------------------------------------------

    #[test]
    fn write_output_json_format() {
        let mut io = IoStreams::test();
        let value = json!({"key": "value", "num": 42});

        write_output(
            value.clone(),
            &OutputFormat::Json,
            &mut io,
            &Transforms::none(),
        )
        .expect("write_output failed");

        let out = io.stdout_as_string();
        let parsed: serde_json::Value =
            serde_json::from_str(&out).expect("output is not valid JSON");
        assert_eq!(
            parsed, value,
            "JSON reporter must round-trip the value unchanged"
        );
        // Pretty-printed JSON contains newlines (not compact single-line)
        assert!(
            out.contains('\n'),
            "expected pretty-printed JSON with newlines, got: {out}"
        );
    }

    #[test]
    fn write_output_console_format() {
        let mut io = IoStreams::test();
        let value = json!({"title": "Hello", "status": "active"});

        write_output(value, &OutputFormat::Console, &mut io, &Transforms::none())
            .expect("write_output failed");

        let out = io.stdout_as_string();
        // Console reporter for objects produces "key: value" lines
        assert!(
            out.contains("title: Hello"),
            "expected 'title: Hello' in console output, got: {out}"
        );
        assert!(
            out.contains("status: active"),
            "expected 'status: active' in console output, got: {out}"
        );
    }

    #[test]
    fn write_output_csv_format() {
        let mut io = IoStreams::test();
        let value = json!([
            {"name": "Alice", "age": 30},
            {"name": "Bob", "age": 25}
        ]);

        write_output(value, &OutputFormat::Csv, &mut io, &Transforms::none())
            .expect("write_output failed");

        let out = io.stdout_as_string();
        let lines: Vec<&str> = out.lines().collect();
        // First line must be the CSV header containing field names
        assert!(
            lines[0].contains("name") && lines[0].contains("age"),
            "expected CSV headers 'name' and 'age' in first line, got: {}",
            lines[0]
        );
        // Data rows follow
        assert!(
            lines.len() >= 3,
            "expected at least 3 lines (header + 2 data rows), got {} lines",
            lines.len()
        );
    }

    #[test]
    fn write_output_toml_format() {
        let mut io = IoStreams::test();
        let value = json!({"name": "test", "count": 7});

        write_output(value, &OutputFormat::Toml, &mut io, &Transforms::none())
            .expect("write_output failed");

        let out = io.stdout_as_string();
        // TOML key-value syntax uses `=`
        assert!(
            out.contains("name = "),
            "expected TOML key-value pair for 'name', got: {out}"
        );
        assert!(
            out.contains("count = "),
            "expected TOML key-value pair for 'count', got: {out}"
        );
    }

    // ---------------------------------------------------------------
    // Transform pipeline — jq and template integration
    // ---------------------------------------------------------------

    #[test]
    fn write_output_jq_filters_before_reporter() {
        let mut io = IoStreams::test();
        let value = json!({"name": "test", "other": 1});
        let transforms = Transforms {
            jq: Some(".name"),
            template: None,
        };

        write_output(value, &OutputFormat::Json, &mut io, &transforms)
            .expect("write_output failed");

        let out = io.stdout_as_string();
        // jq ".name" extracts the string; JSON reporter wraps it in quotes
        assert!(
            out.contains("test"),
            "expected jq-filtered value 'test' in output, got: {out}"
        );
        assert!(
            !out.contains("other"),
            "jq should have filtered out 'other', but output contains it: {out}"
        );
    }

    #[test]
    fn write_output_template_bypasses_reporter() {
        let mut io = IoStreams::test();
        let value = json!({"name": "world"});
        let transforms = Transforms {
            jq: None,
            template: Some("hello {{ name }}"),
        };

        write_output(value, &OutputFormat::Json, &mut io, &transforms)
            .expect("write_output failed");

        let out = io.stdout_as_string();
        // Template output is written verbatim — bypasses the JSON reporter
        assert_eq!(
            out.trim(),
            "hello world",
            "template should render directly, bypassing the JSON reporter"
        );
        // Verify it's NOT JSON-wrapped
        assert!(
            serde_json::from_str::<serde_json::Value>(&out).is_err(),
            "template output must not be valid JSON — it should be plain text"
        );
    }

    #[test]
    fn write_output_jq_null_result_empty() {
        let mut io = IoStreams::test();
        let value = json!({"anything": true});
        let transforms = Transforms {
            jq: Some("empty"),
            template: None,
        };

        write_output(value, &OutputFormat::Json, &mut io, &transforms)
            .expect("write_output failed");

        let out = io.stdout_as_string();
        assert!(
            out.is_empty(),
            "jq `empty` should produce no output, got: {out:?}"
        );
    }

    // ---------------------------------------------------------------
    // Factory — reporter_for_format covers all variants
    // ---------------------------------------------------------------

    #[test]
    fn reporter_for_format_all_variants() {
        let formats = [
            OutputFormat::Console,
            OutputFormat::Json,
            OutputFormat::Toon,
            OutputFormat::Toml,
            OutputFormat::Csv,
        ];
        let value = json!({"key": "value"});

        for format in &formats {
            let reporter = reporter_for_format(format, false);
            let mut buf = Vec::new();
            reporter
                .report(&value, &mut buf)
                .unwrap_or_else(|e| panic!("reporter for {format:?} failed: {e}"));
            assert!(
                !buf.is_empty(),
                "reporter for {format:?} produced empty output for a non-empty value"
            );
        }
    }
}
