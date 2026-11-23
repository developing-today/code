//! Bounded read-only triple-pattern queries (slice A-GRAPH-9126-01,
//! #10221; design part A §5.1, statements S5.8, S5.10, S5.11).
//!
//! A [`Query`] is a non-empty conjunction of [`TriplePattern`]s over a
//! fixed [`Snapshot`] of assertions. Each pattern position is a wildcard,
//! an exact [`Term`] or a [`Variable`]; a variable bound in one place
//! must bind the same term everywhere it occurs. There are no updates,
//! property paths, federation, entailment or caller-chosen sort.
//!
//! Rows come out in one stable total order: for each pattern in turn,
//! `(graph, subject, predicate, object, assertion id)`, with terms
//! compared by their canonical bytes. Every row carries the assertions it
//! matched and their `source_record` provenance.
//!
//! Every query runs under a read policy and an operator [`Profile`]:
//!
//! - a pattern that names a graph the policy does not allow fails with
//!   [`QueryError::UnauthorizedGraph`]; assertions in other unreadable
//!   graphs are never visited, so they cannot change a result or a cost;
//! - every page is limited, and a request cannot raise the profile's row
//!   ceiling;
//! - evaluation counts index steps, and running past the profile's step
//!   budget fails with [`QueryError::BudgetExceeded`] instead of
//!   returning rows that look complete.
//!
//! A page that is not the last carries a [`Cursor`] bound to the
//! snapshot frontier and the query (see [`super::cursor`]).
//!
//! Module wiring into `graph/mod.rs` belongs to slice 9110-09; until
//! then the integration test includes this file by path.

use std::collections::BTreeMap;
use std::fmt;

use super::cursor::{Cursor, CursorError, Frontier};
use super::term::{Term, Triple};
use crate::canonical_cbor::{Value, encode_canonical};
use crate::signed_envelope::{EnvelopeId, LogicalGraphId};

/// Most patterns one query may join.
pub const MAX_PATTERNS: usize = 8;
/// Longest variable name, in bytes.
pub const MAX_VARIABLE_BYTES: usize = 64;

const FRONTIER_DOMAIN: &[u8] = b"id/graph-frontier/v1\0";
const QUERY_DOMAIN: &[u8] = b"id/graph-query/v1\0";

/// Operator-validated finite ceilings. Requests cannot raise them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// Rows per page when a query names no limit.
    pub page_default: u32,
    /// Largest limit a query may ask for.
    pub row_ceiling: u32,
    /// Most index steps one page evaluation may take.
    pub max_steps: u64,
}

impl Profile {
    /// The design's initial deployment profile.
    pub const INITIAL: Self = Self {
        page_default: 100,
        row_ceiling: 1_000,
        max_steps: 1_000_000,
    };
}

/// Decides which logical graphs the caller may read.
pub trait GraphReadPolicy {
    /// Whether the caller may read `graph`.
    fn may_read(&self, graph: &LogicalGraphId) -> bool;
}

/// A query variable name: 1 to 64 ASCII letters, digits or `_`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Variable(String);

impl Variable {
    /// A variable from its name.
    ///
    /// # Errors
    ///
    /// [`QueryError::InvalidPattern`] for an empty, overlong or
    /// non-identifier name.
    pub fn new(name: &str) -> Result<Self, QueryError> {
        if name.is_empty()
            || name.len() > MAX_VARIABLE_BYTES
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(QueryError::InvalidPattern("bad variable name"));
        }
        Ok(Self(name.to_owned()))
    }

    /// The name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// One position of a triple pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermPattern {
    /// Matches exactly this term.
    Exact(Term),
    /// Binds, or must equal, this variable.
    Var(Variable),
}

/// A pattern over one assertion. `None` positions match anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TriplePattern {
    /// Restricts the match to one logical graph.
    pub graph: Option<LogicalGraphId>,
    /// Subject position.
    pub subject: Option<TermPattern>,
    /// Predicate position.
    pub predicate: Option<TermPattern>,
    /// Object position.
    pub object: Option<TermPattern>,
}

/// A conjunctive query and its page request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// The patterns, joined on shared variables.
    pub patterns: Vec<TriplePattern>,
    /// The variables each row reports, in this order.
    pub projection: Vec<Variable>,
    /// Rows per page; `None` takes the profile's default.
    pub limit: Option<u32>,
    /// Where the previous page stopped.
    pub cursor: Option<Cursor>,
}

/// One verified assertion in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAssertion {
    /// The signed record the triple came from.
    pub record: EnvelopeId,
    /// Its position in that record.
    pub index: u32,
    /// Its logical graph.
    pub graph: LogicalGraphId,
    /// The triple.
    pub triple: Triple,
    /// The evidence the record names, if any.
    pub source_record: Option<EnvelopeId>,
}

/// An assertion's place in the stable order: graph, the three terms'
/// canonical bytes, then the assertion id (record, index).
type AssertionKey = ([u8; 32], [Vec<u8>; 3], (u8, [u8; 32], u32));

fn assertion_key(assertion: &StoredAssertion) -> AssertionKey {
    let id = assertion.record.content_id();
    let term = |t: &Term| t.to_bytes().unwrap_or_default();
    (
        *assertion.graph.as_bytes(),
        [
            term(&assertion.triple.subject),
            term(&assertion.triple.predicate),
            term(&assertion.triple.object),
        ],
        (id.profile().tag(), *id.digest(), assertion.index),
    )
}

/// A fixed set of verified assertions, read at one frontier.
#[derive(Debug, Clone)]
pub struct Snapshot {
    frontier: Frontier,
    assertions: Vec<(AssertionKey, StoredAssertion)>,
}

impl Snapshot {
    /// A snapshot of `assertions`. Entries with the same assertion id
    /// (record and index) are kept once. The frontier is the BLAKE3 hash
    /// of the sorted assertion ids, so it is the same whatever order the
    /// assertions arrive in.
    #[must_use]
    pub fn new(assertions: Vec<StoredAssertion>) -> Self {
        let mut keyed: Vec<(AssertionKey, StoredAssertion)> = assertions
            .into_iter()
            .map(|a| (assertion_key(&a), a))
            .collect();
        keyed.sort_by_key(|a| a.0.2);
        keyed.dedup_by(|a, b| a.0.2 == b.0.2);
        let mut hasher = blake3::Hasher::new();
        hasher.update(FRONTIER_DOMAIN);
        for ((_, _, (tag, digest, index)), _) in &keyed {
            hasher.update(&[*tag]);
            hasher.update(digest);
            hasher.update(&index.to_be_bytes());
        }
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            frontier: Frontier(*hasher.finalize().as_bytes()),
            assertions: keyed,
        }
    }

    /// The snapshot's frontier.
    #[must_use]
    pub const fn frontier(&self) -> Frontier {
        self.frontier
    }

    /// How many assertions it holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.assertions.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.assertions.is_empty()
    }
}

/// One matched assertion, with its provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Matched {
    /// The record the triple was signed in.
    pub record: EnvelopeId,
    /// Its position in that record.
    pub index: u32,
    /// The evidence that record names, if any.
    pub source_record: Option<EnvelopeId>,
}

/// One result row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The projected variables' terms, in projection order.
    pub values: Vec<Term>,
    /// The assertion each pattern matched, in pattern order.
    pub matched: Vec<Matched>,
}

/// What a page cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cost {
    /// Index steps taken.
    pub scanned: u64,
    /// Rows returned.
    pub returned: u32,
}

/// One page of results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPage {
    /// The rows, in the stable order.
    pub rows: Vec<Row>,
    /// Present when more rows follow.
    pub next_cursor: Option<Cursor>,
    /// The frontier the page was read at.
    pub snapshot: Frontier,
    /// True only on the last page of a fully evaluated result.
    pub complete: bool,
    /// What the page cost.
    pub cost: Cost,
}

/// Why a query was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    /// The query is not well formed.
    InvalidPattern(&'static str),
    /// A pattern names a graph the caller may not read.
    UnauthorizedGraph,
    /// The cursor was issued at another snapshot frontier.
    CursorExpired,
    /// The cursor is for another query, or not a cursor.
    CursorMismatch,
    /// Evaluation ran past the profile's step budget.
    BudgetExceeded {
        /// Steps taken before stopping.
        scanned: u64,
    },
}

impl QueryError {
    /// A stable failure class name.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::InvalidPattern(_) => "invalid_pattern",
            Self::UnauthorizedGraph => "unauthorized_graph",
            Self::CursorExpired => "cursor_expired",
            Self::CursorMismatch => "cursor_mismatch",
            Self::BudgetExceeded { .. } => "budget_exceeded",
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPattern(why) => write!(f, "invalid query: {why}"),
            Self::UnauthorizedGraph => write!(f, "query names a graph the caller may not read"),
            Self::CursorExpired => write!(f, "cursor is for another snapshot"),
            Self::CursorMismatch => write!(f, "cursor is for another query"),
            Self::BudgetExceeded { scanned } => {
                write!(f, "query stopped after {scanned} steps, over budget")
            }
        }
    }
}

impl std::error::Error for QueryError {}

impl From<CursorError> for QueryError {
    fn from(error: CursorError) -> Self {
        match error {
            CursorError::Expired => Self::CursorExpired,
            CursorError::Mismatch => Self::CursorMismatch,
        }
    }
}

fn pattern_value(position: Option<&TermPattern>) -> Value {
    match position {
        None => Value::Null,
        Some(TermPattern::Exact(term)) => {
            Value::Array(vec![Value::UInt(0), term.to_value().unwrap_or(Value::Null)])
        }
        Some(TermPattern::Var(var)) => {
            Value::Array(vec![Value::UInt(1), Value::Text(var.0.clone())])
        }
    }
}

impl Query {
    /// A digest of what the query asks (patterns and projection, not the
    /// page size or cursor), which a cursor is bound to.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        let patterns = self
            .patterns
            .iter()
            .map(|p| {
                Value::Array(vec![
                    p.graph
                        .map_or(Value::Null, |g| Value::Bytes(g.as_bytes().to_vec())),
                    pattern_value(p.subject.as_ref()),
                    pattern_value(p.predicate.as_ref()),
                    pattern_value(p.object.as_ref()),
                ])
            })
            .collect();
        let projection = self
            .projection
            .iter()
            .map(|v| Value::Text(v.0.clone()))
            .collect();
        let bytes = encode_canonical(&Value::Array(vec![
            Value::Array(patterns),
            Value::Array(projection),
        ]))
        .unwrap_or_default();
        let mut hasher = blake3::Hasher::new();
        hasher.update(QUERY_DOMAIN);
        hasher.update(&bytes);
        *hasher.finalize().as_bytes()
    }

    fn mentions(&self, var: &Variable) -> bool {
        self.patterns.iter().any(|p| {
            [&p.subject, &p.predicate, &p.object]
                .into_iter()
                .any(|pos| matches!(pos, Some(TermPattern::Var(v)) if v == var))
        })
    }
}

type Bindings = BTreeMap<Variable, Term>;

fn bind(position: Option<&TermPattern>, term: &Term, bindings: &mut Bindings) -> bool {
    match position {
        None => true,
        Some(TermPattern::Exact(want)) => want == term,
        Some(TermPattern::Var(var)) => {
            if let Some(bound) = bindings.get(var) {
                return bound == term;
            }
            bindings.insert(var.clone(), term.clone());
            true
        }
    }
}

fn matches(pattern: &TriplePattern, assertion: &StoredAssertion, bindings: &mut Bindings) -> bool {
    pattern.graph.is_none_or(|g| g == assertion.graph)
        && bind(
            pattern.subject.as_ref(),
            &assertion.triple.subject,
            bindings,
        )
        && bind(
            pattern.predicate.as_ref(),
            &assertion.triple.predicate,
            bindings,
        )
        && bind(pattern.object.as_ref(), &assertion.triple.object, bindings)
}

/// Runs one page of `query` over `snapshot`.
///
/// # Errors
///
/// [`QueryError`] for a malformed query, an unreadable named graph, a
/// cursor for another query or snapshot, or a blown step budget.
pub fn run(
    query: &Query,
    snapshot: &Snapshot,
    policy: &dyn GraphReadPolicy,
    profile: &Profile,
) -> Result<QueryPage, QueryError> {
    if query.patterns.is_empty() {
        return Err(QueryError::InvalidPattern("a query needs a pattern"));
    }
    if query.patterns.len() > MAX_PATTERNS {
        return Err(QueryError::InvalidPattern("too many patterns"));
    }
    if !query.projection.iter().all(|var| query.mentions(var)) {
        return Err(QueryError::InvalidPattern(
            "projected variable is not in any pattern",
        ));
    }
    let limit = query.limit.unwrap_or(profile.page_default);
    if limit == 0 || limit > profile.row_ceiling {
        return Err(QueryError::InvalidPattern(
            "limit must be 1 to the profile's row ceiling",
        ));
    }
    if query
        .patterns
        .iter()
        .filter_map(|p| p.graph.as_ref())
        .any(|g| !policy.may_read(g))
    {
        return Err(QueryError::UnauthorizedGraph);
    }
    let digest = query.digest();
    let offset = match &query.cursor {
        Some(cursor) => cursor.resume(snapshot.frontier, digest)?,
        None => 0,
    };

    let visible: Vec<&(AssertionKey, StoredAssertion)> = snapshot
        .assertions
        .iter()
        .filter(|(_, a)| policy.may_read(&a.graph))
        .collect();
    let mut scanned = 0_u64;
    let mut partial: Vec<(Vec<usize>, Bindings)> = vec![(Vec::new(), Bindings::new())];
    for pattern in &query.patterns {
        let mut next = Vec::new();
        for (chosen, bindings) in &partial {
            for (at, (_, assertion)) in visible.iter().enumerate() {
                scanned += 1;
                if scanned > profile.max_steps {
                    return Err(QueryError::BudgetExceeded { scanned });
                }
                let mut extended = bindings.clone();
                if matches(pattern, assertion, &mut extended) {
                    let mut path = chosen.clone();
                    path.push(at);
                    next.push((path, extended));
                }
            }
        }
        partial = next;
    }
    // `visible` is in assertion-key order and each pass extends the
    // partial rows in order, so `partial` is already in the stable row
    // order: lexicographic over the matched assertions' keys.

    let total = partial.len() as u64;
    let start = offset.min(total);
    let end = start.saturating_add(u64::from(limit)).min(total);
    let rows: Vec<Row> = partial
        .iter()
        .skip(usize::try_from(start).unwrap_or(usize::MAX))
        .take(usize::try_from(end - start).unwrap_or(0))
        .map(|(path, bindings)| Row {
            values: query
                .projection
                .iter()
                .map(|var| bindings[var].clone())
                .collect(),
            matched: path
                .iter()
                .map(|at| {
                    let a = &visible[*at].1;
                    Matched {
                        record: a.record,
                        index: a.index,
                        source_record: a.source_record,
                    }
                })
                .collect(),
        })
        .collect();
    let next_cursor = (end < total).then(|| Cursor::new(snapshot.frontier, digest, end));
    Ok(QueryPage {
        complete: next_cursor.is_none(),
        cost: Cost {
            scanned,
            returned: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        },
        rows,
        next_cursor,
        snapshot: snapshot.frontier,
    })
}
