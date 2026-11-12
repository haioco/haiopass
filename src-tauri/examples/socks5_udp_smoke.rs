//! Manual smoke test for the SOCKS5 UDP path.
//!
//! Starts the loopback SOCKS5 listener against a Trojan server running on
//! `127.0.0.1:18443` (password `testpass`, certificate for `localhost`
//! trusted by the system store), performs a `UDP ASSOCIATE`, and relays one
//! DNS query to `1.1.1.1:53`.
//!
//! Not part of `cargo test`: it needs a live tunnel server.
//!
//! ```bash
//! cargo run --example socks5_udp_smoke
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use haio_bypass_lib::config::TrojanConfig;
use haio_bypass_lib::proxy::socks5::Socks5Server;
use haio_bypass_lib::proxy::udp;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::RwLock;

const SOCKS_PORT: u16 = 11099;

fn dns_query(txid: u16) -> Vec<u8> {
    let mut query = Vec::new();
    query.extend_from_slice(&txid.to_be_bytes());
    query.extend_from_slice(&[0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for label in ["example", "com"] {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    query
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let shared = Arc::new(RwLock::new(Some(TrojanConfig {
        password: "testpass".into(),
        server: "127.0.0.1".into(),
        port: 18443,
        sni: "localhost".into(),
    })));

    let mut server = Socks5Server::new(SOCKS_PORT, shared);
    server.start().await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut control = TcpStream::connect(("127.0.0.1", SOCKS_PORT)).await?;
    control.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut greeting = [0u8; 2];
    control.read_exact(&mut greeting).await?;
    assert_eq!(greeting, [0x05, 0x00], "SOCKS5 greeting failed");

    control
        .write_all(&[0x05, 0x03, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;
    let mut reply = [0u8; 10];
    control.read_exact(&mut reply).await?;
    assert_eq!(reply[0], 0x05);
    assert_eq!(reply[1], 0x00, "UDP ASSOCIATE refused");
    let relay = SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            reply[4], reply[5], reply[6], reply[7],
        )),
        u16::from_be_bytes([reply[8], reply[9]]),
    );
    println!("UDP relay at {}", relay);

    let udp = UdpSocket::bind("127.0.0.1:0").await?;
    let mut packet = vec![0, 0, 0];
    // SOCKS5 datagram header + the query (address is carried in the header,
    // not in the Trojan frame).
    packet.extend_from_slice(&[0x01, 1, 1, 1, 1, 0x00, 0x35]);
    packet.extend_from_slice(&dns_query(0x4242));
    udp.send_to(&packet, relay).await?;

    let mut buf = vec![0u8; 4096];
    let (length, _) = tokio::time::timeout(Duration::from_secs(10), udp.recv_from(&mut buf)).await??;
    let (_, address, payload) = udp::parse_socks5_udp(&buf[..length])
        .ok_or("relay returned a malformed SOCKS5 datagram")?;
    let txid = u16::from_be_bytes([payload[0], payload[1]]);
    let rcode = payload[3] & 0x0f;
    println!("UDP reply from {:?} payload {} bytes", address, payload.len());
    println!("DNS txid={:04x} rcode={}", txid, rcode);
    assert_eq!(txid, 0x4242);
    assert_eq!(rcode, 0);

    connect_check().await?;

    server.stop().await?;
    println!("OK");
    Ok(())
}

/// CONNECT regression through the same listener: the SOCKS5 TCP path must keep
/// working exactly like the HTTP proxy.
async fn connect_check() -> Result<(), Box<dyn std::error::Error>> {
    let mut control = TcpStream::connect(("127.0.0.1", SOCKS_PORT)).await?;
    control.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut greeting = [0u8; 2];
    control.read_exact(&mut greeting).await?;
    let mut request = vec![0x05, 0x01, 0x00, 0x03, 11];
    request.extend_from_slice(b"example.com");
    request.extend_from_slice(&80u16.to_be_bytes());
    control.write_all(&request).await?;
    let mut reply = [0u8; 10];
    control.read_exact(&mut reply).await?;
    assert_eq!(reply[1], 0x00, "CONNECT refused");
    control
        .write_all(b"HEAD / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
        .await?;
    let mut head = [0u8; 48];
    let n = control.read(&mut head).await?;
    let status = String::from_utf8_lossy(&head[..n]);
    println!("CONNECT {}", status.lines().next().unwrap_or(""));
    assert!(status.starts_with("HTTP/1.1 200"), "unexpected origin reply");
    Ok(())
}
