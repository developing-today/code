//! Client-only Rust facade for the native Android client v1.
//!
//! Slice 1 of #7938 (child #13403; design
//! `doc/2026-11-08T22-06-00Z_design_id_android_client`): the Kotlin shell
//! gets a server-free core boundary it can build against for `arm64-v8a`.
//!
//! Dependency direction is one way down, per the #3447 seam map: this
//! facade re-exports the client-safe leaves (`id-core`, `id-caps`,
//! `id-config`, `id-store`, `id-events`, `id-groups`, `id-sync`,
//! `id-directory`) and never depends on `id-worlds` (which owns
//! `wasmtime`, `russh`, and the serve/listener surfaces), `id-web`
//! (which owns `axum`), `id-cli`, or the shell crates. The
//! `tests/android_dependency_boundary.rs` check enforces that boundary;
//! keep it green when adding re-exports.
//!
//! No world hosting, remote-drive integration, recovery semantics,
//! phone-provider surfaces, push service, or background-presence promise
//! lives here — those are later slices.

pub mod session;

pub use session::{ClientIdentity, SessionError, TestWorld};
