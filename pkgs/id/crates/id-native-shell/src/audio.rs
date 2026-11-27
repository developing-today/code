//! Bevy audio presentation for validated audio surfaces (#12657 step 2b).
//!
//! Two layers, deliberately separated, mirroring [`crate::scene`]:
//!
//! - A Bevy-independent render model ([`AudioModel`]): a validated
//!   [`AudioSurface`] becomes a cue schedule — the cues sound one at
//!   a time in document order, each starting when the previous one
//!   ends, and a repeating cue loops until the view changes (so any
//!   cue after it waits indefinitely). The contract carries no
//!   audible fields beyond the cue description ("a presenter
//!   resolves each cue id against the world's own assets and derives
//!   everything audible from the description"), so the model also
//!   derives each cue's fallback tone: a pitch stepped
//!   deterministically from the cue id, for presenters that hold no
//!   world asset for the cue. Every derivation lives here, in one
//!   pure place the tests can pin without an audio device.
//! - The Bevy layer ([`AudioRenderPlugin`]): whenever the presented
//!   view's sequence changes, one entity per cue is rebuilt with the
//!   schedule the model prescribes, and a playback system keeps a
//!   [`Sounding`] marker on exactly the cues the model's phase
//!   function calls audible at the view's playhead. A view whose
//!   surface is not audio (or no view at all) clears the cue
//!   entities; shells without this plugin keep presenting the
//!   audio's text projection through [`crate::render::ScreenGrid`],
//!   exactly as before.
//!
//! The plugin owns presentation state only: this crate's Bevy
//! feature set carries no audio output backend, so device playback
//! is a later wiring step that consumes these same entities and
//! this same schedule.
//!
//! World input stays on the same contract frames: the plugin only
//! reads [`PresentedView`], which the session loop already feeds.

use std::time::Duration;

use bevy::prelude::{
    App, Commands, Component, Entity, Has, IntoScheduleConfigs, Local, Plugin, Query, Res,
    Resource, Time, Update, With,
};
use id::world_presentation::{AudioFormat, AudioSurface, WorldSurface};

use crate::render::PresentedView;
use crate::session::SessionView;

/// One cue, presentation-ready: the contract's description plus the
/// schedule slot and fallback tone this renderer derives for it.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioCueModel {
    /// Stable cue identifier from the contract document.
    pub id: String,
    /// Human-readable cue name from the contract document.
    pub label: String,
    /// The referenced sound's encoding, from the closed format set.
    pub format: AudioFormat,
    /// The sound's length in milliseconds.
    pub duration_ms: u32,
    /// Whether the sound repeats until the view changes.
    pub repeat: bool,
    /// When the cue starts sounding, in milliseconds after the view
    /// is presented: the sum of the durations of the cues before it
    /// in document order.
    pub start_ms: u64,
    /// The derived fallback tone pitch in hertz (see [`cue_tone_hz`]).
    pub tone_hz: f32,
}

/// A whole audio surface as a presentation model: cue models in
/// document order plus the schedule's total length.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioModel {
    /// Optional caption naming the cue list.
    pub title: Option<String>,
    /// Cue models, in the document's order.
    pub cues: Vec<AudioCueModel>,
    /// Total schedule length in milliseconds, or `None` when a
    /// repeating cue makes the schedule open-ended: it loops until
    /// the view changes and the schedule never completes.
    pub total_ms: Option<u64>,
}

/// Where one cue stands at a given playhead position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CuePhase {
    /// The cue's turn has not come (or never comes, when an earlier
    /// cue repeats).
    Waiting,
    /// The cue is sounding, this far into its one pass.
    Sounding {
        /// Milliseconds since the cue started.
        offset_ms: u64,
    },
    /// The cue is sounding, this far into its current repetition.
    Looping {
        /// Milliseconds since the current repetition started.
        offset_ms: u64,
    },
    /// The cue's one pass has ended.
    Finished,
}

/// FNV-1a over the cue id's bytes: the stable hash the tone
/// derivation steps pitch with. Deterministic across runs,
/// platforms and shells — the same document derives the same tones
/// everywhere.
fn id_hash(id: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// The fallback tone pitch this renderer derives for one cue.
///
/// Thirteen semitone steps over the octave above 220 Hz, chosen by
/// the cue id's hash, so a cue list without world assets still
/// sounds as distinguishable tones and the same cue sounds the same
/// in every shell.
#[must_use]
pub fn cue_tone_hz(id: &str) -> f32 {
    let step = id_hash(id) % 13;
    #[allow(clippy::cast_precision_loss)]
    let semitones = step as f32;
    220.0 * (semitones / 12.0).exp2()
}

impl AudioModel {
    /// Presentation model for a validated audio surface. The
    /// surface is expected to have passed
    /// [`AudioSurface::validate`] (the session seam validates on
    /// decode), so cue durations are non-zero and ids unique.
    #[must_use]
    pub fn from_surface(audio: &AudioSurface) -> Self {
        let mut start_ms: u64 = 0;
        let mut open_ended = false;
        let cues: Vec<AudioCueModel> = audio
            .cues
            .iter()
            .map(|cue| {
                let model = AudioCueModel {
                    id: cue.id.clone(),
                    label: cue.label.clone(),
                    format: cue.format,
                    duration_ms: cue.duration_ms,
                    repeat: cue.repeat,
                    start_ms,
                    tone_hz: cue_tone_hz(&cue.id),
                };
                start_ms += u64::from(cue.duration_ms);
                open_ended |= cue.repeat;
                model
            })
            .collect();
        Self {
            title: audio.title.clone(),
            cues,
            total_ms: if open_ended { None } else { Some(start_ms) },
        }
    }

    /// Presentation model for one session view: `Some` only when
    /// the view carries an audio surface. Every other surface keeps
    /// its existing presentation (text grid / form widgets).
    #[must_use]
    pub fn from_view(view: &SessionView) -> Option<Self> {
        match view.surface.as_ref() {
            Some(WorldSurface::Audio(audio)) => Some(Self::from_surface(audio)),
            _ => None,
        }
    }

    /// The phase of the cue at `index` when the view has been
    /// presented for `playhead_ms` milliseconds. An out-of-range
    /// index reports [`CuePhase::Finished`].
    #[must_use]
    pub fn cue_phase(&self, index: usize, playhead_ms: u64) -> CuePhase {
        let Some(cue) = self.cues.get(index) else {
            return CuePhase::Finished;
        };
        // A repeating cue loops until the view changes, so the cues
        // behind it never get a turn.
        if self.cues[..index].iter().any(|earlier| earlier.repeat) {
            return CuePhase::Waiting;
        }
        if playhead_ms < cue.start_ms {
            return CuePhase::Waiting;
        }
        let offset = playhead_ms - cue.start_ms;
        let duration = u64::from(cue.duration_ms);
        if cue.repeat {
            return CuePhase::Looping {
                offset_ms: offset % duration,
            };
        }
        if offset < duration {
            CuePhase::Sounding { offset_ms: offset }
        } else {
            CuePhase::Finished
        }
    }
}

/// The deterministic example world's audio.
///
/// The sound side of the harbor the scene example shows — a buoy
/// bell, gulls, then waves repeating — exercising one-shot cues and
/// a looping cue across two of the closed format set's encodings.
/// The example world emits exactly this document, so tests and
/// demos present the same audio the contract's own tests project.
#[must_use]
pub fn example_audio() -> AudioSurface {
    use id::world_presentation::AudioCue;
    AudioSurface {
        version: 1,
        title: Some("Harbor sounds".to_owned()),
        cues: vec![
            AudioCue {
                id: "buoy-bell".to_owned(),
                label: "Buoy bell".to_owned(),
                format: AudioFormat::Wav,
                duration_ms: 1_200,
                repeat: false,
            },
            AudioCue {
                id: "gulls".to_owned(),
                label: "Gulls".to_owned(),
                format: AudioFormat::Mp3,
                duration_ms: 2_400,
                repeat: false,
            },
            AudioCue {
                id: "waves".to_owned(),
                label: "Waves".to_owned(),
                format: AudioFormat::Ogg,
                duration_ms: 6_000,
                repeat: true,
            },
        ],
    }
}

/// Marker for one presented cue, carrying its contract id.
#[derive(Component, Debug)]
pub struct AudioCueEntity(pub String);

/// The schedule the model prescribed for one presented cue.
#[derive(Component, Debug)]
pub struct CueSchedule {
    /// The cue's position in the model's cue list.
    pub index: usize,
    /// Milliseconds after presentation when the cue starts.
    pub start_ms: u64,
    /// The cue's length in milliseconds.
    pub duration_ms: u32,
    /// Whether the cue loops until the view changes.
    pub repeat: bool,
    /// The cue's derived fallback tone pitch in hertz.
    pub tone_hz: f32,
}

/// Marker kept on exactly the cue entities the model's phase
/// function calls audible at the current playhead.
#[derive(Component, Debug)]
pub struct Sounding;

/// The audio model of the currently presented view, if any.
#[derive(Debug, Resource)]
struct PresentedAudio(AudioModel);

/// When the current view's playhead started, on the app clock.
#[derive(Debug, Resource)]
struct AudioClock {
    /// The app-clock reading at the moment the view was rebuilt.
    start: Duration,
}

/// Bevy plugin: presents the audio surface of [`PresentedView`] as
/// scheduled cue entities.
///
/// Register alongside [`crate::render::ShellRenderPlugin`] (which
/// owns the `PresentedView` resource). The systems only manage
/// components and resources, so tests can run the plugin under
/// `MinimalPlugins` and inspect the spawned entities headlessly.
#[derive(Debug)]
pub struct AudioRenderPlugin;

impl Plugin for AudioRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (rebuild_audio, mark_sounding).chain());
    }
}

/// Rebuild the cue entities when the presented view's sequence
/// changes: despawn the old view's cues, restart the playhead, and
/// spawn the new view's cues with their prescribed schedules.
/// Non-audio views clear the cues and the playback state.
#[allow(clippy::needless_pass_by_value)]
fn rebuild_audio(
    presented: Res<'_, PresentedView>,
    time: Res<'_, Time>,
    mut last_sequence: Local<'_, Option<u64>>,
    cue_entities: Query<'_, '_, Entity, With<AudioCueEntity>>,
    mut commands: Commands<'_, '_>,
) {
    let sequence = presented.0.as_ref().map(|view| view.sequence);
    if sequence == *last_sequence {
        return;
    }
    *last_sequence = sequence;

    for entity in &cue_entities {
        commands.entity(entity).despawn();
    }
    let Some(model) = presented.0.as_ref().and_then(AudioModel::from_view) else {
        commands.remove_resource::<PresentedAudio>();
        commands.remove_resource::<AudioClock>();
        return;
    };
    for (index, cue) in model.cues.iter().enumerate() {
        commands.spawn((
            AudioCueEntity(cue.id.clone()),
            CueSchedule {
                index,
                start_ms: cue.start_ms,
                duration_ms: cue.duration_ms,
                repeat: cue.repeat,
                tone_hz: cue.tone_hz,
            },
        ));
    }
    commands.insert_resource(AudioClock {
        start: time.elapsed(),
    });
    commands.insert_resource(PresentedAudio(model));
}

/// Keep the [`Sounding`] marker on exactly the cues the model's
/// phase function calls audible at the view's playhead.
#[allow(clippy::needless_pass_by_value)]
fn mark_sounding(
    model: Option<Res<'_, PresentedAudio>>,
    clock: Option<Res<'_, AudioClock>>,
    time: Res<'_, Time>,
    cues: Query<'_, '_, (Entity, &CueSchedule, Has<Sounding>)>,
    mut commands: Commands<'_, '_>,
) {
    let (Some(model), Some(clock)) = (model, clock) else {
        return;
    };
    let playhead_ms =
        u64::try_from(time.elapsed().saturating_sub(clock.start).as_millis()).unwrap_or(u64::MAX);
    for (entity, schedule, marked) in &cues {
        let audible = matches!(
            model.0.cue_phase(schedule.index, playhead_ms),
            CuePhase::Sounding { .. } | CuePhase::Looping { .. }
        );
        if audible && !marked {
            commands.entity(entity).insert(Sounding);
        }
        if !audible && marked {
            commands.entity(entity).remove::<Sounding>();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::render::{ScreenGrid, ShellRenderPlugin};
    use bevy::prelude::MinimalPlugins;

    /// A session view carrying the example audio, built through the
    /// same server-frame seam the session loop feeds.
    fn audio_view(sequence: u64) -> SessionView {
        let mut surface = serde_json::to_value(example_audio()).unwrap();
        surface["kind"] = serde_json::json!("audio");
        let mut state = crate::session::SessionState::default();
        state
            .apply_server_frame(serde_json::json!({
                "type": "view", "sequence": sequence, "data": "fallback",
                "surface": surface,
                "view_revision": 0
            }))
            .unwrap();
        state.view().unwrap().clone()
    }

    fn text_view(sequence: u64) -> SessionView {
        let mut state = crate::session::SessionState::default();
        state
            .apply_server_frame(serde_json::json!({
                "type": "view", "sequence": sequence, "data": "plain",
                "view_revision": 0
            }))
            .unwrap();
        state.view().unwrap().clone()
    }

    /// Headless app: the render plugin (owns `PresentedView`) plus
    /// the audio plugin.
    fn audio_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ShellRenderPlugin)
            .add_plugins(AudioRenderPlugin);
        app
    }

    #[test]
    fn example_audio_is_valid_and_the_model_is_deterministic() {
        let audio = example_audio();
        audio.validate().unwrap();
        assert_eq!(audio, example_audio());
        assert_eq!(
            AudioModel::from_surface(&audio),
            AudioModel::from_surface(&example_audio())
        );
    }

    #[test]
    fn model_schedules_cues_back_to_back_in_document_order() {
        let model = AudioModel::from_surface(&example_audio());
        let ids: Vec<&str> = model.cues.iter().map(|cue| cue.id.as_str()).collect();
        assert_eq!(ids, ["buoy-bell", "gulls", "waves"]);
        let starts: Vec<u64> = model.cues.iter().map(|cue| cue.start_ms).collect();
        assert_eq!(starts, [0, 1_200, 3_600]);
        // The waves cue repeats, so the schedule is open-ended.
        assert_eq!(model.total_ms, None);

        // Without a repeating cue the total is the sum of durations.
        let mut finite = example_audio();
        finite.cues[2].repeat = false;
        let model = AudioModel::from_surface(&finite);
        assert_eq!(model.total_ms, Some(9_600));
    }

    #[test]
    fn cue_tones_are_id_derived_within_one_octave() {
        // Same id: identical tone (determinism).
        assert_eq!(cue_tone_hz("waves"), cue_tone_hz("waves"));
        // Every derived tone sits in the octave above 220 Hz, and
        // the example cues do not all share one pitch.
        let tones: Vec<f32> = ["buoy-bell", "gulls", "waves"]
            .iter()
            .map(|id| cue_tone_hz(id))
            .collect();
        assert!(
            tones.iter().all(|tone| (220.0..=440.0).contains(tone)),
            "tones {tones:?} outside 220..=440"
        );
        assert!(
            tones
                .iter()
                .any(|tone| tone.to_bits() != tones[0].to_bits())
        );
    }

    #[test]
    fn phases_follow_the_schedule_until_a_repeat_loops() {
        let model = AudioModel::from_surface(&example_audio());
        assert_eq!(model.cue_phase(0, 0), CuePhase::Sounding { offset_ms: 0 });
        assert_eq!(
            model.cue_phase(0, 1_199),
            CuePhase::Sounding { offset_ms: 1_199 }
        );
        assert_eq!(model.cue_phase(0, 1_200), CuePhase::Finished);
        assert_eq!(model.cue_phase(1, 1_199), CuePhase::Waiting);
        assert_eq!(
            model.cue_phase(1, 1_200),
            CuePhase::Sounding { offset_ms: 0 }
        );
        assert_eq!(model.cue_phase(2, 3_599), CuePhase::Waiting);
        assert_eq!(
            model.cue_phase(2, 3_600),
            CuePhase::Looping { offset_ms: 0 }
        );
        // One full repetition plus 500 ms: the offset wraps.
        assert_eq!(
            model.cue_phase(2, 3_600 + 6_000 + 500),
            CuePhase::Looping { offset_ms: 500 }
        );
        assert_eq!(model.cue_phase(9, 0), CuePhase::Finished);
    }

    #[test]
    fn cues_behind_a_repeating_cue_never_sound() {
        let mut audio = example_audio();
        audio.cues[0].repeat = true;
        audio.cues[2].repeat = false;
        let model = AudioModel::from_surface(&audio);
        assert_eq!(
            model.cue_phase(0, 60_000),
            CuePhase::Looping {
                offset_ms: 60_000 % 1_200
            }
        );
        assert_eq!(model.cue_phase(1, 60_000), CuePhase::Waiting);
        assert_eq!(model.cue_phase(2, 60_000), CuePhase::Waiting);
    }

    #[test]
    fn from_view_only_accepts_audio_surfaces() {
        assert!(AudioModel::from_view(&audio_view(3)).is_some());
        assert!(AudioModel::from_view(&text_view(4)).is_none());
    }

    #[test]
    fn audio_view_spawns_one_entity_per_cue() {
        let mut app = audio_app();
        app.update();
        crate::render::present_in_world(app.world_mut(), audio_view(5));
        app.update();

        let mut query = app.world_mut().query::<(&AudioCueEntity, &CueSchedule)>();
        let spawned: Vec<(String, u64, bool)> = query
            .iter(app.world())
            .map(|(tag, schedule)| (tag.0.clone(), schedule.start_ms, schedule.repeat))
            .collect();
        assert_eq!(spawned.len(), 3);
        let waves = spawned.iter().find(|(id, _, _)| id == "waves").unwrap();
        assert_eq!(waves.1, 3_600);
        assert!(waves.2);
        let bell = spawned.iter().find(|(id, _, _)| id == "buoy-bell").unwrap();
        assert_eq!(bell.1, 0);
        assert!(!bell.2);
    }

    #[test]
    fn the_first_cue_is_sounding_once_presented() {
        let mut app = audio_app();
        app.update();
        crate::render::present_in_world(app.world_mut(), audio_view(5));
        app.update();

        // The playhead sits at the start of the schedule: the bell
        // (start 0) is sounding, the later cues are not.
        let mut query = app.world_mut().query::<(&AudioCueEntity, Has<Sounding>)>();
        let states: Vec<(String, bool)> = query
            .iter(app.world())
            .map(|(tag, sounding)| (tag.0.clone(), sounding))
            .collect();
        assert_eq!(states.len(), 3);
        assert!(states.iter().find(|(id, _)| id == "buoy-bell").unwrap().1);
        assert!(!states.iter().find(|(id, _)| id == "gulls").unwrap().1);
        assert!(!states.iter().find(|(id, _)| id == "waves").unwrap().1);
    }

    #[test]
    fn a_non_audio_view_clears_the_cue_entities() {
        let mut app = audio_app();
        app.update();
        crate::render::present_in_world(app.world_mut(), audio_view(5));
        app.update();
        crate::render::present_in_world(app.world_mut(), text_view(6));
        app.update();
        let mut query = app.world_mut().query::<&AudioCueEntity>();
        assert_eq!(query.iter(app.world()).count(), 0);
    }

    #[test]
    fn shells_without_the_audio_plugin_keep_the_text_projection() {
        // Parity: the text grid still presents an audio surface as
        // its deterministic text projection — the fallback every
        // shell without an audio player shows.
        let surface = WorldSurface::Audio(example_audio());
        let grid = ScreenGrid::from_surface(&surface);
        assert_eq!(grid.text_projection(), example_audio().text_projection());
    }
}
