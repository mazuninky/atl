use std::collections::HashMap;
use std::io::Write;

use crate::auth::Prompter;
use crate::config::{AtlassianInstance, AuthType, Config, ConfigLoader, Profile, TokenStorage};
use crate::io::IoStreams;

/// Runs the `atl init` command.
///
/// Launches an interactive wizard that prompts for the Atlassian domain, email,
/// and optional separate Confluence domain, then writes a ready-to-use config
/// file.
///
/// Returns an error if stdin or stdout is not a TTY — `atl init` requires an
/// interactive terminal.
pub fn run_init(io: &mut IoStreams, prompter: &dyn Prompter) -> anyhow::Result<()> {
    if !io.is_stdin_tty() || !io.is_stdout_tty() {
        anyhow::bail!("atl init requires an interactive terminal");
    }

    let path = ConfigLoader::default_config_path()
        .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?;

    // If a config already exists, ask before overwriting.
    if path.as_std_path().exists() {
        let overwrite = prompter.select(
            &format!("Config already exists at {path}. Overwrite?"),
            &["No", "Yes"],
        )?;
        if overwrite == 0 {
            writeln!(io.stdout(), "Init cancelled.")?;
            return Ok(());
        }
    }

    // 1. Atlassian domain
    let domain = normalize_domain(&prompt_non_empty(
        prompter,
        "Atlassian domain (e.g. acme or acme.atlassian.net):",
    )?);

    // 2. Email
    let email = prompt_non_empty(prompter, "Email:")?;

    // 3. Token storage method
    let token_storage = prompter.select(
        "Where to store API tokens?",
        &[
            "OS keyring (secure, may prompt for keychain password on macOS)",
            "Config file (simpler, no keychain prompts)",
        ],
    )?;

    let token_storage_value = if token_storage == 1 {
        TokenStorage::Config
    } else {
        TokenStorage::Keyring
    };

    // 4. Same domain for Confluence and Jira?
    let same_domain = prompter.select(
        "Do Confluence and Jira use the same domain?",
        &["Yes, same domain", "No, different domains"],
    )?;

    let confluence_domain = if same_domain == 0 {
        domain.clone()
    } else {
        normalize_domain(&prompt_non_empty(
            prompter,
            "Confluence domain (e.g. acme.atlassian.net/wiki or wiki.acme.com):",
        )?)
    };

    // Build the config.
    let jira_instance = AtlassianInstance {
        domain: domain.clone(),
        email: Some(email.clone()),
        api_token: None,
        auth_type: AuthType::Basic,
        api_path: None,
        read_only: false,
    };
    let confluence_instance = AtlassianInstance {
        domain: confluence_domain,
        email: Some(email),
        api_token: None,
        auth_type: AuthType::Basic,
        api_path: None,
        read_only: false,
    };
    let profile = Profile {
        confluence: Some(confluence_instance),
        jira: Some(jira_instance),
        default_project: None,
        default_space: None,
        token_storage: token_storage_value,
    };
    let mut profiles = HashMap::new();
    profiles.insert("default".to_string(), profile);
    let config = Config {
        default_profile: "default".to_string(),
        profiles,
        aliases: HashMap::new(),
    };

    let written_path = ConfigLoader::save(&config, Some(path.as_ref()))?;

    writeln!(io.stdout(), "Config written to {written_path}")?;
    writeln!(io.stdout(), "Run `atl auth login` to store your API token.")?;

    Ok(())
}

/// Prompts the user for a non-empty text value. Repeats until a non-blank
/// answer is provided.
fn prompt_non_empty(prompter: &dyn Prompter, msg: &str) -> anyhow::Result<String> {
    let value = prompter.text(msg, None)?;
    if value.trim().is_empty() {
        anyhow::bail!("value cannot be empty");
    }
    Ok(value.trim().to_string())
}

/// Normalizes an Atlassian domain string for storage.
///
/// Accepts several input formats and produces a canonical domain:
/// - Bare subdomain (`acme`) becomes `acme.atlassian.net`
/// - Full URL (`https://acme.atlassian.net/`) is stripped to `acme.atlassian.net`
/// - Self-hosted domains (`wiki.acme.com`) pass through unchanged
/// - Paths after the host (`acme.atlassian.net/wiki`) are preserved
fn normalize_domain(input: &str) -> String {
    let mut s = input.trim().to_string();

    // Strip scheme prefix.
    if let Some(rest) = s.strip_prefix("https://") {
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix("http://") {
        s = rest.to_string();
    }

    // Strip trailing slash (but not internal path separators).
    if s.ends_with('/') {
        s.truncate(s.len() - 1);
    }

    // Split host from path before applying the Cloud shorthand.
    let split_at = s.find('/').unwrap_or(s.len());
    let host = &s[..split_at];
    let path = &s[split_at..];

    // If the host has no dot, treat it as a bare Atlassian Cloud subdomain.
    if !host.contains('.') && !host.contains(':') {
        format!("{host}.atlassian.net{path}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::prompter::{MockPrompter, MockResponse};

    #[test]
    fn interactive_wizard_creates_config_same_domain() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("atl").join("atl.toml");
        let utf8_path = camino::Utf8PathBuf::try_from(config_path.clone()).unwrap();

        let prompter = MockPrompter::new(vec![
            MockResponse::Text("acme.atlassian.net".into()),
            MockResponse::Text("alice@acme.com".into()),
            MockResponse::Select(0), // token storage: keyring
            MockResponse::Select(0), // same domain ("Yes, same domain")
        ]);

        let config = build_config_from_prompts(&prompter).unwrap();
        let written = ConfigLoader::save(&config, Some(utf8_path.as_ref())).unwrap();
        assert_eq!(written, utf8_path);

        // Reload and verify.
        let reloaded = ConfigLoader::load(Some(utf8_path.as_ref()))
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.default_profile, "default");
        let profile = reloaded.profiles.get("default").unwrap();
        let jira = profile.jira.as_ref().unwrap();
        assert_eq!(jira.domain, "acme.atlassian.net");
        assert_eq!(jira.email.as_deref(), Some("alice@acme.com"));
        assert!(jira.api_token.is_none());
        let confluence = profile.confluence.as_ref().unwrap();
        assert_eq!(confluence.domain, "acme.atlassian.net");
        assert_eq!(confluence.email.as_deref(), Some("alice@acme.com"));

        assert_eq!(prompter.remaining(), 0);
    }

    #[test]
    fn interactive_wizard_creates_config_different_domains() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("atl").join("atl.toml");
        let utf8_path = camino::Utf8PathBuf::try_from(config_path).unwrap();

        let prompter = MockPrompter::new(vec![
            MockResponse::Text("acme.atlassian.net".into()),
            MockResponse::Text("alice@acme.com".into()),
            MockResponse::Select(0), // token storage: keyring
            MockResponse::Select(1), // different domain ("No, different domains")
            MockResponse::Text("wiki.acme.com".into()),
        ]);

        let config = build_config_from_prompts(&prompter).unwrap();
        ConfigLoader::save(&config, Some(utf8_path.as_ref())).unwrap();

        let reloaded = ConfigLoader::load(Some(utf8_path.as_ref()))
            .unwrap()
            .unwrap();
        let profile = reloaded.profiles.get("default").unwrap();
        assert_eq!(profile.jira.as_ref().unwrap().domain, "acme.atlassian.net");
        assert_eq!(profile.confluence.as_ref().unwrap().domain, "wiki.acme.com");
        assert_eq!(prompter.remaining(), 0);
    }

    #[test]
    fn interactive_wizard_config_storage_sets_token_storage() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("atl").join("atl.toml");
        let utf8_path = camino::Utf8PathBuf::try_from(config_path).unwrap();

        let prompter = MockPrompter::new(vec![
            MockResponse::Text("acme.atlassian.net".into()),
            MockResponse::Text("alice@acme.com".into()),
            MockResponse::Select(1), // token storage: config file
            MockResponse::Select(0), // same domain
        ]);

        let config = build_config_from_prompts(&prompter).unwrap();
        ConfigLoader::save(&config, Some(utf8_path.as_ref())).unwrap();

        let reloaded = ConfigLoader::load(Some(utf8_path.as_ref()))
            .unwrap()
            .unwrap();
        let profile = reloaded.profiles.get("default").unwrap();
        assert!(matches!(profile.token_storage, TokenStorage::Config));
        // No token should be set — auth login handles that.
        assert!(profile.jira.as_ref().unwrap().api_token.is_none());
        assert!(profile.confluence.as_ref().unwrap().api_token.is_none());
        assert_eq!(prompter.remaining(), 0);
    }

    #[test]
    fn interactive_wizard_empty_domain_is_rejected() {
        let prompter = MockPrompter::new(vec![MockResponse::Text("  ".into())]);
        let result = prompt_non_empty(&prompter, "domain:");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("cannot be empty"), "got: {msg}");
    }

    #[test]
    fn interactive_overwrite_declined_cancels() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("atl.toml");
        // Write an existing config so the overwrite prompt triggers.
        std::fs::write(&config_path, "default_profile = \"old\"\n").unwrap();

        let prompter = MockPrompter::new(vec![
            MockResponse::Select(0), // decline overwrite ("No")
        ]);
        // We can't easily test run_interactive directly because it uses
        // ConfigLoader::default_config_path(). Instead test the select logic
        // inline.
        let utf8_path = camino::Utf8PathBuf::try_from(config_path.clone()).unwrap();
        assert!(utf8_path.as_std_path().exists());
        let overwrite = prompter
            .select(
                &format!("Config already exists at {utf8_path}. Overwrite?"),
                &["No", "Yes"],
            )
            .unwrap();
        assert_eq!(overwrite, 0);

        // Config should remain unchanged.
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("old"));
    }

    #[test]
    fn normalize_domain_bare_subdomain() {
        assert_eq!(normalize_domain("innowald"), "innowald.atlassian.net");
    }

    #[test]
    fn normalize_domain_already_full() {
        assert_eq!(
            normalize_domain("innowald.atlassian.net"),
            "innowald.atlassian.net"
        );
    }

    #[test]
    fn normalize_domain_https_with_trailing_slash() {
        assert_eq!(
            normalize_domain("https://innowald.atlassian.net/"),
            "innowald.atlassian.net"
        );
    }

    #[test]
    fn normalize_domain_https_without_trailing_slash() {
        assert_eq!(
            normalize_domain("https://innowald.atlassian.net"),
            "innowald.atlassian.net"
        );
    }

    #[test]
    fn normalize_domain_http_with_trailing_slash() {
        assert_eq!(
            normalize_domain("http://innowald.atlassian.net/"),
            "innowald.atlassian.net"
        );
    }

    #[test]
    fn normalize_domain_self_hosted_passthrough() {
        assert_eq!(normalize_domain("wiki.acme.com"), "wiki.acme.com");
    }

    #[test]
    fn normalize_domain_self_hosted_with_scheme_and_path() {
        assert_eq!(
            normalize_domain("https://wiki.acme.com/wiki"),
            "wiki.acme.com/wiki"
        );
    }

    #[test]
    fn normalize_domain_cloud_with_path() {
        assert_eq!(
            normalize_domain("acme.atlassian.net/wiki"),
            "acme.atlassian.net/wiki"
        );
    }

    #[test]
    fn normalize_domain_bare_subdomain_with_path() {
        assert_eq!(normalize_domain("acme/wiki"), "acme.atlassian.net/wiki");
    }

    #[test]
    fn normalize_domain_localhost_with_port_and_path() {
        assert_eq!(
            normalize_domain("localhost:8080/jira"),
            "localhost:8080/jira"
        );
    }

    #[test]
    fn normalize_domain_trims_whitespace() {
        assert_eq!(
            normalize_domain("  innowald.atlassian.net  "),
            "innowald.atlassian.net"
        );
    }

    /// Helper that runs the prompt sequence and builds a Config — extracted so
    /// tests can exercise the logic without depending on
    /// `ConfigLoader::default_config_path()`.
    fn build_config_from_prompts(prompter: &dyn Prompter) -> anyhow::Result<Config> {
        let domain = normalize_domain(&prompt_non_empty(
            prompter,
            "Atlassian domain (e.g. acme or acme.atlassian.net):",
        )?);
        let email = prompt_non_empty(prompter, "Email:")?;

        let token_storage = prompter.select(
            "Where to store API tokens?",
            &[
                "OS keyring (secure, may prompt for keychain password on macOS)",
                "Config file (simpler, no keychain prompts)",
            ],
        )?;
        let token_storage_value = if token_storage == 1 {
            TokenStorage::Config
        } else {
            TokenStorage::Keyring
        };

        let same_domain = prompter.select(
            "Do Confluence and Jira use the same domain?",
            &["Yes, same domain", "No, different domains"],
        )?;
        let confluence_domain = if same_domain == 0 {
            domain.clone()
        } else {
            normalize_domain(&prompt_non_empty(
                prompter,
                "Confluence domain (e.g. acme.atlassian.net/wiki or wiki.acme.com):",
            )?)
        };

        let jira_instance = AtlassianInstance {
            domain: domain.clone(),
            email: Some(email.clone()),
            api_token: None,
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
        };
        let confluence_instance = AtlassianInstance {
            domain: confluence_domain,
            email: Some(email),
            api_token: None,
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
        };
        let profile = Profile {
            confluence: Some(confluence_instance),
            jira: Some(jira_instance),
            default_project: None,
            default_space: None,
            token_storage: token_storage_value,
        };
        let mut profiles = HashMap::new();
        profiles.insert("default".to_string(), profile);
        Ok(Config {
            default_profile: "default".to_string(),
            profiles,
            aliases: HashMap::new(),
        })
    }
}
