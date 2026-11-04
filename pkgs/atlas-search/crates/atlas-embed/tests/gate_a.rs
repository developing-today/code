//! Gate A — embedder parity (developing-today/code#5351, design §5).
//!
//! Asserts, against `pkgs/atlas-search/fixtures/embed-probes.json`
//! (recorded from the production Python corpus pipeline by
//! `gen_fixtures.py`):
//!   * token ids exactly equal on every text-bearing probe;
//!   * output vectors BIT-IDENTICAL (delta 0.0) on every probe,
//!     embedded singly and in one fixed mixed batch.
//!
//! Requires the pinned model snapshot and a dynamically-loadable
//! ONNX Runtime, so it is `#[ignore]`d for ordinary CI:
//!
//!   ATLAS_ORT_LIB=/path/to/libonnxruntime.so \
//!   ATLAS_MODEL_DIR=/path/to/snowflake-arctic-embed-m-v2.0-snapshot \
//!     cargo test -p atlas-embed --test gate_a -- --ignored --nocapture
//!
//! ATLAS_MODEL_DIR defaults to the laptop's HF cache snapshot of the
//! pinned revision. Any mismatch prints the full per-probe table and
//! fails with it — the numbers are the issue-comment evidence either
//! way.

use std::path::PathBuf;

use atlas_embed::Embedder;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    probes: Vec<Probe>,
}

#[derive(Deserialize)]
struct Probe {
    name: String,
    text: Option<String>,
    token_ids: Vec<u32>,
    single_bits: Vec<u32>,
    batch_bits: Vec<u32>,
}

fn delta_stats(got: &[f32; 768], want_bits: &[u32]) -> (f32, usize) {
    let mut max_delta = 0.0f32;
    let mut mismatches = 0usize;
    for (k, &g) in got.iter().enumerate() {
        let w = f32::from_bits(want_bits[k]);
        if g.to_bits() != want_bits[k] {
            mismatches += 1;
        }
        let d = (g - w).abs();
        if d > max_delta {
            max_delta = d;
        }
    }
    (max_delta, mismatches)
}

#[test]
#[ignore = "requires ATLAS_ORT_LIB and the pinned model snapshot; see file docs"]
fn gate_a_bit_identity() {
    let fixtures_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("embed-probes.json");
    let raw = std::fs::read_to_string(&fixtures_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", fixtures_path.display()));
    let fixture: Fixture = serde_json::from_str(&raw).expect("parse embed-probes.json");

    let model_dir = std::env::var_os("ATLAS_MODEL_DIR").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").expect("HOME unset");
        PathBuf::from(home)
            .join(".cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/95c2741480856aa9666782eb4afe11959938017f")
    });
    let ort_lib = std::env::var_os("ATLAS_ORT_LIB")
        .map(PathBuf::from)
        .expect("ATLAS_ORT_LIB must point at a libonnxruntime.so (Gate A environment)");

    let mut embedder = Embedder::load(&model_dir, &ort_lib).expect("load embedder");

    // Resolve each probe's ids: Rust-tokenize text probes (asserting
    // id equality), corpus probes use their recorded ids (their text
    // is personal corpus content, referenced by chunk id — see the
    // fixtures README).
    let mut id_lists: Vec<Vec<u32>> = Vec::with_capacity(fixture.probes.len());
    let mut ids_match: Vec<Option<bool>> = Vec::with_capacity(fixture.probes.len());
    let mut ids_ok = true;
    for probe in &fixture.probes {
        if let Some(text) = &probe.text {
            let got = embedder.tokenize(text).expect("tokenize");
            let eq = got == probe.token_ids;
            if !eq {
                ids_ok = false;
                eprintln!(
                    "TOKEN IDS MISMATCH {}: rust {} ids vs fixture {} ids",
                    probe.name,
                    got.len(),
                    probe.token_ids.len()
                );
            }
            ids_match.push(Some(eq));
            id_lists.push(got);
        } else {
            ids_match.push(None);
            id_lists.push(probe.token_ids.clone());
        }
    }

    let batch = embedder.embed_ids(&id_lists).expect("embed batch");

    let mut report = String::from(
        "probe                  tokens  ids    single max|d| (bitdiff)   batch max|d| (bitdiff)\n",
    );
    let mut all_ok = ids_ok;
    for (i, probe) in fixture.probes.iter().enumerate() {
        let single = embedder
            .embed_ids(std::slice::from_ref(&id_lists[i]))
            .expect("embed single")[0];
        let (sd, sb) = delta_stats(&single, &probe.single_bits);
        let (bd, bb) = delta_stats(&batch[i], &probe.batch_bits);
        if sd != 0.0 || bd != 0.0 {
            all_ok = false;
        }
        report.push_str(&format!(
            "{:<22} {:>6}  {:<5}  {:.3e} ({})        {:.3e} ({})\n",
            probe.name,
            probe.token_ids.len(),
            match ids_match[i] {
                Some(true) => "ok",
                Some(false) => "FAIL",
                None => "n/a",
            },
            sd,
            sb,
            bd,
            bb
        ));
    }
    println!("{report}");
    assert!(all_ok, "Gate A FAILED — per-probe deltas:\n{report}");
}
