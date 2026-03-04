#!/usr/bin/env bash
# deploy.sh — Build + deploy dismony-server to your Contabo VPS
# Usage: VPS_HOST=root@1.2.3.4 ./deploy.sh
set -euo pipefail

VPS_HOST="${VPS_HOST:?Set VPS_HOST=user@your-vps-ip}"
BINARY="dismony-server"
REMOTE_BIN="/usr/local/bin/$BINARY"
SERVICE="$BINARY.service"

echo "==> Building release binary..."
cargo build --release

echo "==> Copying binary to $VPS_HOST:$REMOTE_BIN..."
scp "target/release/$BINARY" "$VPS_HOST:$REMOTE_BIN"
ssh "$VPS_HOST" "chmod +x $REMOTE_BIN"

echo "==> Installing systemd service..."
scp "deploy/$SERVICE" "$VPS_HOST:/etc/systemd/system/$SERVICE"
ssh "$VPS_HOST" "systemctl daemon-reload && systemctl enable $SERVICE && systemctl restart $SERVICE"

echo "==> Waiting for service to start..."
sleep 2
ssh "$VPS_HOST" "systemctl is-active --quiet $SERVICE && echo 'Service is running OK' || (journalctl -u $SERVICE -n 20 && exit 1)"

echo "==> Testing health endpoint..."
ssh "$VPS_HOST" "curl -sf http://localhost:8080/health && echo ' <- health OK'"

echo "==> Deploy complete!"
echo "    Remember to:"
echo "    1. Create system user:  adduser --system --no-create-home dismony"
echo "    2. Install nginx conf:  cp deploy/nginx-dismony.conf /etc/nginx/sites-available/dismony"
echo "    3. Enable nginx site:   ln -s /etc/nginx/sites-available/dismony /etc/nginx/sites-enabled/"
echo "    4. Obtain TLS cert:     certbot --nginx -d your.domain.com"
echo "    5. Edit BIND_ADDR and MAX_PEERS_PER_ROOM in /etc/systemd/system/$SERVICE"
