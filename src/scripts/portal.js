// HaioGate / portal UI.
//
// Security rules enforced here:
// - The webview never receives portal tokens; every call goes through a Rust
//   command.
// - OTP and national code inputs are cleared after success / close.
// - Payment always opens in the system browser (Rust command), never a WebView.
// - No account data, credential or config link is written to localStorage.
//
// Guest mode is derived, not stored: `!loggedIn && state.trojanUrl`. This
// module never touches the keyring itself; it only asks Rust for a session.

import { invoke, listen } from './invoke.js';
import { translations } from './i18n.js';

const $ = (id) => document.getElementById(id);

const T = () => translations[document.documentElement.lang] || translations.en;
const t = (key) => T()[key] || key;
const tf = (key, vars) =>
  t(key).replace(/\{(\w+)\}/g, (match, name) =>
    Object.prototype.hasOwnProperty.call(vars, name) ? String(vars[name]) : match
  );

const SCREENS = {
  restoring: 'hgRestoring',
  loggedout: 'hgLoggedOut',
  login: 'hgLogin',
  list: 'hgList',
  detail: 'hgDetail',
  packages: 'hgPackages',
};

const state = {
  session: { loggedIn: false, user: null, persistent: true, restoring: true },
  accounts: [],
  packages: [],
  orgs: [],
  pending: null,
  detail: null,
  memberCredential: null,
  screen: 'list',
  flow: null,
  selectedPackage: null,
  intent: null,
  orgContext: null,
  resumeAfterLogin: null,
  shahkarContinuation: null,
  otp: { mobile: '', requested: false, timer: null, until: 0, busy: false },
  logoutArmed: false,
  logoutTimer: null,
  focusAt: 0,
  homeAccountId: null,
  homeFetchAt: 0,
  homeFetchInFlight: false,
  initialised: false,
};

/* ------------------------------------------------------------------ */
/* Small DOM helpers                                                  */
/* ------------------------------------------------------------------ */

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = text;
  return node;
}

function button(label, className, onClick) {
  const node = el('button', className || 'btn ghost sm', label);
  node.type = 'button';
  if (onClick) node.addEventListener('click', onClick);
  return node;
}

function clear(node) {
  if (node) node.replaceChildren();
}

function setText(id, value) {
  const node = $(id);
  if (node) node.textContent = value == null ? '' : String(value);
}

function show(id, visible) {
  const node = $(id);
  if (node) node.classList.toggle('hidden', !visible);
}

function normalizeError(error) {
  if (error && typeof error === 'object') {
    if (error.code) return error;
    if (error.message) return { code: 'unknown', message: error.message };
  }
  if (typeof error === 'string') {
    return { code: 'unknown', message: error };
  }
  return { code: 'unknown', message: t('hgUnknownError') };
}

function localizedError(error) {
  const err = normalizeError(error);
  switch (err.code) {
    case 'network':
    case 'timeout':
      return t('hgNetworkError');
    case 'server':
    case 'protocol':
      return t('hgServerError');
    case 'session_expired':
    case 'not_logged_in':
      return t('hgSessionExpired');
    case 'payment_url_invalid':
      return t('hgServerError');
    case 'rate_limited':
      return err.retryAfter
        ? `${err.message} (${err.retryAfter}s)`
        : err.message;
    default:
      return err.message || t('hgUnknownError');
  }
}

function toast(message, isError) {
  if (isError) {
    if (typeof window.showAppError === 'function') window.showAppError(message);
  } else if (typeof window.showAppNotice === 'function') {
    window.showAppNotice(message);
  } else if (typeof window.showAppError === 'function') {
    window.showAppError(message);
  }
}

function setBusy(on, message) {
  if (typeof window.appLoading === 'function') {
    window.appLoading(Boolean(on), message || t('hgBusy'));
  }
}

function normalizeDigits(value) {
  const map = {
    '۰': '0', '۱': '1', '۲': '2', '۳': '3', '۴': '4',
    '۵': '5', '۶': '6', '۷': '7', '۸': '8', '۹': '9',
    '٠': '0', '١': '1', '٢': '2', '٣': '3', '٤': '4',
    '٥': '5', '٦': '6', '٧': '7', '٨': '8', '٩': '9',
  };
  return String(value || '')
    .split('')
    .map((ch) => map[ch] || ch)
    .filter((ch) => /[0-9]/.test(ch))
    .join('');
}

function isMobileValid(value) {
  const digits = normalizeDigits(value);
  return /^09\d{9}$/.test(digits);
}

/* ------------------------------------------------------------------ */
/* Formatting                                                         */
/* ------------------------------------------------------------------ */

function fmtDate(iso) {
  if (!iso) return '—';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '—';
  const locale =
    document.documentElement.lang === 'fa'
      ? 'fa-IR-u-ca-persian'
      : 'en-GB';
  try {
    return new Intl.DateTimeFormat(locale, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    }).format(date);
  } catch (_) {
    return date.toLocaleDateString();
  }
}

function daysLeft(iso) {
  if (!iso) return null;
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return null;
  return Math.ceil((date.getTime() - Date.now()) / 86400000);
}

function fmtPrice(toman) {
  const amount = Number(toman) || 0;
  const text = amount.toLocaleString(
    document.documentElement.lang === 'fa' ? 'fa-IR' : 'en-US'
  );
  return `${text} ${document.documentElement.lang === 'fa' ? 'تومان' : 'Toman'}`;
}

function fmtTraffic(bytes) {
  const value = Number(bytes) || 0;
  if (value <= 0) return '—';
  const gb = value / (1024 * 1024 * 1024);
  const rounded = gb >= 10 ? Math.round(gb) : Math.round(gb * 10) / 10;
  return `${rounded} GB`;
}

function maskLink(link) {
  if (!link) return '—';
  const match = String(link).match(/^[a-z]+:\/\/[^@]*@([^:/?#]+):(\d+)/i);
  return match ? `trojan://••••@${match[1]}:${match[2]}` : '••••';
}

function usagePercent(account) {
  if (!account) return 0;
  const total = Number(account.totalTrafficBytes) || 0;
  const used = Number(account.usageBytes) || 0;
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, Math.round((used / total) * 100)));
}

/* ------------------------------------------------------------------ */
/* Clipboard (WebView fallback)                                       */
/* ------------------------------------------------------------------ */

async function copyText(value, noticeKey) {
  if (!value) return false;
  let copied = false;
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(value);
      copied = true;
    }
  } catch (_) {
    copied = false;
  }

  if (!copied) {
    // WebView2/WebKitGTK may deny the async clipboard API without a
    // permission prompt. The selection-based fallback works without one.
    const area = document.createElement('textarea');
    area.value = value;
    area.setAttribute('readonly', 'readonly');
    area.style.position = 'fixed';
    area.style.opacity = '0';
    area.style.pointerEvents = 'none';
    document.body.appendChild(area);
    area.select();
    try {
      copied = document.execCommand('copy');
    } catch (_) {
      copied = false;
    }
    area.remove();
  }

  if (copied) {
    toast(t(noticeKey || 'hgCopied'));
  } else {
    toast(t('hgCopyFailed'), true);
  }
  return copied;
}

/* ------------------------------------------------------------------ */
/* Session chrome                                                     */
/* ------------------------------------------------------------------ */

function isShahkarVerified() {
  const user = state.session.user;
  return Boolean(user && (user.shahkarVerified || user.shahkarStatus === 1));
}

function displayName() {
  const user = state.session.user;
  if (!user) return '';
  return user.displayName || user.mobile || user.username || '';
}

function renderChrome() {
  const chipText = $('accountChipText');
  const dot = $('accountDot');
  if (!chipText || !dot) return;

  if (state.session.restoring) {
    chipText.textContent = '…';
    dot.dataset.state = 'restoring';
    return;
  }
  if (state.session.loggedIn) {
    chipText.textContent = displayName() || t('hgAccount');
    dot.dataset.state = isShahkarVerified() ? 'on' : 'pending';
  } else {
    const guest = Boolean(window.appHasConfig);
    chipText.textContent = guest ? t('hgGuest') : t('hgLogin');
    dot.dataset.state = guest ? 'guest' : 'none';
  }
}

function renderAccountPanel() {
  const user = state.session.user;
  setText('accountPanelName', user ? displayName() : '—');
  setText('accountPanelMobile', user && user.mobile ? user.mobile : '—');
  setText(
    'accountPanelShahkar',
    isShahkarVerified() ? t('hgShahkarVerified') : t('hgShahkarUnverified')
  );
  const persist = $('accountPanelPersist');
  if (persist) {
    persist.textContent = state.session.loggedIn && !state.session.persistent
      ? t('hgKeyringFallback')
      : '';
    persist.classList.toggle('hidden', !persist.textContent);
  }
  const verify = $('accountPanelShahkarBtn');
  if (verify) {
    const needsVerify = state.session.loggedIn && !isShahkarVerified();
    verify.classList.toggle('hidden', !needsVerify);
  }
  const logout = $('accountPanelLogout');
  if (logout) {
    logout.classList.toggle('hidden', !state.session.loggedIn);
    resetLogoutArmed();
  }
}

function resetLogoutArmed() {
  clearTimeout(state.logoutTimer);
  state.logoutArmed = false;
  const logout = $('accountPanelLogout');
  if (logout) {
    logout.classList.remove('confirming');
    logout.textContent = t('hgLogout');
  }
}

function showScreen(name) {
  Object.entries(SCREENS).forEach(([key, id]) => show(id, key === name));
}

function render() {
  renderChrome();

  if (state.session.restoring) {
    showScreen('restoring');
    return;
  }
  if (!state.session.loggedIn) {
    showScreen(state.screen === 'login' ? 'login' : 'loggedout');
    return;
  }

  if (state.screen === 'detail' && state.detail) {
    showScreen('detail');
    renderDetail();
    return;
  }
  if (state.screen === 'packages') {
    showScreen('packages');
    renderPackages();
    return;
  }
  showScreen('list');
  renderList();
}

/* ------------------------------------------------------------------ */
/* Login                                                              */
/* ------------------------------------------------------------------ */

function openLogin() {
  state.screen = 'login';
  state.otp.requested = false;
  stopOtpTimer();
  const mobile = $('hgMobileInput');
  if (mobile) {
    mobile.value = '';
    mobile.disabled = false;
  }
  show('hgOtpLabel', false);
  show('hgOtpInput', false);
  show('hgResendBtn', false);
  show('hgChangeMobile', false);
  setText('hgLoginMsg', '');
  const submit = $('hgOtpSubmit');
  if (submit) {
    submit.textContent = t('hgSendOtp');
    submit.disabled = false;
  }
  if (typeof window.appEnterPortal === 'function') window.appEnterPortal();
  else if (typeof window.switchAppTab === 'function') window.switchAppTab('haiogate');
  render();
  if (mobile) setTimeout(() => mobile.focus(), 60);
}

function stopOtpTimer() {
  if (state.otp.timer) {
    clearInterval(state.otp.timer);
    state.otp.timer = null;
  }
}

function setLoginMessage(message, isError) {
  const node = $('hgLoginMsg');
  if (!node) return;
  node.textContent = message || '';
  node.className = `hg-form-msg${isError ? ' bad' : message ? ' ok' : ''}`;
}

function startOtpTimer(seconds) {
  stopOtpTimer();
  const total = Math.max(1, Number(seconds) || 60);
  state.otp.until = Date.now() + total * 1000;

  const tick = () => {
    const remaining = Math.max(0, Math.ceil((state.otp.until - Date.now()) / 1000));
    const resend = $('hgResendBtn');
    if (resend) {
      resend.disabled = remaining > 0;
      resend.textContent =
        remaining > 0 ? tf('hgResendIn', { s: remaining }) : t('hgResend');
    }
    if (remaining <= 0) stopOtpTimer();
  };
  tick();
  state.otp.timer = setInterval(tick, 1000);
}

async function requestOtp() {
  if (state.otp.busy) return;
  const mobileInput = $('hgMobileInput');
  const mobile = normalizeDigits(mobileInput ? mobileInput.value : '');
  if (!isMobileValid(mobile)) {
    setLoginMessage(t('hgMobileInvalid'), true);
    mobileInput?.focus();
    return;
  }

  state.otp.busy = true;
  setLoginMessage('', false);
  try {
    const res = await invoke('portal_request_otp', { mobile });
    state.otp.mobile = mobile;
    state.otp.requested = true;
    if (mobileInput) {
      mobileInput.value = mobile;
      mobileInput.disabled = true;
    }
    show('hgOtpLabel', true);
    show('hgOtpInput', true);
    show('hgResendBtn', true);
    show('hgChangeMobile', true);
    const submit = $('hgOtpSubmit');
    if (submit) {
      submit.textContent = t('hgVerifyLogin');
      submit.disabled = false;
    }
    startOtpTimer(res.expiresIn || res.retryAfter || 60);
    setLoginMessage(t('hgOtpSent'), false);
    $('hgOtpInput')?.focus();
  } catch (error) {
    const err = normalizeError(error);
    if (err.code === 'rate_limited') {
      setLoginMessage(t('hgOtpLockout'), true);
      if (err.retryAfter) startOtpTimer(err.retryAfter);
    } else if (err.code === 'network' || err.code === 'timeout') {
      setLoginMessage(t('hgNetworkError'), true);
    } else if (err.code === 'server' || err.status === 502) {
      setLoginMessage(t('hgSmsFailed'), true);
    } else {
      setLoginMessage(localizedError(err), true);
    }
  } finally {
    state.otp.busy = false;
  }
}

async function submitLogin() {
  if (state.otp.busy) return;
  if (!state.otp.requested) {
    await requestOtp();
    return;
  }

  const otp = normalizeDigits($('hgOtpInput')?.value || '');
  if (otp.length !== 6) {
    setLoginMessage(t('hgOtpInvalid'), true);
    return;
  }

  state.otp.busy = true;
  const submit = $('hgOtpSubmit');
  if (submit) submit.disabled = true;
  try {
    const res = await invoke('portal_login_otp', { mobile: state.otp.mobile, otp });
    // Clear the OTP from the DOM immediately: it is single-use.
    const otpInput = $('hgOtpInput');
    if (otpInput) otpInput.value = '';

    state.session = {
      loggedIn: true,
      user: res.user || null,
      persistent: res.persistent !== false,
      restoring: false,
    };
    state.screen = 'list';
    stopOtpTimer();
    setLoginMessage('', false);
    render();
    await loadPending();
    await loadAll({ silent: true });

    const resume = state.resumeAfterLogin;
    state.resumeAfterLogin = null;
    if (typeof resume === 'function') {
      resume();
    }
    if (typeof window.appRefreshHomeCard === 'function') window.appRefreshHomeCard();
  } catch (error) {
    const err = normalizeError(error);
    if (err.code === 'rate_limited') {
      setLoginMessage(t('hgOtpLockout'), true);
      if (err.retryAfter) startOtpTimer(err.retryAfter);
    } else if (err.code === 'network' || err.code === 'timeout') {
      setLoginMessage(t('hgNetworkError'), true);
    } else if (err.code === 'invalid_input' || err.status === 400) {
      setLoginMessage(t('hgOtpInvalid'), true);
    } else {
      setLoginMessage(localizedError(err), true);
    }
    const input = $('hgOtpInput');
    if (input) input.focus();
  } finally {
    state.otp.busy = false;
    if (submit) submit.disabled = false;
  }
}

function changeMobile() {
  state.otp.requested = false;
  stopOtpTimer();
  const mobile = $('hgMobileInput');
  if (mobile) {
    mobile.disabled = false;
    mobile.focus();
  }
  const otp = $('hgOtpInput');
  if (otp) otp.value = '';
  show('hgOtpLabel', false);
  show('hgOtpInput', false);
  show('hgResendBtn', false);
  show('hgChangeMobile', false);
  const submit = $('hgOtpSubmit');
  if (submit) submit.textContent = t('hgSendOtp');
  setLoginMessage('', false);
}

async function logout() {
  try {
    await invoke('portal_logout');
  } catch (_) {
    // Clearing local state is still correct if the network call fails.
  }
  state.session = {
    loggedIn: false,
    user: null,
    persistent: state.session.persistent,
    restoring: false,
  };
  state.accounts = [];
  state.packages = [];
  state.detail = null;
  state.screen = 'loggedout';
  state.memberCredential = null;
  state.logoutArmed = false;
  closeAccountModal();
  render();
  toast(t('hgLogoutKeepsConfig'));
  if (typeof window.appRefreshHomeCard === 'function') window.appRefreshHomeCard();
}

/* ------------------------------------------------------------------ */
/* Loading                                                            */
/* ------------------------------------------------------------------ */

async function loadPending() {
  try {
    state.pending = await invoke('portal_pending_purchase');
  } catch (_) {
    state.pending = null;
  }
}

async function loadPackages() {
  try {
    const packages = await invoke('portal_list_packages');
    state.packages = Array.isArray(packages)
      ? packages.filter((pkg) => !pkg.status || pkg.status === 'active')
      : [];
  } catch (error) {
    handleSessionError(error);
  }
}

async function loadAll(options = {}) {
  if (!state.session.loggedIn) return;
  try {
    const [accounts, packages] = await Promise.all([
      invoke('portal_list_accounts'),
      invoke('portal_list_packages'),
    ]);
    state.accounts = Array.isArray(accounts) ? accounts : [];
    state.packages = Array.isArray(packages)
      ? packages.filter((pkg) => !pkg.status || pkg.status === 'active')
      : [];
  } catch (error) {
    if (!options.silent) toast(localizedError(error), true);
    handleSessionError(error);
    return;
  }
  if (state.screen !== 'detail' && state.screen !== 'packages') {
    render();
  }
  if (typeof window.appRefreshHomeCard === 'function') window.appRefreshHomeCard();
}

function handleSessionError(error) {
  const err = normalizeError(error);
  if (err.code === 'session_expired' || err.code === 'not_logged_in') {
    state.session = {
      loggedIn: false,
      user: null,
      persistent: state.session.persistent,
      restoring: false,
    };
    state.screen = 'loggedout';
    render();
    toast(t('hgSessionExpired'), true);
  }
}

/* ------------------------------------------------------------------ */
/* Account list                                                       */
/* ------------------------------------------------------------------ */

function statusChip(account) {
  const chip = el(
    'span',
    `hg-chip ${account.category === 'active' ? 'ok' : account.category === 'expired' ? 'warn' : 'bad'}`,
    account.statusDisplay || account.status || ''
  );
  return chip;
}

function accountCard(account) {
  const card = el('article', 'hg-card');

  const head = el('div', 'hg-card-head');
  const titles = el('div', 'hg-card-titles');
  titles.appendChild(el('strong', 'hg-card-title', account.packageName || '—'));
  titles.appendChild(account.username ? el('small', 'hg-card-user mono', account.username) : el('small'));
  head.appendChild(titles);
  head.appendChild(statusChip(account));
  if (account.isLegacyUnlimited) {
    head.appendChild(el('span', 'hg-chip legacy', t('hgLegacyBadge')));
  }
  card.appendChild(head);

  const percent = usagePercent(account);
  const bar = el('div', 'hg-bar');
  const fill = el('div', `hg-bar-fill ${percent >= 90 ? 'bad' : ''}`);
  fill.style.width = `${percent}%`;
  bar.appendChild(fill);
  card.appendChild(bar);

  const meta = el('div', 'hg-meta');
  meta.appendChild(el('span', 'hg-meta-usage', account.usageDisplay || `${percent}%`));
  const days = daysLeft(account.expiredAt);
  const expiryText = account.expiredAt
    ? `${t('hgExpiry')}: ${fmtDate(account.expiredAt)}${days !== null ? ` (${days > 0 ? days : 0})` : ''}`
    : `${t('hgExpiry')}: —`;
  meta.appendChild(el('span', 'hg-meta-expiry', expiryText));
  card.appendChild(meta);

  const chips = el('div', 'hg-chip-row');
  if (account.pendingReservesCount > 0) {
    chips.appendChild(
      el('span', 'hg-chip ok', `${t('hgRenewReserved')} ×${account.pendingReservesCount}`)
    );
  }
  if (account.includedUsers) {
    chips.appendChild(el('span', 'hg-chip', tf('hgUsersCount', { n: account.includedUsers })));
  }
  if (chips.childElementCount > 0) card.appendChild(chips);

  const actions = el('div', 'hg-card-actions');
  actions.appendChild(
    button(t('hgDetails'), 'btn ghost sm', () => openDetail(account.id))
  );
  if (account.category !== 'support') {
    actions.appendChild(
      button(t('hgRenew'), 'btn primary sm', () =>
        openPackages({ kind: 'renew', accountId: account.id })
      )
    );
  } else {
    actions.appendChild(el('span', 'hg-meta-expiry', t('hgSupport')));
  }
  card.appendChild(actions);
  return card;
}

function renderList() {
  const active = state.accounts.filter((a) => a.category === 'active');
  const expired = state.accounts.filter((a) => a.category === 'expired');
  const support = state.accounts.filter((a) => a.category === 'support');

  const activeList = $('hgActiveList');
  const expiredList = $('hgExpiredList');
  const supportList = $('hgSupportList');

  if (activeList) {
    clear(activeList);
    active.forEach((account) => activeList.appendChild(accountCard(account)));
  }
  if (expiredList) {
    clear(expiredList);
    expired.forEach((account) => expiredList.appendChild(accountCard(account)));
  }
  if (supportList) {
    clear(supportList);
    support.forEach((account) => supportList.appendChild(accountCard(account)));
  }
  show('hgExpiredHead', expired.length > 0);
  show('hgSupportHead', support.length > 0);
  show('hgNoAccounts', state.accounts.length === 0);
  show('hgKeyringNotice', state.session.loggedIn && !state.session.persistent);

  setText('hgAccountName', displayName());
  setText('hgAccountMobile', state.session.user?.mobile || '');
  const badge = $('hgShahkarBadge');
  if (badge) {
    badge.textContent = isShahkarVerified()
      ? t('hgShahkarVerified')
      : t('hgShahkarUnverified');
    badge.className = `hg-badge ${isShahkarVerified() ? 'ok' : 'warn'}`;
  }

  renderPendingBanner();
}

/* ------------------------------------------------------------------ */
/* Pending payment banner                                             */
/* ------------------------------------------------------------------ */

function pendingPackageName(pending) {
  if (!pending || !pending.packageId) return '';
  const pkg = state.packages.find((item) => item.id === pending.packageId);
  return pkg ? pkg.name : '';
}

function renderPendingBanner() {
  const banner = $('hgPaymentBanner');
  if (!banner) return;
  const pending = state.pending;
  if (!pending || !pending.invoiceId || !state.session.loggedIn) {
    banner.classList.add('hidden');
    return;
  }
  banner.classList.remove('hidden');
  setText(
    'hgPaymentTitle',
    pending.mode === 'renew' ? t('hgRenew') : t('hgPaymentPending')
  );
  const parts = [];
  if (pendingPackageName(pending)) parts.push(pendingPackageName(pending));
  if (pending.stage === 'browser_opened') parts.push(t('hgPaymentPending'));
  const short = String(pending.invoiceId).slice(0, 8);
  if (short) parts.push(`#${short}`);
  setText('hgPaymentMeta', parts.join(' · '));
  show('hgResumePayment', Boolean(pending.invoiceId));
}

async function checkPending(options = {}) {
  const silent = Boolean(options && options.silent);
  const before = state.pending;
  if (!silent) setBusy(true, t('hgCheckStatus'));
  try {
    const result = await invoke('portal_reconcile_pending');
    switch (result.state) {
      case 'paid': {
        state.pending = null;
        if (Array.isArray(result.accounts)) state.accounts = result.accounts;
        const expectedPackage = before?.packageId;
        const found = expectedPackage
          ? state.accounts.some((a) => a.package === expectedPackage)
          : true;
        render();
        await loadAll({ silent: true });
        if (!found) {
          toast(t('hgProvisioning'));
        } else {
          toast(t('hgPaymentSuccess'));
        }
        if (state.screen === 'list') render();
        break;
      }
      case 'canceled':
        state.pending = null;
        renderPendingBanner();
        if (!silent) toast(t('hgPaymentCanceled'));
        break;
      case 'refunded':
        state.pending = null;
        renderPendingBanner();
        if (!silent) toast(t('hgPaymentRefunded'));
        break;
      case 'failed':
        state.pending = null;
        renderPendingBanner();
        if (!silent) toast(t('hgPaymentFailed'), true);
        break;
      case 'expired':
        state.pending = null;
        renderPendingBanner();
        if (!silent) toast(t('hgPendingExpired'));
        break;
      case 'none':
        state.pending = null;
        renderPendingBanner();
        break;
      default:
        await loadPending();
        renderPendingBanner();
        break;
    }
  } catch (error) {
    handleSessionError(error);
    if (!silent) toast(localizedError(error), true);
  } finally {
    if (!silent) setBusy(false);
  }
}

async function resumePayment() {
  const pending = state.pending;
  if (!pending || !pending.invoiceId) return;
  setBusy(true, t('hgResumePayment'));
  try {
    const pay = await invoke('portal_invoice_pay', { invoiceId: pending.invoiceId });
    await invoke('portal_open_payment', {
      invoiceId: pay.invoiceId || pending.invoiceId,
      paymentUrl: pay.paymentUrl,
      mode: pending.mode || 'buy',
      accountId: pending.accountId || null,
      packageId: pending.packageId || null,
      planId: pending.planId || null,
      paymentId: pay.paymentId || pending.paymentId || null,
    });
    await loadPending();
    renderPendingBanner();
    toast(t('hgPaymentReturnHint'));
  } catch (error) {
    handleSessionError(error);
    toast(localizedError(error), true);
  } finally {
    setBusy(false);
  }
}

async function forgetPending() {
  try {
    await invoke('portal_cancel_pending');
  } catch (_) {
    // best effort
  }
  state.pending = null;
  renderPendingBanner();
}

/* ------------------------------------------------------------------ */
/* Detail                                                             */
/* ------------------------------------------------------------------ */

async function openDetail(accountId) {
  state.screen = 'detail';
  state.detail = null;
  state.memberCredential = null;
  showScreen('detail');
  const body = $('hgDetailBody');
  if (body) body.replaceChildren(el('div', 'hg-loading', t('hgRestoring')));
  const actions = $('hgDetailActions');
  if (actions) clear(actions);
  const members = $('hgDetailMembers');
  if (members) clear(members);

  try {
    state.detail = await invoke('portal_account_detail', { accountId });
    render();
  } catch (error) {
    handleSessionError(error);
    if (body) {
      clear(body);
      const box = el('div', 'hg-empty');
      box.appendChild(el('span', null, t('hgAccountUnavailable')));
      box.appendChild(button(t('hgRetry'), 'btn ghost sm', () => openDetail(accountId)));
      body.appendChild(box);
    }
    toast(localizedError(error), true);
  }
}

async function refreshDetail() {
  const id = state.detail?.id;
  if (!id) return;
  try {
    state.detail = await invoke('portal_account_detail', { accountId: id });
    renderDetail();
  } catch (error) {
    handleSessionError(error);
    toast(localizedError(error), true);
  }
}

function detailId() {
  return state.detail?.id || null;
}

function renderDetail() {
  const detail = state.detail;
  if (!detail) return;
  const body = $('hgDetailBody');
  if (!body) return;
  clear(body);

  /* Header */
  const header = el('div', 'hg-detail-header');
  const titles = el('div', 'hg-card-titles');
  titles.appendChild(el('h1', 'hg-detail-title', detail.packageName || '—'));
  titles.appendChild(el('small', 'hg-card-user mono', detail.username || ''));
  header.appendChild(titles);
  header.appendChild(statusChip(detail));
  if (detail.isLegacyUnlimited) {
    header.appendChild(el('span', 'hg-chip legacy', t('hgLegacyBadge')));
  }
  body.appendChild(header);

  /* Usage */
  const usageCard = el('div', 'card hg-detail-card');
  usageCard.appendChild(el('span', 'group-label', t('hgUsage')));
  const percent = Math.round(Number(detail.usagePercent) || usagePercent(detail));
  const big = el('div', 'hg-usage-big');
  big.appendChild(el('strong', null, `${percent}%`));
  big.appendChild(el('span', 'hg-meta-usage', detail.usageDisplay || ''));
  usageCard.appendChild(big);
  const bar = el('div', 'hg-bar');
  const fill = el('div', `hg-bar-fill ${percent >= 90 ? 'bad' : ''}`);
  fill.style.width = `${Math.min(100, percent)}%`;
  bar.appendChild(fill);
  usageCard.appendChild(bar);
  body.appendChild(usageCard);

  /* Expiry */
  const expiryCard = el('div', 'card hg-detail-card');
  expiryCard.appendChild(el('span', 'group-label', t('hgExpiry')));
  const expiryRow = el('div', 'hg-panel-row');
  expiryRow.appendChild(el('span', null, t('hgExpiry')));
  expiryRow.appendChild(el('strong', null, fmtDate(detail.expiredAt)));
  expiryCard.appendChild(expiryRow);
  if (detail.projectedExpiredAt) {
    const projected = el('div', 'hg-panel-row');
    projected.appendChild(el('span', null, t('hgProjectedExpiry')));
    projected.appendChild(el('strong', null, fmtDate(detail.projectedExpiredAt)));
    expiryCard.appendChild(projected);
  }
  const days = daysLeft(detail.expiredAt);
  if (days !== null) {
    expiryCard.appendChild(
      el(
        'small',
        'hg-meta-expiry',
        document.documentElement.lang === 'fa'
          ? `${Math.max(0, days)} روز مانده`
          : `${Math.max(0, days)} days left`
      )
    );
  }
  body.appendChild(expiryCard);

  /* Reserves */
  const reserveCard = el('div', 'card hg-detail-card');
  reserveCard.appendChild(el('span', 'group-label', t('hgQueuedRenewals')));
  if (!detail.pendingReserves || detail.pendingReserves.length === 0) {
    reserveCard.appendChild(el('small', 'hg-meta-expiry', t('hgNoReserves')));
  } else {
    detail.pendingReserves.forEach((reserve) => {
      const row = el('div', 'hg-panel-row');
      row.appendChild(
        el('span', null, reserve.packageName || fmtTraffic(reserve.trafficBytes))
      );
      row.appendChild(el('small', 'hg-meta-expiry', fmtDate(reserve.createdAt)));
      reserveCard.appendChild(row);
    });
  }
  body.appendChild(reserveCard);

  /* Credentials */
  const credCard = el('div', 'card hg-detail-card');
  credCard.appendChild(el('span', 'group-label', t('hgCredentials')));

  const userRow = el('div', 'hg-panel-row');
  userRow.appendChild(el('span', null, t('hgUsername')));
  const userValue = el('strong', 'mono', detail.username || '—');
  userRow.appendChild(userValue);
  userRow.appendChild(
    button(t('hgCopy'), 'btn ghost sm', () =>
      copyText(detail.username, 'hgCopied')
    )
  );
  credCard.appendChild(userRow);

  const passRow = el('div', 'hg-panel-row');
  passRow.appendChild(el('span', null, t('hgPassword')));
  let revealed = false;
  const passValue = el('strong', 'mono', '••••••••');
  passRow.appendChild(passValue);
  const revealBtn = button(t('hgReveal'), 'btn ghost sm', () => {
    revealed = !revealed;
    passValue.textContent = revealed ? detail.password || '—' : '••••••••';
    revealBtn.textContent = revealed ? t('hgHide') : t('hgReveal');
    if (revealed) {
      clearTimeout(revealBtn._hideTimer);
      revealBtn._hideTimer = setTimeout(() => {
        revealed = false;
        passValue.textContent = '••••••••';
        revealBtn.textContent = t('hgReveal');
      }, 60000);
    }
  });
  passRow.appendChild(revealBtn);
  passRow.appendChild(
    button(t('hgCopy'), 'btn ghost sm', () => copyText(detail.password, 'hgCopyWarning'))
  );
  credCard.appendChild(passRow);

  const linkRow = el('div', 'hg-panel-row');
  linkRow.appendChild(el('span', null, t('hgCopyLink')));
  linkRow.appendChild(el('strong', 'mono hg-link-mask', maskLink(detail.trojanUrl)));
  linkRow.appendChild(
    button(t('hgCopy'), 'btn ghost sm', () =>
      copyText(detail.trojanUrl, 'hgCopyLinkWarn')
    )
  );
  credCard.appendChild(linkRow);
  body.appendChild(credCard);

  /* Actions */
  const actions = $('hgDetailActions');
  if (actions) {
    clear(actions);
    actions.appendChild(
      button(t('hgApply'), 'btn primary block', () => applyConfig(detailId(), actions))
    );
    const row = el('div', 'hg-action-row');
    if (detail.category !== 'support') {
      row.appendChild(
        button(t('hgRenew'), 'btn ghost', () =>
          openPackages({ kind: 'renew', accountId: detailId() })
        )
      );
    }
    row.appendChild(button(t('hgSyncQuota'), 'btn ghost', syncQuota));
    actions.appendChild(row);
  }

  renderMembers();
}

async function applyConfig(accountId, container) {
  if (!accountId) return;
  const target = container ? container.querySelector('.btn.primary') : null;
  if (target) target.disabled = true;
  setBusy(true, t('hgApply'));
  try {
    await invoke('portal_apply_config', { accountId });
    toast(t('hgApplySuccess'));
    if (typeof window.appConfigChanged === 'function') {
      await window.appConfigChanged({ source: 'haiogate', accountId });
    }
  } catch (error) {
    toast(localizedError(error), true);
  } finally {
    setBusy(false);
    if (target) target.disabled = false;
  }
}

async function syncQuota() {
  const id = detailId();
  if (!id) return;
  setBusy(true, t('hgSyncQuota'));
  try {
    state.detail = await invoke('portal_sync_quota', { accountId: id });
    renderDetail();
    toast(t('hgSynced'));
  } catch (error) {
    handleSessionError(error);
    toast(localizedError(error), true);
  } finally {
    setBusy(false);
  }
}

/* ------------------------------------------------------------------ */
/* Members                                                            */
/* ------------------------------------------------------------------ */

function memberStatusLabel(status) {
  switch (status) {
    case 'active':
      return t('hgMemberStatusActive');
    case 'pending':
      return t('hgMemberStatusPending');
    case 'revoked':
      return t('hgMemberStatusRevoked');
    default:
      return status || '—';
  }
}

function memberCredentialBox(member) {
  const box = el('div', 'hg-cred-box');
  box.appendChild(el('strong', null, member.displayName || member.username || ''));
  box.appendChild(el('small', 'hg-cred-warn', t('hgMemberCredentialOnce')));
  box.appendChild(el('small', 'hg-cred-warn', t('hgMemberCredentialWarning')));

  const userRow = el('div', 'hg-panel-row');
  userRow.appendChild(el('span', null, t('hgUsername')));
  userRow.appendChild(el('strong', 'mono', member.username || '—'));
  userRow.appendChild(
    button(t('hgCopy'), 'btn ghost sm', () => copyText(member.username, 'hgCopied'))
  );
  box.appendChild(userRow);

  if (member.password) {
    const passRow = el('div', 'hg-panel-row');
    passRow.appendChild(el('span', null, t('hgPassword')));
    passRow.appendChild(el('strong', 'mono', member.password));
    passRow.appendChild(
      button(t('hgCopy'), 'btn ghost sm', () =>
        copyText(member.password, 'hgCopyWarning')
      )
    );
    box.appendChild(passRow);
  }

  if (member.trojanUrl) {
    const linkRow = el('div', 'hg-panel-row');
    linkRow.appendChild(el('span', null, t('hgCopyLink')));
    linkRow.appendChild(el('strong', 'mono hg-link-mask', maskLink(member.trojanUrl)));
    linkRow.appendChild(
      button(t('hgCopy'), 'btn ghost sm', () =>
        copyText(member.trojanUrl, 'hgMemberCredentialWarning')
      )
    );
    box.appendChild(linkRow);
  }
  return box;
}

function renderMembers() {
  const container = $('hgDetailMembers');
  const detail = state.detail;
  if (!container || !detail) return;
  clear(container);

  const head = el('div', 'hg-section-head');
  head.appendChild(el('span', 'section-title', t('hgMembers')));
  const included = detail.includedUsers;
  if (included) {
    const liveMembers = (detail.members || []).filter(
      (member) => member.status === 'active' || member.status === 'pending'
    ).length;
    const used = 1 + liveMembers;
    head.appendChild(el('span', 'count-pill', tf('hgUsersSlots', { used, total: included })));
  } else {
    head.appendChild(el('span', 'hg-chip legacy', t('hgLegacyUnlimited')));
  }
  container.appendChild(head);

  if (state.memberCredential) {
    container.appendChild(memberCredentialBox(state.memberCredential));
  }

  if (detail.isLegacyUnlimited || !included) {
    container.appendChild(
      el('small', 'hg-meta-expiry', t('hgLegacyUnlimited'))
    );
    container.appendChild(el('small', 'hg-meta-expiry', t('hgMemberLoginNote')));
    return;
  }

  /* Primary */
  const primary = el('div', 'hg-member-row');
  const primaryCopy = el('div', 'hg-member-copy');
  primaryCopy.appendChild(el('strong', null, t('hgPrimary')));
  primaryCopy.appendChild(el('small', 'mono', detail.username || ''));
  primary.appendChild(primaryCopy);
  primary.appendChild(el('span', 'hg-chip ok', t('hgMemberStatusActive')));
  primary.appendChild(
    button(t('hgCopy'), 'btn ghost sm', () =>
      copyText(detail.trojanUrl, 'hgCopyLinkWarn')
    )
  );
  container.appendChild(primary);

  /* Members */
  const members = detail.members || [];
  if (members.length === 0) {
    container.appendChild(el('small', 'hg-meta-expiry', t('hgNoMembers')));
  }
  members.forEach((member) => {
    const row = el('div', 'hg-member-row');
    const copy = el('div', 'hg-member-copy');
    copy.appendChild(el('strong', null, member.displayName || member.username));
    copy.appendChild(el('small', 'mono', member.username));
    row.appendChild(copy);
    const isActive = member.status === 'active' || member.status === 'pending';
    row.appendChild(
      el('span', `hg-chip ${isActive ? 'ok' : ''}`, memberStatusLabel(member.status))
    );

    if (isActive) {
      const rotate = button(t('hgMemberRotate'), 'btn ghost sm', () =>
        armConfirm(rotate, t('hgRotateWarn'), () =>
          rotateMember(member)
        )
      );
      row.appendChild(rotate);

      const revoke = button(t('hgMemberRevoke'), 'btn danger sm', () =>
        armConfirm(revoke, t('hgRevokeWarn'), () =>
          revokeMember(member)
        )
      );
      row.appendChild(revoke);
    }
    container.appendChild(row);
  });

  /* Add member form */
  const available = Number(detail.availableSlots);
  const canAdd = detail.category === 'active';
  if (canAdd && Number.isFinite(available) && available > 0) {
    const form = el('div', 'hg-member-form');
    const name = el('input', 'text-input');
    name.type = 'text';
    name.maxLength = 100;
    name.placeholder = t('hgMemberNamePlaceholder');
    name.setAttribute('aria-label', t('hgMemberName'));
    const mobile = el('input', 'text-input mono');
    mobile.type = 'tel';
    mobile.inputMode = 'numeric';
    mobile.maxLength = 11;
    mobile.placeholder = t('hgMemberMobileOptional');
    mobile.setAttribute('aria-label', t('hgMemberMobileOptional'));
    mobile.dir = 'ltr';
    const add = button(t('hgMemberAdd'), 'btn primary sm', () =>
      addMember(name, mobile, add)
    );
    form.appendChild(name);
    form.appendChild(mobile);
    form.appendChild(add);
    container.appendChild(form);
    container.appendChild(el('small', 'hg-meta-expiry', t('hgMemberLoginNote')));
  } else if (canAdd) {
    container.appendChild(el('small', 'hg-meta-expiry', t('hgMemberLimit')));
  }
}

function armConfirm(target, confirmLabel, action) {
  if (target.dataset.armed === '1') {
    delete target.dataset.armed;
    target.classList.remove('confirming');
    action();
    return;
  }
  target.dataset.armed = '1';
  target.classList.add('confirming');
  const original = target.textContent;
  target.textContent = confirmLabel.length > 28 ? t('hgConfirm') : confirmLabel;
  clearTimeout(target._armTimer);
  target._armTimer = setTimeout(() => {
    delete target.dataset.armed;
    target.classList.remove('confirming');
    target.textContent = original;
  }, 4000);
}

async function addMember(nameInput, mobileInput, addButton) {
  const id = detailId();
  if (!id) return;
  const displayName = (nameInput.value || '').trim();
  if (!displayName) {
    nameInput.classList.add('bad');
    nameInput.focus();
    return;
  }
  addButton.disabled = true;
  setBusy(true, t('hgMemberAdd'));
  try {
    const member = await invoke('portal_add_member', {
      accountId: id,
      displayName,
      mobile: normalizeDigits(mobileInput.value) || null,
    });
    state.memberCredential = member;
    toast(t('hgMemberAdded'));
    await refreshDetail();
  } catch (error) {
    handleSessionError(error);
    toast(localizedError(error), true);
  } finally {
    setBusy(false);
    addButton.disabled = false;
  }
}

async function rotateMember(member) {
  const id = detailId();
  if (!id) return;
  setBusy(true, t('hgMemberRotate'));
  try {
    const rotated = await invoke('portal_rotate_member', {
      accountId: id,
      memberId: member.id,
    });
    state.memberCredential = rotated;
    toast(t('hgMemberRotated'));
    await refreshDetail();
  } catch (error) {
    handleSessionError(error);
    toast(localizedError(error), true);
  } finally {
    setBusy(false);
  }
}

async function revokeMember(member) {
  const id = detailId();
  if (!id) return;
  setBusy(true, t('hgMemberRevoke'));
  try {
    await invoke('portal_revoke_member', { accountId: id, memberId: member.id });
    state.memberCredential = null;
    toast(t('hgMemberRevoked'));
    await refreshDetail();
  } catch (error) {
    handleSessionError(error);
    toast(localizedError(error), true);
  } finally {
    setBusy(false);
  }
}

/* ------------------------------------------------------------------ */
/* Packages and purchase                                              */
/* ------------------------------------------------------------------ */

async function openPackages(flow) {
  state.flow = flow;
  state.selectedPackage = null;
  state.intent = null;
  state.orgContext = null;

  // A second purchase while one is pending risks a double charge. Offer the
  // existing payment (or an explicit dismiss) instead.
  if (state.pending && state.pending.invoiceId) {
    state.screen = 'list';
    state.flow = null;
    if (typeof window.appEnterPortal === 'function') window.appEnterPortal();
    render();
    toast(t('hgPendingPurchaseBlocked'), true);
    return;
  }

  state.screen = 'packages';
  if (typeof window.appEnterPortal === 'function') window.appEnterPortal();
  showScreen('packages');
  renderPackages();

  if (!state.session.loggedIn) {
    state.resumeAfterLogin = () => openPackages(flow);
    openLogin();
    return;
  }
  if (state.packages.length === 0) {
    await loadPackages();
  }
  renderPackages();
}

function packageCard(pkg) {
  const card = el('button', 'hg-package-card');
  card.type = 'button';
  const head = el('div', 'hg-card-head');
  head.appendChild(el('strong', 'hg-card-title', pkg.name));
  head.appendChild(el('span', 'hg-chip ok', tf('hgUsersCount', { n: pkg.includedUsers || 1 })));
  card.appendChild(head);
  card.appendChild(el('span', 'hg-package-price', fmtPrice(pkg.price)));
  card.appendChild(el('small', 'hg-meta-expiry', `${t('hgTraffic')}: ${fmtTraffic(pkg.trafficBytes)}`));
  if (pkg.description) {
    card.appendChild(el('small', 'hg-package-desc', pkg.description));
  }

  const currentAccount = state.flow?.kind === 'renew'
    ? state.accounts.find((a) => a.id === state.flow.accountId)
    : null;
  if (currentAccount && currentAccount.package === pkg.id) {
    card.appendChild(el('span', 'hg-chip', t('hgCurrentPackage')));
  }

  if (state.selectedPackage?.id === pkg.id) {
    card.classList.add('selected');
  }
  card.addEventListener('click', () => selectPackage(pkg));
  return card;
}

function renderPackages() {
  const list = $('hgPackagesList');
  if (!list) return;
  clear(list);
  show('hgOrgPicker', false);
  show('hgConfirmBox', false);

  const isRenew = state.flow?.kind === 'renew';
  setText('hgPackagesTitle', isRenew ? t('hgRenew') : t('hgBuyNew'));
  setText(
    'hgPackagesHint',
    state.packages.length ? t('hgSelectPackage') : t('hgNoPackages')
  );

  if (state.packages.length === 0) {
    const box = el('div', 'hg-empty');
    box.appendChild(el('span', null, t('hgNoPackages')));
    list.appendChild(box);
    return;
  }

  state.packages.forEach((pkg) => list.appendChild(packageCard(pkg)));
  if (state.selectedPackage) {
    renderConfirm(state.selectedPackage);
  }
}

function selectPackage(pkg) {
  state.selectedPackage = pkg;
  state.intent = null;

  // Buying a package the user already owns is a renewal, not a duplicate.
  if (state.flow?.kind === 'buy') {
    const existing = state.accounts.find((account) => account.package === pkg.id);
    if (existing) {
      state.flow = { kind: 'renew', accountId: existing.id };
      toast(t('hgDuplicatePackage'));
    }
  }

  renderPackages();
}

function renderConfirm(pkg) {
  const box = $('hgConfirmBox');
  if (!box) return;
  clear(box);
  box.classList.remove('hidden');

  const flow = state.flow || {};
  const isRenew = flow.kind === 'renew';
  const account = isRenew
    ? state.accounts.find((item) => item.id === flow.accountId)
    : null;

  const summary = el('div', 'card hg-confirm-card');
  summary.appendChild(el('span', 'group-label', isRenew ? t('hgRenew') : t('hgBuyNew')));
  summary.appendChild(el('strong', 'hg-confirm-title', pkg.name));

  const rows = [
    [t('hgPrice'), fmtPrice(pkg.price)],
    [t('hgTraffic'), fmtTraffic(pkg.trafficBytes)],
    [t('hgPeriod'), t('hgPeriod30')],
    [t('hgUsersLabel'), tf('hgUsersCount', { n: pkg.includedUsers || 1 })],
  ];
  rows.forEach(([label, value]) => {
    const row = el('div', 'hg-panel-row');
    row.appendChild(el('span', null, label));
    row.appendChild(el('strong', null, value));
    summary.appendChild(row);
  });

  let note = t('hgSeparateAccount');
  if (isRenew && account) {
    note = account.category === 'active' ? t('hgRenewReserveInfo') : t('hgRenewNowInfo');
  }
  summary.appendChild(el('p', 'hg-confirm-note', note));

  if (
    isRenew &&
    account &&
    account.includedUsers &&
    pkg.includedUsers &&
    pkg.includedUsers < account.includedUsers
  ) {
    summary.appendChild(el('p', 'hg-confirm-warn', t('hgDowngradeWarning')));
  }

  if (isRenew && account && account.isLegacyUnlimited) {
    summary.appendChild(el('p', 'hg-confirm-warn', t('hgLegacyConversionWarning')));
  }

  summary.appendChild(
    button(
      isRenew ? t('hgConfirmRenew') : t('hgConfirmBuy'),
      'btn primary block',
      () => confirmPurchase()
    )
  );
  box.appendChild(summary);
  box.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

async function confirmPurchase() {
  const pkg = state.selectedPackage;
  const flow = state.flow;
  if (!pkg || !flow) return;

  if (!state.session.loggedIn) {
    state.resumeAfterLogin = () => openPackages(flow);
    openLogin();
    return;
  }
  if (!isShahkarVerified()) {
    state.shahkarContinuation = () => confirmPurchase();
    openShahkar();
    return;
  }

  setBusy(true, t('hgBusy'));
  try {
    const intent =
      flow.kind === 'renew'
        ? await invoke('portal_start_renew', {
            accountId: flow.accountId,
            packageId: pkg.id,
          })
        : await invoke('portal_start_buy', { packageId: pkg.id });
    state.intent = intent;
    await runBuy(intent, flow, pkg, null);
  } catch (error) {
    await handlePurchaseError(error, state.intent);
  } finally {
    setBusy(false);
  }
}

async function runBuy(intent, flow, pkg, organizationId) {
  let buy;
  try {
    buy = await invoke('portal_buy_plan', {
      planId: intent.planId,
      organizationId: organizationId || null,
    });
  } catch (error) {
    const err = normalizeError(error);
    if (err.code === 'organization_required') {
      state.orgContext = { intent, flow, pkg };
      await showOrgPicker();
      return;
    }
    if (err.code === 'shahkar_required') {
      state.shahkarContinuation = () => runBuy(intent, flow, pkg, organizationId);
      openShahkar();
      return;
    }
    throw error;
  }

  if (buy.status === 'active') {
    await afterPurchase(flow, pkg);
    return;
  }

  if (buy.status !== 'payment_required') {
    throw { code: 'protocol', message: t('hgPaymentFailed') };
  }
  if (!buy.invoiceId || !buy.paymentUrl) {
    throw { code: 'protocol', message: t('hgPaymentFailed') };
  }

  await invoke('portal_open_payment', {
    invoiceId: buy.invoiceId,
    paymentUrl: buy.paymentUrl,
    mode: flow.kind,
    accountId: flow.accountId || null,
    packageId: pkg.id,
    planId: intent.planId,
    paymentId: buy.paymentId || null,
  });

  state.pending = {
    mode: flow.kind,
    accountId: flow.accountId || null,
    packageId: pkg.id,
    planId: intent.planId,
    invoiceId: buy.invoiceId,
    paymentId: buy.paymentId || null,
    stage: 'browser_opened',
  };
  state.screen = 'list';
  state.flow = null;
  state.selectedPackage = null;
  render();
  toast(t('hgPaymentReturnHint'));
}

async function afterPurchase(flow, pkg) {
  await loadAll({ silent: true });
  await loadPending();

  if (flow.kind === 'renew' && flow.accountId) {
    toast(t('hgPaymentSuccess'));
    await openDetail(flow.accountId);
    return;
  }

  const created = state.accounts.find((account) => account.package === pkg.id);
  toast(t('hgPaymentSuccess'));
  state.screen = 'list';
  state.flow = null;
  state.selectedPackage = null;
  render();
  if (created) await openDetail(created.id);
}

async function handlePurchaseError(error, intent) {
  const err = normalizeError(error);
  handleSessionError(err);
  toast(localizedError(err), true);
  if (intent) state.intent = intent;
}

async function showOrgPicker() {
  try {
    const orgs = await invoke('portal_list_orgs');
    state.orgs = Array.isArray(orgs) ? orgs : [];
  } catch (error) {
    toast(localizedError(error), true);
    return;
  }
  const select = $('hgOrgSelect');
  if (!select) return;
  clear(select);
  state.orgs.forEach((org) => {
    const option = el('option', null, org.name || org.slug || String(org.id));
    option.value = String(org.id);
    select.appendChild(option);
  });
  show('hgOrgPicker', true);
  // Avoid a second "pay" click creating a duplicate intent while the org is
  // being chosen.
  show('hgConfirmBox', false);
  toast(t('hgOrgHint'));
}

async function confirmOrg() {
  const context = state.orgContext;
  const select = $('hgOrgSelect');
  if (!context || !select) return;
  const organizationId = select.value;
  if (!organizationId) return;
  show('hgOrgPicker', false);
  setBusy(true, t('hgBusy'));
  try {
    await runBuy(context.intent, context.flow, context.pkg, organizationId);
  } catch (error) {
    await handlePurchaseError(error, context.intent);
  } finally {
    setBusy(false);
    if (state.screen === 'packages' && state.selectedPackage) {
      renderConfirm(state.selectedPackage);
    }
  }
}

/* ------------------------------------------------------------------ */
/* Shahkar                                                            */
/* ------------------------------------------------------------------ */

function openShahkar() {
  const mobile = state.session.user?.mobile || '';
  const mobileInput = $('shahkarMobile');
  if (mobileInput) mobileInput.value = mobile;
  const national = $('shahkarNational');
  if (national) {
    national.value = '';
    national.disabled = false;
  }
  setShahkarMessage('');
  $('shahkarModal')?.classList.remove('hidden');
  if (national) setTimeout(() => national.focus(), 60);
}

function closeShahkar() {
  $('shahkarModal')?.classList.add('hidden');
  const national = $('shahkarNational');
  if (national) national.value = '';
  setShahkarMessage('');
}

function setShahkarMessage(message, isError) {
  const node = $('shahkarMsg');
  if (!node) return;
  node.textContent = message || '';
  node.className = `hg-form-msg${isError ? ' bad' : message ? ' ok' : ''}`;
}

async function submitShahkar() {
  const mobile = state.session.user?.mobile || '';
  if (!isMobileValid(mobile)) {
    setShahkarMessage(t('hgMobileRequired'), true);
    return;
  }
  const national = normalizeDigits($('shahkarNational')?.value || '');
  if (national.length !== 10) {
    setShahkarMessage(t('hgNationalInvalid'), true);
    return;
  }

  const submit = $('shahkarSubmit');
  if (submit) submit.disabled = true;
  setShahkarMessage('', false);
  try {
    const res = await invoke('portal_shahkar_verify', { nationalId: national });
    // Clear immediately: the national code is never persisted.
    const input = $('shahkarNational');
    if (input) input.value = '';

    if (!res.verified) {
      if (res.shahkarStatus === 2) setShahkarMessage(t('hgMismatch'), true);
      else if (res.shahkarStatus === 3) setShahkarMessage(t('hgProviderError'), true);
      else setShahkarMessage(t('hgMismatch'), true);
      return;
    }

    if (res.user) state.session.user = res.user;
    closeShahkar();
    toast(t('hgShahkarSuccess'));
    renderChrome();
    const continuation = state.shahkarContinuation;
    state.shahkarContinuation = null;
    if (typeof continuation === 'function') continuation();
  } catch (error) {
    const err = normalizeError(error);
    const input = $('shahkarNational');
    if (input) input.value = '';
    if (err.code === 'rate_limited') {
      setShahkarMessage(t('hgOtpLockout'), true);
    } else if (err.code === 'network' || err.code === 'timeout') {
      setShahkarMessage(t('hgNetworkError'), true);
    } else if (err.code === 'shahkar_required' || err.status === 403) {
      setShahkarMessage(t('hgShahkarRequired'), true);
    } else if (/duplicate|another account|مشابه|دیگر/i.test(err.message || '')) {
      setShahkarMessage(t('hgDuplicateMobile'), true);
    } else {
      setShahkarMessage(localizedError(err), true);
    }
  } finally {
    if (submit) submit.disabled = false;
  }
}

/* ------------------------------------------------------------------ */
/* Account modal                                                      */
/* ------------------------------------------------------------------ */

function openAccountModal() {
  renderAccountPanel();
  $('accountModal')?.classList.remove('hidden');
}

function closeAccountModal() {
  $('accountModal')?.classList.add('hidden');
  resetLogoutArmed();
}

/* ------------------------------------------------------------------ */
/* Home summary                                                       */
/* ------------------------------------------------------------------ */

async function renderHomeCard(configSource, accountId) {
  const card = $('homeHaioGateCard');
  if (!card) return;
  if (configSource !== 'haiogate' || !accountId) {
    card.classList.add('hidden');
    return;
  }
  card.classList.remove('hidden');

  let account = state.accounts.find((item) => item.id === accountId);
  const canFetch =
    !account &&
    state.session.loggedIn &&
    state.accounts.length === 0 &&
    !state.homeFetchInFlight &&
    Date.now() - (state.homeFetchAt || 0) > 60000;
  if (canFetch) {
    state.homeFetchInFlight = true;
    try {
      const accounts = await invoke('portal_list_accounts');
      if (Array.isArray(accounts)) {
        state.accounts = accounts;
        account = state.accounts.find((item) => item.id === accountId);
      }
    } catch (_) {
      // Offline / expired session: fall through to the unavailable state.
    } finally {
      state.homeFetchInFlight = false;
      state.homeFetchAt = Date.now();
    }
  }

  if (!account) {
    setText('homeHgName', t('hgAccountUnavailable'));
    setText('homeHgUsage', '');
    setText('homeHgExpiry', '');
    const chip = $('homeHgStatus');
    if (chip) {
      chip.textContent = '—';
      chip.className = 'hg-chip';
    }
    return;
  }

  setText('homeHgName', account.packageName || '—');
  setText('homeHgUsage', account.usageDisplay || '');
  const days = daysLeft(account.expiredAt);
  setText(
    'homeHgExpiry',
    account.expiredAt
      ? `${t('hgExpiry')}: ${fmtDate(account.expiredAt)}${days !== null ? ` · ${days > 0 ? days : 0}` : ''}`
      : ''
  );
  const chip = $('homeHgStatus');
  if (chip) {
    chip.textContent = account.statusDisplay || account.status || '';
    chip.className = `hg-chip ${
      account.category === 'active' ? 'ok' : account.category === 'expired' ? 'warn' : 'bad'
    }`;
  }
}

/* ------------------------------------------------------------------ */
/* Events                                                             */
/* ------------------------------------------------------------------ */

function bindEvents() {
  $('accountChip')?.addEventListener('click', () => {
    if (state.session.loggedIn) openAccountModal();
    else {
      if (typeof window.appEnterPortal === 'function') window.appEnterPortal();
      openLogin();
    }
  });

  $('onboardingLoginBtn')?.addEventListener('click', () => {
    if (typeof window.appEnterPortal === 'function') window.appEnterPortal();
    openLogin();
  });

  $('hgLoginBtn')?.addEventListener('click', openLogin);
  $('hgPlansBtn')?.addEventListener('click', () => {
    if (!state.session.loggedIn) {
      toast(t('hgNeedsLoginForPlans'), true);
      openLogin();
      return;
    }
    openPackages({ kind: 'buy' });
  });

  $('hgOtpSubmit')?.addEventListener('click', submitLogin);
  $('hgResendBtn')?.addEventListener('click', requestOtp);
  $('hgChangeMobile')?.addEventListener('click', changeMobile);
  $('hgMobileInput')?.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      submitLogin();
    }
  });
  $('hgOtpInput')?.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      submitLogin();
    }
  });

  $('hgLogoutBtn')?.addEventListener('click', () => {
    if (state.logoutArmed) {
      state.logoutArmed = false;
      logout();
      return;
    }
    state.logoutArmed = true;
    const btn = $('hgLogoutBtn');
    if (btn) btn.textContent = t('hgLogoutConfirm');
    clearTimeout(state.logoutTimer);
    state.logoutTimer = setTimeout(() => {
      state.logoutArmed = false;
      if (btn) btn.textContent = t('hgLogout');
    }, 4000);
  });

  $('hgRefreshBtn')?.addEventListener('click', async () => {
    setBusy(true, t('hgRefresh'));
    try {
      await loadAll({ silent: false });
      await loadPending();
      renderList();
    } finally {
      setBusy(false);
    }
  });

  $('hgBuyBtn')?.addEventListener('click', () => openPackages({ kind: 'buy' }));
  $('buyBannerBtn')?.addEventListener('click', () => {
    if (typeof window.appEnterPortal === 'function') window.appEnterPortal();
    else if (typeof window.switchAppTab === 'function') window.switchAppTab('haiogate');
    if (state.session.loggedIn) openPackages({ kind: 'buy' });
    else openLogin();
  });

  $('hgCheckPayment')?.addEventListener('click', () => checkPending());
  $('hgResumePayment')?.addEventListener('click', resumePayment);
  $('hgForgetPayment')?.addEventListener('click', forgetPending);

  $('hgDetailBack')?.addEventListener('click', () => {
    state.screen = 'list';
    state.detail = null;
    state.memberCredential = null;
    render();
  });
  $('hgPackagesBack')?.addEventListener('click', () => {
    state.screen = state.detail ? 'detail' : 'list';
    state.selectedPackage = null;
    state.flow = null;
    render();
  });
  $('hgOrgConfirm')?.addEventListener('click', confirmOrg);

  $('homeHgManage')?.addEventListener('click', async () => {
    if (typeof window.switchAppTab === 'function') window.switchAppTab('haiogate');
    const accountId = state.homeAccountId;
    if (accountId && state.session.loggedIn) {
      await openDetail(accountId);
    } else if (state.session.loggedIn) {
      state.screen = 'list';
      render();
    } else {
      openLogin();
    }
  });

  $('shahkarCancel')?.addEventListener('click', () => {
    state.shahkarContinuation = null;
    closeShahkar();
  });
  $('shahkarSubmit')?.addEventListener('click', submitShahkar);
  $('shahkarNational')?.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      submitShahkar();
    }
  });

  $('accountPanelClose')?.addEventListener('click', closeAccountModal);
  $('accountPanelShahkarBtn')?.addEventListener('click', () => {
    closeAccountModal();
    openShahkar();
  });
  $('accountPanelLogout')?.addEventListener('click', () => {
    if (state.logoutArmed) {
      state.logoutArmed = false;
      logout();
      return;
    }
    state.logoutArmed = true;
    const btn = $('accountPanelLogout');
    if (btn) {
      btn.classList.add('confirming');
      btn.textContent = t('hgLogoutConfirm');
    }
    clearTimeout(state.logoutTimer);
    state.logoutTimer = setTimeout(resetLogoutArmed, 4000);
  });

  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    if (!$('shahkarModal')?.classList.contains('hidden')) {
      state.shahkarContinuation = null;
      closeShahkar();
    } else if (!$('accountModal')?.classList.contains('hidden')) {
      closeAccountModal();
    }
  });

  window.addEventListener('focus', () => {
    if (!state.session.loggedIn) return;
    const now = Date.now();
    if (now - state.focusAt < 15000) return;
    state.focusAt = now;
    if (!state.pending) return;
    checkPending();
  });

  window.addEventListener('app:language', () => {
    renderChrome();
    renderAccountPanel();
    if (state.session.loggedIn) {
      if (state.screen === 'detail' && state.detail) renderDetail();
      else if (state.screen === 'packages') renderPackages();
      else renderList();
    }
  });

  listen('portal:session', (view) => {
    if (!view || typeof view !== 'object') return;
    applySession(view);
  });
}

/* ------------------------------------------------------------------ */
/* Session application                                                */
/* ------------------------------------------------------------------ */

function applySession(view) {
  // Never regress a resolved session to the restore skeleton: a late
  // `portal_session` response can race the `portal:session` event.
  if (view.restoring && !state.session.restoring) return;

  const wasLoggedIn = state.session.loggedIn;
  state.session = {
    loggedIn: Boolean(view.loggedIn),
    user: view.user || null,
    persistent: view.persistent !== false,
    restoring: Boolean(view.restoring),
  };

  if (state.session.loggedIn) {
    if (state.screen === 'login' || state.screen === 'loggedout') state.screen = 'list';
    if (!wasLoggedIn) {
      // Startup / fresh login: reconcile a purchase that survived a restart.
      loadPending().then(() =>
        loadAll({ silent: true }).then(() => {
          if (state.pending && state.pending.invoiceId) {
            checkPending({ silent: true });
          }
        })
      );
    }
  } else {
    state.accounts = [];
    state.packages = [];
    state.detail = null;
    if (state.screen === 'detail') state.screen = 'list';
  }

  render();
  if (typeof window.appRefreshHomeCard === 'function') window.appRefreshHomeCard();
}

/* ------------------------------------------------------------------ */
/* Public API                                                         */
/* ------------------------------------------------------------------ */

export async function initPortal() {
  if (state.initialised) return;
  state.initialised = true;
  bindEvents();
  render();

  try {
    const view = await invoke('portal_session');
    if (view) applySession(view);
  } catch (_) {
    state.session = {
      loggedIn: false,
      user: null,
      persistent: false,
      restoring: false,
    };
    render();
  }
}

export async function onTabEnter() {
  renderChrome();
  if (state.session.restoring) return;
  if (!state.session.loggedIn) {
    if (state.screen !== 'login') state.screen = 'loggedout';
    render();
    return;
  }
  if (state.screen === 'list') {
    await loadPending();
    await loadAll({ silent: true });
    renderList();
  }
}

export async function refreshSession() {
  try {
    const view = await invoke('portal_session');
    if (view) applySession(view);
  } catch (_) {
    // Keep the current UI; the next command will report the failure.
  }
}

export function isLoggedIn() {
  return state.session.loggedIn;
}

export async function copyToClipboard(value, noticeKey) {
  return copyText(value, noticeKey);
}

export function openLoginScreen() {
  openLogin();
}

export function openBuyFlow() {
  if (state.session.loggedIn) openPackages({ kind: 'buy' });
  else openLogin();
}

export function homeCard(configSource, accountId) {
  state.homeAccountId = accountId || null;
  renderHomeCard(configSource, accountId);
}

window.portalModule = {
  init: initPortal,
  onTabEnter,
  refreshSession,
  isLoggedIn,
  openLogin: openLoginScreen,
  openBuyFlow,
  homeCard,
};
