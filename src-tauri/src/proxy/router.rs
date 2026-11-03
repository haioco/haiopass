use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Where a connection should go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Through the Haio tunnel.
    Tunnel,
    /// Straight to the destination (not proxied).
    Direct,
}

/// One set of routing rules (either "tunnel these" or "bypass these").
#[derive(Debug, Default)]
struct RuleSet {
    entries: HashSet<String>,
}

impl RuleSet {
    fn new(domains: Vec<String>) -> Self {
        let entries = domains
            .into_iter()
            .map(|d| d.trim().trim_end_matches('.').to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect();
        Self { entries }
    }

    /// True when `host` is an entry or a subdomain of an entry.
    ///
    /// Walks the host's label suffixes (`a.b.example.com` → `b.example.com`
    /// → `example.com` → `com`) and does only hash lookups: no allocations
    /// and O(number of labels) regardless of how large the list is.
    fn matches(&self, host: &str) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        if self.entries.contains(host) {
            return true;
        }
        let mut rest = host;
        while let Some((_, tail)) = rest.split_once('.') {
            rest = tail;
            if self.entries.contains(rest) {
                return true;
            }
        }
        false
    }
}

/// Immutable snapshot of every routing rule. Replaced atomically, so an
/// in-flight connection always sees one consistent set.
#[derive(Debug, Default)]
struct Rules {
    include: RuleSet,
    bypass: RuleSet,
}

impl Rules {
    fn route(&self, host: &str) -> Route {
        let normalized = normalize_host(host);
        let host = normalized.as_ref();
        // Exclusions always win, even when the host is also listed as proxied.
        if self.bypass.matches(host) {
            return Route::Direct;
        }
        if self.include.matches(host) {
            return Route::Tunnel;
        }
        Route::Direct
    }
}

pub struct DomainRouter {
    rules: RwLock<Arc<Rules>>,
}

impl DomainRouter {
    pub fn new(domains: Vec<String>, bypass_domains: Vec<String>) -> Self {
        Self {
            rules: RwLock::new(Arc::new(Rules {
                include: RuleSet::new(domains),
                bypass: RuleSet::new(bypass_domains),
            })),
        }
    }

    fn current(&self) -> Arc<Rules> {
        self.rules
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Replace the proxied-domain list, keeping the bypass list intact.
    pub fn set_domains(&self, domains: Vec<String>) {
        let mut guard = self
            .rules
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bypass = RuleSet::new(guard.bypass.entries.iter().cloned().collect());
        *guard = Arc::new(Rules {
            include: RuleSet::new(domains),
            bypass,
        });
    }

    /// Replace the bypass list, keeping the proxied-domain list intact.
    pub fn set_bypass_domains(&self, bypass_domains: Vec<String>) {
        let mut guard = self
            .rules
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let include = RuleSet::new(guard.include.entries.iter().cloned().collect());
        *guard = Arc::new(Rules {
            include,
            bypass: RuleSet::new(bypass_domains),
        });
    }

    pub fn route(&self, host: &str) -> Route {
        self.current().route(host)
    }

    pub fn should_proxy(&self, host: &str) -> bool {
        self.route(host) == Route::Tunnel
    }

    pub fn get_domains(&self) -> Vec<String> {
        let mut list: Vec<String> = self.current().include.entries.iter().cloned().collect();
        list.sort();
        list
    }

    pub fn get_bypass_domains(&self) -> Vec<String> {
        let mut list: Vec<String> = self.current().bypass.entries.iter().cloned().collect();
        list.sort();
        list
    }
}

/// Lowercase a hostname and strip the syntactic noise HTTP allows: a trailing
/// root dot, or brackets around an IPv6 literal. Avoids an allocation when the
/// host is already canonical (the common case).
fn normalize_host(host: &str) -> Cow<'_, str> {
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let host = host.strip_suffix('.').unwrap_or(host);

    if host.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(host.to_ascii_lowercase())
    } else {
        Cow::Borrowed(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router(include: &[&str], bypass: &[&str]) -> DomainRouter {
        DomainRouter::new(
            include.iter().map(|s| s.to_string()).collect(),
            bypass.iter().map(|s| s.to_string()).collect(),
        )
    }

    #[test]
    fn routes_included_domains_and_subdomains() {
        let r = router(&["google.com", "youtube.com"], &[]);
        assert_eq!(r.route("google.com"), Route::Tunnel);
        assert_eq!(r.route("www.google.com"), Route::Tunnel);
        assert_eq!(r.route("a.b.youtube.com"), Route::Tunnel);
        assert_eq!(r.route("example.com"), Route::Direct);
    }

    #[test]
    fn does_not_match_lookalike_suffixes() {
        let r = router(&["google.com"], &[]);
        assert_eq!(r.route("notgoogle.com"), Route::Direct);
        assert_eq!(r.route("google.com.evil.example"), Route::Direct);
        assert_eq!(r.route("google.co"), Route::Direct);
    }

    #[test]
    fn bypass_wins_over_include() {
        let r = router(&["google.com"], &["google.com"]);
        assert_eq!(r.route("google.com"), Route::Direct);
        assert_eq!(r.route("mail.google.com"), Route::Direct);
    }

    #[test]
    fn bypass_only_affects_its_own_subtree() {
        let r = router(&["google.com"], &["ads.google.com"]);
        assert_eq!(r.route("ads.google.com"), Route::Direct);
        assert_eq!(r.route("www.ads.google.com"), Route::Direct);
        assert_eq!(r.route("google.com"), Route::Tunnel);
        assert_eq!(r.route("mail.google.com"), Route::Tunnel);
    }

    #[test]
    fn host_normalisation_is_case_and_dot_insensitive() {
        let r = router(&["google.com"], &[]);
        assert_eq!(r.route("WWW.Google.COM."), Route::Tunnel);
        assert_eq!(r.route("[::1]"), Route::Direct);
    }

    #[test]
    fn ip_entries_match_exactly() {
        let r = router(&[], &["192.168.1.10"]);
        assert_eq!(r.route("192.168.1.10"), Route::Direct);
        assert_eq!(r.route("192.168.1.11"), Route::Direct);
        // Exact match still wins for a bypassed IP.
        let r = router(&["192.168.1.10"], &["192.168.1.10"]);
        assert_eq!(r.route("192.168.1.10"), Route::Direct);
    }

    #[test]
    fn live_updates_keep_the_other_list() {
        let r = router(&["google.com"], &["internal.example"]);
        r.set_bypass_domains(vec!["bank.example".into()]);
        assert_eq!(r.route("google.com"), Route::Tunnel);
        assert_eq!(r.route("internal.example"), Route::Direct);
        assert_eq!(r.route("bank.example"), Route::Direct);

        r.set_domains(vec!["youtube.com".into()]);
        assert_eq!(r.route("google.com"), Route::Direct);
        assert_eq!(r.route("bank.example"), Route::Direct);
        assert_eq!(r.get_bypass_domains(), vec!["bank.example".to_string()]);
    }

    #[test]
    fn empty_rules_route_everything_direct() {
        let r = router(&[], &[]);
        assert_eq!(r.route("example.com"), Route::Direct);
    }
}
