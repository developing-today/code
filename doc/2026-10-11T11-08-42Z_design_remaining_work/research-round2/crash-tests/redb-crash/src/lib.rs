//! Power-loss testing for redb without a kernel or FUSE dependency.
//!
//! - `faultfs`: a `StorageBackend` that loses unsynced writes, tears the write in flight, and cuts
//!   power at any storage call.
//! - `workload`: 50 redb write transactions and the model each prefix must match.
//! - `harness`: runs one cut, reopens the image, and classifies the result.

pub mod faultfs;
pub mod harness;
pub mod workload;
