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
