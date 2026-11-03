use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DNS_TTL: Duration = Duration::from_secs(60);
const DNS_CACHE_LIMIT: usize = 256;

struct CacheEntry {
    resolved_at: Instant,
    addrs: Vec<SocketAddr>,
}

fn dns_cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_addrs(key: &str) -> Option<Vec<SocketAddr>> {
    let mut cache = dns_cache().lock().ok()?;
    if let Some(entry) = cache.get(key) {
        if entry.resolved_at.elapsed() < DNS_TTL {
            return Some(entry.addrs.clone());
        }
    }
    cache.remove(key);
    None
}

fn store_addrs(key: String, addrs: Vec<SocketAddr>) {
    if let Ok(mut cache) = dns_cache().lock() {
        if cache.len() >= DNS_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(
            key,
            CacheEntry {
                resolved_at: Instant::now(),
                addrs,
            },
        );
    }
}

fn socket_addr(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    }
}

async fn resolve(host: &str, port: u16) -> crate::error::Result<Vec<SocketAddr>> {
    let key = socket_addr(host, port);
    if let Some(addrs) = cached_addrs(&key) {
        return Ok(addrs);
    }

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(&key)
        .await
        .map_err(crate::error::HaioError::Io)?
        .collect();
    if addrs.is_empty() {
        return Err(crate::error::HaioError::Proxy(format!(
            "DNS returned no addresses for {}",
            key
        )));
    }

    store_addrs(key, addrs.clone());
    Ok(addrs)
}

pub async fn dial_direct(target_host: &str, target_port: u16) -> crate::error::Result<TcpStream> {
    let addrs = resolve(target_host, target_port).await?;
    let addr_label = socket_addr(target_host, target_port);

    // Try every resolved address within one overall connect budget instead
    // of spending the full timeout on a blackholed IPv6 address.
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut last_error = None;

    for addr in addrs {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(e)) => last_error = Some(e.to_string()),
            Err(_) => last_error = Some(format!("timed out connecting to {}", addr)),
        }
    }

    Err(crate::error::HaioError::Proxy(format!(
        "Direct connect to {} failed: {}",
        addr_label,
        last_error.unwrap_or_else(|| "timeout".to_string())
    )))
}
