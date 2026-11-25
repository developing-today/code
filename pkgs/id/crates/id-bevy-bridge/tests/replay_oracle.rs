//! Acceptance oracle for slice 3 (#13704): the replay oracle, its soak
//! harness and fault injection, and the `bevy_ecs` bump gate (design §4,
//! Q2).
//!
//! The recorded corpus lives in `tests/corpus/*.json`. Each file holds a
//! script of steps (other writers appending, the app proposing intents,
//! ticks), the journal the run recorded, the intent outcomes, and the
//! golden canonical state. `bevy_bump_gate_*` replays every recorded
//! journal from an empty world and requires the golden state; a bump of
//! the pinned `bevy_ecs` that changes a projection outcome fails it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreadable_literal
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use id_bevy_bridge::projection::ProjectionError;
use id_bevy_bridge::{
    BridgeSchema, CanonicalEntity, CanonicalPosition, CanonicalState, ComponentRule, DataClass,
    Divergence, HostedWorldFixture, IdEntityMap, IntentGate, IntentLedger, IntentOutbox,
    IntentOutcome, JournalDelta, JournalHalt, JournalLag, JournalTail, LagBudget, LocalOnly,
    PositionBatch, PredicateRule, PredictedPosition, Proposal, WorldName, WorldPosition,
    applied_position, apply_position, canonical_state, check_replay, configure_bridge_schedule,
    replay,
};
use proptest::prelude::*;
use serde::{Deserialize, Serialize};

/// The recorded corpus, compiled in so the gate needs no file access.
const CORPUS: [(&str, &str); 3] = [
    (
        "steady-two-writers",
        include_str!("corpus/steady-two-writers.json"),
    ),
    (
        "lagging-catch-up",
        include_str!("corpus/lagging-catch-up.json"),
    ),
    (
        "denied-and-snapped-back",
        include_str!("corpus/denied-and-snapped-back.json"),
    ),
];

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

const fn one() -> usize {
    1
}

/// One step of a recorded run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
enum Step {
    /// Another writer journals one position.
    Append { deltas: Vec<JournalDelta> },
    /// The gate starts refusing a predicate.
    Deny { predicate: String },
    /// The app proposes a change based on its applied position, and
    /// optionally predicts the position as presentation state.
    Propose {
        proposal: Proposal,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        predict: Option<[f64; 2]>,
    },
    /// The schedule runs this many times.
    Tick {
        #[serde(default = "one")]
        times: usize,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Outcomes {
    confirmed: Vec<(u64, u64)>,
    rejected: Vec<(u64, String)>,
}

/// The golden state as written in a corpus file: positions as numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct GoldenEntity {
    mapped_at: u64,
    name: Option<String>,
    position: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Golden {
    applied: Option<u64>,
    entities: BTreeMap<String, GoldenEntity>,
}

impl Golden {
    fn canonical(&self) -> CanonicalState {
        CanonicalState {
            applied: self.applied,
            entities: self
                .entities
                .iter()
                .map(|(id, golden)| {
                    let entity = CanonicalEntity {
                        mapped_at: golden.mapped_at,
                        name: golden.name.clone(),
                        position: golden
                            .position
                            .map(|[x, y]| CanonicalPosition::from(WorldPosition { x, y })),
                    };
                    (id.clone(), entity)
                })
                .collect(),
            unmapped_world_state: 0,
        }
    }

    fn from_canonical(state: &CanonicalState) -> Self {
        Self {
            applied: state.applied,
            entities: state
                .entities
                .iter()
                .map(|(id, entity)| {
                    let golden = GoldenEntity {
                        mapped_at: entity.mapped_at,
                        name: entity.name.clone(),
                        position: entity.position.map(|at| [at.x(), at.y()]),
                    };
                    (id.clone(), golden)
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Corpus {
    name: String,
    description: String,
    budget: usize,
    steps: Vec<Step>,
    journal: Vec<PositionBatch>,
    outcomes: Outcomes,
    expected: Golden,
}

fn load(file: &str, text: &str) -> Corpus {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{file}: {error}"))
}

/// A finished live run against an in-memory hosted world.
struct LiveRun {
    world: World,
    host: Arc<HostedWorldFixture>,
    outcomes: Outcomes,
}

/// Run `steps` live. After every tick the replay oracle must hold: the
/// host's journal replayed to the live applied position gives the live
/// canonical state.
fn run_live(label: &str, budget: usize, steps: &[Step]) -> LiveRun {
    let host = Arc::new(HostedWorldFixture::new());
    let mut world = World::new();
    world.insert_resource(JournalTail::new(
        Arc::clone(&host),
        schema(),
        LagBudget::new(budget),
    ));
    world.insert_resource(IntentGate(Arc::clone(&host)));
    world.insert_resource(IntentOutbox::default());
    let mut schedule = Schedule::default();
    configure_bridge_schedule(&mut schedule);
    let mut outcomes = Outcomes::default();
    let mut tick = 0;
    for step in steps {
        match step {
            Step::Append { deltas } => {
                host.append(deltas.clone());
            }
            Step::Deny { predicate } => host.deny_predicate(predicate.clone()),
            Step::Propose { proposal, predict } => {
                let based_on = applied_position(&world);
                let intent_id = world
                    .resource_mut::<IntentOutbox>()
                    .propose(based_on, proposal.clone());
                let (Proposal::Upsert { id, .. } | Proposal::Retract { id }) = proposal;
                let mapped = world
                    .get_resource::<IdEntityMap>()
                    .and_then(|map| map.entity(id));
                if let (Some([x, y]), Some(entity)) = (predict, mapped) {
                    world.entity_mut(entity).insert(PredictedPosition {
                        intent_id,
                        at: WorldPosition { x: *x, y: *y },
                    });
                }
            }
            Step::Tick { times } => {
                for _ in 0..*times {
                    schedule.run(&mut world);
                    tick += 1;
                    for outcome in world.resource::<IntentLedger>().settled() {
                        match outcome {
                            IntentOutcome::Confirmed {
                                intent_id,
                                position,
                            } => outcomes.confirmed.push((*intent_id, *position)),
                            IntentOutcome::Rejected { intent_id, reason } => {
                                outcomes.rejected.push((*intent_id, reason.clone()));
                            }
                        }
                    }
                    assert!(
                        world.get_resource::<JournalHalt>().is_none(),
                        "{label}: the tail halted at tick {tick}"
                    );
                    if let Err(divergence) = check_replay(&mut world, &schema(), &host.journal()) {
                        panic!("{label}: tick {tick} diverged from its replay: {divergence}");
                    }
                }
            }
        }
    }
    LiveRun {
        world,
        host,
        outcomes,
    }
}

fn entity(world: &World, id: &str) -> Entity {
    world.resource::<IdEntityMap>().entity(id).unwrap()
}

/// The bump gate: every recorded journal replays from an empty world to
/// its golden state, and every recorded prefix replays without error.
#[test]
fn bevy_bump_gate_recorded_corpus_replays_to_its_golden_state() {
    for (file, text) in CORPUS {
        let corpus = load(file, text);
        assert_eq!(corpus.name, file, "{file}: the name must match the file");
        assert!(!corpus.description.is_empty(), "{file}: describe the run");
        for batch in &corpus.journal {
            replay(&schema(), &corpus.journal, Some(batch.position)).unwrap_or_else(|error| {
                panic!(
                    "{file}: the prefix to {} does not replay: {error}",
                    batch.position
                )
            });
        }
        let replayed = replay(&schema(), &corpus.journal, Some(u64::MAX)).unwrap();
        assert_eq!(
            replayed,
            corpus.expected.canonical(),
            "{file}: the replay differs from the golden state; a bevy_ecs bump that changes \
             a projection outcome is a mapping bug or a named migration of this corpus"
        );
        assert_eq!(replayed.applied, corpus.journal.last().map(|b| b.position));
    }
}

/// The live runs record the same journal, outcomes and golden state, the
/// oracle holds after every tick, and every prediction has settled.
#[test]
fn recorded_corpus_live_runs_match_their_recording_at_every_tick() {
    let mut accepted = 0;
    let mut rejected = 0;
    for (file, text) in CORPUS {
        let corpus = load(file, text);
        let mut live = run_live(file, corpus.budget, &corpus.steps);
        let state = canonical_state(&mut live.world);
        let actual = Corpus {
            journal: live.host.journal(),
            outcomes: live.outcomes,
            expected: Golden::from_canonical(&state),
            ..corpus.clone()
        };
        let matches = actual.journal == corpus.journal
            && actual.outcomes == corpus.outcomes
            && state == corpus.expected.canonical();
        assert!(
            matches,
            "{file}: the live run differs from its recording; the run recorded:\n{}",
            serde_json::to_string_pretty(&actual).unwrap()
        );
        assert!(live.world.resource::<JournalLag>().caught_up(), "{file}");
        let mut predictions = live.world.query::<&PredictedPosition>();
        assert_eq!(predictions.iter(&live.world).count(), 0, "{file}");
        accepted += corpus.outcomes.confirmed.len();
        rejected += corpus.outcomes.rejected.len();
    }
    // The corpus covers both accepted and rejected intents.
    assert!(accepted > 0 && rejected > 0);
}

fn steady_run() -> LiveRun {
    let (file, text) = CORPUS[0];
    let corpus = load(file, text);
    run_live(file, corpus.budget, &corpus.steps)
}

#[test]
fn fault_a_component_altered_at_a_known_position_is_caught() {
    let mut live = steady_run();
    // babbage's position was last journaled at position 6.
    let babbage = entity(&live.world, "babbage");
    live.world.get_mut::<WorldPosition>(babbage).unwrap().x += 0.5;

    let divergence = check_replay(&mut live.world, &schema(), &live.host.journal()).unwrap_err();
    let Divergence::Entity {
        id,
        live: altered,
        replay: replayed,
    } = divergence
    else {
        panic!("expected an entity divergence");
    };
    assert_eq!(id, "babbage");
    let at = |entity: Option<CanonicalEntity>| entity.and_then(|e| e.position).unwrap();
    assert_eq!(
        at(altered),
        CanonicalPosition::from(WorldPosition { x: -2.5, y: 4.5 })
    );
    assert_eq!(
        at(replayed),
        CanonicalPosition::from(WorldPosition { x: -3.0, y: 4.5 })
    );
}

#[test]
fn fault_the_comparison_is_exact_to_the_bit() {
    let (file, text) = CORPUS[1];
    let corpus = load(file, text);
    let mut live = run_live(file, corpus.budget, &corpus.steps);
    // p1 sits at (2.0, 0.0); -0.0 compares equal as a float but not here.
    let p1 = entity(&live.world, "p1");
    live.world.get_mut::<WorldPosition>(p1).unwrap().y = -0.0;

    assert!(matches!(
        check_replay(&mut live.world, &schema(), &live.host.journal()),
        Err(Divergence::Entity { id, .. }) if id == "p1"
    ));
}

#[test]
fn fault_a_dropped_component_is_caught() {
    let mut live = steady_run();
    let lovelace = entity(&live.world, "lovelace");
    live.world.entity_mut(lovelace).remove::<WorldName>();

    assert!(matches!(
        check_replay(&mut live.world, &schema(), &live.host.journal()),
        Err(Divergence::Entity { id, live: Some(CanonicalEntity { name: None, .. }), .. })
            if id == "lovelace"
    ));
}

#[test]
fn fault_local_state_feeding_back_into_world_state_is_caught() {
    let mut live = steady_run();
    // Presentation and local state are exempt from the comparison.
    live.world.spawn((
        LocalOnly,
        PredictedPosition {
            intent_id: 99,
            at: WorldPosition { x: 1.0, y: 1.0 },
        },
    ));
    check_replay(&mut live.world, &schema(), &live.host.journal()).unwrap();

    // A local entity carrying a world-state component is not.
    live.world
        .spawn((LocalOnly, WorldName("not in the journal".to_owned())));
    assert_eq!(
        check_replay(&mut live.world, &schema(), &live.host.journal()),
        Err(Divergence::Unmapped { live: 1, replay: 0 })
    );
}

#[test]
fn fault_a_position_applied_outside_the_journal_is_caught() {
    let mut live = steady_run();
    let head = applied_position(&live.world).unwrap();
    apply_position(
        &mut live.world,
        &schema(),
        head + 1,
        &[JournalDelta::Upsert {
            position: head + 1,
            id: "intruder".to_owned(),
            values: vec![("name".to_owned(), serde_json::json!("Intruder"))],
        }],
    )
    .unwrap();

    assert_eq!(
        check_replay(&mut live.world, &schema(), &live.host.journal()),
        Err(Divergence::Applied {
            live: Some(head + 1),
            replay: Some(head),
        })
    );
}

#[test]
fn fault_a_corrupted_recording_is_caught() {
    let mut live = steady_run();
    let mut journal = live.host.journal();
    // Rewrite the name journaled at position 5.
    journal[4].deltas[0] = JournalDelta::Upsert {
        position: 5,
        id: "lovelace".to_owned(),
        values: vec![("name".to_owned(), serde_json::json!("Not the countess"))],
    };
    assert!(matches!(
        check_replay(&mut live.world, &schema(), &journal),
        Err(Divergence::Entity { id, .. }) if id == "lovelace"
    ));

    // A recording that repeats a position does not replay at all.
    let mut journal = live.host.journal();
    journal.insert(2, journal[1].clone());
    assert_eq!(
        check_replay(&mut live.world, &schema(), &journal),
        Err(Divergence::Replay(ProjectionError::PositionOrder))
    );
}

/// One generated operation of a soak stream.
#[derive(Debug, Clone)]
enum Op {
    Write(usize, Option<String>, Option<(f64, f64)>),
    Retract(usize),
    Move(usize, f64, f64),
    Drop(usize),
    Tick(usize),
}

const IDS: [&str; 4] = ["a", "b", "c", "d"];

fn coordinate() -> impl Strategy<Value = f64> {
    (-400i32..400).prop_map(|v| f64::from(v) / 4.0)
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (
            0..IDS.len(),
            proptest::option::of("[a-z]{1,6}"),
            proptest::option::of((coordinate(), coordinate())),
        )
            .prop_map(|(id, name, at)| Op::Write(id, name, at)),
        1 => (0..IDS.len()).prop_map(Op::Retract),
        3 => (0..IDS.len(), coordinate(), coordinate()).prop_map(|(id, x, y)| Op::Move(id, x, y)),
        1 => (0..IDS.len()).prop_map(Op::Drop),
        3 => (1..4usize).prop_map(Op::Tick),
    ]
}

fn steps(ops: &[Op]) -> Vec<Step> {
    ops.iter()
        .map(|op| match op {
            Op::Write(id, name, at) => {
                let mut values = Vec::new();
                if let Some(name) = name {
                    values.push(("name".to_owned(), serde_json::json!(name)));
                }
                if let Some((x, y)) = at {
                    values.push(("position".to_owned(), serde_json::json!([x, y])));
                }
                Step::Append {
                    deltas: vec![JournalDelta::Upsert {
                        position: 0,
                        id: IDS[*id].to_owned(),
                        values,
                    }],
                }
            }
            Op::Retract(id) => Step::Append {
                deltas: vec![JournalDelta::Retract {
                    position: 0,
                    id: IDS[*id].to_owned(),
                }],
            },
            Op::Move(id, x, y) => Step::Propose {
                proposal: Proposal::Upsert {
                    id: IDS[*id].to_owned(),
                    values: vec![("position".to_owned(), serde_json::json!([x, y]))],
                },
                predict: Some([*x, *y]),
            },
            Op::Drop(id) => Step::Propose {
                proposal: Proposal::Retract {
                    id: IDS[*id].to_owned(),
                },
                predict: None,
            },
            Op::Tick(times) => Step::Tick { times: *times },
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// Soak: generated streams with other writers, lag and accepted and
    /// rejected intents hold the oracle at every tick, catch up to a
    /// state equal to the full replay, and run deterministically.
    #[test]
    fn soak_generated_streams_match_their_replay(
        budget in 1..4usize,
        ops in proptest::collection::vec(op(), 1..40),
    ) {
        let mut script = steps(&ops);
        // Catch up: enough ticks to apply every position at this budget.
        script.push(Step::Tick { times: ops.len() + 2 });
        let mut live = run_live("generated", budget, &script);
        prop_assert!(live.world.resource::<JournalLag>().caught_up());

        let journal = live.host.journal();
        let state = canonical_state(&mut live.world);
        prop_assert_eq!(&state, &replay(&schema(), &journal, Some(u64::MAX)).unwrap());

        let mut again = run_live("generated again", budget, &script);
        prop_assert_eq!(again.host.journal(), journal);
        prop_assert_eq!(canonical_state(&mut again.world), state);
        prop_assert_eq!(again.outcomes, live.outcomes);
    }
}
