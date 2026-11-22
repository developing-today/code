//! Bevy rendering for validated contract surfaces (design #788 step 2).
//!
//! Two layers, deliberately separated:
//!
//! - A Bevy-independent render model ([`ScreenGrid`]): a validated
//!   [`WorldSurface`] (or the legacy `data` fallback) becomes a grid of
//!   styled cells. This layer is pure, so the contract-parity tests can
//!   prove the native shell presents exactly what the terminal shell
//!   presents, with no window, GPU, or asset server involved.
//! - The Bevy layer ([`ShellRenderPlugin`]): the current view lives in
//!   the [`PresentedView`] resource (the session loop feeds it; tests
//!   and the binary seed it), and a system rebuilds the Bevy UI node
//!   tree — one row node per grid row, one text span per run of
//!   equally-styled cells — whenever the view's sequence changes.
//!
//! Colors are the contract's xterm-256 palette indices, mapped to sRGB
//! by [`palette_color`]. Bold is carried in the model; the UI spans use
//! the default font, so bold is not yet visually distinct — the model
//! keeps the flag so a bold font asset can be wired without a contract
//! or model change.

use bevy::prelude::{
    App, Color, Commands, Component, Entity, Plugin, Query, Res, Resource, Startup, Text,
    TextColor, TextSpan, Update, With, World,
};
use id::world_presentation::WorldSurface;

use crate::form::{FormPresenter, FormStatusOutcome, FormWidgets};
use crate::session::SessionView;

/// One styled cell of the render model.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderCell {
    /// Glyph(s) for this cell; empty for a wide glyph's continuation.
    pub symbol: String,
    /// Foreground color (xterm-256 palette index from the surface).
    pub fg: Color,
    /// Background color (xterm-256 palette index from the surface).
    pub bg: Color,
    /// Bold flag from the surface cell.
    pub bold: bool,
    /// Terminal columns this cell occupies (2 for a wide glyph).
    pub columns: u8,
}

impl RenderCell {
    /// An unstyled cell carrying one symbol (fallback / text surfaces).
    fn plain(symbol: &str) -> Self {
        Self {
            symbol: symbol.to_owned(),
            fg: Color::WHITE,
            bg: Color::BLACK,
            bold: false,
            columns: 1,
        }
    }
}

/// A whole view as a grid of styled cells, ready for a presenter.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenGrid {
    /// Width in terminal columns.
    pub width: u16,
    /// Height in terminal rows.
    pub height: u16,
    /// Row-major rows of cells; each row's cells sum to `width` columns
    /// for a `Screen` surface, and are single unstyled runs otherwise.
    pub rows: Vec<Vec<RenderCell>>,
}

impl ScreenGrid {
    /// Render model for a validated surface.
    #[must_use]
    pub fn from_surface(surface: &WorldSurface) -> Self {
        match surface {
            WorldSurface::Screen(screen) => {
                let width = screen.width;
                let height = screen.height;
                let mut rows = Vec::with_capacity(usize::from(height));
                for row in screen.cells.chunks(usize::from(width.max(1))) {
                    rows.push(
                        row.iter()
                            .map(|cell| RenderCell {
                                symbol: cell.symbol.clone(),
                                fg: palette_color(cell.fg),
                                bg: palette_color(cell.bg),
                                bold: cell.bold,
                                columns: cell.columns,
                            })
                            .collect(),
                    );
                }
                Self {
                    width,
                    height,
                    rows,
                }
            }
            // Text and Form surfaces have no per-cell styling in the
            // contract; present their deterministic text projection as
            // unstyled lines, exactly as the terminal shell does.
            other => Self::from_text(&other.text_projection()),
        }
    }

    /// Render model for plain text (the legacy `data` fallback).
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let rows: Vec<Vec<RenderCell>> = text
            .lines()
            .map(|line| {
                if line.is_empty() {
                    Vec::new()
                } else {
                    vec![RenderCell::plain(line)]
                }
            })
            .collect();
        let width = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol.chars().count())
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0);
        Self {
            width: u16::try_from(width).unwrap_or(u16::MAX),
            height: u16::try_from(rows.len()).unwrap_or(u16::MAX),
            rows,
        }
    }

    /// Render model for one session view: the validated surface when
    /// the host supplied one, the legacy `data` text otherwise. This is
    /// the same fallback rule as [`crate::present`], lifted to the
    /// styled grid.
    #[must_use]
    pub fn from_view(view: &SessionView) -> Self {
        view.surface
            .as_ref()
            .map_or_else(|| Self::from_text(&view.data), Self::from_surface)
    }

    /// The grid's text projection — must equal the surface's own
    /// `text_projection()` for a `Screen` surface (parity oracle).
    #[must_use]
    pub fn text_projection(&self) -> String {
        self.rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Consecutive cells sharing one style, as (text, fg, bg, bold)
    /// runs — the spans a presenter spawns for one row.
    #[must_use]
    pub fn row_runs(row: &[RenderCell]) -> Vec<(String, Color, Color, bool)> {
        let mut runs: Vec<(String, Color, Color, bool)> = Vec::new();
        for cell in row {
            match runs.last_mut() {
                Some((text, fg, bg, bold))
                    if *fg == cell.fg && *bg == cell.bg && *bold == cell.bold =>
                {
                    text.push_str(&cell.symbol);
                }
                _ => runs.push((cell.symbol.clone(), cell.fg, cell.bg, cell.bold)),
            }
        }
        runs
    }
}

/// Map an xterm-256 palette index to its sRGB color.
///
/// Indices 0–15 are the standard/bright system colors, 16–231 the
/// 6×6×6 color cube, and 232–255 the grayscale ramp.
#[must_use]
pub fn palette_color(index: u8) -> Color {
    const SYSTEM: [[u8; 3]; 16] = [
        [0, 0, 0],
        [128, 0, 0],
        [0, 128, 0],
        [128, 128, 0],
        [0, 0, 128],
        [128, 0, 128],
        [0, 128, 128],
        [192, 192, 192],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [0, 0, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    let [r, g, b] = match index {
        0..=15 => SYSTEM[usize::from(index)],
        16..=231 => {
            let n = index - 16;
            let levels = [0_u8, 95, 135, 175, 215, 255];
            [
                levels[usize::from(n / 36)],
                levels[usize::from((n % 36) / 6)],
                levels[usize::from(n % 6)],
            ]
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            [v, v, v]
        }
    };
    Color::srgb(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    )
}

/// The view the renderer currently presents. The session loop replaces
/// this as host `view` frames arrive; `None` renders nothing.
#[derive(Resource, Clone, Debug, Default)]
pub struct PresentedView(pub Option<SessionView>);

/// The trusted form currently rendered (#8938), or `None`.
///
/// The session loop feeds every new view through
/// [`FormPresenter::present`] and every `form_result_status` reply
/// through [`FormPresenter::settle`], so the native shell rotates the
/// presentation token exactly as the SSH presenter does.
#[derive(Resource, Clone, Debug, Default)]
pub struct PresentedForm(pub FormPresenter);

impl PresentedForm {
    /// Fold one presented surface: a trusted form renders widgets, any
    /// other view clears them.
    pub fn apply_view(&mut self, view: &SessionView) {
        self.0.present(view.surface.as_ref());
    }

    /// Fold one `form_result_status` reply; returns the widgets to draw
    /// when the form was re-presented with a rotated token.
    pub fn apply_status(&mut self, status: &serde_json::Value) -> Option<FormWidgets> {
        match self.0.settle(status) {
            Some(FormStatusOutcome::RePresented(_)) => self.0.widgets(),
            _ => None,
        }
    }
}

/// Flat, render-ready lines for a trusted form: title, one line per
/// field, one line per action. Widget rows come from this client's own
/// manifest (see [`crate::form`]); nothing here is guest markup.
#[must_use]
pub fn form_widget_lines(widgets: &FormWidgets) -> Vec<String> {
    let mut lines = vec![widgets.title.clone()];
    for row in &widgets.rows {
        lines.push(format!("{} [{}]", row.caption, row.hint));
    }
    for action in &widgets.actions {
        lines.push(format!(
            "[{}]{}",
            action.id,
            if action.cancel { " (cancel)" } else { "" }
        ));
    }
    lines
}

/// Marker for the root UI node the renderer owns.
#[derive(Component)]
struct ScreenRoot;

/// Marker for one rendered row node (its grid row index).
#[derive(Component)]
struct ScreenRow;

/// Bevy plugin: presents [`PresentedView`] as a UI node tree.
///
/// Register after Bevy's UI/text plugins (the binary uses
/// `DefaultPlugins`). The rebuild system itself only spawns plain
/// `Node`/`Text`/`TextSpan` components, so tests can run it under
/// `MinimalPlugins` and inspect the spawned tree headlessly.
#[derive(Debug)]
pub struct ShellRenderPlugin;

impl Plugin for ShellRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PresentedView>()
            .init_resource::<PresentedForm>()
            .add_systems(Startup, spawn_root)
            .add_systems(Update, rebuild_rows);
    }
}

fn spawn_root(mut commands: Commands<'_, '_>) {
    commands.spawn((
        ScreenRoot,
        bevy::prelude::Node {
            flex_direction: bevy::prelude::FlexDirection::Column,
            width: bevy::prelude::Val::Percent(100.0),
            height: bevy::prelude::Val::Percent(100.0),
            ..Default::default()
        },
    ));
}

/// Rebuild the row nodes when the presented view's sequence changes.
#[allow(clippy::needless_pass_by_value)]
fn rebuild_rows(
    presented: Res<'_, PresentedView>,
    form: Res<'_, PresentedForm>,
    mut last_sequence: bevy::prelude::Local<'_, Option<u64>>,
    mut last_form: bevy::prelude::Local<'_, Option<String>>,
    roots: Query<'_, '_, Entity, With<ScreenRoot>>,
    rows: Query<'_, '_, Entity, With<ScreenRow>>,
    mut commands: Commands<'_, '_>,
) {
    let sequence = presented.0.as_ref().map(|view| view.sequence);
    let form_id = form.0.active().map(|active| active.form_id.clone());
    if sequence == *last_sequence && form_id == *last_form {
        return;
    }
    *last_sequence = sequence;
    *last_form = form_id;

    for entity in &rows {
        commands.entity(entity).despawn();
    }
    let (Ok(root), Some(view)) = (roots.single(), presented.0.as_ref()) else {
        return;
    };
    // #8938: a trusted form renders its widgets; anything else renders
    // the view as before (a form surface without trust falls through to
    // its escape-free text projection inside ScreenGrid).
    let grid = match (form.0.active(), form.0.widgets()) {
        (Some(_), Some(widgets)) => ScreenGrid::from_text(&form_widget_lines(&widgets).join("\n")),
        _ => ScreenGrid::from_view(view),
    };
    for row in &grid.rows {
        let mut row_entity =
            commands.spawn((ScreenRow, bevy::prelude::Node::default(), Text::new("")));
        row_entity.with_children(|parent| {
            for (text, fg, _bg, _bold) in ScreenGrid::row_runs(row) {
                parent.spawn((TextSpan::new(text), TextColor(fg)));
            }
        });
        let row_id = row_entity.id();
        commands.entity(root).add_child(row_id);
    }
}

/// Build a headless app with only the render plugin (tests, tools).
#[must_use]
pub fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::prelude::MinimalPlugins)
        .add_plugins(ShellRenderPlugin);
    app
}

/// Present `view` in `world` and run one update (headless helper).
pub fn present_in_world(world: &mut World, view: SessionView) {
    world.resource_mut::<PresentedView>().0 = Some(view);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use id::world_presentation::parse_surface;

    fn screen_view() -> SessionView {
        let mut state = crate::session::SessionState::default();
        state
            .apply_server_frame(serde_json::json!({
                "type": "view", "sequence": 7, "data": "fallback",
                "surface": {"kind": "screen", "version": 1, "width": 2, "height": 1, "cells": [
                    {"symbol": "h", "fg": 9, "bg": 0, "bold": true, "columns": 1},
                    {"symbol": "i", "fg": 9, "bg": 0, "bold": true, "columns": 1}
                ]},
                "view_revision": 0
            }))
            .unwrap();
        state.view().unwrap().clone()
    }

    #[test]
    fn palette_matches_xterm_256_layout() {
        assert_eq!(palette_color(0), Color::srgb(0.0, 0.0, 0.0));
        assert_eq!(palette_color(15), Color::srgb(1.0, 1.0, 1.0));
        // Cube: index 196 = bright red corner (255, 0, 0).
        assert_eq!(palette_color(196), Color::srgb(1.0, 0.0, 0.0));
        // Grayscale: index 232 starts at 8, index 255 is 238.
        assert_eq!(
            palette_color(255),
            Color::srgb(238.0 / 255.0, 238.0 / 255.0, 238.0 / 255.0)
        );
    }

    #[test]
    fn screen_grid_preserves_styles_and_matches_text_projection() {
        let view = screen_view();
        let grid = ScreenGrid::from_view(&view);
        assert_eq!(grid.width, 2);
        assert_eq!(grid.height, 1);
        assert_eq!(grid.text_projection(), "hi");
        let runs = ScreenGrid::row_runs(&grid.rows[0]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, "hi");
        assert!(runs[0].3, "bold flag must survive into the run");
    }

    #[test]
    fn text_surface_and_fallback_render_as_unstyled_lines() {
        let surface = parse_surface(r#"{"kind":"text","version":1,"lines":["one","two"]}"#)
            .unwrap()
            .expect("text surface");
        let grid = ScreenGrid::from_surface(&surface);
        assert_eq!(grid.text_projection(), "one\ntwo");

        let fallback = ScreenGrid::from_text("legacy\ntext");
        assert_eq!(fallback.text_projection(), "legacy\ntext");
        assert_eq!(fallback.height, 2);
    }

    #[test]
    fn renderer_spawns_one_row_with_the_view_text() {
        let mut app = headless_app();
        app.update(); // startup: root
        present_in_world(app.world_mut(), screen_view());
        app.update(); // rebuild rows

        let mut query = app.world_mut().query::<(&TextSpan, &TextColor)>();
        let spans: Vec<String> = query
            .iter(app.world())
            .map(|(span, _)| span.0.clone())
            .collect();
        assert_eq!(spans, vec!["hi".to_string()]);
    }

    #[test]
    fn renderer_clears_rows_when_the_view_is_removed() {
        let mut app = headless_app();
        app.update();
        present_in_world(app.world_mut(), screen_view());
        app.update();
        app.world_mut().resource_mut::<PresentedView>().0 = None;
        app.update();
        let mut query = app.world_mut().query::<&ScreenRow>();
        assert_eq!(query.iter(app.world()).count(), 0);
    }

    #[test]
    fn a_trusted_form_renders_widget_lines_not_its_text_projection() {
        let mut app = headless_app();
        app.update();
        let mut state = crate::session::SessionState::default();
        state
            .apply_server_frame(serde_json::json!({
                "type": "view", "sequence": 9, "data": "fallback",
                "surface": {"kind": "form", "version": 1, "form_id": "signup",
                    "presentation_token": "0123456789abcdef0123456789abcdef",
                    "title": "Sign up",
                    "fields": [{"kind": "text", "id": "name", "label": "Name",
                                "required": true, "max_length": 40}],
                    "actions": [{"id": "submit", "label": "Send"},
                                {"id": "cancel", "label": "Cancel", "kind": "cancel"}]},
                "view_revision": 0
            }))
            .unwrap();
        let view = state.view().unwrap().clone();

        let mut form = PresentedForm::default();
        form.apply_view(&view);
        assert!(form.0.active().is_some(), "form must be trusted");
        app.world_mut().insert_resource(form);
        present_in_world(app.world_mut(), view);
        app.update();

        let mut query = app.world_mut().query::<&TextSpan>();
        let spans: Vec<String> = query.iter(app.world()).map(|span| span.0.clone()).collect();
        assert_eq!(
            spans,
            vec![
                "Sign up".to_string(),
                "Name * [text, max 40]".to_string(),
                "[submit]".to_string(),
                "[cancel] (cancel)".to_string(),
            ]
        );
    }

    #[test]
    fn an_untrusted_form_falls_back_to_its_text_projection() {
        let mut app = headless_app();
        app.update();
        let mut state = crate::session::SessionState::default();
        state
            .apply_server_frame(serde_json::json!({
                "type": "view", "sequence": 11, "data": "fallback",
                "surface": {"kind": "form", "version": 1, "form_id": "pick",
                    "presentation_token": "0123456789abcdef0123456789abcdef",
                    "title": "Pick",
                    "fields": [{"kind": "text", "id": "name", "label": "Name",
                                "required": true, "max_length": 40}],
                    "actions": [{"id": "submit", "label": "Send"}]},
                "view_revision": 0
            }))
            .unwrap();
        let view = state.view().unwrap().clone();
        // A presenter whose manifest has no text entry cannot collect the
        // text field, so the form is not trusted and renders as text.
        let mut form = PresentedForm(crate::form::FormPresenter::with_manifest(
            serde_json::json!({
                "keys": 0, "encoder": false, "pointer": false, "text_entry": "none",
            }),
        ));
        form.apply_view(&view);
        assert!(form.0.active().is_none());
        app.world_mut().insert_resource(form);
        present_in_world(app.world_mut(), view);
        app.update();
        let mut query = app.world_mut().query::<&TextSpan>();
        let spans: Vec<String> = query.iter(app.world()).map(|span| span.0.clone()).collect();
        assert_eq!(
            spans,
            vec![
                "Pick".to_string(),
                "1. Name (text, max 40)".to_string(),
                "1. Send".to_string(),
            ]
        );
    }
}
