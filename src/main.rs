mod messages;

use std::{
    env,
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

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
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use messages::{ClientMsg, ServerMsg};
use serde::Serialize;
use sha1::Sha1;
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

type HmacSha1 = Hmac<Sha1>;

// ---------------------------------------------------------------------------
// Per-user handle stored in AppState
// ---------------------------------------------------------------------------
#[derive(Clone)]
struct UserHandle {
    username: String,
    contacts: Vec<String>,
    tx: mpsc::UnboundedSender<ServerMsg>,
}

// ---------------------------------------------------------------------------
// Application state shared between all connections
// ---------------------------------------------------------------------------
#[derive(Clone)]
struct AppState {
    users: Arc<DashMap<String, UserHandle>>,
    turn_secret: String,
    turn_url: String,
}

// ---------------------------------------------------------------------------
// HTTP API types
// ---------------------------------------------------------------------------
#[derive(Serialize)]
struct TurnCredentials {
    urls: String,
    username: String,
    credential: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
#[tokio::main]
async fn main() {
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

    let turn_secret = env::var("TURN_SECRET")
        .unwrap_or_else(|_| "change-me-to-a-random-string".into());
    let turn_url = env::var("TURN_URL")
        .unwrap_or_else(|_| "turn:161.97.187.145:3478".into());

    let state = Arc::new(AppState {
        users: Arc::new(DashMap::new()),
        turn_secret,
        turn_url,
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET]);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(health))
        .route("/turn-credentials", get(turn_credentials))
        .layer(cors)
        .with_state(state);

    info!("disintea-server listening on {bind_addr}");

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("Server crashed");
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------
async fn health() -> &'static str {
    "ok"
}

async fn turn_credentials(State(state): State<Arc<AppState>>) -> Json<TurnCredentials> {
    let ttl = 86400u64;
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

// ---------------------------------------------------------------------------
// WebSocket upgrade handler
// ---------------------------------------------------------------------------
async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    info!("WS upgrade from {addr}");
    ws.on_upgrade(move |socket| handle_socket(socket, addr, state))
}

// ---------------------------------------------------------------------------
// Per-connection handler
// ---------------------------------------------------------------------------
async fn handle_socket(socket: WebSocket, addr: SocketAddr, state: Arc<AppState>) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (peer_tx, mut peer_rx) = mpsc::unbounded_channel::<ServerMsg>();

    // Task: drain outbound channel -> WebSocket
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

    let mut my_user_id: Option<String> = None;

    'outer: loop {
        tokio::select! {
            maybe_msg = ws_rx.next() => {
                match maybe_msg {
                    None => break 'outer,
                    Some(Err(e)) => { warn!("WS error from {addr}: {e}"); break 'outer; }
                    Some(Ok(msg)) => {
                        let text = match msg {
                            Message::Text(t) => t.to_string(),
                            Message::Close(_) => break 'outer,
                            Message::Ping(_) | Message::Pong(_) => continue,
                            _ => continue,
                        };

                        let client_msg: ClientMsg = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                let _ = peer_tx.send(ServerMsg::Error { reason: format!("bad message: {e}") });
                                continue;
                            }
                        };

                        match client_msg {
                            // --------------------------------------------------------
                            // Register
                            // --------------------------------------------------------
                            ClientMsg::Register { user_id, username, contacts } => {
                                if my_user_id.is_some() {
                                    let _ = peer_tx.send(ServerMsg::Error { reason: "already registered".into() });
                                    continue;
                                }

                                // Mutual presence notifications
                                for cid in &contacts {
                                    if let Some(handle) = state.users.get(cid.as_str()) {
                                        // Tell that contact we are online
                                        let _ = handle.tx.send(ServerMsg::UserOnline {
                                            user_id: user_id.clone(),
                                            username: username.clone(),
                                        });
                                        // Tell us that contact is online
                                        let _ = peer_tx.send(ServerMsg::UserOnline {
                                            user_id: cid.clone(),
                                            username: handle.username.clone(),
                                        });
                                    }
                                }

                                state.users.insert(user_id.clone(), UserHandle {
                                    username: username.clone(),
                                    contacts: contacts.clone(),
                                    tx: peer_tx.clone(),
                                });

                                my_user_id = Some(user_id.clone());
                                let _ = peer_tx.send(ServerMsg::Registered);
                                info!("{addr} registered as '{username}' ({user_id})");
                            }

                            // --------------------------------------------------------
                            // All other messages require registration
                            // --------------------------------------------------------
                            other => {
                                let Some(ref from_id) = my_user_id else {
                                    let _ = peer_tx.send(ServerMsg::Error { reason: "not registered".into() });
                                    continue;
                                };

                                let from_name = state.users
                                    .get(from_id.as_str())
                                    .map(|h| h.username.clone())
                                    .unwrap_or_default();

                                match other {
                                    ClientMsg::Register { .. } => unreachable!(),

                                    ClientMsg::CallOffer { to, sdp } => {
                                        if let Some(h) = state.users.get(&to) {
                                            let _ = h.tx.send(ServerMsg::IncomingCall {
                                                from: from_id.clone(),
                                                from_name,
                                                sdp,
                                            });
                                        }
                                    }

                                    ClientMsg::CallAnswer { to, sdp } => {
                                        if let Some(h) = state.users.get(&to) {
                                            let _ = h.tx.send(ServerMsg::CallAnswered {
                                                from: from_id.clone(),
                                                sdp,
                                            });
                                        }
                                    }

                                    ClientMsg::CallReject { to } => {
                                        if let Some(h) = state.users.get(&to) {
                                            let _ = h.tx.send(ServerMsg::CallRejected {
                                                from: from_id.clone(),
                                            });
                                        }
                                    }

                                    ClientMsg::HangUp { to } => {
                                        if let Some(h) = state.users.get(&to) {
                                            let _ = h.tx.send(ServerMsg::HangUp {
                                                from: from_id.clone(),
                                            });
                                        }
                                    }

                                    ClientMsg::IceCandidate { to, candidate, sdp_mid, sdp_m_line_index } => {
                                        if let Some(h) = state.users.get(&to) {
                                            let _ = h.tx.send(ServerMsg::IceCandidate {
                                                from: from_id.clone(),
                                                candidate,
                                                sdp_mid,
                                                sdp_m_line_index,
                                            });
                                        }
                                    }

                                    ClientMsg::ChatMessage { to, text, msg_id } => {
                                        let timestamp = SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap()
                                            .as_millis() as u64;

                                        if let Some(h) = state.users.get(&to) {
                                            let _ = h.tx.send(ServerMsg::IncomingMessage {
                                                from: from_id.clone(),
                                                from_name,
                                                text,
                                                msg_id,
                                                timestamp,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Cleanup
    if let Some(user_id) = my_user_id {
        let contacts = state.users
            .get(&user_id)
            .map(|h| h.contacts.clone())
            .unwrap_or_default();

        state.users.remove(&user_id);

        for cid in &contacts {
            if let Some(h) = state.users.get(cid.as_str()) {
                let _ = h.tx.send(ServerMsg::UserOffline { user_id: user_id.clone() });
            }
        }

        info!("{addr} disconnected (user_id={user_id})");
    }

    send_task.abort();
}
