//! Transcript fixture check: the recorded client frames in
//! `tests/fixtures/transcript_step1.json` must match the shapes the
//! machine-readable F13.1 contract (`pkgs/id/contract/shell-contract.v1.json`)
//! declares — same frame kinds, same fields, values inside the
//! contract's bounds. This is the step-1 stand-in for the live
//! `id world contract --check` session harness (step 2).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn read_json(relative: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[test]
fn transcript_frames_match_contract_shapes() {
    let contract = read_json("../../contract/shell-contract.v1.json");
    let transcript = read_json("tests/fixtures/transcript_step1.json");
    let client_frames = contract["client_frames"]
        .as_object()
        .expect("contract client_frames");
    let server_frames = contract["server_frames"]
        .as_object()
        .expect("contract server_frames");

    let frames = transcript["frames"].as_array().expect("transcript frames");
    assert!(
        !frames.is_empty(),
        "transcript must record at least one frame"
    );
    for frame in frames {
        let kind = frame["type"].as_str().expect("frame type");
        let spec = client_frames
            .get(kind)
            .or_else(|| server_frames.get(kind))
            .unwrap_or_else(|| panic!("frame kind {kind} is not a contract frame"));
        let allowed: BTreeSet<&str> = spec["fields"]
            .as_object()
            .expect("contract fields")
            .keys()
            .map(String::as_str)
            .collect();
        for key in frame.as_object().expect("frame object").keys() {
            if key == "type" {
                continue;
            }
            assert!(
                allowed.contains(key.as_str()),
                "frame {kind} carries field {key} the contract does not declare"
            );
        }
        // Bounds the contract states numerically must hold for recorded frames.
        if kind == "paste" {
            let max = spec["fields"]["text"]["max_bytes"]
                .as_u64()
                .expect("paste bound") as usize;
            assert!(frame["text"].as_str().expect("paste text").len() <= max);
        }
        if kind == "resize" {
            for field in ["width", "height"] {
                let bound = &spec["fields"][field];
                let value = frame[field].as_u64().expect("resize value");
                assert!(
                    value >= bound["min"].as_u64().expect("min")
                        && value <= bound["max"].as_u64().expect("max")
                );
            }
        }
        if kind == "input" {
            let hex = frame["data_hex"].as_str().expect("input data_hex");
            assert!(
                hex.len() % 2 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()),
                "data_hex must be hex"
            );
        }
    }
}

#[test]
fn transcript_records_a_view_with_data() {
    let transcript = read_json("tests/fixtures/transcript_step1.json");
    let views: Vec<&Value> = transcript["frames"]
        .as_array()
        .expect("frames")
        .iter()
        .filter(|f| f["type"] == "view")
        .collect();
    assert_eq!(
        views.len(),
        1,
        "step-1 transcript records exactly one host view"
    );
    assert!(
        views[0]["data"]
            .as_str()
            .expect("view data")
            .contains("welcome")
    );
}
