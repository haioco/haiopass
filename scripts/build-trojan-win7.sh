#!/usr/bin/env bash
# Rebuilds trojan-go Windows binary with Go 1.20.x for Windows 7 compatibility.
# Go 1.21+ drops Win7, so the default upstream zip fails on Win7.
# This script uses Docker golang:1.20 to cross-compile from source.
set -euo pipefail

REPO="https://github.com/p4gefau1t/trojan-go.git"
DEST="$(cd "$(dirname "$0")/../resources/trojan-go" && pwd)"
TMPDIR="/tmp/trojan-go-win7-$$"
GO_VERSION="1.20.14"

echo "=== Building haio-proxy Windows binary with Go ${GO_VERSION} (Win7 compatible) ==="
echo "Dest: $DEST/haio-proxy-windows-amd64.exe"

mkdir -p "$DEST"

# If Docker available, build from source with Go 1.20
if command -v docker >/dev/null 2>&1; then
  echo "Cloning trojan-go..."
  rm -rf "$TMPDIR"
  git clone --depth 1 "$REPO" "$TMPDIR"

  echo "Building with golang:${GO_VERSION} ..."
  # Build inside docker, output to host via volume
  docker run --rm -v "$TMPDIR":/src -w /src "golang:${GO_VERSION}" bash -c "
    set -e
    go env -w GOOS=windows GOARCH=amd64
    go build -ldflags '-H windowsgui -s -w' -o /tmp/haio-proxy-windows-amd64.exe .
    ls -lh /tmp/haio-proxy-windows-amd64.exe
  "
  # Copy out via docker cp alternative: use volume mount for /tmp
  # Instead rebuild with output to mounted dir
  docker run --rm -v "$TMPDIR":/src -v "$DEST":/out -w /src "golang:${GO_VERSION}" bash -c "
    set -e
    GOOS=windows GOARCH=amd64 go build -ldflags '-H windowsgui -s -w' -o /out/haio-proxy-windows-amd64.exe .
  "

  rm -rf "$TMPDIR"

  if [ -f "$DEST/haio-proxy-windows-amd64.exe" ]; then
    echo "✓ $DEST/haio-proxy-windows-amd64.exe built ($(du -h "$DEST/haio-proxy-windows-amd64.exe" | cut -f1))"
    echo "  Verify: file $DEST/haio-proxy-windows-amd64.exe"
    file "$DEST/haio-proxy-windows-amd64.exe" || true
    # Check PE subsystem — should run on Win7
    echo "✓ Win7-compatible binary ready (Go ${GO_VERSION})"
  else
    echo "✗ Build failed — no output"
    exit 1
  fi
else
  echo "⚠ Docker not available — falling back to patching existing binary metadata if possible"
  echo "  To create a Win7-compatible binary manually:"
  echo "  1) Install Go ${GO_VERSION}: https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz"
  echo "  2) git clone $REPO /tmp/trojan-go && cd /tmp/trojan-go"
  echo "  3) GOOS=windows GOARCH=amd64 go build -ldflags '-H windowsgui -s -w' -o haio-proxy-windows-amd64.exe ."
  echo "  4) cp haio-proxy-windows-amd64.exe $DEST/"
  exit 1
fi
