# What HaioBypass does to your machine — full transparency

HaioBypass routes traffic to blocked services through a proxy server. This
document lists every host it contacts, every file it creates or modifies, and
how to remove all of it. It is maintained with each release; if you find a
discrepancy, open an issue.

## What the app connects to

| Host | Purpose | When |
|---|---|---|
| The proxy server from your access key (`trojan://…` / `haio://…` URL) | Carries the tunnelled traffic for the domains below | While the bypass is enabled |
| `tools.haiocloud.com` / `raw.githubusercontent.com` (haiocloud/bypass-domains) | Fetch the list of domains to route (updates only; a default list ships in the app) | Hourly while enabled |
| `api.github.com` (via the Tauri updater) | Check for application updates | On startup |
| `go.microsoft.com` | Download the WebView2 Runtime installer (Microsoft-signed, signature-verified before execution) — only if your Windows is missing the runtime | Only during the one-time repair |

The domain list is constrained: it can only ever influence *which domains are
routed* — it cannot change your server, credentials, ports, or enable
features. Lists larger than 512 KB or containing non-domain entries are
rejected outright.

## Files created or modified

The app's own directory (all of it is removed on uninstall):

- `%USERPROFILE%\.haiobypass\state.json` (Windows) / `~/.haiobypass/state.json` — settings, cached domain list, saved access key (as given; treat it as a secret).
- `%TEMP%\HaioBypass-WebView2-Repair\` — temporary download of the Microsoft WebView2 installer, only when the runtime is missing. Safe to delete any time.

While the bypass is enabled, with your explicit consent (each is off by
default and opt-in):

- **System proxy (opt-in, one-time prompt):** Windows — `HKCU\...\Internet Settings` proxy/PAC values (previous value backed up and restored on disconnect); Linux — GNOME proxy settings via `gsettings`; macOS — network setup via `networksetup`.
- **QUIC block (opt-in, off by default):** a firewall rule named `HaioBypass Block QUIC` blocking outbound UDP 443 (Windows), an iptables `DROP` rule (Linux), or a `pf` anchor (macOS). Requires elevation. Removed on disconnect.
- **Dev tool presets (each opt-in, off by default):** backed up and restored on disconnect.
  - Gradle: `~/.gradle/gradle.properties`
  - Maven: `~/.m2/settings.xml`
  - pip: `~/AppData/Roaming/pip/pip.ini` (Windows) or `~/.config/pip/pip.conf`
  - Docker: `~/.docker/config.json`
  - curl: `~/.curlrc`

## Startup registration (opt-in)

- Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value
  `HaioBypass` pointing at the app. **No scheduled task and no elevation is
  used.** Older versions created a `schtasks` entry named `HaioBypass` —
  disable/enable once to clean it up (the current version deletes registry
  entries only).
- The app never writes to `HKLM`, never installs a service, and never
  requires administrator rights for any feature except the optional QUIC
  block.

## There is no payload

HaioBypass contains no bundled proxy executable and does not drop or execute
any binary other than the Microsoft-signed WebView2 runtime installer in the
missing-runtime repair path described above. The proxy client runs inside the
app process itself.

## How to remove everything

1. In the app: turn the bypass **off** (restores proxy settings, firewall
   rule, and all dev-tool files), turn off "Start with system" and any
   enabled presets.
2. Uninstall via Windows Settings/Apps (or the NSIS uninstaller).
3. Delete leftover data (uninstallers do not touch user data):
   - `%USERPROFILE%\.haiobypass\`
   - `%TEMP%\HaioBypass-WebView2-Repair\`
4. Verify the registry is clean (Windows): `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` should have no `HaioBypass` value.
5. Verify the firewall is clean: Windows Defender Firewall → Advanced Settings → Outbound Rules → no `HaioBypass Block QUIC` rule.

## Detection by antivirus

HaioBypass is a censorship-circumvention tool; some vendors classify the
whole category as PUA (potentially unwanted application). The app is built
open-source, does not obfuscate itself, and is not code-signed yet — until a
certificate is available, Windows SmartScreen may show a warning on install.
