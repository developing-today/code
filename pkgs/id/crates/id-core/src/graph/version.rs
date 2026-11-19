//! `succeeds` versioning: the version DAG core (slice A-GRAPH-9129-01,
//! #10223; design part A §8, statements S5.41–S5.43).
//!
//! A new version of an object is linked to the version it replaces by
//! the triple `(predecessor_v1, succeeds, successor_v2)`. The stored
//! direction is always v1 → v2. A reverse index from v2 to v1 speeds up
//! ancestry lookups and never changes the stored direction.
//!
//! Admission rules:
//!
//! - an edge must join two different versions (no self-edge) of the same
//!   logical object with the same schema;
//! - when both versions are known, an edge that would close a cycle is
//!   rejected, so the accepted graph is always acyclic;
//! - when either version is not yet known, the edge stays *pending* and is
//!   re-checked each time a version is added, so incomplete ancestry is
//!   flagged rather than guessed at;
//! - lineage may branch, and every head (a version with no accepted
//!   successor) stays observable. A head is chosen only by an explicit
//!   owner selection; there is no timestamp or arrival-order tie-break.
//!
//! The legacy board `Supersede` record is never rewritten. It maps onto a
//! `succeeds` edge only through [`adapt_supersede`], which requires the
//! caller to have proven the same signer and checks the same logical
//! object and schema.
//!
//! Signature checks, owner/admin edit authorization and the V3 comment
//! carry-forward view belong to other owners; this module takes already
//! verified versions and owner selections as inputs. Module wiring into
//! `graph/mod.rs` belongs to slice 9110-09; until then the integration
//! test includes this file by path.

use std::collections::{HashMap, HashSet};
use std::fmt;

use super::term::{ObjectRef, Term, Triple};

/// Type tag of a relation-name term: UTF-8 bytes naming a predicate.
pub const RELATION_NAME_TAG: u64 = 1;

/// The relation name carried by the `succeeds` predicate term.
pub const SUCCEEDS_NAME: &[u8] = b"id/succeeds";

/// The `succeeds` predicate term.
#[must_use]
pub fn succeeds_predicate() -> Term {
    Term::typed(RELATION_NAME_TAG, SUCCEEDS_NAME.to_vec())
}

/// A deterministic total order over object references (kind, hash
/// profile, digest), used so every listing comes out in the same order.
#[must_use]
pub fn ref_order_key(reference: &ObjectRef) -> (u8, u8, [u8; 32]) {
    let (kind, id) = match reference {
        ObjectRef::Content(id) => (1, id),
        ObjectRef::Envelope(id) => (2, id.content_id()),
    };
    (kind, id.profile().tag(), *id.digest())
}

fn sorted(set: impl IntoIterator<Item = ObjectRef>) -> Vec<ObjectRef> {
    let mut out: Vec<ObjectRef> = set.into_iter().collect();
    out.sort_by_key(ref_order_key);
    out
}

/// A verified version of a logical object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// The version's own identity (usually its signed envelope).
    pub id: ObjectRef,
    /// The stable key of the logical object this is a version of.
    pub logical_object: Term,
    /// The object's schema version; edges join equal schemas only.
    pub schema: u64,
}

/// One stored `succeeds` edge, v1 → v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SucceedsEdge {
    /// The version being replaced.
    pub predecessor: ObjectRef,
    /// The version that replaces it.
    pub successor: ObjectRef,
}

impl SucceedsEdge {
    /// An edge from `predecessor` to `successor`.
    #[must_use]
    pub const fn new(predecessor: ObjectRef, successor: ObjectRef) -> Self {
        Self {
            predecessor,
            successor,
        }
    }

    /// The edge as the triple `(predecessor, succeeds, successor)`.
    #[must_use]
    pub fn to_triple(self) -> Triple {
        Triple::new(
            Term::Object(self.predecessor),
            succeeds_predicate(),
            Term::Object(self.successor),
        )
    }

    /// Reads an edge from a `succeeds` triple.
    ///
    /// # Errors
    ///
    /// [`VersionError::NotSucceeds`] when the predicate is not
    /// `succeeds` or either end is not an object reference.
    pub fn from_triple(triple: &Triple) -> Result<Self, VersionError> {
        if triple.predicate != succeeds_predicate() {
            return Err(VersionError::NotSucceeds("predicate is not succeeds"));
        }
        let (Some(predecessor), Some(successor)) =
            (triple.subject.object_ref(), triple.object.object_ref())
        else {
            return Err(VersionError::NotSucceeds(
                "both ends must be object references",
            ));
        };
        Ok(Self::new(*predecessor, *successor))
    }
}

/// Why an edge or adaptation was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// The two versions belong to different logical objects.
    WrongLogicalObject,
    /// The edge cannot be a successor link (self-edge, schema mismatch,
    /// or a version listed under a different identity).
    InvalidSuccessor(&'static str),
    /// Accepting the edge would close a cycle in complete ancestry.
    LineageCycle,
    /// The triple is not a `succeeds` edge.
    NotSucceeds(&'static str),
    /// A legacy `Supersede` was not proven equivalent to `succeeds`.
    NotEquivalent(&'static str),
}

impl VersionError {
    /// A stable failure class name.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::WrongLogicalObject => "wrong_logical_object",
            Self::InvalidSuccessor(_) => "invalid_successor",
            Self::LineageCycle => "lineage_cycle",
            Self::NotSucceeds(_) => "malformed",
            Self::NotEquivalent(_) => "not_equivalent",
        }
    }
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLogicalObject => write!(f, "versions belong to different logical objects"),
            Self::InvalidSuccessor(why) => write!(f, "invalid successor: {why}"),
            Self::LineageCycle => write!(f, "edge would close a version cycle"),
            Self::NotSucceeds(why) => write!(f, "not a succeeds edge: {why}"),
            Self::NotEquivalent(why) => write!(f, "supersede not proven equivalent: {why}"),
        }
    }
}

impl std::error::Error for VersionError {}

/// What happened to an edge that was not rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// The edge is in the accepted graph (newly or already).
    Accepted,
    /// One or both versions are not known yet; the edge waits.
    Pending {
        /// The unknown versions, in deterministic order.
        missing: Vec<ObjectRef>,
    },
}

/// The result of re-checking pending edges after a version arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The edge that was re-checked.
    pub edge: SucceedsEdge,
    /// Accepted, rejected, or still pending.
    pub outcome: Result<Admission, VersionError>,
}

/// The current version(s) of one logical object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadView {
    /// No known version.
    Empty,
    /// Exactly one head.
    Single(ObjectRef),
    /// The owner selected one of several heads; the rest stay listed.
    Selected {
        /// The owner's selection.
        head: ObjectRef,
        /// The other heads, never hidden.
        alternatives: Vec<ObjectRef>,
    },
    /// Several heads and no valid owner selection: concurrent heads,
    /// an informational result rather than an error.
    Branches(Vec<ObjectRef>),
}

/// The version DAG over verified versions.
#[derive(Debug, Default, Clone)]
pub struct VersionDag {
    versions: HashMap<ObjectRef, Version>,
    successors: HashMap<ObjectRef, HashSet<ObjectRef>>,
    predecessors: HashMap<ObjectRef, HashSet<ObjectRef>>,
    pending: Vec<SucceedsEdge>,
}

impl VersionDag {
    /// An empty DAG.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a verified version and re-checks every pending edge.
    ///
    /// # Errors
    ///
    /// [`VersionError::InvalidSuccessor`] when the same identity is
    /// already known with a different logical object or schema.
    pub fn add_version(&mut self, version: Version) -> Result<Vec<Resolution>, VersionError> {
        if let Some(known) = self.versions.get(&version.id) {
            if *known != version {
                return Err(VersionError::InvalidSuccessor(
                    "version id already known with other contents",
                ));
            }
            return Ok(Vec::new());
        }
        self.versions.insert(version.id, version);
        let waiting = std::mem::take(&mut self.pending);
        Ok(waiting
            .into_iter()
            .map(|edge| Resolution {
                edge,
                outcome: self.admit(edge),
            })
            .collect())
    }

    /// Admits a `succeeds` edge.
    ///
    /// # Errors
    ///
    /// [`VersionError`] for a self-edge, a cross-object or cross-schema
    /// edge, or a cycle in complete ancestry.
    pub fn admit(&mut self, edge: SucceedsEdge) -> Result<Admission, VersionError> {
        if edge.predecessor == edge.successor {
            return Err(VersionError::InvalidSuccessor(
                "a version cannot succeed itself",
            ));
        }
        let missing = sorted(
            [edge.predecessor, edge.successor]
                .into_iter()
                .filter(|id| !self.versions.contains_key(id)),
        );
        if !missing.is_empty() {
            if !self.pending.contains(&edge) {
                self.pending.push(edge);
            }
            return Ok(Admission::Pending { missing });
        }
        let predecessor = &self.versions[&edge.predecessor];
        let successor = &self.versions[&edge.successor];
        if predecessor.logical_object != successor.logical_object {
            return Err(VersionError::WrongLogicalObject);
        }
        if predecessor.schema != successor.schema {
            return Err(VersionError::InvalidSuccessor(
                "versions have different schemas",
            ));
        }
        if self
            .descendants(&edge.successor)
            .contains(&edge.predecessor)
        {
            return Err(VersionError::LineageCycle);
        }
        self.successors
            .entry(edge.predecessor)
            .or_default()
            .insert(edge.successor);
        self.predecessors
            .entry(edge.successor)
            .or_default()
            .insert(edge.predecessor);
        Ok(Admission::Accepted)
    }

    /// Edges still waiting for a version, in arrival order.
    #[must_use]
    pub fn pending(&self) -> &[SucceedsEdge] {
        &self.pending
    }

    /// Accepted edges, in deterministic order.
    #[must_use]
    pub fn edges(&self) -> Vec<SucceedsEdge> {
        let mut out: Vec<SucceedsEdge> = self
            .successors
            .iter()
            .flat_map(|(from, to)| to.iter().map(|to| SucceedsEdge::new(*from, *to)))
            .collect();
        out.sort_by_key(|edge| {
            (
                ref_order_key(&edge.predecessor),
                ref_order_key(&edge.successor),
            )
        });
        out
    }

    /// Direct successors of a version.
    #[must_use]
    pub fn successors_of(&self, id: &ObjectRef) -> Vec<ObjectRef> {
        sorted(self.successors.get(id).into_iter().flatten().copied())
    }

    /// Direct predecessors of a version, from the reverse index.
    #[must_use]
    pub fn predecessors_of(&self, id: &ObjectRef) -> Vec<ObjectRef> {
        sorted(self.predecessors.get(id).into_iter().flatten().copied())
    }

    /// Every version reachable backwards from `id`, excluding `id`.
    #[must_use]
    pub fn ancestors(&self, id: &ObjectRef) -> Vec<ObjectRef> {
        sorted(Self::reach(&self.predecessors, id))
    }

    fn descendants(&self, id: &ObjectRef) -> HashSet<ObjectRef> {
        Self::reach(&self.successors, id)
    }

    fn reach(
        index: &HashMap<ObjectRef, HashSet<ObjectRef>>,
        start: &ObjectRef,
    ) -> HashSet<ObjectRef> {
        let mut seen = HashSet::new();
        let mut stack = vec![*start];
        while let Some(at) = stack.pop() {
            for next in index.get(&at).into_iter().flatten() {
                if seen.insert(*next) {
                    stack.push(*next);
                }
            }
        }
        seen
    }

    /// Every known version of `logical_object` with no accepted
    /// successor, in deterministic order.
    #[must_use]
    pub fn heads(&self, logical_object: &Term) -> Vec<ObjectRef> {
        sorted(
            self.versions
                .values()
                .filter(|version| version.logical_object == *logical_object)
                .filter(|version| {
                    self.successors
                        .get(&version.id)
                        .is_none_or(HashSet::is_empty)
                })
                .map(|version| version.id),
        )
    }

    /// The head view of `logical_object` under an optional, already
    /// verified owner selection. A selection that is not a current head
    /// is ignored and the branch set is shown.
    #[must_use]
    pub fn head_view(
        &self,
        logical_object: &Term,
        owner_selection: Option<&ObjectRef>,
    ) -> HeadView {
        let heads = self.heads(logical_object);
        match heads.as_slice() {
            [] => HeadView::Empty,
            [only] => HeadView::Single(*only),
            _ => match owner_selection.filter(|chosen| heads.contains(chosen)) {
                Some(chosen) => HeadView::Selected {
                    head: *chosen,
                    alternatives: heads.into_iter().filter(|id| id != chosen).collect(),
                },
                None => HeadView::Branches(heads),
            },
        }
    }
}

/// A legacy board `Supersede`, by the identities of its two objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacySupersede {
    /// The earlier object.
    pub previous: ObjectRef,
    /// The object that replaces it.
    pub replacement: ObjectRef,
}

/// Maps a legacy `Supersede` onto a `succeeds` edge, leaving the
/// original record untouched.
///
/// `same_signer` is the caller's proof that both objects were signed by
/// the same author, which `Supersede` requires. The two versions must be
/// the ones the record names, of the same logical object and schema.
///
/// # Errors
///
/// [`VersionError::NotEquivalent`] when any of that is not shown.
pub fn adapt_supersede(
    supersede: &LegacySupersede,
    previous: &Version,
    replacement: &Version,
    same_signer: bool,
) -> Result<SucceedsEdge, VersionError> {
    if previous.id != supersede.previous || replacement.id != supersede.replacement {
        return Err(VersionError::NotEquivalent(
            "versions are not the ones the record names",
        ));
    }
    if !same_signer {
        return Err(VersionError::NotEquivalent(
            "signers differ or are unproven",
        ));
    }
    if supersede.previous == supersede.replacement {
        return Err(VersionError::NotEquivalent(
            "record replaces an object with itself",
        ));
    }
    if previous.logical_object != replacement.logical_object {
        return Err(VersionError::NotEquivalent("different logical objects"));
    }
    if previous.schema != replacement.schema {
        return Err(VersionError::NotEquivalent("different schemas"));
    }
    Ok(SucceedsEdge::new(supersede.previous, supersede.replacement))
}
