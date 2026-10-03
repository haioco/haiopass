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
set -euo pipefail

DEST_DIR="$(cd "$(dirname "$0")/../resources/webview2" && pwd 2>/dev/null || echo "$(pwd)/resources/webview2")"
mkdir -p "$DEST_DIR"

INDEX_URL="https://api.nuget.org/v3-flatcontainer/webview2.runtime.x64/index.json"
INNER_PREFIX="contentFiles/any/any/WebView2"
FIXED_RUNTIME_DIR="$DEST_DIR/WebView2FixedRuntime"

if [ -f "$FIXED_RUNTIME_DIR/msedgewebview2.exe" ] && [ -d "$FIXED_RUNTIME_DIR/EBWebView" ]; then
  echo "✓ Fixed runtime already staged at $FIXED_RUNTIME_DIR"
  exit 0
fi

# Pick a downloader: curl > wget > python3
DOWNLOADER=""
if command -v curl >/dev/null 2>&1; then DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then DOWNLOADER="wget"
elif command -v python3 >/dev/null 2>&1; then DOWNLOADER="python3"
else echo "✗ No downloader available (curl/wget/python3)"; exit 1
fi

echo "Using downloader: $DOWNLOADER"

# Robust download with retries; curl/wget resume partial files between attempts.
fetch() { # fetch <url> <dest>
  local url="$1" dest="$2" attempt
  for attempt in 1 2 3 4 5; do
    case "$DOWNLOADER" in
      curl) curl -fsSL --retry 3 --retry-delay 2 -C - -o "$dest" "$url" && return 0 ;;
      wget) wget -q --tries=3 -O "$dest" "$url" && return 0 ;;
      python3) python3 - "$url" "$dest" <<'PYEOF' && return 0
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
fetch "$INDEX_URL" "$INDEX_TMP"

# Versions array is ascending — the last entry is the newest release.
WV2_VERSION="$(grep -oE '[0-9]+(\.[0-9]+)+' "$INDEX_TMP" | tail -1)"
rm -f "$INDEX_TMP"
[ -n "$WV2_VERSION" ] || { echo "✗ Could not parse latest WebView2 runtime version from feed"; exit 1; }
echo "✓ Latest WebView2 runtime in feed: ${WV2_VERSION}"

NUPKG_URL="https://api.nuget.org/v3-flatcontainer/webview2.runtime.x64/${WV2_VERSION}/webview2.runtime.x64.${WV2_VERSION}.nupkg"
NUPKG_TMP="$DEST_DIR/.webview2-${WV2_VERSION}.nupkg"
fetch "$NUPKG_URL" "$NUPKG_TMP"
echo "✓ Downloaded $(du -h "$NUPKG_TMP" | cut -f1)"

echo "Extracting fixed runtime ($INNER_PREFIX/*)..."
rm -rf "$FIXED_RUNTIME_DIR"
mkdir -p "$FIXED_RUNTIME_DIR"
if command -v unzip >/dev/null 2>&1; then
  unzip -q "$NUPKG_TMP" "$INNER_PREFIX/*" -d /tmp/wv2_extract
  cp -r /tmp/wv2_extract/"$INNER_PREFIX"/* "$FIXED_RUNTIME_DIR/"
  rm -rf /tmp/wv2_extract
else
  python3 - "$NUPKG_TMP" "$INNER_PREFIX" "$FIXED_RUNTIME_DIR" <<'PYEOF'
import sys, zipfile, os, shutil
pkg, prefix, dest = sys.argv[1], sys.argv[2].rstrip("/") + "/", sys.argv[3]
with zipfile.ZipFile(pkg) as z:
    for n in z.namelist():
        if n.startswith(prefix) and not n.endswith("/"):
            out = os.path.join(dest, n[len(prefix):])
            os.makedirs(os.path.dirname(out), exist_ok=True)
            with z.open(n) as src, open(out, "wb") as dst:
                shutil.copyfileobj(src, dst)
PYEOF
fi
rm -f "$NUPKG_TMP"

if [ ! -f "$FIXED_RUNTIME_DIR/msedgewebview2.exe" ] || [ ! -d "$FIXED_RUNTIME_DIR/EBWebView" ]; then
  echo "✗ Extracted runtime is incomplete (missing msedgewebview2.exe or EBWebView/)"
  exit 1
fi

echo "✓ Fixed Runtime ${WV2_VERSION} staged at $FIXED_RUNTIME_DIR ($(du -sh "$FIXED_RUNTIME_DIR" | cut -f1))"
ls "$FIXED_RUNTIME_DIR" | head -5
