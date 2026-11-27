//! Library surface for the `fleet` multicall binary crate.
//!
//! The crate is primarily a binary (`src/main.rs`, one `[[bin]]`
//! target). This library target exists so `cargo test -p fleet
//! --lib ...` resolves: it exposes the `id` domain module closure
//! whose unit tests (notably the `id_serve` envelope golden tests,
//! #16326) must be selectable without building the whole multicall
//! binary harness.
//!
//! Only the modules the `id` domain references via `crate::` live
//! here (`id_key`, `id_postcard`, `id_proxy`, `id_serve`); every
//! module is gated on the same `id` feature as the binary's own
//! `mod` declarations, so feature-combination builds (notably
//! `--no-default-features` for the connect variant) see an empty
//! library that still compiles.

#[cfg(feature = "id")]
pub mod id_key;
#[cfg(feature = "id")]
pub mod id_postcard;
#[cfg(feature = "id")]
pub mod id_proxy;
#[cfg(feature = "id")]
pub mod id_serve;
