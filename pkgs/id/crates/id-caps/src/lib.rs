//! Capability data model of the `id` crate: the capability catalog,
//! execution limits and the declarative world manifest.
//!
//! Step 1 of the pkgs/id crate split (#3447 umbrella, #4403): these three
//! modules moved here from `id-core` (`world_caps`, `world_limits`) and
//! from the facade package (`world_manifest`). The facade re-exports each
//! module at its historical `id::` path, so no call site outside the
//! moved modules changed.
//!
//! This crate depends on `id-core` for [`id_core::json_unique`] only;
//! the dependency points one way — `id-core` never names this crate.

pub mod world_caps;
pub mod world_limits;
pub mod world_manifest;
pub mod world_scopes;
