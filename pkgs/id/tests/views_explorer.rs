#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id::directory_view::{DirectoryView, FrozenView, VoidedView, WaitingView, render_text};
use serde_json::Value;

#[test]
fn test_waiting_events_rendered_with_exact_reason() {
    let view = DirectoryView {
        viewer: "admin".to_owned(),
        accounts: Vec::new(),
        groups: Vec::new(),
        deliveries: Vec::new(),
        waiting: vec![
            WaitingView::new("evt-w1", "hash-dep-abc"),
            WaitingView::new("evt-w2", "hash-dep-def"),
        ],
        voided: Vec::new(),
        frozen: Vec::new(),
    };

    let text = render_text(&view);
    assert!(text.contains("waiting:"));
    assert!(text.contains("  evt-w1  hash-dep-abc  waiting: missing hash-dep-abc"));
    assert!(text.contains("  evt-w2  hash-dep-def  waiting: missing hash-dep-def"));
}

#[test]
fn test_voided_events_rendered_with_exact_reason() {
    let view = DirectoryView {
        viewer: "admin".to_owned(),
        accounts: Vec::new(),
        groups: Vec::new(),
        deliveries: Vec::new(),
        waiting: Vec::new(),
        voided: vec![
            VoidedView::new("evt-v1", "hash-rem-123"),
            VoidedView::new("evt-v2", "hash-rem-456"),
        ],
        frozen: Vec::new(),
    };

    let text = render_text(&view);
    assert!(text.contains("voided:"));
    assert!(text.contains("  evt-v1  hash-rem-123  voided: concurrent with hash-rem-123"));
    assert!(text.contains("  evt-v2  hash-rem-456  voided: concurrent with hash-rem-456"));
}

#[test]
fn test_frozen_chains_rendered_with_exact_reason() {
    let view = DirectoryView {
        viewer: "admin".to_owned(),
        accounts: Vec::new(),
        groups: Vec::new(),
        deliveries: Vec::new(),
        waiting: Vec::new(),
        voided: Vec::new(),
        frozen: vec![
            FrozenView::new("author-f1", 7, "proof-hash-789"),
            FrozenView::new("author-f2", 0, "proof-hash-000"),
        ],
    };

    let text = render_text(&view);
    assert!(text.contains("frozen:"));
    assert!(text.contains("  author-f1  proof-hash-789  frozen: fork at seq 7"));
    assert!(text.contains("  author-f2  proof-hash-000  frozen: fork at seq 0"));
}

#[test]
fn test_empty_sections_omitted_or_none() {
    let view = DirectoryView {
        viewer: "anonymous".to_owned(),
        accounts: Vec::new(),
        groups: Vec::new(),
        deliveries: Vec::new(),
        waiting: Vec::new(),
        voided: Vec::new(),
        frozen: Vec::new(),
    };

    let text = render_text(&view);
    assert!(!text.contains("waiting:"));
    assert!(!text.contains("voided:"));
    assert!(!text.contains("frozen:"));
    assert!(text.contains("accounts:\n  (none)"));
    assert!(text.contains("groups:\n  (none)"));
}

#[test]
fn test_json_serialization_includes_waiting_voided_and_frozen_arrays() {
    let view = DirectoryView {
        viewer: "admin".to_owned(),
        accounts: Vec::new(),
        groups: Vec::new(),
        deliveries: Vec::new(),
        waiting: vec![WaitingView::new("evt-w1", "hash-dep-abc")],
        voided: vec![VoidedView::new("evt-v1", "hash-rem-123")],
        frozen: vec![FrozenView::new("author-f1", 12, "proof-hash-456")],
    };

    let val = serde_json::to_value(&view).expect("serialize DirectoryView");

    let waiting_arr = val
        .get("waiting")
        .and_then(Value::as_array)
        .expect("waiting array");
    assert_eq!(waiting_arr.len(), 1);
    assert_eq!(waiting_arr[0]["id"], "evt-w1");
    assert_eq!(waiting_arr[0]["missing"], "hash-dep-abc");
    assert_eq!(waiting_arr[0]["reason"], "waiting: missing hash-dep-abc");

    let voided_arr = val
        .get("voided")
        .and_then(Value::as_array)
        .expect("voided array");
    assert_eq!(voided_arr.len(), 1);
    assert_eq!(voided_arr[0]["id"], "evt-v1");
    assert_eq!(voided_arr[0]["removal"], "hash-rem-123");
    assert_eq!(
        voided_arr[0]["reason"],
        "voided: concurrent with hash-rem-123"
    );

    let frozen_arr = val
        .get("frozen")
        .and_then(Value::as_array)
        .expect("frozen array");
    assert_eq!(frozen_arr.len(), 1);
    assert_eq!(frozen_arr[0]["id"], "author-f1");
    assert_eq!(frozen_arr[0]["seq"], 12);
    assert_eq!(frozen_arr[0]["proof"], "proof-hash-456");
    assert_eq!(frozen_arr[0]["reason"], "frozen: fork at seq 12");

    // Roundtrip verification
    let json_str = serde_json::to_string(&view).expect("to string");
    let deserialized: DirectoryView = serde_json::from_str(&json_str).expect("deserialize");
    assert_eq!(view, deserialized);
}

#[test]
fn test_mixed_view_shared_types_and_in_step_rendering() {
    let view = DirectoryView {
        viewer: "acct-viewer".to_owned(),
        accounts: Vec::new(),
        groups: Vec::new(),
        deliveries: Vec::new(),
        waiting: vec![WaitingView::new("wait-1", "dep-hash-x")],
        voided: vec![VoidedView::new("void-1", "rem-hash-y")],
        frozen: vec![FrozenView::new("froz-1", 3, "proof-hash-z")],
    };

    let text = render_text(&view);
    let val = serde_json::to_value(&view).expect("serialize");

    // In step: exact reasons match between text and JSON
    assert_eq!(
        val["waiting"][0]["reason"].as_str().unwrap(),
        "waiting: missing dep-hash-x"
    );
    assert!(text.contains("waiting: missing dep-hash-x"));

    assert_eq!(
        val["voided"][0]["reason"].as_str().unwrap(),
        "voided: concurrent with rem-hash-y"
    );
    assert!(text.contains("voided: concurrent with rem-hash-y"));

    assert_eq!(
        val["frozen"][0]["reason"].as_str().unwrap(),
        "frozen: fork at seq 3"
    );
    assert!(text.contains("frozen: fork at seq 3"));
}
