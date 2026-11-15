# Desktop manual test — OTP, HaioGate, multi-user, guest mode

Target: `HaioBypass` 4.0.0 (Tauri 2, 440×700).
Sources of record: `PLAN-OTP-HAIOGATE-DESKTOP-FOCUSED.md` (with §22 multi-user and
§23 guest mode), `PLAN-MULTI-USER-HAIOGATE.md`.

Portal base URL: `https://portal.haio.ir/api/v1/`.
Staging override (debug builds only): `HAIO_PORTAL_BASE_URL=https://…/`.
Extra gateway hosts (hostname list, no scheme): `HAIO_PAYMENT_HOSTS=pay.example.com`.

Use a **test mobile**, **Zibal sandbox / low-value merchant**, and **test national codes**.
Never run the payment steps against production with a real card.

## 0. Preconditions

- [ ] `cargo test` passes (93 tests) and `npm run build` succeeds.
- [ ] A portal account with no HaioGate account, one expired account, one active account,
      and a 2-user/5-user package are available on staging.
- [ ] The Zibal sandbox is reachable from the system browser.

## 1. Login and session (issue 06)

- [ ] Fresh install → onboarding shows two paths: **مهمان** (paste key) and
      **حساب هایو** (login).
- [ ] Header chip shows «ورود» (or «مهمان» when a config exists); clicking it opens the
      HaioGate tab and the login screen **without** requiring a saved config.
- [ ] Invalid mobile (`0912`, `12345`) is rejected inline with the mobile message.
- [ ] Persian digits (`۰۹۱۲…`) and `+98 912…` normalize to `09…`.
- [ ] Request OTP → «کد تایید ارسال شد», resend disabled with a countdown.
- [ ] Wrong OTP → «کد وارد شده نامعتبر یا منقضی شده است.», stays on the OTP step.
- [ ] 5 wrong OTPs → lockout message and a 5-minute countdown.
- [ ] SMS provider failure (staging: force 502) → «ارسال پیامک ناموفق بود…» and a retry.
- [ ] Valid OTP → account list, chip shows the name/mobile, no OTP left in the DOM.
- [ ] Restart the app → session restored from the OS keyring, no login flash
      (skeleton chip on launch).
- [ ] Keyring unavailable (Linux without a Secret Service daemon) → login works, the
      notice «ورود شما تا بستن برنامه حفظ می‌شود…» is visible, restart requires login.
- [ ] Tokens are absent from `~/.haiobypass/state.json`, `localStorage`, and logs.
- [ ] Logout → tokens cleared, config kept, chip returns to guest/login.

## 2. Accounts, detail, apply (issue 06)

- [ ] Active accounts appear under **فعال**; expired/quota-exhausted under **منقضی شده**;
      suspended under **نیازمند پشتیبانی** with no renew action.
- [ ] Card shows package, usage bar, usage text, expiry (Jalali in FA), reserve chip.
- [ ] Detail shows usage %, current expiry, projected expiry, queued renewals, username,
      password reveal/copy (auto-hide after 60 s) and a masked connection link.
- [ ] «به‌روزرسانی مصرف» calls the quota endpoint and updates the numbers.
- [ ] «اعمال روی اپ» writes the config, sets `configSource = haiogate` and
      `haiogateAccountId`; the proxy is **not** enabled automatically.
- [ ] Home shows the compact HaioGate card (package, usage, expiry, «مدیریت اکانت»).
- [ ] Pasting a manual key afterwards switches the source to `manual` and hides the card.
- [ ] Logout keeps the applied config and the home card state.
- [ ] Deleting the config clears the source/account id and returns to onboarding.
- [ ] `state.json` and `portal_pending.json` are `0600` on Unix.
- [ ] Offline / expired session on the home card shows «اطلاعات اکانت در دسترس نیست» without
      blocking the connect toggle.

## 3. Buy, renew, payment (issue 06)

- [ ] Unverified user → buy/renew opens the Shahkar panel; cancel returns to the flow.
- [ ] Shahkar with an invalid national code (<10 digits) stays inline; the field is cleared
      after submit.
- [ ] Shahkar mismatch (status 2) → «اطلاعات هویتی … مطابقت ندارد.»; provider error
      (status 3) → retry message; duplicate mobile → support message.
- [ ] Successful Shahkar resumes the pending purchase automatically and the account list
      shows «تایید شده».
- [ ] Buy 50 GB → system browser opens Zibal (never a WebView) → return to the app.
- [ ] Before the browser opens, `portal_pending.json` exists with `stage = browser_opened`
      and no payment URL / tokens.
- [ ] Return with payment canceled → «پرداخت لغو شد.» and the banner offers resume/dismiss.
- [ ] Kill the app during payment → relaunch → pending is reconciled on startup
      (silent when still pending).
- [ ] Paid → account appears; detail opens; «اعمال روی اپ» works.
- [ ] Paid but not provisioned → «پرداخت موفق بود؛ اکانت در حال آماده‌سازی است.» and manual
      refresh.
- [ ] Renew expired → same username/password, expiry +30 days, usage reset.
- [ ] Renew active → reserve queued, current expiry unchanged, projected expiry moves,
      copy says «تمدید شما رزرو می‌شود…».
- [ ] Renew with a different package → same account, new package.
- [ ] Buying a package already owned routes to **renew** with an explanation.
- [ ] A second purchase while one is pending is blocked; resume/dismiss is offered.
- [ ] Multi-org user → org picker → purchase completes.
- [ ] Invalid/non-Zibal payment URL (force via a proxy or staging) → refused, browser does
      not open.
- [ ] `billing/payments/verify/` never appears in network captures from the app.

## 4. Multi-user (issue 07)

- [ ] 50 GB package card shows «۱ کاربر»; 100 GB «۲ کاربر»; 200 GB «۵ کاربر».
- [ ] Legacy account (`included_users = NULL`) shows «نامحدود (قدیمی)» and no member form.
- [ ] 2-user account: primary + 1 slot. Adding one member works; a second add is rejected.
- [ ] 5-user account allows primary + 4 members.
- [ ] New member credential (username/password/link) is shown once with the bearer warning;
      copy works and the value is not logged.
- [ ] Revoke → member disappears from the live list; the old link stops working for new
      connections.
- [ ] Rotate → new password/link; the previous link stops working.
- [ ] Revoked credentials stop working in guest mode (test the link in a second profile).
- [ ] Renewal onto a smaller package shows the downgrade warning and the backend rejects
      while extra members remain.
- [ ] Active early renewal converts only when the reserve activates (verify the copy, not
      the current period).

## 5. Guest mode (issue 08)

- [ ] First launch → paste a valid `trojan://…@tahrim.haiocloud.com:443?sni=…#HaioGate`
      key → «ذخیره کلید» → dashboard without any portal call.
- [ ] No keyring access, OTP request, or pending polling occurs in guest mode (watch logs).
- [ ] Restart → guest config restored, chip shows «مهمان», HaioGate tab shows the login card.
- [ ] «کپی لینک اتصال» → warning modal → copy → link is complete and pasteable.
- [ ] Clipboard fallback works on WebView2 and WebKitGTK when
      `navigator.clipboard` is denied.
- [ ] Login from guest → config preserved; logout → back to guest, config still works.
- [ ] Guest cannot reach account-only screens without logging in; the HaioGate tab shows
      only the login card.
- [ ] Member onboarding: paste the owner-shared link → connect; after revoke/rotate the
      link fails server-side.

## 6. Security and regressions

- [ ] No tokens, OTPs, national codes, passwords, payment URLs or trojan URLs in app logs
      or crash output.
- [ ] CSP still restricts `connect-src` to IPC; no Tauri HTTP plugin added.
- [ ] `set_state` is no longer exposed to the webview.
- [ ] Proxy toggle, OS proxy consent, bypass list, presets, autostart, tray, updater and
      QUIC block still behave as before.
- [ ] FA/EN switch re-renders every portal string and keeps RTL/LTR layout correct.
- [ ] 440×700 and 360×540: no horizontal overflow, four tabs remain tappable (≥44 px),
      member form wraps on narrow widths.
