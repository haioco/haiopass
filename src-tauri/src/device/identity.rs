//! Public device-identity material, the in-memory TLS material registry, and
//! the identity id convention.
//!
//! `state.json` only ever carries [`DeviceCertificate`] — certificate, CA
//! chain, fingerprint and ids. The matching private key lives exclusively in
//! the OS keyring under `ir.haio.bypass.device`, account = `identity_id`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Utc};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};

use crate::device::DeviceError;

/// Public half of an enrolled device, persisted in `state.json`.
///
/// Every field is safe to show to the UI and safe to write to disk. No key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCertificate {
    /// Keyring account name and the `TrojanConfig::device_identity_id` value.
    pub identity_id: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub member_id: Option<String>,
    pub certificate_pem: String,
    pub ca_chain_pem: String,
    /// 64-hex SHA-256 of the certificate DER, as returned by the server.
    pub fingerprint: String,
    #[serde(default)]
    pub cert_expires_at: Option<String>,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub platform: String,
    /// Unix seconds when this certificate was enrolled on this machine.
    #[serde(default)]
    pub enrolled_at: i64,
}

impl DeviceCertificate {
    /// Parse the leaf chain into rustls types.
    pub fn cert_chain(&self) -> Result<Vec<CertificateDer<'static>>, DeviceError> {
        let chain: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(self.certificate_pem.as_bytes())
                .filter_map(Result::ok)
                .collect();
        if chain.is_empty() {
            return Err(DeviceError::certificate_invalid());
        }
        Ok(chain)
    }

    /// CA chain (may be a single root). Used for display/diagnostics; the
    /// device certificate is already signed by that chain.
    pub fn ca_chain(&self) -> Vec<CertificateDer<'static>> {
        CertificateDer::pem_slice_iter(self.ca_chain_pem.as_bytes())
            .filter_map(Result::ok)
            .collect()
    }

    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Utc::now())
    }

    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        let Some(raw) = self.cert_expires_at.as_deref() else {
            return false;
        };
        match DateTime::parse_from_rfc3339(raw.trim()) {
            Ok(parsed) => parsed.with_timezone(&Utc) <= now,
            Err(_) => false,
        }
    }

    /// First 16 hex characters, safe to show in a list.
    pub fn fingerprint_prefix(&self) -> String {
        self.fingerprint.chars().take(16).collect()
    }
}

/// Identity id convention:
/// - primary credential → `p_<account_id>`
/// - member credential  → `m_<member_id>`
///
/// Ids come from the server as UUIDs; non-alphanumeric characters other than
/// `-`/`_` are dropped so the value is always safe as a keyring account name
/// and cannot be mistaken for a path.
pub fn identity_id_for(account_id: &str, member_id: Option<&str>) -> String {
    match member_id {
        Some(member) => format!("m_{}", sanitize_id(member)),
        None => format!("p_{}", sanitize_id(account_id)),
    }
}

fn sanitize_id(value: &str) -> String {
    let filtered: String = value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .take(64)
        .collect();
    if filtered.is_empty() {
        "unknown".to_string()
    } else {
        filtered
    }
}

/// `windows` / `macos` / `linux` — matches the server-side regex
/// `^[a-z][a-z0-9_-]{1,19}$`.
pub fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Trim a user-supplied device name to the server limit and strip control
/// characters (they break logs and terminal output).
pub fn clean_device_name(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(100)
        .collect();
    cleaned
}

/// Best-effort default name shown before the user edits it.
pub fn default_device_name() -> String {
    for key in ["HAIO_DEVICE_NAME", "COMPUTERNAME", "HOSTNAME"] {
        if let Ok(value) = std::env::var(key) {
            let cleaned = clean_device_name(&value);
            if !cleaned.is_empty() {
                return cleaned;
            }
        }
    }
    "Desktop".to_string()
}

/// Client-auth material for rustls. Debug is redacted on purpose.
pub struct DeviceMaterial {
    pub cert_chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl std::fmt::Debug for DeviceMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceMaterial")
            .field("cert_chain_len", &self.cert_chain.len())
            .field("key", &"<redacted>")
            .finish()
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<DeviceMaterial>>> {
    static MATERIAL: OnceLock<Mutex<HashMap<String, Arc<DeviceMaterial>>>> = OnceLock::new();
    MATERIAL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_registry() -> std::sync::MutexGuard<'static, HashMap<String, Arc<DeviceMaterial>>> {
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Monotonic counter bumped whenever identity material changes. The TLS config
/// cache includes it in its key, so a re-enrollment can never reuse a config
/// built from the previous certificate.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// Parse and install key material for `meta`, replacing any previous material
/// under the same identity id.
pub fn register_material(
    meta: &DeviceCertificate,
    key_pem: &str,
) -> Result<Arc<DeviceMaterial>, DeviceError> {
    if meta.identity_id.trim().is_empty() {
        return Err(DeviceError::identity_missing());
    }
    let cert_chain = meta.cert_chain()?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .map_err(|_| DeviceError::certificate_invalid())?;
    // Fail early (before a network dial) when the key does not match the
    // algorithm rustls can sign with.
    rustls::crypto::ring::sign::any_supported_type(&key)
        .map_err(|_| DeviceError::certificate_invalid())?;

    let material = Arc::new(DeviceMaterial {
        cert_chain,
        key: key.clone_key(),
    });
    lock_registry().insert(meta.identity_id.clone(), material.clone());
    bump_generation();
    Ok(material)
}

/// Current TLS material for an identity, if it was registered this run.
pub fn material(identity_id: &str) -> Option<Arc<DeviceMaterial>> {
    lock_registry().get(identity_id).cloned()
}

/// Drop the material and invalidate every cached TLS config that could have
/// used it.
pub fn unregister_material(identity_id: &str) {
    let mut map = lock_registry();
    if map.remove(identity_id).is_some() {
        bump_generation();
    }
}

/// Remove everything. Used by tests only.
#[cfg(test)]
pub fn clear_all_material() {
    lock_registry().clear();
    bump_generation();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn self_signed_pem() -> (String, String) {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        let cert = params.self_signed(&key).unwrap();
        (cert.pem(), key.serialize_pem())
    }

    #[test]
    fn identity_ids_are_stable_and_safe() {
        assert_eq!(identity_id_for("acc-1", None), "p_acc-1");
        assert_eq!(
            identity_id_for("acc-1", Some("f0e1d2c3-0000-4000-8000-000000000001")),
            "m_f0e1d2c3-0000-4000-8000-000000000001"
        );
        // A hostile value cannot escape the keyring account namespace.
        assert_eq!(identity_id_for("../../etc", None), "p_etc");
        assert_eq!(identity_id_for("", None), "p_unknown");
    }

    #[test]
    fn platform_name_matches_server_regex() {
        let platform = platform_name();
        assert!(platform.chars().next().unwrap().is_ascii_lowercase());
        assert!(platform
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'));
    }

    #[test]
    fn device_name_is_trimmed_and_cleaned() {
        assert_eq!(clean_device_name("  Ali\u{0007}'s PC  "), "Ali's PC");
        let long = "x".repeat(500);
        assert_eq!(clean_device_name(&long).chars().count(), 100);
    }

    #[test]
    fn parses_certificate_pem() {
        let (cert_pem, _) = self_signed_pem();
        let meta = DeviceCertificate {
            identity_id: "p_acc".into(),
            certificate_pem: cert_pem,
            ..Default::default()
        };
        assert_eq!(meta.cert_chain().unwrap().len(), 1);
    }

    #[test]
    fn rejects_garbage_certificate_pem() {
        let meta = DeviceCertificate {
            identity_id: "p_acc".into(),
            certificate_pem: "not a certificate".into(),
            ..Default::default()
        };
        assert_eq!(
            meta.cert_chain().unwrap_err().code,
            "device_certificate_invalid"
        );
    }

    #[test]
    fn registers_and_unregisters_material() {
        clear_all_material();
        let (cert_pem, key_pem) = self_signed_pem();
        let meta = DeviceCertificate {
            identity_id: "p_test".into(),
            certificate_pem: cert_pem,
            ..Default::default()
        };
        let before = generation();
        register_material(&meta, &key_pem).expect("material");
        assert!(material("p_test").is_some());
        assert!(generation() > before, "cache generation must advance");

        unregister_material("p_test");
        assert!(material("p_test").is_none());
    }

    #[test]
    fn rejects_non_key_pem() {
        clear_all_material();
        let (cert_pem, _) = self_signed_pem();
        let meta = DeviceCertificate {
            identity_id: "p_test".into(),
            certificate_pem: cert_pem,
            ..Default::default()
        };
        let err = register_material(&meta, "not a key").unwrap_err();
        assert_eq!(err.code, "device_certificate_invalid");
    }

    #[test]
    fn expiry_is_parsed_as_rfc3339() {
        let meta = DeviceCertificate {
            cert_expires_at: Some("2000-01-01T00:00:00Z".into()),
            ..Default::default()
        };
        assert!(meta.is_expired_at(Utc::now()));
        let future = DeviceCertificate {
            cert_expires_at: Some("2999-01-01T00:00:00Z".into()),
            ..Default::default()
        };
        assert!(!future.is_expired_at(Utc::now()));
        assert!(!DeviceCertificate::default().is_expired_at(Utc::now()));
    }

    #[test]
    fn fingerprint_prefix_is_short() {
        let meta = DeviceCertificate {
            fingerprint: "abcdef0123456789aaaaaaaaaaaaaaaa".into(),
            ..Default::default()
        };
        assert_eq!(meta.fingerprint_prefix(), "abcdef0123456789");
    }
}
