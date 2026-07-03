use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct DomainRouter {
    domains: Arc<RwLock<HashSet<String>>>,
    socks_port: u16,
}

impl DomainRouter {
    pub fn new(domains: Vec<String>, socks_port: u16) -> Self {
        let set: HashSet<String> = domains.into_iter().collect();
        Self {
            domains: Arc::new(RwLock::new(set)),
            socks_port,
        }
    }

    pub async fn set_domains(&self, domains: Vec<String>) {
        let set: HashSet<String> = domains.into_iter().collect();
        *self.domains.write().await = set;
    }

    pub async fn should_proxy(&self, host: &str) -> bool {
        let domains = self.domains.read().await;
        let h = host.to_lowercase();
        if domains.contains(&h) {
            return true;
        }
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
