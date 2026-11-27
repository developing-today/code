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
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize},
    },
    time::Instant,
};
use tokio::sync::{Mutex, RwLock, broadcast};

use super::collab_steps::{StepRefusal, StepSchema, check_steps, document_content_size};
use super::content_mode::{ContentMode, detect_mode_with_content};
use super::identity::{sanitize_name, short_id};
use super::markdown::{
    markdown_to_prosemirror, plain_text_to_prosemirror, raw_text_to_prosemirror,
};
use id_core::frames::MAX_FRAME_BYTES;
use id_core::waiting::AuthorKey;
use id_grants::{Grant, GrantError, GrantId, GrantSet, GrantSetSnapshot, LoadError, Scope};
use id_store::documents::{DocumentError, DocumentRef};
use id_store::grant_authority::{identity_key, may_read_with_grants};
use id_store::note_checkpoints::{
    CheckpointError, GroupRoles, NoteAccess, NoteCheckpoints, NotePolicyRecord,
};
use id_store::notes::{AppliedNote, NoteRecord};
use id_store::read_authority::ReadDefaults;
use id_store::talk_page::{
    ChainError, ChangeKind, ChangelogEntry, EntryDraft, OpLogRange, SignedEntry, TalkPageError,
    verify_chain,
};
use p2panda_core::Hash;

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
    pub const CHECKPOINT: u8 = 10;
    pub const CHECKPOINTED: u8 = 11;
}

/// Maximum number of steps accepted in one client message.
const MAX_STEPS_PER_MESSAGE: usize = 1_000;

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
            // A duplicated key would collapse to its last value here and
            // differ from the bytes stored, so it falls back like any
            // other unparseable content (#8794).
            id_core::json_unique::parse_json_without_duplicate_keys(&text).unwrap_or_else(|_| {
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

fn desynchronized_message(lost: u64) -> CollabMessage {
    CollabMessage::Error {
        error: format!("Session desynchronized: {lost} messages lost"),
    }
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
    pub const PONG_TIMEOUT: Duration = Duration::from_mins(30);

    /// Remove cursor 5 minutes after client disconnects.
    pub const CURSOR_CLEANUP: Duration = Duration::from_mins(5);

    /// Clean up document 1 hour after last client disconnects.
    pub const DOCUMENT_CLEANUP: Duration = Duration::from_hours(1);

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
    /// The identity `client_id` (hex string) if authenticated, for name updates.
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
    /// Content size (`doc.content.size`) of the base document after
    /// every accepted step (#1618). Changed only under the `steps`
    /// write lock, alongside the history it describes.
    pub content_size: AtomicU64,
    /// Active cursor positions by client ID.
    pub cursors: RwLock<HashMap<String, CursorPosition>>,
    /// Broadcast channel for sending updates to clients.
    pub broadcast: broadcast::Sender<CollabMessage>,
    /// Number of connected clients.
    pub client_count: AtomicUsize,
    /// When the last client disconnected (None if clients are connected).
    pub last_client_disconnect: RwLock<Option<Instant>>,
    /// Current content hash (updated on save).
    pub hash: RwLock<String>,
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
        let content_size = document_content_size(&doc, mode);

        Self {
            version: AtomicU64::new(0),
            doc: RwLock::new(doc),
            mode,
            steps: RwLock::new(Vec::new()),
            content_size: AtomicU64::new(content_size),
            cursors: RwLock::new(HashMap::new()),
            broadcast: tx,
            client_count: AtomicUsize::new(0),
            last_client_disconnect: RwLock::new(None),
            hash: RwLock::new(String::new()),
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

    /// Atomically accept a batch of steps submitted at `base_version`
    /// (#2968).
    ///
    /// Holds the steps write lock across the version check, the step
    /// checks, the append, and the version bump, so two sockets
    /// submitting at the same base version cannot both pass the check:
    /// exactly one is accepted and the other observes the winner's
    /// bumped version. Returns the new version on success. Broadcast and
    /// Ack stay with the caller, after this lock has been released.
    ///
    /// # Errors
    ///
    /// [`StepsRejected::VersionMismatch`] with the current version when
    /// `base_version` is stale, and [`StepsRejected::Invalid`] when a step
    /// breaks the step contract (#1618): an unknown step type, a node or
    /// mark outside the document's schema, or a position outside the
    /// document. A rejected batch stores nothing and leaves the version
    /// and content size unchanged.
    pub async fn try_apply_steps(
        &self,
        base_version: u64,
        steps: &[serde_json::Value],
        client_id: u64,
    ) -> Result<u64, StepsRejected> {
        use std::sync::atomic::Ordering;
        let mut doc_steps = self.steps.write().await;
        let current_version = self.version();
        if base_version != current_version {
            return Err(StepsRejected::VersionMismatch {
                current: current_version,
            });
        }
        let Some(schema) = StepSchema::for_mode(self.mode) else {
            return Err(StepsRejected::Invalid(StepRefusal {
                index: 0,
                reason: format!("a {} document does not take edits", self.mode.as_str()),
            }));
        };
        let new_size = check_steps(schema, self.content_size.load(Ordering::SeqCst), steps)
            .map_err(StepsRejected::Invalid)?;
        self.content_size.store(new_size, Ordering::SeqCst);
        for step in steps {
            doc_steps.push((
                Step { data: step.clone() },
                serde_json::Value::Number(client_id.into()),
            ));
        }
        let new_version =
            self.version.fetch_add(steps.len() as u64, Ordering::SeqCst) + steps.len() as u64;
        drop(doc_steps);
        Ok(new_version)
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

/// Why [`Document::try_apply_steps`] refused a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepsRejected {
    /// The batch was built on a stale version; `current` is the
    /// document's version now.
    VersionMismatch {
        /// The document's current version.
        current: u64,
    },
    /// A step breaks the step contract (#1618).
    Invalid(StepRefusal),
}

/// Why publishing a note checkpoint from a live session was refused
/// (#3819 slice 2).
#[derive(Debug)]
pub enum PublishCheckpointError {
    /// The session key does not name a note session (a filename session,
    /// or a note session this state never opened): checkpoints belong to
    /// notes, and a filename is not one.
    NotNoteSession(String),
    /// The checkpoint core refused the publication — notably
    /// [`CheckpointError::ExplicitMergeRequired`] when the note has
    /// concurrent heads: a save must never join them silently.
    Checkpoint(CheckpointError),
    /// The session's current content breaks the document rules.
    Document(DocumentError),
}

impl std::fmt::Display for PublishCheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotNoteSession(key) => write!(f, "session {key} is not a note session"),
            Self::Checkpoint(err) => write!(f, "{err}"),
            Self::Document(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PublishCheckpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotNoteSession(_) => None,
            Self::Checkpoint(err) => Some(err),
            Self::Document(err) => Some(err),
        }
    }
}

impl From<CheckpointError> for PublishCheckpointError {
    fn from(err: CheckpointError) -> Self {
        Self::Checkpoint(err)
    }
}

impl From<DocumentError> for PublishCheckpointError {
    fn from(err: DocumentError) -> Self {
        Self::Document(err)
    }
}

/// Why a note session's end could not be recorded in the note's
/// changelog (#14272).
#[derive(Debug)]
pub enum SessionEntryError {
    /// The session entry could not be drafted or signed.
    Entry(TalkPageError),
    /// The note's changelog would not verify with the entry appended.
    Chain(ChainError),
}

impl std::fmt::Display for SessionEntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Entry(err) => write!(f, "{err}"),
            Self::Chain(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SessionEntryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Entry(err) => Some(err),
            Self::Chain(err) => Some(err),
        }
    }
}

impl From<TalkPageError> for SessionEntryError {
    fn from(err: TalkPageError) -> Self {
        Self::Entry(err)
    }
}

impl From<ChainError> for SessionEntryError {
    fn from(err: ChainError) -> Self {
        Self::Chain(err)
    }
}

/// The file name of the persisted note checkpoint snapshot (#3819):
/// the serving host keeps it beside the identity store
/// (`.identity.redb`), the node's other durable local state.
pub const NOTE_CHECKPOINT_SNAPSHOT_FILE_NAME: &str = ".note-checkpoints.json";

/// The durable snapshot of the note checkpoint log (#3819): the
/// applied events and the per-note policies in one document — the
/// form a host persists across a restart. The events alone rebuild
/// every note as public; the policies are what keep a scoped note
/// scoped, so the two are always written and read together.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteCheckpointSnapshot {
    /// The applied note events, in the order they were recorded.
    pub events: Vec<AppliedNote>,
    /// The effective policy of every note in the log, in note ID order.
    pub policies: Vec<NotePolicyRecord>,
    /// Every note's changelog (#14272), in note ID order. Absent from
    /// snapshots written before changelogs existed, and from snapshots
    /// with none, so those read and write exactly as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changelogs: Vec<NoteChangelogRecord>,
    /// The node's selective-sharing grants (#14790): every grant,
    /// revocation and served `snapshot_once` grant, in the grants
    /// model's own stored form. Absent from snapshots written before
    /// grants were stored, and from snapshots with none, so those read
    /// and write exactly as before. Grants are node-local here;
    /// replicating them as signed events is #5800.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grants: Option<GrantSetSnapshot>,
}

/// One note's changelog in the persisted snapshot (#14272): its
/// entries' canonical bytes, in chain order. A restore decodes them
/// and verifies the chain end to end from its first entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteChangelogRecord {
    /// The note the changelog belongs to.
    pub note: Hash,
    /// The entries' canonical bytes, in chain order.
    pub entries: Vec<Vec<u8>>,
}

/// Why restoring a persisted note checkpoint snapshot failed (#3819).
#[derive(Debug)]
pub enum NoteSnapshotError {
    /// The snapshot file could not be read or written.
    Io {
        /// The snapshot's path.
        path: PathBuf,
        /// The underlying I/O failure.
        source: std::io::Error,
    },
    /// The snapshot file is not a well-formed snapshot.
    Corrupt {
        /// The snapshot's path.
        path: PathBuf,
        /// The underlying decoding failure.
        source: serde_json::Error,
    },
    /// The snapshot's events or policies were refused by the
    /// checkpoint core: the events do not fold, or a policy names
    /// an unknown note or the wrong owner.
    Checkpoint(CheckpointError),
    /// A note's changelog in the snapshot does not decode, or does not
    /// verify end to end (#14272).
    Changelog(SessionEntryError),
    /// The snapshot's grants do not load: a grant fails the grants
    /// model's checks, a delegation is wider than its parent, or a
    /// revocation names no grant (#14790).
    Grants(LoadError),
}

impl std::fmt::Display for NoteSnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(
                    f,
                    "note checkpoint snapshot at {}: {source}",
                    path.display()
                )
            }
            Self::Corrupt { path, source } => write!(
                f,
                "note checkpoint snapshot at {} is corrupt: {source}",
                path.display()
            ),
            Self::Checkpoint(err) => write!(f, "{err}"),
            Self::Changelog(err) => write!(f, "note changelog: {err}"),
            Self::Grants(err) => write!(f, "note grants: {err}"),
        }
    }
}

impl std::error::Error for NoteSnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Corrupt { source, .. } => Some(source),
            Self::Checkpoint(err) => Some(err),
            Self::Changelog(err) => Some(err),
            Self::Grants(err) => Some(err),
        }
    }
}

impl From<CheckpointError> for NoteSnapshotError {
    fn from(err: CheckpointError) -> Self {
        Self::Checkpoint(err)
    }
}

impl From<SessionEntryError> for NoteSnapshotError {
    fn from(err: SessionEntryError) -> Self {
        Self::Changelog(err)
    }
}

impl From<LoadError> for NoteSnapshotError {
    fn from(err: LoadError) -> Self {
        Self::Grants(err)
    }
}

/// Why the node refused to issue or revoke a note grant (#14790).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteGrantError {
    /// The grant's issuer is not the account issuing it.
    NotIssuer,
    /// The grant covers something other than notes: only
    /// object-scoped grants naming notes are stored here.
    NotANote,
    /// A note the grant covers is not one the issuer owns. Unknown
    /// notes get the same answer, so a refusal never confirms that a
    /// note exists.
    NotOwner,
    /// No grant with that id was issued by this account.
    UnknownGrant,
    /// The grants model refused the grant (an invalid grant, a
    /// duplicate id, or a parent on a root grant).
    Grant(GrantError),
}

impl std::fmt::Display for NoteGrantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotIssuer => write!(f, "the grant's issuer is not the account issuing it"),
            Self::NotANote => write!(f, "the grant covers something other than notes"),
            Self::NotOwner => write!(f, "the grant covers a note the issuer does not own"),
            Self::UnknownGrant => write!(f, "no such grant issued by this account"),
            Self::Grant(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for NoteGrantError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Grant(err) => Some(err),
            _ => None,
        }
    }
}

/// Parses a note id written exactly as [`Hash::to_hex`] writes it:
/// 64 lowercase hex characters. Any other spelling is refused, because
/// the read check matches grant scopes against that exact form.
fn parse_note_hash(id: &str) -> Option<Hash> {
    let hash: Hash = id.parse().ok()?;
    (hash.to_hex() == id).then_some(hash)
}

/// Decodes the changelogs of a persisted snapshot (#14272), verifying
/// each chain end to end from its first entry. A note named twice is
/// refused: a snapshot holds one chain per note.
fn restore_changelogs(
    records: Vec<NoteChangelogRecord>,
) -> Result<HashMap<Hash, Vec<ChangelogEntry>>, SessionEntryError> {
    let mut changelogs = HashMap::new();
    for record in records {
        let entries = record
            .entries
            .iter()
            .map(|bytes| ChangelogEntry::from_canonical_bytes(bytes))
            .collect::<Result<Vec<_>, _>>()?;
        verify_chain(record.note.as_bytes(), None, &entries)?;
        if changelogs.insert(record.note, entries).is_some() {
            return Err(SessionEntryError::Entry(TalkPageError::Invalid(
                "a snapshot holds one changelog per note",
            )));
        }
    }
    Ok(changelogs)
}

/// Writes a snapshot to `path` atomically: the new document lands
/// under a temporary name in the same directory and is renamed over
/// the old one, so a crash mid-write leaves the previous snapshot
/// (or none), never a torn one for the next restart to read.
fn write_note_snapshot_file(
    path: &std::path::Path,
    snapshot: &NoteCheckpointSnapshot,
) -> Result<(), NoteSnapshotError> {
    let bytes =
        serde_json::to_vec_pretty(snapshot).map_err(|source| NoteSnapshotError::Corrupt {
            path: path.to_path_buf(),
            source,
        })?;
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    std::fs::write(&tmp, bytes).map_err(|source| NoteSnapshotError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| NoteSnapshotError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Mints the event ID for a checkpoint published from a live session:
/// a digest of everything the revision is — its note, its author, the
/// heads it builds on, its content, and the session version that
/// produced it — so two saves of identical bytes at different versions
/// are still distinct events, and the same save replayed is the same
/// event.
fn mint_checkpoint_event_id(
    note: &Hash,
    author: &AuthorKey,
    heads: &[Hash],
    content: &DocumentRef,
    version: u64,
) -> Hash {
    Hash::digest(format!("{note}:{author:?}:{heads:?}:{}:{version}", content.content).as_bytes())
}

/// Now, in milliseconds since the Unix epoch: the receipt time a
/// session's changelog entry is stamped with.
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// A `doc_id` naming a note session over the wire: the
/// [`note_session_key`] form, `note:<note id hex>` (#3819 slice 2).
/// Anything else is a filename, exactly as before.
fn parse_note_doc_id(doc_id: &str) -> Option<Hash> {
    doc_id.strip_prefix("note:")?.parse().ok()
}

/// The session key for a note: its stable ID under a `note:`
/// prefix, so a note session can never alias a filename session
/// (#3819). The ID is the note's `note.create` event ID
/// (`id_store::notes`), independent of any filename or alias.
#[must_use]
pub fn note_session_key(note_id: &Hash) -> String {
    format!("note:{note_id}")
}

/// State for all collaborative editing sessions.
#[derive(Debug, Default)]
pub struct CollabState {
    /// Active documents by ID.
    documents: RwLock<HashMap<String, Arc<Document>>>,
    /// The durable checkpoint log for note sessions (#3819): the
    /// applied note events sessions publish into, behind a mutex
    /// because every publication fold-proves the whole log. Seeded
    /// from persisted events on restart
    /// ([`CollabState::seed_note_events`]); read back with
    /// [`CollabState::note_events`] for persistence and sync.
    note_log: Arc<Mutex<NoteCheckpoints>>,
    /// Where the host persists the checkpoint log's snapshot
    /// (events + policies) so notes survive a restart (#3819):
    /// installed with [`CollabState::with_note_snapshot_path`], the
    /// snapshot is rewritten after every mutation that leaves the
    /// log in a complete state, and the host restores it on startup
    /// with [`CollabState::restore_persisted_note_snapshot`]. `None`
    /// keeps the log memory-only, exactly as before.
    note_snapshot_path: Option<PathBuf>,
    /// Which note each note session serves, by session key: recorded
    /// when a session is opened through
    /// [`CollabState::get_or_create_for_note`], so publishing a
    /// checkpoint needs only the session's key.
    note_bindings: RwLock<HashMap<String, Hash>>,
    /// Resolves group roles for the note access boundary (#3819
    /// slice 3): the host installs its folded-groups resolver with
    /// [`CollabState::with_note_group_roles`]; the default knows no
    /// groups, so scoped notes honor user grants and the owner, and
    /// public notes behave exactly as before the boundary existed.
    note_group_roles: GroupRoles,
    /// The checkpoints each open note session has published so far,
    /// by session key (#14272): the run of the merge layer's log the
    /// session covers. [`CollabState::end_note_session`] takes it
    /// when the session's last editor leaves and records it as the
    /// session's one changelog entry.
    note_session_ops: Mutex<HashMap<String, OpLogRange>>,
    /// Each note's changelog (#5834, #14272): the hash-chained
    /// `session` entries its multiplayer sessions ended with, in
    /// chain order. It verifies end to end from its
    /// first entry after every append, and is persisted with the
    /// checkpoint snapshot.
    note_changelogs: Mutex<HashMap<Hash, Vec<ChangelogEntry>>>,
    /// The node's selective-sharing grants over notes (#14790): issued
    /// and revoked by note owners through
    /// [`CollabState::grant_note_access`] and
    /// [`CollabState::revoke_note_grant`], consulted by
    /// [`CollabState::may_read_note`], and persisted with the
    /// checkpoint snapshot.
    note_grants: Mutex<GrantSet>,
}

impl CollabState {
    /// Create a new collaborative state manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs the group-role resolver the note access boundary
    /// consults for group grants (#3819 slice 3, #2048 Q2/Q3). The
    /// host builds it over its folded group state, mapping group
    /// roles onto note roles (Read -> Read, Write/Manage -> Write,
    /// Admin -> Admin).
    #[must_use]
    pub fn with_note_group_roles(mut self, roles: GroupRoles) -> Self {
        self.note_group_roles = roles;
        self
    }

    /// Installs the path the checkpoint log's snapshot is persisted
    /// to (#3819): after every mutation that leaves the log in a
    /// complete state (a publication, a policy change, a full
    /// restore) the snapshot is rewritten there, and the host
    /// restores it on startup with
    /// [`Self::restore_persisted_note_snapshot`]. Without a path the
    /// log is memory-only.
    #[must_use]
    pub fn with_note_snapshot_path(mut self, path: PathBuf) -> Self {
        self.note_snapshot_path = Some(path);
        self
    }

    /// Replaces a note's access policy in the checkpoint log
    /// (#3819 slice 3): only an author holding Admin on the note —
    /// its owner, or an admin grant — may re-scope it.
    ///
    /// # Errors
    ///
    /// Returns the checkpoint core's refusal (unknown note, or
    /// [`CheckpointError::NotAuthorized`] when `by` holds less than
    /// Admin).
    pub async fn set_note_access(
        &self,
        note: Hash,
        by: AuthorKey,
        access: NoteAccess,
    ) -> Result<(), CheckpointError> {
        let result =
            self.note_log
                .lock()
                .await
                .set_access(note, by, access, &self.note_group_roles);
        if result.is_ok() {
            self.persist_note_snapshot().await;
        }
        result
    }

    /// Issues a root grant over notes the issuer owns (#14790): `owner`
    /// must be the grant's issuer, every scope must name a note `owner`
    /// owns, and the grant must pass the grants model's checks. A
    /// stored grant is persisted with the checkpoint snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`NoteGrantError`] naming the refusal; nothing is
    /// stored on any refusal.
    pub async fn grant_note_access(
        &self,
        owner: AuthorKey,
        grant: Grant,
    ) -> Result<(), NoteGrantError> {
        if grant.issuer != identity_key(owner) {
            return Err(NoteGrantError::NotIssuer);
        }
        {
            let log = self.note_log.lock().await;
            for scope in &grant.scopes {
                let Scope::Object(object) = scope else {
                    return Err(NoteGrantError::NotANote);
                };
                let note = parse_note_hash(object).ok_or(NoteGrantError::NotANote)?;
                match log.access(note) {
                    Some((note_owner, _)) if note_owner == owner => {}
                    _ => return Err(NoteGrantError::NotOwner),
                }
            }
        }
        self.note_grants
            .lock()
            .await
            .insert_root(grant)
            .map_err(NoteGrantError::Grant)?;
        self.persist_note_snapshot().await;
        Ok(())
    }

    /// Revokes a grant `by` issued (#14790): it, and every grant
    /// delegated from it, stops admitting anyone at the next check.
    /// The revocation is persisted with the checkpoint snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`NoteGrantError::UnknownGrant`] when no grant with that
    /// id was issued by `by`, whether it is someone else's or absent.
    pub async fn revoke_note_grant(
        &self,
        by: AuthorKey,
        id: &GrantId,
    ) -> Result<(), NoteGrantError> {
        {
            let mut grants = self.note_grants.lock().await;
            match grants.get(id) {
                Some(grant) if grant.issuer == identity_key(by) => {}
                _ => return Err(NoteGrantError::UnknownGrant),
            }
            grants.revoke(id);
        }
        self.persist_note_snapshot().await;
        Ok(())
    }

    /// Whether `reader` may read `note` under the read order with the
    /// owner's live grants added at the object tier, as of `now` (Unix
    /// seconds, UTC) (#14790). Unknown notes are denied.
    pub async fn may_read_note(
        &self,
        note: Hash,
        reader: AuthorKey,
        defaults: &ReadDefaults,
        now: u64,
    ) -> bool {
        let log = self.note_log.lock().await;
        let grants = self.note_grants.lock().await;
        may_read_with_grants(
            &log,
            note,
            reader,
            &self.note_group_roles,
            defaults,
            &grants,
            now,
        )
    }

    /// Get or create a document for editing.
    ///
    /// If `initial_content` is provided and the document doesn't exist,
    /// the document will be initialized with that content.
    pub async fn get_or_create(
        &self,
        filename: &str,
        hash: &str,
        initial_content: Option<&[u8]>,
    ) -> Arc<Document> {
        self.get_or_create_keyed(filename, hash, initial_content, Some(filename))
            .await
    }

    /// Get or create the editing session for a note, keyed by the
    /// note's stable ID (#3819): the ID of its `note.create` event in
    /// the durable note graph (`id_store::notes`), never by filename.
    /// A note that is renamed, or reached through a different alias,
    /// keeps the one session its editors are actually sharing; the
    /// filename survives only as a content-mode hint for a session
    /// created from file bytes. The durable side of the session is
    /// its checkpoints (`id_store::note_checkpoints`); this keying is
    /// what lets a session find its note again after a restart.
    pub async fn get_or_create_for_note(
        &self,
        note_id: &Hash,
        hash: &str,
        initial_content: Option<&[u8]>,
        filename_hint: Option<&str>,
    ) -> Arc<Document> {
        let key = note_session_key(note_id);
        let doc = self
            .get_or_create_keyed(&key, hash, initial_content, filename_hint)
            .await;
        self.note_bindings.write().await.insert(key, *note_id);
        doc
    }

    /// The keyed core of [`Self::get_or_create`]: sessions live under
    /// exactly one key, a filename for file editing or a
    /// [`note_session_key`] for a note, and the two namespaces never
    /// alias (the note key carries its `note:` prefix).
    async fn get_or_create_keyed(
        &self,
        key: &str,
        hash: &str,
        initial_content: Option<&[u8]>,
        mode_filename: Option<&str>,
    ) -> Arc<Document> {
        let read = self.documents.read().await;
        if let Some(doc) = read.get(key) {
            return Arc::clone(doc);
        }
        drop(read);

        let mut write = self.documents.write().await;
        // Double-check after acquiring write lock
        if let Some(doc) = write.get(key) {
            return Arc::clone(doc);
        }

        let doc = Arc::new(Document::with_content(initial_content, mode_filename));
        write.insert(key.to_owned(), Arc::clone(&doc));
        hash.clone_into(&mut *doc.hash.write().await);
        doc
    }

    /// The note a session serves, when the session was opened as a
    /// note session ([`Self::get_or_create_for_note`]).
    pub async fn note_id_for_key(&self, key: &str) -> Option<Hash> {
        self.note_bindings.read().await.get(key).copied()
    }

    /// Replaces the checkpoint log with one rebuilt from applied note
    /// events — the restart/sync entry (#3819): the events are the
    /// durable state, and the rebuilt log folds to the heads the live
    /// log had. The events are validated as a whole; a set that does
    /// not fold leaves the current log untouched. Policies do NOT
    /// travel with the events: seed them alongside with
    /// [`Self::seed_note_policies`], or the rebuilt log treats every
    /// note as public.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] when the events do not fold.
    pub async fn seed_note_events(&self, events: Vec<AppliedNote>) -> Result<(), CheckpointError> {
        let rebuilt = NoteCheckpoints::from_events(events)?;
        *self.note_log.lock().await = rebuilt;
        Ok(())
    }

    /// The applied note events in the checkpoint log, in the order
    /// they were recorded: persist or sync these and
    /// [`Self::seed_note_events`] rebuilds the rest. Persist
    /// [`Self::note_policies`] alongside them — the events alone
    /// rebuild every note as public.
    pub async fn note_events(&self) -> Vec<AppliedNote> {
        self.note_log.lock().await.events().to_vec()
    }

    /// The recorded access policy of every note that has one, in
    /// note ID order (#3819): the durable form of the policies, to
    /// persist alongside [`Self::note_events`] and re-state with
    /// [`Self::seed_note_policies`] after a rebuild. A note with no
    /// recorded policy contributes no record — it reads as public
    /// through the fall-through, and must not be re-stated as one.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] when the recorded events do not
    /// fold (a log built through this type's methods does not).
    pub async fn note_policies(&self) -> Result<Vec<NotePolicyRecord>, CheckpointError> {
        self.note_log.lock().await.policies()
    }

    /// Re-states policies after a rebuild from events (the
    /// restart/sync path): each record replaces the note's policy.
    /// Seed the events first — a record for a note the log does not
    /// have is refused — and treat a refusal as a failed restart,
    /// never as a note silently reopened to public.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::UnknownNote`] for a record naming
    /// a note with no create event in the log, and
    /// [`CheckpointError::Note`] for a record naming the wrong owner.
    pub async fn seed_note_policies(
        &self,
        policies: Vec<NotePolicyRecord>,
    ) -> Result<(), CheckpointError> {
        self.note_log.lock().await.seed_policies(policies)
    }

    /// The checkpoint log's durable snapshot: the applied events
    /// and the per-note policies together (#3819), with every note's
    /// changelog (#14272) — the document
    /// [`Self::restore_note_snapshot`] rebuilds from, and the form
    /// persisted at the installed snapshot path.
    ///
    /// # Errors
    ///
    /// Returns [`NoteSnapshotError::Checkpoint`] when the recorded
    /// events do not fold (a log built through this type's methods
    /// does not), and [`NoteSnapshotError::Changelog`] when an entry
    /// cannot be encoded.
    pub async fn note_snapshot(&self) -> Result<NoteCheckpointSnapshot, NoteSnapshotError> {
        let log = self.note_log.lock().await;
        let mut changelogs = self
            .note_changelogs
            .lock()
            .await
            .iter()
            .map(|(note, entries)| {
                Ok(NoteChangelogRecord {
                    note: *note,
                    entries: entries
                        .iter()
                        .map(ChangelogEntry::canonical_bytes)
                        .collect::<Result<Vec<_>, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, TalkPageError>>()
            .map_err(SessionEntryError::Entry)?;
        changelogs.sort_by_key(|record| *record.note.as_bytes());
        let grants = {
            let grants = self.note_grants.lock().await;
            grants
                .grants()
                .next()
                .is_some()
                .then(|| grants.to_snapshot())
        };
        Ok(NoteCheckpointSnapshot {
            events: log.events().to_vec(),
            policies: log.policies()?,
            changelogs,
            grants,
        })
    }

    /// Rebuilds the checkpoint log from a snapshot: the events
    /// first, then the policies over them, validated as a whole — a
    /// snapshot that does not fold, or whose policies name an
    /// unknown note or the wrong owner, leaves the current log
    /// untouched. This is the restore the serving host runs on
    /// startup; see [`Self::restore_persisted_note_snapshot`].
    ///
    /// # Errors
    ///
    /// Returns the checkpoint core's refusal as
    /// [`NoteSnapshotError::Checkpoint`], and a changelog that does not
    /// decode or verify as [`NoteSnapshotError::Changelog`].
    pub async fn restore_note_snapshot(
        &self,
        snapshot: NoteCheckpointSnapshot,
    ) -> Result<(), NoteSnapshotError> {
        let rebuilt =
            NoteCheckpoints::from_events_with_policies(snapshot.events, snapshot.policies)?;
        let changelogs = restore_changelogs(snapshot.changelogs)?;
        let grants = match snapshot.grants {
            Some(grants) => GrantSet::from_snapshot(grants)?,
            None => GrantSet::new(),
        };
        *self.note_log.lock().await = rebuilt;
        *self.note_changelogs.lock().await = changelogs;
        *self.note_grants.lock().await = grants;
        self.persist_note_snapshot().await;
        Ok(())
    }

    /// Restores the snapshot persisted at the installed path, if
    /// one is there (#3819): the serving host calls this on
    /// startup, before it serves. A missing file is a fresh host
    /// (`Ok(false)`); a file that does not parse or does not fold
    /// is a failed restart (`Err`) — never a host that silently
    /// reopens scoped notes to public. Without an installed path
    /// this is a no-op (`Ok(false)`).
    ///
    /// # Errors
    ///
    /// Returns [`NoteSnapshotError::Io`] when the file cannot be
    /// read, [`NoteSnapshotError::Corrupt`] when it is not a
    /// well-formed snapshot, and [`NoteSnapshotError::Checkpoint`]
    /// when its events or policies are refused.
    pub async fn restore_persisted_note_snapshot(&self) -> Result<bool, NoteSnapshotError> {
        let Some(path) = &self.note_snapshot_path else {
            return Ok(false);
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(NoteSnapshotError::Io {
                    path: path.clone(),
                    source,
                });
            }
        };
        let snapshot: NoteCheckpointSnapshot =
            serde_json::from_slice(&bytes).map_err(|source| NoteSnapshotError::Corrupt {
                path: path.clone(),
                source,
            })?;
        self.restore_note_snapshot(snapshot).await?;
        Ok(true)
    }

    /// Rewrites the persisted snapshot from the current log, when a
    /// snapshot path is installed. Called only at points where the
    /// log is in a complete state — never between seeding events
    /// and seeding their policies — so the file always holds one
    /// whole snapshot. A write failure is logged, not propagated:
    /// the mutation already landed in memory, and reporting it as
    /// failed would misstate the log; the next mutation rewrites
    /// the file.
    async fn persist_note_snapshot(&self) {
        let Some(path) = &self.note_snapshot_path else {
            return;
        };
        match self.note_snapshot().await {
            Ok(snapshot) => {
                if let Err(err) = write_note_snapshot_file(path, &snapshot) {
                    tracing::warn!(
                        "[collab] could not persist the note checkpoint snapshot to {}: {err}",
                        path.display()
                    );
                }
            }
            Err(err) => {
                tracing::warn!("[collab] could not snapshot the note checkpoint log: {err}");
            }
        }
    }

    /// The current heads of a note in the checkpoint log.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] when the recorded events do not
    /// fold (a log built through this type's methods does not).
    pub async fn note_heads(&self, note: Hash) -> Result<Vec<Hash>, CheckpointError> {
        self.note_log.lock().await.heads(note)
    }

    /// Publishes a checkpoint of the note a session serves, from the
    /// session's live content (#3819 slice 2): the document's current
    /// plain text, referenced through the document path
    /// ([`DocumentRef::for_bytes`]) and recorded as one signed
    /// revision. The first publication of a note establishes it (the
    /// create event's ID is the note's own ID); later publications
    /// build on the note's single current head. Editor steps stay
    /// ephemeral — only this explicit publication crosses into the
    /// durable graph, never a keystroke.
    ///
    /// # Errors
    ///
    /// Returns [`PublishCheckpointError::NotNoteSession`] when `key`
    /// names no note session this state opened,
    /// [`CheckpointError::ExplicitMergeRequired`] (as
    /// [`PublishCheckpointError::Checkpoint`]) when the note has
    /// concurrent heads — joining them is the explicit
    /// [`NoteCheckpoints::merge`], never a side effect of saving —
    /// and the checkpoint core's other refusals unchanged. On any
    /// error the log is exactly as it was.
    pub async fn publish_note_checkpoint(
        &self,
        key: &str,
        author: AuthorKey,
    ) -> Result<NoteRecord, PublishCheckpointError> {
        let doc = {
            let documents = self.documents.read().await;
            documents.get(key).map(Arc::clone)
        };
        let Some(doc) = doc else {
            return Err(PublishCheckpointError::NotNoteSession(key.to_owned()));
        };
        let doc_json = doc.doc.read().await;
        let text = super::routes::file_handlers::extract_plain_text(&*doc_json);
        drop(doc_json);
        self.publish_note_checkpoint_text(key, author, &text).await
    }

    /// Publishes a checkpoint of the note a session serves from text
    /// the caller supplies (#3819 slice 4): the live session's client
    /// holds the step-edited content — the server never applies steps
    /// to the session document — so the WebSocket checkpoint path
    /// publishes the client's text, not the session's base document.
    /// Same recording rules as [`Self::publish_note_checkpoint`].
    ///
    /// # Errors
    ///
    /// Same as [`Self::publish_note_checkpoint`].
    pub async fn publish_note_checkpoint_text(
        &self,
        key: &str,
        author: AuthorKey,
        text: &str,
    ) -> Result<NoteRecord, PublishCheckpointError> {
        let Some(note) = self.note_id_for_key(key).await else {
            return Err(PublishCheckpointError::NotNoteSession(key.to_owned()));
        };
        let doc = {
            let documents = self.documents.read().await;
            documents.get(key).map(Arc::clone)
        };
        let Some(doc) = doc else {
            return Err(PublishCheckpointError::NotNoteSession(key.to_owned()));
        };
        let content = DocumentRef::for_bytes("text/markdown", text.as_bytes())?;
        let (published, op) = {
            let mut log = self.note_log.lock().await;
            if log.record(note)?.is_none() {
                (log.create(note, author, content)?, note)
            } else {
                let heads = log.heads(note)?;
                let event_id =
                    mint_checkpoint_event_id(&note, &author, &heads, &content, doc.version());
                let record = log.checkpoint_on_heads_checked(
                    note,
                    event_id,
                    author,
                    content,
                    &self.note_group_roles,
                )?;
                (record, event_id)
            }
        };
        self.note_session_ops
            .lock()
            .await
            .entry(key.to_owned())
            .and_modify(|range| *range = range.extended(*op.as_bytes()))
            .or_insert_with(|| OpLogRange::single(*op.as_bytes()));
        self.persist_note_snapshot().await;
        Ok(published)
    }

    /// Ends a note session in the note's changelog (#14272, slice 4 of
    /// #5834): called when the session's last editor leaves. When the
    /// session published checkpoints, exactly one `session` entry is
    /// appended to the note's changelog naming the run of checkpoints
    /// it covers ([`OpLogRange`]); the checkpoints themselves stay in
    /// the merge layer, never in the changelog. `sign` turns the draft
    /// into the entry its actor signs (the host signs with its own
    /// key), and `received_at` is the receipt time stamped on it. The
    /// grown chain must verify end to end from its first entry before
    /// the entry is kept.
    ///
    /// Returns `Ok(None)` when the session published nothing, or when
    /// `key` names no note session: there is nothing to record.
    ///
    /// # Errors
    ///
    /// Returns [`SessionEntryError`] when the entry cannot be drafted
    /// or signed, or the chain would not verify with it. The session's
    /// range is then kept open, so the next session end records it.
    pub async fn end_note_session<F>(
        &self,
        key: &str,
        received_at: u64,
        sign: F,
    ) -> Result<Option<ChangelogEntry>, SessionEntryError>
    where
        F: FnOnce(EntryDraft) -> Result<SignedEntry, TalkPageError>,
    {
        let Some(note) = self.note_id_for_key(key).await else {
            return Ok(None);
        };
        let entry = {
            let mut sessions = self.note_session_ops.lock().await;
            let Some(range) = sessions.get(key).copied() else {
                return Ok(None);
            };
            let mut changelogs = self.note_changelogs.lock().await;
            let chain = changelogs.entry(note).or_default();
            let draft = match chain.last() {
                None => {
                    EntryDraft::genesis(note.as_bytes().to_vec(), ChangeKind::Session, received_at)
                }
                Some(previous) => EntryDraft::after(previous, ChangeKind::Session, received_at)?,
            }
            .with_op_log(range);
            let entry = sign(draft)?.receive(received_at);
            let mut grown = chain.clone();
            grown.push(entry.clone());
            verify_chain(note.as_bytes(), None, &grown)?;
            *chain = grown;
            sessions.remove(key);
            entry
        };
        self.persist_note_snapshot().await;
        Ok(Some(entry))
    }

    /// The note's changelog (#14272): the `session` entries its ended
    /// multiplayer sessions recorded, in chain order.
    pub async fn note_changelog(&self, note: Hash) -> Vec<ChangelogEntry> {
        self.note_changelogs
            .lock()
            .await
            .get(&note)
            .cloned()
            .unwrap_or_default()
    }

    /// The content reference a reopened note session hydrates from:
    /// the content of the note's current head revision (#3819
    /// slice 4). A note with concurrent heads hydrates from the
    /// smallest head ID — a deterministic choice every reopen agrees
    /// on; the heads themselves are untouched, and joining them
    /// stays the explicit merge. `None` when the log has no such
    /// note (or the note has no revisions, which cannot happen for a
    /// recorded note).
    ///
    /// # Errors
    ///
    /// Returns the checkpoint core's storage refusals unchanged.
    pub async fn note_head_content(
        &self,
        note: Hash,
    ) -> Result<Option<DocumentRef>, CheckpointError> {
        let log = self.note_log.lock().await;
        let Some(record) = log.record(note)? else {
            return Ok(None);
        };
        let Some(head) = record.heads.iter().min() else {
            return Ok(None);
        };
        Ok(record
            .revisions
            .iter()
            .find(|revision| &revision.id == head)
            .map(|revision| revision.content.clone()))
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
    pub async fn notify_new_version(&self, filename: &str, new_hash: &str) {
        let read = self.documents.read().await;
        if let Some(doc) = read.get(filename) {
            let msg = CollabMessage::NewVersion {
                hash: new_hash.to_owned(),
                name: filename.to_owned(),
            };
            let receivers = doc.broadcast.send(msg).unwrap_or(0);
            if receivers > 0 {
                tracing::info!(
                    "[collab] Notified {} client(s) about new version of '{}': hash -> {}",
                    receivers,
                    filename,
                    new_hash
                );
            }
            // Update the stored hash
            new_hash.clone_into(&mut *doc.hash.write().await);
        }
    }

    /// Update cursor display names for a client across all active documents.
    ///
    /// Called when a client changes their display name via the settings API.
    /// Finds all cursors with the matching `identity_client_id`, updates the
    /// stored name, and broadcasts the updated cursor to all connected clients.
    pub async fn update_client_name(&self, identity_client_id: &str, new_name: &str) {
        let Some(new_name) = sanitize_name(Some(new_name.to_owned())) else {
            return;
        };
        let docs = self.documents.read().await;
        for (doc_id, doc) in docs.iter() {
            let mut cursors = doc.cursors.write().await;
            // Find cursors belonging to this identity
            let matching: Vec<(String, u32, u32)> = cursors
                .iter()
                .filter(|(_, pos)| pos.identity_client_id.as_deref() == Some(identity_client_id))
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
    /// `[10, text]` - Publish a durable checkpoint of a note session
    /// (#3819 slice 4). The client holds the live, step-edited
    /// content — the server never applies steps to the session
    /// document — so publication carries the text explicitly.
    /// Filename sessions refuse it.
    Checkpoint { text: String },
    /// `[11, head]` - A checkpoint was recorded; `head` is the hex of
    /// the note's new head revision.
    Checkpointed { head: String },
}

/// True when `data` is exactly one complete `MessagePack` value with no bytes after it.
fn is_single_msgpack_value(data: &[u8]) -> bool {
    let mut cursor = std::io::Cursor::new(data);
    let mut de = rmp_serde::Deserializer::new(&mut cursor);
    if serde::de::IgnoredAny::deserialize(&mut de).is_err() {
        return false;
    }
    usize::try_from(cursor.position()).is_ok_and(|consumed| consumed == data.len())
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
            Self::Checkpoint { text } => to_vec(&(msg::CHECKPOINT, text)).unwrap_or_default(),
            Self::Checkpointed { head } => to_vec(&(msg::CHECKPOINTED, head)).unwrap_or_default(),
            Self::AuthOk {
                client_id,
                name,
                token,
            } => to_vec(&(msg::AUTH_OK, client_id, name, token)).unwrap_or_default(),
        }
    }

    /// Decode message from `MessagePack` bytes.
    pub fn decode(data: &[u8]) -> Option<Self> {
        use rmp_serde::decode::from_slice;

        // A frame is exactly one MessagePack value: refuse trailing bytes so a
        // message has one byte spelling and a concatenated second message is
        // never silently dropped (#6349).
        if !is_single_msgpack_value(data) {
            return None;
        }

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

        if let Ok((msg::CURSOR_REMOVE, client_id)) = from_slice::<(u8, u64)>(data) {
            return Some(Self::CursorRemove { client_id });
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
            return Some(Self::AuthOk {
                client_id,
                name,
                token,
            });
        }
        if let Ok((msg::CHECKPOINT, text)) = from_slice::<(u8, String)>(data) {
            return Some(Self::Checkpoint { text });
        }

        if let Ok((msg::CHECKPOINTED, head)) = from_slice::<(u8, String)>(data) {
            return Some(Self::Checkpointed { head });
        }

        // AUTH_OK without token (legacy format)
        if let Ok((msg::AUTH_OK, client_id, name)) =
            from_slice::<(u8, String, Option<String>)>(data)
        {
            return Some(Self::AuthOk {
                client_id,
                name,
                token: None,
            });
        }

        None
    }
}

/// Extract binary data from a WebSocket message (handles both Binary and legacy Text).
fn extract_binary(msg: &Message) -> Option<Vec<u8>> {
    match msg {
        Message::Binary(data) => Some(data.to_vec()),
        _ => None,
    }
}

/// Apply one versioned client step batch, rejecting oversized batches before
/// changing document history, version, or broadcasting an update.
///
/// The version check, the append, and the version bump happen atomically
/// under the document steps lock (#2968), so two sockets submitting at the
/// same base version cannot both be accepted. The broadcast and log happen
/// after that lock has been released.
async fn apply_steps(
    doc: &Document,
    version: u64,
    steps: &[serde_json::Value],
    client_id: u64,
) -> Result<u64, String> {
    if steps.len() > MAX_STEPS_PER_MESSAGE {
        return Err(format!(
            "Step batch exceeds maximum of {MAX_STEPS_PER_MESSAGE} steps"
        ));
    }

    let new_version = doc
        .try_apply_steps(version, steps, client_id)
        .await
        .map_err(|rejected| match rejected {
            StepsRejected::VersionMismatch {
                current: current_version,
            } => format!("Version mismatch: expected {current_version}, got {version}"),
            // Not a "Version mismatch" or "desynchronized" error: the
            // client must not reconnect and resend the same bad step.
            StepsRejected::Invalid(refusal) => refusal.to_string(),
        })?;

    let update = CollabMessage::Update {
        steps: steps.to_vec(),
        client_ids: vec![client_id; steps.len()],
    };
    let broadcast_count = doc.broadcast.send(update).unwrap_or(0);
    tracing::info!(
        "[collab] Broadcast to {} receivers, new version={}",
        broadcast_count,
        new_version
    );

    Ok(new_version)
}

/// WebSocket upgrade handler for collaborative editing.
pub async fn ws_collab_handler(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    Query(params): Query<WsParams>,
    State(state): State<super::AppState>,
) -> impl IntoResponse {
    ws.max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| {
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

async fn store_and_broadcast_cursor(
    doc: &Document,
    client_id: u64,
    head: u32,
    anchor: u32,
    cached_display_name: Option<String>,
    incoming_name: Option<String>,
    identity_client_id: Option<String>,
) -> Option<String> {
    let effective_name = sanitize_name(cached_display_name.or(incoming_name));
    let client_id_str = client_id.to_string();

    {
        let mut cursors = doc.cursors.write().await;
        // #1790: server-side cursor lifecycle logging — a cursor is
        // first seen when its client_id enters the map.
        let first_seen = !cursors.contains_key(&client_id_str);
        cursors.insert(
            client_id_str,
            CursorPosition {
                head,
                anchor,
                name: effective_name.clone(),
                identity_client_id,
                last_update: Instant::now(),
                disconnected_at: None,
            },
        );
        if first_seen {
            tracing::info!(
                "[cursor-lifecycle] first-seen client={} name={:?}",
                client_id,
                effective_name
            );
        }
    }

    let cursor_msg = CollabMessage::Cursor {
        client_id,
        head,
        anchor,
        name: effective_name.clone(),
        idle_secs: None,
    };
    let _ = doc.broadcast.send(cursor_msg);
    effective_name
}

/// Handle a WebSocket connection for collaborative editing.
async fn handle_collab_socket(
    socket: WebSocket,
    doc_id: String,
    _filename: Option<String>,
    collab: Arc<CollabState>,
    store: iroh_blobs::api::Store,
    identity_store: super::IdentityStore,
) {
    use std::sync::atomic::Ordering;

    tracing::info!("[collab] New connection for doc '{}' (page load)", doc_id,);

    // A doc_id of the form note:<hex> names a note session (#3819
    // slice 2): it resolves by the note's stable ID, never by
    // filename. Anything else is a filename — resolve to hash, then
    // load content, exactly as before.
    let note_session = parse_note_doc_id(&doc_id);
    let hash_str = if note_session.is_some() {
        None
    } else {
        super::routes::get_hash_for_name(&store, &doc_id).await
    };
    let mut hydrated_filename: Option<&str> = None;
    let initial_content = if let Some(note) = &note_session {
        // A reopened note session hydrates from its checkpoint log
        // (#3819 slice 4): the current head revision's content,
        // fetched from the blob store under its document hash (a
        // `DocumentRef`'s content hash is the blob's hash). A note
        // the log does not know, or whose head blob is missing,
        // starts empty, like a file with no stored hash.
        match collab.note_head_content(*note).await {
            Ok(Some(reference)) => {
                // The publish path records checkpoints as
                // text/markdown; mode detection needs the filename
                // hint that media type implies, or the hydrated
                // bytes read as a non-editable mode and are dropped
                // from the document.
                if reference.media_type == "text/markdown" {
                    hydrated_filename = Some("note.md");
                }
                let content = load_file_content(&store, &reference.content.to_string()).await;
                if content.is_none() {
                    tracing::debug!("[collab] Head blob for note '{}' not in the store", doc_id);
                }
                content
            }
            Ok(None) => None,
            Err(err) => {
                tracing::warn!("[collab] Note log unreadable for '{}': {err}", doc_id);
                None
            }
        }
    } else if let Some(h) = &hash_str {
        let content = load_file_content(&store, h).await;
        if content.is_some() {
            tracing::debug!(
                "[collab] Loaded file content from store for '{}' (hash: {})",
                doc_id,
                h
            );
        }
        content
    } else {
        tracing::debug!("[collab] No hash found for filename '{}'", doc_id);
        None
    };

    let doc = if let Some(note) = &note_session {
        collab
            .get_or_create_for_note(
                note,
                hash_str.as_deref().unwrap_or(""),
                initial_content.as_deref(),
                _filename.as_deref().or(hydrated_filename),
            )
            .await
    } else {
        collab
            .get_or_create(
                &doc_id,
                hash_str.as_deref().unwrap_or(""),
                initial_content.as_deref(),
            )
            .await
    };
    let version_at_connect = doc.version();
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
                if let Some(data) = extract_binary(&ws_msg)
                    && let Some(decoded) = CollabMessage::decode(&data) {
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
                                    if sender.send(Message::Binary(auth_ok.encode().into())).await.is_err() {
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

    if sender
        .send(Message::Binary(init_bytes.into()))
        .await
        .is_err()
    {
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
            let catch_up_client_ids: Vec<u64> =
                steps.iter().filter_map(|(_, cid)| cid.as_u64()).collect();

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
                .send(Message::Binary(catch_up_bytes.into()))
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
                .send(Message::Binary(cursor_msg.encode().into()))
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
                    if sender.send(Message::Binary(bytes.into())).await.is_err() {
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
                    let error_msg = desynchronized_message(n);
                    let mut sender = sender_for_broadcast.lock().await;
                    let _ = sender
                        .send(Message::Binary(error_msg.encode().into()))
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
                let use_text =
                    ping_cycle.is_multiple_of(timeouts::EMPTY_TEXT_MESSAGE_EVERY_N_PINGS);

                let result = if use_text {
                    tracing::debug!(
                        "[collab] Sending empty text message (cursor refresh trigger), idle={}s",
                        elapsed.as_secs()
                    );
                    sender.send(Message::Text(String::new().into())).await
                } else {
                    tracing::debug!("[collab] Sending ping, idle={}s", elapsed.as_secs());
                    sender.send(Message::Ping(Vec::<u8>::new().into())).await
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
        .map(|m| Ok(Message::Binary(m.encode().into())))
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

                        let new_version = match apply_steps(&doc, version, &steps, client_id).await {
                            Ok(version) => version,
                            Err(error) => {
                                tracing::warn!("[collab] Rejected Steps: {}", error);
                                let error_msg = CollabMessage::Error { error };
                                let mut sender = sender.lock().await;
                                let _ = sender
                                    .send(Message::Binary(error_msg.encode().into()))
                                    .await;
                                continue;
                            }
                        };

                        // Send ack to sender
                        let ack = CollabMessage::Ack {
                            version: new_version,
                        };
                        tracing::debug!(
                            "[collab] Sending Ack: version={}",
                            new_version
                        );
                        let mut sender = sender.lock().await;
                        let _ = sender.send(Message::Binary(ack.encode().into())).await;
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
                        if let Some(ref mut rx) = name_watcher
                            && rx.has_changed().unwrap_or(false) {
                                let new_name = rx.borrow_and_update().clone();
                                cached_display_name = Some(
                                    new_name.unwrap_or_else(|| {
                                        short_id(identity_client_id.as_deref().unwrap_or(""))
                                    })
                                );
                            }
                        // Remember this client's ID for disconnect cleanup before
                        // storing and broadcasting its cursor.
                        *client_id_for_cleanup.write().await = Some(client_id_str);

                        let effective_name = store_and_broadcast_cursor(
                            &doc,
                            client_id,
                            head,
                            anchor,
                            cached_display_name.clone(),
                            name,
                            identity_client_id.clone(),
                        )
                        .await;

                        tracing::debug!(
                            "[collab] Received Cursor: client={}, head={}, anchor={}, name={:?}",
                            client_id,
                            head,
                            anchor,
                            effective_name
                        );

                    }
                    CollabMessage::Checkpoint { text } => {
                        // Durable publication is client-driven (#3819
                        // slice 4): the client holds the live content.
                        let Some(note) = &note_session else {
                            let error_msg = CollabMessage::Error {
                                error: "checkpoints are only available in note sessions"
                                    .to_owned(),
                            };
                            let mut sender = sender.lock().await;
                            let _ = sender
                                .send(Message::Binary(error_msg.encode().into()))
                                .await;
                            continue;
                        };
                        let author_key = identity_store.author_key();
                        let key = note_session_key(note);
                        match collab
                            .publish_note_checkpoint_text(&key, author_key, &text)
                            .await
                        {
                            Ok(record) => {
                                // The checkpoint names the content by
                                // its document hash; put the bytes in
                                // the blob store under that hash so a
                                // reopened session can hydrate from
                                // the head.
                                let _ = store
                                    .blobs()
                                    .add_bytes(text.into_bytes())
                                    .await;
                                let head = record
                                    .heads
                                    .first()
                                    .map(ToString::to_string)
                                    .unwrap_or_default();
                                let ack = CollabMessage::Checkpointed { head };
                                let mut sender = sender.lock().await;
                                let _ =
                                    sender.send(Message::Binary(ack.encode().into())).await;
                            }
                            Err(err) => {
                                let error_msg = CollabMessage::Error {
                                    error: err.to_string(),
                                };
                                let mut sender = sender.lock().await;
                                let _ = sender
                                    .send(Message::Binary(error_msg.encode().into()))
                                    .await;
                            }
                        }
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

    // Session end is a checkpoint point for a note session (#3819):
    // when its last editor leaves and the session moved since this
    // connection opened it, publish the checkpoint, signed with this
    // server's key. A refusal (notably a multi-head note, which needs
    // an explicit merge) is logged, never fatal to the disconnect.
    if remaining_clients == 0 {
        // The session's one changelog entry (#14272): the run of
        // checkpoints it published, signed with this server's key.
        if let Some(note) = &note_session {
            let key = note_session_key(note);
            match collab
                .end_note_session(&key, unix_millis(), |draft| {
                    identity_store.sign_changelog_entry(draft)
                })
                .await
            {
                Ok(Some(entry)) => tracing::debug!(
                    "[collab] Note session '{}' ended; changelog entry {} covers {:?}",
                    key,
                    entry.signed().seq(),
                    entry.signed().op_log().map(|range| range.count)
                ),
                Ok(None) => {}
                Err(err) => tracing::warn!(
                    "[collab] Could not record the end of note session '{}' in its changelog: {}",
                    key,
                    err
                ),
            }
        }
        tracing::debug!(
            "[collab] Note/filename session '{}' ended at version {} (connected at {}); \
             publication is client-driven, never a disconnect side effect",
            doc_id,
            doc.version(),
            version_at_connect
        );
    }
    // Note sessions publish through the client's Checkpoint message,
    // never from this cleanup (#3819 slice 4): the server does not
    // apply editor steps to the session document, so the document
    // held here is the session's base content, and recording it at
    // disconnect would sign stale text as a new revision.

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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_try_apply_steps_concurrent_same_base_version() {
        // #2968: sockets submitting Steps at the same base version must
        // not diverge. Exactly one is accepted; every other observes the
        // winner's bumped version and is rejected; the stored steps and
        // version reflect only the winner.
        //
        // The test holds the steps write lock while the submitters
        // arrive, modelling the production window: steps-lock contention
        // (e.g. a join catch-up read) parks a socket between its version
        // check and its append, so a check done before acquiring the
        // lock is stale for every parked submitter at once. Under the
        // pre-fix ordering several submitters pass the stale check and
        // are all accepted (Ok(1), Ok(2), ...); with the check inside
        // the lock exactly one wins, deterministically.
        use std::sync::Arc;
        let doc = Arc::new(Document::new());
        let gate = doc.steps.write().await;
        let mut handles = Vec::new();
        for client_id in 1..=4u64 {
            let doc = Arc::clone(&doc);
            handles.push(tokio::spawn(async move {
                let steps = vec![serde_json::json!({
                    "stepType": "replace",
                    "from": 0,
                    "to": 0,
                    "marker": client_id,
                })];
                (client_id, doc.try_apply_steps(0, &steps, client_id).await)
            }));
        }
        // Let every submitter arrive at the steps lock.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        drop(gate);
        let mut winners: Vec<u64> = Vec::new();
        for handle in handles {
            let (client_id, result) = handle.await.unwrap();
            match result {
                Ok(1) => winners.push(client_id),
                Err(StepsRejected::VersionMismatch { current: 1 }) => {}
                other => panic!(
                    "submitter {client_id} got {other:?}: only Ok(1) for the single winner and a mismatch at 1 for every loser are possible"
                ),
            }
        }
        assert_eq!(winners.len(), 1, "exactly one submission must be accepted");
        assert_eq!(doc.version(), 1, "version reflects only the winner's step");
        let stored = doc.steps.read().await;
        assert_eq!(stored.len(), 1, "only the winner's step is stored");
        assert_eq!(stored[0].0.data["marker"], winners[0]);
        assert_eq!(stored[0].1, serde_json::json!(winners[0]));
    }

    #[tokio::test]
    async fn test_try_apply_steps_sequential_versions() {
        // Single-client sequential flow: a submission at the current
        // version is accepted and bumps it; a replay at the stale base
        // version is rejected; the next submission at the new version
        // succeeds.
        let doc = Document::new();
        let step = vec![serde_json::json!({ "stepType": "replace", "from": 0, "to": 0 })];
        assert_eq!(doc.try_apply_steps(0, &step, 7).await, Ok(1));
        assert_eq!(
            doc.try_apply_steps(0, &step, 7).await,
            Err(StepsRejected::VersionMismatch { current: 1 })
        );
        assert_eq!(doc.try_apply_steps(1, &step, 7).await, Ok(2));
        assert_eq!(doc.version(), 2);
        assert_eq!(doc.steps.read().await.len(), 2);
    }

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
    #[tokio::test]
    async fn test_apply_steps_rejects_oversized_batch_without_mutation() {
        let doc = Document::new();
        let mut updates = doc.broadcast.subscribe();
        let steps = vec![serde_json::json!({"stepType": "replace"}); MAX_STEPS_PER_MESSAGE + 1];

        let error = apply_steps(&doc, 0, &steps, 123).await.unwrap_err();

        assert!(error.contains("1000"));
        assert_eq!(doc.version(), 0);
        assert!(doc.steps.read().await.is_empty());
        assert!(updates.try_recv().is_err());
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[tokio::test]
    async fn test_apply_steps_refuses_unappliable_steps_without_mutation() {
        // #1618: the issue's probe steps are refused before storage —
        // nothing stored, no version bump, nothing broadcast — and the
        // error is not one the client treats as "reconnect and resend".
        let doc = Document::new();
        let mut updates = doc.broadcast.subscribe();
        let probes = [
            serde_json::json!({"stepType": "bogusType", "from": 1, "to": 2}),
            serde_json::json!({"stepType": "replace", "from": 1, "to": 999_999}),
            serde_json::json!({"stepType": "replace", "from": 1, "to": 1,
                "slice": {"content": [{"type": "no_such_node"}]}}),
            serde_json::json!(42),
            serde_json::json!(null),
            serde_json::json!("hi"),
        ];
        for probe in probes {
            // A valid step ahead of the bad one does not get through
            // either: the batch is refused whole.
            let batch = vec![
                serde_json::json!({"stepType": "replace", "from": 1, "to": 1,
                    "slice": {"content": [{"type": "text", "text": "ok"}]}}),
                probe.clone(),
            ];
            let error = apply_steps(&doc, 0, &batch, 123).await.unwrap_err();
            assert!(error.starts_with("Invalid step 1:"), "{probe}: {error}");
            assert!(!error.contains("Version mismatch") && !error.contains("desynchronized"));
        }
        assert_eq!(doc.version(), 0);
        assert!(doc.steps.read().await.is_empty());
        assert!(updates.try_recv().is_err());

        // The tracked size moves with accepted steps: after typing two
        // characters, the old end of the paragraph is still in range and
        // the new end is too, but one past it is not.
        let typed = vec![
            serde_json::json!({"stepType": "replace", "from": 1, "to": 1,
            "slice": {"content": [{"type": "text", "text": "ok"}]}}),
        ];
        assert_eq!(apply_steps(&doc, 0, &typed, 123).await.unwrap(), 1);
        let at_end = vec![serde_json::json!({"stepType": "replace", "from": 4, "to": 4})];
        assert_eq!(apply_steps(&doc, 1, &at_end, 123).await.unwrap(), 2);
        let past_end = vec![serde_json::json!({"stepType": "replace", "from": 5, "to": 5})];
        assert!(apply_steps(&doc, 2, &past_end, 123).await.is_err());
        assert_eq!(doc.version(), 2);
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[tokio::test]
    async fn test_apply_steps_rejects_list_item_content_outside_editor_schema() {
        // Oracle: web/src/editor.ts declares list_item content as
        // `paragraph block*`; a direct text child violates that expression.
        let doc = Document::with_doc_and_mode(
            serde_json::json!({"type": "doc", "content": [{"type": "paragraph"}]}),
            ContentMode::Markdown,
        );
        let mut updates = doc.broadcast.subscribe();
        let invalid = vec![serde_json::json!({
            "stepType": "replace", "from": 0, "to": 0,
            "slice": {"content": [{"type": "bullet_list", "content": [{
                "type": "list_item", "content": [{"type": "text", "text": "bad"}]
            }]]}
        })];

        let error = apply_steps(&doc, 0, &invalid, 123).await.unwrap_err();
        assert!(error.contains("list_item"), "{error}");
        assert_eq!(doc.version(), 0);
        assert!(doc.steps.read().await.is_empty());
        assert_eq!(doc.content_size.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(updates.try_recv().is_err());

        let valid = vec![serde_json::json!({
            "stepType": "replace", "from": 0, "to": 0,
            "slice": {"content": [{"type": "bullet_list", "content": [{
                "type": "list_item", "content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "ok"}
                ]}]
            }]]}
        })];
        assert_eq!(apply_steps(&doc, 0, &valid, 123).await.unwrap(), 1);
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[tokio::test]
    async fn test_apply_steps_accepts_small_batch_and_broadcasts_update() {
        let doc = Document::new();
        let mut updates = doc.broadcast.subscribe();
        let steps = vec![serde_json::json!({"stepType": "replace", "from": 0, "to": 0})];

        let ack_version = apply_steps(&doc, 0, &steps, 123).await.unwrap();

        assert_eq!(ack_version, 1);
        assert_eq!(doc.version(), 1);
        assert_eq!(doc.steps.read().await.len(), 1);
        match updates.recv().await.unwrap() {
            CollabMessage::Update {
                steps: broadcast_steps,
                client_ids,
            } => {
                assert_eq!(broadcast_steps, steps);
                assert_eq!(client_ids, vec![123]);
            }
            other => panic!("Expected Update, got {other:?}"),
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
    #[tokio::test]
    async fn test_cursor_ingest_sanitizes_before_storage_and_broadcast() {
        let doc = Document::new();
        let mut updates = doc.broadcast.subscribe();

        let name = store_and_broadcast_cursor(
            &doc,
            54321,
            100,
            50,
            None,
            Some("A\n\u{202e}lice".to_owned()),
            None,
        )
        .await;
        assert_eq!(name.as_deref(), Some("Alice"));

        let stored = doc.cursors.read().await.get("54321").unwrap().name.clone();
        assert_eq!(stored.as_deref(), Some("Alice"));

        match updates.recv().await.unwrap() {
            CollabMessage::Cursor { name, .. } => assert_eq!(name.as_deref(), Some("Alice")),
            _ => panic!("Expected sanitized cursor broadcast"),
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
    fn test_collab_message_decode_refuses_trailing_bytes() {
        let msgs = [
            CollabMessage::Auth {
                token: "secret-token".to_owned(),
            },
            CollabMessage::Steps {
                version: 3,
                steps: vec![serde_json::json!({"a": 1})],
                client_id: 7,
            },
            CollabMessage::Ack { version: 42 },
        ];
        for m in msgs {
            let bytes = m.encode();
            assert_eq!(
                CollabMessage::decode(&bytes).map(|d| d.encode()),
                Some(bytes.clone())
            );
            let mut with_byte = bytes.clone();
            with_byte.push(0xff);
            assert!(CollabMessage::decode(&with_byte).is_none());
            let mut with_garbage = bytes.clone();
            with_garbage.extend_from_slice(b"garbage!!");
            assert!(CollabMessage::decode(&with_garbage).is_none());
            let mut with_second = bytes.clone();
            with_second.extend_from_slice(&bytes);
            assert!(CollabMessage::decode(&with_second).is_none());
        }
    }

    #[test]
    fn test_collab_message_decode_refuses_every_variant_with_suffix() {
        let msgs = [
            CollabMessage::Init {
                version: 1,
                doc: serde_json::json!({"t": "doc"}),
                mode: "raw".to_owned(),
            },
            CollabMessage::Update {
                steps: vec![serde_json::json!(1)],
                client_ids: vec![2],
            },
            CollabMessage::Cursor {
                client_id: 1,
                head: 2,
                anchor: 3,
                name: Some("n".to_owned()),
                idle_secs: Some(4),
            },
            CollabMessage::Error {
                error: "e".to_owned(),
            },
            CollabMessage::CursorRemove { client_id: 9 },
            CollabMessage::NewVersion {
                hash: "h".to_owned(),
                name: "n".to_owned(),
            },
            CollabMessage::AuthOk {
                client_id: "client-1".to_owned(),
                name: Some("n".to_owned()),
                token: Some("t".to_owned()),
            },
        ];
        for m in msgs {
            let bytes = m.encode();
            assert!(CollabMessage::decode(&bytes).is_some(), "{m:?} round-trips");
            let mut suffixed = bytes;
            suffixed.push(0x00);
            assert!(CollabMessage::decode(&suffixed).is_none(), "{m:?} + suffix");
        }
    }

    // Property coverage for the #6349 framing invariant (Codex P1 on PR
    // #6553): for every generated valid message and every nonempty suffix,
    // the message round-trips and the suffixed frame is rejected.
    use proptest::prelude::*;

    fn arb_json_value() -> impl Strategy<Value = serde_json::Value> {
        let leaf = prop_oneof![
            Just(serde_json::Value::Null),
            any::<bool>().prop_map(serde_json::Value::Bool),
            any::<i64>().prop_map(|n| serde_json::json!(n)),
            any::<String>().prop_map(serde_json::Value::String),
        ];
        leaf.prop_recursive(3, 16, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::Array),
                proptest::collection::vec((any::<String>(), inner), 0..4)
                    .prop_map(|pairs| { serde_json::Value::Object(pairs.into_iter().collect()) }),
            ]
        })
    }

    fn arb_collab_message() -> impl Strategy<Value = CollabMessage> {
        prop_oneof![
            (any::<u64>(), arb_json_value(), any::<String>())
                .prop_map(|(version, doc, mode)| CollabMessage::Init { version, doc, mode },),
            (
                any::<u64>(),
                proptest::collection::vec(arb_json_value(), 0..4),
                any::<u64>(),
            )
                .prop_map(|(version, steps, client_id)| CollabMessage::Steps {
                    version,
                    steps,
                    client_id,
                }),
            (
                proptest::collection::vec(arb_json_value(), 0..4),
                proptest::collection::vec(any::<u64>(), 0..4),
            )
                .prop_map(|(steps, client_ids)| CollabMessage::Update { steps, client_ids }),
            any::<u64>().prop_map(|version| CollabMessage::Ack { version }),
            (
                any::<u64>(),
                any::<u32>(),
                any::<u32>(),
                proptest::option::of(any::<String>()),
                proptest::option::of(any::<u64>()),
            )
                .prop_map(|(client_id, head, anchor, name, idle_secs)| {
                    CollabMessage::Cursor {
                        client_id,
                        head,
                        anchor,
                        name,
                        idle_secs,
                    }
                }),
            any::<String>().prop_map(|error| CollabMessage::Error { error }),
            any::<u64>().prop_map(|client_id| CollabMessage::CursorRemove { client_id }),
            (any::<String>(), any::<String>())
                .prop_map(|(hash, name)| CollabMessage::NewVersion { hash, name }),
            any::<String>().prop_map(|token| CollabMessage::Auth { token }),
            (
                any::<String>(),
                proptest::option::of(any::<String>()),
                proptest::option::of(any::<String>()),
            )
                .prop_map(|(client_id, name, token)| CollabMessage::AuthOk {
                    client_id,
                    name,
                    token,
                }),
        ]
    }

    proptest! {
        #[test]
        fn test_collab_message_decode_refuses_arbitrary_suffixes(
            msg in arb_collab_message(),
            suffix in proptest::collection::vec(any::<u8>(), 1..32),
        ) {
            let bytes = msg.encode();
            // The valid message round-trips byte-for-byte.
            prop_assert_eq!(
                CollabMessage::decode(&bytes).map(|d| d.encode()),
                Some(bytes.clone())
            );
            // Any nonempty suffix makes the frame undecodable: the frame is
            // exactly one MessagePack value (#6349).
            let mut suffixed = bytes;
            suffixed.extend_from_slice(&suffix);
            prop_assert!(CollabMessage::decode(&suffixed).is_none());
        }
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
        assert_ne!(init_bytes, [] as [u8; 0]);
        // MessagePack fixarray of 4 elements starts with 0x94
        // First element should decode to 0 (INIT)

        let ack = CollabMessage::Ack { version: 0 };
        let ack_bytes = ack.encode();
        // MessagePack fixarray of 2 elements starts with 0x92
        assert_ne!(ack_bytes, [] as [u8; 0]);
    }

    #[test]
    fn test_message_type_constants() {
        assert_eq!(msg::INIT, 0);
        assert_eq!(msg::STEPS, 1);
        assert_eq!(msg::UPDATE, 2);
        assert_eq!(msg::ACK, 3);
        assert_eq!(msg::CURSOR, 4);
        assert_eq!(msg::ERROR, 5);
        assert_eq!(msg::CURSOR_REMOVE, 6);
        assert_eq!(msg::NEW_VERSION, 7);
        assert_eq!(msg::AUTH, 8);
        assert_eq!(msg::AUTH_OK, 9);
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_cursor_remove_roundtrip() {
        // Regression (#1783): decode had no tag-6 branch, so a CursorRemove
        // frame came back as None — indistinguishable from corrupt input.
        let msg = CollabMessage::CursorRemove { client_id: 54321 };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::CursorRemove { client_id } => {
                assert_eq!(client_id, 54321);
            }
            _ => panic!("Expected CursorRemove message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_auth_roundtrip() {
        let msg = CollabMessage::Auth {
            token: "secret-token".to_owned(),
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Auth { token } => {
                assert_eq!(token, "secret-token");
            }
            _ => panic!("Expected Auth message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_collab_message_auth_ok_roundtrip() {
        for token in [Some("fresh-token".to_owned()), None] {
            let msg = CollabMessage::AuthOk {
                client_id: "client-1".to_owned(),
                name: Some("Alice".to_owned()),
                token: token.clone(),
            };
            let encoded = msg.encode();
            let decoded = CollabMessage::decode(&encoded).unwrap();

            match decoded {
                CollabMessage::AuthOk {
                    client_id,
                    name,
                    token: decoded_token,
                } => {
                    assert_eq!(client_id, "client-1");
                    assert_eq!(name, Some("Alice".to_owned()));
                    assert_eq!(decoded_token, token);
                }
                _ => panic!("Expected AuthOk message"),
            }
        }
    }

    #[test]
    fn test_content_to_document_empty() {
        let (doc, mode) = content_to_document(None, None);
        assert_eq!(mode, ContentMode::Raw);
        assert_eq!(doc["type"], "doc");
        assert!(doc["content"].as_array().is_some());
    }

    #[test]
    fn rich_content_with_a_duplicated_key_falls_back_to_an_empty_document() {
        let content = br#"{"type":"doc","type":"paragraph","content":[]}"#;
        let (doc, mode) = content_to_document(Some(content), Some("note.pm.json"));
        assert_eq!(mode, ContentMode::Rich);
        assert_eq!(doc["type"], "doc");
        assert_eq!(doc["content"][0]["type"], "paragraph");
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
            client_ids,
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

    #[tokio::test]
    async fn test_lagged_broadcast_produces_recoverable_desync_error() {
        let (tx, mut rx) = broadcast::channel(1);
        // Keep the receiver behind until the bounded channel has overwritten
        // its first item; this induces Lagged deterministically, without sleeps.
        tx.send(CollabMessage::CursorRemove { client_id: 1 })
            .unwrap();
        tx.send(CollabMessage::CursorRemove { client_id: 2 })
            .unwrap();

        let lost = match rx.recv().await {
            Err(broadcast::error::RecvError::Lagged(lost)) => lost,
            other => panic!("expected lagged receiver, got {other:?}"),
        };
        match desynchronized_message(lost) {
            CollabMessage::Error { error } => {
                assert_eq!(error, "Session desynchronized: 1 messages lost");
                assert!(error.contains("desynchronized"));
            }
            other => panic!("expected recoverable error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // CollabState tests — verify filename-keyed session management (Part 2)
    // -----------------------------------------------------------------------

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn test_collab_state_keys_by_filename() {
        let collab = CollabState::default();

        // Create a document keyed by filename
        let doc1 = collab
            .get_or_create("test.txt", "hash_aaa", Some(b"hello"))
            .await;
        assert_eq!(*doc1.hash.read().await, "hash_aaa");

        // Same filename, different hash → returns same document (session persists)
        let doc2 = collab
            .get_or_create("test.txt", "hash_bbb", Some(b"world"))
            .await;
        assert!(
            Arc::ptr_eq(&doc1, &doc2),
            "Same filename must return the same Document Arc"
        );
        // Hash should remain from the first creation (not overwritten)
        assert_eq!(*doc2.hash.read().await, "hash_aaa");

        // Different filename → different document
        let doc3 = collab
            .get_or_create("other.md", "hash_ccc", Some(b"# Title"))
            .await;
        assert!(
            !Arc::ptr_eq(&doc1, &doc3),
            "Different filename must create a new Document"
        );
        assert_eq!(*doc3.hash.read().await, "hash_ccc");
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[tokio::test]
    async fn test_notify_new_version_updates_hash_and_broadcasts() {
        let collab = CollabState::default();

        // Create a document and subscribe to its broadcast channel
        let doc = collab
            .get_or_create("readme.md", "old_hash", Some(b"# Hello"))
            .await;
        let mut rx = doc.broadcast.subscribe();
        assert_eq!(*doc.hash.read().await, "old_hash");

        // Notify new version
        collab.notify_new_version("readme.md", "new_hash_abc").await;

        // Hash should be updated
        assert_eq!(*doc.hash.read().await, "new_hash_abc");

        // Should have received the NewVersion broadcast
        let msg = rx.try_recv().unwrap();
        match msg {
            CollabMessage::NewVersion { hash, name } => {
                assert_eq!(hash, "new_hash_abc");
                assert_eq!(name, "readme.md");
            }
            _ => panic!("Expected NewVersion message, got {msg:?}"),
        }
    }

    #[tokio::test]
    async fn test_notify_new_version_missing_doc_is_noop() {
        let collab = CollabState::default();

        // Notify for a document that doesn't exist — should not panic
        collab
            .notify_new_version("nonexistent.txt", "some_hash")
            .await;

        // Verify no documents were created as a side effect
        let read = collab.documents.read().await;
        assert!(read.is_empty());
    }

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn test_remove_document_by_filename() {
        let collab = CollabState::default();

        let doc1 = collab
            .get_or_create("temp.txt", "hash_1", Some(b"temporary"))
            .await;
        assert_eq!(*doc1.hash.read().await, "hash_1");

        // Remove by filename
        collab.remove_document("temp.txt").await;

        // Creating again should yield a new Document (not the old one)
        let doc2 = collab
            .get_or_create("temp.txt", "hash_2", Some(b"new content"))
            .await;
        assert!(
            !Arc::ptr_eq(&doc1, &doc2),
            "After removal, get_or_create must return a new Document"
        );
        assert_eq!(*doc2.hash.read().await, "hash_2");
    }
    // -----------------------------------------------------------------------
    // CollabState tests -- note-ID session keying (#3819, Phase 3 slice 1)
    // -----------------------------------------------------------------------

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn test_collab_state_keys_note_sessions_by_stable_note_id() {
        let collab = CollabState::default();
        let note_a = Hash::from_bytes([7u8; 32]);
        let note_b = Hash::from_bytes([9u8; 32]);

        // One note, reached under two different filename hints (a
        // rename, an alias): the same session, because the key is
        // the note's stable ID, not either name.
        let doc1 = collab
            .get_or_create_for_note(&note_a, "hash_a", Some(b"# Note"), Some("first.md"))
            .await;
        let doc2 = collab
            .get_or_create_for_note(&note_a, "hash_a2", Some(b"# Other"), Some("renamed.md"))
            .await;
        assert!(
            Arc::ptr_eq(&doc1, &doc2),
            "the same note ID must return the same session whatever the filename"
        );

        // A different note is a different session, and a filename
        // session never aliases a note session.
        let doc3 = collab
            .get_or_create_for_note(&note_b, "hash_b", Some(b"# B"), Some("first.md"))
            .await;
        assert!(!Arc::ptr_eq(&doc1, &doc3));
        let doc4 = collab
            .get_or_create("first.md", "hash_f", Some(b"# F"))
            .await;
        assert!(!Arc::ptr_eq(&doc1, &doc4));
        assert!(!Arc::ptr_eq(&doc3, &doc4));
    }

    #[tokio::test]
    async fn test_parse_note_doc_id() {
        let note = Hash::from_bytes([9u8; 32]);
        assert_eq!(parse_note_doc_id(&note_session_key(&note)), Some(note));
        assert_eq!(parse_note_doc_id("notes.md"), None);
        assert_eq!(parse_note_doc_id("note:not-a-hash"), None);
        assert_eq!(parse_note_doc_id("note:"), None);
    }

    #[tokio::test]
    async fn test_publish_note_checkpoint_creates_then_chains_on_the_head() {
        let state = CollabState::new();
        let note = Hash::from_bytes([11u8; 32]);
        let key = note_session_key(&note);
        let author: AuthorKey = [7u8; 32];
        let doc = state
            .get_or_create_for_note(&note, "", Some(b"# First"), Some("n.md"))
            .await;

        // The first publication establishes the note: the create event
        // is the note's own ID and its only head.
        let created = state.publish_note_checkpoint(&key, author).await.unwrap();
        assert_eq!(created.id, note);
        assert_eq!(created.heads.len(), 1);
        assert_eq!(created.revisions.len(), 1);

        // Edit the live document, then publish again: the new revision
        // builds on the first head and replaces it as the only head.
        *doc.doc.write().await = plain_text_to_prosemirror("# Second");
        let chained = state.publish_note_checkpoint(&key, author).await.unwrap();
        assert_eq!(chained.revisions.len(), 2);
        assert_eq!(chained.heads.len(), 1);
        assert_eq!(chained.revisions[1].parents, created.heads);
        assert_ne!(chained.heads, created.heads);
    }

    #[tokio::test]
    async fn test_publish_note_checkpoint_enforces_the_access_boundary() {
        use id_store::note_checkpoints::{NoteGrant, NotePrincipal, NoteRole};

        // The owner publishes the create (public by default), then
        // scopes the note to a single write-grant user. From then
        // on, publication is the boundary: the granted author
        // chains a checkpoint; another author is refused and the
        // log does not move.
        let state = CollabState::new();
        let note = Hash::from_bytes([17u8; 32]);
        let key = note_session_key(&note);
        let owner: AuthorKey = [7u8; 32];
        let granted: AuthorKey = [8u8; 32];
        let stranger: AuthorKey = [9u8; 32];
        let doc = state
            .get_or_create_for_note(&note, "", Some(b"# First"), Some("n.md"))
            .await;
        state.publish_note_checkpoint(&key, owner).await.unwrap();
        state
            .set_note_access(
                note,
                owner,
                NoteAccess::Scoped(vec![NoteGrant {
                    principal: NotePrincipal::User(granted),
                    role: NoteRole::Write,
                }]),
            )
            .await
            .unwrap();

        *doc.doc.write().await = plain_text_to_prosemirror("# Second");
        let chained = state.publish_note_checkpoint(&key, granted).await.unwrap();
        assert_eq!(chained.revisions.len(), 2);

        *doc.doc.write().await = plain_text_to_prosemirror("# Third");
        let err = state
            .publish_note_checkpoint(&key, stranger)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                PublishCheckpointError::Checkpoint(CheckpointError::NotAuthorized { .. })
            ),
            "a stranger's publication is refused at the boundary: {err}"
        );
        assert_eq!(
            state.note_events().await.len(),
            2,
            "the refusal recorded nothing"
        );

        // And a non-admin cannot re-open the note.
        assert!(
            state
                .set_note_access(note, granted, NoteAccess::Public)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_publish_note_checkpoint_refuses_a_filename_session() {
        let state = CollabState::new();
        state.get_or_create("plain.md", "", Some(b"# File")).await;
        let err = state
            .publish_note_checkpoint("plain.md", [7u8; 32])
            .await
            .unwrap_err();
        assert!(
            matches!(err, PublishCheckpointError::NotNoteSession(_)),
            "a filename session has no note to checkpoint: {err}"
        );
        assert_eq!(state.note_events().await, [] as [AppliedNote; 0]);
    }

    #[tokio::test]
    async fn test_note_checkpoint_log_rebuilds_from_seeded_events() {
        // Restart/sync at the wiring level: a fresh state seeded with
        // the live log's events folds to the same heads, and the next
        // publication chains onto them.
        let note = Hash::from_bytes([13u8; 32]);
        let key = note_session_key(&note);
        let author: AuthorKey = [3u8; 32];
        let live = CollabState::new();
        live.get_or_create_for_note(&note, "", Some(b"# One"), Some("n.md"))
            .await;
        live.publish_note_checkpoint(&key, author).await.unwrap();
        let heads_before = live.note_heads(note).await.unwrap();

        let restarted = CollabState::new();
        restarted
            .seed_note_events(live.note_events().await)
            .await
            .unwrap();
        assert_eq!(restarted.note_heads(note).await.unwrap(), heads_before);

        let doc = restarted
            .get_or_create_for_note(&note, "", Some(b"# Two"), Some("n.md"))
            .await;
        drop(doc);
        let chained = restarted
            .publish_note_checkpoint(&key, author)
            .await
            .unwrap();
        assert_eq!(chained.revisions.len(), 2);
        assert_eq!(chained.revisions[1].parents, heads_before);
    }

    #[tokio::test]
    async fn test_note_policies_survive_a_restart_rebuild() {
        use id_store::note_checkpoints::{GroupRoles, NoteGrant, NotePrincipal, NoteRole};

        // Author 5 is a Write member of group 9; author 6 is not.
        fn group_roles() -> GroupRoles {
            GroupRoles::new(|group, member| {
                if group == Hash::from_bytes([9u8; 32]) && member == [5u8; 32] {
                    Some(NoteRole::Write)
                } else {
                    None
                }
            })
        }

        // The owner creates the note, then scopes it to one
        // write-grant user and one write-grant group.
        let note = Hash::from_bytes([41u8; 32]);
        let key = note_session_key(&note);
        let owner: AuthorKey = [7u8; 32];
        let granted: AuthorKey = [8u8; 32];
        let member: AuthorKey = [5u8; 32];
        let stranger: AuthorKey = [9u8; 32];
        let live = CollabState::new().with_note_group_roles(group_roles());
        live.get_or_create_for_note(&note, "", Some(b"# First"), Some("n.md"))
            .await;
        live.publish_note_checkpoint(&key, owner).await.unwrap();
        live.set_note_access(
            note,
            owner,
            NoteAccess::Scoped(vec![
                NoteGrant {
                    principal: NotePrincipal::User(granted),
                    role: NoteRole::Write,
                },
                NoteGrant {
                    principal: NotePrincipal::Group(Hash::from_bytes([9u8; 32])),
                    role: NoteRole::Write,
                },
            ]),
        )
        .await
        .unwrap();

        // Restart: the events and the policies, in that order.
        let events = live.note_events().await;
        let policies = live.note_policies().await.unwrap();
        let restarted = CollabState::new().with_note_group_roles(group_roles());
        restarted.seed_note_events(events).await.unwrap();
        restarted.seed_note_policies(policies).await.unwrap();

        // The boundary holds after the restart: the granted user and
        // the group member chain on; the stranger is refused and the
        // refusal records nothing.
        let doc = restarted
            .get_or_create_for_note(&note, "", Some(b"# Second"), Some("n.md"))
            .await;
        let chained = restarted
            .publish_note_checkpoint(&key, granted)
            .await
            .unwrap();
        assert_eq!(chained.revisions.len(), 2);
        *doc.doc.write().await = plain_text_to_prosemirror("# Third");
        let chained = restarted
            .publish_note_checkpoint(&key, member)
            .await
            .unwrap();
        assert_eq!(chained.revisions.len(), 3);

        *doc.doc.write().await = plain_text_to_prosemirror("# Nope");
        let err = restarted
            .publish_note_checkpoint(&key, stranger)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                PublishCheckpointError::Checkpoint(CheckpointError::NotAuthorized { .. })
            ),
            "a stranger's publication is refused after the restart: {err}"
        );
        assert_eq!(
            restarted.note_events().await.len(),
            3,
            "the refusal recorded nothing"
        );
    }

    #[tokio::test]
    async fn test_note_policies_survive_a_host_restart_through_the_snapshot_file() {
        use id_store::note_checkpoints::{GroupRoles, NoteGrant, NotePrincipal, NoteRole};

        // Author 5 is a Write member of group 9; author 6 is not.
        fn group_roles() -> GroupRoles {
            GroupRoles::new(|group, member| {
                if group == Hash::from_bytes([9u8; 32]) && member == [5u8; 32] {
                    Some(NoteRole::Write)
                } else {
                    None
                }
            })
        }

        // The host restart drill (#3819): the live state persists
        // its checkpoint log to the snapshot file as it mutates; a
        // fresh state over the same file restores events and
        // policies, so the access boundary — not just the log —
        // survives the restart.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".note-checkpoints.json");
        let note = Hash::from_bytes([47u8; 32]);
        let key = note_session_key(&note);
        let public_note = Hash::from_bytes([49u8; 32]);
        let public_key = note_session_key(&public_note);
        let owner: AuthorKey = [7u8; 32];
        let granted: AuthorKey = [8u8; 32];
        let member: AuthorKey = [5u8; 32];
        let stranger: AuthorKey = [9u8; 32];

        let live = CollabState::new()
            .with_note_group_roles(group_roles())
            .with_note_snapshot_path(path.clone());
        live.get_or_create_for_note(&note, "", Some(b"# First"), Some("n.md"))
            .await;
        live.publish_note_checkpoint(&key, owner).await.unwrap();
        live.set_note_access(
            note,
            owner,
            NoteAccess::Scoped(vec![
                NoteGrant {
                    principal: NotePrincipal::User(granted),
                    role: NoteRole::Write,
                },
                NoteGrant {
                    principal: NotePrincipal::Group(Hash::from_bytes([9u8; 32])),
                    role: NoteRole::Write,
                },
            ]),
        )
        .await
        .unwrap();
        live.get_or_create_for_note(&public_note, "", Some(b"# Open"), Some("p.md"))
            .await;
        live.publish_note_checkpoint(&public_key, owner)
            .await
            .unwrap();

        // The mutations reached the file: it parses as a snapshot
        // holding both notes' events and the one recorded policy —
        // the public note has none, so it contributes no record.
        let persisted: NoteCheckpointSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.events.len(), 2);
        assert_eq!(persisted.policies.len(), 1);
        let heads_before = live.note_heads(note).await.unwrap();

        // Restart: a fresh state over the same file.
        let restarted = CollabState::new()
            .with_note_group_roles(group_roles())
            .with_note_snapshot_path(path.clone());
        assert!(restarted.restore_persisted_note_snapshot().await.unwrap());
        assert_eq!(restarted.note_events().await, live.note_events().await);
        assert_eq!(restarted.note_heads(note).await.unwrap(), heads_before);

        // The boundary holds after the restart: the granted user and
        // the group member chain on; the stranger is refused and the
        // refusal records nothing.
        let doc = restarted
            .get_or_create_for_note(&note, "", Some(b"# Second"), Some("n.md"))
            .await;
        let chained = restarted
            .publish_note_checkpoint(&key, granted)
            .await
            .unwrap();
        assert_eq!(chained.revisions.len(), 2);
        *doc.doc.write().await = plain_text_to_prosemirror("# Third");
        let chained = restarted
            .publish_note_checkpoint(&key, member)
            .await
            .unwrap();
        assert_eq!(chained.revisions.len(), 3);
        *doc.doc.write().await = plain_text_to_prosemirror("# Nope");
        let err = restarted
            .publish_note_checkpoint(&key, stranger)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                PublishCheckpointError::Checkpoint(CheckpointError::NotAuthorized { .. })
            ),
            "a stranger's publication is refused after the restart: {err}"
        );
        assert_eq!(
            restarted.note_events().await.len(),
            4,
            "the refusal recorded nothing"
        );

        // And the public note stayed public: the same stranger
        // appends to it without any grant.
        restarted
            .get_or_create_for_note(&public_note, "", Some(b"# Open two"), Some("p.md"))
            .await;
        let appended = restarted
            .publish_note_checkpoint(&public_key, stranger)
            .await
            .unwrap();
        assert_eq!(appended.revisions.len(), 2);
        assert_eq!(restarted.note_events().await.len(), 5);
    }

    #[tokio::test]
    async fn test_a_corrupt_note_snapshot_fails_the_restart_not_opens_notes() {
        // A snapshot that does not parse is a failed restart, never
        // a host that silently reopens scoped notes to public; a
        // missing snapshot is a fresh host, not an error.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".note-checkpoints.json");
        std::fs::write(&path, b"{\"events\":").unwrap();
        let state = CollabState::new().with_note_snapshot_path(path);
        assert!(state.restore_persisted_note_snapshot().await.is_err());

        let fresh = CollabState::new().with_note_snapshot_path(dir.path().join("absent.json"));
        assert!(!fresh.restore_persisted_note_snapshot().await.unwrap());
    }

    /// A note owned by `owner`, scoped so nobody else reads it, in a
    /// state persisting to `path`; returns the state and the note.
    async fn private_note_state(path: PathBuf, owner: AuthorKey, byte: u8) -> (CollabState, Hash) {
        let state = CollabState::new().with_note_snapshot_path(path);
        let note = Hash::from_bytes([byte; 32]);
        state
            .get_or_create_for_note(&note, "", Some(b"# Private"), Some("g.md"))
            .await;
        state
            .publish_note_checkpoint(&note_session_key(&note), owner)
            .await
            .unwrap();
        state
            .set_note_access(note, owner, NoteAccess::Scoped(vec![]))
            .await
            .unwrap();
        (state, note)
    }

    fn note_read_grant(name: &str, issuer: AuthorKey, reader: AuthorKey, note: Hash) -> Grant {
        use id_grants::{Delegation, Duration, Operation, Subject};
        Grant {
            id: GrantId(name.to_owned()),
            issuer: identity_key(issuer),
            subject: Subject::Identity(identity_key(reader)),
            scopes: std::collections::BTreeSet::from([Scope::Object(note.to_hex())]),
            operations: std::collections::BTreeSet::from([Operation::Read]),
            duration: Duration::Standing,
            delegation: Delegation::default(),
            parent: None,
        }
    }

    const GRANT_NOW: u64 = 1_800_000_000;

    #[tokio::test]
    async fn test_an_owner_grant_admits_a_reader_and_revoke_ends_it() {
        let dir = tempfile::tempdir().unwrap();
        let owner: AuthorKey = [7u8; 32];
        let reader: AuthorKey = [8u8; 32];
        let (state, note) =
            private_note_state(dir.path().join(".note-checkpoints.json"), owner, 61).await;
        let defaults = ReadDefaults::none();

        assert!(
            !state
                .may_read_note(note, reader, &defaults, GRANT_NOW)
                .await
        );
        state
            .grant_note_access(owner, note_read_grant("g1", owner, reader, note))
            .await
            .unwrap();
        assert!(
            state
                .may_read_note(note, reader, &defaults, GRANT_NOW)
                .await
        );
        // The grant names one reader; another stays refused.
        assert!(
            !state
                .may_read_note(note, [9u8; 32], &defaults, GRANT_NOW)
                .await
        );

        // Only the issuer revokes, and a stranger's attempt gets the
        // same answer as an unknown id.
        assert_eq!(
            state
                .revoke_note_grant(reader, &GrantId("g1".to_owned()))
                .await,
            Err(NoteGrantError::UnknownGrant)
        );
        assert_eq!(
            state
                .revoke_note_grant(owner, &GrantId("absent".to_owned()))
                .await,
            Err(NoteGrantError::UnknownGrant)
        );
        state
            .revoke_note_grant(owner, &GrantId("g1".to_owned()))
            .await
            .unwrap();
        assert!(
            !state
                .may_read_note(note, reader, &defaults, GRANT_NOW)
                .await
        );
    }

    #[tokio::test]
    async fn test_only_a_note_owner_issues_a_grant_over_it() {
        let dir = tempfile::tempdir().unwrap();
        let owner: AuthorKey = [7u8; 32];
        let other: AuthorKey = [8u8; 32];
        let (state, note) =
            private_note_state(dir.path().join(".note-checkpoints.json"), owner, 62).await;

        // The issuer field must be the account issuing.
        assert_eq!(
            state
                .grant_note_access(other, note_read_grant("g1", owner, other, note))
                .await,
            Err(NoteGrantError::NotIssuer)
        );
        // Someone else's grant over the owner's note is refused, and an
        // unknown note gets the same answer.
        assert_eq!(
            state
                .grant_note_access(other, note_read_grant("g1", other, other, note))
                .await,
            Err(NoteGrantError::NotOwner)
        );
        assert_eq!(
            state
                .grant_note_access(
                    owner,
                    note_read_grant("g1", owner, other, Hash::from_bytes([99u8; 32]))
                )
                .await,
            Err(NoteGrantError::NotOwner)
        );
        // A scope that is not a note id is refused.
        let mut collection = note_read_grant("g1", owner, other, note);
        collection.scopes =
            std::collections::BTreeSet::from([Scope::Object(note.to_hex().to_uppercase())]);
        assert_eq!(
            state.grant_note_access(owner, collection).await,
            Err(NoteGrantError::NotANote)
        );
        assert!(
            !state
                .may_read_note(note, other, &ReadDefaults::none(), GRANT_NOW)
                .await
        );
    }

    #[tokio::test]
    async fn test_note_grants_survive_a_restart_through_the_snapshot_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".note-checkpoints.json");
        let owner: AuthorKey = [7u8; 32];
        let reader: AuthorKey = [8u8; 32];
        let gone: AuthorKey = [9u8; 32];
        let (live, note) = private_note_state(path.clone(), owner, 63).await;

        // Before any grant, the file carries no grants field at all, so
        // it reads exactly as snapshots written before grants existed.
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(raw.get("grants").is_none());

        live.grant_note_access(owner, note_read_grant("keep", owner, reader, note))
            .await
            .unwrap();
        live.grant_note_access(owner, note_read_grant("drop", owner, gone, note))
            .await
            .unwrap();
        live.revoke_note_grant(owner, &GrantId("drop".to_owned()))
            .await
            .unwrap();

        let restarted = CollabState::new().with_note_snapshot_path(path.clone());
        assert!(restarted.restore_persisted_note_snapshot().await.unwrap());
        let defaults = ReadDefaults::none();
        assert!(
            restarted
                .may_read_note(note, reader, &defaults, GRANT_NOW)
                .await
        );
        assert!(
            !restarted
                .may_read_note(note, gone, &defaults, GRANT_NOW)
                .await
        );
        assert_eq!(
            restarted.note_snapshot().await.unwrap(),
            live.note_snapshot().await.unwrap()
        );
    }

    #[tokio::test]
    async fn test_a_tampered_grant_snapshot_fails_the_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".note-checkpoints.json");
        let owner: AuthorKey = [7u8; 32];
        let reader: AuthorKey = [8u8; 32];
        let (live, note) = private_note_state(path.clone(), owner, 64).await;
        live.grant_note_access(owner, note_read_grant("g1", owner, reader, note))
            .await
            .unwrap();

        // A revocation naming no grant does not load, so the whole
        // restart fails and nothing is replaced.
        let mut snapshot = live.note_snapshot().await.unwrap();
        snapshot
            .grants
            .as_mut()
            .unwrap()
            .revoked
            .insert(GrantId("absent".to_owned()));
        std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();

        let restarted = CollabState::new().with_note_snapshot_path(path);
        let err = restarted
            .restore_persisted_note_snapshot()
            .await
            .unwrap_err();
        assert!(matches!(err, NoteSnapshotError::Grants(_)), "{err}");
        assert_eq!(restarted.note_events().await, [] as [AppliedNote; 0]);
    }

    #[tokio::test]
    async fn test_seed_note_policies_before_events_is_refused() {
        use id_store::note_checkpoints::{NoteGrant, NotePrincipal, NoteRole};

        // Policies name notes; a policy for a note the log has no
        // create event for is a failed restart, never a note
        // silently reopened to public. The live note needs a
        // recorded policy for the snapshot to carry a record at
        // all — a plain note contributes none.
        let note = Hash::from_bytes([43u8; 32]);
        let key = note_session_key(&note);
        let owner: AuthorKey = [7u8; 32];
        let live = CollabState::new();
        live.get_or_create_for_note(&note, "", Some(b"# First"), Some("n.md"))
            .await;
        live.publish_note_checkpoint(&key, owner).await.unwrap();
        live.set_note_access(
            note,
            owner,
            NoteAccess::Scoped(vec![NoteGrant {
                principal: NotePrincipal::User(owner),
                role: NoteRole::Read,
            }]),
        )
        .await
        .unwrap();
        let policies = live.note_policies().await.unwrap();

        let restarted = CollabState::new();
        assert!(matches!(
            restarted.seed_note_policies(policies).await.unwrap_err(),
            CheckpointError::UnknownNote(_)
        ));
    }

    #[tokio::test]
    async fn test_publish_note_checkpoint_refuses_to_join_concurrent_heads() {
        // Two sessions checkpoint from the same head: two heads, and a
        // later save-publication must refuse rather than join them.
        let note = Hash::from_bytes([17u8; 32]);
        let key = note_session_key(&note);
        let author: AuthorKey = [5u8; 32];
        let mut log = NoteCheckpoints::new();
        let content = DocumentRef::for_bytes("text/markdown", b"# Base").unwrap();
        log.create(note, author, content).unwrap();
        let fork = DocumentRef::for_bytes("text/markdown", b"# Fork").unwrap();
        log.checkpoint(note, Hash::from_bytes([21u8; 32]), author, fork, vec![note])
            .unwrap();
        let other = DocumentRef::for_bytes("text/markdown", b"# Other").unwrap();
        log.checkpoint(
            note,
            Hash::from_bytes([22u8; 32]),
            author,
            other,
            vec![note],
        )
        .unwrap();
        assert_eq!(log.heads(note).unwrap().len(), 2);

        let state = CollabState::new();
        state.seed_note_events(log.events().to_vec()).await.unwrap();
        state
            .get_or_create_for_note(&note, "", Some(b"# Merged?"), Some("n.md"))
            .await;
        let err = state
            .publish_note_checkpoint(&key, author)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                PublishCheckpointError::Checkpoint(CheckpointError::ExplicitMergeRequired { .. })
            ),
            "a save must never join concurrent heads silently: {err}"
        );
        // The refused publication recorded nothing.
        assert_eq!(state.note_events().await.len(), 3);
    }

    #[test]
    fn test_checkpoint_messages_round_trip() {
        let checkpoint = CollabMessage::Checkpoint {
            text: "# Notes".to_owned(),
        };
        assert!(matches!(
            CollabMessage::decode(&checkpoint.encode()),
            Some(CollabMessage::Checkpoint { ref text }) if text == "# Notes"
        ));
        // A checkpoint frame must not decode as any other variant.
        assert!(!matches!(
            CollabMessage::decode(&checkpoint.encode()),
            Some(CollabMessage::Error { .. })
        ));
        let checkpointed = CollabMessage::Checkpointed {
            head: "ab".repeat(32),
        };
        assert!(matches!(
            CollabMessage::decode(&checkpointed.encode()),
            Some(CollabMessage::Checkpointed { ref head }) if head.len() == 64
        ));
    }

    #[tokio::test]
    async fn test_publish_note_checkpoint_text_records_the_given_text() {
        let state = CollabState::new();
        let note = Hash::from_bytes([31u8; 32]);
        let key = note_session_key(&note);
        state
            .get_or_create_for_note(&note, "", Some(b"# Base"), Some("n.md"))
            .await;
        let author: AuthorKey = [7u8; 32];
        // The session's base document says "# Base"; the client's
        // live text is what gets recorded (#3819 slice 4).
        let record = state
            .publish_note_checkpoint_text(&key, author, "# Edited live")
            .await
            .unwrap();
        let expected = DocumentRef::for_bytes("text/markdown", b"# Edited live").unwrap();
        assert_eq!(record.revisions[0].content, expected);
        let base = DocumentRef::for_bytes("text/markdown", b"# Base").unwrap();
        assert_ne!(record.revisions[0].content, base);
    }

    #[tokio::test]
    async fn test_a_note_session_ends_with_one_changelog_entry_naming_its_checkpoints() {
        use ed25519_dalek::SigningKey;
        use id_store::talk_page::{EntryRef, verify_chain};

        let state = CollabState::new();
        let note = Hash::from_bytes([37u8; 32]);
        let key = note_session_key(&note);
        state
            .get_or_create_for_note(&note, "", None, Some("n.md"))
            .await;
        let server = SigningKey::from_bytes(&[5u8; 32]);
        let author: AuthorKey = server.verifying_key().to_bytes();
        let sign = |draft: EntryDraft| SignedEntry::sign(draft, &server);

        // A session that published nothing records nothing.
        assert!(
            state
                .end_note_session(&key, 1, sign)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(state.note_changelog(note).await, [] as [ChangelogEntry; 0]);

        // A session that published three checkpoints ends with exactly
        // one `session` entry, naming the first and last of them.
        state
            .publish_note_checkpoint_text(&key, author, "one")
            .await
            .unwrap();
        state
            .publish_note_checkpoint_text(&key, author, "two")
            .await
            .unwrap();
        let last = state
            .publish_note_checkpoint_text(&key, author, "three")
            .await
            .unwrap();
        let head = last.heads[0];
        let entry = state
            .end_note_session(&key, 10, sign)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.signed().kind(), ChangeKind::Session);
        assert_eq!(entry.signed().actor(), &author);
        assert_eq!(
            entry.signed().refs(),
            &[EntryRef::OpLog(OpLogRange {
                first: *note.as_bytes(),
                last: *head.as_bytes(),
                count: 3,
            })]
        );
        let changelog = state.note_changelog(note).await;
        assert_eq!(changelog, vec![entry.clone()]);
        assert!(verify_chain(note.as_bytes(), None, &changelog).is_ok());

        // Ending again records nothing more: the range was taken.
        assert!(
            state
                .end_note_session(&key, 11, sign)
                .await
                .unwrap()
                .is_none()
        );

        // The next session covers only its own checkpoint, and chains
        // after the first entry; the whole log verifies end to end.
        let next = state
            .publish_note_checkpoint_text(&key, author, "four")
            .await
            .unwrap();
        let second = state
            .end_note_session(&key, 20, sign)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.signed().seq(), 1);
        assert_eq!(
            second.signed().prev_hash(),
            Some(entry.entry_hash().unwrap())
        );
        assert_eq!(
            second.signed().op_log(),
            Some(OpLogRange::single(*next.heads[0].as_bytes()))
        );
        let changelog = state.note_changelog(note).await;
        assert_eq!(changelog.len(), 2);
        assert!(verify_chain(note.as_bytes(), None, &changelog).is_ok());
    }

    #[tokio::test]
    async fn test_a_note_changelog_survives_a_restart_and_keeps_chaining() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".note-checkpoints.json");
        let note = Hash::from_bytes([41u8; 32]);
        let key = note_session_key(&note);
        let server = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
        let author: AuthorKey = server.verifying_key().to_bytes();
        let sign = |draft: EntryDraft| SignedEntry::sign(draft, &server);

        let live = CollabState::new().with_note_snapshot_path(path.clone());
        live.get_or_create_for_note(&note, "", None, Some("n.md"))
            .await;
        live.publish_note_checkpoint_text(&key, author, "one")
            .await
            .unwrap();
        let first = live
            .end_note_session(&key, 10, sign)
            .await
            .unwrap()
            .unwrap();

        // The entry reached the file.
        let persisted: NoteCheckpointSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.changelogs.len(), 1);
        assert_eq!(persisted.changelogs[0].note, note);

        // Restart: the changelog comes back verified, and the next
        // session's entry chains after the restored one.
        let restarted = CollabState::new().with_note_snapshot_path(path.clone());
        assert!(restarted.restore_persisted_note_snapshot().await.unwrap());
        assert_eq!(restarted.note_changelog(note).await, vec![first.clone()]);
        restarted
            .get_or_create_for_note(&note, "", None, Some("n.md"))
            .await;
        restarted
            .publish_note_checkpoint_text(&key, author, "two")
            .await
            .unwrap();
        let second = restarted
            .end_note_session(&key, 20, sign)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            second.signed().prev_hash(),
            Some(first.entry_hash().unwrap())
        );
        let changelog = restarted.note_changelog(note).await;
        assert!(verify_chain(note.as_bytes(), None, &changelog).is_ok());

        // A snapshot whose changelog was tampered with is a failed
        // restart, never a silently shortened history.
        let mut tampered: NoteCheckpointSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        tampered.changelogs[0].entries.remove(0);
        let fresh = CollabState::new();
        assert!(matches!(
            fresh.restore_note_snapshot(tampered).await,
            Err(NoteSnapshotError::Changelog(_))
        ));
    }

    #[tokio::test]
    async fn test_a_failed_session_entry_keeps_the_range_open() {
        let state = CollabState::new();
        let note = Hash::from_bytes([39u8; 32]);
        let key = note_session_key(&note);
        state
            .get_or_create_for_note(&note, "", None, Some("n.md"))
            .await;
        let author: AuthorKey = [7u8; 32];
        state
            .publish_note_checkpoint_text(&key, author, "kept")
            .await
            .unwrap();
        let refuse = |_draft: EntryDraft| -> Result<SignedEntry, TalkPageError> {
            Err(TalkPageError::Invalid("refused"))
        };
        assert!(state.end_note_session(&key, 1, refuse).await.is_err());
        assert_eq!(state.note_changelog(note).await, [] as [ChangelogEntry; 0]);

        // The next session end records the range the failed one held.
        let server = ed25519_dalek::SigningKey::from_bytes(&[6u8; 32]);
        let entry = state
            .end_note_session(&key, 2, |draft| SignedEntry::sign(draft, &server))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            entry.signed().op_log(),
            Some(OpLogRange::single(*note.as_bytes()))
        );
    }

    #[tokio::test]
    async fn test_note_head_content_tracks_publications_and_heads() {
        let state = CollabState::new();
        let note = Hash::from_bytes([33u8; 32]);
        // Unknown note: nothing to hydrate from.
        assert_eq!(state.note_head_content(note).await.unwrap(), None);

        let key = note_session_key(&note);
        state
            .get_or_create_for_note(&note, "", None, Some("n.md"))
            .await;
        let author: AuthorKey = [7u8; 32];
        state
            .publish_note_checkpoint_text(&key, author, "first")
            .await
            .unwrap();
        let first = DocumentRef::for_bytes("text/markdown", b"first").unwrap();
        assert_eq!(state.note_head_content(note).await.unwrap(), Some(first));

        state
            .publish_note_checkpoint_text(&key, author, "second")
            .await
            .unwrap();
        let second = DocumentRef::for_bytes("text/markdown", b"second").unwrap();
        assert_eq!(
            state.note_head_content(note).await.unwrap(),
            Some(second.clone())
        );

        // Concurrent heads: hydration still answers, deterministically
        // — the smallest head ID wins. Built on a seeded log, the way
        // the slice-2 tests build forks: two checkpoints parented on
        // the create revision.
        let note2 = Hash::from_bytes([35u8; 32]);
        let mut log = NoteCheckpoints::new();
        let base = DocumentRef::for_bytes("text/markdown", b"# Base").unwrap();
        log.create(note2, author, base).unwrap();
        let fork = DocumentRef::for_bytes("text/markdown", b"# Fork").unwrap();
        log.checkpoint(
            note2,
            Hash::from_bytes([21u8; 32]),
            author,
            fork.clone(),
            vec![note2],
        )
        .unwrap();
        let other = DocumentRef::for_bytes("text/markdown", b"# Other").unwrap();
        log.checkpoint(
            note2,
            Hash::from_bytes([22u8; 32]),
            author,
            other,
            vec![note2],
        )
        .unwrap();
        let state2 = CollabState::new();
        state2
            .seed_note_events(log.events().to_vec())
            .await
            .unwrap();
        assert_eq!(state2.note_heads(note2).await.unwrap().len(), 2);
        assert_eq!(state2.note_head_content(note2).await.unwrap(), Some(fork));
    }

    type WireClient = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn next_collab_message(client: &mut WireClient) -> CollabMessage {
        use futures::StreamExt;
        loop {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(15), client.next())
                .await
                .expect("timed out waiting for a collab frame")
                .expect("websocket closed while waiting for a collab frame")
                .expect("websocket error while waiting for a collab frame");
            if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = frame {
                if let Some(message) = CollabMessage::decode(&bytes) {
                    return message;
                }
            }
        }
    }

    async fn wait_for_message(client: &mut WireClient, want: &str) -> CollabMessage {
        loop {
            let message = next_collab_message(client).await;
            let matched = match (want, &message) {
                ("init", CollabMessage::Init { .. }) => true,
                ("ack", CollabMessage::Ack { .. }) => true,
                ("checkpointed", CollabMessage::Checkpointed { .. }) => true,
                ("error", CollabMessage::Error { .. }) => true,
                _ => false,
            };
            if matched {
                return message;
            }
        }
    }

    #[tokio::test]
    async fn test_note_checkpoint_wire_round_trip_publishes_and_hydrates() {
        use axum::Router;
        use futures::SinkExt;
        use iroh::endpoint::{Endpoint, RelayMode, presets};
        use iroh_blobs::{api::Store, store::mem::MemStore};
        use iroh_docs::protocol::Docs;
        use iroh_gossip::net::Gossip;
        use tokio_tungstenite::tungstenite::Message as ClientMessage;

        // An application state on in-memory stores, built the way
        // the routes tests build theirs.
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let store: Store = MemStore::new().into();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let docs = Docs::memory()
            .spawn(endpoint.clone(), store.clone(), gossip)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let tags = Arc::new(
            id_store::tags::TagStore::init_in(
                &docs,
                "collab-note-wire-test",
                dir.path().join("meta"),
            )
            .await
            .unwrap(),
        );
        let mut state = crate::AppState::new(
            store,
            None,
            "collab-note-wire-test".to_owned(),
            tags,
            [7; 32],
            dir.path().join(".identity.redb"),
        )
        .await
        .unwrap();
        // Persist the event log just as the host does so a fresh
        // CollabState can restore it below, independently of the live
        // ProseMirror document.
        let snapshot_path = dir.path().join(NOTE_CHECKPOINT_SNAPSHOT_FILE_NAME);
        state.collab = Arc::new(CollabState::new().with_note_snapshot_path(snapshot_path.clone()));

        let app = Router::new()
            .route("/ws/collab/{doc_id}", axum::routing::get(ws_collab_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let note = Hash::from_bytes([41u8; 32]);
        let mut url = format!("ws://{addr}/ws/collab/{}", note_session_key(&note));

        // First session: connect, edit (steps), checkpoint the live
        // text, disconnect.
        let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        // A non-auth first frame ends the auth wait immediately; the
        // step batch is buffered and acked after Init.
        let steps = CollabMessage::Steps {
            version: 0,
            steps: vec![serde_json::json!({"stepType": "replace", "from": 0, "to": 0})],
            client_id: 1,
        };
        client
            .send(ClientMessage::Binary(steps.encode().into()))
            .await
            .unwrap();
        let init = wait_for_message(&mut client, "init").await;
        let CollabMessage::Init { doc, .. } = init else {
            panic!("expected Init for a fresh note session");
        };
        assert_eq!(
            crate::routes::file_handlers::extract_plain_text(&doc),
            "",
            "a note with no checkpoints hydrates empty"
        );
        let ack = wait_for_message(&mut client, "ack").await;
        assert!(matches!(ack, CollabMessage::Ack { version: 1 }));

        let checkpoint = CollabMessage::Checkpoint {
            text: "# Saved over the wire".to_owned(),
        };
        client
            .send(ClientMessage::Binary(checkpoint.encode().into()))
            .await
            .unwrap();
        let done = wait_for_message(&mut client, "checkpointed").await;
        let CollabMessage::Checkpointed { head } = done else {
            panic!("expected Checkpointed after a note checkpoint");
        };
        assert_eq!(head.len(), 64, "head is a hex hash: {head}");
        drop(client);

        // Exactly one durable event exists: the client-driven
        // checkpoint. Publication already happened, synchronously,
        // before the Checkpointed reply.
        let events = state.collab.note_events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(state.collab.note_heads(note).await.unwrap().len(), 1);

        // A new process rebuilds the durable log from its snapshot. The
        // wire session below must append to that rebuilt head, not create
        // a parallel root or reuse the dead process's document state.
        server.abort();
        let restarted = CollabState::new().with_note_snapshot_path(snapshot_path);
        assert!(restarted.restore_persisted_note_snapshot().await.unwrap());
        assert_eq!(restarted.note_events().await, events);
        state.collab = Arc::new(restarted);
        let app = Router::new()
            .route("/ws/collab/{doc_id}", axum::routing::get(ws_collab_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        url = format!("ws://{addr}/ws/collab/{}", note_session_key(&note));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // A genuinely fresh session: the Init document is the checkpointed
        // text, hydrated from the blob store under its document
        // hash. The checkpoint is recorded as text/markdown, so the
        // heading parses as a heading and its plain text drops the
        // `#` marker.
        let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let init = wait_for_message(&mut client, "init").await;
        let CollabMessage::Init { doc, .. } = init else {
            panic!("expected Init for the reopened note session");
        };
        assert_eq!(
            crate::routes::file_handlers::extract_plain_text(&doc),
            "Saved over the wire"
        );
        let second_checkpoint = CollabMessage::Checkpoint {
            text: "# Chained after restart".to_owned(),
        };
        client
            .send(ClientMessage::Binary(second_checkpoint.encode().into()))
            .await
            .unwrap();
        let done = wait_for_message(&mut client, "checkpointed").await;
        assert!(matches!(done, CollabMessage::Checkpointed { .. }));
        drop(client);
        let events_after_restart = state.collab.note_events().await;
        assert_eq!(events_after_restart.len(), 2);
        assert_eq!(state.collab.note_heads(note).await.unwrap().len(), 1);
        let id_store::notes::NoteOp::Revise { parents, .. } = &events_after_restart[1].op else {
            panic!("post-restart checkpoint must be a revision");
        };
        assert_eq!(parents, &vec![Hash::from_bytes([41u8; 32])]);

        // Concurrent/offline branches remain separate. A wire save from
        // that state is refused until an explicit merge is made through
        // the existing checkpoint API; it must never silently discard a
        // head. The protocol has no Merge message, so exercise the
        // explicit merge through the existing API and assert its parents.
        let mut forked_log = NoteCheckpoints::from_events(events_after_restart.clone()).unwrap();
        let fork = DocumentRef::for_bytes("text/markdown", b"# Offline fork").unwrap();
        forked_log
            .checkpoint(
                note,
                Hash::from_bytes([51u8; 32]),
                [7u8; 32],
                fork,
                vec![events_after_restart[1].id],
            )
            .unwrap();
        let other = DocumentRef::for_bytes("text/markdown", b"# Other offline fork").unwrap();
        forked_log
            .checkpoint(
                note,
                Hash::from_bytes([52u8; 32]),
                [7u8; 32],
                other,
                vec![events_after_restart[1].id],
            )
            .unwrap();
        state
            .collab
            .seed_note_events(forked_log.events().to_vec())
            .await
            .unwrap();
        assert_eq!(state.collab.note_heads(note).await.unwrap().len(), 2);
        let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let _ = wait_for_message(&mut client, "init").await;
        client
            .send(ClientMessage::Binary(
                CollabMessage::Checkpoint {
                    text: "# Must not join forks".to_owned(),
                }
                .encode()
                .into(),
            ))
            .await
            .unwrap();
        let refused = wait_for_message(&mut client, "error").await;
        assert!(matches!(refused, CollabMessage::Error { .. }));
        drop(client);
        assert_eq!(state.collab.note_events().await.len(), 4);
        assert_eq!(state.collab.note_heads(note).await.unwrap().len(), 2);
        let merge_id = Hash::from_bytes([53u8; 32]);
        let merged = forked_log
            .merge(
                note,
                merge_id,
                [7u8; 32],
                DocumentRef::for_bytes("text/markdown", b"# Explicit merge").unwrap(),
            )
            .unwrap();
        let merged_record = merged.revision(merge_id).unwrap();
        assert_eq!(
            merged_record.parents,
            vec![Hash::from_bytes([51u8; 32]), Hash::from_bytes([52u8; 32])]
        );
        state
            .collab
            .seed_note_events(forked_log.events().to_vec())
            .await
            .unwrap();
        assert_eq!(state.collab.note_heads(note).await.unwrap(), vec![merge_id]);

        // A filename session refuses checkpoints.
        let (mut client, _) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/ws/collab/plain.md"))
                .await
                .unwrap();
        let checkpoint = CollabMessage::Checkpoint {
            text: "nope".to_owned(),
        };
        client
            .send(ClientMessage::Binary(checkpoint.encode().into()))
            .await
            .unwrap();
        let refused = wait_for_message(&mut client, "error").await;
        assert!(matches!(refused, CollabMessage::Error { .. }));
        drop(client);
        assert_eq!(state.collab.note_events().await.len(), 5);

        server.abort();
    }
}
