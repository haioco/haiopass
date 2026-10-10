# AV false-positive hardening — implementation plan

**Status:** plan only. Nothing in this document has been implemented.
**Baseline:** `main` @ `7dd65c9` ("Merge fix/webview2-runtime-wiring"), app version 2.0.3.
**Author of the analysis:** repo audit, 2026-10-10.

---

## 1. The problem

Windows antivirus flags `HaioBypass` — in some cases quarantining it, in others warning.

Two independent causes, verified:

### 1.1 Nothing we ship is code-signed

Read from the published release assets over HTTP range requests. The PE `Certificate Table`
data directory (optional header offset, `magic`-dependent) and the MSI compound-file
signature streams:

| Asset | Signature state |
|---|---|
| `HaioBypass_2.0.3_x64-setup.exe` | cert table `offset=0 size=0` → **unsigned** |
| `HaioBypassWin7_2.0.3_x64-setup.exe` | cert table `offset=0 size=0` → **unsigned** |
| `HaioBypass_2.0.2_x64-setup.exe` | cert table `offset=0 size=0` → **unsigned** |
| `HaioBypassWin7_2.0.2_x64-setup.exe` | cert table `offset=0 size=0` → **unsigned** |
| `HaioBypass_2.0.3_x64_en-US.msi` | no `MsiDigitalSignature` stream → **unsigned** |
| `HaioBypassWin7_2.0.3_x64_en-US.msi` | no `MsiDigitalSignature` stream → **unsigned** |

`resources/trojan-go/haio-proxy-windows-amd64.exe` is also unsigned (cert table `0/0`).

Cause: `.github/workflows/release.yml` gates both the SignPath submit step and the
"replace unsigned assets with signed versions" step on `vars.SIGNPATH_ENABLED == 'true'`.
That variable is not set, so both steps have been silently skipped on every release to
date. `SIGNPATH.md` describes the setup as "one-time manual" and was evidently never
completed — its §"Submitting the first signed build to Microsoft" instructions refer to a
certificate that does not exist.

### 1.2 The runtime behaviour is a dropper profile

These fire on behaviour-based engines **regardless of signature**:

| # | Behaviour | Location |
|---|---|---|
| 1 | A complete trojan-go client is `include_bytes!`-embedded in the app binary | `src-tauri/src/trojan/bundled.rs` |
| 2 | Written to `%APPDATA%/…/haio-proxy.exe` at runtime | `bundled.rs::extract_bundled`, called from `manager.rs::ensure_binary` |
| 3 | Spawned as a hidden child (`CREATE_NO_WINDOW = 0x08000000`) | `trojan/manager.rs`, both `start` and the watchdog |
| 4 | A watchdog re-extracts and re-spawns the payload up to 3× | `manager.rs::start_watchdog` |
| 5 | Kill-by-image-name cleanup | `manager.rs::kill_stray_processes` → `taskkill /F /IM haio-proxy.exe /T`, `pkill -9 -x haio-proxy` |
| 6 | System-wide proxy takeover + settings broadcast | `osproxy/windows.rs::set_proxy`, `broadcast_settings_change` |
| 7 | Firewall rule creation (`netsh advfirewall … action=block`) | `osproxy/quic.rs::block` (Windows impl) |
| 8 | High-privilege persistence: `schtasks /Create /SC ONLOGON /RL HIGHEST` | `autostart/windows.rs::enable` |
| 9 | Rewrites credential-bearing dev-tool config | `appproxy/{gradle,maven,pip,docker,curl}.rs` |
| 10 | Remote-controlled selective routing | `domains/fetcher.rs` + `domains/fallback.rs::DOMAINS_URLS` |
| 11 | Download-execute from temp | `webview_check.rs::download_installer` / `run_installer` |
| 12 | ~150 MB of Microsoft runtime unpacked to Program Files | `webviewInstallMode: fixedRuntime` in both tauri configs |

Note that the repo already *knows* about this — `fetch-trojan.sh`, `bundled.rs` and
`manager.rs` all carry comments about reducing AV false positives. The mitigation applied
was renaming the payload, which does not change its hash and does not affect any
behaviour-based rule.

---

## 2. The constraint: SignPath is not available yet

This plan is written for that reality. Two consequences:

**Signing alone would not be enough.** A signature clears SmartScreen and raises the trust
floor, but behaviour-based engines (Defender behavioural/ASR, ESET, Kaspersky heuristics)
score on what the code does. The drop→execute→hide→respawn→firewall→scheduled-task chain
fires on a signed binary exactly the same as an unsigned one. Fixing only the signature
would convert "quarantine" into "SmartScreen warning" and leave the real problem.

**So the centre of gravity shifts to removing the payload entirely.** Implementing the
Trojan client in-process eliminates causes 1, 2, 3, 4 and 5 in one move — which is
precisely the class of behaviour that produces *quarantine*, as opposed to *warning*.
That is the correct order of operations while no cert is available.

Work items are therefore ordered by detection-reduction per unit of effort, with signing
deferred to a later phase (W12) rather than treated as the prerequisite.

---

## 3. Some detection is irreducible

Selective traffic redirection through a proxy server, in order to reach services that are
blocked, is a censorship-circumvention tool. Several vendors will score it as PUA
permanently and no amount of engineering removes that.

The achievable target:

- **Zero quarantine / zero behavioural alerts** on mainstream consumer AV
  (Windows Defender, ESET, Kaspersky, Avast/AVG, Bitdefender).
- **SmartScreen warning** remains until a certificate with reputation exists — accepted for now.
- **PUA classification** may remain — accepted and not worth fighting.

Design and prioritise against that ceiling. Do not spend effort trying to reach
"undetected everywhere".

---

## 4. Work items

### Phase 1 — cheap, independent, no behaviour change (≈2 hours total)

---

#### W1. `webviewInstallMode` → `embedBootstrapper`

**Files:** `src-tauri/tauri.conf.json`, `src-tauri/tauri.win7.conf.json`
(both under `bundle.windows.webviewInstallMode`)

```diff
-"webviewInstallMode": { "type": "fixedRuntime", "path": "./WebView2FixedRuntime" }
+"webviewInstallMode": { "type": "embedBootstrapper" }
```

and for win7, `"./WebView2FixedRuntime109"` likewise.

**Why:** the win10 installer went 69 MB → 198 MB between 2.0.2 and 2.0.3, and the MSI
90 MB → 271 MB, entirely because of the fixed runtime. An NSIS installer that unpacks
~150 MB of Microsoft binaries plus a 14 MB proxy exe to Program Files is a textbook
`Installer/Waclsac` / `NSIS` heuristic match. `embedBootstrapper` is ~1.8 MB, works
offline, and everything it installs is Microsoft-signed.

**Ripple effects — do these too:**
- Delete the WebView2 staging steps from `.github/workflows/release.yml` (the `Prepare
  fixed WebView2 runtime` and `Prepare Windows 7 offline WebView2 109` steps) and from
  `.github/workflows/ci.yml`.
- Delete `scripts/fetch-webview2-fixed.sh` and `scripts/fetch-webview2-109.sh`.
- Delete the `scripts/` runtime-cooldown marker logic in `webview_check.rs` only if it
  becomes unreachable; otherwise leave it (it is still the fallback for machines where
  the bootstrapper was blocked by policy).
- `webview_check.rs::bundled_runtime()` becomes dead — remove it and the
  `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` wiring, and simplify to registry-only detection.

**Regression risk:** this re-introduces the "WebView2 not found" ticket class that
2.0.x spent several releases fixing — the reason the fixed runtime was bundled. The
mitigation is `webview_check::preflight()`, which already exists and already downloads the
Evergreen installer as a repair path. **Test on a clean Windows 10 VM with no Evergreen
runtime installed before shipping.**

**Done when:** installer is < 30 MB, launches on a clean VM, and `.github/workflows/release.yml`
no longer contains a WebView2 staging step.

---

#### W2. Delete the `/RL HIGHEST` scheduled task

**File:** `src-tauri/src/autostart/windows.rs::enable`

Remove the `schtasks` branch entirely; keep only the `HKCU\Software\Microsoft\Windows\
CurrentVersion\Run` write, which needs no elevation. `is_enabled()` and `disable()`
simplify to registry-only.

**Why:** `schtasks /Create /SC ONLOGON /RL HIGHEST` is the single most malware-shaped
persistence primitive in the codebase. Removing it also means the app never requests
elevation, which takes the UAC prompt out of the detection surface altogether.

**Done when:** `grep -rn schtasks src-tauri/src` returns nothing.

---

#### W3. Delete kill-by-image-name

**File:** `src-tauri/src/trojan/manager.rs::kill_stray_processes`

Remove the whole function and its call site in `stop()`.

**Why:** `taskkill /F /IM <name> /T` and `pkill -9 -x <name>` are cleanup primitives that
appear in essentially every malware family. We already hold the child handle.

**Note:** W3 is subsumed by W7 (with no child process there is nothing to kill), but do it
first so the tree builds and each step is independently reviewable.

---

#### W4. Remove the loose proxy binaries from the win7 bundle

**File:** `src-tauri/tauri.win7.conf.json`

```diff
-"resources": ["../resources/trojan-go/*"],
```

**Why:** this drops four unsigned `.exe` files into Program Files at install time. An
installer that scatters unsigned executables is worse than anything the app does later.

**Also:** once W7 lands, `resources/trojan-go/` has no consumer at all — delete the
directory (≈56 MB of repo weight) in the same commit.

---

#### W5. Resolve the `wix: null` inconsistency

**Files:** both tauri configs (`bundle.windows.wix`), and `.github/workflows/release.yml`

Both configs set `"wix": null`, yet MSIs are produced and published
(`HaioBypass_2.0.3_x64_en-US.msi`, 271 MB). Either the key is not being honoured or the
`"targets": "all"` sibling overrides it.

Decide explicitly:
- **Drop the MSI** — halves the unsigned artifact surface, and NSIS is enough for now; or
- **Keep the MSI** — it is load-bearing for the enterprise/GPO intent in `PLAN.md` §9 —
  and remove the dead `"wix": null` so the config stops lying.

Recommend: keep it, remove the dead key. MSI is the enterprise story.

**Done when:** the config and the release assets agree.

---

### Phase 2 — the real fix: in-process Trojan client (W6/W7)

This is the highest-value work in the plan and the reason the plan is reordered away from
"sign first". Design detail in §5.

---

#### W6. Implement the Trojan client in Rust

**New file:** `src-tauri/src/trojan/client.rs`
**New deps:** `tokio-rustls`, `rustls` (0.23), `rustls-native-certs`, `sha2`, `hex`
**Delete:** `src-tauri/src/trojan/bundled.rs`, `src-tauri/src/trojan/config_writer.rs`,
`resources/trojan-go/`, `scripts/fetch-trojan.sh`, `scripts/build-trojan-win7.sh`
**CI:** remove the "Download bundled haio-proxy binaries" and the Go/Docker rebuild step
from `release.yml` and `ci.yml`.

`client.rs` exposes one function:

```rust
pub async fn dial(config: &TrojanConfig, host: &str, port: u16)
    -> crate::error::Result<TrojanStream>
```

Full protocol spec, call graph and dependency-version risk in §5.

---

#### W7. Remove the child-process machinery

Once W6 is in place, delete:

- `trojan/manager.rs::start_watchdog` / `stop_watchdog` / `start` / `stop` /
  `kill_stray_processes` / `ensure_binary` — replace `TrojanManager` with a thin holder
  around `Arc<RwLock<Option<TrojanConfig>>>`.
- The `child` field, `std::process::Stdio`, `tokio::process::Command`,
  `creation_flags(0x08000000)`, the `log_path`/`haio-proxy.log` file.
- `commands.rs::wait_for_port` — there is no listening port to wait on any more.
- The health monitor's SOCKS-port liveness branch (`health.rs::check_socks5_health`) —
  replace with a cheap TLS reachability probe to the configured server.

**Consequential call-site changes** (each needs checking, not blind editing):

| Site | Current | Becomes |
|---|---|---|
| `app/commands.rs` `enable_proxy` step 2 | `trojan.start(tc, port)` + `wait_for_port(port)` | just store `tc` in the manager |
| `app/commands.rs` `install_and_start_trojan` | `ensure_binary` + `start` | rename to something like `connect`; just store config |
| `app/commands.rs` `quit_and_cleanup` | `trojan.stop()` | `trojan.clear()` |
| `app/commands.rs` health monitor | `trojan.stop()` + `trojan.start()` on 2 consecutive failures | drop the auto-restart; emit `health:failed` and let the UI offer a manual retry |
| `DomainRouter` | stores `socks_port`, exposes `socks_addr()` | drop both — it becomes pure domain-matching |
| `proxy/server.rs` | `socks::dial_socks5(&router.socks_addr(), host, port)` | `trojan::client::dial(&cfg, host, port)` |
| `proxy/socks.rs` | `dial_socks5` | delete; keep `dial_direct` |
| `trojan/status` event | emits a child PID | emits a boolean only — **the frontend consumes this, find and fix it** |

**Why the health monitor matters:** its current job is to detect that the dropped payload
died and silently respawn it. With an in-process client there is no process to die, and
auto-restart-on-failure is itself a backdoor-shaped behaviour. Convert it to a UI prompt.

**Done when:** `grep -rn "Command::new\|taskkill\|pkill\|creation_flags\|include_bytes" src-tauri/src`
returns nothing, and `resources/trojan-go/` is gone.

---

### Phase 3 — reduce the behavioural score of the features that remain (W8–W10)

---

#### W8. Default every side-effecting feature OFF, with explicit consent

**Files:** `src-tauri/src/config/mod.rs`, `src-tauri/src/appproxy/*`, `src-tauri/src/osproxy/*`,
and the settings UI under `frontend/`

- `config/mod.rs::Store::new()` currently seeds
  `enabled_presets: vec!["gradle".into()]`. Change to `vec![]`.
- The QUIC firewall block (`osproxy/quic.rs::block`) requires admin. Make it a separate,
  explicitly-labelled opt-in that explains *why* it needs admin, rather than something
  attempted implicitly during `enable_proxy`.
- The OS PAC takeover (`osproxy/windows.rs::set_proxy` + `broadcast_settings_change`)
  cannot be avoided for split tunnelling, so it stays — but it must be opt-in at the
  session level with a visible tray indicator while it is active, and a one-time
  explanatory prompt rather than a silent registry write.
- Each dev-tool preset stays individually opt-in, and the UI must name the exact file it
  will modify (`~/.docker/config.json`, `~/.m2/settings.xml`, …) before the toggle flips.

**Why:** the dev-tool presets rewrite files that hold package-registry and container-registry
credentials and TLS settings. "Modifies npm/maven/docker/pip/gradle/curl configuration" is a
named heuristic cluster. The firewall rule is "modifies system firewall". Both are far more
defensible when the user asked for them by name.

**Done when:** a fresh install, with the app never toggled by the user, modifies zero files
outside its own config directory.

---

#### W9. Constrain the remote domain list

**File:** `src-tauri/src/domains/`

Currently the app fetches `https://tools.haiocloud.com/domains.txt` hourly and routes
every match through the tunnel. That is remote-config-driven selective traffic
redirection — the classic RAT routing shape.

Mitigations, in order of value:
1. Ship the domain list in the binary as the default; treat the fetch as an *update*, and
   show the user a diff/diff-count before applying it.
2. If the fetch stays, hard-cap what a remote list can do: it may only ever *add* routing
   for domains, never change the server, credentials, or enable features.
3. Pin the accepted content shape (line count, charset, total bytes) and reject anything
   that looks anomalous, so a compromised endpoint cannot repurpose the client.

**Done when:** a compromised domains endpoint cannot alter anything but the domain set.

---

#### W10. Trim the WebView2 download-and-run path

**File:** `src-tauri/src/webview_check.rs`

`download_installer` + `run_installer` fetch `MicrosoftEdgeWebview2Setup.exe` from
`go.microsoft.com` into `%TEMP%\haiobypass-webview2\` and execute it `/silent /install`.
The binary is legitimately Microsoft-signed, but the shape is identical to a dropper.

Reduce the shape:
- Verify the download's Authenticode signature (publisher `Microsoft Corporation`) before
  executing. Cheap with `winverify`/`WinVerifyTrust`, or a Rust verifier.
- Verify the expected SHA-256 against a pinned value for the Evergreen bootstrapper.
- Rename the temp directory to something clearly product-branded.

**Why it is lower priority than W6:** the payload is genuinely signed by Microsoft and the
URL is a Microsoft-controlled redirector. Most engines special-case this. Keep it last.

---

### Phase 4 — packaging, reputation, transparency (W11–W12)

---

#### W11. Publish what the app does to the machine

**New file:** `TRANSPARENCY.md` at repo root, linked from the download page and the release body.

Plain language, no hedging:
- What hosts it connects to, and that the purpose is routing blocked services through a
  proxy.
- Every file it creates or modifies, with paths.
- What it registers for startup.
- How to uninstall completely, including cleaning the registry Run key, the PAC URL, the
  firewall rule and `~/.haiobypass`.

**Why:** transparency documentation measurably reduces PUA scoring and, practically,
absorbs the support tickets that follow every detection event. Cheapest item on the list.

---

#### W12. Signing — deferred, not cancelled

Everything here is blocked on provisioning and should be started the day a certificate
exists, because reputation accrues over days-to-weeks.

1. Finish the SignPath Foundation setup in `SIGNPATH.md` §"One-time manual setup" and set
   `SIGNPATH_ENABLED=true`. Confirm the signature actually lands by re-running the PE
   certificate-table check on the published asset — the pipeline failing open silently for
   four releases is exactly the trap here.
2. Once W6 is done there is no longer a dropped payload to sign, so SignPath only has to
   cover `*-setup.exe` and `*.msi`. This substantially raises the odds that the free
   Foundation programme is sufficient.
3. Submit to <https://www.microsoft.com/en-us/wdsi/filesubmission> for SmartScreen
   reputation, following `SIGNPATH.md` §"Submitting the first signed build to Microsoft".
4. Submit to major AV vendors and to VirusTotal for whitelisting once the binary is clean.

Add a CI guard so this can never silently regress:

```yaml
# after the publish job
- name: Verify published Windows assets are signed
  # download *.exe, assert non-zero PE certificate table, assert DigiCert/SignPath signer
```

---

## 5. Design detail — W6, the in-process Trojan client

### 5.1 Protocol

Trojan, client → server, over TLS. One TCP connection carries exactly one session —
it is a raw pipe and **cannot be multiplexed**.

Request header, immediately after the TLS handshake and before any payload:

```
hex(SHA224(password))   56 bytes, lowercase ASCII hex
CRLF                    0x0D 0x0A
CMD                     0x01 = CONNECT
CRLF
SOCKS5.ADDR             ATYP + ADDR + PORT(2, big-endian)
CRLF
<payload bytes>
```

`SOCKS5.ADDR` encodings:
- `0x01` — 4-byte IPv4
- `0x03` — 1-byte length + domain name
- `0x04` — 16-byte IPv6

Server replies with one byte: `0x00` = success, non-zero = Trojan error code. Read and
discard it before splicing.

### 5.2 TLS parameters that must match the current trojan-go config

From `trojan/config_writer.rs`, which is the behavioural contract the existing server
expects. Reproduce exactly, or connections will break:

| Setting | Value | Note |
|---|---|---|
| SNI | `config.sni` | distinct from `remote_addr`; may be a CDN front |
| Server name verification | against **`sni`**, not `remote_addr` | this is the whole point of the `sni` field |
| ALPN | `["h2", "http/1.1"]` | carry the same list; end-to-end semantics are irrelevant (it is a TCP tunnel) but the server may care |
| Curves | default (`""` in trojan-go means no override) | do not set `curve_prefs` |
| Cert verification | on (`verify: true`) | do not add an insecure mode |

Connect to `remote_addr:remote_port`; validate the certificate against `sni`.

### 5.3 TLS session resumption — required for performance parity

The current config sets `"reuse_session": true` and `"session_ticket": true`. Because a
Trojan stream cannot be multiplexed, "reuse" cannot mean reusing the TCP stream — it means
reusing the *TLS session* across new TCP connections, so the handshake skips certificate
validation.

Implement with rustls' shared in-memory session cache installed on **every** client config:

```rust
let cache = Arc::new(rustls::client::ClientSessionMemoryCache::new(256));
let config = rustls::ClientConfig::builder()
    .with_root_certificates(rustls_native_certs::load_native_certs()?)
    .with_client_auth_cert(...)
    .with_session_preallocation(...)
    .with_session_memory_cache(cache);   // <-- required
```

Skipping this makes every browser connection pay a full certificate chain validation.
That is a visible latency regression on exactly the traffic path users care about.

### 5.4 Call graph change

Today:

```
proxy::server::handle_connection
  └─ if router.should_proxy(host)
       └─ socks::dial_socks5(127.0.0.1:{proxy_port}, host, port)   ← SOCKS5 to trojan-go
            └─ trojan-go (child process) ──TLS──▶ remote server
       └─ else
            └─ socks::dial_direct(host, port)
```

After:

```
proxy::server::handle_connection(cfg: Arc<TrojanConfig>, ...)
  └─ if router.should_proxy(host)
       └─ trojan::client::dial(cfg, host, port)                   ← TLS to remote directly
  └─ else
       └─ socks::dial_direct(host, port)
```

`DomainRouter` loses `socks_port` and `socks_addr()`. `ProxyServer` gains an
`Arc<RwLock<Option<TrojanConfig>>>` (the same handle the manager owns) and passes it into
`handle_connection`. Because the config is now shared mutable state rather than a file
written to a child process, **changing the server/credential at runtime becomes a plain
swap** — no restart, no reconnect.

### 5.5 Stream type

`dial` returns a TLS stream, not a `TcpStream`. `copy_bidirectional` needs
`AsyncRead + AsyncWrite + Send + Unpin`. Use:

```rust
pub type TrojanStream = Box<dyn AsyncRead + AsyncWrite + Send + Unpin>;
```

and box the `client::TlsStream<TcpStream>`. Keep the existing `IDLE_TIMEOUT` wrap and the
`HTTP/1.1 200 Connection Established` / `502 Bad Gateway` semantics unchanged — only the
dial call differs, so the error-path behaviour in `proxy/server.rs` should not need edits.

### 5.6 Dependency risk — read this before adding crates

`reqwest = { version = "0.12", features = ["rustls-tls"] }` already pulls `rustls` and a
crypto provider. Adding `tokio-rustls` + `rustls` directly will fail to compile if the
versions or providers disagree.

- Pin `rustls = "0.23"` and `tokio-rustls = "0.26"`; check what `cargo tree` reports after
  `cargo update` and align to whatever reqwest resolved to.
- Match the provider reqwest selected (0.12 defaults to `ring`; `aws-lc-rs` is the
  alternative). Do not enable both — they conflict.
- `rustls-native-certs` on Windows reads the system store; consider whether the Evergreen
  WebView2 availability work in W1 affects this (it should not).
- `rustls` 0.23 also needs an explicit `CryptoProvider` if more than one is compiled in.

Verify with a trivial `ClientConfig` build before writing the dial path.

---

## 6. Verification checklist

Per item, before merging:

- [ ] `grep -rn "taskkill\|pkill\|creation_flags\|include_bytes\|Command::new(\"schtasks\")" src-tauri/src` → empty
- [ ] `ls resources/trojan-go` → does not exist
- [ ] Built NSIS installer < 30 MB
- [ ] Fresh Windows 10 VM with **no** Evergreen WebView2 runtime: install, launch, connect, browse a filtered domain
- [ ] Fresh Windows 10 VM **with** Evergreen runtime: same
- [ ] Server credential rotation at runtime takes effect without an app restart
- [ ] Disabling the proxy fully restores system state (PAC URL gone, Run key gone, firewall rule gone, dev-tool files restored)
- [ ] Upload installer to a fresh VirusTotal account — expect no *quarantine*, PUA acceptable
- [ ] Windows Security → Virus & threat protection shows no detection history for the install
- [ ] Windows Security → Firewall & network protection shows no `HaioBypass Block QUIC` rule unless explicitly enabled
- [ ] `schtasks /Query /TN HaioBypass` after enable → not found

---

## 7. Sequencing

```
Day 1   W1  W2  W3  W4  W5          small, independent, immediately reviewable
Day 2-4 W6                            the Trojan client (§5 is the design doc)
Day 5   W7  + frontend PID fix       delete the child-process machinery
Day 6   W8  W9  W10                  consent/defaults, domain-list constraint
Day 7   W11                          transparency doc
---- cert becomes available ----
Day 8   W12  + CI signature guard
```

W1–W5 are each independently revertable and can be split across reviewers. **W6 and W7
should land together** — W6 alone leaves the app unable to connect, and W7 alone cannot
compile.

---

## 8. Out of scope

- **The v3 native agent** (`PLAN.md` §9). `PLAN.md` already names bundled-binary AV false
  positives as a driver for it; this plan makes v2 shippable in the meantime rather than
  superseding v3.
- **Reaching "undetected everywhere".** See §3 — not achievable for this product category.
- **Obfuscation.** No packers, no anti-analysis, no string encryption. These convert a
  heuristic detection into a permanent one and make future false-positive submissions
  impossible.
- **Chinese AV market.** Separate distribution problem, separate work.

---

## 9. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| In-process client has worse throughput/latency than trojan-go | Medium | §5.3 session resumption is mandatory; benchmark against the old build before shipping |
| W1 re-opens the "WebView2 not found" ticket class | Medium | It is the reason the fixed runtime was bundled. Test on a clean VM; `webview_check::preflight()` is the fallback. Ship W1 behind a flag if unsure |
| rustls/reqwest version or provider conflict | High | §5.6 — check `cargo tree` before writing any dial code |
| Frontend consumes the dropped-process PID somewhere | Medium | `grep` the frontend for the `trojan:status` event payload before W7 |
| Behaviour-based alerts persist despite all of the above | Medium | §3 — PUA classification is expected and acceptable; measure against the §6 checklist, not against "no detections" |
| Signing slips again | Low | W12's CI guard makes it fail loudly instead of silently |