use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Client → Server
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First message: identify yourself
    Register { user_id: String, username: String },
    /// Add someone to your contacts (server stores it)
    AddContact { contact_id: String },
    /// Remove a contact
    RemoveContact { contact_id: String },
    /// Fetch message history with a contact
    GetHistory { with_user_id: String, before: Option<u64>, limit: Option<u32> },
    /// Initiate a call
    CallOffer { to: String, sdp: String },
    /// Accept a call
    CallAnswer { to: String, sdp: String },
    /// Decline a call
    CallReject { to: String },
    /// End a call
    HangUp { to: String },
    /// WebRTC ICE candidate
    IceCandidate {
        to: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u32>,
    },
    /// Send a chat message
    ChatMessage { to: String, text: String, msg_id: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// Server → Client
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Registration acknowledged
    Registered,
    /// Full contact list on login
    ContactList { contacts: Vec<ContactInfo> },
    /// A contact came online
    UserOnline { user_id: String, username: String },
    /// A contact went offline
    UserOffline { user_id: String },
    /// Contact was added (server confirms, with current online state)
    ContactAdded { user_id: String, username: String, online: bool },
    /// Someone added you as a contact
    AddedByUser { user_id: String, username: String, online: bool },
    /// Message history response
    MessageHistory { with_user_id: String, messages: Vec<HistoryMessage> },
    /// Incoming call
    IncomingCall { from: String, from_name: String, sdp: String },
    /// Call was accepted
    CallAnswered { from: String, sdp: String },
    /// Call was rejected
    CallRejected { from: String },
    /// Hang up
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

#[derive(Debug, Clone, Serialize)]
pub struct ContactInfo {
    pub user_id: String,
    pub username: String,
    pub online: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryMessage {
    pub msg_id: String,
    pub from_id: String,
    pub text: String,
    pub timestamp: u64,
}
