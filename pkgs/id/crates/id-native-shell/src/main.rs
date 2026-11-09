//! `id-native-shell` binary: the Bevy application target for F13.6.
//!
//! Step 1 keeps the app on [`MinimalPlugins`] (no window yet) so the
//! package builds and starts without desktop system libraries; the
//! window/render plugin set and the live session loop are step 2 of
//! design #788. What step 1 proves is the shape the design requires:
//! an isolated Bevy app target that boots on the shared client
//! identity and speaks only F13.1 contract frames (see the library
//! crate for the frame builders and view fallback).

use bevy::prelude::{App, MinimalPlugins, Startup};
use id_native_shell::{CONTRACT_NAME, CONTRACT_VERSION};

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_systems(Startup, announce)
        .run();
}

/// Announce the shell and the contract it speaks, once, at startup.
fn announce() {
    println!(
        "id-native-shell: F13.6 native reference shell; {CONTRACT_NAME} contract v{CONTRACT_VERSION}; \
         window/session wiring lands in design #788 step 2"
    );
}
