# disintea-server

WebSocket signaling server for the Disintea voice/screen-sharing app.

## Stack
- Rust + axum 0.8 (WebSocket upgrade, HTTP)
- tokio full (async runtime)
- dashmap (lock-free concurrent room state)
- serde_json (message serialization)

## Protocol

All messages are JSON with a `type` field.

**Client → Server**
| type | fields | description |
|---|---|---|
| `join` | `room_code`, `peer_id` | Join/create a room |
| `offer` | `sdp`, `to?` | Relay WebRTC SDP offer |
| `answer` | `sdp`, `to?` | Relay WebRTC SDP answer |
| `ice_candidate` | `candidate`, `sdp_mid?`, `sdp_m_line_index?`, `to?` | Relay ICE candidate |
| `leave` | — | Graceful disconnect |

**Server → Client**
| type | fields | description |
|---|---|---|
| `joined` | `peer_count`, `existing_peers` | Confirmation after `join` |
| `peer_joined` | `peer_id` | New peer entered the room |
| `peer_left` | `peer_id` | Peer disconnected |
| `offer` | `sdp`, `from` | Relayed offer |
| `answer` | `sdp`, `from` | Relayed answer |
| `ice_candidate` | `candidate`, `sdp_mid?`, `sdp_m_line_index?`, `from` | Relayed ICE |
| `room_full` | `max` | Room at capacity |
| `error` | `reason` | Server error |

## Running locally

```bash
cp .env.example .env
cargo run
# Server starts on 0.0.0.0:8080
# Health check: curl http://localhost:8080/health
```

## Configuration (`.env`)

| Variable | Default | Description |
|---|---|---|
| `BIND_ADDR` | `0.0.0.0:8080` | Listen address |
| `MAX_PEERS_PER_ROOM` | `2` | Max peers per room (1-on-1 default) |
| `RUST_LOG` | `disintea_server=info` | Log level |
| `TURN_SECRET` | *(required for client)* | Shared secret with coturn |

## Deploying to VPS

```bash
# Set your VPS IP/user
VPS_HOST=root@YOUR_VPS_IP ./deploy/deploy.sh

# Install coturn TURN server
DOMAIN=your.domain.com TURN_SECRET=your-secret ./deploy/setup-coturn.sh

# Install nginx reverse proxy (WSS termination)
cp deploy/nginx-disintea.conf /etc/nginx/sites-available/disintea
ln -s /etc/nginx/sites-available/disintea /etc/nginx/sites-enabled/
certbot --nginx -d your.domain.com
nginx -t && systemctl reload nginx
```
