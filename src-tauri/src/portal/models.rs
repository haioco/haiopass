//! Portal API DTOs and sanitized UI views.
//!
//! API structs are tolerant of missing/aliased fields so a partially deployed
//! backend does not break the whole screen. UI structs are camelCase and omit
//! tokens, national ids, raw meta-data and provider payloads.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/* ------------------------------------------------------------------ */
/* Tolerant scalar deserializers                                      */
/* ------------------------------------------------------------------ */

fn as_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub fn de_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(as_string(&value).unwrap_or_default())
}

pub fn de_opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(as_string(&value).filter(|s| !s.is_empty()))
}

pub fn de_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match &value {
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
        serde_json::Value::String(s) => s.parse().unwrap_or(0),
        serde_json::Value::Bool(_) => 0,
        _ => 0,
    })
}

pub fn de_opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match &value {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

pub fn de_f64<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match &value {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

pub fn de_u32<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match &value {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) as u32,
        serde_json::Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    })
}

pub fn de_opt_u32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match &value {
        serde_json::Value::Number(n) => n.as_u64().map(|v| v as u32),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

pub fn de_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match &value {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        serde_json::Value::String(s) => matches!(s.as_str(), "1" | "true" | "True"),
        _ => false,
    })
}

pub fn de_opt_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match &value {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Number(n) => Some(n.as_i64().unwrap_or(0) != 0),
        serde_json::Value::String(s) => match s.as_str() {
            "1" | "true" | "True" => Some(true),
            "0" | "false" | "False" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

/* ------------------------------------------------------------------ */
/* API models                                                         */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PortalUser {
    #[serde(default, deserialize_with = "de_opt_string")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub username: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub email: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub first_name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub last_name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub role: String,
    #[serde(default, deserialize_with = "de_string")]
    pub display_name: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_verified: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub onboarding_completed: bool,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub mobile: Option<String>,
    #[serde(default, deserialize_with = "de_u32")]
    pub shahkar_status: u32,
    #[serde(default, deserialize_with = "de_string")]
    pub created_at: String,
    // Present in the API, deliberately never copied into a UI view.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub national_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthTokens {
    #[serde(default, deserialize_with = "de_string")]
    pub access: String,
    #[serde(default, deserialize_with = "de_string")]
    pub refresh: String,
    #[serde(default)]
    pub user: Option<PortalUser>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OtpRequest {
    #[serde(default, deserialize_with = "de_bool")]
    pub sent: bool,
    #[serde(default, deserialize_with = "de_opt_u32")]
    pub expires_in: Option<u32>,
    #[serde(default, deserialize_with = "de_opt_u32")]
    pub retry_after: Option<u32>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ShahkarResponse {
    #[serde(default, deserialize_with = "de_bool")]
    pub verified: bool,
    #[serde(default, deserialize_with = "de_u32")]
    pub shahkar_status: u32,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub reason: Option<String>,
    #[serde(default)]
    pub user: Option<PortalUser>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackageApi {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub slug: String,
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub price: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub traffic_bytes: i64,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_free: bool,
    #[serde(default, deserialize_with = "de_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_u32")]
    pub sort_order: u32,
    #[serde(default, deserialize_with = "de_opt_u32")]
    pub included_users: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReserveApi {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub package: String,
    #[serde(default, deserialize_with = "de_string")]
    pub package_name: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub traffic_bytes: i64,
    #[serde(default, deserialize_with = "de_string")]
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemberDeviceApi {
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub platform: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub fingerprint_prefix: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub bound_at: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub last_seen_at: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub cert_expires_at: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub status: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemberApi {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub display_name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub username: String,
    #[serde(default, deserialize_with = "de_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_string")]
    pub status_display: String,
    #[serde(default, deserialize_with = "de_u32")]
    pub member_index: u32,
    #[serde(default, deserialize_with = "de_bool")]
    pub device_lock: bool,
    #[serde(default)]
    pub device: Option<MemberDeviceApi>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub activated_at: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub revoked_at: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub mobile: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub email: Option<String>,
    // Only returned on create/rotate (and only to the owner).
    #[serde(default, deserialize_with = "de_opt_string")]
    pub password: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub trojan_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountApi {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub package: String,
    #[serde(default, deserialize_with = "de_string")]
    pub package_name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub username: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub total_traffic_bytes: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub usage_bytes: i64,
    #[serde(default, deserialize_with = "de_string")]
    pub usage_display: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub traffic_is_over: bool,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub expired_at: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_string")]
    pub status_display: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub last_sync: Option<String>,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_legacy: bool,
    #[serde(default, deserialize_with = "de_u32")]
    pub pending_reserves_count: u32,
    #[serde(default, deserialize_with = "de_string")]
    pub created_at: String,
    #[serde(default, deserialize_with = "de_string")]
    pub updated_at: String,
    // Detail fields
    #[serde(default, deserialize_with = "de_opt_string")]
    pub password: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub trojan_url: Option<String>,
    #[serde(default, deserialize_with = "de_f64")]
    pub usage_percent: f64,
    #[serde(default)]
    pub pending_reserves: Vec<ReserveApi>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub projected_expired_at: Option<String>,
    // Multi-user fields
    #[serde(default, deserialize_with = "de_opt_u32")]
    pub included_users: Option<u32>,
    #[serde(default, deserialize_with = "de_opt_bool")]
    pub is_legacy_unlimited: Option<bool>,
    #[serde(default, deserialize_with = "de_u32")]
    pub active_members_count: u32,
    #[serde(default, deserialize_with = "de_opt_u32")]
    pub available_slots: Option<u32>,
    #[serde(default, deserialize_with = "de_bool")]
    pub device_lock: bool,
    #[serde(default, deserialize_with = "de_string")]
    pub device_name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub device_platform: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub device_bound_at: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub device_cert_expires_at: Option<String>,
    #[serde(default)]
    pub members: Vec<MemberApi>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlanRefApi {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub slug: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InvoiceApi {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub organization: Option<String>,
    #[serde(default)]
    pub plan: Option<PlanRefApi>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub subscription: Option<String>,
    #[serde(default, deserialize_with = "de_i64")]
    pub amount: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub original_amount: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub discount_amount: i64,
    #[serde(default, deserialize_with = "de_f64")]
    pub discount_percent: f64,
    #[serde(default, deserialize_with = "de_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub due_date: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub paid_at: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    #[serde(default, deserialize_with = "de_string")]
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BuyApi {
    #[serde(default, deserialize_with = "de_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub payment_url: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub payment_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub invoice_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub subscription_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub credit_applied: Option<i64>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub paid_by: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InvoicePayApi {
    #[serde(default, deserialize_with = "de_string")]
    pub payment_url: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub payment_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub invoice_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OrganizationApi {    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub slug: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub owner: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub owner_email: String,
    #[serde(default, deserialize_with = "de_string")]
    pub my_role: String,
    #[serde(default, deserialize_with = "de_string")]
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PurchaseIntentApi {
    #[serde(default, deserialize_with = "de_string")]
    pub plan_id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub package_id: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_free: bool,
}

/* ------------------------------------------------------------------ */
/* Device enrollment / invites (device-lock programme)                */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InvitePreviewApi {
    #[serde(default, deserialize_with = "de_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_string")]
    pub account_name_masked: String,
    #[serde(default, deserialize_with = "de_string")]
    pub package_name: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub traffic_bytes: i64,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "de_u32")]
    pub member_index: u32,
    #[serde(default, deserialize_with = "de_u32")]
    pub slots_total: u32,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub account_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub member_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InviteApi {
    #[serde(default, deserialize_with = "de_opt_string")]
    pub token: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub invite_url: String,
    #[serde(default, deserialize_with = "de_string")]
    pub deep_link: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub member_id: Option<String>,
    #[serde(default, deserialize_with = "de_u32")]
    pub member_index: u32,
    #[serde(default, deserialize_with = "de_string")]
    pub package_name: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PendingInviteApi {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub member: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub member_name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_string")]
    pub token_prefix: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub consumed_at: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub device_name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub platform: String,
    #[serde(default, deserialize_with = "de_string")]
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EnrollmentCredentialApi {
    #[serde(default, deserialize_with = "de_string")]
    pub server: String,
    #[serde(default, deserialize_with = "de_u32")]
    pub port: u32,
    #[serde(default, deserialize_with = "de_string")]
    pub sni: String,
    #[serde(default, deserialize_with = "de_string")]
    pub password: String,
    #[serde(default, deserialize_with = "de_string")]
    pub username: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EnrollmentPackageApi {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub traffic_bytes: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EnrollmentApi {
    #[serde(default, deserialize_with = "de_string")]
    pub certificate: String,
    #[serde(default, deserialize_with = "de_string")]
    pub ca_chain: String,
    #[serde(default, deserialize_with = "de_string")]
    pub trojan_url: String,
    #[serde(default)]
    pub credential: Option<EnrollmentCredentialApi>,
    #[serde(default)]
    pub package: Option<EnrollmentPackageApi>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub account_id: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub member_id: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub device_fingerprint: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub cert_expires_at: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemberCreateApi {
    #[serde(default)]
    pub member: Option<MemberApi>,
    #[serde(default, deserialize_with = "de_bool")]
    pub invite_required: bool,
    #[serde(default)]
    pub invite: Option<InviteApi>,
    // Legacy (flag-off) response is a bare MemberCredentialSerializer; handled
    // by the caller, which falls back to parsing `MemberApi` directly.
}

/* ------------------------------------------------------------------ */
/* UI views (camelCase, sanitized)                                    */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserView {
    pub id: String,
    pub username: String,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub role: String,
    pub display_name: String,
    pub is_verified: bool,
    pub onboarding_completed: bool,
    pub mobile: Option<String>,
    pub shahkar_status: u32,
    pub shahkar_verified: bool,
    pub created_at: String,
}

impl UserView {
    pub fn from_api(user: &PortalUser) -> Self {
        let display = if !user.display_name.is_empty() {
            user.display_name.clone()
        } else {
            let joined = format!("{} {}", user.first_name.trim(), user.last_name.trim());
            if joined.trim().is_empty() {
                user.mobile.clone().unwrap_or_else(|| user.username.clone().unwrap_or_default())
            } else {
                joined.trim().to_string()
            }
        };
        Self {
            id: user.id.clone().unwrap_or_default(),
            username: user.username.clone().unwrap_or_default(),
            email: user.email.clone().unwrap_or_default(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            role: user.role.clone(),
            display_name: display,
            is_verified: user.is_verified,
            onboarding_completed: user.onboarding_completed,
            mobile: user.mobile.clone(),
            shahkar_status: user.shahkar_status,
            shahkar_verified: user.shahkar_status == 1,
            created_at: user.created_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageView {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub price: i64,
    pub traffic_bytes: i64,
    pub is_free: bool,
    pub status: String,
    pub sort_order: u32,
    pub included_users: u32,
}

impl PackageView {
    pub fn from_api(pkg: &PackageApi) -> Self {
        Self {
            id: pkg.id.clone(),
            name: pkg.name.clone(),
            slug: pkg.slug.clone(),
            description: pkg.description.clone(),
            price: pkg.price,
            traffic_bytes: pkg.traffic_bytes,
            is_free: pkg.is_free,
            status: pkg.status.clone(),
            sort_order: pkg.sort_order,
            included_users: pkg.included_users.unwrap_or(1).max(1),
        }
    }

    pub fn is_active(&self) -> bool {
        self.status.is_empty() || self.status == "active"
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub package: String,
    pub package_name: String,
    pub username: String,
    pub total_traffic_bytes: i64,
    pub usage_bytes: i64,
    pub usage_display: String,
    pub traffic_is_over: bool,
    pub expired_at: Option<String>,
    pub status: String,
    pub status_display: String,
    pub last_sync: Option<String>,
    pub is_legacy: bool,
    pub pending_reserves_count: u32,
    pub created_at: String,
    pub updated_at: String,
    /// Rust-computed grouping: `active` | `expired` | `support`.
    pub category: String,
    pub included_users: Option<u32>,
    pub is_legacy_unlimited: bool,
    pub active_members_count: u32,
    pub available_slots: Option<u32>,
    /// Device-lock programme: locked accounts are certificate-only.
    pub device_lock: bool,
    pub device_name: String,
    pub device_platform: String,
    pub device_bound_at: Option<String>,
    pub device_cert_expires_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReserveView {
    pub id: String,
    pub package: String,
    pub package_name: String,
    pub traffic_bytes: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDeviceView {
    pub name: String,
    pub platform: String,
    pub fingerprint_prefix: Option<String>,
    pub bound_at: Option<String>,
    pub last_seen_at: Option<String>,
    pub cert_expires_at: Option<String>,
    pub status: String,
}

impl MemberDeviceView {
    pub fn from_api(device: &MemberDeviceApi) -> Self {
        Self {
            name: device.name.clone(),
            platform: device.platform.clone(),
            fingerprint_prefix: device.fingerprint_prefix.clone(),
            bound_at: device.bound_at.clone(),
            last_seen_at: device.last_seen_at.clone(),
            cert_expires_at: device.cert_expires_at.clone(),
            status: device.status.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub id: String,
    pub display_name: String,
    pub username: String,
    pub status: String,
    pub status_display: String,
    pub member_index: u32,
    pub device_lock: bool,
    pub device: Option<MemberDeviceView>,
    pub activated_at: Option<String>,
    pub revoked_at: Option<String>,
    pub mobile: Option<String>,
    pub email: Option<String>,
    /// Only present on create/rotate responses; never logged. Device-lock
    /// members never receive one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trojan_url: Option<String>,
}

impl MemberView {
    pub fn from_api(member: &MemberApi, include_credential: bool) -> Self {
        Self {
            id: member.id.clone(),
            display_name: member.display_name.clone(),
            username: member.username.clone(),
            status: member.status.clone(),
            status_display: member.status_display.clone(),
            member_index: member.member_index,
            device_lock: member.device_lock,
            device: member.device.as_ref().map(MemberDeviceView::from_api),
            activated_at: member.activated_at.clone(),
            revoked_at: member.revoked_at.clone(),
            mobile: member.mobile.clone(),
            email: member.email.clone(),
            password: if include_credential {
                member.password.clone()
            } else {
                None
            },
            trojan_url: if include_credential {
                member.trojan_url.clone()
            } else {
                None
            },
        }
    }
}

/// Result of `POST members/` on both contract versions:
/// - device-lock: `{member, invite_required, invite?}` (no password/url)
/// - legacy: `MemberCredentialSerializer`
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberCreateView {
    pub member: MemberView,
    pub invite_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite: Option<InviteView>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDetailView {
    #[serde(flatten)]
    pub base: AccountView,
    pub password: String,
    pub trojan_url: String,
    pub usage_percent: f64,
    pub pending_reserves: Vec<ReserveView>,
    pub projected_expired_at: Option<String>,
    pub members: Vec<MemberView>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceView {
    pub id: String,
    pub organization: Option<String>,
    pub plan: Option<PlanRefView>,
    pub subscription: Option<String>,
    pub amount: i64,
    pub original_amount: i64,
    pub discount_amount: i64,
    pub discount_percent: f64,
    pub status: String,
    pub due_date: Option<String>,
    pub paid_at: Option<String>,
    pub description: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRefView {
    pub id: String,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationView {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub owner: Option<String>,
    pub owner_email: String,
    pub my_role: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuyResponse {
    pub status: String,
    pub payment_url: Option<String>,
    pub payment_id: Option<String>,
    pub invoice_id: Option<String>,
    pub subscription_id: Option<String>,
    pub credit_applied: Option<i64>,
    pub paid_by: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpRequestView {
    pub sent: bool,
    pub expires_in: Option<u32>,
    pub retry_after: Option<u32>,
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShahkarView {
    pub verified: bool,
    pub shahkar_status: u32,
    pub user: Option<UserView>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoicePayView {
    pub payment_url: String,
    pub payment_id: Option<String>,
    pub invoice_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub logged_in: bool,
    pub user: Option<UserView>,
    pub persistent: bool,
    pub restoring: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseIntentView {
    pub plan_id: String,
    pub package_id: String,
    pub is_free: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitePreviewView {
    pub status: String,
    pub account_name_masked: String,
    pub package_name: String,
    pub traffic_bytes: i64,
    pub traffic_display: String,
    pub expires_at: Option<String>,
    pub member_index: u32,
    pub slots_total: u32,
    pub account_id: String,
    pub member_id: Option<String>,
}

impl InvitePreviewView {
    pub fn from_api(preview: &InvitePreviewApi) -> Self {
        Self {
            status: preview.status.clone(),
            account_name_masked: preview.account_name_masked.clone(),
            package_name: preview.package_name.clone(),
            traffic_bytes: preview.traffic_bytes,
            traffic_display: format_bytes(preview.traffic_bytes),
            expires_at: preview.expires_at.clone(),
            member_index: preview.member_index,
            slots_total: preview.slots_total.max(1),
            account_id: preview.account_id.clone().unwrap_or_default(),
            member_id: preview.member_id.clone(),
        }
    }
}

/// Invite shown to the owner after it is minted. `invite_url`/`deep_link`
/// contain the one-time token and are the only fields the UI may copy/QR.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteView {
    pub invite_url: String,
    pub deep_link: String,
    pub expires_at: Option<String>,
    pub member_id: Option<String>,
    pub member_index: u32,
    pub package_name: String,
    pub account_id: String,
}

impl InviteView {
    pub fn from_api(invite: &InviteApi) -> Self {
        Self {
            invite_url: invite.invite_url.clone(),
            deep_link: invite.deep_link.clone(),
            expires_at: invite.expires_at.clone(),
            member_id: invite.member_id.clone(),
            member_index: invite.member_index,
            package_name: invite.package_name.clone(),
            account_id: invite.account_id.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInviteView {
    pub id: String,
    pub member_id: Option<String>,
    pub member_name: String,
    pub status: String,
    pub token_prefix: String,
    pub expires_at: Option<String>,
    pub consumed_at: Option<String>,
    pub device_name: String,
    pub platform: String,
    pub created_at: String,
}

impl From<&PendingInviteApi> for PendingInviteView {
    fn from(invite: &PendingInviteApi) -> Self {
        Self {
            id: invite.id.clone(),
            member_id: invite.member.clone(),
            member_name: invite.member_name.clone(),
            status: invite.status.clone(),
            token_prefix: invite.token_prefix.clone(),
            expires_at: invite.expires_at.clone(),
            consumed_at: invite.consumed_at.clone(),
            device_name: invite.device_name.clone(),
            platform: invite.platform.clone(),
            created_at: invite.created_at.clone(),
        }
    }
}

/// UI-safe enrollment result. Deliberately omits the generated password and
/// the `trojan://` link: a locked credential cannot be used without its
/// certificate, so showing a bearer link would be misleading.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentResultView {
    pub account_id: String,
    pub member_id: Option<String>,
    pub identity_id: String,
    pub device_fingerprint: String,
    pub fingerprint_prefix: String,
    pub cert_expires_at: Option<String>,
    pub expires_at: Option<String>,
    pub package_name: String,
    pub traffic_bytes: i64,
    pub traffic_display: String,
    pub username: String,
    pub server: String,
    pub port: u32,
    pub sni: String,
    pub device_name: String,
    pub platform: String,
}

impl EnrollmentResultView {
    pub fn from_api(result: &EnrollmentApi, identity_id: &str) -> Self {
        let credential = result.credential.clone().unwrap_or_default();
        let package = result.package.clone().unwrap_or_default();
        let package_name = if package.name.is_empty() {
            String::new()
        } else {
            package.name.clone()
        };
        Self {
            account_id: result.account_id.clone(),
            member_id: result.member_id.clone(),
            identity_id: identity_id.to_string(),
            device_fingerprint: result.device_fingerprint.clone(),
            fingerprint_prefix: result.device_fingerprint.chars().take(16).collect(),
            cert_expires_at: result.cert_expires_at.clone(),
            expires_at: result.expires_at.clone(),
            package_name,
            traffic_bytes: package.traffic_bytes,
            traffic_display: format_bytes(package.traffic_bytes),
            username: credential.username.clone(),
            server: credential.server.clone(),
            port: credential.port,
            sni: credential.sni.clone(),
            device_name: String::new(),
            platform: String::new(),
        }
    }
}

/// Sanitized local identity for the settings/home badge.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentityView {
    pub identity_id: String,
    pub account_id: Option<String>,
    pub member_id: Option<String>,
    pub fingerprint_prefix: String,
    pub cert_expires_at: Option<String>,
    pub device_name: String,
    pub platform: String,
    pub keyring_persistent: bool,
}

impl DeviceIdentityView {
    pub fn from_certificate(
        cert: &crate::device::DeviceCertificate,
        persistent: bool,
    ) -> Self {
        Self {
            identity_id: cert.identity_id.clone(),
            account_id: cert.account_id.clone(),
            member_id: cert.member_id.clone(),
            fingerprint_prefix: cert.fingerprint_prefix(),
            cert_expires_at: cert.cert_expires_at.clone(),
            device_name: cert.device_name.clone(),
            platform: cert.platform.clone(),
            keyring_persistent: persistent,
        }
    }
}

/* ------------------------------------------------------------------ */
/* Classification helpers                                             */
/* ------------------------------------------------------------------ */

pub fn parse_ts(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&Utc));
    }
    // Naive timestamps from DRF without a timezone: treat as UTC.
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, format) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?));
    }
    None
}

pub fn classify(
    status: &str,
    expired_at: Option<&str>,
    traffic_is_over: bool,
    now: DateTime<Utc>,
) -> &'static str {
    match status {
        "suspended" | "blocked" | "disabled" => return "support",
        _ => {}
    }
    if matches!(status, "expired" | "quota_exhausted" | "ended" | "canceled" | "cancelled") {
        return "expired";
    }
    if traffic_is_over {
        return "expired";
    }
    if let Some(expired) = expired_at.and_then(parse_ts) {
        if expired <= now {
            return "expired";
        }
    }
    if status == "active" || status.is_empty() {
        "active"
    } else {
        "support"
    }
}

pub fn format_bytes(bytes: i64) -> String {
    if bytes <= 0 {
        return "0 GB".to_string();
    }
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.0} MB", value / MB)
    } else {
        format!("{:.0} KB", value / 1024.0)
    }
}

impl AccountView {
    pub fn from_api(account: &AccountApi, now: DateTime<Utc>) -> Self {
        let category = classify(
            &account.status,
            account.expired_at.as_deref(),
            account.traffic_is_over,
            now,
        )
        .to_string();
        let legacy_unlimited = account
            .is_legacy_unlimited
            .unwrap_or(account.included_users.is_none());
        let included = account.included_users;
        let members = account.active_members_count;
        let available = account.available_slots.or_else(|| {
            included.map(|total| total.saturating_sub(1).saturating_sub(members))
        });
        let usage_display = if account.usage_display.is_empty() {
            format!(
                "{} / {}",
                format_bytes(account.usage_bytes),
                format_bytes(account.total_traffic_bytes)
            )
        } else {
            account.usage_display.clone()
        };
        let status_display = if account.status_display.is_empty() {
            match category.as_str() {
                "active" => "فعال".to_string(),
                "expired" => "منقضی شده".to_string(),
                _ => "نیازمند بررسی".to_string(),
            }
        } else {
            account.status_display.clone()
        };

        Self {
            id: account.id.clone(),
            package: account.package.clone(),
            package_name: account.package_name.clone(),
            username: account.username.clone(),
            total_traffic_bytes: account.total_traffic_bytes,
            usage_bytes: account.usage_bytes,
            usage_display,
            traffic_is_over: account.traffic_is_over,
            expired_at: account.expired_at.clone(),
            status: account.status.clone(),
            status_display,
            last_sync: account.last_sync.clone(),
            is_legacy: account.is_legacy,
            pending_reserves_count: account.pending_reserves_count,
            created_at: account.created_at.clone(),
            updated_at: account.updated_at.clone(),
            category,
            included_users: included,
            is_legacy_unlimited: legacy_unlimited,
            active_members_count: members,
            available_slots: available,
            device_lock: account.device_lock,
            device_name: account.device_name.clone(),
            device_platform: account.device_platform.clone(),
            device_bound_at: account.device_bound_at.clone(),
            device_cert_expires_at: account.device_cert_expires_at.clone(),
        }
    }
}

impl AccountDetailView {
    pub fn from_api(account: &AccountApi, now: DateTime<Utc>) -> Self {
        let base = AccountView::from_api(account, now);
        let usage_percent = if account.usage_percent > 0.0 {
            account.usage_percent
        } else if account.total_traffic_bytes > 0 {
            (account.usage_bytes as f64 / account.total_traffic_bytes as f64) * 100.0
        } else {
            0.0
        };

        Self {
            base,
            password: account.password.clone().unwrap_or_default(),
            trojan_url: account.trojan_url.clone().unwrap_or_default(),
            usage_percent: usage_percent.clamp(0.0, 100.0),
            pending_reserves: account
                .pending_reserves
                .iter()
                .map(|reserve| ReserveView {
                    id: reserve.id.clone(),
                    package: reserve.package.clone(),
                    package_name: reserve.package_name.clone(),
                    traffic_bytes: reserve.traffic_bytes,
                    created_at: reserve.created_at.clone(),
                })
                .collect(),
            projected_expired_at: account.projected_expired_at.clone(),
            members: account
                .members
                .iter()
                .map(|member| MemberView::from_api(member, false))
                .collect(),
        }
    }
}

impl InvoiceView {
    pub fn from_api(invoice: &InvoiceApi) -> Self {
        Self {
            id: invoice.id.clone(),
            organization: invoice.organization.clone(),
            plan: invoice.plan.as_ref().map(|plan| PlanRefView {
                id: plan.id.clone(),
                name: plan.name.clone(),
                slug: plan.slug.clone(),
            }),
            subscription: invoice.subscription.clone(),
            amount: invoice.amount,
            original_amount: invoice.original_amount,
            discount_amount: invoice.discount_amount,
            discount_percent: invoice.discount_percent,
            status: invoice.status.clone(),
            due_date: invoice.due_date.clone(),
            paid_at: invoice.paid_at.clone(),
            description: invoice.description.clone(),
            created_at: invoice.created_at.clone(),
        }
    }
}

impl From<&OrganizationApi> for OrganizationView {
    fn from(org: &OrganizationApi) -> Self {
        Self {
            id: org.id.clone(),
            name: org.name.clone(),
            slug: org.slug.clone(),
            owner: org.owner.clone(),
            owner_email: org.owner_email.clone(),
            my_role: org.my_role.clone(),
            created_at: org.created_at.clone(),
        }
    }
}

impl From<&BuyApi> for BuyResponse {
    fn from(buy: &BuyApi) -> Self {
        Self {
            status: buy.status.clone(),
            payment_url: buy.payment_url.clone(),
            payment_id: buy.payment_id.clone(),
            invoice_id: buy.invoice_id.clone(),
            subscription_id: buy.subscription_id.clone(),
            credit_applied: buy.credit_applied,
            paid_by: buy.paid_by.clone(),
        }
    }
}

impl From<&PurchaseIntentApi> for PurchaseIntentView {
    fn from(intent: &PurchaseIntentApi) -> Self {
        Self {
            plan_id: intent.plan_id.clone(),
            package_id: intent.package_id.clone(),
            is_free: intent.is_free,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap()
    }

    #[test]
    fn classifies_active() {
        assert_eq!(
            classify("active", Some("2026-12-15T00:00:00Z"), false, now()),
            "active"
        );
    }

    #[test]
    fn classifies_expired_status() {
        assert_eq!(classify("expired", Some("2027-01-01T00:00:00Z"), false, now()), "expired");
        assert_eq!(classify("quota_exhausted", None, false, now()), "expired");
    }

    #[test]
    fn classifies_expired_by_date() {
        assert_eq!(
            classify("active", Some("2026-01-01T00:00:00Z"), false, now()),
            "expired"
        );
    }

    #[test]
    fn classifies_expired_by_quota() {
        assert_eq!(classify("active", None, true, now()), "expired");
    }

    #[test]
    fn classifies_suspended_as_support() {
        assert_eq!(classify("suspended", None, false, now()), "support");
    }

    #[test]
    fn null_expiry_is_not_active_expiry() {
        assert_eq!(classify("active", None, false, now()), "active");
    }

    #[test]
    fn unknown_status_is_support() {
        assert_eq!(classify("mystery", None, false, now()), "support");
    }

    #[test]
    fn parses_naive_timestamps() {
        assert!(parse_ts("2026-11-15T12:00:00").is_some());
        assert!(parse_ts("2026-11-15 12:00:00").is_some());
        assert!(parse_ts("not-a-date").is_none());
    }

    #[test]
    fn view_omits_national_id_and_tokens() {
        let user = PortalUser {
            id: Some("7".into()),
            national_id: Some("0012345678".into()),
            ..Default::default()
        };
        // The struct has no extra fields; this test documents that the view
        // type simply does not carry the national id.
        let view = UserView::from_api(&user);
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("national"));
        assert!(!json.contains("0012345678"));
    }

    #[test]
    fn account_view_marks_legacy_unlimited() {
        let account = AccountApi {
            id: "acc".into(),
            status: "active".into(),
            included_users: None,
            ..Default::default()
        };
        let view = AccountView::from_api(&account, now());
        assert!(view.is_legacy_unlimited);
        assert!(view.available_slots.is_none());
    }

    #[test]
    fn account_view_computes_available_slots() {
        let account = AccountApi {
            id: "acc".into(),
            status: "active".into(),
            included_users: Some(5),
            active_members_count: 2,
            ..Default::default()
        };
        let view = AccountView::from_api(&account, now());
        assert_eq!(view.available_slots, Some(2)); // 5 - 1 primary - 2 members
    }

    #[test]
    fn detail_view_is_flat_for_the_ui() {
        let account = AccountApi {
            id: "acc".into(),
            package_name: "HaioGate 100GB".into(),
            status: "active".into(),
            password: Some("pw".into()),
            trojan_url: Some("trojan://pw@host:443#HaioGate".into()),
            included_users: Some(2),
            members: vec![MemberApi {
                id: "m1".into(),
                display_name: "Ali".into(),
                username: "u_m1".into(),
                status: "active".into(),
                password: Some("member-pw".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let detail = AccountDetailView::from_api(&account, now());
        let json = serde_json::to_value(&detail).unwrap();
        // JS reads `detail.packageName`, not `detail.base.packageName`.
        assert!(json.get("base").is_none());
        assert_eq!(json["packageName"], "HaioGate 100GB");
        assert_eq!(json["password"], "pw");
        // Credentials are only exposed on the explicit create/rotate response.
        assert!(json["members"][0].get("password").is_none());
    }

    #[test]
    fn created_member_exposes_credential_once() {
        let member = MemberApi {
            id: "m1".into(),
            username: "u_m1".into(),
            password: Some("member-pw".into()),
            trojan_url: Some("trojan://member-pw@host:443".into()),
            ..Default::default()
        };
        let view = MemberView::from_api(&member, true);
        assert_eq!(view.password.as_deref(), Some("member-pw"));
        assert!(view.trojan_url.is_some());
    }

    #[test]
    fn member_device_summary_is_exposed() {
        let member = MemberApi {
            id: "m1".into(),
            display_name: "Ali".into(),
            status: "active".into(),
            device_lock: true,
            device: Some(MemberDeviceApi {
                name: "Pixel 8".into(),
                platform: "android".into(),
                fingerprint_prefix: Some("a1b2c3d4".into()),
                cert_expires_at: Some("2028-11-14T14:30:00Z".into()),
                status: "active".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let view = MemberView::from_api(&member, false);
        let device = view.device.expect("device");
        assert_eq!(device.name, "Pixel 8");
        assert_eq!(device.platform, "android");
        assert!(view.device_lock);
        // Locked members never carry a credential in a list response.
        assert!(view.password.is_none());
        assert!(view.trojan_url.is_none());
    }

    #[test]
    fn locked_account_detail_is_flagged() {
        let account = AccountApi {
            id: "acc".into(),
            status: "active".into(),
            included_users: Some(2),
            device_lock: true,
            device_name: "Pixel 8".into(),
            device_platform: "android".into(),
            device_cert_expires_at: Some("2028-11-14T14:30:00Z".into()),
            ..Default::default()
        };
        let detail = AccountDetailView::from_api(&account, now());
        assert!(detail.base.device_lock);
        assert_eq!(detail.base.device_name, "Pixel 8");
        assert_eq!(
            detail.base.device_cert_expires_at.as_deref(),
            Some("2028-11-14T14:30:00Z")
        );
    }

    #[test]
    fn invite_preview_formats_traffic_and_defaults_slots() {
        let preview = InvitePreviewApi {
            package_name: "HaioGate 100GB".into(),
            traffic_bytes: 107_374_182_400,
            slots_total: 0,
            account_id: Some("acc".into()),
            member_id: Some("mem".into()),
            ..Default::default()
        };
        let view = InvitePreviewView::from_api(&preview);
        assert_eq!(view.traffic_display, "100.0 GB");
        assert_eq!(view.slots_total, 1);
        assert_eq!(view.member_id.as_deref(), Some("mem"));
    }

    #[test]
    fn enrollment_view_never_serializes_the_password() {
        let result = EnrollmentApi {
            certificate: "cert".into(),
            ca_chain: "ca".into(),
            trojan_url: "trojan://pw@host:443".into(),
            credential: Some(EnrollmentCredentialApi {
                server: "host".into(),
                port: 443,
                sni: "host".into(),
                password: "super-secret".into(),
                username: "u".into(),
            }),
            package: Some(EnrollmentPackageApi {
                name: "HaioGate 100GB".into(),
                traffic_bytes: 107_374_182_400,
                ..Default::default()
            }),
            account_id: "acc".into(),
            member_id: Some("mem".into()),
            device_fingerprint: "a".repeat(64),
            cert_expires_at: Some("2028-11-14T14:30:00Z".into()),
            ..Default::default()
        };
        let view = EnrollmentResultView::from_api(&result, "m_mem");
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("super-secret"));
        assert!(!json.contains("trojan://"));
        assert_eq!(view.fingerprint_prefix.len(), 16);
        assert_eq!(view.traffic_display, "100.0 GB");
        assert_eq!(view.identity_id, "m_mem");
    }

    #[test]
    fn pending_invite_view_hides_the_token() {
        let invite = PendingInviteApi {
            id: "inv".into(),
            member_name: "Ali".into(),
            token_prefix: "hgi_ABCD".into(),
            status: "pending".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&PendingInviteView::from(&invite)).unwrap();
        assert!(json.contains("hgi_ABCD"));
        assert!(!json.contains("\"token\""));
    }
}
