//! Social objects: the public surface for envelopes, profiles and
//! bounded traversal.
//!
//! Issue #13095 (remainder #14334), slice S3 of the implementation
//! design `doc/2026-11-24T05-55-00Z_design_id13095_social-object-envelope`
//! §5; parent design #9128 §§2–5 and §7.
//!
//! This module re-exports the pieces a caller needs from
//! [`crate::graph`] in one place, so downstream code depends on a
//! stable path rather than on how the graph layer is split:
//!
//! - the envelope, [`SocialObject`], with its five profiles
//!   ([`SocialObjectKind`], [`ALL_PROFILES`]), profile rules
//!   ([`ProfileRule`], [`Requirement`]), validator and strict
//!   canonical encoding ([`EnvelopeError`]);
//! - the one-shot bounded walk, [`traverse`], and the paged walk with
//!   a resumable cursor, [`traverse_page`];
//! - the default bounds of #9128 §7 ([`DEFAULT_MAX_DEPTH`],
//!   [`DEFAULT_MAX_VISITED`], [`DEFAULT_MAX_BYTES`]).
//!
//! Everything here is pure. Nothing checks that a referenced object
//! exists, that an author signed an envelope or may post where it
//! points: callers apply storage, admission and access control before
//! they hand objects to these functions, and pass only the objects a
//! reader may see through the `authorized` predicate.

pub use crate::graph::social::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_DEPTH, DEFAULT_MAX_VISITED, SocialObject, SocialObjectKind,
    Traversal, TraversalEntry, TraversalMark, TraversalProfile, traverse,
};
pub use crate::graph::social_envelope::{
    ALL_PROFILES, EnvelopeError, MAX_AUTHOR_BYTES, MAX_ENVELOPE_BYTES, ProfileRule, Requirement,
    SOCIAL_SCHEMA_VERSION,
};
pub use crate::graph::social_paging::{PagingError, TraversalCursor, TraversalPage, traverse_page};
pub use crate::graph::term::ObjectRef;
