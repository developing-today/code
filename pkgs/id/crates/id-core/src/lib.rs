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
//!   `MAX_WORLD_MODULE_BYTES` are defined there and re-exported at their
//!   historical paths, and its `RuntimeLimits::from_args` takes a
//!   `WorldLimitArgs` trait the binary crate implements for its CLI
//!   argument struct. Since step 1 of the split (#4403) `world_limits`
//!   lives in the `id-caps` crate together with `world_caps` and
//!   `world_manifest`; `id-caps` depends on this crate (for
//!   `json_unique`), never the reverse.

pub mod abuse;
pub mod access;
pub mod block_list;
pub mod boundary;
pub mod boundary_surfaces;
pub mod connectivity;
pub mod discovery;
pub mod frames;
pub mod hex;
pub mod key_envelope;
pub mod seed_phrase;
pub mod ip_evidence;
pub mod json_unique;
pub mod membership;
pub mod rejection;
pub mod residency;
pub mod sanctions;
pub mod sync_method;
pub mod text;
pub mod time;
pub mod tuple;
mod unicode;
pub use unicode::is_format_char;
pub mod waiting;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_forgeries;

#[cfg(feature = "world")]
pub mod petname;
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

/// Filename for the server's Ed25519 keypair.
///
/// The node's long-term identity key, stored in the node's data directory.
pub const KEY_FILE: &str = ".iroh-key";

/// Filename for the client's Ed25519 keypair.
///
/// Used when connecting to remote peers. Separate from the server key to allow
/// different identities for serving vs. connecting.
pub const CLIENT_KEY_FILE: &str = ".iroh-key-client";

/// Filename for the serve lock file.
///
/// Created when `id serve` is running. Contains the node ID and addresses
/// for local clients to connect. Automatically removed on clean shutdown.
pub const SERVE_LOCK: &str = ".iroh-serve.lock";
