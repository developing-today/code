//! Acceptance oracle for slice 1: schema-driven world projection must be
//! explicit, fail-closed, and retraction-driven (design §1, #13702).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bevy_ecs::prelude::*;
use id_bevy_bridge::{
    BridgeSchema, ComponentRule, DataClass, IdEntityMap, JournalDelta, LocalOnly, WorldName,
    WorldPosition, project_fixture,
};

fn fixture_schema() -> BridgeSchema {
    let mut schema = BridgeSchema::new();
    schema.register(
        "name",
        id_bevy_bridge::PredicateRule {
            class: DataClass::World,
            component: ComponentRule::Name,
        },
    );
    schema.register(
        "position",
        id_bevy_bridge::PredicateRule {
            class: DataClass::World,
            component: ComponentRule::Position2,
        },
    );
    schema.register(
        "client.selection",
        id_bevy_bridge::PredicateRule {
            class: DataClass::Presentation,
            component: ComponentRule::Name,
        },
    );
    schema.register(
        "cache.local",
        id_bevy_bridge::PredicateRule {
            class: DataClass::Local,
            component: ComponentRule::Name,
        },
    );
    schema
}

#[test]
fn fixture_projects_classified_world_state_and_fails_closed() {
    let mut world = World::new();
    let local = world.spawn(LocalOnly).id();
    let fixture = vec![
        JournalDelta::Upsert {
            position: 1,
            id: "world-1".to_string(),
            values: vec![
                ("name".to_string(), serde_json::json!("Ada")),
                (
                    "position".to_string(),
                    serde_json::json!([1.5000000000000002, 2.0]),
                ),
                (
                    "unclassified.predicate".to_string(),
                    serde_json::json!("secret"),
                ),
                (
                    "client.selection".to_string(),
                    serde_json::json!("selected"),
                ),
                ("cache.local".to_string(), serde_json::json!("cache")),
            ],
        },
        JournalDelta::Upsert {
            position: 2,
            id: "world-2".to_string(),
            values: vec![("name".to_string(), serde_json::json!("Babbage"))],
        },
    ];

    assert_eq!(
        project_fixture(&mut world, &fixture_schema(), &fixture),
        Ok(Some(2))
    );
    let (first, second) = {
        let map = world.resource::<IdEntityMap>();
        (map.entity("world-1"), map.entity("world-2"))
    };
    let Some(first) = first else {
        panic!("first fixture entity should be mapped");
    };
    let Some(second) = second else {
        panic!("second fixture entity should be mapped");
    };
    assert_eq!(
        world.get::<WorldName>(first).map(|name| name.0.as_str()),
        Some("Ada")
    );
    assert_eq!(
        world.get::<WorldPosition>(first),
        Some(&WorldPosition {
            x: 1.5000000000000002,
            y: 2.0
        })
    );
    assert!(world.get::<WorldName>(second).is_some());
    assert!(world.get::<LocalOnly>(local).is_some());
    let map = world.resource::<IdEntityMap>();
    assert_eq!(map.len(), 2);
    assert_eq!(map.position("world-1"), Some(1));
}

#[test]
fn retraction_despawns_and_removes_mapping_at_its_position() {
    let mut world = World::new();
    let create = [JournalDelta::Upsert {
        position: 4,
        id: "gone".to_string(),
        values: vec![("name".to_string(), serde_json::json!("Transient"))],
    }];
    assert_eq!(
        project_fixture(&mut world, &fixture_schema(), &create),
        Ok(Some(4))
    );
    let entity = world.resource::<IdEntityMap>().entity("gone").unwrap();
    let retract = [JournalDelta::Retract {
        position: 5,
        id: "gone".to_string(),
    }];
    assert_eq!(
        project_fixture(&mut world, &fixture_schema(), &retract),
        Ok(Some(5))
    );
    let map = world.resource::<IdEntityMap>();
    assert!(!map.contains("gone"));
    assert!(map.is_empty());
    assert!(world.get_entity(entity).is_err());
}

#[test]
fn later_position_must_not_be_applied_before_an_earlier_one() {
    let mut world = World::new();
    let fixture = [
        JournalDelta::Retract {
            position: 8,
            id: "gone".to_string(),
        },
        JournalDelta::Retract {
            position: 7,
            id: "gone".to_string(),
        },
    ];
    assert_eq!(
        project_fixture(&mut world, &fixture_schema(), &fixture),
        Err(id_bevy_bridge::projection::ProjectionError::PositionOrder)
    );
}
