//! Worlds layer of the `id` crate.
//!
//! Step 5 of the pkgs/id crate split (#3447 umbrella, #4408): the
//! world family lives here, over `id-core`, `id-caps`, `id-store`,
//! `id-events`, `id-groups` and `id-sync`.
//!
//! Contents:
//!
//! - The runtime: [`world`] (core, actor, replica, replication,
//!   handle), [`world_session`], [`world_hub`], [`world_net`],
//!   [`world_store`], [`world_records`], [`world_compile`],
//!   [`world_native`], [`world_fixture`], [`world_cache`] and, under
//!   the `sandbox` feature, [`sandbox`] — the wasmtime guest runner
//!   whose fuel budgets are the split's determinism canary.
//! - [`shell_contract`]: the F13.1 shell contract the native shell
//!   speaks.
//! - [`groups::worlds`]: seam 6 of the design — the groups→worlds
//!   bridge is the consumer side of the relationship, so it lives in
//!   the worlds crate, over the group model in `id-events`.
//! - [`blob_gate`]: the blob gate's folded-worlds adapter, which
//!   folds through the bridge above (step 4 parked it in the facade
//!   to move together with the bridge; it cannot live in `id-groups`,
//!   which is below this crate and must not name world types).
//! - The directory cluster — [`directory`], [`directory_auth`],
//!   [`directory_view`], [`directory_mail`] — the world runtime's own
//!   state: `WorldCore` holds the directory, the world actor executes
//!   directory actions against it, and `world_store` persists it
//!   beside the journal, so the cluster moves as the consumer side
//!   (the seam-6 pattern). Seam 10's remaining half is unchanged:
//!   step 6's `id-directory` extraction must still invert or re-home
//!   `WorldCore`'s hold on the directory, exactly as it would have
//!   had to from the facade. The facade re-exports these modules at
//!   their historical `id::` paths until then.
//!
//! The facade re-exports each module at its historical `id::` path,
//! so no call site outside the moved modules changed. The module
//! aliases below (`frames`, `world_caps`, …) exist so the moved
//! modules' `crate::` paths resolve exactly as they did in the
//! facade; they name the lower crates that own those modules.

pub mod blob_gate;
pub mod groups;
#[cfg(feature = "sandbox")]
pub mod sandbox;
mod session_host;
pub mod shell_contract;
pub mod world;
pub mod world_cache;
pub mod world_compile;
#[cfg(feature = "sandbox")]
pub mod world_dev;
pub mod world_doctor;
pub mod world_fixture;
pub mod world_hub;
pub mod world_line;
pub mod world_native;
pub mod world_net;
pub mod world_records;
pub mod world_replication_net;
pub mod world_session;
#[cfg(feature = "ssh")]
pub mod world_ssh;
pub mod world_store;
pub mod world_terminal;

pub use id_caps::world_caps;
pub use id_caps::world_limits;
pub use id_core::abuse;
pub use id_core::connectivity;
pub use id_core::frames;
pub(crate) use id_core::is_format_char;
pub use id_core::petname;
pub(crate) use id_core::text;
pub(crate) use id_core::time;
pub use id_core::world_presentation;
pub use id_directory::directory;
pub use id_directory::directory_auth;
pub use id_directory::directory_mail;
pub use id_directory::directory_view;
pub use id_events::event_store;
pub use id_store::chain;
pub use id_store::store;
pub use id_store::tags;
pub use id_sync::artifact;
pub use id_sync::envelope_outbox;

#[cfg(test)]
pub(crate) use id_core::test_forgeries;
