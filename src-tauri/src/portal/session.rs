//! In-memory token state + single-flight refresh support.
//!
//! Tokens never leave this module except through an `Authorization` header
//! built inside the portal client; no command returns them.

use super::keyring_store::{StoreHandle, TokenStore};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
}

impl Tokens {
    pub fn is_valid(&self) -> bool {
        !self.access.is_empty() && !self.refresh.is_empty()
    }
}

pub struct SessionManager {
    tokens: Mutex<Option<Tokens>>,
    /// Serializes `load()` so the keyring is read once.
    load_lock: Mutex<()>,
    /// Serializes refresh: two concurrent 401s must not rotate the refresh
    /// token twice and invalidate each other.
    refresh_lock: Mutex<()>,
    store: Arc<dyn TokenStore>,
    persistent: AtomicBool,
    loaded: AtomicBool,
}

impl SessionManager {
    pub fn new(handle: StoreHandle) -> Self {
        Self {
            tokens: Mutex::new(None),
            load_lock: Mutex::new(()),
            refresh_lock: Mutex::new(()),
            store: handle.store,
            persistent: AtomicBool::new(handle.persistent),
            loaded: AtomicBool::new(false),
        }
    }

    /// True when the session survives an app restart.
    pub fn is_persistent(&self) -> bool {
        self.persistent.load(Ordering::SeqCst)
    }

    fn mark_not_persistent(&self) {
        self.persistent.store(false, Ordering::SeqCst);
    }

    /// Read the keyring once into memory. A corrupt entry is deleted.
    pub async fn load(&self) -> Option<Tokens> {
        let _guard = self.load_lock.lock().await;
        if self.loaded.load(Ordering::SeqCst) {
            return self.tokens.lock().await.clone();
        }

        let store = self.store.clone();
        let read = tokio::task::spawn_blocking(move || store.load()).await;
        let loaded = match read {
            Ok(Ok(value)) => value,
            Ok(Err(code)) => {
                tracing::warn!("portal keyring read failed ({:?}); using memory session", code);
                self.mark_not_persistent();
                None
            }
            Err(_) => {
                self.mark_not_persistent();
                None
            }
        };

        let parsed = match loaded {
            Some(raw) => match serde_json::from_str::<Tokens>(&raw) {
                Ok(tokens) if tokens.is_valid() => Some(tokens),
                _ => {
                    tracing::warn!("portal keyring entry was corrupt; clearing it");
                    let store = self.store.clone();
                    let _ = tokio::task::spawn_blocking(move || store.clear()).await;
                    None
                }
            },
            None => None,
        };

        if let Some(tokens) = &parsed {
            *self.tokens.lock().await = Some(tokens.clone());
        }
        self.loaded.store(true, Ordering::SeqCst);
        parsed
    }

    pub async fn current(&self) -> Option<Tokens> {
        self.tokens.lock().await.clone()
    }

    pub async fn access_token(&self) -> Option<String> {
        self.tokens.lock().await.as_ref().map(|t| t.access.clone())
    }

    pub async fn refresh_token(&self) -> Option<String> {
        self.tokens.lock().await.as_ref().map(|t| t.refresh.clone())
    }

    pub async fn has_session(&self) -> bool {
        self.tokens.lock().await.is_some()
    }

    /// Persist a new token pair. Memory is updated first so a keyring failure
    /// never loses the working session for this run.
    pub async fn save(&self, tokens: Tokens) {
        *self.tokens.lock().await = Some(tokens.clone());
        self.loaded.store(true, Ordering::SeqCst);

        if !self.is_persistent() {
            return;
        }

        let value = match serde_json::to_string(&tokens) {
            Ok(value) => value,
            Err(_) => {
                self.mark_not_persistent();
                return;
            }
        };
        let store = self.store.clone();
        let written = tokio::task::spawn_blocking(move || store.save(&value)).await;
        match written {
            Ok(Ok(())) => {}
            Ok(Err(code)) => {
                // Log a short code only — the OS error may contain the secret
                // path or even the value in some backends.
                tracing::warn!("portal keyring write failed ({:?}); memory-only session", code);
                self.mark_not_persistent();
            }
            Err(_) => {
                self.mark_not_persistent();
            }
        }
    }

    /// Drop the session from memory and from the keyring.
    pub async fn clear(&self) {
        *self.tokens.lock().await = None;
        self.loaded.store(true, Ordering::SeqCst);

        let store = self.store.clone();
        let cleared = tokio::task::spawn_blocking(move || store.clear()).await;
        if let Ok(Err(code)) = cleared {
            tracing::warn!("portal keyring delete failed ({:?})", code);
        }
    }

    /// Take the refresh lock. Held across the refresh round-trip.
    pub async fn lock_refresh(&self) -> MutexGuard<'_, ()> {
        self.refresh_lock.lock().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::keyring_store::{memory_store, MemoryTokenStore, StoreError};

    fn tokens(access: &str, refresh: &str) -> Tokens {
        Tokens {
            access: access.into(),
            refresh: refresh.into(),
        }
    }

    struct FailingStore;

    impl TokenStore for FailingStore {
        fn load(&self) -> Result<Option<String>, StoreError> {
            Err(StoreError("boom"))
        }
        fn save(&self, _value: &str) -> Result<(), StoreError> {
            Err(StoreError("boom"))
        }
        fn clear(&self) -> Result<(), StoreError> {
            Err(StoreError("boom"))
        }
    }

    #[tokio::test]
    async fn memory_store_saves_and_loads() {
        let manager = SessionManager::new(memory_store());
        assert!(manager.load().await.is_none());
        manager.save(tokens("a", "r")).await;
        assert_eq!(manager.access_token().await.as_deref(), Some("a"));
        assert_eq!(manager.refresh_token().await.as_deref(), Some("r"));
    }

    #[tokio::test]
    async fn memory_session_is_not_persistent() {
        let manager = SessionManager::new(memory_store());
        assert!(!manager.is_persistent());
    }

    #[tokio::test]
    async fn clear_drops_tokens() {
        let manager = SessionManager::new(memory_store());
        manager.save(tokens("a", "r")).await;
        manager.clear().await;
        assert!(manager.current().await.is_none());
    }

    #[tokio::test]
    async fn keyring_failure_falls_back_to_memory() {
        let handle = StoreHandle {
            store: Arc::new(FailingStore),
            persistent: true,
        };
        let manager = SessionManager::new(handle);
        manager.save(tokens("a", "r")).await;
        assert_eq!(manager.access_token().await.as_deref(), Some("a"));
        assert!(!manager.is_persistent(), "a failed keyring write must demote the session");
    }

    #[tokio::test]
    async fn corrupt_entry_is_dropped() {
        let store = Arc::new(MemoryTokenStore::new());
        store.save("not-json").unwrap();
        let handle = StoreHandle {
            store: store.clone(),
            persistent: false,
        };
        let manager = SessionManager::new(handle);
        assert!(manager.load().await.is_none());
        assert!(store.load().unwrap().is_none(), "corrupt entry should be deleted");
    }
}
