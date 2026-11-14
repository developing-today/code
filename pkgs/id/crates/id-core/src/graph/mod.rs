//! Graph module root (slice 9110-02, #10160).
//!
//! This root intentionally declares **no** child modules yet. The
//! graph workstreams (program #9106 part A) add their own distinct
//! child module files under this root as their slices land; the
//! root itself is owned by 9110-02 and consumers do not edit it.
//! Feature exports for graph modules are wired by slice 9110-09
//! once the exact module outputs exist — nothing absent is
//! declared or exported here.

/// Name of this module root, for registry/coverage tooling.
pub const ROOT_NAME: &str = "graph";
