//! Presenter-client reference app for ID worlds, Bevy slice 4 (#13705).
//!
//! Design of record: `doc/2026-11-08_design_id5781.md`, §3 (the
//! presenter-client posture comes first), §5 (provider face) and Q4 (2D
//! is the first presenter target). The app is a full-engine Bevy app that
//! joins a world hosted in another process, projects its state and
//! submits intents through the bridge (slices 1 to 3) unchanged.
//!
//! - [`remote`]: the link to the host process. [`remote::RemoteWorld`]
//!   is the bridge's journal source and append gate, so the gate stays
//!   outside the presenter.
//! - [`provider`]: the v1 device manifest the app declares, and the
//!   surfaces the world's ordinary capability grants enable.
//! - [`app`]: the bridge in a Bevy app, the local avatar's moves with
//!   predicted presentation that reconciles or snaps back, the 2D sprite
//!   surface and audio cues.
//! - [`host`]: the demo arena the binary serves in `host` mode.
//!
//! [`run_script`] drives the app headlessly from a move script; the
//! binary's `script` mode and the end-to-end tests both use it.

pub mod app;
pub mod host;
pub mod provider;
pub mod remote;

use std::sync::Arc;

use bevy::input::ButtonInput;
use bevy::prelude::{App, Gamepad, GamepadButton, KeyCode, MinimalPlugins};
use id_bevy_bridge::{AppendGate, JournalSource, LagBudget, check_replay};
use serde::Serialize;

pub use app::{
    AVATAR, AudioCue, Cue, DrawnEntity, PresentedFrame, PresenterEvent, PresenterLog,
    PresenterPlugin, connect, drawn_sprites, presenter_schema, queued_cues,
};
pub use provider::{Surface, SurfaceGrants, device_manifest};
pub use remote::{RemoteError, RemoteWorld};

/// Render surface size the app declares, in pixels.
pub const SURFACE_SIZE: (u32, u32) = (640, 480);

/// Positions the app applies per tick.
pub const PRESENTER_LAG_BUDGET: usize = 8;

/// Ticks the app runs after each scripted press so the move settles.
pub const TICKS_PER_PRESS: usize = 3;

/// An app with the presenter wired in and no window: for scripts and
/// tests.
#[must_use]
pub fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(PresenterPlugin);
    app
}

/// Join the world behind `link`: say hello with the app's manifest,
/// derive the granted surfaces, and connect the bridge to it.
///
/// # Errors
///
/// The host refused the manifest, granted an unknown provider name, or
/// the link failed.
pub fn join(app: &mut App, link: &Arc<RemoteWorld>) -> Result<SurfaceGrants, RemoteError> {
    let (width, height) = SURFACE_SIZE;
    let granted = link.hello(&device_manifest(width, height))?;
    let grants = SurfaceGrants::from_granted_names(&granted)
        .map_err(|error| RemoteError::Refused(format!("{error:?}")))?;
    let source: Arc<dyn JournalSource> = Arc::clone(link);
    let gate: Arc<dyn AppendGate> = Arc::clone(link);
    connect(
        app.world_mut(),
        source,
        gate,
        grants.clone(),
        LagBudget::new(PRESENTER_LAG_BUDGET),
    );
    Ok(grants)
}

/// One scripted press: a direction key, or a d-pad button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// A keyboard key.
    Key(KeyCode),
    /// A gamepad button.
    Pad(GamepadButton),
}

/// Parse a move script: `R`, `L`, `U`, `D` press arrow keys; `r`, `l`,
/// `u`, `d` press the gamepad's d-pad. Anything else is refused.
///
/// # Errors
///
/// Names the first character that is not a move.
pub fn parse_script(script: &str) -> Result<Vec<Press>, char> {
    script
        .chars()
        .map(|step| match step {
            'R' => Ok(Press::Key(KeyCode::ArrowRight)),
            'L' => Ok(Press::Key(KeyCode::ArrowLeft)),
            'U' => Ok(Press::Key(KeyCode::ArrowUp)),
            'D' => Ok(Press::Key(KeyCode::ArrowDown)),
            'r' => Ok(Press::Pad(GamepadButton::DPadRight)),
            'l' => Ok(Press::Pad(GamepadButton::DPadLeft)),
            'u' => Ok(Press::Pad(GamepadButton::DPadUp)),
            'd' => Ok(Press::Pad(GamepadButton::DPadDown)),
            other => Err(other),
        })
        .collect()
}

/// What a scripted run showed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScriptRun {
    /// The surfaces the world granted.
    pub surfaces: Vec<&'static str>,
    /// Every frame the render surface was given, one per tick.
    pub frames: Vec<PresentedFrame>,
    /// What the presenter showed, in order.
    pub events: Vec<PresenterEvent>,
    /// Cues queued on the audio surface.
    pub cues: Vec<Cue>,
    /// Sprites drawn at the end, by ID entity, in pixels.
    pub sprites: Vec<(String, [f32; 2])>,
    /// Whether the final world matched a replay of the host's journal
    /// (design §4), and the divergence if not.
    pub replay: Result<(), String>,
}

/// Run one tick and keep its frame.
fn tick(app: &mut App, frames: &mut Vec<PresentedFrame>) {
    app.update();
    frames.push(app.world().resource::<PresentedFrame>().clone());
    // A scripted press lasts one tick: release it and forget the edge,
    // so the next press of the same key is a new press.
    if let Some(mut keys) = app.world_mut().get_resource_mut::<ButtonInput<KeyCode>>() {
        keys.release_all();
        keys.clear();
    }
    let mut pads = app.world_mut().query::<&mut Gamepad>();
    for mut pad in pads.iter_mut(app.world_mut()) {
        pad.digital_mut().release_all();
        pad.digital_mut().clear();
    }
}

/// Join the world behind `link`, press `presses` one at a time with a few
/// ticks after each, and report what the app showed.
///
/// # Errors
///
/// Joining failed.
pub fn run_script(link: &Arc<RemoteWorld>, presses: &[Press]) -> Result<ScriptRun, RemoteError> {
    let mut app = headless_app();
    let grants = join(&mut app, link)?;
    app.world_mut()
        .insert_resource(ButtonInput::<KeyCode>::default());
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    let mut frames = Vec::new();
    // Catch up with the world before the first press.
    for _ in 0..TICKS_PER_PRESS {
        tick(&mut app, &mut frames);
    }
    for press in presses {
        match press {
            Press::Key(key) => {
                if let Some(mut keys) = app.world_mut().get_resource_mut::<ButtonInput<KeyCode>>() {
                    keys.press(*key);
                }
            }
            Press::Pad(button) => {
                if let Some(mut gamepad) = app.world_mut().get_mut::<Gamepad>(pad) {
                    gamepad.digital_mut().press(*button);
                }
            }
        }
        for _ in 0..TICKS_PER_PRESS {
            tick(&mut app, &mut frames);
        }
    }
    let journal = link.whole_journal()?;
    let replay = check_replay(app.world_mut(), &presenter_schema(), &journal)
        .map_err(|divergence| divergence.to_string());
    let events = app.world().resource::<PresenterLog>().events.clone();
    let cues = queued_cues(app.world_mut());
    let sprites = drawn_sprites(app.world_mut());
    Ok(ScriptRun {
        surfaces: grants.labels(),
        frames,
        events,
        cues,
        sprites,
        replay,
    })
}
