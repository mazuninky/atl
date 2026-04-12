use std::io::Write;

use serde_json::Value;

use super::Reporter;

pub struct TomlReporter;

impl Reporter for TomlReporter {
    fn report(&self, value: &Value, writer: &mut dyn Write) -> anyhow::Result<()> {
        let sanitized = strip_nulls(value);
        let toml_str = toml::to_string_pretty(&sanitized)?;
        write!(writer, "{toml_str}")?;
        Ok(())
    }
}

/// Recursively strip null values since the TOML crate cannot serialize them.
fn strip_nulls(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let filtered: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect();
            Value::Object(filtered)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(strip_nulls).collect()),
        other => other.clone(),
    }
}
