use std::io::Write;

use serde_json::Value;
use toon_format::encode_default;

use super::Reporter;

pub struct ToonReporter;

impl Reporter for ToonReporter {
    fn report(&self, value: &Value, writer: &mut dyn Write) -> anyhow::Result<()> {
        let encoded = encode_default(value)?;
        write!(writer, "{encoded}")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report_to_string(value: &Value) -> String {
        let reporter = ToonReporter;
        let mut buf = Vec::new();
        reporter.report(value, &mut buf).expect("report failed");
        String::from_utf8(buf).expect("non-UTF-8 output")
    }

    #[test]
    fn object_encodes_as_toon() {
        let output = report_to_string(&json!({"key": "value"}));
        assert!(
            !output.is_empty(),
            "expected non-empty toon output for object"
        );
        assert!(
            output.contains("key"),
            "expected 'key' in toon output, got: {output}"
        );
        assert!(
            output.contains("value"),
            "expected 'value' in toon output, got: {output}"
        );
    }

    #[test]
    fn array_encodes_as_toon() {
        let output = report_to_string(&json!([1, 2, 3]));
        assert!(
            !output.is_empty(),
            "expected non-empty toon output for array"
        );
        assert!(
            output.contains('1'),
            "expected '1' in toon output, got: {output}"
        );
        assert!(
            output.contains('2'),
            "expected '2' in toon output, got: {output}"
        );
        assert!(
            output.contains('3'),
            "expected '3' in toon output, got: {output}"
        );
    }

    #[test]
    fn scalar_encodes_as_toon() {
        let output = report_to_string(&json!("hello"));
        assert!(
            !output.is_empty(),
            "expected non-empty toon output for scalar"
        );
        assert!(
            output.contains("hello"),
            "expected 'hello' in toon output, got: {output}"
        );
    }
}
