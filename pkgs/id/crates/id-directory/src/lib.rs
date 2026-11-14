//! The directory layer of the `id` crate: the home-server directory,
//! its views, auth and mail surfaces, the directory log, invitations
//! and the seitan admission protocol.
//!
//! Step 6b of the pkgs/id crate split (#3447 umbrella, #4409): the
//! directory cluster moved here from `id-worlds` (`directory`,
//! `directory_auth`, `directory_mail`, `directory_view`) and from the
//! facade (`directory_log`, `invite`, `invites`, `seitan`). The facade
//! and `id-worlds` both re-export these modules at their historical
//! paths, so no call site outside the moved modules changed.
//!
//! Dependencies point one way down, per the design map: this crate
//! depends on `id-core`, `id-caps`, `id-events` and `id-sync` — never
//! on `id-worlds`. Its world-presenting surface is the
//! [`id_caps::world_scopes::WorldScopes`] vocabulary (step 6a moved
//! it into `id-caps` for exactly this reason). The relationship runs
//! the other way: `id-worlds` depends on this crate, because
//! `WorldCore` holds a [`directory::Directory`] as its own persisted
//! state (the world store opens and replays it beside the journal) —
//! the same pattern by which `WorldCore` holds `id-core` state types.

pub mod directory;
pub mod directory_auth;
pub mod directory_log;
pub mod directory_mail;
pub mod directory_view;
pub mod invite;
pub mod invites;
pub mod seitan;
