# HaioBypass-app

Cross-platform desktop app (Windows, Linux, macOS) that replaces the HaioBypass browser extension.

## What it does

Routes filtered domains (YouTube, X, Instagram, etc.) through a Haio Trojan proxy while leaving all other traffic direct — exactly like the browser extension, but at the OS level.

## Architecture

- **Tauri v2** (Rust backend + web frontend)
- **In-app local HTTP CONNECT proxy** with domain matching (replaces browser PAC script)
- **Bundled trojan-go** binary (no separate install scripts needed)
- **OS proxy configurator** (Windows registry, Linux gsettings/KDE, macOS networksetup)
- **Dev-tool proxy injector** for Gradle, Maven, npm, pip, git, Docker, Go, curl
- **System tray** with toggle, status, quit
- **Auto-updater** via GitHub Releases
- **Auto-start on boot** (Windows scheduled task, Linux .desktop, macOS LaunchAgent)

## Prerequisites

- [Rust](https://rustup.rs/) (1.70+)
- Node.js + npm
- Tauri CLI: `cargo install tauri-cli`
- Download trojan-go binaries: `bash scripts/fetch-trojan.sh`

## Development

```bash
# Install frontend deps
npm install

# Run in dev mode
cargo tauri dev
```

## Build

```bash
cargo tauri build
```

Output:
- Windows: `src-tauri/target/release/bundle/nsis/` (NSIS installer) + MSI
- Linux: `src-tauri/target/release/bundle/appimage/` (AppImage) + `.deb`
- macOS: `src-tauri/target/release/bundle/dmg/` (.app + .dmg)

## Dev Tool Presets

The app can inject proxy settings into these tools (toggle in Settings):

| Preset | File modified |
|--------|--------------|
| Gradle | `~/.gradle/gradle.properties` |
| Maven | `~/.m2/settings.xml` |
| npm | `npm config` |
| pip | `~/.config/pip/pip.conf` |
| git | `git config --global http.proxy` |
| Docker | `~/.docker/config.json` |
| Go | `~/.haiobypass/goproxy.env` |
| curl | `~/.curlrc` |

Default on: **Gradle + git** (covers Android Studio builds + git).

## Project structure

```
src-tauri/src/
├── main.rs              # Entry point
├── lib.rs               # Tauri setup + AppState
├── app/                 # Tauri commands + events
├── config/              # State storage + trojan:// parser
├── trojan/              # TrojanMgr (bundled binary, start/stop)
├── domains/             # Domain list fetcher + fallback
├── proxy/               # HTTP CONNECT proxy + PAC endpoint
├── osproxy/             # OS proxy configurator (Win/Linux/macOS)
├── appproxy/            # Dev-tool preset injector (8 presets)
├── autostart/           # Auto-start on boot (Win/Linux/macOS)
├── health.rs            # Connection health check
├── tray.rs              # System tray
├── updater.rs           # Auto-updater placeholder
└── error.rs             # Error types

src/
├── index.html           # UI (adapted from extension popup)
├── styles/style.css     # Styles (from extension popup.css)
└── scripts/
    ├── invoke.js        # Tauri invoke wrapper
    ├── i18n.js          # EN/FA translations
    ├── main.js          # Core UI logic
    └── settings.js      # Dev-tool preset toggles
```

## License

Proprietary — Haio Cloud
