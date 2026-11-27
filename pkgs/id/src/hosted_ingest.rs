//! Hosted import ingest (spec §6.2 step 4; tracker #2852 slice 4b,
//! #13158).
//!
//! Applies a verified hosted export at the importing (self-hosted)
//! node. This is the step downstream of [`crate::hosted_export::verify_import`]:
//! nothing here runs until verification has passed, and the
//! [`VerifiedImport`] it produced must be handed in — a token that
//! names a different export is refused before anything is written.
//!
//! Ingest is not a privileged write path. Each verified object goes
//! through the event store's normal ingest ([`EventStore::ingest`]),
//! the same verification, fork, and waiting rules any peer's event
//! faces (#2030: validity at ingest, counting at view), so a
//! hosted-origin object is indistinguishable from a self-hosted one
//! once stored. Each verified blob is placed into the node's blob
//! store under the hash of its bytes — the BLAKE3 hash a
//! `document.put` header cites — so citations resolve locally.
//!
//! Every entry is accounted for in the returned [`ImportReport`]:
//! applied, already present, or refused with the rule that refused
//! it. A refused entry never blocks the entries after it, and no
//! entry is silently dropped (#3810's scope requirement).
//!
//! Objects are offered to the store in the export's canonical
//! (hash) order, which is not chain order: an object whose
//! dependencies have not landed yet is held by the store's waiting
//! pool and released as the rest of the export arrives. An object
//! still waiting after the whole export has been offered is
//! reported refused under the waiting rule — the export does not
//! carry something it depends on. The store keeps its held row, so
//! a later import that carries the dependency releases it.

use iroh_blobs::Hash as BlobHash;
use iroh_blobs::api::Store as BlobStore;

use crate::chain;
use crate::event_store::{EventStore, Ingest, Refusal, StoreError};
use crate::hosted_export::{
    ExportBlob, ExportObject, HostedExport, VerifiedImport, export_fingerprint,
};

/// What ingest did with one entry of the export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryOutcome {
    /// The entry was not in the target and is stored now.
    Applied,
    /// The entry was already in the target; nothing changed.
    AlreadyPresent,
    /// The entry was refused. `rule` names the rule that refused it,
    /// in the refusing layer's own terms.
    Refused {
        /// The rule that refused the entry.
        rule: String,
    },
}

/// One object's line in an [`ImportReport`], named by its export
/// hash (SHA-256 of its bytes, hex).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectEntry {
    /// The object's export hash, hex.
    pub hash: String,
    /// What ingest did with it.
    pub outcome: EntryOutcome,
}

/// One blob's line in an [`ImportReport`], named by its export
/// hash (SHA-256 of its bytes, hex).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobEntry {
    /// The blob's export hash, hex.
    pub hash: String,
    /// What ingest did with it.
    pub outcome: EntryOutcome,
}

/// The per-entry account of one import: every object and every blob
/// of the verified export, named and dispositioned, in the export's
/// canonical order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportReport {
    /// The binding the export belongs to.
    pub binding_id: String,
    /// One line per object, in export order.
    pub objects: Vec<ObjectEntry>,
    /// One line per blob, in export order.
    pub blobs: Vec<BlobEntry>,
}

impl ImportReport {
    /// How many entries (objects and blobs) were applied.
    #[must_use]
    pub fn applied(&self) -> usize {
        self.count_where(EntryOutcome::Applied)
    }

    /// How many entries were already present.
    #[must_use]
    pub fn already_present(&self) -> usize {
        self.count_where(EntryOutcome::AlreadyPresent)
    }

    /// How many entries were refused.
    #[must_use]
    pub fn refused(&self) -> usize {
        self.objects
            .iter()
            .map(|entry| &entry.outcome)
            .chain(self.blobs.iter().map(|entry| &entry.outcome))
            .filter(|outcome| matches!(outcome, EntryOutcome::Refused { .. }))
            .count()
    }

    /// True when nothing was refused: the whole export is in the
    /// target now (applied by this import or already present).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.refused() == 0
    }

    fn count_where(&self, wanted: EntryOutcome) -> usize {
        self.objects
            .iter()
            .map(|entry| &entry.outcome)
            .chain(self.blobs.iter().map(|entry| &entry.outcome))
            .filter(|outcome| **outcome == wanted)
            .count()
    }
}

/// Why an import did not run at all. A per-entry refusal is not an
/// error: it is a line in the [`ImportReport`].
#[derive(Debug)]
pub enum ImportError {
    /// The [`VerifiedImport`] handed in does not name this export:
    /// it was minted by verifying something else. Nothing was
    /// ingested.
    VerificationMismatch,
    /// The target event store failed (a redb or corruption error,
    /// not a judgement on an entry).
    Store(StoreError),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VerificationMismatch => {
                write!(f, "the verified import does not name this export")
            }
            Self::Store(err) => write!(f, "event store error: {err}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<StoreError> for ImportError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

/// The rule a store [`Refusal`] names, in the store's own terms.
fn refusal_rule(refusal: &Refusal) -> String {
    match refusal {
        Refusal::Malformed(message) => format!("malformed: {message}"),
        Refusal::Frozen => "frozen: the event is above its author's fork point".to_owned(),
        Refusal::WaitingFull => {
            "waiting: the author already holds as many waiting events as the caps allow".to_owned()
        }
    }
}

/// One offer of one object to the store, before the waiting pool's
/// final disposition is known.
enum Step {
    /// The store decided the object for good.
    Done(EntryOutcome),
    /// The store holds the object pending a dependency.
    Waiting,
}

fn offer(store: &mut EventStore, object: &ExportObject, now_ms: u64) -> Result<Step, StoreError> {
    match store.ingest(&object.bytes, now_ms)? {
        // Accepted applies the event; Voided stores it without
        // applying (it still counts as present); a Fork stores the
        // headers as the proof. In all three the object is in the
        // store now, under exactly the rules a peer's event faces.
        Ingest::Accepted { .. } | Ingest::Voided { .. } | Ingest::Fork { .. } => {
            Ok(Step::Done(EntryOutcome::Applied))
        }
        Ingest::Duplicate { .. } => Ok(Step::Done(EntryOutcome::AlreadyPresent)),
        Ingest::Refused(refusal) => Ok(Step::Done(EntryOutcome::Refused {
            rule: refusal_rule(&refusal),
        })),
        Ingest::Waiting { .. } => Ok(Step::Waiting),
    }
}

/// The final disposition of an object the store held as waiting:
/// by the end of the import its dependencies have either landed
/// (the store holds it now) or they are not in the export at all.
fn settle_waiting(
    store: &mut EventStore,
    object: &ExportObject,
    now_ms: u64,
) -> Result<EntryOutcome, StoreError> {
    if let Ok(event) = chain::decode(&object.bytes) {
        let author = *event.header.verifying_key.as_bytes();
        if let Some(stored) = store.event(&author, event.header.seq_num)?
            && stored.id == event.id
        {
            return Ok(EntryOutcome::Applied);
        }
    }
    match offer(store, object, now_ms)? {
        Step::Done(outcome) => Ok(outcome),
        Step::Waiting => Ok(EntryOutcome::Refused {
            rule: "waiting: a dependency of this object is not present \
                   and the export does not carry it"
                .to_owned(),
        }),
    }
}

/// Place one blob in the node's blob store, under the BLAKE3 hash
/// of its bytes — the hash a `document.put` header cites.
async fn place_blob(blobs: &BlobStore, blob: &ExportBlob) -> EntryOutcome {
    let hash = BlobHash::new(&blob.bytes);
    match blobs.has(hash).await {
        Ok(true) => EntryOutcome::AlreadyPresent,
        Ok(false) => match blobs.add_bytes(blob.bytes.clone()).await {
            Ok(added) if added.hash == hash => EntryOutcome::Applied,
            Ok(added) => EntryOutcome::Refused {
                rule: format!(
                    "blob store stored the blob under {} instead of {}",
                    added.hash, hash
                ),
            },
            Err(err) => EntryOutcome::Refused {
                rule: format!("blob store error: {err}"),
            },
        },
        Err(err) => EntryOutcome::Refused {
            rule: format!("blob store error: {err}"),
        },
    }
}

/// Ingest a verified export into the target node (§6.2 step 4).
///
/// `verified` is the token [`crate::hosted_export::verify_import`]
/// minted for this export: it must name the export's binding, key,
/// entry counts, §7 client-side-encryption choice, and its
/// `unkeyed_export` trust decision must match the manifest's — and,
/// first of all, its [`crate::hosted_export::export_fingerprint`]
/// must match the export handed in, so a token minted for one
/// export can never authorize ingesting another (#16054).
/// `now_ms` is the importing node's clock, handed to the event
/// store like any other ingest's.
///
/// # Errors
///
/// Returns [`ImportError::VerificationMismatch`] — ingesting
/// nothing — when the token does not name this exact export, and
/// [`ImportError::Store`] when the event store itself fails. A
/// refusal of an individual entry is reported in the returned
/// [`ImportReport`], never as an error.
pub async fn ingest_import(
    store: &mut EventStore,
    blobs: &BlobStore,
    export: &HostedExport,
    verified: &VerifiedImport,
    now_ms: u64,
) -> Result<ImportReport, ImportError> {
    if verified.export_fingerprint() != export_fingerprint(export)
        || verified.binding_id() != export.manifest.binding_id
        || verified.iroh_id() != export.manifest.iroh_id
        || verified.object_count() != export.objects.len() as u64
        || verified.blob_count() != export.blobs.len() as u64
        || verified.unkeyed_export() != export.manifest.unkeyed_export
        || verified.client_side_encryption() != export.manifest.client_side_encryption
    {
        return Err(ImportError::VerificationMismatch);
    }

    let mut objects = Vec::with_capacity(export.objects.len());
    let mut waiting: Vec<usize> = Vec::new();
    for object in &export.objects {
        let outcome = match offer(store, object, now_ms)? {
            Step::Done(outcome) => outcome,
            Step::Waiting => {
                waiting.push(objects.len());
                EntryOutcome::AlreadyPresent // placeholder, settled below
            }
        };
        objects.push(ObjectEntry {
            hash: object.hash.clone(),
            outcome,
        });
    }
    for index in waiting {
        objects[index].outcome = settle_waiting(store, &export.objects[index], now_ms)?;
    }

    let mut blob_entries = Vec::with_capacity(export.blobs.len());
    for blob in &export.blobs {
        blob_entries.push(BlobEntry {
            hash: blob.hash.clone(),
            outcome: place_blob(blobs, blob).await,
        });
    }

    Ok(ImportReport {
        binding_id: export.manifest.binding_id.clone(),
        objects,
        blobs: blob_entries,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use ed25519_dalek::SigningKey as DalekKey;
    use iroh_blobs::store::mem::MemStore;
    use p2panda_core::SigningKey;

    use crate::chain::{self, ChainExt, Event};
    use crate::hosted_binding::HostedBindingService;
    use sha2::{Digest, Sha256};

    use crate::hosted_binding::hex_encode;
    use crate::hosted_export::{ExportError, build_export, verify_import};
    use id_core::waiting::WaitingLimits;
    use id_store::documents::{DOCUMENT_KIND, DocumentRef};

    const SUB: &str = "google-subject-1";
    const CREATED: &str = "2026-11-13T00:00:00Z";
    const SEED: u8 = 21;

    fn dalek_key(seed: u8) -> DalekKey {
        DalekKey::from_bytes(&[seed; 32])
    }

    fn p2p_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn ext(kind: &str) -> ChainExt {
        ChainExt {
            kind: kind.to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
            thread: None,
        }
    }

    fn open_store() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, store)
    }

    fn blob_store() -> BlobStore {
        let store: BlobStore = MemStore::new().into();
        store
    }

    /// A three-event chain by the binding's root key: two content
    /// events and a `document.put` citing one blob. Returns the
    /// events and the blob bytes.
    fn source_chain() -> (Vec<Event>, Vec<u8>) {
        source_chain_with(b"the document the export carries")
    }

    /// The same chain shape over different document bytes, so two
    /// exports can share identity, keys, counts, and trust flags
    /// while carrying different entries.
    fn source_chain_with(document: &[u8]) -> (Vec<Event>, Vec<u8>) {
        let key = p2p_key(SEED);
        let content = document.to_vec();
        let first = chain::sign(&key, 0, None, None, ext("message")).unwrap();
        let second = chain::sign(&key, 1, Some(first.id), None, ext("message")).unwrap();
        let mut doc_ext = ext(DOCUMENT_KIND);
        doc_ext.document = Some(DocumentRef::for_bytes("text/plain", &content).unwrap());
        let third = chain::sign(&key, 2, Some(second.id), None, doc_ext).unwrap();
        (vec![first, second, third], content)
    }

    /// Build an export over `events` from a fresh binding for
    /// `sub` with the §7 choice `cse`, as the hosted service would
    /// produce it. When `blob_bytes` is given, the export carries
    /// that one blob and the chain's `document.put` event cites it.
    fn make_export_for(
        sub: &str,
        events: &[Event],
        blob_bytes: Option<&[u8]>,
        cse: Option<bool>,
    ) -> HostedExport {
        let mut service = HostedBindingService::with_seed("hosted.example", [9u8; 32]);
        service.signup_account(sub, None, "free").unwrap();
        let binding = service
            .complete_browser_ceremony(sub, &dalek_key(SEED), cse)
            .unwrap();
        let blob = blob_bytes.map(|bytes| ExportBlob::new(bytes.to_vec()));
        let objects: Vec<ExportObject> = events
            .iter()
            .map(|event| {
                let refs = match (&blob, event.header.extensions.kind.as_str()) {
                    (Some(blob), DOCUMENT_KIND) => vec![blob.hash.clone()],
                    _ => Vec::new(),
                };
                ExportObject::from_envelope(event.bytes.clone(), refs).unwrap()
            })
            .collect();
        build_export(
            &service,
            &binding,
            Vec::new(),
            objects,
            blob.into_iter().collect(),
            Some(&dalek_key(SEED)),
            "export-1",
            CREATED,
        )
    }

    /// Build a verified export over `events` from a fresh binding,
    /// as the hosted service would produce it. When `blob_bytes` is
    /// given, the export carries that one blob and the chain's
    /// `document.put` event cites it.
    fn make_export(events: &[Event], blob_bytes: Option<&[u8]>) -> HostedExport {
        make_export_for(SUB, events, blob_bytes, None)
    }

    #[tokio::test]
    async fn round_trip_lands_every_object_and_blob() {
        let (events, content) = source_chain();
        // The source node holds the chain first: export is what a
        // store's contents become.
        let (_src_dir, mut source) = open_store();
        for event in &events {
            assert!(matches!(
                source.ingest(&event.bytes, 0).unwrap(),
                Ingest::Accepted { .. }
            ));
        }
        let export = make_export(&events, Some(&content));
        let verified = verify_import(&export, false).unwrap();

        let (_dir, mut target) = open_store();
        let blobs = blob_store();
        let report = ingest_import(&mut target, &blobs, &export, &verified, 0)
            .await
            .unwrap();
        assert!(report.is_complete(), "report: {report:?}");
        assert_eq!(report.applied(), 4); // 3 objects + 1 blob
        assert_eq!(report.objects.len(), 3);
        assert_eq!(report.blobs.len(), 1);

        // The target's object set is the export's set: every object
        // is stored, byte-identical, at its author's sequence.
        let author = *p2p_key(SEED).verifying_key().as_bytes();
        for event in &events {
            let stored = target
                .event(&author, event.header.seq_num)
                .unwrap()
                .expect("ingested event is stored");
            assert_eq!(stored.id, event.id);
            assert_eq!(stored.bytes, event.bytes);
        }
        // The blob is retrievable by the hash the document.put
        // header cites, so the citation resolves locally.
        let cited = BlobHash::new(&content);
        let fetched = blobs.get_bytes(cited).await.unwrap();
        assert_eq!(&fetched[..], &content[..]);
    }

    #[tokio::test]
    async fn reingest_reports_everything_already_present() {
        let (events, content) = source_chain();
        let export = make_export(&events, Some(&content));
        let verified = verify_import(&export, false).unwrap();
        let (_dir, mut target) = open_store();
        let blobs = blob_store();
        let first = ingest_import(&mut target, &blobs, &export, &verified, 0)
            .await
            .unwrap();
        assert!(first.is_complete());
        let second = ingest_import(&mut target, &blobs, &export, &verified, 0)
            .await
            .unwrap();
        assert!(second.is_complete(), "report: {second:?}");
        assert_eq!(second.applied(), 0);
        assert_eq!(second.already_present(), 4);
    }

    #[test]
    fn an_object_that_is_not_an_event_is_refused_at_verification() {
        // Section 6.3 (slice 4b production, #13157): an export object
        // IS its signed envelope, so bytes that are not an event fail
        // section 6.2 verification itself. The refusal this test used
        // to meet at ingest, under the store's malformed rule, now
        // happens one layer earlier, before anything is offered to
        // a store.
        let mut service = HostedBindingService::with_seed("hosted.example", [9u8; 32]);
        service.signup_account(SUB, None, "free").unwrap();
        let binding = service
            .complete_browser_ceremony(SUB, &dalek_key(SEED), None)
            .unwrap();
        let (events, content) = source_chain();
        let good = ExportObject::from_envelope(events[0].bytes.clone(), Vec::new()).unwrap();
        let middle = ExportObject::from_envelope(events[1].bytes.clone(), Vec::new()).unwrap();
        let blob = ExportBlob::new(content.clone());
        let citing =
            ExportObject::from_envelope(events[2].bytes.clone(), vec![blob.hash.clone()]).unwrap();
        // Hand-built: `from_envelope` refuses these bytes by
        // construction, so the raw struct is the only way to carry
        // them - exactly what a hostile producer would send.
        let bogus = ExportObject {
            hash: hex_encode(&Sha256::digest(b"not an event")),
            signer: hex_encode(&dalek_key(SEED).verifying_key().to_bytes()),
            signature: hex_encode(&[0u8; 64]),
            bytes: b"not an event".to_vec(),
            blob_refs: Vec::new(),
        };
        let export = build_export(
            &service,
            &binding,
            Vec::new(),
            vec![good, middle, bogus, citing],
            vec![blob],
            Some(&dalek_key(SEED)),
            "export-1",
            CREATED,
        );
        assert_eq!(
            verify_import(&export, false).unwrap_err(),
            ExportError::ObjectSignatureInvalid
        );
    }

    #[tokio::test]
    async fn an_object_missing_its_predecessor_is_refused_as_waiting() {
        let (events, content) = source_chain();
        // Export only seq 1: its backlink names an event the export
        // does not carry.
        let export = make_export(&events[1..2], None);
        let verified = verify_import(&export, false).unwrap();
        let (_dir, mut target) = open_store();
        let blobs = blob_store();
        let report = ingest_import(&mut target, &blobs, &export, &verified, 0)
            .await
            .unwrap();
        assert_eq!(report.objects.len(), 1);
        match &report.objects[0].outcome {
            EntryOutcome::Refused { rule } => {
                assert!(rule.starts_with("waiting"), "rule: {rule}");
            }
            other => panic!("expected a waiting refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_token_for_another_export_is_refused_before_any_write() {
        let (events, content) = source_chain();
        let export = make_export(&events, Some(&content));
        // The token is opaque: the only way to hold a token for a
        // different export is to verify one. Same root key and
        // entry shapes, but a different binding record.
        let other = make_export_for("other-subject-9", &events, Some(&content), None);
        let wrong = verify_import(&other, false).unwrap();
        assert_ne!(
            wrong.binding_id(),
            verify_import(&export, false).unwrap().binding_id()
        );
        assert_ne!(
            wrong.export_fingerprint(),
            verify_import(&export, false).unwrap().export_fingerprint()
        );
        let (_dir, mut target) = open_store();
        let blobs = blob_store();
        let err = ingest_import(&mut target, &blobs, &export, &wrong, 0)
            .await
            .unwrap_err();
        assert!(matches!(err, ImportError::VerificationMismatch));
        // Nothing landed.
        let author = *p2p_key(SEED).verifying_key().as_bytes();
        assert!(target.event(&author, 0).unwrap().is_none());
        assert!(blobs.get_bytes(BlobHash::new(&content)).await.is_err());
    }

    #[tokio::test]
    async fn a_token_with_a_different_cse_choice_is_refused_before_any_write() {
        let (events, content) = source_chain();
        let export = make_export(&events, Some(&content));
        // Same root key and entry shapes, but the binding stored
        // the data as plaintext: the token's §7 choice disagrees
        // with the export's.
        let plain = make_export_for("plain-subject-2", &events, Some(&content), Some(false));
        let wrong = verify_import(&plain, false).unwrap();
        assert!(!wrong.client_side_encryption());
        let (_dir, mut target) = open_store();
        let blobs = blob_store();
        let err = ingest_import(&mut target, &blobs, &export, &wrong, 0)
            .await
            .unwrap_err();
        assert!(matches!(err, ImportError::VerificationMismatch));
        // The token is the authority to proceed only with the exact
        // verified interpretation; reject before any object or blob
        // is written.
        let author = *p2p_key(SEED).verifying_key().as_bytes();
        assert!(target.event(&author, 0).unwrap().is_none());
        assert!(blobs.get_bytes(BlobHash::new(&content)).await.is_err());
    }

    /// Two exports from ONE binding with identical identity,
    /// keys, counts, and trust flags but different entries
    /// (#16054): the same chain shape over different document
    /// bytes, so the exports differ only in their third event and
    /// their blob.
    fn make_same_binding_pair() -> (HostedExport, HostedExport) {
        let mut service = HostedBindingService::with_seed("hosted.example", [9u8; 32]);
        service.signup_account(SUB, None, "free").unwrap();
        let binding = service
            .complete_browser_ceremony(SUB, &dalek_key(SEED), None)
            .unwrap();
        let build = |events: &[Event], blob_bytes: &[u8], export_id: &str| {
            let blob = ExportBlob::new(blob_bytes.to_vec());
            let objects: Vec<ExportObject> = events
                .iter()
                .map(|event| {
                    let refs = match event.header.extensions.kind.as_str() {
                        DOCUMENT_KIND => vec![blob.hash.clone()],
                        _ => Vec::new(),
                    };
                    ExportObject::from_envelope(event.bytes.clone(), refs).unwrap()
                })
                .collect();
            build_export(
                &service,
                &binding,
                Vec::new(),
                objects,
                vec![blob],
                Some(&dalek_key(SEED)),
                export_id,
                CREATED,
            )
        };
        let (events_a, content_a) = source_chain();
        let (events_b, content_b) = source_chain_with(b"a different document payload");
        (
            build(&events_a, &content_a, "export-a"),
            build(&events_b, &content_b, "export-b"),
        )
    }

    #[tokio::test]
    async fn a_token_cannot_cross_to_a_same_shape_export_with_different_entries() {
        // #16054: the token is bound to the exact export. Identity,
        // counts, and trust flags all match across this pair, so
        // only the fingerprint can refuse the cross-use.
        let (export_a, export_b) = make_same_binding_pair();
        let token_a = verify_import(&export_a, false).unwrap();
        let token_b = verify_import(&export_b, false).unwrap();
        assert_eq!(token_a.binding_id(), token_b.binding_id());
        assert_eq!(token_a.iroh_id(), token_b.iroh_id());
        assert_eq!(token_a.object_count(), token_b.object_count());
        assert_eq!(token_a.blob_count(), token_b.blob_count());
        assert_eq!(token_a.unkeyed_export(), token_b.unkeyed_export());
        assert_eq!(
            token_a.client_side_encryption(),
            token_b.client_side_encryption()
        );
        assert_ne!(token_a.export_fingerprint(), token_b.export_fingerprint());

        // The mismatched token writes nothing: no events, no blobs.
        let (_dir, mut target) = open_store();
        let blobs = blob_store();
        let err = ingest_import(&mut target, &blobs, &export_a, &token_b, 0)
            .await
            .unwrap_err();
        assert!(matches!(err, ImportError::VerificationMismatch));
        let author = *p2p_key(SEED).verifying_key().as_bytes();
        for seq in 0..3 {
            assert!(target.event(&author, seq).unwrap().is_none());
        }
        for export in [&export_a, &export_b] {
            for blob in &export.blobs {
                assert!(blobs.get_bytes(BlobHash::new(&blob.bytes)).await.is_err());
            }
        }

        // The exact token/export pair still imports whole.
        let report = ingest_import(&mut target, &blobs, &export_a, &token_a, 0)
            .await
            .unwrap();
        assert!(report.is_complete(), "report: {report:?}");
        assert_eq!(report.applied(), 4); // 3 objects + 1 blob
    }

    /// Build the keyed and unkeyed shapes of one export from a
    /// single binding: identical binding ID, Iroh ID, objects, and
    /// blobs — only the manifest's `unkeyed_export` trust flag and
    /// its signer differ.
    fn make_keyed_and_unkeyed_exports(
        events: &[Event],
        blob_bytes: Option<&[u8]>,
    ) -> (HostedExport, HostedExport) {
        let mut service = HostedBindingService::with_seed("hosted.example", [9u8; 32]);
        service.signup_account(SUB, None, "free").unwrap();
        let binding = service
            .complete_browser_ceremony(SUB, &dalek_key(SEED), None)
            .unwrap();
        let blob = blob_bytes.map(|bytes| ExportBlob::new(bytes.to_vec()));
        let objects: Vec<ExportObject> = events
            .iter()
            .map(|event| {
                let refs = match (&blob, event.header.extensions.kind.as_str()) {
                    (Some(blob), DOCUMENT_KIND) => vec![blob.hash.clone()],
                    _ => Vec::new(),
                };
                ExportObject::from_envelope(event.bytes.clone(), refs).unwrap()
            })
            .collect();
        let blobs: Vec<ExportBlob> = blob.into_iter().collect();
        let build = |user_key: Option<&DalekKey>| {
            build_export(
                &service,
                &binding,
                Vec::new(),
                objects.clone(),
                blobs.clone(),
                user_key,
                "export-1",
                CREATED,
            )
        };
        (build(Some(&dalek_key(SEED))), build(None))
    }

    #[tokio::test]
    async fn a_keyed_token_cannot_ingest_an_unkeyed_export() {
        let (events, content) = source_chain();
        let (keyed, unkeyed) = make_keyed_and_unkeyed_exports(&events, Some(&content));
        assert!(!keyed.manifest.unkeyed_export);
        assert!(unkeyed.manifest.unkeyed_export);
        // Same binding, Iroh ID, and entry counts: only the trust
        // flag differs, so only the token binding can refuse this.
        assert_eq!(keyed.manifest.binding_id, unkeyed.manifest.binding_id);
        assert_eq!(keyed.manifest.iroh_id, unkeyed.manifest.iroh_id);
        assert_eq!(keyed.objects.len(), unkeyed.objects.len());
        assert_eq!(keyed.blobs.len(), unkeyed.blobs.len());

        let keyed_token = verify_import(&keyed, false).unwrap();
        assert!(!keyed_token.unkeyed_export());
        let (_dir, mut target) = open_store();
        let blobs = blob_store();
        let err = ingest_import(&mut target, &blobs, &unkeyed, &keyed_token, 0)
            .await
            .unwrap_err();
        assert!(matches!(err, ImportError::VerificationMismatch));
        // Nothing landed.
        let author = *p2p_key(SEED).verifying_key().as_bytes();
        assert!(target.event(&author, 0).unwrap().is_none());

        // The mirror image: a token minted under operator
        // confirmation for the unkeyed export does not authorize
        // ingesting the keyed export either.
        let unkeyed_token = verify_import(&unkeyed, true).unwrap();
        assert!(unkeyed_token.unkeyed_export());
        let err = ingest_import(&mut target, &blobs, &keyed, &unkeyed_token, 0)
            .await
            .unwrap_err();
        assert!(matches!(err, ImportError::VerificationMismatch));
        assert!(target.event(&author, 0).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_confirmed_unkeyed_token_ingests_its_unkeyed_export() {
        let (events, content) = source_chain();
        let (_keyed, unkeyed) = make_keyed_and_unkeyed_exports(&events, Some(&content));
        let verified = verify_import(&unkeyed, true).unwrap();
        assert!(verified.unkeyed_export());
        let (_dir, mut target) = open_store();
        let blobs = blob_store();
        let report = ingest_import(&mut target, &blobs, &unkeyed, &verified, 0)
            .await
            .unwrap();
        assert!(report.is_complete(), "report: {report:?}");
        assert_eq!(report.applied(), 4); // 3 objects + 1 blob
        let author = *p2p_key(SEED).verifying_key().as_bytes();
        assert!(target.event(&author, 0).unwrap().is_some());
    }
}
