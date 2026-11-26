//! Graph module root (slice 9110-02, #10160; wired for #12232).
//!
//! The child modules below are the graph workstreams of program #9106
//! part A that have landed. A slice that adds a module file under this
//! root declares it here in the same change, in name order:
//! `.github/scripts/check-rust-module-reach.sh` fails when a source file
//! is not reachable from its crate root. A module that does not exist
//! yet is never declared.

pub mod append_list;
pub mod attribute;
pub mod board_view;
pub mod comment_projection;
pub mod cross_signing;
pub mod cursor;
pub mod delegation;
pub mod feed_cursor;
pub mod navigation;
pub mod navigation_wire;
pub mod query;
pub mod record;
pub mod social;
pub mod social_envelope;
pub mod social_paging;
pub mod space_binding;
pub mod term;
pub mod version;

/// Name of this module root, for registry/coverage tooling.
pub const ROOT_NAME: &str = "graph";
