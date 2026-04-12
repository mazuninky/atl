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
