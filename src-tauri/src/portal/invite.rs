//! Invite link parsing and redaction.
//!
//! Accepted forms:
//! - `hgi_<43 base64url chars>`
//! - `haio://join/hgi_...`
//! - `https://my.haio.ir/join/hgi_...`
//!
//! The token is validated locally before any network call, and it must never
//! appear in a log line, an error message or a diagnostic payload.

use super::error::PortalError;

pub const TOKEN_PREFIX: &str = "hgi_";
pub const TOKEN_BODY_LEN: usize = 43;
const HTTPS_HOST: &str = "my.haio.ir";
const JOIN_PATH: &str = "/join/";

/// `^hgi_[A-Za-z0-9_-]{43}$`.
pub fn is_valid_token(token: &str) -> bool {
    let Some(body) = token.strip_prefix(TOKEN_PREFIX) else {
        return false;
    };
    body.len() == TOKEN_BODY_LEN
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn invalid() -> PortalError {
    // Never echo the rejected input: it is very likely a mistyped token.
    PortalError::invalid_input("لینک دعوت معتبر نیست.")
}

/// Validate a pasted invite link (or bare token) and return the token.
pub fn parse_invite_link(raw: &str) -> Result<String, PortalError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 512 {
        return Err(invalid());
    }

    // Bare token pasted without a scheme.
    if raw.starts_with(TOKEN_PREFIX) {
        return if is_valid_token(raw) {
            Ok(raw.to_string())
        } else {
            Err(invalid())
        };
    }

    let parsed = url::Url::parse(raw).map_err(|_| invalid())?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(invalid());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(invalid());
    }

    let scheme = parsed.scheme();
    let host = parsed.host_str().unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    if path.is_empty() {
        return Err(invalid());
    }

    let token = match scheme {
        "haio" => {
            // `haio://join/<token>`
            if !host.eq_ignore_ascii_case("join") {
                return Err(invalid());
            }
            path.trim_start_matches('/')
        }
        "https" => {
            // `https://my.haio.ir/join/<token>`
            if !host.eq_ignore_ascii_case(HTTPS_HOST) {
                return Err(invalid());
            }
            if !path.starts_with(JOIN_PATH) {
                return Err(invalid());
            }
            let rest = path.trim_start_matches(JOIN_PATH);
            if rest.contains('/') {
                return Err(invalid());
            }
            rest
        }
        _ => return Err(invalid()),
    };

    if is_valid_token(token) {
        Ok(token.to_string())
    } else {
        Err(invalid())
    }
}

/// Log-safe rendering of an invite link: shape only, never the token.
pub fn redact_invite_link(raw: &str) -> String {
    match parse_invite_link(raw) {
        Ok(_) => {
            if raw.trim().starts_with(TOKEN_PREFIX) {
                "hgi_***".to_string()
            } else if raw.trim().starts_with("haio://") {
                "haio://join/hgi_***".to_string()
            } else {
                format!("https://{}/join/hgi_***", HTTPS_HOST)
            }
        }
        Err(_) => "<invalid-invite-link>".to_string(),
    }
}

/// Prefix safe to show in a list (the server returns the same shape).
pub fn token_prefix(token: &str) -> String {
    token.chars().take(TOKEN_PREFIX.len() + 4).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(ch: char) -> String {
        format!("{}{}", TOKEN_PREFIX, ch.to_string().repeat(TOKEN_BODY_LEN))
    }

    #[test]
    fn accepts_a_bare_token() {
        let token = token('A');
        assert_eq!(parse_invite_link(&token).unwrap(), token);
        assert!(is_valid_token(&token));
    }

    #[test]
    fn accepts_the_custom_scheme() {
        let token = token('b');
        let link = format!("haio://join/{}", token);
        assert_eq!(parse_invite_link(&link).unwrap(), token);
    }

    #[test]
    fn accepts_the_https_landing_link() {
        let token = token('9');
        let link = format!("https://my.haio.ir/join/{}", token);
        assert_eq!(parse_invite_link(&link).unwrap(), token);
        // A trailing slash after the token is tolerated.
        assert_eq!(parse_invite_link(&format!("{}/", link)).unwrap(), token);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let token = token('x');
        assert_eq!(parse_invite_link(&format!("  {}  ", token)).unwrap(), token);
    }

    #[test]
    fn rejects_wrong_shape() {
        for bad in [
            "",
            "hgi_",
            "hgi_short",
            &format!("hgi_{}", "A".repeat(TOKEN_BODY_LEN + 1)),
            &format!("hgi_{}", "!".repeat(TOKEN_BODY_LEN)),
            "https://evil.example/join/hgi_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "https://my.haio.ir/other/hgi_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "https://my.haio.ir/join/hgi_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA/extra",
            "haio://other/hgi_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "http://my.haio.ir/join/hgi_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert!(parse_invite_link(bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn rejects_query_and_credentials() {
        let token = token('q');
        for bad in [
            format!("https://my.haio.ir/join/{}?next=/x", token),
            format!("https://user@my.haio.ir/join/{}", token),
        ] {
            assert!(parse_invite_link(&bad).is_err());
        }
    }

    #[test]
    fn redaction_never_leaks_the_token() {
        let token = token('Z');
        let link = format!("https://my.haio.ir/join/{}", token);
        let redacted = redact_invite_link(&link);
        assert!(!redacted.contains(&token));
        assert_eq!(redacted, "https://my.haio.ir/join/hgi_***");

        let redacted_deep = redact_invite_link(&format!("haio://join/{}", token));
        assert!(!redacted_deep.contains(&token));
        assert_eq!(redacted_deep, "haio://join/hgi_***");

        assert_eq!(
            redact_invite_link("nonsense token value"),
            "<invalid-invite-link>"
        );
    }

    #[test]
    fn prefix_is_short_and_does_not_leak_the_body() {
        let token = token('K');
        let prefix = token_prefix(&token);
        assert_eq!(prefix, "hgi_KKKK");
        assert!(!token.starts_with(&prefix) || prefix.len() < token.len());
    }

    #[test]
    fn error_message_does_not_echo_the_input() {
        let secret = format!("hgi_{}", "SECRET".repeat(8));
        let message = parse_invite_link(&secret).unwrap_err().message;
        assert!(!message.contains("SECRET"));
        assert!(!message.contains("hgi_"));
    }
}
