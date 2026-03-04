#!/usr/bin/env bash
# setup-coturn.sh — Install and configure coturn TURN server on Ubuntu/Debian
# Run this on your Contabo VPS as root
set -euo pipefail

DOMAIN="${DOMAIN:?Set DOMAIN=your.domain.com}"
AUTH_SECRET="${TURN_SECRET:?Set TURN_SECRET=your-random-secret}"

echo "==> Installing coturn..."
apt-get install -y coturn

echo "==> Enabling coturn daemon..."
sed -i 's/#TURNSERVER_ENABLED=1/TURNSERVER_ENABLED=1/' /etc/default/coturn

echo "==> Writing /etc/turnserver.conf..."
cat > /etc/turnserver.conf <<EOF
# Dismony coturn configuration
listening-port=3478
tls-listening-port=5349

realm=$DOMAIN
server-name=$DOMAIN

# Time-limited HMAC credentials (no DB needed)
use-auth-secret
static-auth-secret=$AUTH_SECRET

# TLS — point to Let's Encrypt certs
cert=/etc/letsencrypt/live/$DOMAIN/fullchain.pem
pkey=/etc/letsencrypt/live/$DOMAIN/privkey.pem

# Security
no-multicast-peers
denied-peer-ip=0.0.0.0-0.255.255.255
denied-peer-ip=127.0.0.0-127.255.255.255
denied-peer-ip=10.0.0.0-10.255.255.255
denied-peer-ip=172.16.0.0-172.31.255.255
denied-peer-ip=192.168.0.0-192.168.255.255

log-file=/var/log/turnserver.log
EOF

echo "==> Starting coturn..."
systemctl enable coturn
systemctl restart coturn
systemctl is-active --quiet coturn && echo "coturn is running OK"
