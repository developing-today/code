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
//! - [`algorithm_lab`] — per-algorithm benchmarks and environment
//!   probes behind `algorithms benchmark | doctor` (§6.1, #13729);
//! - [`timeparse`] — the §5.4 time grammar (RFC3339 + Gregorian
//!   calendar durations);
//! - [`naming`] — generation/partition coordinates and slugs (§5.2);
//! - [`graph`] — the cross-signing graph as data/policy (§§3.2, 24.2),
//!   never a hardcoded loop;
//! - [`profile`] — leaf profiles mirroring `pkgs/idp-pki`'s `idp-ca`;
//! - [`ocsp`] — the RFC 6960 responder core: delegated responder
//!   certificates, response signing, inspection and verification
//!   against the chain code (§§4.3, 6.1);
//! - [`model`] — typed requests and the bundle manifest schema (§7.1);
//! - [`engine`] — key generation, issuance, chain building and
//!   validation over rcgen + aws-lc-rs;
//! - [`bundle`] — bundle directories: manifest load/save, artifact
//!   index, checksum verification.
//! - [`inventory`] — read-only cross-bundle inventory: `inventory
//!   list|show|search|audit` over bundle directories (spec §6.1).
//! - [`batch`] — `batch validate | plan | apply | status`: a
//!   reviewed multi-step plan (issuer + leaf sets) applied
//!   all-or-nothing over bundle directories (§6.5, #13730).
//! - [`dns`] — offline CAA and TLSA/DANE change-sets: `dns plan |
//!   render | verify` against the #13724 contract (no live DNS).
//! - [`trust`] — §3.5 trust-bundle export and the broker-client
//!   trust store: two-phase rollover (stage → acknowledge →
//!   activate → retire) with last-known-good retention (BT-2,
//!   #14400; spec §27.3 item 9 in its minimal broker form).
//! - [`trust_consumer`] — operator trust verbs over a root set:
//!   `inspect`, `pin`, `verify --pins | --store`, and
//!   `export --target <consumer>` with per-consumer
//!   install/verify/remove guidance (spec §§6.1, 44; #15612).
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
pub mod algorithm_lab;
pub mod batch;
pub mod bundle;
pub mod chain;
pub mod cli;
pub mod dns;
pub mod engine;
pub mod error;
pub mod format;
pub mod graph;
pub mod inventory;
pub mod jwk;
pub mod model;
pub mod naming;
pub mod ocsp;
pub mod profile;
pub mod publish;
pub mod replace;
pub mod timeparse;
pub mod trust;
pub mod trust_consumer;

pub use error::{CertError, Result};

/// The CLI entry point the fleet multicall binary dispatches to:
/// `fleet cert <args...>` (and `fleet pki <args...>`). Returns the
/// process exit code (0 ok · 2 usage · 1 failure).
pub fn run(args: &[String]) -> i32 {
    cli::run(args)
}
