import { invoke, listen } from './invoke.js';
import { translations } from './i18n.js';
import { initTheme, getThemeMode, getResolvedTheme, setThemeMode, toggleTheme } from './theme.js';
import { initPortal, copyToClipboard } from './portal.js';
import './settings.js';

const $ = (id) => document.getElementById(id);

const configInput = $('configInput');
const configStatus = $('configStatus');
const saveConfigBtn = $('saveConfigBtn');
const toggle = $('proxyToggle');
const toggleTitle = $('toggleTitle');
const domainCount = $('domainCount');
const domainMeta = $('domainMeta');
const statusBadge = $('statusBadge');
const statDomains = $('statDomains');
const statBypass = $('statBypass');
const statServer = $('statServer');
const statusOrb = $('statusOrb');
const portInput = $('portInput');
const errorMsg = $('errorMsg');
const errorText = $('errorText');
const errorClose = $('errorClose');
const consentModal = $('consentModal');
const overlay = $('overlay');
const overlayMsg = $('overlayMsg');
const connStatus = $('connStatus');
const connDot = $('connDot');
const connText = $('connText');
const langToggle = $('langToggle');
const configOnlyCard = $('configOnlyCard');
const dashboard = $('dashboard');
const tabBar = $('tabBar');
const appContent = $('appContent');
const themeToggle = $('themeToggle');
const themeSegmented = $('themeSegmented');
const btnSetup = $('btnSetup');
const trojanStatus = $('trojanStatus');
const trojanStatusText = $('trojanStatusText');
const appVersion = $('appVersion');
const updateBanner = $('updateBanner');
const updateText = $('updateText');
const btnInstallUpdate = $('btnInstallUpdate');
const bypassInput = $('bypassInput');
const bypassForm = $('bypassForm');
const bypassSearch = $('bypassSearch');
const bypassList = $('bypassList');
const bypassCount = $('bypassCount');
const quickBypassCount = $('quickBypassCount');
const btnAddBypass = $('btnAddBypass');
const btnRefreshDomains = $('btnRefreshDomains');
const btnCopyConfig = $('btnCopyConfig');
const shareWarnModal = $('shareWarnModal');
const shareWarnCopy = $('shareWarnCopy');
const shareWarnCancel = $('shareWarnCancel');
const btnDeleteConfig = $('btnDeleteConfig');
const googlePreset = $('googlePreset');
const googlePresetAction = $('googlePresetAction');
const aiDirectWarn = $('aiDirectWarn');
const aiDirectFix = $('aiDirectFix');

const LANG_KEY = 'haiobypass:lang';

// Curated Google web domains that are safe to route directly. Only product
// surfaces whose pages do not depend on Google AI assets belong here; the AI
// hosts below must always stay on the tunnel so anti-sanction keeps working.
const GOOGLE_PRESET_DOMAINS = Object.freeze([
  'books.google.com',
  'calendar.google.com',
  'docs.google.com',
  'doubleclickbygoogle.com',
  'drive.google.com',
  'earth.google.com',
  'google-analytics.com',
  'googleadservices.com',
  'googlesyndication.com',
  'googletagmanager.com',
  'keep.google.com',
  'mail.google.com',
  'maps.google.com',
  'news.google.com',
  'photos.google.com',
  'scholar.google.com',
  'sheets.google.com',
  'slides.google.com',
  'translate.google.com',
]);

// Hosts Gemini / Flow / AI Studio depend on. They serve the AI pages' own
// scripts and images and they answer the region-eligibility check, so routing
// them direct makes Google see the Iranian IP ("Gemini isn't currently
// supported in your country") and drops the images behind the national filter.
const GOOGLE_AI_TUNNEL_ONLY = Object.freeze([
  'accounts.google.com',
  'apis.google.com',
  'fonts.googleapis.com',
  'googleusercontent.com',
  'gstatic.com',
  'play.google.com',
  'www.google.com',
]);
const VALID_TABS = ['home', 'haiogate', 'bypass', 'settings'];
const CONN_KEY = {
  off: 'proxyOff',
  checking: 'checking',
  connected: 'connected',
  failed: 'failed',
};

let currentLang = 'en';
let lastRefreshTime = 0;
let refreshTimer = null;
let domainSourceUrl = '';
let pendingUpdate = null;
let bypassDomains = [];
let parsedConfig = null;
let lastStatus = null;
let lastDomainError = '';
let connState = 'off';
let deleteTimer = null;
let toastTimer = null;
let trojanRunning = false;
let needsProxyConsent = false;
let currentTab = 'home';
let configSource = 'manual';
let haiogateAccountId = null;
let copyWarnResolve = null;

/* ------------------------------------------------------------------ */
/* Language                                                            */
/* ------------------------------------------------------------------ */

function readStoredLang() {
  try {
    const stored = localStorage.getItem(LANG_KEY);
    return stored === 'fa' ? 'fa' : 'en';
  } catch (_) {
    return 'en';
  }
}

function storeLang(lang) {
  try {
    localStorage.setItem(LANG_KEY, lang);
  } catch (_) {
    // Non-fatal.
  }
}

function setLanguage(lang) {
  currentLang = translations[lang] ? lang : 'en';
  const t = translations[currentLang];
  const html = document.documentElement;
  html.lang = currentLang;
  html.dir = currentLang === 'fa' ? 'rtl' : 'ltr';
  langToggle.textContent = t.langToggle;

  document.querySelectorAll('[data-i18n]').forEach((el) => {
    const key = el.getAttribute('data-i18n');
    if (t[key]) el.textContent = t[key];
  });
  document.querySelectorAll('[data-i18n-placeholder]').forEach((el) => {
    const key = el.getAttribute('data-i18n-placeholder');
    if (t[key]) el.placeholder = t[key];
  });
  document.querySelectorAll('[data-i18n-title]').forEach((el) => {
    const key = el.getAttribute('data-i18n-title');
    if (t[key]) {
      el.title = t[key];
      el.setAttribute('aria-label', t[key]);
    }
  });

  updateThemeUI(getThemeMode(), getResolvedTheme());
  if (pendingUpdate) renderUpdateBanner();
  renderBypassDomains(bypassDomains);
  renderConfigStatus();
  updateTrojanUI();
  resetDeleteButton();

  if (lastStatus) {
    applyStatusLabels(lastStatus);
  } else {
    toggleTitle.textContent = t.proxyDisabled;
    domainCount.textContent = '';
    statDomains.textContent = '\u2014';
  }
  setConn(connState, t[CONN_KEY[connState] || 'proxyOff']);
  updateServerStat();
  refreshHomeCard();
  window.dispatchEvent(new CustomEvent('app:language', { detail: currentLang }));
}

langToggle.addEventListener('click', () => {
  const next = currentLang === 'fa' ? 'en' : 'fa';
  storeLang(next);
  setLanguage(next);
});

/* ------------------------------------------------------------------ */
/* Theme                                                               */
/* ------------------------------------------------------------------ */

function updateThemeUI(mode, resolved) {
  const t = translations[currentLang];
  themeToggle.title = t.themeToggle;
  themeToggle.setAttribute('aria-label', t.themeToggle);

  themeSegmented.querySelectorAll('[data-theme-mode]').forEach((button) => {
    const active = button.dataset.themeMode === mode;
    button.classList.toggle('active', active);
    button.setAttribute('aria-pressed', String(active));
  });

  // Keep the moon/sun glyph in sync for the brief moment before CSS applies.
  themeToggle.dataset.resolved = resolved;
}

themeToggle.addEventListener('click', toggleTheme);

themeSegmented.addEventListener('click', (event) => {
  const button = event.target.closest('[data-theme-mode]');
  if (button) setThemeMode(button.dataset.themeMode);
});

/* ------------------------------------------------------------------ */
/* Tabs & navigation                                                   */
/* ------------------------------------------------------------------ */

function switchTab(name) {
  currentTab = VALID_TABS.includes(name) ? name : 'home';

  document.querySelectorAll('.view-panel').forEach((panel) => {
    panel.classList.toggle('hidden', panel.dataset.view !== currentTab);
  });

  document.querySelectorAll('.tab-btn').forEach((button) => {
    const active = button.dataset.tab === currentTab;
    button.classList.toggle('active', active);
    button.setAttribute('aria-selected', String(active));
    button.tabIndex = active ? 0 : -1;
  });

  appContent.scrollTop = 0;

  if (currentTab === 'settings' && window.settingsModule) {
    window.settingsModule.loadPresets().catch((error) => {
      showError(String(error) || translations[currentLang].settingsLoadFailed);
    });
  }

  if (currentTab === 'haiogate' && window.portalModule) {
    window.portalModule.onTabEnter().catch(() => {
      // The HaioGate panel renders its own retry state.
    });
  }
}

// The portal module navigates to the HaioGate tab from the header chip, the
// onboarding card, and the buy banner.
window.switchAppTab = switchTab;

tabBar.addEventListener('click', (event) => {
  const button = event.target.closest('[data-tab]');
  if (button) switchTab(button.dataset.tab);
});

tabBar.addEventListener('keydown', (event) => {
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
  const tabs = [...tabBar.querySelectorAll('.tab-btn')];
  const index = tabs.indexOf(document.activeElement);
  if (index === -1) return;

  const delta = event.key === 'ArrowRight' ? 1 : -1;
  const next = tabs[(index + delta + tabs.length) % tabs.length];
  event.preventDefault();
  next.focus();
  switchTab(next.dataset.tab);
});

document.querySelectorAll('[data-goto]').forEach((button) => {
  button.addEventListener('click', () => switchTab(button.dataset.goto));
});

function showDashboard() {
  configOnlyCard.classList.add('hidden');
  dashboard.classList.remove('hidden');
  tabBar.classList.remove('hidden');
  switchTab(currentTab);
}

function showOnboarding() {
  dashboard.classList.add('hidden');
  tabBar.classList.add('hidden');
  configOnlyCard.classList.remove('hidden');
  currentTab = 'home';
  showError(null);
}

// The HaioGate tab must be reachable before a config exists (new user buys a
// package first, then applies it). This reveals the app shell without
// pretending a config is saved.
function enterPortalTab() {
  configOnlyCard.classList.add('hidden');
  dashboard.classList.remove('hidden');
  tabBar.classList.remove('hidden');
  switchTab('haiogate');
}

window.appEnterPortal = enterPortalTab;

/* ------------------------------------------------------------------ */
/* Shared UI helpers                                                   */
/* ------------------------------------------------------------------ */

function setLoading(on, msg) {
  const t = translations[currentLang];
  overlayMsg.textContent = msg || (on ? t.enabling : '');
  overlay.classList.toggle('visible', on);
  toggle.disabled = on;
  portInput.disabled = on;
}

function showError(msg) {
  if (!msg) {
    errorMsg.classList.add('hidden');
    errorMsg.classList.remove('ok');
    clearTimeout(toastTimer);
    return;
  }

  errorMsg.classList.remove('ok');
  errorText.textContent = msg;
  errorMsg.classList.remove('hidden');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => errorMsg.classList.add('hidden'), 8000);
}

// Success / neutral notices share the toast, with a success accent.
function showNotice(msg) {
  if (!msg) return;
  errorText.textContent = msg;
  errorMsg.classList.add('ok');
  errorMsg.classList.remove('hidden');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    errorMsg.classList.add('hidden');
    errorMsg.classList.remove('ok');
  }, 6000);
}

errorClose.addEventListener('click', () => showError(null));

// Let feature modules (settings, portal) surface errors in the same toast and
// reuse the global overlay.
window.showAppError = showError;
window.showAppNotice = showNotice;
window.appLoading = setLoading;

function setConn(state, text) {
  connState = state;
  connDot.className = `conn-dot ${state}`;
  connText.textContent = text;
  connStatus.className = `conn-status ${state}`;
  statusOrb.dataset.state = state;
}

function formatTimestamp(ts) {
  if (!ts) return '';
  return new Date(ts * 1000).toLocaleString();
}

function setConfigStatus(msg, ok) {
  configStatus.textContent = msg;
  configStatus.className = `config-status ${ok === true ? 'ok' : ok === false ? 'bad' : ''}`;
}

function renderConfigStatus() {
  if (!parsedConfig) {
    setConfigStatus('', null);
    return;
  }
  setConfigStatus(`\u2713 ${parsedConfig.server}:${parsedConfig.port}`, true);
}

function updateServerStat() {
  statServer.textContent = parsedConfig
    ? `${parsedConfig.server}:${parsedConfig.port}`
    : '\u2014';
}

/* ------------------------------------------------------------------ */
/* Home HaioGate summary & config source                              */
/* ------------------------------------------------------------------ */

function refreshHomeCard() {
  if (window.portalModule?.homeCard) {
    window.portalModule.homeCard(configSource, haiogateAccountId);
  }
}

async function appConfigChanged(detail = {}) {
  configSource = detail.source || 'manual';
  haiogateAccountId = detail.accountId || null;
  await loadConfig();
  await loadStatus({ skipHealth: true });
  refreshHomeCard();
}

window.appConfigChanged = appConfigChanged;
window.appRefreshHomeCard = refreshHomeCard;
window.appHasConfig = false;

function domainCountText(data) {
  const t = translations[currentLang];
  if (!data || !data.domainCount) return '';
  let suffix = t.domainsProxied;  if (data.usingFallback) suffix = t.domainsFallback;
  else if (data.usingCache) suffix = t.domainsCached;
  return `${Number(data.domainCount).toLocaleString()} ${suffix}`;
}

function applyStatusLabels(data, skipHealth = false) {
  const t = translations[currentLang];
  const enabled = Boolean(data.enabled);
  needsProxyConsent = enabled && data.proxyConsent === false;
  toggle.checked = enabled;
  statusBadge.textContent = enabled ? 'ON' : 'OFF';
  statusBadge.classList.toggle('on', enabled);
  toggleTitle.textContent = needsProxyConsent
    ? t.consentNeeded
    : enabled
      ? t.proxyActive
      : t.proxyDisabled;
  domainCount.textContent = domainCountText(data);
  statDomains.textContent = data.domainCount
    ? Number(data.domainCount).toLocaleString()
    : '0';
  statBypass.textContent = String(bypassDomains.length);

  updateDomainMeta(data);

  if (data.httpProxyPort) portInput.value = data.httpProxyPort;

  if (data.version) {
    appVersion.textContent = `v${data.version}`;
    appVersion.title = data.lastUpdateAt
      ? `${t.lastFetch}: ${formatTimestamp(data.lastUpdateAt)}`
      : t.updateUpToDate;
  }

  if (typeof data.configSource === 'string') configSource = data.configSource;
  haiogateAccountId = data.haiogateAccountId || null;
  refreshHomeCard();

  if (needsProxyConsent) {
    // The tunnel may be running, but without OS-proxy consent browser
    // traffic never reaches it. Do not report a false "Connected" state.
    setConn('failed', t.consentNeeded);
    consentModal?.classList.remove('hidden');
  } else if (enabled) {
    if (!skipHealth) runHealthCheck();
  } else {
    setConn('off', t.proxyOff);
  }
}

function updateDomainMeta(data) {
  const t = translations[currentLang];
  domainMeta.innerHTML = '';

  if (data.domainSourceUrl) {
    domainSourceUrl = data.domainSourceUrl;
    const link = document.createElement('a');
    link.className = 'domain-link';
    link.href = '#';
    link.textContent = t.domainsLink;
    link.addEventListener('click', (event) => {
      event.preventDefault();
      if (window.__TAURI__?.shell?.open) {
        window.__TAURI__.shell.open(domainSourceUrl);
      } else {
        window.open(domainSourceUrl, '_blank', 'noopener');
      }
    });
    domainMeta.appendChild(link);
  }

  if (data.lastFetch) {
    const time = document.createElement('span');
    time.className = 'domain-fetch-time';
    time.textContent = `${t.lastFetch}: ${formatTimestamp(data.lastFetch)}`;
    domainMeta.appendChild(time);
  }
}

async function runHealthCheck() {
  if (!toggle.checked) return;
  const t = translations[currentLang];
  setConn('checking', t.checking);

  try {
    const res = await invoke('test_connection');
    if (res && res.httpProxyOk === false) {
      setConn('failed', t.proxyDown);
    } else if (res && res.tunnelOk === false) {
      setConn('failed', t.failed);
    } else if (res && res.ok) {
      setConn('connected', t.connected);
    } else {
      setConn('failed', t.failed);
    }
  } catch (_) {
    setConn('failed', t.failed);
  }
}

/* ------------------------------------------------------------------ */
/* Access key                                                          */
/* ------------------------------------------------------------------ */

function parseAccessKey(uri) {
  const match = (uri || '').trim().match(
    /^(?:trojan|haio):\/\/([^@]+)@([^:/?#]+):(\d+)(?:\?([^#]*))?/i
  );
  if (!match) return null;

  let password;
  try {
    password = decodeURIComponent(match[1]);
  } catch (_) {
    return null;
  }

  const server = match[2];
  const port = Number.parseInt(match[3], 10);
  const query = new URLSearchParams(match[4] || '');
  const sni = query.get('sni') || server;

  if (!password || !server || !port || port < 1 || port > 65535) return null;
  return { password, server, port, sni, raw: uri.trim() };
}

async function loadConfig() {
  try {
    const state = await invoke('get_state');
    if (state.trojanUrl) {
      configInput.value = state.trojanUrl;
      if (typeof state.configSource === 'string') configSource = state.configSource;
      haiogateAccountId = state.haiogateAccountId || null;
      const parsed = parseAccessKey(state.trojanUrl);
      if (parsed) {
        parsedConfig = parsed;
        window.appHasConfig = true;
        renderConfigStatus();
        updateServerStat();
        btnSetup.disabled = false;
        showDashboard();
        window.portalModule?.refreshSession?.();
        refreshHomeCard();
        return parsed;
      }
      window.appHasConfig = false;
      parsedConfig = null;
      setConfigStatus(translations[currentLang].invalidConfig, false);
    } else {
      window.appHasConfig = false;
      configInput.value = '';
      parsedConfig = null;
      setConfigStatus('', null);
    }
  } catch (error) {
    parsedConfig = null;
    window.appHasConfig = false;
    setConfigStatus(String(error) || translations[currentLang].invalidConfig, false);
  }

  btnSetup.disabled = true;
  updateServerStat();
  window.portalModule?.refreshSession?.();
  refreshHomeCard();
  showOnboarding();
  return null;
}

saveConfigBtn.addEventListener('click', async () => {
  const t = translations[currentLang];
  const parsed = parseAccessKey(configInput.value);

  if (!parsed) {
    configInput.classList.add('bad');
    setConfigStatus(t.invalidConfig, false);
    showError(t.invalidConfig);
    return;
  }

  configInput.classList.remove('bad');
  saveConfigBtn.disabled = true;

  try {
    const res = await invoke('save_config', { trojanUrl: parsed.raw });
    if (!res || !res.success) {
      setConfigStatus(t.saveFailed, false);
      showError(t.saveFailed);
      return;
    }

    parsedConfig = parsed;
    window.appHasConfig = true;
    configSource = 'manual';
    haiogateAccountId = null;
    renderConfigStatus();
    updateServerStat();
    btnSetup.disabled = false;
    saveConfigBtn.textContent = `\u2713 ${t.saved}`;
    setTimeout(() => {
      saveConfigBtn.textContent = translations[currentLang].saveConfig;
    }, 1800);

    showDashboard();
    window.portalModule?.refreshSession?.();
    await loadStatus();
  } catch (error) {
    setConfigStatus(String(error) || t.saveFailed, false);
    showError(String(error) || t.saveFailed);
  } finally {
    saveConfigBtn.disabled = false;
  }
});

/* ------------------------------------------------------------------ */
/* Copy connection link (guest + manual + HaioGate)                   */
/* ------------------------------------------------------------------ */

function resolveCopyWarning(allowed) {
  shareWarnModal?.classList.add('hidden');
  if (copyWarnResolve) {
    const resolve = copyWarnResolve;
    copyWarnResolve = null;
    resolve(allowed);
  }
}

function askShareWarning() {
  return new Promise((resolve) => {
    if (!shareWarnModal) {
      resolve(true);
      return;
    }
    copyWarnResolve = resolve;
    shareWarnModal.classList.remove('hidden');
  });
}

async function currentTrojanUrl() {
  if (configInput.value && parseAccessKey(configInput.value)) {
    return configInput.value.trim();
  }
  try {
    const state = await invoke('get_state');
    return typeof state.trojanUrl === 'string' ? state.trojanUrl : '';
  } catch (_) {
    return '';
  }
}

btnCopyConfig?.addEventListener('click', async () => {
  const link = await currentTrojanUrl();
  if (!link) {
    showError(translations[currentLang].hgNoConfig);
    return;
  }
  // The link carries the access key: warn before putting it on the clipboard.
  const allowed = await askShareWarning();
  if (!allowed) return;
  const copied = await copyToClipboard(link, 'hgLinkCopied');
  if (!copied) showError(translations[currentLang].hgCopyFailed);
});

shareWarnCancel?.addEventListener('click', () => resolveCopyWarning(false));
shareWarnCopy?.addEventListener('click', () => resolveCopyWarning(true));

// Delete config — two-click confirm, auto-disconnects if proxy is active.
function resetDeleteButton() {
  clearTimeout(deleteTimer);
  deleteTimer = null;
  btnDeleteConfig.classList.remove('confirming');
  btnDeleteConfig.disabled = false;
  btnDeleteConfig.textContent = translations[currentLang].deleteConfig;
}

btnDeleteConfig.addEventListener('click', async () => {
  const t = translations[currentLang];

  if (!deleteTimer) {
    btnDeleteConfig.classList.add('confirming');
    btnDeleteConfig.textContent = t.deleteConfirm;
    deleteTimer = setTimeout(resetDeleteButton, 3000);
    return;
  }

  resetDeleteButton();
  btnDeleteConfig.disabled = true;

  try {
    const res = await invoke('delete_config');
    if (res && res.success) {
      configInput.value = '';
      parsedConfig = null;
      window.appHasConfig = false;
      configSource = 'manual';
      haiogateAccountId = null;
      lastStatus = null;
      toggle.checked = false;
      statusBadge.textContent = 'OFF';
      statusBadge.classList.remove('on');
      toggleTitle.textContent = t.proxyDisabled;
      domainCount.textContent = '';
      domainMeta.innerHTML = '';
      statDomains.textContent = '\u2014';
      statServer.textContent = '\u2014';
      setConn('off', t.proxyOff);
      btnSetup.disabled = true;
      trojanRunning = false;
      updateTrojanUI();
      setConfigStatus('', null);
      showOnboarding();
      window.portalModule?.refreshSession?.();
    } else {
      showError(res?.error || t.deleteFailed);
    }
  } catch (error) {
    showError(String(error) || t.deleteFailed);
  } finally {
    btnDeleteConfig.disabled = false;
  }
});

/* ------------------------------------------------------------------ */
/* Status & connection                                                 */
/* ------------------------------------------------------------------ */

async function loadStatus(options = {}) {
  let data;
  try {
    data = await invoke('get_status');
  } catch (error) {
    showError(String(error) || translations[currentLang].statusFailed);
    return;
  }

  lastStatus = data;
  applyStatusLabels(data, Boolean(options.skipHealth));

  if (data.lastFetchError && (data.usingFallback || data.usingCache)) {
    if (data.lastFetchError !== lastDomainError) {
      lastDomainError = data.lastFetchError;
      showError(
        translations[currentLang].domainRefreshFailed.replace('{e}', data.lastFetchError)
      );
    }
  } else {
    lastDomainError = '';
  }
}

toggle.addEventListener('change', async () => {
  const t = translations[currentLang];
  showError(null);
  setLoading(true, toggle.checked ? t.enabling : t.disabling);

  const port = Number.parseInt(portInput.value, 10) || 11032;
  let ok = false;

  try {
    await invoke('set_port', { httpPort: port });
    const res = toggle.checked
      ? await invoke('enable_proxy')
      : await invoke('disable_proxy');

    ok = Boolean(res && res.success);
    if (!ok) showError(res?.error || t.unknownError);
  } catch (error) {
    showError(String(error) || t.unknownError);
  } finally {
    setLoading(false);
  }

  await loadStatus();
});

portInput.addEventListener('change', async () => {
  const t = translations[currentLang];
  const port = Number.parseInt(portInput.value, 10);

  if (!port || port < 1024 || port > 65535) {
    showError(t.invalidPort);
    portInput.value = lastStatus?.httpProxyPort || 11032;
    return;
  }

  try {
    await invoke('set_port', { httpPort: port });
    const state = await invoke('get_state');
    if (state.enabled) {
      setLoading(true, t.reloading);
      await invoke('enable_proxy');
      setLoading(false);
      await loadStatus();
    }
  } catch (error) {
    setLoading(false);
    showError(String(error) || t.unknownError);
  }
});

function updateTrojanUI() {
  const t = translations[currentLang];
  if (trojanRunning) {
    btnSetup.textContent = t.running;
    btnSetup.disabled = true;
    trojanStatus.classList.remove('hidden');
    trojanStatusText.textContent = t.engineRunning;
  } else {
    btnSetup.textContent = t.installStart;
    btnSetup.disabled = !parsedConfig;
    trojanStatus.classList.add('hidden');
  }
}

btnSetup.addEventListener('click', async () => {
  const t = translations[currentLang];
  setLoading(true, t.connecting);

  try {
    const res = await invoke('connect_trojan');
    if (res && res.success) {
      trojanRunning = true;
    } else {
      showError(res?.error || t.connectFailed);
    }
  } catch (error) {
    showError(String(error) || t.connectFailed);
  } finally {
    setLoading(false);
    updateTrojanUI();
  }
});

/* ------------------------------------------------------------------ */
/* Bypass list                                                         */
/* ------------------------------------------------------------------ */

function missingPresetDomains(preset) {
  const current = new Set(bypassDomains);
  return preset.filter((domain) => !current.has(domain));
}

function updateGooglePreset() {
  if (!googlePreset || !googlePresetAction) return;

  const t = translations[currentLang];
  const missing = missingPresetDomains(GOOGLE_PRESET_DOMAINS);
  const allAdded = missing.length === 0;

  googlePreset.disabled = allAdded;
  googlePresetAction.textContent = allAdded
    ? t.bypassPresetAdded
    : t.bypassPresetAdd.replace('{n}', String(missing.length));
}

// Any bypass entry that covers (or is covered by) an AI host sends Gemini's
// assets and its eligibility check straight out from the customer's own IP.
function directAiHosts() {
  return bypassDomains.filter((entry) =>
    GOOGLE_AI_TUNNEL_ONLY.some(
      (host) =>
        entry === host || entry.endsWith(`.${host}`) || host.endsWith(`.${entry}`)
    )
  );
}

function renderAiDirectWarn() {
  if (!aiDirectWarn) return;
  aiDirectWarn.classList.toggle('hidden', directAiHosts().length === 0);
}

function bypassEmptyState(message, hint) {
  const empty = document.createElement('div');
  empty.className = 'bypass-empty';

  const icon = document.createElement('span');
  icon.innerHTML =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l7 3v5c0 4.6-3 8.4-7 10-4-1.6-7-5.4-7-10V6l7-3z"/><path d="M8.5 12h7"/></svg>';
  empty.appendChild(icon);

  const text = document.createElement('span');
  text.textContent = message;
  empty.appendChild(text);

  if (hint) {
    const small = document.createElement('small');
    small.textContent = hint;
    empty.appendChild(small);
  }

  return empty;
}

function bypassRow(domain) {
  const t = translations[currentLang];
  const item = document.createElement('div');
  item.className = 'bypass-item';

  const globe = document.createElement('span');
  globe.className = 'globe';
  globe.innerHTML =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3a15 15 0 0 1 0 18a15 15 0 0 1 0-18z"/></svg>';
  item.appendChild(globe);

  const label = document.createElement('span');
  label.className = 'bypass-domain';
  label.textContent = domain;
  label.title = domain;
  item.appendChild(label);

  const remove = document.createElement('button');
  remove.type = 'button';
  remove.className = 'bypass-remove';
  remove.innerHTML =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg>';
  remove.title = t.bypassRemove;
  remove.setAttribute('aria-label', `${t.bypassRemove}: ${domain}`);
  remove.addEventListener('click', () => removeBypassDomain(domain, remove));
  item.appendChild(remove);

  return item;
}

function renderBypassDomains(domains) {
  const t = translations[currentLang];
  bypassDomains = Array.isArray(domains) ? domains : [];

  bypassCount.textContent = String(bypassDomains.length);
  statBypass.textContent = String(bypassDomains.length);
  quickBypassCount.textContent = bypassDomains.length
    ? t.bypassCountLabel.replace('{n}', String(bypassDomains.length))
    : t.bypassNone;

  updateGooglePreset();
  renderAiDirectWarn();

  const query = (bypassSearch?.value || '').trim().toLowerCase();
  const filtered = query
    ? bypassDomains.filter((domain) => domain.toLowerCase().includes(query))
    : bypassDomains;

  bypassList.innerHTML = '';

  if (bypassDomains.length === 0) {
    bypassList.appendChild(bypassEmptyState(t.bypassEmpty, t.bypassEmptyHint));
    return;
  }

  if (filtered.length === 0) {
    bypassList.appendChild(bypassEmptyState(t.bypassNoResults, t.bypassNoResultsHint));
    return;
  }

  const fragment = document.createDocumentFragment();
  filtered.forEach((domain) => fragment.appendChild(bypassRow(domain)));
  bypassList.appendChild(fragment);
}

async function loadBypassDomains() {
  try {
    const res = await invoke('get_bypass_domains');
    renderBypassDomains(res.domains || []);
  } catch (_) {
    // Non-fatal: the bypass page simply stays empty.
  }
}

async function addBypassDomain() {
  const t = translations[currentLang];
  const value = bypassInput.value.trim();
  if (!value) {
    bypassInput.focus();
    return;
  }

  btnAddBypass.disabled = true;
  try {
    const res = await invoke('add_bypass_domain', { domain: value });
    bypassInput.value = '';
    bypassInput.classList.remove('bad');
    showError(null);
    if (Array.isArray(res?.domains)) {
      renderBypassDomains(res.domains);
    } else {
      await loadBypassDomains();
    }
  } catch (_) {
    bypassInput.classList.add('bad');
    showError(t.bypassInvalid);
    bypassInput.focus();
  } finally {
    btnAddBypass.disabled = false;
  }
}

async function removeBypassDomain(domain, button) {
  const t = translations[currentLang];
  if (button) button.disabled = true;
  try {
    const res = await invoke('remove_bypass_domain', { domain });
    showError(null);
    if (Array.isArray(res?.domains)) {
      renderBypassDomains(res.domains);
    } else {
      await loadBypassDomains();
    }
  } catch (error) {
    showError(String(error) || t.bypassRemoveFailed);
    if (button) button.disabled = false;
  }
}

bypassForm.addEventListener('submit', (event) => {
  event.preventDefault();
  addBypassDomain();
});

bypassSearch.addEventListener('input', () => {
  renderBypassDomains(bypassDomains);
});

googlePreset.addEventListener('click', async () => {
  const t = translations[currentLang];
  const missing = missingPresetDomains(GOOGLE_PRESET_DOMAINS);
  if (missing.length === 0) return;

  googlePreset.disabled = true;
  try {
    const merged = [...new Set([...bypassDomains, ...missing])].sort();
    const res = await invoke('set_bypass_domains', { domains: merged });
    showError(null);
    if (Array.isArray(res?.domains)) {
      renderBypassDomains(res.domains);
    } else {
      await loadBypassDomains();
    }
  } catch (error) {
    showError(String(error) || t.bypassPresetFailed);
  } finally {
    updateGooglePreset();
  }
});

aiDirectFix?.addEventListener('click', async () => {
  const t = translations[currentLang];
  const drop = new Set(directAiHosts());
  if (drop.size === 0) return;

  aiDirectFix.disabled = true;
  try {
    const kept = bypassDomains.filter((domain) => !drop.has(domain));
    const res = await invoke('set_bypass_domains', { domains: kept });
    showError(null);
    if (Array.isArray(res?.domains)) {
      renderBypassDomains(res.domains);
    } else {
      await loadBypassDomains();
    }
  } catch (error) {
    showError(String(error) || t.bypassRemoveFailed);
  } finally {
    aiDirectFix.disabled = false;
    renderAiDirectWarn();
  }
});

/* ------------------------------------------------------------------ */
/* Domain refresh                                                      */
/* ------------------------------------------------------------------ */
btnRefreshDomains.addEventListener('click', async () => {
  const t = translations[currentLang];
  const now = Date.now();

  if (now - lastRefreshTime < 10000) {
    const remaining = Math.ceil((10000 - (now - lastRefreshTime)) / 1000);
    showError(t.refreshWait.replace('{s}', String(remaining)));
    return;
  }

  lastRefreshTime = now;
  btnRefreshDomains.disabled = true;
  btnRefreshDomains.classList.add('refreshing');

  const tick = () => {
    const remaining = Math.ceil((10000 - (Date.now() - lastRefreshTime)) / 1000);
    btnRefreshDomains.textContent = remaining > 0 ? `${remaining}s` : t.refreshDomains;
  };
  refreshTimer = setInterval(tick, 500);

  try {
    await invoke('refresh_domains');
    await loadStatus();
  } catch (error) {
    showError(String(error) || t.refreshFailed);
  } finally {
    clearInterval(refreshTimer);
    refreshTimer = null;
    btnRefreshDomains.textContent = t.refreshDomains;
    btnRefreshDomains.disabled = false;
    btnRefreshDomains.classList.remove('refreshing');
  }
});

/* ------------------------------------------------------------------ */
/* Updates                                                             */
/* ------------------------------------------------------------------ */

function renderUpdateBanner() {
  const t = translations[currentLang];
  if (!pendingUpdate) {
    updateBanner.classList.add('hidden');
    return;
  }
  updateText.textContent = t.updateAvailable.replace('{v}', `v${pendingUpdate.version}`);
  btnInstallUpdate.textContent = t.installUpdate;
  updateBanner.classList.remove('hidden');
}

btnInstallUpdate.addEventListener('click', async () => {
  const t = translations[currentLang];
  btnInstallUpdate.disabled = true;
  setLoading(true, t.updating);

  try {
    await invoke('install_update');
    setLoading(false);
    showError(t.updateRestart);
  } catch (error) {
    setLoading(false);
    showError(String(error) || t.updateFailed);
    btnInstallUpdate.disabled = false;
  }
});

async function checkForUpdates() {
  try {
    const res = await invoke('check_for_updates');
    if (res && res.available) {
      pendingUpdate = { version: res.version, notes: res.notes };
      renderUpdateBanner();
    }
  } catch (_) {
    // Non-fatal — updater endpoint may be unreachable.
  }
}

/* ------------------------------------------------------------------ */
/* Backend events                                                      */
/* ------------------------------------------------------------------ */

listen('status:update', () => {
  loadStatus();
});

listen('update:available', (data) => {
  pendingUpdate = { version: data.version, notes: data.notes };
  renderUpdateBanner();
});

listen('trojan:status', (data) => {
  trojanRunning = Boolean(data.running);
  updateTrojanUI();
});

listen('health:check', (data) => {
  if (!toggle.checked || needsProxyConsent) return;
  const t = translations[currentLang];
  if (data.ok) {
    setConn('connected', t.connected);
  } else if (data.httpProxyOk === false) {
    setConn('failed', t.proxyDown);
  } else if (data.tunnelOk === false) {
    setConn('failed', t.failed);
  } else {
    setConn('checking', t.checking);
  }
});

listen('domains:updated', () => {
  if (toggle.checked) loadStatus({ skipHealth: true });
});

listen('health:failed', () => {
  if (!toggle.checked || needsProxyConsent) return;
  const t = translations[currentLang];
  setConn('failed', t.failed);
  showError(t.healthFailed);
});

/* ------------------------------------------------------------------ */
/* Init                                                                */
/* ------------------------------------------------------------------ */

async function init() {
  initTheme(updateThemeUI);
  setLanguage(readStoredLang());
  updateThemeUI(getThemeMode(), getResolvedTheme());
  setConn('off', translations[currentLang].proxyOff);

  // Portal initialisation is additive: if it fails, the VPN UI keeps working.
  await initPortal().catch(() => {});

  const config = await loadConfig();
  if (config) {
    await Promise.all([loadStatus(), loadBypassDomains()]);
  } else {
    await loadBypassDomains();
  }
  refreshHomeCard();
  checkForUpdates();
}

init();
