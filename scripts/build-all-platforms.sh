#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-.}")/.." && pwd)"
cd "$REPO_ROOT"

echo "=== Cutest Disk Tree – Build and prepare release (this OS) ==="
echo ""

if [[ ! -f ".tauri-private-key" ]] || [[ ! -f ".tauri-public-key" ]]; then
  echo "Error: .tauri-private-key and .tauri-public-key must exist in the repo root."
  exit 1
fi

echo "Before continuing, please confirm:"
echo "  1. You have bumped the version in src-tauri/tauri.conf.json (and package.json if you use it)."
echo "  2. You have run 'npm install' and dependencies are up to date."
echo "  3. This is the Odin94/cutest-disk-tree repo."
echo ""
read -p "Have you done all of the above? (y/N) " -n 1 -r
echo
if [[ ! "${REPLY,,}" =~ ^y ]]; then
  echo "Stopping. Complete the steps above and run this script again."
  exit 1
fi

PRIVATE_KEY_B64="$(cat .tauri-private-key)"
TAURI_PRIVATE_KEY="$(echo "$PRIVATE_KEY_B64" | base64 -d 2>/dev/null || echo "$PRIVATE_KEY_B64" | base64 -D 2>/dev/null)"

echo "Running npm install..."
npm install

echo "Generating icons if needed..."
node scripts/gen-icon.cjs 2>/dev/null || true

echo "Building signed Tauri app (this may take a while)..."
export TAURI_SIGNING_PRIVATE_KEY="$TAURI_PRIVATE_KEY"
npm run tauri build

echo ""
echo "Build finished. Running release script to generate latest.json and manifest..."
echo ""
RELEASE_SKIP_CONFIRM=1 exec "$REPO_ROOT/scripts/release.sh"
