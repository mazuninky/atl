use std::path::PathBuf;

use tempfile::TempDir;

/// A temporary test configuration with automatic cleanup.
///
/// The underlying `TempDir` is kept alive for RAII -- the config file remains valid
/// as long as this value is alive.
pub struct TestConfig {
    _dir: TempDir,
    pub config_path: PathBuf,
}

/// Builder for constructing a [`TestConfig`] with per-service domains and settings.
pub struct TestConfigBuilder {
    jira_domain: Option<String>,
    jira_flavor: Option<String>,
    confluence_domain: Option<String>,
    confluence_api_path: Option<String>,
    read_only: bool,
}

impl TestConfigBuilder {
    /// Create a new builder with default settings (read_only=false, no api_path).
    #[must_use]
    pub fn new() -> Self {
        Self {
            jira_domain: None,
            jira_flavor: None,
            confluence_domain: None,
            confluence_api_path: None,
            read_only: false,
        }
    }

    /// Set the Jira instance domain (e.g. `http://127.0.0.1:4010`).
    #[must_use]
    pub fn jira(mut self, domain: &str) -> Self {
        self.jira_domain = Some(domain.to_string());
        self
    }

    /// Set the explicit Jira flavor override (e.g. `"cloud"` or `"data_center"`).
    ///
    /// When unset, the flavor is auto-detected from the domain — `*.atlassian.net`
    /// resolves to Cloud, everything else (including `127.0.0.1` used by the
    /// contract test Prism mock) resolves to Data Center. Set this to `"cloud"`
    /// to exercise the Jira Cloud code paths (v3 `/search/jql`, v3 `/issue/bulk`,
    /// v3 archive) against a Prism mock bound to a non-Cloud host.
    #[must_use]
    pub fn jira_flavor(mut self, flavor: &str) -> Self {
        self.jira_flavor = Some(flavor.to_string());
        self
    }

    /// Set the Confluence instance domain (e.g. `http://127.0.0.1:4011`).
    #[must_use]
    pub fn confluence(mut self, domain: &str) -> Self {
        self.confluence_domain = Some(domain.to_string());
        self
    }

    /// Set the Confluence API path (e.g. `/wiki/rest/api`).
    #[must_use]
    pub fn confluence_api_path(mut self, path: &str) -> Self {
        self.confluence_api_path = Some(path.to_string());
        self
    }

    /// Set whether instances should be read-only.
    #[must_use]
    pub fn read_only(mut self, val: bool) -> Self {
        self.read_only = val;
        self
    }

    /// Build the [`TestConfig`], writing a TOML config file into a temporary directory.
    ///
    /// The resulting config has a single profile named "test" with default_project="TEST"
    /// and default_space="TEST".
    ///
    /// # Panics
    ///
    /// Panics if neither [`Self::jira`] nor [`Self::confluence`] was called —
    /// a config without any service is useless and would only surface as an
    /// opaque "no service configured" error deep inside atl.
    pub fn build(self) -> TestConfig {
        assert!(
            self.jira_domain.is_some() || self.confluence_domain.is_some(),
            "TestConfigBuilder::build() called without configuring jira() or confluence(). \
             Set at least one service before calling build()."
        );
        let dir = TempDir::new().expect("failed to create temp dir for test config");

        let jira_section = self
            .jira_domain
            .as_ref()
            .map(|domain| {
                let flavor_line = self
                    .jira_flavor
                    .as_ref()
                    .map(|f| format!("flavor = \"{f}\"\n"))
                    .unwrap_or_default();

                format!(
                    r#"
[profiles.test.jira]
domain = "{domain}"
email = "test@example.com"
auth_type = "basic"
{flavor_line}read_only = {read_only}
"#,
                    read_only = self.read_only,
                )
            })
            .unwrap_or_default();

        let confluence_section = self
            .confluence_domain
            .as_ref()
            .map(|domain| {
                let api_path_line = self
                    .confluence_api_path
                    .as_ref()
                    .map(|p| format!("api_path = \"{p}\"\n"))
                    .unwrap_or_default();

                format!(
                    r#"
[profiles.test.confluence]
domain = "{domain}"
email = "test@example.com"
auth_type = "basic"
{api_path_line}read_only = {read_only}
"#,
                    read_only = self.read_only,
                )
            })
            .unwrap_or_default();

        let toml_content = format!(
            r#"default_profile = "test"

[profiles.test]
default_project = "TEST"
default_space = "TEST"
{jira_section}{confluence_section}"#,
        );

        let config_path = dir.path().join("atl.toml");
        std::fs::write(&config_path, toml_content).expect("failed to write test config");

        TestConfig {
            _dir: dir,
            config_path,
        }
    }
}

impl Default for TestConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
