//! Host load/play fixture for the Burrow guest (#13603).
//!
//! Loads the checked-in `examples/roc-burrow/burrow.wasm` through the regular
//! ID host (`id::sandbox`) and plays the five-step `burrow/detour-to-exit`
//! vector, asserting each literal domain projection through the host rather
//! than through the guest's own unit-test surface.
//!
//! Oracle: the expected states are the literal projections in
//! `book/research/evidence/app-vectors.json` (scenario
//! `burrow/detour-to-exit`, tabulated in #13603's acceptance table); the
//! snapshot/restore checks compare a restored instance against the original
//! instance playing the same next operation. The oracle never derives
//! expected values from the guest.
//!
//! Scope distinctions the assertions keep explicit:
//! - **temporary UI state**: the view's grid glyphs and status lines are a
//!   presentation of the domain state; they never appear in `records`;
//! - **guest-domain records**: `records` is exactly `digger`, `soil`,
//!   `cleared_count`, `phase` and the last `outcome`; the immutable level
//!   configuration (rock, exit, quota) is a guest constant, not state;
//! - **host permissions**: the participant ID is the host `update` argument;
//!   host (participant 0) events never drive the guest;
//! - **host deduplication**: the inputs carry their vector `event` tags
//!   verbatim and the guest does not read them; journal sequencing and
//!   deduplication are a host concern above this fixture.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::sandbox::{Sandbox, SandboxLimits, WorldInstance};
use id::world_presentation::{WorldSurface, parse_surface};
use serde_json::{Value, json};

/// The guest module exactly as the example ships it.
const BURROW_WASM: &[u8] = include_bytes!("../examples/roc-burrow/burrow.wasm");

/// The host-assigned participant who plays the vector's actor "A".
const DIGGER: u64 = 1;
/// The host itself: system events, never a player.
const HOST: u64 = 0;

/// The vector's initial projection, plus the guest's empty last outcome.
fn initial() -> Value {
    json!({
        "digger": [0, 1],
        "soil": [[1, 1], [2, 1], [2, 2]],
        "cleared_count": 0,
        "phase": "digging",
        "outcome": "",
    })
}

/// One open instance of the Burrow world, plus the host-side reads the
/// fixture needs. Every call here goes through the regular `WorldInstance`
/// API.
struct Host {
    world: WorldInstance,
}

impl Host {
    /// Open a fresh instance from the checked-in guest module.
    fn open(seed: u64) -> Self {
        let sandbox =
            Sandbox::compile(BURROW_WASM, SandboxLimits::default()).expect("burrow.wasm compiles");
        Self {
            world: sandbox.instantiate(seed).expect("burrow guest instantiates"),
        }
    }

    /// Re-open a snapshot in a new instance: the host's restore path.
    fn restore(seed: u64, snapshot: &[u8]) -> Self {
        let mut host = Self::open(seed);
        host.world.restore(snapshot).expect("snapshot restores");
        host
    }

    /// Deliver one input event as `participant`.
    fn play(&mut self, participant: u64, event: &str) {
        self.world
            .update(participant, event.as_bytes())
            .expect("the guest accepts the event");
    }

    /// The guest-domain projection the host mirrors to readers.
    fn records(&mut self) -> Value {
        let raw = self
            .world
            .records()
            .expect("records runs")
            .expect("the burrow guest exports records");
        serde_json::from_slice(&raw).expect("records are JSON")
    }

    /// The portable rendered view, as row strings of the screen surface.
    fn view_rows(&mut self) -> Vec<String> {
        let raw = self.world.view().expect("view runs");
        let text = String::from_utf8(raw).expect("view is UTF-8");
        let surface = parse_surface(&text)
            .expect("view parses")
            .expect("the burrow view is a structured surface");
        let WorldSurface::Screen(screen) = surface else {
            panic!("the burrow view must be a screen surface: {text}");
        };
        screen
            .cells
            .chunks(usize::from(screen.width))
            .map(|row| row.iter().map(|cell| cell.symbol.as_str()).collect())
            .collect()
    }

    /// The guest's whole-state snapshot.
    fn snapshot(&mut self) -> Vec<u8> {
        self.world
            .snapshot()
            .expect("snapshot runs")
            .expect("the burrow guest exports snapshot")
    }
}

/// The five vector steps: (input as supplied, expected outcome, the literal
/// after-state's domain fields).
const STEPS: [(&str, &str, Value); 5] = [
    (
        r#"{"kind":"Dig","direction":"E","event":"d1"}"#,
        "cleared",
        json!({"digger": [1, 1], "soil": [[2, 1], [2, 2]], "cleared_count": 1, "phase": "digging"}),
    ),
    (
        r#"{"kind":"Dig","direction":"E","event":"d2"}"#,
        "cleared",
        json!({"digger": [2, 1], "soil": [[2, 2]], "cleared_count": 2, "phase": "digging"}),
    ),
    (
        r#"{"kind":"Dig","direction":"E","event":"d3"}"#,
        "blocked-rock",
        json!({"digger": [2, 1], "soil": [[2, 2]], "cleared_count": 2, "phase": "digging"}),
    ),
    (
        r#"{"kind":"Dig","direction":"S","event":"d4"}"#,
        "cleared",
        json!({"digger": [2, 2], "soil": [], "cleared_count": 3, "phase": "digging"}),
    ),
    (
        r#"{"kind":"Walk","direction":"E","event":"w1"}"#,
        "escaped",
        json!({"digger": [3, 2], "soil": [], "cleared_count": 3, "phase": "escaped"}),
    ),
];

/// The full records document a step must publish: the literal after-state
/// plus the step's outcome.
fn expected(after: &Value, outcome: &str) -> Value {
    let mut doc = after.clone();
    doc["outcome"] = json!(outcome);
    doc
}

/// The full five-step vector from #13603's acceptance table, through the
/// host, with the semantic text grid checked at the start and the escape.
#[test]
fn host_loads_and_plays_the_five_step_literal_vector() {
    let mut host = Host::open(7);
    assert_eq!(host.records(), initial());

    // The semantic text grid: digger west, two soil cells and the rock on
    // the middle row; the third soil and the exit on the bottom row.
    let rows = host.view_rows();
    assert!(
        rows.iter().any(|row| row.contains("A s s #")),
        "initial grid rows: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains(". . s X")),
        "initial grid rows: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("soil left=3 cleared=0/3")),
        "initial status rows: {rows:?}"
    );

    for (input, outcome, after) in &STEPS {
        host.play(DIGGER, input);
        assert_eq!(host.records(), expected(after, outcome), "{input}");
    }

    // Escaped: the digger stands on the exit cell and no soil remains.
    let rows = host.view_rows();
    assert!(
        rows.iter().any(|row| row.contains(". . . A")),
        "escaped grid rows: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("soil left=0 cleared=3/3")),
        "escaped status rows: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("phase=escaped last=escaped")),
        "escaped status rows: {rows:?}"
    );

    // The loop is over: a further dig is refused and the state is final.
    host.play(DIGGER, r#"{"kind":"Dig","direction":"W","event":"d5"}"#);
    let (.., final_after) = &STEPS[4];
    assert_eq!(host.records(), expected(final_after, "refused"));
}

/// A snapshot restores into a new instance; the restored instance reports
/// the same records and view, and the next accepted operations produce the
/// same projections on both instances.
#[test]
fn snapshot_restore_reproduces_records_view_and_the_next_operation() {
    let mut original = Host::open(7);
    for (input, ..) in &STEPS[..2] {
        original.play(DIGGER, input);
    }
    let (.., after_two) = &STEPS[1];
    assert_eq!(original.records(), expected(after_two, "cleared"));

    let snapshot = original.snapshot();
    let mut restored = Host::restore(7, &snapshot);
    assert_eq!(restored.records(), original.records());
    assert_eq!(restored.view_rows(), original.view_rows());
    assert_eq!(restored.snapshot(), snapshot);

    // The remaining steps, applied independently to both instances: the
    // blocked dig, the third clear and the escape all agree.
    for (input, outcome, after) in &STEPS[2..] {
        original.play(DIGGER, input);
        restored.play(DIGGER, input);
        assert_eq!(original.records(), expected(after, outcome), "{input}");
        assert_eq!(restored.records(), original.records(), "{input}");
    }
    assert_eq!(restored.view_rows(), original.view_rows());
}

/// Host events never drive the guest, and the view's presentation text is
/// not domain state: records carry exactly the five domain keys.
#[test]
fn host_events_and_view_state_stay_outside_the_domain_records() {
    let mut host = Host::open(7);

    // A host (participant 0) delivery is not a player command.
    host.play(HOST, r#"{"kind":"Dig","direction":"E","event":"d1"}"#);
    assert_eq!(host.records(), initial());

    // Records are the domain projection only: no grid glyphs, no status
    // lines, no level constants.
    let records = host.records();
    let mut keys: Vec<&str> = records
        .as_object()
        .expect("records are an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["cleared_count", "digger", "outcome", "phase", "soil"]
    );
    let text = records.to_string();
    assert!(!text.contains("soil left"), "{text}");
    assert!(!text.contains("rock"), "{text}");
}

/// Refusals the vector does not exercise still change nothing: walking
/// into uncleared soil and digging off the board edge.
#[test]
fn walk_into_soil_and_dig_off_the_edge_change_nothing() {
    let mut host = Host::open(7);

    host.play(DIGGER, r#"{"kind":"Walk","direction":"E","event":"w0"}"#);
    assert_eq!(host.records(), expected(&initial(), "blocked-soil"));

    host.play(DIGGER, r#"{"kind":"Dig","direction":"W","event":"d0"}"#);
    assert_eq!(host.records(), expected(&initial(), "blocked-edge"));
}
