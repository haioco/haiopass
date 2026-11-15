//! HTTP client for `portal.haio.ir`.
//!
//! All portal traffic runs in Rust. The webview CSP keeps `connect-src`
//! restricted to IPC, and this module never returns a token or a payment URL to
//! JavaScript.

use super::error::{parse_drf_error, PortalError};
use reqwest::header::RETRY_AFTER;
use serde::de::DeserializeOwned;
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://portal.haio.ir/api/v1/";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// `Host:port` suffix used in the User-Agent. Reads the crate version so a
/// version bump does not need a source edit here.
fn user_agent() -> String {
    format!("HaioBypass/{} (desktop)", env!("CARGO_PKG_VERSION"))
}

fn is_valid_base_url(value: &str) -> bool {
    value.starts_with("https://") && value.ends_with('/')
}

/// Base URL. A staging override is only honoured in debug builds.
fn resolve_base_url() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(value) = std::env::var("HAIO_PORTAL_BASE_URL") {
            if is_valid_base_url(&value) {
                return value;
            }
            tracing::warn!("ignoring HAIO_PORTAL_BASE_URL: must be https://.../ ");
        }
    }
    DEFAULT_BASE_URL.to_string()
}

pub struct PortalClient {
    http: reqwest::Client,
    base_url: String,
}

impl PortalClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(user_agent())
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!("portal HTTP client fell back to defaults: {}", e.is_builder());
                reqwest::Client::new()
            });

        Self {
            http,
            base_url: resolve_base_url(),
        }
    }

    #[cfg(test)]
    pub fn with_base_url(base_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(user_agent())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("test client");
        Self {
            http,
            base_url: base_url.to_string(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path.trim_start_matches('/'))
    }

    /// Build a JSON request. `None` body means "no body" (GET/DELETE).
    pub fn json_request(
        &self,
        method: reqwest::Method,
        path: &str,
        bearer: Option<&str>,
        body: Option<&serde_json::Value>,
    ) -> reqwest::RequestBuilder {
        let mut builder = self
            .http
            .request(method, self.url(path))
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(token) = bearer {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = body {
            builder = builder.json(body);
        }
        builder
    }

    /// Send and decode. Non-2xx responses become typed `PortalError`s; DRF
    /// bodies are parsed for field errors and provider codes.
    pub async fn send_json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, PortalError> {
        let response = request.send().await?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(parse_drf_error(status.as_u16(), retry_after, &body));
        }

        // 204 / empty 2xx: a `serde_json::Value` target accepts null; a typed
        // target fails cleanly as a protocol error.
        if body.trim().is_empty() {
            return serde_json::from_value(serde_json::Value::Null)
                .map_err(|_| PortalError::protocol());
        }

        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| PortalError::protocol())?;
        serde_json::from_value(value).map_err(|_| PortalError::protocol())
    }
}

impl Default for PortalClient {
    fn default() -> Self {
        Self::new()
    }
}

/* ------------------------------------------------------------------ */
/* Payment URL validation                                             */
/* ------------------------------------------------------------------ */

/// Extra gateway hosts, comma-separated, operator-controlled via env. Each
/// entry is a hostname (optionally `*.suffix`), never a scheme or path.
fn extra_gateway_hosts() -> Vec<String> {
    std::env::var("HAIO_PAYMENT_HOSTS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(|entry| entry.trim().to_ascii_lowercase())
                .filter(|entry| {
                    !entry.is_empty()
                        && entry
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '*')
                })
                .collect()
        })
        .unwrap_or_default()
}

fn host_matches(host: &str, allowed: &str) -> bool {
    let allowed = allowed.trim_start_matches("*.").trim_start_matches('.');
    if allowed.is_empty() {
        return false;
    }
    host == allowed || host.ends_with(&format!(".{}", allowed))
}

/// Accept only HTTPS URLs on Zibal (or an explicitly configured gateway).
///
/// Rejects `javascript:`, `data:`, `file:`, custom schemes, credentials in the
/// URL, and arbitrary hosts. The URL is never persisted.
pub fn validate_payment_url(raw: &str) -> Result<url::Url, PortalError> {
    let parsed = url::Url::parse(raw.trim()).map_err(|_| PortalError::payment_url_invalid())?;
    if parsed.scheme() != "https" {
        return Err(PortalError::payment_url_invalid());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(PortalError::payment_url_invalid());
    }
    let host = parsed
        .host_str()
        .map(|h| h.to_ascii_lowercase())
        .ok_or_else(PortalError::payment_url_invalid)?;

    let mut allowed = vec!["zibal.ir".to_string()];
    allowed.extend(extra_gateway_hosts());
    if allowed.iter().any(|entry| host_matches(&host, entry)) {
        Ok(parsed)
    } else {
        Err(PortalError::payment_url_invalid())
    }
}

/// Log-safe representation: scheme + host + path only, query stripped.
pub fn redact_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("?");
            format!("{}://{}{}", parsed.scheme(), host, parsed.path())
        }
        Err(_) => "<invalid-url>".to_string(),
    }
}

/// Best-effort extraction of `results` from a DRF paginated or plain list.
pub fn list_from_value(value: serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(mut map) => match map.remove("results") {
            Some(serde_json::Value::Array(items)) => items,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

pub fn decode_list<T: DeserializeOwned>(value: serde_json::Value) -> Result<Vec<T>, PortalError> {
    list_from_value(value)
        .into_iter()
        .map(|item| serde_json::from_value(item).map_err(|_| PortalError::protocol()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::error::codes;

    #[test]
    fn accepts_zibal_hosts() {
        assert!(validate_payment_url("https://gateway.zibal.ir/start/abc").is_ok());
        assert!(validate_payment_url("https://zibal.ir/pay").is_ok());
    }

    #[test]
    fn rejects_insecure_schemes() {
        assert_eq!(
            validate_payment_url("http://gateway.zibal.ir/x").unwrap_err().code,
            codes::PAYMENT_URL_INVALID
        );
        assert!(validate_payment_url("javascript:alert(1)").is_err());
        assert!(validate_payment_url("data:text/html,hi").is_err());
        assert!(validate_payment_url("file:///etc/passwd").is_err());
        assert!(validate_payment_url("haiobypass://payment").is_err());
    }

    #[test]
    fn rejects_lookalike_hosts() {
        assert!(validate_payment_url("https://zibal.ir.evil.com/pay").is_err());
        assert!(validate_payment_url("https://evil.com/zibal.ir").is_err());
        assert!(validate_payment_url("https://zibal.ir@evil.com/pay").is_err());
    }

    #[test]
    fn redaction_strips_query() {
        let redacted = redact_url("https://gateway.zibal.ir/start/abc?token=secret");
        assert_eq!(redacted, "https://gateway.zibal.ir/start/abc");
    }

    #[test]
    fn decodes_paginated_payload() {
        let value = serde_json::json!({"count": 1, "results": [{"id": "a"}]});
        let items: Vec<serde_json::Value> = decode_list(value).unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn decodes_plain_array() {
        let value = serde_json::json!([{"id": "a"}, {"id": "b"}]);
        let items: Vec<serde_json::Value> = decode_list(value).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn user_agent_contains_version() {
        assert!(user_agent().contains(env!("CARGO_PKG_VERSION")));
    }
}
