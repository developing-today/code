//! Timing/RSS probe for the embedder — spike instrumentation for
//! steps 2–4 (not a benchmark suite). Run:
//!
//!   ATLAS_ORT_LIB=/path/to/libonnxruntime.so \
//!     cargo run --release -p atlas-embed --example timing
//!
//! ATLAS_MODEL_DIR defaults to the pinned HF snapshot, as in the
//! Gate A test.

use std::path::PathBuf;
use std::time::Instant;

use atlas_embed::Embedder;

fn rss_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: f64 = rest.trim().trim_end_matches(" kB").trim().parse().unwrap();
            return kb / 1024.0;
        }
    }
    f64::NAN
}

fn main() {
    let model_dir = std::env::var_os("ATLAS_MODEL_DIR").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").expect("HOME unset");
        PathBuf::from(home).join(".cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/95c2741480856aa9666782eb4afe11959938017f")
    });
    let ort_lib = PathBuf::from(std::env::var_os("ATLAS_ORT_LIB").expect("ATLAS_ORT_LIB required"));

    let t = Instant::now();
    let mut emb = Embedder::load(&model_dir, &ort_lib).expect("load");
    println!(
        "load (env+tokenizer+session): {:.0} ms, RSS {:.0} MB",
        t.elapsed().as_secs_f64() * 1e3,
        rss_mb()
    );

    let fox = "The quick brown fox jumps over the lazy dog.";
    let t = Instant::now();
    let v = emb.embed(&[fox]).expect("embed");
    println!(
        "first single embed (15 tok): {:.0} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    let t = Instant::now();
    for _ in 0..20 {
        emb.embed(&[fox]).expect("embed");
    }
    println!(
        "warm single embed (15 tok): {:.1} ms avg, RSS {:.0} MB",
        t.elapsed().as_secs_f64() * 1e3 / 20.0,
        rss_mb()
    );
    println!("checksum dim0..3: {:?}", &v[0][..3]);

    let long = "Embeddings turn text into vectors that capture semantic meaning. ".repeat(24);
    let t = Instant::now();
    emb.embed(&[long.as_str()]).expect("embed long");
    println!(
        "single embed (~{} tok): {:.0} ms",
        emb.tokenize(&long).unwrap().len(),
        t.elapsed().as_secs_f64() * 1e3
    );

    let batch: Vec<&str> = (0..32).map(|_| fox).collect();
    let t = Instant::now();
    emb.embed(&batch).expect("embed batch");
    println!(
        "batch of 32 short: {:.0} ms, RSS {:.0} MB",
        t.elapsed().as_secs_f64() * 1e3,
        rss_mb()
    );
}
