//! Portal service: owns tokens, HTTP, purchase intent and member management.
//!
//! Initialisation is infallible: a keyring or filesystem failure degrades the
//! feature, never the app. Every public method returns `PortalError` with a
//! stable code so the frontend can branch without parsing strings.

pub mod client;
pub mod commands;
pub mod error;
pub mod keyring_store;
pub mod models;
pub mod pending;
pub mod session;

use client::{decode_list, PortalClient};
use error::{codes, PortalError};
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
fn validate_id(kind: &str, value: &str) -> Result<String, PortalError> {
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
    ) -> Result<MemberView, PortalError> {
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

        let member: MemberApi = self
            .authorized(
                Method::POST,
                &format!("antisanction/{}/members/", account_id),
                Some(serde_json::Value::Object(body)),
            )
            .await?;
        Ok(MemberView::from_api(&member, true))
    }

    pub async fn rotate_member(
        &self,
        account_id: &str,
        member_id: &str,
    ) -> Result<MemberView, PortalError> {
        let account_id = validate_id("اکانت", account_id)?;
        let member_id = validate_id("کاربر", member_id)?;
        let member: MemberApi = self
            .authorized(
                Method::POST,
                &format!("antisanction/{}/members/{}/rotate/", account_id, member_id),
                Some(serde_json::json!({})),
            )
            .await?;
        Ok(MemberView::from_api(&member, true))
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
