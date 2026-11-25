//! The presenter-client app: the bridge wired into a Bevy app, the local
//! avatar's intents with predicted presentation, and the 2D surfaces.
//!
//! The tick runs the bridge's order (design §2.1):
//!
//! 1. [`InputSet`]: keyboard and gamepad become a [`MoveRequest`], only
//!    while the input surface is granted.
//! 2. [`JournalSet`]: the bridge submits queued intents to the remote
//!    append gate, applies journal positions within the lag budget, then
//!    confirms intents and drops settled predictions.
//! 3. [`GameplaySet`]: the presenter logs how each of its intents
//!    settled, proposes the next move with a predicted position, composes
//!    the [`PresentedFrame`], and draws it as sprites while the render
//!    surface is granted.
//!
//! A move is proposed in [`GameplaySet`] and submitted in the next
//! tick's [`JournalSet`], so its prediction is drawn for at least one
//! frame. When the gate rejects it, the next frame draws the avatar back
//! at its applied position: the visible snap-back. When the gate accepts
//! it, the prediction holds until the confirming position applies, and
//! the frame after shows the same place as world state.
//!
//! Nothing here writes a world-state component. The avatar's next
//! position is computed from its applied [`WorldPosition`] (world state)
//! and the request, and the intent carries the result explicitly, so a
//! replay re-derives the same decision from journal data (design §4).

use std::sync::Arc;

use bevy::input::ButtonInput;
use bevy::prelude::{
    App, Color, Commands, Component, Entity, Gamepad, GamepadButton, IntoScheduleConfigs, KeyCode,
    Plugin, Query, Res, ResMut, Resource, Sprite, Transform, Update, Vec2, With, World,
};
use id_bevy_bridge::{
    AppendGate, BridgeSchema, ComponentRule, DataClass, GameplaySet, IdEntityMap, InputSet,
    IntentGate, IntentLedger, IntentOutbox, IntentOutcome, JournalLag, JournalSource, JournalTail,
    LagBudget, LocalOnly, PredicateRule, PredictedPosition, Proposal, WorldName, WorldPosition,
    applied_position, configure_bridge_schedule, presented_position,
};
use serde::Serialize;

use crate::provider::{SurfaceGrants, audio_granted, input_granted, render_granted};

/// The demo world's ID entity for the local avatar.
pub const AVATAR: &str = "avatar-you";

/// How far one move takes the avatar, in world units.
pub const STEP: f64 = 1.0;

/// Pixels per world unit on the render surface.
pub const PIXELS_PER_UNIT: f32 = 48.0;

/// The schema this app projects through: a name and a 2D position, both
/// world state.
#[must_use]
pub fn presenter_schema() -> BridgeSchema {
    let mut schema = BridgeSchema::new();
    schema.register(
        "name",
        PredicateRule {
            class: DataClass::World,
            component: ComponentRule::Name,
        },
    );
    schema.register(
        "position",
        PredicateRule {
            class: DataClass::World,
            component: ComponentRule::Position2,
        },
    );
    schema
}

/// The ID entity this presenter moves.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct LocalAvatar(pub String);

/// A move asked for by input this tick: a unit direction.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct MoveRequest(pub Option<[f64; 2]>);

/// The presenter's one intent in flight and where it predicted the
/// avatar would be.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct InFlight(Option<(u64, WorldPosition)>);

impl InFlight {
    /// The intent awaiting its outcome, if any.
    #[must_use]
    pub fn intent_id(&self) -> Option<u64> {
        self.0.map(|(intent_id, _)| intent_id)
    }
}

/// Something the presenter showed, in order.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PresenterEvent {
    /// A move was proposed and its predicted position drawn.
    Predicted {
        /// The intent.
        intent_id: u64,
        /// The predicted position.
        at: [f64; 2],
    },
    /// The journal confirmed the move; the prediction became world state.
    Reconciled {
        /// The intent.
        intent_id: u64,
        /// The confirming journal position.
        position: u64,
        /// The avatar's applied position afterwards.
        at: [f64; 2],
    },
    /// The gate rejected the move; the avatar was drawn back.
    SnappedBack {
        /// The intent.
        intent_id: u64,
        /// Where the prediction had drawn it.
        from: [f64; 2],
        /// The applied position it returned to.
        to: [f64; 2],
        /// The gate's reason.
        reason: String,
    },
}

/// Every [`PresenterEvent`] so far.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct PresenterLog {
    /// Events in the order they happened.
    pub events: Vec<PresenterEvent>,
}

/// One entity as the presenter draws it this frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DrawnEntity {
    /// The ID entity.
    pub id: String,
    /// Its world-state name, if it has one.
    pub name: Option<String>,
    /// Where it is drawn, in world units.
    pub at: [f64; 2],
    /// Whether `at` is a prediction rather than applied world state.
    pub predicted: bool,
}

/// What the render surface shows this frame, built from applied world
/// state and presentation-class predictions only.
#[derive(Resource, Debug, Clone, Default, PartialEq, Serialize)]
pub struct PresentedFrame {
    /// The journal position the frame shows.
    pub applied: Option<u64>,
    /// Drawn entities, ordered by ID.
    pub entities: Vec<DrawnEntity>,
}

impl PresentedFrame {
    /// The drawn entity for `id`, if any.
    #[must_use]
    pub fn entity(&self, id: &str) -> Option<&DrawnEntity> {
        self.entities.iter().find(|drawn| drawn.id == id)
    }
}

/// A sprite drawing one ID entity. Presentation only: it is
/// [`LocalOnly`] and carries no world-state component.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct DrawnSprite {
    /// The ID entity it draws.
    pub id: String,
}

/// A sound cue on the audio output surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cue {
    /// A move was confirmed.
    Confirm,
    /// A move was rejected and snapped back.
    SnapBack,
}

impl Cue {
    /// The cue's tone in hertz.
    #[must_use]
    pub const fn tone_hz(self) -> u32 {
        match self {
            Self::Confirm => 660,
            Self::SnapBack => 220,
        }
    }
}

/// A cue queued for the audio output surface. Presentation only.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioCue(pub Cue);

/// Wires the bridge and the presenter's systems into an app.
///
/// The app still needs the world: [`connect`] inserts the tail, the gate,
/// the local avatar and the granted surfaces.
#[derive(Debug, Clone, Copy, Default)]
pub struct PresenterPlugin;

impl Plugin for PresenterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MoveRequest>()
            .init_resource::<InFlight>()
            .init_resource::<PresenterLog>()
            .init_resource::<PresentedFrame>()
            .init_resource::<IntentOutbox>()
            .edit_schedule(Update, configure_bridge_schedule)
            .add_systems(Update, read_input.run_if(input_granted).in_set(InputSet))
            .add_systems(
                Update,
                (
                    settle_in_flight,
                    queue_cues.run_if(audio_granted),
                    propose_move,
                    compose_frame,
                    draw_sprites.run_if(render_granted),
                )
                    .chain()
                    .in_set(GameplaySet),
            );
    }
}

/// Point the app at a hosted world: `source` is its journal, `gate` its
/// append gate, `grants` the surfaces the world granted.
pub fn connect(
    world: &mut World,
    source: Arc<dyn JournalSource>,
    gate: Arc<dyn AppendGate>,
    grants: SurfaceGrants,
    budget: LagBudget,
) {
    world.insert_resource(JournalTail::new(source, presenter_schema(), budget));
    world.insert_resource(IntentGate(gate));
    world.insert_resource(LocalAvatar(AVATAR.to_owned()));
    world.insert_resource(grants);
}

/// The direction a key asks for.
const fn key_direction(key: KeyCode) -> Option<[f64; 2]> {
    match key {
        KeyCode::ArrowRight | KeyCode::KeyD => Some([1.0, 0.0]),
        KeyCode::ArrowLeft | KeyCode::KeyA => Some([-1.0, 0.0]),
        KeyCode::ArrowUp | KeyCode::KeyW => Some([0.0, 1.0]),
        KeyCode::ArrowDown | KeyCode::KeyS => Some([0.0, -1.0]),
        _ => None,
    }
}

/// The direction a gamepad button asks for.
const fn button_direction(button: GamepadButton) -> Option<[f64; 2]> {
    match button {
        GamepadButton::DPadRight => Some([1.0, 0.0]),
        GamepadButton::DPadLeft => Some([-1.0, 0.0]),
        GamepadButton::DPadUp => Some([0.0, 1.0]),
        GamepadButton::DPadDown => Some([0.0, -1.0]),
        _ => None,
    }
}

const DIRECTION_KEYS: [KeyCode; 8] = [
    KeyCode::ArrowRight,
    KeyCode::ArrowLeft,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::KeyD,
    KeyCode::KeyA,
    KeyCode::KeyW,
    KeyCode::KeyS,
];

const DIRECTION_BUTTONS: [GamepadButton; 4] = [
    GamepadButton::DPadRight,
    GamepadButton::DPadLeft,
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
];

/// Turn a just-pressed direction key or d-pad button into a move request.
fn read_input(
    keys: Option<Res<'_, ButtonInput<KeyCode>>>,
    gamepads: Query<'_, '_, &Gamepad>,
    mut request: ResMut<'_, MoveRequest>,
) {
    let from_keys = keys.and_then(|keys| {
        DIRECTION_KEYS
            .into_iter()
            .find(|key| keys.just_pressed(*key))
            .and_then(key_direction)
    });
    let from_pad = || {
        gamepads.iter().find_map(|gamepad| {
            DIRECTION_BUTTONS
                .into_iter()
                .find(|button| gamepad.just_pressed(*button))
                .and_then(button_direction)
        })
    };
    if let Some(direction) = from_keys.or_else(from_pad) {
        request.0 = Some(direction);
    }
}

const fn pair(at: WorldPosition) -> [f64; 2] {
    [at.x, at.y]
}

fn avatar_entity(world: &World) -> Option<Entity> {
    let avatar = world.get_resource::<LocalAvatar>()?;
    world.get_resource::<IdEntityMap>()?.entity(&avatar.0)
}

/// Log how the in-flight intent settled during this tick's
/// [`JournalSet`](id_bevy_bridge::JournalSet).
fn settle_in_flight(world: &mut World) {
    let Some((intent_id, predicted)) = world.resource::<InFlight>().0 else {
        return;
    };
    let Some(outcome) = world.get_resource::<IntentLedger>().and_then(|ledger| {
        ledger
            .settled()
            .iter()
            .find(|outcome| outcome.intent_id() == intent_id)
            .cloned()
    }) else {
        return;
    };
    let applied = avatar_entity(world)
        .and_then(|entity| world.get::<WorldPosition>(entity).copied())
        .map_or([0.0, 0.0], pair);
    let event = match outcome {
        IntentOutcome::Confirmed { position, .. } => PresenterEvent::Reconciled {
            intent_id,
            position,
            at: applied,
        },
        IntentOutcome::Rejected { reason, .. } => PresenterEvent::SnappedBack {
            intent_id,
            from: pair(predicted),
            to: applied,
            reason,
        },
    };
    world.resource_mut::<PresenterLog>().events.push(event);
    world.resource_mut::<InFlight>().0 = None;
}

/// Queue a cue for each outcome logged this tick.
fn queue_cues(
    log: Res<'_, PresenterLog>,
    mut seen: bevy::prelude::Local<'_, usize>,
    mut commands: Commands<'_, '_>,
) {
    for event in log.events.iter().skip(*seen) {
        let cue = match event {
            PresenterEvent::Reconciled { .. } => Some(Cue::Confirm),
            PresenterEvent::SnappedBack { .. } => Some(Cue::SnapBack),
            PresenterEvent::Predicted { .. } => None,
        };
        if let Some(cue) = cue {
            commands.spawn((AudioCue(cue), LocalOnly));
        }
    }
    *seen = log.events.len();
}

/// Propose the requested move, one intent at a time, with a predicted
/// position the frame draws at once.
fn propose_move(world: &mut World) {
    let Some(direction) = world.resource_mut::<MoveRequest>().0.take() else {
        return;
    };
    if world.resource::<InFlight>().0.is_some() {
        return;
    }
    let Some(entity) = avatar_entity(world) else {
        return;
    };
    let Some(current) = world.get::<WorldPosition>(entity).copied() else {
        return;
    };
    let target = WorldPosition {
        x: STEP.mul_add(direction[0], current.x),
        y: STEP.mul_add(direction[1], current.y),
    };
    let based_on = applied_position(world);
    let avatar = world.resource::<LocalAvatar>().0.clone();
    let intent_id = world.resource_mut::<IntentOutbox>().propose(
        based_on,
        Proposal::Upsert {
            id: avatar,
            values: vec![(
                "position".to_owned(),
                serde_json::json!([target.x, target.y]),
            )],
        },
    );
    world.entity_mut(entity).insert(PredictedPosition {
        intent_id,
        at: target,
    });
    world.resource_mut::<InFlight>().0 = Some((intent_id, target));
    world
        .resource_mut::<PresenterLog>()
        .events
        .push(PresenterEvent::Predicted {
            intent_id,
            at: pair(target),
        });
}

/// Build the frame from applied world state and predictions.
fn compose_frame(world: &mut World) {
    let mut ids: Vec<(String, Entity)> = world
        .get_resource::<IdEntityMap>()
        .map(|map| {
            map.iter()
                .map(|(id, entity, _)| (id.to_owned(), entity))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    let entities = ids
        .into_iter()
        .filter_map(|(id, entity)| {
            let (at, predicted) = presented_position(world, entity)?;
            Some(DrawnEntity {
                name: world.get::<WorldName>(entity).map(|name| name.0.clone()),
                id,
                at: pair(at),
                predicted,
            })
        })
        .collect();
    let applied = applied_position(world);
    world.insert_resource(PresentedFrame { applied, entities });
}

/// Colour for a drawn entity: the avatar, its prediction, or anything
/// else in the world.
fn colour(drawn: &DrawnEntity, avatar: Option<&LocalAvatar>) -> Color {
    let mine = avatar.is_some_and(|avatar| avatar.0 == drawn.id);
    match (mine, drawn.predicted) {
        (true, true) => Color::srgb(0.55, 0.85, 1.0),
        (true, false) => Color::srgb(0.1, 0.45, 0.9),
        (false, _) => Color::srgb(0.95, 0.7, 0.2),
    }
}

/// World units to sprite pixels.
#[allow(clippy::cast_possible_truncation)]
fn to_pixels(at: [f64; 2]) -> (f32, f32) {
    (
        at[0] as f32 * PIXELS_PER_UNIT,
        at[1] as f32 * PIXELS_PER_UNIT,
    )
}

/// Keep one sprite per drawn entity, placed where the frame says.
fn draw_sprites(
    frame: Res<'_, PresentedFrame>,
    avatar: Option<Res<'_, LocalAvatar>>,
    mut sprites: Query<'_, '_, (Entity, &DrawnSprite, &mut Transform, &mut Sprite)>,
    mut commands: Commands<'_, '_>,
) {
    let size = Vec2::splat(PIXELS_PER_UNIT * 0.8);
    let mut placed = Vec::new();
    for (entity, sprite, mut transform, mut look) in &mut sprites {
        if let Some(drawn) = frame.entity(&sprite.id) {
            let (x, y) = to_pixels(drawn.at);
            transform.translation.x = x;
            transform.translation.y = y;
            look.color = colour(drawn, avatar.as_deref());
            placed.push(sprite.id.clone());
        } else {
            commands.entity(entity).despawn();
        }
    }
    for drawn in &frame.entities {
        if placed.contains(&drawn.id) {
            continue;
        }
        let (x, y) = to_pixels(drawn.at);
        commands.spawn((
            DrawnSprite {
                id: drawn.id.clone(),
            },
            LocalOnly,
            Sprite::from_color(colour(drawn, avatar.as_deref()), size),
            Transform::from_xyz(x, y, 0.0),
        ));
    }
}

/// The sprites currently drawn, by ID entity, with their pixel position.
pub fn drawn_sprites(world: &mut World) -> Vec<(String, [f32; 2])> {
    let mut query = world.query_filtered::<(&DrawnSprite, &Transform), With<LocalOnly>>();
    let mut drawn: Vec<(String, [f32; 2])> = query
        .iter(world)
        .map(|(sprite, transform)| {
            (
                sprite.id.clone(),
                [transform.translation.x, transform.translation.y],
            )
        })
        .collect();
    drawn.sort_by(|a, b| a.0.cmp(&b.0));
    drawn
}

/// The cues queued on the audio surface so far, oldest first.
pub fn queued_cues(world: &mut World) -> Vec<Cue> {
    let mut query = world.query::<(Entity, &AudioCue)>();
    let mut cues: Vec<(Entity, Cue)> = query
        .iter(world)
        .map(|(entity, cue)| (entity, cue.0))
        .collect();
    cues.sort_by_key(|(entity, _)| *entity);
    cues.into_iter().map(|(_, cue)| cue).collect()
}

/// How far behind the journal the app is, if the tail has run.
#[must_use]
pub fn lag(world: &World) -> Option<JournalLag> {
    world.get_resource::<JournalLag>().copied()
}
