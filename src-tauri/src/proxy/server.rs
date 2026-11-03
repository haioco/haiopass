use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{Notify, RwLock};

use super::pac;
use super::router::DomainRouter;
use super::socks;
use crate::config::TrojanConfig;
use crate::trojan::manager::SharedTrojanConfig;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Larger than tokio's 8 KiB default: the tunnel carries TLS records and
/// media, where a bigger buffer cuts syscalls and wake-ups noticeably.
const STREAM_BUFFER_SIZE: usize = 32 * 1024;

pub struct ProxyServer {
    handle: Option<tokio::task::JoinHandle<()>>,
    port: u16,
    router: Arc<DomainRouter>,
    trojan_config: SharedTrojanConfig,
    running: Arc<RwLock<bool>>,
    notify: Arc<Notify>,
    pac_cache: Arc<RwLock<Option<String>>>,
}

impl ProxyServer {
    pub fn new(port: u16, router: DomainRouter, trojan_config: SharedTrojanConfig) -> Self {
        Self {
            handle: None,
            port,
            router: Arc::new(router),
            trojan_config,
            running: Arc::new(RwLock::new(false)),
            notify: Arc::new(Notify::new()),
            pac_cache: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn set_domains(&self, domains: Vec<String>) {
        self.router.set_domains(domains);
        self.invalidate_pac().await;
    }

    pub async fn set_bypass_domains(&self, domains: Vec<String>) {
        self.router.set_bypass_domains(domains);
        self.invalidate_pac().await;
    }

    async fn invalidate_pac(&self) {
        *self.pac_cache.write().await = None;
    }

    pub async fn start(&mut self) -> crate::error::Result<()> {
        if *self.running.read().await {
            return Ok(());
        }

        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await.map_err(|e| {
            crate::error::HaioError::Proxy(format!("Failed to bind {}: {}", addr, e))
        })?;

        *self.running.write().await = true;

        let router = self.router.clone();
        let trojan_config = self.trojan_config.clone();
        let running = self.running.clone();
        let port = self.port;
        let notify = self.notify.clone();
        let pac_cache = self.pac_cache.clone();

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
                                let _ = stream.set_nodelay(true);
                                let router = router.clone();
                                let trojan_config = trojan_config.clone();
                                let pac_cache = pac_cache.clone();
                                tokio::spawn(handle_connection(
                                    stream,
                                    router,
                                    trojan_config,
                                    port,
                                    pac_cache,
                                ));
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

/// Read the live tunnel configuration and open a tunnelled connection.
/// Preserves the one-retry semantics the old SOCKS5 dialer had.
async fn dial_trojan(
    trojan_config: &SharedTrojanConfig,
    host: &str,
    port: u16,
) -> crate::error::Result<crate::trojan::client::TrojanStream> {
    let config: TrojanConfig = trojan_config.read().await.clone().ok_or_else(|| {
        crate::error::HaioError::Proxy(
            "Tunnel is not connected — no server credentials are set".into(),
        )
    })?;

    match crate::trojan::client::dial(&config, host, port).await {
        Ok(stream) => Ok(stream),
        Err(e) => {
            tracing::warn!(
                "Tunnel dial to {}:{} failed once, retrying: {}",
                host,
                port,
                e
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
            let config: TrojanConfig = trojan_config.read().await.clone().ok_or_else(|| {
                crate::error::HaioError::Proxy(
                    "Tunnel is not connected — no server credentials are set".into(),
                )
            })?;
            crate::trojan::client::dial(&config, host, port).await
        }
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    router: Arc<DomainRouter>,
    trojan_config: SharedTrojanConfig,
    pac_port: u16,
    pac_cache: Arc<RwLock<Option<String>>>,
) {
    // Read the request line and all headers with a timeout.
    let (head, stream) =
        match tokio::time::timeout(Duration::from_secs(10), read_http_head(stream)).await {
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

    let Some((method, target, _version)) = split_request_line(&head.request_line) else {
        let mut stream = stream;
        let _ = stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
            .await;
        return;
    };

    // Handle PAC request
    if method == "GET" && (target == "/pac.js" || target.starts_with("/pac.js?")) {
        // Building the PAC embeds every routed domain, so cache it until the
        // lists change. The OS may fetch this file repeatedly.
        let pac_script = {
            let mut cache = pac_cache.write().await;
            match cache.as_ref() {
                Some(script) => script.clone(),
                None => {
                    let domains = router.get_domains();
                    let bypass = router.get_bypass_domains();
                    let script = pac::build_pac_script(&domains, &bypass, pac_port);
                    *cache = Some(script.clone());
                    script
                }
            }
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
            pac_script.len(),
            pac_script
        );
        let mut stream = stream;
        let _ = stream.write_all(response.as_bytes()).await;
        return;
    }

    // Handle CONNECT
    if method == "CONNECT" {
        let Some((host, port)) = parse_host_port(target, 443) else {
            let mut stream = stream;
            let _ = stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
                .await;
            return;
        };

        let mut stream = stream;

        if router.should_proxy(&host) {
            match dial_trojan(&trojan_config, &host, port).await {
                Ok(remote) => {
                    relay_connect(
                        stream,
                        remote,
                        &head.leftover,
                        &host,
                        port,
                        "CONNECT tunnel",
                    )
                    .await;
                }
                Err(e) => {
                    tracing::error!("Tunnel connect to {}:{} failed: {}", host, port, e);
                    let _ = stream
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                        .await;
                }
            }
        } else {
            match socks::dial_direct(&host, port).await {
                Ok(remote) => {
                    relay_connect(
                        stream,
                        remote,
                        &head.leftover,
                        &host,
                        port,
                        "CONNECT direct",
                    )
                    .await;
                }
                Err(e) => {
                    tracing::error!("Direct connect to {}:{} failed: {}", host, port, e);
                    let _ = stream
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                        .await;
                }
            }
        }
        return;
    }

    // Handle plain HTTP proxy request (absolute-URI: GET http://host/path)
    let Some(request) = rewrite_plain_http(&head) else {
        let mut stream = stream;
        let _ = stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
            .await;
        return;
    };

    let host = request.host.clone();
    let port = request.port;
    let label = format!("HTTP {}:{}", host, port);
    let mut stream = stream;

    if router.should_proxy(&host) {
        match dial_trojan(&trojan_config, &host, port).await {
            Ok(mut remote) => {
                if let Err(e) = remote.write_all(&request.payload).await {
                    tracing::warn!("Tunnel write {}:{} failed: {}", host, port, e);
                    let _ = stream
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                }
                copy_streams(stream, remote, &label).await;
            }
            Err(e) => {
                tracing::error!("Tunnel connect to {}:{} failed: {}", host, port, e);
                let _ = stream
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                    .await;
            }
        }
    } else {
        match socks::dial_direct(&host, port).await {
            Ok(mut remote) => {
                if let Err(e) = remote.write_all(&request.payload).await {
                    tracing::warn!("Direct write {}:{} failed: {}", host, port, e);
                    let _ = stream
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                }
                copy_streams(stream, remote, &label).await;
            }
            Err(e) => {
                tracing::error!("Direct connect to {}:{} failed: {}", host, port, e);
                let _ = stream
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                    .await;
            }
        }
    }
}

async fn relay_connect<U>(
    mut stream: tokio::net::TcpStream,
    mut remote: U,
    leftover: &[u8],
    host: &str,
    port: u16,
    label: &str,
) where
    U: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let _ = stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await;

    // Bytes read past the CONNECT headers belong to the tunnelled protocol
    // (usually the TLS ClientHello), so replay them before piping.
    if !leftover.is_empty() {
        if let Err(e) = remote.write_all(leftover).await {
            tracing::warn!("{} {}:{} replay failed: {}", label, host, port, e);
            return;
        }
    }

    copy_streams(stream, remote, &format!("{} {}:{}", label, host, port)).await;
}

async fn copy_streams<U>(mut stream: tokio::net::TcpStream, mut remote: U, label: &str)
where
    U: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let result = tokio::time::timeout(
        IDLE_TIMEOUT,
        tokio::io::copy_bidirectional_with_sizes(
            &mut stream,
            &mut remote,
            STREAM_BUFFER_SIZE,
            STREAM_BUFFER_SIZE,
        ),
    )
    .await;

    match result {
        Ok(Ok((up, down))) => {
            tracing::debug!("{} done (up={} down={})", label, up, down);
        }
        Ok(Err(e)) => {
            tracing::warn!("{} stream error: {}", label, e);
        }
        Err(_) => {
            tracing::warn!("{} timed out after {:?}", label, IDLE_TIMEOUT);
        }
    }
}

const MAX_HEADER_BYTES: usize = 16 * 1024;

struct HttpHead {
    request_line: String,
    headers: Vec<String>,
    /// Bytes read past the end of the headers. For CONNECT this can be the
    /// start of the TLS handshake; for plain HTTP it can be the request body.
    leftover: Vec<u8>,
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn parse_head(buf: &[u8], end: usize) -> crate::error::Result<HttpHead> {
    let text = std::str::from_utf8(&buf[..end]).map_err(|_| {
        crate::error::HaioError::Proxy("Request headers are not valid UTF-8".into())
    })?;

    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("").trim_end().to_string();
    if request_line.is_empty() {
        return Err(crate::error::HaioError::Proxy("Empty request line".into()));
    }

    let headers = lines
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();

    Ok(HttpHead {
        request_line,
        headers,
        leftover: buf[end..].to_vec(),
    })
}

async fn read_http_head(
    mut stream: tokio::net::TcpStream,
) -> crate::error::Result<(HttpHead, tokio::net::TcpStream)> {
    use tokio::io::AsyncReadExt;

    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];

    loop {
        if let Some(end) = find_header_end(&buf) {
            return Ok((parse_head(&buf, end)?, stream));
        }

        if buf.len() >= MAX_HEADER_BYTES {
            return Err(crate::error::HaioError::Proxy("Headers too large".into()));
        }

        let n = stream.read(&mut chunk).await.map_err(|e| {
            crate::error::HaioError::Proxy(format!("Failed to read request: {}", e))
        })?;
        if n == 0 {
            return Err(crate::error::HaioError::Proxy(
                "Connection closed before headers".into(),
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn header_name(line: &str) -> &str {
    line.split(':').next().unwrap_or("").trim()
}

fn split_request_line(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next().unwrap_or("HTTP/1.1");
    Some((method, target, version))
}

fn parse_host_port(target: &str, default_port: u16) -> Option<(String, u16)> {
    if let Some(rest) = target.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = format!("[{}]", &rest[..end]);
        let port = match rest[end + 1..].strip_prefix(':') {
            Some(port) => port.parse().ok()?,
            None => default_port,
        };
        return Some((host, port));
    }

    match target.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Some((host.to_string(), port.parse().ok()?)),
        _ if !target.is_empty() => Some((target.to_string(), default_port)),
        _ => None,
    }
}

struct PlainHttpRequest {
    host: String,
    port: u16,
    payload: Vec<u8>,
}

/// Rewrite an absolute-URI proxy request into an origin-form request and keep
/// every end-to-end header. Only proxy-specific headers are dropped.
fn rewrite_plain_http(head: &HttpHead) -> Option<PlainHttpRequest> {
    let (method, target, version) = split_request_line(&head.request_line)?;
    let url = url::Url::parse(target).ok()?;
    if url.scheme() != "http" {
        return None;
    }

    let host = url.host_str()?.to_string();
    let port = url.port().unwrap_or(80);
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let query = url
        .query()
        .map(|query| format!("?{}", query))
        .unwrap_or_default();

    let mut payload = Vec::with_capacity(512);
    payload.extend_from_slice(format!("{} {}{} {}\r\n", method, path, query, version).as_bytes());

    let mut has_host = false;
    for line in &head.headers {
        let name = header_name(line);
        if name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("proxy-authorization")
        {
            continue;
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        payload.extend_from_slice(line.as_bytes());
        payload.extend_from_slice(b"\r\n");
    }

    if !has_host {
        let host_header = if port == 80 {
            host.clone()
        } else {
            format!("{}:{}", host, port)
        };
        payload.extend_from_slice(format!("Host: {}\r\n", host_header).as_bytes());
    }

    payload.extend_from_slice(b"\r\n");
    // Body bytes that arrived together with the headers.
    payload.extend_from_slice(&head.leftover);

    Some(PlainHttpRequest {
        host,
        port,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> HttpHead {
        let bytes = raw.as_bytes();
        let end = find_header_end(bytes).expect("complete headers");
        parse_head(bytes, end).expect("valid head")
    }

    #[test]
    fn header_end_is_found_at_crlfcrlf() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n\r\nbody"), Some(18));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn plain_http_keeps_end_to_end_headers() {
        let head = parse(
            "GET http://example.com/path?q=1 HTTP/1.1\r\n\
             Host: example.com\r\n\
             User-Agent: test-agent\r\n\
             Cookie: session=abc\r\n\
             Proxy-Connection: keep-alive\r\n\
             Proxy-Authorization: Basic secret\r\n\
             \r\n",
        );
        let request = rewrite_plain_http(&head).expect("absolute URI");
        let payload = String::from_utf8(request.payload).expect("ascii");

        assert_eq!(request.host, "example.com");
        assert_eq!(request.port, 80);
        assert!(payload.starts_with("GET /path?q=1 HTTP/1.1\r\n"));
        assert!(payload.contains("User-Agent: test-agent\r\n"));
        assert!(payload.contains("Cookie: session=abc\r\n"));
        assert!(payload.contains("Host: example.com\r\n"));
        assert!(!payload.to_ascii_lowercase().contains("proxy-connection"));
        assert!(!payload.to_ascii_lowercase().contains("proxy-authorization"));
    }

    #[test]
    fn plain_http_adds_host_when_missing() {
        let head = parse(
            "POST http://example.com:8080/upload HTTP/1.1\r\n\
             Content-Length: 5\r\n\
             \r\nhello",
        );
        let request = rewrite_plain_http(&head).expect("absolute URI");
        let payload = String::from_utf8(request.payload).expect("ascii");

        assert_eq!(request.port, 8080);
        assert!(payload.contains("Host: example.com:8080\r\n"));
        assert!(payload.ends_with("\r\n\r\nhello"));
    }

    #[test]
    fn connect_target_parsing_handles_ipv6_and_defaults() {
        assert_eq!(
            parse_host_port("example.com:8443", 443),
            Some(("example.com".to_string(), 8443))
        );
        assert_eq!(
            parse_host_port("example.com", 443),
            Some(("example.com".to_string(), 443))
        );
        assert_eq!(
            parse_host_port("[::1]:443", 443),
            Some(("[::1]".to_string(), 443))
        );
        assert_eq!(
            parse_host_port("[::1]", 443),
            Some(("[::1]".to_string(), 443))
        );
    }

    fn free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr").port()
    }

    #[tokio::test]
    async fn plain_http_proxy_forwards_headers_to_origin() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let origin = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("origin bind");
        let origin_port = origin.local_addr().expect("origin addr").port();
        let (request_tx, request_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.expect("origin accept");
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.expect("origin read");
            let _ = request_tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
        });

        let proxy_port = free_port();
        let router = DomainRouter::new(Vec::new(), Vec::new());
        let mut proxy = ProxyServer::new(proxy_port, router, Arc::new(RwLock::new(None)));
        proxy.start().await.expect("proxy start");

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", proxy_port))
            .await
            .expect("client connect");
        let request = format!(
            "GET http://127.0.0.1:{}/hello?x=1 HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Cookie: session=abc\r\n\
             User-Agent: haio-test\r\n\
             Proxy-Connection: keep-alive\r\n\
             \r\n",
            origin_port, origin_port
        );
        client
            .write_all(request.as_bytes())
            .await
            .expect("client write");

        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("client read");
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200 OK"));

        let forwarded = request_rx.await.expect("origin request");
        assert!(forwarded.starts_with("GET /hello?x=1 HTTP/1.1\r\n"));
        assert!(forwarded.contains("Cookie: session=abc\r\n"));
        assert!(forwarded.contains("User-Agent: haio-test\r\n"));
        assert!(!forwarded.to_ascii_lowercase().contains("proxy-connection"));

        proxy.stop().await.expect("proxy stop");
    }
}
