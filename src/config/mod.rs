mod loader;

pub use loader::ConfigLoader;

use serde::{Deserialize, Serialize};

pub const CONFIG_FILE_NAME: &str = "atl.toml";
pub const CONFIG_DIR_NAME: &str = "atl";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub default_profile: String,
    #[serde(default)]
    pub profiles: std::collections::HashMap<String, Profile>,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub aliases: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenStorage {
    #[default]
    Keyring,
    Config,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confluence: Option<AtlassianInstance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jira: Option<AtlassianInstance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_space: Option<String>,
    #[serde(default)]
    pub token_storage: TokenStorage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlassianInstance {
    pub domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// API token stored directly in the config file. Alternative to
    /// the OS keyring — simpler setup, works without keychain prompts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
    #[serde(default)]
    pub auth_type: AuthType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_path: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    /// Explicit Jira flavor override. When `None`, the flavor is
    /// auto-detected from the domain (`*.atlassian.net` = Cloud,
    /// everything else = Data Center / Server). Ignored for
    /// Confluence instances — it only affects Jira routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavor: Option<JiraFlavor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthType {
    #[default]
    Basic,
    Bearer,
}

/// Jira deployment flavor.
///
/// Jira Cloud (`*.atlassian.net`) and Jira Data Center / Server have
/// diverging REST APIs: the v3 endpoints (`/rest/api/3/*`) only exist on
/// Cloud, while v2 endpoints (`/rest/api/2/*`) exist on both but with
/// different pagination shapes for some routes. The client layer branches
/// on this enum to pick the right route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JiraFlavor {
    #[default]
    Cloud,
    DataCenter,
}

impl Config {
    pub fn resolve_profile(&self, name: Option<&str>) -> Option<&Profile> {
        let key = name.unwrap_or(&self.default_profile);
        self.profiles.get(key)
    }
}

impl AtlassianInstance {
    /// Resolves the API token using the full `env → TOML → keyring` chain.
    ///
    /// # Parameters
    ///
    /// * `profile` — profile name, used to build the keyring service key
    ///   (`"atl:<profile>:<kind>"`).
    /// * `kind` — `"confluence"` or `"jira"`; selects the keyring entry that
    ///   belongs to this instance inside the profile.
    /// * `store` — secret store used to look up the keyring entry. Tests
    ///   pass an [`crate::auth::InMemoryStore`]; production code passes
    ///   [`crate::auth::SystemKeyring`].
    ///
    /// Returns `None` when no token is available from any source.
    /// Resolves the Jira flavor for this instance.
    ///
    /// An explicit [`AtlassianInstance::flavor`] override wins. Otherwise the
    /// flavor is auto-detected from the domain: anything under
    /// `*.atlassian.net` is treated as [`JiraFlavor::Cloud`], everything else
    /// (self-hosted hostnames like `jira.company.com`) is treated as
    /// [`JiraFlavor::DataCenter`].
    ///
    /// The detection is scheme-insensitive — `https://acme.atlassian.net` and
    /// `acme.atlassian.net` resolve identically. Path suffixes such as
    /// `acme.atlassian.net/wiki` are preserved (Cloud).
    #[must_use]
    pub fn resolved_flavor(&self) -> JiraFlavor {
        if let Some(f) = self.flavor {
            return f;
        }
        let d = self
            .domain
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        if d.ends_with(".atlassian.net") || d.contains(".atlassian.net/") {
            JiraFlavor::Cloud
        } else {
            JiraFlavor::DataCenter
        }
    }

    pub fn resolved_token(
        &self,
        profile: &str,
        kind: &str,
        store: &dyn crate::auth::SecretStore,
    ) -> Option<String> {
        // 1. Env var always wins — keeps CI workflows stable.
        if let Ok(env_token) = std::env::var("ATL_API_TOKEN")
            && !env_token.trim().is_empty()
        {
            return Some(env_token);
        }

        // 2. Token from config file.
        if let Some(toml_token) = self.api_token.as_ref() {
            return Some(toml_token.clone());
        }

        // 3. Keyring lookup. A missing keyring backend returns `Ok(None)`
        //    from the store (see `SystemKeyring::get`), so we fall through
        //    to `None` without surfacing an error.
        let account = self.email.as_deref().unwrap_or("default");
        let svc = crate::auth::service_name(profile, kind);
        match store.get(&svc, account) {
            Ok(secret) => secret,
            Err(err) => {
                tracing::debug!("keyring lookup failed for {svc}: {err}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_profile_default() {
        let mut profiles = std::collections::HashMap::new();
        profiles.insert("default".to_string(), Profile::default());
        let config = Config {
            default_profile: "default".to_string(),
            profiles,
            ..Default::default()
        };
        assert!(config.resolve_profile(None).is_some());
    }

    #[test]
    fn resolve_profile_explicit() {
        let mut profiles = std::collections::HashMap::new();
        profiles.insert("staging".to_string(), Profile::default());
        let config = Config {
            default_profile: "default".to_string(),
            profiles,
            ..Default::default()
        };
        assert!(config.resolve_profile(Some("staging")).is_some());
        assert!(config.resolve_profile(Some("missing")).is_none());
    }

    // -------------------------------------------------------------------
    // resolved_token resolution chain
    //
    // These tests mutate `ATL_API_TOKEN`, so they acquire a shared lock
    // first to stop them from racing each other on the global env.
    // -------------------------------------------------------------------

    use crate::auth::SecretStore as _;
    use crate::test_util::env_lock;

    fn make_instance(token: Option<&str>, email: Option<&str>) -> AtlassianInstance {
        AtlassianInstance {
            domain: "example.atlassian.net".into(),
            email: email.map(String::from),
            api_token: token.map(String::from),
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor: None,
        }
    }

    // -------------------------------------------------------------------
    // resolved_flavor: auto-detect + explicit override
    // -------------------------------------------------------------------

    fn make_flavor_instance(domain: &str, flavor: Option<JiraFlavor>) -> AtlassianInstance {
        AtlassianInstance {
            domain: domain.into(),
            email: None,
            api_token: None,
            auth_type: AuthType::Basic,
            api_path: None,
            read_only: false,
            flavor,
        }
    }

    #[test]
    fn resolved_flavor_cloud_from_bare_atlassian_net() {
        let inst = make_flavor_instance("acme.atlassian.net", None);
        assert_eq!(inst.resolved_flavor(), JiraFlavor::Cloud);
    }

    #[test]
    fn resolved_flavor_cloud_from_https_url_with_trailing_slash() {
        let inst = make_flavor_instance("https://acme.atlassian.net/", None);
        assert_eq!(inst.resolved_flavor(), JiraFlavor::Cloud);
    }

    #[test]
    fn resolved_flavor_cloud_from_atlassian_net_with_wiki_subpath() {
        let inst = make_flavor_instance("acme.atlassian.net/wiki", None);
        assert_eq!(inst.resolved_flavor(), JiraFlavor::Cloud);
    }

    #[test]
    fn resolved_flavor_data_center_from_self_hosted_domain() {
        let inst = make_flavor_instance("jira.company.com", None);
        assert_eq!(inst.resolved_flavor(), JiraFlavor::DataCenter);
    }

    #[test]
    fn resolved_flavor_explicit_data_center_overrides_atlassian_net() {
        let inst = make_flavor_instance("acme.atlassian.net", Some(JiraFlavor::DataCenter));
        assert_eq!(inst.resolved_flavor(), JiraFlavor::DataCenter);
    }

    #[test]
    fn resolved_flavor_explicit_cloud_overrides_self_hosted() {
        let inst = make_flavor_instance("jira.company.com", Some(JiraFlavor::Cloud));
        assert_eq!(inst.resolved_flavor(), JiraFlavor::Cloud);
    }

    #[test]
    fn resolved_token_env_wins_over_toml_and_keyring() {
        let _g = env_lock();
        // SAFETY: serialized by env_lock().
        unsafe { std::env::set_var("ATL_API_TOKEN", "env-token") };

        let store = crate::auth::InMemoryStore::new();
        store
            .set("atl:default:jira", "alice@acme.com", "kr-token")
            .unwrap();
        let inst = make_instance(Some("toml-token"), Some("alice@acme.com"));

        assert_eq!(
            inst.resolved_token("default", "jira", &store).as_deref(),
            Some("env-token")
        );

        unsafe { std::env::remove_var("ATL_API_TOKEN") };
    }

    #[test]
    fn resolved_token_toml_over_keyring() {
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let store = crate::auth::InMemoryStore::new();
        store
            .set("atl:default:jira", "alice@acme.com", "kr-token")
            .unwrap();
        let inst = make_instance(Some("toml-token"), Some("alice@acme.com"));

        assert_eq!(
            inst.resolved_token("default", "jira", &store).as_deref(),
            Some("toml-token")
        );
    }

    #[test]
    fn resolved_token_keyring_when_no_env_or_toml() {
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let store = crate::auth::InMemoryStore::new();
        store
            .set("atl:default:jira", "alice@acme.com", "kr-token")
            .unwrap();
        let inst = make_instance(None, Some("alice@acme.com"));

        assert_eq!(
            inst.resolved_token("default", "jira", &store).as_deref(),
            Some("kr-token")
        );
    }

    #[test]
    fn resolved_token_none_when_nothing_set() {
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let store = crate::auth::InMemoryStore::new();
        let inst = make_instance(None, Some("alice@acme.com"));

        assert!(inst.resolved_token("default", "jira", &store).is_none());
    }

    #[test]
    fn resolved_token_uses_default_account_when_email_missing() {
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let store = crate::auth::InMemoryStore::new();
        store
            .set("atl:default:jira", "default", "bearer-token")
            .unwrap();
        let inst = make_instance(None, None);

        assert_eq!(
            inst.resolved_token("default", "jira", &store).as_deref(),
            Some("bearer-token")
        );
    }

    #[test]
    fn resolved_token_scoped_by_profile_and_kind() {
        let _g = env_lock();
        unsafe { std::env::remove_var("ATL_API_TOKEN") };

        let store = crate::auth::InMemoryStore::new();
        store
            .set("atl:default:jira", "default", "jira-tok")
            .unwrap();
        store
            .set("atl:default:confluence", "default", "conf-tok")
            .unwrap();
        store
            .set("atl:staging:jira", "default", "staging-tok")
            .unwrap();
        let inst = make_instance(None, None);

        assert_eq!(
            inst.resolved_token("default", "jira", &store).as_deref(),
            Some("jira-tok")
        );
        assert_eq!(
            inst.resolved_token("default", "confluence", &store)
                .as_deref(),
            Some("conf-tok")
        );
        assert_eq!(
            inst.resolved_token("staging", "jira", &store).as_deref(),
            Some("staging-tok")
        );
    }
}
