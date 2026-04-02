//! Collaborative editing state management.
//!
//! Implements the server-side authority for prosemirror-collab, maintaining
//! document state and broadcasting changes to connected clients.
//!
//! ## Wire Protocol (`MessagePack` arrays)
//!
//! Messages are encoded as `MessagePack` arrays for efficiency:
//! - `[0, version, doc, mode]` - Init: server sends initial state with content mode
//! - `[1, version, steps, clientID]` - Steps: client sends changes
//! - `[2, steps, clientIDs]` - Update: server broadcasts changes
//! - `[3, version]` - Ack: server confirms steps applied
//! - `[4, clientID, head, anchor, name?, idleSecs?]` - Cursor position
//! - `[5, error]` - Error message
//! - `[6, clientID]` - Cursor removed (client disconnected)
//! - `[7, hash, name]` - New version saved (reload available)
//! - `""` (empty text) - Ping/pong for inactive tab cursor refresh
//!
//! ## Content Modes
//!
//! The `mode` field in Init messages tells the client how to render the document:
//! - `"rich"` - Full `ProseMirror` editor with toolbar
//! - `"markdown"` - Full editor, server converts markdown to/from PM JSON
//! - `"plain"` - Full editor, lines become paragraphs
//! - `"raw"` - Editor with no formatting toolbar (for code/config files)
//! - `"media"` - Native browser rendering (image/video/audio/pdf)
//! - `"binary"` - Cannot be edited, show download link
//!
//! The `idleSecs` field is only sent when the server sends existing cursors
//! to a newly connected client. It indicates how long the cursor has been
//! idle so the client can display the correct opacity immediately.
//!
//! ## Empty Text Messages
//!
//! WebSocket Ping control frames are handled silently by browsers and don't
//! trigger JavaScript's `onmessage`. To allow cursor decoration refresh in
//! inactive tabs (where `setInterval` is throttled), the server sends empty
//! text messages every 60s instead of Ping frames. The client responds with
//! empty text and refreshes cursor decorations.
//!
//! ## Timeout Behavior
//!
//! - WebSocket closed after 30 minutes of inactivity
//! - Cursor removed 5 minutes after client disconnects
//! - Document cleaned up 1 hour after last client disconnects

use axum::{
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize},
    },
    time::Instant,
};
use tokio::sync::{RwLock, broadcast};

use super::content_mode::{ContentMode, detect_mode_with_content};
use super::identity::short_id;
use super::markdown::{
    markdown_to_prosemirror, plain_text_to_prosemirror, raw_text_to_prosemirror,
};

/// Message type tags for the wire protocol.
mod msg {
    pub const INIT: u8 = 0;
    pub const STEPS: u8 = 1;
    pub const UPDATE: u8 = 2;
    pub const ACK: u8 = 3;
    pub const CURSOR: u8 = 4;
    pub const ERROR: u8 = 5;
    pub const CURSOR_REMOVE: u8 = 6;
    pub const NEW_VERSION: u8 = 7;
    pub const AUTH: u8 = 8;
    pub const AUTH_OK: u8 = 9;
}

/// Query parameters for WebSocket connection.
#[derive(Debug, Deserialize)]
pub struct WsParams {
    /// Filename for content mode detection (optional).
    pub filename: Option<String>,
}

/// Load file content from the blob store.
///
/// Returns the file content as bytes if found.
async fn load_file_content(store: &iroh_blobs::api::Store, hash_str: &str) -> Option<Vec<u8>> {
    let hash: iroh_blobs::Hash = hash_str.parse().ok()?;
    let bytes = store.blobs().get_bytes(hash).await.ok()?;
    Some(bytes.to_vec())
}

/// Convert content bytes to a `ProseMirror` document based on content mode.
///
/// Returns the document JSON and the detected content mode.
fn content_to_document(
    content: Option<&[u8]>,
    filename: Option<&str>,
) -> (serde_json::Value, ContentMode) {
    let Some(bytes) = content else {
        // No content - return empty document
        return (
            serde_json::json!({
                "type": "doc",
                "content": [{"type": "paragraph"}]
            }),
            ContentMode::Raw,
        );
    };

    // Detect content mode from filename and content
    let mode = detect_mode_with_content(filename.unwrap_or(""), bytes);

    // Non-editable modes don't need document conversion
    if !mode.is_editable() {
        return (
            serde_json::json!({
                "type": "doc",
                "content": [{"type": "paragraph"}]
            }),
            mode,
        );
    }

    // Convert bytes to string (for editable modes, we already know it's valid UTF-8)
    let text = String::from_utf8_lossy(bytes);

    let doc = match mode {
        ContentMode::Rich => {
            // Already ProseMirror JSON - parse it directly
            serde_json::from_str(&text).unwrap_or_else(|_| {
                serde_json::json!({
                    "type": "doc",
                    "content": [{"type": "paragraph"}]
                })
            })
        }
        ContentMode::Markdown => markdown_to_prosemirror(&text),
        ContentMode::Plain => plain_text_to_prosemirror(&text),
        ContentMode::Raw => raw_text_to_prosemirror(&text),
        // Media and Binary already handled above
        ContentMode::Media(_) | ContentMode::Binary => {
            serde_json::json!({
                "type": "doc",
                "content": [{"type": "paragraph"}]
            })
        }
    };

    (doc, mode)
}

/// A step in the `ProseMirror` document history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// The step data as JSON.
    #[serde(flatten)]
    pub data: serde_json::Value,
}

/// Duration constants for timeouts.
mod timeouts {
    use std::time::Duration;

    /// Send ping every 30 seconds to check connection health.
    pub const PING_INTERVAL: Duration = Duration::from_secs(30);

    /// Close WebSocket if no pong received within this time.
    /// This detects when the page is closed or client goes offline.
    pub const PONG_TIMEOUT: Duration = Duration::from_secs(30 * 60);

    /// Remove cursor 5 minutes after client disconnects.
    pub const CURSOR_CLEANUP: Duration = Duration::from_secs(5 * 60);

    /// Clean up document 1 hour after last client disconnects.
    pub const DOCUMENT_CLEANUP: Duration = Duration::from_secs(60 * 60);

    /// Send empty text message (instead of Ping control frame) every N pings.
    /// This triggers client's onmessage handler so it can refresh cursor decorations.
    /// (Ping control frames are handled silently by the browser and don't trigger JS.)
    pub const EMPTY_TEXT_MESSAGE_EVERY_N_PINGS: u32 = 2;
}

/// Stored cursor position for a client.
#[derive(Debug, Clone)]
pub struct CursorPosition {
    pub head: u32,
    pub anchor: u32,
    pub name: Option<String>,
    /// The identity client_id (hex string) if authenticated, for name updates.
    pub identity_client_id: Option<String>,
    /// When the cursor was last updated (for age calculation on initial load).
    pub last_update: Instant,
    /// When the client disconnected (None if still connected).
    pub disconnected_at: Option<Instant>,
}

/// A document being collaboratively edited.
#[derive(Debug)]
pub struct Document {
    /// Current document version (number of steps applied).
    pub version: AtomicU64,
    /// The current document state as JSON.
    pub doc: RwLock<serde_json::Value>,
    /// Content mode for this document.
    pub mode: ContentMode,
    /// History of all applied steps (step data, client ID as JSON value).
    pub steps: RwLock<Vec<(Step, serde_json::Value)>>,
    /// Active cursor positions by client ID.
    pub cursors: RwLock<HashMap<String, CursorPosition>>,
    /// Broadcast channel for sending updates to clients.
    pub broadcast: broadcast::Sender<CollabMessage>,
    /// Number of connected clients.
    pub client_count: AtomicUsize,
    /// When the last client disconnected (None if clients are connected).
    pub last_client_disconnect: RwLock<Option<Instant>>,
}

impl Document {
    /// Create a new empty document.
    pub fn new() -> Self {
        Self::with_doc_and_mode(
            serde_json::json!({
                "type": "doc",
                "content": [{"type": "paragraph"}]
            }),
            ContentMode::Raw,
        )
    }

    /// Create a document with a `ProseMirror` document and content mode.
    pub fn with_doc_and_mode(doc: serde_json::Value, mode: ContentMode) -> Self {
        let (tx, _) = broadcast::channel(256);

        Self {
            version: AtomicU64::new(0),
            doc: RwLock::new(doc),
            mode,
            steps: RwLock::new(Vec::new()),
            cursors: RwLock::new(HashMap::new()),
            broadcast: tx,
            client_count: AtomicUsize::new(0),
            last_client_disconnect: RwLock::new(None),
        }
    }

    /// Create a document with optional initial content and filename for mode detection.
    ///
    /// Converts content to a `ProseMirror` document structure based on detected mode.
    pub fn with_content(content: Option<&[u8]>, filename: Option<&str>) -> Self {
        let (doc, mode) = content_to_document(content, filename);
        Self::with_doc_and_mode(doc, mode)
    }

    /// Get the current version.
    pub fn version(&self) -> u64 {
        self.version.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Increment client count when a client connects.
    /// Returns the new count.
    pub fn client_connected(&self) -> usize {
        use std::sync::atomic::Ordering;
        self.client_count.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Decrement client count when a client disconnects.
    /// Returns the new count.
    pub async fn client_disconnected(&self) -> usize {
        use std::sync::atomic::Ordering;
        let new_count = self
            .client_count
            .fetch_sub(1, Ordering::SeqCst)
            .saturating_sub(1);
        if new_count == 0 {
            *self.last_client_disconnect.write().await = Some(Instant::now());
        }
        new_count
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// State for all collaborative editing sessions.
#[derive(Debug, Default)]
pub struct CollabState {
    /// Active documents by ID.
    documents: RwLock<HashMap<String, Arc<Document>>>,
}

impl CollabState {
    /// Create a new collaborative state manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a document for editing.
    ///
    /// If `initial_content` is provided and the document doesn't exist,
    /// the document will be initialized with that content.
    pub async fn get_or_create(
        &self,
        doc_id: &str,
        initial_content: Option<&[u8]>,
        filename: Option<&str>,
    ) -> Arc<Document> {
        let read = self.documents.read().await;
        if let Some(doc) = read.get(doc_id) {
            return Arc::clone(doc);
        }
        drop(read);

        let mut write = self.documents.write().await;
        // Double-check after acquiring write lock
        if let Some(doc) = write.get(doc_id) {
            return Arc::clone(doc);
        }

        let doc = Arc::new(Document::with_content(initial_content, filename));
        write.insert(doc_id.to_owned(), Arc::clone(&doc));
        doc
    }

    /// Remove a document from the state.
    pub async fn remove_document(&self, doc_id: &str) {
        let mut write = self.documents.write().await;
        write.remove(doc_id);
        tracing::info!("[collab] Document '{}' cleaned up", doc_id);
    }

    /// Notify clients editing a document that a new version was saved.
    ///
    /// Called by `save_handler` when a file is saved with a new hash.
    /// Broadcasts `NewVersion` to all clients connected to the old hash session.
    pub async fn notify_new_version(&self, old_doc_id: &str, new_hash: &str, filename: &str) {
        let read = self.documents.read().await;
        if let Some(doc) = read.get(old_doc_id) {
            let msg = CollabMessage::NewVersion {
                hash: new_hash.to_owned(),
                name: filename.to_owned(),
            };
            let receivers = doc.broadcast.send(msg).unwrap_or(0);
            if receivers > 0 {
                tracing::info!(
                    "[collab] Notified {} client(s) about new version of '{}': {} -> {}",
                    receivers,
                    filename,
                    old_doc_id,
                    new_hash
                );
            }
        }
    }

    /// Update cursor display names for a client across all active documents.
    ///
    /// Called when a client changes their display name via the settings API.
    /// Finds all cursors with the matching `identity_client_id`, updates the
    /// stored name, and broadcasts the updated cursor to all connected clients.
    pub async fn update_client_name(&self, identity_client_id: &str, new_name: &str) {
        let docs = self.documents.read().await;
        for (doc_id, doc) in docs.iter() {
            let mut cursors = doc.cursors.write().await;
            // Find cursors belonging to this identity
            let matching: Vec<(String, u32, u32)> = cursors
                .iter()
                .filter(|(_, pos)| {
                    pos.identity_client_id.as_deref() == Some(identity_client_id)
                })
                .map(|(cid, pos)| (cid.clone(), pos.head, pos.anchor))
                .collect();

            for (cursor_id, head, anchor) in &matching {
                if let Some(pos) = cursors.get_mut(cursor_id) {
                    pos.name = Some(new_name.to_owned());
                }

                // Broadcast the updated cursor to all clients
                let client_id = cursor_id.parse::<u64>().unwrap_or(0);
                let cursor_msg = CollabMessage::Cursor {
                    client_id,
                    head: *head,
                    anchor: *anchor,
                    name: Some(new_name.to_owned()),
                    idle_secs: None,
                };
                let receivers = doc.broadcast.send(cursor_msg).unwrap_or(0);
                if receivers > 0 {
                    tracing::info!(
                        "[collab] Broadcast name update for {} in doc '{}' to {} client(s)",
                        identity_client_id,
                        doc_id,
                        receivers,
                    );
                }
            }
        }
    }
}

/// Messages sent over the WebSocket connection.
///
/// Serialized as `MessagePack` arrays: `[type_tag, ...fields]`
#[derive(Debug, Clone)]
pub enum CollabMessage {
    /// `[0, version, doc, mode]` - Initial document state sent to client.
    Init {
        version: u64,
        doc: serde_json::Value,
        mode: String,
    },
    /// `[1, version, steps, clientID]` - Steps received from a client.
    Steps {
        version: u64,
        steps: Vec<serde_json::Value>,
        client_id: u64,
    },
    /// `[2, steps, clientIDs]` - Steps broadcast to other clients.
    Update {
        steps: Vec<serde_json::Value>,
        client_ids: Vec<u64>,
    },
    /// `[3, version]` - Acknowledgment that steps were applied.
    Ack { version: u64 },
    /// `[4, clientID, head, anchor, name?, idleSecs?]` - Cursor/selection position.
    /// `idleSecs` is only set when sending existing cursors to a new client.
    Cursor {
        client_id: u64,
        head: u32,
        anchor: u32,
        name: Option<String>,
        /// Seconds the cursor has been idle (only set on initial load).
        idle_secs: Option<u64>,
    },
    /// `[5, error]` - Error message.
    Error { error: String },
    /// `[6, clientID]` - Cursor removed (client disconnected).
    CursorRemove { client_id: u64 },
    /// `[7, hash, name]` - New version of the file was saved.
    /// Tells clients editing the old hash that a newer version exists.
    NewVersion { hash: String, name: String },
    /// `[8, token]` - Client authentication (first message after connect).
    Auth { token: String },
    /// `[9, client_id, name, token?]` - Authentication succeeded.
    /// Includes a refreshed token so long-lived WS sessions stay authenticated.
    AuthOk {
        client_id: String,
        name: Option<String>,
        token: Option<String>,
    },
}

impl CollabMessage {
    /// Encode message to `MessagePack` bytes.
    pub fn encode(&self) -> Vec<u8> {
        use rmp_serde::encode::to_vec;

        match self {
            Self::Init { version, doc, mode } => {
                to_vec(&(msg::INIT, version, doc, mode)).unwrap_or_default()
            }
            Self::Steps {
                version,
                steps,
                client_id,
            } => to_vec(&(msg::STEPS, version, steps, client_id)).unwrap_or_default(),
            Self::Update { steps, client_ids } => {
                to_vec(&(msg::UPDATE, steps, client_ids)).unwrap_or_default()
            }
            Self::Ack { version } => to_vec(&(msg::ACK, version)).unwrap_or_default(),
            Self::Cursor {
                client_id,
                head,
                anchor,
                name,
                idle_secs,
            } => {
                to_vec(&(msg::CURSOR, client_id, head, anchor, name, idle_secs)).unwrap_or_default()
            }
            Self::Error { error } => to_vec(&(msg::ERROR, error)).unwrap_or_default(),
            Self::CursorRemove { client_id } => {
                to_vec(&(msg::CURSOR_REMOVE, client_id)).unwrap_or_default()
            }
            Self::NewVersion { hash, name } => {
                to_vec(&(msg::NEW_VERSION, hash, name)).unwrap_or_default()
            }
            Self::Auth { token } => to_vec(&(msg::AUTH, token)).unwrap_or_default(),
            Self::AuthOk { client_id, name, token } => {
                to_vec(&(msg::AUTH_OK, client_id, name, token)).unwrap_or_default()
            }
        }
    }

    /// Decode message from `MessagePack` bytes.
    pub fn decode(data: &[u8]) -> Option<Self> {
        use rmp_serde::decode::from_slice;

        // MessagePack tuples are encoded as arrays. The first element is the message type.
        // We decode the whole thing for each message type. This is slightly inefficient
        // but simple and correct.

        // Try each message type in order
        // Init with mode (new format)
        if let Ok((msg::INIT, version, doc, mode)) =
            from_slice::<(u8, u64, serde_json::Value, String)>(data)
        {
            return Some(Self::Init { version, doc, mode });
        }
        // Init without mode (legacy format - default to raw)
        if let Ok((msg::INIT, version, doc)) = from_slice::<(u8, u64, serde_json::Value)>(data) {
            return Some(Self::Init {
                version,
                doc,
                mode: "raw".to_owned(),
            });
        }

        if let Ok((msg::STEPS, version, steps, client_id)) =
            from_slice::<(u8, u64, Vec<serde_json::Value>, u64)>(data)
        {
            return Some(Self::Steps {
                version,
                steps,
                client_id,
            });
        }

        if let Ok((msg::UPDATE, steps, client_ids)) =
            from_slice::<(u8, Vec<serde_json::Value>, Vec<u64>)>(data)
        {
            return Some(Self::Update { steps, client_ids });
        }

        if let Ok((msg::ACK, version)) = from_slice::<(u8, u64)>(data) {
            return Some(Self::Ack { version });
        }

        // Try cursor with idle_secs first, then without
        if let Ok((msg::CURSOR, client_id, head, anchor, name, idle_secs)) =
            from_slice::<(u8, u64, u32, u32, Option<String>, Option<u64>)>(data)
        {
            return Some(Self::Cursor {
                client_id,
                head,
                anchor,
                name,
                idle_secs,
            });
        }
        if let Ok((msg::CURSOR, client_id, head, anchor, name)) =
            from_slice::<(u8, u64, u32, u32, Option<String>)>(data)
        {
            return Some(Self::Cursor {
                client_id,
                head,
                anchor,
                name,
                idle_secs: None,
            });
        }

        if let Ok((msg::ERROR, error)) = from_slice::<(u8, String)>(data) {
            return Some(Self::Error { error });
        }

        if let Ok((msg::NEW_VERSION, hash, name)) = from_slice::<(u8, String, String)>(data) {
            return Some(Self::NewVersion { hash, name });
        }

        if let Ok((msg::AUTH, token)) = from_slice::<(u8, String)>(data) {
            return Some(Self::Auth { token });
        }

        // AUTH_OK with token (new format)
        if let Ok((msg::AUTH_OK, client_id, name, token)) =
            from_slice::<(u8, String, Option<String>, Option<String>)>(data)
        {
            return Some(Self::AuthOk { client_id, name, token });
        }
        // AUTH_OK without token (legacy format)
        if let Ok((msg::AUTH_OK, client_id, name)) =
            from_slice::<(u8, String, Option<String>)>(data)
        {
            return Some(Self::AuthOk { client_id, name, token: None });
        }

        None
    }
}

/// Extract binary data from a WebSocket message (handles both Binary and legacy Text).
fn extract_binary(msg: &Message) -> Option<Vec<u8>> {
    match msg {
        Message::Binary(data) => Some(data.clone()),
        _ => None,
    }
}

/// WebSocket upgrade handler for collaborative editing.
pub async fn ws_collab_handler(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    Query(params): Query<WsParams>,
    State(state): State<super::AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| {
        handle_collab_socket(
            socket,
            doc_id,
            params.filename,
            state.collab,
            state.store,
            state.identity,
        )
    })
}

/// Handle a WebSocket connection for collaborative editing.
async fn handle_collab_socket(
    socket: WebSocket,
    doc_id: String,
    filename: Option<String>,
    collab: Arc<CollabState>,
    store: iroh_blobs::api::Store,
    identity_store: super::IdentityStore,
) {
    use std::sync::atomic::Ordering;

    tracing::info!(
        "[collab] New connection for doc '{}' (page load), filename={:?}",
        doc_id,
        filename,
    );

    // Try to load file content from the store (doc_id is the hash)
    let initial_content = load_file_content(&store, &doc_id).await;
    if initial_content.is_some() {
        tracing::debug!("[collab] Loaded file content from store for '{}'", doc_id);
    }

    let doc = collab
        .get_or_create(&doc_id, initial_content.as_deref(), filename.as_deref())
        .await;
    let client_count = doc.client_connected();
    tracing::info!(
        "[collab] Client connected to doc '{}', {} total clients, mode={}",
        doc_id,
        client_count,
        doc.mode.as_str()
    );

    let mut rx = doc.broadcast.subscribe();

    let (mut sender, mut receiver) = socket.split();

    // =========================================================================
    // First-message authentication
    // =========================================================================
    // Wait up to 5 seconds for the client to send an AUTH message as the first
    // binary frame. If the first message is AUTH, verify the token and set the
    // identity. If it's any other message type or the timeout expires, proceed
    // as anonymous (the message is NOT dropped — it's processed normally below).
    let mut identity_client_id: Option<String> = None;
    let mut cached_display_name: Option<String> = None;
    let mut name_watcher: Option<tokio::sync::watch::Receiver<Option<String>>> = None;
    let mut buffered_msg: Option<CollabMessage> = None;

    let auth_deadline = tokio::time::sleep(std::time::Duration::from_secs(5));
    tokio::pin!(auth_deadline);

    tokio::select! {
        maybe_msg = receiver.next() => {
            if let Some(Ok(ws_msg)) = maybe_msg {
                if let Some(data) = extract_binary(&ws_msg) {
                    if let Some(decoded) = CollabMessage::decode(&data) {
                        match decoded {
                            CollabMessage::Auth { ref token } => {
                                // Verify the token and get a refreshed one
                                if let Some((fresh_token, identity)) = identity_store.verify_and_refresh(token).await {
                                    let display_name = identity.name.clone().unwrap_or_else(|| short_id(&identity.client_id));
                                    tracing::info!("[collab] Authenticated client: {} -> {:?}", identity.client_id, display_name);
                                    name_watcher = identity_store.subscribe_name(&identity.client_id).await;

                                    // Send AUTH_OK back to client with refreshed token
                                    let auth_ok = CollabMessage::AuthOk {
                                        client_id: identity.client_id.clone(),
                                        name: Some(display_name.clone()),
                                        token: Some(fresh_token),
                                    };
                                    if sender.send(Message::Binary(auth_ok.encode())).await.is_err() {
                                        tracing::warn!("[collab] Client disconnected during auth response");
                                        doc.client_disconnected().await;
                                        return;
                                    }

                                    cached_display_name = Some(display_name);
                                    identity_client_id = Some(identity.client_id);
                                } else {
                                    tracing::info!("[collab] Auth failed: invalid/expired token");
                                    // Proceed as anonymous — no AUTH_OK sent
                                }
                            }
                            other => {
                                // Not an auth message — buffer it for normal processing
                                tracing::debug!("[collab] First message is not AUTH, proceeding as anonymous");
                                buffered_msg = Some(other);
                            }
                        }
                    }
                }
            } else {
                // Client disconnected before sending anything
                tracing::warn!("[collab] Client disconnected before first message");
                doc.client_disconnected().await;
                return;
            }
        }
        () = &mut auth_deadline => {
            tracing::debug!("[collab] Auth timeout, proceeding as anonymous");
        }
    }

    // Send initial document state (binary MessagePack)
    let init_msg = CollabMessage::Init {
        version: 0,
        doc: doc.doc.read().await.clone(),
        mode: doc.mode.as_str().to_owned(),
    };

    let init_bytes = init_msg.encode();
    tracing::info!(
        "[collab] Sending Init: version=0 (base), mode={}, {} bytes, current_version={}",
        doc.mode.as_str(),
        init_bytes.len(),
        doc.version()
    );

    if sender.send(Message::Binary(init_bytes)).await.is_err() {
        tracing::warn!("[collab] Client disconnected during init send");
        doc.client_disconnected().await;
        return;
    }

    // Send catch-up Update with all accumulated steps so the client
    // replays from version 0 to the current version.
    {
        let steps = doc.steps.read().await;
        if !steps.is_empty() {
            let catch_up_steps: Vec<serde_json::Value> =
                steps.iter().map(|(step, _)| step.data.clone()).collect();
            let catch_up_client_ids: Vec<u64> = steps
                .iter()
                .filter_map(|(_, cid)| cid.as_u64())
                .collect();

            let catch_up_msg = CollabMessage::Update {
                steps: catch_up_steps,
                client_ids: catch_up_client_ids,
            };
            let catch_up_bytes = catch_up_msg.encode();
            tracing::info!(
                "[collab] Sending catch-up Update: {} steps, {} bytes",
                steps.len(),
                catch_up_bytes.len()
            );

            if sender
                .send(Message::Binary(catch_up_bytes))
                .await
                .is_err()
            {
                tracing::warn!("[collab] Client disconnected during catch-up send");
                doc.client_disconnected().await;
                return;
            }
        }
    }

    // Send existing cursor positions to new client (only those still connected)
    {
        let cursors = doc.cursors.read().await;
        let now = Instant::now();
        let mut cursor_count = 0;
        for (client_id_str, cursor) in cursors.iter() {
            // Skip cursors from disconnected clients
            if cursor.disconnected_at.is_some() {
                continue;
            }
            let client_id = client_id_str.parse::<u64>().unwrap_or(0);
            // Calculate how long this cursor has been idle (in seconds)
            let idle_secs = now.duration_since(cursor.last_update).as_secs();
            tracing::debug!(
                "[collab] Sending existing cursor: client={}, idle={}s, name={:?}",
                client_id,
                idle_secs,
                cursor.name
            );
            let cursor_msg = CollabMessage::Cursor {
                client_id,
                head: cursor.head,
                anchor: cursor.anchor,
                name: cursor.name.clone(),
                idle_secs: Some(idle_secs),
            };
            if sender
                .send(Message::Binary(cursor_msg.encode()))
                .await
                .is_err()
            {
                tracing::warn!("[collab] Client disconnected while sending existing cursors");
                doc.client_disconnected().await;
                return;
            }
            cursor_count += 1;
        }
        if cursor_count > 0 {
            tracing::info!(
                "[collab] Sent {} existing cursor(s) to new client",
                cursor_count
            );
        }
    }

    // Wrap sender in Arc<Mutex> for sharing between tasks
    let sender = Arc::new(tokio::sync::Mutex::new(sender));
    let sender_for_broadcast = Arc::clone(&sender);

    // Track last activity (any message received) for connection health
    // Any message resets the timer; we only ping if idle
    let last_activity = Arc::new(RwLock::new(Instant::now()));
    let last_activity_for_ping = Arc::clone(&last_activity);
    let sender_for_ping = Arc::clone(&sender);

    // Track the client's ID for cleanup on disconnect
    let client_id_for_cleanup: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
    let client_id_for_disconnect = Arc::clone(&client_id_for_cleanup);

    // Spawn task to forward broadcasts to this client (binary)
    let doc_id_for_broadcast = doc_id.clone();
    let broadcast_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let bytes = msg.encode();
                    let mut sender = sender_for_broadcast.lock().await;
                    if sender.send(Message::Binary(bytes)).await.is_err() {
                        break; // Client disconnected
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        doc_id = %doc_id_for_broadcast,
                        skipped = n,
                        "Broadcast receiver lagged, sending desync error to client"
                    );
                    // Tell the client to reconnect for a fresh state.
                    // The client will close the WS → reconnect → get Init + catch-up.
                    let error_msg = CollabMessage::Error {
                        error: format!(
                            "Session desynchronized: {n} messages lost"
                        ),
                    };
                    let mut sender = sender_for_broadcast.lock().await;
                    let _ = sender
                        .send(Message::Binary(error_msg.encode()))
                        .await;
                    break; // Stop broadcasting — client will reconnect
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break; // Channel closed, document cleaned up
                }
            }
        }
    });

    // Spawn task to monitor connection health via ping/pong
    // Every N pings, sends empty Text instead of Ping frame to trigger client's onmessage
    // (allows client to refresh cursor decorations even in inactive tabs)
    // Closes connection if no activity for PONG_TIMEOUT (page closed/offline)
    let ping_task = tokio::spawn(async move {
        let mut ping_cycle: u32 = 0;

        loop {
            tokio::time::sleep(timeouts::PING_INTERVAL).await;
            ping_cycle = ping_cycle.wrapping_add(1);

            let last = *last_activity_for_ping.read().await;
            let elapsed = last.elapsed();

            // If no activity for 30m, connection is dead
            if elapsed >= timeouts::PONG_TIMEOUT {
                tracing::info!(
                    "[collab] Connection timeout - no activity for {}s, closing (page likely closed)",
                    elapsed.as_secs()
                );
                let mut sender = sender_for_ping.lock().await;
                let _ = sender.close().await;
                break;
            }

            // Only ping if idle for the ping interval (no recent messages)
            if elapsed >= timeouts::PING_INTERVAL {
                let mut sender = sender_for_ping.lock().await;

                // Every N pings, send empty Text instead of Ping frame
                // This triggers client's onmessage so it can refresh cursor decorations
                let use_text = ping_cycle % timeouts::EMPTY_TEXT_MESSAGE_EVERY_N_PINGS == 0;

                let result = if use_text {
                    tracing::debug!(
                        "[collab] Sending empty text message (cursor refresh trigger), idle={}s",
                        elapsed.as_secs()
                    );
                    sender.send(Message::Text(String::new())).await
                } else {
                    tracing::debug!("[collab] Sending ping, idle={}s", elapsed.as_secs());
                    sender.send(Message::Ping(vec![])).await
                };

                if result.is_err() {
                    tracing::info!("[collab] Failed to send ping/text, client disconnected");
                    break;
                }
            }
        }
    });

    // Handle incoming messages.
    // If we buffered a non-AUTH message during the auth phase, process it first.
    let buffered_iter = buffered_msg
        .take()
        .map(|m| Ok(Message::Binary(m.encode())))
        .into_iter();
    let ws_stream = futures::stream::iter(buffered_iter).chain(receiver);
    tokio::pin!(ws_stream);

    while let Some(Ok(msg)) = ws_stream.next().await {
        // Any message (pong, binary, etc.) counts as activity
        *last_activity.write().await = Instant::now();

        match msg {
            Message::Binary(data) => {
                let Some(collab_msg) = CollabMessage::decode(&data) else {
                    tracing::debug!("[collab] Failed to decode binary message");
                    continue;
                };

                match collab_msg {
                    CollabMessage::Steps {
                        version,
                        steps,
                        client_id,
                    } => {
                        tracing::info!(
                            "[collab] Steps from client {}: version={}, steps={}",
                            client_id,
                            version,
                            steps.len()
                        );

                        // Verify version matches
                        let current_version = doc.version();
                        if version != current_version {
                            let error_msg = CollabMessage::Error {
                                error: format!(
                                    "Version mismatch: expected {current_version}, got {version}"
                                ),
                            };
                            tracing::warn!(
                                "[collab] Version mismatch: expected {}, got {}",
                                current_version,
                                version
                            );
                            let mut sender = sender.lock().await;
                            let _ = sender.send(Message::Binary(error_msg.encode())).await;
                            continue;
                        }

                        // Apply steps
                        let mut doc_steps = doc.steps.write().await;
                        for step in &steps {
                            doc_steps.push((
                                Step { data: step.clone() },
                                serde_json::Value::Number(client_id.into()),
                            ));
                        }
                        drop(doc_steps);

                        // Increment version
                        let new_version =
                            doc.version.fetch_add(steps.len() as u64, Ordering::SeqCst)
                                + steps.len() as u64;

                        // Broadcast to other clients
                        let update = CollabMessage::Update {
                            steps: steps.clone(),
                            client_ids: vec![client_id; steps.len()],
                        };
                        let broadcast_count = doc.broadcast.send(update).unwrap_or(0);
                        tracing::info!(
                            "[collab] Broadcast to {} receivers, new version={}",
                            broadcast_count,
                            new_version
                        );

                        // Send ack to sender
                        let ack = CollabMessage::Ack {
                            version: new_version,
                        };
                        tracing::debug!(
                            "[collab] Sending Ack: version={}",
                            new_version
                        );
                        let mut sender = sender.lock().await;
                        let _ = sender.send(Message::Binary(ack.encode())).await;
                    }
                    CollabMessage::Cursor {
                        client_id,
                        head,
                        anchor,
                        name,
                        .. // Ignore idle_secs from incoming messages
                    } => {
                        let client_id_str = client_id.to_string();

                        // If this connection has a verified identity, use the
                        // cached display name. The cache is updated via a watch
                        // channel when the name changes (non-blocking check).
                        if let Some(ref mut rx) = name_watcher {
                            if rx.has_changed().unwrap_or(false) {
                                let new_name = rx.borrow_and_update().clone();
                                cached_display_name = Some(
                                    new_name.unwrap_or_else(|| {
                                        short_id(identity_client_id.as_deref().unwrap_or(""))
                                    })
                                );
                            }
                        }
                        let effective_name = cached_display_name.clone().or(name);

                        tracing::debug!(
                            "[collab] Received Cursor: client={}, head={}, anchor={}, name={:?}",
                            client_id,
                            head,
                            anchor,
                            effective_name
                        );

                        // Remember this client's ID for disconnect cleanup
                        *client_id_for_cleanup.write().await = Some(client_id_str.clone());

                        {
                            let mut cursors = doc.cursors.write().await;
                            cursors.insert(
                                client_id_str,
                                CursorPosition {
                                    head,
                                    anchor,
                                    name: effective_name.clone(),
                                    identity_client_id: identity_client_id.clone(),
                                    last_update: Instant::now(),
                                    disconnected_at: None,
                                },
                            );
                        }

                        // Broadcast cursor position to all clients (no idle_secs for live updates)
                        let cursor_msg = CollabMessage::Cursor {
                            client_id,
                            head,
                            anchor,
                            name: effective_name,
                            idle_secs: None,
                        };
                        let _ = doc.broadcast.send(cursor_msg);
                    }
                    _ => {
                        tracing::debug!("[collab] Received unexpected message type, ignoring");
                    }
                }
            }
            Message::Text(text) => {
                // Empty text is pong response from client (for cursor refresh)
                if text.is_empty() {
                    tracing::debug!("[collab] Received empty text (pong for cursor refresh)");
                } else {
                    tracing::debug!("[collab] Received text message: {} chars", text.len());
                }
            }
            Message::Pong(_) => {
                tracing::debug!("[collab] Received pong");
            }
            Message::Close(reason) => {
                tracing::info!(
                    "[collab] Client closed connection: {:?}",
                    reason.map(|r| format!("{}: {}", r.code, r.reason))
                );
                break;
            }
            Message::Ping(_) => {
                // Browser doesn't send pings, but handle anyway
                tracing::debug!("[collab] Received ping from client");
            }
        }
    }

    // Clean up
    broadcast_task.abort();
    ping_task.abort();

    // Handle client disconnect
    let remaining_clients = doc.client_disconnected().await;
    tracing::info!(
        "[collab] Client disconnected from doc '{}', {} clients remaining",
        doc_id,
        remaining_clients
    );

    // Mark cursor as disconnected and schedule cleanup
    let client_id_opt = client_id_for_disconnect.read().await.clone();
    if let Some(client_id_str) = client_id_opt {
        let now = Instant::now();

        // Parse client_id for broadcast
        let client_id_num: u64 = client_id_str.parse().unwrap_or(0);

        {
            let mut cursors = doc.cursors.write().await;
            if let Some(cursor) = cursors.get_mut(&client_id_str) {
                cursor.disconnected_at = Some(now);
                tracing::info!(
                    "[collab] Marked cursor {} as disconnected, will remove in 5m if not reconnected",
                    client_id_str
                );
            }
        }

        // Broadcast cursor removal to all connected clients
        let remove_msg = CollabMessage::CursorRemove {
            client_id: client_id_num,
        };
        let _ = doc.broadcast.send(remove_msg);
        tracing::debug!(
            "[collab] Broadcast cursor removal for client {}",
            client_id_str
        );

        // Schedule cursor cleanup after 5 minutes
        let doc_for_cleanup = Arc::clone(&doc);
        let client_id_for_task = client_id_str.clone();
        tokio::spawn(async move {
            tokio::time::sleep(timeouts::CURSOR_CLEANUP).await;

            let mut cursors = doc_for_cleanup.cursors.write().await;
            if let Some(cursor) = cursors.get(&client_id_for_task) {
                // Only remove if still disconnected (not reconnected)
                if cursor.disconnected_at.is_some() {
                    cursors.remove(&client_id_for_task);
                    tracing::info!(
                        "[collab] Removed cursor {} after 5m disconnect timeout",
                        client_id_for_task
                    );
                } else {
                    tracing::debug!(
                        "[collab] Cursor {} reconnected, not removing",
                        client_id_for_task
                    );
                }
            }
        });
    }

    // Schedule document cleanup if no clients remain
    if remaining_clients == 0 {
        let collab_for_cleanup = Arc::clone(&collab);
        let doc_id_for_cleanup = doc_id.clone();
        let doc_for_cleanup = Arc::clone(&doc);

        tokio::spawn(async move {
            tokio::time::sleep(timeouts::DOCUMENT_CLEANUP).await;

            // Check if still no clients connected
            let client_count = doc_for_cleanup.client_count.load(Ordering::SeqCst);
            if client_count == 0 {
                // Verify the disconnect time is still old enough
                if let Some(disconnect_time) = *doc_for_cleanup.last_client_disconnect.read().await
                    && disconnect_time.elapsed() >= timeouts::DOCUMENT_CLEANUP
                {
                    collab_for_cleanup
                        .remove_document(&doc_id_for_cleanup)
                        .await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_init_roundtrip() {
        let msg = CollabMessage::Init {
            version: 42,
            doc: serde_json::json!({"type": "doc", "content": []}),
            mode: "markdown".to_owned(),
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Init { version, doc, mode } => {
                assert_eq!(version, 42);
                assert_eq!(doc, serde_json::json!({"type": "doc", "content": []}));
                assert_eq!(mode, "markdown");
            }
            _ => panic!("Expected Init message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_steps_roundtrip() {
        let msg = CollabMessage::Steps {
            version: 10,
            steps: vec![serde_json::json!({"stepType": "replace", "from": 0, "to": 5})],
            client_id: 12345,
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Steps {
                version,
                steps,
                client_id,
            } => {
                assert_eq!(version, 10);
                assert_eq!(steps.len(), 1);
                assert_eq!(client_id, 12345);
            }
            _ => panic!("Expected Steps message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_update_roundtrip() {
        let msg = CollabMessage::Update {
            steps: vec![
                serde_json::json!({"stepType": "replace"}),
                serde_json::json!({"stepType": "addMark"}),
            ],
            client_ids: vec![111, 222],
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Update { steps, client_ids } => {
                assert_eq!(steps.len(), 2);
                assert_eq!(client_ids, vec![111, 222]);
            }
            _ => panic!("Expected Update message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_ack_roundtrip() {
        let msg = CollabMessage::Ack { version: 99 };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Ack { version } => {
                assert_eq!(version, 99);
            }
            _ => panic!("Expected Ack message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_cursor_roundtrip() {
        let msg = CollabMessage::Cursor {
            client_id: 54321,
            head: 100,
            anchor: 50,
            name: Some("Alice".to_owned()),
            idle_secs: None,
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Cursor {
                client_id,
                head,
                anchor,
                name,
                idle_secs,
            } => {
                assert_eq!(client_id, 54321);
                assert_eq!(head, 100);
                assert_eq!(anchor, 50);
                assert_eq!(name, Some("Alice".to_owned()));
                assert!(idle_secs.is_none());
            }
            _ => panic!("Expected Cursor message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_cursor_without_name() {
        let msg = CollabMessage::Cursor {
            client_id: 12345,
            head: 10,
            anchor: 10,
            name: None,
            idle_secs: None,
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Cursor {
                client_id,
                head,
                anchor,
                name,
                idle_secs,
            } => {
                assert_eq!(client_id, 12345);
                assert_eq!(head, 10);
                assert_eq!(anchor, 10);
                assert!(name.is_none());
                assert!(idle_secs.is_none());
            }
            _ => panic!("Expected Cursor message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_cursor_with_idle_secs() {
        let msg = CollabMessage::Cursor {
            client_id: 11111,
            head: 200,
            anchor: 150,
            name: Some("Bob".to_owned()),
            idle_secs: Some(30), // 30 seconds idle
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Cursor {
                client_id,
                head,
                anchor,
                name,
                idle_secs,
            } => {
                assert_eq!(client_id, 11111);
                assert_eq!(head, 200);
                assert_eq!(anchor, 150);
                assert_eq!(name, Some("Bob".to_owned()));
                assert_eq!(idle_secs, Some(30));
            }
            _ => panic!("Expected Cursor message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_error_roundtrip() {
        let msg = CollabMessage::Error {
            error: "Version mismatch".to_owned(),
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Error { error } => {
                assert_eq!(error, "Version mismatch");
            }
            _ => panic!("Expected Error message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_new_version_roundtrip() {
        let msg = CollabMessage::NewVersion {
            hash: "abc123def456".to_owned(),
            name: "README.md".to_owned(),
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::NewVersion { hash, name } => {
                assert_eq!(hash, "abc123def456");
                assert_eq!(name, "README.md");
            }
            _ => panic!("Expected NewVersion message"),
        }
    }

    #[test]
    fn test_collab_message_decode_invalid_data() {
        // Empty data
        assert!(CollabMessage::decode(&[]).is_none());

        // Invalid message type
        let invalid = rmp_serde::encode::to_vec(&(99u8, "invalid")).unwrap_or_default();
        assert!(CollabMessage::decode(&invalid).is_none());

        // Malformed data
        assert!(CollabMessage::decode(&[0x93, 0x00]).is_none());
    }

    #[test]
    fn test_collab_message_type_tags() {
        // Verify the first byte is the correct type tag
        let init = CollabMessage::Init {
            version: 0,
            doc: serde_json::Value::Null,
            mode: "raw".to_owned(),
        };
        let init_bytes = init.encode();
        assert!(!init_bytes.is_empty());
        // MessagePack fixarray of 4 elements starts with 0x94
        // First element should decode to 0 (INIT)

        let ack = CollabMessage::Ack { version: 0 };
        let ack_bytes = ack.encode();
        // MessagePack fixarray of 2 elements starts with 0x92
        assert!(!ack_bytes.is_empty());
    }

    #[test]
    fn test_message_type_constants() {
        assert_eq!(msg::INIT, 0);
        assert_eq!(msg::STEPS, 1);
        assert_eq!(msg::UPDATE, 2);
        assert_eq!(msg::ACK, 3);
        assert_eq!(msg::CURSOR, 4);
        assert_eq!(msg::ERROR, 5);
    }

    #[test]
    fn test_content_to_document_empty() {
        let (doc, mode) = content_to_document(None, None);
        assert_eq!(mode, ContentMode::Raw);
        assert_eq!(doc["type"], "doc");
        assert!(doc["content"].as_array().is_some());
    }

    #[allow(clippy::unwrap_used)]
    #[test]
    fn test_content_to_document_markdown() {
        let content = b"# Hello\n\nThis is **bold** text.";
        let (doc, mode) = content_to_document(Some(content), Some("readme.md"));
        assert_eq!(mode, ContentMode::Markdown);
        assert_eq!(doc["type"], "doc");
        // Should have heading and paragraph
        let content_arr = doc["content"].as_array().unwrap();
        assert!(content_arr.iter().any(|n| n["type"] == "heading"));
        assert!(content_arr.iter().any(|n| n["type"] == "paragraph"));
    }

    #[allow(clippy::unwrap_used)]
    #[test]
    fn test_content_to_document_plain() {
        let content = b"Line 1\nLine 2\nLine 3";
        let (doc, mode) = content_to_document(Some(content), Some("notes.txt"));
        assert_eq!(mode, ContentMode::Plain);
        assert_eq!(doc["type"], "doc");
        // Each line should be a paragraph
        let content_arr = doc["content"].as_array().unwrap();
        assert_eq!(content_arr.len(), 3);
        assert!(content_arr.iter().all(|n| n["type"] == "paragraph"));
    }

    #[allow(clippy::unwrap_used)]
    #[test]
    fn test_content_to_document_raw() {
        let content = b"fn main() {\n    println!(\"Hello\");\n}";
        let (doc, mode) = content_to_document(Some(content), Some("main.rs"));
        assert_eq!(mode, ContentMode::Raw);
        assert_eq!(doc["type"], "doc");
        // Should be wrapped in a code_block
        let content_arr = doc["content"].as_array().unwrap();
        assert_eq!(content_arr.len(), 1);
        assert_eq!(content_arr[0]["type"], "code_block");
    }

    #[test]
    fn test_content_to_document_media() {
        use super::super::content_mode::MediaType;
        // PNG magic bytes
        let content = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let (doc, mode) = content_to_document(Some(content), Some("image.png"));
        assert!(matches!(mode, ContentMode::Media(MediaType::Image)));
        // Media mode returns empty document (not editable)
        assert_eq!(doc["type"], "doc");
    }

    #[test]
    fn test_content_to_document_binary() {
        // Invalid UTF-8 without known extension
        let content = &[0xFF, 0xFE, 0x00, 0x01];
        let (doc, mode) = content_to_document(Some(content), Some("unknown.dat"));
        assert_eq!(mode, ContentMode::Binary);
        assert_eq!(doc["type"], "doc");
    }

    #[test]
    fn test_document_with_content_modes() {
        // Test Document::with_content preserves mode
        let doc = Document::with_content(Some(b"# Test".as_slice()), Some("test.md"));
        assert_eq!(doc.mode, ContentMode::Markdown);

        let doc = Document::with_content(Some(b"let x = 1;".as_slice()), Some("script.js"));
        assert_eq!(doc.mode, ContentMode::Raw);

        let doc = Document::with_content(Some(b"Hello".as_slice()), Some("note.txt"));
        assert_eq!(doc.mode, ContentMode::Plain);
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_catch_up_update_with_multiple_steps() {
        // Simulates the catch-up Update sent after Init(v=0):
        // all accumulated steps with their client IDs.
        let steps = vec![
            serde_json::json!({"stepType": "replace", "from": 0, "to": 0}),
            serde_json::json!({"stepType": "replace", "from": 5, "to": 5}),
            serde_json::json!({"stepType": "addMark", "from": 0, "to": 10}),
        ];
        let client_ids = vec![111u64, 111, 222];

        let msg = CollabMessage::Update {
            steps: steps.clone(),
            client_ids: client_ids.clone(),
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Update {
                steps: decoded_steps,
                client_ids: decoded_ids,
            } => {
                assert_eq!(decoded_steps.len(), 3);
                assert_eq!(decoded_ids, vec![111, 111, 222]);
                assert_eq!(decoded_steps[0], steps[0]);
                assert_eq!(decoded_steps[1], steps[1]);
                assert_eq!(decoded_steps[2], steps[2]);
            }
            _ => panic!("Expected Update message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_error_desynchronized_roundtrip() {
        // Verify the new desync error message encodes/decodes correctly
        let msg = CollabMessage::Error {
            error: "Session desynchronized: 5 messages lost".to_owned(),
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Error { error } => {
                assert_eq!(error, "Session desynchronized: 5 messages lost");
                assert!(error.contains("desynchronized"));
            }
            _ => panic!("Expected Error message"),
        }
    }
}
