//! Acceptance oracle for slice 2 (#13703): the journal tail in
//! `JournalSet`, its lag budget, and the intent round-trip, run against
//! the in-memory hosted world (design §2).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use bevy_ecs::prelude::*;
use id_bevy_bridge::projection::ProjectionError;
use id_bevy_bridge::{
    AppliedPosition, BridgeSchema, ComponentRule, DataClass, GameplaySet, HostedWorldFixture,
    IdEntityMap, Intent, IntentGate, IntentLedger, IntentOutbox, IntentOutcome, JournalDelta,
    JournalHalt, JournalLag, JournalTail, LagBudget, PositionBatch, PredicateRule,
    PredictedPosition, Proposal, WorldName, WorldPosition, applied_position, apply_position,
    configure_bridge_schedule, presented_position,
};

fn schema() -> BridgeSchema {
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

fn upsert(id: &str, values: Vec<(&str, serde_json::Value)>) -> JournalDelta {
    JournalDelta::Upsert {
        position: 0,
        id: id.to_owned(),
        values: values
            .into_iter()
            .map(|(predicate, value)| (predicate.to_owned(), value))
            .collect(),
    }
}

/// A world wired to `host` with the bridge's schedule and `budget`.
fn bridge(host: &Arc<HostedWorldFixture>, budget: usize) -> (World, Schedule) {
    let mut world = World::new();
    world.insert_resource(JournalTail::new(
        Arc::clone(host),
        schema(),
        LagBudget::new(budget),
    ));
    world.insert_resource(IntentGate(Arc::clone(host)));
    world.insert_resource(IntentOutbox::default());
    let mut schedule = Schedule::default();
    configure_bridge_schedule(&mut schedule);
    (world, schedule)
}

fn entity(world: &World, id: &str) -> Entity {
    world.resource::<IdEntityMap>().entity(id).unwrap()
}

fn position(world: &World, id: &str) -> Option<WorldPosition> {
    world.get::<WorldPosition>(entity(world, id)).copied()
}

#[test]
fn positions_apply_in_order_and_a_regressing_journal_halts_the_tail() {
    let host = Arc::new(HostedWorldFixture::new());
    host.append(vec![upsert("a", vec![("name", serde_json::json!("A"))])]);
    host.append(vec![upsert("b", vec![("name", serde_json::json!("B"))])]);
    // A corrupt journal repeats position 2 with different content.
    host.append_raw(PositionBatch {
        position: 2,
        deltas: vec![JournalDelta::Retract {
            position: 2,
            id: "a".to_owned(),
        }],
    });
    let (mut world, mut schedule) = bridge(&host, 10);

    schedule.run(&mut world);

    assert_eq!(applied_position(&world), Some(2));
    assert_eq!(
        world.get_resource::<JournalHalt>(),
        Some(&JournalHalt {
            position: 2,
            error: ProjectionError::PositionOrder,
        })
    );
    // The repeated position was not applied: `a` is still there.
    assert!(world.resource::<IdEntityMap>().contains("a"));

    // A halted tail stays halted.
    host.append(vec![upsert("c", vec![("name", serde_json::json!("C"))])]);
    schedule.run(&mut world);
    assert_eq!(applied_position(&world), Some(2));
    assert!(!world.resource::<IdEntityMap>().contains("c"));

    // The same rule holds for direct application: nothing at or below
    // the applied position applies.
    assert_eq!(
        apply_position(&mut world, &schema(), 1, &[]),
        Err(ProjectionError::PositionOrder)
    );
}

#[test]
fn a_position_applies_wholly_or_not_at_all() {
    let host = Arc::new(HostedWorldFixture::new());
    // Position 1 has a valid delta and an invalid one.
    host.append(vec![
        upsert("left", vec![("name", serde_json::json!("Left"))]),
        upsert("right", vec![("position", serde_json::json!("not a pair"))]),
    ]);
    let (mut world, mut schedule) = bridge(&host, 10);

    schedule.run(&mut world);

    assert_eq!(applied_position(&world), None);
    assert!(world.resource::<IdEntityMap>().is_empty());
    let mut names = world.query::<&WorldName>();
    assert_eq!(names.iter(&world).count(), 0);
    assert_eq!(
        world.resource::<JournalHalt>().error,
        ProjectionError::InvalidValue("position".to_owned())
    );
    assert_eq!(world.resource::<JournalLag>().behind, 1);

    // A delta stamped with another position is refused the same way.
    let mut fresh = World::new();
    assert_eq!(
        apply_position(
            &mut fresh,
            &schema(),
            3,
            &[JournalDelta::Retract {
                position: 4,
                id: "x".to_owned(),
            }]
        ),
        Err(ProjectionError::PositionMismatch {
            expected: 3,
            found: 4,
        })
    );
    assert_eq!(applied_position(&fresh), None);
}

/// What a gameplay system saw on one tick.
#[derive(Resource, Default)]
struct Seen(Vec<(Option<u64>, usize)>);

#[allow(clippy::needless_pass_by_value)]
fn observe(
    applied: Res<'_, AppliedPosition>,
    names: Query<'_, '_, &WorldName>,
    mut seen: ResMut<'_, Seen>,
) {
    seen.0.push((applied.get(), names.iter().count()));
}

#[test]
fn gameplay_systems_only_see_whole_positions() {
    let host = Arc::new(HostedWorldFixture::new());
    // Every position spawns a pair, so a consistent prefix at position p
    // always holds 2p named entities.
    for pair in 0..4 {
        host.append(vec![
            upsert(
                &format!("pair-{pair}-a"),
                vec![("name", serde_json::json!("a"))],
            ),
            upsert(
                &format!("pair-{pair}-b"),
                vec![("name", serde_json::json!("b"))],
            ),
        ]);
    }
    let (mut world, mut schedule) = bridge(&host, 1);
    world.insert_resource(Seen::default());
    schedule.add_systems(observe.in_set(GameplaySet));

    for _ in 0..5 {
        schedule.run(&mut world);
    }

    let seen = &world.resource::<Seen>().0;
    assert_eq!(
        seen,
        &vec![
            (Some(1), 2),
            (Some(2), 4),
            (Some(3), 6),
            (Some(4), 8),
            (Some(4), 8),
        ]
    );
}

#[test]
fn the_lag_budget_caps_positions_per_tick_and_reports_lag() {
    let host = Arc::new(HostedWorldFixture::new());
    for n in 0..10 {
        host.append(vec![upsert(
            &format!("e{n}"),
            vec![("name", serde_json::json!("e"))],
        )]);
    }
    let (mut world, mut schedule) = bridge(&host, 3);

    let mut trace = Vec::new();
    for _ in 0..5 {
        schedule.run(&mut world);
        trace.push((applied_position(&world), *world.resource::<JournalLag>()));
    }

    let lag = |behind, applied_last_tick| JournalLag {
        behind,
        applied_last_tick,
    };
    assert_eq!(
        trace,
        vec![
            (Some(3), lag(7, 3)),
            (Some(6), lag(4, 3)),
            (Some(9), lag(1, 3)),
            (Some(10), lag(0, 1)),
            (Some(10), lag(0, 0)),
        ]
    );
    assert!(world.resource::<JournalLag>().caught_up());
    assert_eq!(LagBudget::new(0).per_tick(), 1);
}

/// A hosted world where `avatar` stands at (0, 0) and two later
/// positions touch other entities, and a bridge that has applied only
/// position 1 of 3.
fn lagging_world() -> (Arc<HostedWorldFixture>, World, Schedule) {
    let host = Arc::new(HostedWorldFixture::new());
    host.append(vec![upsert(
        "avatar",
        vec![
            ("name", serde_json::json!("Avatar")),
            ("position", serde_json::json!([0.0, 0.0])),
        ],
    )]);
    host.append(vec![upsert(
        "tree",
        vec![("name", serde_json::json!("Tree"))],
    )]);
    host.append(vec![upsert(
        "rock",
        vec![("name", serde_json::json!("Rock"))],
    )]);
    let (mut world, mut schedule) = bridge(&host, 1);
    schedule.run(&mut world);
    assert_eq!(applied_position(&world), Some(1));
    (host, world, schedule)
}

/// Propose moving the avatar to (5, 5) and predict it on screen, as a
/// gameplay system would.
fn propose_move(world: &mut World) -> u64 {
    let based_on = applied_position(world);
    let intent_id = world.resource_mut::<IntentOutbox>().propose(
        based_on,
        Proposal::Upsert {
            id: "avatar".to_owned(),
            values: vec![("position".to_owned(), serde_json::json!([5.0, 5.0]))],
        },
    );
    let avatar = entity(world, "avatar");
    world.entity_mut(avatar).insert(PredictedPosition {
        intent_id,
        at: WorldPosition { x: 5.0, y: 5.0 },
    });
    intent_id
}

#[test]
fn an_accepted_intent_becomes_world_state_only_at_its_confirming_position() {
    let (host, mut world, mut schedule) = lagging_world();
    let intent = propose_move(&mut world);
    let origin = WorldPosition { x: 0.0, y: 0.0 };
    let moved = WorldPosition { x: 5.0, y: 5.0 };

    // Tick 2: the gate journals the move at position 4; the tail applies
    // position 2 only. World state is unchanged; the prediction shows.
    schedule.run(&mut world);
    assert_eq!(host.head(), Some(4));
    assert_eq!(applied_position(&world), Some(2));
    assert_eq!(world.resource::<IntentLedger>().awaiting(intent), Some(4));
    assert_eq!(position(&world, "avatar"), Some(origin));
    let avatar = entity(&world, "avatar");
    assert_eq!(presented_position(&world, avatar), Some((moved, true)));

    // Tick 3: position 3. Still unchanged.
    schedule.run(&mut world);
    assert_eq!(applied_position(&world), Some(3));
    assert_eq!(position(&world, "avatar"), Some(origin));
    assert!(world.resource::<IntentLedger>().settled().is_empty());

    // Tick 4: the confirming position applies. The move is now world
    // state, the intent is confirmed, and the prediction is gone.
    schedule.run(&mut world);
    assert_eq!(applied_position(&world), Some(4));
    assert_eq!(position(&world, "avatar"), Some(moved));
    assert_eq!(
        world.resource::<IntentLedger>().settled(),
        &[IntentOutcome::Confirmed {
            intent_id: intent,
            position: 4,
        }]
    );
    assert_eq!(world.resource::<IntentLedger>().awaiting(intent), None);
    assert_eq!(presented_position(&world, avatar), Some((moved, false)));
}

#[test]
fn a_rejected_intent_changes_no_world_state_and_the_prediction_snaps_back() {
    let (host, mut world, mut schedule) = lagging_world();
    host.deny_predicate("position");
    let intent = propose_move(&mut world);
    let origin = WorldPosition { x: 0.0, y: 0.0 };
    let avatar = entity(&world, "avatar");
    assert!(presented_position(&world, avatar).is_some_and(|(_, predicted)| predicted));

    schedule.run(&mut world);

    assert_eq!(host.head(), Some(3), "nothing was journaled");
    assert_eq!(
        world.resource::<IntentLedger>().settled(),
        &[IntentOutcome::Rejected {
            intent_id: intent,
            reason: "denied predicate `position`".to_owned(),
        }]
    );
    assert_eq!(position(&world, "avatar"), Some(origin));
    assert_eq!(presented_position(&world, avatar), Some((origin, false)));

    // Catching up changes nothing either.
    schedule.run(&mut world);
    assert_eq!(applied_position(&world), Some(3));
    assert_eq!(position(&world, "avatar"), Some(origin));
}

#[test]
fn an_intent_based_on_a_stale_view_of_its_entity_is_rejected() {
    let (host, mut world, mut schedule) = lagging_world();
    // Someone else moves the avatar after the app's applied position.
    host.append(vec![upsert(
        "avatar",
        vec![("position", serde_json::json!([1.0, 1.0]))],
    )]);
    let intent = propose_move(&mut world);

    schedule.run(&mut world);

    let settled = world.resource::<IntentLedger>().settled();
    assert!(
        matches!(
            settled,
            [IntentOutcome::Rejected { intent_id, reason }]
                if *intent_id == intent && reason.starts_with("stale")
        ),
        "{settled:?}"
    );
    assert_eq!(host.head(), Some(4));
    let avatar = entity(&world, "avatar");
    assert!(world.get::<PredictedPosition>(avatar).is_none());
}

#[test]
fn intents_travel_as_shell_contract_input_frames() {
    let intent = Intent {
        intent_id: 7,
        based_on: Some(3),
        proposal: Proposal::Retract {
            id: "avatar".to_owned(),
        },
    };
    let frame = intent.to_input_frame();
    assert_eq!(frame["type"], "input");
    let data = frame["data_hex"].as_str().unwrap();
    assert!(data.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(Intent::from_input_frame(&frame), Ok(intent));

    assert!(Intent::from_input_frame(&serde_json::json!({"type": "paste", "text": "x"})).is_err());
    assert!(
        Intent::from_input_frame(&serde_json::json!({"type": "input", "data_hex": "abc"})).is_err()
    );
}
