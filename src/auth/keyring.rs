//! [`SecretStore`] implementations: [`SystemKeyring`] for production and
//! [`InMemoryStore`] for tests.

use std::collections::HashMap;
use std::sync::{Mutex, Once};

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

/// Production store backed by the platform keyring (`keyring-core` crate
/// plus a per-platform default store crate).
///
/// Instances are zero-sized; construct via `SystemKeyring` literal.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemKeyring;

/// Installs the platform-appropriate default credential store on the first
/// call. Subsequent calls are a single relaxed atomic load (`Once`).
///
/// On unsupported targets, or when the platform store fails to construct
/// (e.g. headless Linux without `keyutils`, Docker without a kernel
/// keyring), the function silently leaves no default installed. In that
/// case `keyring_core::Entry::new` returns `Error::NoDefaultStore`, which
/// `SystemKeyring::get` / `delete` translate into the graceful `Ok(None)`
/// / `Ok(())` contract documented on [`SecretStore`].
fn ensure_default_store() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        #[cfg(target_os = "macos")]
        {
            match apple_native_keyring_store::keychain::Store::new() {
                Ok(store) => keyring_core::set_default_store(store),
                Err(err) => debug!(
                    "failed to construct macOS Keychain credential store: {err}; \
                     keyring lookups will degrade to None"
                ),
            }
        }
        #[cfg(target_os = "windows")]
        {
            match windows_native_keyring_store::Store::new() {
                Ok(store) => keyring_core::set_default_store(store),
                Err(err) => debug!(
                    "failed to construct Windows credential store: {err}; \
                     keyring lookups will degrade to None"
                ),
            }
        }
        #[cfg(target_os = "linux")]
        {
            match linux_keyutils_keyring_store::Store::new() {
                Ok(store) => keyring_core::set_default_store(store),
                Err(err) => debug!(
                    "failed to construct Linux keyutils credential store: {err}; \
                     keyring lookups will degrade to None"
                ),
            }
        }
    });
}

impl SecretStore for SystemKeyring {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
        ensure_default_store();
        let entry = match keyring_core::Entry::new(service, account) {
            Ok(e) => e,
            Err(keyring_core::Error::NoStorageAccess(err)) => {
                debug!("no keyring storage access on Entry::new: {err}");
                return Ok(None);
            }
            Err(keyring_core::Error::PlatformFailure(err)) => {
                debug!("keyring platform failure on Entry::new: {err}");
                return Ok(None);
            }
            Err(keyring_core::Error::NoDefaultStore) => {
                debug!("no default keyring store installed (service={service})");
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };

        match entry.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(keyring_core::Error::NoStorageAccess(err)) => {
                debug!("no keyring storage access on get_password: {err}");
                Ok(None)
            }
            Err(keyring_core::Error::PlatformFailure(err)) => {
                debug!("keyring platform failure on get_password: {err}");
                Ok(None)
            }
            Err(keyring_core::Error::NoDefaultStore) => {
                debug!("no default keyring store installed on get_password");
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<()> {
        ensure_default_store();
        // Propagate every error here. Silently dropping a write would leave
        // the user thinking their token was saved when it wasn't —
        // including the `NoDefaultStore` case on unsupported platforms.
        let entry = keyring_core::Entry::new(service, account)?;
        entry.set_password(secret)?;
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        ensure_default_store();
        let entry = match keyring_core::Entry::new(service, account) {
            Ok(e) => e,
            Err(keyring_core::Error::NoStorageAccess(_)) => return Ok(()),
            Err(keyring_core::Error::PlatformFailure(_)) => return Ok(()),
            Err(keyring_core::Error::NoDefaultStore) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(keyring_core::Error::NoStorageAccess(_)) => Ok(()),
            Err(keyring_core::Error::PlatformFailure(_)) => Ok(()),
            Err(keyring_core::Error::NoDefaultStore) => Ok(()),
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

    // -------------------------------------------------------------------
    // Defaults & construction
    // -------------------------------------------------------------------

    #[test]
    fn in_memory_default_is_equivalent_to_new() {
        let from_default: InMemoryStore = Default::default();
        let from_new = InMemoryStore::new();
        assert_eq!(from_default.is_empty(), from_new.is_empty());
        assert_eq!(from_default.len(), from_new.len());
    }

    #[test]
    fn system_keyring_default_zero_sized() {
        // SystemKeyring is `Default + Clone + Copy` so callers can use
        // `SystemKeyring` as a literal anywhere a `&dyn SecretStore` is
        // accepted. A non-default impl would force every command handler
        // to `SystemKeyring::default()`.
        let _: SystemKeyring = Default::default();
        let _: SystemKeyring = SystemKeyring;
    }

    // -------------------------------------------------------------------
    // SecretStore contract: empty service / account / secret strings.
    // The trait does not specify validation — these are pure pass-through
    // operations and test that the store doesn't reject them silently.
    // -------------------------------------------------------------------

    #[test]
    fn in_memory_accepts_empty_service_and_account() {
        let store = InMemoryStore::new();
        store.set("", "", "value").unwrap();
        assert_eq!(store.get("", "").unwrap().as_deref(), Some("value"));
    }

    #[test]
    fn in_memory_accepts_empty_secret() {
        // Empty secret is a valid value — distinguishes "stored empty" from
        // "missing" via the `Option<String>` return type.
        let store = InMemoryStore::new();
        store.set("svc", "acct", "").unwrap();
        assert_eq!(store.get("svc", "acct").unwrap().as_deref(), Some(""));
    }

    #[test]
    fn in_memory_overwrite_with_empty_secret_does_not_delete() {
        // Setting "" must still leave the entry present.
        let store = InMemoryStore::new();
        store.set("svc", "acct", "old").unwrap();
        store.set("svc", "acct", "").unwrap();
        assert_eq!(store.get("svc", "acct").unwrap().as_deref(), Some(""));
        assert_eq!(store.len(), 1);
    }

    // -------------------------------------------------------------------
    // SecretStore contract: delete only removes the targeted entry.
    // -------------------------------------------------------------------

    #[test]
    fn in_memory_delete_only_removes_matching_entry() {
        let store = InMemoryStore::new();
        store.set("svc", "alice", "tok-a").unwrap();
        store.set("svc", "bob", "tok-b").unwrap();
        store.set("svc-other", "alice", "tok-c").unwrap();

        store.delete("svc", "alice").unwrap();

        assert!(store.get("svc", "alice").unwrap().is_none());
        assert_eq!(store.get("svc", "bob").unwrap().as_deref(), Some("tok-b"));
        assert_eq!(
            store.get("svc-other", "alice").unwrap().as_deref(),
            Some("tok-c")
        );
        assert_eq!(store.len(), 2);
    }

    // -------------------------------------------------------------------
    // SecretStore + service_name composition: mirrors the production
    // resolved_token path so a regression in either side surfaces here.
    // -------------------------------------------------------------------

    #[test]
    fn in_memory_with_service_name_keyring_layout() {
        use crate::auth::service_name;
        let store = InMemoryStore::new();

        store
            .set(&service_name("default", "jira"), "alice@acme.com", "j-tok")
            .unwrap();
        store
            .set(
                &service_name("default", "confluence"),
                "alice@acme.com",
                "c-tok",
            )
            .unwrap();
        store
            .set(&service_name("staging", "jira"), "alice@acme.com", "s-tok")
            .unwrap();

        assert_eq!(
            store
                .get(&service_name("default", "jira"), "alice@acme.com")
                .unwrap()
                .as_deref(),
            Some("j-tok")
        );
        assert_eq!(
            store
                .get(&service_name("default", "confluence"), "alice@acme.com")
                .unwrap()
                .as_deref(),
            Some("c-tok")
        );
        assert_eq!(
            store
                .get(&service_name("staging", "jira"), "alice@acme.com")
                .unwrap()
                .as_deref(),
            Some("s-tok")
        );
        // Nonexistent profile must not surface another profile's token.
        assert!(
            store
                .get(&service_name("missing", "jira"), "alice@acme.com")
                .unwrap()
                .is_none()
        );
    }

    // -------------------------------------------------------------------
    // SecretStore is `Send + Sync` — atl shares one store across
    // commands. This is a compile-time assertion: if the trait or the
    // impls regress, the test fails to compile.
    // -------------------------------------------------------------------

    #[test]
    fn secret_store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<InMemoryStore>();
        assert_send_sync::<SystemKeyring>();
    }

    // -------------------------------------------------------------------
    // Cross-thread sharing through Arc<dyn SecretStore> — the production
    // pattern. Verifies the Mutex inside InMemoryStore actually serializes
    // concurrent writes (no lost updates).
    // -------------------------------------------------------------------

    #[test]
    fn in_memory_thread_safe_writes() {
        use std::sync::Arc;
        use std::thread;

        let store: Arc<dyn SecretStore> = Arc::new(InMemoryStore::new());
        let mut handles = Vec::new();
        for i in 0..16 {
            let s = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                let svc = format!("svc-{i}");
                s.set(&svc, "acct", &format!("v-{i}")).unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // We don't have InMemoryStore::len through &dyn SecretStore, but
        // we can verify each entry made it back.
        for i in 0..16 {
            let svc = format!("svc-{i}");
            assert_eq!(
                store.get(&svc, "acct").unwrap().as_deref(),
                Some(&*format!("v-{i}"))
            );
        }
    }
}
