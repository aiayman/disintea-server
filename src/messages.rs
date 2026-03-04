use serde::{Deserialize, Serialize};

/// Messages sent from the client to the server
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First message after connecting — identify yourself and provide your contacts list
    Register {
        user_id: String,
        username: String,
        /// IDs of contacts so the server can notify them of your presence
        contacts: Vec<String>,
    },
    /// Initiate a call to another user (contains the SDP offer)
    CallOffer { to: String, sdp: String },
    /// Accept an incoming call (contains the SDP answer)
    CallAnswer { to: String, sdp: String },
    /// Decline an incoming call
    CallReject { to: String },
    /// End an ongoing call
    HangUp { to: String },
    /// WebRTC ICE candidate
    IceCandidate {
        to: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u32>,
    },
    /// Send a chat message to another user
    ChatMessage { to: String, text: String, msg_id: String },
}

/// Messages sent from the server to the client
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Registration acknowledged
    Registered,
    /// One of your contacts came online
    UserOnline { user_id: String, username: String },
    /// One of your contacts went offline
    UserOffline { user_id: String },
    /// Someone is calling you
    IncomingCall { from: String, from_name: String, sdp: String },
    /// Your call was accepted
    CallAnswered { from: String, sdp: String },
    /// Your call was rejected
    CallRejected { from: String },
    /// The other side hung up
    HangUp { from: String },
    /// Relayed ICE candidate
    IceCandidate {
        from: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u32>,
    },
    /// Incoming chat message
    IncomingMessage {
        from: String,
        from_name: String,
        text: String,
        msg_id: String,
        timestamp: u64,
    },
    /// Server error
    Error { reason: String },
}
