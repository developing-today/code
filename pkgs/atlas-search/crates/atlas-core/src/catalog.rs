//! Hot-side cold catalog — the §5 fan-out contract's index and the
//! §6 step-1 resolution for the #6042 cold-tier shard format.
//!
//! Design: `doc/2026-11-10T20-06-24Z_design_atlas_cold_tier_shard_format/`
//! (§5 cold catalog + fan-out contract, §6 restore procedure).
//! Issue #8229 (follow-on 3 of #6042).
//!
//! Cold shards sit **outside the default fan-out set**. Default and
//! `version = latest` queries never open a cold shard, and this
//! module is the reason that property holds structurally rather
//! than by convention: the catalog is the **only** index the query
//! path consults about cold content, and it answers exactly two
//! questions —
//!
//! - **Explicit addressing** (#6026 §5.3): a pinned version
//!   (`version = <version_seq>`) or an `as_of` resolution that
//!   lands on a cold version. [`ColdCatalog::resolve`] maps
//!   `(file_id, version_seq)` to its shard; the restore path (§6)
//!   fetches that object and serves it.
//! - **Shard-addressed search**: a query that explicitly names
//!   `tier: cold` (optionally `shard: <shard_id>`).
//!   [`ColdCatalog::search_scope`] turns that address into the
//!   object keys to fan out to, using the manifest tables in place.
//!
//! There is no third way: [`ColdCatalog::lookup`] states whether a
//! version is cold or not, so a cold version's absence from default
//! results is a stated property (the catalog knows the version is
//! cold), never a silent miss. The default fan-out path never calls
//! into this module at all — there is no scan API that would let it.
//!
//! Layer boundaries (owned elsewhere, not invented here):
//!
//! - The manifest shape is the landed [`ColdManifest`](crate::cold::ColdManifest)
//!   reader (#8228 slice 1); this module consumes it, never redefines it.
//! - Fetching a shard object and verifying it against its footer is
//!   [`ColdShard::open`](crate::cold::ColdShard::open) (#8228); the
//!   `manifest_sha256` recorded here is the footer-verified digest
//!   the fetch path hands over, and registration binds the row to it.
//! - Packing shards is the writer follow-on (#8227); fanning out
//!   over addressed shards is the fan-out consumer wiring (#5755);
//!   the parameter surface is #5774. This module wires into none of
//!   them — it is the index they will share.

use std::collections::HashMap;

use crate::cold::ColdManifest;
use crate::error::{Error, Result};

fn catalog_error(msg: impl Into<String>) -> Error {
    Error::fatal(format!("cold catalog: {}", msg.into()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One version identity: the `(file_id, version_seq)` pair the §5
/// contract resolves. `file_id` arrives already normalised by the
/// [`ColdManifest`](crate::cold::ColdManifest) parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionKey {
    pub file_id: String,
    pub version_seq: u64,
}

/// One row per canonical shard (§5): the shard's identity, where its
/// object lives, the footer-verified manifest digest that binds the
/// row to the manifest it describes, the corpus and class the shard
/// was packed under, and every `(file_id, version_seq)` it holds.
#[derive(Debug, Clone)]
pub struct ColdCatalogEntry {
    shard_id: String,
    object_key: String,
    manifest_sha256: [u8; 32],
    corpus: String,
    class: String,
    versions: Vec<VersionKey>,
}

impl ColdCatalogEntry {
    pub fn shard_id(&self) -> &str {
        &self.shard_id
    }

    pub fn object_key(&self) -> &str {
        &self.object_key
    }

    pub fn manifest_sha256(&self) -> &[u8; 32] {
        &self.manifest_sha256
    }

    pub fn corpus(&self) -> &str {
        &self.corpus
    }

    pub fn class(&self) -> &str {
        &self.class
    }

    /// Every version the shard holds, sorted by `(file_id, version_seq)`.
    pub fn versions(&self) -> &[VersionKey] {
        &self.versions
    }

    /// The merged contiguous `(start, end)` `version_seq` ranges the
    /// shard holds for one file — the §5 row's range half, derived
    /// from the version half so the two cannot disagree.
    pub fn ranges_for(&self, file_id: &str) -> Vec<(u64, u64)> {
        let mut seqs: Vec<u64> = self
            .versions
            .iter()
            .filter(|v| v.file_id == file_id)
            .map(|v| v.version_seq)
            .collect();
        seqs.sort_unstable();
        seqs.dedup();
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        for seq in seqs {
            match ranges.last_mut() {
                // Contiguous (or the same seq twice — deduped above,
                // so this arm is exactly seq == end + 1) extends.
                Some(last) if seq == last.1 + 1 => last.1 = seq,
                _ => ranges.push((seq, seq)),
            }
        }
        ranges
    }

    fn address(&self) -> ColdAddress {
        ColdAddress {
            shard_id: self.shard_id.clone(),
            object_key: self.object_key.clone(),
            manifest_sha256: self.manifest_sha256,
        }
    }
}

/// Where a pinned cold version lives: the shard to resolve it to
/// (§6 step 1) and the manifest digest the fetch path must verify
/// the fetched object against before serving anything from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdAddress {
    pub shard_id: String,
    pub object_key: String,
    pub manifest_sha256: [u8; 32],
}

/// The response path's stated property (§5): a version the catalog
/// holds is cold — serve it through the restore path, never through
/// default results. A version the catalog does not hold is not cold
/// per this catalog: the hot tier owns the answer (hot, or missing
/// on hot terms), and no cold shard is ever opened for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColdLookup {
    Cold(ColdAddress),
    NotCold,
}

/// The hot tier's index over canonical cold shards: one row per
/// shard, and the only thing the query path consults about cold
/// content.
#[derive(Debug, Default, Clone)]
pub struct ColdCatalog {
    entries: Vec<ColdCatalogEntry>,
    by_version: HashMap<(String, u64), usize>,
}

impl ColdCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The shard ids the catalog holds, in registration order.
    pub fn shard_ids(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.shard_id.as_str()).collect()
    }

    /// Record one canonical shard (§5 row): its object key, the
    /// footer-verified manifest digest, and the manifest the digest
    /// binds the row to.
    ///
    /// Shards are never edited in place (§2): re-registering a
    /// `shard_id` with a different object key or digest is a hard
    /// error naming the shard. Re-registering the identical row is
    /// a no-op success (restore is idempotent in the same spirit,
    /// §6). A shard that holds no versions, or whose manifest is not
    /// literally `cold`, is rejected — the catalog indexes cold
    /// shards, nothing else.
    pub fn register(
        &mut self,
        object_key: impl Into<String>,
        manifest_sha256: [u8; 32],
        manifest: &ColdManifest,
    ) -> Result<()> {
        let object_key = object_key.into();
        if manifest.tier != "cold" {
            return Err(catalog_error(format!(
                "shard {} has tier {:?}, not the literal \"cold\"; \
                 the cold catalog indexes cold shards only",
                manifest.shard_id, manifest.tier
            )));
        }
        if manifest.versions.is_empty() {
            return Err(catalog_error(format!(
                "shard {} holds no versions; a catalog row with nothing to resolve is rejected",
                manifest.shard_id
            )));
        }
        if let Some(existing) = self
            .entries
            .iter()
            .find(|e| e.shard_id == manifest.shard_id)
        {
            if existing.object_key == object_key && existing.manifest_sha256 == manifest_sha256 {
                return Ok(());
            }
            return Err(catalog_error(format!(
                "shard {} is already catalogued at {:?} with manifest sha {}; \
                 shards are never edited in place (§2) — a changed shard packs under a new shard_id",
                manifest.shard_id,
                existing.object_key,
                hex(&existing.manifest_sha256)
            )));
        }
        let mut versions: Vec<VersionKey> = manifest
            .versions
            .iter()
            .map(|v| VersionKey {
                file_id: v.file_id.clone(),
                version_seq: v.version_seq,
            })
            .collect();
        versions.sort();
        versions.dedup();
        let at = self.entries.len();
        for v in &versions {
            self.by_version
                .insert((v.file_id.clone(), v.version_seq), at);
        }
        self.entries.push(ColdCatalogEntry {
            shard_id: manifest.shard_id.clone(),
            object_key,
            manifest_sha256,
            corpus: manifest.corpus.clone(),
            class: manifest.retention.corpus_class.clone(),
            versions,
        });
        Ok(())
    }

    /// Explicit version reach (§5, §6 step 1): a pinned version
    /// resolves through the catalog to its shard. `None` means the
    /// catalog holds no such version — the version is not cold per
    /// this catalog, and no cold shard is opened for it.
    pub fn resolve(&self, file_id: &str, version_seq: u64) -> Option<ColdAddress> {
        self.by_version
            .get(&(file_id.to_string(), version_seq))
            .map(|at| self.entries[*at].address())
    }

    /// The response path's stated property (§5): cold, with the
    /// address to serve it through, or not cold — never a silent
    /// miss in either direction.
    pub fn lookup(&self, file_id: &str, version_seq: u64) -> ColdLookup {
        match self.resolve(file_id, version_seq) {
            Some(address) => ColdLookup::Cold(address),
            None => ColdLookup::NotCold,
        }
    }

    pub fn is_cold(&self, file_id: &str, version_seq: u64) -> bool {
        matches!(self.lookup(file_id, version_seq), ColdLookup::Cold(_))
    }

    /// Shard-addressed search scoping (§5): a query that explicitly
    /// names `tier: cold` fans out to the catalog's shards —
    /// optionally narrowed to one `shard:` — as read-only search
    /// targets, using the manifest tables in place.
    ///
    /// Any other tier is a hard error: cold shards sit outside the
    /// default fan-out set, and default queries never reach this
    /// function — the catalog offers no scan that would let them.
    /// Naming an uncatalogued shard is a hard error naming it: an
    /// address that resolves nowhere must fail loudly, never fan
    /// out to nothing and report an empty set as a result.
    pub fn search_scope(&self, tier: &str, shard: Option<&str>) -> Result<Vec<ColdAddress>> {
        if tier != "cold" {
            return Err(catalog_error(format!(
                "tier {tier:?} does not consult the cold catalog; \
                 default and latest queries never open a cold shard (§5)"
            )));
        }
        match shard {
            Some(want) => self
                .entries
                .iter()
                .find(|e| e.shard_id == want)
                .map(|e| vec![e.address()])
                .ok_or_else(|| {
                    catalog_error(format!(
                        "shard-addressed search names uncatalogued shard {want:?}; \
                         the address resolves nowhere"
                    ))
                }),
            None => Ok(self.entries.iter().map(|e| e.address()).collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cold::ColdVersion;
    use serde_json::{Value as Json, json};

    /// A §3 manifest off the landed reader types: the design is the
    /// oracle, [`ColdManifest`]'s parse is the ground truth — the
    /// catalog consumes manifests, it never rebuilds their shape.
    fn manifest(
        shard_id: &str,
        corpus: &str,
        class: &str,
        versions: &[(&str, u64)],
    ) -> ColdManifest {
        let versions: Vec<Json> = versions
            .iter()
            .map(|(file_id, seq)| {
                json!({
                    "file_id": file_id,
                    "version_seq": seq,
                    "path_at_version": "a.txt",
                    "content_hash": format!("hash-{file_id}-{seq}"),
                    "ingested_at": "2026-11-01T00:00:00Z",
                    "occurrences": []
                })
            })
            .collect();
        let doc = json!({
            "format_version": 1,
            "shard_id": shard_id,
            "corpus": corpus,
            "tier": "cold",
            "created_at": "2026-11-24T00:00:00Z",
            "created_by": "atlas-test",
            "versions": versions,
            "span_table": [],
            "vector_model": "embed-v1",
            "vector_dimensions": 2,
            "vector_table": [],
            "retention": {
                "class": class,
                "state": "cold",
                "cold_since": "2026-11-20T00:00:00Z",
                "retire_not_before": "2027-02-20T00:00:00Z"
            },
            "provenance": {"hot_shard_id": "hot-1", "export_run_id": "run-1"}
        });
        serde_json::from_value(doc).expect("fixture manifest parses as a §3 document")
    }

    fn sha(tag: &str) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        Sha256::digest(tag.as_bytes()).into()
    }

    fn two_shard_catalog() -> ColdCatalog {
        let mut catalog = ColdCatalog::new();
        catalog
            .register(
                "rustfs://cold/shard-a",
                sha("manifest-a"),
                &manifest("shard-a", "code", "code", &[("file-1", 1), ("file-1", 2)]),
            )
            .expect("shard-a registers");
        catalog
            .register(
                "rustfs://cold/shard-b",
                sha("manifest-b"),
                &manifest("shard-b", "docs", "docs", &[("file-2", 1)]),
            )
            .expect("shard-b registers");
        catalog
    }

    #[test]
    fn pinned_cold_version_resolves_to_its_shard() {
        let catalog = two_shard_catalog();
        // Explicit addressing (§5): the pin lands on a cold version,
        // the catalog names the shard — §6 step 1.
        let a = catalog.resolve("file-1", 2).expect("v2 of file-1 is cold");
        assert_eq!(a.shard_id, "shard-a");
        assert_eq!(a.object_key, "rustfs://cold/shard-a");
        assert_eq!(a.manifest_sha256, sha("manifest-a"));
        let b = catalog.resolve("file-2", 1).expect("v1 of file-2 is cold");
        assert_eq!(b.shard_id, "shard-b");
        // The row binds the manifest it describes: corpus, class,
        // and the version ranges the shard holds (§5 row).
        assert_eq!(catalog.shard_ids(), vec!["shard-a", "shard-b"]);
    }

    #[test]
    fn versions_outside_the_catalog_are_not_cold_never_a_silent_miss() {
        let catalog = two_shard_catalog();
        // file-1 v3 was never packed cold: the catalog states NotCold,
        // so the response path serves hot terms — never a silent miss
        // in either direction (§5: stated, never silent).
        assert_eq!(catalog.lookup("file-1", 3), ColdLookup::NotCold);
        assert_eq!(catalog.lookup("file-9", 1), ColdLookup::NotCold);
        assert!(!catalog.is_cold("file-1", 3));
        assert!(catalog.is_cold("file-1", 1));
        assert!(catalog.resolve("file-1", 3).is_none());
    }

    #[test]
    fn reregistering_the_identical_row_is_a_noop() {
        let mut catalog = two_shard_catalog();
        catalog
            .register(
                "rustfs://cold/shard-a",
                sha("manifest-a"),
                &manifest("shard-a", "code", "code", &[("file-1", 1), ("file-1", 2)]),
            )
            .expect("identical re-registration is a no-op success");
        assert_eq!(catalog.len(), 2);
        assert_eq!(
            catalog
                .resolve("file-1", 1)
                .expect("still resolves")
                .shard_id,
            "shard-a"
        );
    }

    #[test]
    fn reregistering_a_changed_row_is_rejected_shards_are_immutable() {
        let mut catalog = two_shard_catalog();
        // Same shard_id, different manifest digest: shards are never
        // edited in place (§2) — a changed shard packs under a new id.
        let err = catalog
            .register(
                "rustfs://cold/shard-a",
                sha("manifest-a-CHANGED"),
                &manifest("shard-a", "code", "code", &[("file-1", 1), ("file-1", 2)]),
            )
            .expect_err("changed re-registration must fail");
        assert!(
            err.to_string().contains("shard-a"),
            "error names the shard: {err}"
        );
        // Same shard_id moved to a different object: also rejected.
        let err = catalog
            .register(
                "rustfs://cold/shard-a-elsewhere",
                sha("manifest-a"),
                &manifest("shard-a", "code", "code", &[("file-1", 1), ("file-1", 2)]),
            )
            .expect_err("moved re-registration must fail");
        assert!(
            err.to_string().contains("shard-a"),
            "error names the shard: {err}"
        );
        // The catalog is unchanged by both attempts.
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn non_cold_and_empty_manifests_are_rejected() {
        let mut catalog = ColdCatalog::new();
        let mut doc = json!({
            "format_version": 1,
            "shard_id": "shard-hot",
            "corpus": "code",
            "tier": "hot",
            "created_at": "2026-11-24T00:00:00Z",
            "created_by": "atlas-test",
            "versions": [{
                "file_id": "file-1", "version_seq": 1,
                "path_at_version": "a.txt", "content_hash": "h",
                "ingested_at": "2026-11-01T00:00:00Z", "occurrences": []
            }],
            "span_table": [],
            "vector_model": "embed-v1",
            "vector_dimensions": 2,
            "vector_table": [],
            "retention": {
                "class": "code", "state": "hot",
                "cold_since": "2026-11-20T00:00:00Z",
                "retire_not_before": "2027-02-20T00:00:00Z"
            },
            "provenance": {"hot_shard_id": "hot-1", "export_run_id": "run-1"}
        });
        let hot: ColdManifest =
            serde_json::from_value(doc.clone()).expect("hot-tier manifest parses");
        let err = catalog
            .register("rustfs://cold/shard-hot", sha("x"), &hot)
            .expect_err("a hot manifest is not cold-catalogueable");
        assert!(err.to_string().contains("cold"), "error: {err}");
        doc["tier"] = json!("cold");
        doc["versions"] = json!([]);
        let empty: ColdManifest = serde_json::from_value(doc).expect("empty manifest parses");
        let err = catalog
            .register("rustfs://cold/shard-empty", sha("y"), &empty)
            .expect_err("a versionless shard resolves nothing");
        assert!(err.to_string().contains("no versions"), "error: {err}");
        assert!(catalog.is_empty());
    }

    #[test]
    fn search_scope_addresses_cold_shards_and_refuses_default_tiers() {
        let catalog = two_shard_catalog();
        // tier: cold with no shard addresses every cold shard.
        let all = catalog
            .search_scope("cold", None)
            .expect("cold tier scopes");
        assert_eq!(all.len(), 2);
        // shard: narrows to the one named shard.
        let one = catalog
            .search_scope("cold", Some("shard-b"))
            .expect("named shard scopes");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].object_key, "rustfs://cold/shard-b");
        // An address that resolves nowhere fails loudly — never an
        // empty fan-out reported as a result.
        let err = catalog
            .search_scope("cold", Some("shard-missing"))
            .expect_err("unknown shard must fail");
        assert!(err.to_string().contains("shard-missing"), "error: {err}");
        // Default tiers never open a cold shard (§5): the catalog
        // refuses them outright — there is no scan API for default
        // fan-out to misuse.
        for tier in ["hot", "latest", "default", ""] {
            let err = catalog
                .search_scope(tier, None)
                .expect_err("non-cold tier must not consult the catalog");
            assert!(
                err.to_string().contains(tier),
                "error names the tier: {err}"
            );
        }
    }

    #[test]
    fn entry_ranges_merge_contiguous_seqs_per_file() {
        let entry = ColdCatalogEntry {
            shard_id: "s".to_string(),
            object_key: "k".to_string(),
            manifest_sha256: sha("m"),
            corpus: "code".to_string(),
            class: "code".to_string(),
            versions: ["f", "f", "f", "g"]
                .iter()
                .zip([1u64, 2, 4, 1])
                .map(|(f, s)| VersionKey {
                    file_id: f.to_string(),
                    version_seq: s,
                })
                .collect(),
        };
        // 1,2 merge; 4 stands alone; the other file is separate.
        assert_eq!(entry.ranges_for("f"), vec![(1, 2), (4, 4)]);
        assert_eq!(entry.ranges_for("g"), vec![(1, 1)]);
        assert!(entry.ranges_for("missing").is_empty());
    }

    #[test]
    fn registered_row_carries_corpus_class_and_sorted_versions() {
        let catalog = two_shard_catalog();
        // Reach in through resolve + ranges: the public surface the
        // restore path (#8228) and the fan-out wiring (#5755) share.
        let at = catalog
            .by_version
            .get(&("file-1".to_string(), 1))
            .expect("indexed");
        let entry = &catalog.entries[*at];
        assert_eq!(entry.corpus(), "code");
        assert_eq!(entry.class(), "code");
        assert_eq!(
            entry.versions(),
            &[
                VersionKey {
                    file_id: "file-1".to_string(),
                    version_seq: 1
                },
                VersionKey {
                    file_id: "file-1".to_string(),
                    version_seq: 2
                },
            ]
        );
        assert_eq!(entry.ranges_for("file-1"), vec![(1, 2)]);
        let _: &ColdVersion = &manifest("m", "code", "code", &[("f", 1)]).versions[0];
    }
}
