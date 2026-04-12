use std::io::Write;

use tracing::info;

use crate::config::{self, ConfigLoader};
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

pub fn run_init(
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let path = ConfigLoader::default_config_path()
        .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?;

    if path.as_std_path().exists() {
        anyhow::bail!("config already exists at {path}; edit it directly or delete to re-init");
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent.as_std_path())?;
    }

    let mut file = std::fs::File::create(path.as_std_path())?;
    file.write_all(config::default_config().as_bytes())?;

    info!("Config written to {path}");

    let value = serde_json::json!({
        "status": "created",
        "path": path.as_str(),
        "message": "Edit the file to add your Atlassian credentials"
    });

    write_output(value, format, io, transforms)
}
