//! Bevy 3D rendering for validated scene surfaces (#12657 step 2a).
//!
//! Two layers, deliberately separated, mirroring [`crate::render`]:
//!
//! - A Bevy-independent render model ([`SceneModel`]): a validated
//!   [`SceneSurface`] becomes a list of node models — primitive shape,
//!   position and size carried through bit-faithfully (widened to
//!   `f32`), a color derived deterministically from the node's shape
//!   and id, and a camera framing computed from the scene's bounds.
//!   The contract carries no visual fields beyond shape, position and
//!   size ("a renderer derives everything visual from the shape,
//!   position and size"), so every visual choice lives here, in one
//!   pure place the tests can pin without a window or GPU.
//! - The Bevy layer ([`SceneRenderPlugin`]): whenever the presented
//!   view's sequence changes, one entity per scene node is rebuilt
//!   with the mesh, material and transform the model prescribes, and
//!   the scene camera is reframed. A view whose surface is not a scene
//!   (or no view at all) clears the scene entities; shells without
//!   this plugin keep presenting the scene's text projection through
//!   [`crate::render::ScreenGrid`], exactly as before.
//!
//! World input stays on the same contract frames: the plugin only
//! reads [`PresentedView`], which the session loop already feeds.

use bevy::asset::Assets;
use bevy::mesh::{Mesh, Mesh3d, MeshBuilder, Meshable};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::{
    App, Color, Commands, Component, Cuboid, DirectionalLight, Entity, Local, Plane3d, Plugin,
    Query, Res, ResMut, Sphere, Startup, Transform, Update, Vec2, Vec3, With,
};
use id::world_presentation::{SceneShape, SceneSurface, WorldSurface};

use crate::render::PresentedView;
use crate::session::SessionView;

/// One scene node, render-ready: the contract's shape, position and
/// size, plus the color this renderer derives for it.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneNodeModel {
    /// Stable node identifier from the contract document.
    pub id: String,
    /// Human-readable node name from the contract document.
    pub label: String,
    /// The node's geometry from the contract's closed shape set.
    pub shape: SceneShape,
    /// Position in world units.
    pub position: Vec3,
    /// Extents in world units (diameters for a sphere or point).
    pub size: Vec3,
    /// Derived base color (see [`node_color`]).
    pub color: Color,
}

/// A whole scene as a render model: node models in document order
/// plus the deterministic camera framing that shows all of them.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneModel {
    /// Optional caption naming the scene.
    pub title: Option<String>,
    /// Node models, in the document's order.
    pub nodes: Vec<SceneNodeModel>,
    /// Where the scene camera sits.
    pub camera_position: Vec3,
    /// What the scene camera looks at (the nodes' centroid).
    pub camera_target: Vec3,
}

/// Narrowing a contract `f64` geometry component to render `f32`.
#[allow(clippy::cast_possible_truncation)]
const fn geo(value: f64) -> f32 {
    value as f32
}

const fn geo3(values: [f64; 3]) -> Vec3 {
    Vec3::new(geo(values[0]), geo(values[1]), geo(values[2]))
}

/// FNV-1a over the node id's bytes: the stable hash the color
/// derivation varies lightness with. Deterministic across runs,
/// platforms and shells — the same document renders the same colors
/// everywhere.
fn id_hash(id: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// The color this renderer derives for one node.
///
/// A fixed hue family per shape (so a scene reads consistently —
/// boxes are blue-grey, spheres warm orange, planes green, points
/// yellow) with the lightness stepped deterministically by the node
/// id's hash, so same-shape nodes stay distinguishable.
#[must_use]
pub fn node_color(shape: SceneShape, id: &str) -> Color {
    let hue = match shape {
        SceneShape::Box => 210.0,
        SceneShape::Sphere => 25.0,
        SceneShape::Plane => 130.0,
        SceneShape::Point => 50.0,
    };
    let step = f32::from(u8::try_from(id_hash(id) % 5).unwrap_or(0));
    let lightness = step.mul_add(0.05, 0.45);
    Color::hsl(hue, 0.55, lightness)
}

impl SceneModel {
    /// Render model for a validated scene surface.
    #[must_use]
    pub fn from_surface(scene: &SceneSurface) -> Self {
        let nodes: Vec<SceneNodeModel> = scene
            .nodes
            .iter()
            .map(|node| SceneNodeModel {
                id: node.id.clone(),
                label: node.label.clone(),
                shape: node.shape,
                position: geo3(node.position),
                size: geo3(node.size),
                color: node_color(node.shape, &node.id),
            })
            .collect();
        // Frame the whole scene: the camera looks at the nodes'
        // centroid from a fixed direction, far enough back that the
        // bounding sphere of positions-plus-extents fits the view.
        #[allow(clippy::cast_precision_loss)]
        let count = nodes.len().max(1) as f32;
        let centroid = nodes
            .iter()
            .fold(Vec3::ZERO, |sum, node| sum + node.position)
            / count;
        let radius = nodes
            .iter()
            .map(|node| {
                node.size
                    .length()
                    .mul_add(0.5, node.position.distance(centroid))
            })
            .fold(1.0_f32, f32::max);
        let direction = Vec3::new(1.0, 0.55, 1.0).normalize();
        Self {
            title: scene.title.clone(),
            nodes,
            camera_position: centroid + direction * (radius * 2.5),
            camera_target: centroid,
        }
    }

    /// Render model for one session view: `Some` only when the view
    /// carries a scene surface. Every other surface keeps its
    /// existing presentation (text grid / form widgets).
    #[must_use]
    pub fn from_view(view: &SessionView) -> Option<Self> {
        match view.surface.as_ref() {
            Some(WorldSurface::Scene(scene)) => Some(Self::from_surface(scene)),
            _ => None,
        }
    }
}

/// The deterministic example world's scene.
///
/// A small harbor — water, a dock, a buoy and a beacon — exercising
/// every shape in the closed set. The example world emits exactly
/// this document, so tests and demos present the same scene the
/// contract's own tests project.
#[must_use]
pub fn example_scene() -> SceneSurface {
    use id::world_presentation::SceneNode;
    SceneSurface {
        version: 1,
        title: Some("Harbor".to_owned()),
        nodes: vec![
            SceneNode {
                id: "water".to_owned(),
                label: "Water".to_owned(),
                shape: SceneShape::Plane,
                position: [0.0, 0.0, 0.0],
                size: [24.0, 0.1, 24.0],
            },
            SceneNode {
                id: "dock".to_owned(),
                label: "Dock".to_owned(),
                shape: SceneShape::Box,
                position: [0.0, 0.5, -4.0],
                size: [8.0, 1.0, 3.0],
            },
            SceneNode {
                id: "buoy".to_owned(),
                label: "Buoy".to_owned(),
                shape: SceneShape::Sphere,
                position: [5.0, 0.5, 3.0],
                size: [1.0, 1.0, 1.0],
            },
            SceneNode {
                id: "beacon".to_owned(),
                label: "Beacon".to_owned(),
                shape: SceneShape::Point,
                position: [-6.0, 3.0, -6.0],
                size: [0.5, 0.5, 0.5],
            },
        ],
    }
}

/// Marker for the scene camera the plugin owns.
#[derive(Component, Debug)]
struct SceneCamera;

/// Marker for one rendered scene node, carrying its contract id.
#[derive(Component, Debug)]
pub struct SceneNodeEntity(pub String);

/// Bevy plugin: presents the scene surface of [`PresentedView`] as
/// 3D entities.
///
/// Register alongside [`crate::render::ShellRenderPlugin`] (which
/// owns the `PresentedView` resource). The rebuild system only
/// spawns mesh/material/transform components, so tests can run it
/// under `MinimalPlugins` with the two `Assets` resources inserted
/// and inspect the spawned entities headlessly.
#[derive(Debug)]
pub struct SceneRenderPlugin;

impl Plugin for SceneRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_stage)
            .add_systems(Update, rebuild_scene);
    }
}

/// Spawn the standing stage: the scene camera and a directional
/// light. Both persist across views; the rebuild system reframes
/// the camera and swaps the node entities per view.
fn spawn_stage(mut commands: Commands<'_, '_>) {
    commands.spawn((
        SceneCamera,
        bevy::prelude::Camera3d::default(),
        Transform::from_xyz(6.0, 4.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight::default(),
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// The mesh for one node model: unit primitives, with the node's
/// extents baked in for the box and the plane.
fn node_mesh(node: &SceneNodeModel) -> Mesh {
    match node.shape {
        SceneShape::Box => Cuboid::from_size(node.size).into(),
        SceneShape::Sphere | SceneShape::Point => Sphere::new(0.5).into(),
        SceneShape::Plane => Plane3d::new(Vec3::Y, Vec2::new(node.size.x, node.size.z) * 0.5)
            .mesh()
            .build(),
    }
}

/// The transform for one node model: its contract position, scaled
/// to its extents for the unit primitives (sphere, point) whose
/// mesh is size-agnostic.
const fn node_transform(node: &SceneNodeModel) -> Transform {
    let transform = Transform::from_translation(node.position);
    match node.shape {
        SceneShape::Sphere | SceneShape::Point => transform.with_scale(node.size),
        SceneShape::Box | SceneShape::Plane => transform,
    }
}

/// Rebuild the scene entities when the presented view's sequence
/// changes: despawn the old scene's nodes, reframe the camera, and
/// spawn the new scene's nodes. Non-scene views clear the scene.
#[allow(clippy::needless_pass_by_value)]
fn rebuild_scene(
    presented: Res<'_, PresentedView>,
    mut last_sequence: Local<'_, Option<u64>>,
    node_entities: Query<'_, '_, Entity, With<SceneNodeEntity>>,
    mut cameras: Query<'_, '_, &mut Transform, With<SceneCamera>>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<StandardMaterial>>,
    mut commands: Commands<'_, '_>,
) {
    let sequence = presented.0.as_ref().map(|view| view.sequence);
    if sequence == *last_sequence {
        return;
    }
    *last_sequence = sequence;

    for entity in &node_entities {
        commands.entity(entity).despawn();
    }
    let Some(model) = presented.0.as_ref().and_then(SceneModel::from_view) else {
        return;
    };
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(model.camera_position)
            .looking_at(model.camera_target, Vec3::Y);
    }
    for node in &model.nodes {
        commands.spawn((
            SceneNodeEntity(node.id.clone()),
            Mesh3d(meshes.add(node_mesh(node))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: node.color,
                ..Default::default()
            })),
            node_transform(node),
        ));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::render::{ScreenGrid, ShellRenderPlugin};
    use bevy::prelude::MinimalPlugins;

    /// A session view carrying the example scene, built through the
    /// same server-frame seam the session loop feeds.
    fn scene_view(sequence: u64) -> SessionView {
        let mut surface = serde_json::to_value(example_scene()).unwrap();
        surface["kind"] = serde_json::json!("scene");
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

    /// Headless app: the render plugin (owns `PresentedView`), the
    /// scene plugin, and the asset stores the rebuild system writes.
    fn scene_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ShellRenderPlugin)
            .add_plugins(SceneRenderPlugin);
        app.world_mut().insert_resource(Assets::<Mesh>::default());
        app.world_mut()
            .insert_resource(Assets::<StandardMaterial>::default());
        app
    }

    #[test]
    fn example_scene_is_valid_and_the_model_is_deterministic() {
        let scene = example_scene();
        scene.validate().unwrap();
        assert_eq!(scene, example_scene());
        assert_eq!(
            SceneModel::from_surface(&scene),
            SceneModel::from_surface(&example_scene())
        );
    }

    #[test]
    fn model_carries_nodes_in_document_order_with_contract_geometry() {
        let model = SceneModel::from_surface(&example_scene());
        let ids: Vec<&str> = model.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["water", "dock", "buoy", "beacon"]);
        let buoy = &model.nodes[2];
        assert_eq!(buoy.shape, SceneShape::Sphere);
        assert_eq!(buoy.position, Vec3::new(5.0, 0.5, 3.0));
        assert_eq!(buoy.size, Vec3::ONE);
        let dock = &model.nodes[1];
        assert_eq!(dock.position, Vec3::new(0.0, 0.5, -4.0));
        assert_eq!(dock.size, Vec3::new(8.0, 1.0, 3.0));
    }

    #[test]
    fn node_colors_are_shape_keyed_and_id_varied() {
        // Same shape, same id: identical color (determinism).
        assert_eq!(
            node_color(SceneShape::Box, "dock"),
            node_color(SceneShape::Box, "dock")
        );
        // Different shapes land in different hue families.
        assert_ne!(
            node_color(SceneShape::Box, "dock"),
            node_color(SceneShape::Sphere, "dock")
        );
        // The hue family is the shape's: two boxes share a hue even
        // when the id hash steps their lightness apart.
        let one = bevy::color::Hsla::from(node_color(SceneShape::Box, "alpha"));
        let two = bevy::color::Hsla::from(node_color(SceneShape::Box, "beta"));
        assert_eq!(one.hue, two.hue);
    }

    #[test]
    fn camera_frames_the_whole_example_scene() {
        let model = SceneModel::from_surface(&example_scene());
        // Centroid of the four example positions.
        assert_eq!(model.camera_target, Vec3::new(-0.25, 1.0, -1.75));
        // Every node (position plus its bounding radius) sits inside
        // the framed sphere, and the camera stands off it along the
        // fixed direction.
        let radius = model
            .nodes
            .iter()
            .map(|node| {
                node.size
                    .length()
                    .mul_add(0.5, node.position.distance(model.camera_target))
            })
            .fold(1.0_f32, f32::max);
        let distance = model.camera_position.distance(model.camera_target);
        let expected = radius * 2.5;
        assert!((distance - expected).abs() < 0.01);
        assert!(model.camera_position.y > model.camera_target.y);
    }

    #[test]
    fn from_view_only_accepts_scene_surfaces() {
        assert!(SceneModel::from_view(&scene_view(3)).is_some());
        assert!(SceneModel::from_view(&text_view(4)).is_none());
    }

    #[test]
    fn scene_view_spawns_one_entity_per_node() {
        let mut app = scene_app();
        app.update(); // startup: stage + roots
        crate::render::present_in_world(app.world_mut(), scene_view(5));
        app.update(); // rebuild scene

        let mut query = app.world_mut().query::<(&SceneNodeEntity, &Transform)>();
        let spawned: Vec<(String, Transform)> = query
            .iter(app.world())
            .map(|(tag, transform)| (tag.0.clone(), *transform))
            .collect();
        assert_eq!(spawned.len(), 4);
        let buoy = spawned.iter().find(|(id, _)| id == "buoy").unwrap();
        assert_eq!(buoy.1.translation, Vec3::new(5.0, 0.5, 3.0));
        assert_eq!(buoy.1.scale, Vec3::ONE);
        let dock = spawned.iter().find(|(id, _)| id == "dock").unwrap();
        assert_eq!(dock.1.translation, Vec3::new(0.0, 0.5, -4.0));
    }

    #[test]
    fn a_non_scene_view_clears_the_scene_entities() {
        let mut app = scene_app();
        app.update();
        crate::render::present_in_world(app.world_mut(), scene_view(5));
        app.update();
        crate::render::present_in_world(app.world_mut(), text_view(6));
        app.update();
        let mut query = app.world_mut().query::<&SceneNodeEntity>();
        assert_eq!(query.iter(app.world()).count(), 0);
        // The stage (camera) persists across views.
        let mut cameras = app.world_mut().query::<&SceneCamera>();
        assert_eq!(cameras.iter(app.world()).count(), 1);
    }

    #[test]
    fn shells_without_the_scene_plugin_keep_the_text_projection() {
        // Parity: the text grid still presents a scene surface as its
        // deterministic text projection — the fallback every shell
        // without a scene renderer shows.
        let surface = WorldSurface::Scene(example_scene());
        let grid = ScreenGrid::from_surface(&surface);
        assert_eq!(grid.text_projection(), example_scene().text_projection());
    }
}
