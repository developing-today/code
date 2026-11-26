//! Host load/play fixture for the Sketch guest (#13656, slice 2 of #13608).
//!
//! Loads the checked-in `examples/rust-sketch/sketch.wasm` through the regular
//! ID host (`id::sandbox`) and plays the six-step `sketch/redo-as-new-stroke`
//! vector from the issue, asserting each literal domain projection through the
//! host rather than through the guest's own unit-test surface.
//!
//! Oracle: the expected states are the literal projections written in #13608's
//! acceptance table; the snapshot/restore checks compare a restored instance
//! against the original instance playing the same next operation. The oracle
//! never derives expected values from the guest.
//!
//! Scope distinctions the assertions keep explicit:
//! - **temporary UI state**: draft vertices live only in the instance and the
//!   snapshot drops them; they never reach `records`;
//! - **guest-domain records**: `records` is exactly `visible_strokes` +
//!   `tombstones`; snapshot-only fields (`folded`, `retained_strokes`) stay out
//!   of the portable projection;
//! - **host permissions**: the participant ID is the host `update` argument; a
//!   guest-supplied `author` string grants nothing;
//! - **host deduplication**: the guest's event-id fold is exercised here; the
//!   journal's own sequencing/dedup is a host concern above this fixture.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::sandbox::{Sandbox, SandboxLimits, WorldInstance};
use serde_json::{Value, json};

/// The guest module exactly as the example crate ships it.
const SKETCH_WASM: &[u8] = include_bytes!("../examples/rust-sketch/sketch.wasm");

/// The host-assigned participant that authors the vector's strokes.
const AUTHOR: u64 = 7;
/// A second host-assigned participant used for the permission assertions.
const OTHER: u64 = 9;

/// The empty projection, as written in the issue vector.
const EMPTY: &str = r#"{"visible_strokes":[],"tombstones":[]}"#;
/// Steps one to three: a committed stroke.
const COMMITTED_A1: &str = r#"{"visible_strokes":[{"id":"A.1","author":"A","points":[[1,1],[3,1],[3,2]],"copy_of":null}],"tombstones":[]}"#;
/// Steps four and five: the undo tombstone, then the fresh-ID copy.
const TOMBSTONED_A1: &str = r#"{"visible_strokes":[],"tombstones":["A.1"]}"#;
const COPIED_A2: &str = r#"{"visible_strokes":[{"id":"A.2","author":"A","points":[[1,1],[3,1],[3,2]],"copy_of":"A.1"}],"tombstones":["A.1"]}"#;

/// One open instance of the Sketch world, plus the host-side reads the fixture
/// needs. Every call here goes through the regular `WorldInstance` API.
struct Host {
    world: WorldInstance,
}

impl Host {
    /// Open a fresh instance from the checked-in guest module.
    fn open(seed: u64) -> Self {
        let sandbox =
            Sandbox::compile(SKETCH_WASM, SandboxLimits::default()).expect("sketch.wasm compiles");
        Self {
            world: sandbox
                .instantiate(seed)
                .expect("sketch guest instantiates"),
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
            .expect("the sketch guest exports records");
        serde_json::from_slice(&raw).expect("records are JSON")
    }

    /// The portable rendered view.
    fn view(&mut self) -> String {
        String::from_utf8(self.world.view().expect("view runs")).expect("view is UTF-8")
    }

    /// The guest's whole-state snapshot.
    fn snapshot(&mut self) -> Vec<u8> {
        self.world
            .snapshot()
            .expect("snapshot runs")
            .expect("the sketch guest exports snapshot")
    }
}

fn expected(literal: &str) -> Value {
    serde_json::from_str(literal).expect("the literal projections are JSON")
}

/// The full six-step vector from #13608's acceptance table, through the host.
#[test]
fn host_loads_and_plays_the_six_step_literal_vector() {
    let mut host = Host::open(0);
    assert_eq!(host.records(), expected(EMPTY));
    assert_eq!(host.view(), "Sketch: 0 committed stroke(s)");

    host.play(AUTHOR, r#"{"kind":"BeginDraft","at":[1,1]}"#);
    assert_eq!(host.records(), expected(EMPTY));

    host.play(
        AUTHOR,
        r#"{"kind":"AppendDraftVertices","points":[[3,1],[3,2]]}"#,
    );
    assert_eq!(host.records(), expected(EMPTY));

    host.play(
        AUTHOR,
        r#"{"kind":"ConfirmStroke","id":"A.1","event":"add1"}"#,
    );
    assert_eq!(host.records(), expected(COMMITTED_A1));
    assert_eq!(host.view(), "Sketch: 1 committed stroke(s)");

    host.play(
        AUTHOR,
        r#"{"kind":"UndoOwnStroke","id":"A.1","event":"u1"}"#,
    );
    assert_eq!(host.records(), expected(TOMBSTONED_A1));

    host.play(
        AUTHOR,
        r#"{"kind":"CopyOwnStroke","id":"A.2","copy_of":"A.1","event":"copy1"}"#,
    );
    assert_eq!(host.records(), expected(COPIED_A2));

    host.play(AUTHOR, r#"{"kind":"Redeliver","ref":"u1"}"#);
    assert_eq!(host.records(), expected(COPIED_A2));
}

/// A snapshot restores into a new instance; the restored instance reports the
/// same records and view, and the next accepted operation produces the same
/// projection on both instances.
#[test]
fn snapshot_restore_reproduces_records_view_and_the_next_operation() {
    let mut original = Host::open(0);
    original.play(AUTHOR, r#"{"kind":"BeginDraft","at":[1,1]}"#);
    original.play(
        AUTHOR,
        r#"{"kind":"AppendDraftVertices","points":[[3,1],[3,2]]}"#,
    );
    original.play(
        AUTHOR,
        r#"{"kind":"ConfirmStroke","id":"A.1","event":"add1"}"#,
    );
    assert_eq!(original.records(), expected(COMMITTED_A1));

    let snapshot = original.snapshot();
    let mut restored = Host::restore(0, &snapshot);
    assert_eq!(restored.records(), original.records());
    assert_eq!(restored.view(), original.view());
    assert_eq!(restored.snapshot(), snapshot);

    // Next accepted operation: the owner's undo, applied independently to both.
    let undo = r#"{"kind":"UndoOwnStroke","id":"A.1","event":"u1"}"#;
    original.play(AUTHOR, undo);
    restored.play(AUTHOR, undo);
    assert_eq!(original.records(), expected(TOMBSTONED_A1));
    assert_eq!(restored.records(), original.records());

    // Next accepted operation after the tombstone: the fresh-ID copy.
    let copy = r#"{"kind":"CopyOwnStroke","id":"A.2","copy_of":"A.1","event":"copy1"}"#;
    original.play(AUTHOR, copy);
    restored.play(AUTHOR, copy);
    assert_eq!(original.records(), expected(COPIED_A2));
    assert_eq!(restored.records(), original.records());
    assert_eq!(restored.view(), original.view());

    // Redelivery of the folded undo is a no-op on either instance.
    let redeliver = r#"{"kind":"Redeliver","ref":"u1"}"#;
    original.play(AUTHOR, redeliver);
    restored.play(AUTHOR, redeliver);
    assert_eq!(restored.records(), original.records());
}

/// Draft vertices are temporary UI state: invisible in records and view, absent
/// from the snapshot, and unrecoverable after restore.
#[test]
fn draft_vertices_are_temporary_ui_state_and_never_reach_records() {
    let mut host = Host::open(0);
    host.play(AUTHOR, r#"{"kind":"BeginDraft","at":[2,2]}"#);
    host.play(AUTHOR, r#"{"kind":"AppendDraftVertices","points":[[4,4]]}"#);
    assert_eq!(host.records(), expected(EMPTY));
    assert_eq!(host.view(), "Sketch: 0 committed stroke(s)");

    // Records carry only the portable domain projection; the snapshot also
    // carries the fold/retention bookkeeping needed for redelivery.
    let records = host.records();
    let mut keys: Vec<&str> = records
        .as_object()
        .expect("records are an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["tombstones", "visible_strokes"]);
    let snapshot: Value = serde_json::from_slice(&host.snapshot()).expect("snapshot is JSON");
    assert!(snapshot.get("folded").is_some());
    assert!(records.get("folded").is_none());

    // The draft is not part of the portable state: restoring drops it, so the
    // confirm that would have worked on the live instance does nothing.
    let snapshot = host.snapshot();
    let mut restored = Host::restore(0, &snapshot);
    restored.play(
        AUTHOR,
        r#"{"kind":"ConfirmStroke","id":"A.9","event":"add9"}"#,
    );
    assert_eq!(restored.records(), expected(EMPTY));
}

/// The participant ID comes from the host `update` argument; a guest string
/// cannot grant authority, and another participant's stroke is untouchable.
#[test]
fn host_participant_ids_gate_permissions_and_guest_strings_grant_nothing() {
    let mut host = Host::open(0);
    host.play(AUTHOR, r#"{"kind":"BeginDraft","at":[1,1]}"#);
    host.play(
        AUTHOR,
        r#"{"kind":"AppendDraftVertices","points":[[3,1],[3,2]]}"#,
    );
    host.play(
        AUTHOR,
        r#"{"kind":"ConfirmStroke","id":"A.1","event":"add1"}"#,
    );
    assert_eq!(host.records(), expected(COMMITTED_A1));

    // The other participant cannot undo the author's stroke...
    host.play(OTHER, r#"{"kind":"UndoOwnStroke","id":"A.1","event":"u9"}"#);
    assert_eq!(host.records(), expected(COMMITTED_A1));

    // ...nor copy it, even while claiming an author in the event body.
    host.play(
        OTHER,
        r#"{"kind":"CopyOwnStroke","id":"B.1","copy_of":"A.1","event":"copy9","author":"A"}"#,
    );
    assert_eq!(host.records(), expected(COMMITTED_A1));

    // A draft belongs to its opener: the other participant's vertices are
    // refused, so the author's confirm contains only the author's point.
    host.play(AUTHOR, r#"{"kind":"BeginDraft","at":[5,5]}"#);
    host.play(OTHER, r#"{"kind":"AppendDraftVertices","points":[[9,9]]}"#);
    host.play(
        AUTHOR,
        r#"{"kind":"ConfirmStroke","id":"A.2","event":"add2"}"#,
    );
    let records = host.records();
    let stroke = records["visible_strokes"]
        .as_array()
        .expect("visible_strokes is an array")
        .iter()
        .find(|stroke| stroke["id"] == json!("A.2"))
        .expect("the author's second stroke committed");
    assert_eq!(stroke["points"], json!([[5, 5]]));
}

/// Event-id folding accepts a redelivered event once and never rewrites the
/// guest's accepted history; a new event id does not duplicate a stroke ID.
#[test]
fn redelivery_is_folded_by_event_id_and_never_duplicates_records() {
    let mut host = Host::open(0);
    host.play(AUTHOR, r#"{"kind":"BeginDraft","at":[1,1]}"#);
    host.play(
        AUTHOR,
        r#"{"kind":"ConfirmStroke","id":"A.1","event":"add1"}"#,
    );
    let accepted = host.records();

    // Same event id, different stroke id: the fold wins, nothing is added.
    host.play(AUTHOR, r#"{"kind":"BeginDraft","at":[1,1]}"#);
    host.play(
        AUTHOR,
        r#"{"kind":"ConfirmStroke","id":"A.7","event":"add1"}"#,
    );
    assert_eq!(host.records(), accepted);

    // New event id, existing stroke id: the stroke already exists, so the
    // guest refuses the duplicate identity as well.
    host.play(AUTHOR, r#"{"kind":"BeginDraft","at":[1,1]}"#);
    host.play(
        AUTHOR,
        r#"{"kind":"ConfirmStroke","id":"A.1","event":"add7"}"#,
    );
    assert_eq!(host.records(), accepted);

    // The explicit redelivery event of the accepted confirm is a no-op.
    host.play(AUTHOR, r#"{"kind":"Redeliver","ref":"add1"}"#);
    assert_eq!(host.records(), accepted);
}
