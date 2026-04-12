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
