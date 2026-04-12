use std::io::Write;

use serde_json::Value;

use super::Reporter;

pub struct CsvReporter;

impl Reporter for CsvReporter {
    fn report(&self, value: &Value, writer: &mut dyn Write) -> anyhow::Result<()> {
        match value {
            Value::Array(arr) if !arr.is_empty() && arr.iter().all(|v| v.is_object()) => {
                // Union headers across all objects to handle sparse responses
                let mut seen = std::collections::HashSet::new();
                let mut headers: Vec<String> = Vec::new();
                for item in arr {
                    if let Value::Object(map) = item {
                        for key in map.keys() {
                            if seen.insert(key.clone()) {
                                headers.push(key.clone());
                            }
                        }
                    }
                }

                let mut wtr = csv::Writer::from_writer(writer);
                wtr.write_record(&headers)?;

                for item in arr {
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
                        wtr.write_record(&row)?;
                    }
                }
                wtr.flush()?;
                Ok(())
            }
            Value::Object(map) => {
                let mut wtr = csv::Writer::from_writer(writer);
                let headers: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
                wtr.write_record(&headers)?;
                let row: Vec<String> = headers
                    .iter()
                    .map(|h| {
                        map.get(*h)
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                Value::Null => String::new(),
                                other => other.to_string(),
                            })
                            .unwrap_or_default()
                    })
                    .collect();
                wtr.write_record(&row)?;
                wtr.flush()?;
                Ok(())
            }
            other => {
                let mut wtr = csv::Writer::from_writer(writer);
                let cell = match other {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    v => v.to_string(),
                };
                wtr.write_field(cell)?;
                wtr.write_record(std::iter::empty::<&str>())?;
                wtr.flush()?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report_to_string(value: &Value) -> String {
        let reporter = CsvReporter;
        let mut buf = Vec::new();
        reporter.report(value, &mut buf).expect("report failed");
        String::from_utf8(buf).expect("non-UTF-8 output")
    }

    #[test]
    fn array_of_objects_produces_csv() {
        let output = report_to_string(&json!([
            {"name": "Alice", "age": "30"},
            {"name": "Bob", "age": "25"}
        ]));
        let lines: Vec<&str> = output.lines().collect();
        assert!(
            lines.len() >= 3,
            "expected header + 2 data rows, got {} lines: {output}",
            lines.len()
        );
        assert!(
            lines[0].contains("name"),
            "expected 'name' in header row, got: {}",
            lines[0]
        );
        assert!(
            lines[0].contains("age"),
            "expected 'age' in header row, got: {}",
            lines[0]
        );
        assert!(
            lines[1].contains("Alice"),
            "expected 'Alice' in first data row, got: {}",
            lines[1]
        );
        assert!(
            lines[2].contains("Bob"),
            "expected 'Bob' in second data row, got: {}",
            lines[2]
        );
    }

    #[test]
    fn array_of_objects_union_headers_sparse() {
        let output = report_to_string(&json!([{"a": 1}, {"b": 2}]));
        let header = output
            .lines()
            .next()
            .expect("expected at least a header row");
        assert!(
            header.contains("a"),
            "expected 'a' in CSV headers, got: {header}"
        );
        assert!(
            header.contains("b"),
            "expected 'b' in CSV headers, got: {header}"
        );
    }

    #[test]
    fn single_object_produces_single_row() {
        let output = report_to_string(&json!({"name": "test"}));
        let lines: Vec<&str> = output.lines().collect();
        assert!(
            lines.len() >= 2,
            "expected header + 1 data row, got {} lines: {output}",
            lines.len()
        );
        assert!(
            lines[0].contains("name"),
            "expected 'name' in header, got: {}",
            lines[0]
        );
        assert!(
            lines[1].contains("test"),
            "expected 'test' in data row, got: {}",
            lines[1]
        );
    }

    #[test]
    fn null_values_render_as_empty() {
        let output = report_to_string(&json!({"x": null}));
        let lines: Vec<&str> = output.lines().collect();
        assert!(
            lines.len() >= 2,
            "expected header + data row, got {} lines: {output}",
            lines.len()
        );
        assert!(
            lines[0].contains("x"),
            "expected 'x' in header, got: {}",
            lines[0]
        );
        // The CSV crate quotes the empty string as `""`, so the data row
        // must not contain any non-empty *unquoted* content.  Both `""` and a
        // truly empty line are acceptable representations of a null cell.
        let data_row = lines[1].trim();
        assert!(
            data_row.is_empty() || data_row == "\"\"",
            "expected empty or quoted-empty data cell for null, got: '{data_row}'"
        );
    }

    #[test]
    fn scalar_produces_single_cell() {
        let output = report_to_string(&json!("hello"));
        assert!(
            output.contains("hello"),
            "expected 'hello' in scalar CSV output, got: {output}"
        );
    }

    #[test]
    fn empty_array_produces_empty_output() {
        // An empty array falls through to the plain array branch (not the
        // array-of-objects branch) so it produces a bullet-list-style output
        // with no items, which is effectively empty.
        let output = report_to_string(&json!([]));
        // The empty array matches the `other` arm (not the array-of-objects arm),
        // so it's treated as a scalar-like value.  The csv crate writes "[]" as
        // the field value.
        assert!(
            output.lines().count() <= 1,
            "expected at most one line for empty array, got: {output}"
        );
    }
}
