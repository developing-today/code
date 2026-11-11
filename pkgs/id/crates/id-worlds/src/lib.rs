//! Worlds layer of the `id` crate.
//!
//! Step 5 of the pkgs/id crate split (#3447 umbrella, #4408): the
//! world family lives here, over `id-core`, `id-caps`, `id-store`,
//! `id-events`, `id-groups` and `id-sync`.
//!
//! This first step-5 PR brings:
//!
//! - [`blob_gate`]: the blob gate's folded-worlds adapter
//!   ([`blob_gate::FoldedWorlds`]), parked in the facade by step 4 so
//!   it could move together with the cross-world bridge it folds.
//! - [`groups::worlds`]: seam 6 of the design — the groups→worlds
//!   bridge is the consumer side of the relationship, so it lives in
//!   the worlds crate, over the group model in `id-events`.
//! - [`world_cache`], [`world_line`], [`world_terminal`]: the family
//!   leaves that name nothing above them.
//!
//! The world core, sessions, sandbox and the rest of the family
//! follow in later step-5 PRs. The facade re-exports each module at
//! its historical `id::` path, so no call site outside the moved
//! modules changed.

pub mod blob_gate;
pub mod groups;
pub mod world_cache;
pub mod world_line;
pub mod world_terminal;
