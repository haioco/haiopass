#!/usr/bin/env bash
# Downloads WebView2 Runtime 109 (last version with Windows 7 support)
# for offline embedding in the Windows 7 build.
# Microsoft offline installer: MicrosoftEdgeWebView2RuntimeInstallerX64.exe
# Fixed Runtime can also be extracted from https://www.nuget.org/packages/Microsoft.Web.WebView2
set -euo pipefail

DEST_DIR="$(cd "$(dirname "$0")/../resources/webview2" && pwd 2>/dev/null || echo "$(pwd)/resources/webview2")"
mkdir -p "$DEST_DIR"

# Version 109.0.1518.140 — last Win7/8/8.1 compatible
WV2_VERSION="109.0.1518.140"
INSTALLER_URL="https://msedge.sf.dl.delivery.mp.microsoft.com/filestreamingservice/files/c1d53a34-a40a-4f6f-9b8f-2e5b4c2e8e9d/MicrosoftEdgeWebView2RuntimeInstallerX64.exe"

# NuGet fixed-runtime package (alternative, contains unpacked runtime folder)
NUGET_URL="https://www.nuget.org/api/v2/package/Microsoft.Web.WebView2/${WV2_VERSION}"
NUGET_ZIP="/tmp/webview2.${WV2_VERSION}.nupkg"

INSTALLER_PATH="$DEST_DIR/MicrosoftEdgeWebView2RuntimeInstallerX64_109.exe"
FIXED_RUNTIME_DIR="$DEST_DIR/WebView2FixedRuntime109"

if [ -f "$INSTALLER_PATH" ] && [ -s "$INSTALLER_PATH" ]; then
  echo "✓ $INSTALLER_PATH already exists ($(du -h "$INSTALLER_PATH" | cut -f1))"
else
  echo "Downloading WebView2 Runtime ${WV2_VERSION} offline installer..."
  echo "URL: $INSTALLER_URL"
  # Note: Microsoft CDN URL rotates; fallback to nuget if 404
  if ! curl -fsSL -o "$INSTALLER_PATH" "$INSTALLER_URL"; then
    echo "⚠ Direct CDN URL failed, trying NuGet package..."
    curl -fsSL -o "$NUGET_ZIP" "$NUGET_URL"
    # Extract fixed runtime from nuget if needed — user can manually place installer
    echo "⚠ Downloaded NuGet package to $NUGET_ZIP — extract Fixed Runtime manually"
    echo "   unzip -q \"$NUGET_ZIP\" 'build/native/x64/*' -d /tmp/wv2 && cp -r /tmp/wv2/build/native/x64 \"$FIXED_RUNTIME_DIR\""
    # Don't fail — CI can use fixedRuntime path instead
    rm -f "$NUGET_ZIP" 2>/dev/null || true
  else
    echo "✓ $INSTALLER_PATH saved ($(du -h "$INSTALLER_PATH" | cut -f1))"
  fi
fi

# Also try to prepare Fixed Runtime folder from nuget for Tauri fixedRuntime mode
if [ ! -d "$FIXED_RUNTIME_DIR" ] || [ -z "$(ls -A "$FIXED_RUNTIME_DIR" 2>/dev/null)" ]; then
  echo "Preparing Fixed Runtime folder (for tauri fixedRuntime mode)..."
  NUGET_TMP="/tmp/wv2_nuget_${WV2_VERSION}.nupkg"
  if curl -fsSL -o "$NUGET_TMP" "$NUGET_URL" 2>/dev/null; then
    rm -rf /tmp/wv2_extract
    mkdir -p /tmp/wv2_extract
    unzip -q "$NUGET_TMP" -d /tmp/wv2_extract 2>/dev/null || true
    # NuGet contains WebView2Loader.dll etc under build/native - copy what exists
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
ls -lh "$DEST_DIR" 2>/dev/null || echo " (empty, manual download required — see https://developer.microsoft.com/en-us/microsoft-edge/webview2/#download-section )"
echo ""
echo "Tauri Win7 overlay should point to:"
echo "  offlineInstaller: $INSTALLER_PATH"
echo "  fixedRuntime: $FIXED_RUNTIME_DIR"
