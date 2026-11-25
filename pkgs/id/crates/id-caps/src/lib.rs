//! Capability and policy layer of the `id` crate: capability catalog,
//! execution limits, declarative world manifest, library gates, and
//! MCP bridge and policy modules.
//!
//! Step 1 of the pkgs/id crate split (#3447 umbrella, #4403): the initial
//! capability modules moved here from `id-core` (`world_caps`, `world_limits`) and
//! from the facade package (`world_manifest`); the crate has since grown. The facade re-exports each
//! module at its historical `id::` path, so no call site outside the
//! moved modules changed.
//!
//! This crate depends on `id-core` for [`id_core::json_unique`] only;
//! the dependency points one way — `id-core` never names this crate.

pub mod account_quota;
pub mod agent_delegation;
pub mod agent_majority;
pub mod agent_quota_ledger;
pub mod cap_baseline;
pub mod cap_class;
pub mod cap_diff;
pub mod cap_notice;
pub mod declaration;
pub mod http_fetch_url;
pub mod library_attach;
pub mod library_gate;
pub mod library_manifest;
pub mod library_resolution;
pub mod mcp_bridge;
pub mod mcp_policy;
pub mod owner_review;
pub mod portal;
pub mod role_config;
pub mod surface_host;
pub mod used_caps;
pub mod work_surfaces;
pub mod world_agents;
pub mod world_caps;
pub mod world_limits;
pub mod world_manifest;
pub mod world_scopes;
