//! Sync layer of the `id` crate.
//!
//! Step 3 of the pkgs/id crate split (#3447 umbrella, #4406): the sync
//! family lives here — [`log_sync`] (the p2panda log-sync sessions and
//! the range-sync bridge), the wire layer ([`protocol`], the meta
//! protocol; [`fileops`], the file operations the protocol serves;
//! [`dht`], the opt-in mainline lookup policy) and the wire half of
//! [`meta_client`] (request/response plumbing over an open connection).
//!
//! Step 3c added the friend-envelope and artifact transfer family:
//! [`envelope`] (the signed wire type, out of the facade's directory),
//! [`envelope_net`] and [`envelope_outbox`] (their transport and durable
//! queue) and [`artifact`] (signed artifacts and their push/pull
//! transfer). Their seam to the worlds and directory that still live in
//! the facade is [`host::SessionHost`] plus
//! [`artifact::ArtifactSink`], both implemented facade-side.
//!
//! The facade re-exports each module at its historical `id::` path, so
//! no call site outside the moved modules changed. Composition stays
//! in the facade until step 7: the `MetaClient` enum that chooses
//! between a running server and the in-process local node is defined
//! there, over this crate's [`meta_client`] plumbing.

pub mod announcement;
pub mod artifact;
pub mod chained_message;
mod checked_json;
pub mod dht;
pub mod envelope;
pub mod envelope_net;
pub mod envelope_outbox;
pub mod fanout_answer;
pub mod federation;
pub mod fileops;
pub mod graph_frontier;
pub(crate) mod hex;
pub mod host;
pub mod log_sync;
pub mod meta_client;
pub mod protocol;
pub mod registry;
pub mod relay_gate;
pub mod relay_ingest;
pub mod transfer;
