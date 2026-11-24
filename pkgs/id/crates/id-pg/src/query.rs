//! The data-shaped query object (design §3, #5816 shape).
//!
//! The object deliberately can *express* shapes the adapter refuses —
//! writes, RPC targets, boolean trees, embeds, offset pagination, count
//! preferences — so that refusing them by name is a decision of the
//! translator, exercised by the §10 model tests, rather than something
//! the type system hides.

use serde::{Deserialize, Serialize};

use crate::value::Value;

/// The HTTP method the query asks for. Only `Get` is translatable;
/// every other method is a write and is refused (design §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    /// `GET` — the only read method.
    Get,
    /// `POST`.
    Post,
    /// `PATCH`.
    Patch,
    /// `PUT`.
    Put,
    /// `DELETE`.
    Delete,
}

/// What the query addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// One relation (table or view) by its bare name within the
    /// endpoint's exposed schema.
    Collection(String),
    /// A PostgREST `/rpc/<name>` function call. Refused:
    /// `rpc_unsupported`.
    Rpc(String),
    /// The PostgREST root (`GET /`), i.e. schema/OpenAPI discovery.
    /// Refused: `discovery_unsupported`.
    Root,
    /// A raw-SQL `/query` endpoint. Refused: `sql_unsupported`.
    Sql,
}

/// One element of the requested projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectItem {
    /// A single declared column by name.
    Column(String),
    /// `*` — the adapter never forwards a star; it expands to the
    /// collection's declared column list (design §3.1).
    All,
    /// A PostgREST embed, `relation(...)` — a join. Refused:
    /// `join_unsupported`.
    Embed {
        /// The embedded relation's name.
        relation: String,
        /// The embedded projection.
        items: Vec<SelectItem>,
    },
}

/// The closed filter operator set, plus the operators the design
/// refuses by name (kept in the vocabulary so the refusal is a
/// translator decision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOp {
    /// `eq`.
    Eq,
    /// `neq`.
    Neq,
    /// `lt`.
    Lt,
    /// `lte`.
    Lte,
    /// `gt`.
    Gt,
    /// `gte`.
    Gte,
    /// `in` — the leaf value must be a non-empty [`Value::List`].
    In,
    /// `is.null` — the leaf value is ignored.
    IsNull,
    /// `like`. Refused: `text_pattern_unsupported`.
    Like,
    /// `ilike`. Refused: `text_pattern_unsupported`.
    ILike,
    /// `match`. Refused: `text_pattern_unsupported`.
    Match,
    /// `fts`. Refused: `text_pattern_unsupported`.
    Fts,
    /// `plfts`. Refused: `text_pattern_unsupported`.
    PlFts,
    /// `wfts`. Refused: `text_pattern_unsupported`.
    Wfts,
}

impl FilterOp {
    /// Whether the design refuses this operator as a text pattern
    /// (§3.2: `like`, `ilike`, `match`, and the full-text family).
    pub(crate) fn is_text_pattern(self) -> bool {
        matches!(
            self,
            FilterOp::Like
                | FilterOp::ILike
                | FilterOp::Match
                | FilterOp::Fts
                | FilterOp::PlFts
                | FilterOp::Wfts
        )
    }

    /// The PostgREST operator token for a supported operator.
    pub(crate) fn postgrest_token(self) -> Option<&'static str> {
        match self {
            FilterOp::Eq => Some("eq"),
            FilterOp::Neq => Some("neq"),
            FilterOp::Lt => Some("lt"),
            FilterOp::Lte => Some("lte"),
            FilterOp::Gt => Some("gt"),
            FilterOp::Gte => Some("gte"),
            FilterOp::In => Some("in"),
            FilterOp::IsNull => Some("is"),
            _ => None,
        }
    }
}

/// One filter predicate: a column, an operator, and a literal value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterLeaf {
    /// The column the predicate filters on.
    pub column: String,
    /// The operator.
    pub op: FilterOp,
    /// The literal value (a list for `in`; ignored for `is.null`).
    pub value: Value,
}

/// The filter expression tree. Only a flat conjunction of leaves is
/// translatable; `Or` and `Not` are refused as `boolean_unsupported`.
/// Nested `And` nodes are still a conjunction and are flattened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FilterExpr {
    /// A conjunction of sub-expressions (leaves or further `And`s).
    And(Vec<FilterExpr>),
    /// A disjunction. Refused: `boolean_unsupported`.
    Or(Vec<FilterExpr>),
    /// A negation. Refused: `boolean_unsupported`.
    Not(Box<FilterExpr>),
    /// A single predicate.
    Leaf(FilterLeaf),
}

/// Sort direction for one ordering key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Ascending (`asc`).
    Asc,
    /// Descending (`desc`).
    Desc,
}

/// One ordering key: a declared orderable column and a direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderKey {
    /// The column to order by.
    pub column: String,
    /// The direction.
    pub direction: Direction,
}

/// How the query pages through the result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pagination {
    /// No cursor: the first page.
    None,
    /// Keyset pagination: the cursor is the last row's ordering-key
    /// tuple, aligned with the *effective* ordering (the query's
    /// ordering keys with the collection's unique key appended as the
    /// final tiebreak).
    Keyset {
        /// The cursor tuple, one value per effective ordering key.
        cursor: Vec<Value>,
    },
    /// Offset pagination. Refused: `offset_unsupported`.
    Offset {
        /// The offset.
        offset: u32,
    },
    /// Page-number pagination. Refused: `offset_unsupported`.
    Page {
        /// The page number.
        page: u32,
    },
}

/// A count preference (the PostgREST `Prefer: count=...` family).
/// Any preference is refused: `count_unsupported` — a count discloses
/// the size of a withheld set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountPref {
    /// No count requested.
    None,
    /// `count=exact`.
    Exact,
    /// `count=planned`.
    Planned,
    /// `count=estimated`.
    Estimated,
}

/// A query against one collection, in the #5816 data shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    /// The HTTP method.
    pub method: Method,
    /// What the query addresses.
    pub target: Target,
    /// The requested projection. An empty list means all declared
    /// columns, exactly as [`SelectItem::All`] does.
    pub select: Vec<SelectItem>,
    /// The filter expression, if any.
    pub filter: Option<FilterExpr>,
    /// The requested ordering keys (may be empty; the unique key
    /// tiebreak is appended by the translator).
    pub order: Vec<OrderKey>,
    /// The pagination mode.
    pub pagination: Pagination,
    /// The requested row limit. `None` means the collection cap.
    pub limit: Option<u32>,
    /// The count preference.
    pub count: CountPref,
}

impl Query {
    /// A plain read of one collection: `GET`, no projection, filter,
    /// ordering, pagination, limit, or count preference. Builders of
    /// real queries start here and set the fields they need.
    pub fn read(relation: &str) -> Self {
        Query {
            method: Method::Get,
            target: Target::Collection(relation.to_string()),
            select: Vec::new(),
            filter: None,
            order: Vec::new(),
            pagination: Pagination::None,
            limit: None,
            count: CountPref::None,
        }
    }
}
