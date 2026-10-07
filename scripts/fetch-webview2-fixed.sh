#!/usr/bin/env bash
# Stages the newest fixed-version WebView2 Runtime (x64) as a fixed-runtime
# folder for Tauri's fixedRuntime bundling mode
# (src-tauri/tauri.conf.json -> bundle.windows.webviewInstallMode).
#
# Source: WebView2.Runtime.X64 nupkg (fixed-version redistribution of the
# official Microsoft binaries, contentFiles/any/any/WebView2/*).
# Auto-tracks the newest version in the feed so every release ships a
# current Chromium. The win7 build pins 109 separately
# (scripts/fetch-webview2-109.sh) because 109 is the last Win7-capable major.
#
# NOTE: staged INSIDE src-tauri on purpose. Tauri resources with a `..` in
# their path get extracted under `_up_` (tauri-utils rewrites `..` to
# `_up_`), which is how 2.0.2 shipped a runtime nobody could find.
# `src-tauri/WebView2FixedRuntime` + config path `./WebView2FixedRuntime`
# extracts to `$INSTDIR\WebView2FixedRuntime` — next to the exe, where the
# app's preflight (src-tauri/src/webview_check.rs) wires it up via
# WEBVIEW2_BROWSER_EXECUTABLE_FOLDER.
#
# Since 152.0.4191.53 the runtime no longer fits NuGet's 250 MB package
# limit, so the feed splits it in TWO packages:
#   WebView2.Runtime.X64       — loader shell (msedgewebview2.exe, paks, ...)
#   WebView2.Runtime.X64.Core — the browser core (msedge.dll, ~150 MB)
# Both are downloaded and merged into one staging dir. A staging without
# the Core half LOOKS fine and passes file-count checks, but msedgewebview2
# can never boot without msedge.dll — the loader finds it, launches it, it
# dies, and the app hangs in WebView2 environment creation. That exact
# half-package is what shipped broken before this was caught; the
# msedge.dll assertion below now refuses it.
set -euo pipefail

DEST_DIR="$(cd "$(dirname "$0")/../src-tauri" && pwd 2>/dev/null || echo "$(pwd)/src-tauri")"
mkdir -p "$DEST_DIR"

BASE_INDEX_URL="https://api.nuget.org/v3-flatcontainer/webview2.runtime.x64/index.json"
CORE_INDEX_URL="https://api.nuget.org/v3-flatcontainer/webview2.runtime.x64.core/index.json"
FIXED_RUNTIME_DIR="$DEST_DIR/WebView2FixedRuntime"

validate_runtime() { # validate_runtime <dir> — payload completeness check
  local d="$1" n
  # Deliberately does NOT check EBWebView/: 109 predates it, 128+ ships it.
  # msedge.dll ships in every COMPLETE runtime (native in 109, via the
  # .Core package in 152+) — its absence means an incomplete payload,
  # which is precisely the "looks staged, never boots" failure.
  for f in msedgewebview2.exe msedge.dll resources.pak icudtl.dat; do
    if [ ! -f "$d/$f" ]; then
      echo "  missing runtime payload file: $f"
      return 1
    fi
  done
  # A truncated or error-page download still extracts "something"; a real
  # fixed runtime is 50+ files.
  n=$(find "$d" -type f 2>/dev/null | wc -l | tr -d ' ')
  if [ "${n:-0}" -lt 30 ]; then
    echo "  only $n files extracted — payload looks truncated"
    return 1
  fi
  return 0
}

if validate_runtime "$FIXED_RUNTIME_DIR"; then
  echo "✓ Fixed runtime already staged at $FIXED_RUNTIME_DIR"
  exit 0
fi

# Pick a downloader: curl > wget > python
DOWNLOADER=""
if command -v curl >/dev/null 2>&1; then DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then DOWNLOADER="wget"
else DOWNLOADER="python"
fi
# Python is also the extractor (nupkg = zip; the payload root can differ
# between the base and .Core packages, so unzip with a fixed prefix is not
# safe). Prefer python3, fall back to python (Windows runners).
if command -v python3 >/dev/null 2>&1; then PY=python3
else PY=python
fi

echo "Using downloader: $DOWNLOADER; extractor: $PY"

# Robust download with retries; curl/wget resume partial files between attempts.
fetch() { # fetch <url> <dest>
  local url="$1" dest="$2" attempt
  for attempt in 1 2 3 4 5; do
    case "$DOWNLOADER" in
      curl) curl -fsSL --retry 3 --retry-delay 2 -C - -o "$dest" "$url" && return 0 ;;
      wget) wget -q --tries=3 -O "$dest" "$url" && return 0 ;;
      python) "$PY" - "$url" "$dest" <<'PYEOF' && return 0
import sys, urllib.request
url, out = sys.argv[1], sys.argv[2]
try:
    urllib.request.urlretrieve(url, out)
except Exception as e:
    print(f"✗ download failed: {e}", file=sys.stderr); sys.exit(1)
PYEOF
        ;;
    esac
    echo "⚠ download attempt ${attempt} failed — retrying in 3s..."
    sleep 3
  done
  echo "✗ download failed after 5 attempts: $url"
  return 1
}

INDEX_TMP="$DEST_DIR/.webview2-index.json"
fetch "$BASE_INDEX_URL" "$INDEX_TMP"

# Versions array is ascending — the last entry is the newest release.
WV2_VERSION="$(grep -oE '[0-9]+(\.[0-9]+)+' "$INDEX_TMP" | tail -1)"
rm -f "$INDEX_TMP"
[ -n "$WV2_VERSION" ] || { echo "✗ Could not parse latest WebView2 runtime version from feed"; exit 1; }
echo "✓ Latest WebView2 runtime in feed: ${WV2_VERSION}"

BASE_URL="https://api.nuget.org/v3-flatcontainer/webview2.runtime.x64/${WV2_VERSION}/webview2.runtime.x64.${WV2_VERSION}.nupkg"
CORE_URL="https://api.nuget.org/v3-flatcontainer/webview2.runtime.x64.core/${WV2_VERSION}/webview2.runtime.x64.core.${WV2_VERSION}.nupkg"

BASE_TMP="$DEST_DIR/.webview2-${WV2_VERSION}.nupkg"
CORE_TMP="$DEST_DIR/.webview2-core-${WV2_VERSION}.nupkg"
fetch "$BASE_URL" "$BASE_TMP"
fetch "$CORE_URL" "$CORE_TMP"

# A proxy/CDN error page can arrive as a tiny "successful" response; without
# this guard the extraction below happily produced a broken runtime.
for f in "$BASE_TMP" "$CORE_TMP"; do
  BYTES=$(wc -c < "$f" | tr -d ' ')
  if [ "${BYTES:-0}" -lt 20000000 ]; then
    echo "✗ nupkg $f is only ${BYTES} bytes — download truncated or blocked"
    rm -f "$BASE_TMP" "$CORE_TMP"
    exit 1
  fi
done
echo "✓ Downloaded base $(du -h "$BASE_TMP" | cut -f1) + core $(du -h "$CORE_TMP" | cut -f1)"

echo "Extracting and merging WebView2 payload (base + core)..."
rm -rf "$FIXED_RUNTIME_DIR"
mkdir -p "$FIXED_RUNTIME_DIR"
for pkg in "$BASE_TMP" "$CORE_TMP"; do
  "$PY" - "$pkg" "$FIXED_RUNTIME_DIR" <<'PYEOF'
import sys, zipfile, os, shutil
pkg, dest = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(pkg) as z:
    names = [n for n in z.namelist() if not n.endswith('/')]
    # Locate the WebView2 payload root inside the nupkg. The base package
    # uses contentFiles/any/any/WebView2; the .Core package may nest it
    # differently (e.g. .../WebView2.Core) — find the single directory
    # segment starting with `WebView2` and map everything under it to the
    # staging root so both packages merge.
    roots = set()
    for n in names:
        parts = n.split('/')
        for i, p in enumerate(parts[:-1]):
            if p == 'WebView2' or p.startswith('WebView2.'):
                roots.add('/'.join(parts[:i + 1]))
                break
    if not roots:
        raise SystemExit(f"✗ no WebView2 payload found in {pkg}")
    if len(roots) > 1:
        raise SystemExit(f"✗ ambiguous WebView2 payload roots in {pkg}: {roots}")
    root = sorted(roots)[0] + '/'
    for n in names:
        if n.startswith(root):
            out = os.path.join(dest, n[len(root):])
            os.makedirs(os.path.dirname(out), exist_ok=True)
            with z.open(n) as src, open(out, 'wb') as dst:
                shutil.copyfileobj(src, dst)
PYEOF
done
rm -f "$BASE_TMP" "$CORE_TMP"

if ! validate_runtime "$FIXED_RUNTIME_DIR"; then
  echo "✗ Merged WebView2 ${WV2_VERSION} runtime is incomplete — refusing to ship it"
  exit 1
fi

echo "✓ Fixed Runtime ${WV2_VERSION} staged at $FIXED_RUNTIME_DIR ($(du -sh "$FIXED_RUNTIME_DIR" | cut -f1))"
ls "$FIXED_RUNTIME_DIR" | head -5
