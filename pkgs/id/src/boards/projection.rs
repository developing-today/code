use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{ArtifactHash, Theme};

/// Which layer of a composed board view hid an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewLayer {
    /// The viewer's own filter: personal hides and muted authors.
    Personal,
    /// The board's own signed record: moderation log and tombstones.
    Board,
    /// The serving host's policy set.
    Host,
}

/// One object a composed view hid, and the layer that hid it.
///
/// The object itself stays in the graph — hidden is not erased — so
/// this entry (plus the public moderation log for board-layer hides)
/// is how a viewer, or an auditor, can tell that something is missing
/// from the projection and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HiddenObject {
    /// Hash of the hidden object.
    pub object: ArtifactHash,
    /// Layer that hid it (outermost applicable: board, host, personal).
    pub layer: ViewLayer,
    /// Moderation action or tombstone that hid it, for board-layer
    /// hides; `None` for host and personal hides, which are not signed
    /// graph records.
    pub action: Option<ArtifactHash>,
    /// Recorded reason, when the hiding record carries one.
    pub reason: Option<String>,
}

/// Caller-supplied outer layers for [`BoardGraph::compose_view`].
///
/// Host and personal filters are serving/viewing state, not signed
/// graph objects — a host's policy and a viewer's mutes are theirs to
/// keep — so they are inputs to the projection, never inserted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewLayers {
    /// Objects this viewer has personally hidden.
    pub personal_hidden: BTreeSet<ArtifactHash>,
    /// Authors (canonical Ed25519 public keys) this viewer has muted.
    pub muted_authors: BTreeSet<String>,
    /// Objects the serving host will not serve.
    pub host_hidden: BTreeSet<ArtifactHash>,
}

/// A composed three-layer view of one board. This is a projection of
/// the graph, not a copy of it: every hash names an object that stays
/// in the graph, hidden ones included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardView {
    /// Board root hash this view was composed for.
    pub board: ArtifactHash,
    /// Theme the board applies by reference, resolved from the graph.
    pub theme: Option<Theme>,
    /// Visible current objects: the board root and its content objects,
    /// in deterministic content-hash order.
    pub objects: Vec<ArtifactHash>,
    /// Objects hidden by one of the three layers, with the layer and
    /// (for board-layer hides) the record that hid them.
    pub hidden: Vec<HiddenObject>,
    /// Every moderation action recorded for this board, in
    /// deterministic content-hash order. The moderation log is public:
    /// it is listed whether or not each entry currently hides anything.
    pub moderation_log: Vec<ArtifactHash>,
}

/// Whether a replica holds a source board's full signed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaState {
    /// The replica holds every object of the source board set.
    Complete,
    /// The replica is behind on at least one signed listing version:
    /// its view of who serves the board (and what the set is) is old.
    Stale,
    /// Objects are missing without a listing-version lag (for example
    /// the board root itself has not arrived yet).
    Incomplete,
}

/// Explicit comparison of one replica against a source board set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaStatus {
    /// Board root compared.
    pub board: ArtifactHash,
    /// Complete / stale / incomplete verdict.
    pub state: ReplicaState,
    /// Objects in the source board set.
    pub expected_objects: usize,
    /// Of those, objects this replica already holds.
    pub present_objects: usize,
    /// Source objects this replica does not hold yet.
    pub missing: Vec<ArtifactHash>,
    /// Per host, `(replica version, source version)` where the signed
    /// listing version in this replica lags the source.
    pub listing_version_lag: BTreeMap<String, (u64, u64)>,
}

/// Outcome of one [`BoardGraph::replicate_board_from`] pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicationReport {
    /// Board root replicated.
    pub board: ArtifactHash,
    /// Hashes newly accepted into this graph, in acceptance order.
    pub transferred: Vec<ArtifactHash>,
    /// Exported objects this graph already held.
    pub already_present: usize,
    /// Replica status against the source after the pass.
    pub status: ReplicaStatus,
}

/// One host's local serving policy. This is the host layer, kept
/// deliberately OUTSIDE the signed graph: it is unsigned, host-local,
/// and governs only what this host serves. Delisting a board here
/// stops this host serving it; the board's signed objects stay in the
/// graph and on every other host, portability is replication plus
/// re-listing elsewhere, and no delisting — here or in a signed
/// [`BoardListing`] withdrawal — is a promise of global deletion.
/// Host policy is also separate from board moderation: moderation is
/// the board's signed log inside the graph, this policy is not.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostServingPolicy {
    /// Board roots this host will not serve.
    pub delisted_boards: BTreeSet<ArtifactHash>,
    /// Optional local note (for example the legal basis); plain data.
    pub note: Option<String>,
}

impl HostServingPolicy {
    /// Whether this host serves `board` under its local policy.
    pub fn serves_board(&self, board: &ArtifactHash) -> bool {
        !self.delisted_boards.contains(board)
    }

    /// Delist `board` on this host only. Returns true if it was served
    /// before this call. The signed graph is untouched.
    pub fn delist_board(&mut self, board: &ArtifactHash) -> bool {
        self.delisted_boards.insert(board.clone())
    }
}
