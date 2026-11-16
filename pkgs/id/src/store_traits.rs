//! The store trait surface, re-exported from `id-store`.
//!
//! Step 2a of the pkgs/id crate split (#4405): the implementation of this
//! module — the traits, the shared handle, the log stream — lives in
//! `id_store::store_traits`, generic over the
//! [`id_store::store_traits::EventStoreBackend`] trait (seam 3 of the
//! design: the traits crate holds traits only, so the handle cannot name
//! the concrete event store that sits above it). This facade module holds
//! nothing but the re-exports and the one type alias that pins the
//! handle's store parameter to this crate's [`crate::event_store::EventStore`],
//! so every historical `id::store_traits` path and signature is unchanged.

pub use id_store::store_traits::{ChainLog, EventStoreBackend, TraitStoreError};

/// A clonable handle to one event store that implements p2panda-store's
/// traits. See `id_store::store_traits::SharedEventStore` for the
/// implementation; this alias is that handle over the facade's
/// [`crate::event_store::EventStore`].
pub type SharedEventStore =
    id_store::store_traits::SharedEventStore<crate::event_store::EventStore>;
