import { invoke, listen } from './invoke.js';

const presetsList = document.getElementById('presetsList');
const autostartToggle = document.getElementById('autostartToggle');
const quicBlockToggle = document.getElementById('quicBlockToggle');

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

window.settingsModule = {
  async loadPresets() {
    const data = await invoke('get_presets');
    const state = await invoke('get_state');
    const enabled = state.enabled_presets || [];

    presetsList.innerHTML = '';

    for (const [name, label] of Object.entries(PRESET_LABELS)) {
      const isAvailable = data.available.includes(name);
      const isEnabled = enabled.includes(name);

      const item = document.createElement('div');
      item.className = 'preset-item';
      item.innerHTML = `
        <div>
          <div class="preset-name">${label}</div>
          <div class="preset-status">${isAvailable ? 'Detected' : 'Not installed'}</div>
          <div class="preset-file">Modifies: ${PRESET_FILES[name]}</div>
        </div>
        <label class="switch" style="width:40px;height:22px;">
          <input type="checkbox" ${isEnabled ? 'checked' : ''} ${!isAvailable ? 'disabled' : ''} />
          <span class="track" style="border-radius:22px;"></span>
        </label>
      `;

      const checkbox = item.querySelector('input[type="checkbox"]');
      checkbox.addEventListener('change', async () => {
        await invoke('toggle_preset', { name, on: checkbox.checked });
      });

      presetsList.appendChild(item);
    }

    autostartToggle.checked = state.autostart || false;
    if (quicBlockToggle) quicBlockToggle.checked = state.block_quic || false;
  }
};

autostartToggle.addEventListener('change', async () => {
  await invoke('set_autostart', { enabled: autostartToggle.checked });
});

if (quicBlockToggle) {
  quicBlockToggle.addEventListener('change', async () => {
    await invoke('set_quic_block', { enabled: quicBlockToggle.checked });
  });
}

// OS proxy consent: shown once before the first system proxy takeover.
const consentModal = document.getElementById('consentModal');
if (consentModal) {
  listen('consent:os-proxy', () => {
    consentModal.classList.remove('hidden');
  });

  document.getElementById('consentAllow').addEventListener('click', async () => {
    consentModal.classList.add('hidden');
    await invoke('set_proxy_consent', { consent: true });
  });

  document.getElementById('consentDeny').addEventListener('click', () => {
    consentModal.classList.add('hidden');
  });
}
