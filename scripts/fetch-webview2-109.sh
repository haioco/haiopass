#!/usr/bin/env bash
# Downloads WebView2 Runtime 109 (last version with Windows 7 support)
# for offline embedding in the Windows 7 build.
set -euo pipefail

DEST_DIR="$(cd "$(dirname "$0")/../resources/webview2" && pwd 2>/dev/null || echo "$(pwd)/resources/webview2")"
mkdir -p "$DEST_DIR"

# Version 109.0.1518.140 — last Win7/8/8.1 compatible
WV2_VERSION="109.0.1518.140"

# Microsoft offline installer URL (stable CDN path)
INSTALLER_URL="https://msedge.sf.dl.delivery.mp.microsoft.com/filestreamingservice/files/c1d53a34-a40a-4f6f-9b8f-2e5b4c2e8e9d/MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
# Fallback: NuGet fixed-runtime package (contains unpacked runtime folder)
NUGET_URL="https://www.nuget.org/api/v2/package/Microsoft.Web.WebView2/${WV2_VERSION}"

INSTALLER_PATH="$DEST_DIR/MicrosoftEdgeWebView2RuntimeInstallerX64_109.exe"
FIXED_RUNTIME_DIR="$DEST_DIR/WebView2FixedRuntime109"

# Pick a downloader: curl > wget > python3
DOWNLOADER=""
if command -v curl >/dev/null 2>&1; then DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then DOWNLOADER="wget"
elif command -v python3 >/dev/null 2>&1; then DOWNLOADER="python3"
else echo "✗ No downloader available (curl/wget/python3)"; exit 1
fi

echo "Using downloader: $DOWNLOADER"

download() {
  local url="$1" out="$2"
  case "$DOWNLOADER" in
    curl) curl -fsSL -o "$out" "$url" ;;
    wget) wget -q -O "$out" "$url" ;;
    python3) python3 - "$url" "$out" <<'PYEOF'
import sys, urllib.request
url, out = sys.argv[1], sys.argv[2]
try:
    urllib.request.urlretrieve(url, out)
except Exception as e:
    print(f"✗ download failed: {e}", file=sys.stderr); sys.exit(1)
PYEOF
    ;;
  esac
}

if [ -f "$INSTALLER_PATH" ] && [ -s "$INSTALLER_PATH" ]; then
  echo "✓ $INSTALLER_PATH already exists ($(du -h "$INSTALLER_PATH" | cut -f1))"
else
  echo "Downloading WebView2 Runtime ${WV2_VERSION} offline installer..."
  if ! download "$INSTALLER_URL" "$INSTALLER_PATH"; then
    echo "⚠ CDN URL failed, trying NuGet fallback..."
    NUGET_ZIP="/tmp/webview2.${WV2_VERSION}.nupkg"
    if download "$NUGET_URL" "$NUGET_ZIP"; then
      echo "  Extracting fixed runtime from NuGet..."
      rm -rf /tmp/wv2_extract && mkdir -p /tmp/wv2_extract
      unzip -q "$NUGET_ZIP" -d /tmp/wv2_extract 2>/dev/null || true
      if [ -d "/tmp/wv2_extract/build" ]; then
        mkdir -p "$FIXED_RUNTIME_DIR"
        cp -r /tmp/wv2_extract/build/native/x64/* "$FIXED_RUNTIME_DIR/" 2>/dev/null || true
        echo "✓ Fixed Runtime staged at $FIXED_RUNTIME_DIR"
      fi
      rm -rf /tmp/wv2_extract "$NUGET_ZIP"
    else
      echo "✗ Both CDN and NuGet downloads failed"
      echo "  Manual: download from https://developer.microsoft.com/en-us/microsoft-edge/webview2/#download-section"
      echo "  and place MicrosoftEdgeWebView2RuntimeInstallerX64.exe in $DEST_DIR/"
      exit 1
    fi
  else
    echo "✓ $INSTALLER_PATH saved ($(du -h "$INSTALLER_PATH" | cut -f1))"
  fi
fi

# Also try to prepare Fixed Runtime folder for tauri fixedRuntime mode
if [ ! -d "$FIXED_RUNTIME_DIR" ] || [ -z "$(ls -A "$FIXED_RUNTIME_DIR" 2>/dev/null)" ]; then
  echo "Preparing Fixed Runtime folder (for tauri fixedRuntime mode)..."
  NUGET_TMP="/tmp/wv2_nuget_${WV2_VERSION}.nupkg"
  if download "$NUGET_URL" "$NUGET_TMP" 2>/dev/null; then
    rm -rf /tmp/wv2_extract && mkdir -p /tmp/wv2_extract
    unzip -q "$NUGET_TMP" -d /tmp/wv2_extract 2>/dev/null || true
    if [ -d "/tmp/wv2_extract/build" ]; then
      mkdir -p "$FIXED_RUNTIME_DIR"
      cp -r /tmp/wv2_extract/build/native/x64/* "$FIXED_RUNTIME_DIR/" 2>/dev/null || true
      echo "✓ Fixed Runtime staged at $FIXED_RUNTIME_DIR"
    fi
    rm -rf /tmp/wv2_extract "$NUGET_TMP"
  else
    echo "⚠ Could not fetch NuGet fixed runtime — offlineInstaller mode will be used if installer exists"
  fi
fi

echo ""
echo "WebView2 109 resources:"
ls -lh "$DEST_DIR" 2>/dev/null || echo " (empty, manual download required)"
echo ""
echo "Tauri Win7 overlay should point to:"
echo "  offlineInstaller: $INSTALLER_PATH"
echo "  fixedRuntime: $FIXED_RUNTIME_DIR"