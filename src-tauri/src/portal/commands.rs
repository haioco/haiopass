//! `#[tauri::command]` wrappers for the portal service.
//!
//! Every command returns `Result<T, PortalError>`; the frontend branches on
//! `error.code`. No command returns tokens, the national code, or a payment URL
//! that failed validation.

use super::client::validate_payment_url;
use super::error::{codes, PortalError};
use super::invite::parse_invite_link;
use super::models::{
    AccountDetailView, AccountView, BuyResponse, DeviceIdentityView, EnrollmentCredentialApi,
    EnrollmentResultView, InvitePreviewView, InviteView, InvoicePayView, InvoiceView,
    MemberCreateView, OrganizationView, PackageView, PendingInviteView, PurchaseIntentView,
    SessionView, ShahkarView, UserView,
};
use super::pending::{mode as pending_mode, stage, PendingPurchase};
use crate::device::{identity as device_identity, DeviceCertificate, DeviceError};
use crate::AppState;
use std::sync::Arc;
use tauri::State;

type CmdResult<T> = Result<T, PortalError>;

fn invalid(message: &str) -> PortalError {
    PortalError::invalid_input(message)
}

/// Open the validated gateway URL in the system browser.
///
/// `tauri-plugin-shell`'s `open` is the plugin the app already ships with; the
/// upstream deprecation notice points at `tauri-plugin-opener`, which is not a
/// dependency here and would add a second, unnecessary surface.
#[allow(deprecated)]
fn open_in_browser(app: &tauri::AppHandle, url: &str) -> Result<(), PortalError> {
    use tauri_plugin_shell::ShellExt;
    app.shell()
        .open(url.to_string(), None)
        .map_err(|_| PortalError::new(codes::OPEN_FAILED, "باز کردن درگاه پرداخت ناموفق بود."))
}

/* ------------------------------------------------------------------ */
/* Session                                                            */
/* ------------------------------------------------------------------ */

#[tauri::command]
pub async fn portal_session(state: State<'_, Arc<AppState>>) -> CmdResult<SessionView> {
    Ok(state.portal.session_view().await)
}

#[tauri::command]
pub async fn portal_request_otp(
    state: State<'_, Arc<AppState>>,
    mobile: String,
) -> CmdResult<super::models::OtpRequestView> {
    let res = state.portal.request_otp(&mobile).await?;
    Ok(super::models::OtpRequestView {
        sent: res.sent,
        expires_in: res.expires_in,
        retry_after: res.retry_after,
        provider: res.provider,
    })
}

#[tauri::command]
pub async fn portal_login_otp(
    state: State<'_, Arc<AppState>>,
    mobile: String,
    otp: String,
) -> CmdResult<serde_json::Value> {
    let user = state.portal.login_otp(&mobile, &otp).await?;
    Ok(serde_json::json!({
        "user": user,
        "persistent": state.portal.is_persistent(),
    }))
}

#[tauri::command]
pub async fn portal_logout(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> CmdResult<serde_json::Value> {
    state.portal.logout().await;
    // Device-lock guarantee: logout deletes the keyring entry and the public
    // certificate metadata. A legacy password config is left untouched.
    let device_cleared =
        clear_local_device_identity(&app, state.inner(), None, None, true).await;
    Ok(serde_json::json!({ "success": true, "deviceCleared": device_cleared }))
}

#[tauri::command]
pub async fn portal_me(state: State<'_, Arc<AppState>>) -> CmdResult<UserView> {
    state.portal.me().await
}

#[tauri::command]
pub async fn portal_shahkar_verify(
    state: State<'_, Arc<AppState>>,
    national_id: String,
) -> CmdResult<ShahkarView> {
    let res = state.portal.shahkar_verify(&national_id).await?;
    let user = match res.user.as_ref() {
        Some(user) => Some(UserView::from_api(user)),
        // Only refresh the cached profile on success; a mismatch must not turn
        // into a second network failure.
        None if res.verified => Some(state.portal.me().await?),
        None => None,
    };
    Ok(ShahkarView {
        verified: res.verified,
        shahkar_status: res.shahkar_status,
        user,
    })
}

/* ------------------------------------------------------------------ */
/* HaioGate                                                           */
/* ------------------------------------------------------------------ */

#[tauri::command]
pub async fn portal_list_packages(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Vec<PackageView>> {
    state.portal.list_packages().await
}

#[tauri::command]
pub async fn portal_list_accounts(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<AccountView>> {
    state.portal.list_accounts().await
}

#[tauri::command]
pub async fn portal_account_detail(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> CmdResult<AccountDetailView> {
    state.portal.account_detail(&account_id).await
}

#[tauri::command]
pub async fn portal_sync_quota(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> CmdResult<AccountDetailView> {
    state.portal.sync_quota(&account_id).await
}

#[tauri::command]
pub async fn portal_start_buy(
    state: State<'_, Arc<AppState>>,
    package_id: String,
) -> CmdResult<PurchaseIntentView> {
    state.portal.start_buy(&package_id).await
}

#[tauri::command]
pub async fn portal_start_renew(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    package_id: Option<String>,
) -> CmdResult<PurchaseIntentView> {
    state
        .portal
        .start_renew(&account_id, package_id.as_deref())
        .await
}

#[tauri::command]
pub async fn portal_buy_plan(
    state: State<'_, Arc<AppState>>,
    plan_id: String,
    organization_id: Option<String>,
) -> CmdResult<BuyResponse> {
    state
        .portal
        .buy_plan(&plan_id, organization_id.as_deref())
        .await
}

#[tauri::command]
pub async fn portal_invoice(
    state: State<'_, Arc<AppState>>,
    invoice_id: String,
) -> CmdResult<InvoiceView> {
    state.portal.invoice(&invoice_id).await
}

#[tauri::command]
pub async fn portal_invoice_pay(
    state: State<'_, Arc<AppState>>,
    invoice_id: String,
) -> CmdResult<InvoicePayView> {
    state.portal.invoice_pay(&invoice_id).await
}

#[tauri::command]
pub async fn portal_list_orgs(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Vec<OrganizationView>> {
    state.portal.list_orgs().await
}

/* ------------------------------------------------------------------ */
/* Payment                                                            */
/* ------------------------------------------------------------------ */

/// Validate the gateway URL, persist the non-sensitive recovery record, then
/// open the system browser. Never a WebView, never the payment URL on disk.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn portal_open_payment(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    invoice_id: String,
    payment_url: String,
    mode: String,
    account_id: Option<String>,
    package_id: Option<String>,
    plan_id: Option<String>,
    payment_id: Option<String>,
) -> CmdResult<serde_json::Value> {
    if mode != pending_mode::BUY && mode != pending_mode::RENEW {
        return Err(invalid("نوع عملیات نامعتبر است."));
    }
    let invoice_id = invoice_id.trim().to_string();
    if invoice_id.is_empty() || invoice_id.len() > 64 {
        return Err(invalid("شناسه فاکتور معتبر نیست."));
    }
    validate_payment_url(&payment_url)?;

    let mut pending = PendingPurchase::new(&mode);
    pending.account_id = account_id;
    pending.package_id = package_id;
    pending.plan_id = plan_id;
    pending.invoice_id = Some(invoice_id);
    pending.payment_id = payment_id;
    pending.stage = stage::CREATED.to_string();

    // Persist BEFORE opening: if the browser opens and the app dies, the next
    // launch can still reconcile the invoice.
    state.portal.persist_pending(&pending).await?;
    tracing::info!(
        "opening payment for invoice {} mode {}",
        pending.short_invoice(),
        pending.mode
    );
    open_in_browser(&app, &payment_url)?;
    let _ = state
        .portal
        .set_pending_stage(stage::BROWSER_OPENED)
        .await;
    pending.stage = stage::BROWSER_OPENED.to_string();

    Ok(serde_json::json!({ "opened": true, "pending": pending }))
}

#[tauri::command]
pub async fn portal_reconcile_pending(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<serde_json::Value> {
    state.portal.reconcile_pending().await
}

#[tauri::command]
pub async fn portal_pending_purchase(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Option<PendingPurchase>> {
    Ok(state.portal.pending_purchase().await)
}

#[tauri::command]
pub async fn portal_cancel_pending(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<serde_json::Value> {
    state.portal.cancel_pending().await?;
    Ok(serde_json::json!({ "success": true }))
}

/* ------------------------------------------------------------------ */
/* Apply config                                                       */
/* ------------------------------------------------------------------ */

/// Fetch the account detail in Rust, parse `trojan_url`, save it and mark the
/// config source as HaioGate. The proxy is never enabled implicitly.
#[tauri::command]
pub async fn portal_apply_config(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> CmdResult<serde_json::Value> {
    let detail = state.portal.account_detail(&account_id).await?;
    // A locked credential has no bearer link; it can only be applied through
    // enrollment (`portal_primary_enroll` / `portal_enroll_invite`).
    if detail.base.device_lock {
        return Err(PortalError::new(
            codes::DEVICE_ENROLLMENT_REQUIRED,
            "این اکانت فقط با ثبت دستگاه کار می‌کند. برای اتصال، دستگاه را ثبت کنید.",
        ));
    }
    if detail.trojan_url.trim().is_empty() {
        return Err(PortalError::new(
            codes::INVALID_INPUT,
            "لینک اتصال این اکانت معتبر نیست. با پشتیبانی تماس بگیرید.",
        ));
    }
    let parsed = crate::config::trojan_url::parse_trojan_url(&detail.trojan_url).ok_or_else(|| {
        PortalError::new(
            codes::INVALID_INPUT,
            "لینک اتصال این اکانت معتبر نیست. با پشتیبانی تماس بگیرید.",
        )
    })?;

    // This config is password-only, so any previously enrolled identity is no
    // longer the active credential. Delete its key so it cannot linger.
    forget_device_identity(state.inner()).await;

    let (server, port, enabled) = {
        let mut config = state.config.write().await;
        let state_mut = config.get_mut();
        state_mut.trojan_url = detail.trojan_url.clone();
        state_mut.trojan_config = Some(crate::config::TrojanConfig {
            password: parsed.password.clone(),
            server: parsed.server.clone(),
            port: parsed.port,
            sni: parsed.sni.clone(),
            device_identity_id: None,
        });
        state_mut.device_certificate = None;
        state_mut.config_source = crate::config::ConfigSource::Haiogate;
        state_mut.haiogate_account_id = Some(detail.base.id.clone());
        let enabled = state_mut.enabled;
        config.save().map_err(|_| {
            PortalError::storage("ذخیره تنظیمات اتصال ناموفق بود.")
        })?;
        (parsed.server, parsed.port, enabled)
    };

    // If the tunnel is already running, swap the credentials in place instead
    // of asking the user to toggle. Enabling the proxy itself stays manual.
    if enabled {
        if let Some(tc) = state.config.read().await.get().trojan_config.clone() {
            let trojan = state.trojan.read().await;
            trojan.set(tc).await;
        }
    }

    Ok(serde_json::json!({
        "success": true,
        "server": server,
        "port": port,
    }))
}

/* ------------------------------------------------------------------ */
/* Device enrollment                                                  */
/* ------------------------------------------------------------------ */

fn device_error(err: DeviceError) -> PortalError {
    PortalError::new(err.code, err.message)
}

fn build_device_certificate(
    result: &super::models::EnrollmentApi,
    identity_id: &str,
    device_name: &str,
) -> Result<DeviceCertificate, PortalError> {
    if result.certificate.trim().is_empty()
        || result.device_fingerprint.trim().is_empty()
        || result.account_id.trim().is_empty()
    {
        return Err(PortalError::protocol());
    }
    Ok(DeviceCertificate {
        identity_id: identity_id.to_string(),
        account_id: Some(result.account_id.clone()),
        member_id: result.member_id.clone(),
        certificate_pem: result.certificate.clone(),
        ca_chain_pem: result.ca_chain.clone(),
        fingerprint: result.device_fingerprint.clone(),
        cert_expires_at: result.cert_expires_at.clone(),
        device_name: device_name.to_string(),
        platform: device_identity::platform_name().to_string(),
        enrolled_at: chrono::Utc::now().timestamp(),
    })
}

fn enrollment_result_view(
    result: &super::models::EnrollmentApi,
    identity_id: &str,
    device_name: &str,
) -> EnrollmentResultView {
    let mut view = EnrollmentResultView::from_api(result, identity_id);
    view.device_name = device_name.to_string();
    view.platform = device_identity::platform_name().to_string();
    view
}

/// Persist the certificate metadata and install the certificate-bound Trojan
/// config. Called only after a successful enroll response, so a failed or
/// offline attempt never leaves a partial config on disk.
async fn apply_enrollment(
    state: &AppState,
    meta: &DeviceCertificate,
    credential: &EnrollmentCredentialApi,
    account_id: &str,
) -> Result<(), PortalError> {
    let server = credential.server.trim();
    if server.is_empty()
        || credential.password.is_empty()
        || credential.port == 0
        || credential.port > u16::MAX as u32
    {
        return Err(PortalError::protocol());
    }
    let sni = if credential.sni.trim().is_empty() {
        server.to_string()
    } else {
        credential.sni.trim().to_string()
    };
    let trojan_config = crate::config::TrojanConfig {
        password: credential.password.clone(),
        server: server.to_string(),
        port: credential.port as u16,
        sni,
        device_identity_id: Some(meta.identity_id.clone()),
    };

    let enabled = {
        let mut config = state.config.write().await;
        let state_mut = config.get_mut();
        // No bearer link is stored for a locked credential: the link alone
        // cannot connect and would only invite sharing.
        state_mut.trojan_url = String::new();
        state_mut.trojan_config = Some(trojan_config.clone());
        state_mut.device_certificate = Some(meta.clone());
        state_mut.config_source = crate::config::ConfigSource::Haiogate;
        state_mut.haiogate_account_id = Some(account_id.to_string());
        let enabled = state_mut.enabled;
        config
            .save()
            .map_err(|_| PortalError::storage("ذخیره تنظیمات اتصال ناموفق بود."))?;
        enabled
    };

    if enabled {
        let trojan = state.trojan.read().await;
        trojan.set(trojan_config).await;
    }
    Ok(())
}

fn clear_device_identity_record(state: &AppState, identity_id: &str) {
    if let Err(err) = state.device.forget(identity_id) {
        tracing::warn!(
            "could not delete device key {}: {}",
            identity_id,
            err.code
        );
    }
}

/// Delete the local key + certificate metadata only (no config changes).
/// Used by manual save, reset and account switches.
async fn forget_device_identity(state: &AppState) {
    let existing = {
        let config = state.config.read().await;
        config.get().device_certificate.clone()
    };
    if let Some(meta) = existing {
        clear_device_identity_record(state, &meta.identity_id);
        let mut config = state.config.write().await;
        config.get_mut().device_certificate = None;
        let _ = config.save();
    }
}

/// Full teardown used by logout/reset/delete: drop the keyring entry, the
/// public metadata, and the applied config when it was bound to that identity.
/// Returns true when a device identity existed.
pub(crate) async fn clear_local_device_identity(
    app: &tauri::AppHandle,
    state: &AppState,
    match_account: Option<&str>,
    match_member: Option<&str>,
    disable_if_active: bool,
) -> bool {
    let existing = {
        let config = state.config.read().await;
        config.get().device_certificate.clone()
    };
    let Some(meta) = existing else {
        return false;
    };
    if let Some(account) = match_account {
        if meta.account_id.as_deref() != Some(account) {
            return false;
        }
    }
    if let Some(member) = match_member {
        if meta.member_id.as_deref() != Some(member) {
            return false;
        }
    }

    let enabled = { state.config.read().await.get().enabled };
    if disable_if_active && enabled {
        let _ = crate::app::commands::disable_proxy(app.clone()).await;
    }

    clear_device_identity_record(state, &meta.identity_id);

    {
        let mut config = state.config.write().await;
        let state_mut = config.get_mut();
        state_mut.device_certificate = None;
        let bound = state_mut
            .trojan_config
            .as_ref()
            .map(|tc| tc.device_identity_id.as_deref() == Some(meta.identity_id.as_str()))
            .unwrap_or(false);
        if bound {
            state_mut.trojan_url = String::new();
            state_mut.trojan_config = None;
            state_mut.config_source = crate::config::ConfigSource::Manual;
            state_mut.haiogate_account_id = None;
        }
        let _ = config.save();
    }
    true
}

/// Resolve a usable device name: caller-provided, else the machine name.
fn resolve_device_name(raw: Option<String>) -> String {
    let cleaned = device_identity::clean_device_name(raw.as_deref().unwrap_or_default());
    if cleaned.is_empty() {
        device_identity::default_device_name()
    } else {
        cleaned
    }
}

async fn current_identity_id(state: &AppState) -> Option<String> {
    state
        .config
        .read()
        .await
        .get()
        .device_certificate
        .as_ref()
        .map(|cert| cert.identity_id.clone())
}

/// One keypair per applied credential: delete the previous identity's keyring
/// entry only after the new config has been persisted. A failed enroll must
/// leave the working identity untouched.
fn drop_previous_identity(state: &AppState, previous: Option<&str>, current: &str) {
    if let Some(previous) = previous {
        if previous != current {
            clear_device_identity_record(state, previous);
        }
    }
}

/// Deterministic UUID v5 per identity. The control plane's lost-response
/// replay check requires the same `request_id` and the same SPKI; the key is
/// reused by `ensure_csr`, so a retry after a dropped response is idempotent.
fn enrollment_request_id(identity_id: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, identity_id.as_bytes()).to_string()
}

/// Public guest enrollment: preview (for ids), keypair + keyring write, CSR
/// enroll, then persist and apply. The key is kept across a failed request so
/// a retry can replay the same SPKI + request id; a successful enrollment
/// replaces any previous identity's key.
#[tauri::command]
pub async fn portal_preview_invite(
    state: State<'_, Arc<AppState>>,
    token: String,
) -> CmdResult<InvitePreviewView> {
    state.portal.preview_invite(&token).await
}

#[tauri::command]
pub async fn portal_enroll_invite(
    state: State<'_, Arc<AppState>>,
    token: String,
    device_name: Option<String>,
) -> CmdResult<serde_json::Value> {
    let token = parse_invite_link(&token)?;
    tracing::info!(
        "device enrollment requested for {}",
        super::invite::redact_invite_link(&token)
    );
    let preview = state.portal.preview_invite_token(&token).await?;
    if preview.account_id.is_empty() {
        return Err(PortalError::protocol());
    }
    let device_name = resolve_device_name(device_name);
    let identity_id =
        device_identity::identity_id_for(&preview.account_id, preview.member_id.as_deref());
    let previous_identity = current_identity_id(state.inner()).await;

    let prepared = state.device.ensure_csr(&identity_id).map_err(device_error)?;
    let request_id = enrollment_request_id(&identity_id);
    let result = state
        .portal
        .enroll_invite(&token, &prepared.csr_pem, &device_name, &request_id)
        .await?;

    let meta = build_device_certificate(&result, &identity_id, &device_name)?;
    state.device.commit(&meta).map_err(device_error)?;
    let credential = result.credential.clone().ok_or_else(PortalError::protocol)?;
    apply_enrollment(state.inner(), &meta, &credential, &result.account_id).await?;
    drop_previous_identity(state.inner(), previous_identity.as_deref(), &identity_id);

    Ok(serde_json::json!({
        "enrollment": enrollment_result_view(&result, &identity_id, &device_name),
        "accountId": result.account_id,
    }))
}

#[tauri::command]
pub async fn portal_primary_invite(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    ttl_hours: Option<u32>,
) -> CmdResult<InviteView> {
    state.portal.primary_invite(&account_id, ttl_hours).await
}

#[tauri::command]
pub async fn portal_member_invite(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    member_id: String,
    ttl_hours: Option<u32>,
) -> CmdResult<InviteView> {
    state
        .portal
        .member_invite(&account_id, &member_id, ttl_hours)
        .await
}

#[tauri::command]
pub async fn portal_primary_enroll(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    device_name: Option<String>,
) -> CmdResult<serde_json::Value> {
    let account_id = super::validate_id("اکانت", &account_id)?;
    let device_name = resolve_device_name(device_name);
    let identity_id = device_identity::identity_id_for(&account_id, None);
    let previous_identity = current_identity_id(state.inner()).await;

    let prepared = state.device.ensure_csr(&identity_id).map_err(device_error)?;
    let request_id = enrollment_request_id(&identity_id);
    let result = state
        .portal
        .primary_enroll(&account_id, &prepared.csr_pem, &device_name, &request_id)
        .await?;

    let meta = build_device_certificate(&result, &identity_id, &device_name)?;
    state.device.commit(&meta).map_err(device_error)?;
    let credential = result.credential.clone().ok_or_else(PortalError::protocol)?;
    apply_enrollment(state.inner(), &meta, &credential, &account_id).await?;
    drop_previous_identity(state.inner(), previous_identity.as_deref(), &identity_id);

    Ok(serde_json::json!({
        "enrollment": enrollment_result_view(&result, &identity_id, &device_name),
        "accountId": account_id,
    }))
}

#[tauri::command]
pub async fn portal_primary_reset_device(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    account_id: String,
    reason: Option<String>,
) -> CmdResult<serde_json::Value> {
    let account_id = super::validate_id("اکانت", &account_id)?;
    let revoked = state
        .portal
        .primary_reset_device(&account_id, reason.as_deref().unwrap_or(""))
        .await?;
    let device_cleared = clear_local_device_identity(
        &app,
        state.inner(),
        Some(&account_id),
        None,
        true,
    )
    .await;
    Ok(serde_json::json!({
        "reset": true,
        "devicesRevoked": revoked,
        "deviceCleared": device_cleared,
    }))
}

#[tauri::command]
pub async fn portal_member_reset_device(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    account_id: String,
    member_id: String,
    reason: Option<String>,
) -> CmdResult<serde_json::Value> {
    let revoked = state
        .portal
        .member_reset_device(
            &account_id,
            &member_id,
            reason.as_deref().unwrap_or(""),
        )
        .await?;
    let device_cleared = clear_local_device_identity(
        &app,
        state.inner(),
        Some(&account_id),
        Some(&member_id),
        true,
    )
    .await;
    Ok(serde_json::json!({
        "reset": true,
        "devicesRevoked": revoked,
        "deviceCleared": device_cleared,
    }))
}

#[tauri::command]
pub async fn portal_list_invites(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> CmdResult<Vec<PendingInviteView>> {
    state.portal.list_invites(&account_id).await
}

#[tauri::command]
pub async fn portal_cancel_invite(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    invite_id: String,
) -> CmdResult<serde_json::Value> {
    state.portal.cancel_invite(&account_id, &invite_id).await?;
    Ok(serde_json::json!({ "success": true, "revoked": true }))
}

#[tauri::command]
pub async fn portal_device_identity(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Option<DeviceIdentityView>> {
    let config = state.config.read().await;
    Ok(config.get().device_certificate.as_ref().map(|cert| {
        DeviceIdentityView::from_certificate(cert, state.device.is_persistent())
    }))
}

#[tauri::command]
pub async fn portal_forget_device_identity(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> CmdResult<serde_json::Value> {
    let cleared = clear_local_device_identity(&app, state.inner(), None, None, true).await;
    Ok(serde_json::json!({ "success": true, "deviceCleared": cleared }))
}

/* ------------------------------------------------------------------ */
/* Members                                                            */
/* ------------------------------------------------------------------ */

#[tauri::command]
pub async fn portal_add_member(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    display_name: String,
    mobile: Option<String>,
    email: Option<String>,
    issue_invite: Option<bool>,
) -> CmdResult<MemberCreateView> {
    state
        .portal
        .add_member(
            &account_id,
            &display_name,
            mobile.as_deref(),
            email.as_deref(),
            issue_invite.unwrap_or(false),
        )
        .await
}

#[tauri::command]
pub async fn portal_rotate_member(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    account_id: String,
    member_id: String,
) -> CmdResult<MemberCreateView> {
    let rotated = state.portal.rotate_member(&account_id, &member_id).await?;
    // Device-lock rotate revokes the old device and mints a new invite; the
    // local key belongs to the revoked device if it is this machine.
    if rotated.invite_required {
        let _ = clear_local_device_identity(
            &app,
            state.inner(),
            Some(&account_id),
            Some(&member_id),
            true,
        )
        .await;
    }
    Ok(rotated)
}

#[tauri::command]
pub async fn portal_revoke_member(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    account_id: String,
    member_id: String,
) -> CmdResult<serde_json::Value> {
    state.portal.revoke_member(&account_id, &member_id).await?;
    let device_cleared = clear_local_device_identity(
        &app,
        state.inner(),
        Some(&account_id),
        Some(&member_id),
        true,
    )
    .await;
    Ok(serde_json::json!({
        "success": true,
        "revoked": true,
        "deviceCleared": device_cleared,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrollment_request_id_is_a_stable_uuid() {
        let first = enrollment_request_id("p_acc");
        let second = enrollment_request_id("p_acc");
        let other = enrollment_request_id("m_mem");
        assert_eq!(first, second, "retries must replay the same request id");
        assert_ne!(first, other);
        assert!(uuid::Uuid::parse_str(&first).is_ok());
    }

    #[test]
    fn device_errors_keep_their_code() {
        let err = device_error(DeviceError::keyring_unavailable());
        assert_eq!(err.code, "device_keyring_unavailable");
        assert!(err.message.contains("ذخیره‌سازی"));
    }
}
