//! Bevy widget presentation for validated widget surfaces (#12657 step 2c).
//!
//! Two layers, deliberately separated, mirroring [`crate::scene`]
//! and [`crate::audio`]:
//!
//! - A Bevy-independent render model ([`WidgetModel`]): a validated
//!   [`WidgetSurface`] becomes a control list in document order —
//!   each control stacked on its own row, carrying the kind, label,
//!   action and toggle state the contract prescribes, plus the two
//!   derivations this renderer owns: an accent color per control
//!   (a fixed hue family per kind, lightness stepped
//!   deterministically from the control id) and the focus order
//!   (controls that deliver an action are focusable, display-only
//!   controls are not; focus starts on the first focusable control
//!   and steps with wrap-around). The contract carries no visual or
//!   interactive fields beyond the control description ("a presenter
//!   resolves each action id against the world's own actions and
//!   derives everything interactive from the description"), so every
//!   such choice lives here, in one pure place the tests can pin
//!   without a window or input device.
//! - The Bevy layer ([`WidgetRenderPlugin`]): whenever the
//!   presented view's sequence changes, one entity per control is
//!   rebuilt with the placement the model prescribes, and a focus
//!   system keeps a [`Focused`] marker on exactly the control the
//!   model's focus state names. A view whose surface is not a
//!   widget document (or no view at all) clears the control
//!   entities; shells without this plugin keep presenting the
//!   widget document's text projection through
//!   [`crate::render::ScreenGrid`], exactly as before.
//!
//! The plugin owns presentation state only: capturing keyboard or
//! pointer input, moving the focus with it, and delivering a
//! focused control's named action to the world are later wiring
//! steps that consume these same entities and the model's pure
//! navigation functions ([`WidgetModel::focus_step`]).
//!
//! World input stays on the same contract frames: the plugin only
//! reads [`PresentedView`], which the session loop already feeds.

use bevy::prelude::{
    App, Color, Commands, Component, Entity, Has, IntoScheduleConfigs, Local, Plugin, Query, Res,
    Resource, Update, With,
};
use id::world_presentation::{WidgetControl, WidgetSurface, WorldSurface};

use crate::render::PresentedView;
use crate::session::SessionView;

/// The presentation-side kind of one control, from the contract's
/// closed kind set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetControlKind {
    /// An action control (`button`).
    Button,
    /// A stateful control (`toggle`).
    Toggle,
    /// A navigation control (`link`).
    Link,
}

impl WidgetControlKind {
    /// The kind of one contract control.
    #[must_use]
    pub const fn of(control: &WidgetControl) -> Self {
        match control {
            WidgetControl::Button { .. } => Self::Button,
            WidgetControl::Toggle { .. } => Self::Toggle,
            WidgetControl::Link { .. } => Self::Link,
        }
    }
}

/// One control, presentation-ready: the contract's description plus
/// the row, focusability and accent this renderer derives for it.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetControlModel {
    /// Stable control identifier from the contract document.
    pub id: String,
    /// Human-readable control name from the contract document.
    pub label: String,
    /// The control's kind, from the closed set.
    pub kind: WidgetControlKind,
    /// The world action invoking the control delivers, when it
    /// acts; `None` names a display-only control.
    pub action: Option<String>,
    /// The control's presented state: `Some` for a toggle, `None`
    /// for the stateless kinds.
    pub on: Option<bool>,
    /// The row the control occupies in the stack: its position in
    /// document order, one control per row.
    pub row: usize,
    /// Whether keyboard focus can land on the control: exactly the
    /// controls that deliver an action.
    pub focusable: bool,
    /// Derived accent color (see [`control_color`]).
    pub accent: Color,
}

/// A whole widget document as a presentation model: control models
/// in document order plus where focus starts.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetModel {
    /// Optional caption naming the control list.
    pub title: Option<String>,
    /// Control models, in the document's order.
    pub controls: Vec<WidgetControlModel>,
    /// Where focus starts: the first focusable control's index, or
    /// `None` when no control delivers an action.
    pub focus_index: Option<usize>,
}

/// FNV-1a over the control id's bytes: the stable hash the accent
/// derivation steps lightness with. Deterministic across runs,
/// platforms and shells — the same document derives the same
/// accents everywhere.
fn id_hash(id: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// The accent color this renderer derives for one control.
///
/// A fixed hue family per kind (so a control list reads
/// consistently — buttons are blue, toggles green, links violet)
/// with the lightness stepped deterministically by the control
/// id's hash, so same-kind controls stay distinguishable.
#[must_use]
pub fn control_color(kind: WidgetControlKind, id: &str) -> Color {
    let hue = match kind {
        WidgetControlKind::Button => 210.0,
        WidgetControlKind::Toggle => 130.0,
        WidgetControlKind::Link => 265.0,
    };
    let step = f32::from(u8::try_from(id_hash(id) % 5).unwrap_or(0));
    let lightness = step.mul_add(0.05, 0.45);
    Color::hsl(hue, 0.55, lightness)
}

impl WidgetModel {
    /// Presentation model for a validated widget surface. The
    /// surface is expected to have passed [`WidgetSurface::validate`]
    /// (the session seam validates on decode), so the control set
    /// is non-empty and ids unique.
    #[must_use]
    pub fn from_surface(widgets: &WidgetSurface) -> Self {
        let controls: Vec<WidgetControlModel> = widgets
            .controls
            .iter()
            .enumerate()
            .map(|(row, control)| {
                let kind = WidgetControlKind::of(control);
                WidgetControlModel {
                    id: control.id().to_owned(),
                    label: control.label().to_owned(),
                    kind,
                    action: control.action().map(ToOwned::to_owned),
                    on: match control {
                        WidgetControl::Toggle { on, .. } => Some(*on),
                        _ => None,
                    },
                    row,
                    focusable: control.action().is_some(),
                    accent: control_color(kind, control.id()),
                }
            })
            .collect();
        let focus_index = controls.iter().position(|control| control.focusable);
        Self {
            title: widgets.title.clone(),
            controls,
            focus_index,
        }
    }

    /// Presentation model for one session view: `Some` only when
    /// the view carries a widget surface. Every other surface keeps
    /// its existing presentation (text grid / scene / audio).
    #[must_use]
    pub fn from_view(view: &SessionView) -> Option<Self> {
        match view.surface.as_ref() {
            Some(WorldSurface::Widget(widgets)) => Some(Self::from_surface(widgets)),
            _ => None,
        }
    }

    /// The focus position one step from `current` in the given
    /// direction, skipping display-only controls and wrapping
    /// around the ends of the list. `None` when no control is
    /// focusable. This is the pure navigation rule the input
    /// wiring drives; the plugin's focus marker follows it.
    #[must_use]
    pub fn focus_step(&self, current: Option<usize>, forward: bool) -> Option<usize> {
        let count = self.controls.len();
        if count == 0 || !self.controls.iter().any(|control| control.focusable) {
            return None;
        }
        let mut index = match current {
            Some(index) => index % count,
            // Stepping from no focus starts just off the end the
            // step enters from, so the first candidate examined is
            // the list's first (forward) or last (backward) control.
            None if forward => count - 1,
            None => 0,
        };
        loop {
            index = if forward {
                (index + 1) % count
            } else {
                (index + count - 1) % count
            };
            if self.controls[index].focusable {
                return Some(index);
            }
        }
    }
}

/// The deterministic example world's widget document.
///
/// The controls of the harbor the scene and audio examples show —
/// start the tour, the harbor lights as a toggle, and the link
/// into the harbor — exercising every kind in the closed set. The
/// example world emits exactly this document, so tests and demos
/// present the same widgets the contract's own tests project.
#[must_use]
pub fn example_widgets() -> WidgetSurface {
    WidgetSurface {
        version: 1,
        title: Some("Harbor Controls".to_owned()),
        controls: vec![
            WidgetControl::Button {
                id: "start".to_owned(),
                label: "Start".to_owned(),
                action: Some("start".to_owned()),
            },
            WidgetControl::Toggle {
                id: "lights".to_owned(),
                label: "Lights".to_owned(),
                action: Some("lights".to_owned()),
                on: true,
            },
            WidgetControl::Link {
                id: "enter".to_owned(),
                label: "Enter the harbor".to_owned(),
                action: "enter_harbor".to_owned(),
            },
        ],
    }
}

/// Marker for one presented control, carrying its contract id.
#[derive(Component, Debug)]
pub struct WidgetControlEntity(pub String);

/// The placement the model prescribed for one presented control.
#[derive(Component, Debug)]
pub struct ControlPlacement {
    /// The control's position in the model's control list.
    pub index: usize,
    /// The row the control occupies in the stack.
    pub row: usize,
    /// The control's kind, from the closed set.
    pub kind: WidgetControlKind,
    /// The world action invoking the control delivers, when it acts.
    pub action: Option<String>,
    /// The control's presented state (`Some` for a toggle).
    pub on: Option<bool>,
    /// Whether keyboard focus can land on the control.
    pub focusable: bool,
    /// The control's derived accent color.
    pub accent: Color,
}

/// Marker kept on exactly the control entity the model's focus
/// state names.
#[derive(Component, Debug)]
pub struct Focused;

/// The focus state of the currently presented widget view.
#[derive(Debug, Resource)]
struct WidgetFocus {
    /// The focused control's index, when any control is focusable.
    current: Option<usize>,
}

/// Bevy plugin: presents the widget surface of [`PresentedView`]
/// as placed control entities.
///
/// Register alongside [`crate::render::ShellRenderPlugin`] (which
/// owns the `PresentedView` resource). The systems only manage
/// components and resources, so tests can run the plugin under
/// `MinimalPlugins` and inspect the spawned entities headlessly.
#[derive(Debug)]
pub struct WidgetRenderPlugin;

impl Plugin for WidgetRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (rebuild_widgets, mark_focused).chain());
    }
}

/// Rebuild the control entities when the presented view's sequence
/// changes: despawn the old view's controls, restart focus at the
/// model's initial position, and spawn the new view's controls with
/// their prescribed placements. Non-widget views clear the controls
/// and the focus state.
#[allow(clippy::needless_pass_by_value)]
fn rebuild_widgets(
    presented: Res<'_, PresentedView>,
    mut last_sequence: Local<'_, Option<u64>>,
    control_entities: Query<'_, '_, Entity, With<WidgetControlEntity>>,
    mut commands: Commands<'_, '_>,
) {
    let sequence = presented.0.as_ref().map(|view| view.sequence);
    if sequence == *last_sequence {
        return;
    }
    *last_sequence = sequence;

    for entity in &control_entities {
        commands.entity(entity).despawn();
    }
    let Some(model) = presented.0.as_ref().and_then(WidgetModel::from_view) else {
        commands.remove_resource::<WidgetFocus>();
        return;
    };
    for control in &model.controls {
        commands.spawn((
            WidgetControlEntity(control.id.clone()),
            ControlPlacement {
                index: control.row,
                row: control.row,
                kind: control.kind,
                action: control.action.clone(),
                on: control.on,
                focusable: control.focusable,
                accent: control.accent,
            },
        ));
    }
    commands.insert_resource(WidgetFocus {
        current: model.focus_index,
    });
}

/// Keep the [`Focused`] marker on exactly the control entity the
/// focus state names.
#[allow(clippy::needless_pass_by_value)]
fn mark_focused(
    focus: Option<Res<'_, WidgetFocus>>,
    controls: Query<'_, '_, (Entity, &ControlPlacement, Has<Focused>)>,
    mut commands: Commands<'_, '_>,
) {
    let Some(focus) = focus else {
        return;
    };
    for (entity, placement, marked) in &controls {
        let focused = focus.current == Some(placement.index);
        if focused && !marked {
            commands.entity(entity).insert(Focused);
        }
        if !focused && marked {
            commands.entity(entity).remove::<Focused>();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::render::{ScreenGrid, ShellRenderPlugin};
    use bevy::prelude::MinimalPlugins;

    /// A session view carrying the example widgets, built through
    /// the same server-frame seam the session loop feeds.
    fn widget_view(sequence: u64) -> SessionView {
        let mut surface = serde_json::to_value(example_widgets()).unwrap();
        surface["kind"] = serde_json::json!("widget");
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
    /// the widget plugin.
    fn widget_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ShellRenderPlugin)
            .add_plugins(WidgetRenderPlugin);
        app
    }

    #[test]
    fn example_widgets_are_valid_and_the_model_is_deterministic() {
        let widgets = example_widgets();
        widgets.validate().unwrap();
        assert_eq!(widgets, example_widgets());
        assert_eq!(
            WidgetModel::from_surface(&widgets),
            WidgetModel::from_surface(&example_widgets())
        );
    }

    #[test]
    fn model_stacks_controls_in_document_order_with_their_state() {
        let model = WidgetModel::from_surface(&example_widgets());
        let ids: Vec<&str> = model.controls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["start", "lights", "enter"]);
        let rows: Vec<usize> = model.controls.iter().map(|c| c.row).collect();
        assert_eq!(rows, [0, 1, 2]);
        // Only the toggle carries state; the kinds round-trip.
        let states: Vec<Option<bool>> = model.controls.iter().map(|c| c.on).collect();
        assert_eq!(states, [None, Some(true), None]);
        let kinds: Vec<WidgetControlKind> = model.controls.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            [
                WidgetControlKind::Button,
                WidgetControlKind::Toggle,
                WidgetControlKind::Link
            ]
        );
        // Every example control acts, so every one is focusable and
        // focus starts on the first.
        assert!(model.controls.iter().all(|c| c.focusable));
        assert_eq!(model.focus_index, Some(0));
    }

    #[test]
    fn display_only_controls_are_not_focusable_and_focus_skips_them() {
        let mut widgets = example_widgets();
        widgets.controls.insert(
            1,
            WidgetControl::Button {
                id: "about".to_owned(),
                label: "About".to_owned(),
                action: None,
            },
        );
        let model = WidgetModel::from_surface(&widgets);
        assert!(!model.controls[1].focusable);
        // Focus steps over the display-only control in both
        // directions.
        assert_eq!(model.focus_step(Some(0), true), Some(2));
        assert_eq!(model.focus_step(Some(2), false), Some(0));
    }

    #[test]
    fn focus_steps_wrap_around_and_start_from_the_ends() {
        let model = WidgetModel::from_surface(&example_widgets());
        assert_eq!(model.focus_step(Some(0), true), Some(1));
        assert_eq!(model.focus_step(Some(2), true), Some(0));
        assert_eq!(model.focus_step(Some(0), false), Some(2));
        assert_eq!(model.focus_step(None, true), Some(0));
        assert_eq!(model.focus_step(None, false), Some(2));

        // A document whose controls are all display-only has no
        // focus at all.
        let display_only = WidgetSurface {
            version: 1,
            title: None,
            controls: vec![WidgetControl::Button {
                id: "about".to_owned(),
                label: "About".to_owned(),
                action: None,
            }],
        };
        let model = WidgetModel::from_surface(&display_only);
        assert_eq!(model.focus_index, None);
        assert_eq!(model.focus_step(None, true), None);
        assert_eq!(model.focus_step(Some(0), true), None);
    }

    #[test]
    fn accents_are_kind_families_stepped_by_id() {
        // Same kind and id: identical accent (determinism).
        assert_eq!(
            control_color(WidgetControlKind::Button, "start"),
            control_color(WidgetControlKind::Button, "start")
        );
        // The kind families differ for the same id.
        assert_ne!(
            control_color(WidgetControlKind::Button, "start"),
            control_color(WidgetControlKind::Toggle, "start")
        );
        assert_ne!(
            control_color(WidgetControlKind::Toggle, "start"),
            control_color(WidgetControlKind::Link, "start")
        );
    }

    #[test]
    fn from_view_only_accepts_widget_surfaces() {
        assert!(WidgetModel::from_view(&widget_view(3)).is_some());
        assert!(WidgetModel::from_view(&text_view(4)).is_none());
    }

    #[test]
    fn widget_view_spawns_one_entity_per_control() {
        let mut app = widget_app();
        app.update();
        crate::render::present_in_world(app.world_mut(), widget_view(5));
        app.update();

        let mut query = app
            .world_mut()
            .query::<(&WidgetControlEntity, &ControlPlacement)>();
        let spawned: Vec<(String, usize, Option<bool>)> = query
            .iter(app.world())
            .map(|(tag, placement)| (tag.0.clone(), placement.row, placement.on))
            .collect();
        assert_eq!(spawned.len(), 3);
        let lights = spawned.iter().find(|(id, _, _)| id == "lights").unwrap();
        assert_eq!(lights.1, 1);
        assert_eq!(lights.2, Some(true));
        let start = spawned.iter().find(|(id, _, _)| id == "start").unwrap();
        assert_eq!(start.1, 0);
        assert_eq!(start.2, None);
    }

    #[test]
    fn the_first_actionable_control_is_focused_once_presented() {
        let mut app = widget_app();
        app.update();
        crate::render::present_in_world(app.world_mut(), widget_view(5));
        app.update();

        let mut query = app
            .world_mut()
            .query::<(&WidgetControlEntity, Has<Focused>)>();
        let states: Vec<(String, bool)> = query
            .iter(app.world())
            .map(|(tag, focused)| (tag.0.clone(), focused))
            .collect();
        assert_eq!(states.len(), 3);
        assert!(states.iter().find(|(id, _)| id == "start").unwrap().1);
        assert!(!states.iter().find(|(id, _)| id == "lights").unwrap().1);
        assert!(!states.iter().find(|(id, _)| id == "enter").unwrap().1);
    }

    #[test]
    fn a_non_widget_view_clears_the_control_entities() {
        let mut app = widget_app();
        app.update();
        crate::render::present_in_world(app.world_mut(), widget_view(5));
        app.update();
        crate::render::present_in_world(app.world_mut(), text_view(6));
        app.update();
        let mut query = app.world_mut().query::<&WidgetControlEntity>();
        assert_eq!(query.iter(app.world()).count(), 0);
    }

    #[test]
    fn shells_without_the_widget_plugin_keep_the_text_projection() {
        // Parity: the text grid still presents a widget document as
        // its deterministic text projection — the fallback every
        // shell without a widget presenter shows.
        let surface = WorldSurface::Widget(example_widgets());
        let grid = ScreenGrid::from_surface(&surface);
        assert_eq!(grid.text_projection(), example_widgets().text_projection());
    }
}
