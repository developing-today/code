//! fleet-search — the Atlas search domain of the fleet multicall
//! binary (epic developing-today/code#5412).
//!
//! Two surfaces, one crate, separable by feature (the fleet-queue
//! shape):
//!
//!   * `client` — `fleet search` / `fleet search get-chunk`: the
//!     atlas-search client surface over the live Atlas service's job
//!     protocol (step 5, #5433). See [`cli`].
//!   * `serve` — `fleet serve search`: the Atlas service itself, the
//!     atlas-search bin crate's serve half rehomed (#5604). See
//!     [`serve`]. atlas-core / atlas-embed / atlas-proto are consumed
//!     as-is from the atlas-search workspace; nothing there changed.

#[cfg(feature = "client")]
mod cli;
#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
mod proto;
#[cfg(feature = "client")]
mod pyjson;
#[cfg(feature = "client")]
mod render;

#[cfg(feature = "client")]
pub use cli::run;

// Verbatim copy of atlas-search's server.rs (parity-guarded): it keeps
// that crate's whole surface (e.g. `init_service_config`, which this
// role does not expose), so unused/visibility lints are allowed here
// rather than editing the copy.
#[cfg(feature = "serve")]
pub mod serve;
#[cfg(feature = "serve")]
#[allow(dead_code, private_interfaces)]
pub mod server;
