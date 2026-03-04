use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;

use crate::messages::ServerMsg;

/// Handle to a connected peer: their ID and a channel to push messages to them
#[derive(Debug, Clone)]
pub struct PeerHandle {
    pub peer_id: String,
    pub tx: mpsc::UnboundedSender<ServerMsg>,
}

/// Thread-safe map of room_code → list of peers in that room
pub type RoomMap = Arc<DashMap<String, Vec<PeerHandle>>>;

pub fn new_room_map() -> RoomMap {
    Arc::new(DashMap::new())
}

impl PeerHandle {
    pub fn send(&self, msg: ServerMsg) {
        let _ = self.tx.send(msg);
    }
}
