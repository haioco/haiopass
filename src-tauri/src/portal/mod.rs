//! Portal service: owns tokens, HTTP, purchase intent and member management.
//!
//! Initialisation is infallible: a keyring or filesystem failure degrades the
//! feature, never the app. Every public method returns `PortalError` with a
//! stable code so the frontend can branch without parsing strings.

pub mod client;
pub mod commands;
pub mod error;
pub mod invite;
pub mod keyring_store;
pub mod models;
pub mod pending;
pub mod session;

use client::{decode_list, PortalClient};
use error::{codes, PortalError};
use invite::{is_valid_token, parse_invite_link};
use keyring_store::StoreHandle;
use models::*;
use pending::{mode, stage, PendingPurchase, PendingStore};
use reqwest::Method;
use session::{SessionManager, Tokens};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

/// Ids are opaque to the app but are interpolated into URL paths. Restrict
/// them to characters DRF UUID/int ids can contain so a crafted value cannot
/// pivot the request to another endpoint.
pub(crate) fn validate_id(kind: &str, value: &str) -> Result<String, PortalError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(PortalError::invalid_input(format!(
            "شناسه {} معتبر نیست.",
            kind
        )));
    }
    Ok(trimmed.to_string())
}

/// Build the public/owner enrollment body. The platform string matches the
/// server regex; `app_version` follows the crate version automatically.
fn enrollment_body(csr_pem: &str, device_name: &str, request_id: &str) -> serde_json::Value {
    serde_json::json!({
        "csr": csr_pem,
        "device_name": crate::device::identity::clean_device_name(device_name),
        "platform": crate::device::identity::platform_name(),
        "app_version": env!("CARGO_PKG_VERSION"),
        "request_id": request_id,
    })
}

/// `ttl_hours` is optional; the server owns the default and the 168h maximum.
fn invite_body(ttl_hours: Option<u32>) -> Result<serde_json::Value, PortalError> {
    match ttl_hours {
        None => Ok(serde_json::json!({})),
        Some(hours) if (1..=168).contains(&hours) => {
            Ok(serde_json::json!({ "ttl_hours": hours }))
        }
        Some(_) => Err(PortalError::invalid_input(
            "مدت اعتبار لینک دعوت باید بین ۱ تا ۱۶۸ ساعت باشد.",
        )),
    }
}

fn reset_body(reason: &str) -> serde_json::Value {
    let reason: String = reason
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(120)
        .collect();
    if reason.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::json!({ "reason": reason })
    }
}

fn devices_revoked(value: &serde_json::Value) -> u32 {
    value
        .get("devices_revoked")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32
}

pub fn normalize_mobile(raw: &str) -> Option<String> {
    let mut digits = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let digit = match ch {
            '0'..='9' => ch,
            // Persian / Arabic-Indic digits.
            '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (ch as u32 - 0x06F0) as u8),
            '\u{0660}'..='\u{0669}' => char::from(b'0' + (ch as u32 - 0x0660) as u8),
            _ => continue,
        };
        digits.push(digit);
    }
    if digits.len() == 12 && digits.starts_with("989") {
        // +98 912... → 0912...
        digits = format!("0{}", &digits[2..]);
    }
    if digits.len() == 10 && digits.starts_with('9') {
        digits.insert(0, '0');
    }
    if digits.len() == 11 && digits.starts_with("09") {
        Some(digits)
    } else {
        None
    }
}

pub fn normalize_national_id(raw: &str) -> Option<String> {
    let mut digits = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let digit = match ch {
            '0'..='9' => ch,
            '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (ch as u32 - 0x06F0) as u8),
            '\u{0660}'..='\u{0669}' => char::from(b'0' + (ch as u32 - 0x0660) as u8),
            _ => continue,
        };
        digits.push(digit);
    }
    if digits.len() == 10 {
        Some(digits)
    } else {
        None
    }
}

/// Parse both member-create contract versions:
/// - device-lock: `{member, invite_required, invite?}`
/// - legacy: `MemberCredentialSerializer`
fn parse_member_create(value: serde_json::Value) -> Result<MemberCreateView, PortalError> {
    if value.get("member").is_some() {
        let created: MemberCreateApi =
            serde_json::from_value(value).map_err(|_| PortalError::protocol())?;
        let member = created.member.unwrap_or_default();
        return Ok(MemberCreateView {
            member: MemberView::from_api(&member, false),
            invite_required: true,
            invite: created.invite.as_ref().map(InviteView::from_api),
        });
    }
    let member: MemberApi = serde_json::from_value(value).map_err(|_| PortalError::protocol())?;
    Ok(MemberCreateView {
        member: MemberView::from_api(&member, true),
        invite_required: false,
        invite: None,
    })
}

pub struct PortalService {
    client: PortalClient,
    session: SessionManager,
    pending: PendingStore,
    user: Mutex<Option<UserView>>,
    restoring: AtomicBool,
}

impl Default for PortalService {
    fn default() -> Self {
        Self::new()
    }
}

impl PortalService {
    pub fn new() -> Self {
        Self::with_store(keyring_store::default_store())
    }

    pub fn with_store(handle: StoreHandle) -> Self {
        if !handle.persistent {
            tracing::info!("portal session store is memory-only (no persistent keyring)");
        }
        Self {
            client: PortalClient::new(),
            session: SessionManager::new(handle),
            pending: PendingStore::new(),
            user: Mutex::new(None),
            restoring: AtomicBool::new(true),
        }
    }

    #[cfg(test)]
    pub fn for_tests(base_url: &str) -> Self {
        Self {
            client: PortalClient::with_base_url(base_url),
            session: SessionManager::new(keyring_store::memory_store()),
            pending: PendingStore::at(std::env::temp_dir().join("haiobypass-test-pending.json")),
            user: Mutex::new(None),
            restoring: AtomicBool::new(false),
        }
    }

    pub fn base_url(&self) -> &str {
        self.client.base_url()
    }

    pub fn is_persistent(&self) -> bool {
        self.session.is_persistent()
    }

    /* -------------------------------------------------------------- */
    /* Session                                                        */
    /* -------------------------------------------------------------- */

    pub async fn session_view(&self) -> SessionView {
        let restoring = self.restoring.load(Ordering::SeqCst);
        let user = self.user.lock().await.clone();
        let logged_in = user.is_some() && self.session.has_session().await;
        SessionView {
            logged_in,
            user,
            persistent: self.session.is_persistent(),
            restoring,
        }
    }

    /// Startup restore: read the keyring, validate `/auth/me/`, refresh once if
    /// the access token is stale. Never blocks app startup.
    pub async fn restore(&self) -> SessionView {
        self.restoring.store(true, Ordering::SeqCst);
        let loaded = self.session.load().await.is_some();
        if loaded {
            match self.fetch_me().await {
                Ok(user) => {
                    *self.user.lock().await = Some(user);
                }
                Err(err) if err.code == codes::SESSION_EXPIRED => {
                    self.session.clear().await;
                    *self.user.lock().await = None;
                }
                Err(err) => {
                    // Offline or a transient server error: keep the tokens so a
                    // later retry can restore the session. The UI stays logged
                    // out for now because no user has been validated.
                    tracing::warn!("portal restore could not validate session: {}", err.code);
                }
            }
        }
        self.restoring.store(false, Ordering::SeqCst);
        self.session_view().await
    }

    pub async fn request_otp(&self, mobile: &str) -> Result<OtpRequest, PortalError> {
        let mobile = normalize_mobile(mobile)
            .ok_or_else(|| PortalError::invalid_input("شماره موبایل معتبر نیست."))?;
        let body = serde_json::json!({ "mobile": mobile });
        let request = self
            .client
            .json_request(Method::POST, "auth/otp/request/", None, Some(&body));
        self.client.send_json::<OtpRequest>(request).await
    }

    pub async fn login_otp(&self, mobile: &str, otp: &str) -> Result<UserView, PortalError> {
        let mobile = normalize_mobile(mobile)
            .ok_or_else(|| PortalError::invalid_input("شماره موبایل معتبر نیست."))?;
        let otp: String = otp.chars().filter(|c| c.is_ascii_digit()).collect();
        if otp.len() != 6 {
            return Err(PortalError::invalid_input("کد تایید ۶ رقمی را وارد کنید."));
        }
        let body = serde_json::json!({ "mobile": mobile, "otp": otp });
        let request = self
            .client
            .json_request(Method::POST, "auth/otp/login/", None, Some(&body));
        let auth: AuthTokens = self.client.send_json(request).await?;
        let tokens = Tokens {
            access: auth.access.clone(),
            refresh: auth.refresh.clone(),
        };
        if !tokens.is_valid() {
            return Err(PortalError::protocol());
        }

        let user = match auth.user {
            Some(user) => UserView::from_api(&user),
            None => self.fetch_me().await?,
        };
        self.session.save(tokens).await;
        *self.user.lock().await = Some(user.clone());
        self.restoring.store(false, Ordering::SeqCst);
        Ok(user)
    }

    pub async fn logout(&self) {
        self.session.clear().await;
        *self.user.lock().await = None;
    }

    pub async fn me(&self) -> Result<UserView, PortalError> {
        let user = self.fetch_me().await?;
        *self.user.lock().await = Some(user.clone());
        Ok(user)
    }

    async fn fetch_me(&self) -> Result<UserView, PortalError> {
        let user: PortalUser = self.authorized(Method::GET, "auth/me/", None).await?;
        Ok(UserView::from_api(&user))
    }

    pub async fn shahkar_verify(&self, national_id: &str) -> Result<ShahkarResponse, PortalError> {
        let national_id = normalize_national_id(national_id)
            .ok_or_else(|| PortalError::invalid_input("کد ملی باید ۱۰ رقم باشد."))?;
        // The national code is sent once and never persisted or logged.
        let body = serde_json::json!({ "national_id": national_id });
        let response: ShahkarResponse = self
            .authorized(Method::POST, "auth/shahkar/verify/", Some(body))
            .await?;
        if let Some(user) = &response.user {
            *self.user.lock().await = Some(UserView::from_api(user));
        } else if response.verified {
            if let Ok(user) = self.fetch_me().await {
                *self.user.lock().await = Some(user);
            }
        }
        Ok(response)
    }

    /* -------------------------------------------------------------- */
    /* Authorized request plumbing                                    */
    /* -------------------------------------------------------------- */

    async fn authorized<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, PortalError> {
        let access = self
            .session
            .access_token()
            .await
            .ok_or_else(PortalError::not_logged_in)?;

        let request = self
            .client
            .json_request(method.clone(), path, Some(&access), body.as_ref());
        match self.client.send_json::<T>(request).await {
            Ok(value) => Ok(value),
            Err(err) if err.code == codes::SESSION_EXPIRED => {
                if self.refresh_after_401(&access).await {
                    let retry_access = self
                        .session
                        .access_token()
                        .await
                        .ok_or_else(PortalError::session_expired)?;
                    let retry = self
                        .client
                        .json_request(method, path, Some(&retry_access), body.as_ref());
                    self.client.send_json::<T>(retry).await
                } else {
                    self.session.clear().await;
                    *self.user.lock().await = None;
                    Err(PortalError::session_expired())
                }
            }
            Err(err) => Err(err),
        }
    }

    /// Single-flight refresh. If another task already rotated the token, the
    /// stale 401 is retried with the fresh token without a second refresh.
    async fn refresh_after_401(&self, stale_access: &str) -> bool {
        let _guard = self.session.lock_refresh().await;
        if let Some(current) = self.session.access_token().await {
            if current != stale_access {
                return true;
            }
        }
        let refresh = match self.session.refresh_token().await {
            Some(value) => value,
            None => return false,
        };

        let body = serde_json::json!({ "refresh": refresh });
        let request = self
            .client
            .json_request(Method::POST, "auth/refresh/", None, Some(&body));
        match self.client.send_json::<AuthTokens>(request).await {
            Ok(auth) => {
                if auth.access.is_empty() || auth.refresh.is_empty() {
                    return false;
                }
                // The user may have logged out while the refresh was in
                // flight; do not resurrect the session.
                if self.session.current().await.is_none() {
                    return false;
                }
                self.session
                    .save(Tokens {
                        access: auth.access,
                        refresh: auth.refresh,
                    })
                    .await;
                true
            }
            Err(err) => {
                tracing::warn!("portal token refresh failed: {}", err.code);
                false
            }
        }
    }

    /* -------------------------------------------------------------- */
    /* HaioGate                                                       */
    /* -------------------------------------------------------------- */

    pub async fn list_packages(&self) -> Result<Vec<PackageView>, PortalError> {
        let value: serde_json::Value = self
            .authorized(Method::GET, "antisanction/packages/?page_size=100", None)
            .await?;
        let mut packages: Vec<PackageView> = decode_list::<PackageApi>(value)?
            .iter()
            .map(PackageView::from_api)
            .collect();
        packages.sort_by_key(|pkg| (pkg.sort_order, pkg.price));
        Ok(packages)
    }

    pub async fn list_accounts(&self) -> Result<Vec<AccountView>, PortalError> {
        let value: serde_json::Value = self
            .authorized(Method::GET, "antisanction/?page_size=100", None)
            .await?;
        let now = chrono::Utc::now();
        let mut accounts: Vec<AccountView> = decode_list::<AccountApi>(value)?
            .iter()
            .map(|account| AccountView::from_api(account, now))
            .collect();
        // Active first, then expired, then support; newest first inside a group.
        accounts.sort_by(|a, b| {
            let rank = |category: &str| match category {
                "active" => 0,
                "expired" => 1,
                _ => 2,
            };
            rank(&a.category)
                .cmp(&rank(&b.category))
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        Ok(accounts)
    }

    pub async fn account_detail(&self, account_id: &str) -> Result<AccountDetailView, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let account: AccountApi = self
            .authorized(
                Method::GET,
                &format!("antisanction/{}/", account_id),
                None,
            )
            .await?;
        Ok(AccountDetailView::from_api(&account, chrono::Utc::now()))
    }

    pub async fn sync_quota(&self, account_id: &str) -> Result<AccountDetailView, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let account: AccountApi = self
            .authorized(
                Method::GET,
                &format!("antisanction/{}/quota/", account_id),
                None,
            )
            .await?;
        Ok(AccountDetailView::from_api(&account, chrono::Utc::now()))
    }

    pub async fn start_buy(&self, package_id: &str) -> Result<PurchaseIntentView, PortalError> {
        let package_id = validate_id("پکیج", package_id)?;
        let body = serde_json::json!({ "package_id": package_id });
        let intent: PurchaseIntentApi = self
            .authorized(Method::POST, "antisanction/", Some(body))
            .await?;
        if intent.plan_id.is_empty() {
            return Err(PortalError::protocol());
        }
        Ok(PurchaseIntentView::from(&intent))
    }

    pub async fn start_renew(
        &self,
        account_id: &str,
        package_id: Option<&str>,
    ) -> Result<PurchaseIntentView, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let mut body = serde_json::Map::new();
        if let Some(package_id) = package_id {
            body.insert(
                "package_id".into(),
                serde_json::Value::String(validate_id("پکیج", package_id)?),
            );
        }
        let intent: PurchaseIntentApi = self
            .authorized(
                Method::POST,
                &format!("antisanction/{}/renew/", account_id),
                Some(serde_json::Value::Object(body)),
            )
            .await?;
        if intent.plan_id.is_empty() {
            return Err(PortalError::protocol());
        }
        Ok(PurchaseIntentView::from(&intent))
    }

    pub async fn buy_plan(
        &self,
        plan_id: &str,
        organization_id: Option<&str>,
    ) -> Result<BuyResponse, PortalError> {
        let plan_id = validate_id("پلن", plan_id)?;
        let mut body = serde_json::Map::new();
        body.insert(
            "plan_id".into(),
            serde_json::Value::String(plan_id),
        );
        if let Some(org) = organization_id {
            body.insert(
                "organization_id".into(),
                serde_json::Value::String(validate_id("سازمان", org)?),
            );
        }
        let buy: BuyApi = self
            .authorized(
                Method::POST,
                "billing/subscriptions/buy/",
                Some(serde_json::Value::Object(body)),
            )
            .await?;

        if buy.status == "payment_required"
            && buy
                .payment_url
                .as_deref()
                .map(|url| client::validate_payment_url(url).is_err())
                .unwrap_or(true)
        {
            return Err(PortalError::payment_url_invalid());
        }
        Ok(BuyResponse::from(&buy))
    }

    pub async fn invoice(&self, invoice_id: &str) -> Result<InvoiceView, PortalError> {
        let invoice_id = validate_id("فاکتور", invoice_id)?;
        // Safe to poll: this is the invoice endpoint, never the gateway
        // callback (`billing/payments/verify/`).
        let invoice: InvoiceApi = self
            .authorized(
                Method::GET,
                &format!("billing/invoices/{}/", invoice_id),
                None,
            )
            .await?;
        Ok(InvoiceView::from_api(&invoice))
    }

    pub async fn invoice_pay(&self, invoice_id: &str) -> Result<InvoicePayView, PortalError> {
        let invoice_id = validate_id("فاکتور", invoice_id)?;
        let pay: InvoicePayApi = self
            .authorized(
                Method::POST,
                &format!("billing/invoices/{}/pay/", invoice_id),
                Some(serde_json::json!({})),
            )
            .await?;
        // Resume flow gets a fresh gateway URL; validate it like a new one.
        client::validate_payment_url(&pay.payment_url)?;
        Ok(InvoicePayView {
            payment_url: pay.payment_url,
            payment_id: pay.payment_id,
            invoice_id: pay.invoice_id.unwrap_or(invoice_id),
        })
    }

    pub async fn list_orgs(&self) -> Result<Vec<OrganizationView>, PortalError> {
        let value: serde_json::Value = self
            .authorized(Method::GET, "orgs/?page_size=100", None)
            .await?;
        Ok(decode_list::<OrganizationApi>(value)?
            .iter()
            .map(OrganizationView::from)
            .collect())
    }

    /* -------------------------------------------------------------- */
    /* Members                                                        */
    /* -------------------------------------------------------------- */

    pub async fn add_member(
        &self,
        account_id: &str,
        display_name: &str,
        mobile: Option<&str>,
        email: Option<&str>,
        issue_invite: bool,
    ) -> Result<MemberCreateView, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let display_name = display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > 100 {
            return Err(PortalError::invalid_input("نام کاربر را وارد کنید."));
        }
        let mut body = serde_json::Map::new();
        body.insert(
            "display_name".into(),
            serde_json::Value::String(display_name.to_string()),
        );
        if let Some(mobile) = mobile.filter(|value| !value.trim().is_empty()) {
            if let Some(normalized) = normalize_mobile(mobile) {
                body.insert("mobile".into(), serde_json::Value::String(normalized));
            }
        }
        if let Some(email) = email.filter(|value| !value.trim().is_empty()) {
            body.insert(
                "email".into(),
                serde_json::Value::String(email.trim().to_string()),
            );
        }
        // Only sent for device-lock accounts; the legacy backend ignores it.
        if issue_invite {
            body.insert("issue_invite".into(), serde_json::Value::Bool(true));
        }

        let value: serde_json::Value = self
            .authorized(
                Method::POST,
                &format!("antisanction/{}/members/", account_id),
                Some(serde_json::Value::Object(body)),
            )
            .await?;
        parse_member_create(value)
    }

    pub async fn rotate_member(
        &self,
        account_id: &str,
        member_id: &str,
    ) -> Result<MemberCreateView, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let member_id = validate_id("کاربر", member_id)?;
        let value: serde_json::Value = self
            .authorized(
                Method::POST,
                &format!("antisanction/{}/members/{}/rotate/", account_id, member_id),
                Some(serde_json::json!({})),
            )
            .await?;
        parse_member_create(value)
    }

    pub async fn revoke_member(
        &self,
        account_id: &str,
        member_id: &str,
    ) -> Result<(), PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let member_id = validate_id("کاربر", member_id)?;
        let _: serde_json::Value = self
            .authorized(
                Method::DELETE,
                &format!("antisanction/{}/members/{}/", account_id, member_id),
                None,
            )
            .await?;
        Ok(())
    }

    /* -------------------------------------------------------------- */
    /* Device invites and enrollment                                  */
    /* -------------------------------------------------------------- */

    /// Public invite preview. No portal session is required (and none is
    /// created); the token is validated locally before the request.
    pub async fn preview_invite(&self, raw: &str) -> Result<InvitePreviewView, PortalError> {
        let token = parse_invite_link(raw)?;
        self.preview_invite_token(&token).await
    }

    /// Internal variant for callers that already parsed the token.
    pub(crate) async fn preview_invite_token(
        &self,
        token: &str,
    ) -> Result<InvitePreviewView, PortalError> {
        if !is_valid_token(token) {
            return Err(PortalError::invalid_input("لینک دعوت معتبر نیست."));
        }
        let request = self.client.json_request(
            Method::GET,
            &format!("antisanction/invites/{}/", token),
            None,
            None,
        );
        let preview: InvitePreviewApi = self.client.send_json(request).await?;
        Ok(InvitePreviewView::from_api(&preview))
    }

    /// Public single-use enrollment with a CSR. Returns the raw certificate
    /// response; the command layer persists it only after this succeeds.
    pub async fn enroll_invite(
        &self,
        token: &str,
        csr_pem: &str,
        device_name: &str,
        request_id: &str,
    ) -> Result<EnrollmentApi, PortalError> {
        if !is_valid_token(token) {
            return Err(PortalError::invalid_input("لینک دعوت معتبر نیست."));
        }
        let body = enrollment_body(csr_pem, device_name, request_id);
        let request = self.client.json_request(
            Method::POST,
            &format!("antisanction/invites/{}/enroll/", token),
            None,
            Some(&body),
        );
        self.client.send_json(request).await
    }

    pub async fn primary_invite(
        &self,
        account_id: &str,
        ttl_hours: Option<u32>,
    ) -> Result<InviteView, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let invite: InviteApi = self
            .authorized(
                Method::POST,
                &format!("antisanction/{}/primary/invite/", account_id),
                Some(invite_body(ttl_hours)?),
            )
            .await?;
        Ok(InviteView::from_api(&invite))
    }

    pub async fn member_invite(
        &self,
        account_id: &str,
        member_id: &str,
        ttl_hours: Option<u32>,
    ) -> Result<InviteView, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let member_id = validate_id("کاربر", member_id)?;
        let invite: InviteApi = self
            .authorized(
                Method::POST,
                &format!(
                    "antisanction/{}/members/{}/invite/",
                    account_id, member_id
                ),
                Some(invite_body(ttl_hours)?),
            )
            .await?;
        Ok(InviteView::from_api(&invite))
    }

    /// Owner re-enrolls the primary credential (lost/replaced device flow).
    pub async fn primary_enroll(
        &self,
        account_id: &str,
        csr_pem: &str,
        device_name: &str,
        request_id: &str,
    ) -> Result<EnrollmentApi, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let body = enrollment_body(csr_pem, device_name, request_id);
        self.authorized(
            Method::POST,
            &format!("antisanction/{}/primary/enroll/", account_id),
            Some(body),
        )
        .await
    }

    /// Revoke the primary device server-side. Returns how many device rows
    /// were revoked.
    pub async fn primary_reset_device(
        &self,
        account_id: &str,
        reason: &str,
    ) -> Result<u32, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let value: serde_json::Value = self
            .authorized(
                Method::POST,
                &format!("antisanction/{}/primary/reset-device/", account_id),
                Some(reset_body(reason)),
            )
            .await?;
        Ok(devices_revoked(&value))
    }

    pub async fn member_reset_device(
        &self,
        account_id: &str,
        member_id: &str,
        reason: &str,
    ) -> Result<u32, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let member_id = validate_id("کاربر", member_id)?;
        let value: serde_json::Value = self
            .authorized(
                Method::POST,
                &format!(
                    "antisanction/{}/members/{}/reset-device/",
                    account_id, member_id
                ),
                Some(reset_body(reason)),
            )
            .await?;
        Ok(devices_revoked(&value))
    }

    pub async fn list_invites(
        &self,
        account_id: &str,
    ) -> Result<Vec<PendingInviteView>, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let value: serde_json::Value = self
            .authorized(
                Method::GET,
                &format!("antisanction/{}/invites/", account_id),
                None,
            )
            .await?;
        Ok(decode_list::<PendingInviteApi>(value)?
            .iter()
            .map(PendingInviteView::from)
            .collect())
    }

    pub async fn cancel_invite(
        &self,
        account_id: &str,
        invite_id: &str,
    ) -> Result<(), PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let invite_id = validate_id("دعوت", invite_id)?;
        let _: serde_json::Value = self
            .authorized(
                Method::POST,
                &format!(
                    "antisanction/{}/invites/{}/cancel/",
                    account_id, invite_id
                ),
                Some(serde_json::json!({})),
            )
            .await?;
        Ok(())
    }

    /* -------------------------------------------------------------- */
    /* Pending purchase                                               */
    /* -------------------------------------------------------------- */

    pub async fn pending_purchase(&self) -> Option<PendingPurchase> {
        self.pending.load().await
    }

    pub async fn cancel_pending(&self) -> Result<(), PortalError> {
        self.pending.clear().await
    }

    pub async fn persist_pending(&self, pending: &PendingPurchase) -> Result<(), PortalError> {
        self.pending.save(pending).await
    }

    pub fn pending_path(&self) -> std::borrow::Cow<'_, str> {
        self.pending.path().to_string_lossy()
    }

    /// Reconcile a pending purchase on window focus or startup.
    ///
    /// Never calls the destructive gateway callback; only the invoice
    /// endpoint. `state` is one of:
    /// `none | pending | paid | canceled | refunded | failed | expired`.
    pub async fn reconcile_pending(&self) -> Result<serde_json::Value, PortalError> {
        let pending = match self.pending.load().await {
            Some(pending) => pending,
            None => {
                return Ok(serde_json::json!({ "state": "none" }));
            }
        };

        let invoice_id = match pending.invoice_id.clone() {
            Some(id) => id,
            None => {
                self.pending.clear().await?;
                return Ok(serde_json::json!({ "state": "expired" }));
            }
        };

        let invoice = match self.invoice(&invoice_id).await {
            Ok(invoice) => invoice,
            Err(err) => {
                if err.code == codes::SESSION_EXPIRED || err.code == codes::NOT_LOGGED_IN {
                    // Keep the pending record so it can be reconciled after the
                    // user logs in again.
                    return Err(err);
                }
                // Invoice disappeared / server error: keep pending for retry
                // unless it is definitively gone.
                if err.code == codes::NOT_FOUND {
                    self.pending.clear().await?;
                    return Ok(serde_json::json!({ "state": "failed" }));
                }
                return Ok(serde_json::json!({
                    "state": "pending",
                    "error": err.code,
                }));
            }
        };

        match invoice.status.as_str() {
            "paid" => {
                // Payment is terminal for the invoice. Clear the recovery file
                // and hand the refreshed account list to the UI; provisioning
                // may lag, and the list refresh covers that.
                self.pending.clear().await?;
                let accounts = self.list_accounts().await.unwrap_or_default();
                Ok(serde_json::json!({
                    "state": "paid",
                    "invoice": invoice,
                    "accounts": accounts,
                }))
            }
            "canceled" | "cancelled" => {
                self.pending.clear().await?;
                Ok(serde_json::json!({ "state": "canceled", "invoice": invoice }))
            }
            "refunded" => {
                self.pending.clear().await?;
                Ok(serde_json::json!({ "state": "refunded", "invoice": invoice }))
            }
            "failed" | "expired" => {
                self.pending.clear().await?;
                Ok(serde_json::json!({ "state": "failed", "invoice": invoice }))
            }
            _ => Ok(serde_json::json!({ "state": "pending", "invoice": invoice })),
        }
    }

    /// Mark the pending record's stage. Used after the browser is opened and
    /// when a payment is known to be paid.
    pub async fn set_pending_stage(&self, new_stage: &str) -> Result<(), PortalError> {
        if let Some(mut pending) = self.pending.load().await {
            pending.stage = new_stage.to_string();
            self.pending.save(&pending).await?;
        }
        Ok(())
    }

    pub fn pending_mode_buy() -> &'static str {
        mode::BUY
    }

    pub fn pending_mode_renew() -> &'static str {
        mode::RENEW
    }

    pub fn stage_browser_opened() -> &'static str {
        stage::BROWSER_OPENED
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring_store::memory_store;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Minimal scripted HTTP/1.1 server: each connection gets the next canned
    /// response. Dependency-free stand-in for wiremock.
    async fn spawn_server(responses: Vec<(u16, String)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 16384];
                let _ = socket.read(&mut buf).await;
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    429 => "Too Many Requests",
                    _ => "Error",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        format!("http://{}/api/v1/", addr)
    }

    #[test]
    fn normalizes_persian_digits() {
        assert_eq!(
            normalize_mobile("۰۹۱۲۳۴۵۶۷۸۹").as_deref(),
            Some("09123456789")
        );
        assert_eq!(
            normalize_mobile("+98 912 345 6789").as_deref(),
            Some("09123456789")
        );
        assert_eq!(
            normalize_mobile("9123456789").as_deref(),
            Some("09123456789")
        );
        assert!(normalize_mobile("12345").is_none());
    }

    #[test]
    fn normalizes_national_id() {
        assert_eq!(
            normalize_national_id("۰۰۱۲۳۴۵۶۷۸").as_deref(),
            Some("0012345678")
        );
        assert!(normalize_national_id("123").is_none());
    }

    #[test]
    fn rejects_path_injection_ids() {
        assert!(validate_id("اکانت", "../auth/me").is_err());
        assert!(validate_id("اکانت", "a b").is_err());
        assert!(validate_id("اکانت", "").is_err());
        assert!(validate_id("اکانت", "0f8fad5b-d9cb-469f-a165-70867728950e").is_ok());
        assert!(validate_id("پکیج", "42").is_ok());
    }

    #[tokio::test]
    async fn session_view_defaults_to_logged_out() {
        let service = PortalService::with_store(memory_store());
        let view = service.session_view().await;
        assert!(!view.logged_in);
        assert!(view.restoring);
        assert!(!view.persistent);
    }

    #[tokio::test]
    async fn logout_clears_user_cache() {
        let service = PortalService::with_store(memory_store());
        *service.user.lock().await = Some(UserView::default());
        service.session.save(Tokens { access: "a".into(), refresh: "r".into() }).await;
        service.logout().await;
        assert!(service.user.lock().await.is_none());
        assert!(service.session.current().await.is_none());
    }

    #[test]
    fn stage_helpers_match_file_contract() {
        assert_eq!(PortalService::pending_mode_buy(), "buy");
        assert_eq!(PortalService::pending_mode_renew(), "renew");
        assert_eq!(PortalService::stage_browser_opened(), "browser_opened");
    }

    #[test]
    fn invite_ttl_is_bounded() {
        assert!(invite_body(None).unwrap().as_object().unwrap().is_empty());
        assert_eq!(invite_body(Some(72)).unwrap()["ttl_hours"], 72);
        assert_eq!(invite_body(Some(1)).unwrap()["ttl_hours"], 1);
        assert_eq!(invite_body(Some(168)).unwrap()["ttl_hours"], 168);
        assert_eq!(invite_body(Some(0)).unwrap_err().code, codes::INVALID_INPUT);
        assert_eq!(
            invite_body(Some(169)).unwrap_err().code,
            codes::INVALID_INPUT
        );
    }

    #[test]
    fn reset_reason_is_cleaned() {
        assert!(reset_body("  ").as_object().unwrap().is_empty());
        let body = reset_body("  lost\u{0007} phone  ");
        assert_eq!(body["reason"], "lost phone");
        let long = reset_body(&"x".repeat(300));
        assert_eq!(long["reason"].as_str().unwrap().chars().count(), 120);
    }

    #[test]
    fn devices_revoked_is_tolerant() {
        assert_eq!(devices_revoked(&serde_json::json!({"devices_revoked": 2})), 2);
        assert_eq!(devices_revoked(&serde_json::json!({})), 0);
        assert_eq!(
            devices_revoked(&serde_json::json!({"devices_revoked": "x"})),
            0
        );
    }

    #[test]
    fn parses_locked_member_create_with_invite() {
        let value = serde_json::json!({
            "member": {
                "id": "m1", "display_name": "Ali", "username": "u_m1",
                "status": "pending", "member_index": 1, "device_lock": true,
                "device": null
            },
            "invite_required": true,
            "invite": {
                "token": "hgi_x", "invite_url": "https://my.haio.ir/join/hgi_x",
                "deep_link": "haio://join/hgi_x", "expires_at": "2026-11-18T14:30:00Z",
                "member_id": "m1", "member_index": 1, "package_name": "HaioGate 100GB",
                "account_id": "acc"
            }
        });
        let created = parse_member_create(value).expect("locked create");
        assert!(created.invite_required);
        assert_eq!(created.member.id, "m1");
        assert!(created.invite.is_some());
        // Locked create never exposes a credential.
        assert!(created.member.password.is_none());
        assert!(created.member.trojan_url.is_none());
    }

    #[test]
    fn parses_a_pending_locked_member_without_invite() {
        let value = serde_json::json!({
            "member": {
                "id": "m2", "display_name": "Sara", "username": "u_m2",
                "status": "pending", "member_index": 2, "device_lock": true
            },
            "invite_required": true
        });
        let created = parse_member_create(value).expect("pending create");
        assert!(created.invite_required);
        assert!(created.invite.is_none());
    }

    #[test]
    fn parses_a_legacy_member_credential_response() {
        let value = serde_json::json!({
            "id": "m3", "display_name": "Reza", "username": "u_m3",
            "status": "active", "member_index": 3,
            "password": "pw", "trojan_url": "trojan://pw@host:443"
        });
        let created = parse_member_create(value).expect("legacy create");
        assert!(!created.invite_required);
        assert_eq!(created.member.password.as_deref(), Some("pw"));
        assert!(created.member.trojan_url.is_some());
    }

    #[tokio::test]
    async fn invite_preview_is_public_and_typed() {
        let base = spawn_server(vec![(
            200,
            r#"{"status":"pending","account_name_masked":"a***@example.com","package_name":"HaioGate 100GB","traffic_bytes":107374182400,"expires_at":"2026-11-18T14:30:00Z","member_index":1,"slots_total":2,"account_id":"acc","member_id":"mem"}"#
                .to_string(),
        )])
        .await;
        let service = PortalService::for_tests(&base);
        let token = format!("hgi_{}", "A".repeat(43));
        let preview = service.preview_invite(&token).await.expect("preview");
        assert_eq!(preview.package_name, "HaioGate 100GB");
        assert_eq!(preview.traffic_display, "100.0 GB");
        assert_eq!(preview.member_index, 1);
        assert_eq!(preview.slots_total, 2);
        assert_eq!(preview.member_id.as_deref(), Some("mem"));
    }

    #[tokio::test]
    async fn invite_expired_is_typed_not_a_generic_409() {
        let base = spawn_server(vec![(
            410,
            r#"{"detail":"This invite expired.","code":"invite_expired"}"#.to_string(),
        )])
        .await;
        let service = PortalService::for_tests(&base);
        let token = format!("hgi_{}", "B".repeat(43));
        let err = service.preview_invite(&token).await.expect_err("expired");
        assert_eq!(err.code, codes::INVITE_EXPIRED);
        // The local Persian copy wins over the English provider detail.
        assert!(err.message.contains("منقضی"));
    }

    #[tokio::test]
    async fn enroll_invite_parses_the_exact_backend_payload() {
        let base = spawn_server(vec![(
            200,
            r#"{
                "certificate":"-----BEGIN CERTIFICATE-----\nleaf\n-----END CERTIFICATE-----\n",
                "ca_chain":"-----BEGIN CERTIFICATE-----\nca\n-----END CERTIFICATE-----\n",
                "trojan_url":"trojan://pw@tahrim.haiocloud.com:443?sni=tahrim.haiocloud.com",
                "credential":{"server":"tahrim.haiocloud.com","port":443,"sni":"tahrim.haiocloud.com","password":"pw","username":"antisanction_x_m1"},
                "package":{"id":"pkg","name":"HaioGate 100GB","traffic_bytes":107374182400},
                "expires_at":"2026-12-15T14:30:00Z",
                "account_id":"acc",
                "member_id":"mem",
                "device_fingerprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "cert_expires_at":"2028-11-14T14:30:00Z"
            }"#
            .to_string(),
        )])
        .await;
        let service = PortalService::for_tests(&base);
        let token = format!("hgi_{}", "C".repeat(43));
        let result = service
            .enroll_invite(&token, "-----BEGIN CERTIFICATE REQUEST-----\nx\n", "Pixel 8", "00000000-0000-4000-8000-000000000000")
            .await
            .expect("enroll");
        assert_eq!(result.account_id, "acc");
        assert_eq!(result.member_id.as_deref(), Some("mem"));
        let credential = result.credential.expect("credential");
        assert_eq!(credential.password, "pw");
        assert_eq!(credential.server, "tahrim.haiocloud.com");
        assert_eq!(result.package.expect("package").name, "HaioGate 100GB");
        assert_eq!(result.device_fingerprint.len(), 64);
    }

    #[tokio::test]
    async fn invalid_invite_token_never_reaches_the_network() {
        let service = PortalService::for_tests("http://127.0.0.1:1/api/v1/");
        let err = service.preview_invite("not-a-token").await.expect_err("rejected");
        assert_eq!(err.code, codes::INVALID_INPUT);
        assert!(!err.message.contains("not-a-token"));
    }

    /* ---------------- HTTP behaviour (scripted local server) ------------- */

    #[tokio::test]
    async fn refreshes_once_and_retries_the_request() {
        let base = spawn_server(vec![
            (401, r#"{"detail":"expired"}"#.to_string()),
            (200, r#"{"access":"new-a","refresh":"new-r"}"#.to_string()),
            (
                200,
                r#"{"id":7,"mobile":"09123456789","shahkar_status":1,"first_name":"A"}"#
                    .to_string(),
            ),
        ])
        .await;

        let service = PortalService::for_tests(&base);
        service
            .session
            .save(Tokens {
                access: "old-a".into(),
                refresh: "old-r".into(),
            })
            .await;

        let user = service.me().await.expect("refresh then retry");
        assert_eq!(user.mobile.as_deref(), Some("09123456789"));
        assert!(user.shahkar_verified);
        // The rotated token pair replaced the old one.
        assert_eq!(service.session.access_token().await.as_deref(), Some("new-a"));
        assert_eq!(service.session.refresh_token().await.as_deref(), Some("new-r"));
    }

    #[tokio::test]
    async fn failed_refresh_clears_the_session() {
        let base = spawn_server(vec![
            (401, r#"{"detail":"expired"}"#.to_string()),
            (401, r#"{"detail":"refresh invalid"}"#.to_string()),
        ])
        .await;

        let service = PortalService::for_tests(&base);
        service
            .session
            .save(Tokens {
                access: "old-a".into(),
                refresh: "old-r".into(),
            })
            .await;

        let err = service.me().await.expect_err("session must expire");
        assert_eq!(err.code, codes::SESSION_EXPIRED);
        assert!(service.session.current().await.is_none());
    }

    #[tokio::test]
    async fn shahkar_required_is_mapped_from_the_portal() {
        let base = spawn_server(vec![(
            403,
            r#"{"detail":"verify first","code":"shahkar_required"}"#.to_string(),
        )])
        .await;

        let service = PortalService::for_tests(&base);
        service
            .session
            .save(Tokens {
                access: "a".into(),
                refresh: "r".into(),
            })
            .await;

        let err = service
            .start_buy("pkg-1")
            .await
            .expect_err("403 must be typed");
        assert_eq!(err.code, codes::SHAHKAR_REQUIRED);
    }

    #[tokio::test]
    async fn network_failure_is_typed() {
        let service = PortalService::for_tests("http://127.0.0.1:1/api/v1/");
        let err = service.request_otp("09123456789").await.expect_err("refused");
        assert!(matches!(err.code.as_str(), codes::NETWORK | codes::TIMEOUT));
    }

    #[tokio::test]
    async fn invalid_ids_never_reach_the_network() {
        let service = PortalService::for_tests("http://127.0.0.1:1/api/v1/");
        service
            .session
            .save(Tokens {
                access: "a".into(),
                refresh: "r".into(),
            })
            .await;
        let err = service.account_detail("../auth/me").await.expect_err("rejected");
        assert_eq!(err.code, codes::INVALID_INPUT);
    }

    #[tokio::test]
    async fn reconciles_a_paid_invoice_without_the_gateway_callback() {
        let base = spawn_server(vec![
            (
                200,
                r#"{"id":"inv-1","status":"paid","amount":50000,"plan":{"id":"9","name":"P","slug":"p"}}"#
                    .to_string(),
            ),
            (200, r#"{"results":[]}"#.to_string()),
        ])
        .await;
        let service = PortalService::for_tests(&base);
        service
            .session
            .save(Tokens {
                access: "a".into(),
                refresh: "r".into(),
            })
            .await;

        let mut pending = PendingPurchase::new(pending::mode::BUY);
        pending.invoice_id = Some("inv-1".into());
        service.persist_pending(&pending).await.unwrap();

        let result = service.reconcile_pending().await.unwrap();
        assert_eq!(result["state"], "paid");
        assert!(service.pending_purchase().await.is_none(), "paid clears pending");
    }
}
