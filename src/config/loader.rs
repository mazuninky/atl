use camino::{Utf8Path, Utf8PathBuf};
use tracing::debug;

use super::{CONFIG_DIR_NAME, CONFIG_FILE_NAME, Config};
use crate::error::Error;

pub struct ConfigLoader;

impl ConfigLoader {
    pub fn load(explicit_path: Option<&Utf8Path>) -> anyhow::Result<Option<Config>> {
        let path = match explicit_path {
            Some(p) => {
                if p.is_file() {
                    Some(p.to_path_buf())
                } else {
                    return Err(Error::Config(format!("config file not found: {p}")).into());
                }
            }
            None => Self::find_config()?,
        };

        match path {
            Some(p) => {
                debug!("Loading config from {p}");
                let content = std::fs::read_to_string(p.as_std_path())?;
                let config: Config = toml::from_str(&content)?;

                Ok(Some(config))
            }
            None => {
                debug!("No config file found, using defaults");
                Ok(None)
            }
        }
    }

    fn find_config() -> anyhow::Result<Option<Utf8PathBuf>> {
        // Check env var first
        if let Ok(path) = std::env::var("ATL_CONFIG") {
            let p = Utf8PathBuf::from(&path);
            if p.is_file() {
                return Ok(Some(p));
            }
            anyhow::bail!("ATL_CONFIG points to '{path}' which is not a file");
        }

        // Check XDG / platform config dir
        if let Some(config_dir) = dirs::config_dir()
            && let Ok(utf8) = Utf8PathBuf::try_from(config_dir)
        {
            let p = utf8.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME);
            if p.is_file() {
                return Ok(Some(p));
            }
        }

        // Check home dir
        if let Some(home) = dirs::home_dir()
            && let Ok(utf8) = Utf8PathBuf::try_from(home)
        {
            let p = utf8.join(format!(".{CONFIG_FILE_NAME}"));
            if p.is_file() {
                return Ok(Some(p));
            }
        }

        Ok(None)
    }

    /// Resolve the config file path (must exist). Used by config management commands.
    pub fn resolve_config_path() -> anyhow::Result<Utf8PathBuf> {
        if let Ok(path) = std::env::var("ATL_CONFIG") {
            let p = Utf8PathBuf::from(&path);
            if p.is_file() {
                return Ok(p);
            }
            anyhow::bail!("ATL_CONFIG points to '{path}' which is not a file");
        }
        Self::find_config()?
            .ok_or_else(|| anyhow::anyhow!("no config file found; run `atl init` first"))
    }

    pub fn default_config_path() -> Option<Utf8PathBuf> {
        dirs::config_dir().and_then(|d| {
            Utf8PathBuf::try_from(d)
                .ok()
                .map(|p| p.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME))
        })
    }

    /// Serializes `config` as pretty TOML and writes it to disk.
    ///
    /// Path resolution mirrors [`Self::load`]: if `explicit_path` is provided,
    /// it is used verbatim (the parent directory is created if it does not
    /// exist). Otherwise the first existing config file in the standard search
    /// chain is updated. If no config file exists, the platform default path
    /// (`$XDG_CONFIG_HOME/atl/atl.toml` or equivalent) is created.
    ///
    /// Returns the absolute path that was written.
    pub fn save(config: &Config, explicit_path: Option<&Utf8Path>) -> anyhow::Result<Utf8PathBuf> {
        let path = match explicit_path {
            Some(p) => p.to_path_buf(),
            None => match Self::find_config()? {
                Some(p) => p,
                None => Self::default_config_path().ok_or_else(|| {
                    Error::Config("cannot determine default config file path".into())
                })?,
            },
        };

        if let Some(parent) = path.parent()
            && !parent.as_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent.as_std_path())?;
        }

        let content = toml::to_string_pretty(config).map_err(Error::from)?;
        write_config_file(path.as_std_path(), content.as_bytes())?;
        debug!("Wrote config to {path}");
        Ok(path)
    }
}

/// Writes the config file with owner-only permissions on Unix (`0o600`) so
/// any `api_token` field cannot be read by other users on the host.
/// On non-Unix platforms falls back to `std::fs::write`, which uses the
/// default OS-assigned permissions.
fn write_config_file(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(content)?;
        f.flush()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_explicit_path_not_found() {
        let path = Utf8Path::new("/nonexistent/atl.toml");
        let result = ConfigLoader::load(Some(path));
        assert!(result.is_err());
    }

    #[test]
    fn load_explicit_valid_config() {
        let dir = std::env::temp_dir();
        let path = dir.join("atl_test_config.toml");
        std::fs::write(
            &path,
            r#"
default_profile = "test"

[profiles.test]
[profiles.test.jira]
domain = "test.atlassian.net"
email = "test@test.com"
"#,
        )
        .unwrap();

        let utf8_path = Utf8PathBuf::try_from(path.clone()).unwrap();
        let config = ConfigLoader::load(Some(utf8_path.as_path()))
            .unwrap()
            .unwrap();
        assert_eq!(config.default_profile, "test");
        assert!(config.profiles.contains_key("test"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_none_without_config_returns_none() {
        // With no explicit path and no env var, if no config file exists
        // in standard locations, should return None (not error)
        unsafe {
            std::env::remove_var("ATL_CONFIG");
        }
        let result = ConfigLoader::load(None);
        // This may return Some or None depending on whether the user
        // has a real config file — just verify it doesn't error
        assert!(result.is_ok());
    }

    #[test]
    fn default_config_path_returns_some() {
        let path = ConfigLoader::default_config_path();
        assert!(path.is_some());
    }

    #[test]
    fn save_roundtrip_preserves_existing_fields() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let raw = dir.path().join("atl.toml");
        std::fs::write(
            &raw,
            r#"
default_profile = "prod"

[profiles.prod]
default_project = "ACME"

[profiles.prod.jira]
domain = "acme.atlassian.net"
email = "alice@example.com"
api_token = "tok"
"#,
        )
        .unwrap();
        let path = Utf8PathBuf::try_from(raw).unwrap();

        // Load, mutate aliases, save via ConfigLoader::save, reload.
        let mut config = ConfigLoader::load(Some(path.as_path())).unwrap().unwrap();
        config
            .aliases
            .insert("myq".to_string(), "jira me".to_string());
        let written = ConfigLoader::save(&config, Some(path.as_path())).unwrap();
        assert_eq!(written, path);

        let reloaded = ConfigLoader::load(Some(path.as_path())).unwrap().unwrap();
        assert_eq!(reloaded.default_profile, "prod");
        assert!(reloaded.profiles.contains_key("prod"));
        let profile = reloaded.profiles.get("prod").unwrap();
        assert_eq!(profile.default_project.as_deref(), Some("ACME"));
        let jira = profile.jira.as_ref().unwrap();
        assert_eq!(jira.domain, "acme.atlassian.net");
        assert_eq!(jira.email.as_deref(), Some("alice@example.com"));
        assert_eq!(jira.api_token.as_deref(), Some("tok"));
        assert_eq!(
            reloaded.aliases.get("myq").map(String::as_str),
            Some("jira me")
        );
    }

    #[test]
    fn save_creates_parent_directory() {
        use std::collections::HashMap;

        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("subdir").join("atl.toml");
        let path = Utf8PathBuf::try_from(nested).unwrap();
        let mut aliases = HashMap::new();
        aliases.insert("foo".to_string(), "jira me".to_string());
        let config = Config {
            default_profile: "default".to_string(),
            aliases,
            ..Default::default()
        };
        let written = ConfigLoader::save(&config, Some(path.as_path())).unwrap();
        assert_eq!(written, path);
        assert!(path.exists());
    }
}
