//! Live retrieval proof for task 17164 (manual example, not a CI
//! test): embed a semantic query with the production query
//! embedder, fan out over a local shard dir, and print the top
//! hits with their chunk text read back from the shard itself.
//!
//! Run on the laptop:
//!   ATLAS_ORT_LIB=<libonnxruntime> ATLAS_MODEL_DIR=<snapshot> \
//!   cargo run --release -p atlas-core --example retrieval_proof -- <shard_dir> <query>

use std::path::PathBuf;

use atlas_core::filters::Filters;
use atlas_core::shards::scan::{FanoutQuery, fanout_scan};
use atlas_core::shards::{Tier, TierCeiling, open_from_spec};
use atlas_core::{AtlasEmbedder, DEFAULT_VEC_TABLE, QueryEmbedder};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shard_dir = args
        .get(1)
        .expect("usage: retrieval_proof <shard_dir> <query>");
    let query = args
        .get(2)
        .expect("usage: retrieval_proof <shard_dir> <query>");

    let model_dir = std::env::var("ATLAS_MODEL_DIR").expect("ATLAS_MODEL_DIR");
    let ort_lib = std::env::var("ATLAS_ORT_LIB").expect("ATLAS_ORT_LIB");
    let emb = atlas_embed::Embedder::load(&PathBuf::from(model_dir), &PathBuf::from(ort_lib))
        .expect("embedder load");
    let embedder = AtlasEmbedder::new(emb);
    let qvec = embedder.embed_query(query, 768).expect("embed query");
    println!("query: {query}");
    println!("query vector dims: {}", qvec.len());

    let set = open_from_spec(
        &format!("dir:{shard_dir}"),
        TierCeiling(Tier::Internal),
        DEFAULT_VEC_TABLE,
    )
    .expect("discover shards");
    for s in set.shards() {
        println!(
            "shard: {} tier={} topic={} chunks={} model={}",
            s.name,
            s.tier.name(),
            s.topic,
            s.chunks,
            s.embedding_model
        );
    }
    let filters = Filters::new(vec![], vec![], vec![], vec![], "personal", None, None).unwrap();
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
    for hit in &result.hits {
        println!(
            "hit: chunk_id={} dist={:.6} shard={}",
            hit.chunk_id, hit.dist, hit.shard
        );
        // Read the chunk text back from the shard file itself.
        let db = PathBuf::from(shard_dir).join(format!("{}.db", hit.shard));
        let conn = rusqlite::Connection::open(&db).expect("open shard");
        let (message_id, text): (String, String) = conn
            .query_row(
                "SELECT message_id, text FROM chunks WHERE id = ?1",
                rusqlite::params![hit.chunk_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("chunk row");
        let preview: String = text.chars().take(240).collect();
        println!("  source (message_id): {message_id}");
        println!("  text preview: {}", preview.replace('\n', " | "));
    }
}
