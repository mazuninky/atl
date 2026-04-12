use std::io::Write;

use serde_json::Value;

use super::Reporter;

pub struct JsonReporter;

impl Reporter for JsonReporter {
    fn report(&self, value: &Value, writer: &mut dyn Write) -> anyhow::Result<()> {
        serde_json::to_writer_pretty(writer, value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report_to_string(value: &Value) -> String {
        let reporter = JsonReporter;
        let mut buf = Vec::new();
        reporter.report(value, &mut buf).expect("report failed");
        String::from_utf8(buf).expect("non-UTF-8 output")
    }

    #[test]
    fn json_object_pretty_printed() {
        let output = report_to_string(&json!({"key": "value"}));
        let parsed: Value = serde_json::from_str(&output).expect("output is not valid JSON");
        assert_eq!(parsed, json!({"key": "value"}));
        // Pretty-printed output has newlines and indentation
        assert!(
            output.contains('\n'),
            "expected pretty-printed output with newlines, got: {output}"
        );
        assert!(
            output.contains("  "),
            "expected indented output, got: {output}"
        );
    }

    #[test]
    fn json_array_pretty_printed() {
        let output = report_to_string(&json!([1, 2, 3]));
        let parsed: Value = serde_json::from_str(&output).expect("output is not valid JSON");
        assert_eq!(parsed, json!([1, 2, 3]));
        assert!(
            output.contains('\n'),
            "expected pretty-printed array with newlines, got: {output}"
        );
    }

    #[test]
    fn json_scalar_value() {
        let output = report_to_string(&json!("hello"));
        assert_eq!(output, "\"hello\"");
    }
}
