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
//! persisted at the same points. A crash between an admission and
//! its write loses that admission — and nothing else: anti-entropy
//! is idempotent, so a peer re-sends what the restarted node lacks.
//!
//! # Opening is replay
//!
//! Opening a node replays its version records, in admission order,
//! through `Replica::receive` — the same gate, the same join check,
//! the same `now` each version was admitted at — and rebuilds the
//! store exactly: versions, heads, held versions, reports and their
//! statuses are all derived from the admissions, and a `Replica`'s
//! reports change only through admissions. Persisted report records
//! are then checked against the rebuilt reports (id, canonical
//! bytes, status). A record missing because the process died
//! between the version write and the report write is re-written;
//! any other divergence is [`RecordStoreError::ReportDiverged`],
//! never a silent heal.
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

use id_core::content_id::raw_content_id;
use id_core::id_shape::{ContentId, HashProfile};
use id_store::content_store::{BlobBackend, ContentStore, ContentStoreConfig, ContentStoreError};
use iroh_blobs::Hash;
use iroh_blobs::api::blobs::Blobs;
use iroh_blobs::store::fs::FsStore;

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
    /// Opening the on-disk blob backend failed.
    Backend(String),
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
            Self::Backend(error) => write!(f, "blob backend failed: {error}"),
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

impl std::error::Error for RecordStoreError {}

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
#[derive(Debug, Default)]
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
        let tmp_path = self.index_path.with_extension("tmp");
        let mut file = fs::File::create(&tmp_path)?;
        use std::io::Write as _;
        file.write_all(&self.index.encode())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, &self.index_path)?;
        Ok(())
    }

    /// Ingest a version record and name it in the index, in
    /// admission order.
    ///
    /// # Errors
    ///
    /// On a content-store or index-write failure.
    pub async fn append_version(
        &mut self,
        record: &VersionRecord,
    ) -> Result<ContentId, RecordStoreError> {
        let id = self.put(&record.encode()).await?;
        self.index.versions.push(id);
        self.flush_index()?;
        Ok(id)
    }

    /// Ingest a report record and name it in the index as the
    /// latest record for its report.
    ///
    /// # Errors
    ///
    /// On a content-store or index-write failure.
    pub async fn put_report(
        &mut self,
        stored: &StoredReport,
    ) -> Result<ContentId, RecordStoreError> {
        let id = self.put(&encode_stored_report(stored)).await?;
        self.index.reports.insert(stored.report.report_id(), id);
        self.flush_index()?;
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

/// A stored replica over the production on-disk backend.
pub type DiskReplica = StoredReplica<FsBlobs>;

impl<B: BlobBackend + Sync> StoredReplica<B> {
    /// Open a node: replay the version records `node` names into
    /// `replica` (fresh, with the node's signer table and policy),
    /// then check the persisted report records against the reports
    /// the replay rebuilt.
    ///
    /// # Errors
    ///
    /// On a store failure, a version refused on replay
    /// ([`RecordStoreError::ReplayRefused`]), or a report record
    /// that does not match the rebuilt report
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
        for report in &reports {
            let report_id = report.report.report_id();
            recorded.insert(report_id);
            match rebuilt.get(&report_id) {
                Some(rebuilt_report) if rebuilt_report == report => {}
                _ => return Err(RecordStoreError::ReportDiverged(report_id)),
            }
        }
        // A report the replay rebuilt but no record names: the
        // process died between the version write and the report
        // write. Heal it by writing the record now.
        for (report_id, report) in &rebuilt {
            if !recorded.contains(report_id) {
                stored.node.put_report(report).await?;
            }
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
        let before = self.report_snapshot();
        let source = self.replica.name().to_owned();
        let outcome = self
            .replica
            .produce(envelope, now)
            .map_err(StoredError::Ingest)?;
        self.persist_admission(envelope, IngressPath::Local, &source, now, &before)
            .await?;
        Ok(outcome)
    }

    /// Admit one envelope from a peer, and persist it.
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
        let before = self.report_snapshot();
        let outcome = self
            .replica
            .receive(path, envelope, source, now)
            .map_err(StoredError::Ingest)?;
        self.persist_admission(envelope, path, source, now, &before)
            .await?;
        Ok(outcome)
    }

    /// Reconcile with another stored replica over one link, then
    /// persist what each side admitted during the exchange (from
    /// the exchange's own outcomes) and every report that changed.
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
        let self_before = self.report_snapshot();
        let other_before = other.report_snapshot();
        let self_name = self.replica.name().to_owned();
        let other_name = other.replica.name().to_owned();
        let exchange = reconcile(&mut self.replica, &mut other.replica, config, now);
        for (event_id, outcome) in &exchange.forward.outcomes {
            if outcome.is_ok() {
                other
                    .persist_exchange_admission(event_id, &self_name, now)
                    .await?;
            }
        }
        for (event_id, outcome) in &exchange.backward.outcomes {
            if outcome.is_ok() {
                self.persist_exchange_admission(event_id, &other_name, now)
                    .await?;
            }
        }
        self.persist_reports_since(&self_before).await?;
        other.persist_reports_since(&other_before).await?;
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
        let content = ContentStore::new(backend, ContentStoreConfig::default());
        let node = NodeStore::new(content, dir.join(INDEX_FILE))?;
        Self::open(node, replica).await
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use ed25519_dalek::SigningKey;
    use iroh_blobs::Hash;

    use super::super::slices::b07_conflict_report::{RetentionPolicy, VersionHeader};
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
}
