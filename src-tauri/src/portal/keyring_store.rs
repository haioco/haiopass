//! OS keyring persistence for the portal session.
//!
//! Tokens are stored as one JSON blob under a fixed service/account pair. The
//! mobile number is never part of the entry name. When the platform keyring is
//! unavailable (or this build resolved the mock store), we keep the session in
//! memory and report `persistent = false` to the UI instead of pretending.

use keyring::credential::CredentialPersistence;
use std::sync::{Arc, Mutex};

pub const KEYRING_SERVICE: &str = "ir.haio.bypass.portal";
pub const KEYRING_ACCOUNT: &str = "session";

/// Minimal, redacted error. Never carries the secret or the raw OS error text,
/// which may include the credential path.
#[derive(Debug, Clone, Copy)]
pub struct StoreError(pub &'static str);

pub type StoreResult<T> = Result<T, StoreError>;

pub trait TokenStore: Send + Sync {
    fn load(&self) -> StoreResult<Option<String>>;
    fn save(&self, value: &str) -> StoreResult<()>;
    fn clear(&self) -> StoreResult<()>;
}

pub struct KeyringTokenStore {
    service: String,
    account: String,
}

impl KeyringTokenStore {
    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.to_string(),
            account: account.to_string(),
        }
    }

    fn entry(&self) -> StoreResult<keyring::Entry> {
        keyring::Entry::new(&self.service, &self.account)
            .map_err(|_| StoreError("keyring_entry_failed"))
    }
}

impl TokenStore for KeyringTokenStore {
    fn load(&self) -> StoreResult<Option<String>> {
        let entry = self.entry()?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(StoreError("keyring_read_failed")),
        }
    }

    fn save(&self, value: &str) -> StoreResult<()> {
        let entry = self.entry()?;
        entry
            .set_password(value)
            .map_err(|_| StoreError("keyring_write_failed"))
    }

    fn clear(&self) -> StoreResult<()> {
        let entry = self.entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(StoreError("keyring_delete_failed")),
        }
    }
}

pub struct MemoryTokenStore {
    value: Mutex<Option<String>>,
}

impl Default for MemoryTokenStore {
    fn default() -> Self {
        Self {
            value: Mutex::new(None),
        }
    }
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> StoreResult<Option<String>> {
        Ok(self
            .value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }

    fn save(&self, value: &str) -> StoreResult<()> {
        *self
            .value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(value.to_string());
        Ok(())
    }

    fn clear(&self) -> StoreResult<()> {
        *self
            .value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        Ok(())
    }
}

pub struct StoreHandle {
    pub store: Arc<dyn TokenStore>,
    /// True only when credentials survive an app restart.
    pub persistent: bool,
}

/// Is this build backed by a store that persists until an explicit delete?
///
/// The mock store (no platform feature enabled) reports `EntryOnly` and dies
/// with the process, so it must be treated as non-persistent.
pub fn keyring_is_persistent() -> bool {
    matches!(
        keyring::default::default_credential_builder().persistence(),
        CredentialPersistence::UntilDelete
    )
}

/// Build the default store: keyring when it can persist, memory otherwise.
pub fn default_store() -> StoreHandle {
    if keyring_is_persistent() {
        StoreHandle {
            store: Arc::new(KeyringTokenStore::new(KEYRING_SERVICE, KEYRING_ACCOUNT)),
            persistent: true,
        }
    } else {
        StoreHandle {
            store: Arc::new(MemoryTokenStore::new()),
            persistent: false,
        }
    }
}

pub fn memory_store() -> StoreHandle {
    StoreHandle {
        store: Arc::new(MemoryTokenStore::new()),
        persistent: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let store = MemoryTokenStore::new();
        assert!(store.load().unwrap().is_none());
        store.save("{\"access\":\"a\"}").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some("{\"access\":\"a\"}"));
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn memory_store_clear_is_idempotent() {
        let store = MemoryTokenStore::new();
        store.clear().unwrap();
        store.clear().unwrap();
    }

    #[test]
    fn persistence_detection_runs() {
        // Must not panic on any platform; the resulting value depends on the
        // compiled-in credential store features.
        let _ = keyring_is_persistent();
    }
}
