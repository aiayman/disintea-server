use serde::{Deserialize, Serialize};

/// Messages sent from the client to the server
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First message after connecting — request to join a room
    Join {
        room_code: String,
        peer_id: String,
    },
    /// WebRTC SDP offer, relayed to the other peer(s)
    Offer {
        sdp: String,
        /// target peer_id; if None, broadcast to all others in room
        to: Option<String>,
    },
    /// WebRTC SDP answer
    Answer {
        sdp: String,
        to: Option<String>,
    },
    /// ICE candidate
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u32>,
        to: Option<String>,
    },
    /// Graceful disconnect
    Leave,
}

/// Messages sent from the server to the client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Acknowledgment that the peer joined the room
    Joined {
        peer_count: usize,
        /// IDs of existing peers already in the room
        existing_peers: Vec<String>,
    },
    /// Notified when a new peer enters the room
    PeerJoined { peer_id: String },
    /// Notified when a peer leaves or disconnects
    PeerLeft { peer_id: String },
    /// Relayed offer from another peer
    Offer { sdp: String, from: String },
    /// Relayed answer from another peer
    Answer { sdp: String, from: String },
    /// Relayed ICE candidate from another peer
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u32>,
        from: String,
    },
    /// Server error
    Error { reason: String },
    /// Room is full
    RoomFull { max: usize },
}
