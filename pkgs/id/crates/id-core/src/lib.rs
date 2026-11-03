//! Foundation leaf modules of the `id` crate.
//!
//! Phase 1 of the pkgs/id crate split (#3447): the modules here are the
//! crate's leaves — everything in the binary crate depends on them, they
//! depended on (almost) nothing in it. The binary crate re-exports each
//! module at its historical `crate::` path, so no call site outside the
//! moved modules changed.
//!
//! The two historical exceptions are handled explicitly:
//!
//! - [`Refusal`] lived in `directory` but is produced by [`petname`]
//!   too, so the definition moved here; `directory` re-exports it.
//! - `world_limits` aggregates the limit types of the sandbox, native
//!   and session layers, so `SandboxLimits`, `NativeLimits` and
//!   `MAX_WORLD_MODULE_BYTES` are defined in [`world_limits`] and
//!   re-exported at their historical paths, and
//!   [`world_limits::RuntimeLimits::from_args`] takes a
//!   [`world_limits::WorldLimitArgs`] trait the binary crate implements
//!   for its CLI argument struct.

pub mod access;
pub mod connectivity;
pub mod discovery;
pub mod membership;
pub mod sync_method;
pub mod time;
pub mod tuple;
pub mod waiting;

#[cfg(feature = "world")]
pub mod petname;
#[cfg(feature = "world")]
pub mod world_caps;
#[cfg(feature = "world")]
pub mod world_limits;
#[cfg(feature = "world")]
pub mod world_presentation;

#[cfg(feature = "web")]
pub mod qr;

/// A request the directory refuses, carried in the error so transports can
/// choose a status without parsing messages.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// No valid credential, session or key was presented.
    #[error("{0}")]
    Unauthenticated(String),
    /// The caller is known but may not do this.
    #[error("{0}")]
    Forbidden(String),
    /// The named account, group or member does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The change clashes with what already exists.
    #[error("{0}")]
    Conflict(String),
    /// Too many requests for the same thing in a short time.
    #[error("{0}")]
    RateLimited(String),
}
