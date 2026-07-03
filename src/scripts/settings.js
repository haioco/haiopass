import { invoke } from './invoke.js';

const presetsList = document.getElementById('presetsList');
const autostartToggle = document.getElementById('autostartToggle');

export const PRESET_LABELS = {
  gradle: 'Gradle (Android Studio)',
  maven: 'Maven',
  pip: 'pip (Python)',
  docker: 'Docker',
  curl: 'curl',
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
  }
};

autostartToggle.addEventListener('change', async () => {
  await invoke('set_autostart', { enabled: autostartToggle.checked });
});
