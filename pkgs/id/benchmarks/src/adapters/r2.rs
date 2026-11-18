//! R2 object workload adapter + native operation counters for the
//! #9133 benchmark contract (slice C9133-02-R2, issue #10163).
//!
//! Modelled against a Fake R2 capability fixture — no network, no
//! provider calls. The capability surface follows the parent-verified
//! R2 conditional-operations table on the issue:
//!
//! - Conditional WRITES are the verified conditional surface:
//!   put-if-absent, put-if-match and copy-if-match are supported and
//!   their outcomes are counted natively.
//! - Conditional DeleteObject is UNSUPPORTED. It is never claimed:
//!   `FakeR2::delete_conditional` and `R2Adapter::delete_conditional`
//!   refuse every call as unsupported.
//! - A plain delete is possible only for a captured, immutable,
//!   generation-qualified key, and only after the #9084 owner fence
//!   and key non-reuse are proven to the adapter. A stale delete
//!   targets only its captured old key and cannot alias a newer
//!   generation (generation keys are never reused — the adapter
//!   keeps an ever-used key registry, mirroring the reference store
//!   in `crate`).
//!
//! Oracle (reference): "Fake R2 capability fixture verifies
//! conditional writes, opaque generation-qualified locator
//! construction and never-reuse; conditional DeleteObject is
//! unsupported. A stale delete targets only its captured old key and
//! cannot alias a new generation."

use crate::{content_id, physical_key};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Engine name in the benchmark plan's engine set (`crate::ENGINES`).
pub const ENGINE: &str = "r2";

/// Opaque generation-qualified locator for one stored object
/// generation. The key is the only addressing the adapter exposes;
/// it embeds the content digest prefix and the generation, exactly
/// as constructed by `crate::physical_key`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locator {
    pub content_id: String,
    pub generation: u64,
    pub key: String,
}

/// Native R2 operation counters: every adapter/fixture operation
/// increments exactly the counters its outcome implies, so a
/// benchmark run can report the operation mix it actually drove.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub puts: u64,
    pub gets: u64,
    pub checksums: u64,
    pub copies: u64,
    pub conditional_accepted: u64,
    pub conditional_rejected: u64,
    pub deletes: u64,
    pub deletes_refused: u64,
    pub unsupported: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum R2Error {
    /// No object exists under the addressed key.
    NotFound { key: String },
    /// A write condition (if-absent / if-match) did not hold.
    ConditionFailed { key: String },
    /// The operation is outside R2's verified capability surface.
    Unsupported { operation: String },
    /// A captured-key plain delete lacked a proven precondition
    /// (owner fence, capture, or non-reuse).
    DeleteRefused { reason: String },
    /// A generation key the adapter has already issued was offered
    /// again; generation keys are never reused.
    KeyReuse { key: String },
}

struct FakeObject {
    bytes: Vec<u8>,
    etag: String,
}

/// Fake R2 capability fixture: an in-memory object store carrying
/// exactly R2's verified conditional surface for this contract.
/// The etag of an object is its content identity
/// (`crate::content_id`), so if-match conditions compare digests.
#[derive(Default)]
pub struct FakeR2 {
    objects: HashMap<String, FakeObject>,
    counters: Counters,
}

impl FakeR2 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// Unconditional put (the plain PutObject leg).
    pub fn put(&mut self, key: &str, bytes: &[u8]) -> String {
        self.counters.puts += 1;
        let etag = content_id(bytes);
        self.objects.insert(
            key.to_string(),
            FakeObject {
                bytes: bytes.to_vec(),
                etag: etag.clone(),
            },
        );
        etag
    }

    /// Conditional put: create only when the key is absent
    /// (If-None-Match: *).
    pub fn put_if_absent(&mut self, key: &str, bytes: &[u8]) -> Result<String, R2Error> {
        if self.objects.contains_key(key) {
            self.counters.conditional_rejected += 1;
            return Err(R2Error::ConditionFailed {
                key: key.to_string(),
            });
        }
        let etag = self.put(key, bytes);
        self.counters.conditional_accepted += 1;
        Ok(etag)
    }

    /// Conditional put: replace only when the current etag matches
    /// (If-Match).
    pub fn put_if_match(
        &mut self,
        key: &str,
        bytes: &[u8],
        if_match_etag: &str,
    ) -> Result<String, R2Error> {
        match self.objects.get(key) {
            Some(obj) if obj.etag == if_match_etag => {
                let etag = self.put(key, bytes);
                self.counters.conditional_accepted += 1;
                Ok(etag)
            }
            _ => {
                self.counters.conditional_rejected += 1;
                Err(R2Error::ConditionFailed {
                    key: key.to_string(),
                })
            }
        }
    }

    /// Conditional copy: copy only when the source etag matches
    /// (CopyObject with a match condition on the source).
    pub fn copy_if_match(
        &mut self,
        source_key: &str,
        dest_key: &str,
        if_match_etag: &str,
    ) -> Result<String, R2Error> {
        let bytes = match self.objects.get(source_key) {
            Some(obj) if obj.etag == if_match_etag => obj.bytes.clone(),
            _ => {
                self.counters.conditional_rejected += 1;
                return Err(R2Error::ConditionFailed {
                    key: source_key.to_string(),
                });
            }
        };
        let etag = self.put(dest_key, &bytes);
        self.counters.copies += 1;
        self.counters.conditional_accepted += 1;
        Ok(etag)
    }

    pub fn get(&mut self, key: &str) -> Result<(Vec<u8>, String), R2Error> {
        self.counters.gets += 1;
        match self.objects.get(key) {
            Some(obj) => Ok((obj.bytes.clone(), obj.etag.clone())),
            None => Err(R2Error::NotFound {
                key: key.to_string(),
            }),
        }
    }

    /// Checksum of the stored object, recomputed from its bytes —
    /// the native checksum leg never trusts a stored digest.
    pub fn checksum(&mut self, key: &str) -> Result<String, R2Error> {
        self.counters.checksums += 1;
        match self.objects.get(key) {
            Some(obj) => Ok(content_id(&obj.bytes)),
            None => Err(R2Error::NotFound {
                key: key.to_string(),
            }),
        }
    }

    /// Plain (unconditional) delete of one exact key.
    pub fn delete(&mut self, key: &str) -> bool {
        let removed = self.objects.remove(key).is_some();
        if removed {
            self.counters.deletes += 1;
        }
        removed
    }

    /// Conditional DeleteObject is unsupported in R2's verified
    /// capability surface. Every call is refused as unsupported and
    /// counted; nothing is deleted.
    pub fn delete_conditional(&mut self, _key: &str, _if_match_etag: &str) -> Result<(), R2Error> {
        self.counters.unsupported += 1;
        Err(R2Error::Unsupported {
            operation: "conditional DeleteObject".to_string(),
        })
    }

    pub fn contains(&self, key: &str) -> bool {
        self.objects.contains_key(key)
    }
}

#[derive(Default)]
struct ContentState {
    current: u64,
    keys: BTreeMap<u64, String>,
    tombstoned: HashSet<u64>,
}

/// The R2 workload adapter: generation-qualified locators and
/// lifecycle over a `FakeR2`, with the delete discipline the issue
/// mandates.
#[derive(Default)]
pub struct R2Adapter {
    store: FakeR2,
    contents: HashMap<String, ContentState>,
    ever_used_keys: HashSet<String>,
    captured_keys: HashMap<String, (String, u64)>,
    owner_fence_proven: bool,
}

impl R2Adapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn counters(&self) -> Counters {
        self.store.counters()
    }

    /// Record the #9084 owner-fence proof. Until this is called,
    /// every captured-key delete is refused: the fence is an
    /// external authority's proof, never an adapter assumption.
    pub fn prove_owner_fence(&mut self) {
        self.owner_fence_proven = true;
    }

    fn mint_key(&mut self, cid: &str, generation: u64) -> Result<String, R2Error> {
        let key = physical_key(cid, generation);
        if !self.ever_used_keys.insert(key.clone()) {
            return Err(R2Error::KeyReuse { key });
        }
        Ok(key)
    }

    /// Put an object's bytes. New content lands at generation 1;
    /// re-putting live content is idempotent (same locator); putting
    /// tombstoned content back is an explicit resurrection at the
    /// next generation under a fresh, never-used key.
    pub fn put_object(&mut self, bytes: &[u8]) -> Result<Locator, R2Error> {
        let cid = content_id(bytes);
        if let Some(state) = self.contents.get(&cid) {
            let generation = state.current;
            let key = state.keys[&generation].clone();
            if !state.tombstoned.contains(&generation) {
                return Ok(Locator {
                    content_id: cid,
                    generation,
                    key,
                });
            }
            let generation = generation + 1;
            let key = self.mint_key(&cid, generation)?;
            self.store.put_if_absent(&key, bytes)?;
            let state = self.contents.get_mut(&cid).expect("content state");
            state.keys.insert(generation, key.clone());
            state.current = generation;
            return Ok(Locator {
                content_id: cid,
                generation,
                key,
            });
        }
        let generation = 1;
        let key = self.mint_key(&cid, generation)?;
        self.store.put_if_absent(&key, bytes)?;
        let mut keys = BTreeMap::new();
        keys.insert(generation, key.clone());
        self.contents.insert(
            cid.clone(),
            ContentState {
                current: generation,
                keys,
                tombstoned: HashSet::new(),
            },
        );
        Ok(Locator {
            content_id: cid,
            generation,
            key,
        })
    }

    /// Get the bytes addressed by a locator (its captured generation
    /// key — a stale locator still addresses its own generation).
    pub fn get_object(&mut self, locator: &Locator) -> Result<Vec<u8>, R2Error> {
        let (bytes, _etag) = self.store.get(&locator.key)?;
        Ok(bytes)
    }

    /// Checksum the object a locator addresses and compare it with
    /// the locator's content identity.
    pub fn checksum_matches(&mut self, locator: &Locator) -> Result<bool, R2Error> {
        let digest = self.store.checksum(&locator.key)?;
        Ok(digest == locator.content_id)
    }

    /// Tombstone the current generation of a content and capture
    /// its key for a possible later fenced plain delete.
    pub fn tombstone(&mut self, content_id_: &str) -> Result<Locator, R2Error> {
        let state = self
            .contents
            .get_mut(content_id_)
            .ok_or_else(|| R2Error::NotFound {
                key: content_id_.to_string(),
            })?;
        let generation = state.current;
        let key = state.keys[&generation].clone();
        state.tombstoned.insert(generation);
        self.captured_keys
            .insert(key.clone(), (content_id_.to_string(), generation));
        Ok(Locator {
            content_id: content_id_.to_string(),
            generation,
            key,
        })
    }

    /// Plain-delete a captured generation key. Refused unless the
    /// key was captured at tombstone, the #9084 owner fence is
    /// proven, and the key is not the content's live current key —
    /// so a stale delete can only ever remove its own captured old
    /// generation and can never alias a newer one.
    pub fn delete_captured_key(&mut self, locator: &Locator) -> Result<bool, R2Error> {
        let refuse = |reason: &str| R2Error::DeleteRefused {
            reason: reason.to_string(),
        };
        if !self.owner_fence_proven {
            self.store.counters.deletes_refused += 1;
            return Err(refuse("owner fence (#9084) not proven"));
        }
        match self.captured_keys.get(&locator.key) {
            Some((cid, generation))
                if *cid == locator.content_id && *generation == locator.generation => {}
            _ => {
                self.store.counters.deletes_refused += 1;
                return Err(refuse("key was not captured at tombstone"));
            }
        }
        if let Some(state) = self.contents.get(&locator.content_id) {
            if state.current == locator.generation
                && !state.tombstoned.contains(&locator.generation)
            {
                self.store.counters.deletes_refused += 1;
                return Err(refuse("key is the live current generation"));
            }
        }
        let deleted = self.store.delete(&locator.key);
        Ok(deleted)
    }

    /// Conditional delete is unsupported — see the module docs.
    pub fn delete_conditional(
        &mut self,
        locator: &Locator,
        if_match_etag: &str,
    ) -> Result<(), R2Error> {
        self.store.delete_conditional(&locator.key, if_match_etag)
    }

    /// The delete capability statement, in the contract's terms.
    pub fn delete_support() -> &'static str {
        "conditional DeleteObject unsupported; plain delete of a captured \
         never-reused generation key only after the #9084 owner fence"
    }
}
