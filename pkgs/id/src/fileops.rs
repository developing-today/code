//! File operations, re-exported from `id-sync`.
//!
//! Step 3b of the pkgs/id crate split (#4406): the implementation of
//! this module lives in `id_sync::fileops`. This facade module holds
//! the re-exports and the tests that run against the facade's own
//! `LocalNode` (which stays here until step 7), so every historical
//! `id::fileops` path is unchanged.

pub use id_sync::fileops::*;
