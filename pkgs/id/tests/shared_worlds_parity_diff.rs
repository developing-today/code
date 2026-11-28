//! Hosted-edge parity diff harness (#15950).
//!
//! The golden baseline (`fixtures/shared_worlds_spike_audit.json`, pinned by
//! `shared_worlds_spike.rs`) is the local-run reference a hosted-edge run of
//! the scripted shared-worlds spike must reproduce. This harness turns
//! "compare authority/audit outcomes" into a command: feed a candidate
//! (hosted-edge) transcript through [`diff_against_golden`] and every
//! divergence in authority decisions (admit/refuse outcomes) or audit events
//! is reported as a line naming the event index, the field, and both sides,
//! so the hosted-edge run can record its divergences back on #10054
//! mechanically instead of by eyeballing JSON.
//!
//! Oracles: the committed golden fixture itself (identity cases), plus
//! synthetic mutations with known divergence counts (mutation cases).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;

/// Authority-decision fields compare first so admit/refuse divergences sort
/// before cosmetic ones; every other field follows in sorted order.
const PRIORITY_FIELDS: [&str; 2] = ["outcome", "kind"];

/// Load the committed golden baseline: `{"events": [...], "head": "<hex>"}`.
pub fn load_golden() -> Value {
    serde_json::from_str(include_str!("fixtures/shared_worlds_spike_audit.json"))
        .expect("golden baseline fixture must parse")
}

fn fmt_val(value: Option<&Value>) -> String {
    match value {
        Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "<unprintable>".to_owned()),
        None => "<missing>".to_owned(),
    }
}

fn ordered_keys(
    a: &serde_json::Map<String, Value>,
    b: &serde_json::Map<String, Value>,
) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut priority = Vec::new();
    let mut rest: BTreeSet<&str> = BTreeSet::new();
    for key in a.keys().chain(b.keys()).map(String::as_str) {
        if PRIORITY_FIELDS.contains(&key) {
            if !priority.contains(&key) {
                priority.push(key);
            }
        } else {
            rest.insert(key);
        }
    }
    priority
        .into_iter()
        .map(str::to_owned)
        .chain(rest.into_iter().map(str::to_owned))
        .collect()
}

/// Compare a candidate transcript against the golden baseline.
///
/// Returns one line per divergence: a missing `events` array, an event-count
/// mismatch, per-event field mismatches, extra events past the shared
/// prefix, or a hash-chain head mismatch. Empty means the candidate
/// reproduces the local run exactly.
pub fn diff_against_golden(golden: &Value, candidate: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match (
        golden.get("events").and_then(Value::as_array),
        candidate.get("events").and_then(Value::as_array),
    ) {
        (Some(expected), Some(actual)) => {
            if expected.len() != actual.len() {
                out.push(format!(
                    "event count differs: golden has {} events, candidate has {}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (index, (ge, ce)) in expected.iter().zip(actual.iter()).enumerate() {
                match (ge.as_object(), ce.as_object()) {
                    (Some(gm), Some(cm)) => {
                        for key in ordered_keys(gm, cm) {
                            if gm.get(&key) != cm.get(&key) {
                                out.push(format!(
                                    "event {index} field `{key}` differs: golden={} candidate={}",
                                    fmt_val(gm.get(&key)),
                                    fmt_val(cm.get(&key))
                                ));
                            }
                        }
                    }
                    _ if ge != ce => out.push(format!(
                        "event {index} differs: golden={} candidate={}",
                        fmt_val(Some(ge)),
                        fmt_val(Some(ce))
                    )),
                    _ => {}
                }
            }
            for (index, ce) in actual.iter().enumerate().skip(expected.len()) {
                out.push(format!(
                    "event {index} is extra in candidate: {}",
                    fmt_val(Some(ce))
                ));
            }
        }
        _ => out.push("candidate or golden is missing an `events` array".to_owned()),
    }
    if golden.get("head") != candidate.get("head") {
        out.push(format!(
            "hash-chain head differs: golden={} candidate={}",
            fmt_val(golden.get("head")),
            fmt_val(candidate.get("head"))
        ));
    }
    out
}

#[test]
fn golden_baseline_shape_is_events_plus_head() {
    let golden = load_golden();
    let events = golden
        .get("events")
        .and_then(Value::as_array)
        .expect("golden has an events array");
    assert!(!events.is_empty(), "golden baseline must not be empty");
    let head = golden
        .get("head")
        .and_then(Value::as_str)
        .expect("golden has a head string");
    assert_eq!(head.len(), 64, "head must be a 32-byte hex digest");
    assert!(
        head.chars().all(|c| c.is_ascii_hexdigit()),
        "head must be hex"
    );
}

#[test]
fn golden_matches_itself_with_no_divergence() {
    let golden = load_golden();
    assert_eq!(
        diff_against_golden(&golden, &golden.clone()),
        Vec::<String>::new()
    );
}

#[test]
fn outcome_mutation_reports_the_authority_divergence() {
    let golden = load_golden();
    let mut candidate = golden.clone();
    candidate["events"][5]["outcome"] = Value::String("refused".to_owned());
    // A real divergent hosted run carries a head computed over its own
    // events, so its recorded head differs from golden's as well.
    candidate["head"] = Value::String("f".repeat(64));
    let divergences = diff_against_golden(&golden, &candidate);
    assert_eq!(
        divergences.len(),
        2,
        "one field line plus the head line (head covers every event): {divergences:?}"
    );
    assert!(
        divergences[0].contains("event 5")
            && divergences[0].contains("outcome")
            && divergences[0].contains("executed")
            && divergences[0].contains("refused"),
        "field line names the event, the field, and both sides: {divergences:?}"
    );
    assert!(
        divergences[1].contains("head"),
        "mutating an event invalidates the chain head: {divergences:?}"
    );
}

#[test]
fn dropped_event_reports_count_and_head() {
    let golden = load_golden();
    let mut candidate = golden.clone();
    candidate
        .get_mut("events")
        .and_then(Value::as_array_mut)
        .expect("candidate has events")
        .pop();
    // The shortened transcript's chain head cannot equal golden's.
    candidate["head"] = Value::String("f".repeat(64));
    let divergences = diff_against_golden(&golden, &candidate);
    assert!(
        divergences
            .iter()
            .any(|line| line.contains("event count differs")
                && line.contains("25")
                && line.contains("24")),
        "count line names both lengths: {divergences:?}"
    );
    assert!(
        divergences.iter().any(|line| line.contains("head")),
        "dropping an event invalidates the chain head: {divergences:?}"
    );
}

#[test]
fn head_tamper_alone_reports_only_the_head() {
    let golden = load_golden();
    let mut candidate = golden.clone();
    candidate["head"] = Value::String("0".repeat(64));
    assert_eq!(
        diff_against_golden(&golden, &candidate),
        vec![format!(
            "hash-chain head differs: golden={} candidate={}",
            fmt_val(golden.get("head")),
            fmt_val(candidate.get("head"))
        )]
    );
}

#[test]
fn missing_events_array_fails_closed() {
    let golden = load_golden();
    let divergences = diff_against_golden(&golden, &Value::Null);
    assert!(
        divergences
            .iter()
            .any(|line| line.contains("missing an `events` array")),
        "a shapeless candidate is one explicit divergence, not a panic: {divergences:?}"
    );
}
