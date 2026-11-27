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
    DisclosureContext, SearchMode, SearchOptions, Sensitivity, run_get_chunk_with_context,
    run_search,
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
    (7, Some("public"), "canonical public note", "s2"),
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
        // Message m1's text is composed of its chunks' texts
        // (1, 2, 4) joined by gap text — the real corpus shape,
        // which the #12624 span filter locates spans in. Other
        // messages hold a single chunk each, so their text
        // needs no span structure.
        let text = if message_id == "m1" {
            "FULL MESSAGE TEXT: shareable deployment notes; \
             personal project journal; SECRET production API key"
                .to_string()
        } else {
            format!("FULL MESSAGE TEXT of {message_id}, beyond any single chunk")
        };
        Ok(Some(MessageRow {
            id: message_id.to_string(),
            role: Some("user".to_string()),
            text,
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
    assert_eq!(served_ids(&out), vec![1, 7]);
    let whole = out.to_string();
    assert!(
        whole.contains("canonical public note"),
        "canonical shareable spelling must serve publicly: {whole}"
    );
    for leaked in [
        "personal project journal",
        "local-only homelab runbook",
        "SECRET production API key",
        "personal-life diary entry",
        "unlabeled scratch note",
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
    assert_eq!(served_ids(&out), vec![1, 2, 3, 7]);
}

#[test]
fn hosted_context_narrows_a_wider_grant() {
    let out = search(DisclosureContext::HostedPerUser, "secret-bearing", |_| {});
    assert_eq!(served_ids(&out), vec![1, 2, 7]);
}

#[test]
fn context_never_widens_a_narrower_grant() {
    // The intersection is symmetric: the widest context with a
    // personal grant still serves only personal-and-below.
    let out = search(DisclosureContext::LocalOwner, "personal", |_| {});
    assert_eq!(served_ids(&out), vec![1, 2, 7]);
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
    // Chunk 6 (no label) classifies as secret-bearing: servable
    // only where secret-bearing is. Chunk 7 carries the
    // canonical `public` spelling and dual-reads as shareable
    // (#13506 stage 2), so it serves wherever shareable does.
    let owner = search(DisclosureContext::LocalOwner, "secret-bearing", |_| {});
    assert!(served_ids(&owner).contains(&6));
    assert!(served_ids(&owner).contains(&7));
    let public = search(DisclosureContext::PublicSurface, "secret-bearing", |_| {});
    assert!(!served_ids(&public).contains(&6));
    assert!(served_ids(&public).contains(&7));
    let fleet = search(DisclosureContext::FleetInternal, "secret-bearing", |_| {});
    assert!(!served_ids(&fleet).contains(&6));
    assert!(served_ids(&fleet).contains(&7));
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
    assert_eq!(served_ids(&out), vec![1, 7]);
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
    assert_eq!(served_ids(&out), vec![1, 7]);
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
    let err = run_get_chunk_with_context(
        &LeakBackend,
        5,
        0,
        DisclosureContext::LocalOwner,
        Sensitivity::SecretBearing,
    )
    .expect_err("excluded chunk must not be served");
    assert_eq!(err.to_string(), "chunk 5 not found");
    // Secret-bearing: served to the owner, withheld publicly.
    let owner = run_get_chunk_with_context(
        &LeakBackend,
        4,
        0,
        DisclosureContext::LocalOwner,
        Sensitivity::SecretBearing,
    )
    .expect("owner get-chunk");
    assert_eq!(owner["text"], "SECRET production API key");
    let err = run_get_chunk_with_context(
        &LeakBackend,
        4,
        0,
        DisclosureContext::PublicSurface,
        Sensitivity::SecretBearing,
    )
    .expect_err("secret chunk must not cross to a public surface");
    assert_eq!(err.to_string(), "chunk 4 not found");
    // Shareable crosses everywhere.
    let public = run_get_chunk_with_context(
        &LeakBackend,
        1,
        0,
        DisclosureContext::PublicSurface,
        Sensitivity::SecretBearing,
    )
    .expect("public get-chunk");
    assert_eq!(public["text"], "shareable deployment notes");
}

#[test]
fn get_chunk_neighbors_are_filtered() {
    let out = run_get_chunk_with_context(
        &LeakBackend,
        1,
        2,
        DisclosureContext::FleetInternal,
        Sensitivity::SecretBearing,
    )
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
    // And under #12624 the payloads are span-filtered: chunk
    // 4's secret-bearing span is omitted from chunk 1's
    // message, while chunk 2's personal span still crosses.
    let text = entry["message"]["text"].as_str().expect("message text");
    assert!(text.contains("personal project journal"), "{text}");
    assert!(
        !text.contains("SECRET production API key"),
        "secret-bearing sibling span crossed: {text}"
    );
}

// ---------------------------------------------------------------
// #12624: span-level filtering of full-content payloads. A
// message's chunks can classify differently from the chunk a
// payload is attached to; the payload may contain only the
// spans whose own chunks pass the same grant ∩ context
// intersection the disclosure pass applies to candidates.
// Withheld spans are omitted (the codebase's withholding
// idiom) and counted in the envelope notes; a failing chunk
// whose text cannot be located in the message text makes the
// payload unfilterable and it is withheld whole (design
// section 4: an output that cannot be safely filtered is
// withheld).

/// (chunk_id, label, text, message, chunk_idx)
const SPAN_CORPUS: [(i64, Option<&str>, &str, &str, i64); 6] = [
    (
        101,
        Some("shareable"),
        "the deploy window opens at dawn",
        "mmix",
        0,
    ),
    (
        102,
        Some("secret-bearing"),
        "the vault code is 493812",
        "mmix",
        1,
    ),
    (
        103,
        Some("personal"),
        "remember to water the plants",
        "mmix",
        2,
    ),
    (
        104,
        Some("excluded_personal_life"),
        "diary entry: the diagnosis results",
        "mmix",
        3,
    ),
    (
        105,
        Some("shareable"),
        "a perfectly ordinary opening",
        "mbad",
        0,
    ),
    (
        106,
        Some("secret-bearing"),
        "an unlocatable secret passage",
        "mbad",
        1,
    ),
];

/// Message `mmix`'s text is its chunks' texts joined by
/// unattributed gap text ("LOG", the separators, "END") — the
/// shape the real corpus takes: chunk texts occur verbatim in
/// the message text, and not all message text belongs to a
/// chunk. Message `mbad` models a store inconsistency: chunk
/// 106's text does not occur in its text at all.
const MIX_MESSAGE: &str = "LOG the deploy window opens at dawn; \
    the vault code is 493812; remember to water the plants; \
    diary entry: the diagnosis results END";
const BAD_MESSAGE: &str = "a perfectly ordinary opening followed by completely different words";

struct SpanBackend;

fn span_detail(id: i64) -> ChunkDetail {
    let (_, label, text, message, idx) = SPAN_CORPUS
        .iter()
        .find(|(cid, _, _, _, _)| *cid == id)
        .unwrap_or_else(|| panic!("unknown chunk {id}"));
    ChunkDetail {
        chunk_id: id,
        text: text.to_string(),
        chunk_idx: *idx,
        message_id: message.to_string(),
        m_role: Some("user".to_string()),
        session_id: "smix".to_string(),
        s_title: Some("session".to_string()),
        s_client: None,
        s_created_at: None,
        l_kind: None,
        l_topics: None,
        l_projects: None,
        eff_sensitivity: label.map(str::to_string),
    }
}

impl SearchBackend for SpanBackend {
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
        Ok(BranchPools {
            fts: [101, 105]
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
        Ok(ids.iter().map(|&id| span_detail(id)).collect())
    }
    fn neighbors(&self, message_id: &str, _idx: i64, _n: i64) -> Result<Vec<NeighborRow>> {
        let ids: &[i64] = match message_id {
            "mmix" => &[102, 103, 104],
            "mbad" => &[106],
            _ => &[],
        };
        Ok(ids
            .iter()
            .map(|&id| {
                let d = span_detail(id);
                NeighborRow {
                    chunk_id: id,
                    chunk_idx: d.chunk_idx,
                    text: d.text,
                }
            })
            .collect())
    }
    fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        let text = match message_id {
            "mmix" => MIX_MESSAGE,
            "mbad" => BAD_MESSAGE,
            _ => return Ok(None),
        };
        Ok(Some(MessageRow {
            id: message_id.to_string(),
            role: Some("user".to_string()),
            text: text.to_string(),
        }))
    }
}

fn span_search(
    context: DisclosureContext,
    ceiling: &str,
    tweak: impl Fn(&mut SearchOptions),
) -> Json {
    let mut o = SearchOptions::new("deployment");
    o.mode = SearchMode::Fts;
    o.k = 10;
    o.context = context;
    tweak(&mut o);
    run_search(&SpanBackend, &FakeEmbedder, &filters(ceiling), &o).expect("search")
}

fn entry_for(out: &Json, chunk_id: i64) -> &Json {
    out["results"]
        .as_array()
        .expect("results array")
        .iter()
        .find(|r| r["chunk_id"] == chunk_id)
        .unwrap_or_else(|| panic!("no result entry for chunk {chunk_id}: {out}"))
}

#[test]
fn with_message_payload_omits_spans_above_the_intersection() {
    // Oracle: #12624 — the payload attached to the served
    // shareable chunk 101 contains only spans whose own chunks
    // pass fleet-internal ∩ the caller's grant; the served
    // chunk's span, the personal sibling's span, and
    // unattributed gap text still cross.
    let out = span_search(DisclosureContext::FleetInternal, "secret-bearing", |o| {
        o.with_message = true;
    });
    let entry = entry_for(&out, 101);
    assert_eq!(entry["text"], "the deploy window opens at dawn");
    let text = entry["message"]["text"]
        .as_str()
        .expect("payload served on a non-public surface");
    assert!(text.contains("the deploy window opens at dawn"), "{text}");
    assert!(text.contains("remember to water the plants"), "{text}");
    assert!(
        text.contains("LOG"),
        "gap text rides on the served chunk's gate: {text}"
    );
    assert!(
        !text.contains("493812"),
        "secret-bearing sibling span crossed: {text}"
    );
    assert!(
        !text.contains("diagnosis"),
        "excluded sibling span crossed: {text}"
    );
    // The withholding is on the record, not silent.
    assert!(
        out["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("message span(s)")),
        "notes record the span withholding: {out}"
    );
}

#[test]
fn excluded_sibling_span_never_crosses_even_for_the_owner() {
    // The owner's context admits the secret-bearing span, but
    // excluded_personal_life is ineligible in every context —
    // as a candidate (#12404) and as a payload span alike.
    let out = span_search(DisclosureContext::LocalOwner, "secret-bearing", |o| {
        o.with_message = true;
    });
    let text = entry_for(&out, 101)["message"]["text"]
        .as_str()
        .expect("payload served");
    assert!(
        text.contains("493812"),
        "owner receives the secret-bearing span: {text}"
    );
    assert!(
        !text.contains("diagnosis"),
        "excluded sibling span crossed to the owner: {text}"
    );
}

#[test]
fn hosted_payload_omits_spans_above_personal() {
    let out = span_search(DisclosureContext::HostedPerUser, "secret-bearing", |o| {
        o.with_message = true;
    });
    let text = entry_for(&out, 101)["message"]["text"]
        .as_str()
        .expect("payload served");
    assert!(text.contains("remember to water the plants"), "{text}");
    assert!(!text.contains("493812"), "secret span crossed: {text}");
    assert!(!text.contains("diagnosis"), "excluded span crossed: {text}");
}

#[test]
fn hydration_payload_omits_spans_above_the_intersection() {
    // Hydration is the uncapped full-content path; the span
    // rule is the same as --with-message's.
    let out = span_search(DisclosureContext::FleetInternal, "secret-bearing", |o| {
        o.parameters.hydrate = 1;
    });
    let h = &entry_for(&out, 101)["hydrated"];
    assert_eq!(h["chunk_text"], "the deploy window opens at dawn");
    let text = h["message"]["text"]
        .as_str()
        .expect("hydrated payload served");
    assert!(text.contains("remember to water the plants"), "{text}");
    assert!(
        !text.contains("493812"),
        "secret-bearing sibling span crossed: {text}"
    );
    assert!(
        !text.contains("diagnosis"),
        "excluded sibling span crossed: {text}"
    );
}

#[test]
fn unlocatable_sibling_span_withholds_the_whole_payload() {
    // Chunk 106 fails the intersection and its text cannot be
    // located in its message's text, so the payload cannot be
    // safely filtered: it is withheld whole (the served
    // chunk's own row still serves), and the notes say so.
    let out = span_search(DisclosureContext::FleetInternal, "secret-bearing", |o| {
        o.with_message = true;
    });
    let entry = entry_for(&out, 105);
    assert_eq!(entry["text"], "a perfectly ordinary opening");
    assert!(
        entry["message"].is_null(),
        "unfilterable payload was served: {entry}"
    );
    assert!(
        out["notes"].as_array().expect("notes").iter().any(|n| n
            .as_str()
            .unwrap_or("")
            .contains("could not be span-filtered")),
        "notes record the payload withholding: {out}"
    );
}

/// Records the ceiling pair a get-chunk occurrence lookup receives:
/// the tier fan-out ceiling and the full caller Sensitivity
/// ceiling. The oracle is the intersection rule (#12404): the
/// occurrence lookup must be gated by the SAME effective ceiling
/// as the chunk itself, not by the destination context's standalone
/// ceiling.
struct OccurrenceCeilingProbe {
    seen: std::sync::Mutex<Option<(atlas_core::shards::TierCeiling, Sensitivity)>>,
}

impl SearchBackend for OccurrenceCeilingProbe {
    fn name(&self) -> &'static str {
        "probe"
    }
    fn vec_table(&self) -> &str {
        "vec_chunks"
    }
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
    fn branch_pools(&self, _q: &BranchQuery, _f: &Filters) -> Result<BranchPools> {
        Ok(BranchPools::default())
    }
    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        Ok(ids.iter().map(|&id| detail(id)).collect())
    }
    fn neighbors(&self, _m: &str, _i: i64, _n: i64) -> Result<Vec<NeighborRow>> {
        Ok(Vec::new())
    }
    fn occurrences(
        &self,
        _canonical_hash: &str,
        tier_ceiling: atlas_core::shards::TierCeiling,
        caller_ceiling: Sensitivity,
    ) -> Result<Vec<atlas_core::occurrences::ChunkOccurrence>> {
        *self.seen.lock().unwrap() = Some((tier_ceiling, caller_ceiling));
        Ok(Vec::new())
    }
    fn message(&self, _m: &str) -> Result<Option<MessageRow>> {
        Ok(None)
    }
}

#[test]
fn get_chunk_occurrence_lookup_uses_the_effective_ceiling() {
    use atlas_core::shards::{Tier, TierCeiling};

    // A personal grant under fleet-internal (ceiling local-only):
    // the intersection is personal. The occurrence lookup must ask
    // for the INTERNAL tier (personal maps there) and the personal
    // row ceiling — not the secret tier the bare LocalOwner context
    // used to request (P1: a personal-capped caller on a
    // secret-tier service received secret-tier source locators, or,
    // on a personal-only service, every get-chunk failed).
    let probe = OccurrenceCeilingProbe {
        seen: std::sync::Mutex::new(None),
    };
    run_get_chunk_with_context(
        &probe,
        2,
        0,
        DisclosureContext::FleetInternal,
        Sensitivity::Personal,
    )
    .expect("personal chunk under fleet-internal");
    assert_eq!(
        *probe.seen.lock().unwrap(),
        Some((TierCeiling(Tier::Internal), Sensitivity::Personal)),
        "occurrence lookup gated by grant ∩ context (personal ∩ local-only = personal)"
    );

    // A secret grant under public-surface: the intersection is
    // shareable, so the lookup asks for the PUBLIC tier and the
    // shareable row ceiling.
    let probe = OccurrenceCeilingProbe {
        seen: std::sync::Mutex::new(None),
    };
    run_get_chunk_with_context(
        &probe,
        1,
        0,
        DisclosureContext::PublicSurface,
        Sensitivity::SecretBearing,
    )
    .expect("shareable chunk on a public surface");
    assert_eq!(
        *probe.seen.lock().unwrap(),
        Some((TierCeiling(Tier::Public), Sensitivity::Shareable)),
        "occurrence lookup gated by grant ∩ context (secret ∩ shareable = shareable)"
    );
}
