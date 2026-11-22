//! Disclosure-context leak tests (#12404, #9096 design
//! section 4). Every test runs the real runner over a fake
//! backend whose branch pool deliberately contains content of
//! every classification — the stub does not pre-filter, so
//! anything withheld was withheld by the output-boundary
//! disclosure pass, and anything served crossed it. The oracle
//! is the design's context table:
//!
//!   fleet-internal  -> local-only ceiling
//!   hosted-per-user -> personal ceiling
//!   public-surface  -> shareable ceiling
//!   local-owner     -> secret-bearing ceiling
//!
//! intersected with the caller's grant ceiling, with
//! `excluded_personal_life` ineligible in every context and
//! missing/unknown sensitivity labels fail-closed to
//! secret-bearing.

use atlas_core::backend::SearchBackend;
use atlas_core::embed::QueryEmbedder;
use atlas_core::error::Result;
use atlas_core::filters::Filters;
use atlas_core::types::{
    BranchHit, BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow,
};
use atlas_core::{
    DisclosureContext, SearchMode, SearchOptions, run_get_chunk_with_context, run_search,
};
use atlas_proto::RawMode;
use serde_json::Value as Json;

/// (chunk_id, label, text, session)
const CORPUS: [(i64, Option<&str>, &str, &str); 7] = [
    (1, Some("shareable"), "shareable deployment notes", "s1"),
    (2, Some("personal"), "personal project journal", "s1"),
    (3, Some("local-only"), "local-only homelab runbook", "s2"),
    (4, Some("secret-bearing"), "SECRET production API key", "s1"),
    (
        5,
        Some("excluded_personal_life"),
        "personal-life diary entry",
        "s2",
    ),
    (6, None, "unlabeled scratch note", "s2"),
    (7, Some("internal"), "mislabeled ops note", "s2"),
];

struct LeakBackend;

fn detail(id: i64) -> ChunkDetail {
    let (_, label, text, session) = CORPUS
        .iter()
        .find(|(cid, _, _, _)| *cid == id)
        .unwrap_or_else(|| panic!("unknown chunk {id}"));
    ChunkDetail {
        chunk_id: id,
        text: text.to_string(),
        // Chunk 1 sits in message m1 at index 1; its message
        // also holds chunks 2 (index 2) and 4 (index 4), which
        // the neighbors stub below serves as its context.
        chunk_idx: id,
        message_id: if id == 1 || id == 2 || id == 4 {
            "m1".to_string()
        } else {
            format!("m{id}")
        },
        m_role: Some("user".to_string()),
        session_id: session.to_string(),
        s_title: Some("session".to_string()),
        s_client: None,
        s_created_at: None,
        l_kind: None,
        l_topics: None,
        l_projects: None,
        eff_sensitivity: label.map(str::to_string),
    }
}

impl SearchBackend for LeakBackend {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn vec_table(&self) -> &str {
        "vec_chunks"
    }
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
    fn branch_pools(&self, _q: &BranchQuery, _f: &Filters) -> Result<BranchPools> {
        // No filter-before-rank here on purpose: the pool is
        // the whole corpus, so the boundary pass is the only
        // thing that can withhold a candidate.
        Ok(BranchPools {
            fts: CORPUS
                .iter()
                .map(|(chunk_id, _, _, _)| BranchHit {
                    chunk_id: *chunk_id,
                    dist: None,
                })
                .collect(),
            vector: Vec::new(),
            candidate_models: Default::default(),
        })
    }
    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        Ok(ids.iter().map(|&id| detail(id)).collect())
    }
    fn neighbors(&self, message_id: &str, _idx: i64, _n: i64) -> Result<Vec<NeighborRow>> {
        if message_id != "m1" {
            return Ok(Vec::new());
        }
        Ok([2, 4]
            .iter()
            .map(|&id| NeighborRow {
                chunk_id: id,
                chunk_idx: id,
                text: detail(id).text,
            })
            .collect())
    }
    fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        Ok(Some(MessageRow {
            id: message_id.to_string(),
            role: Some("user".to_string()),
            text: format!("FULL MESSAGE TEXT of {message_id}, beyond any single chunk"),
        }))
    }
}

struct FakeEmbedder;

impl QueryEmbedder for FakeEmbedder {
    fn embed_query(&self, _text: &str, _trunc: usize) -> Result<Vec<f32>> {
        Ok(vec![0.0; 4])
    }
}

fn filters(ceiling: &str) -> Filters {
    Filters::new(vec![], vec![], vec![], vec![], ceiling, None, None).expect("filters")
}

fn search(context: DisclosureContext, ceiling: &str, tweak: impl Fn(&mut SearchOptions)) -> Json {
    let mut o = SearchOptions::new("deployment");
    o.mode = SearchMode::Fts;
    o.k = 10;
    o.context = context;
    tweak(&mut o);
    run_search(&LeakBackend, &FakeEmbedder, &filters(ceiling), &o).expect("search")
}

fn served_ids(out: &Json) -> Vec<i64> {
    out["results"]
        .as_array()
        .expect("results array")
        .iter()
        .map(|r| r["chunk_id"].as_i64().expect("chunk id"))
        .collect()
}

#[test]
fn public_surface_filters_even_a_fully_cleared_caller() {
    // The packet's headline case: caller clearance is maximal,
    // the destination is public — only shareable content may
    // cross, in ANY field of the envelope.
    let out = search(DisclosureContext::PublicSurface, "secret-bearing", |_| {});
    assert_eq!(served_ids(&out), vec![1]);
    let whole = out.to_string();
    for leaked in [
        "personal project journal",
        "local-only homelab runbook",
        "SECRET production API key",
        "personal-life diary entry",
        "unlabeled scratch note",
        "mislabeled ops note",
    ] {
        assert!(
            !whole.contains(leaked),
            "envelope leaked {leaked:?}: {whole}"
        );
    }
    // The withholding is on the record, not silent.
    assert!(
        out["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("public-surface")),
        "notes record the disclosure decision: {out}"
    );
}

#[test]
fn fleet_internal_withholds_secret_bearing_from_a_cleared_caller() {
    // fleet-internal's ceiling is local-only: secret-bearing
    // content needs the dedicated secret path, which an
    // ordinary search is not.
    let out = search(DisclosureContext::FleetInternal, "secret-bearing", |_| {});
    assert_eq!(served_ids(&out), vec![1, 2, 3]);
}

#[test]
fn hosted_context_narrows_a_wider_grant() {
    let out = search(DisclosureContext::HostedPerUser, "secret-bearing", |_| {});
    assert_eq!(served_ids(&out), vec![1, 2]);
}

#[test]
fn context_never_widens_a_narrower_grant() {
    // The intersection is symmetric: the widest context with a
    // personal grant still serves only personal-and-below.
    let out = search(DisclosureContext::LocalOwner, "personal", |_| {});
    assert_eq!(served_ids(&out), vec![1, 2]);
}

#[test]
fn excluded_content_is_ineligible_in_every_context() {
    for context in DisclosureContext::all() {
        let out = search(context, "secret-bearing", |_| {});
        assert!(
            !served_ids(&out).contains(&5),
            "{} served the excluded chunk",
            context.name()
        );
        assert!(
            !out.to_string().contains("personal-life diary entry"),
            "{} leaked the excluded chunk's text",
            context.name()
        );
    }
}

#[test]
fn missing_and_unknown_labels_fail_closed_to_secret_bearing() {
    // Chunks 6 (no label) and 7 (non-ladder label) classify as
    // secret-bearing: servable only where secret-bearing is.
    let owner = search(DisclosureContext::LocalOwner, "secret-bearing", |_| {});
    assert!(served_ids(&owner).contains(&6));
    assert!(served_ids(&owner).contains(&7));
    let public = search(DisclosureContext::PublicSurface, "secret-bearing", |_| {});
    assert!(!served_ids(&public).contains(&6));
    assert!(!served_ids(&public).contains(&7));
    let fleet = search(DisclosureContext::FleetInternal, "secret-bearing", |_| {});
    assert!(!served_ids(&fleet).contains(&6));
    assert!(!served_ids(&fleet).contains(&7));
}

#[test]
fn unstated_context_defaults_to_public_surface() {
    // The library default is fail-closed: a caller that never
    // states a context gets the strictest outbound treatment.
    let mut o = SearchOptions::new("deployment");
    o.mode = SearchMode::Fts;
    o.k = 10;
    let out =
        run_search(&LeakBackend, &FakeEmbedder, &filters("secret-bearing"), &o).expect("search");
    assert_eq!(served_ids(&out), vec![1]);
}

#[test]
fn grouped_output_counts_only_permitted_hits() {
    let out = search(DisclosureContext::FleetInternal, "secret-bearing", |o| {
        o.group_by_session = true
    });
    let groups = out["results"].as_array().expect("groups");
    let s1 = groups
        .iter()
        .find(|g| g["session_id"] == "s1")
        .expect("s1 group");
    // s1 holds chunks 1, 2, 4; chunk 4 (secret-bearing) is
    // withheld, so neither the chunks nor the hit_count may
    // reflect it — a count is metadata, and metadata is covered.
    assert_eq!(s1["hit_count"], 2);
    let chunk_ids: Vec<i64> = s1["chunks"]
        .as_array()
        .expect("chunks")
        .iter()
        .map(|c| c["chunk_id"].as_i64().unwrap())
        .collect();
    assert_eq!(chunk_ids, vec![1, 2]);
    assert!(!out.to_string().contains("SECRET production API key"));
}

#[test]
fn raw_branch_rows_are_filtered() {
    let out = search(DisclosureContext::PublicSurface, "secret-bearing", |o| {
        o.parameters.raw_mode = RawMode::Branches
    });
    assert_eq!(served_ids(&out), vec![1]);
    assert!(!out.to_string().contains("SECRET production API key"));
    assert!(!out.to_string().contains("personal-life diary entry"));
}

#[test]
fn expand_drops_neighbors_above_the_intersection() {
    // Chunk 1's message-mates are chunk 2 (personal) and chunk
    // 4 (secret-bearing). Expansion is a boundary too.
    let after_texts = |context: DisclosureContext| -> Vec<String> {
        let out = search(context, "secret-bearing", |o| {
            o.k = 1;
            o.expand = 2;
        });
        out["results"][0]["context"]["after"]
            .as_array()
            .expect("after neighbors")
            .iter()
            .map(|n| n["text"].as_str().unwrap_or("").to_string())
            .collect()
    };
    let fleet = after_texts(DisclosureContext::FleetInternal);
    assert!(fleet.iter().any(|t| t.contains("personal project journal")));
    assert!(!fleet.iter().any(|t| t.contains("SECRET")));
    let public = after_texts(DisclosureContext::PublicSurface);
    assert!(
        public.is_empty(),
        "public expand served neighbors: {public:?}"
    );
    let owner = after_texts(DisclosureContext::LocalOwner);
    assert!(
        owner
            .iter()
            .any(|t| t.contains("SECRET production API key"))
    );
}

#[test]
fn get_chunk_gates_on_context_and_never_serves_excluded() {
    // Excluded: not found even for the owner — and the error is
    // the same shape as a missing chunk, so the lookup cannot
    // confirm the content exists.
    let err = run_get_chunk_with_context(&LeakBackend, 5, 0, DisclosureContext::LocalOwner)
        .expect_err("excluded chunk must not be served");
    assert_eq!(err.to_string(), "chunk 5 not found");
    // Secret-bearing: served to the owner, withheld publicly.
    let owner = run_get_chunk_with_context(&LeakBackend, 4, 0, DisclosureContext::LocalOwner)
        .expect("owner get-chunk");
    assert_eq!(owner["text"], "SECRET production API key");
    let err = run_get_chunk_with_context(&LeakBackend, 4, 0, DisclosureContext::PublicSurface)
        .expect_err("secret chunk must not cross to a public surface");
    assert_eq!(err.to_string(), "chunk 4 not found");
    // Shareable crosses everywhere.
    let public = run_get_chunk_with_context(&LeakBackend, 1, 0, DisclosureContext::PublicSurface)
        .expect("public get-chunk");
    assert_eq!(public["text"], "shareable deployment notes");
}

#[test]
fn get_chunk_neighbors_are_filtered() {
    let out = run_get_chunk_with_context(&LeakBackend, 1, 2, DisclosureContext::FleetInternal)
        .expect("get-chunk");
    let after = out["context"]["after"].as_array().expect("after");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0]["chunk_id"], 2);
}

#[test]
fn public_surface_withholds_full_content_payloads() {
    // --with-message and hydration attach text beyond the
    // classified chunk; a public surface cannot filter them at
    // chunk granularity, so they are withheld outright while
    // the chunk's own (shareable) text still serves.
    let out = search(DisclosureContext::PublicSurface, "secret-bearing", |o| {
        o.with_message = true;
        o.parameters.hydrate = 1;
    });
    let entry = &out["results"][0];
    assert_eq!(entry["text"], "shareable deployment notes");
    assert!(entry.get("message").is_none(), "message leaked: {entry}");
    assert!(entry.get("hydrated").is_none(), "hydration leaked: {entry}");
    assert!(!out.to_string().contains("FULL MESSAGE TEXT"));
    // The same request on a fleet-internal surface keeps the
    // payloads: the withholding is the context's, not the
    // feature's.
    let fleet = search(DisclosureContext::FleetInternal, "secret-bearing", |o| {
        o.with_message = true;
        o.parameters.hydrate = 1;
    });
    let entry = &fleet["results"][0];
    assert!(entry.get("message").is_some());
    assert!(entry.get("hydrated").is_some());
}
