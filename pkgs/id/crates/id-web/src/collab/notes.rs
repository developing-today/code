//! Note checkpoint persistence for collaborative editing (#16255 slice 7).
//!
//! The durable snapshot of the note checkpoint log: the types a host
//! persists across a restart, and the helpers that write and restore
//! them. Moved out of [`super::collab`] (which was over the
//! production-line-guard limit) with no behavior change; [`super::collab`]
//! re-exports every name so all existing paths keep working.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use id_core::waiting::AuthorKey;
use id_grants::{GrantError, GrantSetSnapshot, LoadError};
use id_store::documents::DocumentRef;
use id_store::note_checkpoints::{CheckpointError, NotePolicyRecord};
use id_store::notes::AppliedNote;
use id_store::talk_page::{ChangelogEntry, TalkPageError, verify_chain};
use p2panda_core::Hash;

use super::SessionEntryError;

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
pub(crate) fn parse_note_hash(id: &str) -> Option<Hash> {
    let hash: Hash = id.parse().ok()?;
    (hash.to_hex() == id).then_some(hash)
}

/// Decodes the changelogs of a persisted snapshot (#14272), verifying
/// each chain end to end from its first entry. A note named twice is
/// refused: a snapshot holds one chain per note.
pub(crate) fn restore_changelogs(
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

/// Writes a snapshot to `path` durably and atomically, through the
/// crate-wide durable-write idiom (#16358): the new document is
/// written under a temporary name in the same directory, fsynced,
/// renamed over the old one, and the parent directory is synced
/// after the rename. A crash mid-write therefore leaves the previous
/// snapshot (or none) — never a torn one for the next restart to
/// read — and once this returns `Ok(())` the new snapshot itself
/// survives a crash: neither its bytes nor the rename that installed
/// it can be silently rolled back. A failure of the post-rename
/// directory sync surfaces as an error, not success, even though the
/// new file is already installed at that point.
pub(crate) fn write_note_snapshot_file(
    path: &std::path::Path,
    snapshot: &NoteCheckpointSnapshot,
) -> Result<(), NoteSnapshotError> {
    write_note_snapshot_file_with_parent_sync(path, snapshot, |dir| {
        std::fs::File::open(dir)?.sync_all()
    })
}

/// [`write_note_snapshot_file`] with the parent-directory sync step
/// supplied by the caller: the same injectable seam
/// [`id_core::durable_write::write_durable_with_parent_sync`]
/// provides, so a test can observe — and inject — the post-rename
/// sync failure.
pub(crate) fn write_note_snapshot_file_with_parent_sync(
    path: &std::path::Path,
    snapshot: &NoteCheckpointSnapshot,
    sync_parent: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> Result<(), NoteSnapshotError> {
    let bytes =
        serde_json::to_vec_pretty(snapshot).map_err(|source| NoteSnapshotError::Corrupt {
            path: path.to_path_buf(),
            source,
        })?;
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    id_core::durable_write::write_durable_with_parent_sync(path, &tmp, &bytes, sync_parent)
        .map_err(|source| NoteSnapshotError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other(source),
        })?;
    Ok(())
}

/// Mints the event ID for a checkpoint published from a live session:
/// a digest of everything the revision is — its note, its author, the
/// heads it builds on, its content, and the session version that
/// produced it — so two saves of identical bytes at different versions
/// are still distinct events, and the same save replayed is the same
/// event.
pub(crate) fn mint_checkpoint_event_id(
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
pub(crate) fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// A `doc_id` naming a note session over the wire: the
/// [`note_session_key`] form, `note:<note id hex>` (#3819 slice 2).
/// Anything else is a filename, exactly as before.
pub(crate) fn parse_note_doc_id(doc_id: &str) -> Option<Hash> {
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
