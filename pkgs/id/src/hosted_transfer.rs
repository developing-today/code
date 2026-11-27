//! Verified resumable user-store transfer and safe source cleanup:
//! the export/copy/verify orchestration over the fenced migration
//! control plane (issue #3402, Phase 5 of the #2283 program).
//!
//! [`crate::hosted_migration`] decides which migration events are
//! legal; this module performs the work those events describe, under
//! the machine's fencing. [`crate::hosted_export`] already owns the
//! export format and import verification; this module owns moving a
//! verified export between jurisdiction-specific buckets and retiring
//! the source without ever deleting a blob another store in the
//! jurisdiction still reaches.
//!
//! ## What is here
//!
//! - [`TransferPlan`]: the manifest of one migration, derived from a
//!   [`HostedExport`]. Reachable blobs are exactly the union of the
//!   objects' blob citations: a cited blob the export does not carry
//!   is refused ([`TransferError::MissingBlob`], mirroring
//!   §6.2), and a carried blob no object cites is refused
//!   ([`TransferError::UnreferencedBlob`]). Every object's and blob's
//!   hash is recomputed from its bytes at plan time, so the signed
//!   bytes are the bytes the plan names — never a re-encoding.
//! - [`TransferJournal`]: the durable, serializable record of the
//!   transfer — the plan plus per-entry copy state and an audit log.
//!   A coordinator that dies mid-copy persists the journal; a fresh
//!   coordinator resumes from it and does not re-copy entries whose
//!   target bytes already hash to the plan (content addressing is
//!   the resume proof).
//! - [`TransferCoordinator`]: drives the [`MigrationRecord`] through
//!   `Frozen → Copying → Verifying → Switched → Cleaning` while it
//!   copies, verifies, switches and cleans up. The CAS switch is
//!   refused until read-back verification of the target — hashes,
//!   content and signatures, via [`verify_import`] on the export
//!   reconstructed from the target bucket — has passed. Pre-switch
//!   failure aborts through the machine (token rotates, target is
//!   discarded); post-switch the machine refuses abort and
//!   [`TransferCoordinator::resume_action`] points forward, at
//!   cleanup.
//! - Safe cleanup: source deletion happens only after the switch,
//!   only under a [`CleanupProof`] whose reachability scan version
//!   matches the scan the caller just took (a stale proof deletes
//!   nothing), and only for blobs the jurisdiction-local scan no
//!   longer reaches from any other store. Shared blobs are retained,
//!   named in the [`CleanupReport`], never deleted on one store's
//!   say-so. Retention must be affirmatively satisfied; there is no
//!   default-true path.
//!
//! ## Runbook (one account; exercised end-to-end in the tests)
//!
//! 1. Freeze: drive the record to `Frozen` via #3404's machine
//!    (begin, freeze acknowledgement with the durable watermark).
//! 2. Plan: [`TransferPlan::from_export`] over the frozen store's
//!    export; persist the journal before copying anything.
//! 3. Copy: [`TransferCoordinator::copy_all`] into the target
//!    jurisdiction's bucket; on any error, persist the journal and
//!    resume later — completed entries are not re-copied.
//! 4. Verify: [`TransferCoordinator::verify`] reads the target back
//!    and re-runs import verification; only then
//!    [`TransferCoordinator::switch`] performs the CAS switch.
//! 5. Cleanup: after retention elapses, take a fresh reachability
//!    scan of the source jurisdiction, build the proof from it, and
//!    [`TransferCoordinator::cleanup`]; then retire the record.
//!
//! ## What is not here
//!
//! The live bucket adapters (R2/DO in the hosted data plane), the
//! durable storage of the journal (the directory in #3399), and the
//! freeze/drain handshake (#3404's entry guards). Those feed this
//! core buckets and records; this core decides what may be copied,
//! switched and deleted.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::hosted_binding::hex_encode;
use crate::hosted_export::{ExportBlob, ExportError, HostedExport, verify_import};
use crate::hosted_migration::{
    MigrationError, MigrationEvent, MigrationPhase, MigrationRecord, TransitionContext,
};

fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

/// One jurisdiction-specific bucket, keyed by content hash. The
/// hosted data plane adapts R2/DO storage to this trait; tests use
/// in-memory buckets with fault injection. Errors are the adapter's
/// message; the coordinator wraps them in
/// [`TransferError::Bucket`] with the operation and hash.
pub trait BlobBucket {
    /// Fetch the bytes stored under `hash`, if any.
    fn get(&self, hash: &str) -> Result<Option<Vec<u8>>, String>;
    /// Store `bytes` under `hash`. Must be idempotent: storing the
    /// same hash twice stores the same content once.
    fn put(&mut self, hash: &str, bytes: &[u8]) -> Result<(), String>;
    /// Delete the entry under `hash`, if present.
    fn delete(&mut self, hash: &str) -> Result<(), String>;
}

/// Whether a plan entry is a signed object or a blob it cites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    /// A signed graph object: its bytes are the stored envelope,
    /// verbatim, and its signature must survive the transfer.
    Object,
    /// A content blob cited by at least one object in the plan.
    Blob,
}

/// Copy state of one plan entry, as recorded in the journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryState {
    /// Not yet copied (or copied by an attempt that never journaled
    /// the completion — resume re-checks the target by hash).
    Pending,
    /// Present in the target bucket; not yet read-back verified.
    Copied,
    /// Read back from the target and hash-verified during
    /// [`TransferCoordinator::verify`].
    Verified,
}

/// One entry of a [`TransferPlan`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    /// Content hash (SHA-256, hex) naming the entry in both buckets.
    pub hash: String,
    /// Object or blob.
    pub kind: EntryKind,
    /// Byte length of the entry's content.
    pub size: u64,
    /// Copy state; mutated only through the journal.
    pub state: EntryState,
}

/// The manifest of one migration: every object of the export and
/// exactly the blobs those objects reach, in deterministic order
/// (objects by hash, then blobs by hash).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferPlan {
    /// The migration this plan serves; must match the record's.
    pub migration_id: String,
    /// The export the plan was derived from.
    pub export_id: String,
    /// The binding the export belongs to.
    pub binding_id: String,
    /// Source jurisdiction label, from the migration record.
    pub source_jurisdiction: String,
    /// Target jurisdiction label, from the migration record.
    pub target_jurisdiction: String,
    /// The blob hashes the export's objects cite, sorted: the
    /// reachability set the copy must move and cleanup must respect.
    pub reachable_blobs: Vec<String>,
    /// Objects then blobs, each sorted by hash.
    pub entries: Vec<PlanEntry>,
}

impl TransferPlan {
    /// Derive the plan from a migration record in flight and the
    /// frozen source store's export. The record must carry the
    /// migration ID and target (i.e. be past `Begin`); the export's
    /// hashes are recomputed from its bytes, its reachable set is
    /// computed from the objects' citations, and any cited-but-absent
    /// or carried-but-unreachable blob is a refusal, not a skip.
    ///
    /// # Errors
    ///
    /// [`TransferError`] naming the first rule that failed.
    pub fn from_export(
        record: &MigrationRecord,
        export: &HostedExport,
    ) -> Result<Self, TransferError> {
        let Some(migration_id) = record.migration_id.clone() else {
            return Err(TransferError::RecordNotBegun);
        };
        let Some(target) = record.target.clone() else {
            return Err(TransferError::RecordNotBegun);
        };
        let mut reachable: BTreeSet<String> = BTreeSet::new();
        let mut seen_objects: BTreeSet<String> = BTreeSet::new();
        let mut entries: Vec<PlanEntry> = Vec::new();
        for object in &export.objects {
            if sha256_hex(&object.bytes) != object.hash {
                return Err(TransferError::Export(ExportError::ObjectHashMismatch));
            }
            if !seen_objects.insert(object.hash.clone()) {
                return Err(TransferError::Export(ExportError::DuplicateObject));
            }
            for cited in &object.blob_refs {
                reachable.insert(cited.clone());
            }
            entries.push(PlanEntry {
                hash: object.hash.clone(),
                kind: EntryKind::Object,
                size: object.bytes.len() as u64,
                state: EntryState::Pending,
            });
        }
        let mut carried: BTreeSet<String> = BTreeSet::new();
        for blob in &export.blobs {
            if sha256_hex(&blob.bytes) != blob.hash {
                return Err(TransferError::Export(ExportError::BlobHashMismatch));
            }
            carried.insert(blob.hash.clone());
        }
        for cited in &reachable {
            if !carried.contains(cited) {
                return Err(TransferError::MissingBlob { hash: cited.clone() });
            }
        }
        for blob in &export.blobs {
            if !reachable.contains(&blob.hash) {
                return Err(TransferError::UnreferencedBlob {
                    hash: blob.hash.clone(),
                });
            }
            entries.push(PlanEntry {
                hash: blob.hash.clone(),
                kind: EntryKind::Blob,
                size: blob.bytes.len() as u64,
                state: EntryState::Pending,
            });
        }
        entries.sort_by(|a, b| {
            let rank = |kind: EntryKind| match kind {
                EntryKind::Object => 0,
                EntryKind::Blob => 1,
            };
            rank(a.kind).cmp(&rank(b.kind)).then(a.hash.cmp(&b.hash))
        });
        Ok(Self {
            migration_id,
            export_id: export.manifest.export_id.clone(),
            binding_id: export.manifest.binding_id.clone(),
            source_jurisdiction: record.source.clone(),
            target_jurisdiction: target,
            reachable_blobs: reachable.into_iter().collect(),
            entries,
        })
    }

    /// Entries not yet journaled as copied or verified.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.state == EntryState::Pending)
            .count()
    }
}

/// The durable transfer record: the plan (with per-entry state) and
/// an audit log of the steps taken. Serialize it before copying and
/// after every step; a replacement coordinator resumes from it
/// verbatim. The signed bytes themselves live in the buckets — the
/// journal names them by hash, so a tampered journal entry fails
/// against the content it names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferJournal {
    /// The plan being executed, with live entry states.
    pub plan: TransferPlan,
    /// True only after [`TransferCoordinator::verify`] passed on a
    /// full read-back of the target. The switch refuses without it.
    pub verified: bool,
    /// Audit log lines, in order (`begin_copy`, `copied:<hash>`,
    /// `verified`, `switched`, `aborted:<reason>`,
    /// `cleanup:deleted=<n>,retained=<m>`).
    pub log: Vec<String>,
}

impl TransferJournal {
    /// A fresh journal over a plan: nothing copied, nothing verified.
    #[must_use]
    pub fn new(plan: TransferPlan) -> Self {
        Self {
            plan,
            verified: false,
            log: Vec::new(),
        }
    }
}

/// A jurisdiction-local reachability scan: the blob hashes still
/// reachable from any store in the source jurisdiction, and the
/// scan's version. Cleanup compares the proof's version against a
/// fresh scan's; a scan taken before another store's write is stale
/// proof and deletes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReachabilityScan {
    /// Monotonic version of the jurisdiction's reachability view.
    pub version: u64,
    /// Blob hashes reachable from stores other than the migrating
    /// one (the migrating store's own citations are the plan's).
    pub reachable_from_other_stores: BTreeSet<String>,
}

/// The caller's attestation that source cleanup may proceed: the
/// reachability scan it was computed from, by version, and that the
/// post-switch retention requirement has been satisfied. Both are
/// affirmative inputs; neither defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupProof {
    /// Version of the [`ReachabilityScan`] the caller just took.
    pub scan_version: u64,
    /// Retention for the switched source has elapsed / been waived
    /// by the runbook's retention rule.
    pub retention_satisfied: bool,
}

/// What a cleanup pass did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupReport {
    /// Hashes deleted from the source bucket, sorted.
    pub deleted: Vec<String>,
    /// Blob hashes retained because another store in the
    /// jurisdiction still reaches them, sorted.
    pub retained_shared: Vec<String>,
}

/// What a resumed coordinator should do next, from the record's
/// phase and the journal. Post-switch phases never point backward:
/// recovery after the switch is forward, per the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeAction {
    /// Record is `Pending`: wait for the freeze acknowledgement.
    AwaitFreeze,
    /// Record is `Frozen`: begin the copy.
    BeginCopy,
    /// Record is `Copying` with entries still pending.
    Copy,
    /// Record is `Verifying` (or copy-complete `Copying`): verify.
    Verify,
    /// Verified and in `Verifying`: perform the CAS switch.
    Switch,
    /// Record is `Switched` or `Cleaning`: clean up the source.
    Cleanup,
    /// Record is `Stable` with no migration open (e.g. after an
    /// abort): nothing to resume.
    Nothing,
    /// Record is `Retired`: the migration is complete.
    Done,
}

/// Every refusal is a typed error naming the rule that failed; a
/// refused step mutates neither the record nor the buckets, except
/// where the step's contract says partial copy progress is journaled
/// (that progress is the resume mechanism, not a side effect).
#[derive(Clone, Debug, PartialEq, Error)]
pub enum TransferError {
    /// The migration machine refused the event.
    #[error("migration refused: {0}")]
    Migration(#[from] MigrationError),
    /// Export verification refused the export.
    #[error("export refused: {0}")]
    Export(#[from] ExportError),
    /// The record has no migration open (no ID / target yet).
    #[error("migration record has not begun: no migration id or target recorded")]
    RecordNotBegun,
    /// The plan and the record name different migrations.
    #[error("plan is for migration {plan}, record is for migration {record}")]
    MigrationMismatch {
        /// Migration ID in the plan / journal.
        plan: String,
        /// Migration ID in the record.
        record: String,
    },
    /// An object cites a blob the export does not carry.
    #[error("object cites blob {hash} the export does not carry")]
    MissingBlob {
        /// The cited blob's hash.
        hash: String,
    },
    /// A carried blob no object cites: copying it would move data
    /// the manifest does not account for.
    #[error("blob {hash} is carried by the export but cited by no object")]
    UnreferencedBlob {
        /// The unreferenced blob's hash.
        hash: String,
    },
    /// The source bucket does not hold an entry the plan names.
    #[error("source bucket is missing planned entry {hash}")]
    SourceMissing {
        /// The missing entry's hash.
        hash: String,
    },
    /// Bytes read under a hash do not hash to it: the store is
    /// corrupt or the journal was tampered with. Names the side.
    #[error("{side} bytes under {hash} do not hash to that name")]
    CorruptEntry {
        /// `"source"` or `"target"`.
        side: &'static str,
        /// The entry's expected hash.
        hash: String,
    },
    /// A bucket adapter failed.
    #[error("bucket {operation} failed for {hash}: {message}")]
    Bucket {
        /// `"get"`, `"put"` or `"delete"`.
        operation: &'static str,
        /// The entry being operated on.
        hash: String,
        /// The adapter's error message.
        message: String,
    },
    /// Copy was asked to switch or verify before every entry landed.
    #[error("copy incomplete: {pending} plan entries not yet copied")]
    CopyIncomplete {
        /// Entries still pending.
        pending: usize,
    },
    /// The CAS switch was attempted before read-back verification.
    #[error("switch refused: target has not passed read-back verification")]
    SwitchRequiresVerification,
    /// Cleanup was attempted before the switch.
    #[error("cleanup refused: record is in phase {phase:?}, before the switch")]
    CleanupBeforeSwitch {
        /// The record's current phase.
        phase: MigrationPhase,
    },
    /// Cleanup without satisfied retention deletes nothing.
    #[error("cleanup refused: source retention has not been satisfied")]
    RetentionNotSatisfied,
    /// The proof names a scan version other than the fresh scan's.
    #[error("cleanup refused: proof is for scan {proof}, fresh scan is {fresh}")]
    StaleReachabilityProof {
        /// Scan version in the proof.
        proof: u64,
        /// Scan version just taken.
        fresh: u64,
    },
}

/// Drives one migration's transfer: a [`MigrationRecord`] plus its
/// [`TransferJournal`]. All phase changes go through the machine
/// with the record's own epoch and fencing token, so a coordinator
/// holding a stale record fails its CAS exactly as the machine
/// specifies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferCoordinator {
    /// The fenced migration record.
    pub record: MigrationRecord,
    /// The transfer journal.
    pub journal: TransferJournal,
}

impl TransferCoordinator {
    /// Open a coordinator over a record past `Begin` and the plan
    /// derived from its export. The plan's migration must be the
    /// record's.
    ///
    /// # Errors
    ///
    /// [`TransferError::MigrationMismatch`] when the plan names a
    /// different migration than the record.
    pub fn new(record: MigrationRecord, plan: TransferPlan) -> Result<Self, TransferError> {
        if let Some(record_id) = record.migration_id.as_deref() {
            if record_id != plan.migration_id {
                return Err(TransferError::MigrationMismatch {
                    plan: plan.migration_id.clone(),
                    record: record_id.to_string(),
                });
            }
        }
        Ok(Self {
            record,
            journal: TransferJournal::new(plan),
        })
    }

    fn apply(&mut self, event: &MigrationEvent) -> Result<(), TransferError> {
        let ctx = TransitionContext {
            expected_epoch: self.record.epoch,
            fencing_token: &self.record.fencing_token,
        };
        self.record = self.record.transition(event, &ctx)?;
        Ok(())
    }

    /// What a coordinator resuming from this state should do next.
    #[must_use]
    pub fn resume_action(&self) -> ResumeAction {
        match self.record.phase {
            MigrationPhase::Pending => ResumeAction::AwaitFreeze,
            MigrationPhase::Frozen => ResumeAction::BeginCopy,
            MigrationPhase::Copying => {
                if self.journal.plan.pending_count() == 0 {
                    ResumeAction::Verify
                } else {
                    ResumeAction::Copy
                }
            }
            MigrationPhase::Verifying => {
                if self.journal.verified {
                    ResumeAction::Switch
                } else {
                    ResumeAction::Verify
                }
            }
            MigrationPhase::Switched | MigrationPhase::Cleaning => ResumeAction::Cleanup,
            MigrationPhase::Stable => ResumeAction::Nothing,
            MigrationPhase::Retired => ResumeAction::Done,
        }
    }

    /// Begin the copy: `Frozen → Copying`. Refused by the machine
    /// from any other phase (export waits for the freeze
    /// acknowledgement by construction).
    ///
    /// # Errors
    ///
    /// [`TransferError::Migration`] when the machine refuses.
    pub fn begin_copy(&mut self) -> Result<(), TransferError> {
        self.apply(&MigrationEvent::BeginCopy)?;
        self.journal.log.push("begin_copy".to_string());
        Ok(())
    }

    /// Copy every pending entry from the source bucket to the
    /// target bucket, journaling each completion. Resumable: an
    /// entry whose target bytes already hash to the plan (copied by
    /// an attempt that died before journaling) is adopted without a
    /// re-put, and `Copied`/`Verified` entries are skipped. On a
    /// bucket error the entries completed so far stay journaled —
    /// persist the journal and call again to resume.
    ///
    /// # Errors
    ///
    /// [`TransferError`] on the first failing entry; the journal
    /// reflects exactly the entries that completed before it.
    pub fn copy_all(
        &mut self,
        source: &dyn BlobBucket,
        target: &mut dyn BlobBucket,
    ) -> Result<(), TransferError> {
        if self.record.phase == MigrationPhase::Frozen {
            self.begin_copy()?;
        }
        let hashes: Vec<String> = self
            .journal
            .plan
            .entries
            .iter()
            .filter(|entry| entry.state == EntryState::Pending)
            .map(|entry| entry.hash.clone())
            .collect();
        for hash in hashes {
            if let Some(existing) = target.get(&hash).map_err(|message| TransferError::Bucket {
                operation: "get",
                hash: hash.clone(),
                message,
            })? {
                if sha256_hex(&existing) == hash {
                    self.mark(&hash, EntryState::Copied);
                    continue;
                }
                return Err(TransferError::CorruptEntry {
                    side: "target",
                    hash,
                });
            }
            let Some(bytes) = source.get(&hash).map_err(|message| TransferError::Bucket {
                operation: "get",
                hash: hash.clone(),
                message,
            })?
            else {
                return Err(TransferError::SourceMissing { hash });
            };
            if sha256_hex(&bytes) != hash {
                return Err(TransferError::CorruptEntry {
                    side: "source",
                    hash,
                });
            }
            target
                .put(&hash, &bytes)
                .map_err(|message| TransferError::Bucket {
                    operation: "put",
                    hash: hash.clone(),
                    message,
                })?;
            self.mark(&hash, EntryState::Copied);
            self.journal.log.push(format!("copied:{hash}"));
        }
        Ok(())
    }

    fn mark(&mut self, hash: &str, state: EntryState) {
        for entry in &mut self.journal.plan.entries {
            if entry.hash == hash {
                entry.state = state;
            }
        }
    }

    /// Read-back verification: every plan entry is re-read from the
    /// target, its hash recomputed, and the export reconstructed
    /// from the target bytes is passed through [`verify_import`] —
    /// so manifest, binding chain and every object signature are
    /// verified against the bytes that actually crossed, not the
    /// bytes that were sent. Moves `Copying → Verifying` first when
    /// needed; a failed verification leaves the record in
    /// `Verifying`, where the pre-switch abort is still available.
    ///
    /// # Errors
    ///
    /// [`TransferError::CopyIncomplete`] when entries are pending;
    /// [`TransferError`] naming the corrupt entry or the export rule
    /// that failed otherwise.
    pub fn verify(
        &mut self,
        export: &HostedExport,
        target: &dyn BlobBucket,
        confirm_untrusted: bool,
    ) -> Result<(), TransferError> {
        let pending = self.journal.plan.pending_count();
        if pending > 0 {
            return Err(TransferError::CopyIncomplete { pending });
        }
        if self.record.phase == MigrationPhase::Copying {
            self.apply(&MigrationEvent::BeginVerify)?;
        }
        let mut objects = Vec::new();
        let mut blobs = Vec::new();
        for object in &export.objects {
            let bytes = read_verified(target, &object.hash)?;
            let mut rebuilt = object.clone();
            rebuilt.bytes = bytes;
            objects.push(rebuilt);
        }
        for blob in &export.blobs {
            let bytes = read_verified(target, &blob.hash)?;
            blobs.push(ExportBlob {
                hash: blob.hash.clone(),
                bytes,
            });
        }
        let rebuilt = HostedExport {
            manifest: export.manifest.clone(),
            objects,
            blobs,
            binding: export.binding.clone(),
            rebind_records: export.rebind_records.clone(),
            service_public_key: export.service_public_key.clone(),
        };
        verify_import(&rebuilt, confirm_untrusted)?;
        for entry in &mut self.journal.plan.entries {
            entry.state = EntryState::Verified;
        }
        self.journal.verified = true;
        self.journal.log.push("verified".to_string());
        Ok(())
    }

    /// The CAS switch: `Verifying → Switched`. Refused unless
    /// read-back verification passed in this journal; the machine
    /// additionally refuses without the freeze acknowledgement.
    ///
    /// # Errors
    ///
    /// [`TransferError::SwitchRequiresVerification`] or the
    /// machine's refusal.
    pub fn switch(&mut self) -> Result<(), TransferError> {
        if !self.journal.verified {
            return Err(TransferError::SwitchRequiresVerification);
        }
        self.apply(&MigrationEvent::CasSwitch)?;
        self.journal.log.push("switched".to_string());
        Ok(())
    }

    /// Pre-switch abort: the machine CASes back to the source and
    /// rotates the fencing token; the coordinator then discards the
    /// target copies named in the journal and resets the journal so
    /// a later attempt starts clean. Post-switch, the machine
    /// refuses ([`MigrationError::SwitchIsIrreversible`]) and the
    /// target is left exactly as it stands — recovery is forward.
    ///
    /// # Errors
    ///
    /// The machine's refusal, or a bucket error discarding the
    /// target (the record has already aborted at that point; the
    /// error names the entry whose discard failed).
    pub fn abort(
        &mut self,
        target: &mut dyn BlobBucket,
        reason: &str,
        new_fencing_token: &str,
    ) -> Result<(), TransferError> {
        self.apply(&MigrationEvent::Abort {
            reason: reason.to_string(),
            new_fencing_token: new_fencing_token.to_string(),
        })?;
        let copied: Vec<String> = self
            .journal
            .plan
            .entries
            .iter()
            .filter(|entry| entry.state != EntryState::Pending)
            .map(|entry| entry.hash.clone())
            .collect();
        for hash in copied {
            target
                .delete(&hash)
                .map_err(|message| TransferError::Bucket {
                    operation: "delete",
                    hash,
                    message,
                })?;
        }
        for entry in &mut self.journal.plan.entries {
            entry.state = EntryState::Pending;
        }
        self.journal.verified = false;
        self.journal.log.push(format!("aborted:{reason}"));
        Ok(())
    }

    /// Safe source cleanup, after the switch: moves `Switched →
    /// Cleaning` when needed, then deletes the migrated store's
    /// entries from the source bucket — except blobs the fresh
    /// jurisdiction-local reachability scan still reaches from other
    /// stores, which are retained and named. The proof must name
    /// the fresh scan's version and satisfied retention; any other
    /// combination deletes nothing.
    ///
    /// # Errors
    ///
    /// [`TransferError`] naming the failed gate or bucket error.
    pub fn cleanup(
        &mut self,
        source: &mut dyn BlobBucket,
        fresh_scan: &ReachabilityScan,
        proof: &CleanupProof,
    ) -> Result<CleanupReport, TransferError> {
        match self.record.phase {
            MigrationPhase::Switched => self.apply(&MigrationEvent::BeginCleanup)?,
            MigrationPhase::Cleaning => {}
            phase => return Err(TransferError::CleanupBeforeSwitch { phase }),
        }
        if proof.scan_version != fresh_scan.version {
            return Err(TransferError::StaleReachabilityProof {
                proof: proof.scan_version,
                fresh: fresh_scan.version,
            });
        }
        if !proof.retention_satisfied {
            return Err(TransferError::RetentionNotSatisfied);
        }
        let mut deleted = Vec::new();
        let mut retained_shared = Vec::new();
        let entries = self.journal.plan.entries.clone();
        for entry in &entries {
            if entry.kind == EntryKind::Blob
                && fresh_scan
                    .reachable_from_other_stores
                    .contains(&entry.hash)
            {
                retained_shared.push(entry.hash.clone());
                continue;
            }
            source
                .delete(&entry.hash)
                .map_err(|message| TransferError::Bucket {
                    operation: "delete",
                    hash: entry.hash.clone(),
                    message,
                })?;
            deleted.push(entry.hash.clone());
        }
        deleted.sort();
        retained_shared.sort();
        self.journal.log.push(format!(
            "cleanup:deleted={},retained={}",
            deleted.len(),
            retained_shared.len()
        ));
        Ok(CleanupReport {
            deleted,
            retained_shared,
        })
    }

    /// Retire the record after cleanup and required retention:
    /// `Cleaning → Retired`, via the machine.
    ///
    /// # Errors
    ///
    /// The machine's refusal.
    pub fn retire(&mut self) -> Result<(), TransferError> {
        self.apply(&MigrationEvent::Retire)
    }
}

fn read_verified(bucket: &dyn BlobBucket, hash: &str) -> Result<Vec<u8>, TransferError> {
    let Some(bytes) = bucket.get(hash).map_err(|message| TransferError::Bucket {
        operation: "get",
        hash: hash.to_string(),
        message,
    })?
    else {
        return Err(TransferError::CorruptEntry {
            side: "target",
            hash: hash.to_string(),
        });
    };
    if sha256_hex(&bytes) != hash {
        return Err(TransferError::CorruptEntry {
            side: "target",
            hash: hash.to_string(),
        });
    }
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::hosted_binding::HostedBindingService;
    use crate::hosted_export::{ExportBlob, ExportObject, build_export};
    use ed25519_dalek::SigningKey;
    use std::collections::HashMap;

    const SUB: &str = "google-subject-1";
    const CREATED: &str = "2026-11-13T00:00:00Z";
    const MIGRATION_ID: &str = "0123456789abcdef0123456789abcdef";
    const TOKEN: &str = "fedcba9876543210fedcba9876543210";
    const NEXT_TOKEN: &str = "00000000000000000000000000000000";
    const WATERMARK: &str = "freeze-watermark-epoch-7";

    #[derive(Default)]
    struct MemoryBucket {
        entries: HashMap<String, Vec<u8>>,
        put_counts: HashMap<String, usize>,
        fail_after_puts: Option<usize>,
        puts: usize,
    }

    impl BlobBucket for MemoryBucket {
        fn get(&self, hash: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.entries.get(hash).cloned())
        }

        fn put(&mut self, hash: &str, bytes: &[u8]) -> Result<(), String> {
            if let Some(limit) = self.fail_after_puts {
                if self.puts >= limit {
                    return Err("injected put failure".to_string());
                }
            }
            self.puts += 1;
            *self.put_counts.entry(hash.to_string()).or_insert(0) += 1;
            self.entries.insert(hash.to_string(), bytes.to_vec());
            Ok(())
        }

        fn delete(&mut self, hash: &str) -> Result<(), String> {
            self.entries.remove(hash);
            Ok(())
        }
    }

    fn root_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn service() -> HostedBindingService {
        HostedBindingService::with_seed("hosted.example", [9u8; 32])
    }

    fn panda_key(key: &SigningKey) -> p2panda_core::SigningKey {
        p2panda_core::SigningKey::from_bytes(&key.to_bytes())
    }

    fn ext(kind: &str, document: Option<crate::documents::DocumentRef>) -> crate::chain::ChainExt {
        crate::chain::ChainExt {
            kind: kind.to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document,
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

    fn envelope(
        key: &SigningKey,
        ext: crate::chain::ChainExt,
    ) -> crate::chain::Event {
        crate::chain::sign(&panda_key(key), 0, None, None, ext).unwrap()
    }

    /// An export with one message object (no blobs) and one document
    /// object citing one blob — the one-account runbook fixture.
    fn sample_export() -> HostedExport {
        let mut svc = service();
        svc.signup_account(SUB, None, "free").unwrap();
        let binding = svc
            .complete_browser_ceremony(SUB, &root_key(21), None)
            .unwrap();
        let content = b"blob payload";
        let blob = ExportBlob::new(content.to_vec());
        let document = crate::documents::DocumentRef::for_bytes("text/plain", content).unwrap();
        let doc_object = ExportObject::from_envelope(
            envelope(&root_key(21), ext("document.put", Some(document))).bytes,
            vec![blob.hash.clone()],
        )
        .unwrap();
        let msg_object = ExportObject::from_envelope(
            envelope(&root_key(21), ext("message", None)).bytes,
            Vec::new(),
        )
        .unwrap();
        build_export(
            &svc,
            &binding,
            Vec::new(),
            vec![doc_object, msg_object],
            vec![blob],
            Some(&root_key(21)),
            "export-1",
            CREATED,
        )
    }

    fn frozen_record() -> MigrationRecord {
        let stable = MigrationRecord::stable("us").unwrap();
        let ctx = TransitionContext {
            expected_epoch: stable.epoch,
            fencing_token: &stable.fencing_token,
        };
        let pending = stable
            .transition(
                &MigrationEvent::Begin {
                    target: "eu".to_string(),
                    migration_id: MIGRATION_ID.to_string(),
                    fencing_token: TOKEN.to_string(),
                },
                &ctx,
            )
            .unwrap();
        let ctx = TransitionContext {
            expected_epoch: pending.epoch,
            fencing_token: &pending.fencing_token,
        };
        pending
            .transition(
                &MigrationEvent::AcknowledgeFreeze {
                    watermark: WATERMARK.to_string(),
                },
                &ctx,
            )
            .unwrap()
    }

    fn coordinator(export: &HostedExport) -> TransferCoordinator {
        let record = frozen_record();
        let plan = TransferPlan::from_export(&record, export).unwrap();
        TransferCoordinator::new(record, plan).unwrap()
    }

    fn seeded_source(export: &HostedExport) -> MemoryBucket {
        let mut bucket = MemoryBucket::default();
        for object in &export.objects {
            bucket.put(&object.hash, &object.bytes).unwrap();
        }
        for blob in &export.blobs {
            bucket.put(&blob.hash, &blob.bytes).unwrap();
        }
        bucket
    }

    fn scan(version: u64, reachable: &[&str]) -> ReachabilityScan {
        ReachabilityScan {
            version,
            reachable_from_other_stores: reachable.iter().map(|hash| hash.to_string()).collect(),
        }
    }

    #[test]
    fn one_account_runbook_end_to_end() {
        let export = sample_export();
        let mut source = seeded_source(&export);
        let mut target = MemoryBucket::default();
        let mut coord = coordinator(&export);
        assert_eq!(coord.resume_action(), ResumeAction::BeginCopy);

        coord.copy_all(&source, &mut target).unwrap();
        assert_eq!(coord.journal.plan.pending_count(), 0);
        // The journal survives a serde round-trip mid-flight: the
        // durable record is the resume mechanism.
        let restored: TransferJournal =
            serde_json::from_str(&serde_json::to_string(&coord.journal).unwrap()).unwrap();
        assert_eq!(restored, coord.journal);

        coord.verify(&export, &target, false).unwrap();
        assert_eq!(coord.resume_action(), ResumeAction::Switch);
        coord.switch().unwrap();
        assert_eq!(coord.record.phase, MigrationPhase::Switched);

        let report = coord
            .cleanup(
                &mut source,
                &scan(7, &[]),
                &CleanupProof {
                    scan_version: 7,
                    retention_satisfied: true,
                },
            )
            .unwrap();
        assert_eq!(report.deleted.len(), 3);
        assert!(report.retained_shared.is_empty());
        assert!(source.entries.is_empty());
        assert_eq!(target.entries.len(), 3);

        coord.retire().unwrap();
        assert_eq!(coord.resume_action(), ResumeAction::Done);
    }

    #[test]
    fn interrupted_copy_resumes_without_recopying() {
        let export = sample_export();
        let source = seeded_source(&export);
        let mut target = MemoryBucket {
            fail_after_puts: Some(1),
            ..Default::default()
        };
        let mut coord = coordinator(&export);
        // Fault injection: the target dies after the first put.
        assert!(matches!(
            coord.copy_all(&source, &mut target),
            Err(TransferError::Bucket {
                operation: "put",
                ..
            })
        ));
        assert_eq!(coord.journal.plan.pending_count(), 2);

        target.fail_after_puts = None;
        coord.copy_all(&source, &mut target).unwrap();
        assert_eq!(coord.journal.plan.pending_count(), 0);
        // No entry was put twice across the interruption and resume.
        for count in target.put_counts.values() {
            assert_eq!(*count, 1);
        }
        coord.verify(&export, &target, false).unwrap();
        coord.switch().unwrap();
    }

    #[test]
    fn corruption_blocks_switch_and_preswitch_abort_recovers() {
        let export = sample_export();
        let source = seeded_source(&export);
        let mut target = MemoryBucket::default();
        let mut coord = coordinator(&export);
        coord.copy_all(&source, &mut target).unwrap();

        // Corrupt one target entry after the copy.
        let blob_hash = export.blobs[0].hash.clone();
        target.entries.insert(blob_hash.clone(), b"tampered".to_vec());
        assert!(matches!(
            coord.verify(&export, &target, false),
            Err(TransferError::CorruptEntry {
                side: "target",
                ..
            })
        ));
        // The switch refuses on the unverified journal.
        assert_eq!(
            coord.switch(),
            Err(TransferError::SwitchRequiresVerification)
        );
        // Pre-switch abort: record returns to the source, the target
        // is discarded, and the source is untouched.
        coord
            .abort(&mut target, "target corruption", NEXT_TOKEN)
            .unwrap();
        assert_eq!(coord.record.phase, MigrationPhase::Stable);
        assert!(target.entries.is_empty());
        assert_eq!(source.entries.len(), 3);
        assert_eq!(coord.resume_action(), ResumeAction::Nothing);
    }

    #[test]
    fn post_switch_recovery_is_forward_only() {
        let export = sample_export();
        let mut source = seeded_source(&export);
        let mut target = MemoryBucket::default();
        let mut coord = coordinator(&export);
        coord.copy_all(&source, &mut target).unwrap();
        coord.verify(&export, &target, false).unwrap();
        coord.switch().unwrap();

        // Abort after the switch is refused by the machine, and the
        // resume pointer aims at cleanup, never back at the source.
        assert!(matches!(
            coord.abort(&mut target, "too late", NEXT_TOKEN),
            Err(TransferError::Migration(
                MigrationError::SwitchIsIrreversible
            ))
        ));
        assert_eq!(coord.resume_action(), ResumeAction::Cleanup);
        let report = coord
            .cleanup(
                &mut source,
                &scan(1, &[]),
                &CleanupProof {
                    scan_version: 1,
                    retention_satisfied: true,
                },
            )
            .unwrap();
        assert_eq!(report.deleted.len(), 3);
    }

    #[test]
    fn signature_preservation_is_verified_on_target_bytes() {
        let export = sample_export();
        let source = seeded_source(&export);
        let mut target = MemoryBucket::default();
        let mut coord = coordinator(&export);
        coord.copy_all(&source, &mut target).unwrap();
        // Object bytes in the target are byte-identical to the
        // signed source bytes — nothing was re-encoded or re-signed.
        for object in &export.objects {
            assert_eq!(target.entries[&object.hash], object.bytes);
        }
        // verify() re-runs full import verification (manifest,
        // binding chain, object signatures) on the target bytes.
        coord.verify(&export, &target, false).unwrap();
        assert!(coord.journal.verified);

        // A plan built over tampered object bytes refuses at plan
        // time: the hash no longer names the signed bytes.
        let mut tampered = export.clone();
        tampered.objects[0].bytes = b"forged payload".to_vec();
        let record = frozen_record();
        assert_eq!(
            TransferPlan::from_export(&record, &tampered),
            Err(TransferError::Export(ExportError::ObjectHashMismatch))
        );
    }

    #[test]
    fn shared_blob_is_retained_and_stale_or_unretained_proofs_delete_nothing() {
        let export = sample_export();
        let mut source = seeded_source(&export);
        let mut target = MemoryBucket::default();
        let mut coord = coordinator(&export);
        coord.copy_all(&source, &mut target).unwrap();
        coord.verify(&export, &target, false).unwrap();
        coord.switch().unwrap();

        let blob_hash = export.blobs[0].hash.clone();
        // Stale proof: deletes nothing.
        assert!(matches!(
            coord.cleanup(
                &mut source,
                &scan(9, &[]),
                &CleanupProof {
                    scan_version: 8,
                    retention_satisfied: true,
                },
            ),
            Err(TransferError::StaleReachabilityProof { proof: 8, fresh: 9 })
        ));
        assert_eq!(source.entries.len(), 3);
        // Retention unproven: deletes nothing.
        assert_eq!(
            coord.cleanup(
                &mut source,
                &scan(9, &[]),
                &CleanupProof {
                    scan_version: 9,
                    retention_satisfied: false,
                },
            ),
            Err(TransferError::RetentionNotSatisfied)
        );
        assert_eq!(source.entries.len(), 3);

        // The blob is still reached by another store in the
        // jurisdiction: it is retained, the objects are deleted.
        let report = coord
            .cleanup(
                &mut source,
                &scan(9, &[blob_hash.as_str()]),
                &CleanupProof {
                    scan_version: 9,
                    retention_satisfied: true,
                },
            )
            .unwrap();
        assert_eq!(report.retained_shared, vec![blob_hash.clone()]);
        assert_eq!(report.deleted.len(), 2);
        assert!(source.entries.contains_key(&blob_hash));
        assert_eq!(source.entries.len(), 1);
    }

    #[test]
    fn plan_refuses_missing_and_unreferenced_blobs() {
        let record = frozen_record();
        // Cited but not carried.
        let mut missing = sample_export();
        missing.blobs.clear();
        // Keep the manifest counts honest so only the reachability
        // rule can fail; plan derivation does not consult counts.
        assert!(matches!(
            TransferPlan::from_export(&record, &missing),
            Err(TransferError::MissingBlob { .. })
        ));
        // Carried but not cited.
        let mut unreferenced = sample_export();
        unreferenced
            .blobs
            .push(ExportBlob::new(b"orphan".to_vec()));
        assert!(matches!(
            TransferPlan::from_export(&record, &unreferenced),
            Err(TransferError::UnreferencedBlob { .. })
        ));
    }
}
