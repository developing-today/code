//! The wire form of the native graph navigation API (#7612 slice 2).
//!
//! The #5831 record asks for semantic parity across the CLI,
//! script/API and agent surfaces: the same operations, filters,
//! ordering and bounds, with only presentation differing. Parity holds
//! by construction when every surface speaks one request shape and one
//! answer shape and hands them to one executor. This module is that
//! shape and that executor:
//!
//! - [`WireGraph`]: a typed graph snapshot as JSON (nodes, edges, and the
//!   ids the viewer may not see), built into a [`NavGraph`] through the
//!   same typed inserts every other caller uses.
//! - [`WireRequest`] / [`WireResponse`]: one operation call and its
//!   answer. Kinds and directions travel by their record names; a cursor
//!   travels as base64url text and is opaque to every surface.
//! - [`execute`]: runs a request against [`super::navigation`] and
//!   nothing else, so no surface carries its own query code.
//! - [`execute_json`]: the script/API entry point, JSON in and JSON out.
//! - [`render_text`]: the terminal presentation of an answer. It is the
//!   only place a surface differs, and it reads the answer, never the
//!   graph.
//!
//! A refusal is an answer, not a transport error: an unknown or
//! invisible target, a cursor minted for another request, and a request
//! the executor cannot read all come back as [`WireResponse::Error`]
//! with a stable code.

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use super::navigation::{
    Bound, Bounds, Direction, Edge, EdgeKind, GraphError, NavCursor, NavError, NavGraph, Node,
    NodeId, NodeKind, Page, PathResult, Request, Step, Truncation, backlinks, history_for_object,
    neighbors, path, threads_for_object,
};

/// The page size a request gets when it names none.
pub const DEFAULT_PAGE_SIZE: usize = 20;

/// Looks up a node kind by its record name.
#[must_use]
pub fn node_kind(name: &str) -> Option<NodeKind> {
    NodeKind::ALL.into_iter().find(|kind| kind.name() == name)
}

/// Looks up an edge kind by its record name.
#[must_use]
pub fn edge_kind(name: &str) -> Option<EdgeKind> {
    EdgeKind::ALL.into_iter().find(|kind| kind.name() == name)
}

/// Looks up a direction by its wire name (`out`, `in` or `both`).
#[must_use]
pub fn direction(name: &str) -> Option<Direction> {
    match name {
        "out" => Some(Direction::Out),
        "in" => Some(Direction::In),
        "both" => Some(Direction::Both),
        _ => None,
    }
}

/// One node of a [`WireGraph`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireNode {
    /// The node's id.
    pub id: String,
    /// The node kind's record name.
    pub kind: String,
    /// Creation time, the first half of the ordering key.
    pub created_at: u64,
}

/// One edge of a [`WireGraph`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEdge {
    /// The edge kind's record name.
    pub kind: String,
    /// The source node's id.
    pub from: String,
    /// The target node's id.
    pub to: String,
    /// Creation time, the first half of the ordering key.
    pub created_at: u64,
}

/// A typed graph snapshot as JSON, with the viewer's hidden set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireGraph {
    /// The nodes.
    #[serde(default)]
    pub nodes: Vec<WireNode>,
    /// The edges.
    #[serde(default)]
    pub edges: Vec<WireEdge>,
    /// Ids of nodes this snapshot's viewer may not see.
    #[serde(default)]
    pub hidden: BTreeSet<String>,
}

/// Why a [`WireGraph`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireGraphError {
    /// A node names a kind outside the v1 set.
    UnknownNodeKind(String),
    /// An edge names a kind outside the v1 set.
    UnknownEdgeKind(String),
    /// The typed graph refused a node or edge.
    Graph(GraphError),
}

impl std::fmt::Display for WireGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownNodeKind(kind) => write!(f, "unknown node kind `{kind}`"),
            Self::UnknownEdgeKind(kind) => write!(f, "unknown edge kind `{kind}`"),
            Self::Graph(err) => write!(f, "graph refused an insert: {err:?}"),
        }
    }
}

impl std::error::Error for WireGraphError {}

impl WireGraph {
    /// Builds the typed graph through [`NavGraph`]'s checked inserts.
    ///
    /// # Errors
    ///
    /// [`WireGraphError`] for an unknown kind or an insert the typed
    /// graph refuses (duplicate, missing endpoint, untyped relationship).
    pub fn build(&self) -> Result<NavGraph, WireGraphError> {
        let mut graph = NavGraph::new();
        for node in &self.nodes {
            let kind = node_kind(&node.kind)
                .ok_or_else(|| WireGraphError::UnknownNodeKind(node.kind.clone()))?;
            graph
                .insert_node(Node {
                    id: NodeId::new(node.id.clone()),
                    kind,
                    created_at: node.created_at,
                })
                .map_err(WireGraphError::Graph)?;
        }
        for edge in &self.edges {
            let kind = edge_kind(&edge.kind)
                .ok_or_else(|| WireGraphError::UnknownEdgeKind(edge.kind.clone()))?;
            graph
                .insert_edge(Edge {
                    kind,
                    from: NodeId::new(edge.from.clone()),
                    to: NodeId::new(edge.to.clone()),
                    created_at: edge.created_at,
                })
                .map_err(WireGraphError::Graph)?;
        }
        Ok(graph)
    }

    /// Whether this snapshot's viewer may see `id`.
    #[must_use]
    pub fn can_see(&self, id: &NodeId) -> bool {
        !self.hidden.contains(id.as_str())
    }
}

/// The five operations of the record, by wire name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireOp {
    /// [`neighbors`].
    Neighbors,
    /// [`backlinks`].
    Backlinks,
    /// [`threads_for_object`].
    ThreadsForObject,
    /// [`history_for_object`].
    HistoryForObject,
    /// [`path`].
    Path,
}

/// One operation call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireRequest {
    /// The operation.
    pub op: WireOp,
    /// The node the operation starts from.
    pub target: String,
    /// Edge kinds to follow, by record name; absent means every kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_kinds: Option<Vec<String>>,
    /// `out`, `in` or `both`; absent means `both`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// A cursor from an earlier answer to this same request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Items per page; absent means [`DEFAULT_PAGE_SIZE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<usize>,
    /// For `path`: the node the path ends at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// For `path`: the deepest walk the caller wants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
}

impl WireRequest {
    /// A request for `op` from `target` with every default.
    #[must_use]
    pub fn new(op: WireOp, target: impl Into<String>) -> Self {
        Self {
            op,
            target: target.into(),
            edge_kinds: None,
            direction: None,
            cursor: None,
            page_size: None,
            to: None,
            max_depth: None,
        }
    }
}

/// A node as an answer carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireNodeOut {
    /// The node's id.
    pub id: String,
    /// The node kind's record name.
    pub kind: String,
    /// Creation time.
    pub created_at: u64,
}

impl From<&Node> for WireNodeOut {
    fn from(node: &Node) -> Self {
        Self {
            id: node.id.as_str().to_owned(),
            kind: node.kind.name().to_owned(),
            created_at: node.created_at,
        }
    }
}

/// One step as an answer carries it: the edge and its far node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireStep {
    /// The edge kind's record name.
    pub edge: String,
    /// The edge's source.
    pub from: String,
    /// The edge's target.
    pub to: String,
    /// The edge's creation time.
    pub created_at: u64,
    /// The node at the far end.
    pub node: WireNodeOut,
}

impl From<&Step> for WireStep {
    fn from(step: &Step) -> Self {
        Self {
            edge: step.edge.kind.name().to_owned(),
            from: step.edge.from.as_str().to_owned(),
            to: step.edge.to.as_str().to_owned(),
            created_at: step.edge.created_at,
            node: WireNodeOut::from(&step.node),
        }
    }
}

/// An item of a page: a step, or a bare node for `threads-for-object`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WireItem {
    /// A step.
    Step(WireStep),
    /// A node.
    Node(WireNodeOut),
}

/// The truncation marker as an answer carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireTruncation {
    /// The bound that cut the result.
    pub bound: WireBound,
    /// Its limit.
    pub limit: usize,
}

/// A bound by wire name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireBound {
    /// [`Bound::FanOut`].
    FanOut,
    /// [`Bound::Results`].
    Results,
    /// [`Bound::Depth`].
    Depth,
}

impl From<Truncation> for WireTruncation {
    fn from(cut: Truncation) -> Self {
        Self {
            bound: match cut.bound {
                Bound::FanOut => WireBound::FanOut,
                Bound::Results => WireBound::Results,
                Bound::Depth => WireBound::Depth,
            },
            limit: cut.limit,
        }
    }
}

/// The stable refusal codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireErrorCode {
    /// The target (or `to`) is unknown or not visible: one answer for
    /// both, so a refusal never reveals existence.
    NotFound,
    /// The cursor is malformed or belongs to another request.
    InvalidCursor,
    /// The request names an unknown edge kind or direction, or a `path`
    /// request has no `to`.
    BadRequest,
}

/// One operation's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum WireResponse {
    /// A page of a paged operation.
    Page {
        /// The items, in `(created_at, id)` order.
        items: Vec<WireItem>,
        /// The cursor for the next page; absent at the end.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
        /// Set when a bound cut the result.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<WireTruncation>,
    },
    /// A `path` answer.
    Path {
        /// The steps from target to `to`; absent when no path was found.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<Vec<WireStep>>,
        /// Set when a bound, not the graph, ended the search.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<WireTruncation>,
    },
    /// A refusal.
    Error {
        /// The stable code.
        code: WireErrorCode,
        /// A human-readable detail, safe to show the caller.
        message: String,
    },
}

impl WireResponse {
    fn error(code: WireErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }

    fn from_nav_error(err: &NavError) -> Self {
        match err {
            NavError::NotFound => Self::error(WireErrorCode::NotFound, "no such node"),
            NavError::InvalidCursor => Self::error(
                WireErrorCode::InvalidCursor,
                "the cursor does not belong to this request",
            ),
        }
    }
}

fn page_of<T>(page: &Page<T>, item: impl Fn(&T) -> WireItem) -> WireResponse {
    WireResponse::Page {
        items: page.items.iter().map(item).collect(),
        next: page
            .next
            .as_ref()
            .map(|cursor| URL_SAFE_NO_PAD.encode(cursor.as_bytes())),
        truncated: page.truncated.map(WireTruncation::from),
    }
}

fn path_of(result: &PathResult) -> WireResponse {
    WireResponse::Path {
        steps: result
            .steps
            .as_ref()
            .map(|steps| steps.iter().map(WireStep::from).collect()),
        truncated: result.truncated.map(WireTruncation::from),
    }
}

/// Builds the core request a wire request names.
fn core_request(request: &WireRequest) -> Result<Request, WireResponse> {
    let mut core = Request::new(
        NodeId::new(request.target.clone()),
        request.page_size.unwrap_or(DEFAULT_PAGE_SIZE),
    );
    if let Some(names) = &request.edge_kinds {
        let mut kinds = Vec::with_capacity(names.len());
        for name in names {
            let kind = edge_kind(name).ok_or_else(|| {
                WireResponse::error(
                    WireErrorCode::BadRequest,
                    format!("unknown edge kind `{name}`"),
                )
            })?;
            kinds.push(kind);
        }
        core = core.with_edge_kinds(kinds);
    }
    if let Some(name) = &request.direction {
        let dir = direction(name).ok_or_else(|| {
            WireResponse::error(
                WireErrorCode::BadRequest,
                format!("unknown direction `{name}`"),
            )
        })?;
        core = core.with_direction(dir);
    }
    if let Some(text) = &request.cursor {
        let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| {
            WireResponse::error(WireErrorCode::InvalidCursor, "the cursor is not readable")
        })?;
        core = core.after(NavCursor::from_bytes(bytes));
    }
    Ok(core)
}

/// Runs one request against the navigation core. Every surface calls
/// this and nothing else.
#[must_use]
pub fn execute(
    graph: &NavGraph,
    request: &WireRequest,
    can_see: &dyn Fn(&NodeId) -> bool,
    bounds: &Bounds,
) -> WireResponse {
    let core = match core_request(request) {
        Ok(core) => core,
        Err(answer) => return answer,
    };
    let answer = match request.op {
        WireOp::Neighbors => {
            neighbors(graph, &core, can_see, bounds).map(|p| page_of(&p, step_item))
        }
        WireOp::Backlinks => {
            backlinks(graph, &core, can_see, bounds).map(|p| page_of(&p, step_item))
        }
        WireOp::HistoryForObject => {
            history_for_object(graph, &core, can_see, bounds).map(|p| page_of(&p, step_item))
        }
        WireOp::ThreadsForObject => threads_for_object(graph, &core, can_see, bounds)
            .map(|p| page_of(&p, |node| WireItem::Node(WireNodeOut::from(node)))),
        WireOp::Path => {
            let Some(to) = &request.to else {
                return WireResponse::error(WireErrorCode::BadRequest, "path needs `to`");
            };
            let depth = request.max_depth.unwrap_or(bounds.max_depth);
            path(
                graph,
                &core,
                &NodeId::new(to.clone()),
                depth,
                can_see,
                bounds,
            )
            .map(|r| path_of(&r))
        }
    };
    answer.unwrap_or_else(|err| WireResponse::from_nav_error(&err))
}

fn step_item(step: &Step) -> WireItem {
    WireItem::Step(WireStep::from(step))
}

/// The script/API surface: a JSON [`WireRequest`] in, a JSON
/// [`WireResponse`] out. Unreadable JSON is a `bad-request` answer.
#[must_use]
pub fn execute_json(
    graph: &NavGraph,
    request_json: &str,
    can_see: &dyn Fn(&NodeId) -> bool,
    bounds: &Bounds,
) -> String {
    let answer = match serde_json::from_str::<WireRequest>(request_json) {
        Ok(request) => execute(graph, &request, can_see, bounds),
        Err(err) => WireResponse::error(WireErrorCode::BadRequest, err.to_string()),
    };
    to_json(&answer)
}

/// Serializes an answer. The wire types hold only strings, numbers and
/// enums, so serialization cannot fail; the fallback keeps the function
/// total without a panic path.
#[must_use]
pub fn to_json(answer: &WireResponse) -> String {
    serde_json::to_string(answer).unwrap_or_else(|_| {
        String::from(r#"{"result":"error","code":"bad-request","message":"unserializable"}"#)
    })
}

fn bound_name(bound: WireBound) -> &'static str {
    match bound {
        WireBound::FanOut => "fan-out",
        WireBound::Results => "results",
        WireBound::Depth => "depth",
    }
}

fn step_line(step: &WireStep) -> String {
    format!(
        "{} --{}--> {}  [{} {} @{}]",
        step.from, step.edge, step.to, step.node.kind, step.node.id, step.created_at
    )
}

/// The terminal presentation of an answer: one line per item, then the
/// truncation marker and the next cursor when present. It reads only
/// the answer, so it cannot change what an operation means.
#[must_use]
pub fn render_text(answer: &WireResponse) -> String {
    let mut lines = Vec::new();
    match answer {
        WireResponse::Page {
            items,
            next,
            truncated,
        } => {
            if items.is_empty() {
                lines.push(String::from("(no results)"));
            }
            for item in items {
                lines.push(match item {
                    WireItem::Step(step) => step_line(step),
                    WireItem::Node(node) => {
                        format!("{} {} @{}", node.kind, node.id, node.created_at)
                    }
                });
            }
            if let Some(cut) = truncated {
                lines.push(format!(
                    "truncated: {} bound at {}",
                    bound_name(cut.bound),
                    cut.limit
                ));
            }
            if let Some(cursor) = next {
                lines.push(format!("next: {cursor}"));
            }
        }
        WireResponse::Path { steps, truncated } => {
            match steps {
                Some(steps) if steps.is_empty() => lines.push(String::from("(same node)")),
                Some(steps) => lines.extend(steps.iter().map(step_line)),
                None => lines.push(String::from("(no path)")),
            }
            if let Some(cut) = truncated {
                lines.push(format!(
                    "truncated: {} bound at {}",
                    bound_name(cut.bound),
                    cut.limit
                ));
            }
        }
        WireResponse::Error { code, message } => {
            let code = match code {
                WireErrorCode::NotFound => "not-found",
                WireErrorCode::InvalidCursor => "invalid-cursor",
                WireErrorCode::BadRequest => "bad-request",
            };
            lines.push(format!("error ({code}): {message}"));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    //! Oracle: the #5831 decision record. These tests hold the wire
    //! layer to the core: every answer `execute` gives must equal what
    //! the core function gives for the same request.

    use super::*;

    fn fixture() -> WireGraph {
        let json = r#"{
            "nodes": [
                {"id": "note", "kind": "note", "created_at": 1},
                {"id": "board", "kind": "board", "created_at": 2},
                {"id": "post", "kind": "post", "created_at": 3},
                {"id": "reply", "kind": "reply", "created_at": 4},
                {"id": "chat", "kind": "chat-thread", "created_at": 5},
                {"id": "v2", "kind": "version", "created_at": 6},
                {"id": "fork", "kind": "fork", "created_at": 7},
                {"id": "alice", "kind": "actor", "created_at": 0},
                {"id": "secret", "kind": "note", "created_at": 8}
            ],
            "edges": [
                {"kind": "shared-as", "from": "note", "to": "post", "created_at": 10},
                {"kind": "contains", "from": "board", "to": "post", "created_at": 11},
                {"kind": "contains", "from": "post", "to": "reply", "created_at": 12},
                {"kind": "anchored-to", "from": "reply", "to": "note", "created_at": 13},
                {"kind": "edited-as", "from": "note", "to": "v2", "created_at": 14},
                {"kind": "forked-from", "from": "fork", "to": "note", "created_at": 15},
                {"kind": "authored-by", "from": "note", "to": "alice", "created_at": 16},
                {"kind": "references", "from": "secret", "to": "note", "created_at": 17}
            ],
            "hidden": ["secret"]
        }"#;
        serde_json::from_str(json).unwrap()
    }

    fn run(graph: &WireGraph, request: &WireRequest) -> WireResponse {
        let built = graph.build().unwrap();
        execute(&built, request, &|id| graph.can_see(id), &Bounds::default())
    }

    fn ids(answer: &WireResponse) -> Vec<String> {
        match answer {
            WireResponse::Page { items, .. } => items
                .iter()
                .map(|item| match item {
                    WireItem::Step(step) => step.node.id.clone(),
                    WireItem::Node(node) => node.id.clone(),
                })
                .collect(),
            WireResponse::Path { steps, .. } => steps
                .iter()
                .flatten()
                .map(|step| step.node.id.clone())
                .collect(),
            WireResponse::Error { .. } => Vec::new(),
        }
    }

    #[test]
    fn an_untyped_fixture_is_refused_at_build() {
        let mut graph = fixture();
        graph.edges.push(WireEdge {
            kind: String::from("contains"),
            from: String::from("alice"),
            to: String::from("note"),
            created_at: 99,
        });
        assert!(matches!(
            graph.build(),
            Err(WireGraphError::Graph(
                GraphError::UntypedRelationship { .. }
            ))
        ));
        let mut graph = fixture();
        graph.nodes[0].kind = String::from("widget");
        assert_eq!(
            graph.build().unwrap_err(),
            WireGraphError::UnknownNodeKind(String::from("widget"))
        );
    }

    #[test]
    fn execute_answers_exactly_what_the_core_answers() {
        let wire = fixture();
        let graph = wire.build().unwrap();
        let can_see = |id: &NodeId| wire.can_see(id);
        let bounds = Bounds::default();
        let core = neighbors(
            &graph,
            &Request::new(NodeId::new("note"), DEFAULT_PAGE_SIZE),
            &can_see,
            &bounds,
        )
        .unwrap();
        let answer = run(&wire, &WireRequest::new(WireOp::Neighbors, "note"));
        assert_eq!(answer, page_of(&core, step_item));
        let core_ids: Vec<String> = core
            .items
            .iter()
            .map(|step| step.node.id.as_str().to_owned())
            .collect();
        assert_eq!(ids(&answer), core_ids);
        assert!(!ids(&answer).contains(&String::from("secret")));
    }

    #[test]
    fn a_cursor_walk_over_the_wire_matches_one_full_page() {
        let wire = fixture();
        let whole = run(&wire, &WireRequest::new(WireOp::Neighbors, "note"));
        let mut walked = Vec::new();
        let mut request = WireRequest::new(WireOp::Neighbors, "note");
        request.page_size = Some(2);
        loop {
            let answer = run(&wire, &request);
            walked.extend(ids(&answer));
            let WireResponse::Page { next, .. } = answer else {
                panic!("expected a page");
            };
            match next {
                Some(cursor) => request.cursor = Some(cursor),
                None => break,
            }
        }
        assert_eq!(walked, ids(&whole));
    }

    #[test]
    fn a_cursor_from_another_request_is_refused_on_the_wire() {
        let wire = fixture();
        let mut request = WireRequest::new(WireOp::Neighbors, "note");
        request.page_size = Some(1);
        let WireResponse::Page {
            next: Some(cursor), ..
        } = run(&wire, &request)
        else {
            panic!("expected a next cursor");
        };
        let mut other = WireRequest::new(WireOp::Backlinks, "note");
        other.cursor = Some(cursor);
        assert!(matches!(
            run(&wire, &other),
            WireResponse::Error {
                code: WireErrorCode::InvalidCursor,
                ..
            }
        ));
        let mut garbage = WireRequest::new(WireOp::Neighbors, "note");
        garbage.cursor = Some(String::from("!!"));
        assert!(matches!(
            run(&wire, &garbage),
            WireResponse::Error {
                code: WireErrorCode::InvalidCursor,
                ..
            }
        ));
    }

    #[test]
    fn hidden_and_unknown_targets_get_the_same_refusal() {
        let wire = fixture();
        let hidden = run(&wire, &WireRequest::new(WireOp::Neighbors, "secret"));
        let unknown = run(&wire, &WireRequest::new(WireOp::Neighbors, "nowhere"));
        assert_eq!(hidden, unknown);
        assert!(matches!(
            hidden,
            WireResponse::Error {
                code: WireErrorCode::NotFound,
                ..
            }
        ));
    }

    #[test]
    fn bad_names_are_bad_requests_not_silent_defaults() {
        let wire = fixture();
        let mut request = WireRequest::new(WireOp::Neighbors, "note");
        request.edge_kinds = Some(vec![String::from("likes")]);
        assert!(matches!(
            run(&wire, &request),
            WireResponse::Error {
                code: WireErrorCode::BadRequest,
                ..
            }
        ));
        let mut request = WireRequest::new(WireOp::Neighbors, "note");
        request.direction = Some(String::from("sideways"));
        assert!(matches!(
            run(&wire, &request),
            WireResponse::Error {
                code: WireErrorCode::BadRequest,
                ..
            }
        ));
        let request = WireRequest::new(WireOp::Path, "note");
        assert!(matches!(
            run(&wire, &request),
            WireResponse::Error {
                code: WireErrorCode::BadRequest,
                ..
            }
        ));
    }

    #[test]
    fn the_json_surface_round_trips_the_same_answer() {
        let wire = fixture();
        let graph = wire.build().unwrap();
        let can_see = |id: &NodeId| wire.can_see(id);
        let bounds = Bounds::default();
        for op in [
            WireOp::Neighbors,
            WireOp::Backlinks,
            WireOp::ThreadsForObject,
            WireOp::HistoryForObject,
        ] {
            let request = WireRequest::new(op, "note");
            let direct = execute(&graph, &request, &can_see, &bounds);
            let json = execute_json(
                &graph,
                &serde_json::to_string(&request).unwrap(),
                &can_see,
                &bounds,
            );
            let parsed: WireResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, direct, "{op:?}");
        }
        let answer: WireResponse =
            serde_json::from_str(&execute_json(&graph, "{not json", &can_see, &bounds)).unwrap();
        assert!(matches!(
            answer,
            WireResponse::Error {
                code: WireErrorCode::BadRequest,
                ..
            }
        ));
    }

    #[test]
    fn path_over_the_wire_reaches_through_the_typed_graph() {
        let wire = fixture();
        let mut request = WireRequest::new(WireOp::Path, "fork");
        request.to = Some(String::from("reply"));
        let answer = run(&wire, &request);
        assert_eq!(ids(&answer), vec!["note", "reply"]);
        request.max_depth = Some(1);
        let WireResponse::Path { steps, truncated } = run(&wire, &request) else {
            panic!("expected a path answer");
        };
        assert!(steps.is_none());
        assert_eq!(
            truncated,
            Some(WireTruncation {
                bound: WireBound::Depth,
                limit: 1
            })
        );
    }

    #[test]
    fn text_presentation_reads_only_the_answer() {
        let wire = fixture();
        let text = render_text(&run(&wire, &WireRequest::new(WireOp::Backlinks, "note")));
        assert!(text.contains("reply --anchored-to--> note"));
        assert!(!text.contains("secret"));
        assert_eq!(
            render_text(&run(&wire, &WireRequest::new(WireOp::Neighbors, "nowhere"))),
            "error (not-found): no such node"
        );
    }
}
