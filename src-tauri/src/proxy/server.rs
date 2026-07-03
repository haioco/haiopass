use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, Notify};

use super::router::DomainRouter;
use super::socks;
use super::pac;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub struct ProxyServer {
    handle: Option<tokio::task::JoinHandle<()>>,
    port: u16,
    router: Arc<DomainRouter>,
    running: Arc<RwLock<bool>>,
    notify: Arc<Notify>,
}

impl ProxyServer {
    pub fn new(port: u16, router: DomainRouter) -> Self {
        Self {
            handle: None,
            port,
            router: Arc::new(router),
            running: Arc::new(RwLock::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub async fn set_domains(&self, domains: Vec<String>) {
        self.router.set_domains(domains).await;
    }

    pub async fn start(&mut self) -> crate::error::Result<()> {
        if *self.running.read().await {
            return Ok(());
        }

        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await
            .map_err(|e| crate::error::HaioError::Proxy(format!("Failed to bind {}: {}", addr, e)))?;

        *self.running.write().await = true;

        let router = self.router.clone();
        let running = self.running.clone();
        let port = self.port;
        let notify = self.notify.clone();

        let handle = tokio::spawn(async move {
            tracing::info!("HTTP proxy listening on {}", addr);
            loop {
                tokio::select! {
                    _ = notify.notified() => {
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _)) => {
                                let router = router.clone();
                                tokio::spawn(handle_connection(stream, router, port));
                            }
                            Err(e) => {
                                if !*running.read().await {
                                    break;
                                }
                                tracing::error!("Accept error: {}", e);
                            }
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    pub async fn stop(&mut self) -> crate::error::Result<()> {
        *self.running.write().await = false;
        self.notify.notify_one();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        Ok(())
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    router: Arc<DomainRouter>,
    pac_port: u16,
) {
    // Read the initial request line and headers using a timeout
    let (request_line, stream) = match tokio::time::timeout(
        Duration::from_secs(10),
        read_request(stream),
    ).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            tracing::debug!("Failed to read request: {}", e);
            return;
        }
        Err(_) => {
            tracing::debug!("Timed out reading request headers");
            return;
        }
    };

    // Handle PAC request
    if request_line.starts_with("GET /pac.js") {
        let domains = router.get_domains().await;
        let pac_script = pac::build_pac_script(&domains, pac_port, router.socks_port());
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\n\r\n{}",
            pac_script.len(),
            pac_script
        );
        let mut stream = stream;
        let _ = stream.write_all(response.as_bytes()).await;
        return;
    }

    // Handle CONNECT
    if request_line.starts_with("CONNECT ") {
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            return;
        }
        let host_port = parts[1];
        let (host, port) = match host_port.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().unwrap_or(443)),
            None => (host_port, 443),
        };

        let mut stream = stream;

        if router.should_proxy(host).await {
            match socks::dial_socks5(&router.socks_addr(), host, port).await {
                Ok(mut remote) => {
                    let _ = stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await;
                    let result = tokio::time::timeout(
                        IDLE_TIMEOUT,
                        tokio::io::copy_bidirectional(&mut stream, &mut remote),
                    ).await;
                    match result {
                        Ok(Ok((up, down))) => {
                            tracing::debug!("CONNECT {}:{} done (up={} down={})", host, port, up, down);
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("CONNECT {}:{} stream error: {}", host, port, e);
                        }
                        Err(_) => {
                            tracing::warn!("CONNECT {}:{} timed out after {:?}", host, port, IDLE_TIMEOUT);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("SOCKS5 connect to {}:{} failed: {}", host, port, e);
                    let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                }
            }
        } else {
            match socks::dial_direct(host, port).await {
                Ok(mut remote) => {
                    let _ = stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await;
                    let result = tokio::time::timeout(
                        IDLE_TIMEOUT,
                        tokio::io::copy_bidirectional(&mut stream, &mut remote),
                    ).await;
                    match result {
                        Ok(Ok((up, down))) => {
                            tracing::debug!("DIRECT {}:{} done (up={} down={})", host, port, up, down);
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("DIRECT {}:{} stream error: {}", host, port, e);
                        }
                        Err(_) => {
                            tracing::warn!("DIRECT {}:{} timed out after {:?}", host, port, IDLE_TIMEOUT);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Direct connect to {}:{} failed: {}", host, port, e);
                    let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                }
            }
        }
        return;
    }

    // Handle plain HTTP proxy request (absolute-URI: GET http://host/path)
    let method = request_line.split_whitespace().next().unwrap_or("");
    if matches!(method, "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS") {
        let upper = request_line.to_uppercase();
        if let Some(_rest) = upper.split_once(" HTTP://") {
            let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
            if parts.len() < 3 {
                return;
            }
            let url_str = parts[1];
            let suffix = parts[2..].join(" ");

            let url = match url::Url::parse(url_str) {
                Ok(u) => u,
                Err(_) => return,
            };
            let host = match url.host_str() {
                Some(h) => h,
                None => return,
            };
            let port = url.port().unwrap_or(80);

            let path = url.path();
            let query = url.query().map(|q| format!("?{}", q)).unwrap_or_default();
            let origin_line = format!("{} {}{} {}", parts[0], path, query, suffix);

            let mut stream = stream;

            if router.should_proxy(host).await {
                match socks::dial_socks5(&router.socks_addr(), host, port).await {
                    Ok(mut remote) => {
                        let _ = remote.write_all(origin_line.as_bytes()).await;
                        let result = tokio::time::timeout(
                            IDLE_TIMEOUT,
                            tokio::io::copy_bidirectional(&mut stream, &mut remote),
                        ).await;
                        if let Err(e) = match result {
                            Ok(Ok(_)) => Ok(()),
                            Ok(Err(e)) => Err(e.to_string()),
                            Err(_) => Err("timeout".to_string()),
                        } {
                            tracing::warn!("HTTP proxy {}:{} error: {}", host, port, e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("SOCKS5 connect to {}:{} failed: {}", host, port, e);
                        let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                    }
                }
            } else {
                match socks::dial_direct(host, port).await {
                    Ok(mut remote) => {
                        let _ = remote.write_all(origin_line.as_bytes()).await;
                        let result = tokio::time::timeout(
                            IDLE_TIMEOUT,
                            tokio::io::copy_bidirectional(&mut stream, &mut remote),
                        ).await;
                        if let Err(e) = match result {
                            Ok(Ok(_)) => Ok(()),
                            Ok(Err(e)) => Err(e.to_string()),
                            Err(_) => Err("timeout".to_string()),
                        } {
                            tracing::warn!("HTTP direct {}:{} error: {}", host, port, e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Direct connect to {}:{} failed: {}", host, port, e);
                        let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                    }
                }
            }
        }
    }
}

async fn read_request(
    stream: tokio::net::TcpStream,
) -> crate::error::Result<(String, tokio::net::TcpStream)> {
    use tokio::io::AsyncReadExt;

    let mut stream = stream;
    let mut buf = Vec::with_capacity(512);
    let mut tmp = [0u8; 1];

    // Read byte-by-byte until we find \r\n\r\n (end of HTTP headers)
    // This ensures we don't over-read into the SSL ClientHello
    loop {
        let n = stream.read(&mut tmp).await
            .map_err(|e| crate::error::HaioError::Proxy(format!("Failed to read request: {}", e)))?;
        if n == 0 {
            return Err(crate::error::HaioError::Proxy("Connection closed before headers".into()));
        }
        buf.push(tmp[0]);

        // Check for end-of-headers: \r\n\r\n
        if buf.len() >= 4 && buf[buf.len()-4..] == *b"\r\n\r\n" {
            break;
        }

        // Safety limit
        if buf.len() > 8192 {
            return Err(crate::error::HaioError::Proxy("Headers too large".into()));
        }
    }

    let raw = String::from_utf8_lossy(&buf).to_string();
    let request_line = raw.lines().next().unwrap_or("").trim().to_string();

    Ok((request_line, stream))
}
