//! Query-embedding cache (design PR #5667 section 3.1; issue #9941,
//! slice 1 of #5668).
//!
//! A query vector is a pure function of (normalized query text, model
//! id, model revision): identical inputs give bit-identical outputs.
//! Therefore the cache needs no TTL and no invalidation — a model
//! change changes the key. Two tiers:
//!
//! - service memory: an LRU of ~10k entries (the design's ~30 MB at
//!   768 dims), always on;
//! - a shared Turso cache DB in the internal tier, configured by env
//!   (`ATLAS_EMBED_CACHE_TURSO_URL` + `ATLAS_EMBED_CACHE_TURSO_TOKEN`),
//!   so every client of the service shares it. Writes are
//!   asynchronous and best-effort — a cache write never sits in the
//!   request path, and a cache outage never fails a search.
//!
//! Normalization (the design's `normalized_text`): trim and collapse
//! every whitespace run to a single ASCII space
//! (`split_whitespace` + join). Case is preserved — the embedder is
//! case-sensitive, so case variants must re-key (the PBT below pins
//! both directions). The embedder is always fed the normalized text,
//! so the cache is a pure function of its key by construction.
//!
//! The cached value is the RAW full-dimension vector, before
//! [`crate::embed::finish_query_vector`]: truncation is a per-request
//! parameter, not part of the key, so the shared tail is applied
//! after retrieval on both hit and miss paths — identical outputs.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};

use crate::embed::{AtlasEmbedder, FixedEmbedder, QueryEmbedder, finish_query_vector};
use crate::error::Result;

/// Default memory-tier capacity (design section 3.1: ~10k entries).
pub const DEFAULT_MEMORY_CAPACITY: usize = 10_000;

/// Env vars configuring the shared Turso tier. Both must be set; when
/// either is absent the cache runs memory-only.
pub const ENV_TURSO_URL: &str = "ATLAS_EMBED_CACHE_TURSO_URL";
pub const ENV_TURSO_TOKEN: &str = "ATLAS_EMBED_CACHE_TURSO_TOKEN";

const TURSO_TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS query_embedding_cache \
    (key BLOB PRIMARY KEY, vector BLOB NOT NULL, created_unix INTEGER NOT NULL)";

/// The design's query-text normalization: trim + collapse whitespace
/// runs to one ASCII space. Idempotent; case-preserving.
pub fn normalize_query_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cache key: sha256 over the design's three fields
/// (`model_id || model_rev || normalized_text`). The two identity
/// fields are length-prefixed so the concatenation is unambiguous —
/// no split of the byte stream can alias a different
/// (model_id, model_rev, text) triple to the same digest input.
pub fn cache_key(model_id: &str, model_rev: &str, normalized_text: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update((model_id.len() as u64).to_le_bytes());
    h.update(model_id.as_bytes());
    h.update((model_rev.len() as u64).to_le_bytes());
    h.update(model_rev.as_bytes());
    h.update(normalized_text.as_bytes());
    h.finalize().into()
}

fn vector_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn blob_to_vector(b: &[u8]) -> Option<Vec<f32>> {
    if b.is_empty() || b.len() % 4 != 0 {
        return None;
    }
    Some(
        b.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

// ------------------------------------------------------------ memory tier --

struct LruInner {
    map: HashMap<[u8; 32], Vec<f32>>,
    order: VecDeque<[u8; 32]>,
    capacity: usize,
}

struct MemoryTier {
    inner: Mutex<LruInner>,
}

impl MemoryTier {
    fn new(capacity: usize) -> MemoryTier {
        MemoryTier {
            inner: Mutex::new(LruInner {
                map: HashMap::new(),
                order: VecDeque::new(),
                capacity: capacity.max(1),
            }),
        }
    }

    fn get(&self, key: &[u8; 32]) -> Option<Vec<f32>> {
        let mut g = self.inner.lock().ok()?;
        let v = g.map.get(key)?.clone();
        if let Some(pos) = g.order.iter().position(|k| k == key) {
            g.order.remove(pos);
            g.order.push_back(*key);
        }
        Some(v)
    }

    fn put(&self, key: [u8; 32], v: Vec<f32>) {
        let Ok(mut g) = self.inner.lock() else { return };
        if g.map.insert(key, v).is_none() {
            g.order.push_back(key);
        } else if let Some(pos) = g.order.iter().position(|k| k == &key) {
            g.order.remove(pos);
            g.order.push_back(key);
        }
        while g.map.len() > g.capacity {
            match g.order.pop_front() {
                Some(old) => {
                    g.map.remove(&old);
                }
                None => break,
            }
        }
    }
}

// ------------------------------------------------------------ shared tier --

/// The shared second tier. Implementations are best-effort by
/// contract: `get` returns `None` on any failure, `put` drops the
/// write on any failure. Neither may panic or surface an error —
/// a cache outage must not fail a search.
pub trait SharedEmbeddingStore: Send + Sync {
    fn get(&self, key: &[u8; 32]) -> Option<Vec<f32>>;
    fn put(&self, key: &[u8; 32], vector: &[f32]);
}

/// The shared tier over a Turso/libsql cache DB, spoken to with the
/// same HTTPS pipeline protocol as [`crate::turso::TursoBackend`]
/// (that backend is read-only by construction; the cache DB is a
/// separate, small, internal-tier DB with its own rw token).
pub struct TursoEmbeddingStore {
    url: String,
    token: String,
    agent: ureq::Agent,
    table_ready: OnceLock<()>,
}

impl TursoEmbeddingStore {
    pub fn new(url: &str, token: &str) -> TursoEmbeddingStore {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(2)))
            .build()
            .into();
        TursoEmbeddingStore {
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            agent,
            table_ready: OnceLock::new(),
        }
    }

    /// One pipeline round-trip: a single execute statement plus
    /// close. Returns the result rows on success, `None` on any
    /// failure (transport, HTTP status, protocol shape).
    fn execute(&self, sql: &str, args: Vec<Json>) -> Option<Vec<Vec<Json>>> {
        let body = json!({
            "requests": [
                {"type": "execute", "stmt": {"sql": sql, "args": args}},
                {"type": "close"}
            ]
        });
        let body = serde_json::to_string(&body).ok()?;
        let mut resp = self
            .agent
            .post(&format!("{}/v2/pipeline", self.url))
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send(body.as_str())
            .ok()?;
        let text = resp.body_mut().read_to_string().ok()?;
        let doc: Json = serde_json::from_str(&text).ok()?;
        let result = doc.get("results")?.get(0)?.get("response")?.get("result")?;
        let rows = result.get("rows")?.as_array()?;
        Some(
            rows.iter()
                .map(|r| r.as_array().cloned().unwrap_or_default())
                .collect(),
        )
    }

    fn ensure_table(&self) {
        self.table_ready.get_or_init(|| {
            let _ = self.execute(TURSO_TABLE_DDL, vec![]);
        });
    }
}

fn blob_arg(b: &[u8]) -> Json {
    json!({"type": "blob", "value": base64::engine::general_purpose::STANDARD.encode(b)})
}

impl SharedEmbeddingStore for TursoEmbeddingStore {
    fn get(&self, key: &[u8; 32]) -> Option<Vec<f32>> {
        self.ensure_table();
        let rows = self.execute(
            "SELECT vector FROM query_embedding_cache WHERE key = ?",
            vec![blob_arg(key)],
        )?;
        let cell = rows.first()?.first()?;
        if cell.get("type").and_then(Json::as_str) != Some("blob") {
            return None;
        }
        let raw = base64::engine::general_purpose::STANDARD
            .decode(cell.get("value")?.as_str()?)
            .ok()?;
        blob_to_vector(&raw)
    }

    fn put(&self, key: &[u8; 32], vector: &[f32]) {
        self.ensure_table();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let _ = self.execute(
            "INSERT OR REPLACE INTO query_embedding_cache (key, vector, created_unix) \
             VALUES (?, ?, ?)",
            vec![
                blob_arg(key),
                blob_arg(&vector_to_blob(vector)),
                json!({"type": "integer", "value": now.to_string()}),
            ],
        );
    }
}

// ----------------------------------------------------------------- cache --

/// The two-tier query-embedding cache. Keys are derived from the
/// model identity fixed at construction plus the normalized text.
pub struct QueryEmbeddingCache {
    model_id: String,
    model_rev: String,
    memory: MemoryTier,
    store: Option<Arc<dyn SharedEmbeddingStore>>,
    writer: Option<SyncSender<([u8; 32], Vec<f32>)>>,
}

impl QueryEmbeddingCache {
    /// Memory-only cache (no shared tier).
    pub fn new(model_id: &str, model_rev: &str) -> QueryEmbeddingCache {
        Self::with_store(model_id, model_rev, None, DEFAULT_MEMORY_CAPACITY)
    }

    /// Cache with an optional shared tier; writes to the tier are
    /// queued to a background writer thread (bounded channel, drops
    /// when full — best-effort by design) and never block the caller.
    pub fn with_store(
        model_id: &str,
        model_rev: &str,
        store: Option<Arc<dyn SharedEmbeddingStore>>,
        memory_capacity: usize,
    ) -> QueryEmbeddingCache {
        let writer = store.as_ref().map(|s| {
            let (tx, rx) = sync_channel::<([u8; 32], Vec<f32>)>(256);
            let store = Arc::clone(s);
            std::thread::spawn(move || {
                while let Ok((key, vector)) = rx.recv() {
                    store.put(&key, &vector);
                }
            });
            tx
        });
        QueryEmbeddingCache {
            model_id: model_id.to_string(),
            model_rev: model_rev.to_string(),
            memory: MemoryTier::new(memory_capacity),
            store,
            writer,
        }
    }

    /// Build from env: the shared Turso tier is present iff both
    /// [`ENV_TURSO_URL`] and [`ENV_TURSO_TOKEN`] are set and
    /// non-empty; otherwise the cache is memory-only.
    pub fn from_env(model_id: &str, model_rev: &str) -> QueryEmbeddingCache {
        let url = std::env::var(ENV_TURSO_URL).unwrap_or_default();
        let token = std::env::var(ENV_TURSO_TOKEN).unwrap_or_default();
        let store: Option<Arc<dyn SharedEmbeddingStore>> = if url.is_empty() || token.is_empty() {
            None
        } else {
            Some(Arc::new(TursoEmbeddingStore::new(&url, &token)))
        };
        Self::with_store(model_id, model_rev, store, DEFAULT_MEMORY_CAPACITY)
    }

    pub fn key_for(&self, normalized_text: &str) -> [u8; 32] {
        cache_key(&self.model_id, &self.model_rev, normalized_text)
    }

    /// Look up a normalized text: memory tier first, shared tier
    /// second (a shared hit is promoted into memory). `None` = miss
    /// or tier failure; the caller falls back to the embedder.
    pub fn get(&self, normalized_text: &str) -> Option<Vec<f32>> {
        let key = self.key_for(normalized_text);
        if let Some(v) = self.memory.get(&key) {
            return Some(v);
        }
        let v = self.store.as_ref()?.get(&key)?;
        self.memory.put(key, v.clone());
        Some(v)
    }

    /// Record a freshly computed vector: memory synchronously (so an
    /// immediate repeat hits), shared tier asynchronously.
    pub fn put(&self, normalized_text: &str, vector: Vec<f32>) {
        let key = self.key_for(normalized_text);
        self.memory.put(key, vector.clone());
        if let Some(tx) = &self.writer {
            let _ = tx.try_send((key, vector));
        }
    }
}

// -------------------------------------------------------------- embedder --

/// A raw query embedder: normalized text in, full-dimension raw
/// vector out (prompt applied by the implementation, NO truncation
/// or renormalization — that shared tail belongs to
/// [`finish_query_vector`] so cached and fresh vectors pass through
/// exactly the same shaping).
pub trait RawQueryEmbedder: Send + Sync {
    fn embed_raw(&self, text: &str) -> Result<Vec<f32>>;
}

impl RawQueryEmbedder for AtlasEmbedder {
    fn embed_raw(&self, text: &str) -> Result<Vec<f32>> {
        AtlasEmbedder::embed_raw(self, text)
    }
}

impl RawQueryEmbedder for FixedEmbedder {
    fn embed_raw(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(self.raw.clone())
    }
}

/// A [`QueryEmbedder`] decorator adding the section-3.1 cache in
/// front of a raw embedder. The text is normalized before both the
/// key and the embedder, so the cache is a pure function of its key.
pub struct CachingEmbedder<E: RawQueryEmbedder> {
    inner: E,
    cache: Arc<QueryEmbeddingCache>,
}

impl<E: RawQueryEmbedder> CachingEmbedder<E> {
    pub fn new(inner: E, cache: Arc<QueryEmbeddingCache>) -> CachingEmbedder<E> {
        CachingEmbedder { inner, cache }
    }
}

impl<E: RawQueryEmbedder> QueryEmbedder for CachingEmbedder<E> {
    fn embed_query(&self, text: &str, trunc: usize) -> Result<Vec<f32>> {
        let normalized = normalize_query_text(text);
        if let Some(raw) = self.cache.get(&normalized) {
            return finish_query_vector(raw, trunc);
        }
        let raw = self.inner.embed_raw(&normalized)?;
        self.cache.put(&normalized, raw.clone());
        finish_query_vector(raw, trunc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counting raw embedder: the provider-call oracle. Its vector is
    /// a deterministic function of the text (sha256-expanded), so
    /// distinct texts get distinct vectors and repeats are stable.
    struct CountingEmbedder {
        calls: AtomicUsize,
    }

    impl RawQueryEmbedder for CountingEmbedder {
        fn embed_raw(&self, text: &str) -> Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut seed = {
                let mut h = Sha256::new();
                h.update(text.as_bytes());
                h.finalize()
            };
            let mut out = Vec::new();
            while out.len() < 8 {
                for b in seed {
                    out.push((b as f32) / 255.0 + 0.01);
                }
                let mut h = Sha256::new();
                h.update(seed);
                seed = h.finalize();
            }
            Ok(out)
        }
    }

    /// Build a caching embedder over a fresh counting embedder,
    /// returning both the embedder and a handle on the counter.
    fn caching(
        cache: Arc<QueryEmbeddingCache>,
    ) -> (CachingEmbedder<CountingRef>, Arc<CountingEmbedder>) {
        let inner = Arc::new(CountingEmbedder {
            calls: AtomicUsize::new(0),
        });
        let wrapper = CountingRef(Arc::clone(&inner));
        (CachingEmbedder::new(wrapper, cache), inner)
    }

    struct CountingRef(Arc<CountingEmbedder>);

    impl RawQueryEmbedder for CountingRef {
        fn embed_raw(&self, text: &str) -> Result<Vec<f32>> {
            self.0.embed_raw(text)
        }
    }

    #[test]
    fn repeat_query_served_from_cache_provider_called_once() {
        let cache = Arc::new(QueryEmbeddingCache::new("m", "r1"));
        let (emb, inner) = caching(cache);
        let a = emb.embed_query("hello world", 0).unwrap();
        let b = emb.embed_query("hello world", 0).unwrap();
        let c = emb.embed_query("  hello   world ", 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c, "whitespace variants normalize to the same entry");
        assert_eq!(
            inner.calls.load(Ordering::SeqCst),
            1,
            "oracle: provider call count stays 1 across repeats"
        );
        let d = emb.embed_query("hello world", 4).unwrap();
        assert_eq!(d.len(), 4, "truncation still applied per request");
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
        let e = emb.embed_query("different text", 0).unwrap();
        assert_ne!(a, e);
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn memory_tier_shared_across_embedder_instances() {
        let cache = Arc::new(QueryEmbeddingCache::new("m", "r1"));
        let (emb1, inner1) = caching(Arc::clone(&cache));
        let (emb2, inner2) = caching(cache);
        let a = emb1.embed_query("shared query", 0).unwrap();
        let b = emb2.embed_query("shared query", 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(inner1.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            inner2.calls.load(Ordering::SeqCst),
            0,
            "second instance served from the shared memory tier"
        );
    }

    /// In-memory SharedEmbeddingStore standing in for the Turso tier.
    struct FakeStore {
        map: Mutex<HashMap<[u8; 32], Vec<f32>>>,
        gets: AtomicUsize,
        puts: AtomicUsize,
    }

    impl SharedEmbeddingStore for FakeStore {
        fn get(&self, key: &[u8; 32]) -> Option<Vec<f32>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.map.lock().unwrap().get(key).cloned()
        }
        fn put(&self, key: &[u8; 32], vector: &[f32]) {
            self.puts.fetch_add(1, Ordering::SeqCst);
            self.map.lock().unwrap().insert(*key, vector.to_vec());
        }
    }

    #[test]
    fn shared_tier_serves_a_fresh_memory_cache() {
        let store = Arc::new(FakeStore {
            map: Mutex::new(HashMap::new()),
            gets: AtomicUsize::new(0),
            puts: AtomicUsize::new(0),
        });
        let store_dyn: Arc<dyn SharedEmbeddingStore> = store.clone();
        let cache1 = Arc::new(QueryEmbeddingCache::with_store(
            "m",
            "r1",
            Some(Arc::clone(&store_dyn)),
            16,
        ));
        let (emb1, inner1) = caching(cache1);
        let a = emb1.embed_query("cross client query", 0).unwrap();
        assert_eq!(inner1.calls.load(Ordering::SeqCst), 1);
        // The write is async: wait (bounded) for the writer thread.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while store.puts.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(store.puts.load(Ordering::SeqCst), 1, "async write landed");

        // A second cache (fresh memory, same shared tier + identity)
        // must be served by the shared tier without a provider call.
        let cache2 = Arc::new(QueryEmbeddingCache::with_store(
            "m",
            "r1",
            Some(store_dyn),
            16,
        ));
        let (emb2, inner2) = caching(cache2);
        let b = emb2.embed_query("cross client query", 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            inner2.calls.load(Ordering::SeqCst),
            0,
            "shared tier hit: provider not called"
        );
        assert!(store.gets.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn unavailable_shared_tier_never_fails_a_search() {
        // Port 1 on loopback: nothing listens; every tier op fails.
        let store: Arc<dyn SharedEmbeddingStore> =
            Arc::new(TursoEmbeddingStore::new("http://127.0.0.1:1", "unused"));
        let cache = Arc::new(QueryEmbeddingCache::with_store("m", "r1", Some(store), 16));
        let (emb, inner) = caching(cache);
        let a = emb.embed_query("resilient query", 0).unwrap();
        let b = emb.embed_query("resilient query", 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            inner.calls.load(Ordering::SeqCst),
            1,
            "memory tier still serves the repeat with the shared tier down"
        );
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let cache = QueryEmbeddingCache::with_store("m", "r1", None, 3);
        cache.put("a", vec![1.0]);
        cache.put("b", vec![2.0]);
        cache.put("c", vec![3.0]);
        assert!(cache.get("a").is_some(), "touch a so b becomes LRU");
        cache.put("d", vec![4.0]);
        assert!(cache.get("b").is_none(), "b evicted");
        assert!(cache.get("a").is_some());
        assert!(cache.get("c").is_some());
        assert!(cache.get("d").is_some());
    }

    #[test]
    fn blob_roundtrip() {
        let v = vec![0.0f32, -1.5, 3.25, 1e-30];
        assert_eq!(blob_to_vector(&vector_to_blob(&v)), Some(v));
        assert_eq!(blob_to_vector(&[1, 2, 3]), None);
        assert_eq!(blob_to_vector(&[]), None);
    }

    use proptest::prelude::*;

    proptest! {
        /// Oracle: normalization is idempotent, yields single-spaced
        /// trimmed text, and is invariant under whitespace padding /
        /// re-spacing of the same token sequence.
        #[test]
        fn normalization_is_canonical(text in "\\PC{0,60}") {
            let n = normalize_query_text(&text);
            prop_assert_eq!(&n, &normalize_query_text(&n), "idempotent");
            let tokens: Vec<&str> = text.split_whitespace().collect();
            prop_assert_eq!(&n, &tokens.join(" "));
            let padded = format!("  {}  ", tokens.join(" \t  "));
            prop_assert_eq!(&n, &normalize_query_text(&padded));
        }

        /// Oracle: whitespace variants of one token sequence share a
        /// key; any change to a token (including case) re-keys; any
        /// change to model id or revision re-keys.
        #[test]
        fn key_whitespace_invariant_but_content_sensitive(
            tokens in proptest::collection::vec("[a-z]{1,8}", 1..6),
            gap in "[ \\t]{1,3}",
            model in "[a-z0-9./-]{1,20}",
            rev in "[a-f0-9]{4,16}",
        ) {
            let spaced = tokens.join(&gap);
            let canonical = tokens.join(" ");
            prop_assert_eq!(
                cache_key(&model, &rev, &normalize_query_text(&spaced)),
                cache_key(&model, &rev, &normalize_query_text(&canonical)),
            );
            let base = cache_key(&model, &rev, &canonical);
            let mut changed = tokens.clone();
            changed[0] = format!("{}x", changed[0]);
            prop_assert_ne!(base, cache_key(&model, &rev, &changed.join(" ")));
            let mut upper = tokens.clone();
            upper[0] = upper[0].to_uppercase();
            prop_assert_ne!(
                base,
                cache_key(&model, &rev, &upper.join(" ")),
                "case must re-key: the embedder is case-sensitive"
            );
            prop_assert_ne!(base, cache_key(&format!("{model}x"), &rev, &canonical));
            prop_assert_ne!(base, cache_key(&model, &format!("{rev}x"), &canonical));
        }

        /// Oracle: field framing is unambiguous — moving a character
        /// across the model_id/model_rev boundary changes the key.
        #[test]
        fn key_field_boundaries_unambiguous(
            a in "[a-z]{1,6}", b in "[a-z]{1,6}", c in "[a-z]{1,6}",
            text in "[a-z ]{1,20}",
        ) {
            let left = cache_key(&format!("{a}{b}"), &c, &text);
            let right = cache_key(&a, &format!("{b}{c}"), &text);
            prop_assert_ne!(left, right);
        }
    }
}
