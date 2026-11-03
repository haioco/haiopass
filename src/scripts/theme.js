// Theme handling: system / light / dark, persisted locally.
// Kept small and dependency-free so the UI can paint the right colours before
// any async Tauri work happens.

const STORAGE_KEY = 'haiobypass:theme';
const VALID = new Set(['system', 'light', 'dark']);
const media = window.matchMedia('(prefers-color-scheme: dark)');

let mode = 'system';
let onApply = null;

function readStoredMode() {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return VALID.has(stored) ? stored : 'system';
  } catch (_) {
    return 'system';
  }
}

function storeMode(value) {
  try {
    localStorage.setItem(STORAGE_KEY, value);
  } catch (_) {
    // Storage can be unavailable in hardened webviews; the theme still works
    // for this session.
  }
}

function resolvedTheme() {
  if (mode === 'system') {
    return media.matches ? 'dark' : 'light';
  }
  return mode;
}

function apply() {
  const resolved = resolvedTheme();
  document.documentElement.dataset.theme = resolved;
  document.documentElement.dataset.themeMode = mode;
  document.documentElement.style.colorScheme = resolved;
  if (onApply) onApply(mode, resolved);
}

const mediaListener = () => {
  if (mode === 'system') apply();
};

export function initTheme(callback) {
  onApply = callback;
  mode = readStoredMode();
  apply();

  if (typeof media.addEventListener === 'function') {
    media.addEventListener('change', mediaListener);
  } else if (typeof media.addListener === 'function') {
    media.addListener(mediaListener);
  }
}

export function getThemeMode() {
  return mode;
}

export function getResolvedTheme() {
  return resolvedTheme();
}

export function setThemeMode(next) {
  if (!VALID.has(next)) return;
  mode = next;
  storeMode(mode);
  apply();
}

/** Quick header toggle: always flips the visible theme explicitly. */
export function toggleTheme() {
  setThemeMode(resolvedTheme() === 'dark' ? 'light' : 'dark');
}
