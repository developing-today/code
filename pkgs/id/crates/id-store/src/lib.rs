//! Store layer of the `id` crate.
//!
//! Step 2a of the pkgs/id crate split (#3447 umbrella, #4405): the store
//! family lives here — the store traits and blob store, the event header
//! chain, documents, the tag model (claims, index, tags) and search —
//! together with two kinds of vocabulary the family owns:
//!
//! - the event store's own types ([`event_types`], [`range`]): the traits
//!   in [`store_traits`] name [`event_types::StoreError`], [`event_types::Ingest`]
//!   and the range-sync `(Key, Id)` pairs, so the types sit below the
//!   event-store implementation (step 2b, `id-events`), never above it;
//! - the op vocabularies the signed header embeds ([`chat`], [`notes`],
//!   [`note_links`], [`session_checkpoint`], [`issuer_rotation`]) and the
//!   group policy record ([`policy`]): [`chain::ChainExt`] carries these
//!   types as fields, so their definitions live with the chain, and the
//!   facade modules that fold them re-export them from here.
//!
//! This crate depends on `id-core` only. The facade re-exports each module
//! at its historical `id::` path, so no call site outside the moved
//! modules changed.

/// The port contract (`record-store` port kind) this crate's adapters
/// advertise against, owned by `id-core` (slice 9110-02, #10160).
pub use id_core::port;

/// Content-ID helpers, owned by `id-core`. [`content_store`] names them
/// as a sibling module, so the module is re-exported at this root.
pub use id_core::content_id;

pub mod admission;
pub mod chain;
pub mod chat;
pub mod chat_anchor;
pub mod content_store;
pub mod documents;
pub mod event_types;
pub mod graph_index;
pub mod identity;
pub mod issuer_rotation;
pub mod media;
pub mod namespace_registry;
pub mod note_blocks;
pub mod note_checkpoints;
pub mod note_links;
pub mod note_projections;
pub mod notes;
pub mod numeric_registry;
pub mod policy;
pub mod range;
pub mod search;
pub mod session_checkpoint;
pub mod store;
pub mod store_traits;
pub mod tag_claims;
pub mod tag_index;
pub mod tags;

pub use store::STORE_PATH;
