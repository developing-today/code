//! Postgres-over-HTTP query translator with allowlist (design #5805, §3).
//!
//! The adapter speaks exactly one interface: a PostgREST-compatible REST
//! surface over one exposed schema. This crate is the pure translation
//! layer. It takes the data-shaped query object ([`Query`]) plus the
//! owner's collection allowlists ([`CollectionAllowlist`]) and either
//! produces the [`PostgrestRequest`] to send or refuses the query with a
//! stable feature name from the design's §3.2 refusal table — always
//! before any network call (design §9, invariant 1).
//!
//! Scope boundaries, per the design's implementation slices (§11):
//!
//! - This crate is §3 (query translation) and the §10 property/model
//!   tests over it. Slice #13760.
//! - Typed mapping of response values (§5.1) is #13761; the error
//!   mapping and grantee collapse of *source* errors (§6) is #13762;
//!   the live/materialized read paths and the HTTP transport are
//!   #13763. Registration and owner config (§4) are #13758 — until that
//!   lands, allowlists are constructed directly in the shapes the
//!   design specifies.
//! - The translator only ever produces a `GET` request for one
//!   collection. It has no way to express a write (design §9,
//!   invariant 10).
//!
//! Refusal precedence, when a query violates several rules at once, is
//! fixed and deterministic so the refusal table is model-testable:
//!
//! 1. Non-`GET` method → `write_unsupported`.
//! 2. Non-collection target → `rpc_unsupported` /
//!    `discovery_unsupported` / `sql_unsupported`.
//! 3. Count preference → `count_unsupported`.
//! 4. Offset/page pagination → `offset_unsupported`.
//! 5. Non-conjunctive filter tree → `boolean_unsupported`.
//! 6. Embed in the projection → `join_unsupported`.
//! 7. Relation not in the owner's allowlist → `not_available`.
//! 8. Undeclared / non-filterable / non-orderable column →
//!    `field_unsupported`.
//! 9. Text-pattern operator → `text_pattern_unsupported`.
//! 10. Limit above the collection cap → `limit_exceeded`.
//!
//! Malformed-value edges the design does not name are refused as
//! `field_unsupported` rather than serialized into a request the source
//! would reject or misread: a comparison against `null` (null tests go
//! through `is.null`), an empty `in` list, a non-finite float, a keyset
//! cursor whose arity does not match the effective ordering, and a
//! `null` inside a keyset cursor (ordering keys are the declared unique
//! key and declared orderable columns; a null there has no total-order
//! position to page from).

mod allowlist;
mod query;
mod refusal;
mod translate;
mod value;

pub use allowlist::{CollectionAllowlist, ColumnDecl, DEFAULT_MAX_LIMIT};
pub use query::{
    CountPref, Direction, FilterExpr, FilterLeaf, FilterOp, Method, OrderKey, Pagination, Query,
    SelectItem, Target,
};
pub use refusal::{Feature, Refusal, RefusalCode};
pub use translate::{PostgrestRequest, Translator, effective_ordering};
pub use value::Value;
