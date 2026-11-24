//! fleet-cert — the `fleet cert` domain (alias `fleet pki`):
//! PKI/X.509 roots, issuers, leaves, chains and bundles.
//!
//! Governing spec: `doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/
//! sources/fleet_architecture_spec_v0_3.md` (§§3, 5.4, 6, 7, 13
//! phases B–D, 24, 25, 26), tracked by developing-today/code#9157
//! under the fleet CA program epic #9155.
//!
//! Library-first (spec §6.4): every CLI verb in [`cli`] is a thin
//! shell over the public API in this crate —
//!
//! - [`algorithm`] — the algorithm registry (§§3.1, 6.3, 24.1);
//! - [`timeparse`] — the §5.4 time grammar (RFC3339 + Gregorian
//!   calendar durations);
//! - [`naming`] — generation/partition coordinates and slugs (§5.2);
//! - [`graph`] — the cross-signing graph as data/policy (§§3.2, 24.2),
//!   never a hardcoded loop;
//! - [`profile`] — leaf profiles mirroring `pkgs/idp-pki`'s `idp-ca`;
//! - [`model`] — typed requests and the bundle manifest schema (§7.1);
//! - [`engine`] — key generation, issuance, chain building and
//!   validation over rcgen + aws-lc-rs;
//! - [`bundle`] — bundle directories: manifest load/save, artifact
//!   index, checksum verification.
//!
//! Safety rails baked into the API, not just the CLI:
//! - Root/issuer signing always requires an explicitly passed signer
//!   source (a key directory argument). There is no ambient key
//!   discovery anywhere in this crate (§24.3).
//! - Legacy algorithms are policy-gated for CA use (§24.1(1)).
//! - The cross-sign matrices forbid ML-DSA→classical edges; a
//!   PQ-only trust selection is a verifier policy enforced by the
//!   chain validator (§3.2).

pub mod algorithm;
pub mod bundle;
pub mod chain;
pub mod cli;
pub mod crl;
pub mod engine;
pub mod error;
pub mod graph;
pub mod model;
pub mod naming;
pub mod profile;
pub mod revoke;
pub mod timeparse;

pub use error::{CertError, Result};

/// The CLI entry point the fleet multicall binary dispatches to:
/// `fleet cert <args...>` (and `fleet pki <args...>`). Returns the
/// process exit code (0 ok · 2 usage · 1 failure).
pub fn run(args: &[String]) -> i32 {
    cli::run(args)
}
