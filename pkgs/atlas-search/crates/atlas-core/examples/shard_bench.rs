//! Shard-size scan benchmark for task 17164 (manual example):
//! embed one query, then time fanout_scan over a shard dir.
//! Usage: shard_bench <shard_dir> <query> [iterations]

use std::path::PathBuf;
use std::time::Instant;

use atlas_core::filters::Filters;
use atlas_core::shards::scan::{FanoutQuery, fanout_scan};
use atlas_core::shards::{Tier, TierCeiling, open_from_spec};
use atlas_core::{AtlasEmbedder, QueryEmbedder, DEFAULT_VEC_TABLE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shard_dir = args.get(1).expect("usage: shard_bench <shard_dir> <query> [iters]");
    let query = args.get(2).expect("usage: shard_bench <shard_dir> <query> [iters]");
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);

    let model_dir = std::env::var("ATLAS_MODEL_DIR").expect("ATLAS_MODEL_DIR");
    let ort_lib = std::env::var("ATLAS_ORT_LIB").expect("ATLAS_ORT_LIB");
    let emb = atlas_embed::Embedder::load(&PathBuf::from(model_dir), &PathBuf::from(ort_lib))
        .expect("embedder load");
    let embedder = AtlasEmbedder::new(emb);
    let qvec = embedder.embed_query(query, 768).expect("embed query");

    let set = open_from_spec(
        &format!("dir:{shard_dir}"),
        TierCeiling(Tier::Internal),
        DEFAULT_VEC_TABLE,
    )
    .expect("discover shards");
    let chunks: u64 = set.shards().iter().map(|s| s.chunks).sum();
    let filters = Filters::new(vec![], vec![], vec![], vec![], "personal", None, None).unwrap();
    let mut times: Vec<f64> = Vec::new();
    for _ in 0..iters {
        let t0 = Instant::now();
        let result = fanout_scan(
            &set,
            &FanoutQuery {
                qvec: &qvec,
                shard_top_k: 10,
                global_top_k: 10,
                filters: &filters,
                topics: None,
                ceiling: TierCeiling(Tier::Internal),
            },
        )
        .expect("fanout scan");
        std::hint::black_box(result.hits.len());
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = times[times.len() / 2];
    let p95 = times[(times.len() as f64 * 0.95) as usize];
    println!(
        "shard_dir={shard_dir} chunks={chunks} iters={iters} scan_median_ms={median:.3} scan_p95_ms={p95:.3}"
    );
}
