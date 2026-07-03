use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct DomainRouter {
    domains: Arc<RwLock<HashSet<String>>>,
    suffix_index: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    socks_port: u16,
}

impl DomainRouter {
    pub fn new(domains: Vec<String>, socks_port: u16) -> Self {
        let set: HashSet<String> = domains.iter().cloned().collect();
        let suffix_index = build_suffix_index(&set);
        Self {
            domains: Arc::new(RwLock::new(set)),
            suffix_index: Arc::new(RwLock::new(suffix_index)),
            socks_port,
        }
    }

    pub async fn set_domains(&self, domains: Vec<String>) {
        let set: HashSet<String> = domains.into_iter().collect();
        let suffix_index = build_suffix_index(&set);
        *self.domains.write().await = set;
        *self.suffix_index.write().await = suffix_index;
    }

    pub async fn should_proxy(&self, host: &str) -> bool {
        let h = host.to_lowercase();

        // Fast path: exact match (O(1))
        {
            let domains = self.domains.read().await;
            if domains.contains(&h) {
                return true;
            }
        }

        // Slow path: subdomain match using suffix index
        // e.g., "www.google.com" -> lookup suffix "google.com" in suffix_index
        if let Some(two_part) = extract_two_part_suffix(&h) {
            let suffix_index = self.suffix_index.read().await;
            if let Some(base_domains) = suffix_index.get(&two_part) {
                // Check if the host is a subdomain of any base domain
                // e.g., "www.news.google.com" -> suffix "news.google.com" -> check "google.com"
                if base_domains.iter().any(|d| h == *d || h.ends_with(&format!(".{}", d))) {
                    return true;
                }
            }
        }

        // Fallback: full scan (rare)
        let domains = self.domains.read().await;
        for d in domains.iter() {
            if h.ends_with(&format!(".{}", d)) {
                return true;
            }
        }
        false
    }

    pub fn socks_addr(&self) -> String {
        format!("127.0.0.1:{}", self.socks_port)
    }

    pub async fn get_domains(&self) -> Vec<String> {
        let domains = self.domains.read().await;
        let mut list: Vec<String> = domains.iter().cloned().collect();
        list.sort();
        list
    }
}

/// Build a suffix index: "google.com" -> { "youtube.com", "google.com", ... }
/// Keys are two-part suffixes (e.g., "google.com", "co.uk")
/// Values are the base domains that end with that suffix
fn build_suffix_index(domains: &HashSet<String>) -> HashMap<String, HashSet<String>> {
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();

    for domain in domains {
        if let Some(suffix) = extract_two_part_suffix(domain) {
            index.entry(suffix).or_default().insert(domain.clone());
        }
    }

    index
}

/// Extract a two-part suffix from a domain for indexing.
/// "www.news.google.com" -> "google.com"
/// "youtube.com" -> "youtube.com"
/// "t.co" -> "t.co"
fn extract_two_part_suffix(domain: &str) -> Option<String> {
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() >= 2 {
        Some(format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1]))
    } else {
        None
    }
}
