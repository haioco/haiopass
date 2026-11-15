# Plan — Desktop OTP Login, HaioGate Accounts, Renewal, Purchase, Shahkar, and Payment

**Status:** Proposal for approval. **No implementation has been started.**
**Date:** 2026-11-07
**Platform:** Desktop — Tauri v2 (`/home/devcloud/Documents/mine/Haio-Forever/HaioBypass-app`)
**Backend:** Existing HaioCloud control plane (`portal.haio.ir`); shares the backend renewal fix with the Android plan.
**Companion plan:** `../HaioBypass-android/PLAN-OTP-HAIOGATE-ANDROID-FOCUSED.md` — the Android
implementation of the same product flow. This document is the desktop scope of record.
**No application code, Rust module, HTML, CSS, or JavaScript has been changed.**

---

## 1. Scope

### In scope

1. **OTP login** with the HaioCloud portal: mobile number → SMS OTP → access/refresh tokens →
   persistent desktop session → logout.
2. **List the user's HaioGate accounts**, grouped into active, expired/ended, and
   support-only.
3. **Renew an expired HaioGate account**, keeping the same username/password and VPN config.
4. **Renew an active account early** as a queued reserve, with a visible projected expiry.
5. **Buy a new HaioGate package** from inside the desktop app.
6. **Shahkar gate and in-app national-code verification** before buy/renew.
7. **Pay through the existing Zibal gateway**, opened in the system browser, with pending
   purchase persistence, polling, and recovery after app restart.
8. **Apply the HaioGate connection config** to the existing in-process Trojan/proxy engine.

### Out of scope

- Android implementation (separate plan; same API contract).
- The cloud companion / super-app strategy.
- Migrating legacy HaioBypass accounts or the legacy Android store.
- In-app wallet, coupons, credit purchase, refunds, or invoice PDF download.
- A full web-console replacement, cloud resource management, or billing admin.
- Embedded payment WebView, a new payment provider, or card storage.
- Public package catalog before login.
- Deep-link payment return (`haiobypass://payment`) in the MVP; Phase 2 only.
- Rewriting the VPN engine, proxy, OS proxy, routing, tray, updater, or autostart.

### Non-negotiable product rules

- The portal (`portal.haio.ir`) is the single identity for OTP, Shahkar, HaioGate, and billing.
- Login is optional for the VPN; it is required only for HaioGate and account screens.
- Shahkar is enforced before **buy** and **renew**, not at app login.
- The Shahkar mobile is read-only and locked to the logged-in account.
- The app never stores OTPs, national codes, payment URLs, portal tokens, or HaioGate
  passwords in `state.json`, logs, analytics, or crash reports.
- Portal tokens live in the OS keyring, not in the webview or plain files.
- Payment opens in the system browser; the app never renders the gateway in a WebView.
- The app never calls the destructive Zibal callback endpoint for polling.

---

## 2. Executive summary

The desktop app already has a working Tauri v2 shell, an in-process Trojan client, a local HTTP
proxy, OS proxy integration, a domain router, a tray, autostart, and an auto-updater. It has
**no account system, no portal API client, and no store**. Its only external HTTP call is the
domain-list fetch; the “Buy Config” banner links to `console.haio.ir`, and onboarding expects
a pasted `haio://` access key.

The backend already provides OTP, Shahkar, HaioGate packages/accounts/renewal, invoices, and
Zibal payment. The desktop work is therefore:

1. A Rust **portal module** that owns the portal HTTP client, keyring-backed session, token
   refresh, and typed commands.
2. A **HaioGate tab** in the existing SPA with login, account list/detail, renew, buy, Shahkar,
   and payment states.
3. A **config-source** addition so applying a HaioGate config is not confused with a manually
   pasted key.

The webview has a strict CSP (`connect-src ipc: http://ipc.localhost`). Do **not** broaden it to
call `portal.haio.ir` from JavaScript: that would expose tokens to the webview and add CORS
failure modes. All portal calls go through Rust `invoke` commands. The frontend never sees an
access or refresh token.

**Recommended release shape**

- Add a fourth bottom tab: **هایو گیت**. Tabs become: خانه، هایو گیت، عبور مستقیم، تنظیمات.
- Keep manual access-key onboarding as the first path; add “ورود و خرید هایو گیت” as a second
  path.
- Replace the external `console.haio.ir` buy banner with an in-app HaioGate action, keeping an
  external “مشاهده در پنل” link as fallback.
- Add a header account chip for login/account/logout.
- Buy and renew follow:
  `login → Shahkar (if needed) → buy/renew intent → buy plan → system browser → poll invoice →
  refresh account → apply config`.
- Add `config_source` (`manual` | `haiogate`) and `haiogate_account_id` to `config::State`.
- Store portal tokens in the OS keyring; if the keyring is unavailable, keep the session in
  memory only and tell the user it will not survive restart.

**Rough effort:** 20–26 engineer-days including QA, plus 1–2 shared backend days for the
renewal fix. Desktop release `2.1.0`.

---

## 3. Current desktop state

### 3.1 Stack

- Tauri v2, Rust 2021, plain HTML/CSS/JavaScript frontend built with Vite 5.
- Version `2.0.6`; identifier `ir.haio.bypass`; window 440×700 (min 360×540), resizable,
  centered.
- `withGlobalTauri: true`; frontend calls `window.__TAURI__.core.invoke` through
  `src/scripts/invoke.js`.
- `Cargo.toml` already has `reqwest 0.12` (rustls), `tokio` full, `serde`, `serde_json`,
  `chrono`, `keyring 3`, and `tauri-plugin-shell`.
- `keyring` is declared but **not used anywhere** in `src-tauri/src`; there is no portal code.

### 3.2 Existing state and config

`src-tauri/src/config/mod.rs` stores `State` as camelCase JSON in
`~/.haiobypass/state.json` with `fs::write` (pretty JSON, no explicit permission hardening).
Fields include `enabled`, `proxy_port`, `http_proxy_port`, `trojan_url`, `trojan_config`,
`cached_domains`, `bypass_domains`, `enabled_presets`, `proxy_consent`, `block_quic`,
`autostart`, `minimize_to_tray`, and update metadata.

There is no `config_source`, account id, portal session, pending purchase, or user cache.

### 3.3 Existing commands

`src-tauri/src/app/commands.rs` exposes:

`enable_proxy`, `disable_proxy`, `get_status`, `test_connection`, `save_config`,
`delete_config`, `get_state`, `set_state`, bypass commands, `connect_trojan`, presets,
`refresh_domains`, `set_port`, `set_autostart`, `set_minimize_to_tray`, `set_proxy_consent`,
`set_quic_block`, `set_proxy_port`, update commands, and `quit_and_restore`.

`save_config` parses `trojan://` and `haio://` via `config/trojan_url.rs`, stores `trojan_url`
and `trojan_config`, and saves. `delete_config` disables the proxy then clears both. The parser
already accepts the HaioGate URL shape
`trojan://<urlencoded-password>@tahrim.haiocloud.com:443?sni=...#HaioGate`.

`set_state` accepts a full `State` from the frontend; this is a mass-assignment risk for any new
sensitive field and must be constrained when `config_source` is added.

### 3.4 Existing frontend

- `src/index.html`: onboarding card (paste access key) and dashboard with three views
  (`home`, `bypass`, `settings`) plus a three-item tab bar.
- `src/scripts/main.js` (1,033 lines): tab switching, onboarding, config save/delete, proxy
  toggle, domains, bypass list, updates, status and health handling.
- `src/scripts/settings.js`: presets, autostart, tray, QUIC, OS-proxy consent.
- `src/scripts/i18n.js`: `fa` and `en` translation objects.
- `src/styles/style.css` (1,571 lines): design tokens, cards, tabs, modals, toasts, responsive
  layout. Reuse these tokens; do not introduce a second design system.
- `VALID_TABS = ['home', 'bypass', 'settings']`; the tab bar is a flex row, so adding a fourth
  item is structurally easy. Labels are 9.8px and may need a small size/gap adjustment.
- The home view has a `<a class="buy-banner" href="https://console.haio.ir">`; this is the only
  store entry point today.

### 3.5 Security constraints already present

- CSP: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'
  data:; connect-src ipc: http://ipc.localhost`.
- `capabilities/default.json` permits `core:default` and `shell:allow-open`.
- No deep-link plugin and no HTTP plugin.
- `tracing` is present; the current app logs operational events, not request bodies.

---

## 4. Decisions

| # | Decision | Recommendation | Reason |
|---|---|---|---|
| D1 | Where does portal HTTP run? | **Rust only**, through typed Tauri commands. | CSP blocks external HTTP; keeping tokens out of JS is safer. |
| D2 | What identity system? | **Portal** (`portal.haio.ir`). | OTP, Shahkar, Zibal, invoices, and HaioGate are implemented there. |
| D3 | Is login mandatory? | **No.** Required only for the HaioGate tab and account actions. | Manual access-key users keep working. |
| D4 | How is the store presented? | **Fourth tab “هایو گیت”**, not a modal-only flow. | It needs lists, detail, packages, and payment states; 440×700 is too tight for nested modals. |
| D5 | Where are tokens stored? | **OS keyring**; memory-only fallback if unavailable. | `keyring` is already a dependency; never put tokens in `state.json`. |
| D6 | How does payment return? | **System browser + pending purchase + invoice polling on window focus/startup.** | No backend return-URL change; survives app restart. |
| D7 | Can active accounts renew early? | **Yes**, queued as a reserve with projected expiry. | Matches backend behavior; useful for prepayment. |
| D8 | Can suspended accounts self-renew? | **No**, route to support. | Suspension may be administrative; self-service could bypass support. |
| D9 | Where does the applied config come from? | **Manual paste or HaioGate**, tracked by `config_source`. | Prevents confusion and enables the right home summary. |
| D10 | Deep link in MVP? | **No.** | Tauri deep-link plugin, manifest registration, and backend return-URL work are a separate phase. |
| D11 | Multiple accounts on one package? | **Require the shared backend renewal fix.** | Otherwise the backend renews the newest account, not the selected one. |

---

## 5. Shared backend prerequisite

The desktop and Android apps use the same portal API. The backend fix described in the Android
plan §7 is a prerequisite for reliable renewal when a user has **more than one account on the
same package**:

- `POST /antisanction/{id}/renew/` currently records only a package intent, and only when the
  package changes.
- The subscription signal selects `AntiSanctionAccount.objects.filter(user=user, package=package).order_by('-created_at').first()`.
- A user with two accounts on the same package who renews the older one may therefore renew the
  newer one instead.
- Fix: write `meta_data_json.pending_renewal = {account_id, package_id, created_at}` from the
  renew endpoint, honor and validate it in the signal, clear it (plus legacy
  `pending_package_id`) after success, and fall back safely for stale/foreign intents.

This is **1–2 backend days**, shared with Android; the desktop plan does not require a second
backend change. If the fix is not approved, the desktop app must:

- Allow renewal only when the user has one account per package.
- Disable self-service renewal for duplicate-package accounts and show
  «برای تمدید این اکانت با پشتیبانی تماس بگیرید.»

No other backend work is required for OTP, Shahkar, buy, renew, invoice polling, or Zibal.

Optional follow-ups (not MVP): non-destructive payment-status endpoint, per-payment return URL,
buy idempotency key, public package catalog, server-side account status filter.

---

## 6. Desktop architecture

### 6.1 Component boundaries

```text
┌─────────────────────────────────────────────────────────────┐
│ Frontend (HTML/CSS/JS, Vite, CSP self)                      │
│  index.html + main.js + portal.js + i18n.js + style.css     │
│  - renders login, accounts, detail, packages, payment       │
│  - never sees access/refresh tokens                         │
└───────────────────────────┬─────────────────────────────────┘
                            │ invoke(...) / events
┌───────────────────────────▼─────────────────────────────────┐
│ Tauri commands (Rust)                                       │
│  portal::commands                                           │
│  - request_otp, login_otp, session, logout, me              │
│  - shahkar_verify                                           │
│  - list_packages, list_accounts, account_detail, sync_quota │
│  - start_buy, start_renew, buy_plan, invoice, invoice_pay   │
│  - list_orgs, open_payment, reconcile_pending                │
│  - apply_config, pending_purchase, cancel_pending            │
└───────────────────────────┬─────────────────────────────────┘
                            │ reqwest (rustls)
┌───────────────────────────▼─────────────────────────────────┐
│ Portal service (Rust)                                       │
│  client.rs   - reqwest client, base URL, auth header        │
│  session.rs  - in-memory tokens + single-flight refresh     │
│  keyring.rs  - OS keyring persistence, fallback             │
│  models.rs   - API DTOs + UI DTOs                           │
│  pending.rs  - pending purchase JSON (non-sensitive)        │
│  error.rs    - typed, serializable errors                   │
└───────────────────────────┬─────────────────────────────────┘
                            │
                  https://portal.haio.ir/api/v1/
```

### 6.2 Ownership rules

- **Rust owns** tokens, refresh, HTTP, pending purchase, payment URL validation, and parsing the
  HaioGate `trojan_url`.
- **JavaScript owns** presentation, form validation hints, countdown display, tab state, and
  rendering. It may validate mobile/national-code format before calling Rust, but Rust must
  validate again.
- **`config::State` owns** the applied VPN config and `config_source`; portal tokens are never
  written there.
- **`portal_pending.json` owns** only non-sensitive purchase recovery data.

### 6.3 Request flow

1. JS calls `invoke('portal_login_otp', { mobile, otp })`.
2. Rust calls the portal, receives `{access, refresh, user}`.
3. Rust writes tokens to the keyring (or memory fallback), stores them in `SessionManager`, and
   returns only the sanitized `User`.
4. JS renders the account screen. It cannot read the tokens.
5. Later calls use the in-memory access token; on 401 Rust refreshes once, persists the rotated
   refresh token, and retries.
6. On startup Rust loads the keyring, validates `/auth/me/`, refreshes if needed, and returns a
   sanitized session or `null`.

### 6.4 Blocking work

Keyring reads/writes and filesystem writes are blocking. Run them with
`tauri::async_runtime::spawn_blocking` so the Tauri command executor and UI stay responsive.

### 6.5 Failure isolation

- Portal failures must never affect the proxy, OS proxy, domains, tray, or updater.
- The portal module is additive in `AppState`; if initialization fails, the VPN still starts and
  the HaioGate tab shows a retry state.
- A corrupt keyring entry or pending file is cleared with a logged warning, never a panic.

---

## 7. Portal API contract

Base URL: `https://portal.haio.ir/api/v1/`.

### 7.1 Auth

| Method | Path | Body | Response | Notes |
|---|---|---|---|---|
| POST | `auth/otp/request/` | `{mobile}` | `{sent, expires_in, retry_after, provider, raw}` | 429 rate limit; 502 SMS provider |
| POST | `auth/otp/login/` | `{mobile, otp}` | `{access, refresh, user}` | Auto-creates user; 400 invalid/expired; 429 lockout |
| POST | `auth/refresh/` | `{refresh}` | `{access, refresh}` | Rotation enabled; persist both |
| GET | `auth/me/` | — | `User` | Session validation |
| POST | `auth/shahkar/verify/` | `{mobile, national_id}` | `{verified, shahkar_status, reason, api_call_status_code, identified_response_code, user}` | Status 0/1/2/3 |

### 7.2 HaioGate

| Method | Path | Body | Response | Notes |
|---|---|---|---|---|
| GET | `antisanction/packages/?page_size=100` | — | paginated `Package` | Auth required |
| GET | `antisanction/?page_size=100` | — | paginated `Account` | All statuses for current user |
| GET | `antisanction/{id}/` | — | `AccountDetail` | Includes password, trojan URL, reserves |
| GET | `antisanction/{id}/quota/` | — | `AccountDetail` | Triggers proxy quota sync |
| POST | `antisanction/` | `{package_id}` | `{plan_id, package_id, is_free, detail}` | Auth + Shahkar; does not create account |
| POST | `antisanction/{id}/renew/` | `{package_id?}` | `{plan_id, package_id, is_free, detail}` | Auth + Shahkar; records selected account after fix |

### 7.3 Billing and organizations

| Method | Path | Body | Response | Notes |
|---|---|---|---|---|
| POST | `billing/subscriptions/buy/` | `{plan_id, organization_id?, coupon?, use_credit?}` | `{status, payment_url?, payment_id?, invoice_id?, ...}` | Auth + Shahkar; `active` or `payment_required` |
| GET | `billing/invoices/{id}/` | — | `Invoice` | Safe to poll |
| GET | `billing/invoices/?page_size=100` | — | paginated `Invoice` | Ambiguous-buy recovery |
| POST | `billing/invoices/{id}/pay/` | `{use_credit?}` | `{payment_url, payment_id, invoice_id}` | Resume a pending invoice |
| GET | `orgs/?page_size=100` | — | paginated `Organization` | Multi-org picker |
| GET | `billing/payments/verify/?pid=` | — | redirect | **Gateway callback — never poll** |

### 7.4 Important backend behavior

- `GET antisanction/` returns all accounts, paginated. The desktop groups them.
- Status values: `active`, `expired`, `quota_exhausted`, `suspended`.
- Renewing an active, not-over-quota account queues an `AntiSanctionReserve`; current expiry
  does not move. Detail exposes `pending_reserves` and `projected_expired_at`.
- Renewing expired/quota-exhausted immediately reactivates: +30 days, usage reset.
- Buying a package the user already owns causes the post-payment signal to renew that account
  instead of creating a duplicate.
- `GET billing/payments/verify/` is the Zibal callback and can mark a pending payment failed;
  the app must poll `billing/invoices/{invoice_id}/` instead.

### 7.5 Error envelope

DRF can return `{detail}`, `{field: ["..."]}`, or `{detail, code}`. Map these in Rust to a typed
`PortalError`:

| Code | Meaning | UI action |
|---|---|---|
| `shahkar_required` | 403; Shahkar not verified | Open Shahkar, remember action, retry once |
| `mobile_required` | 400; no valid mobile | Explain, support |
| `organization_required` | 400; multiple orgs | Show org picker, retry with `organization_id` |
| `rate_limited` | 429 | Countdown using `retry_after` / `Retry-After` |
| `payment_required` | Not an error | Continue to browser |

Never surface raw provider codes or stack traces to the user.

---

## 8. Rust portal module

### 8.1 Module layout

Create `src-tauri/src/portal/`:

| File | Responsibility |
|---|---|
| `mod.rs` | `PortalService`, shared state, re-exports |
| `client.rs` | `reqwest::Client`, base URL, authorized request helper, 401 retry |
| `models.rs` | API DTOs and sanitized UI DTOs |
| `session.rs` | In-memory token state, refresh lock, login/load/clear |
| `keyring_store.rs` | OS keyring persistence, memory fallback |
| `pending.rs` | `portal_pending.json` read/write/clear |
| `error.rs` | `PortalError` serializable error type and mapping |
| `commands.rs` | `#[tauri::command]` functions registered in `lib.rs` |

Add `portal: Arc<PortalService>` to `AppState` in `lib.rs`.

### 8.2 Models

API models use snake_case with `serde` renames. UI models use camelCase so the JavaScript is
consistent with the rest of the app.

`PortalUser`

```text
id: i64
username: Option<String>
email: Option<String>
first_name: String
last_name: String
role: String
display_name: String
is_verified: bool
onboarding_completed: bool
mobile: Option<String>
shahkar_status: i32
national_id: Option<String>  // never sent to JS, never persisted
created_at: String
```

`UserView` sent to JS omits `national_id`. It includes `mobile`, `shahkar_status`, and
`shahkar_verified: bool`.

`PackageView`: `id`, `name`, `slug`, `description`, `price`, `traffic_bytes`, `is_free`,
`status`, `sort_order`.

`AccountView`: `id`, `package`, `package_name`, `username`, `total_traffic_bytes`,
`usage_bytes`, `usage_display`, `traffic_is_over`, `expired_at`, `status`, `status_display`,
`last_sync`, `is_legacy`, `pending_reserves_count`, `created_at`, `updated_at`, plus a
Rust-computed `category: "active" | "expired" | "support"`.

`AccountDetailView` adds `password`, `trojan_url`, `usage_percent`, `pending_reserves`,
`projected_expired_at`, `meta_data_json` (sanitized or omitted).

`ReserveView`: `id`, `package`, `package_name`, `traffic_bytes`, `created_at`.

`InvoiceView`: `id`, `organization`, `plan` (`{id, name, slug}` or null), `subscription`,
`amount`, `original_amount`, `discount_amount`, `discount_percent`, `status`, `due_date`,
`paid_at`, `description`, `created_at`.

`BuyResponse`: `status`, `payment_url`, `payment_id`, `invoice_id`, `subscription_id`,
`credit_applied`, `paid_by`.

`OrganizationView`: `id`, `name`, `slug`, `owner`, `owner_email`, `my_role`, `created_at`.

Do not expose `national_id`, tokens, raw `meta_data_json`, or gateway responses to JS.

### 8.3 HTTP client

- `reqwest::Client` built once in `PortalService::new`:
  - `rustls-tls`, HTTP/2 enabled, 20s connect/read timeout.
  - `User-Agent: HaioBypass/2.1.0 (desktop)`.
  - No cookie jar; bearer auth only.
  - Redirect policy: limited to 3; do not follow redirects for API POSTs (especially buy).
- Base URL from a constant or build-time env:
  `const PORTAL_BASE_URL: &str = "https://portal.haio.ir/api/v1/";`
  Allow an env override for staging:
  `HAIO_PORTAL_BASE_URL`, validated to end with `/`.
- Helper `send_json<T>`:
  1. Add `Authorization: Bearer` if authenticated.
  2. Send JSON.
  3. If 401 and authenticated: refresh once, rebuild request, retry once.
  4. If still 401: clear session, return `PortalError::SessionExpired`.
  5. On 2xx: deserialize; map parse errors to `PortalError::Protocol`.
  6. On non-2xx: read body, parse DRF error, map status/code.
- Do not retry non-idempotent POSTs at the HTTP layer. The buy coordinator handles ambiguous
  outcomes by listing invoices, not by blind retry.
- Do not log request/response bodies. `tracing` may log method, path, status, and duration.

### 8.4 Session manager

```rust
pub struct SessionManager {
    inner: tokio::sync::Mutex<Option<Tokens>>,
    refresh_lock: tokio::sync::Mutex<()>,
    store: Arc<dyn TokenStore>,
}
```

- `Tokens { access: String, refresh: String }`.
- `load()`: read keyring JSON, set memory. Never return tokens to the caller.
- `save(tokens)`: memory first, then keyring in `spawn_blocking`; if keyring fails, keep memory
  and mark `persistent = false`.
- `ensure_access_token()`: return current access.
- `refresh()`:
  1. Acquire `refresh_lock`.
  2. Re-read memory; another task may already have refreshed.
  3. Call `POST auth/refresh/` with the current refresh token.
  4. Persist **both** new access and refresh atomically.
  5. On failure, clear memory and keyring; return `SessionExpired`.
- Single-flight behavior prevents multiple concurrent 401s from rotating the refresh token
  twice and invalidating each other.
- `logout()`: clear memory and keyring; keep the non-sensitive pending purchase for recovery,
  but hide it until the user logs in again. `portal_cancel_pending` is the explicit clear.
- `session_view()`: return `{loggedIn, user?, persistent}`; never tokens.

### 8.5 Keyring storage

- Service name: `ir.haio.bypass.portal`; account name: `session`.
- Store a JSON string `{"access":"...","refresh":"..."}`.
- Use `keyring::Entry::new(service, user)` and `set_password` / `get_password` /
  `delete_credential`.
- Run blocking calls via `tauri::async_runtime::spawn_blocking`.
- The current `keyring = "3"` dependency has **no store feature enabled**, so it falls back to
  the mock store; this must be fixed before tokens can persist.
- Enable platform-native keyring features in `Cargo.toml`:
  - Windows: `windows-native`
  - macOS: `apple-native`
  - Linux: `sync-secret-service` + `crypto-rust` (Secret Service; `vendored` optional to
    statically link libdbus)
- Do **not** treat `linux-native` as a persistent fallback: it uses the kernel session keyring,
  which does not survive a reboot. If Secret Service is unavailable, use the explicit
  memory-only fallback below.
- Fallback policy:
  - At startup, check `keyring::default::default_credential_builder().persistence()`. If it is
    not `CredentialPersistence::UntilDelete`, the build is using the mock store; set
    `persistent: false` and show the notice instead of silently pretending to persist.
  - If the keyring is unavailable, keep tokens in memory only.
  - Show a non-blocking notice: «ورود شما تا بستن برنامه حفظ می‌شود؛ ذخیره‌سازی امن سیستم در
    دسترس نیست.»
  - Never write tokens to `state.json`, a plain file, or `localStorage`.
- Keyring errors must not expose the token or the underlying secret-service error text to the
  UI; log a short error code only.

### 8.6 Pending purchase store

`~/.haiobypass/portal_pending.json`, permission `0600` on Unix:

```json
{
  "mode": "buy",
  "accountId": null,
  "packageId": "...",
  "planId": "...",
  "invoiceId": "...",
  "paymentId": "...",
  "createdAt": 1760000000,
  "stage": "browser_opened"
}
```

- `stage`: `created` | `browser_opened` | `paid` | `provisioning` | `done` | `failed`.
- Never store `payment_url`, tokens, mobile, national code, passwords, or trojan URLs.
- Atomic write: write to `portal_pending.json.tmp`, `fsync`, rename.
- Clear when terminal (done/failed/canceled) or older than 24 hours.
- On a corrupt file, log and delete it.

### 8.7 Tauri commands

Register in `lib.rs`:

| Command | Input | Output |
|---|---|---|
| `portal_session` | — | `{loggedIn, user?, persistent, restoring?}` |
| `portal_request_otp` | `{mobile}` | `{sent, expiresIn, retryAfter, provider}` |
| `portal_login_otp` | `{mobile, otp}` | `{user: UserView, persistent}` |
| `portal_logout` | — | `{success}` |
| `portal_me` | — | `UserView` |
| `portal_shahkar_verify` | `{nationalId}` | `{verified, shahkarStatus, user: UserView}` |
| `portal_list_packages` | — | `PackageView[]` |
| `portal_list_accounts` | — | `AccountView[]` |
| `portal_account_detail` | `{accountId}` | `AccountDetailView` |
| `portal_sync_quota` | `{accountId}` | `AccountDetailView` |
| `portal_start_buy` | `{packageId}` | `{planId, packageId, isFree}` |
| `portal_start_renew` | `{accountId, packageId?}` | `{planId, packageId, isFree}` |
| `portal_buy_plan` | `{planId, organizationId?}` | `BuyResponse` |
| `portal_invoice` | `{invoiceId}` | `InvoiceView` |
| `portal_invoice_pay` | `{invoiceId}` | `{paymentUrl, paymentId, invoiceId}` |
| `portal_list_orgs` | — | `OrganizationView[]` |
| `portal_open_payment` | `{invoiceId, paymentUrl, mode, accountId?, packageId?, planId?, paymentId?}` | `{opened, pending}` |
| `portal_reconcile_pending` | — | `{state, invoice?, accounts?}` |
| `portal_pending_purchase` | — | pending or null |
| `portal_cancel_pending` | — | `{success}` |
| `portal_apply_config` | `{accountId}` | `{success, server, port}` |

Rules:

- Commands return structured `PortalError`, not `String`, so the UI can branch on `code`.
- `portal_login_otp` never returns tokens.
- `portal_open_payment` validates the URL and persists the pending purchase **before** opening
  the browser.
- `portal_apply_config` fetches the account detail in Rust, parses `trojan_url` with the existing
  parser, writes the config, and sets `config_source = haiogate`.
- `portal_reconcile_pending` never calls `billing/payments/verify/`.
- `portal_cancel_pending` clears the pending file; it does not cancel a paid invoice.

### 8.8 `AppState` and initialization

- Add `portal: Arc<PortalService>` to `AppState`.
- Initialize before `tauri::Builder::manage`, but do not fail app startup if keyring
  initialization fails.
- On `setup`, spawn a non-blocking `portal::session::restore` that:
  1. Loads keyring tokens.
  2. Calls `/auth/me/` with refresh-if-needed.
  3. Emits `portal:session` to the frontend with the sanitized result.
- If restore fails, emit `{loggedIn:false, reason}`; do not show a blocking error on launch.
- `portal_session` returns `{loggedIn:false, restoring:true}` until restore completes; the
  frontend shows a small skeleton in the account chip/HaioGate tab and updates on
  `portal:session`, avoiding a login flash on every launch.

---

## 9. Config source and home integration

### 9.1 `config::State` additions

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConfigSource {
    Manual,
    Haiogate,
}

impl Default for ConfigSource {
    fn default() -> Self { Self::Manual }
}
```

Add:

```rust
#[serde(default)]
pub config_source: ConfigSource,
#[serde(default)]
pub haiogate_account_id: Option<String>,
```

Defaults preserve existing `state.json` files: missing source becomes `manual`, missing account
id becomes `None`.

### 9.2 Rules

| Action | `config_source` | `haiogate_account_id` |
|---|---|---|
| Manual access-key save | `manual` | `None` |
| HaioGate “اعمال روی اپ” | `haiogate` | account UUID |
| Delete config | `manual` | `None` |
| Logout | unchanged | unchanged |
| App restart | persisted | persisted |

- `save_config` must set `config_source = Manual` and clear `haiogate_account_id`.
- `portal_apply_config` must use the same parser/save path but set `Haiogate` and the account id.
- `delete_config` must clear both after disabling the proxy.
- If the user applies a HaioGate config, then pastes a manual key, the source becomes `manual`;
  the home summary switches accordingly.
- The applied config survives logout; only the portal session is cleared.

### 9.3 `set_state` hardening

`set_state` currently accepts a full `State` from the frontend. A repo grep shows **no frontend
caller** (`get_state` is used in `main.js`/`settings.js`, `set_state` is not). Recommendation:
remove `set_state` from the `invoke_handler` as part of this work, or keep it internal-only.

If it must remain exposed:

- Preserve `config_source` and `haiogate_account_id` from the current state unless the caller is
  an internal command that explicitly changes them.
- Do not allow the frontend to set `config_source = "haiogate"` without a real account id or
  without parsing a valid config.
- Validate every field that can affect proxy/OS state; do not mass-assign the whole `State`.

### 9.4 Home view

- If `config_source == manual`: existing home hero and server display.
- If `config_source == haiogate` and an account id exists:
  - Show a compact “هایو گیت” card under the hero: package name, usage, expiry, queued renewal.
  - “مدیریت اکانت” opens the HaioGate detail view.
  - Server stat can remain the Trojan host.
- If the account detail cannot be fetched (offline/session expired), show “اطلاعات اکانت در
  دسترس نیست” and a retry; do not block the connect toggle.
- Home must not fetch portal data on every status tick. Load once on dashboard entry and on
  explicit refresh.

### 9.5 State file permissions

While touching `config::Store::save`, set `0600` on Unix:

```rust
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600));
}
```

This is a small, in-scope hardening because the file stores the VPN access key. Portal tokens
never go into this file.

---

## 10. User flows

### 10.1 Login with OTP

1. Entry points: HaioGate tab when logged out, header account chip, or “ورود و خرید هایو گیت”
   on onboarding.
2. User enters a mobile number. Normalize Persian/Arabic digits, strip spaces/dashes, validate
   `^09\d{9}$`.
3. `portal_request_otp` → show “کد ۶ رقمی ارسال شد”, countdown from `expires_in` /
   `retry_after`; disable resend until the countdown ends.
4. User enters the 6-digit code; `portal_login_otp`.
5. On success:
   - Rust stores tokens in the keyring (or memory fallback).
   - JS receives only the sanitized user.
   - Resume the action that triggered login: open HaioGate, buy, or renew.
6. Invalid/expired OTP: stay on the OTP step, show the error.
7. 429 lockout: disable submit for the 5-minute lockout and show the countdown.
8. SMS provider failure: allow retry.
9. Next launch: Rust restores the session and validates `/auth/me/`; if refresh fails, the UI
   shows logged out.

### 10.2 HaioGate account list

1. If no session: login card with benefits and a login button.
2. If logged in: load `portal_list_packages` and `portal_list_accounts` in parallel; load
   `/auth/me/` if the cached user is stale.
3. Rust computes `category`:
   - `active`: status active, `expired_at` in the future, not over quota.
   - `expired`: status expired/quota_exhausted, `expired_at` in the past, or traffic over.
   - `support`: suspended or inconsistent.
4. JS renders:
   - Account strip: mobile, Shahkar badge, logout.
   - Pending-payment banner, if any.
   - Active section: cards with package, usage bar, expiry, queued renewal count, renew button.
   - Expired section: cards with reason, renew button.
   - Support section: cards without renew.
   - “خرید پکیج جدید” action.
5. Empty states:
   - No accounts: “هنوز اکانت هایو گیت ندارید” + buy.
   - No packages: “در حال حاضر پکیج فعالی برای خرید وجود ندارد”.
6. Refresh:
   - Pull-to-refresh is not native on desktop; use a refresh button.
   - Refresh on tab entry and after payment.
   - Do not poll accounts continuously.

### 10.3 Account detail

- Header: package name, status chip, username.
- Usage card: used/total, percentage, `usage_display`.
- Expiry card: current expiry; projected expiry if reserves exist.
- Queued renewals: package, traffic, created date, count.
- Credentials: password masked by default; reveal/copy buttons; optional auto-hide after 60s.
- Connection: “اعمال روی اپ” calls `portal_apply_config`.
- Actions: “تمدید” (same or another package), “به‌روزرسانی مصرف” (`portal_sync_quota`).
- Legacy badge if `is_legacy`.
- Suspended: no renew; support message.

### 10.4 Renew an expired account

1. Tap renew on an expired card.
2. If logged out → login. If `shahkar_status != 1` → Shahkar.
3. Load active packages. Preselect the current package if active; otherwise require an active
   package.
4. Confirmation sheet:
   - Package, price, traffic, 30-day period.
   - “پس از پرداخت، همین اکانت فعال می‌شود و اعتبار قبلی حفظ می‌شود.”
5. `portal_start_renew({accountId, packageId})`.
6. `portal_buy_plan({planId, organizationId?})`.
7. `active` → refresh accounts/detail; `payment_required` → persist pending + open browser.
8. After payment: refresh the same account until active, expiry ~30 days ahead, usage zero.
9. If still expired after 60s: show “در حال فعال‌سازی” and manual refresh; after 15 minutes,
   support.

### 10.5 Renew an active account (early renewal)

Same flow, but the confirmation explains:

> «تمدید شما رزرو می‌شود و پس از پایان دوره فعلی فعال می‌شود. تا آن زمان اکانت فعلی بدون
> تغییر باقی می‌ماند.»

After payment:

- `pending_reserves_count` increases.
- `projected_expired_at` moves forward.
- The current expiry does not change; the UI must not claim it did.
- If a different package was selected, the reserve is pinned to that package and switches tier
  when consumed.

### 10.6 Buy a new package

1. “خرید پکیج جدید” → login → Shahkar if needed.
2. Show active packages with price, traffic, description.
3. If the user already has an account on the selected package, route to renew and explain:
   «شما قبلاً اکانتی با این پکیج دارید؛ خرید مجدد به معنی تمدید آن است.»
4. If the user owns accounts on other packages, explain that this purchase creates a **separate**
   account. To switch an existing account to another package, use “تمدید” on that account.
5. `portal_start_buy({packageId})` → `planId`.
5. `portal_buy_plan({planId})`.
6. `active` (free/fully covered) → refresh accounts.
7. `payment_required` → pending + browser.
8. After payment, refresh accounts; open the new account detail; offer “اعمال روی اپ”.

### 10.7 Shahkar

- Triggered before buy/renew if `shahkar_status != 1`, or when a command returns
  `shahkar_required`.
- Panel/modal:
  - Mobile: read-only, prefilled from `me`; caption “شماره موبایل حساب شما”.
  - National code: `inputmode="numeric"`, maxlength 10, Persian/Arabic digit normalization.
  - Explanation: “برای خرید و تمدید هایو گیت، احراز هویت شاهکار لازم است.”
  - Privacy: “کد ملی فقط برای استعلام شاهکار ارسال می‌شود و در برنامه ذخیره نمی‌شود.”
- `portal_shahkar_verify({nationalId})`.
- Results:
  - Verified → update user, clear field, return to pending action, retry once.
  - Mismatch (2) → stay, allow correction.
  - Provider error (3) → stay, retry.
  - 400 duplicate mobile → support.
  - 429 → countdown.
- Do not persist the national code. Clear it on close/success.
- If `mobile` is missing/invalid, do not allow submit; show support.

### 10.8 Payment and return

- Before opening, Rust persists `PendingPurchase` and validates the URL.
- Open with the system browser, not a WebView.
- Show a pre-payment note: «پس از پرداخت، به برنامه بازگردید تا وضعیت خرید بررسی شود.»
- On window focus and app startup:
  1. Read pending; if none, do nothing.
  2. If older than 24h, clear and show “پرداخت منقضی شد”.
  3. `portal_invoice({invoiceId})`.
  4. `paid` → refresh accounts/detail; mark success; clear pending.
  5. `canceled`/`refunded` → failure; clear pending.
  6. `pending` → keep banner; “بررسی مجدد” and “ادامه پرداخت”.
- Never call `billing/payments/verify/`.
- If paid but account not visible, poll accounts every 3s for up to 60s; then show
  “پرداخت موفق بود؛ اکانت در حال آماده‌سازی است” and support after 15 minutes.
- If the app exits, pending survives and is reconciled on next launch.
- If the user returns without paying, keep the pending invoice; resume via
  `portal_invoice_pay`, which returns a fresh payment URL.

### 10.9 Apply config and logout

- Apply:
  - `portal_apply_config({accountId})` fetches detail, parses `trojan_url`, saves config,
    sets `config_source = haiogate` and `haiogate_account_id`.
  - Do not auto-enable the proxy; ask the user to use the main toggle.
  - On parse failure, keep the old config and show “لینک اتصال این اکانت معتبر نیست. با پشتیبانی
    تماس بگیرید.”
- Logout:
  - Clear tokens, cached user, and org selection.
  - Keep the pending purchase (non-sensitive) and hide its banner until login; keep
    `trojan_url`, `trojan_config`, and `config_source`.
  - Show: «با خروج از حساب، تنظیمات اتصال فعلی حذف نمی‌شود.»

---

## 11. Screen-by-screen UX

### 11.1 Global layout

- Keep the 440×700 shell, header, content, and tab bar.
- Add a fourth tab: `هایو گیت` between خانه and عبور مستقیم.
- Tab bar labels shrink slightly if needed; keep touch targets ≥44px.
- Add a header account chip:
  - Logged out: “ورود”.
  - Logged in: display name or mobile + small Shahkar dot.
  - Click: account panel/login.
- Content panels scroll independently; keep the tab bar fixed.
- Follow the existing design tokens (`--surface`, `--border`, `--brand`, `--radius`); no new
  palette.
- RTL: use `dir="rtl"`, logical properties, and test both Persian and English.

### 11.2 Onboarding

Current card: paste access key and save.

Add below it:

- Divider: “یا”.
- Secondary card: “حساب هایو دارید؟”
- “ورود با شماره موبایل” button → login panel.
- Short line: “خرید و تمدید هایو گیت، مشاهده مصرف و اتصال مستقیم.”
- Keep the existing “کلید دسترسی ندارید؟ از console.haio.ir” link as a fallback.

### 11.3 HaioGate tab — logged out

- Card title: “هایو گیت”.
- Subtitle: “یک حساب برای خرید، تمدید و مدیریت دسترسی به سرویس‌های جهانی.”
- Benefits list (3 items):
  - خرید و تمدید در همان برنامه
  - مشاهده مصرف و تاریخ انقضا
  - اتصال با یک کلیک پس از خرید
- Primary: “ورود با شماره موبایل”.
- Secondary: “مشاهده پلن‌ها” (login required; shows a short explanation).

### 11.4 HaioGate tab — logged in

- Account strip:
  - Display name/mobile.
  - Shahkar badge: “تایید شده” / “تایید نشده”.
  - “خروج” text button.
- Pending payment banner at the top:
  - “پرداخت در انتظار تایید” + invoice id short form.
  - Buttons: “بررسی وضعیت”, “ادامه پرداخت”.
- Sections:
  - **فعال** — cards.
  - **منقضی شده** — cards.
  - **نیازمند پشتیبانی** — cards.
- Card content:
  - Package name + status chip.
  - Usage bar and text.
  - Expiry (Jalali) + relative “N روز مانده”.
  - “تمدید رزرو شده” chip if `pending_reserves_count > 0`.
  - Actions: “جزئیات”, “تمدید”.
- Primary action at bottom: “خرید پکیج جدید”.
- Refresh button in the section header.

### 11.5 Account detail

- Back button to list.
- Header: package, status, username.
- Usage card: large percentage, used/total, bar.
- Expiry card: current and projected.
- Queued renewals list.
- Credentials card:
  - Username with copy.
  - Password masked with reveal/copy.
  - Auto-hide after 60 seconds (optional).
- Primary: “اعمال روی اپ”.
- Secondary: “تمدید”, “به‌روزرسانی مصرف”.
- Support note for suspended accounts.

### 11.6 Package picker and confirmation

- Full panel, not a tiny modal.
- Package cards: name, price, traffic, description, current-package badge.
- Disabled current package if inactive, with explanation.
- Confirm view:
  - Package summary, price, period.
  - Renewal outcome: immediate activation vs queued reserve.
  - “پرداخت و تمدید” / “پرداخت و خرید”.
- If the selected package already has an account, switch to renew copy.

### 11.7 Shahkar panel

- Full panel or large modal; mobile read-only.
- National code input.
- Explanation + privacy note.
- Primary “تایید هویت”; secondary “انصراف”.
- On success, return to the pending action automatically.
- Do not use a small toast for mismatch/provider errors; show inline.

### 11.8 Account panel

- Mobile, display name.
- Shahkar status + “احراز هویت” action.
- Session persistence note if keyring fallback is active.
- “خروج از حساب” with confirmation.
- App version, portal base URL (optional debug only).

### 11.9 Persian copy (key strings)

| Key | Persian |
|---|---|
| hg_tab | هایو گیت |
| hg_login_title | ورود به حساب هایو |
| hg_mobile | شماره موبایل |
| hg_otp | کد تایید ۶ رقمی |
| hg_otp_sent | کد تایید ارسال شد |
| hg_resend | ارسال دوباره کد |
| hg_otp_invalid | کد وارد شده نامعتبر یا منقضی شده است. |
| hg_otp_lockout | تعداد تلاش‌ها بیش از حد مجاز است. لطفاً ۵ دقیقه دیگر تلاش کنید. |
| hg_shahkar_title | احراز هویت شاهکار |
| hg_shahkar_required | برای خرید و تمدید هایو گیت، احراز هویت لازم است. |
| hg_mismatch | اطلاعات هویتی با شماره موبایل مطابقت ندارد. |
| hg_provider_error | سرویس شاهکار در دسترس نیست. کمی بعد دوباره تلاش کنید. |
| hg_national_code | کد ملی |
| hg_shahkar_success | هویت شما با موفقیت تایید شد. |
| hg_active | اکانت‌های فعال |
| hg_expired | اکانت‌های منقضی شده |
| hg_support | نیازمند پشتیبانی |
| hg_renew | تمدید |
| hg_buy | خرید پکیج |
| hg_renew_reserved | تمدید شما رزرو شد و پس از پایان دوره فعلی فعال می‌شود. |
| hg_renew_success | اکانت شما فعال شد. |
| hg_payment_pending | در انتظار تایید پرداخت... |
| hg_payment_success | پرداخت با موفقیت انجام شد. |
| hg_provisioning | پرداخت موفق بود؛ اکانت در حال آماده‌سازی است. |
| hg_apply | اعمال روی اپ |
| hg_logout_keeps_config | با خروج از حساب، تنظیمات اتصال فعلی حذف نمی‌شود. |
| hg_no_accounts | هنوز اکانت هایو گیت ندارید. |
| hg_no_packages | در حال حاضر پکیج فعالی برای خرید وجود ندارد. |
| hg_keyring_fallback | ورود شما تا بستن برنامه حفظ می‌شود؛ ذخیره‌سازی امن سیستم در دسترس نیست. |
| hg_payment_return_hint | پس از پرداخت، به برنامه بازگردید تا وضعیت خرید بررسی شود. |

Add all keys to both `fa` and `en` in `i18n.js`.

---

## 12. Payment state machine

```text
Idle
  │ Buy / Renew
  ▼
EnsureLoggedIn ── no ──► Login ── success ──┐
  │ yes                                     │
  ▼                                         │
EnsureShahkar ── no ──► Shahkar ── success ─┘
  │ yes
  ▼
ResolveOrg (only if organization_required)
  │
  ▼
portal_start_buy / portal_start_renew
  │  → plan_id
  ▼
portal_buy_plan
  │
  ├─ active ──► refresh accounts ──► success
  │
  └─ payment_required
        │
        ▼
  persist pending + portal_open_payment
        │
        ▼
  system browser (Zibal)
        │
        ▼
  focus / startup: portal_reconcile_pending
        │
        ├─ paid ──► refresh accounts ──► detail ──► apply config
        ├─ canceled/refunded ──► failure
        └─ pending ──► banner; 24h expiry
```

### 12.1 Pending purchase fields

`mode`: `buy` | `renew`
`account_id`: UUID or null
`package_id`: UUID
`plan_id`: UUID
`invoice_id`: UUID
`payment_id`: UUID or null
`created_at`: Unix seconds
`stage`: `created` | `browser_opened` | `paid` | `provisioning` | `done` | `failed`

No payment URL, tokens, mobile, national code, password, or trojan URL.

### 12.2 Rules

- Disable buy/renew while a command is in flight; prevent double clicks.
- Persist pending **before** opening the browser, atomically.
- On a buy timeout with unknown outcome:
  - List invoices, find a pending invoice for the same plan/org created in the last ~15
    minutes.
  - Resume it with `portal_invoice_pay`; do not create a duplicate.
- On startup, reconcile pending younger than 24 hours.
- If `active` (free/fully covered), skip the browser.
- If `organization_required`, show the org picker and retry with `organization_id`.
- If `shahkar_required`, route to Shahkar, remember the action, retry once.
- Do not auto-connect the VPN after applying a config.
- If the user logs out while a pending purchase exists, keep the pending file (it contains no
  secrets) and hide its banner until login. The invoice remains server-side; after login,
  reconcile it before starting another purchase. `portal_cancel_pending` is the explicit
  “forget this purchase” action.

### 12.3 Active vs expired outcome

| Account at renewal start | After payment | UI |
|---|---|---|
| Active, future expiry, not over quota | Reserve queued; expiry unchanged | “تمدید رزرو شد”; projected expiry |
| Expired time | Active; +30 days; usage reset | “اکانت فعال شد” |
| Quota exhausted | Active; +30 days; usage reset | “اکانت فعال شد” |
| Suspended | Not offered | “نیازمند پشتیبانی” |
| New purchase | New account created | Open detail; apply config |

### 12.4 Recovery cases

- **Browser closed without paying**: pending; banner; resume.
- **App killed**: pending survives; reconcile next launch.
- **Paid, account not ready**: poll 60s; provisioning message.
- **Provisioning failed**: support with invoice id; do not retry payment.
- **Duplicate click**: in-flight guard; if duplicate invoices exist, use the oldest pending.
- **Session expired during reconcile**: refresh once; if that fails, keep pending, show login,
  resume after login.

---

## 13. Security and privacy

### 13.1 Tokens

- OS keyring only. Not in `state.json`, `localStorage`, `sessionStorage`, IndexedDB, logs,
  crash reports, or the webview.
- Access and refresh tokens are held in Rust memory and never returned by a command.
- Persist rotated refresh tokens atomically; a failed save must not leave a half-written entry.
- Clear both tokens on refresh failure or logout.
- Keyring service name is fixed; do not include the mobile number in the entry name.

### 13.2 Logging

- Never log request/response bodies, headers containing `Authorization`, OTPs, national codes,
  passwords, payment URLs, or trojan URLs.
- `tracing` may log method, path, status, and elapsed time.
- Redact query strings on payment URLs; do not log the full URL.
- Do not send telemetry or crash reports with account data.

### 13.3 National code and OTP

- National code: send only; clear the field after success/close; never persist.
- OTP: clear after login; never log or display after success.
- Do not put the national code in a URL query string.
- Desktop clipboard: copy buttons for username/password only; optional clipboard clear after
  60 seconds.

### 13.4 Payment

- Open only `https://` URLs whose host is in the Zibal allowlist
  (`zibal.ir`, `*.zibal.ir`, or the exact configured gateway host).
- Do not allow `javascript:`, `data:`, `file:`, custom schemes, or arbitrary hosts.
- Do not persist the payment URL.
- Do not render the gateway in a WebView, iframe, or `window.open` inside the app.
- Use the system browser via the existing shell plugin, wrapped by a Rust command.
- The app never handles card data.

### 13.5 Local files

- `state.json`: existing VPN config; set `0600` on Unix. It still contains the VPN password;
  moving it to the keyring is a separate hardening task.
- `portal_pending.json`: non-sensitive; `0600` on Unix.
- Keyring errors: log an error code only, never the secret or full OS error if it may contain
  the secret.

### 13.6 Webview and CSP

- Keep `connect-src` restricted to IPC. Do not add `https://portal.haio.ir` to `connect-src`.
- Do not add the Tauri HTTP plugin.
- Do not expose a generic “fetch any URL” command to JS.
- Validate every command input (UUID shape, mobile, national code, URL host).
- If a debug portal base URL is supported, show it only in debug builds.

### 13.7 Supply chain

- Keep `reqwest` on rustls; no native OpenSSL dependency.
- Do not add a new HTTP client or payment SDK.
- Pin new Rust dependencies and run `cargo audit` in CI if available.
- Frontend has no new npm runtime dependency; use existing Vite/JS.

---

## 14. Edge cases

| Case | Behaviour | Copy |
|---|---|---|
| OTP request 429 | Countdown, disable resend | «برای ارسال دوباره، {n} ثانیه صبر کنید.» |
| OTP SMS 502 | Allow retry | «ارسال پیامک ناموفق بود. دوباره تلاش کنید.» |
| OTP invalid/expired | Stay on OTP step | «کد وارد شده نامعتبر یا منقضی شده است.» |
| OTP lockout | Disable submit, 5-minute timer | «تعداد تلاش‌ها بیش از حد مجاز است. لطفاً ۵ دقیقه دیگر تلاش کنید.» |
| Session expired | Refresh once, else login and resume | «نشست شما منقضی شده است. دوباره وارد شوید.» |
| Shahkar required | Open Shahkar, remember action | «برای ادامه، احراز هویت لازم است.» |
| Shahkar mismatch | Stay, allow edit | «اطلاعات هویتی با شماره موبایل مطابقت ندارد.» |
| Shahkar provider error | Retry | «سرویس شاهکار در دسترس نیست. کمی بعد دوباره تلاش کنید.» |
| Duplicate mobile | Support | «این شماره موبایل به حساب دیگری متصل است. با پشتیبانی تماس بگیرید.» |
| Multiple orgs | Picker | «حساب سازمانی مورد نظر را انتخاب کنید.» |
| No mobile | Support | «برای خرید، شماره موبایل معتبر روی حساب لازم است.» |
| No active package | Empty state | «در حال حاضر پکیج فعالی برای خرید وجود ندارد.» |
| Current package inactive | Require another | «پکیج فعلی شما غیرفعال شده است؛ یک پکیج فعال انتخاب کنید.» |
| Active renew | Reserve, no immediate expiry change | «تمدید شما رزرو شد و پس از پایان دوره فعلی فعال می‌شود.» |
| Expired renew | Immediate activation | «اکانت شما فعال شد.» |
| Suspended | No self-service | «این اکانت نیازمند بررسی پشتیبانی است.» |
| Payment canceled | Keep pending; resume | «پرداخت لغو شد.» |
| Payment pending | Keep banner | «در انتظار تایید پرداخت...» |
| Paid, account pending | Refresh/support | «پرداخت موفق بود؛ اکانت در حال آماده‌سازی است.» |
| Provisioning failed | Support | «پرداخت انجام شد اما اکانت ساخته نشد. با پشتیبانی تماس بگیرید.» |
| Duplicate package purchase | Route to renew | «شما قبلاً اکانتی با این پکیج دارید؛ خرید مجدد به معنی تمدید آن است.» |
| Multiple accounts same package | Backend fix; else support | «برای تمدید این اکانت با پشتیبانی تماس بگیرید.» |
| Invalid trojan URL | Keep old config | «لینک اتصال این اکانت معتبر نیست. با پشتیبانی تماس بگیرید.» |
| Keyring unavailable | Memory session + notice | «ورود شما تا بستن برنامه حفظ می‌شود؛ ذخیره‌سازی امن سیستم در دسترس نیست.» |
| Corrupt pending file | Delete, continue | «وضعیت پرداخت قبلی قابل بازیابی نبود.» |
| Pending > 24h | Clear | «پرداخت قبلی منقضی شده است.» |
| Network offline | Retry | «اتصال به سرور برقرار نشد. اینترنت خود را بررسی کنید.» |
| Server 5xx | Retry | «خطای موقت سرور. دوباره تلاش کنید.» |
| Payment URL invalid host | Do not open | «آدرس درگاه پرداخت معتبر نیست.» |
| Logout with config | Keep config | «با خروج از حساب، تنظیمات اتصال فعلی حذف نمی‌شود.» |

---

## 15. Testing plan

### 15.1 Rust unit tests

- `parse_trojan_url` already has tests; add a HaioGate-shaped URL case with `#HaioGate`.
- `PortalError` mapping:
  - 400 field errors.
  - 401 → session expired.
  - 403 `shahkar_required`.
  - 400 `organization_required` with org list.
  - 429 with `Retry-After`.
  - 5xx and network errors.
- Token refresh:
  - 401 → refresh → retry succeeds.
  - Rotated refresh is persisted.
  - Concurrent requests trigger one refresh.
  - Refresh failure clears tokens.
- Keyring:
  - Save/load/delete with a `MemoryTokenStore`.
  - Keyring failure falls back to memory and `persistent = false`.
- Pending store:
  - Atomic write, corrupt file recovery, 24h expiry, stage transitions.
- Account classification:
  - active, expired by date, expired by quota, suspended, null expiry.
- Config source:
  - manual save sets manual; apply sets haiogate; delete resets.
  - `set_state` is removed; if retained, it preserves portal fields and cannot set
    `haiogate` without a valid account id/config.
- Payment URL validation:
  - accepted `https://gateway.zibal.ir/...`, rejected `http://`, `javascript:`, arbitrary host.

Use `wiremock` or `httpmock` as a dev-dependency for the HTTP client tests. If adding a
dependency is undesirable, isolate the HTTP layer behind a trait and use a fake in tests.

### 15.2 Frontend/manual E2E

Use a staging/test mobile and Zibal sandbox or a low-value test merchant. Do not use production
SMS/payments in automated tests.

1. Fresh install → onboarding → login with OTP → session restored after restart.
2. Keyring unavailable → login works, notice shown, restart requires login.
3. Unverified user → buy → Shahkar → success → purchase continues.
4. Buy 50 GB → browser → return → account appears → apply config → connect.
5. Expired account → renew same package → active, same credentials.
6. Expired account → renew different package → same account, new quota.
7. Active account → renew → reserve queued; projected expiry; current expiry unchanged.
8. Kill app during payment → reopen → pending reconciled.
9. Cancel payment → return → pending banner → resume.
10. Two accounts same package → renew older → correct account (requires backend fix).
11. Multi-org user → picker → purchase completes.
12. Logout → config remains; login again → accounts visible.
13. Language switch FA/EN → all portal strings and RTL/LTR correct.
14. 440×700 and 360×540 → no horizontal overflow, buttons reachable.
15. Release build → no tokens/OTP/national code in logs; keyring works on Windows/macOS/Linux.

### 15.3 Backend tests

The shared fix in §5 needs:

- Selected account marker written on renew.
- Signal renews the selected account for same-package and tier-switch cases.
- Foreign/invalid marker ignored.
- Intent cleared after success.
- Legacy `pending_package_id` rows still work.

### 15.4 Platform matrix

- Windows 10/11 (WebView2, NSIS install).
- macOS 10.15+ (Intel and Apple Silicon if built).
- Linux: AppImage/deb with WebKitGTK 4.1 and a secret-service provider.
- Keyring behavior on each platform; fallback path on a Linux session without secret-service.

---

## 16. Phases and estimates

| Phase | Work | Estimate |
|---|---|---|
| 0. Portal core + OTP | Rust module, reqwest client, keyring, refresh, OTP/login/me/logout commands, login UI | 4–6 days |
| 1. Accounts + detail + apply | List/group, detail, quota sync, credentials, config source, apply config | 4–5 days |
| 2. Shahkar | Panel, validation, status handling, gate interception | 2 days |
| 3. Buy/renew/payment | Buy/renew intents, org picker, pending store, browser open, invoice polling, recovery | 6–8 days |
| 4. Integration + release | Fourth tab, i18n, CSS, home summary, tests, platform builds | 4–5 days |
| **Desktop total** | | **20–26 engineer-days** |
| Shared backend fix | Selected-account intent, signal, tests | 1–2 days |

Assumptions:

- One Rust/Tauri engineer familiar with the codebase; QA support for platform E2E.
- Zibal sandbox/test merchant is available.
- The Android and desktop work can share the API contract and backend fix.
- Desktop release `2.1.0`; no change to the updater endpoint.

---

## 17. Acceptance criteria

### Login and session

- [ ] User can request an OTP, log in, and see the HaioGate tab without restarting.
- [ ] Session survives app restart when the keyring is available.
- [ ] Keyring-unavailable fallback keeps the session in memory and shows a clear notice.
- [ ] Rotated refresh tokens are persisted; concurrent 401s do not invalidate the session.
- [ ] Logout clears tokens and user cache, keeps the VPN config, and preserves a non-sensitive
      pending purchase hidden until login.
- [ ] OTP resend countdown, invalid code, lockout, and SMS failure have distinct messages.

### Accounts

- [ ] Active accounts appear in the active section.
- [ ] Expired/quota-exhausted accounts appear in the expired section and can be renewed.
- [ ] Suspended/inconsistent accounts appear in support and cannot be self-renewed.
- [ ] Detail shows usage, current and projected expiry, queued renewals, username, password
      reveal/copy, and trojan URL status.
- [ ] “اعمال روی اپ” applies the HaioGate config and sets `config_source = haiogate`.
- [ ] Manual save later switches the source to `manual`; logout does not change it.

### Buy and renew

- [ ] Buying with a test payment creates an account and shows it in the list.
- [ ] Renewing an expired account reactivates the same account with +30 days and reset usage.
- [ ] Renewing an active account queues a reserve and shows projected expiry without changing
      current expiry.
- [ ] Switching package during renewal keeps the same username/password.
- [ ] Buying a package the user already owns routes to renewal, not a duplicate.
- [ ] Multiple accounts on the same package renew the selected account (requires backend fix).
- [ ] Multi-org users can choose an organization and complete the purchase.

### Shahkar

- [ ] Unverified users are blocked before buy/renew and routed to Shahkar.
- [ ] Mobile is read-only and locked to the logged-in account.
- [ ] National code is 10 digits, normalized, and never persisted.
- [ ] Success updates `shahkar_status`, returns to the pending action, and retries once.
- [ ] Mismatch, provider error, duplicate mobile, and 429 are handled separately.

### Payment

- [ ] Zibal opens in the system browser, not a WebView.
- [ ] Pending purchase is persisted before the browser opens.
- [ ] Returning to the app polls the invoice and shows paid/canceled/pending correctly.
- [ ] App restart during payment recovers the pending purchase.
- [ ] Paid-but-provisioning shows a clear state and manual refresh.
- [ ] `billing/payments/verify/` is never called by the app.
- [ ] Invalid/non-Zibal payment URLs are rejected.
- [ ] A second purchase is blocked while one is pending; resume is offered.

### Security and release

- [ ] Tokens are never visible to JavaScript, `state.json`, localStorage, or logs.
- [ ] `state.json` and `portal_pending.json` are `0600` on Unix.
- [ ] No OTP, national code, password, payment URL, or trojan URL in logs/crash reports.
- [ ] CSP remains restricted to IPC; no Tauri HTTP plugin or generic fetch command.
- [ ] Portal failures do not affect proxy/OS proxy/domains/tray/updater.
- [ ] Windows, macOS, and Linux builds pass the platform matrix.
- [ ] No regression in manual config, onboarding, proxy toggle, bypass, settings, or updates.

---

## 18. Open questions

1. **Tab name**: “هایو گیت” vs “فروشگاه” for the fourth tab. Recommendation: “هایو گیت”, with
   “خرید پکیج” inside.
2. **Keyring fallback**: memory-only session vs a local encrypted fallback. Recommendation:
   memory-only; never write plaintext tokens.
3. **Early renewal**: allow for all active accounts, or only near expiry? Recommendation:
   allow; show reserve semantics.
4. **Suspended accounts**: support-only vs self-service renewal. Recommendation: support-only.
5. **Backend fix**: approve the selected-account intent. Without it, disable duplicate-package
   renewal.
6. **Payment return**: keep polling in MVP, or add a deep link in Phase 2? Recommendation:
   polling; deep link later.
7. **Account panel placement**: header chip vs a fifth tab. Recommendation: header chip; four
   tabs is already dense at 440px.
8. **Config auto-apply**: after buying, should the app apply the config automatically?
   Recommendation: offer a clear “اعمال روی اپ” action; do not silently replace the user's
   current config.
9. **VPN password storage**: move `trojan_url`/`trojan_config` from `state.json` to the keyring?
   Recommendation: separate hardening task after this release.
10. **Portal base URL override**: support `HAIO_PORTAL_BASE_URL` for staging in debug only?
    Recommendation: yes, debug only.

---

## 19. File-level change map

### Rust

| File / area | Change |
|---|---|
| `src-tauri/Cargo.toml` | Enable platform keyring features; add `wiremock`/`httpmock` dev-dependency if used |
| `src-tauri/src/lib.rs` | `mod portal`; add `portal: Arc<PortalService>` to `AppState`; register commands; spawn session restore |
| `src-tauri/src/portal/mod.rs` | New; `PortalService` |
| `src-tauri/src/portal/client.rs` | New; reqwest client, authorized requests, 401 retry |
| `src-tauri/src/portal/models.rs` | New; API/UI DTOs |
| `src-tauri/src/portal/session.rs` | New; tokens, refresh lock, load/save/clear |
| `src-tauri/src/portal/keyring_store.rs` | New; keyring + memory fallback |
| `src-tauri/src/portal/pending.rs` | New; `portal_pending.json` |
| `src-tauri/src/portal/error.rs` | New; serializable errors |
| `src-tauri/src/portal/commands.rs` | New; Tauri commands |
| `src-tauri/src/config/mod.rs` | Add `ConfigSource`, `config_source`, `haiogate_account_id`; 0600 save |
| `src-tauri/src/app/commands.rs` | `save_config` sets manual/clears account; `delete_config` clears both; `set_state` hardening; add `portal_apply_config` or place in portal commands |
| `src-tauri/capabilities/default.json` | No change if payment uses a Rust command + existing shell plugin; verify `shell:allow-open` scope |
| `src-tauri/tauri.conf.json` | No CSP change; no new plugin in MVP |
| `src-tauri/gen/schemas/*` | Regenerated by Tauri if capabilities change; do not hand-edit |

### Frontend

| File / area | Change |
|---|---|
| `src/index.html` | Fourth tab, HaioGate panel, onboarding secondary CTA, header account chip, payment banner, login/Shahkar/detail/package containers |
| `src/scripts/main.js` | `VALID_TABS` add `haiogate`; tab handling; import/init `portal.js`; buy banner in-app; re-render on language change |
| `src/scripts/portal.js` | New; login, accounts, detail, packages, renew/buy, payment, Shahkar, account panel |
| `src/scripts/i18n.js` | Add FA/EN keys for all portal strings |
| `src/styles/style.css` | `.hg-*` components, 4-tab spacing, account strip, cards, payment banner, panels, responsive 360×540 |
| `src/scripts/invoke.js` | No change; reuse |
| `src/scripts/settings.js` | No change unless an account row is added; recommendation: keep account in header/HaioGate |

### Tests

| File / area | Change |
|---|---|
| `src-tauri/src/portal/*` | Unit tests alongside modules or `tests/` |
| `src-tauri/tests/` | Optional integration tests for HTTP/refresh/pending |
| Manual checklist | `DESKTOP-TEST-OTP-HAIOGATE.md` or section in `docs/` |

### Not changed

- Trojan client, proxy server, router, domains, OS proxy, QUIC, tray, autostart, updater.
- `save_config` parsing behavior for manual keys.
- Existing onboarding visual language.
- Android app, web frontend, backend OTP/Shahkar/Zibal providers.

---

## 20. What ships when

### MVP (this plan)

- OTP login with keyring-backed session and refresh.
- HaioGate active/expired/support lists.
- Account detail with usage, expiry, projected expiry, queued renewals, credentials.
- Renew expired; renew active as reserve; optional package switch.
- Buy new package; duplicate-package routing to renewal.
- Shahkar gate + national-code verification.
- Zibal system-browser payment with pending persistence and invoice polling.
- Apply HaioGate config; `config_source` protection; home summary.
- Desktop release `2.1.0`.

### Phase 2 (after MVP usage data)

- `haiobypass://payment` deep link via `tauri-plugin-deep-link`.
- Per-payment return URL on the backend.
- Non-destructive payment-status endpoint.
- Buy idempotency key.
- Public package catalog.
- Optional migration of the VPN access key to the OS keyring.
- Shared cloud companion/account-center work (separate plan).

---

## 22. Multi-user addendum (1 / 2 / 5 users)

This section extends the plan with per-config user limits. Full server/control-plane design:
`/home/devcloud/Documents/mine/Haio-Forever/haio-cloud-ai-infra/PLAN-MULTI-USER-HAIOGATE.md`.

### 22.1 Product rules

- A package includes a fixed number of user credentials: 50 GB = 1, 100 GB = 2, 200 GB = 5.
- A “user” means a named credential (username/password/trojan URL), not a concurrent TCP
  connection.
- Existing accounts are **legacy unlimited** (`included_users = NULL`) and keep working
  unchanged until expiry or renewal.
- At renewal, the selected package's count applies. The primary credential is rotated, old
  shared credentials are revoked, and the owner onboards members.
- Members are owner-managed: add, revoke, rotate within available slots.

### 22.2 Desktop changes

- Package cards show «N کاربر».
- Renewal sheet shows the count and, for legacy accounts, warns that the current shared link
  will change after renewal.
- Active early renewal: reserve only; member changes apply at the start of the new period.
- Expired renewal: immediate; after payment open the members panel.
- Add a members panel to the HaioGate detail view (scrollable in the 440×700 window):
  primary row + members, slot counter, add/revoke/rotate, copy/QR.
- New Rust portal commands:
  - `portal_add_member`
  - `portal_rotate_member`
  - `portal_revoke_member`
- Keep tokens in the OS keyring; member credentials go to JS only for display, never logs.
- DTO additions: `included_users`, `is_legacy_unlimited`, `active_members_count`,
  `available_slots`, `members`.
- Add FA/EN i18n keys and CSS for the members panel.
- Existing OTP/Shahkar/payment architecture is unchanged.
- Additive estimate: **4–6 engineer-days**.

### 22.3 Backend note

The shared backend renewal fix in §5 is already covered in the current control plane by
`AntiSanctionPurchaseIntent` (NEW vs RENEW + account). Verify this on the target branch; do
not implement a second intent mechanism.

### 22.4 Acceptance additions

- [ ] A 2-user package shows 2 slots; adding a second credential works; a third is rejected.
- [ ] A 5-user package allows primary + 4 members.
- [ ] Legacy account shows unlimited and no member actions until renewal.
- [ ] Expired renewal rotates the old link and opens onboarding.
- [ ] Active renewal keeps the current period unchanged; new count applies at activation.
- [ ] Member revoke/rotate works; old credentials stop for new connections.

---

## 23. Guest mode addendum

Full plan: `/home/devcloud/Documents/mine/Haio-Forever/HaioBypass-android/PLAN-GUEST-MODE-ANDROID-DESKTOP.md`.

Guest mode is required so a user who only received a config can use the app without OTP,
Shahkar, or a portal account; it is also the member onboarding path for the 1/2/5-user
HaioGate packages.

Changes to this desktop plan:

- Keep `#configOnlyCard` as the guest path but make the choice explicit:
  - paste + «ذخیره و اتصال»;
  - secondary «ورود به حساب هایو»;
  - a «مهمان» label and a short explanation.
- Derive guest state from “no portal session + `state.trojanUrl`”; do not persist a guest flag
  or create a guest token.
- Add a header guest chip and login action; add a login card in the `هایو گیت` tab.
- Add «کپی لینک اتصال» in home/settings (full `trojanUrl`), with a bearer-secret warning and
  a WebView clipboard fallback.
- `save_config` sets source `manual` and clears `haiogate_account_id`; login does not modify
  the config; logout keeps it.
- Guest mode must not initialize portal HTTP, keyring, refresh, or pending polling.
- No backend changes; add a clipboard plugin/command only if the WebView API is unreliable.
- Estimate: +2–3 desktop engineer-days (shared guest plan total 6–10 with Android).

Acceptance additions:

- [ ] Fresh install can paste a config and reach the dashboard without login.
- [ ] Desktop can copy the full current config; link is masked in the UI and never logged.
- [ ] Guest → login and logout preserve the config.
- [ ] Guest `هایو گیت` tab shows login, not packages/accounts.
- [ ] Restart preserves guest config and derives guest mode.
- [ ] Member can onboard via a shared link in guest mode; revoke/rotation cuts it off.

---

## 24. Plan-only statement

This document is a proposal. No Rust module, command, frontend file, CSS rule, translation, or
configuration has been changed. Implementation starts only after the open questions are
resolved and the multi-user/guest-mode rollout in the shared plans is approved.
