//! `#[tauri::command]` wrappers for the portal service.
//!
//! Every command returns `Result<T, PortalError>`; the frontend branches on
//! `error.code`. No command returns tokens, the national code, or a payment URL
//! that failed validation.

use super::client::validate_payment_url;
use super::error::{codes, PortalError};
use super::models::{
    AccountDetailView, AccountView, BuyResponse, InvoicePayView, InvoiceView, OrganizationView,
    PackageView, PurchaseIntentView, SessionView, ShahkarView, UserView,
};
use super::pending::{mode as pending_mode, stage, PendingPurchase};
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
pub async fn portal_logout(state: State<'_, Arc<AppState>>) -> CmdResult<serde_json::Value> {
    state.portal.logout().await;
    Ok(serde_json::json!({ "success": true }))
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

    let (server, port, enabled) = {
        let mut config = state.config.write().await;
        let state_mut = config.get_mut();
        state_mut.trojan_url = detail.trojan_url.clone();
        state_mut.trojan_config = Some(crate::config::TrojanConfig {
            password: parsed.password.clone(),
            server: parsed.server.clone(),
            port: parsed.port,
            sni: parsed.sni.clone(),
        });
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
/* Members                                                            */
/* ------------------------------------------------------------------ */

#[tauri::command]
pub async fn portal_add_member(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    display_name: String,
    mobile: Option<String>,
    email: Option<String>,
) -> CmdResult<super::models::MemberView> {
    state
        .portal
        .add_member(
            &account_id,
            &display_name,
            mobile.as_deref(),
            email.as_deref(),
        )
        .await
}

#[tauri::command]
pub async fn portal_rotate_member(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    member_id: String,
) -> CmdResult<super::models::MemberView> {
    state.portal.rotate_member(&account_id, &member_id).await
}

#[tauri::command]
pub async fn portal_revoke_member(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    member_id: String,
) -> CmdResult<serde_json::Value> {
    state.portal.revoke_member(&account_id, &member_id).await?;
    Ok(serde_json::json!({ "success": true, "revoked": true }))
}
