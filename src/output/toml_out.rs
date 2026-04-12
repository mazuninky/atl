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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report_to_string(value: &Value) -> String {
        let reporter = TomlReporter;
        let mut buf = Vec::new();
        reporter.report(value, &mut buf).expect("report failed");
        String::from_utf8(buf).expect("non-UTF-8 output")
    }

    #[test]
    fn object_serializes_as_toml() {
        let output = report_to_string(&json!({"key": "value"}));
        assert!(
            output.contains("key = \"value\""),
            "expected TOML key-value pair, got: {output}"
        );
    }

    #[test]
    fn nested_object_serializes() {
        let output = report_to_string(&json!({"section": {"inner": 1}}));
        assert!(
            output.contains("[section]"),
            "expected TOML section header, got: {output}"
        );
        assert!(
            output.contains("inner = 1"),
            "expected nested key-value pair, got: {output}"
        );
    }

    #[test]
    fn null_values_stripped() {
        let output = report_to_string(&json!({"keep": 1, "drop": null}));
        assert!(
            output.contains("keep"),
            "expected 'keep' to be preserved, got: {output}"
        );
        assert!(
            !output.contains("drop"),
            "expected 'drop' (null) to be stripped, got: {output}"
        );
    }

    #[test]
    fn nested_null_stripped_recursively() {
        let output = report_to_string(&json!({
            "outer": {
                "keep": "yes",
                "remove": null
            }
        }));
        assert!(
            output.contains("keep"),
            "expected nested 'keep' to be preserved, got: {output}"
        );
        assert!(
            !output.contains("remove"),
            "expected nested 'remove' (null) to be stripped, got: {output}"
        );
    }

    #[test]
    fn array_at_root_errors() {
        let reporter = TomlReporter;
        let mut buf = Vec::new();
        let result = reporter.report(&json!([1, 2, 3]), &mut buf);
        assert!(
            result.is_err(),
            "expected error when serializing a bare array as TOML, but got Ok"
        );
    }
}
