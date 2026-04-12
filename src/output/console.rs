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
