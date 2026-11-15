//! Non-sensitive pending-purchase recovery file (`portal_pending.json`).
//!
//! Purpose: if the app is killed during Zibal payment, the next launch can
//! reconcile the invoice. The file therefore must never contain tokens, the
//! payment URL, mobile numbers, national codes, passwords or trojan URLs.

use super::error::PortalError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "portal_pending.json";
pub const MAX_AGE_SECS: i64 = 24 * 60 * 60;

pub mod stage {
    pub const CREATED: &str = "created";
    pub const BROWSER_OPENED: &str = "browser_opened";
    pub const PAID: &str = "paid";
    pub const PROVISIONING: &str = "provisioning";
    pub const DONE: &str = "done";
    pub const FAILED: &str = "failed";
}

pub mod mode {
    pub const BUY: &str = "buy";
    pub const RENEW: &str = "renew";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingPurchase {
    pub mode: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub package_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub invoice_id: Option<String>,
    #[serde(default)]
    pub payment_id: Option<String>,
    pub created_at: i64,
    pub stage: String,
}

impl PendingPurchase {
    pub fn new(mode: &str) -> Self {
        Self {
            mode: mode.to_string(),
            account_id: None,
            package_id: None,
            plan_id: None,
            invoice_id: None,
            payment_id: None,
            created_at: now_secs(),
            stage: stage::CREATED.to_string(),
        }
    }

    pub fn with_invoice(mut self, invoice_id: Option<String>, payment_id: Option<String>) -> Self {
        self.invoice_id = invoice_id;
        self.payment_id = payment_id;
        self
    }

    pub fn is_expired(&self, now: i64) -> bool {
        now.saturating_sub(self.created_at) > MAX_AGE_SECS
    }

    pub fn short_invoice(&self) -> String {
        self.invoice_id
            .as_deref()
            .map(|id| id.chars().take(8).collect::<String>())
            .unwrap_or_default()
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct PendingStore {
    path: PathBuf,
}

impl Default for PendingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingStore {
    /// Default location: `~/.haiobypass/portal_pending.json`, beside `state.json`.
    pub fn new() -> Self {
        Self {
            path: crate::config::Store::config_dir().join(FILE_NAME),
        }
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load_blocking(&self) -> Option<PendingPurchase> {
        if !self.path.exists() {
            return None;
        }
        let data = match fs::read_to_string(&self.path) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!("portal pending read failed: {}", e.kind());
                return None;
            }
        };
        match serde_json::from_str::<PendingPurchase>(&data) {
            Ok(pending) => Some(pending),
            Err(_) => {
                // Corrupt file: log a code-free warning and delete it. Never
                // panic or block startup.
                tracing::warn!("portal pending file was corrupt; deleting it");
                let _ = fs::remove_file(&self.path);
                None
            }
        }
    }

    fn save_blocking(&self, pending: &PendingPurchase) -> Result<(), PortalError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)
                .map_err(|_| PortalError::storage("ذخیره وضعیت پرداخت ناموفق بود."))?;
        }
        let data = serde_json::to_string_pretty(pending)
            .map_err(|_| PortalError::storage("ذخیره وضعیت پرداخت ناموفق بود."))?;

        let tmp = self.path.with_extension("json.tmp");
        {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&tmp)
                .map_err(|_| PortalError::storage("ذخیره وضعیت پرداخت ناموفق بود."))?;
            file.write_all(data.as_bytes())
                .map_err(|_| PortalError::storage("ذخیره وضعیت پرداخت ناموفق بود."))?;
            let _ = file.sync_all();
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }

        if fs::rename(&tmp, &self.path).is_err() {
            // Windows cannot always rename over an existing file.
            let _ = fs::remove_file(&self.path);
            fs::rename(&tmp, &self.path)
                .map_err(|_| PortalError::storage("ذخیره وضعیت پرداخت ناموفق بود."))?;
        }
        Ok(())
    }

    fn clear_blocking(&self) -> Result<(), PortalError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(PortalError::storage("حذف وضعیت پرداخت ناموفق بود.")),
        }
    }

    /// Load and drop an expired (> 24 h) record.
    pub async fn load(&self) -> Option<PendingPurchase> {
        let store = self.clone();
        let pending = tokio::task::spawn_blocking(move || store.load_blocking())
            .await
            .ok()
            .flatten()?;
        if pending.is_expired(now_secs()) {
            tracing::info!("portal pending purchase expired; clearing");
            let store = self.clone();
            let _ = tokio::task::spawn_blocking(move || store.clear_blocking()).await;
            return None;
        }
        Some(pending)
    }

    pub async fn save(&self, pending: &PendingPurchase) -> Result<(), PortalError> {
        let store = self.clone();
        let pending = pending.clone();
        tokio::task::spawn_blocking(move || store.save_blocking(&pending))
            .await
            .map_err(|_| PortalError::storage("ذخیره وضعیت پرداخت ناموفق بود."))?
    }

    pub async fn clear(&self) -> Result<(), PortalError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.clear_blocking())
            .await
            .map_err(|_| PortalError::storage("حذف وضعیت پرداخت ناموفق بود."))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> PendingStore {
        let dir = std::env::temp_dir().join(format!(
            "haiobypass-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        PendingStore::at(dir.join(FILE_NAME))
    }

    #[tokio::test]
    async fn round_trip() {
        let store = temp_store("pending-round-trip");
        let _ = store.clear().await;
        let mut pending = PendingPurchase::new(mode::BUY);
        pending.package_id = Some("pkg".into());
        pending.invoice_id = Some("0123456789abcdef".into());
        store.save(&pending).await.unwrap();
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.mode, "buy");
        assert_eq!(loaded.short_invoice(), "01234567");
        store.clear().await.unwrap();
        assert!(store.load().await.is_none());
    }

    #[tokio::test]
    async fn corrupt_file_is_deleted() {
        let store = temp_store("pending-corrupt");
        if let Some(dir) = store.path().parent() {
            let _ = fs::create_dir_all(dir);
        }
        fs::write(store.path(), b"{not json").unwrap();
        assert!(store.load().await.is_none());
        assert!(!store.path().exists());
    }

    #[tokio::test]
    async fn expired_pending_is_cleared() {
        let store = temp_store("pending-expired");
        let mut pending = PendingPurchase::new(mode::RENEW);
        pending.created_at = now_secs() - MAX_AGE_SECS - 60;
        store.save(&pending).await.unwrap();
        assert!(store.load().await.is_none());
        assert!(!store.path().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let store = temp_store("pending-perms");
        let _ = store.clear().await;
        store.save(&PendingPurchase::new(mode::BUY)).await.unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = store.clear().await;
    }
}
