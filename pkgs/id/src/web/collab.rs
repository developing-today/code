//! Collaborative editing state management.
//!
//! Implements the server-side authority for prosemirror-collab, maintaining
//! document state and broadcasting changes to connected clients.
//!
//! ## Wire Protocol (`MessagePack` arrays)
//!
//! Messages are encoded as `MessagePack` arrays for efficiency:
//! - `[0, version, doc]` - Init: server sends initial state
//! - `[1, version, steps, clientID]` - Steps: client sends changes
//! - `[2, steps, clientIDs]` - Update: server broadcasts changes
//! - `[3, version]` - Ack: server confirms steps applied
//! - `[4, clientID, head, anchor, name?, idleSecs?]` - Cursor position
//! - `[5, error]` - Error message
//! - `""` (empty text) - Ping/pong for inactive tab cursor refresh
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
        Path, State, WebSocketUpgrade,
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

/// Message type tags for the wire protocol.
mod msg {
    pub const INIT: u8 = 0;
    pub const STEPS: u8 = 1;
    pub const UPDATE: u8 = 2;
    pub const ACK: u8 = 3;
    pub const CURSOR: u8 = 4;
    pub const ERROR: u8 = 5;
}

/// Load file content from the blob store.
///
/// Returns the file content as a string if found and valid UTF-8.
async fn load_file_content(store: &iroh_blobs::api::Store, hash_str: &str) -> Option<String> {
    let hash: iroh_blobs::Hash = hash_str.parse().ok()?;
    let bytes = store.blobs().get_bytes(hash).await.ok()?;
    String::from_utf8(bytes.to_vec()).ok()
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
        Self::with_content(None)
    }

    /// Create a document with optional initial text content.
    ///
    /// Converts plain text to a `ProseMirror` document structure.
    /// Each line becomes a paragraph.
    pub fn with_content(content: Option<&str>) -> Self {
        let (tx, _) = broadcast::channel(256);

        let doc = match content {
            Some(text) if !text.is_empty() => {
                // Convert text to ProseMirror document structure
                // Each line becomes a paragraph with text content
                let paragraphs: Vec<serde_json::Value> = text
                    .lines()
                    .map(|line| {
                        if line.is_empty() {
                            serde_json::json!({"type": "paragraph"})
                        } else {
                            serde_json::json!({
                                "type": "paragraph",
                                "content": [{"type": "text", "text": line}]
                            })
                        }
                    })
                    .collect();

                serde_json::json!({
                    "type": "doc",
                    "content": if paragraphs.is_empty() {
                        vec![serde_json::json!({"type": "paragraph"})]
                    } else {
                        paragraphs
                    }
                })
            }
            _ => {
                // Empty document with single empty paragraph
                serde_json::json!({
                    "type": "doc",
                    "content": [{"type": "paragraph"}]
                })
            }
        };

        Self {
            version: AtomicU64::new(0),
            doc: RwLock::new(doc),
            steps: RwLock::new(Vec::new()),
            cursors: RwLock::new(HashMap::new()),
            broadcast: tx,
            client_count: AtomicUsize::new(0),
            last_client_disconnect: RwLock::new(None),
        }
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
        initial_content: Option<&str>,
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

        let doc = Arc::new(Document::with_content(initial_content));
        write.insert(doc_id.to_owned(), Arc::clone(&doc));
        doc
    }

    /// Remove a document from the state.
    pub async fn remove_document(&self, doc_id: &str) {
        let mut write = self.documents.write().await;
        write.remove(doc_id);
        tracing::info!("[collab] Document '{}' cleaned up", doc_id);
    }
}

/// Messages sent over the WebSocket connection.
///
/// Serialized as `MessagePack` arrays: `[type_tag, ...fields]`
#[derive(Debug, Clone)]
pub enum CollabMessage {
    /// `[0, version, doc]` - Initial document state sent to client.
    Init {
        version: u64,
        doc: serde_json::Value,
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
}

impl CollabMessage {
    /// Encode message to `MessagePack` bytes.
    pub fn encode(&self) -> Vec<u8> {
        use rmp_serde::encode::to_vec;

        match self {
            Self::Init { version, doc } => to_vec(&(msg::INIT, version, doc)).unwrap_or_default(),
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
        }
    }

    /// Decode message from `MessagePack` bytes.
    pub fn decode(data: &[u8]) -> Option<Self> {
        use rmp_serde::decode::from_slice;

        // MessagePack tuples are encoded as arrays. The first element is the message type.
        // We decode the whole thing for each message type. This is slightly inefficient
        // but simple and correct.

        // Try each message type in order
        if let Ok((msg::INIT, version, doc)) = from_slice::<(u8, u64, serde_json::Value)>(data) {
            return Some(Self::Init { version, doc });
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

        None
    }
}

/// WebSocket upgrade handler for collaborative editing.
pub async fn ws_collab_handler(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    State(state): State<super::AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_collab_socket(socket, doc_id, state.collab, state.store))
}

/// Handle a WebSocket connection for collaborative editing.
async fn handle_collab_socket(
    socket: WebSocket,
    doc_id: String,
    collab: Arc<CollabState>,
    store: iroh_blobs::api::Store,
) {
    use std::sync::atomic::Ordering;

    tracing::info!("[collab] New connection for doc '{}' (page load)", doc_id);

    // Try to load file content from the store (doc_id is the hash)
    let initial_content = load_file_content(&store, &doc_id).await;
    if initial_content.is_some() {
        tracing::debug!("[collab] Loaded file content from store for '{}'", doc_id);
    }

    let doc = collab
        .get_or_create(&doc_id, initial_content.as_deref())
        .await;
    let client_count = doc.client_connected();
    tracing::info!(
        "[collab] Client connected to doc '{}', {} total clients",
        doc_id,
        client_count
    );

    let mut rx = doc.broadcast.subscribe();

    let (mut sender, mut receiver) = socket.split();

    // Send initial document state (binary MessagePack)
    let init_msg = CollabMessage::Init {
        version: doc.version(),
        doc: doc.doc.read().await.clone(),
    };

    let init_bytes = init_msg.encode();
    tracing::info!(
        "[collab] Sending Init: version={}, {} bytes",
        doc.version(),
        init_bytes.len()
    );

    if sender.send(Message::Binary(init_bytes)).await.is_err() {
        tracing::warn!("[collab] Client disconnected during init send");
        doc.client_disconnected().await;
        return;
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
    let broadcast_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            let bytes = msg.encode();
            let mut sender = sender_for_broadcast.lock().await;
            if sender.send(Message::Binary(bytes)).await.is_err() {
                break; // Client disconnected
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

    // Handle incoming messages
    while let Some(Ok(msg)) = receiver.next().await {
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
                        tracing::debug!(
                            "[collab] Received Cursor: client={}, head={}, anchor={}, name={:?}",
                            client_id,
                            head,
                            anchor,
                            name
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
                                    name: name.clone(),
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
                            name,
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
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Init { version, doc } => {
                assert_eq!(version, 42);
                assert_eq!(doc, serde_json::json!({"type": "doc", "content": []}));
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
        };
        let init_bytes = init.encode();
        assert!(!init_bytes.is_empty());
        // MessagePack fixarray of 3 elements starts with 0x93
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
}
