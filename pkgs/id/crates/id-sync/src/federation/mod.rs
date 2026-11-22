//! Federation module root (slice 9110-02, #10160).
//!
//! Federation slices (program #9106 part B) live under
//! [`slices`]; each slice adds its own distinct module file and
//! consumers do not edit this root. The root records the slice
//! registry only — see [`slices::B_SLICE_REGISTRY`].

pub mod exchange_net;
pub mod record_join;
pub mod record_store;
pub mod slices;
