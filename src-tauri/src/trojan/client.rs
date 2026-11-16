//! In-process Trojan protocol client.
//!
//! Replaces the bundled trojan-go child process: no binary is written to disk,
//! no process is spawned, no SOCKS5 hop exists. One TLS connection carries
//! exactly one tunnelled session (the protocol cannot be multiplexed), so a
//! new `dial` per outbound connection is the intended usage. TLS sessions are
//! resumed across connections via a shared session cache to keep handshake
//! cost down (see AV-HARDENING-PLAN.md §5.3).
//!
//! Wire format (client → server, immediately after the TLS handshake):
//!
//! ```text
//! hex(SHA224(password))   56 bytes, lowercase ASCII hex
//! CRLF
//! CMD                     0x01 = CONNECT
//! SOCKS5.ADDR             ATYP + ADDR + PORT(2, big-endian), no separator
//! CRLF
//! <payload>
//! ```
//!
//! Note there is no CRLF between CMD and the address — an extra one there
//! makes trojan-gfw servers reject the request as "not trojan request" and
//! fall back to their web server (verified against production).
//!
//! The server sends **no reply** — the tunnelled payload starts flowing in
//! both directions immediately after the request (verified against the
//! production trojan-gfw server). Reading a "status byte" here would
//! consume the first payload byte of every stream.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::TrojanConfig;
use crate::error::HaioError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Trojan request command: CONNECT (tunnel a TCP stream).
const CMD_CONNECT: u8 = 0x01;
/// Trojan request command: UDP_ASSOCIATE (relay datagrams in framed packets).
const CMD_UDP_ASSOCIATE: u8 = 0x03;
/// SOCKS5 address type: 4-byte IPv4.
const ATYP_IPV4: u8 = 0x01;
/// SOCKS5 address type: 1-byte length + domain name.
const ATYP_DOMAIN: u8 = 0x03;
/// SOCKS5 address type: 16-byte IPv6.
const ATYP_IPV6: u8 = 0x04;

/// The tunnelled stream: a TLS stream boxed for use with `copy_bidirectional`.
pub trait TrojanStreamImpl: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> TrojanStreamImpl for T {}
pub type TrojanStream = Box<dyn TrojanStreamImpl>;

/// Build the rustls client config, matching the contract trojan-go was
/// configured with (`trojan/config_writer.rs`, now deleted):
/// - certificate verification on, against `sni` (not `remote_addr`)
/// - ALPN `["h2", "http/1.1"]`
/// - TLS session resumption enabled for password-only credentials
/// - client certificate + key for device-lock credentials
///
/// Native cert stores (Windows: system store) provide the roots.
fn build_tls_config(identity: Option<&str>) -> crate::error::Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    for cert in loaded.certs {
        if let Err(e) = roots.add(cert) {
            tracing::warn!("Skipping unparseable root certificate: {}", e);
        }
    }
    if roots.is_empty() {
        for err in &loaded.errors {
            tracing::error!("Native certificate store error: {}", err);
        }
        return Err(HaioError::Trojan(
            "No trusted root certificates available on this system".into(),
        ));
    }

    let builder = rustls::ClientConfig::builder().with_root_certificates(roots);
    let mut config = match identity {
        Some(identity_id) => {
            let material = crate::device::identity::material(identity_id).ok_or_else(|| {
                HaioError::Trojan(
                    "The enrolled device certificate is not loaded; re-enroll this device".into(),
                )
            })?;
            builder
                .with_client_auth_cert(
                    material.cert_chain.clone(),
                    material.key.clone_key(),
                )
                .map_err(|e| HaioError::Trojan(format!("Invalid device certificate: {}", e)))?
        }
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    // Session resumption is safe and useful for password-only credentials.
    // With a client certificate it is a possession-check bypass: a resumed
    // TLS 1.3 session does not re-authenticate the certificate (plan §22.1),
    // so device identities always perform a full handshake.
    config.resumption = if resumption_enabled(identity) {
        // Shared in-memory session cache: every connection skips full
        // certificate chain validation whenever the server allows resumption.
        rustls::client::Resumption::in_memory_sessions(256)
    } else {
        rustls::client::Resumption::disabled()
    };
    Ok(config)
}

fn resumption_enabled(identity: Option<&str>) -> bool {
    identity.is_none()
}

/// TLS config cache key. The device-identity generation is part of the key, so
/// a re-enrollment can never reuse a config built with the previous
/// certificate. Password-only entries (`identity == None`) share one config per
/// SNI.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TlsCacheKey {
    sni: String,
    identity: Option<String>,
    generation: u64,
}

fn tls_cache_key(sni: &str, identity: Option<&str>, generation: u64) -> TlsCacheKey {
    TlsCacheKey {
        sni: sni.to_string(),
        identity: identity.map(str::to_string),
        generation,
    }
}

fn tls_cache() -> &'static Mutex<HashMap<TlsCacheKey, Arc<rustls::ClientConfig>>> {
    static CACHE: OnceLock<Mutex<HashMap<TlsCacheKey, Arc<rustls::ClientConfig>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_cache() -> std::sync::MutexGuard<'static, HashMap<TlsCacheKey, Arc<rustls::ClientConfig>>>
{
    tls_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The client config is cached per `(sni, device identity, generation)`: the
/// system trust store load is not free, and this is what keeps the session
/// cache effective across connections.
fn tls_config(
    sni: &str,
    identity: Option<&str>,
) -> crate::error::Result<Arc<rustls::ClientConfig>> {
    let generation = crate::device::identity::generation();
    let key = tls_cache_key(sni, identity, generation);

    {
        let mut cache = lock_cache();
        // Entries from an older identity generation can never be valid again.
        if cache.keys().any(|entry| entry.generation != generation) {
            cache.clear();
        }
        if let Some(config) = cache.get(&key) {
            return Ok(config.clone());
        }
    }

    let config = Arc::new(build_tls_config(identity)?);
    lock_cache().insert(key, config.clone());
    Ok(config)
}

/// Lowercase hex of SHA224(password) — the Trojan auth token.
fn password_hex(password: &str) -> String {
    use sha2::{Digest, Sha224};
    let digest = Sha224::digest(password.as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Append the SOCKS5 address encoding of `host:port` (ATYP + ADDR + PORT).
fn push_address(out: &mut Vec<u8>, host: &str, port: u16) {
    if let Ok(v4) = host.parse::<std::net::Ipv4Addr>() {
        out.push(ATYP_IPV4);
        out.extend_from_slice(&v4.octets());
    } else if let Ok(v6) = host.parse::<std::net::Ipv6Addr>() {
        out.push(ATYP_IPV6);
        out.extend_from_slice(&v6.octets());
    } else {
        out.push(ATYP_DOMAIN);
        out.push(host.len() as u8);
        out.extend_from_slice(host.as_bytes());
    }
    out.extend_from_slice(&port.to_be_bytes());
}

/// Open a tunnelled connection to `host:port` through the Trojan server.
///
/// Performs: TCP connect to the server → TLS handshake (SNI = `config.sni`,
/// verified) → Trojan request. On success the returned stream is a raw pipe to
/// `host:port`.
pub async fn dial(config: &TrojanConfig, host: &str, port: u16) -> crate::error::Result<TrojanStream> {
    open(config, CMD_CONNECT, host, port).await
}

/// Open a UDP relay session (`UDP_ASSOCIATE`).
///
/// The returned stream is not a pipe to a host: each direction carries
/// length-prefixed, address-tagged datagrams (see `proxy::udp`). The server
/// ignores the associate address, so `0.0.0.0:0` is used and the first frame
/// names the real destination.
pub async fn dial_udp(config: &TrojanConfig) -> crate::error::Result<TrojanStream> {
    open(config, CMD_UDP_ASSOCIATE, "0.0.0.0", 0).await
}

async fn open(
    config: &TrojanConfig,
    command: u8,
    host: &str,
    port: u16,
) -> crate::error::Result<TrojanStream> {
    let addr = format!("{}:{}", config.server, config.port);
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| {
            HaioError::Trojan(format!(
                "Connect to {} timed out after {:?}",
                addr, CONNECT_TIMEOUT
            ))
        })?
        .map_err(|e| HaioError::Trojan(format!("Connect to {} failed: {}", addr, e)))?;
    tcp.set_nodelay(true)?;

    // SNI is distinct from the dial address: it may name a CDN front. The
    // certificate is validated against this name — that is the whole point
    // of the `sni` field.
    let sni = if config.sni.is_empty() {
        &config.server
    } else {
        &config.sni
    };
    let server_name = rustls::pki_types::ServerName::try_from(sni.to_string())
        .map_err(|e| HaioError::Trojan(format!("Invalid SNI '{}': {}", sni, e)))?;

    let connector = tokio_rustls::TlsConnector::from(tls_config(
        sni,
        config.device_identity_id.as_deref(),
    )?);
    let mut tls = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        connector.connect(server_name, tcp),
    )
    .await
    .map_err(|_| {
        HaioError::Trojan(format!(
            "TLS handshake with {} timed out after {:?}",
            addr, HANDSHAKE_TIMEOUT
        ))
    })?
    .map_err(|e| HaioError::Trojan(format!("TLS handshake with {} failed: {}", addr, e)))?;

    let mut request = Vec::with_capacity(56 + 2 + 1 + 1 + 1 + 256 + 2 + 2);
    request.extend_from_slice(password_hex(&config.password).as_bytes());
    request.extend_from_slice(b"\r\n");
    request.push(command);
    push_address(&mut request, host, port);
    request.extend_from_slice(b"\r\n");

    tls.write_all(&request).await?;
    tls.flush().await?;

    // No server reply: the stream is a raw pipe from here. Do not read
    // anything — the next byte belongs to the tunnelled session.

    tracing::debug!("Trojan tunnel established to {}:{} (cmd {})", host, port, command);
    Ok(Box::new(tls))
}

/// Cheap liveness probe for the health monitor: TCP connect + TLS handshake
/// against the configured server, then drop the connection. Validates
/// reachability and certificate validity without opening a tunnelled session.
pub async fn probe(config: &TrojanConfig) -> bool {
    let addr = format!("{}:{}", config.server, config.port);
    let tcp = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr)).await {
        Ok(Ok(tcp)) => tcp,
        _ => return false,
    };
    let sni = if config.sni.is_empty() {
        &config.server
    } else {
        &config.sni
    };
    let Ok(server_name) = rustls::pki_types::ServerName::try_from(sni.to_string()) else {
        return false;
    };
    let Ok(connector) = tls_config(sni, config.device_identity_id.as_deref())
        .map(tokio_rustls::TlsConnector::from)
    else {
        return false;
    };
    tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(server_name, tcp))
        .await
        .is_ok_and(|r| r.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hex_is_56_lowercase_hex_chars() {
        let hex = password_hex("test");
        assert_eq!(hex.len(), 56);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn known_sha224_vector() {
        // SHA224("abc") = 23097d223405d8228642a477bda255b32a7ce3d249090bb2...
        assert_eq!(&password_hex("abc")[..16], "23097d223405d822");
    }

    #[test]
    fn address_encoding_domain() {
        let mut out = Vec::new();
        push_address(&mut out, "example.com", 443);
        assert_eq!(out, [0x03, 11, b'e', b'x', b'a', b'm', b'p', b'l', b'e', b'.', b'c', b'o', b'm', 0x01, 0xbb]);
    }

    #[test]
    fn address_encoding_ipv4() {
        let mut out = Vec::new();
        push_address(&mut out, "1.2.3.4", 80);
        assert_eq!(out, [0x01, 1, 2, 3, 4, 0x00, 0x50]);
    }

    #[test]
    fn address_encoding_ipv6() {
        let mut out = Vec::new();
        push_address(&mut out, "::1", 443);
        let mut expect = vec![ATYP_IPV6];
        expect.extend_from_slice(&[0u8; 15]);
        expect.push(1);
        expect.extend_from_slice(&443u16.to_be_bytes());
        assert_eq!(out, expect);
    }

    #[test]
    fn request_header_shape() {
        let config = TrojanConfig {
            password: "pw".into(),
            server: "example.com".into(),
            port: 443,
            sni: "example.com".into(),
            device_identity_id: None,
        };
        let mut header = Vec::new();
        header.extend_from_slice(password_hex(&config.password).as_bytes());
        header.extend_from_slice(b"\r\n");
        header.push(CMD_CONNECT);
        push_address(&mut header, "example.org", 8080);
        header.extend_from_slice(b"\r\n");
        // hex(56) + CRLF + CMD + ATYP + len + "example.org"(11) + port(2) + CRLF
        assert_eq!(header.len(), 56 + 2 + 1 + 1 + 1 + 11 + 2 + 2);
        assert_eq!(&header[56..58], b"\r\n");
        assert_eq!(header[58], CMD_CONNECT);
        assert_eq!(&header[59..61], &[ATYP_DOMAIN, 11]);
        assert_eq!(&header[header.len() - 2..], b"\r\n");
    }

    #[test]
    fn tls_cache_key_separates_sni_identity_and_generation() {
        let legacy = tls_cache_key("tahrim.haiocloud.com", None, 7);
        let legacy_same = tls_cache_key("tahrim.haiocloud.com", None, 7);
        assert_eq!(legacy, legacy_same);

        let other_sni = tls_cache_key("cdn.example.com", None, 7);
        assert_ne!(legacy, other_sni);

        let device = tls_cache_key("tahrim.haiocloud.com", Some("p_acc"), 7);
        assert_ne!(legacy, device);
        let other_device = tls_cache_key("tahrim.haiocloud.com", Some("m_mem"), 7);
        assert_ne!(device, other_device);

        // A re-enrollment bumps the generation: the old certificate config
        // must never be reused.
        let renewed = tls_cache_key("tahrim.haiocloud.com", Some("p_acc"), 8);
        assert_ne!(device, renewed);
    }

    #[test]
    fn device_identities_disable_session_resumption() {
        // A resumed TLS 1.3 session skips the client-certificate possession
        // check, so resumption is only allowed for password-only configs.
        assert!(resumption_enabled(None));
        assert!(!resumption_enabled(Some("p_acc")));
    }
}
