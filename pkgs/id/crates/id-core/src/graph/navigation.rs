//! Native graph navigation API v1 (#7612, decision record on #5831).
//!
//! The record fixes one typed graph over the #5677 note↔board reference
//! model and one canonical interface for walking it. This module is that
//! interface, pure and store-agnostic: a caller builds a [`NavGraph`]
//! snapshot from its stores (board objects, note links, versions) and
//! every surface — CLI, script/API, agent — binds to these functions
//! rather than to its own query code (record Decision 3: parity is
//! semantic, so there is only one implementation to be semantic about).
//!
//! What the record fixes, and where it is held here:
//!
//! - **Typed nodes and edges only** (Decision 1, invariant 1). [`NodeKind`]
//!   and [`EdgeKind`] are the v1 sets; [`NavGraph::insert_edge`] refuses an
//!   edge whose endpoints are not of the kinds that edge type relates.
//! - **A fixed operation set** (Decision 2): [`neighbors`], [`backlinks`],
//!   [`threads_for_object`], [`history_for_object`] and the bounded
//!   [`path`]. The paged operations take one [`Request`] shape and
//!   answer one [`Page`] shape.
//! - **Deterministic order, no duplicates** (Decision 4, invariant 3):
//!   every paged operation orders its results by `(created_at, id)`
//!   ascending, and a [`NavCursor`] resumes strictly after the last key
//!   returned, so a cursor walk never repeats an item and an item added
//!   mid-walk appears only if its key falls after the cursor.
//! - **No silent truncation** (invariant 4): a fan-out or result cap that
//!   cuts a result sets [`Page::truncated`] to the [`Truncation`] naming
//!   the bound and its limit. Running out of page is not truncation: it
//!   is the next cursor.
//! - **Opaque, filter-bound cursors** (invariant 5): a cursor is canonical
//!   CBOR carrying a digest of the operation, target, direction and edge
//!   filter it was minted for; any other request refuses it.
//! - **Visibility** (invariant 6): every operation takes a visibility
//!   predicate. A node the caller cannot see is never returned, never
//!   steers a walk, and as a target is refused exactly as an unknown node
//!   is ([`NavError::NotFound`]), so a refusal does not reveal existence.
//! - **One lineage** (invariant 7): [`history_for_object`] reads the same
//!   `edited-as` and `forked-from` edges any other traversal reads.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use crate::canonical_cbor::{Value, decode_canonical, encode_canonical};

/// Domain separator for request digests bound into cursors.
const REQUEST_DOMAIN: &[u8] = b"id.graph.navigation.request.v1";

/// Cursor format version.
pub const NAV_CURSOR_VERSION: u64 = 1;

/// The v1 node types (record Decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeKind {
    /// A note.
    Note,
    /// A board.
    Board,
    /// A post on a board.
    Post,
    /// A reply to a post.
    Reply,
    /// A chat thread.
    ChatThread,
    /// An edit state of an object.
    Version,
    /// A diverged object.
    Fork,
    /// A person or agent.
    Actor,
}

impl NodeKind {
    /// Every v1 node kind, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Note,
        Self::Board,
        Self::Post,
        Self::Reply,
        Self::ChatThread,
        Self::Version,
        Self::Fork,
        Self::Actor,
    ];

    /// The record's name for this kind.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Board => "board",
            Self::Post => "post",
            Self::Reply => "reply",
            Self::ChatThread => "chat-thread",
            Self::Version => "version",
            Self::Fork => "fork",
            Self::Actor => "actor",
        }
    }

    /// Whether this kind is an object (anything but an actor).
    #[must_use]
    pub const fn is_object(self) -> bool {
        !matches!(self, Self::Actor)
    }
}

/// The v1 edge types (record Decision 1). Every edge is directed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKind {
    /// A note references a note or another object.
    References,
    /// A note is shared as a post.
    SharedAs,
    /// A reply is anchored to a note or to a note version (a passage).
    AnchoredTo,
    /// A board contains a post; a post contains a reply.
    Contains,
    /// An object is authored by an actor.
    AuthoredBy,
    /// An object has an edit state (a version).
    EditedAs,
    /// An object is forked from another object.
    ForkedFrom,
    /// An object mentions an object or an actor.
    Mentions,
}

impl EdgeKind {
    /// Every v1 edge kind, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::References,
        Self::SharedAs,
        Self::AnchoredTo,
        Self::Contains,
        Self::AuthoredBy,
        Self::EditedAs,
        Self::ForkedFrom,
        Self::Mentions,
    ];

    /// The record's name for this kind.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::References => "references",
            Self::SharedAs => "shared-as",
            Self::AnchoredTo => "anchored-to",
            Self::Contains => "contains",
            Self::AuthoredBy => "authored-by",
            Self::EditedAs => "edited-as",
            Self::ForkedFrom => "forked-from",
            Self::Mentions => "mentions",
        }
    }

    /// Whether an edge of this kind may run from a `from` node to a `to`
    /// node (record Decision 1). A relationship the table does not allow
    /// is not traversable until a recorded amendment adds it.
    #[must_use]
    pub const fn allows(self, from: NodeKind, to: NodeKind) -> bool {
        use NodeKind::{Board, Fork, Note, Post, Reply, Version};
        match self {
            Self::References => matches!(from, Note) && to.is_object(),
            Self::SharedAs => matches!((from, to), (Note, Post)),
            Self::AnchoredTo => matches!((from, to), (Reply, Note | Version)),
            Self::Contains => matches!((from, to), (Board, Post) | (Post, Reply)),
            Self::AuthoredBy => from.is_object() && matches!(to, NodeKind::Actor),
            Self::EditedAs => from.is_object() && !matches!(from, Version) && matches!(to, Version),
            Self::ForkedFrom => matches!(from, Fork) && to.is_object(),
            Self::Mentions => from.is_object(),
        }
    }

    /// The edge kinds that point *at* an object from content that refers
    /// to it — what [`backlinks`] walks. Structural edges (`contains`,
    /// `authored-by`, `edited-as`, `forked-from`) are not backlinks; they
    /// are reachable through [`neighbors`] and [`history_for_object`].
    pub const BACKLINKS: [Self; 4] = [
        Self::References,
        Self::SharedAs,
        Self::AnchoredTo,
        Self::Mentions,
    ];
}

/// A node identifier: the caller's stable object or actor id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub String);

impl NodeId {
    /// A node id from any string-like id.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One typed node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The node's id.
    pub id: NodeId,
    /// The node's v1 kind.
    pub kind: NodeKind,
    /// When the node was created, the first half of its order key.
    pub created_at: u64,
}

/// One typed, directed edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// The edge's v1 kind.
    pub kind: EdgeKind,
    /// The node the edge runs from.
    pub from: NodeId,
    /// The node the edge runs to.
    pub to: NodeId,
    /// When the edge was created, the first half of its order key.
    pub created_at: u64,
}

impl Edge {
    /// The edge's id, the second half of its order key: kind, source and
    /// target, which are unique per edge in a [`NavGraph`].
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}:{}>{}", self.kind.name(), self.from.0, self.to.0)
    }
}

/// Why a node or edge was refused by [`NavGraph`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// A node with this id is already in the graph.
    DuplicateNode(NodeId),
    /// An edge of this kind between these nodes is already in the graph.
    DuplicateEdge(String),
    /// An edge names a node the graph does not hold.
    MissingEndpoint(NodeId),
    /// The edge kind does not relate nodes of these kinds (invariant 1).
    UntypedRelationship {
        /// The edge kind.
        kind: EdgeKind,
        /// The source node's kind.
        from: NodeKind,
        /// The target node's kind.
        to: NodeKind,
    },
}

/// A snapshot of the typed graph (record Decision 4: one operation call
/// reads one snapshot).
#[derive(Debug, Clone, Default)]
pub struct NavGraph {
    nodes: BTreeMap<NodeId, Node>,
    edges: Vec<Edge>,
    edge_ids: HashSet<String>,
    outgoing: BTreeMap<NodeId, Vec<usize>>,
    incoming: BTreeMap<NodeId, Vec<usize>>,
}

impl NavGraph {
    /// An empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node.
    ///
    /// # Errors
    ///
    /// [`GraphError::DuplicateNode`] when the id is already present.
    pub fn insert_node(&mut self, node: Node) -> Result<(), GraphError> {
        if self.nodes.contains_key(&node.id) {
            return Err(GraphError::DuplicateNode(node.id));
        }
        self.nodes.insert(node.id.clone(), node);
        Ok(())
    }

    /// Add an edge between two nodes already in the graph.
    ///
    /// # Errors
    ///
    /// [`GraphError::MissingEndpoint`] for an unknown endpoint,
    /// [`GraphError::UntypedRelationship`] when the edge kind does not
    /// relate the endpoints' kinds, and [`GraphError::DuplicateEdge`] for
    /// a second edge of the same kind between the same nodes.
    pub fn insert_edge(&mut self, edge: Edge) -> Result<(), GraphError> {
        let from = self
            .nodes
            .get(&edge.from)
            .ok_or_else(|| GraphError::MissingEndpoint(edge.from.clone()))?
            .kind;
        let to = self
            .nodes
            .get(&edge.to)
            .ok_or_else(|| GraphError::MissingEndpoint(edge.to.clone()))?
            .kind;
        if !edge.kind.allows(from, to) {
            return Err(GraphError::UntypedRelationship {
                kind: edge.kind,
                from,
                to,
            });
        }
        let id = edge.id();
        if !self.edge_ids.insert(id.clone()) {
            return Err(GraphError::DuplicateEdge(id));
        }
        let index = self.edges.len();
        self.outgoing
            .entry(edge.from.clone())
            .or_default()
            .push(index);
        self.incoming
            .entry(edge.to.clone())
            .or_default()
            .push(index);
        self.edges.push(edge);
        Ok(())
    }

    /// The node with this id, if the graph holds it.
    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// How many nodes the graph holds.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// How many edges the graph holds.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// The edges incident to `id` in `direction`, as `(edge, other end)`.
    fn incident(&self, id: &NodeId, direction: Direction) -> Vec<(&Edge, &NodeId)> {
        let mut found = Vec::new();
        if matches!(direction, Direction::Out | Direction::Both) {
            for &index in self.outgoing.get(id).into_iter().flatten() {
                let edge = &self.edges[index];
                found.push((edge, &edge.to));
            }
        }
        if matches!(direction, Direction::In | Direction::Both) {
            for &index in self.incoming.get(id).into_iter().flatten() {
                let edge = &self.edges[index];
                found.push((edge, &edge.from));
            }
        }
        found
    }
}

/// Which way an operation follows edges from its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Edges that start at the target.
    Out,
    /// Edges that end at the target.
    In,
    /// Both.
    #[default]
    Both,
}

impl Direction {
    const fn tag(self) -> u64 {
        match self {
            Self::Out => 0,
            Self::In => 1,
            Self::Both => 2,
        }
    }
}

/// The caps every traversal carries (record Decision 4). Page size is
/// the caller's [`Request::page_size`], clamped to `max_page_size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Most edges examined at any one node.
    pub max_fanout: usize,
    /// Most results one operation produces across all its pages.
    pub max_results: usize,
    /// Deepest a walk goes from its target.
    pub max_depth: usize,
    /// Largest page a request may ask for.
    pub max_page_size: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            max_fanout: 1_000,
            max_results: 10_000,
            max_depth: 6,
            max_page_size: 100,
        }
    }
}

/// A bound that cut a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// [`Bounds::max_fanout`].
    FanOut,
    /// [`Bounds::max_results`].
    Results,
    /// [`Bounds::max_depth`].
    Depth,
}

/// The truncation marker: which bound cut the result, at what limit
/// (invariant 4: never a silent partial result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncation {
    /// The bound that cut the result.
    pub bound: Bound,
    /// Its limit.
    pub limit: usize,
}

/// An opaque continuation token for the next page of one request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavCursor(Vec<u8>);

impl NavCursor {
    fn mint(digest: [u8; 32], key: &OrderKey) -> Self {
        let value = Value::Array(vec![
            Value::UInt(NAV_CURSOR_VERSION),
            Value::Bytes(digest.to_vec()),
            Value::UInt(key.0),
            Value::Text(key.1.clone()),
        ]);
        Self(encode_canonical(&value).unwrap_or_default())
    }

    /// Wraps cursor bytes received from a caller.
    #[must_use]
    pub const fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The opaque bytes to hand to a caller.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The order key this cursor resumes after, if it was minted for a
    /// request with this digest.
    fn resume(&self, digest: [u8; 32]) -> Result<OrderKey, NavError> {
        let Ok(Value::Array(items)) = decode_canonical(&self.0) else {
            return Err(NavError::InvalidCursor);
        };
        let [
            Value::UInt(NAV_CURSOR_VERSION),
            Value::Bytes(for_request),
            Value::UInt(created_at),
            Value::Text(id),
        ] = items.as_slice()
        else {
            return Err(NavError::InvalidCursor);
        };
        if for_request.as_slice() != digest.as_slice() {
            return Err(NavError::InvalidCursor);
        }
        Ok((*created_at, id.clone()))
    }
}

/// `(created_at, id)`: the one total order of every paged operation.
type OrderKey = (u64, String);

/// One paged request (record Decision 2: the same shape for every
/// operation on every surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The node the operation starts from.
    pub target: NodeId,
    /// Only these edge kinds, or every kind the operation walks.
    pub edge_kinds: Option<BTreeSet<EdgeKind>>,
    /// Which way to follow edges, for operations that take a direction.
    pub direction: Direction,
    /// Where the previous page ended.
    pub cursor: Option<NavCursor>,
    /// How many results this page may hold (clamped to
    /// `1..=`[`Bounds::max_page_size`]).
    pub page_size: usize,
}

impl Request {
    /// A first-page request for `target` with no filter, both directions
    /// and `page_size` results.
    #[must_use]
    pub fn new(target: NodeId, page_size: usize) -> Self {
        Self {
            target,
            edge_kinds: None,
            direction: Direction::Both,
            cursor: None,
            page_size,
        }
    }

    /// The same request with an edge-kind filter.
    #[must_use]
    pub fn with_edge_kinds(mut self, kinds: impl IntoIterator<Item = EdgeKind>) -> Self {
        self.edge_kinds = Some(kinds.into_iter().collect());
        self
    }

    /// The same request with a direction.
    #[must_use]
    pub const fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    /// The same request resumed at `cursor`.
    #[must_use]
    pub fn after(mut self, cursor: NavCursor) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// What the request asks, without its page size or cursor: the
    /// digest a cursor is bound to (invariant 5).
    fn digest(&self, operation: Operation) -> [u8; 32] {
        let kinds = self.edge_kinds.as_ref().map_or(Value::Null, |kinds| {
            Value::Array(
                kinds
                    .iter()
                    .map(|kind| Value::Text(kind.name().to_string()))
                    .collect(),
            )
        });
        let bytes = encode_canonical(&Value::Array(vec![
            Value::Text(operation.name().to_string()),
            Value::Text(self.target.0.clone()),
            Value::UInt(self.direction.tag()),
            kinds,
        ]))
        .unwrap_or_default();
        let mut hasher = blake3::Hasher::new();
        hasher.update(REQUEST_DOMAIN);
        hasher.update(&bytes);
        *hasher.finalize().as_bytes()
    }

    fn admits(&self, kind: EdgeKind) -> bool {
        self.edge_kinds
            .as_ref()
            .is_none_or(|kinds| kinds.contains(&kind))
    }
}

/// The paged operations, for cursor binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Neighbors,
    Backlinks,
    Threads,
    History,
}

impl Operation {
    const fn name(self) -> &'static str {
        match self {
            Self::Neighbors => "neighbors",
            Self::Backlinks => "backlinks",
            Self::Threads => "threads-for-object",
            Self::History => "history-for-object",
        }
    }
}

/// Why an operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavError {
    /// The target is unknown or not visible to the caller; the two are
    /// one answer so a refusal never reveals existence (invariant 6).
    NotFound,
    /// The cursor is malformed or was minted for another request
    /// (invariant 5).
    InvalidCursor,
}

/// One result: a typed edge and the node at its far end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The edge followed.
    pub edge: Edge,
    /// The node it leads to (from the operation's point of view).
    pub node: Node,
}

/// One page of results (record Decision 2: one response shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    /// The results, in `(created_at, id)` order.
    pub items: Vec<T>,
    /// The cursor for the next page, or `None` at the end.
    pub next: Option<NavCursor>,
    /// The bound that cut the result, if one did.
    pub truncated: Option<Truncation>,
}

impl<T> Page<T> {
    /// Whether this is the last page.
    #[must_use]
    pub const fn is_end(&self) -> bool {
        self.next.is_none()
    }
}

/// Visibility predicate: whether the caller may see a node.
pub type CanSee<'a> = &'a dyn Fn(&NodeId) -> bool;

/// Resolve and check a request's target.
fn visible_target<'g>(
    graph: &'g NavGraph,
    target: &NodeId,
    can_see: CanSee<'_>,
) -> Result<&'g Node, NavError> {
    match graph.node(target) {
        Some(node) if can_see(target) => Ok(node),
        _ => Err(NavError::NotFound),
    }
}

/// Order, cut and page a candidate list. `candidates` is every visible
/// result of one request; `fanout_cut` says whether a fan-out cap
/// already cut them.
fn paginate<T>(
    mut candidates: Vec<(OrderKey, T)>,
    request: &Request,
    operation: Operation,
    bounds: &Bounds,
    fanout_cut: Option<Truncation>,
) -> Result<Page<T>, NavError> {
    let digest = request.digest(operation);
    let after = request
        .cursor
        .as_ref()
        .map(|cursor| cursor.resume(digest))
        .transpose()?;
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.dedup_by(|a, b| a.0 == b.0);
    let mut truncated = fanout_cut;
    if candidates.len() > bounds.max_results {
        candidates.truncate(bounds.max_results);
        truncated = Some(Truncation {
            bound: Bound::Results,
            limit: bounds.max_results,
        });
    }
    let page_size = request.page_size.clamp(1, bounds.max_page_size.max(1));
    let mut rest = candidates
        .into_iter()
        .filter(|(key, _)| after.as_ref().is_none_or(|after| key > after))
        .peekable();
    let mut items = Vec::new();
    let mut last = None;
    while items.len() < page_size {
        let Some((key, item)) = rest.next() else {
            break;
        };
        last = Some(key);
        items.push(item);
    }
    let next = match (rest.peek(), last) {
        (Some(_), Some(last)) => Some(NavCursor::mint(digest, &last)),
        _ => None,
    };
    Ok(Page {
        items,
        next,
        truncated,
    })
}

/// The visible edges at `id` in `direction` that pass `admits`, capped
/// at the fan-out bound in `(created_at, edge id)` order.
fn visible_steps<'g>(
    graph: &'g NavGraph,
    id: &NodeId,
    direction: Direction,
    admits: &dyn Fn(EdgeKind) -> bool,
    can_see: CanSee<'_>,
    bounds: &Bounds,
) -> (Vec<(&'g Edge, &'g Node)>, Option<Truncation>) {
    let mut steps: Vec<(&Edge, &Node)> = graph
        .incident(id, direction)
        .into_iter()
        .filter(|(edge, _)| admits(edge.kind))
        .filter(|(_, other)| can_see(other))
        .filter_map(|(edge, other)| graph.node(other).map(|node| (edge, node)))
        .collect();
    steps.sort_by_cached_key(|(edge, _)| (edge.created_at, edge.id()));
    let cut = (steps.len() > bounds.max_fanout).then(|| {
        steps.truncate(bounds.max_fanout);
        Truncation {
            bound: Bound::FanOut,
            limit: bounds.max_fanout,
        }
    });
    (steps, cut)
}

/// `neighbors(node, edgeTypes?, direction)`: every visible edge at the
/// target in the request's direction and edge filter, with the node at
/// its far end, ordered by the edge's `(created_at, id)`.
///
/// # Errors
///
/// [`NavError::NotFound`] for an unknown or invisible target,
/// [`NavError::InvalidCursor`] for a cursor from another request.
pub fn neighbors(
    graph: &NavGraph,
    request: &Request,
    can_see: CanSee<'_>,
    bounds: &Bounds,
) -> Result<Page<Step>, NavError> {
    visible_target(graph, &request.target, can_see)?;
    let (steps, cut) = visible_steps(
        graph,
        &request.target,
        request.direction,
        &|kind| request.admits(kind),
        can_see,
        bounds,
    );
    let candidates = steps
        .into_iter()
        .map(|(edge, node)| {
            (
                (edge.created_at, edge.id()),
                Step {
                    edge: edge.clone(),
                    node: node.clone(),
                },
            )
        })
        .collect();
    paginate(candidates, request, Operation::Neighbors, bounds, cut)
}

/// `backlinks(node)`: the visible content that points at the target —
/// incoming [`EdgeKind::BACKLINKS`] edges (narrowed further by the
/// request's edge filter), with the node each comes from. The request's
/// direction is ignored: a backlink is incoming by definition.
///
/// # Errors
///
/// As [`neighbors`].
pub fn backlinks(
    graph: &NavGraph,
    request: &Request,
    can_see: CanSee<'_>,
    bounds: &Bounds,
) -> Result<Page<Step>, NavError> {
    visible_target(graph, &request.target, can_see)?;
    let (steps, cut) = visible_steps(
        graph,
        &request.target,
        Direction::In,
        &|kind| EdgeKind::BACKLINKS.contains(&kind) && request.admits(kind),
        can_see,
        bounds,
    );
    let candidates = steps
        .into_iter()
        .map(|(edge, node)| {
            (
                (edge.created_at, edge.id()),
                Step {
                    edge: edge.clone(),
                    node: node.clone(),
                },
            )
        })
        .collect();
    paginate(candidates, request, Operation::Backlinks, bounds, cut)
}

/// The thread a post or reply belongs to: a post is its own thread; a
/// reply's thread is the visible post that contains it.
fn thread_root<'g>(graph: &'g NavGraph, node: &'g Node, can_see: CanSee<'_>) -> Option<&'g Node> {
    match node.kind {
        NodeKind::Post | NodeKind::ChatThread => Some(node),
        NodeKind::Reply => graph
            .incident(&node.id, Direction::In)
            .into_iter()
            .filter(|(edge, parent)| edge.kind == EdgeKind::Contains && can_see(parent))
            .filter_map(|(_, parent)| graph.node(parent))
            .find(|parent| parent.kind == NodeKind::Post),
        _ => None,
    }
}

/// `threads-for-object(object)`: the visible discussion threads the
/// object takes part in, as their root nodes (posts and chat threads),
/// ordered by the root's `(created_at, id)`. An object takes part in a
/// thread when the thread shares it (`shared-as`), a reply in it is
/// anchored to it (`anchored-to`), a post or reply in it references or
/// mentions it, or it is itself a post or a reply in that thread. The
/// request's direction and edge filter do not apply: the participation
/// rule is the operation.
///
/// # Errors
///
/// As [`neighbors`].
pub fn threads_for_object(
    graph: &NavGraph,
    request: &Request,
    can_see: CanSee<'_>,
    bounds: &Bounds,
) -> Result<Page<Node>, NavError> {
    let target = visible_target(graph, &request.target, can_see)?;
    let mut roots: BTreeMap<NodeId, &Node> = BTreeMap::new();
    if let Some(root) = thread_root(graph, target, can_see) {
        roots.insert(root.id.clone(), root);
    }
    let (outgoing, out_cut) = visible_steps(
        graph,
        &target.id,
        Direction::Out,
        &|kind| kind == EdgeKind::SharedAs,
        can_see,
        bounds,
    );
    let (incoming, in_cut) = visible_steps(
        graph,
        &target.id,
        Direction::In,
        &|kind| {
            matches!(
                kind,
                EdgeKind::AnchoredTo | EdgeKind::References | EdgeKind::Mentions
            )
        },
        can_see,
        bounds,
    );
    for (_, node) in outgoing.into_iter().chain(incoming) {
        if let Some(root) = thread_root(graph, node, can_see) {
            roots.insert(root.id.clone(), root);
        }
    }
    let candidates = roots
        .into_values()
        .map(|root| ((root.created_at, root.id.0.clone()), root.clone()))
        .collect();
    paginate(
        candidates,
        request,
        Operation::Threads,
        bounds,
        out_cut.or(in_cut),
    )
}

/// `history-for-object(object)`: the object's lineage — its edit states
/// (`edited-as` versions) and the visible objects forked from it
/// (incoming `forked-from`) — ordered by the edge's `(created_at, id)`.
/// The request's direction and edge filter do not apply (invariant 7:
/// one lineage, however it is viewed).
///
/// # Errors
///
/// As [`neighbors`].
pub fn history_for_object(
    graph: &NavGraph,
    request: &Request,
    can_see: CanSee<'_>,
    bounds: &Bounds,
) -> Result<Page<Step>, NavError> {
    visible_target(graph, &request.target, can_see)?;
    let (versions, version_cut) = visible_steps(
        graph,
        &request.target,
        Direction::Out,
        &|kind| kind == EdgeKind::EditedAs,
        can_see,
        bounds,
    );
    let (forks, fork_cut) = visible_steps(
        graph,
        &request.target,
        Direction::In,
        &|kind| kind == EdgeKind::ForkedFrom,
        can_see,
        bounds,
    );
    let candidates = versions
        .into_iter()
        .chain(forks)
        .map(|(edge, node)| {
            (
                (edge.created_at, edge.id()),
                Step {
                    edge: edge.clone(),
                    node: node.clone(),
                },
            )
        })
        .collect();
    paginate(
        candidates,
        request,
        Operation::History,
        bounds,
        version_cut.or(fork_cut),
    )
}

/// The answer to a bounded [`path`] query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathResult {
    /// The steps from `from` to `to`, shortest first found, or `None`.
    pub steps: Option<Vec<Step>>,
    /// The bound that cut the search, if one did. `steps: None` with no
    /// truncation means no visible path exists at all.
    pub truncated: Option<Truncation>,
}

/// `path(from, to, maxDepth)`: a shortest visible path from `from` to
/// `to` over edges the request's filter admits, followed in the
/// request's direction, at most `max_depth` steps (clamped to
/// [`Bounds::max_depth`]). The search is breadth first with each node's
/// edges in `(created_at, id)` order, so the same graph always yields
/// the same path. A search the depth, fan-out or result cap stopped
/// says so; it never reports "no path" when a bound, not the graph,
/// ended it.
///
/// # Errors
///
/// [`NavError::NotFound`] when either end is unknown or invisible.
pub fn path(
    graph: &NavGraph,
    request: &Request,
    to: &NodeId,
    max_depth: usize,
    can_see: CanSee<'_>,
    bounds: &Bounds,
) -> Result<PathResult, NavError> {
    let from = visible_target(graph, &request.target, can_see)?;
    visible_target(graph, to, can_see)?;
    if from.id == *to {
        return Ok(PathResult {
            steps: Some(Vec::new()),
            truncated: None,
        });
    }
    let depth_cap = max_depth.min(bounds.max_depth);
    let mut truncated: Option<Truncation> = None;
    let mut parent: BTreeMap<NodeId, Step> = BTreeMap::new();
    let mut seen: BTreeSet<NodeId> = BTreeSet::from([from.id.clone()]);
    let mut queue: VecDeque<(NodeId, usize)> = VecDeque::from([(from.id.clone(), 0)]);
    while let Some((current, depth)) = queue.pop_front() {
        if depth == depth_cap {
            // Anything past here would be deeper than the cap allows.
            if !graph.incident(&current, request.direction).is_empty() {
                truncated.get_or_insert(Truncation {
                    bound: Bound::Depth,
                    limit: depth_cap,
                });
            }
            continue;
        }
        let (steps, cut) = visible_steps(
            graph,
            &current,
            request.direction,
            &|kind| request.admits(kind),
            can_see,
            bounds,
        );
        if let Some(cut) = cut {
            truncated.get_or_insert(cut);
        }
        for (edge, node) in steps {
            if !seen.insert(node.id.clone()) {
                continue;
            }
            parent.insert(
                node.id.clone(),
                Step {
                    edge: edge.clone(),
                    node: node.clone(),
                },
            );
            if node.id == *to {
                return Ok(PathResult {
                    steps: Some(unwind(&parent, &from.id, to)),
                    truncated: None,
                });
            }
            if seen.len() > bounds.max_results {
                return Ok(PathResult {
                    steps: None,
                    truncated: Some(Truncation {
                        bound: Bound::Results,
                        limit: bounds.max_results,
                    }),
                });
            }
            queue.push_back((node.id.clone(), depth + 1));
        }
    }
    Ok(PathResult {
        steps: None,
        truncated,
    })
}

/// Rebuild the path ending at `to` from the breadth-first parent map.
fn unwind(parent: &BTreeMap<NodeId, Step>, from: &NodeId, to: &NodeId) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut at = to.clone();
    while at != *from {
        let Some(step) = parent.get(&at) else {
            break;
        };
        steps.push(step.clone());
        at = if step.edge.to == at {
            step.edge.from.clone()
        } else {
            step.edge.to.clone()
        };
    }
    steps.reverse();
    steps
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn id(text: &str) -> NodeId {
        NodeId::new(text)
    }

    fn node(graph: &mut NavGraph, name: &str, kind: NodeKind, created_at: u64) {
        graph
            .insert_node(Node {
                id: id(name),
                kind,
                created_at,
            })
            .unwrap();
    }

    fn edge(graph: &mut NavGraph, kind: EdgeKind, from: &str, to: &str, created_at: u64) {
        graph
            .insert_edge(Edge {
                kind,
                from: id(from),
                to: id(to),
                created_at,
            })
            .unwrap();
    }

    /// A small fixture: a board with a post sharing a note, a reply
    /// anchored to a version of that note, an author, a fork, and a
    /// second note referencing the first.
    fn fixture() -> NavGraph {
        let mut graph = NavGraph::new();
        node(&mut graph, "alice", NodeKind::Actor, 1);
        node(&mut graph, "board", NodeKind::Board, 2);
        node(&mut graph, "note", NodeKind::Note, 3);
        node(&mut graph, "note-v1", NodeKind::Version, 4);
        node(&mut graph, "note-v2", NodeKind::Version, 5);
        node(&mut graph, "post", NodeKind::Post, 6);
        node(&mut graph, "reply", NodeKind::Reply, 7);
        node(&mut graph, "other", NodeKind::Note, 8);
        node(&mut graph, "fork", NodeKind::Fork, 9);
        node(&mut graph, "secret", NodeKind::Note, 10);
        edge(&mut graph, EdgeKind::AuthoredBy, "note", "alice", 3);
        edge(&mut graph, EdgeKind::EditedAs, "note", "note-v1", 4);
        edge(&mut graph, EdgeKind::EditedAs, "note", "note-v2", 5);
        edge(&mut graph, EdgeKind::SharedAs, "note", "post", 6);
        edge(&mut graph, EdgeKind::Contains, "board", "post", 6);
        edge(&mut graph, EdgeKind::Contains, "post", "reply", 7);
        edge(&mut graph, EdgeKind::AnchoredTo, "reply", "note-v1", 7);
        edge(&mut graph, EdgeKind::References, "other", "note", 8);
        edge(&mut graph, EdgeKind::ForkedFrom, "fork", "note", 9);
        edge(&mut graph, EdgeKind::References, "secret", "note", 10);
        graph
    }

    fn everyone(_: &NodeId) -> bool {
        true
    }

    fn not_secret(node: &NodeId) -> bool {
        node.as_str() != "secret"
    }

    fn names(steps: &[Step]) -> Vec<&str> {
        steps.iter().map(|step| step.node.id.as_str()).collect()
    }

    /// Walk every page of an operation and return all items in order.
    fn walk(
        op: fn(&NavGraph, &Request, CanSee<'_>, &Bounds) -> Result<Page<Step>, NavError>,
        graph: &NavGraph,
        request: Request,
        can_see: CanSee<'_>,
        bounds: &Bounds,
    ) -> Vec<Step> {
        let mut all = Vec::new();
        let mut request = request;
        loop {
            let page = op(graph, &request, can_see, bounds).unwrap();
            all.extend(page.items);
            match page.next {
                Some(cursor) => request = request.after(cursor),
                None => return all,
            }
        }
    }

    #[test]
    fn edges_are_typed_and_untyped_relationships_are_refused() {
        // Oracle: #5831 record invariant 1 — typed edges only; a
        // relationship no v1 edge type names is refused.
        let mut graph = fixture();
        let refused = graph.insert_edge(Edge {
            kind: EdgeKind::Contains,
            from: id("note"),
            to: id("reply"),
            created_at: 11,
        });
        assert_eq!(
            refused,
            Err(GraphError::UntypedRelationship {
                kind: EdgeKind::Contains,
                from: NodeKind::Note,
                to: NodeKind::Reply,
            })
        );
        assert_eq!(
            graph.insert_edge(Edge {
                kind: EdgeKind::AuthoredBy,
                from: id("alice"),
                to: id("alice"),
                created_at: 11,
            }),
            Err(GraphError::UntypedRelationship {
                kind: EdgeKind::AuthoredBy,
                from: NodeKind::Actor,
                to: NodeKind::Actor,
            })
        );
        assert_eq!(
            graph.insert_edge(Edge {
                kind: EdgeKind::References,
                from: id("note"),
                to: id("missing"),
                created_at: 11,
            }),
            Err(GraphError::MissingEndpoint(id("missing")))
        );
        assert!(matches!(
            graph.insert_edge(Edge {
                kind: EdgeKind::References,
                from: id("other"),
                to: id("note"),
                created_at: 12,
            }),
            Err(GraphError::DuplicateEdge(_))
        ));
        assert!(matches!(
            graph.insert_node(Node {
                id: id("note"),
                kind: NodeKind::Note,
                created_at: 1,
            }),
            Err(GraphError::DuplicateNode(_))
        ));
        assert_eq!(graph.edge_count(), 10);
        // Every kind pair the table allows is accepted, every other one
        // refused — the table is the whole rule.
        for kind in EdgeKind::ALL {
            let allowed = NodeKind::ALL
                .iter()
                .flat_map(|from| NodeKind::ALL.iter().map(move |to| (*from, *to)))
                .filter(|(from, to)| kind.allows(*from, *to))
                .count();
            assert!(allowed > 0, "{} relates nothing", kind.name());
        }
    }

    #[test]
    fn neighbors_follow_direction_and_filter_in_order() {
        let graph = fixture();
        let bounds = Bounds::default();
        let out = neighbors(
            &graph,
            &Request::new(id("note"), 50).with_direction(Direction::Out),
            &everyone,
            &bounds,
        )
        .unwrap();
        // Oracle: #5831 record Decision 4 — (created_at, id) ascending.
        assert_eq!(
            names(&out.items),
            vec!["alice", "note-v1", "note-v2", "post"]
        );
        assert!(out.is_end());
        assert_eq!(out.truncated, None);
        let into = neighbors(
            &graph,
            &Request::new(id("note"), 50)
                .with_direction(Direction::In)
                .with_edge_kinds([EdgeKind::References]),
            &everyone,
            &bounds,
        )
        .unwrap();
        assert_eq!(names(&into.items), vec!["other", "secret"]);
        let both = neighbors(&graph, &Request::new(id("note"), 50), &everyone, &bounds).unwrap();
        assert_eq!(both.items.len(), 7);
    }

    #[test]
    fn cursor_walks_are_complete_ordered_and_duplicate_free() {
        // Oracle: #5831 record invariant 3 — a cursor walk never returns
        // an item twice, and its pages concatenate to the one ordered
        // result.
        let graph = fixture();
        let bounds = Bounds::default();
        let whole = neighbors(&graph, &Request::new(id("note"), 50), &everyone, &bounds)
            .unwrap()
            .items;
        for page_size in 1..=4 {
            let walked = walk(
                neighbors,
                &graph,
                Request::new(id("note"), page_size),
                &everyone,
                &bounds,
            );
            assert_eq!(walked, whole, "page size {page_size}");
        }
    }

    #[test]
    fn an_item_added_mid_walk_appears_only_after_the_cursor() {
        let mut graph = fixture();
        let bounds = Bounds::default();
        let first = neighbors(
            &graph,
            &Request::new(id("note"), 3).with_direction(Direction::In),
            &everyone,
            &bounds,
        )
        .unwrap();
        // In edges of note, in order: other(8), fork(9), secret(10).
        assert_eq!(names(&first.items), vec!["other", "fork", "secret"]);
        let cursor = first.next.clone();
        assert!(cursor.is_none());
        // Re-walk with a smaller page and add edges between pages: one
        // keyed before the cursor (never shown) and one after (shown).
        let first = neighbors(
            &graph,
            &Request::new(id("note"), 1).with_direction(Direction::In),
            &everyone,
            &bounds,
        )
        .unwrap();
        assert_eq!(names(&first.items), vec!["other"]);
        node(&mut graph, "early", NodeKind::Note, 2);
        node(&mut graph, "late", NodeKind::Note, 20);
        edge(&mut graph, EdgeKind::Mentions, "early", "note", 2);
        edge(&mut graph, EdgeKind::Mentions, "late", "note", 20);
        let rest = walk(
            neighbors,
            &graph,
            Request::new(id("note"), 1)
                .with_direction(Direction::In)
                .after(first.next.unwrap()),
            &everyone,
            &bounds,
        );
        assert_eq!(names(&rest), vec!["fork", "secret", "late"]);
    }

    #[test]
    fn cursors_are_bound_to_their_request() {
        // Oracle: #5831 record invariant 5 — a cursor minted for one
        // filter set is invalid for another.
        let graph = fixture();
        let bounds = Bounds::default();
        let page = neighbors(&graph, &Request::new(id("note"), 1), &everyone, &bounds).unwrap();
        let cursor = page.next.unwrap();
        let other_filter = Request::new(id("note"), 1)
            .with_edge_kinds([EdgeKind::EditedAs])
            .after(cursor.clone());
        assert_eq!(
            neighbors(&graph, &other_filter, &everyone, &bounds),
            Err(NavError::InvalidCursor)
        );
        let other_direction = Request::new(id("note"), 1)
            .with_direction(Direction::Out)
            .after(cursor.clone());
        assert_eq!(
            neighbors(&graph, &other_direction, &everyone, &bounds),
            Err(NavError::InvalidCursor)
        );
        let other_operation = Request::new(id("note"), 1).after(cursor);
        assert_eq!(
            backlinks(&graph, &other_operation, &everyone, &bounds),
            Err(NavError::InvalidCursor)
        );
        let garbage = Request::new(id("note"), 1).after(NavCursor::from_bytes(vec![1, 2, 3]));
        assert_eq!(
            neighbors(&graph, &garbage, &everyone, &bounds),
            Err(NavError::InvalidCursor)
        );
    }

    #[test]
    fn caps_mark_truncation_and_page_ends_do_not() {
        // Oracle: #5831 record invariant 4 — every bounded result says
        // which bound cut it; running out of page is a cursor, not a cut.
        let graph = fixture();
        let fanout = Bounds {
            max_fanout: 2,
            ..Bounds::default()
        };
        let page = neighbors(&graph, &Request::new(id("note"), 50), &everyone, &fanout).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(
            page.truncated,
            Some(Truncation {
                bound: Bound::FanOut,
                limit: 2
            })
        );
        let results = Bounds {
            max_results: 3,
            ..Bounds::default()
        };
        let page = neighbors(&graph, &Request::new(id("note"), 50), &everyone, &results).unwrap();
        assert_eq!(page.items.len(), 3);
        assert_eq!(
            page.truncated,
            Some(Truncation {
                bound: Bound::Results,
                limit: 3
            })
        );
        let paged = neighbors(
            &graph,
            &Request::new(id("note"), 2),
            &everyone,
            &Bounds::default(),
        )
        .unwrap();
        assert_eq!(paged.truncated, None);
        assert!(!paged.is_end());
        // Page size is clamped to the bound and never zero.
        let tiny = Bounds {
            max_page_size: 2,
            ..Bounds::default()
        };
        let clamped = neighbors(&graph, &Request::new(id("note"), 99), &everyone, &tiny).unwrap();
        assert_eq!(clamped.items.len(), 2);
        let zero = neighbors(&graph, &Request::new(id("note"), 0), &everyone, &tiny).unwrap();
        assert_eq!(zero.items.len(), 1);
    }

    #[test]
    fn invisible_nodes_never_appear_or_reveal_themselves() {
        // Oracle: #5831 record invariant 6 — a walk never reveals a node
        // or edge the caller cannot see, and an invisible target is
        // refused exactly as an unknown one.
        let graph = fixture();
        let bounds = Bounds::default();
        let page = backlinks(&graph, &Request::new(id("note"), 50), &not_secret, &bounds).unwrap();
        assert_eq!(names(&page.items), vec!["other"]);
        assert_eq!(
            neighbors(&graph, &Request::new(id("secret"), 5), &not_secret, &bounds),
            Err(NavError::NotFound)
        );
        assert_eq!(
            neighbors(
                &graph,
                &Request::new(id("missing"), 5),
                &not_secret,
                &bounds
            ),
            Err(NavError::NotFound)
        );
        // A hidden node is not a path end and not a path waypoint.
        let to_secret = path(
            &graph,
            &Request::new(id("other"), 1),
            &id("secret"),
            6,
            &not_secret,
            &bounds,
        );
        assert_eq!(to_secret, Err(NavError::NotFound));
        let mut graph = graph;
        node(&mut graph, "island", NodeKind::Note, 30);
        edge(&mut graph, EdgeKind::References, "secret", "island", 30);
        let around = path(
            &graph,
            &Request::new(id("other"), 1),
            &id("island"),
            6,
            &not_secret,
            &bounds,
        )
        .unwrap();
        assert_eq!(around.steps, None);
        assert_eq!(around.truncated, None);
    }

    #[test]
    fn backlinks_are_incoming_content_edges_only() {
        let graph = fixture();
        let bounds = Bounds::default();
        // note's incoming edges: references from other and secret,
        // forked-from from fork. The fork is lineage, not a backlink.
        let page = backlinks(&graph, &Request::new(id("note"), 50), &everyone, &bounds).unwrap();
        assert_eq!(names(&page.items), vec!["other", "secret"]);
        // The version a reply is anchored to has the reply as backlink.
        let page = backlinks(&graph, &Request::new(id("note-v1"), 50), &everyone, &bounds).unwrap();
        assert_eq!(names(&page.items), vec!["reply"]);
        // The request's filter narrows the backlink kinds further.
        let page = backlinks(
            &graph,
            &Request::new(id("note"), 50).with_edge_kinds([EdgeKind::Mentions]),
            &everyone,
            &bounds,
        )
        .unwrap();
        assert!(page.items.is_empty());
        assert!(page.is_end());
    }

    #[test]
    fn threads_for_object_finds_the_discussions_it_takes_part_in() {
        let mut graph = fixture();
        let bounds = Bounds::default();
        // The note is shared as the post: one thread.
        let page =
            threads_for_object(&graph, &Request::new(id("note"), 50), &everyone, &bounds).unwrap();
        let roots: Vec<&str> = page.items.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(roots, vec!["post"]);
        // The version is anchored to by a reply in the post's thread.
        let page = threads_for_object(&graph, &Request::new(id("note-v1"), 50), &everyone, &bounds)
            .unwrap();
        let roots: Vec<&str> = page.items.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(roots, vec!["post"]);
        // A reply's thread is its post; a second post mentioning the
        // note adds a second thread, in order.
        let page =
            threads_for_object(&graph, &Request::new(id("reply"), 50), &everyone, &bounds).unwrap();
        assert_eq!(page.items[0].id, id("post"));
        node(&mut graph, "post-2", NodeKind::Post, 40);
        edge(&mut graph, EdgeKind::Mentions, "post-2", "note", 40);
        let page =
            threads_for_object(&graph, &Request::new(id("note"), 50), &everyone, &bounds).unwrap();
        let roots: Vec<&str> = page.items.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(roots, vec!["post", "post-2"]);
        // An actor takes part in no thread.
        let page =
            threads_for_object(&graph, &Request::new(id("alice"), 50), &everyone, &bounds).unwrap();
        assert!(page.items.is_empty());
    }

    #[test]
    fn history_reads_the_one_lineage() {
        // Oracle: #5831 record invariant 7 — history-for-object reads the
        // same edited-as / forked-from edges as any other traversal.
        let graph = fixture();
        let bounds = Bounds::default();
        let history = walk(
            history_for_object,
            &graph,
            Request::new(id("note"), 1),
            &everyone,
            &bounds,
        );
        assert_eq!(names(&history), vec!["note-v1", "note-v2", "fork"]);
        let lineage_edges = neighbors(
            &graph,
            &Request::new(id("note"), 50)
                .with_edge_kinds([EdgeKind::EditedAs, EdgeKind::ForkedFrom]),
            &everyone,
            &bounds,
        )
        .unwrap();
        assert_eq!(
            history
                .iter()
                .map(|step| step.edge.clone())
                .collect::<Vec<_>>(),
            lineage_edges
                .items
                .iter()
                .map(|step| step.edge.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn path_is_shortest_deterministic_and_bounded() {
        let graph = fixture();
        let bounds = Bounds::default();
        // other -> note -> post -> reply over any direction.
        let found = path(
            &graph,
            &Request::new(id("other"), 1),
            &id("reply"),
            6,
            &everyone,
            &bounds,
        )
        .unwrap();
        let steps = found.steps.unwrap();
        assert_eq!(names(&steps), vec!["note", "note-v1", "reply"]);
        assert_eq!(found.truncated, None);
        // Same query, same path, every time.
        for _ in 0..3 {
            let again = path(
                &graph,
                &Request::new(id("other"), 1),
                &id("reply"),
                6,
                &everyone,
                &bounds,
            )
            .unwrap();
            assert_eq!(again.steps.as_deref(), Some(steps.as_slice()));
        }
        // Following edges forward only, other cannot reach reply.
        let forward = path(
            &graph,
            &Request::new(id("other"), 1).with_direction(Direction::Out),
            &id("reply"),
            6,
            &everyone,
            &bounds,
        )
        .unwrap();
        assert_eq!(forward.steps, None);
        // A depth cap that stops the search says so instead of "no path".
        let shallow = path(
            &graph,
            &Request::new(id("other"), 1),
            &id("reply"),
            2,
            &everyone,
            &bounds,
        )
        .unwrap();
        assert_eq!(shallow.steps, None);
        assert_eq!(
            shallow.truncated,
            Some(Truncation {
                bound: Bound::Depth,
                limit: 2
            })
        );
        // The bounds cap the caller's depth.
        let capped = path(
            &graph,
            &Request::new(id("other"), 1),
            &id("reply"),
            99,
            &everyone,
            &Bounds {
                max_depth: 1,
                ..Bounds::default()
            },
        )
        .unwrap();
        assert_eq!(capped.truncated.map(|cut| cut.limit), Some(1));
        // A node is a zero-step path to itself.
        let same = path(
            &graph,
            &Request::new(id("note"), 1),
            &id("note"),
            0,
            &everyone,
            &bounds,
        )
        .unwrap();
        assert_eq!(same.steps, Some(Vec::new()));
    }
}
