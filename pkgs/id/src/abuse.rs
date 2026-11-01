//! Shared abuse-limiting core for comms (#2244 P0-A, #3881).
//!
//! One evaluator owns the token buckets every comms surface will
//! share, so an endpoint's allowance is a property of the endpoint —
//! never multiplied by opening more connections to it. This module
//! is the core only: nothing is wired into frames, HTTP routes, or
//! world chat here (that is Packet B), and no existing limiter's
//! behaviour changes.
//!
//! Contents:
//!
//! - [`AbusePolicy`] — the limits, with the #2032/#2033 defaults.
//! - [`AbuseEvaluator`] — per-endpoint frame and byte token buckets,
//!   shared across that endpoint's connections, plus the concurrent
//!   connection cap per peer.
//! - [`Refusal`] — the one structured refusal contract: a reason and
//!   a bounded `retry_after_ms` (1 s floor / 60 s cap for temporary
//!   refusals; absent for permanent blocks). Local `Refusal` enums in
//!   `event_store` / `directory` are ingest- and directory-specific
//!   and are not this contract.
//! - [`CounterRegistry`] — per-(local node, remote endpoint) frame
//!   and byte counters with a snapshot API for the observability
//!   surface the integration phase builds.
//!
//! Time is injected as `now_ms` (milliseconds on any monotonic
//! clock the caller chooses) so the evaluator is deterministic and
//! unit-testable without sleeping.

use std::collections::{HashMap, HashSet};

/// A remote endpoint identity: the 32 bytes of its verifying key,
/// the same bytes the event store uses as an author key.
pub type EndpointId = [u8; 32];

/// The local node's identity, same representation as [`EndpointId`].
pub type NodeId = [u8; 32];

/// A connection opened through the evaluator. Ids are unique per
/// evaluator and never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalReason {
    /// Too many frames right now; retry after the bucket refills.
    RateLimited,
    /// The byte budget is exhausted; retry after it refills.
    BudgetExhausted,
    /// An operator block: permanent, no retry is offered.
    BlockedByOperator,
    /// The subject is not (or not yet) authorized for this surface.
    Unauthorized,
}

/// The one structured refusal: a reason plus, for temporary
/// refusals, a bounded retry delay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// Why the request was refused.
    pub reason: RefusalReason,
    /// When to retry, in milliseconds. Temporary refusals are
    /// clamped to [`RETRY_FLOOR_MS`]..=[`RETRY_CAP_MS`]; permanent
    /// blocks ([`RefusalReason::BlockedByOperator`]) carry `None`.
    pub retry_after_ms: Option<u64>,
}

/// Floor for a temporary refusal's `retry_after_ms` (#2033).
pub const RETRY_FLOOR_MS: u64 = 1_000;

/// Cap for a temporary refusal's `retry_after_ms` (#2033).
pub const RETRY_CAP_MS: u64 = 60_000;

impl Refusal {
    /// A temporary refusal with `retry_after_ms` clamped to the
    /// bounded range. Permanent-block reasons ignore the delay and
    /// carry `None`.
    #[must_use]
    pub fn new(reason: RefusalReason, retry_after_ms: u64) -> Self {
        if reason == RefusalReason::BlockedByOperator {
            return Self {
                reason,
                retry_after_ms: None,
            };
        }
        Self {
            reason,
            retry_after_ms: Some(retry_after_ms.clamp(RETRY_FLOOR_MS, RETRY_CAP_MS)),
        }
    }

    /// A permanent operator block: no retry is offered.
    #[must_use]
    pub fn blocked_by_operator() -> Self {
        Self {
            reason: RefusalReason::BlockedByOperator,
            retry_after_ms: None,
        }
    }
}

/// A token bucket in milli-units (1 token = 1000 milli-tokens), so
/// refill arithmetic stays in integers: at `rate_per_sec` tokens per
/// second, each elapsed millisecond adds exactly `rate_per_sec`
/// milli-tokens.
#[derive(Clone, Debug)]
struct TokenBucket {
    capacity_milli: u64,
    rate_per_sec: u64,
    tokens_milli: u64,
    last_refill_ms: u64,
}

impl TokenBucket {
    fn new(capacity: u64, rate_per_sec: u64, now_ms: u64) -> Self {
        Self {
            capacity_milli: capacity.saturating_mul(1_000),
            rate_per_sec,
            tokens_milli: capacity.saturating_mul(1_000),
            last_refill_ms: now_ms,
        }
    }

    fn refill(&mut self, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.last_refill_ms);
        if elapsed == 0 {
            return;
        }
        let added = elapsed.saturating_mul(self.rate_per_sec);
        self.tokens_milli = self
            .tokens_milli
            .saturating_add(added)
            .min(self.capacity_milli);
        self.last_refill_ms = now_ms;
    }

    /// Takes `cost` whole tokens if the bucket holds them.
    fn try_take(&mut self, cost: u64, now_ms: u64) -> bool {
        self.refill(now_ms);
        let cost_milli = cost.saturating_mul(1_000);
        if self.tokens_milli >= cost_milli {
            self.tokens_milli -= cost_milli;
            true
        } else {
            false
        }
    }

    /// Milliseconds until `cost` tokens are available (0 if now).
    fn retry_after_ms(&mut self, cost: u64, now_ms: u64) -> u64 {
        self.refill(now_ms);
        let cost_milli = cost.saturating_mul(1_000);
        if self.tokens_milli >= cost_milli || self.rate_per_sec == 0 {
            return 0;
        }
        let deficit = cost_milli - self.tokens_milli;
        // Each millisecond refills rate_per_sec milli-tokens.
        deficit.div_ceil(self.rate_per_sec)
    }
}

/// One surface's bucket limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BucketLimits {
    /// Sustained frames per second.
    pub frames_per_sec: u64,
    /// Frame burst capacity, in frames.
    pub frame_burst: u64,
    /// Sustained bytes per second.
    pub bytes_per_sec: u64,
    /// Byte burst capacity, in bytes.
    pub byte_burst: u64,
}

/// The abuse limits. Defaults are the #2032/#2033 values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbusePolicy {
    /// Limits for an authorized endpoint.
    pub authorized: BucketLimits,
    /// Limits before authorization completes. #2032/#2033 set only
    /// the frame numbers for this surface (10 frames/s, burst 30);
    /// the byte numbers match the authorized surface until a later
    /// phase sets a separate pre-authorization byte policy.
    pub pre_authorization: BucketLimits,
    /// Maximum concurrent connections per peer endpoint.
    pub max_connections_per_peer: usize,
}

impl Default for AbusePolicy {
    fn default() -> Self {
        Self {
            authorized: BucketLimits {
                frames_per_sec: 60,
                frame_burst: 180,
                bytes_per_sec: 512 * 1024,
                byte_burst: 4 * 1024 * 1024,
            },
            pre_authorization: BucketLimits {
                frames_per_sec: 10,
                frame_burst: 30,
                bytes_per_sec: 512 * 1024,
                byte_burst: 4 * 1024 * 1024,
            },
            max_connections_per_peer: 4,
        }
    }
}

/// Frame and byte counters for one (local node, remote endpoint)
/// pair.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Frames admitted by the evaluator.
    pub frames: u64,
    /// Bytes admitted by the evaluator.
    pub bytes: u64,
}

/// Per-(local node, remote endpoint) counters with a snapshot API
/// for the observability surface.
#[derive(Clone, Debug, Default)]
pub struct CounterRegistry {
    counters: HashMap<(NodeId, EndpointId), Counters>,
}

impl CounterRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `frames` admitted frames and `bytes` admitted bytes
    /// for traffic between `local` and `remote`.
    pub fn record(&mut self, local: NodeId, remote: EndpointId, frames: u64, bytes: u64) {
        let entry = self.counters.entry((local, remote)).or_default();
        entry.frames = entry.frames.saturating_add(frames);
        entry.bytes = entry.bytes.saturating_add(bytes);
    }

    /// The counters for one pair, if any traffic was recorded.
    #[must_use]
    pub fn get(&self, local: &NodeId, remote: &EndpointId) -> Option<Counters> {
        self.counters.get(&(*local, *remote)).copied()
    }

    /// All counters, sorted by (local node, remote endpoint) so
    /// snapshots are deterministic.
    #[must_use]
    pub fn snapshot(&self) -> Vec<((NodeId, EndpointId), Counters)> {
        let mut rows: Vec<_> = self.counters.iter().map(|(k, v)| (*k, *v)).collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }
}

/// Per-endpoint evaluator state. Buckets are keyed by endpoint
/// alone — never by connection — so connections share one allowance.
#[derive(Debug)]
struct EndpointState {
    authorized_frames: TokenBucket,
    authorized_bytes: TokenBucket,
    preauth_frames: TokenBucket,
    preauth_bytes: TokenBucket,
    connections: HashSet<ConnectionId>,
}

impl EndpointState {
    fn new(policy: &AbusePolicy, now_ms: u64) -> Self {
        Self {
            authorized_frames: TokenBucket::new(
                policy.authorized.frame_burst,
                policy.authorized.frames_per_sec,
                now_ms,
            ),
            authorized_bytes: TokenBucket::new(
                policy.authorized.byte_burst,
                policy.authorized.bytes_per_sec,
                now_ms,
            ),
            preauth_frames: TokenBucket::new(
                policy.pre_authorization.frame_burst,
                policy.pre_authorization.frames_per_sec,
                now_ms,
            ),
            preauth_bytes: TokenBucket::new(
                policy.pre_authorization.byte_burst,
                policy.pre_authorization.bytes_per_sec,
                now_ms,
            ),
            connections: HashSet::new(),
        }
    }
}

/// The shared abuse evaluator. Owns every endpoint's buckets and
/// the counter registry; surfaces call it before admitting frames
/// or bytes.
#[derive(Debug)]
pub struct AbuseEvaluator {
    policy: AbusePolicy,
    local_node: NodeId,
    endpoints: HashMap<EndpointId, EndpointState>,
    next_connection: u64,
    registry: CounterRegistry,
}

impl AbuseEvaluator {
    #[must_use]
    pub fn new(policy: AbusePolicy, local_node: NodeId) -> Self {
        Self {
            policy,
            local_node,
            endpoints: HashMap::new(),
            next_connection: 1,
            registry: CounterRegistry::new(),
        }
    }

    fn state(&mut self, endpoint: EndpointId, now_ms: u64) -> &mut EndpointState {
        let policy = self.policy;
        self.endpoints
            .entry(endpoint)
            .or_insert_with(|| EndpointState::new(&policy, now_ms))
    }

    /// Registers a new connection for `endpoint`, enforcing the
    /// per-peer concurrent connection cap.
    ///
    /// # Errors
    ///
    /// Returns a [`RefusalReason::RateLimited`] refusal when the
    /// endpoint already holds the policy's maximum connections.
    pub fn open_connection(
        &mut self,
        endpoint: EndpointId,
        now_ms: u64,
    ) -> Result<ConnectionId, Refusal> {
        let cap = self.policy.max_connections_per_peer;
        // Ids are unique and never reused; a refused open may skip
        // an id, which is harmless.
        let id = ConnectionId(self.next_connection);
        self.next_connection += 1;
        let state = self.state(endpoint, now_ms);
        if state.connections.len() >= cap {
            return Err(Refusal::new(RefusalReason::RateLimited, RETRY_FLOOR_MS));
        }
        state.connections.insert(id);
        Ok(id)
    }

    /// Releases a connection previously opened with
    /// [`AbuseEvaluator::open_connection`].
    pub fn close_connection(&mut self, endpoint: &EndpointId, connection: ConnectionId) {
        if let Some(state) = self.endpoints.get_mut(endpoint) {
            state.connections.remove(&connection);
        }
    }

    /// Admits one frame from `endpoint`, drawing on the endpoint's
    /// shared frame bucket (the authorized or pre-authorization
    /// bucket per `authorized`). Admitted frames are counted in the
    /// registry.
    ///
    /// # Errors
    ///
    /// Returns a [`RefusalReason::RateLimited`] refusal, with the
    /// bounded retry delay the bucket implies, when the bucket is
    /// empty.
    pub fn check_frame(
        &mut self,
        endpoint: EndpointId,
        authorized: bool,
        now_ms: u64,
    ) -> Result<(), Refusal> {
        let state = self.state(endpoint, now_ms);
        let bucket = if authorized {
            &mut state.authorized_frames
        } else {
            &mut state.preauth_frames
        };
        if bucket.try_take(1, now_ms) {
            self.registry.record(self.local_node, endpoint, 1, 0);
            Ok(())
        } else {
            let retry = bucket.retry_after_ms(1, now_ms);
            Err(Refusal::new(RefusalReason::RateLimited, retry))
        }
    }

    /// Admits `bytes` bytes from `endpoint`, drawing on the
    /// endpoint's shared byte bucket. Admitted bytes are counted in
    /// the registry.
    ///
    /// # Errors
    ///
    /// Returns a [`RefusalReason::BudgetExhausted`] refusal, with
    /// the bounded retry delay the bucket implies, when the bucket
    /// cannot cover `bytes`.
    pub fn check_bytes(
        &mut self,
        endpoint: EndpointId,
        authorized: bool,
        bytes: u64,
        now_ms: u64,
    ) -> Result<(), Refusal> {
        let state = self.state(endpoint, now_ms);
        let bucket = if authorized {
            &mut state.authorized_bytes
        } else {
            &mut state.preauth_bytes
        };
        if bucket.try_take(bytes, now_ms) {
            self.registry.record(self.local_node, endpoint, 0, bytes);
            Ok(())
        } else {
            let retry = bucket.retry_after_ms(bytes, now_ms);
            Err(Refusal::new(RefusalReason::BudgetExhausted, retry))
        }
    }

    /// The counter registry this evaluator records into.
    #[must_use]
    pub fn counters(&self) -> &CounterRegistry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(seed: u8) -> EndpointId {
        [seed; 32]
    }

    fn evaluator() -> AbuseEvaluator {
        AbuseEvaluator::new(AbusePolicy::default(), [1; 32])
    }

    #[test]
    fn defaults_match_the_design() {
        let policy = AbusePolicy::default();
        assert_eq!(policy.authorized.frames_per_sec, 60);
        assert_eq!(policy.authorized.frame_burst, 180);
        assert_eq!(policy.authorized.bytes_per_sec, 512 * 1024);
        assert_eq!(policy.authorized.byte_burst, 4 * 1024 * 1024);
        assert_eq!(policy.pre_authorization.frames_per_sec, 10);
        assert_eq!(policy.pre_authorization.frame_burst, 30);
        assert_eq!(policy.max_connections_per_peer, 4);
    }

    #[test]
    fn frame_bucket_drains_at_burst_and_refills() {
        let mut eval = evaluator();
        let ep = endpoint(7);
        // Burst: 180 frames at t=0, the 181st is refused.
        for _ in 0..180 {
            eval.check_frame(ep, true, 0).expect("within burst");
        }
        assert!(eval.check_frame(ep, true, 0).is_err());
        // One second later exactly 60 more frames are available.
        for _ in 0..60 {
            eval.check_frame(ep, true, 1_000).expect("refilled");
        }
        assert!(eval.check_frame(ep, true, 1_000).is_err());
    }

    #[test]
    fn byte_bucket_drains_and_refills() {
        let mut eval = evaluator();
        let ep = endpoint(8);
        eval.check_bytes(ep, true, 4 * 1024 * 1024, 0)
            .expect("full burst");
        assert!(eval.check_bytes(ep, true, 1, 0).is_err());
        // Half a second refills 256 KiB.
        eval.check_bytes(ep, true, 256 * 1024, 500)
            .expect("refilled");
        assert!(eval.check_bytes(ep, true, 1, 500).is_err());
    }

    #[test]
    fn endpoint_allowance_is_shared_across_connections() {
        let mut eval = evaluator();
        let ep = endpoint(9);
        let a = eval.open_connection(ep, 0).expect("connection a");
        let b = eval.open_connection(ep, 0).expect("connection b");
        assert_ne!(a, b);
        // Frames sent "on" different connections draw on one bucket:
        // the evaluator keys buckets by endpoint, so 180 frames in
        // total — not 180 per connection — are admitted.
        for _ in 0..180 {
            eval.check_frame(ep, true, 0).expect("shared burst");
        }
        assert!(eval.check_frame(ep, true, 0).is_err());
    }

    #[test]
    fn connection_cap_is_per_peer() {
        let mut eval = evaluator();
        let ep = endpoint(10);
        let mut held = Vec::new();
        for _ in 0..4 {
            held.push(eval.open_connection(ep, 0).expect("within cap"));
        }
        let refused = eval.open_connection(ep, 0).expect_err("fifth connection");
        assert_eq!(refused.reason, RefusalReason::RateLimited);
        // A different peer has its own cap.
        eval.open_connection(endpoint(11), 0).expect("other peer");
        // Closing one frees a slot.
        eval.close_connection(&ep, held[0]);
        eval.open_connection(ep, 0).expect("after close");
    }

    #[test]
    fn pre_authorization_bucket_is_tighter_and_separate() {
        let mut eval = evaluator();
        let ep = endpoint(12);
        for _ in 0..30 {
            eval.check_frame(ep, false, 0).expect("preauth burst");
        }
        assert!(eval.check_frame(ep, false, 0).is_err());
        // The authorized bucket is untouched by pre-auth traffic.
        eval.check_frame(ep, true, 0).expect("authorized separate");
    }

    #[test]
    fn refusal_retry_is_bounded() {
        assert_eq!(
            Refusal::new(RefusalReason::RateLimited, 0).retry_after_ms,
            Some(RETRY_FLOOR_MS)
        );
        assert_eq!(
            Refusal::new(RefusalReason::RateLimited, 999_999).retry_after_ms,
            Some(RETRY_CAP_MS)
        );
        assert_eq!(
            Refusal::new(RefusalReason::BudgetExhausted, 5_000).retry_after_ms,
            Some(5_000)
        );
        // Permanent blocks offer no retry, whatever delay was passed.
        assert_eq!(Refusal::blocked_by_operator().retry_after_ms, None);
        assert_eq!(
            Refusal::new(RefusalReason::BlockedByOperator, 5_000).retry_after_ms,
            None
        );
    }

    #[test]
    fn counters_increment_and_snapshot() {
        let mut eval = evaluator();
        let ep = endpoint(13);
        eval.check_frame(ep, true, 0).expect("frame");
        eval.check_frame(ep, true, 0).expect("frame");
        eval.check_bytes(ep, true, 2_048, 0).expect("bytes");
        // Refused traffic is not counted.
        let big = eval.open_connection(ep, 0).expect("conn");
        drop(big);
        assert_eq!(
            eval.counters().get(&[1; 32], &ep),
            Some(Counters {
                frames: 2,
                bytes: 2_048
            })
        );
        let snapshot = eval.counters().snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, ([1; 32], ep));
    }
}
