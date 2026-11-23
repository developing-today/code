//! Persistence for record nodes (#12492, #12232 slice 4).
//!
//! Step 2 of the walking skeleton runs between two in-process
//! [`Replica`]s: everything a node holds — the versions its
//! [`ConflictStore`] admitted and the conflict reports the store
//! built — is in memory, and a process exit loses all of it. This
//! module keeps versions and reports on disk through
//! `id_store::content_store`, so a node restarts with what it had.
//!
//! # What is kept
//!
//! Two records, both ingested as verified content under
//! `raw-blake3-256` content ids (a [`ContentStore`] recomputes the
//! digest on every read, so what loads is what was written):
//!
//! - a **version record**: the gate [`Envelope`] a node admitted,
//!   the ingress path and source it arrived by, and the time it was
//!   admitted at. The envelope has no canonical encoding in B32;
//!   this module defines one.
//! - a **report record**: a [`StoredReport`] — the report's
//!   canonical bytes plus its status.
//!
//! A small index file in the node directory names the records:
//! version records in admission order, and the latest record per
//! report id. The index is rewritten (write-then-rename) at every
//! write-through point. The blob backend is iroh's `FsStore`, the
//! on-disk backend production uses.
//!
//! # Write-through points
//!
//! [`StoredReplica`] owns a [`Replica`] and a [`NodeStore`] and
//! presents the surface the sync path uses. A version admitted by
//! `produce` or `receive` is persisted in that call; a version
//! admitted during an exchange is persisted at the exchange's end,
//! from the exchange's own outcomes; every new or changed report is
//! persisted at the same points. Each write-through point publishes
//! the version and the reports it changed in one index update, so a
//! crash leaves the old index (the admission is lost, and
//! anti-entropy re-sends it) or the new one — never a version
//! without its reports. A crash between an admission and its write
//! loses that admission — and nothing else: anti-entropy is
//! idempotent, so a peer re-sends what the restarted node lacks. A
//! A persistence failure or cancellation before the write-through
//! point commits rolls the admission back instead (see
//! [`StoredReplica::produce`]), so an unpersisted version is neither
//! kept live nor left half-logged.
//!
//! # Opening is replay
//!
//! Opening a node replays its version records, in admission order,
//! through `Replica::receive` — the same gate, the same join check,
//! the same `now` each version was admitted at — and rebuilds the
//! store exactly: versions, heads, held versions, reports and their
//! statuses are all derived from the admissions, and a `Replica`'s
//! reports change only through admissions. Persisted report records
//! are then checked against the rebuilt reports. A report id is the
//! hash of the report's canonical bytes, so a record under an id
//! the replay rebuilt can only be stale in its status: it predates
//! an admission the version log already holds, and is re-written
//! from the replay. A record missing because the process died
//! between the version write and the report write is re-written
//! too. A persisted report id the replay never rebuilt is
//! [`RecordStoreError::ReportDiverged`], never a silent heal.
//!
//! # The transport, composed
//!
//! The exchange also runs over the wire (B2(c),
//! `super::exchange_net`). [`StoredReplica::reconcile_remote_with`]
//! is the initiator half: it runs the remote exchange against the
//! wrapped replica and persists at the exchange's end, from the
//! exchange's own outcomes — the versions this node admitted in the
//! backward transfer, under the responder's name as source — with
//! the same rollback discipline as [`StoredReplica::reconcile_with`].
//! [`ServingNode`] is the responder half: it holds the replica
//! behind the exchange protocol and persists each session's
//! admissions (reported by the protocol's session observer) at the
//! session's end, versions first and changed reports after, in one
//! index update per session.
//!
//! # Scope limits
//!
//! - Refusals are not persisted. A refused version is refused
//!   again, deterministically, when a peer re-offers it after a
//!   restart; only the never-offer-again optimisation is lost.
//! - Report expiry (`ConflictStore::expire`) is not reachable
//!   through a `Replica`, so replay reproduces the reports map
//!   exactly. Wiring expiry into the persisted node is later work.
//! - One writer per node directory; no locking.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use id_core::content_id::raw_content_id;
use id_core::durable_write::DurableWriteError;
use id_core::id_shape::{ContentId, HashProfile};
use id_store::content_store::{BlobBackend, ContentStore, ContentStoreConfig, ContentStoreError};
use iroh::{Endpoint, EndpointAddr};
use iroh_blobs::Hash;
use iroh_blobs::api::blobs::Blobs;
use iroh_blobs::store::fs::FsStore;
use tokio::sync::{Mutex as TokioMutex, mpsc};

use super::exchange_net::{ExchangeProtocol, SessionAdmissions, reconcile_remote_named};
use super::slices::b07_conflict_report::{
    ConflictReport, ConflictStore, IngestError, IngestOutcome, ReportId, ReportStatus,
    StoredReport, VersionRef,
};
use super::slices::b08_sync_scope::SyncScopeConfig;
use super::slices::b09_anti_entropy::{Exchange, Replica, reconcile};
use super::slices::b32_ingest_gate::{Envelope, EventId, IngressPath, LogicalTime, payload_digest};

/// Domain prefix of an encoded gate envelope.
const ENVELOPE_DOMAIN: &[u8] = b"id-sync/record-store-envelope/v1";
/// Domain prefix of an encoded version record.
const VERSION_RECORD_DOMAIN: &[u8] = b"id-sync/record-store-version/v1";
/// Domain prefix of an encoded report record.
const REPORT_RECORD_DOMAIN: &[u8] = b"id-sync/record-store-report/v1";
/// Domain prefix of the node index file.
const INDEX_DOMAIN: &[u8] = b"id-sync/record-store-index/v1";
/// The report domain B07 computes report ids over (see
/// `ConflictReport::canonical_bytes`); a report record carries the
/// canonical bytes unchanged, and the decoder reads them back.
const REPORT_DOMAIN: &[u8] = b"id-sync/b07-conflict-report/v1";

/// The file in a node directory that names its records.
const INDEX_FILE: &str = "index";
/// The directory in a node directory that holds the blob store.
const BLOBS_DIR: &str = "blobs";

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

/// The ingress path tag B07's canonical form uses; the record
/// codecs here use the same tags.
const fn path_tag(path: IngressPath) -> u8 {
    match path {
        IngressPath::Local => 0,
        IngressPath::Loopback => 1,
        IngressPath::Internal => 2,
        IngressPath::Hosted => 3,
        IngressPath::Remote => 4,
    }
}

const fn path_from_tag(tag: u8) -> Option<IngressPath> {
    match tag {
        0 => Some(IngressPath::Local),
        1 => Some(IngressPath::Loopback),
        2 => Some(IngressPath::Internal),
        3 => Some(IngressPath::Hosted),
        4 => Some(IngressPath::Remote),
        _ => None,
    }
}

/// Why a record-store operation failed.
#[derive(Debug)]
pub enum RecordStoreError {
    /// The content store failed.
    Content(ContentStoreError),
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// Replacing the index failed; the error records whether rename already installed it.
    DurableWrite(DurableWriteError),
    /// Opening the on-disk blob backend failed.
    Backend(String),
    /// A remote exchange (see
    /// [`StoredReplica::reconcile_remote_with`]) failed before its
    /// write-through point; nothing was persisted.
    Exchange(String),
    /// Stored bytes are not the record they were written as.
    Decode(&'static str),
    /// The index names content the backend does not hold.
    MissingContent([u8; 32]),
    /// A persisted version was refused when replayed at open: the
    /// log and the store's policy disagree.
    ReplayRefused {
        /// The version that was refused.
        event_id: EventId,
        /// Why the store refused it on replay.
        error: IngestError,
    },
    /// A persisted report record does not match the report the
    /// version replay rebuilt under the same id.
    ReportDiverged(ReportId),
}

impl fmt::Display for RecordStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Content(error) => write!(f, "content store failed: {error}"),
            Self::Io(error) => write!(f, "record store io failed: {error}"),
            Self::DurableWrite(error) => write!(f, "record store durable write failed: {error}"),
            Self::Backend(error) => write!(f, "blob backend failed: {error}"),
            Self::Exchange(error) => write!(f, "remote exchange failed: {error}"),
            Self::Decode(what) => write!(f, "stored record does not decode: {what}"),
            Self::MissingContent(digest) => {
                write!(f, "index names missing content {}", hex32(digest))
            }
            Self::ReplayRefused { event_id, error } => write!(
                f,
                "persisted version {} was refused on replay: {error:?}",
                hex32(event_id)
            ),
            Self::ReportDiverged(report_id) => write!(
                f,
                "persisted report {} does not match the rebuilt report",
                hex32(report_id)
            ),
        }
    }
}

impl std::error::Error for RecordStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DurableWrite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Content(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ContentStoreError> for RecordStoreError {
    fn from(error: ContentStoreError) -> Self {
        Self::Content(error)
    }
}

impl From<std::io::Error> for RecordStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

fn hex32(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Why a [`StoredReplica`] admission failed: the store refused the
/// version, or an admitted version could not be persisted.
#[derive(Debug)]
pub enum StoredError {
    /// The replica's store (and its gate) refused the envelope.
    Ingest(IngestError),
    /// The version was admitted but writing it (or a report it
    /// changed) to disk failed.
    Store(RecordStoreError),
}

impl fmt::Display for StoredError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ingest(error) => write!(f, "ingest refused the version: {error:?}"),
            Self::Store(error) => write!(f, "persisting the version failed: {error}"),
        }
    }
}

impl std::error::Error for StoredError {}

/// A byte cursor over a stored record; every read is checked.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8], what: &'static str) -> Self {
        Self {
            bytes,
            pos: 0,
            what,
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], RecordStoreError> {
        let end = self
            .pos
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(RecordStoreError::Decode(self.what))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn take32(&mut self) -> Result<[u8; 32], RecordStoreError> {
        let slice = self.take(32)?;
        let mut out = [0u8; 32];
        out.copy_from_slice(slice);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, RecordStoreError> {
        Ok(self.take(1)?[0])
    }

    fn u64(&mut self) -> Result<u64, RecordStoreError> {
        let slice = self.take(8)?;
        let mut out = [0u8; 8];
        out.copy_from_slice(slice);
        Ok(u64::from_be_bytes(out))
    }

    fn field(&mut self) -> Result<&'a [u8], RecordStoreError> {
        let len = usize::try_from(self.u64()?).map_err(|_| RecordStoreError::Decode(self.what))?;
        self.take(len)
    }

    fn ids(&mut self) -> Result<Vec<[u8; 32]>, RecordStoreError> {
        let count =
            usize::try_from(self.u64()?).map_err(|_| RecordStoreError::Decode(self.what))?;
        let mut out = Vec::with_capacity(count.min(1 << 16));
        for _ in 0..count {
            out.push(self.take32()?);
        }
        Ok(out)
    }

    fn optional(&mut self) -> Result<Option<[u8; 32]>, RecordStoreError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.take32()?)),
            _ => Err(RecordStoreError::Decode(self.what)),
        }
    }

    fn domain(&mut self, expected: &[u8]) -> Result<(), RecordStoreError> {
        if self.take(expected.len())? == expected {
            Ok(())
        } else {
            Err(RecordStoreError::Decode(self.what))
        }
    }

    fn finish(&self) -> Result<(), RecordStoreError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(RecordStoreError::Decode(self.what))
        }
    }
}

fn text(field: &[u8], what: &'static str) -> Result<String, RecordStoreError> {
    String::from_utf8(field.to_vec()).map_err(|_| RecordStoreError::Decode(what))
}

/// The canonical encoding of a gate envelope: domain-separated,
/// with every field the gate's admission depends on, in the field
/// order of B32's signed preimage where the two overlap.
pub fn encode_envelope(envelope: &Envelope) -> Vec<u8> {
    let mut out = ENVELOPE_DOMAIN.to_vec();
    push_field(&mut out, envelope.signer.as_bytes());
    out.extend_from_slice(&envelope.verifying_key);
    match &envelope.signature {
        None => out.push(0),
        Some(signature) => {
            out.push(1);
            out.extend_from_slice(signature);
        }
    }
    push_field(&mut out, envelope.scope.as_bytes());
    push_field(&mut out, &envelope.payload);
    out.extend_from_slice(&envelope.payload_digest);
    out.extend_from_slice(&envelope.event_id);
    out.extend_from_slice(&envelope.issued_at.to_be_bytes());
    out
}

/// Decode an envelope written by [`encode_envelope`]. The declared
/// payload digest is recomputed and must match, so a torn or
/// tampered record is refused here as well as by the content store.
///
/// # Errors
///
/// [`RecordStoreError::Decode`] on any malformed field.
pub fn decode_envelope(bytes: &[u8]) -> Result<Envelope, RecordStoreError> {
    let mut cursor = Cursor::new(bytes, "envelope");
    cursor.domain(ENVELOPE_DOMAIN)?;
    let signer = text(cursor.field()?, "envelope")?;
    let verifying_key = cursor.take32()?;
    let signature = match cursor.u8()? {
        0 => None,
        1 => {
            let slice = cursor.take(64)?;
            let mut signature = [0u8; 64];
            signature.copy_from_slice(slice);
            Some(signature)
        }
        _ => return Err(RecordStoreError::Decode("envelope")),
    };
    let scope = text(cursor.field()?, "envelope")?;
    let payload = cursor.field()?.to_vec();
    let declared_digest = cursor.take32()?;
    let event_id = cursor.take32()?;
    let issued_at = cursor.u64()?;
    cursor.finish()?;
    if payload_digest(&payload) != declared_digest {
        return Err(RecordStoreError::Decode("envelope payload digest"));
    }
    Ok(Envelope {
        signer,
        verifying_key,
        signature,
        scope,
        payload,
        payload_digest: declared_digest,
        event_id,
        issued_at,
    })
}

/// One admitted version, as the node store keeps it: everything
/// replay needs to admit it again, identically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRecord {
    /// The signed gate envelope that was admitted.
    pub envelope: Envelope,
    /// The ingress path it arrived by.
    pub path: IngressPath,
    /// The peer or producer label it arrived from.
    pub source: String,
    /// The time the node admitted it at (the `now` of its ingest).
    pub admitted_at: LogicalTime,
}

impl VersionRecord {
    /// The canonical record bytes; their content id is the record's
    /// address in the node store.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = VERSION_RECORD_DOMAIN.to_vec();
        push_field(&mut out, &encode_envelope(&self.envelope));
        out.push(path_tag(self.path));
        push_field(&mut out, self.source.as_bytes());
        out.extend_from_slice(&self.admitted_at.to_be_bytes());
        out
    }

    /// Decode a record written by [`VersionRecord::encode`].
    ///
    /// # Errors
    ///
    /// [`RecordStoreError::Decode`] on any malformed field.
    pub fn decode(bytes: &[u8]) -> Result<Self, RecordStoreError> {
        let mut cursor = Cursor::new(bytes, "version record");
        cursor.domain(VERSION_RECORD_DOMAIN)?;
        let envelope = decode_envelope(cursor.field()?)?;
        let path = path_from_tag(cursor.u8()?).ok_or(RecordStoreError::Decode("version record"))?;
        let source = text(cursor.field()?, "version record")?;
        let admitted_at = cursor.u64()?;
        cursor.finish()?;
        Ok(Self {
            envelope,
            path,
            source,
            admitted_at,
        })
    }
}

/// The canonical record bytes of a stored report: its status, then
/// the report's own canonical bytes as a field.
pub fn encode_stored_report(stored: &StoredReport) -> Vec<u8> {
    let mut out = REPORT_RECORD_DOMAIN.to_vec();
    match &stored.status {
        ReportStatus::Open => out.push(0),
        ReportStatus::Resolved { by, at } => {
            out.push(1);
            out.extend_from_slice(by);
            out.extend_from_slice(&at.to_be_bytes());
        }
        ReportStatus::Superseded { by } => {
            out.push(2);
            out.extend_from_slice(by);
        }
    }
    push_field(&mut out, &stored.report.canonical_bytes());
    out
}

/// Decode a report written by [`encode_stored_report`].
///
/// # Errors
///
/// [`RecordStoreError::Decode`] on any malformed field.
pub fn decode_stored_report(bytes: &[u8]) -> Result<StoredReport, RecordStoreError> {
    let mut cursor = Cursor::new(bytes, "report record");
    cursor.domain(REPORT_RECORD_DOMAIN)?;
    let status = match cursor.u8()? {
        0 => ReportStatus::Open,
        1 => {
            let by = cursor.take32()?;
            let at = cursor.u64()?;
            ReportStatus::Resolved { by, at }
        }
        2 => ReportStatus::Superseded {
            by: cursor.take32()?,
        },
        _ => return Err(RecordStoreError::Decode("report record")),
    };
    let report = decode_report(cursor.field()?)?;
    cursor.finish()?;
    Ok(StoredReport { report, status })
}

/// Decode the canonical bytes of a [`ConflictReport`] (the form
/// `ConflictReport::canonical_bytes` writes and `report_id` hashes).
fn decode_report(bytes: &[u8]) -> Result<ConflictReport, RecordStoreError> {
    let mut cursor = Cursor::new(bytes, "conflict report");
    cursor.domain(REPORT_DOMAIN)?;
    let object_id = cursor.take32()?;
    let version_count =
        usize::try_from(cursor.u64()?).map_err(|_| RecordStoreError::Decode("conflict report"))?;
    let mut versions = Vec::with_capacity(version_count.min(1 << 16));
    for _ in 0..version_count {
        versions.push(decode_version_ref(&mut cursor)?);
    }
    let base = cursor.ids()?;
    let scope_count =
        usize::try_from(cursor.u64()?).map_err(|_| RecordStoreError::Decode("conflict report"))?;
    let mut scopes = Vec::with_capacity(scope_count.min(1 << 16));
    for _ in 0..scope_count {
        scopes.push(text(cursor.field()?, "conflict report")?);
    }
    let policy_digest = cursor.take32()?;
    let detected_at = cursor.u64()?;
    let expires_at = cursor.u64()?;
    let supersedes = cursor.optional()?;
    cursor.finish()?;
    Ok(ConflictReport {
        object_id,
        versions,
        base,
        scopes,
        policy_digest,
        detected_at,
        expires_at,
        supersedes,
    })
}

fn decode_version_ref(cursor: &mut Cursor<'_>) -> Result<VersionRef, RecordStoreError> {
    let event_id = cursor.take32()?;
    let object_id = cursor.take32()?;
    let parents = cursor.ids()?;
    let resolves = cursor.optional()?;
    let signer = text(cursor.field()?, "conflict report")?;
    let verifying_key = cursor.take32()?;
    let path = path_from_tag(cursor.u8()?).ok_or(RecordStoreError::Decode("conflict report"))?;
    let source = text(cursor.field()?, "conflict report")?;
    let scope = text(cursor.field()?, "conflict report")?;
    let issued_at = cursor.u64()?;
    let body_digest = cursor.take32()?;
    let body_len = cursor.u64()?;
    Ok(VersionRef {
        event_id,
        object_id,
        parents,
        resolves,
        signer,
        verifying_key,
        path,
        source,
        scope,
        issued_at,
        body_digest,
        body_len,
    })
}

/// The index file's contents: which records the node holds.
#[derive(Debug, Default, Clone)]
struct Index {
    /// Version records, in admission order.
    versions: Vec<ContentId>,
    /// The latest report record per report id.
    reports: BTreeMap<ReportId, ContentId>,
}

impl Index {
    fn encode(&self) -> Vec<u8> {
        let mut out = INDEX_DOMAIN.to_vec();
        out.extend_from_slice(&(self.versions.len() as u64).to_be_bytes());
        for id in &self.versions {
            out.extend_from_slice(id.digest());
        }
        out.extend_from_slice(&(self.reports.len() as u64).to_be_bytes());
        for (report_id, id) in &self.reports {
            out.extend_from_slice(report_id);
            out.extend_from_slice(id.digest());
        }
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, RecordStoreError> {
        let mut cursor = Cursor::new(bytes, "node index");
        cursor.domain(INDEX_DOMAIN)?;
        let version_count =
            usize::try_from(cursor.u64()?).map_err(|_| RecordStoreError::Decode("node index"))?;
        let mut versions = Vec::with_capacity(version_count.min(1 << 16));
        for _ in 0..version_count {
            versions.push(ContentId::new(HashProfile::RawBlake3_256, cursor.take32()?));
        }
        let report_count =
            usize::try_from(cursor.u64()?).map_err(|_| RecordStoreError::Decode("node index"))?;
        let mut reports = BTreeMap::new();
        for _ in 0..report_count {
            let report_id = cursor.take32()?;
            let id = ContentId::new(HashProfile::RawBlake3_256, cursor.take32()?);
            reports.insert(report_id, id);
        }
        cursor.finish()?;
        Ok(Self { versions, reports })
    }
}

/// One node's on-disk store: verified content plus the index that
/// names it.
#[derive(Debug)]
pub struct NodeStore<B> {
    content: ContentStore<B>,
    index_path: PathBuf,
    index: Index,
}

impl<B: BlobBackend + Sync> NodeStore<B> {
    /// A node store over `content`, with its index at `index_path`.
    /// An existing index is read; a missing one starts empty.
    ///
    /// # Errors
    ///
    /// On an unreadable or malformed index.
    pub fn new(content: ContentStore<B>, index_path: PathBuf) -> Result<Self, RecordStoreError> {
        let index = match fs::read(&index_path) {
            Ok(bytes) => Index::decode(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(error) => return Err(RecordStoreError::Io(error)),
        };
        Ok(Self {
            content,
            index_path,
            index,
        })
    }

    /// The content store the records live in.
    pub const fn content(&self) -> &ContentStore<B> {
        &self.content
    }

    async fn put(&self, bytes: &[u8]) -> Result<ContentId, RecordStoreError> {
        let id = raw_content_id(bytes);
        self.content.ingest(&id, bytes.len() as u64, bytes).await?;
        Ok(id)
    }

    async fn get(&self, id: &ContentId) -> Result<Vec<u8>, RecordStoreError> {
        self.content
            .get(id)
            .await?
            .ok_or(RecordStoreError::MissingContent(*id.digest()))
    }

    fn flush_index(&self) -> Result<(), RecordStoreError> {
        // Commit the staged index entries: one write-then-rename
        // publishes a write-through point's version and report
        // records together, so the index on disk never names a
        // version without the reports its admission changed.
        let tmp_path = self.index_path.with_extension("tmp");
        id_core::durable_write::write_durable(&self.index_path, &tmp_path, &self.index.encode())
            .map_err(RecordStoreError::DurableWrite)
    }

    /// Snapshot the staged index entries, so a failed write-through
    /// point can roll the index back to what is on disk.
    fn checkpoint(&self) -> Index {
        self.index.clone()
    }

    /// Restore an index snapshot taken by [`NodeStore::checkpoint`].
    fn restore(&mut self, checkpoint: Index) {
        self.index = checkpoint;
    }

    /// Ingest a version record and stage its index entry, in
    /// admission order. The entry reaches disk only when the caller
    /// commits the write-through point with [`NodeStore::flush_index`];
    /// staging without committing leaves the on-disk index unchanged.
    ///
    /// # Errors
    ///
    /// On a content-store failure.
    pub async fn append_version(
        &mut self,
        record: &VersionRecord,
    ) -> Result<ContentId, RecordStoreError> {
        let id = self.put(&record.encode()).await?;
        self.index.versions.push(id);
        Ok(id)
    }

    /// Ingest a report record and stage it as the latest record for
    /// its report. Like [`NodeStore::append_version`], the entry
    /// reaches disk only at the caller's [`NodeStore::flush_index`].
    ///
    /// # Errors
    ///
    /// On a content-store failure.
    pub async fn put_report(
        &mut self,
        stored: &StoredReport,
    ) -> Result<ContentId, RecordStoreError> {
        let id = self.put(&encode_stored_report(stored)).await?;
        self.index.reports.insert(stored.report.report_id(), id);
        Ok(id)
    }

    /// Every record the index names, verified and decoded: version
    /// records in admission order, then one report record per
    /// report.
    ///
    /// # Errors
    ///
    /// On missing content or a record that does not decode.
    pub async fn load(&self) -> Result<(Vec<VersionRecord>, Vec<StoredReport>), RecordStoreError> {
        let mut versions = Vec::with_capacity(self.index.versions.len());
        for id in &self.index.versions {
            versions.push(VersionRecord::decode(&self.get(id).await?)?);
        }
        let mut reports = Vec::with_capacity(self.index.reports.len());
        for id in self.index.reports.values() {
            reports.push(decode_stored_report(&self.get(id).await?)?);
        }
        Ok((versions, reports))
    }
}

/// The production blob backend: iroh's blob store over an
/// `FsStore`. The wrapper holds the api handle, not only a `Blobs`
/// view, because a store releases its database only when it is
/// shut down — dropping the handles leaves the database locked,
/// and a node directory can be reopened (by this process, as in
/// the restart test, or by a later process) only after shutdown.
/// A process exit is a shutdown; [`StoredReplica::shutdown`] is
/// the in-process form.
#[derive(Debug, Clone)]
pub struct FsBlobs {
    api: iroh_blobs::api::Store,
    blobs: Blobs,
}

impl FsBlobs {
    /// Shut the underlying store down.
    async fn shutdown(&self) -> Result<(), RecordStoreError> {
        self.api
            .shutdown()
            .await
            .map_err(|error| RecordStoreError::Backend(error.to_string()))
    }
}

impl BlobBackend for FsBlobs {
    async fn has(&self, hash: Hash) -> Result<bool, anyhow::Error> {
        BlobBackend::has(&self.blobs, hash).await
    }

    async fn read(&self, hash: Hash) -> Result<Option<Vec<u8>>, anyhow::Error> {
        BlobBackend::read(&self.blobs, hash).await
    }

    async fn commit(&self, bytes: Vec<u8>) -> Result<Hash, anyhow::Error> {
        BlobBackend::commit(&self.blobs, bytes).await
    }
}

/// A [`Replica`] whose versions and reports are kept in a
/// [`NodeStore`]: it restarts with what it had.
#[derive(Debug)]
pub struct StoredReplica<B> {
    replica: Replica,
    node: NodeStore<B>,
}

/// Restore a single stored replica if an admission future is dropped
/// before its write-through point commits. In particular, dropping an
/// async future while a blob write is pending must not leave an
/// admission visible in memory without its version record.
struct AdmissionRollback<'a, B: BlobBackend + Sync> {
    stored: &'a mut StoredReplica<B>,
    saved_replica: Option<Replica>,
    saved_index: Option<Index>,
}

impl<'a, B: BlobBackend + Sync> AdmissionRollback<'a, B> {
    fn new(stored: &'a mut StoredReplica<B>) -> Self {
        Self {
            saved_replica: Some(stored.replica.clone()),
            saved_index: Some(stored.node.checkpoint()),
            stored,
        }
    }

    fn commit(mut self) {
        self.saved_replica = None;
        self.saved_index = None;
    }
}

impl<B: BlobBackend + Sync> std::ops::Deref for AdmissionRollback<'_, B> {
    type Target = StoredReplica<B>;

    fn deref(&self) -> &Self::Target {
        self.stored
    }
}

impl<B: BlobBackend + Sync> std::ops::DerefMut for AdmissionRollback<'_, B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.stored
    }
}

impl<B: BlobBackend + Sync> Drop for AdmissionRollback<'_, B> {
    fn drop(&mut self) {
        if let Some(replica) = self.saved_replica.take() {
            self.stored.replica = replica;
        }
        if let Some(index) = self.saved_index.take() {
            self.stored.node.restore(index);
        }
    }
}

/// Restore both sides if an exchange future is dropped before both
/// write-through points finish.
struct ExchangeRollback<'a, B: BlobBackend + Sync> {
    left: &'a mut StoredReplica<B>,
    right: &'a mut StoredReplica<B>,
    left_replica: Option<Replica>,
    right_replica: Option<Replica>,
    left_index: Option<Index>,
    right_index: Option<Index>,
}

impl<'a, B: BlobBackend + Sync> ExchangeRollback<'a, B> {
    fn new(left: &'a mut StoredReplica<B>, right: &'a mut StoredReplica<B>) -> Self {
        Self {
            left_replica: Some(left.replica.clone()),
            right_replica: Some(right.replica.clone()),
            left_index: Some(left.node.checkpoint()),
            right_index: Some(right.node.checkpoint()),
            left,
            right,
        }
    }

    fn commit(mut self) {
        self.left_replica = None;
        self.right_replica = None;
        self.left_index = None;
        self.right_index = None;
    }
}

impl<B: BlobBackend + Sync> Drop for ExchangeRollback<'_, B> {
    fn drop(&mut self) {
        if let Some(replica) = self.left_replica.take() {
            self.left.replica = replica;
        }
        if let Some(replica) = self.right_replica.take() {
            self.right.replica = replica;
        }
        if let Some(index) = self.left_index.take() {
            self.left.node.restore(index);
        }
        if let Some(index) = self.right_index.take() {
            self.right.node.restore(index);
        }
    }
}

/// Restore this node's side if a remote exchange is dropped or
/// fails before its write-through point commits. The responder's
/// side is the responder's own affair: it persists (or loses) what
/// it admitted under its own discipline, and a later exchange
/// re-offers whatever this node's rollback forgot.
struct RemoteRollback<'a, B: BlobBackend + Sync> {
    stored: &'a mut StoredReplica<B>,
    saved_replica: Option<Replica>,
    saved_index: Option<Index>,
}

impl<'a, B: BlobBackend + Sync> RemoteRollback<'a, B> {
    fn new(stored: &'a mut StoredReplica<B>) -> Self {
        Self {
            saved_replica: Some(stored.replica.clone()),
            saved_index: Some(stored.node.checkpoint()),
            stored,
        }
    }

    fn commit(mut self) {
        self.saved_replica = None;
        self.saved_index = None;
    }
}

impl<B: BlobBackend + Sync> std::ops::Deref for RemoteRollback<'_, B> {
    type Target = StoredReplica<B>;

    fn deref(&self) -> &Self::Target {
        self.stored
    }
}

impl<B: BlobBackend + Sync> std::ops::DerefMut for RemoteRollback<'_, B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.stored
    }
}

impl<B: BlobBackend + Sync> Drop for RemoteRollback<'_, B> {
    fn drop(&mut self) {
        if let Some(replica) = self.saved_replica.take() {
            self.stored.replica = replica;
        }
        if let Some(index) = self.saved_index.take() {
            self.stored.node.restore(index);
        }
    }
}

/// A stored replica over the production on-disk backend.
pub type DiskReplica = StoredReplica<FsBlobs>;

impl<B: BlobBackend + Sync> StoredReplica<B> {
    /// Open a node: replay the version records `node` names into
    /// `replica` (fresh, with the node's signer table and policy),
    /// then check the persisted report records against the reports
    /// the replay rebuilt. A report record whose status predates
    /// the version log is re-written from the replay, not refused.
    ///
    /// # Errors
    ///
    /// On a store failure, a version refused on replay
    /// ([`RecordStoreError::ReplayRefused`]), or a persisted report
    /// id the replay never rebuilt
    /// ([`RecordStoreError::ReportDiverged`]).
    pub async fn open(node: NodeStore<B>, mut replica: Replica) -> Result<Self, RecordStoreError> {
        let (versions, reports) = node.load().await?;
        for record in &versions {
            replica
                .receive(
                    record.path,
                    &record.envelope,
                    &record.source,
                    record.admitted_at,
                )
                .map_err(|error| RecordStoreError::ReplayRefused {
                    event_id: record.envelope.event_id,
                    error,
                })?;
        }
        let mut stored = Self { replica, node };
        let rebuilt: BTreeMap<ReportId, StoredReport> = stored
            .replica
            .store()
            .reports()
            .map(|(id, report)| (*id, report.clone()))
            .collect();
        let mut recorded: BTreeSet<ReportId> = BTreeSet::new();
        let mut healed = false;
        for report in &reports {
            let report_id = report.report.report_id();
            recorded.insert(report_id);
            match rebuilt.get(&report_id) {
                Some(rebuilt_report) if rebuilt_report == report => {}
                Some(rebuilt_report) => {
                    // The same id names the same canonical bytes, so
                    // only the status can differ: this record predates
                    // an admission the version log already holds (the
                    // process died between the version write and the
                    // report write). The log is authoritative; rewrite
                    // the record from the replay.
                    stored.node.put_report(rebuilt_report).await?;
                    healed = true;
                }
                None => return Err(RecordStoreError::ReportDiverged(report_id)),
            }
        }
        // A report the replay rebuilt but no record names: the
        // process died between the version write and the report
        // write. Heal it by writing the record now.
        for (report_id, report) in &rebuilt {
            if !recorded.contains(report_id) {
                stored.node.put_report(report).await?;
                healed = true;
            }
        }
        if healed {
            stored.node.flush_index()?;
        }
        Ok(stored)
    }

    /// The wrapped replica.
    pub const fn replica(&self) -> &Replica {
        &self.replica
    }

    /// The wrapped replica's store.
    pub const fn store(&self) -> &ConflictStore {
        self.replica.store()
    }

    /// The replica's name.
    pub fn name(&self) -> &str {
        self.replica.name()
    }

    /// Admit a version a local producer signed, and persist it.
    /// When persisting fails or this future is cancelled, the
    /// admission is rolled back — the
    /// version leaves the replica and the index — so the envelope
    /// can be retried instead of being rejected as a replay, no
    /// child can be admitted on a version that was never logged,
    /// and peers never see an unpersisted version in the inventory.
    /// Rolling back clones the replica, so each admission costs a
    /// copy of the store; that is the price of the rollback, chosen
    /// over leaving the node unusable until it is reopened.
    ///
    /// # Errors
    ///
    /// [`StoredError::Ingest`] when the store refuses the version;
    /// [`StoredError::Store`] when an admitted version (or a report
    /// it changed) could not be written.
    pub async fn produce(
        &mut self,
        envelope: &Envelope,
        now: LogicalTime,
    ) -> Result<IngestOutcome, StoredError> {
        let mut rollback = AdmissionRollback::new(self);
        let before = rollback.report_snapshot();
        let source = rollback.replica.name().to_owned();
        let outcome = rollback
            .replica
            .produce(envelope, now)
            .map_err(StoredError::Ingest)?;
        rollback
            .persist_admission(envelope, IngressPath::Local, &source, now, &before)
            .await?;
        rollback.commit();
        Ok(outcome)
    }

    /// Admit one envelope from a peer, and persist it. As
    /// [`StoredReplica::produce`], a persist failure or cancellation
    /// rolls the admission back.
    ///
    /// # Errors
    ///
    /// As [`StoredReplica::produce`].
    pub async fn receive(
        &mut self,
        path: IngressPath,
        envelope: &Envelope,
        source: &str,
        now: LogicalTime,
    ) -> Result<IngestOutcome, StoredError> {
        let mut rollback = AdmissionRollback::new(self);
        let before = rollback.report_snapshot();
        let outcome = rollback
            .replica
            .receive(path, envelope, source, now)
            .map_err(StoredError::Ingest)?;
        rollback
            .persist_admission(envelope, path, source, now, &before)
            .await?;
        rollback.commit();
        Ok(outcome)
    }

    /// Reconcile with another stored replica over one link, then
    /// persist what each side admitted during the exchange (from
    /// the exchange's own outcomes) and every report that changed.
    /// When persisting fails or this future is cancelled, the
    /// exchange is rolled back on both sides — replicas and indexes
    /// return to what they had, and
    /// the exchange can run again — so a half-persisted exchange
    /// never leaves one side holding a version the other lost.
    ///
    /// # Errors
    ///
    /// On a persistence failure after the exchange.
    pub async fn reconcile_with(
        &mut self,
        other: &mut Self,
        config: &SyncScopeConfig,
        now: LogicalTime,
    ) -> Result<Exchange, RecordStoreError> {
        let rollback = ExchangeRollback::new(self, other);
        let self_before = rollback.left.report_snapshot();
        let other_before = rollback.right.report_snapshot();
        let self_name = rollback.left.replica.name().to_owned();
        let other_name = rollback.right.replica.name().to_owned();
        let exchange = reconcile(
            &mut rollback.left.replica,
            &mut rollback.right.replica,
            config,
            now,
        );
        let persisted = async {
            for (event_id, outcome) in &exchange.forward.outcomes {
                if outcome.is_ok() {
                    rollback
                        .right
                        .persist_exchange_admission(event_id, &self_name, now)
                        .await?;
                }
            }
            for (event_id, outcome) in &exchange.backward.outcomes {
                if outcome.is_ok() {
                    rollback
                        .left
                        .persist_exchange_admission(event_id, &other_name, now)
                        .await?;
                }
            }
            rollback.left.persist_reports_since(&self_before).await?;
            rollback.right.persist_reports_since(&other_before).await?;
            rollback.left.node.flush_index()?;
            rollback.right.node.flush_index()?;
            Ok::<(), RecordStoreError>(())
        }
        .await;
        persisted?;
        rollback.commit();
        Ok(exchange)
    }

    /// Run one exchange with the replica at `target` over the
    /// transport (B2(c), `super::exchange_net`), then persist what
    /// this node admitted during it — the backward transfer's
    /// versions, from the exchange's own outcomes, under the
    /// responder's name as source — and every report that changed.
    /// The responder persists its own admissions on its side (see
    /// [`ServingNode`]). When the exchange or persisting fails, or
    /// this future is cancelled, this node's side is rolled back —
    /// replica and index return to what they had, and the exchange
    /// can run again — so a half-persisted exchange never leaves
    /// this node holding a version its log lost.
    ///
    /// # Errors
    ///
    /// [`RecordStoreError::Exchange`] when the session fails;
    /// a store error when an admitted version (or a report it
    /// changed) could not be written.
    pub async fn reconcile_remote_with(
        &mut self,
        endpoint: &Endpoint,
        target: impl Into<EndpointAddr>,
        config: &SyncScopeConfig,
        now: LogicalTime,
    ) -> Result<Exchange, RecordStoreError> {
        let mut rollback = RemoteRollback::new(self);
        let before = rollback.report_snapshot();
        let (exchange, remote_name) =
            reconcile_remote_named(endpoint, target, &mut rollback.replica, config, now)
                .await
                .map_err(|error| RecordStoreError::Exchange(error.to_string()))?;
        let persisted = async {
            for (event_id, outcome) in &exchange.backward.outcomes {
                if outcome.is_ok() {
                    rollback
                        .persist_exchange_admission(event_id, &remote_name, now)
                        .await?;
                }
            }
            rollback.persist_reports_since(&before).await?;
            rollback.node.flush_index()?;
            Ok::<(), RecordStoreError>(())
        }
        .await;
        persisted?;
        rollback.commit();
        Ok(exchange)
    }

    async fn persist_exchange_admission(
        &mut self,
        event_id: &EventId,
        source: &str,
        now: LogicalTime,
    ) -> Result<(), RecordStoreError> {
        let Some(envelope) = self.replica.envelope(event_id).cloned() else {
            return Ok(());
        };
        self.node
            .append_version(&VersionRecord {
                envelope,
                path: IngressPath::Internal,
                source: source.to_owned(),
                admitted_at: now,
            })
            .await?;
        Ok(())
    }

    async fn persist_admission(
        &mut self,
        envelope: &Envelope,
        path: IngressPath,
        source: &str,
        now: LogicalTime,
        reports_before: &BTreeMap<ReportId, StoredReport>,
    ) -> Result<(), StoredError> {
        self.node
            .append_version(&VersionRecord {
                envelope: envelope.clone(),
                path,
                source: source.to_owned(),
                admitted_at: now,
            })
            .await
            .map_err(StoredError::Store)?;
        self.persist_reports_since(reports_before)
            .await
            .map_err(StoredError::Store)?;
        // One index update publishes the version and the reports it
        // changed together: a crash leaves the old index (the
        // admission is lost and re-sent) or the new one, never a
        // version without its reports.
        self.node.flush_index().map_err(StoredError::Store)?;
        Ok(())
    }

    fn report_snapshot(&self) -> BTreeMap<ReportId, StoredReport> {
        self.replica
            .store()
            .reports()
            .map(|(id, report)| (*id, report.clone()))
            .collect()
    }

    async fn persist_reports_since(
        &mut self,
        before: &BTreeMap<ReportId, StoredReport>,
    ) -> Result<(), RecordStoreError> {
        let current: Vec<(ReportId, StoredReport)> = self
            .replica
            .store()
            .reports()
            .map(|(id, report)| (*id, report.clone()))
            .collect();
        for (report_id, report) in &current {
            if before.get(report_id) != Some(report) {
                self.node.put_report(report).await?;
            }
        }
        Ok(())
    }
}

impl DiskReplica {
    /// Open the node whose directory is `dir`: an iroh `FsStore`
    /// under `dir/blobs` behind a [`ContentStore`], the index at
    /// `dir/index`, and `replica` replayed to what the node had.
    ///
    /// # Errors
    ///
    /// As [`StoredReplica::open`], and on failing to create or load
    /// the on-disk backend.
    pub async fn open_dir(dir: &Path, replica: Replica) -> Result<Self, RecordStoreError> {
        fs::create_dir_all(dir)?;
        let blobs_dir = dir.join(BLOBS_DIR);
        fs::create_dir_all(&blobs_dir)?;
        let fs_store = FsStore::load(&blobs_dir)
            .await
            .map_err(|error| RecordStoreError::Backend(error.to_string()))?;
        let api: iroh_blobs::api::Store = fs_store.into();
        let backend = FsBlobs {
            blobs: api.blobs().clone(),
            api,
        };
        // Keep a handle to the backend: the node below owns a clone
        // of it, and on an init failure that clone is dropped with
        // the node — dropping without shutting down leaves the
        // database locked, so the retained handle shuts it down
        // before the error returns and the directory can be retried.
        let content = ContentStore::new(backend.clone(), ContentStoreConfig::default());
        let node = match NodeStore::new(content, dir.join(INDEX_FILE)) {
            Ok(node) => node,
            Err(error) => {
                let _ = backend.shutdown().await;
                return Err(error);
            }
        };
        match Self::open(node, replica).await {
            Ok(stored) => Ok(stored),
            Err(error) => {
                let _ = backend.shutdown().await;
                Err(error)
            }
        }
    }

    /// Shut the node's blob store down, releasing its database so
    /// the directory can be reopened. Dropping the node without
    /// shutting down leaves the database locked until the process
    /// exits; a process exit is itself a shutdown, so this is the
    /// in-process form of a restart.
    ///
    /// # Errors
    ///
    /// When the underlying store refuses to shut down.
    pub async fn shutdown(&self) -> Result<(), RecordStoreError> {
        self.node.content().backend().shutdown().await
    }
}

/// A stored node serving federation exchanges: the responder half
/// of the composition of the on-disk store (B2(b)) with the
/// transport (B2(c)). The replica sits behind the exchange
/// protocol's lock; each session's admissions come back over the
/// channel [`ServingNode::protocol`] hands out, and the node's
/// owner persists them with [`ServingNode::persist_session`] —
/// versions first, then every report that changed since the last
/// persisted snapshot, in one index update per session, the same
/// write-through shape as [`StoredReplica::persist_admission`].
///
/// A session whose admissions are never persisted is the serving
/// form of a crash between an admission and its write: the
/// versions live only in memory, and anti-entropy re-sends them
/// after a restart. Persisting late is safe for the same reason —
/// a session's admissions are persisted with the session's own
/// logical time, so replay reproduces the admission exactly.
#[derive(Debug, Clone)]
pub struct ServingNode<B> {
    replica: Arc<TokioMutex<Replica>>,
    node: Arc<TokioMutex<NodeStore<B>>>,
    reports: Arc<TokioMutex<BTreeMap<ReportId, StoredReport>>>,
}

impl<B: BlobBackend + Sync> ServingNode<B> {
    /// Open a serving node: replay `node`'s records into `replica`
    /// (as [`StoredReplica::open`]), then hold the rebuilt replica
    /// and the store behind the locks the protocol and the owner
    /// share.
    ///
    /// # Errors
    ///
    /// As [`StoredReplica::open`].
    pub async fn open(node: NodeStore<B>, replica: Replica) -> Result<Self, RecordStoreError> {
        let stored = StoredReplica::open(node, replica).await?;
        Ok(Self::from_stored(stored))
    }

    fn from_stored(stored: StoredReplica<B>) -> Self {
        let StoredReplica { replica, node } = stored;
        let reports = replica
            .store()
            .reports()
            .map(|(id, report)| (*id, report.clone()))
            .collect();
        Self {
            replica: Arc::new(TokioMutex::new(replica)),
            node: Arc::new(TokioMutex::new(node)),
            reports: Arc::new(TokioMutex::new(reports)),
        }
    }

    /// The exchange protocol handler for this node, and the
    /// receiver its session admissions arrive on. Register the
    /// handler with the router; drain the receiver with
    /// [`ServingNode::persist_session`] for as long as the node
    /// serves.
    pub fn protocol(&self) -> (ExchangeProtocol, mpsc::UnboundedReceiver<SessionAdmissions>) {
        let (send, receive) = mpsc::unbounded_channel();
        let observer = Arc::new(move |admissions: SessionAdmissions| {
            // The receiver lives as long as the node serves; a send
            // that fails means the owner stopped draining, and the
            // admissions are lost the way a crash loses them (see
            // the type's docs) — never a reason to fail the session.
            let _ = send.send(admissions);
        });
        (
            ExchangeProtocol::with_observer(self.replica.clone(), observer),
            receive,
        )
    }

    /// Persist one session's admissions: the version records for
    /// what the session admitted (in admission order, under the
    /// initiator's name as source, at the session's logical time),
    /// then a record for every report that changed since the last
    /// persisted snapshot, published in one index update.
    ///
    /// # Errors
    ///
    /// On a content-store or index failure. What was staged before
    /// the failure reaches disk only at a later session's index
    /// update; a crash before that loses it, as the type's docs
    /// describe.
    pub async fn persist_session(
        &self,
        admissions: &SessionAdmissions,
    ) -> Result<(), RecordStoreError> {
        let mut node = self.node.lock().await;
        {
            let replica = self.replica.lock().await;
            for event_id in &admissions.admitted {
                if let Some(envelope) = replica.envelope(event_id).cloned() {
                    node.append_version(&VersionRecord {
                        envelope,
                        path: IngressPath::Internal,
                        source: admissions.initiator.clone(),
                        admitted_at: admissions.now,
                    })
                    .await?;
                }
            }
        }
        let current: BTreeMap<ReportId, StoredReport> = {
            let replica = self.replica.lock().await;
            replica
                .store()
                .reports()
                .map(|(id, report)| (*id, report.clone()))
                .collect()
        };
        let mut snapshot = self.reports.lock().await;
        for (report_id, report) in &current {
            if snapshot.get(report_id) != Some(report) {
                node.put_report(report).await?;
            }
        }
        node.flush_index()?;
        *snapshot = current;
        Ok(())
    }
}

impl ServingNode<FsBlobs> {
    /// Open the serving node whose directory is `dir` (see
    /// [`DiskReplica::open_dir`]).
    ///
    /// # Errors
    ///
    /// As [`DiskReplica::open_dir`].
    pub async fn open_dir(dir: &Path, replica: Replica) -> Result<Self, RecordStoreError> {
        let stored = DiskReplica::open_dir(dir, replica).await?;
        Ok(Self::from_stored(stored))
    }

    /// Shut the node's blob store down, releasing its database so
    /// the directory can be reopened (see
    /// [`StoredReplica::shutdown`]).
    ///
    /// # Errors
    ///
    /// When the underlying store refuses to shut down.
    pub async fn shutdown(&self) -> Result<(), RecordStoreError> {
        self.node.lock().await.content().backend().shutdown().await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use ed25519_dalek::SigningKey;
    use iroh_blobs::Hash;
    use proptest::prelude::*;

    use super::super::slices::b07_conflict_report::{RetentionPolicy, VersionHeader, conflict_key};
    use super::super::slices::b32_ingest_gate::{
        FreshnessWindow, SignerAuthorization, sign_envelope,
    };
    use super::*;

    /// In-memory blob backend keyed the way Iroh keys blobs.
    #[derive(Debug, Default, Clone)]
    struct MapBackend {
        blobs: Arc<Mutex<BTreeMap<[u8; 32], Vec<u8>>>>,
    }

    impl BlobBackend for MapBackend {
        async fn has(&self, hash: Hash) -> Result<bool, anyhow::Error> {
            Ok(self.blobs.lock().unwrap().contains_key(hash.as_bytes()))
        }

        async fn read(&self, hash: Hash) -> Result<Option<Vec<u8>>, anyhow::Error> {
            Ok(self.blobs.lock().unwrap().get(hash.as_bytes()).cloned())
        }

        async fn commit(&self, bytes: Vec<u8>) -> Result<Hash, anyhow::Error> {
            let hash = Hash::new(&bytes);
            self.blobs.lock().unwrap().insert(*hash.as_bytes(), bytes);
            Ok(hash)
        }
    }

    const START: u64 = 10_000;

    /// A [`MapBackend`] that fails commits from `fail_after` on, so
    /// a test can fault a persist at a chosen point and then let it
    /// succeed again.
    #[derive(Debug, Clone)]
    struct FailingBackend {
        inner: MapBackend,
        commits: Arc<AtomicUsize>,
        fail_after: Arc<AtomicUsize>,
    }

    impl FailingBackend {
        fn new(fail_after: usize) -> Self {
            Self {
                inner: MapBackend::default(),
                commits: Arc::new(AtomicUsize::new(0)),
                fail_after: Arc::new(AtomicUsize::new(fail_after)),
            }
        }

        fn stop_failing(&self) {
            self.fail_after.store(usize::MAX, Ordering::SeqCst);
        }

        /// Fail commits from `n` on (counting from the backend's
        /// first commit), so a test can arm a failure mid-scenario.
        fn fail_from(&self, n: usize) {
            self.fail_after.store(n, Ordering::SeqCst);
        }
    }

    impl BlobBackend for FailingBackend {
        async fn has(&self, hash: Hash) -> Result<bool, anyhow::Error> {
            self.inner.has(hash).await
        }

        async fn read(&self, hash: Hash) -> Result<Option<Vec<u8>>, anyhow::Error> {
            self.inner.read(hash).await
        }

        async fn commit(&self, bytes: Vec<u8>) -> Result<Hash, anyhow::Error> {
            let n = self.commits.fetch_add(1, Ordering::SeqCst);
            if n >= self.fail_after.load(Ordering::SeqCst) {
                return Err(anyhow::anyhow!("simulated blob commit failure"));
            }
            self.inner.commit(bytes).await
        }
    }

    /// A backend whose first blob commit can be observed and then
    /// held pending, so tests can cancel an admission at its await.
    #[derive(Debug, Clone, Default)]
    struct BlockingBackend {
        inner: MapBackend,
        entered: Arc<tokio::sync::Notify>,
        block_next: Arc<AtomicBool>,
    }

    impl BlobBackend for BlockingBackend {
        async fn has(&self, hash: Hash) -> Result<bool, anyhow::Error> {
            self.inner.has(hash).await
        }

        async fn read(&self, hash: Hash) -> Result<Option<Vec<u8>>, anyhow::Error> {
            self.inner.read(hash).await
        }

        async fn commit(&self, _bytes: Vec<u8>) -> Result<Hash, anyhow::Error> {
            if self.block_next.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                std::future::pending().await
            } else {
                self.inner.commit(_bytes).await
            }
        }
    }

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn plain_version(event: EventId, body: &[u8]) -> Envelope {
        let header = VersionHeader {
            object_id: [0x11; 32],
            resolves: None,
            parents: vec![],
            body: body.to_vec(),
        };
        sign_envelope(&key(), "alice", "notes", &header.encode(), event, START)
    }

    /// A version following `parents`, naming their conflict key when
    /// it merges two or more of them (the store requires a merge to
    /// name what it resolves).
    fn version_with_parents(
        event: EventId,
        body: &[u8],
        parents: &[EventId],
        now: u64,
    ) -> Envelope {
        let header = VersionHeader {
            object_id: [0x11; 32],
            resolves: (parents.len() >= 2).then(|| conflict_key(parents)),
            parents: parents.to_vec(),
            body: body.to_vec(),
        };
        sign_envelope(&key(), "alice", "notes", &header.encode(), event, now)
    }

    fn conflict_store() -> ConflictStore {
        let signers = vec![SignerAuthorization {
            identity: "alice".to_owned(),
            verifying_key: key().verifying_key().to_bytes(),
            scopes: ["notes".to_owned()].into_iter().collect(),
        }];
        ConflictStore::new(
            signers,
            FreshnessWindow {
                max_age: 1_000_000,
                max_future_skew: 30,
            },
            RetentionPolicy { retain_for: 3_600 },
        )
    }

    fn sample_report() -> StoredReport {
        let version = |event_id: EventId, signer: &str| VersionRef {
            event_id,
            object_id: [0x11; 32],
            parents: vec![[0x01; 32]],
            resolves: None,
            signer: signer.to_owned(),
            verifying_key: [9; 32],
            path: IngressPath::Internal,
            source: "node-b".to_owned(),
            scope: "notes".to_owned(),
            issued_at: START,
            body_digest: [0x02; 32],
            body_len: 42,
        };
        StoredReport {
            report: ConflictReport {
                object_id: [0x11; 32],
                versions: vec![version([0x03; 32], "alice"), version([0x04; 32], "bob")],
                base: vec![[0x01; 32]],
                scopes: vec!["notes".to_owned()],
                policy_digest: [0x05; 32],
                detected_at: START + 1,
                expires_at: START + 3_601,
                supersedes: Some([0x06; 32]),
            },
            status: ReportStatus::Resolved {
                by: [0x07; 32],
                at: START + 2,
            },
        }
    }

    #[test]
    fn envelope_codec_round_trips_signed_and_unsigned() {
        let signed = plain_version([0x42; 32], b"not a record");
        assert_eq!(decode_envelope(&encode_envelope(&signed)).unwrap(), signed);
        let mut unsigned = signed.clone();
        unsigned.signature = None;
        assert_eq!(
            decode_envelope(&encode_envelope(&unsigned)).unwrap(),
            unsigned
        );
    }

    #[test]
    fn version_record_codec_round_trips() {
        let record = VersionRecord {
            envelope: plain_version([0x42; 32], b"not a record"),
            path: IngressPath::Internal,
            source: "node-b".to_owned(),
            admitted_at: START + 1,
        };
        assert_eq!(VersionRecord::decode(&record.encode()).unwrap(), record);
    }

    #[test]
    fn report_record_codec_round_trips_and_preserves_the_report_id() {
        let stored = sample_report();
        let decoded = decode_stored_report(&encode_stored_report(&stored)).unwrap();
        assert_eq!(decoded, stored);
        assert_eq!(decoded.report.report_id(), stored.report.report_id());
        assert_eq!(
            decoded.report.canonical_bytes(),
            stored.report.canonical_bytes()
        );
    }

    #[tokio::test]
    async fn node_store_round_trips_records_in_admission_order() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index");
        let backend = MapBackend::default();
        let first = VersionRecord {
            envelope: plain_version([0x42; 32], b"one"),
            path: IngressPath::Local,
            source: "node-a".to_owned(),
            admitted_at: START,
        };
        let second = VersionRecord {
            envelope: plain_version([0x43; 32], b"two"),
            path: IngressPath::Internal,
            source: "node-b".to_owned(),
            admitted_at: START + 1,
        };
        let report = sample_report();
        {
            let content = ContentStore::new(backend.clone(), ContentStoreConfig::default());
            let mut node = NodeStore::new(content, index_path.clone()).unwrap();
            node.append_version(&first).await.unwrap();
            node.append_version(&second).await.unwrap();
            node.put_report(&report).await.unwrap();
            // Appends and report writes stage their index entries;
            // the index is committed by this flush.
            node.flush_index().unwrap();
        }
        let content = ContentStore::new(backend, ContentStoreConfig::default());
        let node = NodeStore::new(content, index_path).unwrap();
        let (versions, reports) = node.load().await.unwrap();
        assert_eq!(versions, vec![first, second]);
        assert_eq!(reports, vec![report]);
    }

    #[tokio::test]
    async fn a_stored_replica_restarts_with_what_it_had() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index");
        let backend = MapBackend::default();
        let envelope = plain_version([0x42; 32], b"not a record");
        let node = |backend: MapBackend, index_path: PathBuf| {
            NodeStore::new(
                ContentStore::new(backend, ContentStoreConfig::default()),
                index_path,
            )
            .unwrap()
        };
        {
            let replica = Replica::new("node-a", "home", conflict_store());
            let mut stored =
                StoredReplica::open(node(backend.clone(), index_path.clone()), replica)
                    .await
                    .unwrap();
            let outcome = stored.produce(&envelope, START).await.unwrap();
            assert_eq!(outcome, IngestOutcome::Applied);
        }
        let replica = Replica::new("node-a", "home", conflict_store());
        let stored = StoredReplica::open(node(backend, index_path), replica)
            .await
            .unwrap();
        assert!(stored.replica().inventory().contains(&[0x42; 32]));
        assert_eq!(stored.replica().envelope(&[0x42; 32]), Some(&envelope));
        assert_eq!(stored.store().heads(&[0x11; 32]), vec![[0x42; 32]]);
    }

    #[tokio::test]
    async fn a_stale_report_record_heals_on_open() {
        // The crash window from the review: the resolving version
        // reached the log but its report record still says Open.
        // Opening replays the log, rebuilds the report as Resolved,
        // and rewrites the record instead of refusing to open.
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index");
        let backend = MapBackend::default();
        let base = [0x42; 32];
        let alice = [0x43; 32];
        let bob = [0x44; 32];
        let merge = [0x45; 32];
        let mut model = Replica::new("node-a", "home", conflict_store());
        model
            .produce(&version_with_parents(base, b"base", &[], START), START)
            .unwrap();
        model
            .produce(
                &version_with_parents(alice, b"alice", &[base], START + 1),
                START + 1,
            )
            .unwrap();
        model
            .produce(
                &version_with_parents(bob, b"bob", &[base], START + 2),
                START + 2,
            )
            .unwrap();
        let report_id = model.store().open_report(&[0x11; 32]).unwrap();
        let open_record = model.store().report(&report_id).unwrap().clone();
        assert_eq!(open_record.status, ReportStatus::Open);
        let outcome = model
            .produce(
                &version_with_parents(merge, b"merge", &[alice, bob], START + 3),
                START + 3,
            )
            .unwrap();
        assert_eq!(outcome, IngestOutcome::Resolved(report_id));
        let resolved = model.store().report(&report_id).unwrap().clone();
        // The torn state, built by hand: the log holds the merge,
        // the index still names the Open report record.
        let node = |backend: MapBackend| {
            NodeStore::new(
                ContentStore::new(backend, ContentStoreConfig::default()),
                index_path.clone(),
            )
            .unwrap()
        };
        {
            let mut node = node(backend.clone());
            let records = [
                (base, b"base".as_slice(), vec![], START),
                (alice, b"alice".as_slice(), vec![base], START + 1),
                (bob, b"bob".as_slice(), vec![base], START + 2),
                (merge, b"merge".as_slice(), vec![alice, bob], START + 3),
            ];
            for (event, body, parents, at) in records {
                node.append_version(&VersionRecord {
                    envelope: version_with_parents(event, body, &parents, at),
                    path: IngressPath::Local,
                    source: "node-a".to_owned(),
                    admitted_at: at,
                })
                .await
                .unwrap();
            }
            node.put_report(&open_record).await.unwrap();
            node.flush_index().unwrap();
        }
        let stored = StoredReplica::open(
            node(backend.clone()),
            Replica::new("node-a", "home", conflict_store()),
        )
        .await
        .unwrap();
        assert_eq!(stored.store().report(&report_id).unwrap(), &resolved);
        // The heal persisted: a second open finds the Resolved
        // record, not the stale one.
        drop(stored);
        let stored = StoredReplica::open(
            node(backend),
            Replica::new("node-a", "home", conflict_store()),
        )
        .await
        .unwrap();
        assert_eq!(stored.store().report(&report_id).unwrap(), &resolved);
    }

    #[tokio::test]
    async fn a_failed_version_persist_rolls_the_admission_back() {
        // The blob backend fails while appending the version: the
        // admission must leave no trace, so the same envelope
        // retries cleanly instead of being rejected as a replay.
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index");
        let backend = FailingBackend::new(0);
        let node = |backend: FailingBackend| {
            NodeStore::new(
                ContentStore::new(backend, ContentStoreConfig::default()),
                index_path.clone(),
            )
            .unwrap()
        };
        let mut stored = StoredReplica::open(
            node(backend.clone()),
            Replica::new("node-a", "home", conflict_store()),
        )
        .await
        .unwrap();
        let envelope = plain_version([0x42; 32], b"not a record");
        let error = stored.produce(&envelope, START).await.unwrap_err();
        assert!(matches!(error, StoredError::Store(_)));
        assert!(!stored.replica().inventory().contains(&[0x42; 32]));
        assert_eq!(stored.replica().envelope(&[0x42; 32]), None);
        assert!(stored.store().heads(&[0x11; 32]).is_empty());
        backend.stop_failing();
        let outcome = stored.produce(&envelope, START).await.unwrap();
        assert_eq!(outcome, IngestOutcome::Applied);
        assert!(stored.replica().inventory().contains(&[0x42; 32]));
    }

    #[tokio::test]
    async fn cancelling_a_pending_admission_rolls_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index");
        let backend = BlockingBackend::default();
        let node = NodeStore::new(
            ContentStore::new(backend.clone(), ContentStoreConfig::default()),
            index_path,
        )
        .unwrap();
        let mut stored =
            StoredReplica::open(node, Replica::new("node-a", "home", conflict_store()))
                .await
                .unwrap();
        let envelope = plain_version([0x42; 32], b"cancel me");
        let entered = backend.entered.clone();
        backend.block_next.store(true, Ordering::SeqCst);
        {
            let admission = stored.produce(&envelope, START);
            tokio::pin!(admission);
            tokio::select! {
                _ = entered.notified() => {},
                result = &mut admission => panic!("blocked admission unexpectedly returned: {result:?}"),
            }
        }
        assert!(!stored.replica().inventory().contains(&[0x42; 32]));
        assert_eq!(stored.replica().envelope(&[0x42; 32]), None);
        assert!(stored.store().heads(&[0x11; 32]).is_empty());
    }

    #[tokio::test]
    async fn cancelling_a_pending_receive_rolls_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let backend = BlockingBackend::default();
        let node = NodeStore::new(
            ContentStore::new(backend.clone(), ContentStoreConfig::default()),
            dir.path().join("index"),
        )
        .unwrap();
        let mut stored =
            StoredReplica::open(node, Replica::new("node-a", "home", conflict_store()))
                .await
                .unwrap();
        let envelope = plain_version([0x43; 32], b"cancel receive");
        let entered = backend.entered.clone();
        backend.block_next.store(true, Ordering::SeqCst);
        {
            let admission = stored.receive(IngressPath::Remote, &envelope, "node-b", START);
            tokio::pin!(admission);
            tokio::select! {
                _ = entered.notified() => {},
                result = &mut admission => panic!("blocked receive unexpectedly returned: {result:?}"),
            }
        }
        assert!(!stored.replica().inventory().contains(&[0x43; 32]));
        assert_eq!(stored.replica().envelope(&[0x43; 32]), None);
        assert!(stored.store().heads(&[0x11; 32]).is_empty());
    }

    #[tokio::test]
    async fn cancelling_a_pending_exchange_rolls_both_sides_back() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let backend_a = BlockingBackend::default();
        let backend_b = BlockingBackend::default();
        let mut a = StoredReplica::open(
            NodeStore::new(
                ContentStore::new(backend_a, ContentStoreConfig::default()),
                dir_a.path().join("index"),
            )
            .unwrap(),
            Replica::new("node-a", "home", conflict_store()),
        )
        .await
        .unwrap();
        let mut b = StoredReplica::open(
            NodeStore::new(
                ContentStore::new(backend_b.clone(), ContentStoreConfig::default()),
                dir_b.path().join("index"),
            )
            .unwrap(),
            Replica::new("node-b", "home", conflict_store()),
        )
        .await
        .unwrap();
        let x = [0x44; 32];
        let y = [0x45; 32];
        a.produce(&plain_version(x, b"x"), START).await.unwrap();
        b.produce(&plain_version(y, b"y"), START).await.unwrap();
        let entered = backend_b.entered.clone();
        backend_b.block_next.store(true, Ordering::SeqCst);
        let link = SyncScopeConfig::new("notes", "home");
        {
            let exchange = a.reconcile_with(&mut b, &link, START + 1);
            tokio::pin!(exchange);
            tokio::select! {
                _ = entered.notified() => {},
                result = &mut exchange => panic!("blocked exchange unexpectedly returned: {result:?}"),
            }
        }
        assert!(a.replica().inventory().contains(&x));
        assert!(!a.replica().inventory().contains(&y));
        assert!(!b.replica().inventory().contains(&x));
        assert!(b.replica().inventory().contains(&y));
    }

    #[tokio::test]
    async fn a_failed_report_persist_rolls_the_admission_back() {
        // The version blob lands but the report blob it created
        // does not: the admission — and the index entry the version
        // staged — must roll back too, so the retry re-admits the
        // version instead of finding a phantom in the index.
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index");
        // Commits 0 and 1 are the base and alice versions; commit 2
        // is bob's version, commit 3 the conflict report it creates.
        let backend = FailingBackend::new(3);
        let node = |backend: FailingBackend| {
            NodeStore::new(
                ContentStore::new(backend, ContentStoreConfig::default()),
                index_path.clone(),
            )
            .unwrap()
        };
        let mut stored = StoredReplica::open(
            node(backend.clone()),
            Replica::new("node-a", "home", conflict_store()),
        )
        .await
        .unwrap();
        let base = [0x42; 32];
        let alice = [0x43; 32];
        let bob = [0x44; 32];
        stored
            .produce(&version_with_parents(base, b"base", &[], START), START)
            .await
            .unwrap();
        stored
            .produce(
                &version_with_parents(alice, b"alice", &[base], START + 1),
                START + 1,
            )
            .await
            .unwrap();
        let envelope = version_with_parents(bob, b"bob", &[base], START + 2);
        let error = stored.produce(&envelope, START + 2).await.unwrap_err();
        assert!(matches!(error, StoredError::Store(_)));
        assert!(!stored.replica().inventory().contains(&bob));
        assert_eq!(stored.store().heads(&[0x11; 32]), vec![alice]);
        assert!(stored.store().open_report(&[0x11; 32]).is_none());
        backend.stop_failing();
        let outcome = stored.produce(&envelope, START + 2).await.unwrap();
        assert!(matches!(outcome, IngestOutcome::Conflict(_)));
        assert_eq!(stored.store().heads(&[0x11; 32]), vec![alice, bob],);
        assert!(stored.store().open_report(&[0x11; 32]).is_some());
    }

    #[tokio::test]
    async fn a_failed_reconcile_rolls_both_sides_back() {
        // Each side holds one version; the exchange admits both
        // everywhere in memory, then persisting on B fails. Both
        // replicas return to what they had, and the exchange retries
        // cleanly once the backend works again.
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let index_a = dir_a.path().join("index");
        let index_b = dir_b.path().join("index");
        let backend_a = FailingBackend::new(usize::MAX);
        let backend_b = FailingBackend::new(usize::MAX);
        let node_a = |backend: FailingBackend| {
            NodeStore::new(
                ContentStore::new(backend, ContentStoreConfig::default()),
                index_a.clone(),
            )
            .unwrap()
        };
        let node_b = |backend: FailingBackend| {
            NodeStore::new(
                ContentStore::new(backend, ContentStoreConfig::default()),
                index_b.clone(),
            )
            .unwrap()
        };
        let mut a = StoredReplica::open(
            node_a(backend_a.clone()),
            Replica::new("node-a", "home", conflict_store()),
        )
        .await
        .unwrap();
        let mut b = StoredReplica::open(
            node_b(backend_b.clone()),
            Replica::new("node-b", "home", conflict_store()),
        )
        .await
        .unwrap();
        let x = [0x42; 32];
        let y = [0x43; 32];
        a.produce(&version_with_parents(x, b"x", &[], START), START)
            .await
            .unwrap();
        b.produce(&version_with_parents(y, b"y", &[], START), START)
            .await
            .unwrap();
        // B has committed once (its own version); fail its next
        // commit, which is the exchange version it persists first.
        backend_b.fail_from(1);
        let link = SyncScopeConfig::new("notes", "home");
        let error = a
            .reconcile_with(&mut b, &link, START + 1)
            .await
            .unwrap_err();
        assert!(matches!(error, RecordStoreError::Content(_)));
        assert!(a.replica().inventory().contains(&x));
        assert!(!a.replica().inventory().contains(&y));
        assert!(!b.replica().inventory().contains(&x));
        assert!(b.replica().inventory().contains(&y));
        assert!(a.store().open_report(&[0x11; 32]).is_none());
        assert!(b.store().open_report(&[0x11; 32]).is_none());
        backend_b.stop_failing();
        a.reconcile_with(&mut b, &link, START + 2).await.unwrap();
        assert!(a.replica().inventory().contains(&y));
        assert!(b.replica().inventory().contains(&x));
        assert!(a.store().open_report(&[0x11; 32]).is_some());
        assert!(b.store().open_report(&[0x11; 32]).is_some());
    }

    #[tokio::test]
    async fn a_failed_open_releases_the_blob_store() {
        // Opening fails after the blob backend loaded (the index
        // does not decode): the directory must open again in this
        // process once the index is fixed, not stay locked until
        // the process exits.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index"), b"not an index").unwrap();
        let replica = Replica::new("node-a", "home", conflict_store());
        assert!(DiskReplica::open_dir(dir.path(), replica).await.is_err());
        std::fs::remove_file(dir.path().join("index")).unwrap();
        let replica = Replica::new("node-a", "home", conflict_store());
        let stored = DiskReplica::open_dir(dir.path(), replica).await.unwrap();
        stored.shutdown().await.unwrap();
    }

    fn path_strategy() -> impl Strategy<Value = IngressPath> {
        (0..5u8).prop_map(|tag| match tag {
            0 => IngressPath::Local,
            1 => IngressPath::Loopback,
            2 => IngressPath::Internal,
            3 => IngressPath::Hosted,
            _ => IngressPath::Remote,
        })
    }

    fn envelope_strategy() -> impl Strategy<Value = Envelope> {
        (
            any::<String>(),
            any::<[u8; 32]>(),
            prop::option::of(any::<[u8; 64]>()),
            any::<String>(),
            prop::collection::vec(any::<u8>(), 0..64),
            any::<[u8; 32]>(),
            any::<u64>(),
        )
            .prop_map(
                |(signer, verifying_key, signature, scope, payload, event_id, issued_at)| {
                    let payload_digest = payload_digest(&payload);
                    Envelope {
                        signer,
                        verifying_key,
                        signature,
                        scope,
                        payload,
                        payload_digest,
                        event_id,
                        issued_at,
                    }
                },
            )
    }

    fn version_record_strategy() -> impl Strategy<Value = VersionRecord> {
        (
            envelope_strategy(),
            path_strategy(),
            any::<String>(),
            any::<u64>(),
        )
            .prop_map(|(envelope, path, source, admitted_at)| VersionRecord {
                envelope,
                path,
                source,
                admitted_at,
            })
    }

    fn version_ref_strategy() -> impl Strategy<Value = VersionRef> {
        (
            any::<[u8; 32]>(),
            any::<[u8; 32]>(),
            prop::collection::vec(any::<[u8; 32]>(), 0..4),
            prop::option::of(any::<[u8; 32]>()),
            any::<String>(),
            any::<[u8; 32]>(),
            path_strategy(),
            any::<String>(),
            any::<String>(),
            any::<u64>(),
            any::<[u8; 32]>(),
            any::<u64>(),
        )
            .prop_map(
                |(
                    event_id,
                    object_id,
                    parents,
                    resolves,
                    signer,
                    verifying_key,
                    path,
                    source,
                    scope,
                    issued_at,
                    body_digest,
                    body_len,
                )| VersionRef {
                    event_id,
                    object_id,
                    parents,
                    resolves,
                    signer,
                    verifying_key,
                    path,
                    source,
                    scope,
                    issued_at,
                    body_digest,
                    body_len,
                },
            )
    }

    fn report_strategy() -> impl Strategy<Value = ConflictReport> {
        (
            any::<[u8; 32]>(),
            prop::collection::vec(version_ref_strategy(), 0..4),
            prop::collection::vec(any::<[u8; 32]>(), 0..4),
            prop::collection::vec(any::<String>(), 0..3),
            any::<[u8; 32]>(),
            any::<u64>(),
            any::<u64>(),
            prop::option::of(any::<[u8; 32]>()),
        )
            .prop_map(
                |(
                    object_id,
                    versions,
                    base,
                    scopes,
                    policy_digest,
                    detected_at,
                    expires_at,
                    supersedes,
                )| ConflictReport {
                    object_id,
                    versions,
                    base,
                    scopes,
                    policy_digest,
                    detected_at,
                    expires_at,
                    supersedes,
                },
            )
    }

    fn status_strategy() -> impl Strategy<Value = ReportStatus> {
        prop_oneof![
            Just(ReportStatus::Open),
            (any::<[u8; 32]>(), any::<u64>())
                .prop_map(|(by, at)| ReportStatus::Resolved { by, at }),
            any::<[u8; 32]>().prop_map(|by| ReportStatus::Superseded { by }),
        ]
    }

    fn stored_report_strategy() -> impl Strategy<Value = StoredReport> {
        (report_strategy(), status_strategy())
            .prop_map(|(report, status)| StoredReport { report, status })
    }

    /// One generated admission: how it links to what came before,
    /// and its body. Mode 0 starts a new root (a fresh head, and a
    /// conflict when one exists); mode 1 extends the last admitted
    /// version; mode 2 merges every current head (resolving the open
    /// report when there is one).
    #[derive(Debug, Clone)]
    struct Op {
        mode: u8,
        body: Vec<u8>,
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        (any::<u8>(), prop::collection::vec(any::<u8>(), 0..32))
            .prop_map(|(mode, body)| Op { mode, body })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Oracle: the codec is a bijection on the values it writes
        /// — decode(encode(x)) is x, for every generated envelope.
        #[test]
        fn envelope_codec_round_trips_generated(envelope in envelope_strategy()) {
            prop_assert_eq!(
                decode_envelope(&encode_envelope(&envelope)).unwrap(),
                envelope
            );
        }

        /// Oracle: as above, for version records.
        #[test]
        fn version_record_codec_round_trips_generated(record in version_record_strategy()) {
            prop_assert_eq!(VersionRecord::decode(&record.encode()).unwrap(), record);
        }

        /// Oracle: as above, for report records — and the decoded
        /// report keeps its id and canonical bytes.
        #[test]
        fn report_record_codec_round_trips_generated(stored in stored_report_strategy()) {
            let decoded = decode_stored_report(&encode_stored_report(&stored)).unwrap();
            prop_assert_eq!(decoded.report.report_id(), stored.report.report_id());
            prop_assert_eq!(
                decoded.report.canonical_bytes(),
                stored.report.canonical_bytes()
            );
            prop_assert_eq!(decoded, stored);
        }

        /// Oracle: the in-memory [`Replica`] is the model — a
        /// reopened stored replica holds the same inventory, heads,
        /// reports and envelopes after any generated admission
        /// sequence. (64 cases of up to 12 ops each; every admission
        /// runs a strict ed25519 verify, which is slow at opt-level
        /// 0, so the count stays low.)
        #[test]
        fn restart_matches_the_in_memory_model(
            base in any::<[u8; 32]>(),
            ops in prop::collection::vec(op_strategy(), 1..13),
        ) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let dir = tempfile::tempdir().unwrap();
                let index_path = dir.path().join("index");
                let backend = MapBackend::default();
                let node = |backend: MapBackend| {
                    NodeStore::new(
                        ContentStore::new(backend, ContentStoreConfig::default()),
                        index_path.clone(),
                    )
                    .unwrap()
                };
                let mut model = Replica::new("node-a", "home", conflict_store());
                let mut stored = StoredReplica::open(
                    node(backend.clone()),
                    Replica::new("node-a", "home", conflict_store()),
                )
                .await
                .unwrap();
                let mut last: Option<EventId> = None;
                for (i, op) in ops.iter().enumerate() {
                    let mut event_id = base;
                    event_id[0] ^= i as u8;
                    event_id[1] ^= (i >> 8) as u8;
                    let now = START + i as u64;
                    let heads = model.store().heads(&[0x11; 32]);
                    let parents: Vec<EventId> = match op.mode % 3 {
                        0 => vec![],
                        1 => last.iter().copied().collect(),
                        _ if heads.len() >= 2 => heads,
                        _ => last.iter().copied().collect(),
                    };
                    let envelope = version_with_parents(event_id, &op.body, &parents, now);
                    match (
                        model.produce(&envelope, now),
                        stored.produce(&envelope, now).await,
                    ) {
                        (Ok(model_outcome), Ok(stored_outcome)) => {
                            assert_eq!(stored_outcome, model_outcome, "op {i} outcome");
                            last = Some(event_id);
                        }
                        (Err(model_error), Err(StoredError::Ingest(stored_error))) => {
                            assert_eq!(stored_error, model_error, "op {i} refusal");
                        }
                        (model_result, stored_result) => {
                            panic!(
                                "op {i}: model and stored disagree: {model_result:?} vs {stored_result:?}"
                            );
                        }
                    }
                }
                drop(stored);
                let reopened = StoredReplica::open(
                    node(backend),
                    Replica::new("node-a", "home", conflict_store()),
                )
                .await
                .unwrap();
                assert_eq!(reopened.replica().inventory(), model.inventory());
                assert_eq!(
                    reopened.store().heads(&[0x11; 32]),
                    model.store().heads(&[0x11; 32])
                );
                let model_reports: BTreeMap<ReportId, StoredReport> = model
                    .store()
                    .reports()
                    .map(|(id, report)| (*id, report.clone()))
                    .collect();
                let stored_reports: BTreeMap<ReportId, StoredReport> = reopened
                    .store()
                    .reports()
                    .map(|(id, report)| (*id, report.clone()))
                    .collect();
                assert_eq!(stored_reports, model_reports);
                for (i, _) in ops.iter().enumerate() {
                    let mut event_id = base;
                    event_id[0] ^= i as u8;
                    event_id[1] ^= (i >> 8) as u8;
                    assert_eq!(
                        reopened.replica().envelope(&event_id),
                        model.envelope(&event_id),
                        "event {event_id:?}"
                    );
                }
            });
        }
    }
}
