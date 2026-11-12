//! Loopback SOCKS5 listener with UDP support.
//!
//! The HTTP proxy cannot carry UDP (PAC and system proxy settings are
//! TCP-only), so UDP-capable applications get a SOCKS5 endpoint instead:
//!
//! - `CONNECT` opens a normal Trojan TCP session (same path as the HTTP proxy)
//! - `UDP ASSOCIATE` opens a Trojan UDP relay session and pipes datagrams in
//!   both directions using the framing in `proxy::udp`
//!
//! The listener binds `127.0.0.1` only and requires no authentication, which
//! is safe because it is unreachable from other hosts. One relay session
//! carries every destination of an association; the server supports that and
//! replies carry their real source address.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Notify, RwLock, Semaphore};

use super::udp::{self, MAX_DATAGRAM, SocksAddress};
use crate::config::TrojanConfig;
use crate::error::{HaioError, Result};
use crate::trojan::client::{self, TrojanStream};
use crate::trojan::manager::SharedTrojanConfig;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Idle sessions are closed: UDP has no close handshake, so a vanished client
/// would otherwise keep a tunnel session open forever.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_ASSOCIATIONS: usize = 128;
const BUFFER_SIZE: usize = 32 * 1024;

const CMD_CONNECT: u8 = 0x01;
const CMD_UDP_ASSOCIATE: u8 = 0x03;

pub struct Socks5Server {
    handle: Option<tokio::task::JoinHandle<()>>,
    port: u16,
    trojan_config: SharedTrojanConfig,
    running: Arc<RwLock<bool>>,
    notify: Arc<Notify>,
}

impl Socks5Server {
    pub fn new(port: u16, trojan_config: SharedTrojanConfig) -> Self {
        Self {
            handle: None,
            port,
            trojan_config,
            running: Arc::new(RwLock::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn start(&mut self) -> Result<()> {
        if *self.running.read().await {
            return Ok(());
        }
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| HaioError::Proxy(format!("Failed to bind SOCKS5 {}: {}", addr, e)))?;

        *self.running.write().await = true;
        let trojan_config = self.trojan_config.clone();
        let running = self.running.clone();
        let notify = self.notify.clone();

        let handle = tokio::spawn(async move {
            tracing::info!("SOCKS5 (TCP+UDP) listening on {}", addr);
            let permits = Arc::new(Semaphore::new(MAX_ASSOCIATIONS));
            loop {
                tokio::select! {
                    _ = notify.notified() => break,
                    result = listener.accept() => {
                        let (stream, _) = match result {
                            Ok(pair) => pair,
                            Err(e) => {
                                if !*running.read().await { break; }
                                tracing::error!("SOCKS5 accept error: {}", e);
                                continue;
                            }
                        };
                        let _ = stream.set_nodelay(true);
                        let config = trojan_config.clone();
                        let permits = permits.clone();
                        tokio::spawn(async move {
                            let Ok(_permit) = permits.try_acquire_owned() else {
                                tracing::warn!("SOCKS5 connection refused: association limit reached");
                                return;
                            };
                            if let Err(e) = handle_client(stream, config).await {
                                tracing::debug!("SOCKS5 connection ended: {}", e);
                            }
                        });
                    }
                }
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        *self.running.write().await = false;
        self.notify.notify_one();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        Ok(())
    }
}

async fn handle_client(mut stream: TcpStream, config: SharedTrojanConfig) -> Result<()> {
    // Greeting: VER, NMETHODS, METHODS; only "no authentication" is offered,
    // and the listener is loopback-only.
    let mut head = [0u8; 2];
    tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.read_exact(&mut head))
        .await
        .map_err(|_| HaioError::Proxy("SOCKS5 greeting timed out".into()))??;
    if head[0] != 0x05 {
        return Err(HaioError::Proxy("not a SOCKS5 client".into()));
    }
    let mut methods = vec![0u8; head[1] as usize];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&0x00) {
        let _ = stream.write_all(&[0x05, 0xff]).await;
        return Err(HaioError::Proxy("SOCKS5 client requires authentication".into()));
    }
    stream.write_all(&[0x05, 0x00]).await?;

    // Request: VER, CMD, RSV, ATYP, ADDR, PORT.
    let mut request = [0u8; 4];
    tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.read_exact(&mut request))
        .await
        .map_err(|_| HaioError::Proxy("SOCKS5 request timed out".into()))??;
    if request[0] != 0x05 || request[2] != 0x00 {
        let _ = send_reply(&mut stream, 0x07, none_address()).await;
        return Err(HaioError::Proxy("malformed SOCKS5 request".into()));
    }
    let address = read_address(&mut stream, request[3]).await?;

    match request[1] {
        CMD_CONNECT => {
            let (host, port) = match address {
                SocksAddress::Ip(addr) => (addr.ip().to_string(), addr.port()),
                SocksAddress::Domain(host, port) => (host, port),
            };
            let tunnel = match dial_with_retry(&config, &host, port).await {
                Ok(tunnel) => tunnel,
                Err(e) => {
                    let _ = send_reply(&mut stream, 0x05, none_address()).await;
                    return Err(e);
                }
            };
            send_reply(&mut stream, 0x00, none_address()).await?;
            relay_tcp(stream, tunnel).await
        }
        CMD_UDP_ASSOCIATE => run_udp_association(stream, &config).await,
        _ => {
            let _ = send_reply(&mut stream, 0x07, none_address()).await;
            Err(HaioError::Proxy("unsupported SOCKS5 command".into()))
        }
    }
}

fn none_address() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
}

async fn send_reply(stream: &mut TcpStream, reply: u8, address: SocketAddr) -> Result<()> {
    let mut out = vec![0x05, reply, 0x00];
    match address.ip() {
        IpAddr::V4(v4) => {
            out.push(0x01);
            out.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            out.push(0x04);
            out.extend_from_slice(&v6.octets());
        }
    }
    out.extend_from_slice(&address.port().to_be_bytes());
    stream.write_all(&out).await?;
    Ok(())
}

async fn read_address(stream: &mut TcpStream, atyp: u8) -> Result<SocksAddress> {
    match atyp {
        0x01 => {
            let mut octets = [0u8; 4];
            stream.read_exact(&mut octets).await?;
            let mut port = [0u8; 2];
            stream.read_exact(&mut port).await?;
            Ok(SocksAddress::Ip(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::from(octets)),
                u16::from_be_bytes(port),
            )))
        }
        0x04 => {
            let mut octets = [0u8; 16];
            stream.read_exact(&mut octets).await?;
            let mut port = [0u8; 2];
            stream.read_exact(&mut port).await?;
            Ok(SocksAddress::Ip(SocketAddr::new(
                IpAddr::V6(Ipv6Addr::from(octets)),
                u16::from_be_bytes(port),
            )))
        }
        0x03 => {
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).await?;
            let mut name = vec![0u8; length[0] as usize];
            stream.read_exact(&mut name).await?;
            let mut port = [0u8; 2];
            stream.read_exact(&mut port).await?;
            let host = String::from_utf8(name)
                .map_err(|_| HaioError::Proxy("invalid SOCKS5 hostname".into()))?;
            Ok(SocksAddress::Domain(host, u16::from_be_bytes(port)))
        }
        _ => Err(HaioError::Proxy("unsupported SOCKS5 address type".into())),
    }
}

async fn current_config(trojan_config: &SharedTrojanConfig) -> Result<TrojanConfig> {
    trojan_config.read().await.clone().ok_or_else(|| {
        HaioError::Proxy("Tunnel is not connected — no server credentials are set".into())
    })
}

async fn dial_with_retry(
    trojan_config: &SharedTrojanConfig,
    host: &str,
    port: u16,
) -> Result<TrojanStream> {
    let config = current_config(trojan_config).await?;
    match client::dial(&config, host, port).await {
        Ok(stream) => Ok(stream),
        Err(e) => {
            tracing::warn!("Tunnel dial to {}:{} failed once, retrying: {}", host, port, e);
            tokio::time::sleep(Duration::from_millis(500)).await;
            let config = current_config(trojan_config).await?;
            client::dial(&config, host, port).await
        }
    }
}

async fn relay_tcp(mut client: TcpStream, tunnel: TrojanStream) -> Result<()> {
    let (mut client_read, mut client_write) = client.split();
    let (mut tunnel_read, mut tunnel_write) = tokio::io::split(tunnel);
    let mut to_tunnel = vec![0u8; BUFFER_SIZE];
    let mut to_client = vec![0u8; BUFFER_SIZE];
    loop {
        let step = tokio::time::timeout(IDLE_TIMEOUT, async {
            tokio::select! {
                result = client_read.read(&mut to_tunnel) => {
                    let n = result?;
                    if n == 0 { return Ok(false); }
                    tunnel_write.write_all(&to_tunnel[..n]).await?;
                }
                result = tunnel_read.read(&mut to_client) => {
                    let n = result?;
                    if n == 0 { return Ok(false); }
                    client_write.write_all(&to_client[..n]).await?;
                }
            }
            Ok::<bool, std::io::Error>(true)
        })
        .await;
        match step {
            Ok(Ok(true)) => continue,
            Ok(Ok(false)) => break,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                tracing::debug!("SOCKS5 TCP session idle for {:?}, closing", IDLE_TIMEOUT);
                break;
            }
        }
    }
    Ok(())
}

async fn run_udp_association(mut control: TcpStream, config: &SharedTrojanConfig) -> Result<()> {
    let trojan_config = current_config(config).await?;
    let tunnel = client::dial_udp(&trojan_config).await?;

    let udp = UdpSocket::bind("127.0.0.1:0").await?;
    let local = udp.local_addr()?;
    send_reply(&mut control, 0x00, local).await?;
    tracing::debug!("SOCKS5 UDP relay at {}", local);

    let peer_ip = control.peer_addr().ok().map(|addr| addr.ip());
    let (mut tunnel_read, mut tunnel_write) = tokio::io::split(tunnel);
    let mut client_udp: Option<SocketAddr> = None;
    let mut datagram = vec![0u8; 65535];
    let mut pending: Vec<u8> = Vec::with_capacity(BUFFER_SIZE);
    let mut pending_pos = 0usize;
    let mut reply = Vec::with_capacity(4 + 16 + 2 + MAX_DATAGRAM);
    let mut chunk = vec![0u8; BUFFER_SIZE];

    let idle = tokio::time::sleep(IDLE_TIMEOUT);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            _ = &mut idle => {
                tracing::debug!("SOCKS5 UDP association idle for {:?}, closing", IDLE_TIMEOUT);
                break;
            }
            result = udp.recv_from(&mut datagram) => {
                let (length, from) = result?;
                // The association belongs to the TCP client: only datagrams
                // from that host (and, once known, that port) are relayed.
                if peer_ip.map(|ip| ip != from.ip()).unwrap_or(false) {
                    continue;
                }
                if let Some(expected) = client_udp {
                    if expected.port() != from.port() {
                        continue;
                    }
                }
                client_udp = Some(from);

                let Some((frag, address, payload)) = udp::parse_socks5_udp(&datagram[..length]) else {
                    continue;
                };
                if frag != 0 || payload.is_empty() || payload.len() > MAX_DATAGRAM {
                    continue;
                }
                let destination = match address {
                    SocksAddress::Ip(addr) => addr,
                    SocksAddress::Domain(host, port) => match resolve(&host, port).await {
                        Some(addr) => addr,
                        None => continue,
                    },
                };
                tunnel_write.write_all(&udp::encode_frame(destination, payload)).await?;
            }
            result = tunnel_read.read(&mut chunk) => {
                let length = result?;
                if length == 0 {
                    break;
                }
                pending.extend_from_slice(&chunk[..length]);
                if pending.len() > 4 * MAX_DATAGRAM {
                    return Err(HaioError::Proxy("UDP frame buffer overflow".into()));
                }
                while let Some((frame, consumed)) = udp::decode_frame(&pending[pending_pos..]) {
                    if let Some(client_addr) = client_udp {
                        // Reuse one reply buffer: a datagram-per-allocation
                        // reply path dominated CPU on DNS-heavy workloads.
                        udp::encode_socks5_udp_into(&mut reply, frame.address, &frame.payload);
                        udp.send_to(&reply, client_addr).await?;
                    }
                    pending_pos += consumed;
                }
                if pending_pos > 0 {
                    // Drop consumed frames once per read instead of shifting
                    // the tail for every frame (O(n^2) on bursts).
                    pending.drain(..pending_pos);
                    pending_pos = 0;
                }
            }
        }
        idle.as_mut().reset(tokio::time::Instant::now() + IDLE_TIMEOUT);
    }

    Ok(())
}

async fn resolve(host: &str, port: u16) -> Option<SocketAddr> {
    match tokio::net::lookup_host((host, port)).await {
        Ok(mut addrs) => addrs.next(),
        Err(_) => None,
    }
}
