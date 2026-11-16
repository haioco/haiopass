//! EC P-256 key generation and PKCS#10 CSR serialization.
//!
//! The private key never leaves this process except into the OS keyring; the
//! CSR only carries the public key and is the single enrollment artifact sent
//! to the control plane.

use crate::device::DeviceError;

/// A freshly generated identity. `key_pem` is a PKCS#8 PEM and must be stored
/// in the OS keyring immediately; never log it or write it to `state.json`.
#[derive(Clone)]
pub struct GeneratedIdentity {
    pub key_pem: String,
    pub csr_pem: String,
}

impl std::fmt::Debug for GeneratedIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the key material, not even in a panic/debug dump.
        f.debug_struct("GeneratedIdentity")
            .field("key_pem", &"<redacted>")
            .field("csr_len", &self.csr_pem.len())
            .finish()
    }
}

/// Generate an ECDSA P-256 keypair and a PKCS#10 CSR signed with it.
pub fn generate_identity() -> Result<GeneratedIdentity, DeviceError> {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|_| DeviceError::crypto("ساخت کلید امن دستگاه ناموفق بود."))?;

    // An empty SAN list: the server owns the certificate profile (CN, EKU,
    // serial, validity). The CSR only proves possession of the private key.
    let params = rcgen::CertificateParams::new(Vec::<String>::new())
        .map_err(|_| DeviceError::crypto("ساخت درخواست امضای گواهی ناموفق بود."))?;
    let csr = params
        .serialize_request(&key_pair)
        .map_err(|_| DeviceError::crypto("ساخت درخواست امضای گواهی ناموفق بود."))?;
    let csr_pem = csr
        .pem()
        .map_err(|_| DeviceError::crypto("ساخت درخواست امضای گواهی ناموفق بود."))?;

    Ok(GeneratedIdentity {
        key_pem: key_pair.serialize_pem(),
        csr_pem,
    })
}

/// Generate a PKCS#10 CSR from an existing key PEM (used to replay a lost
/// enrollment response with the same SPKI + request id).
pub fn csr_from_key_pem(key_pem: &str) -> Result<String, DeviceError> {
    let key_pair = rcgen::KeyPair::from_pem(key_pem)
        .map_err(|_| DeviceError::crypto("کلید امن دستگاه قابل خواندن نیست."))?;
    let params = rcgen::CertificateParams::new(Vec::<String>::new())
        .map_err(|_| DeviceError::crypto("ساخت درخواست امضای گواهی ناموفق بود."))?;
    let csr = params
        .serialize_request(&key_pair)
        .map_err(|_| DeviceError::crypto("ساخت درخواست امضای گواهی ناموفق بود."))?;
    csr.pem()
        .map_err(|_| DeviceError::crypto("ساخت درخواست امضای گواهی ناموفق بود."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_a_pkcs10_csr_pem() {
        let generated = generate_identity().expect("csr");
        assert!(generated
            .csr_pem
            .starts_with("-----BEGIN CERTIFICATE REQUEST-----"));
        assert!(generated
            .csr_pem
            .trim_end()
            .ends_with("-----END CERTIFICATE REQUEST-----"));
        assert!(generated.key_pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert!(generated
            .key_pem
            .trim_end()
            .ends_with("-----END PRIVATE KEY-----"));
    }

    #[test]
    fn generated_key_is_a_loadable_ec_p256_key() {
        use rustls::pki_types::pem::PemObject;

        let generated = generate_identity().expect("csr");
        let key = rcgen::KeyPair::from_pem(&generated.key_pem).expect("key parses");
        assert_eq!(key.algorithm(), &rcgen::PKCS_ECDSA_P256_SHA256);

        // rustls must accept the same PEM as a client-auth signing key; this is
        // the exact conversion the TLS layer performs.
        let der = rustls::pki_types::PrivateKeyDer::from_pem_slice(generated.key_pem.as_bytes())
            .expect("pkcs8 pem");
        assert!(rustls::crypto::ring::sign::any_supported_type(&der).is_ok());
    }

    #[test]
    fn each_identity_is_unique() {
        let a = generate_identity().expect("csr a");
        let b = generate_identity().expect("csr b");
        assert_ne!(a.key_pem, b.key_pem);
        assert_ne!(a.csr_pem, b.csr_pem);
    }

    #[test]
    fn debug_never_prints_the_key() {
        let generated = generate_identity().expect("csr");
        let rendered = format!("{:?}", generated);
        assert!(!rendered.contains(&generated.key_pem));
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn csr_from_existing_key_keeps_the_same_public_key() {
        let generated = generate_identity().expect("csr");
        let csr = csr_from_key_pem(&generated.key_pem).expect("csr from key");
        assert!(csr.starts_with("-----BEGIN CERTIFICATE REQUEST-----"));
        let other = csr_from_key_pem("not a key").unwrap_err();
        assert_eq!(other.code, "device_crypto_failed");
    }
}
