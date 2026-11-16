//! Slice-2 flag behavior (#9942, #5668 s3.3): --hydrate,
//! --raw-rows (ungrouped / branches), and the guarantee that the
//! shipped defaults render exactly as before. Runs the real
//! runner over a fake backend so the envelope shapes are the
//! production ones.

use atlas_core::backend::SearchBackend;
use atlas_core::embed::QueryEmbedder;
use atlas_core::error::Result;
use atlas_core::filters::Filters;
use atlas_core::types::{
    BranchHit, BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow,
};
use atlas_core::{SearchMode, SearchOptions, run_search};
use atlas_proto::RawMode;
use serde_json::Value as Json;

struct FakeBackend;

impl SearchBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn vec_table(&self) -> &str {
        "vec_chunks"
    }
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
    fn branch_pools(&self, _q: &BranchQuery, _filters: &Filters) -> Result<BranchPools> {
        Ok(BranchPools {
            fts: [1, 2, 3]
                .iter()
                .map(|&chunk_id| BranchHit {
                    chunk_id,
                    dist: None,
                })
                .collect(),
            vector: Vec::new(),
            candidate_models: Default::default(),
        })
    }
    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        Ok(ids
            .iter()
            .map(|&id| ChunkDetail {
                chunk_id: id,
                text: format!("full text of chunk {id}, the complete stored content"),
                chunk_idx: id,
                message_id: format!("m{id}"),
                m_role: Some("user".to_string()),
                session_id: "s1".to_string(),
                s_title: None,
                s_client: None,
                s_created_at: None,
                l_kind: None,
                l_topics: None,
                l_projects: None,
                eff_sensitivity: None,
            })
            .collect())
    }
    fn neighbors(&self, _message_id: &str, _chunk_idx: i64, _n: i64) -> Result<Vec<NeighborRow>> {
        Ok(Vec::new())
    }
    fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        Ok(Some(MessageRow {
            id: message_id.to_string(),
            role: Some("user".to_string()),
            text: format!("the complete uncapped text of {message_id}, end to end"),
        }))
    }
}

struct FakeEmbedder;

impl QueryEmbedder for FakeEmbedder {
    fn embed_query(&self, _text: &str, _trunc: usize) -> Result<Vec<f32>> {
        Ok(vec![0.0; 4])
    }
}

fn opts() -> SearchOptions {
    let mut o = SearchOptions::new("atlas");
    o.mode = SearchMode::Fts;
    o
}

fn results(out: &Json) -> &Vec<Json> {
    out["results"].as_array().expect("results is an array")
}

#[test]
fn defaults_render_the_shipped_shape() {
    let out = run_search(&FakeBackend, &FakeEmbedder, &Filters::default(), &opts()).unwrap();
    assert_eq!(results(&out).len(), 3);
    for entry in results(&out) {
        assert!(entry.get("hydrated").is_none(), "no hydration by default");
        assert!(
            entry.get("chunk_id").is_some(),
            "flat entries carry chunk_id"
        );
        assert!(
            entry.get("score").is_some(),
            "flat entries carry the fused score"
        );
        assert!(
            entry.get("chunks").is_none(),
            "flat entries are not session groups"
        );
    }
    assert!(out.get("raw").is_none(), "no raw marker by default");
}

#[test]
fn hydrate_marks_exactly_the_top_n() {
    let mut o = opts();
    o.parameters.hydrate = 2;
    let out = run_search(&FakeBackend, &FakeEmbedder, &Filters::default(), &o).unwrap();
    let r = results(&out);
    assert_eq!(r.len(), 3);
    for entry in &r[..2] {
        let h = entry.get("hydrated").expect("top-2 entries are hydrated");
        assert!(
            h["chunk_text"]
                .as_str()
                .unwrap()
                .contains("complete stored content"),
            "hydrated chunk_text is the full text"
        );
        assert!(
            h["message"]["text"]
                .as_str()
                .unwrap()
                .contains("complete uncapped text"),
            "hydrated message is uncapped"
        );
    }
    assert!(
        r[2].get("hydrated").is_none(),
        "third result stays a detail row"
    );
}

#[test]
fn raw_ungrouped_flattens_grouped_output() {
    let mut o = opts();
    o.group_by_session = true;
    o.parameters.raw_mode = RawMode::Ungrouped;
    let out = run_search(&FakeBackend, &FakeEmbedder, &Filters::default(), &o).unwrap();
    assert_eq!(out["raw"], "ungrouped");
    for entry in results(&out) {
        assert!(
            entry.get("chunk_id").is_some(),
            "ungrouped rows are flat fused entries"
        );
        assert!(
            entry.get("chunks").is_none(),
            "ungrouped rows are not session groups"
        );
    }
}

#[test]
fn raw_branches_returns_prefusion_rows() {
    let mut o = opts();
    o.parameters.raw_mode = RawMode::Branches;
    let out = run_search(&FakeBackend, &FakeEmbedder, &Filters::default(), &o).unwrap();
    assert_eq!(out["raw"], "branches");
    let r = results(&out);
    assert_eq!(r.len(), 3);
    for (i, row) in r.iter().enumerate() {
        assert_eq!(row["branch"], "fts");
        assert_eq!(row["rank"], i + 1);
        assert_eq!(row["chunk_id"], i as i64 + 1);
        assert!(
            row.get("score").is_none(),
            "branch rows are pre-fusion: no fused score"
        );
    }
}

#[test]
fn hydrate_with_grouping_is_refused_not_ignored() {
    let mut o = opts();
    o.group_by_session = true;
    o.parameters.hydrate = 1;
    let err = run_search(&FakeBackend, &FakeEmbedder, &Filters::default(), &o).unwrap_err();
    assert!(
        err.to_string().contains("hydrate requires flat output"),
        "unexpected error: {err}"
    );
}

#[test]
fn expand_still_means_neighbor_chunks() {
    let mut o = opts();
    o.expand = 1;
    let out = run_search(&FakeBackend, &FakeEmbedder, &Filters::default(), &o).unwrap();
    // The fake has no neighbors, but the shipped field must ride
    // on every entry exactly as before slice 2.
    for entry in results(&out) {
        assert!(
            entry.get("context").is_some(),
            "expand keeps its shipped meaning"
        );
        assert!(entry.get("hydrated").is_none(), "expand is not hydration");
    }
}
