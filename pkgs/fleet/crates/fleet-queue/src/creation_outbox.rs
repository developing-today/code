//! creation-outbox — the bounded creation outbox pilot for fleet
//! GitHub filings (developing-today/code#3706; design delta task 8938,
//! Drew's bounded-pilot approval 2026-11-04).
//!
//! Producers enqueue immutable creation intents (issue / pull_request /
//! comment) into a durable table instead of calling `gh` directly.
//! One serial drainer claims the oldest due entry under a lease,
//! paces creates (minimum gap + hourly budget), honors Retry-After
//! with exponential backoff and a visible circuit breaker, embeds a
//! hidden outbox marker in every submitted body, and marks an entry
//! `done` only after the created object is read back and its
//! number/URL persisted. An ambiguous create (the 2026-11-01
//! silent-loss shape: exit 0, no output, no error) is never counted
//! done and never blindly retried: the entry goes `uncertain` and is
//! reconciled by its marker first.
//!
//! This module is the pure core: payload hashing, backoff, the
//! circuit breaker, the drain state machine, and the acceptance
//! trials as tests against a fake creator. Storage lives in the
//! Turso backend (`turso_queue`, table `creation_outbox`); the
//! drainer's GitHub leg lives in the client (`client::outbox`).
//! The markdown action-outbox is untouched and remains the explicit
//! prepared-action fallback.

use serde_json::{Map, Value, json};

/// Minimum seconds between two creates by the drainer (pilot ceiling
/// from the task-8938 design, not a claim about GitHub's thresholds).
pub const MIN_GAP_SECONDS: i64 = 2;
/// Shared account-wide creation budget per rolling hour (pilot).
pub const HOURLY_BUDGET: usize = 300;
/// Lease granted to a drainer claim, in seconds.
pub const LEASE_SECONDS: i64 = 120;
/// First backoff delay (seconds) after a rate limit / transient
/// failure with no Retry-After.
pub const BACKOFF_BASE_SECONDS: i64 = 30;
/// Backoff cap (seconds).
pub const BACKOFF_CAP_SECONDS: i64 = 3600;
/// Consecutive rate limits that open the circuit breaker.
pub const CIRCUIT_THRESHOLD: u32 = 3;
/// How long the circuit stays open once tripped (seconds).
pub const CIRCUIT_OPEN_SECONDS: i64 = 300;
/// Default maximum delivery attempts before an entry goes `blocked`
/// for human review.
pub const DEFAULT_MAX_ATTEMPTS: i64 = 8;

/// The creation kinds the pilot carries. Anything else is rejected
/// at enqueue: interactive human `gh` use, review submissions and
/// releases are out of scope (#3706).
pub const KINDS: [&str; 4] = ["issue", "pull_request", "issue_comment", "pr_comment"];

/// Entry states. `pending` = due or waiting for `available_at`;
/// `leased` = a drainer holds it; `uncertain` = a create may or may
/// not have landed — reconcile by marker before any retry;
/// `done` = created object verified by read-back; `blocked` =
/// attempts exhausted or an unresolvable uncertainty, needs a human.
pub const STATES: [&str; 5] = ["pending", "leased", "uncertain", "done", "blocked"];

/// The hidden marker embedded in every body the drainer submits.
/// Reconciliation searches for the marker text; the id inside it is
/// the outbox row id.
pub fn marker(outbox_id: &str) -> String {
    format!("<!-- fleet-outbox:{outbox_id} -->")
}

/// Extract the outbox id a marker in `body` names, if any.
pub fn marker_id(body: &str) -> Option<String> {
    let start = body.find("<!-- fleet-outbox:")? + "<!-- fleet-outbox:".len();
    let rest = &body[start..];
    let end = rest.find(" -->")?;
    let id = &rest[..end];
    if id.is_empty() || id.contains(char::is_whitespace) {
        return None;
    }
    Some(id.to_string())
}

// ---------------------------------------------------------------------------
// SHA-256 (no new workspace dependency for one hash: FIPS 180-4,
// verified against the standard test vectors in the tests below)
// ---------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// Canonical JSON for hashing: object keys sorted recursively, so
/// the same intent hashes identically regardless of key order.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                out.insert(k.clone(), canonical(&map[k]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// The payload hash stored on the row. Same key + same hash = replay
/// (return the original id); same key + different hash = conflict.
pub fn payload_hash(payload: &Value) -> String {
    sha256_hex(canonical(payload).to_string().as_bytes())
}

// ---------------------------------------------------------------------------
// Backoff + circuit breaker
// ---------------------------------------------------------------------------

/// Delay before the next attempt after a failure. A `Retry-After`
/// from GitHub is honored exactly (never write before the deadline);
/// otherwise exponential backoff `BASE * 2^(attempt-1)` capped at
/// `CAP`, scaled by a jitter factor in `[0.5, 1.5)` derived from
/// `jitter_unit` in `[0, 1)`.
pub fn backoff_delay(attempt: i64, retry_after: Option<i64>, jitter_unit: f64) -> i64 {
    if let Some(ra) = retry_after {
        return ra.max(1);
    }
    let shift = (attempt.max(1) - 1).min(20) as u32;
    let base = BACKOFF_BASE_SECONDS
        .saturating_mul(1i64 << shift)
        .min(BACKOFF_CAP_SECONDS);
    let jittered = (base as f64 * (0.5 + jitter_unit.clamp(0.0, 0.999_999))) as i64;
    jittered.clamp(1, BACKOFF_CAP_SECONDS)
}

/// The account-wide circuit breaker: after CIRCUIT_THRESHOLD
/// consecutive rate limits it opens for CIRCUIT_OPEN_SECONDS and no
/// create is attempted while it is open. Persisted by the Turso
/// backend so every drainer sees the same circuit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Circuit {
    pub consecutive_rate_limits: u32,
    pub open_until: i64,
    pub last_error_class: Option<String>,
}

impl Circuit {
    pub fn is_open(&self, now: i64) -> bool {
        self.open_until > now
    }

    pub fn record_rate_limit(&mut self, now: i64) {
        self.consecutive_rate_limits += 1;
        self.last_error_class = Some("rate_limit".to_string());
        if self.consecutive_rate_limits >= CIRCUIT_THRESHOLD {
            self.open_until = now + CIRCUIT_OPEN_SECONDS;
        }
    }

    pub fn record_success(&mut self) {
        self.consecutive_rate_limits = 0;
        self.open_until = 0;
        self.last_error_class = None;
    }
}

// ---------------------------------------------------------------------------
// Entries, store, creator — the drain state machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub repo: String,
    pub kind: String,
    pub payload: Value,
    pub dedup_key: String,
    pub payload_hash: String,
    pub producer: Option<String>,
    pub state: String,
    pub created_at: i64,
    pub available_at: i64,
    pub attempt_count: i64,
    pub max_attempts: i64,
    pub last_error_class: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<i64>,
    pub github_number: Option<i64>,
    pub github_url: Option<String>,
    pub completed_at: Option<i64>,
}

impl Entry {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "repo": self.repo,
            "kind": self.kind,
            "payload": self.payload,
            "dedup_key": self.dedup_key,
            "payload_hash": self.payload_hash,
            "producer": self.producer,
            "state": self.state,
            "created_at": self.created_at,
            "available_at": self.available_at,
            "attempt_count": self.attempt_count,
            "max_attempts": self.max_attempts,
            "last_error_class": self.last_error_class,
            "lease_owner": self.lease_owner,
            "lease_until": self.lease_until,
            "github_number": self.github_number,
            "github_url": self.github_url,
            "completed_at": self.completed_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EnqueueOutcome {
    /// New row inserted.
    Created(Entry),
    /// Same (repo, dedup_key) and same payload hash: original row.
    Replay(Entry),
    /// Same (repo, dedup_key) but a different payload hash.
    Conflict { existing_id: String },
}

/// Storage the drainer state machine needs. The Turso backend
/// implements the same semantics in SQL (`turso_queue`); the
/// in-memory store here is the test oracle for the trials.
pub trait Store {
    fn enqueue(
        &mut self,
        repo: &str,
        kind: &str,
        payload: Value,
        dedup_key: &str,
        producer: Option<&str>,
        now: i64,
    ) -> Result<EnqueueOutcome, String>;
    /// Claim the oldest due entry (state pending, available_at <=
    /// now, plus expired leases and expired uncertain reconciliations
    /// are the caller's `now` view — see `claim_oldest_due`). Returns
    /// None when nothing is due.
    fn claim_oldest_due(&mut self, owner: &str, now: i64) -> Option<Entry>;
    fn get(&self, id: &str) -> Option<Entry>;
    fn save(&mut self, entry: &Entry);
    fn all(&self) -> Vec<Entry>;
}

/// In-memory store: the trial harness's storage, and the reference
/// semantics for the SQL implementation.
#[derive(Default)]
pub struct MemStore {
    entries: Vec<Entry>,
    seq: u64,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for MemStore {
    fn enqueue(
        &mut self,
        repo: &str,
        kind: &str,
        payload: Value,
        dedup_key: &str,
        producer: Option<&str>,
        now: i64,
    ) -> Result<EnqueueOutcome, String> {
        if !KINDS.contains(&kind) {
            return Err(format!("kind {kind:?} is not an outbox creation kind"));
        }
        let hash = payload_hash(&payload);
        if let Some(existing) = self
            .entries
            .iter()
            .find(|e| e.repo == repo && e.dedup_key == dedup_key)
        {
            if existing.payload_hash == hash {
                return Ok(EnqueueOutcome::Replay(existing.clone()));
            }
            return Ok(EnqueueOutcome::Conflict {
                existing_id: existing.id.clone(),
            });
        }
        self.seq += 1;
        let entry = Entry {
            id: format!("ob-{}-{}", now, self.seq),
            repo: repo.to_string(),
            kind: kind.to_string(),
            payload,
            dedup_key: dedup_key.to_string(),
            payload_hash: hash,
            producer: producer.map(|s| s.to_string()),
            state: "pending".to_string(),
            created_at: now,
            available_at: now,
            attempt_count: 0,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            last_error_class: None,
            lease_owner: None,
            lease_until: None,
            github_number: None,
            github_url: None,
            completed_at: None,
        };
        self.entries.push(entry.clone());
        Ok(EnqueueOutcome::Created(entry))
    }

    fn claim_oldest_due(&mut self, owner: &str, now: i64) -> Option<Entry> {
        // Due = pending and available; or leased with an expired
        // lease (a crashed drainer — the kill/restart trial); or
        // uncertain whose reconciliation delay has elapsed (the
        // caller reconciles before any re-create: see Drainer).
        let idx = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (e.state == "pending" && e.available_at <= now)
                    || (e.state == "leased" && e.lease_until.map(|u| u <= now).unwrap_or(false))
                    || (e.state == "uncertain" && e.available_at <= now)
            })
            .min_by_key(|(_, e)| e.created_at)
            .map(|(i, _)| i)?;
        let e = &mut self.entries[idx];
        if e.state == "leased" {
            // An expired lease means the previous drainer died
            // mid-flight: its create may have landed. The row is
            // uncertain until the marker search proves otherwise —
            // never re-create it blind (kill/restart trial).
            e.state = "uncertain".to_string();
            e.available_at = now;
        }
        if e.state != "uncertain" {
            e.state = "leased".to_string();
        }
        e.lease_owner = Some(owner.to_string());
        e.lease_until = Some(now + LEASE_SECONDS);
        Some(e.clone())
    }

    fn get(&self, id: &str) -> Option<Entry> {
        self.entries.iter().find(|e| e.id == id).cloned()
    }

    fn save(&mut self, entry: &Entry) {
        if let Some(slot) = self.entries.iter_mut().find(|e| e.id == entry.id) {
            *slot = entry.clone();
        }
    }

    fn all(&self) -> Vec<Entry> {
        self.entries.clone()
    }
}

/// What one create attempt did, as the creator leg reports it.
#[derive(Debug, Clone, PartialEq)]
pub enum CreateOutcome {
    /// The create call returned a number/URL. NOT done until the
    /// read-back verifies it (`Creator::read_back`).
    Created { number: i64, url: String },
    /// 403/429. `retry_after` is GitHub's Retry-After, when given.
    RateLimited { retry_after: Option<i64> },
    /// The silent-loss shape: the call may have created the object
    /// but returned nothing usable (empty success, lost response).
    Ambiguous(String),
    /// A deterministic failure (bad payload, 404 target, ...).
    Failed(String),
}

/// The GitHub leg of the drainer. Production: `client::outbox`'s
/// gh-backed creator. Trials: the fake below.
pub trait Creator {
    fn create(&mut self, entry: &Entry) -> CreateOutcome;
    /// Verify the created object exists and carries this entry's
    /// marker. A `gh` exit 0 is never enough — this read-back is
    /// what marks an entry done.
    fn read_back(&mut self, entry: &Entry, number: i64) -> bool;
    /// Search for an object carrying this entry's marker (crash /
    /// ambiguous-create reconciliation).
    fn find_by_marker(&mut self, entry: &Entry) -> Option<(i64, String)>;
}

/// Observable drainer state: the metrics the design requires
/// (pending depth, oldest pending age, per-state counts, attempts,
/// last failure class, circuit state).
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    pub pending: usize,
    pub leased: usize,
    pub uncertain: usize,
    pub done: usize,
    pub blocked: usize,
    pub oldest_pending_age_seconds: Option<i64>,
    pub total_attempts: i64,
    pub last_error_class: Option<String>,
    pub circuit_open_until: Option<i64>,
}

pub fn metrics_of(entries: &[Entry], circuit: &Circuit, now: i64) -> Metrics {
    let count = |state: &str| entries.iter().filter(|e| e.state == state).count();
    let oldest_pending = entries
        .iter()
        .filter(|e| e.state == "pending" || e.state == "uncertain")
        .map(|e| e.created_at)
        .min();
    Metrics {
        pending: count("pending"),
        leased: count("leased"),
        uncertain: count("uncertain"),
        done: count("done"),
        blocked: count("blocked"),
        oldest_pending_age_seconds: oldest_pending.map(|t| (now - t).max(0)),
        total_attempts: entries.iter().map(|e| e.attempt_count).sum(),
        last_error_class: entries
            .iter()
            .filter_map(|e| e.last_error_class.clone())
            .last()
            .or_else(|| circuit.last_error_class.clone()),
        circuit_open_until: if circuit.is_open(now) {
            Some(circuit.open_until)
        } else {
            None
        },
    }
}

/// One drain step's outcome, for the caller's log / CLI output.
#[derive(Debug, Clone, PartialEq)]
pub enum StepOutcome {
    Idle,
    CircuitOpen { until: i64 },
    BudgetExhausted,
    Paced { wait_seconds: i64 },
    Done { id: String, number: i64 },
    Reconciled { id: String, number: i64 },
    RetryScheduled { id: String, delay_seconds: i64 },
    Uncertain { id: String },
    Blocked { id: String, reason: String },
}

/// The serial drainer. One instance drains in created_at order,
/// pacing creates and refusing to create while the circuit is open
/// or the hourly budget is spent.
pub struct Drainer {
    pub owner: String,
    pub circuit: Circuit,
    pub last_create_at: Option<i64>,
    pub create_times: Vec<i64>,
    pub jitter_unit: f64,
}

impl Drainer {
    pub fn new(owner: &str) -> Self {
        Drainer {
            owner: owner.to_string(),
            circuit: Circuit::default(),
            last_create_at: None,
            create_times: Vec::new(),
            jitter_unit: 0.5,
        }
    }

    fn budget_left(&self, now: i64) -> usize {
        let recent = self
            .create_times
            .iter()
            .filter(|t| now - **t < 3600)
            .count();
        HOURLY_BUDGET.saturating_sub(recent)
    }

    /// Run one step at logical time `now`. The caller persists the
    /// store + circuit after the step (the Turso drain does the
    /// equivalent with its claim/resolve verbs).
    pub fn step<S: Store, C: Creator>(
        &mut self,
        store: &mut S,
        creator: &mut C,
        now: i64,
    ) -> StepOutcome {
        if self.circuit.is_open(now) {
            return StepOutcome::CircuitOpen {
                until: self.circuit.open_until,
            };
        }
        let Some(mut entry) = store.claim_oldest_due(&self.owner, now) else {
            return StepOutcome::Idle;
        };

        // Uncertain entries reconcile FIRST: never re-create until
        // the marker search proves the object absent.
        if entry.state == "uncertain" {
            if let Some((number, url)) = creator.find_by_marker(&entry) {
                if creator.read_back(&entry, number) {
                    entry.state = "done".to_string();
                    entry.github_number = Some(number);
                    entry.github_url = Some(url.clone());
                    entry.completed_at = Some(now);
                    entry.lease_owner = None;
                    entry.lease_until = None;
                    store.save(&entry);
                    self.circuit.record_success();
                    return StepOutcome::Reconciled {
                        id: entry.id,
                        number,
                    };
                }
            }
            // Marker search found nothing (or read-back failed):
            // the create provably did not land — fall through and
            // re-create under the same entry. If attempts are gone,
            // a human decides.
            if entry.attempt_count >= entry.max_attempts {
                entry.state = "blocked".to_string();
                entry.last_error_class = Some("uncertain_unresolved".to_string());
                entry.lease_owner = None;
                entry.lease_until = None;
                store.save(&entry);
                return StepOutcome::Blocked {
                    id: entry.id,
                    reason: "uncertain entry exhausted attempts without a marker match".to_string(),
                };
            }
            entry.state = "leased".to_string();
            store.save(&entry);
        }

        // Pacing: minimum gap + hourly budget, checked before the
        // create so a paced step never holds a create mid-flight.
        if let Some(last) = self.last_create_at {
            let wait = MIN_GAP_SECONDS - (now - last);
            if wait > 0 {
                entry.state = "pending".to_string();
                entry.available_at = last + MIN_GAP_SECONDS;
                entry.lease_owner = None;
                entry.lease_until = None;
                store.save(&entry);
                return StepOutcome::Paced { wait_seconds: wait };
            }
        }
        if self.budget_left(now) == 0 {
            entry.state = "pending".to_string();
            entry.available_at = now + 60;
            entry.lease_owner = None;
            entry.lease_until = None;
            store.save(&entry);
            return StepOutcome::BudgetExhausted;
        }

        entry.attempt_count += 1;
        self.last_create_at = Some(now);
        self.create_times.push(now);
        match creator.create(&entry) {
            CreateOutcome::Created { number, url } => {
                if creator.read_back(&entry, number) {
                    entry.state = "done".to_string();
                    entry.github_number = Some(number);
                    entry.github_url = Some(url);
                    entry.completed_at = Some(now);
                    entry.last_error_class = None;
                    entry.lease_owner = None;
                    entry.lease_until = None;
                    store.save(&entry);
                    self.circuit.record_success();
                    StepOutcome::Done {
                        id: entry.id,
                        number,
                    }
                } else {
                    // Claimed success that does not read back is the
                    // silent-loss shape, not a success.
                    entry.state = "uncertain".to_string();
                    entry.last_error_class = Some("readback_failed".to_string());
                    entry.available_at = now + BACKOFF_BASE_SECONDS;
                    entry.lease_owner = None;
                    entry.lease_until = None;
                    store.save(&entry);
                    StepOutcome::Uncertain { id: entry.id }
                }
            }
            CreateOutcome::RateLimited { retry_after } => {
                self.circuit.record_rate_limit(now);
                let delay = backoff_delay(entry.attempt_count, retry_after, self.jitter_unit);
                entry.state = "pending".to_string();
                entry.last_error_class = Some("rate_limit".to_string());
                entry.available_at = now + delay;
                entry.lease_owner = None;
                entry.lease_until = None;
                store.save(&entry);
                StepOutcome::RetryScheduled {
                    id: entry.id,
                    delay_seconds: delay,
                }
            }
            CreateOutcome::Ambiguous(why) => {
                entry.state = "uncertain".to_string();
                entry.last_error_class = Some(format!("ambiguous: {why}"));
                entry.available_at = now + BACKOFF_BASE_SECONDS;
                entry.lease_owner = None;
                entry.lease_until = None;
                store.save(&entry);
                StepOutcome::Uncertain { id: entry.id }
            }
            CreateOutcome::Failed(why) => {
                entry.last_error_class = Some(format!("failed: {why}"));
                entry.lease_owner = None;
                entry.lease_until = None;
                if entry.attempt_count >= entry.max_attempts {
                    entry.state = "blocked".to_string();
                    let id = entry.id.clone();
                    store.save(&entry);
                    StepOutcome::Blocked { id, reason: why }
                } else {
                    let delay = backoff_delay(entry.attempt_count, None, self.jitter_unit);
                    entry.state = "pending".to_string();
                    entry.available_at = now + delay;
                    let id = entry.id.clone();
                    store.save(&entry);
                    StepOutcome::RetryScheduled {
                        id,
                        delay_seconds: delay,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake GitHub: records every created object (with the marker
    /// in its body, as the real drainer submits it), and can be
    /// scripted into the failure shapes the trials exercise.
    #[derive(Default)]
    struct FakeCreator {
        objects: Vec<(i64, String, String)>, // (number, url, body)
        next_number: i64,
        script: Vec<CreateOutcome>,
        rate_limit: Option<Option<i64>>,
        ambiguous_but_creates: bool,
        readback_ok: bool,
    }

    impl FakeCreator {
        fn new() -> Self {
            FakeCreator {
                next_number: 100,
                readback_ok: true,
                ..Default::default()
            }
        }

        fn land(&mut self, entry: &Entry) -> CreateOutcome {
            self.next_number += 1;
            let number = self.next_number;
            let url = format!("https://github.com/{}/issues/{number}", entry.repo);
            let body = format!(
                "{}\n{}",
                entry
                    .payload
                    .get("body")
                    .and_then(|b| b.as_str())
                    .unwrap_or(""),
                marker(&entry.id)
            );
            self.objects.push((number, url.clone(), body));
            CreateOutcome::Created { number, url }
        }
    }

    impl Creator for FakeCreator {
        fn create(&mut self, entry: &Entry) -> CreateOutcome {
            if let Some(rl) = self.rate_limit {
                return CreateOutcome::RateLimited { retry_after: rl };
            }
            if !self.script.is_empty() {
                let outcome = self.script.remove(0);
                if outcome == CreateOutcome::Ambiguous("empty success".to_string())
                    && self.ambiguous_but_creates
                {
                    let _ = self.land(entry);
                }
                return outcome;
            }
            self.land(entry)
        }

        fn read_back(&mut self, entry: &Entry, number: i64) -> bool {
            self.readback_ok
                && self
                    .objects
                    .iter()
                    .any(|(n, _, body)| *n == number && marker_id(body) == Some(entry.id.clone()))
        }

        fn find_by_marker(&mut self, entry: &Entry) -> Option<(i64, String)> {
            self.objects
                .iter()
                .find(|(_, _, body)| marker_id(body) == Some(entry.id.clone()))
                .map(|(n, url, _)| (*n, url.clone()))
        }
    }

    fn payload(title: &str) -> Value {
        json!({"title": title, "body": format!("body of {title}"), "labels": ["queue/queued"]})
    }

    #[test]
    fn sha256_matches_fips_vectors() {
        // Oracle: FIPS 180-4 example vectors.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn payload_hash_is_key_order_independent() {
        // Oracle: the same intent written with different key order
        // is the same intent (replay), never a conflict.
        let a = json!({"title": "t", "body": "b", "labels": ["x"]});
        let b = json!({"labels": ["x"], "body": "b", "title": "t"});
        assert_eq!(payload_hash(&a), payload_hash(&b));
        let c = json!({"title": "t", "body": "CHANGED", "labels": ["x"]});
        assert_ne!(payload_hash(&a), payload_hash(&c));
    }

    #[test]
    fn marker_round_trips() {
        let m = marker("ob-123-1");
        assert_eq!(
            marker_id(&format!("text {m} tail")),
            Some("ob-123-1".to_string())
        );
        assert_eq!(marker_id("no marker here"), None);
    }

    #[test]
    fn dedupe_same_payload_replays_and_different_payload_conflicts() {
        // Acceptance oracle (#3706): duplicate key + same payload
        // returns one id; same key + different payload conflicts
        // instead of overwriting.
        let mut store = MemStore::new();
        let first = store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("a"),
                "k1",
                None,
                1000,
            )
            .unwrap();
        let id = match first {
            EnqueueOutcome::Created(e) => e.id,
            other => panic!("expected Created, got {other:?}"),
        };
        match store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("a"),
                "k1",
                None,
                1001,
            )
            .unwrap()
        {
            EnqueueOutcome::Replay(e) => assert_eq!(e.id, id),
            other => panic!("expected Replay, got {other:?}"),
        }
        match store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("b"),
                "k1",
                None,
                1002,
            )
            .unwrap()
        {
            EnqueueOutcome::Conflict { existing_id } => assert_eq!(existing_id, id),
            other => panic!("expected Conflict, got {other:?}"),
        }
        assert_eq!(store.all().len(), 1);
    }

    #[test]
    fn trial_50_item_burst_drains_serially_with_no_loss_or_duplicate() {
        // Trial 1 oracle (#3706): 50 distinct intents all receive
        // ids, drain serially within the configured budget, every
        // object verified by read-back, no duplicate and no loss.
        let mut store = MemStore::new();
        let mut fake = FakeCreator::new();
        let mut drainer = Drainer::new("trial");
        let mut ids = Vec::new();
        for i in 0..50 {
            match store
                .enqueue(
                    "developing-today/code",
                    "issue",
                    payload(&format!("burst {i}")),
                    &format!("burst-{i}"),
                    Some("new-issue-creator-code-a"),
                    1000,
                )
                .unwrap()
            {
                EnqueueOutcome::Created(e) => ids.push(e.id),
                other => panic!("burst enqueue {i}: {other:?}"),
            }
        }
        let mut now = 1000;
        let mut done = 0;
        for _ in 0..500 {
            match drainer.step(&mut store, &mut fake, now) {
                StepOutcome::Done { .. } => done += 1,
                StepOutcome::Paced { wait_seconds } => now += wait_seconds,
                StepOutcome::Idle => break,
                other => panic!("burst drain: unexpected {other:?}"),
            }
            if done == 50 {
                break;
            }
        }
        assert_eq!(done, 50);
        assert_eq!(fake.objects.len(), 50, "exactly one object per intent");
        assert!(
            store
                .all()
                .iter()
                .all(|e| e.state == "done" && e.github_number.is_some()),
            "every entry done with a persisted verified number"
        );
        let m = metrics_of(&store.all(), &drainer.circuit, now);
        assert_eq!(m.pending, 0);
        assert_eq!(m.done, 50);
    }

    #[test]
    fn trial_kill_restart_reconciles_the_leased_row_by_marker() {
        // Trial 2 oracle (#3706): kill the drainer mid-burst (drop
        // it with a row leased and a create landed but unresolved),
        // restart after lease expiry: verified rows stay done, the
        // in-flight row reconciles by marker, no duplicate per key.
        let mut store = MemStore::new();
        let mut fake = FakeCreator::new();
        for i in 0..3 {
            store
                .enqueue(
                    "developing-today/code",
                    "issue",
                    payload(&format!("kr {i}")),
                    &format!("kr-{i}"),
                    None,
                    1000,
                )
                .unwrap();
        }
        // First drainer: completes row 1, then "dies" holding row 2
        // leased with its create landed but never resolved.
        {
            let mut d1 = Drainer::new("drainer-1");
            let mut now = 1000;
            loop {
                match d1.step(&mut store, &mut fake, now) {
                    StepOutcome::Done { .. } => break,
                    StepOutcome::Paced { wait_seconds } => now += wait_seconds,
                    other => panic!("setup drain: {other:?}"),
                }
            }
            let claimed = store.claim_oldest_due("drainer-1", now).unwrap();
            let _ = fake.land(&claimed); // create landed; response lost in the kill
        }
        let objects_before = fake.objects.len();
        // Restart after lease expiry with a fresh drainer.
        let mut d2 = Drainer::new("drainer-2");
        let mut now = 1000 + LEASE_SECONDS + MIN_GAP_SECONDS + 1;
        let mut reconciled = 0;
        let mut done = 0;
        for _ in 0..100 {
            match d2.step(&mut store, &mut fake, now) {
                StepOutcome::Reconciled { .. } => reconciled += 1,
                StepOutcome::Done { .. } => done += 1,
                StepOutcome::Paced { wait_seconds } => now += wait_seconds,
                StepOutcome::Idle => break,
                other => panic!("restart drain: {other:?}"),
            }
        }
        assert_eq!(
            reconciled, 1,
            "the killed row reconciles by marker, not re-create"
        );
        assert_eq!(done, 1, "the remaining row drains normally");
        assert_eq!(
            fake.objects.len(),
            objects_before + 1,
            "exactly one new object after restart (the never-created row)"
        );
        assert!(store.all().iter().all(|e| e.state == "done"));
    }

    #[test]
    fn trial_forced_403_honors_retry_after_and_the_circuit() {
        // Trial 3 oracle (#3706): injected 403 with Retry-After —
        // no write before the deadline, the entry stays visible
        // with its error class, the circuit opens after repeated
        // 403s and no later row bypasses it; delivery resumes when
        // the fault clears. Repeated without Retry-After below.
        let mut store = MemStore::new();
        let mut fake = FakeCreator::new();
        fake.rate_limit = Some(Some(120));
        store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("rl"),
                "rl-1",
                None,
                1000,
            )
            .unwrap();
        let mut drainer = Drainer::new("trial");
        match drainer.step(&mut store, &mut fake, 1000) {
            StepOutcome::RetryScheduled { delay_seconds, .. } => {
                assert_eq!(delay_seconds, 120, "Retry-After is honored exactly")
            }
            other => panic!("expected retry, got {other:?}"),
        }
        let entry = store.all().into_iter().next().unwrap();
        assert_eq!(entry.state, "pending");
        assert_eq!(entry.available_at, 1120, "no write before the deadline");
        assert_eq!(entry.last_error_class.as_deref(), Some("rate_limit"));
        assert_eq!(fake.objects.len(), 0);

        // Two more rate limits trip the circuit; a step while it is
        // open creates nothing.
        drainer.step(&mut store, &mut fake, 1120);
        drainer.step(&mut store, &mut fake, 1240);
        assert!(drainer.circuit.is_open(1240));
        match drainer.step(&mut store, &mut fake, 1241) {
            StepOutcome::CircuitOpen { .. } => {}
            other => panic!("expected circuit open, got {other:?}"),
        }
        assert_eq!(fake.objects.len(), 0, "no row bypasses the open circuit");

        // Fault clears, circuit expires: delivery resumes.
        fake.rate_limit = None;
        let after = drainer.circuit.open_until + MIN_GAP_SECONDS + 1;
        match drainer.step(&mut store, &mut fake, after) {
            StepOutcome::Done { .. } => {}
            other => panic!("expected done after circuit clears, got {other:?}"),
        }
    }

    #[test]
    fn trial_forced_403_without_retry_after_uses_backoff() {
        // Trial 3b oracle: no Retry-After → exponential backoff,
        // entry remains pending and visible, nothing created.
        let mut store = MemStore::new();
        let mut fake = FakeCreator::new();
        fake.rate_limit = Some(None);
        store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("rl2"),
                "rl-2",
                None,
                2000,
            )
            .unwrap();
        let mut drainer = Drainer::new("trial");
        drainer.jitter_unit = 0.5; // jitter factor exactly 1.0
        match drainer.step(&mut store, &mut fake, 2000) {
            StepOutcome::RetryScheduled { delay_seconds, .. } => {
                assert_eq!(delay_seconds, BACKOFF_BASE_SECONDS)
            }
            other => panic!("expected retry, got {other:?}"),
        }
        assert_eq!(fake.objects.len(), 0);
        assert_eq!(store.all()[0].available_at, 2000 + BACKOFF_BASE_SECONDS);
    }

    #[test]
    fn trial_2026_11_01_ambiguous_success_is_never_counted_done_without_readback() {
        // Trial 4 oracle (#3706, the 2026-11-01 four-attempt
        // silent-loss replay): a gh-style empty success is not done
        // without read-back; when the object did land, the marker
        // recovers it — no silent loss, no duplicate.
        let mut store = MemStore::new();
        let mut fake = FakeCreator::new();
        fake.script = vec![CreateOutcome::Ambiguous("empty success".to_string())];
        fake.ambiguous_but_creates = true; // the POST actually landed
        store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("amb"),
                "amb-1",
                None,
                3000,
            )
            .unwrap();
        let mut drainer = Drainer::new("trial");
        match drainer.step(&mut store, &mut fake, 3000) {
            StepOutcome::Uncertain { .. } => {}
            other => panic!("expected uncertain, got {other:?}"),
        }
        assert_eq!(store.all()[0].state, "uncertain");
        assert_eq!(store.all()[0].github_number, None, "not counted done");
        // After the reconciliation delay, the marker recovers it.
        let later = 3000 + BACKOFF_BASE_SECONDS + MIN_GAP_SECONDS;
        match drainer.step(&mut store, &mut fake, later) {
            StepOutcome::Reconciled { number, .. } => assert!(number > 100),
            other => panic!("expected reconciled, got {other:?}"),
        }
        assert_eq!(fake.objects.len(), 1, "no duplicate from reconciliation");
        assert_eq!(store.all()[0].state, "done");
    }

    #[test]
    fn ambiguous_create_that_never_landed_recreates_only_after_marker_search() {
        // Companion oracle: when the marker search proves absence,
        // the same entry re-creates — still exactly one object.
        let mut store = MemStore::new();
        let mut fake = FakeCreator::new();
        fake.script = vec![CreateOutcome::Ambiguous("lost response".to_string())];
        fake.ambiguous_but_creates = false;
        store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("amb2"),
                "amb-2",
                None,
                4000,
            )
            .unwrap();
        let mut drainer = Drainer::new("trial");
        assert!(matches!(
            drainer.step(&mut store, &mut fake, 4000),
            StepOutcome::Uncertain { .. }
        ));
        let later = 4000 + BACKOFF_BASE_SECONDS + MIN_GAP_SECONDS;
        assert!(matches!(
            drainer.step(&mut store, &mut fake, later),
            StepOutcome::Done { .. }
        ));
        assert_eq!(fake.objects.len(), 1);
    }

    #[test]
    fn readback_failure_turns_a_claimed_success_uncertain() {
        // Oracle: a create whose object cannot be read back with
        // the entry's marker is not a success (a gh exit 0 never
        // marks done).
        let mut store = MemStore::new();
        let mut fake = FakeCreator::new();
        fake.readback_ok = false;
        store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("rb"),
                "rb-1",
                None,
                5000,
            )
            .unwrap();
        let mut drainer = Drainer::new("trial");
        assert!(matches!(
            drainer.step(&mut store, &mut fake, 5000),
            StepOutcome::Uncertain { .. }
        ));
        assert_eq!(store.all()[0].state, "uncertain");
    }

    #[test]
    fn backoff_grows_caps_and_retry_after_wins() {
        // Oracle: backoff doubles per attempt, caps, and a
        // Retry-After always wins over the computed delay.
        assert_eq!(backoff_delay(1, None, 0.5), 30);
        assert_eq!(backoff_delay(2, None, 0.5), 60);
        assert_eq!(backoff_delay(3, None, 0.5), 120);
        assert_eq!(backoff_delay(30, None, 0.5), BACKOFF_CAP_SECONDS);
        assert_eq!(backoff_delay(1, Some(7), 0.5), 7);
        assert_eq!(backoff_delay(9, Some(900), 0.5), 900);
    }

    #[test]
    fn metrics_report_depth_and_oldest_age() {
        // Oracle: metrics expose pending depth + oldest pending age
        // (the alert fields the design requires).
        let mut store = MemStore::new();
        store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("m1"),
                "m-1",
                None,
                100,
            )
            .unwrap();
        store
            .enqueue(
                "developing-today/code",
                "issue",
                payload("m2"),
                "m-2",
                None,
                200,
            )
            .unwrap();
        let m = metrics_of(&store.all(), &Circuit::default(), 460);
        assert_eq!(m.pending, 2);
        assert_eq!(m.oldest_pending_age_seconds, Some(360));
    }
}
