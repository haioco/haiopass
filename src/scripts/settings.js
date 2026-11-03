import { invoke, listen } from './invoke.js';
import { translations } from './i18n.js';

const presetsList = document.getElementById('presetsList');
const autostartToggle = document.getElementById('autostartToggle');
const closeToTrayToggle = document.getElementById('closeToTrayToggle');
const quicBlockToggle = document.getElementById('quicBlockToggle');

const t = () => translations[document.documentElement.lang] || translations.en;

function reportError(message) {
  if (typeof window.showAppError === 'function') {
    window.showAppError(message);
  } else {
    console.error(message);
  }
}

export const PRESET_LABELS = {
  gradle: 'Gradle (Android Studio)',
  maven: 'Maven',
  pip: 'pip (Python)',
  docker: 'Docker',
  curl: 'curl',
};

// The exact file each preset rewrites while active — shown before the user
// flips a toggle, per the consent requirement.
const PRESET_FILES = {
  gradle: '~/.gradle/gradle.properties',
  maven: '~/.m2/settings.xml',
  pip: 'pip.ini (Windows) / ~/.config/pip/pip.conf',
  docker: '~/.docker/config.json',
  curl: '~/.curlrc',
};

function presetRow(name, label, available, enabled) {
  const text = t();
  const item = document.createElement('div');
  item.className = 'preset-item';

  const copy = document.createElement('div');
  copy.className = 'preset-copy';

  const title = document.createElement('div');
  title.className = 'preset-name';
  title.textContent = label;

  const status = document.createElement('div');
  status.className = `preset-status ${available ? 'available' : ''}`;
  status.textContent = available ? text.detected : text.notInstalled;

  const file = document.createElement('div');
  file.className = 'preset-file';
  file.textContent = `${text.modifies} ${PRESET_FILES[name] || name}`;
  file.title = PRESET_FILES[name] || name;

  copy.append(title, status, file);
  item.appendChild(copy);

  const checkbox = document.createElement('input');
  checkbox.type = 'checkbox';
  checkbox.className = 'native-switch';
  checkbox.checked = enabled;
  checkbox.disabled = !available;
  checkbox.setAttribute('aria-label', `${text.presetToggle}: ${label}`);
  checkbox.addEventListener('change', async () => {
    checkbox.disabled = true;
    try {
      await invoke('toggle_preset', { name, on: checkbox.checked });
    } catch (error) {
      checkbox.checked = !checkbox.checked;
      reportError(String(error) || text.settingsLoadFailed);
    } finally {
      checkbox.disabled = !available;
    }
  });

  item.appendChild(checkbox);
  return item;
}

window.settingsModule = {
  async loadPresets() {
    const [data, state] = await Promise.all([
      invoke('get_presets'),
      invoke('get_state'),
    ]);
    const enabled = data.enabled || [];
    const available = new Set(data.available || []);

    presetsList.innerHTML = '';
    const fragment = document.createDocumentFragment();
    for (const [name, label] of Object.entries(PRESET_LABELS)) {
      fragment.appendChild(
        presetRow(name, label, available.has(name), enabled.includes(name))
      );
    }
    presetsList.appendChild(fragment);

    autostartToggle.checked = Boolean(state.autostart);
    closeToTrayToggle.checked = state.minimizeToTray !== false;
    quicBlockToggle.checked = Boolean(state.blockQuic);
  },
};

async function saveToggle(checkbox, command, payload) {
  const previous = !checkbox.checked;
  checkbox.disabled = true;
  try {
    await invoke(command, payload);
  } catch (error) {
    checkbox.checked = previous;
    reportError(String(error) || t().settingsLoadFailed);
  } finally {
    checkbox.disabled = false;
  }
}

autostartToggle.addEventListener('change', () => {
  saveToggle(autostartToggle, 'set_autostart', {
    enabled: autostartToggle.checked,
  });
});

closeToTrayToggle.addEventListener('change', () => {
  saveToggle(closeToTrayToggle, 'set_minimize_to_tray', {
    enabled: closeToTrayToggle.checked,
  });
});

quicBlockToggle.addEventListener('change', () => {
  saveToggle(quicBlockToggle, 'set_quic_block', {
    enabled: quicBlockToggle.checked,
  });
});

// Refresh the preset rows when the UI language changes.
window.addEventListener('app:language', () => {
  if (presetsList.childElementCount > 0) {
    window.settingsModule.loadPresets().catch((error) => {
      reportError(String(error) || t().settingsLoadFailed);
    });
  }
});

// OS proxy consent: shown once before the first system proxy takeover.
const consentModal = document.getElementById('consentModal');
if (consentModal) {
  listen('consent:os-proxy', () => {
    consentModal.classList.remove('hidden');
  });

  document.getElementById('consentAllow').addEventListener('click', async () => {
    consentModal.classList.add('hidden');
    try {
      await invoke('set_proxy_consent', { consent: true });
    } catch (error) {
      reportError(String(error) || t().unknownError);
    }
  });

  document.getElementById('consentDeny').addEventListener('click', () => {
    consentModal.classList.add('hidden');
  });
}
