use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, Notify};
use tokio::io::{BufReader, AsyncBufReadExt};

use super::router::DomainRouter;
use super::socks;
use super::pac;

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
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();

    if reader.read_line(&mut request_line).await.is_err() {
        return;
    }

    // Read headers
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
        if line.trim().is_empty() {
            break;
        }
    }

    let request_line = request_line.trim().to_string();

    // Reassemble the stream from BufReader
    let stream = reader.into_inner();

    // Handle PAC request
    if request_line.starts_with("GET /pac.js") {
        let domains = router.get_domains().await;
        let pac_script = pac::build_pac_script(&domains, pac_port);
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
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
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
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
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
    if request_line.to_uppercase().starts_with("GET http://")
        || request_line.to_uppercase().starts_with("POST http://")
        || request_line.to_uppercase().starts_with("PUT http://")
        || request_line.to_uppercase().starts_with("DELETE http://")
        || request_line.to_uppercase().starts_with("PATCH http://")
        || request_line.to_uppercase().starts_with("HEAD http://")
        || request_line.to_uppercase().starts_with("OPTIONS http://")
    {
        let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
        if parts.len() < 3 {
            return;
        }
        let url_str = parts[1];
        let rest = parts[2..].join(" ");

        // Parse the absolute URL
        let url = match url::Url::parse(url_str) {
            Ok(u) => u,
            Err(_) => return,
        };
        let host = match url.host_str() {
            Some(h) => h,
            None => return,
        };
        let port = url.port().unwrap_or(80);

        // Build origin-form request line
        let path = url.path();
        let query = url.query().map(|q| format!("?{}", q)).unwrap_or_default();
        let origin_line = format!("{} {}{} {}", parts[0], path, query, rest);

        let mut stream = stream;

        if router.should_proxy(host).await {
            match socks::dial_socks5(&router.socks_addr(), host, port).await {
                Ok(mut remote) => {
                    let _ = remote.write_all(origin_line.as_bytes()).await;
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
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
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut remote).await;
                }
                Err(e) => {
                    tracing::error!("Direct connect to {}:{} failed: {}", host, port, e);
                    let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                }
            }
        }
    }
}
