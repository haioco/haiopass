use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable, machine-readable error codes. The frontend branches on these; the
/// human-readable message is only a fallback for the UI.
pub mod codes {
    pub const NETWORK: &str = "network";
    pub const TIMEOUT: &str = "timeout";
    pub const PROTOCOL: &str = "protocol";
    pub const SERVER: &str = "server";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const SESSION_EXPIRED: &str = "session_expired";
    pub const NOT_LOGGED_IN: &str = "not_logged_in";
    pub const SHAHKAR_REQUIRED: &str = "shahkar_required";
    pub const MOBILE_REQUIRED: &str = "mobile_required";
    pub const ORGANIZATION_REQUIRED: &str = "organization_required";
    pub const INVALID_INPUT: &str = "invalid_input";
    pub const NOT_FOUND: &str = "not_found";
    pub const PAYMENT_URL_INVALID: &str = "payment_url_invalid";
    pub const PENDING_EXISTS: &str = "pending_exists";
    pub const KEYRING_UNAVAILABLE: &str = "keyring_unavailable";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const OPEN_FAILED: &str = "open_failed";
    pub const STORAGE: &str = "storage";

    /* Device-lock / invite codes (server `code` values, passed through so the
     * frontend can map them to the copy in the plan §15.3). */
    pub const INVITE_NOT_FOUND: &str = "invite_not_found";
    pub const INVITE_EXPIRED: &str = "invite_expired";
    pub const INVITE_USED: &str = "invite_used";
    pub const INVITE_REVOKED: &str = "invite_revoked";
    pub const MEMBER_REVOKED: &str = "member_revoked";
    pub const MEMBER_NOT_FOUND: &str = "member_not_found";
    pub const MEMBER_NOT_MANAGEABLE: &str = "not_manageable";
    pub const SLOT_TAKEN: &str = "slot_taken";
    pub const SLOTS_FULL: &str = "slots_full";
    pub const LEGACY_UNLIMITED: &str = "legacy_unlimited";
    pub const CSR_INVALID: &str = "csr_invalid";
    pub const DEVICE_ALREADY_BOUND: &str = "device_already_bound";
    pub const CA_UNAVAILABLE: &str = "ca_unavailable";
    pub const PROXY_ERROR: &str = "proxy_error";
    pub const ENROLLMENT_DISABLED: &str = "enrollment_disabled";
    pub const DEVICE_ENROLLMENT_REQUIRED: &str = "device_enrollment_required";
}

/// Serializable error returned by every `portal_*` command.
///
/// Tokens, national codes, passwords and payment URLs must never be placed in
/// `message`, `fields` or `retry_after`'s neighbours. Only the short provider
/// code and a user-safe message cross the IPC boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalError {
    pub code: String,
    pub message: String,
    /// Seconds until the caller may retry (429 / lockout).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
    /// DRF field errors, e.g. `{"package_id": ["Invalid"]}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<serde_json::Value>,
    /// HTTP status, when the failure came from the portal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

impl PortalError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retry_after: None,
            fields: None,
            status: None,
        }
    }

    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(seconds);
        self
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_fields(mut self, fields: serde_json::Value) -> Self {
        self.fields = Some(fields);
        self
    }

    pub fn network() -> Self {
        Self::new(
            codes::NETWORK,
            "اتصال به سرور برقرار نشد. اینترنت خود را بررسی کنید.",
        )
    }

    pub fn timeout() -> Self {
        Self::new(codes::TIMEOUT, "پاسخی از سرور دریافت نشد. دوباره تلاش کنید.")
    }

    pub fn protocol() -> Self {
        Self::new(codes::PROTOCOL, "پاسخ سرور قابل خواندن نبود.")
    }

    pub fn server(status: u16) -> Self {
        Self::new(
            codes::SERVER,
            "خطای موقت سرور. دوباره تلاش کنید.",
        )
        .with_status(status)
    }

    pub fn not_logged_in() -> Self {
        Self::new(codes::NOT_LOGGED_IN, "برای ادامه وارد حساب خود شوید.")
    }

    pub fn session_expired() -> Self {
        Self::new(
            codes::SESSION_EXPIRED,
            "نشست شما منقضی شده است. دوباره وارد شوید.",
        )
    }

    pub fn shahkar_required() -> Self {
        Self::new(
            codes::SHAHKAR_REQUIRED,
            "برای ادامه، احراز هویت شاهکار لازم است.",
        )
        .with_status(403)
    }

    pub fn organization_required() -> Self {
        Self::new(
            codes::ORGANIZATION_REQUIRED,
            "حساب سازمانی مورد نظر را انتخاب کنید.",
        )
        .with_status(400)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_INPUT, message)
    }

    pub fn payment_url_invalid() -> Self {
        Self::new(codes::PAYMENT_URL_INVALID, "آدرس درگاه پرداخت معتبر نیست.")
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::new(codes::STORAGE, message)
    }
}

impl fmt::Display for PortalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PortalError {}

impl From<reqwest::Error> for PortalError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            PortalError::timeout()
        } else if e.is_connect() || e.is_request() {
            PortalError::network()
        } else if e.is_decode() {
            PortalError::protocol()
        } else {
            PortalError::new(codes::NETWORK, "خطای شبکه رخ داد.")
        }
    }
}

impl From<serde_json::Error> for PortalError {
    fn from(_: serde_json::Error) -> Self {
        PortalError::protocol()
    }
}

/// Parse a Django REST Framework error envelope into a typed error.
///
/// Body shapes handled:
/// - `{"detail": "...", "code": "..."}`
/// - `{"field": ["msg", ...]}`
/// - `{"non_field_errors": ["msg"]}` / `["msg"]`
/// - plain text / HTML (mapped to a status-only error)
pub fn parse_drf_error(status: u16, retry_after: Option<u64>, body: &str) -> PortalError {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();

    let mut code_hint: Option<String> = None;
    let mut detail: Option<String> = None;
    let mut fields: Option<serde_json::Value> = None;

    if let Some(serde_json::Value::Object(map)) = parsed.as_ref() {
        if let Some(value) = map.get("code").and_then(|v| v.as_str()) {
            code_hint = Some(value.to_string());
        }
        if let Some(value) = map.get("detail").and_then(|v| v.as_str()) {
            detail = Some(value.to_string());
        }
        // Field errors: any other array-of-strings value.
        let mut field_map = serde_json::Map::new();
        for (key, value) in map {
            if key == "detail" || key == "code" {
                continue;
            }
            if value.is_array() || value.is_string() {
                field_map.insert(key.clone(), value.clone());
            }
        }
        if !field_map.is_empty() {
            fields = Some(serde_json::Value::Object(field_map));
        }
    }

    // Status-first mapping, then refine with an explicit provider code.
    let mut error = match status {
        400 => PortalError::new(codes::INVALID_INPUT, "درخواست معتبر نیست.").with_status(400),
        401 => PortalError::session_expired(),
        403 => PortalError::shahkar_required(),
        404 => PortalError::new(codes::NOT_FOUND, "موردی پیدا نشد.").with_status(404),
        409 => PortalError::new(codes::PENDING_EXISTS, "یک عملیات در جریان است.")
            .with_status(409),
        429 => {
            let seconds = retry_after.unwrap_or(60);
            PortalError::new(
                codes::RATE_LIMITED,
                "تعداد تلاش‌ها بیش از حد مجاز است. کمی بعد دوباره تلاش کنید.",
            )
            .with_retry_after(seconds)
            .with_status(429)
        }
        s if (500..600).contains(&s) => PortalError::server(s),
        _ => PortalError::new(codes::PROTOCOL, "درخواست ناموفق بود.").with_status(status),
    };

    if let Some(hint) = code_hint.as_deref() {
        match hint {
            "shahkar_required" | "shahkar_not_verified" => error = PortalError::shahkar_required(),
            "mobile_required" | "mobile_missing" => error = PortalError::new(
                codes::MOBILE_REQUIRED,
                "برای خرید، شماره موبایل معتبر روی حساب لازم است.",
            )
            .with_status(status),
            "organization_required" | "multiple_organizations" => {
                error = PortalError::organization_required()
            }
            "rate_limited" | "throttled" => {
                let seconds = retry_after.unwrap_or(60);
                error = PortalError::new(
                    codes::RATE_LIMITED,
                    "تعداد تلاش‌ها بیش از حد مجاز است. کمی بعد دوباره تلاش کنید.",
                )
                .with_retry_after(seconds)
                .with_status(status);
            }
            "invite_not_found" => {
                error = PortalError::new(codes::INVITE_NOT_FOUND, "لینک دعوت پیدا نشد.")
                    .with_status(status)
            }
            "invite_expired" => {
                error = PortalError::new(
                    codes::INVITE_EXPIRED,
                    "این لینک منقضی شده است. از صاحب کانفیگ لینک جدید بگیرید.",
                )
                .with_status(status)
            }
            "invite_used" => {
                error = PortalError::new(
                    codes::INVITE_USED,
                    "این لینک قبلاً استفاده شده است. برای دستگاه جدید لینک تازه بگیرید.",
                )
                .with_status(status)
            }
            "invite_revoked" => {
                error = PortalError::new(codes::INVITE_REVOKED, "این لینک لغو شده است.")
                    .with_status(status)
            }
            "member_revoked" => {
                error = PortalError::new(codes::MEMBER_REVOKED, "این کاربر حذف شده است.")
                    .with_status(status)
            }
            "member_not_found" => {
                error = PortalError::new(codes::MEMBER_NOT_FOUND, "کاربر پیدا نشد.")
                    .with_status(status)
            }
            "not_manageable" => {
                error = PortalError::new(
                    codes::MEMBER_NOT_MANAGEABLE,
                    "این اکانت در وضعیت فعلی قابل مدیریت نیست.",
                )
                .with_status(status)
            }
            "slot_taken" | "slots_full" => {
                error = PortalError::new(codes::SLOTS_FULL, "ظرفیت این پلن تکمیل است.")
                    .with_status(status)
            }
            "legacy_unlimited" => {
                error = PortalError::new(
                    codes::LEGACY_UNLIMITED,
                    "این اکانت قدیمی است و کاربر اضافه نمی‌پذیرد.",
                )
                .with_status(status)
            }
            "csr_invalid" | "invalid_request_id" => {
                error = PortalError::new(
                    codes::CSR_INVALID,
                    "ساخت کلید امن ناموفق بود؛ دوباره تلاش کنید.",
                )
                .with_status(status)
            }
            "device_already_bound" => {
                error = PortalError::new(
                    codes::DEVICE_ALREADY_BOUND,
                    "این کاربر روی دستگاه دیگری ثبت شده است. ابتدا دستگاه فعلی را حذف کنید.",
                )
                .with_status(status)
            }
            "ca_unavailable" => {
                error = PortalError::new(
                    codes::CA_UNAVAILABLE,
                    "ثبت دستگاه موقتاً در دسترس نیست؛ بعداً تلاش کنید.",
                )
                .with_status(status)
            }
            "proxy_error" => {
                error = PortalError::new(codes::PROXY_ERROR, "خطای موقت سرور. دوباره تلاش کنید.")
                    .with_status(status)
            }
            "enrollment_disabled" => {
                error = PortalError::new(
                    codes::ENROLLMENT_DISABLED,
                    "ثبت دستگاه در حال حاضر فعال نیست.",
                )
                .with_status(status)
            }
            _ => {}
        }
    }

    if let Some(detail) = detail {
        // Keep the local, code-specific copy for device/invite errors: the
        // server detail is English and may mention internal states. Other
        // errors fall back to the (short, sanitized) provider detail.
        let keep_local = matches!(
            error.code.as_str(),
            codes::INVITE_NOT_FOUND
                | codes::INVITE_EXPIRED
                | codes::INVITE_USED
                | codes::INVITE_REVOKED
                | codes::MEMBER_REVOKED
                | codes::MEMBER_NOT_FOUND
                | codes::MEMBER_NOT_MANAGEABLE
                | codes::SLOTS_FULL
                | codes::LEGACY_UNLIMITED
                | codes::CSR_INVALID
                | codes::DEVICE_ALREADY_BOUND
                | codes::CA_UNAVAILABLE
                | codes::PROXY_ERROR
                | codes::ENROLLMENT_DISABLED
        );
        if !keep_local && !detail.is_empty() && detail.len() <= 200 && !detail.contains("Traceback")
        {
            error.message = detail;
        }
    }
    if let Some(fields) = fields {
        error = error.with_fields(fields);
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_field_errors() {
        let err = parse_drf_error(400, None, r#"{"package_id":["Invalid package"]}"#);
        assert_eq!(err.code, codes::INVALID_INPUT);
        assert!(err.fields.is_some());
    }

    #[test]
    fn maps_unauthorized() {
        let err = parse_drf_error(401, None, r#"{"detail":"Authentication credentials were not provided."}"#);
        assert_eq!(err.code, codes::SESSION_EXPIRED);
    }

    #[test]
    fn maps_shahkar_required_code() {
        let err = parse_drf_error(403, None, r#"{"detail":"need shahkar","code":"shahkar_required"}"#);
        assert_eq!(err.code, codes::SHAHKAR_REQUIRED);
    }

    #[test]
    fn maps_organization_required() {
        let err = parse_drf_error(
            400,
            None,
            r#"{"detail":"pick an org","code":"organization_required"}"#,
        );
        assert_eq!(err.code, codes::ORGANIZATION_REQUIRED);
    }

    #[test]
    fn maps_rate_limit_with_retry_after() {
        let err = parse_drf_error(429, Some(42), "{}");
        assert_eq!(err.code, codes::RATE_LIMITED);
        assert_eq!(err.retry_after, Some(42));
    }

    #[test]
    fn maps_server_errors() {
        let err = parse_drf_error(503, None, "upstream unavailable");
        assert_eq!(err.code, codes::SERVER);
        assert_eq!(err.status, Some(503));
    }

    #[test]
    fn never_leaks_traceback() {
        let err = parse_drf_error(500, None, "Traceback (most recent call last): ...");
        assert_eq!(err.code, codes::SERVER);
        assert!(!err.message.contains("Traceback"));
    }

    #[test]
    fn maps_invite_codes_and_keeps_local_copy() {
        let err = parse_drf_error(
            410,
            None,
            r#"{"detail":"This invite has expired.","code":"invite_expired"}"#,
        );
        assert_eq!(err.code, codes::INVITE_EXPIRED);
        assert!(err.message.contains("منقضی"));
        assert!(!err.message.contains("This invite"));

        let used = parse_drf_error(409, None, r#"{"code":"invite_used"}"#);
        assert_eq!(used.code, codes::INVITE_USED);

        let revoked = parse_drf_error(410, None, r#"{"code":"invite_revoked"}"#);
        assert_eq!(revoked.code, codes::INVITE_REVOKED);

        let not_found = parse_drf_error(404, None, r#"{"code":"invite_not_found"}"#);
        assert_eq!(not_found.code, codes::INVITE_NOT_FOUND);
    }

    #[test]
    fn maps_enrollment_failures() {
        assert_eq!(
            parse_drf_error(400, None, r#"{"code":"csr_invalid"}"#).code,
            codes::CSR_INVALID
        );
        assert_eq!(
            parse_drf_error(409, None, r#"{"code":"device_already_bound"}"#).code,
            codes::DEVICE_ALREADY_BOUND
        );
        assert_eq!(
            parse_drf_error(503, None, r#"{"code":"ca_unavailable"}"#).code,
            codes::CA_UNAVAILABLE
        );
        assert_eq!(
            parse_drf_error(409, None, r#"{"code":"slot_taken"}"#).code,
            codes::SLOTS_FULL
        );
        assert_eq!(
            parse_drf_error(409, None, r#"{"code":"member_revoked"}"#).code,
            codes::MEMBER_REVOKED
        );
    }
}
