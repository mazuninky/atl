use std::io::Write;

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use super::Reporter;

pub struct ConsoleReporter {
    use_color: bool,
}

impl ConsoleReporter {
    pub fn new(use_color: bool) -> Self {
        Self { use_color }
    }

    fn format_value(&self, value: &Value, writer: &mut dyn Write) -> anyhow::Result<()> {
        match value {
            Value::Array(arr) if !arr.is_empty() && arr.iter().all(Value::is_object) => {
                self.format_table(arr, writer)
            }
            Value::Object(map) => {
                for (key, val) in map {
                    let display = match val {
                        Value::String(s) => s.clone(),
                        Value::Null => "null".to_string(),
                        other => other.to_string(),
                    };
                    if self.use_color {
                        writeln!(writer, "\x1b[1m{key}\x1b[0m: {display}")?;
                    } else {
                        writeln!(writer, "{key}: {display}")?;
                    }
                }
                Ok(())
            }
            Value::Array(arr) => {
                for item in arr {
                    let display = match item {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    writeln!(writer, "- {display}")?;
                }
                Ok(())
            }
            other => {
                writeln!(writer, "{other}")?;
                Ok(())
            }
        }
    }

    fn format_table(&self, items: &[Value], writer: &mut dyn Write) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }

        // Union headers across all objects to handle sparse responses
        let mut seen = std::collections::HashSet::new();
        let mut headers: Vec<String> = Vec::new();
        for item in items {
            if let Value::Object(map) = item {
                for key in map.keys() {
                    if seen.insert(key.clone()) {
                        headers.push(key.clone());
                    }
                }
            }
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL_CONDENSED)
            .set_content_arrangement(ContentArrangement::Dynamic);

        table.set_header(&headers);

        for item in items {
            if let Value::Object(map) = item {
                let row: Vec<String> = headers
                    .iter()
                    .map(|h| {
                        map.get(h.as_str())
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                Value::Null => String::new(),
                                other => other.to_string(),
                            })
                            .unwrap_or_default()
                    })
                    .collect();
                table.add_row(row);
            }
        }

        writeln!(writer, "{table}")?;
        Ok(())
    }
}

impl Reporter for ConsoleReporter {
    fn report(&self, value: &Value, writer: &mut dyn Write) -> anyhow::Result<()> {
        self.format_value(value, writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report_to_string(value: &Value, use_color: bool) -> String {
        let reporter = ConsoleReporter::new(use_color);
        let mut buf = Vec::new();
        reporter.report(value, &mut buf).expect("report failed");
        String::from_utf8(buf).expect("non-UTF-8 output")
    }

    #[test]
    fn object_renders_key_value_pairs() {
        let output = report_to_string(&json!({"name": "Alice", "age": 30}), false);
        assert!(
            output.contains("name: Alice"),
            "expected 'name: Alice' in output, got: {output}"
        );
        assert!(
            output.contains("age: 30"),
            "expected 'age: 30' in output, got: {output}"
        );
    }

    #[test]
    fn object_renders_null_as_null_string() {
        let output = report_to_string(&json!({"x": null}), false);
        assert!(
            output.contains("x: null"),
            "expected 'x: null' in output, got: {output}"
        );
    }

    #[test]
    fn object_with_color_wraps_keys_in_ansi() {
        let output = report_to_string(&json!({"name": "Alice"}), true);
        assert!(
            output.contains("\x1b["),
            "expected ANSI escape codes in colored output, got: {output}"
        );
        assert!(
            output.contains("\x1b[1m"),
            "expected bold ANSI code for key, got: {output}"
        );
    }

    #[test]
    fn object_without_color_no_ansi() {
        let output = report_to_string(&json!({"name": "Alice"}), false);
        assert!(
            !output.contains("\x1b["),
            "expected no ANSI escape codes in uncolored output, got: {output}"
        );
    }

    #[test]
    fn array_of_objects_renders_table() {
        let output = report_to_string(&json!([{"id": "1", "name": "a"}]), false);
        // Dynamic table arrangement may wrap headers in narrow terminals,
        // so we only assert that the data values are present.
        assert!(
            output.contains("1"),
            "expected value '1' in table output, got: {output}"
        );
        assert!(
            output.contains("a"),
            "expected value 'a' in table output, got: {output}"
        );
    }

    #[test]
    fn array_of_objects_union_headers() {
        let output = report_to_string(&json!([{"a": 1}, {"b": 2}]), false);
        assert!(
            output.contains("a"),
            "expected header 'a' from first object, got: {output}"
        );
        assert!(
            output.contains("b"),
            "expected header 'b' from second object, got: {output}"
        );
    }

    #[test]
    fn plain_array_renders_bullet_list() {
        let output = report_to_string(&json!(["x", "y"]), false);
        assert!(
            output.contains("- x"),
            "expected bullet '- x' in output, got: {output}"
        );
        assert!(
            output.contains("- y"),
            "expected bullet '- y' in output, got: {output}"
        );
    }

    #[test]
    fn scalar_renders_directly() {
        let output = report_to_string(&json!(42), false);
        assert_eq!(output, "42\n");
    }
}
