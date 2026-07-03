import { invoke, listen } from './invoke.js';
import { translations } from './i18n.js';
import './settings.js';

const configInput   = document.getElementById('configInput');
const configStatus  = document.getElementById('configStatus');
const saveConfigBtn = document.getElementById('saveConfigBtn');
const toggle        = document.getElementById('proxyToggle');
const toggleTitle   = document.getElementById('toggleTitle');
const domainCount   = document.getElementById('domainCount');
const domainMeta    = document.getElementById('domainMeta');
const statusBadge   = document.getElementById('statusBadge');
const portInput     = document.getElementById('portInput');
const errorMsg      = document.getElementById('errorMsg');
const overlay       = document.getElementById('overlay');
const overlayMsg    = document.getElementById('overlayMsg');
const connStatus    = document.getElementById('connStatus');
const connDot       = document.getElementById('connDot');
const connText      = document.getElementById('connText');
const langToggle    = document.getElementById('langToggle');
const configOnlyCard = document.getElementById('configOnlyCard');
const mainControls   = document.getElementById('mainControls');
const btnSetup       = document.getElementById('btnSetup');
const trojanStatus   = document.getElementById('trojanStatus');
const trojanStatusText = document.getElementById('trojanStatusText');

let currentLang = 'en';
let lastRefreshTime = 0;
let domainSourceUrl = '';

function setLanguage(lang) {
  currentLang = lang;
  const t = translations[lang];
  const html = document.documentElement;
  html.lang = lang;
  html.dir = lang === 'fa' ? 'rtl' : 'ltr';
  langToggle.textContent = t.langToggle;

  document.querySelectorAll('[data-i18n]').forEach(el => {
    const key = el.getAttribute('data-i18n');
    if (t[key]) el.textContent = t[key];
  });
  document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
    const key = el.getAttribute('data-i18n-placeholder');
    if (t[key]) el.placeholder = t[key];
  });

  if (toggle.checked) {
    toggleTitle.textContent = t.proxyActive;
  } else {
    toggleTitle.textContent = t.proxyDisabled;
  }
}

langToggle.addEventListener('click', () => {
  setLanguage(currentLang === 'fa' ? 'en' : 'fa');
});

function setLoading(on, msg) {
  const t = translations[currentLang];
  overlayMsg.textContent = msg || (on ? t.enabling : '');
  overlay.classList.toggle('visible', on);
  toggle.disabled = on;
  portInput.disabled = on;
}

function showError(msg) {
  if (msg) {
    errorMsg.textContent = msg;
    errorMsg.classList.remove('hidden');
  } else {
    errorMsg.classList.add('hidden');
  }
}

function setConn(state, text) {
  connDot.className = 'conn-dot ' + state;
  connText.textContent = text;
  connStatus.className = 'conn-status ' + state;
}

function formatTimestamp(ts) {
  if (!ts) return '';
  const d = new Date(ts * 1000);
  return d.toLocaleString();
}

async function runHealthCheck() {
  const t = translations[currentLang];
  setConn('checking', t.checking);
  const res = await invoke('test_connection');
  if (res && res.ok) {
    setConn('connected', t.connected);
  } else {
    setConn('failed', t.failed);
  }
}

function parseTrojanUrl(uri) {
  const m = (uri || '').trim().match(
    /^trojan:\/\/([^@]+)@([^:/?#]+):(\d+)(?:\?([^#]*))?/i
  );
  if (!m) return null;
  const password = decodeURIComponent(m[1]);
  const server   = m[2];
  const port      = parseInt(m[3], 10);
  const query     = new URLSearchParams(m[4] || '');
  const sni       = query.get('sni') || server;
  if (!password || !server || !port) return null;
  return { password, server, port, sni, raw: uri.trim() };
}

function setConfigStatus(msg, ok) {
  configStatus.textContent = msg;
  configStatus.className = 'config-status ' + (ok === true ? 'ok' : ok === false ? 'bad' : '');
}

function updateDomainMeta(data) {
  const t = translations[currentLang];
  let html = '';

  if (data.domainSourceUrl) {
    domainSourceUrl = data.domainSourceUrl;
    html += `<span class="domain-link-wrap"><a class="domain-link" href="#">${t.domainsLink}</a></span>`;
  }

  if (data.lastFetch) {
    const timeStr = formatTimestamp(data.lastFetch);
    html += `<span class="domain-fetch-time">${t.lastFetch}: ${timeStr}</span>`;
  }

  domainMeta.innerHTML = html;

  const linkEl = domainMeta.querySelector('.domain-link');
  if (linkEl) {
    linkEl.addEventListener('click', (e) => {
      e.preventDefault();
      if (window.__TAURI__?.shell?.open) {
        window.__TAURI__.shell.open(domainSourceUrl);
      }
    });
  }
}

async function loadStatus() {
  const data = await invoke('get_status');
  const t = translations[currentLang];
  const enabled = data.enabled || false;
  toggle.checked = enabled;
  statusBadge.textContent = enabled ? 'ON' : 'OFF';
  statusBadge.classList.toggle('on', enabled);
  toggleTitle.textContent = enabled ? t.proxyActive : t.proxyDisabled;

  if (data.domainCount) {
    if (data.usingFallback) {
      domainCount.textContent = `${data.domainCount} ${t.domainsFallback}`;
    } else if (data.usingCache) {
      domainCount.textContent = `${data.domainCount} ${t.domainsCached}`;
    } else {
      domainCount.textContent = `${data.domainCount} ${t.domainsProxied}`;
    }
    updateDomainMeta(data);
  } else {
    domainCount.textContent = '';
    domainMeta.innerHTML = '';
  }

  if (data.lastFetchError && (data.usingFallback || data.usingCache)) {
    showError(`Domain list refresh failed: ${data.lastFetchError}`);
  }

  if (data.httpProxyPort) portInput.value = data.httpProxyPort;

  if (enabled) {
    runHealthCheck();
  } else {
    setConn('off', t.proxyOff);
  }
}

async function loadConfig() {
  const state = await invoke('get_state');
  if (state.trojanUrl) {
    configInput.value = state.trojanUrl;
    const parsed = parseTrojanUrl(state.trojanUrl);
    if (parsed) {
      setConfigStatus(`\u2713 ${parsed.server}:${parsed.port}`, true);
      configOnlyCard.classList.add('hidden');
      mainControls.classList.remove('hidden');
      btnSetup.disabled = false;
      return parsed;
    }
    setConfigStatus('Invalid config', false);
  } else {
    configInput.value = '';
    setConfigStatus('', null);
    configOnlyCard.classList.remove('hidden');
    mainControls.classList.add('hidden');
  }
  btnSetup.disabled = true;
  return null;
}

saveConfigBtn.addEventListener('click', async () => {
  const t = translations[currentLang];
  const parsed = parseTrojanUrl(configInput.value);
  if (!parsed) {
    setConfigStatus('\u2717 Invalid trojan:// URL', false);
    return;
  }
  const res = await invoke('save_config', { trojanUrl: parsed.raw });
  if (res.success) {
    setConfigStatus(`\u2713 Saved \u2014 ${parsed.server}:${parsed.port}`, true);
    btnSetup.disabled = false;
    configOnlyCard.classList.add('hidden');
    mainControls.classList.remove('hidden');
    saveConfigBtn.textContent = '\u2713 Saved';
    setTimeout(() => { saveConfigBtn.textContent = t.saveConfig; }, 1800);
  } else {
    setConfigStatus('\u2717 Save failed', false);
  }
});

toggle.addEventListener('change', async () => {
  const t = translations[currentLang];
  showError(null);
  setLoading(true, toggle.checked ? t.enabling : t.disabling);

  const port = parseInt(portInput.value, 10) || 11032;
  try {
    await invoke('set_port', { httpPort: port });

    const res = toggle.checked
      ? await invoke('enable_proxy')
      : await invoke('disable_proxy');

    setLoading(false);
    if (!res.success) showError(res.error || 'Unknown error.');
  } catch (e) {
    setLoading(false);
    showError(String(e) || 'Unknown error');
  }
  await loadStatus();
  if (toggle.checked) runHealthCheck();
});

portInput.addEventListener('change', async () => {
  const port = parseInt(portInput.value, 10) || 11032;
  try {
    await invoke('set_port', { httpPort: port });
    const state = await invoke('get_state');
    if (state.enabled) {
      setLoading(true, 'Reloading\u2026');
      await invoke('enable_proxy');
      setLoading(false);
      await loadStatus();
    }
  } catch (e) {
    setLoading(false);
    showError(String(e) || 'Unknown error');
  }
});

btnSetup.addEventListener('click', async () => {
  const t = translations[currentLang];
  setLoading(true, 'Installing\u2026');
  const res = await invoke('install_and_start_trojan');
  setLoading(false);

  if (res.success) {
    btnSetup.textContent = '\u2713 Running';
    btnSetup.disabled = true;
    trojanStatus.classList.remove('hidden');
    trojanStatusText.textContent = 'Trojan client is running';
  } else {
    showError(res.error || 'Install failed');
  }
});

const btnRefreshDomains = document.getElementById('btnRefreshDomains');
if (btnRefreshDomains) {
  btnRefreshDomains.addEventListener('click', async () => {
    const now = Date.now();
    if (now - lastRefreshTime < 10000) {
      const remaining = Math.ceil((10000 - (now - lastRefreshTime)) / 1000);
      btnRefreshDomains.textContent = `${remaining}s`;
      return;
    }

    lastRefreshTime = now;
    btnRefreshDomains.disabled = true;
    btnRefreshDomains.classList.add('refreshing');

    const countdownInterval = setInterval(() => {
      const elapsed = Date.now() - lastRefreshTime;
      const remaining = Math.ceil((10000 - elapsed) / 1000);
      if (remaining > 0) {
        btnRefreshDomains.textContent = `${remaining}s`;
      } else {
        clearInterval(countdownInterval);
        btnRefreshDomains.textContent = translations[currentLang].refreshDomains;
        btnRefreshDomains.disabled = false;
        btnRefreshDomains.classList.remove('refreshing');
      }
    }, 500);

    try {
      await invoke('refresh_domains');
      await loadStatus();
    } catch (e) {
      showError(String(e) || 'Refresh failed');
    }
  });
}

// Listen for events from backend
listen('status:update', (data) => {
  loadStatus();
});

listen('trojan:status', (data) => {
  if (data.running) {
    btnSetup.textContent = '\u2713 Running';
    btnSetup.disabled = true;
    trojanStatus.classList.remove('hidden');
    trojanStatusText.textContent = `Running (PID ${data.pid})`;
  } else {
    btnSetup.textContent = translations[currentLang].installStart;
    btnSetup.disabled = false;
    trojanStatus.classList.add('hidden');
  }
});

// Settings button
document.getElementById('btnSettings').addEventListener('click', () => {
  mainControls.classList.add('hidden');
  document.getElementById('settingsPanel').classList.remove('hidden');
  window.settingsModule.loadPresets();
});

document.getElementById('btnBack').addEventListener('click', () => {
  document.getElementById('settingsPanel').classList.add('hidden');
  mainControls.classList.remove('hidden');
  loadStatus();
});

// Initialize
setLanguage('en');
loadConfig();
loadStatus();
