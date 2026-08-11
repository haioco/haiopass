#!/usr/bin/env bash
# Downloads upstream trojan-go binaries and rebrands them as haio-proxy-*
# (neutral name to reduce AV false-positives on bundled proxy binary).
# Compatible with bash 3 (macOS default) — no associative arrays.
set -euo pipefail

RELEASES="https://github.com/p4gefau1t/trojan-go/releases/latest/download"
DEST="$(cd "$(dirname "$0")/../resources/trojan-go" && pwd)"
mkdir -p "$DEST"

# Pairs of <output_filename> <upstream-asset-zip>
PAIRS=(
  "haio-proxy-windows-amd64.exe trojan-go-windows-amd64.zip"
  "haio-proxy-linux-amd64       trojan-go-linux-amd64.zip"
  "haio-proxy-darwin-amd64     trojan-go-darwin-amd64.zip"
  "haio-proxy-darwin-arm64     trojan-go-darwin-arm64.zip"
)

for pair in "${PAIRS[@]}"; do
  binary_name="${pair%% *}"
  asset="${pair##* }"
  url="$RELEASES/$asset"
  zip_path="/tmp/haio-$asset"
  dest_path="$DEST/$binary_name"

  if [ -f "$dest_path" ]; then
    echo "✓ $dest_path already exists, skipping"
    continue
  fi

  echo "Downloading $asset ..."
  curl -fsSL -o "$zip_path" "$url"

  echo "Extracting $asset to $DEST ..."
  unzip -o "$zip_path" -d "$DEST" 2>/dev/null || true

  # The zip extracts as "trojan-go"; rename to our neutral name.
  if [ -f "$DEST/trojan-go" ]; then
    if [ ! -f "$dest_path" ]; then
      mv "$DEST/trojan-go" "$dest_path"
    else
      rm "$DEST/trojan-go"
    fi
  fi

  if [ -f "$dest_path" ]; then
    if [[ "$dest_path" != *.exe ]]; then
      chmod +x "$dest_path"
    fi
    echo "✓ $dest_path saved ($(du -h "$dest_path" | cut -f1))"
  else
    echo "⚠ Binary not found for $binary_name, check if zip contents are correct"
    unzip -l "$zip_path" | head -10
  fi

  rm -f "$zip_path"
done

echo ""
echo "All haio-proxy binaries:"
ls -lh "$DEST"
