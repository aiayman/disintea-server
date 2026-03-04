#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/.."

# Ensure .env exists
if [[ ! -f "$ROOT/.env" ]]; then
  echo "[!] .env not found. Copy .env.example and fill in values:"
  echo "    cp $ROOT/.env.example $ROOT/.env && nano $ROOT/.env"
  exit 1
fi

# Ensure certs exist
if [[ ! -f "$ROOT/certs/cert.pem" || ! -f "$ROOT/certs/key.pem" ]]; then
  echo "[*] Certs not found, generating self-signed ..."
  bash "$SCRIPT_DIR/gen-self-signed.sh"
fi

# Ensure required directories exist
mkdir -p "$ROOT/logs/nginx" "$ROOT/web-dist"

# Warn if web-dist is empty
if [[ -z "$(ls -A "$ROOT/web-dist" 2>/dev/null)" ]]; then
  echo "[!] WARNING: web-dist/ is empty. Run scripts/deploy-web.sh first to build the frontend."
fi

cd "$ROOT"
echo "[*] Building and starting containers ..."
docker compose up --build -d

echo ""
docker compose ps
echo ""
echo "[+] Disintea running at https://161.97.187.145:8443"
