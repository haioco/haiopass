//! HaioGate device identity: EC P-256 keypair, PKCS#10 CSR, OS-keyring key
//! storage and the public certificate metadata.
//!
//! Security contract:
//! - the private key is generated in-process and written only to the OS
//!   keyring (`ir.haio.bypass.device`);
//! - `state.json` receives [`DeviceCertificate`] only (public material);
//! - when no persistent keyring exists, enrollment is refused instead of
//!   falling back to a plaintext or memory-only key.

pub mod csr;
pub mod identity;
pub mod store;

use std::sync::Arc;

use store::{DeviceKeyStore, DeviceStoreHandle};

pub use csr::GeneratedIdentity;
pub use identity::{DeviceCertificate, DeviceMaterial};

/// Typed, redacted device-identity error. Codes are stable and consumed by the
/// frontend; messages are Persian fallbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceError {
    pub code: &'static str,
    pub message: String,
}

impl DeviceError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn keyring_unavailable() -> Self {
        Self::new(
            "device_keyring_unavailable",
            "ذخیره‌سازی امن سیستم در دسترس نیست؛ ثبت دستگاه ممکن نیست. برنامه را با دسترسی به Keychain/Secret Service اجرا کنید.",
        )
    }

    pub fn key_unavailable() -> Self {
        Self::new(
            "device_key_unavailable",
            "کلید امن این دستگاه پیدا نشد. دستگاه را دوباره ثبت کنید.",
        )
    }

    pub fn identity_missing() -> Self {
        Self::new(
            "device_identity_missing",
            "شناسه دستگاه معتبر نیست. دستگاه را دوباره ثبت کنید.",
        )
    }

    pub fn crypto(message: impl Into<String>) -> Self {
        Self::new("device_crypto_failed", message)
    }

    pub fn certificate_invalid() -> Self {
        Self::new(
            "device_certificate_invalid",
            "گواهی دستگاه معتبر نیست. دستگاه را دوباره ثبت کنید.",
        )
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::new("device_storage_failed", message)
    }
}

impl std::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for DeviceError {}

/// Owns the key store and the enroll/forget lifecycle for one process.
pub struct DeviceService {
    store: Arc<dyn DeviceKeyStore>,
    persistent: bool,
}

impl Default for DeviceService {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceService {
    pub fn new() -> Self {
        Self::with_handle(store::default_store())
    }

    pub fn with_handle(handle: DeviceStoreHandle) -> Self {
        if !handle.persistent {
            tracing::info!("device identity keyring is unavailable; enrollment will be refused");
        }
        Self {
            store: handle.store,
            persistent: handle.persistent,
        }
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self::with_handle(store::memory_store_for_tests())
    }

    /// True only when the private key can survive an app restart.
    pub fn is_persistent(&self) -> bool {
        self.persistent
    }

    /// Generate a keypair + CSR and persist the private key under
    /// `identity_id` before the network call. Refuses to continue without a
    /// persistent keyring: a key that disappears on restart would break the
    /// enrolled device silently.
    ///
    /// If a key already exists (a previous attempt failed after the server
    /// consumed the invite, or the response was lost), the same key is reused
    /// and a fresh CSR is built from it. Combined with the deterministic
    /// request id, that lets the server replay the certificate instead of
    /// answering `invite_used`.
    pub fn ensure_csr(&self, identity_id: &str) -> Result<GeneratedIdentity, DeviceError> {
        if identity_id.trim().is_empty() {
            return Err(DeviceError::identity_missing());
        }
        if !self.persistent {
            return Err(DeviceError::keyring_unavailable());
        }

        if let Some(key_pem) = self
            .store
            .load(identity_id)
            .map_err(|_| DeviceError::keyring_unavailable())?
        {
            match csr::csr_from_key_pem(&key_pem) {
                Ok(csr_pem) => return Ok(GeneratedIdentity { key_pem, csr_pem }),
                Err(_) => {
                    // A corrupt/orphan entry must not brick enrollment: drop it
                    // and generate a fresh key below.
                    let _ = self.store.clear(identity_id);
                }
            }
        }

        let generated = csr::generate_identity()?;
        self.store
            .save(identity_id, &generated.key_pem)
            .map_err(|_| DeviceError::keyring_unavailable())?;
        Ok(generated)
    }

    /// Load the private key for `identity_id` and install the certificate as
    /// live TLS client-auth material.
    pub fn commit(&self, meta: &DeviceCertificate) -> Result<Arc<DeviceMaterial>, DeviceError> {
        let key = self
            .store
            .load(&meta.identity_id)
            .map_err(|_| DeviceError::keyring_unavailable())?
            .ok_or_else(DeviceError::key_unavailable)?;
        identity::register_material(meta, &key)
    }

    /// Delete one identity's key and any cached material. Idempotent.
    pub fn forget(&self, identity_id: &str) -> Result<(), DeviceError> {
        identity::unregister_material(identity_id);
        self.store
            .clear(identity_id)
            .map_err(|_| DeviceError::keyring_unavailable())
    }

    /// Whether a key exists for `identity_id`.
    pub fn has_key(&self, identity_id: &str) -> bool {
        matches!(self.store.load(identity_id), Ok(Some(_)))
    }

    /// Test-only: inspect what was written without exposing it to callers.
    #[cfg(test)]
    pub fn stored_key(&self, identity_id: &str) -> Option<String> {
        self.store.load(identity_id).ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::store::{memory_store_for_tests, MemoryDeviceKeyStore};

    #[test]
    fn refuses_to_enroll_without_a_persistent_keyring() {
        let handle = DeviceStoreHandle {
            store: Arc::new(MemoryDeviceKeyStore::new()),
            persistent: false,
        };
        let service = DeviceService::with_handle(handle);
        assert!(!service.is_persistent());
        let err = service.ensure_csr("p_acc").expect_err("must refuse");
        assert_eq!(err.code, "device_keyring_unavailable");
        assert!(!service.has_key("p_acc"), "no key may be written");
    }

    #[test]
    fn ensure_csr_writes_only_the_keyring_entry() {
        let service = DeviceService::for_tests();
        let generated = service.ensure_csr("p_acc").expect("prepare");
        assert!(generated.csr_pem.contains("BEGIN CERTIFICATE REQUEST"));
        let stored = service.stored_key("p_acc").expect("stored key");
        assert_eq!(stored, generated.key_pem);
        // A different identity is untouched.
        assert!(!service.has_key("m_other"));
    }

    #[test]
    fn ensure_csr_reuses_an_existing_key() {
        let service = DeviceService::for_tests();
        let first = service.ensure_csr("p_acc").expect("first");
        let second = service.ensure_csr("p_acc").expect("second");
        // Same private key (same SPKI) so the server can replay a lost
        // enrollment response; the CSR itself is freshly signed.
        assert_eq!(first.key_pem, second.key_pem);
        assert_ne!(first.csr_pem, second.csr_pem);
        assert!(second.csr_pem.contains("BEGIN CERTIFICATE REQUEST"));
    }

    #[test]
    fn ensure_csr_replaces_a_corrupt_key() {
        let service = DeviceService::for_tests();
        service.store.save("p_acc", "not a key").expect("seed");
        let generated = service.ensure_csr("p_acc").expect("regenerated");
        assert!(generated.key_pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert_ne!(service.stored_key("p_acc").as_deref(), Some("not a key"));
    }

    #[test]
    fn abort_removes_the_key() {
        let service = DeviceService::for_tests();
        let _ = service.ensure_csr("m_one").expect("prepare");
        assert!(service.has_key("m_one"));
        service.forget("m_one").expect("forget");
        assert!(!service.has_key("m_one"));
        // Idempotent.
        service.forget("m_one").expect("forget twice");
    }

    #[test]
    fn commit_requires_a_key() {
        let service = DeviceService::for_tests();
        let meta = DeviceCertificate {
            identity_id: "p_acc".into(),
            certificate_pem: "garbage".into(),
            ..Default::default()
        };
        let err = service.commit(&meta).expect_err("missing key");
        assert_eq!(err.code, "device_key_unavailable");
    }

    #[test]
    fn commit_registers_certificate_material() {
        let service = DeviceService::for_tests();
        let generated = service.ensure_csr("p_acc").expect("prepare");

        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        let cert = params.self_signed(&key).unwrap();

        // The key in the store was generated separately; commit must accept
        // any valid certificate/key pair, so overwrite the key with the one
        // that signed this certificate.
        service
            .store
            .save("p_acc", &key.serialize_pem())
            .expect("store key");
        let meta = DeviceCertificate {
            identity_id: "p_acc".into(),
            certificate_pem: cert.pem(),
            ..Default::default()
        };
        service.commit(&meta).expect("commit");
        assert!(identity::material("p_acc").is_some());
        // The old CSR key is no longer relevant, but prepare output is still
        // structurally valid.
        assert!(generated.csr_pem.contains("CERTIFICATE REQUEST"));
        service.forget("p_acc").unwrap();
        assert!(identity::material("p_acc").is_none());
    }

    #[test]
    fn empty_identity_is_rejected() {
        let service = DeviceService::for_tests();
        assert_eq!(
            service.ensure_csr("   ").unwrap_err().code,
            "device_identity_missing"
        );
    }

    #[test]
    fn test_handle_is_persistent() {
        let handle = memory_store_for_tests();
        assert!(handle.persistent);
    }
}
