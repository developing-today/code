//! `id nav` argument definitions: the CLI surface of the native graph
//! navigation API (#7612, record on #5831).
//!
//! The arguments map one-to-one onto
//! [`id_core::graph::navigation_wire::WireRequest`], so the CLI asks
//! exactly what the script/API and agent surfaces ask, and only the
//! printing differs.

use std::path::PathBuf;

use clap::{Args, ValueEnum};

use id_core::graph::navigation_wire::{WireOp, WireRequest};

/// The five navigation operations.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavOp {
    /// The edges at a node and the node at each far end.
    Neighbors,
    /// The content that points at a node.
    Backlinks,
    /// The discussion threads a node takes part in.
    Threads,
    /// A node's edit states and forks.
    History,
    /// The shortest path to `--to`, within a depth bound.
    Path,
}

impl From<NavOp> for WireOp {
    fn from(op: NavOp) -> Self {
        match op {
            NavOp::Neighbors => Self::Neighbors,
            NavOp::Backlinks => Self::Backlinks,
            NavOp::Threads => Self::ThreadsForObject,
            NavOp::History => Self::HistoryForObject,
            NavOp::Path => Self::Path,
        }
    }
}

/// Arguments for `id nav`.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct NavArgs {
    /// The operation.
    #[arg(value_enum)]
    pub op: NavOp,
    /// The node to start from.
    pub target: String,
    /// The graph snapshot: a JSON file of `nodes`, `edges` and the
    /// viewer's `hidden` ids.
    #[arg(long)]
    pub graph: PathBuf,
    /// Follow only this edge kind (repeatable), by record name such as
    /// `references` or `shared-as`.
    #[arg(long = "edge")]
    pub edges: Vec<String>,
    /// `out`, `in` or `both` (the default).
    #[arg(long)]
    pub direction: Option<String>,
    /// Resume after this cursor, from the `next:` line of an earlier page.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Items per page.
    #[arg(long)]
    pub page_size: Option<usize>,
    /// For `path`: the node the path should reach.
    #[arg(long)]
    pub to: Option<String>,
    /// For `path`: the deepest walk to try.
    #[arg(long)]
    pub max_depth: Option<usize>,
    /// Print the answer as JSON, exactly as the API returns it.
    #[arg(long)]
    pub json: bool,
}

impl NavArgs {
    /// The wire request these arguments name.
    #[must_use]
    pub fn to_request(&self) -> WireRequest {
        let mut request = WireRequest::new(self.op.into(), self.target.clone());
        if !self.edges.is_empty() {
            request.edge_kinds = Some(self.edges.clone());
        }
        request.direction.clone_from(&self.direction);
        request.cursor.clone_from(&self.cursor);
        request.page_size = self.page_size;
        request.to.clone_from(&self.to);
        request.max_depth = self.max_depth;
        request
    }
}
