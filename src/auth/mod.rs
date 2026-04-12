//! Secret storage and interactive prompting for `atl auth`.
//!
//! This module hosts the two trait abstractions that make the authentication
//! wizard testable end-to-end:
//!
//! * [`SecretStore`] — retrieve / persist / delete a secret for a given
//!   `(service, account)` pair. The production implementation is
//!   [`keyring::SystemKeyring`], which delegates to the platform credential
//!   store. Tests use [`keyring::InMemoryStore`] so they never touch a real
//!   keyring.
//! * [`Prompter`] — text / password / select / confirm prompts. The production
//!   implementation is [`prompter::InquirePrompter`], which wraps the
//!   `inquire` crate. Tests use [`prompter::MockPrompter`] to drive a scripted
//!   conversation.
//!
//! The resolver that walks `env → TOML → keyring` lives on
//! [`crate::config::AtlassianInstance`] as `resolved_token` — see
//! `config/mod.rs`.

pub mod keyring;
pub mod prompter;

pub use keyring::{InMemoryStore, SecretStore, SystemKeyring};
pub use prompter::{InquirePrompter, MockPrompter, MockResponse, Prompter};

/// Returns the keyring service name under which `atl` stores tokens for a
/// given profile / service combination.
///
/// The format (`"atl:<profile>:<kind>"`) is stable — it appears in the
/// OS keychain UI, so changing it would strand existing tokens.
#[must_use]
pub fn service_name(profile: &str, kind: &str) -> String {
    format!("atl:{profile}:{kind}")
}

/// Returns a clone of `instance` with `api_token` populated from the
/// resolution chain (env → TOML → keyring).
///
/// Call sites use this to decouple token lookup from
/// [`crate::client::build_http_client`]: instead of teaching the client
/// builder about keyrings, we pre-resolve the token at the command-handler
/// level and hand it a clone with the token already inlined.
///
/// Returns the cloned instance unchanged when no token is available — the
/// client builder will then surface the familiar "no API token configured"
/// error to the user.
#[must_use]
pub fn resolve_instance(
    instance: &crate::config::AtlassianInstance,
    profile: &str,
    kind: &str,
) -> crate::config::AtlassianInstance {
    // Keeping the store as a local avoids threading it through every
    // command handler. The `SystemKeyring` itself is zero-sized so
    // construction is free.
    let store = SystemKeyring;
    resolve_instance_with(instance, profile, kind, &store)
}

/// Same as [`resolve_instance`] but uses the supplied [`SecretStore`]. Used
/// by tests so they can inject an [`InMemoryStore`].
#[must_use]
pub fn resolve_instance_with(
    instance: &crate::config::AtlassianInstance,
    profile: &str,
    kind: &str,
    store: &dyn SecretStore,
) -> crate::config::AtlassianInstance {
    let mut cloned = instance.clone();
    if let Some(token) = instance.resolved_token(profile, kind, store) {
        cloned.api_token = Some(token);
    }
    cloned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_format() {
        assert_eq!(service_name("default", "jira"), "atl:default:jira");
        assert_eq!(
            service_name("staging", "confluence"),
            "atl:staging:confluence"
        );
    }

    #[test]
    fn service_name_handles_empty_components() {
        // Not a contract, but documents current behaviour.
        assert_eq!(service_name("", "jira"), "atl::jira");
        assert_eq!(service_name("default", ""), "atl:default:");
    }
}
