use crate::domains::fallback::FALLBACK_DOMAINS;

pub struct DomainStore {
    pub cached_domains: Vec<String>,
    pub last_fetch: Option<i64>,
    pub using_fallback: bool,
    pub using_cache: bool,
    pub last_fetch_error: Option<String>,
}

impl Default for DomainStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DomainStore {
    pub fn new() -> Self {
        Self {
            cached_domains: Vec::new(),
            last_fetch: None,
            using_fallback: false,
            using_cache: false,
            last_fetch_error: None,
        }
    }

    pub fn effective_domains(&self) -> Vec<String> {
        if !self.cached_domains.is_empty() {
            self.cached_domains.clone()
        } else {
            FALLBACK_DOMAINS.iter().map(|s| s.to_string()).collect()
        }
    }

    pub fn refresh(&mut self, domains: Vec<String>) {
        if !domains.is_empty() {
            self.cached_domains = domains;
            self.last_fetch = Some(chrono::Utc::now().timestamp());
            self.using_fallback = false;
            self.using_cache = false;
            self.last_fetch_error = None;
        }
    }

    pub fn set_fallback(&mut self) {
        self.using_fallback = true;
        self.using_cache = false;
    }
}
