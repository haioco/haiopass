#!/usr/bin/env bash
# Generate the Tauri v2 signed updater manifest (latest.json) and upload it
# to the GitHub release identified by $TAG.
#
# Required env:
#   TAG                            e.g. v1.0.10
#   GITHUB_TOKEN                   repo token with contents:write
#   TAURI_SIGNING_PRIVATE_KEY      minisign private key (Tauri updater)
#   TAURI_SIGNING_PRIVATE_KEY_PASSWORD
#
# Usage: bash scripts/generate-updater-json.sh
set -euo pipefail

TAG="${TAG:?TAG env var required (e.g. v1.0.10)}"
VERSION="${TAG#v}"
REPO="${GITHUB_REPOSITORY:-haioco/haiopass}"
BASE="https://github.com/${REPO}/releases/download/${TAG}"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

echo ">> downloading release assets for ${TAG}"
declare -a WANTS=(
  "HaioBypass_${VERSION}_x64-setup.exe|windows-x86_64"
  "HaioBypass_${VERSION}_amd64.AppImage|linux-x86_64"
  "HaioBypass_aarch64.app.tar.gz|darwin-aarch64"
  "HaioBypass_x64.app.tar.gz|darwin-x86_64"
)

PLATFORMS=""
for entry in "${WANTS[@]}"; do
  asset="${entry%%|*}"
  plat="${entry##*|}"
  url="${BASE}/${asset}"
  # gh release download works on draft releases (public curl URLs do not)
  if ! gh release download "$TAG" --repo "$REPO" --dir . --pattern "$asset" --clobber; then
    echo "!! skip ${plat}: asset not found (${asset})" >&2
    continue
  fi
  echo ">> signing ${asset}"
  npx --yes @tauri-apps/cli@v2 signer sign \
    -k "${TAURI_SIGNING_PRIVATE_KEY}" \
    -p "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" \
    -- "${asset}" > /dev/null
  sig=$(tr -d '\n' < "${asset}.sig")
  PLATFORMS=$(printf '%s\n    "%s": {"signature": "%s", "url": "%s"},' \
    "$PLATFORMS" "$plat" "$sig" "$url")
done

[ -n "$PLATFORMS" ] || { echo "!! no platforms to publish" >&2; exit 1; }
# strip trailing comma of last entry
PLATFORMS="${PLATFORMS%,}"

NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > latest.json <<EOF
{
  "version": "${VERSION}",
  "pub_date": "${NOW}",
  "platforms": {${PLATFORMS}
  }
}
EOF

echo ">> latest.json:"
cat latest.json

echo ">> uploading latest.json to release ${TAG}"
gh release upload "${TAG}" latest.json --repo "${REPO}" --clobber
echo ">> done"
