mod messages;
mod rooms;

use std::{env, net::SocketAddr, sync::Arc, time::{SystemTime, UNIX_EPOCH}};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::Method,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use messages::{ClientMsg, ServerMsg};
use rooms::{new_room_map, PeerHandle, RoomMap};
use serde::Serialize;
use sha1::Sha1;
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

type HmacSha1 = Hmac<Sha1>;

#[derive(Clone)]
struct AppState {
    rooms: RoomMap,
    max_peers: usize,
    turn_secret: String,
    turn_url: String,
}

#[derive(Serialize)]
struct TurnCredentials {
    urls: String,
    username: String,
    credential: String,
}

#[tokio::main]
async fn main() {
    // Load .env if present, ignore if missing
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "disintea_server=info,tower_http=warn".into()),
        )
        .init();

    let bind_addr: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .expect("BIND_ADDR must be a valid socket address");

    let max_peers: usize = env::var("MAX_PEERS_PER_ROOM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);

    let turn_secret = env::var("TURN_SECRET")
        .unwrap_or_else(|_| "change-me-to-a-random-string".into());
    let turn_url = env::var("TURN_URL")
        .unwrap_or_else(|_| "turn:161.97.187.145:3478".into());

    let state = AppState {
        rooms: new_room_map(),
        max_peers,
        turn_secret,
        turn_url,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET]);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(health))
        .route("/turn-credentials", get(turn_credentials))
        .layer(cors)
        .with_state(Arc::new(state));

    info!("disintea-server listening on {bind_addr} (max_peers_per_room={max_peers})");

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("Server crashed");
}

async fn health() -> &'static str {
    "ok"
}

/// Return short-lived coturn HMAC credentials for browser clients.
async fn turn_credentials(State(state): State<Arc<AppState>>) -> Json<TurnCredentials> {
    let ttl = 86400u64; // 24 hours
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + ttl;

    let username = format!("{timestamp}:disintea");

    let mut mac = HmacSha1::new_from_slice(state.turn_secret.as_bytes())
        .expect("HMAC init failed");
    mac.update(username.as_bytes());
    let credential = BASE64.encode(mac.finalize().into_bytes());

    Json(TurnCredentials {
        urls: state.turn_url.clone(),
        username,
        credential,
    })
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    info!("WS upgrade from {addr}");
    ws.on_upgrade(move |socket| handle_socket(socket, addr, state))
}

async fn handle_socket(socket: WebSocket, addr: SocketAddr, state: Arc<AppState>) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Per-peer outbound channel
    let (peer_tx, mut peer_rx) = mpsc::unbounded_channel::<ServerMsg>();

    // Spawn task that drains the outbound channel → WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = peer_rx.recv().await {
            match serde_json::to_string(&msg) {
                Ok(text) => {
                    if ws_tx.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => error!("serialize error: {e}"),
            }
        }
    });

    // State for this connection
    let mut joined_room: Option<String> = None;
    let mut my_peer_id: Option<String> = None;

    // Helper to send a message to this peer via the channel
    let send = |msg: ServerMsg, tx: &mpsc::UnboundedSender<ServerMsg>| {
        let _ = tx.send(msg);
    };

    loop {
        tokio::select! {
            maybe_msg = ws_rx.next() => {
                match maybe_msg {
                    None => break, // client disconnected
                    Some(Err(e)) => { warn!("WS error from {addr}: {e}"); break; }
                    Some(Ok(msg)) => {
                        let text = match msg {
                            Message::Text(t) => t.to_string(),
                            Message::Close(_) => break,
                            Message::Ping(_) | Message::Pong(_) => continue,
                            _ => continue,
                        };

                        let client_msg: ClientMsg = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                send(ServerMsg::Error { reason: format!("bad message: {e}") }, &peer_tx);
                                continue;
                            }
                        };

                        match client_msg {
                            ClientMsg::Join { room_code, peer_id } => {
                                if joined_room.is_some() {
                                    send(ServerMsg::Error { reason: "already joined".into() }, &peer_tx);
                                    continue;
                                }

                                let mut room = state.rooms.entry(room_code.clone()).or_default();

                                if room.len() >= state.max_peers {
                                    send(ServerMsg::RoomFull { max: state.max_peers }, &peer_tx);
                                    continue;
                                }

                                let existing_peers: Vec<String> =
                                    room.iter().map(|p| p.peer_id.clone()).collect();

                                // Notify existing peers
                                for p in room.iter() {
                                    let _ = p.tx.send(ServerMsg::PeerJoined { peer_id: peer_id.clone() });
                                }

                                room.push(PeerHandle { peer_id: peer_id.clone(), tx: peer_tx.clone() });

                                drop(room); // release dashmap lock

                                let peer_count = state.rooms.get(&room_code).map(|r| r.len()).unwrap_or(1);
                                send(ServerMsg::Joined { peer_count, existing_peers }, &peer_tx);

                                joined_room = Some(room_code);
                                my_peer_id = Some(peer_id);
                                info!("{addr} joined room (peer_id={:?})", my_peer_id);
                            }

                            ClientMsg::Leave => break,

                            // ------ relay messages ------
                            ClientMsg::Offer { sdp, to } => {
                                relay(&state, &joined_room, &my_peer_id, to.as_deref(), |from| {
                                    ServerMsg::Offer { sdp: sdp.clone(), from }
                                });
                            }
                            ClientMsg::Answer { sdp, to } => {
                                relay(&state, &joined_room, &my_peer_id, to.as_deref(), |from| {
                                    ServerMsg::Answer { sdp: sdp.clone(), from }
                                });
                            }
                            ClientMsg::IceCandidate { candidate, sdp_mid, sdp_m_line_index, to } => {
                                relay(&state, &joined_room, &my_peer_id, to.as_deref(), |from| {
                                    ServerMsg::IceCandidate {
                                        candidate: candidate.clone(),
                                        sdp_mid: sdp_mid.clone(),
                                        sdp_m_line_index,
                                        from,
                                    }
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Cleanup: remove this peer from the room
    if let (Some(room_code), Some(peer_id)) = (joined_room, my_peer_id) {
        if let Some(mut room) = state.rooms.get_mut(&room_code) {
            room.retain(|p| p.peer_id != peer_id);
            let is_empty = room.is_empty();
            let remaining: Vec<_> = room.iter().cloned().collect();
            drop(room);

            if is_empty {
                state.rooms.remove(&room_code);
                info!("Room '{room_code}' removed (empty)");
            } else {
                for p in &remaining {
                    let _ = p.tx.send(ServerMsg::PeerLeft { peer_id: peer_id.clone() });
                }
            }
        }
        info!("{addr} disconnected (peer_id={peer_id})");
    }

    send_task.abort();
}

/// Relay a message to either a specific peer (`to`) or all peers except the sender
fn relay<F>(
    state: &AppState,
    room_code: &Option<String>,
    sender_id: &Option<String>,
    to: Option<&str>,
    make_msg: F,
) where
    F: Fn(String) -> ServerMsg,
{
    let Some(ref code) = room_code else { return };
    let Some(ref from_id) = sender_id else { return };
    let Some(room) = state.rooms.get(code) else { return };

    for peer in room.iter() {
        if peer.peer_id == *from_id {
            continue; // don't echo back to sender
        }
        if let Some(target) = to {
            if peer.peer_id != target {
                continue;
            }
        }
        let _ = peer.tx.send(make_msg(from_id.clone()));
    }
}
