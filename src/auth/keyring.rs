//! [`SecretStore`] implementations: [`SystemKeyring`] for production and
//! [`InMemoryStore`] for tests.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use tracing::debug;

/// Trait abstraction over a platform credential store.
///
/// Implementors must be `Send + Sync` so `atl` can share the same store
/// instance across command handlers without extra locking.
pub trait SecretStore: Send + Sync {
    /// Returns the secret for the given `(service, account)` pair.
    ///
    /// * `Ok(Some(secret))` — the entry exists.
    /// * `Ok(None)` — the entry does not exist, or the platform credential
    ///   store is inaccessible (Docker, CI, headless Linux without Gnome
    ///   Keyring / KWallet). Missing backends are a graceful "no token" so
    ///   `atl` still falls through to env / TOML.
    /// * `Err(_)` — a genuine platform error the caller should surface.
    fn get(&self, service: &str, account: &str) -> Result<Option<String>>;

    /// Writes `secret` for the given `(service, account)` pair, replacing
    /// any existing value.
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<()>;

    /// Removes the `(service, account)` entry if it exists. A missing entry
    /// is **not** an error — `delete` is idempotent so `atl auth logout`
    /// can be run twice without surprising the user.
    fn delete(&self, service: &str, account: &str) -> Result<()>;
}

/// Production store backed by the platform keyring (`keyring` crate).
///
/// Instances are zero-sized; construct via `SystemKeyring` literal.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemKeyring;

impl SecretStore for SystemKeyring {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
        let entry = match keyring::Entry::new(service, account) {
            Ok(e) => e,
            Err(keyring::Error::NoStorageAccess(_)) => {
                debug!("no keyring storage backend available (service={service})");
                return Ok(None);
            }
            Err(keyring::Error::PlatformFailure(err)) => {
                debug!("keyring platform failure on Entry::new: {err}");
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };

        match entry.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::NoStorageAccess(err)) => {
                debug!("no keyring storage access on get_password: {err}");
                Ok(None)
            }
            Err(keyring::Error::PlatformFailure(err)) => {
                debug!("keyring platform failure on get_password: {err}");
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<()> {
        let entry = keyring::Entry::new(service, account)?;
        entry.set_password(secret)?;
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        let entry = match keyring::Entry::new(service, account) {
            Ok(e) => e,
            Err(keyring::Error::NoStorageAccess(_)) => return Ok(()),
            Err(keyring::Error::PlatformFailure(_)) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(keyring::Error::NoStorageAccess(_)) => Ok(()),
            Err(keyring::Error::PlatformFailure(_)) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// In-memory store for tests.
///
/// Deliberately not `#[cfg(test)]` so integration tests under `tests/` can
/// import it (`atl::auth::InMemoryStore`).
#[derive(Debug, Default)]
pub struct InMemoryStore {
    inner: Mutex<HashMap<(String, String), String>>,
}

impl InMemoryStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the total number of entries. Useful in tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or_default()
    }

    /// Returns `true` when the store contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecretStore for InMemoryStore {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("in-memory store mutex poisoned"))?;
        Ok(guard
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("in-memory store mutex poisoned"))?;
        guard.insert(
            (service.to_string(), account.to_string()),
            secret.to_string(),
        );
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("in-memory store mutex poisoned"))?;
        guard.remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_get_returns_none_when_missing() {
        let store = InMemoryStore::new();
        assert!(store.get("svc", "acct").unwrap().is_none());
    }

    #[test]
    fn in_memory_set_then_get() {
        let store = InMemoryStore::new();
        store.set("svc", "acct", "shh").unwrap();
        assert_eq!(store.get("svc", "acct").unwrap().as_deref(), Some("shh"));
    }

    #[test]
    fn in_memory_overwrite() {
        let store = InMemoryStore::new();
        store.set("svc", "acct", "old").unwrap();
        store.set("svc", "acct", "new").unwrap();
        assert_eq!(store.get("svc", "acct").unwrap().as_deref(), Some("new"));
    }

    #[test]
    fn in_memory_delete_existing() {
        let store = InMemoryStore::new();
        store.set("svc", "acct", "shh").unwrap();
        store.delete("svc", "acct").unwrap();
        assert!(store.get("svc", "acct").unwrap().is_none());
    }

    #[test]
    fn in_memory_delete_missing_is_ok() {
        let store = InMemoryStore::new();
        // Second delete must also succeed — the contract is idempotent.
        store.delete("svc", "acct").unwrap();
        store.delete("svc", "acct").unwrap();
    }

    #[test]
    fn in_memory_scoped_by_service_and_account() {
        let store = InMemoryStore::new();
        store.set("svc1", "a", "one").unwrap();
        store.set("svc2", "a", "two").unwrap();
        store.set("svc1", "b", "three").unwrap();
        assert_eq!(store.get("svc1", "a").unwrap().as_deref(), Some("one"));
        assert_eq!(store.get("svc2", "a").unwrap().as_deref(), Some("two"));
        assert_eq!(store.get("svc1", "b").unwrap().as_deref(), Some("three"));
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn in_memory_is_empty_initially() {
        let store = InMemoryStore::new();
        assert!(store.is_empty());
        store.set("s", "a", "x").unwrap();
        assert!(!store.is_empty());
    }
}
