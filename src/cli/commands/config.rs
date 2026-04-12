use camino::Utf8Path;

use crate::cli::args::{ConfigSetDefaultsArgs, ConfigSubcommand};
use crate::config::ConfigLoader;
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

pub fn run(
    cmd: &ConfigSubcommand,
    config_path: Option<&Utf8Path>,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    match cmd {
        ConfigSubcommand::List => list(config_path, format, io, transforms),
        ConfigSubcommand::Show(args) => {
            show(config_path, args.name.as_deref(), format, io, transforms)
        }
        ConfigSubcommand::Delete(args) => delete(config_path, &args.name, format, io, transforms),
        ConfigSubcommand::SetDefault(args) => {
            set_default(config_path, &args.name, format, io, transforms)
        }
        ConfigSubcommand::SetDefaults(args) => {
            set_defaults(config_path, args, format, io, transforms)
        }
    }
}

fn list(
    config_path: Option<&Utf8Path>,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let config = ConfigLoader::load(config_path)?
        .ok_or_else(|| anyhow::anyhow!("no config file found; run `atl init` first"))?;

    let mut names: Vec<_> = config.profiles.keys().collect();
    names.sort();

    let items: Vec<serde_json::Value> = names
        .iter()
        .map(|name| {
            serde_json::json!({
                "name": name,
                "is_default": *name == &config.default_profile,
            })
        })
        .collect();
    let value = serde_json::Value::Array(items);

    write_output(value, format, io, transforms)
}

fn show(
    config_path: Option<&Utf8Path>,
    name: Option<&str>,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let config = ConfigLoader::load(config_path)?
        .ok_or_else(|| anyhow::anyhow!("no config file found; run `atl init` first"))?;

    let profile_name = name.unwrap_or(&config.default_profile);
    let profile = config
        .profiles
        .get(profile_name)
        .ok_or_else(|| anyhow::anyhow!("profile '{profile_name}' not found"))?;

    let value = serde_json::json!({
        "name": profile_name,
        "profile": serde_json::to_value(profile)?,
    });

    write_output(value, format, io, transforms)
}

fn delete(
    config_path: Option<&Utf8Path>,
    name: &str,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => ConfigLoader::resolve_config_path()?,
    };

    let mut config = ConfigLoader::load(Some(path.as_path()))?
        .ok_or_else(|| anyhow::anyhow!("no config file found"))?;

    if !config.profiles.contains_key(name) {
        anyhow::bail!("profile '{name}' not found");
    }

    if name == config.default_profile {
        anyhow::bail!("cannot delete the default profile '{name}'; change default_profile first");
    }

    config.profiles.remove(name);

    let content = toml::to_string_pretty(&config)?;
    std::fs::write(path.as_std_path(), content)?;

    let value = serde_json::json!({
        "action": "deleted",
        "profile": name,
    });

    write_output(value, format, io, transforms)
}

fn set_default(
    config_path: Option<&Utf8Path>,
    name: &str,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => ConfigLoader::resolve_config_path()?,
    };

    let mut config = ConfigLoader::load(Some(path.as_path()))?
        .ok_or_else(|| anyhow::anyhow!("no config file found"))?;

    if !config.profiles.contains_key(name) {
        anyhow::bail!("profile '{name}' not found");
    }

    config.default_profile = name.to_string();

    let content = toml::to_string_pretty(&config)?;
    std::fs::write(path.as_std_path(), content)?;

    let value = serde_json::json!({
        "action": "set_default",
        "profile": name,
    });

    write_output(value, format, io, transforms)
}

fn set_defaults(
    config_path: Option<&Utf8Path>,
    args: &ConfigSetDefaultsArgs,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => ConfigLoader::resolve_config_path()?,
    };

    let mut config = ConfigLoader::load(Some(path.as_path()))?
        .ok_or_else(|| anyhow::anyhow!("no config file found"))?;

    let profile_name = args.profile.as_deref().unwrap_or(&config.default_profile);
    let profile_name_owned = profile_name.to_string();
    let profile = config
        .profiles
        .get_mut(&profile_name_owned)
        .ok_or_else(|| anyhow::anyhow!("profile '{profile_name_owned}' not found"))?;

    if let Some(proj) = &args.project {
        profile.default_project = Some(proj.clone());
    }
    if let Some(space) = &args.space {
        profile.default_space = Some(space.clone());
    }
    let project = profile.default_project.clone();
    let space = profile.default_space.clone();

    let content = toml::to_string_pretty(&config)?;
    std::fs::write(path.as_std_path(), content)?;

    let value = serde_json::json!({
        "action": "updated",
        "profile": profile_name_owned,
        "project": project,
        "space": space,
    });

    write_output(value, format, io, transforms)
}
