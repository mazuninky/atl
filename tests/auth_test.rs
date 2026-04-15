//! Integration tests for `atl auth` that exercise the CLI binary directly
//! (for help/flag wiring) plus unit-level flow tests that drive the handler
//! with an `InMemoryStore` + `MockPrompter`, never touching a real keyring.

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::cargo_bin;

fn dummy_config(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn run_atl(args: &[&str], config: &Path) -> (i32, String, String) {
    let bin = cargo_bin("atl");
    let out = Command::new(&bin)
        .arg("--config")
        .arg(config)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn atl at {}: {e}", bin.display()));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (out.status.code().unwrap_or(-1), stdout, stderr)
}

#[test]
fn auth_help_lists_subcommands() {
    let dummy = dummy_config("atl-auth-help-dummy.toml");
    let (code, stdout, stderr) = run_atl(&["auth", "--help"], &dummy);
    assert_eq!(code, 0, "stderr: {stderr}");
    for needle in ["login", "logout", "status", "token"] {
        assert!(
            stdout.contains(needle),
            "expected {needle:?} in `atl auth --help`:\n{stdout}"
        );
    }
}

#[test]
fn auth_login_help_lists_flags() {
    let dummy = dummy_config("atl-auth-login-help-dummy.toml");
    let (code, stdout, stderr) = run_atl(&["auth", "login", "--help"], &dummy);
    assert_eq!(code, 0, "stderr: {stderr}");
    for needle in [
        "--service",
        "--profile",
        "--domain",
        "--email",
        "--auth-type",
        "--with-token",
    ] {
        assert!(
            stdout.contains(needle),
            "expected {needle:?} in `atl auth login --help`:\n{stdout}"
        );
    }
}

#[test]
fn auth_status_help_lists_flags() {
    let dummy = dummy_config("atl-auth-status-help-dummy.toml");
    let (code, stdout, stderr) = run_atl(&["auth", "status", "--help"], &dummy);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("--profile"),
        "expected --profile in auth status help:\n{stdout}"
    );
}

#[test]
fn auth_token_help_lists_force_flag() {
    let dummy = dummy_config("atl-auth-token-help-dummy.toml");
    let (code, stdout, stderr) = run_atl(&["auth", "token", "--help"], &dummy);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("--force"),
        "expected --force in auth token help:\n{stdout}"
    );
    assert!(
        stdout.contains("--service"),
        "expected --service in auth token help:\n{stdout}"
    );
}

#[test]
fn auth_logout_help_lists_service_flag() {
    let dummy = dummy_config("atl-auth-logout-help-dummy.toml");
    let (code, stdout, stderr) = run_atl(&["auth", "logout", "--help"], &dummy);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("--service"),
        "expected --service in auth logout help:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Flow tests: drive the handler directly with an InMemoryStore so no real
// keyring is ever touched.
// ---------------------------------------------------------------------------

// The flow tests use `#[tokio::test]` to await the async handler; they hold
// a `std::sync::Mutex` guard across the await to serialize env mutations.
// That is safe here because the guarded code does not itself await on any
// primitive that could re-enter the lock, but clippy's heuristic can't see
// that, so suppress the lint at the module level.
#[allow(clippy::await_holding_lock)]
mod flow {
    //! Handler-level integration tests. These do not spawn the binary —
    //! they drive `atl::cli::commands::auth::run` directly with a
    //! buffer-backed `IoStreams`, an `InMemoryStore`, and an (empty)
    //! `MockPrompter` so no real keyring, TTY, or network is touched.
    //!
    //! The `login` subcommand is not exercised here because it requires
    //! either a real TTY (for interactive prompts) or pipe-friendly stdin
    //! injection on `IoStreams`, neither of which fits cleanly into the
    //! buffer-backed test IO. The login flow is covered by unit tests in
    //! `src/cli/commands/auth.rs` (resolvers, helpers) and by the resolver
    //! chain tests in `src/config/mod.rs`. This file covers `logout`,
    //! `status`, and `token` end-to-end against a real config file.

    use atl::auth::{InMemoryStore, MockPrompter, SecretStore, service_name};
    use atl::cli::args::{
        AuthLogoutArgs, AuthService, AuthStatusArgs, AuthSubcommand, AuthTokenArgs, SingleService,
    };
    use atl::cli::commands::auth as auth_cmd;
    use atl::config::{AtlassianInstance, AuthType, Config, ConfigLoader, Profile};
    use atl::io::IoStreams;
    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn clear_env() {
        // SAFETY: serialized via env_lock() in every caller.
        unsafe {
            std::env::remove_var("ATL_API_TOKEN");
            std::env::remove_var("ATL_PROFILE");
            std::env::remove_var("ATL_CONFIG");
        }
    }

    fn write_config(dir: &TempDir, config: &Config) -> Utf8PathBuf {
        let path = dir.path().join("atl.toml");
        let utf8 = Utf8PathBuf::try_from(path).unwrap();
        ConfigLoader::save(config, Some(utf8.as_path())).unwrap();
        utf8
    }

    fn make_config_with_jira() -> Config {
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
                confluence: None,
                ..Default::default()
            },
        );
        config
    }

    #[tokio::test]
    async fn token_prints_keyring_secret_with_force() {
        let _g = env_lock();
        clear_env();

        let dir = TempDir::new().unwrap();
        let config = make_config_with_jira();
        let path = write_config(&dir, &config);

        let store = InMemoryStore::new();
        let svc = service_name("default", "jira");
        store.set(&svc, "alice@example.com", "kr-token").unwrap();

        let mut io = IoStreams::test();
        let cmd = AuthSubcommand::Token(AuthTokenArgs {
            service: SingleService::Jira,
            profile: Some("default".into()),
            force: true,
        });
        let prompter = MockPrompter::new(vec![]);
        auth_cmd::run(
            &cmd,
            Some(path.as_path()),
            None,
            &mut io,
            &store,
            &prompter,
            0,
        )
        .await
        .unwrap();

        let out = io.stdout_as_string();
        assert!(
            out.contains("kr-token"),
            "expected keyring token in stdout:\n{out}"
        );
    }

    #[tokio::test]
    async fn token_errors_when_no_token_available() {
        let _g = env_lock();
        clear_env();

        let dir = TempDir::new().unwrap();
        let config = make_config_with_jira();
        let path = write_config(&dir, &config);

        let store = InMemoryStore::new();
        let mut io = IoStreams::test();
        let cmd = AuthSubcommand::Token(AuthTokenArgs {
            service: SingleService::Jira,
            profile: Some("default".into()),
            force: true,
        });
        let prompter = MockPrompter::new(vec![]);
        let err = auth_cmd::run(
            &cmd,
            Some(path.as_path()),
            None,
            &mut io,
            &store,
            &prompter,
            0,
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("no token available"),
            "expected 'no token available' error, got: {err}"
        );
    }

    #[tokio::test]
    async fn logout_removes_keyring_entry_and_clears_legacy_token() {
        let _g = env_lock();
        clear_env();

        let dir = TempDir::new().unwrap();
        let mut config = make_config_with_jira();
        // Seed a legacy plaintext token — logout should scrub it.
        if let Some(jira) = config
            .profiles
            .get_mut("default")
            .and_then(|p| p.jira.as_mut())
        {
            jira.api_token = Some("legacy-plaintext".into());
        }
        let path = write_config(&dir, &config);

        let store = InMemoryStore::new();
        let svc = service_name("default", "jira");
        store.set(&svc, "alice@example.com", "kr-token").unwrap();

        let mut io = IoStreams::test();
        let cmd = AuthSubcommand::Logout(AuthLogoutArgs {
            service: AuthService::Jira,
            profile: Some("default".into()),
        });
        let prompter = MockPrompter::new(vec![]);
        auth_cmd::run(
            &cmd,
            Some(path.as_path()),
            None,
            &mut io,
            &store,
            &prompter,
            0,
        )
        .await
        .unwrap();

        assert!(
            store.get(&svc, "alice@example.com").unwrap().is_none(),
            "keyring entry should be removed after logout"
        );

        let reloaded = ConfigLoader::load(Some(path.as_path())).unwrap().unwrap();
        let jira = reloaded
            .profiles
            .get("default")
            .and_then(|p| p.jira.as_ref())
            .unwrap();
        assert!(
            jira.api_token.is_none(),
            "legacy api_token should be cleared from TOML after logout"
        );
    }

    #[tokio::test]
    async fn logout_persists_legacy_token_cleanup_without_keyring_entry() {
        let _g = env_lock();
        clear_env();

        let dir = TempDir::new().unwrap();
        let mut config = make_config_with_jira();
        // Seed a legacy plaintext token — logout should scrub it even when
        // no keyring entry exists for the profile.
        if let Some(jira) = config
            .profiles
            .get_mut("default")
            .and_then(|p| p.jira.as_mut())
        {
            jira.api_token = Some("legacy-plaintext".into());
        }
        let path = write_config(&dir, &config);

        // Empty store — no keyring entry to remove.
        let store = InMemoryStore::new();

        let mut io = IoStreams::test();
        let cmd = AuthSubcommand::Logout(AuthLogoutArgs {
            service: AuthService::Jira,
            profile: Some("default".into()),
        });
        let prompter = MockPrompter::new(vec![]);
        auth_cmd::run(
            &cmd,
            Some(path.as_path()),
            None,
            &mut io,
            &store,
            &prompter,
            0,
        )
        .await
        .unwrap();

        let reloaded = ConfigLoader::load(Some(path.as_path())).unwrap().unwrap();
        let jira = reloaded
            .profiles
            .get("default")
            .and_then(|p| p.jira.as_ref())
            .unwrap();
        assert!(
            jira.api_token.is_none(),
            "legacy api_token should be cleared from TOML after logout even without a keyring entry"
        );
    }

    #[tokio::test]
    async fn logout_is_idempotent() {
        let _g = env_lock();
        clear_env();

        let dir = TempDir::new().unwrap();
        let config = make_config_with_jira();
        let path = write_config(&dir, &config);

        let store = InMemoryStore::new();
        let mut io = IoStreams::test();
        let cmd = AuthSubcommand::Logout(AuthLogoutArgs {
            service: AuthService::Jira,
            profile: Some("default".into()),
        });
        let prompter = MockPrompter::new(vec![]);

        // Two back-to-back logouts should both succeed.
        auth_cmd::run(
            &cmd,
            Some(path.as_path()),
            None,
            &mut io,
            &store,
            &prompter,
            0,
        )
        .await
        .unwrap();
        auth_cmd::run(
            &cmd,
            Some(path.as_path()),
            None,
            &mut io,
            &store,
            &prompter,
            0,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn status_reports_configured_and_missing_services() {
        let _g = env_lock();
        clear_env();

        let dir = TempDir::new().unwrap();
        let config = make_config_with_jira();
        let path = write_config(&dir, &config);

        let store = InMemoryStore::new();
        let mut io = IoStreams::test();
        let cmd = AuthSubcommand::Status(AuthStatusArgs {
            profile: None,
            skip_verify: true,
        });
        let prompter = MockPrompter::new(vec![]);
        auth_cmd::run(
            &cmd,
            Some(path.as_path()),
            None,
            &mut io,
            &store,
            &prompter,
            0,
        )
        .await
        .unwrap();

        let out = io.stdout_as_string();
        assert!(out.contains("default profile"), "stdout:\n{out}");
        assert!(
            out.contains("confluence") && out.contains("(not configured)"),
            "stdout should report confluence as not configured:\n{out}"
        );
        assert!(
            out.contains("jira") && out.contains("no token"),
            "stdout should report jira with no token:\n{out}"
        );
    }

    #[tokio::test]
    async fn status_reports_keyring_source_when_token_present() {
        let _g = env_lock();
        clear_env();

        let dir = TempDir::new().unwrap();
        let config = make_config_with_jira();
        let path = write_config(&dir, &config);

        let store = InMemoryStore::new();
        let svc = service_name("default", "jira");
        store.set(&svc, "alice@example.com", "kr-token").unwrap();

        let mut io = IoStreams::test();
        let cmd = AuthSubcommand::Status(AuthStatusArgs {
            profile: None,
            skip_verify: true,
        });
        let prompter = MockPrompter::new(vec![]);
        auth_cmd::run(
            &cmd,
            Some(path.as_path()),
            None,
            &mut io,
            &store,
            &prompter,
            0,
        )
        .await
        .unwrap();

        let out = io.stdout_as_string();
        assert!(
            out.contains("via keyring"),
            "expected 'via keyring' in status output:\n{out}"
        );
    }
}
