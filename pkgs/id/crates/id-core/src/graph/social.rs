//! Social object graph: nested topic/comment objects and bounded feed
//! projections (slice A-GRAPH-9128-01, #10252; design part A §7,
//! statements S5.30–S5.36, S5.40).
//!
//! Social shapes are application profiles over nested object
//! references. A topic, comment, reaction, feed entry or profile is
//! immutable content plus a signed relation to its parent and optional
//! append admission; signature verification itself belongs to the
//! admission layer, so this module takes already verified objects.
//!
//! Feed ordering and moderation are named projections with provenance,
//! never an inherent order in the graph. Readers traverse iteratively
//! under a selected deployment profile (defaults: max depth 32, max
//! visited nodes 2,000, max response 8 MiB, per §7/§12); cycles end in
//! explicit back-references and budget overruns in explicit truncation
//! entries, never in recursion or silent omission.
//!
//! Feed freshness is kept by one durable projection ledger per feed
//! scope. Every entry receives a monotonically increasing
//! `last_projection_seq` from the ledger's high-water mark, which
//! continues across projection-generation changes and is never reused;
//! exhaustion stops allocation instead of wrapping. A cursor pins the
//! source frontiers of one complete snapshot: `since` pages forward
//! inside that pinned snapshot and `before` pages backward, both in
//! the same stable total order `(last_projection_seq, entry_id)`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

use super::term::ObjectRef;
use super::version::ref_order_key;

/// Default maximum display depth for traversal (design §7).
pub const DEFAULT_MAX_DEPTH: usize = 32;
/// Default maximum visited nodes per traversal (design §7/§12).
pub const DEFAULT_MAX_VISITED: usize = 2_000;
/// Default maximum response bytes per traversal (design §7/§12).
pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;

/// The shape of a social object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SocialObjectKind {
    /// A discussion root.
    Topic,
    /// A comment on a parent object.
    Comment,
    /// A reaction to a parent object.
    Reaction,
    /// A feed entry referencing its content object.
    FeedEntry,
    /// A profile object.
    Profile,
}

/// One immutable social object (design §7). `created_claim` is the
/// signer's untrusted claim, never a trusted wall clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialObject {
    /// The object's own identity.
    pub object_id: ObjectRef,
    /// The object's shape.
    pub kind: SocialObjectKind,
    /// The signer identity that authored the object.
    pub author: String,
    /// Untrusted signer-claimed creation position.
    pub created_claim: u64,
    /// The parent this object is nested under, if any.
    pub parent_ref: Option<ObjectRef>,
    /// The immutable content this object points at.
    pub content_ref: ObjectRef,
    /// The schema the object was written under.
    pub schema_version: u64,
    /// The version this object succeeds, if it is a version.
    pub succeeds_ref: Option<ObjectRef>,
    /// The social object a feed entry surfaces. Required for
    /// [`SocialObjectKind::FeedEntry`] and absent for every other
    /// profile (envelope design #13095 §4); see
    /// [`super::social_envelope`] for the validator.
    pub target_ref: Option<ObjectRef>,
}

/// The selected deployment profile bounding one traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraversalProfile {
    /// Deepest depth that is still expanded; deeper positions are
    /// reported as [`TraversalMark::TruncatedDepth`].
    pub max_depth: usize,
    /// Most objects one traversal may expand.
    pub max_visited: usize,
    /// Most charged bytes one traversal may return.
    pub max_bytes: usize,
}

impl Default for TraversalProfile {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_visited: DEFAULT_MAX_VISITED,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// How one position of a traversal result was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalMark {
    /// The object was expanded at this position.
    Expanded,
    /// The object was already visited; this position points back to
    /// the first visit instead of expanding again.
    BackReference,
    /// The object sits deeper than the profile's max depth.
    TruncatedDepth,
    /// A node or byte budget stopped expansion at this position.
    TruncatedBudget,
}

/// One position of a traversal result, in visit order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraversalEntry {
    /// The object at this position.
    pub object_id: ObjectRef,
    /// Depth below the traversal root (the root is depth 0).
    pub depth: usize,
    /// How this position was produced.
    pub mark: TraversalMark,
}

/// The outcome of one bounded traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Traversal {
    /// Every visited position, in visit order.
    pub entries: Vec<TraversalEntry>,
    /// False when any entry is a truncation: the result is then a
    /// bounded prefix of the authorized relation set, never a claim
    /// of completeness.
    pub complete: bool,
}

/// Bytes one expanded object charges against the profile's byte
/// budget: its two digests plus its author claim bytes.
pub(super) fn entry_cost(object: &SocialObject) -> usize {
    32 + 32 + object.author.len()
}

/// The authorized objects by ID, and each one's authorized children
/// in [`ref_order_key`] order. Unauthorized objects take no part; a
/// repeated listing of one ID keeps the first.
pub(super) fn authorized_index<'a>(
    objects: &'a [SocialObject],
    authorized: impl Fn(&SocialObject) -> bool,
) -> (
    HashMap<ObjectRef, &'a SocialObject>,
    HashMap<ObjectRef, Vec<ObjectRef>>,
) {
    let mut by_id: HashMap<ObjectRef, &SocialObject> = HashMap::new();
    let mut children: HashMap<ObjectRef, Vec<ObjectRef>> = HashMap::new();
    for object in objects {
        if !authorized(object) {
            continue;
        }
        // Objects are immutable, so a repeated listing of one ID is
        // the same object; the first listing wins.
        if by_id.contains_key(&object.object_id) {
            continue;
        }
        by_id.insert(object.object_id, object);
        if let Some(parent) = object.parent_ref {
            children.entry(parent).or_default().push(object.object_id);
        }
    }
    for list in children.values_mut() {
        list.sort_by_key(|id| ref_order_key(id));
        list.dedup();
    }
    (by_id, children)
}

/// Iterative depth-first traversal of the authorized children of
/// `root`, in the deterministic child order given by
/// [`ref_order_key`].
///
/// Only objects the `authorized` predicate accepts take part: an
/// unauthorized object is neither returned nor traversed through, so
/// the result is exactly the authorized relations. A position whose
/// object was already visited is a [`TraversalMark::BackReference`];
/// a position deeper than the profile is
/// [`TraversalMark::TruncatedDepth`]; once a node or byte budget is
/// spent, every further unvisited position is
/// [`TraversalMark::TruncatedBudget`] and nothing more is expanded.
#[must_use]
pub fn traverse(
    objects: &[SocialObject],
    root: &ObjectRef,
    profile: &TraversalProfile,
    authorized: impl Fn(&SocialObject) -> bool,
) -> Traversal {
    let (by_id, children) = authorized_index(objects, authorized);

    let mut entries = Vec::new();
    let mut complete = true;
    if !by_id.contains_key(root) {
        return Traversal { entries, complete };
    }
    let mut visited: HashSet<ObjectRef> = HashSet::new();
    let mut visited_count = 0usize;
    let mut bytes = 0usize;
    let mut stack: Vec<(ObjectRef, usize)> = vec![(*root, 0)];
    while let Some((id, depth)) = stack.pop() {
        if visited.contains(&id) {
            entries.push(TraversalEntry {
                object_id: id,
                depth,
                mark: TraversalMark::BackReference,
            });
            continue;
        }
        if depth > profile.max_depth {
            entries.push(TraversalEntry {
                object_id: id,
                depth,
                mark: TraversalMark::TruncatedDepth,
            });
            complete = false;
            continue;
        }
        let object = by_id[&id];
        let cost = entry_cost(object);
        if visited_count >= profile.max_visited || bytes + cost > profile.max_bytes {
            entries.push(TraversalEntry {
                object_id: id,
                depth,
                mark: TraversalMark::TruncatedBudget,
            });
            complete = false;
            continue;
        }
        visited.insert(id);
        visited_count += 1;
        bytes += cost;
        entries.push(TraversalEntry {
            object_id: id,
            depth,
            mark: TraversalMark::Expanded,
        });
        if let Some(kids) = children.get(&id) {
            for kid in kids.iter().rev() {
                stack.push((*kid, depth + 1));
            }
        }
    }
    Traversal { entries, complete }
}

/// One feed scope; a ledger serves exactly one scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedScope {
    /// The server keeping the ledger.
    pub server_id: String,
    /// The tenant the feed belongs to.
    pub tenant_id: String,
    /// Digest of the query the feed answers.
    pub query_fingerprint: [u8; 32],
}

/// Where one feed entry came from: its source authority's own
/// position. Source positions are provenance, never the merged
/// cursor position (design §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePosition {
    /// The authority the entry originated from.
    pub source_authority: String,
    /// The source authority's epoch.
    pub source_epoch: u64,
    /// The source authority's sequence inside its epoch.
    pub source_sequence: u64,
    /// The object as named by its source authority.
    pub source_object_id: ObjectRef,
}

/// One source authority's frontier inside a pinned snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFrontier {
    /// The authority this frontier belongs to.
    pub source_authority: String,
    /// The highest epoch included from that authority.
    pub source_epoch: u64,
    /// The highest sequence included at that epoch.
    pub source_sequence: u64,
}

/// One entry of a feed projection ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEntry {
    /// The entry's stable identity inside the ledger.
    pub entry_id: String,
    /// The entry's position in the merged projection order, from the
    /// ledger's high-water mark.
    pub last_projection_seq: u64,
    /// The object the entry projects.
    pub object_id: ObjectRef,
    /// Where the entry came from.
    pub source: SourcePosition,
}

/// A paging position over one pinned feed snapshot. A cursor proves
/// freshness only for its own server, tenant, query and pinned
/// source-frontier vector (design §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedCursor {
    /// The server the cursor was issued by.
    pub server_id: String,
    /// The tenant the cursor was issued for.
    pub tenant_id: String,
    /// Digest of the query the cursor continues.
    pub query_fingerprint: [u8; 32],
    /// The projection generation the cursor belongs to.
    pub projection_generation: u64,
    /// The merged position the cursor resumes after.
    pub last_projection_seq: u64,
    /// The entry at that merged position.
    pub last_entry_id: String,
    /// The source frontiers pinned into the snapshot.
    pub snapshot_cutoff: Vec<SourceFrontier>,
}

/// Why a feed operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedError {
    /// The cursor belongs to an older projection generation.
    CursorExpired,
    /// The cursor belongs to another feed scope, or is not a cursor
    /// for this ledger.
    CursorMismatch,
    /// An entry with this ID is already in the ledger.
    DuplicateEntry,
    /// The projection sequence space is exhausted; allocation stops
    /// rather than wrapping.
    SequenceExhausted,
}

impl FeedError {
    /// A stable failure class name.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::CursorExpired => "cursor_expired",
            Self::CursorMismatch => "cursor_mismatch",
            Self::DuplicateEntry => "duplicate_entry",
            Self::SequenceExhausted => "sequence_exhausted",
        }
    }
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CursorExpired => write!(f, "cursor belongs to an older projection generation"),
            Self::CursorMismatch => write!(f, "cursor belongs to another feed scope"),
            Self::DuplicateEntry => write!(f, "entry id already present in the ledger"),
            Self::SequenceExhausted => write!(f, "projection sequence space exhausted"),
        }
    }
}

impl std::error::Error for FeedError {}

/// The durable projection ledger of one complete feed scope.
///
/// The high-water mark (`next_seq`) survives generation rotation and
/// restart restores, so a `last_projection_seq` is never reused for
/// the lifetime of the scope.
#[derive(Debug, Clone)]
pub struct FeedLedger {
    scope: FeedScope,
    generation: u64,
    next_seq: u64,
    exhausted: bool,
    entries: Vec<FeedEntry>,
    entry_ids: BTreeSet<String>,
}

impl FeedLedger {
    /// A fresh ledger for `scope`, at generation 1.
    #[must_use]
    pub fn new(scope: FeedScope) -> Self {
        Self {
            scope,
            generation: 1,
            next_seq: 1,
            exhausted: false,
            entries: Vec::new(),
            entry_ids: BTreeSet::new(),
        }
    }

    /// A ledger restored from durable state after a restart: the
    /// generation and high-water mark continue where they left off.
    #[must_use]
    pub fn restored(scope: FeedScope, generation: u64, next_seq: u64) -> Self {
        Self {
            scope,
            generation,
            next_seq,
            exhausted: false,
            entries: Vec::new(),
            entry_ids: BTreeSet::new(),
        }
    }

    /// The scope this ledger serves.
    #[must_use]
    pub fn scope(&self) -> &FeedScope {
        &self.scope
    }

    /// The current projection generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every entry, in merged projection order.
    #[must_use]
    pub fn entries(&self) -> &[FeedEntry] {
        &self.entries
    }

    /// Appends one entry, assigning its merged position from the
    /// high-water mark. Two entries from different source authorities
    /// at the same source position still receive distinct projection
    /// sequences.
    ///
    /// # Errors
    ///
    /// [`FeedError::DuplicateEntry`] when the entry ID is taken, or
    /// [`FeedError::SequenceExhausted`] when the sequence space is
    /// spent; neither failure appends anything.
    pub fn append(
        &mut self,
        entry_id: String,
        object_id: ObjectRef,
        source: SourcePosition,
    ) -> Result<FeedEntry, FeedError> {
        if self.exhausted {
            return Err(FeedError::SequenceExhausted);
        }
        if self.entry_ids.contains(&entry_id) {
            return Err(FeedError::DuplicateEntry);
        }
        let seq = self.next_seq;
        if seq == u64::MAX {
            self.exhausted = true;
        } else {
            self.next_seq = seq + 1;
        }
        let entry = FeedEntry {
            entry_id: entry_id.clone(),
            last_projection_seq: seq,
            object_id,
            source,
        };
        self.entry_ids.insert(entry_id);
        self.entries.push(entry.clone());
        Ok(entry)
    }

    /// Rotates the projection generation, as after a ledger loss or
    /// rollback. The high-water mark does not move; cursors of older
    /// generations stop being honoured.
    pub fn rotate_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }

    /// A cursor at the ledger's current head, pinning the source
    /// frontiers of everything appended so far.
    #[must_use]
    pub fn cursor(&self) -> FeedCursor {
        let (seq, entry_id) = self
            .entries
            .last()
            .map(|entry| (entry.last_projection_seq, entry.entry_id.clone()))
            .unwrap_or((0, String::new()));
        FeedCursor {
            server_id: self.scope.server_id.clone(),
            tenant_id: self.scope.tenant_id.clone(),
            query_fingerprint: self.scope.query_fingerprint,
            projection_generation: self.generation,
            last_projection_seq: seq,
            last_entry_id: entry_id,
            snapshot_cutoff: self.cutoff(),
        }
    }

    /// The per-authority maxima over every entry, in authority order.
    fn cutoff(&self) -> Vec<SourceFrontier> {
        let mut map: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for entry in &self.entries {
            let current = map
                .entry(entry.source.source_authority.clone())
                .or_insert((0, 0));
            let position = (entry.source.source_epoch, entry.source.source_sequence);
            if position > *current {
                *current = position;
            }
        }
        map.into_iter()
            .map(
                |(source_authority, (source_epoch, source_sequence))| SourceFrontier {
                    source_authority,
                    source_epoch,
                    source_sequence,
                },
            )
            .collect()
    }

    fn validate(&self, cursor: &FeedCursor) -> Result<(), FeedError> {
        if cursor.server_id != self.scope.server_id
            || cursor.tenant_id != self.scope.tenant_id
            || cursor.query_fingerprint != self.scope.query_fingerprint
        {
            return Err(FeedError::CursorMismatch);
        }
        if cursor.projection_generation != self.generation {
            return Err(FeedError::CursorExpired);
        }
        Ok(())
    }

    fn in_snapshot(cursor: &FeedCursor, entry: &FeedEntry) -> bool {
        cursor
            .snapshot_cutoff
            .iter()
            .find(|frontier| frontier.source_authority == entry.source.source_authority)
            .is_some_and(|frontier| {
                (entry.source.source_epoch, entry.source.source_sequence)
                    <= (frontier.source_epoch, frontier.source_sequence)
            })
    }

    fn after(entry: &FeedEntry, cursor: &FeedCursor) -> bool {
        entry.last_projection_seq > cursor.last_projection_seq
            || (entry.last_projection_seq == cursor.last_projection_seq
                && entry.entry_id > cursor.last_entry_id)
    }

    fn is_before_frontier(entry: &FeedEntry, cursor: &FeedCursor) -> bool {
        entry.last_projection_seq < cursor.last_projection_seq
            || (entry.last_projection_seq == cursor.last_projection_seq
                && entry.entry_id < cursor.last_entry_id)
    }

    /// Entries after the cursor's position that belong to its pinned
    /// snapshot, in merged projection order.
    ///
    /// # Errors
    ///
    /// [`FeedError::CursorMismatch`] for a cursor of another scope,
    /// [`FeedError::CursorExpired`] for a cursor of an older
    /// generation.
    pub fn since(&self, cursor: &FeedCursor) -> Result<Vec<FeedEntry>, FeedError> {
        self.validate(cursor)?;
        Ok(self
            .entries
            .iter()
            .filter(|entry| Self::after(entry, cursor) && Self::in_snapshot(cursor, entry))
            .cloned()
            .collect())
    }

    /// Entries before the cursor's position that belong to its pinned
    /// snapshot, in the same stable total order.
    ///
    /// # Errors
    ///
    /// As [`FeedLedger::since`].
    pub fn before(&self, cursor: &FeedCursor) -> Result<Vec<FeedEntry>, FeedError> {
        self.validate(cursor)?;
        Ok(self
            .entries
            .iter()
            .filter(|entry| {
                Self::is_before_frontier(entry, cursor) && Self::in_snapshot(cursor, entry)
            })
            .cloned()
            .collect())
    }
}
