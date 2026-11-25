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
//! - Typed mapping of response values (§5.1) is the `mapping` module
//!   from #13761: a [`TypedDecoder`] reads each page under the owner's
//!   declared column types ([`PgType`]), keeps JSON numbers as text until
//!   the declared type is known, and names a mismatch by column and type
//!   but never by value. The error
//!   mapping and grantee collapse of *source* errors (§6) is
//!   [`Failure::to_error`] from #13762: every refusal, HTTP status,
//!   deadline, and mapping failure becomes a stable [`ErrorCode`], and a
//!   grantee sees one `not_available` body for anything that depends on
//!   the data or the grant. The live and materialized read paths
//!   (§5.2–§5.4) are the `read` module from #13763: [`live_read`] and
//!   [`SnapshotStore`] drive translated requests through a
//!   [`Transport`] that can only `GET`, report `per_request`
//!   consistency, and gate every read on the owner's [`ModeChoices`].
//!   Registration and owner config (§2, §4) are the
//!   [`register`] step from #13758: an [`EndpointConfig`] plus the
//!   transport's [`EndpointProbe`] become a [`RegisteredEndpoint`] only
//!   when the endpoint serves exactly the one exposed schema and the
//!   credential's role is read-only (§9, invariant 9).
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
mod error;
mod json;
mod mapping;
mod query;
mod read;
mod refusal;
mod registration;
mod translate;
mod value;

pub use allowlist::{CollectionAllowlist, ColumnDecl, DEFAULT_MAX_LIMIT};
pub use error::{AdapterError, Audience, ErrorCode, Failure, classify_response};
pub use mapping::{
    FieldShape, JsonShape, MAPPING_VERSION, MappedValue, PgType, TYPE_DATE, TYPE_INTERVAL,
    TYPE_JSON, TYPE_JSONB, TYPE_NUMERIC, TYPE_TIME, TYPE_TIMETZ, TypedDecoder, TypedRow,
};
pub use query::{
    CountPref, Direction, FilterExpr, FilterLeaf, FilterOp, Method, OrderKey, Pagination, Query,
    SelectItem, Target,
};
pub use read::{
    Consistency, ContentRange, DecodeError, HttpResponse, LiveResult, ModeChoices, PageDecoder,
    ReadError, ReadMode, Snapshot, SnapshotCaps, SnapshotStore, Transport, TransportError,
    live_read,
};
pub use refusal::{Feature, Refusal, RefusalCode};
pub use registration::{
    EndpointConfig, EndpointProbe, ObjectKind, Privilege, RegisteredEndpoint, RegistrationError,
    RoleGrant, register,
};
pub use translate::{PostgrestRequest, Translator, effective_ordering};
pub use value::Value;
