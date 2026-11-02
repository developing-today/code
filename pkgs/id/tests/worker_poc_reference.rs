//! Native reference runner for the #2271 Worker-isolate PoC.
//!
//! Drives the checked-in example guests through the native wasmtime host
//! (`crate::sandbox` via the public `id::sandbox` API) over the exact
//! scripted sequences the isolate side runs in
//! `pkgs/id/worker-poc/compare/isolate-runner.mjs`, and emits one JSON line
//! per step: `{world, step, event, view, records, snapshot_hex}`. Step 0 is
//! the freshly instantiated state (event `null`); step N is the state after
//! the Nth scripted event.
//!
//! The comparison harness (`worker-poc/compare/compare.mjs`) diffs these
//! lines against the isolate host's, per step — that diff is the #2271
//! journal-equality evidence. This test also asserts the native host agrees
//! with itself across a forced cold start (fresh instance + `restore` of
//! the mid-sequence snapshot) for both worlds.
//!
//! Run standalone to regenerate the reference file:
//!   WORKER_POC_REFERENCE_OUT=worker-poc/compare/native-reference.jsonl \
//!     cargo test --test worker_poc_reference -- --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id::sandbox::{Sandbox, SandboxLimits};

const COUNTER_WASM: &[u8] = include_bytes!("../examples/roc-counter/counter.wasm");
const TIC_TAC_TOE_WASM: &[u8] = include_bytes!("../examples/tic-tac-toe/tic-tac-toe.wasm");

/// The #2271 scripted sequences, shared verbatim with the isolate runner.
/// Tic-tac-toe: X plays 0, O plays 4, X plays 1, O retries the occupied 4
/// (ignored), 9 is not a move (ignored), O plays 8, X plays 2 and wins,
/// 3 after the win is ignored.
fn sequences() -> Vec<(&'static str, &'static [u8], Vec<&'static [u8]>, usize)> {
    vec![
        ("roc-counter", COUNTER_WASM, vec![b"inc", b"inc", b"inc"], 2),
        (
            "tic-tac-toe",
            TIC_TAC_TOE_WASM,
            vec![b"0", b"4", b"1", b"4", b"9", b"8", b"2", b"3"],
            5,
        ),
    ]
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn step_line(
    world: &str,
    step: usize,
    event: Option<&[u8]>,
    instance: &mut id::sandbox::WorldInstance,
) -> String {
    let view = String::from_utf8(instance.view().unwrap()).unwrap();
    let records = instance
        .records()
        .unwrap()
        .map(|bytes| String::from_utf8(bytes).unwrap());
    let snapshot = instance.snapshot().unwrap();
    let event_json = match event {
        Some(event) => format!(
            "\"{}\"",
            json_escape(&String::from_utf8(event.to_vec()).unwrap())
        ),
        None => "null".to_string(),
    };
    let records_json = match &records {
        Some(records) => format!("\"{}\"", json_escape(records)),
        None => "null".to_string(),
    };
    let snapshot_json = match &snapshot {
        Some(snapshot) => format!("\"{}\"", hex(snapshot)),
        None => "null".to_string(),
    };
    format!(
        "{{\"world\":\"{world}\",\"step\":{step},\"event\":{event_json},\"view\":\"{}\",\"records\":{records_json},\"snapshot_hex\":{snapshot_json}}}",
        json_escape(&view)
    )
}

/// Run one sequence; when `cold_start_after` is `Some(step)`, restart from
/// that step's snapshot in a fresh instance before continuing.
fn run_sequence(
    world: &str,
    wasm: &[u8],
    events: &[&[u8]],
    cold_start_after: Option<usize>,
) -> Vec<String> {
    let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
    let mut instance = runner.instantiate(0).unwrap();
    let mut lines = vec![step_line(world, 0, None, &mut instance)];
    for (index, event) in events.iter().enumerate() {
        let step = index + 1;
        if cold_start_after == Some(step - 1) {
            let snapshot = instance.snapshot().unwrap().unwrap();
            instance = runner.instantiate(0).unwrap();
            instance.restore(&snapshot).unwrap();
        }
        instance.update(1, event).unwrap();
        lines.push(step_line(world, step, Some(event), &mut instance));
    }
    lines
}

#[test]
fn worker_poc_reference_sequences() {
    let mut all_lines: Vec<String> = Vec::new();
    for (world, wasm, events, cold_start_after) in sequences() {
        let uninterrupted = run_sequence(world, wasm, &events, None);
        let cold_started = run_sequence(world, wasm, &events, Some(cold_start_after));
        assert_eq!(
            uninterrupted, cold_started,
            "native host diverged across a forced cold start after step {cold_start_after} in {world}"
        );
        for line in &uninterrupted {
            println!("{line}");
        }
        all_lines.extend(uninterrupted);
    }

    // The counter must actually count; guards against a vacuous equality
    // pass over a guest that never mutated.
    let counter_final = all_lines
        .iter()
        .filter(|line| line.contains("\"world\":\"roc-counter\""))
        .last()
        .unwrap();
    assert!(
        counter_final.contains("count=3"),
        "counter final step does not show count=3: {counter_final}"
    );
    // Tic-tac-toe: X completes the top row (0,1,2); the final records must
    // name X the winner with five plays on the board.
    let ttt_final = all_lines
        .iter()
        .filter(|line| line.contains("\"world\":\"tic-tac-toe\""))
        .last()
        .unwrap();
    assert!(
        ttt_final.contains("\\\"winner\\\":\\\"X\\\""),
        "tic-tac-toe final step does not record X as winner: {ttt_final}"
    );

    if let Ok(path) = std::env::var("WORKER_POC_REFERENCE_OUT") {
        let mut text = all_lines.join("\n");
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        eprintln!("wrote {} reference lines to {path}", all_lines.len());
    }
}
