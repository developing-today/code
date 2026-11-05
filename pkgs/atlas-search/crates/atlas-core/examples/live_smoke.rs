//! LIVE SMOKE (manual; not a CI test) — step 2's verification
//! gate, second half: for a handful of diverse real questions,
//! the Rust scan path's fused top-10 over production Turso must
//! match the deployed Python CLI's top-10 exactly (same corpus,
//! same config: hybrid, pool 100, ceiling personal, RRF k=10).
//!
//! READ-ONLY against production: the Turso token flow resolves
//! a read-only token (env → Bitwarden → 20h cache → mint), and
//! both implementations only ever SELECT.
//!
//! Run on the laptop:
//!
//! ```sh
//! source ~/step2-env.sh
//! cargo run -p atlas-core --example live_smoke --release
//! ```
//!
//! Env knobs: `ATLAS_ORT_LIB` (default: the v3 venv's
//! libonnxruntime), `ATLAS_MODEL_DIR` (default: the pinned HF
//! snapshot from step 1). Questions can be passed as arguments;
//! otherwise a built-in diverse set is used.
//!
//! Loader note: dlopen'ing the venv's libonnxruntime needs an
//! x86-64 `libstdc++.so.6` + `libz.so.1` on `LD_LIBRARY_PATH`
//! (the Python wrapper probes the nix store for the same pair).
//! Prefix the run with e.g.
//! `LD_LIBRARY_PATH=/nix/store/<...>-gcc-<...>-lib/lib:/nix/store/<...>-zlib-<...>/lib`.
//! Step 5's nix packaging must carry both in the closure (the
//! opencode#1554 lesson, designed into #5355).
//!
//! Recorded run (2026-11-04, production corpus, scan backend):
//! 6 of 6 questions matched the Python CLI's top-10 exactly.

use std::process::Command;

use atlas_core::{
    AtlasEmbedder, DEFAULT_VEC_TABLE, Filters, SearchOptions, TursoBackend, run_search,
};
use serde_json::Value as Json;

const DEFAULT_ORT_LIB: &str = "/home/user/atlas/v3-venv/lib/python3.14/site-packages/onnxruntime/capi/libonnxruntime.so.1.31.1";
const DEFAULT_MODEL_DIR: &str = "/home/user/.cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/95c2741480856aa9666782eb4afe11959938017f";
const PYTHON_CLI: &str = "/home/user/.local/bin/atlas-search";

const DEFAULT_QUESTIONS: [&str; 6] = [
    "how do we back up the database",
    "what did we decide about the Muse subscription quota pools",
    "nixpkgs update workflow for the fleet",
    "how does the atlas search ranking work",
    "what happened with the cutter package build failure",
    "explain the reflective event archive design",
];

fn top10(env: &Json) -> Vec<i64> {
    env["results"]
        .as_array()
        .map(|rs| rs.iter().filter_map(|r| r["chunk_id"].as_i64()).collect())
        .unwrap_or_default()
}

fn python_top10(question: &str) -> Result<Vec<i64>, String> {
    let out = Command::new(PYTHON_CLI)
        .args(["search", question])
        .output()
        .map_err(|e| format!("spawn python cli: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "python cli exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let env: Json = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("python cli output not JSON: {e}"))?;
    Ok(top10(&env))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let questions: Vec<String> = if args.is_empty() {
        DEFAULT_QUESTIONS.iter().map(|s| s.to_string()).collect()
    } else {
        args
    };
    let ort_lib = std::env::var("ATLAS_ORT_LIB").unwrap_or_else(|_| DEFAULT_ORT_LIB.to_string());
    let model_dir =
        std::env::var("ATLAS_MODEL_DIR").unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string());

    let embedder = atlas_embed::Embedder::load(
        std::path::Path::new(&model_dir),
        std::path::Path::new(&ort_lib),
    )
    .expect("load embedder");
    let embedder = AtlasEmbedder::new(embedder);
    let backend = TursoBackend::new(DEFAULT_VEC_TABLE).expect("turso backend (token flow)");
    let filters = Filters::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        "personal",
        None,
        None,
    )
    .expect("filters");

    let mut failures = 0;
    for q in &questions {
        let opts = SearchOptions::new(q.clone());
        let rust_env = match run_search(&backend, &embedder, &filters, &opts) {
            Ok(env) => env,
            Err(e) => {
                println!("Q: {q}\n  RUST ERROR: {e}");
                failures += 1;
                continue;
            }
        };
        let rust_top = top10(&rust_env);
        let py_top = match python_top10(q) {
            Ok(v) => v,
            Err(e) => {
                println!("Q: {q}\n  PYTHON ERROR: {e}");
                failures += 1;
                continue;
            }
        };
        if rust_top == py_top {
            println!("Q: {q}\n  MATCH  top10 = {rust_top:?}");
        } else {
            failures += 1;
            let first_diff = rust_top
                .iter()
                .zip(py_top.iter())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| rust_top.len().min(py_top.len()));
            println!(
                "Q: {q}\n  MISMATCH (first diff at rank {})\n  rust:   {rust_top:?}\n  python: {py_top:?}",
                first_diff + 1
            );
        }
    }
    println!(
        "\n{} of {} questions matched exactly",
        questions.len() - failures,
        questions.len()
    );
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
