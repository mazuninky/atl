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
