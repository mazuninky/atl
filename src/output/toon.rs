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
