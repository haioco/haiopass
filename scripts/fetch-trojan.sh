#!/usr/bin/env bash
set -euo pipefail

RELEASES="https://github.com/p4gefau1t/trojan-go/releases/latest/download"
DEST="$(cd "$(dirname "$0")/../resources/trojan-go" && pwd)"
mkdir -p "$DEST"

declare -A TARGETS
TARGETS=(
  ["trojan-go-windows-amd64"]="trojan-go-windows-amd64.zip"
  ["trojan-go-linux-amd64"]="trojan-go-linux-amd64.zip"
  ["trojan-go-darwin-amd64"]="trojan-go-darwin-amd64.zip"
  ["trojan-go-darwin-arm64"]="trojan-go-darwin-arm64.zip"
)

for binary_name in "${!TARGETS[@]}"; do
  asset="${TARGETS[$binary_name]}"
  url="$RELEASES/$asset"
  zip_path="/tmp/haio-$asset"

  if [ "$binary_name" = "trojan-go-windows-amd64" ]; then
    dest_path="$DEST/trojan-go-windows-amd64.exe"
  else
    dest_path="$DEST/$binary_name"
  fi

  if [ -f "$dest_path" ]; then
    echo "✓ $dest_path already exists, skipping"
    continue
  fi

  echo "Downloading $asset ..."
  curl -fsSL -o "$zip_path" "$url"

  echo "Extracting $asset to $DEST ..."
  cd "$DEST"
  unzip -o "$zip_path" 2>/dev/null || true
  cd - >/dev/null

  # The zip might extract the binary with a different name or path
  # Find any extracted file that looks like trojan-go
  find /tmp -maxdepth 2 -name "trojan-go*" -newer "$zip_path" -exec cp {} "$dest_path" \; 2>/dev/null || true

  # Also check current dir and DEST for extracted files
  if [ -f "$DEST/trojan-go" ]; then
    if [[ "$binary_name" == *windows* ]]; then
      mv "$DEST/trojan-go" "$DEST/trojan-go-windows-amd64.exe"
    else
      # Deduplicate - keep the correct name
      if [ ! -f "$dest_path" ]; then
        cp "$DEST/trojan-go" "$dest_path"
      fi
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
echo "All trojan-go binaries:"
ls -lh "$DEST"
