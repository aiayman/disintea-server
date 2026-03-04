#!/usr/bin/env bash
# deploy-web.sh — build the React frontend and copy it into the server's web-dist/
# Run from the root of either repo, or from anywhere after setting CLIENT_DIR / SERVER_DIR.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$SCRIPT_DIR/.."
CLIENT_DIR="${CLIENT_DIR:-$SERVER_DIR/../disintea-client}"

if [[ ! -d "$CLIENT_DIR" ]]; then
  echo "[!] Client directory not found: $CLIENT_DIR"
  echo "    Set CLIENT_DIR env var to the path of disintea-client."
  exit 1
fi

echo "[*] Building React frontend ..."
cd "$CLIENT_DIR"
pnpm install --frozen-lockfile
pnpm build:web

echo "[*] Copying build output to $SERVER_DIR/web-dist/ ..."
mkdir -p "$SERVER_DIR/web-dist"
rsync -av --delete "$CLIENT_DIR/dist/" "$SERVER_DIR/web-dist/"

echo "[+] web-dist/ updated. Re-run 'docker compose restart nginx' or deploy/start.sh to apply."
