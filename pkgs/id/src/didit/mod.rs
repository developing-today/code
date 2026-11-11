//! The did-it tracker data model: schema types and a pure evaluator (#3788,
//! Phase 1 of #2031; design record
//! `doc/2026-10-29T13-04-50Z_investigation_didit_tracker_data_model/`).
//!
//! The tracker is four new `ChainExt.kind` values — `didit.entity`,
//! `didit.record`, `didit.expectation`, `didit.opinion` — whose structure lives
//! in the event **body**, never in the signed header (decision D0). Bodies are
//! canonical CBOR and are passed to [`crate::chain::sign`] as the event body;
//! nothing here changes the header format or any existing verifier.
//!
//! Two rules from the design are load-bearing and enforced here:
//!
//! * **Expectations are declarations, not due rows** (D1/D2). No `due` state is
//!   written: [`evaluate`] is a pure function over `(expectations, records,
//!   now)` that any device can replay for any instant.
//! * **A record has a subject set, not one event per subject** (D4). A walk of
//!   both dogs is one record with a per-subject detail map, so an act keeps its
//!   identity and per-entity detail may diverge.
//!
//! `subjects` is explicit: an empty subject list is invalid, never "everyone"
//! (open question 2 in the design). The evaluator evaluates windows in the
//! expectation's IANA `tz`, never a record's raw offset (open question 1).
//!
//! This module is library-only. Derived summary and full record projections
//! implement §7; app rendering and body persistence (issue #4837) are out of
//! scope.

mod body;
mod eval;

pub use body::{
    ActiveWindow, DiditError, ENTITY_KIND, EXPECTATION_KIND, Entity, Expectation, HoursSpan,
    MAX_BODY_BYTES, Measure, OPINION_KIND, Opinion, Period, PeriodCustom, Place, RECORD_KIND,
    Record, RecordDetail, RecordSummary, VERSION, Window, decode_entity_body,
    decode_expectation_body, decode_opinion_body, decode_record_body, encode_entity_body,
    encode_expectation_body, encode_opinion_body, encode_record_body,
};
pub use eval::{EvalError, Verdict, VerdictState, evaluate};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
