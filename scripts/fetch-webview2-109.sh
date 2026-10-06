#!/usr/bin/env bash
# Stages WebView2 Runtime 109 (last major with Windows 7 support) as a
# fixed-version runtime folder for Tauri's fixedRuntime bundling mode
# (src-tauri/tauri.win7.conf.json -> bundle.windows.webviewInstallMode).
#
# Source: WebView2.Runtime.X64 nupkg (fixed-version redistribution of the
# official Microsoft binaries, contentFiles/any/any/WebView2/*).
# Pinned to 109.0.1518.78 — the newest 109 available in that feed
# (all 109.x run on Win7; 109 is the last Win7-capable major).
set -euo pipefail

DEST_DIR="$(cd "$(dirname "$0")/../resources/webview2" && pwd 2>/dev/null || echo "$(pwd)/resources/webview2")"
mkdir -p "$DEST_DIR"

WV2_VERSION="109.0.1518.78"
NUPKG_URL="https://api.nuget.org/v3-flatcontainer/webview2.runtime.x64/${WV2_VERSION}/webview2.runtime.x64.${WV2_VERSION}.nupkg"
INNER_PREFIX="contentFiles/any/any/WebView2"
FIXED_RUNTIME_DIR="$DEST_DIR/WebView2FixedRuntime109"

validate_runtime() { # validate_runtime <dir> — checks real payload files, not EBWebView
  local d="$1"
  # EBWebView is the WebView2 *user-data* folder name, not part of the runtime
  # payload — checking for it made this validation fail on every run. Verify the
  # files the loader actually needs instead.
  for f in msedgewebview2.exe msedge.dll resources.pak; do
    if [ ! -f "$d/$f" ]; then
      echo "✗ Missing runtime payload file: $f"
      return 1
    fi
  done
  return 0
}

if [ -f "$FIXED_RUNTIME_DIR/msedgewebview2.exe" ] && validate_runtime "$FIXED_RUNTIME_DIR"; then
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

NUPKG_TMP="$DEST_DIR/.webview2-${WV2_VERSION}.nupkg"
case "$DOWNLOADER" in
  curl) curl -fsSL -o "$NUPKG_TMP" "$NUPKG_URL" ;;
  wget) wget -q -O "$NUPKG_TMP" "$NUPKG_URL" ;;
  python3) python3 - "$NUPKG_URL" "$NUPKG_TMP" <<'PYEOF'
import sys, urllib.request
url, out = sys.argv[1], sys.argv[2]
try:
    urllib.request.urlretrieve(url, out)
except Exception as e:
    print(f"✗ download failed: {e}", file=sys.stderr); sys.exit(1)
PYEOF
  ;;
esac
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

if ! validate_runtime "$FIXED_RUNTIME_DIR"; then
  echo "✗ Extracted WebView2 ${WV2_VERSION} runtime is incomplete — refusing to ship it"
  exit 1
fi

echo "✓ Fixed Runtime ${WV2_VERSION} staged at $FIXED_RUNTIME_DIR ($(du -sh "$FIXED_RUNTIME_DIR" | cut -f1))"
ls "$FIXED_RUNTIME_DIR" | head -5
