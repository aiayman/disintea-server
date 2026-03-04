#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/.."

mkdir -p "$ROOT/certs" "$ROOT/logs/nginx" "$ROOT/web-dist"

echo "[*] Generating self-signed TLS certificate for 161.97.187.145 ..."
openssl req -x509 \
  -newkey rsa:4096 \
  -keyout "$ROOT/certs/key.pem" \
  -out    "$ROOT/certs/cert.pem" \
  -days   365 \
  -nodes \
  -subj "/CN=161.97.187.145"

chmod 600 "$ROOT/certs/key.pem"
echo "[+] Certs written to $ROOT/certs/"
