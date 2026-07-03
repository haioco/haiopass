# HaioBypass-app — Desktop App Plan

**Platforms:** Windows · Linux · macOS
**Stack:** Tauri v2 (Rust backend + reused popup web frontend), single workspace
**Routing:** in-app local HTTP CONNECT proxy + OS system proxy + optional PAC endpoint
**Trojan:** bundled trojan-go binary
**Auto-updater:** enabled in v1 (signed feed on GitHub Releases)
**Scope:** full feature parity with the browser extension + dev-tool proxy injection + macOS support

**Ports (non-famous defaults):**
- `proxy_port = 11031` — trojan-go SOCKS5 (local only)
- `http_proxy_port = 11032` — local HTTP CONNECT proxy (OS/system proxy target)

---

## 1. Overview

HaioBypass-app replaces the HaioBypass browser extension with a cross-platform desktop app.
It routes filtered domains (YouTube, X, etc.) through a Haio Trojan proxy while leaving all other
traffic direct.

The extension used the browser's PAC-script API for per-domain routing; desktop apps have no
equivalent, so the app runs an **in-process local HTTP CONNECT proxy** (Rust + tokio) that
matches domains and chains matched ones to trojan-go's SOCKS5 port (127.0.0.1:11031).

It **bundles trojan-go** and manages its lifecycle (replaces install/*.sh/*.ps1 and
native/haio_host.py). It additionally injects proxy settings into dev tools that ignore the OS
proxy (Gradle, npm, pip, git, Docker, Go, Maven, curl), so Android Studio builds work transparently.

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Tauri v2 App (Windows · Linux · macOS)                        │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  Frontend (web) — reused popup.html/css/js + settings     │  │
│  │  config • toggle • port • health • EN/FA • buy banner     │  │
│  │  + dev-tool preset toggles + tray UI                       │  │
│  └──────────────────────┬────────────────────────────────────┘  │
│                          │ #[tauri::command] + invoke()          │
│  ┌──────────────────────▼────────────────────────────────────┐  │
│  │  Rust Backend (tokio async via tauri::async_runtime)       │  │
│  │  ┌────────────┐ ┌────────────┐ ┌────────────────────────┐ │  │
│  │  │ TrojanMgr  │ │ DomainMgr   │ │ RoutingProxy          │ │  │
│  │  │ extract/   │ │ fetch +     │ │ HTTP CONNECT +       │ │  │
│  │  │ start/stop/│ │ sanitize +  │ │ plain HTTP +         │ │  │
│  │  │ restart/   │ │ fallback +  │ │ PAC + domain match → │ │  │
│  │  │ monitor/   │ │ refresh 60m │ │  SOCKS5 11031 / DIRECT│ │  │
│  │  │ autostart  │ │ cache       │ └───────────┬────────────┘ │  │
│  │  └─────┬──────┘ └────────────┘                          │  │
│  │  ┌─────▼───────────────────────────────────────────▼────┐ │  │
│  │  │ OS Proxy Configurator  +  App Proxy Injector        │ │  │
│  │  │ Win:registry · Linux:gsettings/KDE · macOS:networksetup│ │
│  │  │ Gradle/Maven/npm/pip/git/Docker/Go/curl presets      │ │  │
│  │  └─────────────────────────────────────────────────────┘ │  │
│  │  Tray (built-in v2) • Autostart • Encrypted config       │  │
│  │  Auto-updater (signed GitHub Releases feed)               │  │
│  │  Crash sentinel → restore OS proxy + presets on launch    │  │
│  └─────────────────────────────────────────────────────────┘  │
└────────────────────────┬────────────────────────────────────────-┘
                │ SOCKS5 127.0.0.1:11031
        ┌───────▼────────┐                    ┌──────────────┐
        │ trojan-go      │ ──────────────────▶ │ Haio Servers │
        │ (bundled proc) │                    └──────────────┘
        └────────────────┘
```

## 2. Mapping: extension → desktop app

| Extension component | Desktop app equivalent |
|---|---|
| Browser PAC script | In-process HTTP CONNECT proxy + `/pac.js` endpoint |
| `chrome.proxy.settings` | OS proxy configurator (Win/Linux/macOS) |
| `chrome.alarms` (60min) | `tokio::time::interval` task |
| `chrome.storage.local` | JSON config (keyring for password if needed) |
| `install/*.sh`, `install/windows.ps1` | TrojanMgr (bundled binary) |
| `native/haio_host.py` | TrojanMgr actions |
| `popup.html/css/js` | Reused; `chrome.*` → `invoke()` |
| EN/FA i18n | Reused verbatim |
| `chrome.downloads.download` setup script | One-click "Install Trojan & Start" |
| **(new)** Dev tools ignoring OS proxy | App Proxy Injector presets |
| **(new)** Distribution mechanism | Tauri v2 auto-updater |

## 3. Project structure

```
HaioBypass-app/
├── Cargo.toml
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── build.rs
│   ├── icons/
│   ├── capabilities/
│   └── src/
│       ├── main.rs
│       ├── app/
│       │   ├── mod.rs
│       │   ├── commands.rs
│       │   └── events.rs
│       ├── config/
│       │   ├── mod.rs
│       │   └── trojan_url.rs
│       ├── trojan/
│       │   ├── manager.rs
│       │   ├── bundled.rs
│       │   └── config_writer.rs
│       ├── domains/
│       │   ├── fetcher.rs
│       │   ├── fallback.rs
│       │   └── store.rs
│       ├── proxy/
│       │   ├── server.rs
│       │   ├── router.rs
│       │   ├── socks.rs
│       │   └── pac.rs
│       ├── osproxy/
│       │   ├── mod.rs
│       │   ├── windows.rs
│       │   ├── linux.rs
│       │   └── macos.rs
│       ├── appproxy/
│       │   ├── mod.rs
│       │   ├── gradle.rs
│       │   ├── maven.rs
│       │   ├── npm.rs
│       │   ├── pip.rs
│       │   ├── git.rs
│       │   ├── docker.rs
│       │   ├── goproxy.rs
│       │   ├── curl.rs
│       │   └── detect.rs
│       ├── autostart/
│       │   ├── mod.rs
│       │   ├── windows.rs
│       │   ├── linux.rs
│       │   └── macos.rs
│       ├── health.rs
│       ├── tray.rs
│       ├── updater.rs
│       └── error.rs
├── src/
│   ├── index.html
│   ├── styles/
│   │   └── style.css
│   ├── scripts/
│   │   ├── main.js
│   │   ├── settings.js
│   │   ├── invoke.js
│   │   └── i18n.js
│   └── assets/icons/
├── frontend/dist/
├── resources/trojan-go/
│   ├── trojan-go-windows-amd64.exe
│   ├── trojan-go-linux-amd64
│   ├── trojan-go-darwin-amd64
│   └── trojan-go-darwin-arm64
├── scripts/
│   ├── fetch-trojan.sh
│   └── gen-icons.sh
├── .github/workflows/release.yml
└── PLAN.md
```

## 4. Component specs

### 4.1 Frontend
- Copy popup.html/css/js/icons; swap chrome.* → invoke().
- Remove "download setup script" card → one-click "Install Trojan & Start".
- New Settings panel: preset toggles, port, autostart, tray prefs.
- Subscribe to events: status:update, trojan:status, domains:updated, health:check, update:available.
- **Port input** shows/edits `http_proxy_port` (11032), persisted via `set_port` command.

### 4.2 Config
- Config dir: ~/.haiobypass (Linux/macOS) / %USERPROFILE%\.haiobypass (Windows).
- State:
  - `proxyPort` (11031) — trojan-go SOCKS5 (local only)
  - `httpProxyPort` (11032) — HTTP CONNECT proxy (OS proxy target)
  - `enabled`, `cachedDomains`, `lastFetch`, `usingFallback`, `usingCache`, `enabledPresets`
  - `trojanUrl`, `trojanConfig`, `autostart`, `minimizeToTray`
- Password via keyring crate; fallback to config file.

### 4.3 TrojanMgr
- ensure_binary(): extract embedded binary for current OS/arch; chmod +x.
- write_config(): config.json with `verify: true` / `verify_hostname: true` (SNI).
- start/stop/status/restart: tokio::process::Command.
- monitor(): watchdog task on child process; auto-restart on crash with exp backoff (cap 5).
- Capture stderr to `~/.haiobypass/trojan.log`.

### 4.4 DomainMgr
- fetch_domains(): reqwest + rustls, timeout, fallback.
- **Sanitize** remote list: lowercase/trim; reject lines containing `/`, `:`, or whitespace; drop DNS-infra entries (`ns*.`, `*-hostmaster.*`, `dns-admin.*`, etc.); dedupe.
- FALLBACK_DOMAINS: hardcoded list (verbatim from extension) — used only when no fetch and no cache.
- 60-min interval; hot-swap into proxy via `set_domains()`.
- Cache strategy: fetch → `cached_domains`; on failure use cache (`using_cache`); fallback only if cache empty.

### 4.5 RoutingProxy
- TcpListener on 127.0.0.1:HTTP_PROXY_PORT (default 11032).
- CONNECT: domain match → SOCKS5 11031 or direct.
- Plain HTTP `GET/POST http://host/...` (absolute-URI): same routing.
- `/pac.js` endpoint: PAC script using live router domains + `PROXY 127.0.0.1:{http_port}`.
- Stop: drop TcpListener + use Notify for clean exit.

### 4.6 OS Proxy
- Windows: winreg ProxyEnable/ProxyServer + WinINet broadcast.
- Linux: gsettings (GNOME) or kioslaverc (KDE).
- macOS: networksetup `-setwebproxy`/`-setsecurewebproxy` with correct `http_port` (no SOCKS).
- Always backup → apply → restore on exit/crash.

### 4.7 AppProxy Injector
- 8 presets: gradle/maven/npm/pip/git/docker/go/curl.
- Each: apply() backs up originals, writes proxy config.
- clear() restores backups.
- **clear_all iterates the fixed preset set** (not just `self.backups.keys()`), so npm/git/go are always cleared.
- Default on: Gradle + git.

### 4.8 Tray
- Tauri v2 TrayIconBuilder. Menu: Toggle, Status, Open, Start-with-system, Quit.

### 4.9 Auto-updater
- GitHub Releases signed feed. Check on launch + every 12h.
- CI builds + signs + uploads latest.json on tag push.

### 4.10 Crash Sentinel
- Write `~/.haiobypass/proxy.sentinel` on enable.
- On startup: if sentinel exists, restore OS proxy + clear app presets + delete sentinel.
- Panic hook: restore on crash then exit.
- Clean delete in `quit_and_restore`.

## 5. Data flow — Enable Proxy

```
User clicks Enable
  → invoke('enable_proxy')
     1. domains := cachedDomains or FALLBACK_DOMAINS (honest using_fallback flag)
     2. trojan.start(parsed) — wait for SOCKS5 11031 to accept TCP (short timeout)
     3. proxy.set_domains(domains)
     4. proxy.start() — bind HTTP CONNECT proxy on 127.0.0.1:11032
     5. osproxy.backup() + apply(addr)
     6. appproxy.apply(presets, addr)
     7. state.enabled = true; persist + write sentinel
     8. start 60-min refresh interval
     9. fetch_domains() once in background → router.set_domains + emit 'domains:updated'
     10. emit 'status:update'
```

## 6. Build

```
make fetch-trojan   # download binaries into resources/
make dev            # tauri dev
make build          # tauri build → NSIS/AppImage/.deb/.dmg
```

## 7. Implementation phases

1. Scaffold + frontend port + command stubs ✅
2. Config + trojan:// parser ✅
3. TrojanMgr ✅
4. RoutingProxy + /pac.js ✅
5. OS proxy (Win/Linux/macOS) ✅
6. AppProxy injector (8 presets) ✅
7. DomainMgr ✅ (wired in this pass)
8. Health, tray, autostart, settings UI — partially done
9. End-to-end wiring — this pass
10. Auto-updater — placeholder
11. Packaging + CI — future
12. Testing — future

## 8. Risks

- Leftover proxy on crash → sentinel + restore (implemented)
- Linux DE fragmentation → GNOME+KDE first-class
- Bundled binary AV false positives → document exclusion + codesign
- Docker daemon proxy needs root → v1 only config.json
- Auto-updater signing keys → CI secrets only
