//! B07: the first-class conflict-report artifact (#10248, statements
//! S1.34-S1.38; Part B "Wire/data model", `ConflictReport`).
//!
//! A sync conflict's first output is a conflict report, and no merge
//! or resolution comes before it. Every version reaches this store
//! through the B32 ingest gate, so it is signed, bound to its signer
//! and authorized before anything here looks at it. The store keeps
//! each object's versions as a DAG and its *heads* (versions no
//! other stored version names as a parent). One head is a converged
//! object; two or more heads are a divergence.
//!
//! The rules, in order:
//!
//! 1. **Report first.** When applying a version leaves an object with
//!    more than one head, the store builds a [`ConflictReport`], keeps
//!    it addressable by its [`ReportId`] (a domain-separated SHA-256 of
//!    its canonical bytes) and appends `ReportCreated` to the audit log
//!    in the same step. An object has more than one head exactly when
//!    it has an open report, and that report names exactly its heads.
//! 2. **No silent resolution.** A version that joins two or more of a
//!    conflicted object's heads is a resolution. It is refused unless
//!    it names the object's open report in its signed header; a header
//!    that names any other report, or names one without joining heads,
//!    is refused too. Resolution is therefore always a new signed
//!    event that comes after the report it resolves.
//! 3. **Reports are immutable.** When a conflicted object's heads
//!    change but stay plural, a new report supersedes the old one,
//!    whose status becomes `Superseded`; when they collapse to one, the
//!    open report becomes `Resolved` by that event.
//! 4. **Unrelated sync continues.** Every rule is per object: a
//!    conflict on one object never refuses, holds or changes a version
//!    of another.
//!
//! A report identifies the conflicting versions (event ids, parents
//! and claimed issue times), their signers and sources (bound identity,
//! verifying key, ingress path, the peer label the caller gave), the
//! disputed data (each version's body digest and length, plus the
//! common base they diverged from), the scopes involved, the store's
//! policy digest (signer table, freshness window and retention), when
//! it was detected and when its evidence may expire. Retention
//! constrains evidence; it never removes an open report.
//!
//! A version's payload is a signed header, encoded as
//!
//! ```text
//! "id-sync/b07-version/v1"
//! object_id               32 bytes
//! resolves                0x00, or 0x01 followed by a 32-byte report id
//! parent_count            u32, big-endian
//! parents                 parent_count x 32 bytes, strictly ascending
//! body                    the remaining bytes
//! ```
//!
//! Versions whose parents are not yet stored are held and applied, in
//! the same way, when the parents arrive.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::b32_ingest_gate::{
    ED25519_PUBLIC_KEY_LEN, Envelope, EventId, FreshnessWindow, IngestGate, IngressPath,
    LogicalTime, RejectReason, SignerAuthorization,
};

/// The id of a synced object.
pub type ObjectId = [u8; 32];
/// The address of a conflict report: SHA-256 of its canonical bytes.
pub type ReportId = [u8; 32];
/// A SHA-256 digest.
pub type Digest32 = [u8; 32];

/// Domain prefix of a version payload.
pub const VERSION_DOMAIN: &[u8] = b"id-sync/b07-version/v1";
const REPORT_DOMAIN: &[u8] = b"id-sync/b07-conflict-report/v1";
const POLICY_DOMAIN: &[u8] = b"id-sync/b07-store-policy/v1";

fn sha256(bytes: &[u8]) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

fn push_ids(out: &mut Vec<u8>, ids: &[[u8; 32]]) {
    out.extend_from_slice(&(ids.len() as u64).to_be_bytes());
    for id in ids {
        out.extend_from_slice(id);
    }
}

fn push_optional(out: &mut Vec<u8>, id: Option<&[u8; 32]>) {
    match id {
        None => out.push(0),
        Some(id) => {
            out.push(1);
            out.extend_from_slice(id);
        }
    }
}

const fn path_tag(path: IngressPath) -> u8 {
    match path {
        IngressPath::Local => 0,
        IngressPath::Loopback => 1,
        IngressPath::Internal => 2,
        IngressPath::Hosted => 3,
        IngressPath::Remote => 4,
    }
}

/// The signed header a version's payload carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionHeader {
    /// The object this version belongs to.
    pub object_id: ObjectId,
    /// The open report this version resolves, if it is a resolution.
    pub resolves: Option<ReportId>,
    /// The versions this one follows, strictly ascending.
    pub parents: Vec<EventId>,
    /// The version's data.
    pub body: Vec<u8>,
}

impl VersionHeader {
    /// The canonical payload bytes. Parents are written sorted and
    /// without duplicates, so any header encodes to a decodable form.
    pub fn encode(&self) -> Vec<u8> {
        let parents: BTreeSet<&EventId> = self.parents.iter().collect();
        let mut out = VERSION_DOMAIN.to_vec();
        out.extend_from_slice(&self.object_id);
        push_optional(&mut out, self.resolves.as_ref());
        let count = u32::try_from(parents.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&count.to_be_bytes());
        for parent in parents {
            out.extend_from_slice(parent);
        }
        out.extend_from_slice(&self.body);
        out
    }

    /// Parse a payload. `None` for a wrong domain, a truncated field,
    /// an unknown `resolves` tag, or parents that are not strictly
    /// ascending.
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let rest = payload.strip_prefix(VERSION_DOMAIN)?;
        let (object_id, rest) = take32(rest)?;
        let (&tag, rest) = rest.split_first()?;
        let (resolves, rest) = match tag {
            0 => (None, rest),
            1 => {
                let (id, rest) = take32(rest)?;
                (Some(id), rest)
            }
            _ => return None,
        };
        let (count, rest) = rest.split_first_chunk::<4>()?;
        let count = usize::try_from(u32::from_be_bytes(*count)).ok()?;
        let parent_bytes = count.checked_mul(32)?;
        if rest.len() < parent_bytes {
            return None;
        }
        let (parent_bytes, body) = rest.split_at(parent_bytes);
        let parents: Vec<EventId> = parent_bytes
            .chunks_exact(32)
            .map(|chunk| chunk.try_into().unwrap_or([0; 32]))
            .collect();
        if parents.windows(2).any(|pair| pair[0] >= pair[1]) {
            return None;
        }
        Some(Self {
            object_id,
            resolves,
            parents,
            body: body.to_vec(),
        })
    }
}

fn take32(bytes: &[u8]) -> Option<([u8; 32], &[u8])> {
    let (head, rest) = bytes.split_first_chunk::<32>()?;
    Some((*head, rest))
}

/// One version as the store recorded it: what was signed, who signed
/// it and how it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRef {
    /// The version's event id.
    pub event_id: EventId,
    /// The object it belongs to.
    pub object_id: ObjectId,
    /// Its parents, ascending.
    pub parents: Vec<EventId>,
    /// The report it names as resolved, if any.
    pub resolves: Option<ReportId>,
    /// The identity the gate bound the signing key to.
    pub signer: String,
    /// The key that signed it.
    pub verifying_key: [u8; ED25519_PUBLIC_KEY_LEN],
    /// The ingress path it arrived by.
    pub path: IngressPath,
    /// The peer or producer label the caller supplied.
    pub source: String,
    /// The scope it was admitted into.
    pub scope: String,
    /// The signer's claimed issue time (a claim, not proof).
    pub issued_at: LogicalTime,
    /// SHA-256 of the body.
    pub body_digest: Digest32,
    /// The body's length in bytes.
    pub body_len: u64,
}

impl VersionRef {
    fn push_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.event_id);
        out.extend_from_slice(&self.object_id);
        push_ids(out, &self.parents);
        push_optional(out, self.resolves.as_ref());
        push_field(out, self.signer.as_bytes());
        out.extend_from_slice(&self.verifying_key);
        out.push(path_tag(self.path));
        push_field(out, self.source.as_bytes());
        push_field(out, self.scope.as_bytes());
        out.extend_from_slice(&self.issued_at.to_be_bytes());
        out.extend_from_slice(&self.body_digest);
        out.extend_from_slice(&self.body_len.to_be_bytes());
    }
}

/// The durable record of one divergence, made before any resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictReport {
    /// The diverged object.
    pub object_id: ObjectId,
    /// The conflicting versions (the object's heads), by event id.
    pub versions: Vec<VersionRef>,
    /// The latest versions every conflicting version descends from:
    /// the base the dispute diverged from. Empty when they share none.
    pub base: Vec<EventId>,
    /// The scopes the conflicting versions were admitted into.
    pub scopes: Vec<String>,
    /// The store policy in force (signers, freshness, retention).
    pub policy_digest: Digest32,
    /// When the store detected the divergence.
    pub detected_at: LogicalTime,
    /// When a closed report's evidence may be dropped.
    pub expires_at: LogicalTime,
    /// The report this one replaces, if the conflict changed shape.
    pub supersedes: Option<ReportId>,
}

impl ConflictReport {
    /// The canonical bytes the report's address is computed over.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = REPORT_DOMAIN.to_vec();
        out.extend_from_slice(&self.object_id);
        out.extend_from_slice(&(self.versions.len() as u64).to_be_bytes());
        for version in &self.versions {
            version.push_canonical(&mut out);
        }
        push_ids(&mut out, &self.base);
        out.extend_from_slice(&(self.scopes.len() as u64).to_be_bytes());
        for scope in &self.scopes {
            push_field(&mut out, scope.as_bytes());
        }
        out.extend_from_slice(&self.policy_digest);
        out.extend_from_slice(&self.detected_at.to_be_bytes());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        push_optional(&mut out, self.supersedes.as_ref());
        out
    }

    /// The report's address.
    pub fn report_id(&self) -> ReportId {
        sha256(&self.canonical_bytes())
    }

    /// What a resolution must name as its parents: every conflicting
    /// version. A resolution is a new signed version with these parents
    /// whose header names this report.
    pub fn resolution_parents(&self) -> Vec<EventId> {
        self.versions.iter().map(|v| v.event_id).collect()
    }
}

/// Where a report stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStatus {
    /// The divergence is unresolved.
    Open,
    /// A signed version naming this report joined the heads into one.
    Resolved {
        /// The resolving event.
        by: EventId,
        /// When it was applied.
        at: LogicalTime,
    },
    /// The conflict changed shape and a newer report replaced this one.
    Superseded {
        /// The replacing report.
        by: ReportId,
    },
}

/// A report and its current status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReport {
    /// The immutable report.
    pub report: ConflictReport,
    /// Its status, which changes only by later events.
    pub status: ReportStatus,
}

/// One append-only audit entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEntry {
    /// A version was applied to its object.
    Recorded {
        /// The object.
        object_id: ObjectId,
        /// The version.
        event_id: EventId,
    },
    /// A version was held until its parents arrive.
    Held {
        /// The object.
        object_id: ObjectId,
        /// The version.
        event_id: EventId,
    },
    /// A held version was refused when its parents arrived.
    Refused {
        /// The object.
        object_id: ObjectId,
        /// The version.
        event_id: EventId,
    },
    /// A conflict report was created.
    ReportCreated {
        /// The object.
        object_id: ObjectId,
        /// The new report.
        report_id: ReportId,
        /// The report it superseded, if any.
        supersedes: Option<ReportId>,
    },
    /// An open report was resolved.
    Resolved {
        /// The object.
        object_id: ObjectId,
        /// The resolved report.
        report_id: ReportId,
        /// The resolving event.
        by: EventId,
    },
}

/// What happened to an admitted version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    /// Applied; the object has one head and no open report.
    Applied,
    /// Applied, and the object is (still) diverged: this is the report.
    Conflict(ReportId),
    /// Applied as the resolution of this report.
    Resolved(ReportId),
    /// Admitted and held until its parents are stored.
    Held,
}

/// Why a version was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// The ingest gate refused the envelope.
    Rejected(RejectReason),
    /// The payload is not a canonical version header, or names itself
    /// as a parent.
    MalformedVersion,
    /// A parent belongs to a different object.
    ForeignParent(EventId),
    /// The version joins heads of a conflicted object without naming
    /// its open report.
    ResolutionRequired {
        /// The report a resolution must name.
        open: ReportId,
    },
    /// The version names a report it does not resolve: not the
    /// object's open report, or named without joining its heads.
    NotAResolution {
        /// The report the version named.
        named: ReportId,
    },
}

/// How long closed reports keep their evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Seconds after detection a report's evidence may be dropped once
    /// the report is closed.
    pub retain_for: LogicalTime,
}

/// Digest of the store policy: the signer table (order-insensitive),
/// the freshness window and the retention policy.
pub fn policy_digest(
    signers: &[SignerAuthorization],
    freshness: FreshnessWindow,
    retention: RetentionPolicy,
) -> Digest32 {
    let mut sorted: Vec<&SignerAuthorization> = signers.iter().collect();
    sorted.sort_by(|a, b| {
        (a.identity.as_str(), a.verifying_key).cmp(&(b.identity.as_str(), b.verifying_key))
    });
    let mut out = POLICY_DOMAIN.to_vec();
    out.extend_from_slice(&(sorted.len() as u64).to_be_bytes());
    for signer in sorted {
        push_field(&mut out, signer.identity.as_bytes());
        out.extend_from_slice(&signer.verifying_key);
        out.extend_from_slice(&(signer.scopes.len() as u64).to_be_bytes());
        for scope in &signer.scopes {
            push_field(&mut out, scope.as_bytes());
        }
    }
    out.extend_from_slice(&freshness.max_age.to_be_bytes());
    out.extend_from_slice(&freshness.max_future_skew.to_be_bytes());
    out.extend_from_slice(&retention.retain_for.to_be_bytes());
    sha256(&out)
}

#[derive(Debug, Clone, Default)]
struct ObjectState {
    heads: BTreeSet<EventId>,
    open: Option<ReportId>,
}

/// A sync store that reports every divergence before any resolution.
#[derive(Debug, Clone)]
pub struct ConflictStore {
    gate: IngestGate,
    policy_digest: Digest32,
    retention: RetentionPolicy,
    versions: BTreeMap<EventId, VersionRef>,
    objects: BTreeMap<ObjectId, ObjectState>,
    held: BTreeMap<EventId, VersionRef>,
    reports: BTreeMap<ReportId, StoredReport>,
    audit: Vec<AuditEntry>,
}

impl ConflictStore {
    /// A store whose ingest gate has this signer table and window.
    pub fn new(
        signers: Vec<SignerAuthorization>,
        freshness: FreshnessWindow,
        retention: RetentionPolicy,
    ) -> Self {
        Self {
            policy_digest: policy_digest(&signers, freshness, retention),
            gate: IngestGate::new(signers, freshness),
            retention,
            versions: BTreeMap::new(),
            objects: BTreeMap::new(),
            held: BTreeMap::new(),
            reports: BTreeMap::new(),
            audit: Vec::new(),
        }
    }

    /// The ingest gate every version passes.
    pub const fn gate(&self) -> &IngestGate {
        &self.gate
    }

    /// The store policy digest reports record.
    pub const fn policy_digest(&self) -> Digest32 {
        self.policy_digest
    }

    /// The append-only audit log.
    pub fn audit(&self) -> &[AuditEntry] {
        &self.audit
    }

    /// A stored version.
    pub fn version(&self, event_id: &EventId) -> Option<&VersionRef> {
        self.versions.get(event_id)
    }

    /// Whether a version is held waiting for its parents.
    pub fn is_held(&self, event_id: &EventId) -> bool {
        self.held.contains_key(event_id)
    }

    /// An object's heads, ascending.
    pub fn heads(&self, object_id: &ObjectId) -> Vec<EventId> {
        self.objects
            .get(object_id)
            .map(|state| state.heads.iter().copied().collect())
            .unwrap_or_default()
    }

    /// An object's open report, if it is diverged.
    pub fn open_report(&self, object_id: &ObjectId) -> Option<ReportId> {
        self.objects.get(object_id).and_then(|state| state.open)
    }

    /// A report by its address.
    pub fn report(&self, report_id: &ReportId) -> Option<&StoredReport> {
        self.reports.get(report_id)
    }

    /// Every retained report, by address.
    pub fn reports(&self) -> impl Iterator<Item = (&ReportId, &StoredReport)> {
        self.reports.iter()
    }

    /// Admit one envelope through the gate and apply it to its object.
    /// `source` labels the peer or producer it came from.
    pub fn ingest(
        &mut self,
        path: IngressPath,
        envelope: &Envelope,
        source: &str,
        now: LogicalTime,
    ) -> Result<IngestOutcome, IngestError> {
        let admission = self
            .gate
            .admit(path, envelope, now)
            .map_err(IngestError::Rejected)?;
        let header =
            VersionHeader::decode(&envelope.payload).ok_or(IngestError::MalformedVersion)?;
        if header.parents.contains(&envelope.event_id) {
            return Err(IngestError::MalformedVersion);
        }
        let version = VersionRef {
            event_id: envelope.event_id,
            object_id: header.object_id,
            parents: header.parents,
            resolves: header.resolves,
            signer: admission.signer,
            verifying_key: envelope.verifying_key,
            path,
            source: source.to_owned(),
            scope: admission.scope,
            issued_at: envelope.issued_at,
            body_digest: sha256(&header.body),
            body_len: header.body.len() as u64,
        };
        self.check_parents(&version)?;
        if version
            .parents
            .iter()
            .any(|p| !self.versions.contains_key(p))
        {
            self.audit.push(AuditEntry::Held {
                object_id: version.object_id,
                event_id: version.event_id,
            });
            self.held.insert(version.event_id, version);
            return Ok(IngestOutcome::Held);
        }
        let outcome = self.apply(version, now)?;
        self.drain_held(now);
        Ok(outcome)
    }

    /// Drop closed reports whose evidence has expired. Open reports are
    /// never dropped. Returns how many were dropped.
    pub fn expire(&mut self, now: LogicalTime) -> usize {
        let before = self.reports.len();
        self.reports.retain(|_, stored| {
            stored.status == ReportStatus::Open || stored.report.expires_at > now
        });
        before - self.reports.len()
    }

    fn owner(&self, event_id: &EventId) -> Option<ObjectId> {
        self.versions
            .get(event_id)
            .or_else(|| self.held.get(event_id))
            .map(|v| v.object_id)
    }

    fn check_parents(&self, version: &VersionRef) -> Result<(), IngestError> {
        for parent in &version.parents {
            if let Some(owner) = self.owner(parent)
                && owner != version.object_id
            {
                return Err(IngestError::ForeignParent(*parent));
            }
        }
        Ok(())
    }

    fn apply(
        &mut self,
        version: VersionRef,
        now: LogicalTime,
    ) -> Result<IngestOutcome, IngestError> {
        let state = self
            .objects
            .get(&version.object_id)
            .cloned()
            .unwrap_or_default();
        let joined = version
            .parents
            .iter()
            .filter(|p| state.heads.contains(*p))
            .count();
        match (state.open, version.resolves) {
            (Some(open), Some(named)) if named == open && joined >= 2 => {}
            (Some(open), None) if joined >= 2 => {
                return Err(IngestError::ResolutionRequired { open });
            }
            (_, Some(named)) => return Err(IngestError::NotAResolution { named }),
            _ => {}
        }

        let object_id = version.object_id;
        let event_id = version.event_id;
        let mut heads = state.heads;
        for parent in &version.parents {
            heads.remove(parent);
        }
        heads.insert(event_id);
        self.versions.insert(event_id, version);
        self.audit.push(AuditEntry::Recorded {
            object_id,
            event_id,
        });

        let (open, outcome) = match (state.open, heads.len()) {
            (None, 1) => (None, IngestOutcome::Applied),
            (Some(open), 1) => {
                if let Some(stored) = self.reports.get_mut(&open) {
                    stored.status = ReportStatus::Resolved {
                        by: event_id,
                        at: now,
                    };
                }
                self.audit.push(AuditEntry::Resolved {
                    object_id,
                    report_id: open,
                    by: event_id,
                });
                (None, IngestOutcome::Resolved(open))
            }
            (previous, _) => {
                let report_id = self.create_report(object_id, &heads, previous, now);
                (Some(report_id), IngestOutcome::Conflict(report_id))
            }
        };
        self.objects.insert(object_id, ObjectState { heads, open });
        Ok(outcome)
    }

    fn create_report(
        &mut self,
        object_id: ObjectId,
        heads: &BTreeSet<EventId>,
        supersedes: Option<ReportId>,
        now: LogicalTime,
    ) -> ReportId {
        let versions: Vec<VersionRef> = heads
            .iter()
            .filter_map(|head| self.versions.get(head).cloned())
            .collect();
        let scopes: BTreeSet<String> = versions.iter().map(|v| v.scope.clone()).collect();
        let report = ConflictReport {
            object_id,
            versions,
            base: self.common_base(heads),
            scopes: scopes.into_iter().collect(),
            policy_digest: self.policy_digest,
            detected_at: now,
            expires_at: now.saturating_add(self.retention.retain_for),
            supersedes,
        };
        let report_id = report.report_id();
        if let Some(previous) = supersedes
            && let Some(stored) = self.reports.get_mut(&previous)
        {
            stored.status = ReportStatus::Superseded { by: report_id };
        }
        self.reports.insert(
            report_id,
            StoredReport {
                report,
                status: ReportStatus::Open,
            },
        );
        self.audit.push(AuditEntry::ReportCreated {
            object_id,
            report_id,
            supersedes,
        });
        report_id
    }

    /// Every stored version `event_id` descends from, itself included.
    fn ancestry(&self, event_id: EventId) -> BTreeSet<EventId> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![event_id];
        while let Some(next) = stack.pop() {
            if seen.insert(next)
                && let Some(version) = self.versions.get(&next)
            {
                stack.extend(version.parents.iter().copied());
            }
        }
        seen
    }

    /// The maximal versions every head descends from.
    fn common_base(&self, heads: &BTreeSet<EventId>) -> Vec<EventId> {
        let mut common: Option<BTreeSet<EventId>> = None;
        for head in heads {
            let ancestry = self.ancestry(*head);
            common = Some(match common {
                None => ancestry,
                Some(acc) => acc.intersection(&ancestry).copied().collect(),
            });
        }
        let common = common.unwrap_or_default();
        common
            .iter()
            .filter(|candidate| {
                !common
                    .iter()
                    .any(|other| other != *candidate && self.ancestry(*other).contains(*candidate))
            })
            .copied()
            .collect()
    }

    fn drain_held(&mut self, now: LogicalTime) {
        loop {
            let ready = self.held.iter().find_map(|(id, version)| {
                version
                    .parents
                    .iter()
                    .all(|p| self.versions.contains_key(p))
                    .then_some(*id)
            });
            let Some(id) = ready else { return };
            let Some(version) = self.held.remove(&id) else {
                return;
            };
            let object_id = version.object_id;
            let applied = self
                .check_parents(&version)
                .and_then(|()| self.apply(version, now));
            if applied.is_err() {
                self.audit.push(AuditEntry::Refused {
                    object_id,
                    event_id: id,
                });
            }
        }
    }
}
