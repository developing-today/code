//! Triples module root (slice 9110-02, #10160).
//!
//! No triples child module has landed yet, so none is declared. A slice
//! that adds a module file under this root declares it here in the same
//! change, in name order: `.github/scripts/check-id-module-reach.sh`
//! fails when a source file is not reachable from its crate root.

/// Name of this module root, for registry/coverage tooling.
pub const ROOT_NAME: &str = "triples";
