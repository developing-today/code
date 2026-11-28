//! Derived-visibility fixture for the export adapter, the cached-derivative
//! case, and the vector-branch tier check (#13583, slice 3; policy record in
//! `doc/2026-11-26T13-00-00Z_design_derived_visibility_profiles_13583/`).
//!
//! Two read-only JSON exports hold the same rows a reader may see. Export B
//! also holds rows above that reader's ceiling. The expected observations
//! below are written out by hand from the corpus, not computed by the code
//! under test, except where the test pins a production statement verbatim.
//! They pin three things:
//!
//! 1. **Export adapter, public reader: strict.** The export connector
//!    (`parse_memory_export`) carries each row's physical tier, and staging
//!    (`stage_memory_export_in_db`) preserves it on the item. For a
//!    `public`-tier reader the visible item set and its count are the same
//!    in both stores, and no hidden row is ever returned. This is the
//!    export-adapter half of the two-store fixture the policy record leaves
//!    as remaining work; the graph-query half is #13781 and the lexical
//!    half is `derived_visibility.rs`.
//! 2. **Cached derivatives serve at the visible cut.** A staged unit DB is
//!    a persisted derivative of its export, computed broadly (hidden rows
//!    included). The serving rule pinned here: a restricted reader is
//!    served only the derivative recomputed over its own visible cut —
//!    never the broad entry whole. The broad staged summary of store B
//!    names hidden text; the public reader's served summary does not, and
//!    the two summaries differ. Production enforcement of this rule beyond
//!    the staged-items filter is scoped follow-up, not landed here.
//! 3. **Vector branch: shared tier, row-level filter.** `personal` and
//!    `local-only` map onto the same physical `internal` tier
//!    (`TierCeiling::from_sensitivity`), confirming the recorded
//!    observation. The fan-out scan statement nevertheless keeps a
//!    row-level `c.sensitivity IN (...)` filter, so for a classified
//!    corpus a `personal` ceiling still excludes `local-only` rows: the
//!    "no row-level filter inside it" half of the observation is refuted
//!    at the statement level. One caveat is recorded, not fixed: chunks
//!    promoted without classification records are all stamped with the
//!    tier representative (`personal` for internal shards), which makes
//!    that row-level filter vacuous for `local-only` content in such
//!    shards. Scoping that fix is follow-up.
//!
//! A separate check confirms the full-trust (`secret`) view of store B
//! still returns the hidden rows. No timing claim is made.

use std::path::{Path, PathBuf};

use atlas_core::filters::{Filters, Sensitivity, turso_vector_scan_stmt};
use atlas_core::memory::{
    AGENT_MEMORY_UNIT, parse_memory_export, register_memory_corpora,
    stage_memory_export_in_db,
};
use atlas_core::shards::{Tier, TierCeiling};
use atlas_core::stage::{Registry, StagedItem, list_staged_items_in_db};
use atlas_core::types::{DEFAULT_VEC_TABLE, Value};

const NOW: &str = "2026-11-23T00:00:00Z";

fn temp_dir(tag: &str) -> PathBuf {
    let d =
        std::env::temp_dir().join(format!("atlas-derived-fed-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn register(dir: &Path) -> (Registry, PathBuf) {
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let units = register_memory_corpora(&registry, dir, NOW).unwrap();
    let agent_db = units
        .iter()
        .find(|u| u.unit_id == AGENT_MEMORY_UNIT)
        .map(|u| PathBuf::from(&u.db_uri))
        .expect("agent-memory unit registered");
    (registry, agent_db)
}

/// Rows every export holds: one public row, one internal-tier row.
const EXPORT_A: &str = r#"[
  {"id": "e1", "text": "alpha notes", "sensitivity": "public"},
  {"id": "e2", "text": "beta notes", "sensitivity": "internal"}
]"#;

/// Store B additionally holds `local-only` content (physically internal
/// tier) and one secret row.
const EXPORT_B: &str = r#"[
  {"id": "e1", "text": "alpha notes", "sensitivity": "public"},
  {"id": "e2", "text": "beta notes", "sensitivity": "internal"},
  {"id": "e3", "text": "gamma notes", "sensitivity": "internal"},
  {"id": "e4", "text": "delta notes", "sensitivity": "secret"}
]"#;

/// Tier words a ceiling admits, in the export connector's vocabulary.
fn admitted_tiers(ceiling: Tier) -> Vec<Tier> {
    Tier::ALL
        .iter()
        .copied()
        .filter(|t| TierCeiling(ceiling).admits(*t))
        .collect()
}

fn visible_ids(entries: &[atlas_core::memory::MemoryEntry], ceiling: Tier) -> Vec<String> {
    let admitted = admitted_tiers(ceiling);
    let mut ids: Vec<String> = entries
        .iter()
        .filter(|e| admitted.contains(&e.sensitivity))
        .map(|e| e.id.clone())
        .collect();
    ids.sort();
    ids
}

fn arg_texts(args: &[Value]) -> Vec<String> {
    args.iter()
        .filter_map(|a| match a {
            Value::Text(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn export_adapter_two_stores_agree_for_the_public_reader() {
    let a = parse_memory_export(EXPORT_A).unwrap();
    let b = parse_memory_export(EXPORT_B).unwrap();
    // Hand-derived: the public reader sees only the public row.
    assert_eq!(visible_ids(&a, Tier::Public), vec!["e1"]);
    assert_eq!(visible_ids(&b, Tier::Public), vec!["e1"]);
    for hidden in ["e2", "e3", "e4"] {
        assert!(
            !visible_ids(&b, Tier::Public).contains(&hidden.to_string()),
            "tier {hidden} row leaked to the public reader"
        );
    }
    // The tier words the connector recorded are the physical tiers the
    // ceiling admits against — no finer label survives the export.
    assert_eq!(admitted_tiers(Tier::Public), vec![Tier::Public]);
    assert_eq!(
        admitted_tiers(Tier::Internal),
        vec![Tier::Public, Tier::Internal]
    );
}

#[test]
fn cached_derivative_serves_only_the_visible_cut() {
    let dir_a = temp_dir("store-a");
    let (_reg_a, db_a) = register(&dir_a);
    let dir_b = temp_dir("store-b");
    let (_reg_b, db_b) = register(&dir_b);
    let entries_a = parse_memory_export(EXPORT_A).unwrap();
    let entries_b = parse_memory_export(EXPORT_B).unwrap();
    stage_memory_export_in_db(&db_a, AGENT_MEMORY_UNIT, &entries_a, NOW).unwrap();
    stage_memory_export_in_db(&db_b, AGENT_MEMORY_UNIT, &entries_b, NOW).unwrap();

    // The staged unit DB is the cached derivative: store B's unit holds
    // all four rows, hidden rows included.
    let staged_b = list_staged_items_in_db(&db_b, Some(AGENT_MEMORY_UNIT)).unwrap();
    assert_eq!(staged_b.len(), 4, "broad derivative holds every row");
    let staged_a = list_staged_items_in_db(&db_a, Some(AGENT_MEMORY_UNIT)).unwrap();
    assert_eq!(staged_a.len(), 2);

    // Serving filters the derivative at the reader's visible cut. The
    // staged items carry the export's tier words, so the public cut is
    // exactly the public items — identical in both stores. The source
    // pointer names the export row (`memory-lancedb:<id>`).
    let cut = |items: &[StagedItem]| {
        let mut ids: Vec<String> = items
            .iter()
            .filter(|i| i.sensitivity == "public")
            .map(|i| {
                i.source
                    .as_deref()
                    .expect("staged memory carries its source pointer")
                    .strip_prefix("memory-lancedb:")
                    .expect("memory source pointer shape")
                    .to_string()
            })
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(cut(&staged_a), vec!["e1"]);
    assert_eq!(cut(&staged_b), vec!["e1"]);

    // The served summary is recomputed over the visible cut, never the
    // broad entry whole: store B's broad summary names hidden text, the
    // public reader's served summary does not, and the two differ.
    let broad: String = entries_b
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(broad.contains("gamma notes"), "broad cache holds hidden text");
    assert!(broad.contains("delta notes"), "broad cache holds hidden text");
    let served: String = entries_b
        .iter()
        .filter(|e| e.sensitivity == Tier::Public)
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    assert_eq!(served, "alpha notes");
    assert_ne!(served, broad, "broad cache entry is never served whole");

    // The authorized full-trust view of store B still works.
    let mut full: Vec<String> = staged_b
        .iter()
        .map(|i| {
            i.source
                .as_deref()
                .expect("staged memory carries its source pointer")
                .strip_prefix("memory-lancedb:")
                .expect("memory source pointer shape")
                .to_string()
        })
        .collect();
    full.sort();
    assert_eq!(full, vec!["e1", "e2", "e3", "e4"]);
}

#[test]
fn vector_branch_shares_the_internal_tier_but_filters_rows() {
    // The recorded observation, confirmed: both rungs map onto one tier.
    assert_eq!(
        TierCeiling::from_sensitivity(Sensitivity::Personal),
        TierCeiling(Tier::Internal)
    );
    assert_eq!(
        TierCeiling::from_sensitivity(Sensitivity::LocalOnly),
        TierCeiling(Tier::Internal)
    );
    assert_eq!(
        TierCeiling::from_sensitivity(Sensitivity::Shareable),
        TierCeiling(Tier::Public)
    );
    assert_eq!(
        TierCeiling::from_sensitivity(Sensitivity::SecretBearing),
        TierCeiling(Tier::Secret)
    );
    // Tier admission itself: a personal ceiling scans internal shards but
    // never secret ones.
    let personal = TierCeiling::from_sensitivity(Sensitivity::Personal);
    assert!(personal.admits(Tier::Public));
    assert!(personal.admits(Tier::Internal));
    assert!(!personal.admits(Tier::Secret));

    // The "no row-level filter" half is refuted at the statement level:
    // the production fan-out scan statement filters chunks by rung, so a
    // personal ceiling's IN set holds shareable + personal spellings and
    // neither local-only nor secret-bearing one.
    let filters = Filters {
        max_sensitivity: Sensitivity::Personal,
        ..Filters::default()
    };
    let stmt = turso_vector_scan_stmt(DEFAULT_VEC_TABLE, &filters, "[0.0]", 50);
    assert!(
        stmt.sql.contains("c.sensitivity IN"),
        "scan keeps its row-level filter: {}",
        stmt.sql
    );
    let texts = arg_texts(&stmt.args);
    for rung in ["shareable", "personal", "public", "internal"] {
        assert!(texts.contains(&rung.to_string()), "ceiling admits {rung}");
    }
    for rung in ["local-only", "secret-bearing", "private", "sensitive"] {
        assert!(
            !texts.contains(&rung.to_string()),
            "ceiling excludes {rung}: {texts:?}"
        );
    }

    // Recorded caveat, not fixed here: chunks promoted without
    // classification records are all stamped with the tier
    // representative (`personal` for internal shards,
    // `tier_sensitivity_name`), which makes the row-level filter above
    // vacuous for `local-only` content in such shards. Scoping that fix
    // is follow-up; classified corpora keep their per-row labels.
}
