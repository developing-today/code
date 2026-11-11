//! Shared compiled-world cache — P1 of the #5965 compile-economics design
//! (`doc/2026-11-05_design_id5965.md` §2, §4 tiers 0–1, §7 P1; issue #6235).
//!
//! Until now the only cache lived inside each compile job's scratch
//! directory (tier 0) and died with the job. This module is the shared,
//! server-side tier 1: a content-addressed store keyed by everything
//! that determines the output bytes, so a second compile of identical
//! inputs is served without invoking the compiler at all.
//!
//! The key (§2.1) is BLAKE3 over the platform tree hash, the source
//! closure hash, the DLL closure hash, the compiler binary digest
//! (`compiler_blake3`, as already computed by the compile path) and the
//! target. The seed is *not* a key component — it parameterises `init`
//! at install time, not the compiled bytes — and neither are the
//! compile limits, which bound a job without changing its output.
//!
//! Sharing scopes (§2.3): an entry is stored at `author` scope (the
//! default for unpublished source) or `server` scope. `public` is
//! stubbed at `server` behaviour until P3, as the P1 issue allows.
//! Cross-author reuse of a `server`/`public` entry is gated on the
//! determinism proof of §2.2: a proof is on record for a
//! (compiler, platform) pair only after the same spec compiled twice
//! in clean job directories yielded byte-identical artifacts. Without
//! a proof, that pair's entries are visible to their author only.
//!
//! A hit is never trusted on its label: the stored artifact's BLAKE3
//! is re-verified against the content hash in the entry metadata
//! before the bytes are served, and a mismatch evicts the entry.
//!
//! Eviction (§4 tier 1) is LRU over `lru_bytes` (default 2 GiB),
//! ordered by last hit (falling back to store time). Invalidation
//! beats every tier: [`WorldCache::invalidate_compiler`] and
//! [`WorldCache::invalidate_platform`] hard-evict matching entries
//! and drop any determinism proof that names the revoked component.
//! Tier 2 (pinning) is P3 and not implemented here.
//!
//! Metering is logged only (§7 P1): callers record the measured
//! compiler cost of a miss (0 on a hit) through the `tracing` log;
//! no charging, delay, or gating happens in this phase.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

/// Default tier-1 budget: `cache.lru_bytes` in design §4 (2 GiB).
pub const DEFAULT_LRU_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Who may reuse a cache entry (design §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheScope {
    /// Only the author key that caused the compile.
    Author,
    /// Any author on this server (determinism-gated, §2.2).
    Server,
    /// Anyone who can name the key. Stubbed at `Server` behaviour in
    /// P1, as the #6235 scope allows.
    Public,
}

/// The components that determine a compiled artifact's bytes (§2.1).
/// Every component is the lowercase hex BLAKE3 digest the compile path
/// already computes, except `target`, which is the target name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheKeyInput {
    /// Hash of the platform tree (platform.roc, platform modules,
    /// `targets/*/host.*`).
    pub platform_tree_hash: String,
    /// Hash of the ordered (file name, file bytes) source closure.
    pub source_closure_hash: String,
    /// Hash of the sorted content hashes of the DLL artifacts the
    /// compile links; the empty closure hashes to a constant.
    pub dll_closure_hash: String,
    /// Digest of the `roc` binary that would run the compile.
    pub compiler_blake3: String,
    /// Compile target: `wasm32` or `x64musl`.
    pub target: String,
}

/// The DLL closure component of the key (§2.1, §3): BLAKE3 over the
/// sorted content hashes of the linked DLL artifacts. No DLL linking
/// exists yet, so callers pass an empty slice and get the constant
/// empty-closure hash the #6235 scope reserves.
#[must_use]
pub fn dll_closure_hash(artifact_hashes: &[String]) -> String {
    let mut sorted: Vec<&str> = artifact_hashes.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"id-dll-closure-v1\0");
    for hash in sorted {
        let bytes = hash.as_bytes();
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher.finalize().to_hex().to_string()
}

/// Derive the cache key from its components (§2.1). The seed and the
/// compile limits are deliberately not inputs.
#[must_use]
pub fn cache_key(input: &CacheKeyInput) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"id-compiled-world-cache-key-v1\0");
    for component in [
        input.platform_tree_hash.as_str(),
        input.source_closure_hash.as_str(),
        input.dll_closure_hash.as_str(),
        input.compiler_blake3.as_str(),
        input.target.as_str(),
    ] {
        let bytes = component.as_bytes();
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher.finalize().to_hex().to_string()
}

/// Content hash of an artifact: the name its bytes are stored under,
/// re-verified on every hit before the bytes are served (§2.1).
#[must_use]
pub fn artifact_hash(artifact: &[u8]) -> String {
    blake3::hash(artifact).to_hex().to_string()
}

/// One stored entry: the metadata half of the cache. The artifact
/// bytes live separately under their own content hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    /// The derived cache key (also the metadata file's name).
    pub key: String,
    /// The key components, kept so invalidation can match on the
    /// compiler digest or the platform tree hash (§4).
    pub platform_tree_hash: String,
    /// Digest of the compiler that produced the artifact.
    pub compiler_blake3: String,
    /// Compile target (`wasm32` / `x64musl`).
    pub target: String,
    /// Author key that caused the compile.
    pub author: String,
    /// Sharing scope chosen at compile time.
    pub scope: CacheScope,
    /// Content hash the artifact is stored under.
    pub artifact_hash: String,
    /// Artifact length in bytes; the unit LRU accounting charges.
    pub artifact_bytes: u64,
    /// The compiler's self-reported version at store time, carried so
    /// a hit can rebuild the same build provenance as a fresh compile.
    pub compiler_version: String,
    /// Unix seconds when the entry was stored.
    pub created_at: u64,
    /// Unix seconds of the most recent hit (store time until then).
    pub last_hit_at: u64,
    /// Monotonic use counter (per cache process, seeded from the
    /// highest persisted value on open): the LRU eviction order.
    /// Wall-clock seconds tie too easily to order evictions by.
    pub last_used_seq: u64,
}

/// In-memory counters for one cache process lifetime. The durable
/// metering record is the `tracing` log (§7 P1: metering logged only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that found no visible entry.
    pub misses: u64,
    /// Entries stored after a miss.
    pub stores: u64,
    /// Entries evicted by LRU pressure.
    pub lru_evictions: u64,
    /// Entries evicted because the stored artifact failed its
    /// content-hash re-verification on a hit.
    pub corrupt_evictions: u64,
    /// Entries hard-evicted by invalidation.
    pub invalidations: u64,
}

/// What a lookup found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// Verified artifact bytes plus the entry they came from.
    Hit {
        /// The artifact bytes, hash-verified against the entry.
        artifact: Vec<u8>,
        /// The entry that served the hit.
        entry: CacheEntry,
    },
    /// No visible entry (absent, another author's, or proof-gated);
    /// the caller compiles.
    Miss,
}

/// What a store did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreOutcome {
    /// The entry is in the cache.
    Stored,
    /// The artifact alone exceeds the LRU budget; nothing was kept.
    Oversize,
}

#[derive(Debug, Default)]
struct Index {
    entries: BTreeMap<String, CacheEntry>,
    proofs: BTreeSet<(String, String)>,
    stats: CacheStats,
    next_seq: u64,
}

impl Index {
    fn next_use_seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }
}

impl Index {
    fn total_bytes(&self) -> u64 {
        self.entries
            .values()
            .map(|entry| entry.artifact_bytes)
            .sum()
    }
}

/// The shared compiled-world cache: a directory of content-addressed
/// artifacts plus per-key entry metadata and determinism proofs.
#[derive(Debug)]
pub struct WorldCache {
    root: PathBuf,
    lru_bytes: u64,
    index: Mutex<Index>,
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

impl WorldCache {
    /// Open (creating if needed) the cache rooted at `root` with the
    /// given tier-1 LRU budget in bytes. Existing entries and proofs
    /// are re-indexed from disk; unreadable metadata is skipped rather
    /// than wedging the server on one corrupt file.
    ///
    /// # Errors
    ///
    /// Fails when the cache directories cannot be created or read.
    pub fn open(root: PathBuf, lru_bytes: u64) -> Result<Self> {
        for dir in [
            root.join("entries"),
            root.join("artifacts"),
            root.join("proofs"),
        ] {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("create cache directory {}", dir.display()))?;
        }
        let mut index = Index::default();
        let entries_dir = root.join("entries");
        for file in std::fs::read_dir(&entries_dir)
            .with_context(|| format!("read cache entries {}", entries_dir.display()))?
        {
            let path = file?.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(entry) = serde_json::from_str::<CacheEntry>(&text)
            {
                index.next_seq = index.next_seq.max(entry.last_used_seq);
                index.entries.insert(entry.key.clone(), entry);
            }
        }
        let proofs_dir = root.join("proofs");
        for file in std::fs::read_dir(&proofs_dir)
            .with_context(|| format!("read cache proofs {}", proofs_dir.display()))?
        {
            let path = file?.path();
            if let Some(name) = path.file_name().and_then(|name| name.to_str())
                && let Some((compiler, platform)) = name
                    .strip_suffix(".json")
                    .and_then(|stem| stem.split_once("--"))
            {
                index
                    .proofs
                    .insert((compiler.to_owned(), platform.to_owned()));
            }
        }
        Ok(Self {
            root,
            lru_bytes,
            index: Mutex::new(index),
        })
    }

    /// The tier-1 LRU budget this cache was opened with.
    #[must_use]
    pub fn lru_bytes(&self) -> u64 {
        self.lru_bytes
    }

    /// Current counters.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        self.index
            .lock()
            .map(|index| index.stats)
            .unwrap_or_default()
    }

    /// Number of entries currently indexed.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.index.lock().map_or(0, |index| index.entries.len())
    }

    /// Total artifact bytes currently charged against the LRU budget.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.index.lock().map_or(0, |index| index.total_bytes())
    }

    /// Whether a determinism proof (§2.2) is on record for this
    /// (compiler, platform) pair.
    #[must_use]
    pub fn has_determinism_proof(&self, compiler_blake3: &str, platform_tree_hash: &str) -> bool {
        self.index.lock().is_ok_and(|index| {
            index
                .proofs
                .contains(&(compiler_blake3.to_owned(), platform_tree_hash.to_owned()))
        })
    }

    /// Record a determinism proof from a double compile (§2.2): the
    /// proof is recorded only when both clean-room compiles of the
    /// same spec produced the same artifact hash. Returns whether the
    /// proof is on record after the call.
    ///
    /// # Errors
    ///
    /// Fails when the proof cannot be persisted.
    pub fn record_determinism_proof(
        &self,
        compiler_blake3: &str,
        platform_tree_hash: &str,
        first_artifact_hash: &str,
        second_artifact_hash: &str,
    ) -> Result<bool> {
        if first_artifact_hash != second_artifact_hash {
            return Ok(false);
        }
        let pair = (compiler_blake3.to_owned(), platform_tree_hash.to_owned());
        if self.has_determinism_proof(compiler_blake3, platform_tree_hash) {
            return Ok(true);
        }
        let path = self
            .root
            .join("proofs")
            .join(format!("{compiler_blake3}--{platform_tree_hash}.json"));
        let body = serde_json::json!({
            "compiler_blake3": compiler_blake3,
            "platform_tree_hash": platform_tree_hash,
            "artifact_hash": first_artifact_hash,
            "proved_at": now_seconds(),
        });
        std::fs::write(&path, body.to_string())
            .with_context(|| format!("write determinism proof {}", path.display()))?;
        if let Ok(mut index) = self.index.lock() {
            index.proofs.insert(pair);
        }
        Ok(true)
    }

    /// Whether `entry` may serve a lookup by `author`: author-scope
    /// entries only ever serve their own author; server/public
    /// entries serve any author only with a determinism proof on
    /// record for the entry's (compiler, platform) pair, and their
    /// own author regardless (§2.2, §2.3).
    fn visible_to(index: &Index, entry: &CacheEntry, author: &str) -> bool {
        if entry.author == author {
            return true;
        }
        match entry.scope {
            CacheScope::Author => false,
            CacheScope::Server | CacheScope::Public => index.proofs.contains(&(
                entry.compiler_blake3.clone(),
                entry.platform_tree_hash.clone(),
            )),
        }
    }

    /// Look up the entry for `input` on behalf of `author`. A hit
    /// re-verifies the artifact's content hash before serving it; a
    /// failed verification evicts the entry and reports a miss (§2.1).
    ///
    /// # Errors
    ///
    /// Fails only on unexpected I/O errors reading a present entry's
    /// artifact; a missing artifact file is a miss, not an error.
    pub fn lookup(&self, input: &CacheKeyInput, author: &str) -> Result<Lookup> {
        let key = cache_key(input);
        let entry = {
            let mut index = self
                .index
                .lock()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            let visible = index
                .entries
                .get(&key)
                .filter(|entry| Self::visible_to(&index, entry, author))
                .cloned();
            let Some(entry) = visible else {
                index.stats.misses += 1;
                return Ok(Lookup::Miss);
            };
            entry
        };
        let artifact_path = self.root.join("artifacts").join(&entry.artifact_hash);
        let artifact = match std::fs::read(&artifact_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.evict(&key);
                self.note_miss();
                return Ok(Lookup::Miss);
            }
            Err(error) => {
                return Err(error)
                    .context(format!("read cached artifact {}", artifact_path.display()));
            }
        };
        if artifact_hash(&artifact) != entry.artifact_hash
            || artifact.len() as u64 != entry.artifact_bytes
        {
            self.evict(&key);
            self.note_miss();
            return Ok(Lookup::Miss);
        }
        if let Ok(mut index) = self.index.lock() {
            index.stats.hits += 1;
            let seq = index.next_use_seq();
            if let Some(stored) = index.entries.get_mut(&key) {
                stored.last_hit_at = now_seconds();
                stored.last_used_seq = seq;
                let _ = self.write_entry(stored);
            }
        }
        Ok(Lookup::Hit { artifact, entry })
    }

    /// Count a miss resolved outside the index scan (a present entry
    /// whose artifact was missing or failed re-verification).
    pub fn note_miss(&self) {
        if let Ok(mut index) = self.index.lock() {
            index.stats.misses += 1;
        }
    }

    /// Store a freshly compiled artifact under its key at the given
    /// scope, then evict least-recently-hit entries until the cache
    /// is within budget. The just-stored entry is exempt from its own
    /// eviction pass; an artifact that alone exceeds the budget is
    /// not kept at all.
    ///
    /// # Errors
    ///
    /// Fails when the artifact or its metadata cannot be written.
    pub fn store(
        &self,
        input: &CacheKeyInput,
        author: &str,
        scope: CacheScope,
        artifact: &[u8],
        compiler_version: &str,
    ) -> Result<StoreOutcome> {
        let artifact_bytes = artifact.len() as u64;
        if artifact_bytes > self.lru_bytes {
            return Ok(StoreOutcome::Oversize);
        }
        let hash = artifact_hash(artifact);
        let artifact_path = self.root.join("artifacts").join(&hash);
        if !artifact_path.is_file() {
            let temp = self.root.join("artifacts").join(format!(".{hash}.tmp"));
            std::fs::write(&temp, artifact)
                .with_context(|| format!("write cached artifact {}", temp.display()))?;
            std::fs::rename(&temp, &artifact_path)
                .with_context(|| format!("install cached artifact {}", artifact_path.display()))?;
        }
        let now = now_seconds();
        let mut entry = CacheEntry {
            key: cache_key(input),
            platform_tree_hash: input.platform_tree_hash.clone(),
            compiler_blake3: input.compiler_blake3.clone(),
            target: input.target.clone(),
            author: author.to_owned(),
            scope,
            artifact_hash: hash,
            artifact_bytes,
            compiler_version: compiler_version.to_owned(),
            created_at: now,
            last_hit_at: now,
            last_used_seq: 0,
        };
        let evictions = {
            let mut index = self
                .index
                .lock()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            entry.last_used_seq = index.next_use_seq();
            self.write_entry(&entry)?;
            index.stats.stores += 1;
            index.entries.insert(entry.key.clone(), entry.clone());
            let mut evicted = Vec::new();
            while index.total_bytes() > self.lru_bytes {
                let victim = index
                    .entries
                    .values()
                    .filter(|candidate| candidate.key != entry.key)
                    .min_by(|left, right| {
                        left.last_used_seq
                            .cmp(&right.last_used_seq)
                            .then(left.key.cmp(&right.key))
                    })
                    .map(|candidate| candidate.key.clone());
                let Some(victim) = victim else {
                    break;
                };
                if let Some(removed) = index.entries.remove(&victim) {
                    index.stats.lru_evictions += 1;
                    evicted.push(removed);
                }
            }
            evicted
        };
        for removed in &evictions {
            self.remove_entry_files(removed, true);
        }
        Ok(StoreOutcome::Stored)
    }

    /// Hard-evict every entry produced by this compiler digest, and
    /// drop its determinism proofs (§4: invalidation beats every
    /// tier). Returns how many entries were evicted.
    pub fn invalidate_compiler(&self, compiler_blake3: &str) -> usize {
        self.invalidate(|entry| entry.compiler_blake3 == compiler_blake3)
    }

    /// Hard-evict every entry built against this platform tree, and
    /// drop its determinism proofs (§4). Returns how many entries
    /// were evicted.
    pub fn invalidate_platform(&self, platform_tree_hash: &str) -> usize {
        self.invalidate(|entry| entry.platform_tree_hash == platform_tree_hash)
    }

    fn invalidate(&self, matches: impl Fn(&CacheEntry) -> bool) -> usize {
        let (removed, proofs) = {
            let Ok(mut index) = self.index.lock() else {
                return 0;
            };
            let keys: Vec<String> = index
                .entries
                .values()
                .filter(|entry| matches(entry))
                .map(|entry| entry.key.clone())
                .collect();
            let mut removed = Vec::new();
            for key in keys {
                if let Some(entry) = index.entries.remove(&key) {
                    index.stats.invalidations += 1;
                    removed.push(entry);
                }
            }
            let stale_proofs: Vec<(String, String)> = index
                .proofs
                .iter()
                .filter(|(compiler, platform)| {
                    removed.iter().any(|entry| {
                        entry.compiler_blake3 == *compiler || entry.platform_tree_hash == *platform
                    })
                })
                .cloned()
                .collect();
            for pair in &stale_proofs {
                index.proofs.remove(pair);
            }
            (removed, stale_proofs)
        };
        for entry in &removed {
            self.remove_entry_files(entry, true);
        }
        for (compiler, platform) in &proofs {
            let _ = std::fs::remove_file(
                self.root
                    .join("proofs")
                    .join(format!("{compiler}--{platform}.json")),
            );
        }
        removed.len()
    }

    fn write_entry(&self, entry: &CacheEntry) -> Result<()> {
        let path = self
            .root
            .join("entries")
            .join(format!("{}.json", entry.key));
        let text = serde_json::to_string(entry).context("serialize cache entry")?;
        std::fs::write(&path, text).with_context(|| format!("write cache entry {}", path.display()))
    }

    fn remove_entry_files(&self, entry: &CacheEntry, drop_artifact_if_orphan: bool) {
        let _ = std::fs::remove_file(
            self.root
                .join("entries")
                .join(format!("{}.json", entry.key)),
        );
        if drop_artifact_if_orphan {
            let still_referenced = self.index.lock().is_ok_and(|index| {
                index
                    .entries
                    .values()
                    .any(|other| other.artifact_hash == entry.artifact_hash)
            });
            if !still_referenced {
                let _ =
                    std::fs::remove_file(self.root.join("artifacts").join(&entry.artifact_hash));
            }
        }
    }

    /// Evict an entry whose stored artifact failed verification.
    fn evict(&self, key: &str) {
        let removed = {
            let Ok(mut index) = self.index.lock() else {
                return;
            };
            let removed = index.entries.remove(key);
            if removed.is_some() {
                index.stats.corrupt_evictions += 1;
            }
            removed
        };
        if let Some(entry) = removed {
            self.remove_entry_files(&entry, true);
        }
    }
}

/// Sanity bound used by callers that accept an operator-supplied
/// budget: the budget must be at least one maximum-size module, so a
/// typo cannot silently disable the cache.
pub fn validate_lru_bytes(lru_bytes: u64, max_module_bytes: u64) -> Result<()> {
    ensure!(
        lru_bytes >= max_module_bytes,
        "compiled-world cache budget {lru_bytes} bytes is below one maximum module ({max_module_bytes} bytes)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(source: &str) -> CacheKeyInput {
        CacheKeyInput {
            platform_tree_hash: "platform-a".to_owned(),
            source_closure_hash: source.to_owned(),
            dll_closure_hash: dll_closure_hash(&[]),
            compiler_blake3: "compiler-a".to_owned(),
            target: "wasm32".to_owned(),
        }
    }

    fn cache(budget: u64) -> (tempfile::TempDir, WorldCache) {
        let dir = tempfile::tempdir().unwrap();
        let cache = WorldCache::open(dir.path().join("cache"), budget).unwrap();
        (dir, cache)
    }

    #[test]
    fn key_changes_with_every_component_and_only_those() {
        let base = input("source-a");
        let base_key = cache_key(&base);
        assert_eq!(
            base_key,
            cache_key(&input("source-a")),
            "same inputs, same key"
        );
        let mut changed = base.clone();
        changed.platform_tree_hash = "platform-b".to_owned();
        assert_ne!(base_key, cache_key(&changed), "platform is in the key");
        changed = base.clone();
        changed.dll_closure_hash = dll_closure_hash(&["lib".to_owned()]);
        assert_ne!(base_key, cache_key(&changed), "dll closure is in the key");
        changed = base.clone();
        changed.compiler_blake3 = "compiler-b".to_owned();
        assert_ne!(base_key, cache_key(&changed), "compiler is in the key");
        changed = base.clone();
        changed.target = "x64musl".to_owned();
        assert_ne!(base_key, cache_key(&changed), "target is in the key");
        assert_ne!(
            base_key,
            cache_key(&input("source-b")),
            "source is in the key"
        );
        // The seed is not a component at all: two inputs that differ
        // only by seed are the same input here by construction.
    }

    #[test]
    fn dll_closure_is_order_independent_and_empty_is_constant() {
        let empty = dll_closure_hash(&[]);
        assert_eq!(empty, dll_closure_hash(&[]));
        let forward = dll_closure_hash(&["a".to_owned(), "b".to_owned()]);
        let reverse = dll_closure_hash(&["b".to_owned(), "a".to_owned()]);
        assert_eq!(forward, reverse);
        assert_ne!(empty, forward);
    }

    #[test]
    fn author_scope_entry_serves_only_its_author() {
        let (_dir, cache) = cache(1 << 20);
        cache
            .store(
                &input("s"),
                "alice",
                CacheScope::Author,
                b"artifact",
                "roc 1",
            )
            .unwrap();
        assert!(matches!(
            cache.lookup(&input("s"), "alice").unwrap(),
            Lookup::Hit { .. }
        ));
        assert_eq!(cache.lookup(&input("s"), "bob").unwrap(), Lookup::Miss);
    }

    #[test]
    fn server_scope_needs_a_determinism_proof_for_cross_author_hits() {
        let (_dir, cache) = cache(1 << 20);
        cache
            .store(
                &input("s"),
                "alice",
                CacheScope::Server,
                b"artifact",
                "roc 1",
            )
            .unwrap();
        // No proof on record: the pair operates at author scope only.
        assert_eq!(cache.lookup(&input("s"), "bob").unwrap(), Lookup::Miss);
        assert!(matches!(
            cache.lookup(&input("s"), "alice").unwrap(),
            Lookup::Hit { .. }
        ));
        // A failed proof (different artifact hashes) changes nothing.
        assert!(
            !cache
                .record_determinism_proof("compiler-a", "platform-a", "hash-a", "hash-b")
                .unwrap()
        );
        assert_eq!(cache.lookup(&input("s"), "bob").unwrap(), Lookup::Miss);
        // A real proof opens cross-author hits.
        assert!(
            cache
                .record_determinism_proof("compiler-a", "platform-a", "hash-a", "hash-a")
                .unwrap()
        );
        assert!(matches!(
            cache.lookup(&input("s"), "bob").unwrap(),
            Lookup::Hit { .. }
        ));
    }

    #[test]
    fn public_scope_is_stubbed_at_server_behaviour() {
        let (_dir, cache) = cache(1 << 20);
        cache
            .store(
                &input("s"),
                "alice",
                CacheScope::Public,
                b"artifact",
                "roc 1",
            )
            .unwrap();
        assert_eq!(cache.lookup(&input("s"), "bob").unwrap(), Lookup::Miss);
        cache
            .record_determinism_proof("compiler-a", "platform-a", "h", "h")
            .unwrap();
        assert!(matches!(
            cache.lookup(&input("s"), "bob").unwrap(),
            Lookup::Hit { .. }
        ));
    }

    #[test]
    fn a_hit_reverifies_the_artifact_and_evicts_on_mismatch() {
        let (dir, cache) = cache(1 << 20);
        cache
            .store(
                &input("s"),
                "alice",
                CacheScope::Author,
                b"artifact",
                "roc 1",
            )
            .unwrap();
        // Corrupt the stored artifact under its content-hash name.
        let hash = artifact_hash(b"artifact");
        std::fs::write(dir.path().join("cache/artifacts").join(hash), b"tampered!").unwrap();
        assert_eq!(cache.lookup(&input("s"), "alice").unwrap(), Lookup::Miss);
        assert_eq!(cache.entry_count(), 0);
        assert_eq!(cache.stats().corrupt_evictions, 1);
    }

    #[test]
    fn lru_evicts_the_least_recently_hit_entry() {
        let (_dir, cache) = cache(10);
        cache
            .store(
                &input("old"),
                "alice",
                CacheScope::Author,
                b"12345",
                "roc 1",
            )
            .unwrap();
        cache
            .store(
                &input("new"),
                "alice",
                CacheScope::Author,
                b"67890",
                "roc 1",
            )
            .unwrap();
        assert_eq!(cache.total_bytes(), 10);
        // Touch "old" so "new" becomes the coldest, then store a third
        // entry that forces one eviction.
        assert!(matches!(
            cache.lookup(&input("old"), "alice").unwrap(),
            Lookup::Hit { .. }
        ));
        cache
            .store(
                &input("third"),
                "alice",
                CacheScope::Author,
                b"abcde",
                "roc 1",
            )
            .unwrap();
        assert_eq!(cache.total_bytes(), 10);
        assert_eq!(cache.lookup(&input("new"), "alice").unwrap(), Lookup::Miss);
        assert!(matches!(
            cache.lookup(&input("old"), "alice").unwrap(),
            Lookup::Hit { .. }
        ));
        assert_eq!(cache.stats().lru_evictions, 1);
    }

    #[test]
    fn an_oversize_artifact_is_not_stored() {
        let (_dir, cache) = cache(4);
        let outcome = cache
            .store(&input("s"), "alice", CacheScope::Author, b"12345", "roc 1")
            .unwrap();
        assert_eq!(outcome, StoreOutcome::Oversize);
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn invalidation_by_compiler_and_platform_drops_entries_and_proofs() {
        let (_dir, cache) = cache(1 << 20);
        cache
            .store(
                &input("s"),
                "alice",
                CacheScope::Server,
                b"artifact",
                "roc 1",
            )
            .unwrap();
        cache
            .record_determinism_proof("compiler-a", "platform-a", "h", "h")
            .unwrap();
        assert_eq!(cache.invalidate_compiler("compiler-a"), 1);
        assert_eq!(cache.entry_count(), 0);
        assert!(!cache.has_determinism_proof("compiler-a", "platform-a"));

        cache
            .store(
                &input("s"),
                "alice",
                CacheScope::Server,
                b"artifact",
                "roc 1",
            )
            .unwrap();
        assert_eq!(cache.invalidate_platform("platform-a"), 1);
        assert_eq!(cache.entry_count(), 0);
        assert_eq!(cache.stats().invalidations, 2);
    }

    #[test]
    fn entries_and_proofs_survive_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        {
            let cache = WorldCache::open(root.clone(), 1 << 20).unwrap();
            cache
                .store(
                    &input("s"),
                    "alice",
                    CacheScope::Server,
                    b"artifact",
                    "roc 1",
                )
                .unwrap();
            cache
                .record_determinism_proof("compiler-a", "platform-a", "h", "h")
                .unwrap();
        }
        let cache = WorldCache::open(root, 1 << 20).unwrap();
        assert_eq!(cache.entry_count(), 1);
        assert!(cache.has_determinism_proof("compiler-a", "platform-a"));
        assert!(matches!(
            cache.lookup(&input("s"), "bob").unwrap(),
            Lookup::Hit { .. }
        ));
    }

    #[test]
    fn different_inputs_miss() {
        let (_dir, cache) = cache(1 << 20);
        cache
            .store(
                &input("s"),
                "alice",
                CacheScope::Author,
                b"artifact",
                "roc 1",
            )
            .unwrap();
        assert_eq!(
            cache.lookup(&input("other"), "alice").unwrap(),
            Lookup::Miss
        );
    }
}
