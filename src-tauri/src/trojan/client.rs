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
//! CRLF
//! SOCKS5.ADDR             ATYP + ADDR + PORT(2, big-endian)
//! CRLF
//! <payload>
//! ```
//!
//! The server replies with a single status byte (`0x00` = success) before the
//! tunnelled payload starts flowing in both directions.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::TrojanConfig;
use crate::error::HaioError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Trojan request command: CONNECT (tunnel a TCP stream).
const CMD_CONNECT: u8 = 0x01;
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
/// - TLS session resumption enabled
///
/// Native cert stores (Windows: system store) provide the roots.
fn build_tls_config() -> crate::error::Result<rustls::ClientConfig> {
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

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    // Shared in-memory session cache: every connection skips full certificate
    // chain validation whenever the server allows resumption. Required for
    // performance parity with trojan-go (`reuse_session`/`session_ticket`).
    config.resumption = rustls::client::Resumption::in_memory_sessions(256);
    Ok(config)
}

/// The client config is built once (loading the system trust store is not
/// free) and shared for the lifetime of the process, which is what makes the
/// session cache effective across connections.
fn shared_tls_config() -> crate::error::Result<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    match CONFIG.get_or_init(|| build_tls_config().map(Arc::new).map_err(|e| e.to_string())) {
        Ok(config) => Ok(config.clone()),
        Err(msg) => Err(HaioError::Trojan(msg.clone())),
    }
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
/// verified) → Trojan request → server status byte. On success the returned
/// stream is a raw pipe to `host:port`.
pub async fn dial(config: &TrojanConfig, host: &str, port: u16) -> crate::error::Result<TrojanStream> {
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

    let connector = tokio_rustls::TlsConnector::from(shared_tls_config()?);
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

    let mut request = Vec::with_capacity(56 + 2 + 1 + 2 + 1 + 256 + 2 + 2);
    request.extend_from_slice(password_hex(&config.password).as_bytes());
    request.extend_from_slice(b"\r\n");
    request.push(CMD_CONNECT);
    request.extend_from_slice(b"\r\n");
    push_address(&mut request, host, port);
    request.extend_from_slice(b"\r\n");

    tls.write_all(&request).await?;
    tls.flush().await?;

    // Trojan reply: one byte, 0x00 = allowed. Read before any payload flows.
    let mut status = [0u8; 1];
    tls.read_exact(&mut status).await?;
    if status[0] != 0x00 {
        return Err(HaioError::Trojan(format!(
            "Tunnel to {}:{} rejected by server (code {})",
            host, port, status[0]
        )));
    }

    tracing::debug!("Trojan tunnel established to {}:{}", host, port);
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
    let Ok(connector) = shared_tls_config().map(tokio_rustls::TlsConnector::from) else {
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
        };
        let mut header = Vec::new();
        header.extend_from_slice(password_hex(&config.password).as_bytes());
        header.extend_from_slice(b"\r\n");
        header.push(CMD_CONNECT);
        header.extend_from_slice(b"\r\n");
        push_address(&mut header, "example.org", 8080);
        header.extend_from_slice(b"\r\n");
        // hex(56) + CRLF + CMD + CRLF + ATYP + len + "example.org"(11) + port(2) + CRLF
        assert_eq!(header.len(), 56 + 2 + 1 + 2 + 1 + 1 + 11 + 2 + 2);
        assert_eq!(&header[56..59], b"\r\n\x01");
        assert_eq!(&header[59..61], b"\r\n");
        assert_eq!(&header[header.len() - 2..], b"\r\n");
    }
}
