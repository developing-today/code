//! Sync layer of the `id` crate.
//!
//! Step 3 of the pkgs/id crate split (#3447 umbrella, #4406): the sync
//! family lives here — [`log_sync`] (the p2panda log-sync sessions and
//! the range-sync bridge), the wire layer ([`protocol`], the meta
//! protocol; [`fileops`], the file operations the protocol serves;
//! [`dht`], the opt-in mainline lookup policy) and the wire half of
//! [`meta_client`] (request/response plumbing over an open connection).
//!
//! The facade re-exports each module at its historical `id::` path, so
//! no call site outside the moved modules changed. Composition stays
//! in the facade until step 7: the `MetaClient` enum that chooses
//! between a running server and the in-process local node is defined
//! there, over this crate's [`meta_client`] plumbing.

pub mod dht;
pub mod fileops;
pub mod log_sync;
pub mod meta_client;
pub mod protocol;
