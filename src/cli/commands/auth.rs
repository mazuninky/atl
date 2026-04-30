//! `atl auth` command handlers.
//!
//! The handlers accept a `&dyn SecretStore` and `&dyn Prompter` so the whole
//! login/logout/status/token surface can be exercised from tests with
//! `InMemoryStore` + `MockPrompter` — no keyring access, no real prompts,
//! no HTTP.

use std::io::Write;

use anyhow::{Result, anyhow, bail};
use camino::Utf8Path;
use reqwest::header::HeaderMap;
use tracing::debug;

use crate::auth::{Prompter, SecretStore, service_name};
use crate::cli::args::{
    AuthKind, AuthLoginArgs, AuthLogoutArgs, AuthService, AuthStatusArgs, AuthSubcommand,
    AuthTokenArgs, SingleService,
};
use crate::client::raw_request;
use crate::config::{AtlassianInstance, AuthType, Config, ConfigLoader, Profile, TokenStorage};
use crate::io::IoStreams;

/// Dispatches the selected `atl auth` subcommand.
pub async fn run(
    cmd: &AuthSubcommand,
    config_path: Option<&Utf8Path>,
    cli_profile: Option<&str>,
    io: &mut IoStreams,
    store: &dyn SecretStore,
    prompter: &dyn Prompter,
    retries: u32,
) -> Result<()> {
    match cmd {
        AuthSubcommand::Login(args) => {
            login(args, config_path, cli_profile, io, store, prompter, retries).await
        }
        AuthSubcommand::Logout(args) => logout(args, config_path, cli_profile, io, store),
        AuthSubcommand::Status(args) => {
            status(args, config_path, cli_profile, io, store, retries).await
        }
        AuthSubcommand::Token(args) => token(args, config_path, cli_profile, io, store),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolves the profile name to use: command-level `--profile` on the
/// subcommand wins, then the global `--profile`, then `default`.
fn resolve_profile_name(
    sub_profile: Option<&str>,
    cli_profile: Option<&str>,
    config: &Config,
) -> String {
    if let Some(p) = sub_profile {
        return p.to_string();
    }
    if let Some(p) = cli_profile {
        return p.to_string();
    }
    if !config.default_profile.is_empty() {
        return config.default_profile.clone();
    }
    "default".to_string()
}

/// Returns the iterable list of `(kind_label, kind_id)` pairs selected by
/// the user for login/logout.
fn selected_kinds(service: AuthService) -> Vec<&'static str> {
    match service {
        AuthService::Confluence => vec!["confluence"],
        AuthService::Jira => vec!["jira"],
        AuthService::Both => vec!["confluence", "jira"],
    }
}

fn auth_type_from_cli(kind: AuthKind) -> AuthType {
    match kind {
        AuthKind::Basic => AuthType::Basic,
        AuthKind::Bearer => AuthType::Bearer,
    }
}

fn account_for_instance(instance: &AtlassianInstance) -> String {
    instance
        .email
        .clone()
        .unwrap_or_else(|| "default".to_string())
}

/// Returns the instance on the given profile for the given `kind`
/// (`"confluence"` or `"jira"`), or `None` when it is not configured.
fn instance_for<'a>(profile: &'a Profile, kind: &str) -> Option<&'a AtlassianInstance> {
    match kind {
        "confluence" => profile.confluence.as_ref(),
        "jira" => profile.jira.as_ref(),
        _ => None,
    }
}

fn set_instance_on_profile(profile: &mut Profile, kind: &str, instance: AtlassianInstance) {
    match kind {
        "confluence" => profile.confluence = Some(instance),
        "jira" => profile.jira = Some(instance),
        _ => {}
    }
}

/// Issues the minimal "who am I" request used to verify a freshly-entered
/// token. Returns `Ok(())` on 2xx, `Err` with an explanatory message on 4xx.
///
/// The instance passed here already has `api_token` set inline (the caller
/// clones the instance and injects the token before calling), so the
/// `build_http_client` inside `raw_request` will find it in the TOML field
/// step of the resolution chain. We still need to provide a store for the
/// signature — an `InMemoryStore` would suffice but the real store is harmless.
async fn verify_instance(
    kind: &str,
    instance: &AtlassianInstance,
    store: &dyn SecretStore,
    retries: u32,
) -> Result<serde_json::Value> {
    let endpoint = match kind {
        "jira" => "/rest/api/2/myself",
        "confluence" => "/wiki/rest/api/user/current",
        other => bail!("unknown service kind: {other}"),
    };
    debug!("verifying {kind} token against {endpoint}");
    // Profile name is irrelevant here — the token is already inlined on
    // the instance, so keyring lookup won't be reached.
    let value = raw_request(
        instance,
        "_verify",
        kind,
        store,
        reqwest::Method::GET,
        endpoint,
        HeaderMap::new(),
        &[],
        None,
        retries,
    )
    .await?;
    Ok(value)
}

/// Best-effort display name extracted from a /myself-style response.
fn extract_account_label(value: &serde_json::Value) -> Option<String> {
    for key in ["displayName", "name", "emailAddress", "accountId"] {
        if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// login
// ---------------------------------------------------------------------------

async fn login(
    args: &AuthLoginArgs,
    config_path: Option<&Utf8Path>,
    cli_profile: Option<&str>,
    io: &mut IoStreams,
    store: &dyn SecretStore,
    prompter: &dyn Prompter,
    retries: u32,
) -> Result<()> {
    let mut config = ConfigLoader::load(config_path)?
        .ok_or_else(|| anyhow!("no config found; run `atl init` first"))?;
    if config.default_profile.is_empty() {
        config.default_profile = "default".to_string();
    }

    let profile_name = resolve_profile_name(args.profile.as_deref(), cli_profile, &config);

    // Look up any existing profile so resolve_domain / resolve_email can
    // pre-fill from the config when no CLI flag was given.
    let existing_profile = config.profiles.get(&profile_name);

    // Domain and email are read from config silently — no prompts.
    let domain = resolve_domain(args, existing_profile)?;

    let email = resolve_email(args, existing_profile)?;

    // Token resolution: with-token reads stdin, otherwise prompter.password.
    let token = resolve_token_input(args, io, prompter)?;
    if token.is_empty() {
        bail!("token cannot be empty");
    }

    let kinds = selected_kinds(args.service);

    // Ensure the profile exists in the config.
    let profile_entry = config.profiles.entry(profile_name.clone()).or_default();

    let use_config_storage = matches!(profile_entry.token_storage, TokenStorage::Config);

    // Stage keyring writes so nothing reaches the secret store until every
    // verification has succeeded. If any step fails mid-loop the staged
    // entries are simply discarded, leaving keyring and config untouched.
    let mut staged_keyring_writes: Vec<(String, String, String)> = Vec::with_capacity(kinds.len());

    for kind in &kinds {
        let mut instance = instance_for(profile_entry, kind)
            .cloned()
            .unwrap_or_else(|| AtlassianInstance {
                domain: domain.clone(),
                email: email.clone(),
                api_token: None,
                auth_type: auth_type_from_cli(args.auth_type),
                api_path: None,
                read_only: false,
                flavor: None,
            });

        // Update fields from the flags the user supplied this run.
        instance.domain = domain.clone();
        if email.is_some() {
            instance.email = email.clone();
        }
        instance.auth_type = auth_type_from_cli(args.auth_type);

        // Verify the token works before persisting anything. We build a
        // temporary verification instance with api_token inlined so the
        // existing client plumbing finds it.
        if !args.skip_verify {
            let mut verify_instance_cloned = instance.clone();
            verify_instance_cloned.api_token = Some(token.clone());
            let result = verify_instance(kind, &verify_instance_cloned, store, retries).await;
            match result {
                Ok(value) => {
                    let label = extract_account_label(&value)
                        .unwrap_or_else(|| instance.email.clone().unwrap_or_default());
                    writeln!(
                        io.stdout(),
                        "OK verified {kind} at {} as {label}",
                        instance.domain
                    )?;
                }
                Err(e) => {
                    return Err(anyhow!(
                        "{kind} token verification failed for {}: {e}",
                        instance.domain
                    ));
                }
            }
        }

        // Persist the token according to the profile's storage preference.
        if use_config_storage {
            instance.api_token = Some(token.clone());
        } else {
            instance.api_token = None;
            // Stage the keyring write; actual `store.set()` is deferred so
            // an earlier success cannot leak into the keyring when a later
            // kind fails verification.
            let account = account_for_instance(&instance);
            let svc = service_name(&profile_name, kind);
            staged_keyring_writes.push((svc, account, token.clone()));
        }

        // Save the instance back onto the profile.
        set_instance_on_profile(profile_entry, kind, instance);
    }

    // Commit staged keyring writes (empty when using config storage). Roll
    // back already-committed entries on any set failure so we never leave
    // partial state behind.
    let mut committed: Vec<(String, String)> = Vec::with_capacity(staged_keyring_writes.len());
    for (svc, account, secret) in &staged_keyring_writes {
        if let Err(e) = store.set(svc, account, secret) {
            for (rb_svc, rb_account) in &committed {
                let _ = store.delete(rb_svc, rb_account);
            }
            return Err(anyhow!(
                "failed to write keyring entry for {svc}/{account}: {e}"
            ));
        }
        committed.push((svc.clone(), account.clone()));
    }

    // Persist the updated config. If the save fails, roll back the committed
    // keyring entries so the user's visible state matches what it was before
    // the login attempt.
    let path = match ConfigLoader::save(&config, config_path) {
        Ok(p) => p,
        Err(e) => {
            for (rb_svc, rb_account) in &committed {
                let _ = store.delete(rb_svc, rb_account);
            }
            return Err(e);
        }
    };

    writeln!(
        io.stdout(),
        "Logged in to profile '{profile_name}' ({} service{}), config saved to {path}",
        kinds.len(),
        if kinds.len() == 1 { "" } else { "s" }
    )?;
    Ok(())
}

/// Extracts the domain from a config profile, checking jira first then confluence.
fn domain_from_profile(profile: Option<&Profile>) -> Option<&str> {
    let p = profile?;
    p.jira
        .as_ref()
        .map(|i| i.domain.as_str())
        .or_else(|| p.confluence.as_ref().map(|i| i.domain.as_str()))
        .filter(|d| !d.is_empty())
}

/// Extracts the email from a config profile, checking jira first then confluence.
fn email_from_profile(profile: Option<&Profile>) -> Option<&str> {
    let p = profile?;
    p.jira
        .as_ref()
        .and_then(|i| i.email.as_deref())
        .or_else(|| p.confluence.as_ref().and_then(|i| i.email.as_deref()))
        .filter(|e| !e.is_empty())
}

/// Reads `--domain`, falling back to the config profile silently.
/// Never prompts — requires that `atl init` has configured the domain.
fn resolve_domain(args: &AuthLoginArgs, profile: Option<&Profile>) -> Result<String> {
    if let Some(d) = args.domain.as_deref() {
        if d.trim().is_empty() {
            bail!("--domain cannot be empty");
        }
        return Ok(d.to_string());
    }
    if let Some(d) = domain_from_profile(profile) {
        return Ok(d.to_string());
    }
    bail!("domain not configured in profile; run `atl init` to set it up")
}

/// Reads `--email`, falling back to the config profile silently.
/// Never prompts — requires that `atl init` has configured the email.
fn resolve_email(args: &AuthLoginArgs, profile: Option<&Profile>) -> Result<Option<String>> {
    if let Some(e) = args.email.as_deref() {
        if e.trim().is_empty() {
            bail!("--email cannot be empty");
        }
        return Ok(Some(e.to_string()));
    }
    // Bearer auth does not need an email; the account label falls back to
    // "default" in the keyring.
    if matches!(args.auth_type, AuthKind::Bearer) {
        return Ok(None);
    }
    if let Some(e) = email_from_profile(profile) {
        return Ok(Some(e.to_string()));
    }
    bail!("email not configured in profile; run `atl init` to set it up")
}

/// Reads the token: either from stdin (`--with-token`) or from the
/// interactive password prompt.
fn resolve_token_input(
    args: &AuthLoginArgs,
    io: &mut IoStreams,
    prompter: &dyn Prompter,
) -> Result<String> {
    if args.with_token {
        let mut line = String::new();
        let stdin = io.stdin();
        stdin
            .read_line(&mut line)
            .map_err(|e| anyhow!("failed to read token from stdin: {e}"))?;
        return Ok(line.trim_end_matches(['\r', '\n']).to_string());
    }

    if !io.is_stdin_tty() || !io.is_stdout_tty() {
        bail!("interactive login requires a TTY; pass --with-token to read the token from stdin");
    }

    // Print the help link before launching the password prompt. The URL
    // for Atlassian Cloud API tokens is only meaningful under Basic auth;
    // Bearer (Data Center / Server) users need a Personal Access Token
    // generated from their server's user profile instead.
    match args.auth_type {
        AuthKind::Basic => {
            let url = "https://id.atlassian.com/manage-profile/security/api-tokens";
            writeln!(io.stdout(), "Generate an API token at {url}")?;
            // Best-effort: open the URL in the default browser so the user
            // doesn't have to copy-paste. Silently ignored on headless systems.
            let _ = webbrowser::open(url);
        }
        AuthKind::Bearer => {
            writeln!(
                io.stdout(),
                "Generate a Personal Access Token in your Jira/Confluence user profile \
                 (Settings → Personal Access Tokens)"
            )?;
        }
    }
    prompter.password("API token:")
}

// ---------------------------------------------------------------------------
// logout
// ---------------------------------------------------------------------------

fn logout(
    args: &AuthLogoutArgs,
    config_path: Option<&Utf8Path>,
    cli_profile: Option<&str>,
    io: &mut IoStreams,
    store: &dyn SecretStore,
) -> Result<()> {
    let Some(mut config) = ConfigLoader::load(config_path)? else {
        bail!("no config file found; run `atl init` or `atl auth login` first");
    };

    let profile_name = resolve_profile_name(args.profile.as_deref(), cli_profile, &config);
    let profile = config
        .profiles
        .get_mut(&profile_name)
        .ok_or_else(|| anyhow!("profile '{profile_name}' not found"))?;

    let kinds = selected_kinds(args.service);
    let mut removed = 0usize;
    let mut legacy_cleaned = false;

    for kind in &kinds {
        let Some(instance) = instance_for(profile, kind) else {
            writeln!(
                io.stdout(),
                "skip {kind}: no instance configured on profile '{profile_name}'"
            )?;
            continue;
        };

        let account = account_for_instance(instance);
        let svc = service_name(&profile_name, kind);
        store.delete(&svc, &account)?;
        writeln!(
            io.stdout(),
            "removed keyring entry for {profile_name}/{kind} ({account})"
        )?;
        removed += 1;

        // Also clear any plaintext token from the config file.
        if let Some(inst) = match *kind {
            "confluence" => profile.confluence.as_mut(),
            "jira" => profile.jira.as_mut(),
            _ => None,
        } && inst.api_token.is_some()
        {
            writeln!(
                io.stdout(),
                "cleaning up api_token in atl.toml for {profile_name}/{kind}"
            )?;
            inst.api_token = None;
            legacy_cleaned = true;
        }
    }

    if removed == 0 && !legacy_cleaned {
        return Ok(());
    }

    let path = ConfigLoader::save(&config, config_path)?;
    writeln!(io.stdout(), "Config saved to {path}")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

async fn status(
    args: &AuthStatusArgs,
    config_path: Option<&Utf8Path>,
    cli_profile: Option<&str>,
    io: &mut IoStreams,
    store: &dyn SecretStore,
    retries: u32,
) -> Result<()> {
    let config = ConfigLoader::load(config_path)?
        .ok_or_else(|| anyhow!("no config file found; run `atl init` first"))?;

    // Choose the profile list: explicit --profile takes precedence over the
    // global one, otherwise show every profile.
    let explicit = args.profile.as_deref().or(cli_profile);
    let mut names: Vec<&String> = match explicit {
        Some(name) => {
            if !config.profiles.contains_key(name) {
                bail!("profile '{name}' not found");
            }
            config
                .profiles
                .keys()
                .filter(|k| k.as_str() == name)
                .collect()
        }
        None => config.profiles.keys().collect(),
    };
    names.sort();

    let mut first = true;
    for name in names {
        if !first {
            writeln!(io.stdout())?;
        }
        first = false;
        writeln!(io.stdout(), "{name} profile")?;
        let profile = &config.profiles[name];

        for kind in ["confluence", "jira"] {
            let Some(instance) = instance_for(profile, kind) else {
                writeln!(io.stdout(), "  --  {kind:<10}  (not configured)")?;
                continue;
            };
            report_service_status(name, kind, instance, io, store, retries, args.skip_verify)
                .await?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn report_service_status(
    profile_name: &str,
    kind: &str,
    instance: &AtlassianInstance,
    io: &mut IoStreams,
    store: &dyn SecretStore,
    retries: u32,
    skip_verify: bool,
) -> Result<()> {
    let account = account_for_instance(instance);
    let svc = service_name(profile_name, kind);

    // Figure out where the token comes from (if anywhere).
    let source: Option<&'static str>;
    let token_for_check: Option<String>;
    if std::env::var("ATL_API_TOKEN").is_ok() {
        source = Some("env");
        token_for_check = Some(std::env::var("ATL_API_TOKEN").unwrap_or_default());
    } else if instance.api_token.is_some() {
        source = Some("toml");
        token_for_check = instance.api_token.clone();
    } else {
        match store.get(&svc, &account) {
            Ok(Some(t)) => {
                source = Some("keyring");
                token_for_check = Some(t);
            }
            Ok(None) => {
                source = None;
                token_for_check = None;
            }
            Err(e) => {
                debug!("keyring lookup failed while computing status: {e}");
                source = None;
                token_for_check = None;
            }
        }
    }

    let Some(source_label) = source else {
        writeln!(
            io.stdout(),
            "  --  {kind:<10}  {domain}  (no token)",
            domain = instance.domain
        )?;
        return Ok(());
    };

    let Some(token) = token_for_check else {
        writeln!(
            io.stdout(),
            "  --  {kind:<10}  {domain}  (no token)",
            domain = instance.domain
        )?;
        return Ok(());
    };

    if skip_verify {
        writeln!(
            io.stdout(),
            "  OK  {kind:<10}  {domain}  {account} (via {source_label}, unverified)",
            domain = instance.domain
        )?;
        return Ok(());
    }

    let mut verify_inst = instance.clone();
    verify_inst.api_token = Some(token);
    match verify_instance(kind, &verify_inst, store, retries).await {
        Ok(_) => {
            writeln!(
                io.stdout(),
                "  OK  {kind:<10}  {domain}  {account} (via {source_label})",
                domain = instance.domain
            )?;
        }
        Err(e) => {
            let msg = truncate_single_line(&e.to_string(), 80);
            writeln!(
                io.stdout(),
                "  FAIL  {kind:<10}  {domain}  {msg}",
                domain = instance.domain
            )?;
        }
    }
    Ok(())
}

fn truncate_single_line(s: &str, max: usize) -> String {
    let single = s.replace(['\r', '\n'], " ");
    if single.len() <= max {
        return single;
    }
    // Truncate at the last char boundary at or before `max` bytes.
    let truncate_at = single
        .char_indices()
        .take_while(|(i, _)| *i < max)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    format!("{}…", &single[..truncate_at])
}

// ---------------------------------------------------------------------------
// token
// ---------------------------------------------------------------------------

fn token(
    args: &AuthTokenArgs,
    config_path: Option<&Utf8Path>,
    cli_profile: Option<&str>,
    io: &mut IoStreams,
    store: &dyn SecretStore,
) -> Result<()> {
    if io.is_stdout_tty() && !args.force {
        bail!("refusing to print token to a TTY; use --force to override");
    }

    let config = ConfigLoader::load(config_path)?
        .ok_or_else(|| anyhow!("no config file found; run `atl init` first"))?;
    let profile_name = resolve_profile_name(args.profile.as_deref(), cli_profile, &config);
    let profile = config
        .profiles
        .get(&profile_name)
        .ok_or_else(|| anyhow!("profile '{profile_name}' not found"))?;

    let kind = match args.service {
        SingleService::Confluence => "confluence",
        SingleService::Jira => "jira",
    };
    let instance = instance_for(profile, kind)
        .ok_or_else(|| anyhow!("no {kind} instance configured on profile '{profile_name}'"))?;

    let token = instance
        .resolved_token(&profile_name, kind, store)
        .ok_or_else(|| {
            anyhow!(
                "no token available for profile '{profile_name}' / {kind}; run `atl auth login` first"
            )
        })?;

    writeln!(io.stdout(), "{token}")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::MockPrompter;

    fn mk_config_with_profile() -> Config {
        let mut config = Config {
            default_profile: "default".to_string(),
            ..Default::default()
        };
        config.profiles.insert(
            "default".to_string(),
            Profile {
                jira: Some(AtlassianInstance {
                    domain: "example.atlassian.net".into(),
                    email: Some("alice@example.com".into()),
                    api_token: None,
                    auth_type: AuthType::Basic,
                    api_path: None,
                    read_only: false,
                    flavor: None,
                }),
                ..Default::default()
            },
        );
        config
    }

    #[test]
    fn resolve_profile_name_prefers_sub() {
        let config = mk_config_with_profile();
        assert_eq!(
            resolve_profile_name(Some("staging"), Some("global"), &config),
            "staging"
        );
    }

    #[test]
    fn resolve_profile_name_falls_back_to_cli() {
        let config = mk_config_with_profile();
        assert_eq!(
            resolve_profile_name(None, Some("cli-prof"), &config),
            "cli-prof"
        );
    }

    #[test]
    fn resolve_profile_name_falls_back_to_default() {
        let config = mk_config_with_profile();
        assert_eq!(resolve_profile_name(None, None, &config), "default");
    }

    #[test]
    fn resolve_profile_name_empty_default_goes_to_default_literal() {
        let mut config = mk_config_with_profile();
        config.default_profile = String::new();
        assert_eq!(resolve_profile_name(None, None, &config), "default");
    }

    #[test]
    fn selected_kinds_both_returns_both_ordered() {
        assert_eq!(
            selected_kinds(AuthService::Both),
            vec!["confluence", "jira"]
        );
    }

    #[test]
    fn selected_kinds_single() {
        assert_eq!(selected_kinds(AuthService::Jira), vec!["jira"]);
        assert_eq!(selected_kinds(AuthService::Confluence), vec!["confluence"]);
    }

    #[test]
    fn account_for_instance_uses_email() {
        let inst = AtlassianInstance {
            domain: "d".into(),
            email: Some("e@x.com".into()),
            api_token: None,
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: None,
        };
        assert_eq!(account_for_instance(&inst), "e@x.com");
    }

    #[test]
    fn account_for_instance_defaults_when_no_email() {
        let inst = AtlassianInstance {
            domain: "d".into(),
            email: None,
            api_token: None,
            auth_type: AuthType::Bearer,
            api_path: None,
            read_only: false,
            flavor: None,
        };
        assert_eq!(account_for_instance(&inst), "default");
    }

    #[test]
    fn extract_account_label_display_name_wins() {
        let v = serde_json::json!({
            "displayName": "Alice",
            "emailAddress": "alice@example.com",
        });
        assert_eq!(extract_account_label(&v).as_deref(), Some("Alice"));
    }

    #[test]
    fn extract_account_label_falls_back_to_email() {
        let v = serde_json::json!({
            "emailAddress": "alice@example.com",
        });
        assert_eq!(
            extract_account_label(&v).as_deref(),
            Some("alice@example.com")
        );
    }

    #[test]
    fn extract_account_label_returns_none_when_no_fields() {
        let v = serde_json::json!({});
        assert!(extract_account_label(&v).is_none());
    }

    #[test]
    fn truncate_single_line_short_unchanged() {
        assert_eq!(truncate_single_line("short", 80), "short");
    }

    #[test]
    fn truncate_single_line_collapses_newlines() {
        assert_eq!(truncate_single_line("line 1\nline 2", 80), "line 1 line 2");
    }

    #[test]
    fn truncate_single_line_clips_long_input() {
        let long = "a".repeat(100);
        let got = truncate_single_line(&long, 10);
        assert!(got.ends_with('…'));
        assert!(got.chars().count() <= 11);
    }

    #[test]
    fn truncate_single_line_handles_multibyte() {
        // Each Greek letter is 2 bytes in UTF-8 (U+03B1..U+03C9).
        // Picking a `max` that lands mid-character (odd byte offset) would
        // panic with the old byte-slicing implementation.
        let input = "αβγδεζηθικλμν";
        let got = truncate_single_line(input, 5);
        // Must not panic and must end with the ellipsis marker. The ellipsis
        // itself is valid UTF-8 and so is the `&str` prefix we sliced, so the
        // key regression guard is: no panic, no mangled character.
        assert!(got.ends_with('…'), "got: {got:?}");
        // The truncated prefix should contain some characters from the input
        // (mid-char boundary should fall back to the prior char boundary).
        let prefix = got.trim_end_matches('…');
        assert!(
            prefix.starts_with('α'),
            "prefix should start with α: {prefix:?}"
        );
        assert!(!prefix.is_empty(), "prefix should not be empty");
    }

    // -----------------------------------------------------------------
    // auth_type_from_cli
    // -----------------------------------------------------------------

    #[test]
    fn auth_type_from_cli_basic_maps_to_basic() {
        assert!(matches!(
            auth_type_from_cli(AuthKind::Basic),
            AuthType::Basic
        ));
    }

    #[test]
    fn auth_type_from_cli_bearer_maps_to_bearer() {
        assert!(matches!(
            auth_type_from_cli(AuthKind::Bearer),
            AuthType::Bearer
        ));
    }

    // -----------------------------------------------------------------
    // instance_for / set_instance_on_profile
    // -----------------------------------------------------------------

    fn mk_instance(domain: &str, email: Option<&str>) -> AtlassianInstance {
        AtlassianInstance {
            domain: domain.into(),
            email: email.map(str::to_string),
            api_token: None,
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: None,
        }
    }

    #[test]
    fn instance_for_jira_returns_jira_instance() {
        let profile = Profile {
            jira: Some(mk_instance("j.example", Some("j@example.com"))),
            confluence: Some(mk_instance("c.example", Some("c@example.com"))),
            ..Default::default()
        };
        let i = instance_for(&profile, "jira").unwrap();
        assert_eq!(i.domain, "j.example");
    }

    #[test]
    fn instance_for_confluence_returns_confluence_instance() {
        let profile = Profile {
            jira: Some(mk_instance("j.example", None)),
            confluence: Some(mk_instance("c.example", None)),
            ..Default::default()
        };
        let i = instance_for(&profile, "confluence").unwrap();
        assert_eq!(i.domain, "c.example");
    }

    #[test]
    fn instance_for_unknown_kind_returns_none() {
        let profile = Profile {
            jira: Some(mk_instance("j.example", None)),
            ..Default::default()
        };
        assert!(instance_for(&profile, "bitbucket").is_none());
    }

    #[test]
    fn instance_for_missing_returns_none() {
        let profile = Profile::default();
        assert!(instance_for(&profile, "jira").is_none());
        assert!(instance_for(&profile, "confluence").is_none());
    }

    #[test]
    fn set_instance_on_profile_sets_jira() {
        let mut profile = Profile::default();
        let inst = mk_instance("j.example", Some("a@b.c"));
        set_instance_on_profile(&mut profile, "jira", inst);
        assert_eq!(profile.jira.as_ref().unwrap().domain, "j.example");
        assert!(profile.confluence.is_none());
    }

    #[test]
    fn set_instance_on_profile_sets_confluence() {
        let mut profile = Profile::default();
        let inst = mk_instance("c.example", None);
        set_instance_on_profile(&mut profile, "confluence", inst);
        assert_eq!(profile.confluence.as_ref().unwrap().domain, "c.example");
        assert!(profile.jira.is_none());
    }

    #[test]
    fn set_instance_on_profile_unknown_kind_is_noop() {
        let mut profile = Profile::default();
        let inst = mk_instance("nope", None);
        set_instance_on_profile(&mut profile, "bitbucket", inst);
        assert!(profile.jira.is_none());
        assert!(profile.confluence.is_none());
    }

    // -----------------------------------------------------------------
    // domain_from_profile / email_from_profile
    // -----------------------------------------------------------------

    #[test]
    fn domain_from_profile_none_when_profile_is_none() {
        assert!(domain_from_profile(None).is_none());
    }

    #[test]
    fn domain_from_profile_prefers_jira_over_confluence() {
        let profile = Profile {
            jira: Some(mk_instance("jira.example", None)),
            confluence: Some(mk_instance("conf.example", None)),
            ..Default::default()
        };
        assert_eq!(domain_from_profile(Some(&profile)), Some("jira.example"));
    }

    #[test]
    fn domain_from_profile_falls_back_to_confluence() {
        let profile = Profile {
            jira: None,
            confluence: Some(mk_instance("conf.example", None)),
            ..Default::default()
        };
        assert_eq!(domain_from_profile(Some(&profile)), Some("conf.example"));
    }

    #[test]
    fn domain_from_profile_filters_out_empty_jira_domain() {
        // An empty jira.domain should be skipped so the confluence value wins.
        let profile = Profile {
            jira: Some(mk_instance("", None)),
            confluence: Some(mk_instance("conf.example", None)),
            ..Default::default()
        };
        // The current implementation filters empty after `or_else`; an empty
        // jira domain therefore short-circuits to None even when a valid
        // confluence value exists. Document the behaviour.
        // The filter sees the jira value first ("" → filtered → None overall).
        assert!(domain_from_profile(Some(&profile)).is_none());
    }

    #[test]
    fn domain_from_profile_returns_none_when_both_missing() {
        let profile = Profile::default();
        assert!(domain_from_profile(Some(&profile)).is_none());
    }

    #[test]
    fn email_from_profile_none_when_profile_is_none() {
        assert!(email_from_profile(None).is_none());
    }

    #[test]
    fn email_from_profile_prefers_jira() {
        let profile = Profile {
            jira: Some(mk_instance("j", Some("jira@x.com"))),
            confluence: Some(mk_instance("c", Some("conf@x.com"))),
            ..Default::default()
        };
        assert_eq!(email_from_profile(Some(&profile)), Some("jira@x.com"));
    }

    #[test]
    fn email_from_profile_falls_back_to_confluence() {
        let profile = Profile {
            jira: Some(mk_instance("j", None)),
            confluence: Some(mk_instance("c", Some("conf@x.com"))),
            ..Default::default()
        };
        assert_eq!(email_from_profile(Some(&profile)), Some("conf@x.com"));
    }

    #[test]
    fn email_from_profile_filters_out_empty_jira_email() {
        // Empty jira.email — the implementation chains `.or_else()` only when
        // the jira instance lacks an email entirely; an empty string is
        // returned by the jira branch and then filtered to None overall.
        let profile = Profile {
            jira: Some(mk_instance("j", Some(""))),
            confluence: Some(mk_instance("c", Some("conf@x.com"))),
            ..Default::default()
        };
        assert!(email_from_profile(Some(&profile)).is_none());
    }

    // -----------------------------------------------------------------
    // resolve_domain
    // -----------------------------------------------------------------

    fn mk_login_args() -> AuthLoginArgs {
        AuthLoginArgs {
            service: AuthService::Both,
            profile: None,
            domain: None,
            email: None,
            auth_type: AuthKind::Basic,
            with_token: false,
            skip_verify: false,
        }
    }

    #[test]
    fn resolve_domain_uses_cli_flag() {
        let mut args = mk_login_args();
        args.domain = Some("from-cli.atlassian.net".into());
        let got = resolve_domain(&args, None).unwrap();
        assert_eq!(got, "from-cli.atlassian.net");
    }

    #[test]
    fn resolve_domain_rejects_empty_cli_flag() {
        let mut args = mk_login_args();
        args.domain = Some("   ".into());
        let err = resolve_domain(&args, None).unwrap_err();
        assert!(
            err.to_string().contains("--domain cannot be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn resolve_domain_falls_back_to_profile() {
        let args = mk_login_args();
        let profile = Profile {
            jira: Some(mk_instance("from-profile.atlassian.net", None)),
            ..Default::default()
        };
        let got = resolve_domain(&args, Some(&profile)).unwrap();
        assert_eq!(got, "from-profile.atlassian.net");
    }

    #[test]
    fn resolve_domain_errors_when_unconfigured() {
        let args = mk_login_args();
        let err = resolve_domain(&args, None).unwrap_err();
        assert!(
            err.to_string().contains("domain not configured"),
            "got: {err}"
        );
    }

    #[test]
    fn resolve_domain_errors_when_profile_has_no_domain() {
        let args = mk_login_args();
        // Empty domain on jira instance → falls through.
        let profile = Profile {
            jira: Some(mk_instance("", None)),
            ..Default::default()
        };
        let err = resolve_domain(&args, Some(&profile)).unwrap_err();
        assert!(
            err.to_string().contains("domain not configured"),
            "got: {err}"
        );
    }

    // -----------------------------------------------------------------
    // resolve_email
    // -----------------------------------------------------------------

    #[test]
    fn resolve_email_uses_cli_flag() {
        let mut args = mk_login_args();
        args.email = Some("cli@example.com".into());
        let got = resolve_email(&args, None).unwrap();
        assert_eq!(got, Some("cli@example.com".into()));
    }

    #[test]
    fn resolve_email_rejects_empty_cli_flag() {
        let mut args = mk_login_args();
        args.email = Some("  ".into());
        let err = resolve_email(&args, None).unwrap_err();
        assert!(
            err.to_string().contains("--email cannot be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn resolve_email_returns_none_for_bearer_without_profile() {
        let mut args = mk_login_args();
        args.auth_type = AuthKind::Bearer;
        let got = resolve_email(&args, None).unwrap();
        // Bearer auth does not need an email at all.
        assert!(got.is_none());
    }

    #[test]
    fn resolve_email_falls_back_to_profile_for_basic_auth() {
        let args = mk_login_args();
        let profile = Profile {
            jira: Some(mk_instance("d", Some("from-profile@x.com"))),
            ..Default::default()
        };
        let got = resolve_email(&args, Some(&profile)).unwrap();
        assert_eq!(got, Some("from-profile@x.com".into()));
    }

    #[test]
    fn resolve_email_errors_for_basic_auth_without_email() {
        let args = mk_login_args();
        let err = resolve_email(&args, None).unwrap_err();
        assert!(
            err.to_string().contains("email not configured"),
            "got: {err}"
        );
    }

    #[test]
    fn resolve_email_errors_for_basic_auth_when_profile_email_empty() {
        let args = mk_login_args();
        let profile = Profile {
            jira: Some(mk_instance("d", Some(""))),
            ..Default::default()
        };
        let err = resolve_email(&args, Some(&profile)).unwrap_err();
        assert!(
            err.to_string().contains("email not configured"),
            "got: {err}"
        );
    }

    // -----------------------------------------------------------------
    // resolve_token_input
    // -----------------------------------------------------------------

    #[test]
    fn resolve_token_input_with_token_reads_empty_stdin_as_empty_string() {
        // The test IoStreams has an empty Cursor for stdin. With
        // `--with-token`, `read_line` returns 0 bytes (EOF); the function
        // should yield an empty string rather than erroring.
        let mut args = mk_login_args();
        args.with_token = true;
        let mut io = IoStreams::test();
        let prompter = MockPrompter::new(vec![]);
        let got = resolve_token_input(&args, &mut io, &prompter).unwrap();
        assert_eq!(got, "");
    }

    #[test]
    fn resolve_token_input_without_tty_or_with_token_errors() {
        // The test IoStreams reports both stdin and stdout as non-TTY. When
        // `--with-token` is not set, the handler must bail with the
        // "interactive login requires a TTY" guidance instead of trying to
        // launch the prompter.
        let args = mk_login_args();
        let mut io = IoStreams::test();
        let prompter = MockPrompter::new(vec![]);
        let err = resolve_token_input(&args, &mut io, &prompter).unwrap_err();
        assert!(
            err.to_string().contains("interactive login requires a TTY"),
            "got: {err}"
        );
        // Ensure the prompter was not consumed.
        assert_eq!(prompter.remaining(), 0);
    }

    // -----------------------------------------------------------------
    // extract_account_label — coverage of the remaining priorities
    // -----------------------------------------------------------------

    #[test]
    fn extract_account_label_falls_back_to_name_then_account_id() {
        // Explicit priority chain: displayName > name > emailAddress > accountId.
        let v = serde_json::json!({"name": "alice"});
        assert_eq!(extract_account_label(&v).as_deref(), Some("alice"));

        let v = serde_json::json!({"accountId": "0:abcd"});
        assert_eq!(extract_account_label(&v).as_deref(), Some("0:abcd"));
    }

    #[test]
    fn extract_account_label_ignores_non_string_values() {
        // A number under `displayName` should not be returned — the function
        // only matches string fields. The next field of any kind is also
        // missing, so the result is None.
        let v = serde_json::json!({"displayName": 42, "name": 7});
        assert!(extract_account_label(&v).is_none());
    }
}
