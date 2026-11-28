//! The range-sync set's key and ID shapes.
//!
//! Step 2a of the pkgs/id crate split (#4405): the sync set is store
//! vocabulary — the store traits in this crate hand out `(Key, Id)` pairs —
//! so the shapes live here. The range reconciliation protocol that works
//! over them stays with the event store until step 2b and re-exports these
//! at its historical `id::event_store::range` paths.

/// Bytes in a sync key: the author (32), the sequence number (8, big-endian), then the ID (32).
pub const KEY_LEN: usize = 72;
/// A sync key.
pub type Key = [u8; KEY_LEN];
/// An event ID: the BLAKE3 hash of the signed header.
pub type Id = [u8; 32];
