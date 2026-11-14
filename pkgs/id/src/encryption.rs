//! Group encryption — facade re-export of `id_groups::encryption`
//! (moved in step 4 of the crate split, #4407).
//!
//! The implementation — the `DataScheme` adapter ([`scheme`]), the
//! membership roster, causal ordering, history modes, welcome
//! chunking and the scheme-none notice — lives in the `id-groups`
//! crate over `id-events`' group model. Its one upward reference was
//! the frame cap: `MAX_FRAME_BYTES` has lived in `id_core::frames`
//! since step 3a (the facade's `world_session` only re-exports it),
//! so the moved code names it there and no session type crossed.
//!
//! [`scheme`]: id_groups::encryption::scheme

pub use id_groups::encryption::*;
