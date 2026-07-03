// Thin wrapper around Tauri's invoke() function
// All calls go through #[tauri::command] in commands.rs

export async function invoke(command, args = {}) {
  return window.__TAURI__.core.invoke(command, args);
}

// Event listener helper
export function listen(event, callback) {
  return window.__TAURI__.event.listen(event, (e) => callback(e.payload));
}
