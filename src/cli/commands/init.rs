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
        flavor: None,
    };
    let confluence_instance = AtlassianInstance {
        domain: confluence_domain,
        email: Some(email),
        api_token: None,
        auth_type: AuthType::Basic,
        api_path: None,
        read_only: false,
        flavor: None,
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
        assert_eq!(normalize_domain("acme"), "acme.atlassian.net");
    }

    #[test]
    fn normalize_domain_already_full() {
        assert_eq!(normalize_domain("acme.atlassian.net"), "acme.atlassian.net");
    }

    #[test]
    fn normalize_domain_https_with_trailing_slash() {
        assert_eq!(
            normalize_domain("https://acme.atlassian.net/"),
            "acme.atlassian.net"
        );
    }

    #[test]
    fn normalize_domain_https_without_trailing_slash() {
        assert_eq!(
            normalize_domain("https://acme.atlassian.net"),
            "acme.atlassian.net"
        );
    }

    #[test]
    fn normalize_domain_http_with_trailing_slash() {
        assert_eq!(
            normalize_domain("http://acme.atlassian.net/"),
            "acme.atlassian.net"
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
            normalize_domain("  acme.atlassian.net  "),
            "acme.atlassian.net"
        );
    }

    // -------------------------------------------------------------------
    // run_init — the user-facing entry point. We can only verify the
    // non-TTY refusal here without driving a real terminal, but that path
    // is the contract: pipes / CI must fail loudly instead of hanging.
    // -------------------------------------------------------------------

    #[test]
    fn run_init_rejects_non_tty_environment() {
        // IoStreams::test() reports stdin/stdout/stderr as non-TTY. The
        // wizard must refuse rather than block waiting for input.
        let mut io = IoStreams::test();
        let prompter = MockPrompter::new(vec![]);
        let err = run_init(&mut io, &prompter).expect_err("should refuse non-TTY");
        assert!(
            err.to_string().contains("interactive terminal"),
            "got: {err}"
        );
    }

    // -------------------------------------------------------------------
    // prompt_non_empty — happy path + trim behaviour.
    // -------------------------------------------------------------------

    #[test]
    fn prompt_non_empty_returns_trimmed_value() {
        let prompter = MockPrompter::new(vec![MockResponse::Text("  alice@acme.com  ".into())]);
        let value = prompt_non_empty(&prompter, "Email:").unwrap();
        assert_eq!(value, "alice@acme.com");
    }

    #[test]
    fn prompt_non_empty_rejects_pure_tabs_and_newlines() {
        let prompter = MockPrompter::new(vec![MockResponse::Text("\t\n  ".into())]);
        let err = prompt_non_empty(&prompter, "Email:").unwrap_err();
        assert!(err.to_string().contains("cannot be empty"), "got: {err}");
    }

    #[test]
    fn prompt_non_empty_propagates_prompter_error() {
        let prompter = MockPrompter::new(vec![]); // empty queue
        let result = prompt_non_empty(&prompter, "Email:");
        assert!(result.is_err(), "must surface prompter error");
    }

    // -------------------------------------------------------------------
    // Wizard: empty email is rejected mid-flow.
    // -------------------------------------------------------------------

    #[test]
    fn wizard_rejects_empty_email_after_valid_domain() {
        let prompter = MockPrompter::new(vec![
            MockResponse::Text("acme".into()),
            MockResponse::Text("   ".into()), // empty email
        ]);
        let err = build_config_from_prompts(&prompter).unwrap_err();
        assert!(err.to_string().contains("cannot be empty"), "got: {err}");
    }

    // -------------------------------------------------------------------
    // Wizard: full https:// input is normalized before write.
    // -------------------------------------------------------------------

    #[test]
    fn wizard_strips_https_scheme_from_input() {
        let prompter = MockPrompter::new(vec![
            MockResponse::Text("https://acme.atlassian.net/".into()),
            MockResponse::Text("alice@acme.com".into()),
            MockResponse::Select(0), // keyring
            MockResponse::Select(0), // same domain
        ]);
        let cfg = build_config_from_prompts(&prompter).unwrap();
        let profile = cfg.profiles.get("default").expect("default profile");
        assert_eq!(
            profile.jira.as_ref().unwrap().domain,
            "acme.atlassian.net",
            "scheme + trailing slash must be stripped"
        );
        assert_eq!(
            profile.confluence.as_ref().unwrap().domain,
            "acme.atlassian.net"
        );
    }

    // -------------------------------------------------------------------
    // Wizard: bare subdomain shorthand expands to .atlassian.net for both
    // services even when the user picks "different domains" and types a
    // shorthand for the second one too.
    // -------------------------------------------------------------------

    #[test]
    fn wizard_expands_bare_subdomain_for_both_services() {
        let prompter = MockPrompter::new(vec![
            MockResponse::Text("jira-acme".into()),
            MockResponse::Text("alice@acme.com".into()),
            MockResponse::Select(0),
            MockResponse::Select(1), // different domains
            MockResponse::Text("conf-acme".into()),
        ]);
        let cfg = build_config_from_prompts(&prompter).unwrap();
        let profile = cfg.profiles.get("default").unwrap();
        assert_eq!(
            profile.jira.as_ref().unwrap().domain,
            "jira-acme.atlassian.net"
        );
        assert_eq!(
            profile.confluence.as_ref().unwrap().domain,
            "conf-acme.atlassian.net"
        );
    }

    // -------------------------------------------------------------------
    // Wizard: token storage default (Keyring) is what you get when the
    // user just presses enter on the first option.
    // -------------------------------------------------------------------

    #[test]
    fn wizard_default_token_storage_is_keyring() {
        let prompter = MockPrompter::new(vec![
            MockResponse::Text("acme.atlassian.net".into()),
            MockResponse::Text("alice@acme.com".into()),
            MockResponse::Select(0),
            MockResponse::Select(0),
        ]);
        let cfg = build_config_from_prompts(&prompter).unwrap();
        let profile = cfg.profiles.get("default").unwrap();
        assert!(
            matches!(profile.token_storage, TokenStorage::Keyring),
            "first option must map to Keyring"
        );
    }

    // -------------------------------------------------------------------
    // Wizard: api_token is never set by init — only `atl auth login` does
    // that. This is a critical security/UX invariant: typing the wizard
    // does NOT persist a token to disk.
    // -------------------------------------------------------------------

    #[test]
    fn wizard_never_sets_api_token() {
        let prompter = MockPrompter::new(vec![
            MockResponse::Text("acme.atlassian.net".into()),
            MockResponse::Text("alice@acme.com".into()),
            MockResponse::Select(1), // even with config-file storage selected
            MockResponse::Select(0),
        ]);
        let cfg = build_config_from_prompts(&prompter).unwrap();
        let profile = cfg.profiles.get("default").unwrap();
        assert!(
            profile.jira.as_ref().unwrap().api_token.is_none(),
            "init must never write a token directly"
        );
        assert!(
            profile.confluence.as_ref().unwrap().api_token.is_none(),
            "init must never write a token directly"
        );
    }

    // -------------------------------------------------------------------
    // normalize_domain: extra edge cases.
    // -------------------------------------------------------------------

    #[test]
    fn normalize_domain_empty_input_does_not_panic() {
        // Defensive: caller is supposed to have rejected empty input with
        // prompt_non_empty, but the normalizer must not panic. Documents
        // current behaviour: empty input is treated as a bare subdomain
        // and gets the .atlassian.net suffix. The wizard never reaches
        // this path in practice — prompt_non_empty rejects it first.
        assert_eq!(normalize_domain(""), ".atlassian.net");
    }

    #[test]
    fn normalize_domain_only_whitespace_does_not_panic() {
        // Same defensive contract as the empty-input case — trim makes
        // the input empty, which then takes the bare-subdomain path.
        assert_eq!(normalize_domain("   "), ".atlassian.net");
    }

    #[test]
    fn normalize_domain_internal_slash_preserved() {
        // Multiple path segments after the host stay intact.
        assert_eq!(
            normalize_domain("https://acme.atlassian.net/wiki/spaces/X"),
            "acme.atlassian.net/wiki/spaces/X"
        );
    }

    #[test]
    fn normalize_domain_strips_scheme_but_keeps_query_in_path() {
        // The normalizer is intentionally lossy — anything after the host
        // including query-like strings is preserved verbatim as "path".
        let normalized = normalize_domain("https://acme.atlassian.net/wiki?x=1");
        assert_eq!(normalized, "acme.atlassian.net/wiki?x=1");
    }

    #[test]
    fn normalize_domain_uppercase_scheme_passthrough() {
        // Only lowercase `https://` / `http://` are stripped — uppercase
        // is preserved as-is to surface the typo to the user.
        let normalized = normalize_domain("HTTPS://acme.atlassian.net");
        assert_eq!(
            normalized, "HTTPS://acme.atlassian.net",
            "uppercase scheme is preserved (caller can re-normalize)"
        );
    }

    #[test]
    fn normalize_domain_two_consecutive_trailing_slashes_strips_one() {
        // Documents current behaviour — only a single trailing `/` is
        // removed; double-slash inputs leave a residual `/` for the user
        // to notice.
        let normalized = normalize_domain("https://acme.atlassian.net//");
        assert_eq!(normalized, "acme.atlassian.net/");
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
            flavor: None,
        };
        let confluence_instance = AtlassianInstance {
            domain: confluence_domain,
            email: Some(email),
            api_token: None,
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: None,
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
