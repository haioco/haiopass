//! OS keyring persistence for the device private key.
//!
//! The key is one JSON-free PEM blob per identity, stored under the fixed
//! service `ir.haio.bypass.device` and an account name that names the
//! credential (primary account id or member id). The key is never written to
//! `state.json` or any other plaintext file.
//!
//! When the platform keyring cannot persist until an explicit delete (no
//! Secret Service, a build without a native backend, …) the caller must refuse
//! to enroll rather than silently store the key in memory: a device identity
//! that vanishes on restart is worse than a clear "cannot remember this
//! device" error.

use keyring::credential::CredentialPersistence;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const KEYRING_SERVICE: &str = "ir.haio.bypass.device";

/// Minimal, redacted error. Never carries the secret or the raw OS error text,
/// which may include the credential path.
#[derive(Debug, Clone, Copy)]
pub struct StoreError(pub &'static str);

pub type StoreResult<T> = Result<T, StoreError>;

pub trait DeviceKeyStore: Send + Sync {
    fn load(&self, account: &str) -> StoreResult<Option<String>>;
    fn save(&self, account: &str, value: &str) -> StoreResult<()>;
    fn clear(&self, account: &str) -> StoreResult<()>;
}

pub struct KeyringDeviceKeyStore {
    service: String,
}

impl KeyringDeviceKeyStore {
    pub fn new() -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
        }
    }

    fn entry(&self, account: &str) -> StoreResult<keyring::Entry> {
        keyring::Entry::new(&self.service, account).map_err(|_| StoreError("keyring_entry_failed"))
    }
}

impl Default for KeyringDeviceKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceKeyStore for KeyringDeviceKeyStore {
    fn load(&self, account: &str) -> StoreResult<Option<String>> {
        let entry = self.entry(account)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(StoreError("keyring_read_failed")),
        }
    }

    fn save(&self, account: &str, value: &str) -> StoreResult<()> {
        let entry = self.entry(account)?;
        entry
            .set_password(value)
            .map_err(|_| StoreError("keyring_write_failed"))
    }

    fn clear(&self, account: &str) -> StoreResult<()> {
        let entry = self.entry(account)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(StoreError("keyring_delete_failed")),
        }
    }
}

/// Test-only store. It isolates accounts and never touches the OS keyring.
pub struct MemoryDeviceKeyStore {
    values: Mutex<HashMap<String, String>>,
}

impl Default for MemoryDeviceKeyStore {
    fn default() -> Self {
        Self {
            values: Mutex::new(HashMap::new()),
        }
    }
}

impl MemoryDeviceKeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl DeviceKeyStore for MemoryDeviceKeyStore {
    fn load(&self, account: &str) -> StoreResult<Option<String>> {
        Ok(self.lock().get(account).cloned())
    }

    fn save(&self, account: &str, value: &str) -> StoreResult<()> {
        self.lock().insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn clear(&self, account: &str) -> StoreResult<()> {
        self.lock().remove(account);
        Ok(())
    }
}

pub struct DeviceStoreHandle {
    pub store: Arc<dyn DeviceKeyStore>,
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
pub fn default_store() -> DeviceStoreHandle {
    if keyring_is_persistent() {
        DeviceStoreHandle {
            store: Arc::new(KeyringDeviceKeyStore::new()),
            persistent: true,
        }
    } else {
        DeviceStoreHandle {
            store: Arc::new(MemoryDeviceKeyStore::new()),
            persistent: false,
        }
    }
}

/// Memory store that reports itself as persistent. Only tests should use this;
/// production code must go through [`default_store`].
pub fn memory_store_for_tests() -> DeviceStoreHandle {
    DeviceStoreHandle {
        store: Arc::new(MemoryDeviceKeyStore::new()),
        persistent: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let store = MemoryDeviceKeyStore::new();
        assert!(store.load("p_acc").unwrap().is_none());
        store
            .save("p_acc", "-----BEGIN PRIVATE KEY-----\nx\n")
            .unwrap();
        assert_eq!(
            store.load("p_acc").unwrap().as_deref(),
            Some("-----BEGIN PRIVATE KEY-----\nx\n")
        );
        store.clear("p_acc").unwrap();
        assert!(store.load("p_acc").unwrap().is_none());
    }

    #[test]
    fn memory_store_isolates_identities() {
        let store = MemoryDeviceKeyStore::new();
        store.save("p_a", "key-a").unwrap();
        store.save("m_b", "key-b").unwrap();
        assert_eq!(store.load("p_a").unwrap().as_deref(), Some("key-a"));
        assert_eq!(store.load("m_b").unwrap().as_deref(), Some("key-b"));
        store.clear("p_a").unwrap();
        assert!(store.load("p_a").unwrap().is_none());
        assert_eq!(store.load("m_b").unwrap().as_deref(), Some("key-b"));
    }

    #[test]
    fn memory_store_clear_is_idempotent() {
        let store = MemoryDeviceKeyStore::new();
        store.clear("p_missing").unwrap();
        store.clear("p_missing").unwrap();
    }

    #[test]
    fn persistence_detection_runs() {
        // Must not panic on any platform; the resulting value depends on the
        // compiled-in credential store features.
        let _ = keyring_is_persistent();
    }

    #[test]
    fn service_name_is_stable() {
        // The service/account pair is the contract with the OS credential
        // store. Changing it orphans every enrolled device key.
        assert_eq!(KEYRING_SERVICE, "ir.haio.bypass.device");
    }
}
