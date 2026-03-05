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
use messages::{ClientMsg, ContactInfo, HistoryMessage, ServerMsg};
use serde::Serialize;
use sha1::Sha1;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

type HmacSha1 = Hmac<Sha1>;

// ─────────────────────────────────────────────────────────────────────────────
// In-memory session handle
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Clone)]
struct UserHandle {
    username: String,
    tx: mpsc::UnboundedSender<ServerMsg>,
}

// ─────────────────────────────────────────────────────────────────────────────
// App state
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Clone)]
struct AppState {
    online: Arc<DashMap<String, UserHandle>>,
    db: Pool<Sqlite>,
    turn_secret: String,
    turn_url: String,
}

#[derive(Serialize)]
struct TurnCredentials {
    urls: String,
    username: String,
    credential: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "disintea_server=info,tower_http=warn".into()),
        )
        .init();

    let db_path = env::var("DB_PATH").unwrap_or_else(|_| "/data/disintea.db".into());

    let db = SqlitePoolOptions::new()
        .max_connections(8)
        .connect(&format!("sqlite:{}?mode=rwc", db_path))
        .await
        .expect("Failed to open SQLite database");

    // Create schema
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id       TEXT PRIMARY KEY,
            username TEXT NOT NULL,
            last_seen INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&db)
    .await
    .expect("Failed to create users table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS contacts (
            owner_id   TEXT NOT NULL,
            contact_id TEXT NOT NULL,
            added_at   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (owner_id, contact_id)
        )",
    )
    .execute(&db)
    .await
    .expect("Failed to create contacts table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS messages (
            id        TEXT PRIMARY KEY,
            from_id   TEXT NOT NULL,
            to_id     TEXT NOT NULL,
            text      TEXT NOT NULL,
            timestamp INTEGER NOT NULL
        )",
    )
    .execute(&db)
    .await
    .expect("Failed to create messages table");

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_msg_pair ON messages(from_id, to_id, timestamp)")
        .execute(&db)
        .await
        .ok();

    let bind_addr: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .expect("BIND_ADDR must be a valid socket address");

    let turn_secret = env::var("TURN_SECRET")
        .unwrap_or_else(|_| "change-me-to-a-random-string".into());
    let turn_url =
        env::var("TURN_URL").unwrap_or_else(|_| "turn:161.97.187.145:3478".into());

    let state = Arc::new(AppState {
        online: Arc::new(DashMap::new()),
        db,
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

// ─────────────────────────────────────────────────────────────────────────────
// HTTP handlers
// ─────────────────────────────────────────────────────────────────────────────
async fn health() -> &'static str {
    "ok"
}

async fn turn_credentials(State(state): State<Arc<AppState>>) -> Json<TurnCredentials> {
    let ttl = 86400u64;
    let timestamp = now_secs() + ttl;
    let username = format!("{timestamp}:disintea");

    let mut mac = HmacSha1::new_from_slice(state.turn_secret.as_bytes()).expect("HMAC init");
    mac.update(username.as_bytes());
    let credential = BASE64.encode(mac.finalize().into_bytes());

    Json(TurnCredentials {
        urls: state.turn_url.clone(),
        username,
        credential,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// WebSocket
// ─────────────────────────────────────────────────────────────────────────────
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
    let (peer_tx, mut peer_rx) = mpsc::unbounded_channel::<ServerMsg>();

    // Drain outbound channel → WebSocket
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
                            // ────────────────────────────────────────────────
                            // Register
                            // ────────────────────────────────────────────────
                            ClientMsg::Register { user_id, username } => {
                                if my_user_id.is_some() {
                                    let _ = peer_tx.send(ServerMsg::Error { reason: "already registered".into() });
                                    continue;
                                }

                                // Upsert user in DB
                                if let Err(e) = sqlx::query(
                                    "INSERT INTO users(id, username, last_seen)
                                     VALUES(?, ?, ?)
                                     ON CONFLICT(id) DO UPDATE SET username=excluded.username, last_seen=excluded.last_seen"
                                )
                                .bind(&user_id)
                                .bind(&username)
                                .bind(now_secs() as i64)
                                .execute(&state.db)
                                .await {
                                    error!("DB upsert user: {e}");
                                }

                                // Load contacts from DB
                                let contact_rows = sqlx::query_as::<_, (String, String)>(
                                    "SELECT u.id, u.username
                                     FROM contacts c
                                     JOIN users u ON u.id = c.contact_id
                                     WHERE c.owner_id = ?"
                                )
                                .bind(&user_id)
                                .fetch_all(&state.db)
                                .await
                                .unwrap_or_default();

                                // Also collect users who have us in THEIR contacts
                                // (bidirectional presence: they need to know we came online)
                                let reverse_online = sqlx::query_as::<_, (String,)>(
                                    "SELECT owner_id FROM contacts
                                     WHERE contact_id = ?
                                     AND owner_id != ?"
                                )
                                .bind(&user_id)
                                .bind(&user_id)
                                .fetch_all(&state.db)
                                .await
                                .unwrap_or_default();

                                // Insert into the online map BEFORE building the contact list.
                                // This closes the race window where two clients connect
                                // simultaneously and neither sees the other as online.
                                state.online.insert(user_id.clone(), UserHandle {
                                    username: username.clone(),
                                    tx: peer_tx.clone(),
                                });
                                my_user_id = Some(user_id.clone());

                                // Build contact list and do presence exchange.
                                // Because we inserted above, any concurrently-registering
                                // contact whose insert already ran will be visible here.
                                let mut contact_list = Vec::new();
                                for (cid, cname) in &contact_rows {
                                    let online = state.online.contains_key(cid.as_str());
                                    if online {
                                        // Notify that contact that we came online
                                        if let Some(h) = state.online.get(cid.as_str()) {
                                            let _ = h.tx.send(ServerMsg::UserOnline {
                                                user_id: user_id.clone(),
                                                username: username.clone(),
                                            });
                                        }
                                    }
                                    contact_list.push(ContactInfo {
                                        user_id: cid.clone(),
                                        username: cname.clone(),
                                        online,
                                    });
                                }

                                // Notify reverse watchers that we came online
                                for (oid,) in &reverse_online {
                                    // Skip contacts already notified in the forward loop above
                                    if let Some(h) = state.online.get(oid.as_str()) {
                                        let _ = h.tx.send(ServerMsg::UserOnline {
                                            user_id: user_id.clone(),
                                            username: username.clone(),
                                        });
                                    }
                                }

                                // Ack + send full contact list
                                let _ = peer_tx.send(ServerMsg::Registered);
                                let _ = peer_tx.send(ServerMsg::ContactList { contacts: contact_list });

                                info!("{addr} registered as '{username}' ({user_id})");
                            }

                            // ────────────────────────────────────────────────
                            // All other messages require registration
                            // ────────────────────────────────────────────────
                            other => {
                                let Some(ref from_id) = my_user_id else {
                                    let _ = peer_tx.send(ServerMsg::Error { reason: "not registered".into() });
                                    continue;
                                };

                                let from_name = state.online
                                    .get(from_id.as_str())
                                    .map(|h| h.username.clone())
                                    .unwrap_or_default();

                                match other {
                                    ClientMsg::Register { .. } => unreachable!(),

                                    // ── AddContact ───────────────────────────
                                    ClientMsg::AddContact { contact_id } => {
                                        if contact_id == *from_id {
                                            let _ = peer_tx.send(ServerMsg::Error { reason: "cannot add yourself".into() });
                                            continue;
                                        }

                                        // Fetch contact's username from DB
                                        let contact_user = sqlx::query_as::<_, (String,)>(
                                            "SELECT username FROM users WHERE id = ?"
                                        )
                                        .bind(&contact_id)
                                        .fetch_optional(&state.db)
                                        .await
                                        .unwrap_or(None);

                                        let contact_username = match contact_user {
                                            Some((u,)) => u,
                                            None => {
                                                // Try online map
                                                match state.online.get(contact_id.as_str()) {
                                                    Some(h) => h.username.clone(),
                                                    None => {
                                                        let _ = peer_tx.send(ServerMsg::Error {
                                                            reason: "user not found — they need to connect at least once".into()
                                                        });
                                                        continue;
                                                    }
                                                }
                                            }
                                        };

                                        // Insert contact relationship
                                        let _ = sqlx::query(
                                            "INSERT OR IGNORE INTO contacts(owner_id, contact_id, added_at) VALUES(?, ?, ?)"
                                        )
                                        .bind(from_id)
                                        .bind(&contact_id)
                                        .bind(now_secs() as i64)
                                        .execute(&state.db)
                                        .await;

                                        let online = state.online.contains_key(contact_id.as_str());

                                        // Tell requester about the contact
                                        let _ = peer_tx.send(ServerMsg::ContactAdded {
                                            user_id: contact_id.clone(),
                                            username: contact_username.clone(),
                                            online,
                                        });

                                        // Tell the contact that this user added them
                                        if let Some(h) = state.online.get(contact_id.as_str()) {
                                            let _ = h.tx.send(ServerMsg::AddedByUser {
                                                user_id: from_id.clone(),
                                                username: from_name.clone(),
                                                online: true,
                                            });
                                        }
                                    }

                                    // ── RemoveContact ────────────────────────
                                    ClientMsg::RemoveContact { contact_id } => {
                                        let _ = sqlx::query(
                                            "DELETE FROM contacts WHERE owner_id = ? AND contact_id = ?"
                                        )
                                        .bind(from_id)
                                        .bind(&contact_id)
                                        .execute(&state.db)
                                        .await;
                                    }

                                    // ── GetHistory ───────────────────────────
                                    ClientMsg::GetHistory { with_user_id, before, limit } => {
                                        let lim = limit.unwrap_or(50).min(200) as i64;
                                        let before_ts = before.unwrap_or(u64::MAX) as i64;

                                        let rows = sqlx::query_as::<_, (String, String, String, i64)>(
                                            "SELECT id, from_id, text, timestamp
                                             FROM messages
                                             WHERE ((from_id = ? AND to_id = ?) OR (from_id = ? AND to_id = ?))
                                               AND timestamp < ?
                                             ORDER BY timestamp DESC
                                             LIMIT ?"
                                        )
                                        .bind(from_id)
                                        .bind(&with_user_id)
                                        .bind(&with_user_id)
                                        .bind(from_id)
                                        .bind(before_ts)
                                        .bind(lim)
                                        .fetch_all(&state.db)
                                        .await
                                        .unwrap_or_default();

                                        let mut msgs: Vec<HistoryMessage> = rows
                                            .into_iter()
                                            .map(|(id, fid, text, ts)| HistoryMessage {
                                                msg_id: id,
                                                from_id: fid,
                                                text,
                                                timestamp: ts as u64,
                                            })
                                            .collect();
                                        msgs.reverse(); // oldest first

                                        let _ = peer_tx.send(ServerMsg::MessageHistory {
                                            with_user_id: with_user_id.clone(),
                                            messages: msgs,
                                        });
                                    }

                                    // ── ChatMessage ──────────────────────────
                                    ClientMsg::ChatMessage { to, text, msg_id } => {
                                        let timestamp = now_millis();

                                        // Persist to DB
                                        let _ = sqlx::query(
                                            "INSERT OR IGNORE INTO messages(id, from_id, to_id, text, timestamp) VALUES(?, ?, ?, ?, ?)"
                                        )
                                        .bind(&msg_id)
                                        .bind(from_id)
                                        .bind(&to)
                                        .bind(&text)
                                        .bind(timestamp as i64)
                                        .execute(&state.db)
                                        .await;

                                        // Relay if recipient is online
                                        if let Some(h) = state.online.get(&to) {
                                            let _ = h.tx.send(ServerMsg::IncomingMessage {
                                                from: from_id.clone(),
                                                from_name,
                                                text,
                                                msg_id,
                                                timestamp,
                                            });
                                        }
                                    }

                                    // ── Call signalling ──────────────────────
                                    ClientMsg::CallOffer { to, sdp } => {
                                        if let Some(h) = state.online.get(&to) {
                                            let _ = h.tx.send(ServerMsg::IncomingCall {
                                                from: from_id.clone(), from_name, sdp,
                                            });
                                        } else {
                                            // Recipient is not online — tell the caller immediately
                                            // so they don't get stuck on the call screen.
                                            let _ = peer_tx.send(ServerMsg::CallRejected {
                                                from: to.clone(),
                                            });
                                        }
                                    }
                                    ClientMsg::CallAnswer { to, sdp } => {
                                        if let Some(h) = state.online.get(&to) {
                                            let _ = h.tx.send(ServerMsg::CallAnswered {
                                                from: from_id.clone(), sdp,
                                            });
                                        }
                                    }
                                    ClientMsg::CallReject { to } => {
                                        if let Some(h) = state.online.get(&to) {
                                            let _ = h.tx.send(ServerMsg::CallRejected { from: from_id.clone() });
                                        }
                                    }
                                    ClientMsg::HangUp { to } => {
                                        if let Some(h) = state.online.get(&to) {
                                            let _ = h.tx.send(ServerMsg::HangUp { from: from_id.clone() });
                                        }
                                    }
                                    ClientMsg::IceCandidate { to, candidate, sdp_mid, sdp_m_line_index } => {
                                        if let Some(h) = state.online.get(&to) {
                                            let _ = h.tx.send(ServerMsg::IceCandidate {
                                                from: from_id.clone(),
                                                candidate,
                                                sdp_mid,
                                                sdp_m_line_index,
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

    // ────────────────────────────────────────────────────────────────────────
    // Cleanup on disconnect
    // ────────────────────────────────────────────────────────────────────────
    if let Some(user_id) = my_user_id {
        state.online.remove(&user_id);

        // Update last_seen
        let _ = sqlx::query("UPDATE users SET last_seen = ? WHERE id = ?")
            .bind(now_secs() as i64)
            .bind(&user_id)
            .execute(&state.db)
            .await;

        // Notify everyone online who has this user in their contacts
        let watchers = sqlx::query_as::<_, (String,)>(
            "SELECT owner_id FROM contacts WHERE contact_id = ?"
        )
        .bind(&user_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        for (oid,) in &watchers {
            if let Some(h) = state.online.get(oid.as_str()) {
                let _ = h.tx.send(ServerMsg::UserOffline { user_id: user_id.clone() });
            }
        }

        info!("{addr} disconnected (user_id={user_id})");
    }

    send_task.abort();
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────
fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}
