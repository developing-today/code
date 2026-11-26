//! The counted/uncounted projection of a view (spec §4.1 and §4.3; the
//! predicate half of §9 item 4, for #3789).
//!
//! [`project`] is the §4.1 predicate as a pure function of the view's
//! whole input tuple: the view configuration `C`, the selected set
//! version `S`, the claim frontier `F` and the pinned resolution `R`. The
//! caller supplies `R` as a [`Snapshot`] (every event the evaluation can
//! touch, its causal dependencies, and which events apply under `R`), the
//! owner's set versions already expanded (see [`OwnerVersion`]), and the
//! candidate claims. Retrieval of those candidates from the store is the
//! `AuthorEvents` index of §9 item 3 and is not part of this module.
//!
//! ```text
//! counted(C, S, F, R, c) ⇔ applied(R, c)                       // 1
//!                        ∧ past(F, c)                          // 2
//!                        ∧ ¬rescinded(S, c)                    // 3
//!                        ∧ ( (in_scope(C, c)
//!                             ∧ (admitted_at(S, c) ∨ retro(S, c)))   // 4
//!                          ∨ c ∈ C.exceptions )                // 5
//! ```
//!
//! A claim that does not apply under `R` is in neither collection (§4.3);
//! the live validity query of §4.6 is a separate operation.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use p2panda_core::Hash;

use id_core::waiting::AuthorKey;

use super::view::{ExpandedWriters, GroupMembers, ProjectionError, expand_set};
use super::{Entry, Excludes, Rescind, WriterSet};

/// One event of the pinned snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEvent {
    /// The event's ID.
    pub id: Hash,
    /// Its signer.
    pub author: AuthorKey,
    /// The header's `kind`.
    pub kind: String,
    /// The subject the claim is about, when it has one.
    pub subject: Option<String>,
    /// Its direct causal dependencies: the backlink and the signed `after`.
    pub deps: Vec<Hash>,
}

/// The resolution-pinned snapshot `R`: the events an evaluation may touch
/// and which of them apply under `R`.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    events: BTreeMap<Hash, SnapshotEvent>,
    applied: BTreeSet<Hash>,
}

impl Snapshot {
    /// An empty snapshot.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an event, stating whether it applies under the pinned `R`.
    ///
    /// # Errors
    ///
    /// [`ProjectionError::DuplicateEvent`] when the ID is already present.
    pub fn insert(&mut self, event: SnapshotEvent, applied: bool) -> Result<(), ProjectionError> {
        if self.events.contains_key(&event.id) {
            return Err(ProjectionError::DuplicateEvent(event.id));
        }
        if applied {
            self.applied.insert(event.id);
        }
        self.events.insert(event.id, event);
        Ok(())
    }

    fn get(&self, id: &Hash) -> Result<&SnapshotEvent, ProjectionError> {
        self.events
            .get(id)
            .ok_or(ProjectionError::MissingEvent(*id))
    }

    /// `applied(R, id)`.
    #[must_use]
    pub fn applied(&self, id: &Hash) -> bool {
        self.applied.contains(id)
    }

    /// The strict causal past of `of`: every event reachable through its
    /// dependencies, not `of` itself.
    fn causal_past(&self, of: &Hash) -> Result<HashSet<Hash>, ProjectionError> {
        let mut seen = HashSet::new();
        let mut stack: Vec<Hash> = self.get(of)?.deps.clone();
        while let Some(id) = stack.pop() {
            if seen.insert(id) {
                stack.extend(self.get(&id)?.deps.iter().copied());
            }
        }
        Ok(seen)
    }
}

/// What one version of the owner's chain does to the set it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionBody {
    /// An `idset.writer` version: its prospective expansion and the
    /// expansion narrowed to `include_history` entries (§3.1).
    Admission {
        /// `expand(v)`.
        expand: ExpandedWriters,
        /// `expand_retro(v)`.
        retro: ExpandedWriters,
    },
    /// An `idset.rescind` version (§3.3). It admits nobody.
    Rescind {
        /// The version from whose position the erasure applies.
        from_version: u64,
        /// `expand({ entries: [target], excludes: [] })`.
        target: ExpandedWriters,
        /// The optional `through` bound.
        through: Option<Hash>,
    },
}

/// One set event on the owner's chain, already expanded at its
/// publishing frontier under the pinned resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerVersion {
    /// The publishing event.
    pub event: Hash,
    /// Its sequence in the owner's chain.
    pub seq: u64,
    /// The set its body names.
    pub set_id: Hash,
    /// Admission or rescind.
    pub body: VersionBody,
}

impl OwnerVersion {
    /// An admission version from a decoded writer set, expanded at the
    /// publishing event's `frontier` under `resolution` (§D4, §4.5).
    ///
    /// # Errors
    ///
    /// [`ProjectionError::MissingGroupMembers`] when a group entry has no
    /// pinned membership snapshot.
    pub fn admission(
        event: Hash,
        set: &WriterSet,
        frontier: &[Hash],
        resolution: &Hash,
        groups: &[GroupMembers],
    ) -> Result<Self, ProjectionError> {
        Ok(Self {
            event,
            seq: set.version,
            set_id: set.set_id,
            body: VersionBody::Admission {
                expand: expand_set(set, frontier, resolution, groups, false)?,
                retro: expand_set(set, frontier, resolution, groups, true)?,
            },
        })
    }

    /// A rescind version from a decoded rescind at chain position `seq`.
    ///
    /// # Errors
    ///
    /// [`ProjectionError::MissingGroupMembers`] when a group target has no
    /// pinned membership snapshot.
    pub fn rescind(
        event: Hash,
        seq: u64,
        owner: AuthorKey,
        rescind: &Rescind,
        frontier: &[Hash],
        resolution: &Hash,
        groups: &[GroupMembers],
    ) -> Result<Self, ProjectionError> {
        // The target is entry-shaped: expand it as a one-entry set with no
        // exclusions, so `target: Public` admits every author's claims to
        // the erasure.
        let target_set = WriterSet {
            set_id: rescind.set_id,
            version: seq,
            owner,
            scope: None,
            entries: vec![rescind.target.clone()],
            excludes: Excludes(Vec::<Entry>::new()),
        };
        Ok(Self {
            event,
            seq,
            set_id: rescind.set_id,
            body: VersionBody::Rescind {
                from_version: rescind.from_version,
                target: expand_set(&target_set, frontier, resolution, groups, false)?,
                through: rescind.through,
            },
        })
    }
}

/// A scope over claim subjects or kinds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every value, including a claim with no subject.
    Any,
    /// Only these values.
    Only(BTreeSet<String>),
}

impl Scope {
    fn admits(&self, value: Option<&str>) -> bool {
        match self {
            Self::Any => true,
            Self::Only(values) => value.is_some_and(|v| values.contains(v)),
        }
    }
}

/// The view configuration and the selected inputs (§4.1's tuple).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// The view's ID.
    pub view_id: Hash,
    /// The owner-chain position of the configuration event.
    pub config_seq: u64,
    /// The configuration event at `config_seq`; it must apply under `R`.
    pub config_event: Hash,
    /// The owner whose chain carries the set versions.
    pub owner: AuthorKey,
    /// The selected set.
    pub set_id: Hash,
    /// The selected set version's chain position.
    pub set_seq: u64,
    /// The claim frontier `F`: no claim outside its causal closure counts.
    pub frontier: Vec<Hash>,
    /// The resolution identity `R` names.
    pub resolution: Hash,
    /// Subjects the view counts.
    pub subject_scope: Scope,
    /// Kinds the view counts.
    pub kind_scope: Scope,
    /// Claims counted one-off (§4.4).
    pub exceptions: BTreeSet<Hash>,
}

/// Which half of the scope a claim fell outside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeMismatch {
    /// The claim's subject is outside `subject_scope`.
    Subject,
    /// The claim's kind is outside `kind_scope`.
    Kind,
}

/// Why an applied claim is uncounted (§4.3). One variant per clause that
/// can fail after `applied`, plus discovery's never-admitted reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhyNotCounted {
    /// Clause 2: outside the view's frontier.
    AfterFrontier,
    /// Clause 3: erased by the rescind at this chain position.
    RescindedSince(u64),
    /// Clause 4: the version in force when the claim was written (this
    /// chain position) no longer admitted its author.
    AuthorRemoved(u64),
    /// Clause 4: the claim precedes its author's admission and no version
    /// carrying `include_history` imports it.
    HistoryNotImported,
    /// No version of the set ever admitted the author.
    AuthorNeverAdmitted,
    /// Clause 5: subject or kind outside the view's scope.
    OutOfScope(ScopeMismatch),
}

/// A counted claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The claim.
    pub claim: Hash,
    /// Its signer.
    pub author: AuthorKey,
    /// The chain position of the version that governs it (`gov`), or
    /// `None` when only a view exception counts it.
    pub governed_by: Option<u64>,
}

/// A stored, attributed claim with zero effect on the view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncountedHit {
    /// The claim.
    pub claim: Hash,
    /// Its signer.
    pub author: AuthorKey,
    /// True by construction: the snapshot holds only verified events.
    pub signature_ok: bool,
    /// Why it is not counted.
    pub reason: WhyNotCounted,
}

/// What a projection was computed from (§3.4): never just a number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// The view.
    pub view_id: Hash,
    /// Its configuration position.
    pub config_seq: u64,
    /// The selected set.
    pub set_id: Hash,
    /// The selected set version's chain position.
    pub set_seq: u64,
    /// The selected set version's event.
    pub set_head: Hash,
    /// The claim frontier.
    pub frontier: Vec<Hash>,
    /// The pinned resolution.
    pub resolution: Hash,
}

/// A view's counted and uncounted claims. The two never share a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    /// Counted claims, in claim-ID order.
    pub counted: Vec<Hit>,
    /// Applied but uncounted claims, in claim-ID order.
    pub uncounted: Vec<UncountedHit>,
    /// The input tuple.
    pub provenance: Provenance,
}

/// One admission version of `S`, with the events it had seen.
struct AdmissionVersion<'a> {
    seq: u64,
    event: Hash,
    expand: &'a ExpandedWriters,
    retro: &'a ExpandedWriters,
    past: HashSet<Hash>,
}

/// One rescind version of `S`, with the events it had seen and its bound.
struct RescindVersion<'a> {
    seq: u64,
    from_version: u64,
    target: &'a ExpandedWriters,
    past: HashSet<Hash>,
    bound: Option<HashSet<Hash>>,
}

/// `versions(S)`, split by kind, in chain order.
struct SetHistory<'a> {
    admissions: Vec<AdmissionVersion<'a>>,
    rescinds: Vec<RescindVersion<'a>>,
}

/// Project `view` over `candidates` (§4.1).
///
/// Named exceptions present in the snapshot are evaluated too, whether or
/// not they are candidates; an exception naming a claim the snapshot lacks
/// does not fire (§4.4).
///
/// # Errors
///
/// - [`ProjectionError::Undefined`] when the selected set version or the
///   configuration event does not apply under `R`: there is nothing to
///   project, and projecting a surviving branch would misattribute it.
/// - [`ProjectionError::InvalidWriterSet`] when a version's event is not
///   signed by the view's owner.
/// - [`ProjectionError::MissingEvent`] when a candidate, a frontier head,
///   a version or a causal dependency is not in the snapshot.
pub fn project(
    view: &View,
    versions: &[OwnerVersion],
    snapshot: &Snapshot,
    candidates: &[Hash],
) -> Result<Projection, ProjectionError> {
    if !snapshot.applied(&view.config_event) {
        return Err(ProjectionError::Undefined("view"));
    }
    // versions(S): applied owner-chain events through set_seq that name S.
    let mut selected = Vec::new();
    for version in versions {
        let event = snapshot.get(&version.event)?;
        if event.author != view.owner {
            return Err(ProjectionError::InvalidWriterSet(version.event));
        }
        let names_set = version.set_id == view.set_id;
        if names_set && version.seq <= view.set_seq && snapshot.applied(&version.event) {
            selected.push(version);
        }
    }
    let head = selected
        .iter()
        .find(|v| v.seq == view.set_seq && matches!(v.body, VersionBody::Admission { .. }))
        .ok_or(ProjectionError::Undefined("writer-set"))?
        .event;
    selected.sort_by_key(|v| v.seq);
    let history = set_history(&selected, snapshot)?;

    let mut frontier_past: HashSet<Hash> = HashSet::new();
    for head in &view.frontier {
        frontier_past.insert(*head);
        frontier_past.extend(snapshot.causal_past(head)?);
    }

    let mut claims: BTreeSet<Hash> = candidates.iter().copied().collect();
    claims.extend(
        view.exceptions
            .iter()
            .filter(|id| snapshot.events.contains_key(*id)),
    );

    let mut counted = Vec::new();
    let mut uncounted = Vec::new();
    for id in claims {
        let claim = snapshot.get(&id)?;
        // Clause 1: a claim not in force under R is in neither collection.
        if !snapshot.applied(&id) {
            continue;
        }
        match evaluate(view, &history, &frontier_past, snapshot, claim)? {
            Ok(governed_by) => counted.push(Hit {
                claim: id,
                author: claim.author,
                governed_by,
            }),
            Err(reason) => uncounted.push(UncountedHit {
                claim: id,
                author: claim.author,
                signature_ok: true,
                reason,
            }),
        }
    }
    Ok(Projection {
        counted,
        uncounted,
        provenance: Provenance {
            view_id: view.view_id,
            config_seq: view.config_seq,
            set_id: view.set_id,
            set_seq: view.set_seq,
            set_head: head,
            frontier: view.frontier.clone(),
            resolution: view.resolution,
        },
    })
}

/// Split the selected versions by kind and compute each one's causal past.
fn set_history<'a>(
    selected: &[&'a OwnerVersion],
    snapshot: &Snapshot,
) -> Result<SetHistory<'a>, ProjectionError> {
    let mut history = SetHistory {
        admissions: Vec::new(),
        rescinds: Vec::new(),
    };
    for version in selected {
        let past = snapshot.causal_past(&version.event)?;
        match &version.body {
            VersionBody::Admission { expand, retro } => {
                history.admissions.push(AdmissionVersion {
                    seq: version.seq,
                    event: version.event,
                    expand,
                    retro,
                    past,
                });
            }
            VersionBody::Rescind {
                from_version,
                target,
                through,
            } => {
                // `through` is the "up to here" bound: inclusive, like the
                // frontier's closure.
                let bound = through
                    .map(|t| {
                        snapshot.causal_past(&t).map(|mut past| {
                            past.insert(t);
                            past
                        })
                    })
                    .transpose()?;
                history.rescinds.push(RescindVersion {
                    seq: version.seq,
                    from_version: *from_version,
                    target,
                    past,
                    bound,
                });
            }
        }
    }
    Ok(history)
}

/// Clauses 2–5 for one applied claim: the governing version when it
/// counts, or the reason it does not.
fn evaluate(
    view: &View,
    history: &SetHistory<'_>,
    frontier_past: &HashSet<Hash>,
    snapshot: &Snapshot,
    claim: &SnapshotEvent,
) -> Result<Result<Option<u64>, WhyNotCounted>, ProjectionError> {
    // Clause 2.
    if !frontier_past.contains(&claim.id) {
        return Ok(Err(WhyNotCounted::AfterFrontier));
    }
    let author = &claim.author;
    let claim_past = snapshot.causal_past(&claim.id)?;

    // seen_by: the newest admission version in the claim's causal past.
    let seen_by = history
        .admissions
        .iter()
        .filter(|v| claim_past.contains(&v.event))
        .max_by_key(|v| v.seq);
    let admitted_at = seen_by.filter(|v| v.expand.contains(author));
    // retro: an admission version that saw the claim and imports its
    // author's history; gov takes the earliest.
    let retro = history
        .admissions
        .iter()
        .filter(|v| v.past.contains(&claim.id) && v.retro.contains(author))
        .min_by_key(|v| v.seq);
    let gov = admitted_at.or(retro).map(|v| v.seq);

    // Clause 3: a rescind of S that targets the author, saw the claim,
    // bounds it (when `through` is named) and starts at or before the
    // version governing it.
    if let Some(gov) = gov {
        let rescinded = history
            .rescinds
            .iter()
            .filter(|r| {
                r.target.contains(author)
                    && r.past.contains(&claim.id)
                    && r.bound.as_ref().is_none_or(|b| b.contains(&claim.id))
                    && r.from_version <= view.set_seq
                    && gov >= r.from_version
            })
            .map(|r| r.seq)
            .min();
        if let Some(seq) = rescinded {
            return Ok(Err(WhyNotCounted::RescindedSince(seq)));
        }
    }

    // Clause 5: an exception lifts scope and admission, nothing else.
    if view.exceptions.contains(&claim.id) {
        return Ok(Ok(gov));
    }
    if !view.subject_scope.admits(claim.subject.as_deref()) {
        return Ok(Err(WhyNotCounted::OutOfScope(ScopeMismatch::Subject)));
    }
    if !view.kind_scope.admits(Some(&claim.kind)) {
        return Ok(Err(WhyNotCounted::OutOfScope(ScopeMismatch::Kind)));
    }
    // Clause 4.
    if let Some(gov) = gov {
        return Ok(Ok(Some(gov)));
    }
    let ever_admitted = history
        .admissions
        .iter()
        .any(|v| v.expand.contains(author) || v.retro.contains(author));
    if !ever_admitted {
        return Ok(Err(WhyNotCounted::AuthorNeverAdmitted));
    }
    if let Some(in_force) = seen_by {
        let admitted_before = history
            .admissions
            .iter()
            .any(|v| v.seq < in_force.seq && v.expand.contains(author));
        if admitted_before {
            return Ok(Err(WhyNotCounted::AuthorRemoved(in_force.seq)));
        }
    }
    Ok(Err(WhyNotCounted::HistoryNotImported))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::unused_self)]
mod tests {
    use super::*;

    const OWNER: AuthorKey = [1; 32];
    const ALICE: AuthorKey = [2; 32];
    const BOB: AuthorKey = [3; 32];
    const STRANGER: AuthorKey = [9; 32];

    fn h(n: u8) -> Hash {
        Hash::from([n; 32])
    }

    fn set_id() -> Hash {
        h(200)
    }

    fn key(author: AuthorKey, include_history: bool) -> Entry {
        Entry::Key {
            author,
            include_history,
        }
    }

    /// A small world: events by id, each with an author and deps.
    struct World {
        snapshot: Snapshot,
        versions: Vec<OwnerVersion>,
    }

    impl World {
        fn new() -> Self {
            let mut snapshot = Snapshot::new();
            // The owner's view-configuration event.
            snapshot
                .insert(event(100, OWNER, "view.config", None, &[]), true)
                .unwrap();
            Self {
                snapshot,
                versions: Vec::new(),
            }
        }

        fn claim(&mut self, id: u8, author: AuthorKey, deps: &[u8]) {
            self.claim_with(id, author, "didit.record", Some("walk"), deps, true);
        }

        fn claim_with(
            &mut self,
            id: u8,
            author: AuthorKey,
            kind: &str,
            subject: Option<&str>,
            deps: &[u8],
            applied: bool,
        ) {
            self.snapshot
                .insert(event(id, author, kind, subject, deps), applied)
                .unwrap();
        }

        /// Publish an admission version at chain position `seq`.
        fn admit(
            &mut self,
            id: u8,
            seq: u64,
            deps: &[u8],
            entries: Vec<Entry>,
            excludes: Vec<Entry>,
        ) {
            self.admit_in(id, seq, set_id(), deps, entries, excludes);
        }

        fn admit_in(
            &mut self,
            id: u8,
            seq: u64,
            set: Hash,
            deps: &[u8],
            entries: Vec<Entry>,
            excludes: Vec<Entry>,
        ) {
            self.snapshot
                .insert(event(id, OWNER, "idset.writer", None, deps), true)
                .unwrap();
            let writer = WriterSet {
                set_id: set,
                version: seq,
                owner: OWNER,
                scope: None,
                entries,
                excludes: Excludes(excludes),
            };
            let frontier: Vec<Hash> = deps.iter().map(|d| h(*d)).collect();
            self.versions
                .push(OwnerVersion::admission(h(id), &writer, &frontier, &h(250), &[]).unwrap());
        }

        fn rescind(
            &mut self,
            id: u8,
            seq: u64,
            deps: &[u8],
            from_version: u64,
            target: Entry,
            through: Option<u8>,
        ) {
            self.snapshot
                .insert(event(id, OWNER, "idset.rescind", None, deps), true)
                .unwrap();
            let rescind = Rescind {
                set_id: set_id(),
                from_version,
                target,
                through: through.map(h),
            };
            let frontier: Vec<Hash> = deps.iter().map(|d| h(*d)).collect();
            self.versions.push(
                OwnerVersion::rescind(h(id), seq, OWNER, &rescind, &frontier, &h(250), &[])
                    .unwrap(),
            );
        }

        fn view(&self, set_seq: u64, frontier: &[u8]) -> View {
            View {
                view_id: h(150),
                config_seq: 0,
                config_event: h(100),
                owner: OWNER,
                set_id: set_id(),
                set_seq,
                frontier: frontier.iter().map(|f| h(*f)).collect(),
                resolution: h(250),
                subject_scope: Scope::Any,
                kind_scope: Scope::Any,
                exceptions: BTreeSet::new(),
            }
        }

        fn project(&self, view: &View, candidates: &[u8]) -> Projection {
            let ids: Vec<Hash> = candidates.iter().map(|c| h(*c)).collect();
            project(view, &self.versions, &self.snapshot, &ids).unwrap()
        }
    }

    fn event(
        id: u8,
        author: AuthorKey,
        kind: &str,
        subject: Option<&str>,
        deps: &[u8],
    ) -> SnapshotEvent {
        SnapshotEvent {
            id: h(id),
            author,
            kind: kind.to_owned(),
            subject: subject.map(str::to_owned),
            deps: deps.iter().map(|d| h(*d)).collect(),
        }
    }

    fn counted(p: &Projection) -> Vec<Hash> {
        p.counted.iter().map(|hit| hit.claim).collect()
    }

    fn reason(p: &Projection, id: u8) -> Option<WhyNotCounted> {
        p.uncounted
            .iter()
            .find(|hit| hit.claim == h(id))
            .map(|hit| hit.reason)
    }

    // #3789 acceptance: a third-party claim is stored but uncounted.
    #[test]
    fn a_strangers_claim_is_stored_but_uncounted() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        w.claim(20, ALICE, &[10]);
        w.claim(21, STRANGER, &[10]);
        let p = w.project(&w.view(1, &[20, 21]), &[20, 21]);
        assert_eq!(counted(&p), vec![h(20)]);
        assert_eq!(reason(&p, 21), Some(WhyNotCounted::AuthorNeverAdmitted));
        assert!(p.uncounted.iter().all(|hit| hit.signature_ok));
        assert_eq!(p.provenance.set_head, h(10));
        assert_eq!(p.provenance.set_seq, 1);
    }

    // §3.2: removal stops future counting and never rewrites the past.
    #[test]
    fn removal_keeps_earlier_claims_and_stops_later_ones() {
        let mut w = World::new();
        w.admit(
            10,
            1,
            &[100],
            vec![key(ALICE, false), key(BOB, false)],
            vec![],
        );
        w.claim(20, ALICE, &[10]);
        w.admit(11, 2, &[10, 20], vec![key(BOB, false)], vec![]);
        w.claim(22, ALICE, &[11]);
        w.claim(23, BOB, &[11]);
        let p = w.project(&w.view(2, &[22, 23]), &[20, 22, 23]);
        assert_eq!(counted(&p), vec![h(20), h(23)]);
        assert_eq!(reason(&p, 22), Some(WhyNotCounted::AuthorRemoved(2)));
    }

    // #3789 acceptance: an added writer's history counts only with
    // include-history.
    #[test]
    fn history_counts_only_with_include_history() {
        for (include_history, expect_counted) in [(false, false), (true, true)] {
            let mut w = World::new();
            w.claim(20, BOB, &[]); // predates every version
            w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
            w.admit(
                11,
                2,
                &[10, 20],
                vec![key(ALICE, false), key(BOB, include_history)],
                vec![],
            );
            w.claim(21, BOB, &[11]);
            let p = w.project(&w.view(2, &[11, 21]), &[20, 21]);
            assert!(counted(&p).contains(&h(21)), "new claims count either way");
            assert_eq!(counted(&p).contains(&h(20)), expect_counted);
            if expect_counted {
                let hit = p.counted.iter().find(|hit| hit.claim == h(20)).unwrap();
                assert_eq!(hit.governed_by, Some(2));
            } else {
                assert_eq!(reason(&p, 20), Some(WhyNotCounted::HistoryNotImported));
            }
        }
    }

    #[test]
    fn retro_does_not_import_another_entrys_backlog() {
        let mut w = World::new();
        w.claim(20, ALICE, &[]);
        w.claim(21, BOB, &[]);
        w.admit(
            10,
            1,
            &[100, 20, 21],
            vec![key(ALICE, true), key(BOB, false)],
            vec![],
        );
        let p = w.project(&w.view(1, &[10]), &[20, 21]);
        assert_eq!(counted(&p), vec![h(20)]);
        assert_eq!(reason(&p, 21), Some(WhyNotCounted::HistoryNotImported));
    }

    // #3789 acceptance: a rescind excludes from the stated point.
    #[test]
    fn a_rescind_erases_what_it_saw_and_nothing_after() {
        let mut w = World::new();
        w.admit(
            10,
            1,
            &[100],
            vec![key(ALICE, false), key(BOB, false)],
            vec![],
        );
        w.claim(20, ALICE, &[10]);
        w.claim(21, BOB, &[10]);
        w.rescind(30, 2, &[10, 20, 21], 1, key(ALICE, false), None);
        w.claim(22, ALICE, &[30]);
        w.claim(23, BOB, &[30]);
        let p = w.project(&w.view(1, &[22, 23]), &[20, 21, 22, 23]);
        // The rescind is version 2 but the view selects version 1, so the
        // rescind is not in versions(S).
        assert!(counted(&p).contains(&h(20)));

        w.admit(
            11,
            3,
            &[30],
            vec![key(ALICE, false), key(BOB, false)],
            vec![],
        );
        w.claim(24, BOB, &[11]);
        let p = w.project(&w.view(3, &[22, 23, 24]), &[20, 21, 22, 23, 24]);
        assert_eq!(reason(&p, 20), Some(WhyNotCounted::RescindedSince(2)));
        // Bob was not targeted; Alice's claim written after the rescind is
        // untouched; a rescind is never the admission state in force.
        assert_eq!(counted(&p), vec![h(21), h(22), h(23), h(24)]);
    }

    #[test]
    fn a_public_rescind_erases_everyones_seen_claims() {
        let mut w = World::new();
        w.admit(
            10,
            1,
            &[100],
            vec![key(ALICE, false), key(BOB, false)],
            vec![],
        );
        w.claim(20, ALICE, &[10]);
        w.claim(21, BOB, &[10]);
        w.rescind(
            30,
            2,
            &[20, 21],
            1,
            Entry::Public {
                include_history: false,
            },
            None,
        );
        w.admit(
            11,
            3,
            &[30],
            vec![key(ALICE, false), key(BOB, false)],
            vec![],
        );
        w.claim(22, BOB, &[11]);
        let p = w.project(&w.view(3, &[22]), &[20, 21, 22]);
        assert_eq!(counted(&p), vec![h(22)]);
        assert_eq!(reason(&p, 21), Some(WhyNotCounted::RescindedSince(2)));
    }

    #[test]
    fn a_bounded_rescind_preserves_what_precedes_its_from_version() {
        // Alice admitted at v1 writes 20; v2 re-admits her; she writes 21.
        let build = |from_version: u64| {
            let mut w = World::new();
            w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
            w.claim(20, ALICE, &[10]);
            w.admit(11, 2, &[20], vec![key(ALICE, false)], vec![]);
            w.claim(21, ALICE, &[11]);
            w.rescind(30, 3, &[21], from_version, key(ALICE, false), None);
            w.admit(12, 4, &[30], vec![key(ALICE, false)], vec![]);
            w.project(&w.view(4, &[12]), &[20, 21])
        };
        let from_v2 = build(2);
        assert_eq!(counted(&from_v2), vec![h(20)]);
        assert_eq!(reason(&from_v2, 21), Some(WhyNotCounted::RescindedSince(3)));
        let from_v1 = build(1);
        assert_eq!(counted(&from_v1), [] as [p2panda_core::Hash; 0]);
    }

    #[test]
    fn a_bounded_rescind_honours_its_through_event() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        w.claim(20, ALICE, &[10]);
        w.claim(21, ALICE, &[20]);
        w.rescind(30, 2, &[21], 1, key(ALICE, false), Some(20));
        w.admit(11, 3, &[30], vec![key(ALICE, false)], vec![]);
        let p = w.project(&w.view(3, &[11]), &[20, 21]);
        // `through: 20` erases up to and including 20; 21, which the
        // rescind also saw, lies beyond the bound.
        assert_eq!(counted(&p), vec![h(21)]);
        assert_eq!(reason(&p, 20), Some(WhyNotCounted::RescindedSince(2)));
    }

    #[test]
    fn scope_rejects_with_a_reason_and_keeps_the_claim_visible() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        w.claim(20, ALICE, &[10]);
        w.claim_with(21, ALICE, "didit.record", Some("chat"), &[10], true);
        w.claim_with(22, ALICE, "tag.set", Some("walk"), &[10], true);
        let mut view = w.view(1, &[20, 21, 22]);
        view.subject_scope = Scope::Only(BTreeSet::from(["walk".to_owned()]));
        view.kind_scope = Scope::Only(BTreeSet::from(["didit.record".to_owned()]));
        let p = w.project(&view, &[20, 21, 22]);
        assert_eq!(counted(&p), vec![h(20)]);
        assert_eq!(
            reason(&p, 21),
            Some(WhyNotCounted::OutOfScope(ScopeMismatch::Subject))
        );
        assert_eq!(
            reason(&p, 22),
            Some(WhyNotCounted::OutOfScope(ScopeMismatch::Kind))
        );
    }

    #[test]
    fn exceptions_count_a_strangers_claim_but_never_an_invalid_one() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        w.claim(21, STRANGER, &[10]);
        w.claim_with(22, STRANGER, "didit.record", Some("walk"), &[10], false);
        let mut view = w.view(1, &[21, 22]);
        view.exceptions = BTreeSet::from([h(21), h(22), h(99)]);
        // No candidates: exceptions are fetched by ID; a missing one does
        // not fire; a claim not applied under R is in neither collection.
        let p = w.project(&view, &[]);
        assert_eq!(counted(&p), vec![h(21)]);
        assert_eq!(p.counted[0].governed_by, None);
        assert_eq!(p.uncounted, [] as [UncountedHit; 0]);
    }

    #[test]
    fn the_frontier_closes_the_total_against_later_claims() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        w.claim(20, ALICE, &[10]);
        let view = w.view(1, &[20]);
        let before = w.project(&view, &[20]);
        w.claim(21, ALICE, &[20]);
        let after = w.project(&view, &[20, 21]);
        assert_eq!(counted(&before), counted(&after));
        assert_eq!(before.provenance, after.provenance);
        assert_eq!(reason(&after, 21), Some(WhyNotCounted::AfterFrontier));
    }

    #[test]
    fn another_sets_versions_on_the_same_chain_are_ignored() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        // seq 2 publishes a different set that admits everyone.
        w.admit_in(
            11,
            2,
            h(201),
            &[10],
            vec![Entry::Public {
                include_history: true,
            }],
            vec![],
        );
        w.claim(21, STRANGER, &[11]);
        w.admit(12, 3, &[11], vec![key(ALICE, false)], vec![]);
        let p = w.project(&w.view(3, &[21, 12]), &[21]);
        assert_eq!(reason(&p, 21), Some(WhyNotCounted::AuthorNeverAdmitted));
    }

    #[test]
    fn an_unapplied_set_version_or_config_event_is_undefined() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        // A forked set version at seq 2.
        w.snapshot
            .insert(event(11, OWNER, "idset.writer", None, &[10]), false)
            .unwrap();
        let forked = WriterSet {
            set_id: set_id(),
            version: 2,
            owner: OWNER,
            scope: None,
            entries: vec![key(STRANGER, false)],
            excludes: Excludes::default(),
        };
        w.versions
            .push(OwnerVersion::admission(h(11), &forked, &[h(10)], &h(250), &[]).unwrap());
        let err = project(&w.view(2, &[10]), &w.versions, &w.snapshot, &[]).unwrap_err();
        assert_eq!(err, ProjectionError::Undefined("writer-set"));

        let mut view = w.view(1, &[10]);
        w.snapshot
            .insert(event(101, OWNER, "view.exception", None, &[100]), false)
            .unwrap();
        view.config_event = h(101);
        let err = project(&view, &w.versions, &w.snapshot, &[]).unwrap_err();
        assert_eq!(err, ProjectionError::Undefined("view"));
    }

    #[test]
    fn a_version_signed_by_someone_else_is_refused() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        let mut view = w.view(1, &[10]);
        view.owner = BOB;
        let err = project(&view, &w.versions, &w.snapshot, &[]).unwrap_err();
        assert_eq!(err, ProjectionError::InvalidWriterSet(h(10)));
    }

    #[test]
    fn an_emergency_stop_admits_nobody_prospectively() {
        let mut w = World::new();
        w.admit(
            10,
            1,
            &[100],
            vec![Entry::Public {
                include_history: false,
            }],
            vec![],
        );
        w.claim(20, STRANGER, &[10]);
        w.admit(
            11,
            2,
            &[20],
            vec![Entry::Public {
                include_history: false,
            }],
            vec![Entry::Public {
                include_history: false,
            }],
        );
        w.claim(21, STRANGER, &[11]);
        let p = w.project(&w.view(2, &[21]), &[20, 21]);
        assert_eq!(counted(&p), vec![h(20)]);
        assert_eq!(reason(&p, 21), Some(WhyNotCounted::AuthorRemoved(2)));
    }

    #[test]
    fn missing_events_are_errors_not_silent_skips() {
        let mut w = World::new();
        w.admit(10, 1, &[100], vec![key(ALICE, false)], vec![]);
        let err = project(&w.view(1, &[10]), &w.versions, &w.snapshot, &[h(77)]).unwrap_err();
        assert_eq!(err, ProjectionError::MissingEvent(h(77)));
        assert_eq!(
            w.snapshot
                .insert(event(10, OWNER, "idset.writer", None, &[]), true)
                .unwrap_err(),
            ProjectionError::DuplicateEvent(h(10))
        );
    }
}
