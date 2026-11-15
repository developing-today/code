//! `id-native-shell` binary: the Bevy application target for F13.6.
//!
//! Step 2 (design #788) replaces step 1's `MinimalPlugins` placeholder
//! with Bevy's window/render/UI plugin set (`DefaultPlugins` over the
//! crate's selected features) and the shell's own render plugin
//! ([`id_native_shell::render::ShellRenderPlugin`]), which presents
//! the latest host view as a UI node tree. The live `WorldClient`
//! session loop that feeds the presented view is the remaining step-2
//! piece; until it lands the window opens on an empty view and the
//! announce line says so, rather than pretending to be connected.

use bevy::prelude::{App, DefaultPlugins, Startup};
use id_native_shell::render::ShellRenderPlugin;
use id_native_shell::{CONTRACT_NAME, CONTRACT_VERSION};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ShellRenderPlugin)
        .add_systems(Startup, announce)
        .run();
}

/// Announce the shell and the contract it speaks, once, at startup.
fn announce() {
    println!(
        "id-native-shell: F13.6 native reference shell; {CONTRACT_NAME} contract v{CONTRACT_VERSION}; \
         window/render wiring is live (design #788 step 2); the WorldClient session loop is not yet connected"
    );
}
