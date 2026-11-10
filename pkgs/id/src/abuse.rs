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
//! - [`BudgetPolicy`] — the #2247 rolling-window budgets:
//!   app-wide per direction, per endpoint, per directly observed
//!   IP, and per-counterparty overrides that replace the per-endpoint
//!   defaults for one peer. Charged through
//!   [`AbuseEvaluator::check_budget`] /
//!   [`AbuseEvaluator::check_frame_and_bytes_budgeted`] only when an
//!   owner configures them on the evaluator.
//! - [`CounterRegistry`] — per-(local node, remote endpoint) frame
//!   and byte counters with a snapshot API for the observability
//!   surface the integration phase builds.
//!
//! Time is injected as `now_ms` (milliseconds on any monotonic
//! clock the caller chooses) so the evaluator is deterministic and
//! unit-testable without sleeping.
//!
//! Retained state is bounded by pruning, never by the connection
//! cap: [`AbuseEvaluator::prune_idle_endpoints`] evicts endpoints
//! with no open connections whose buckets have fully refilled
//! (a state indistinguishable from never having been seen, so
//! eviction changes no admission decision), and
//! [`CounterRegistry::prune`] drops counter rows idle past a
//! caller-chosen retention. Callers run both periodically.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

use crate::event_store::BlockList;

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

    /// The retry delay as whole seconds, rounded up, for an HTTP
    /// `Retry-After` header. `None` for permanent blocks.
    #[must_use]
    pub fn retry_after_secs_ceil(&self) -> Option<u64> {
        self.retry_after_ms.map(|ms| ms.div_ceil(1_000))
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

/// A [`Refusal`] as a standard error, so surfaces can propagate it
/// with `?` / anyhow and callers can downcast back to the structured
/// refusal (reason + bounded retry value) at their boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefusalError(pub Refusal);

impl std::fmt::Display for RefusalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.retry_after_ms {
            Some(ms) => write!(f, "refused ({:?}); retry after {ms} ms", self.0.reason),
            None => write!(f, "refused ({:?}); no retry offered", self.0.reason),
        }
    }
}

impl std::error::Error for RefusalError {}

impl From<Refusal> for RefusalError {
    fn from(refusal: Refusal) -> Self {
        Self(refusal)
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

    /// Whether the bucket currently holds its full capacity.
    fn is_full(&mut self, now_ms: u64) -> bool {
        self.refill(now_ms);
        self.tokens_milli >= self.capacity_milli
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

    /// Refill and check whether `cost` whole tokens are available
    /// without consuming them.
    fn can_take(&mut self, cost: u64, now_ms: u64) -> bool {
        self.refill(now_ms);
        self.tokens_milli >= cost.saturating_mul(1_000)
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

/// Which way traffic crosses a budget scope (#2247).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Traffic received from a peer.
    Inbound,
    /// Traffic sent to a peer.
    Outbound,
}

/// One rolling-window budget limit (#2247): at most
/// `bytes_per_window` bytes and — when `messages_per_window` is
/// non-zero — at most that many messages inside any rolling
/// `window_ms` span. A zero byte cap means bytes are not capped by
/// this limit; a zero message cap means messages are not capped.
/// Windows are evaluated with the standard sliding-window
/// approximation (previous fixed window weighted by the fraction
/// still inside the rolling span, plus the current window), so
/// retained state stays O(1) per scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetLimit {
    /// Length of the rolling window, in milliseconds.
    pub window_ms: u64,
    /// Byte cap inside the window; 0 means no byte cap.
    pub bytes_per_window: u64,
    /// Message cap inside the window; 0 means no message cap.
    pub messages_per_window: u64,
}

impl BudgetLimit {
    /// A byte-only limit over `window_ms`.
    #[must_use]
    pub const fn bytes(window_ms: u64, bytes_per_window: u64) -> Self {
        Self {
            window_ms,
            bytes_per_window,
            messages_per_window: 0,
        }
    }
}

/// The #2247 budget policy: rolling-window byte budgets at
/// app-wide (per direction), per-endpoint, and per-direct-IP scope,
/// plus per-counterparty override rows that replace the per-endpoint
/// defaults for one specific endpoint.
///
/// Defaults are the #2247 values: app-wide inbound 32 MiB/s and
/// 20 GiB/h, outbound 64 MiB/s and 40 GiB/h; per peer 100 MiB/h with
/// an 8 MiB/min burst window; per direct IP 250 MiB/h.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetPolicy {
    /// App-wide inbound limits; every inbound byte charges these.
    pub app_inbound: Vec<BudgetLimit>,
    /// App-wide outbound limits; every outbound byte charges these.
    pub app_outbound: Vec<BudgetLimit>,
    /// Default per-endpoint limits.
    pub per_peer: Vec<BudgetLimit>,
    /// Per-IP limits, charged only for directly observed IPs.
    pub per_ip: Vec<BudgetLimit>,
    /// Override rows for specific endpoints, consulted before
    /// `per_peer`: an override REPLACES the default per-endpoint
    /// limits for that endpoint (it does not stack with them), so an
    /// operator can cap — or raise — one peer without touching the
    /// global budget.
    pub counterparty_overrides: Vec<(EndpointId, Vec<BudgetLimit>)>,
}

impl BudgetPolicy {
    /// The per-endpoint limits that apply to `endpoint`: its
    /// override row when one exists, else the default `per_peer`.
    #[must_use]
    pub fn limits_for_peer(&self, endpoint: &EndpointId) -> &[BudgetLimit] {
        for (peer, limits) in &self.counterparty_overrides {
            if peer == endpoint {
                return limits;
            }
        }
        &self.per_peer
    }
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        const MIB: u64 = 1024 * 1024;
        const GIB: u64 = 1024 * MIB;
        Self {
            app_inbound: vec![
                BudgetLimit::bytes(1_000, 32 * MIB),
                BudgetLimit::bytes(3_600_000, 20 * GIB),
            ],
            app_outbound: vec![
                BudgetLimit::bytes(1_000, 64 * MIB),
                BudgetLimit::bytes(3_600_000, 40 * GIB),
            ],
            per_peer: vec![
                BudgetLimit::bytes(60_000, 8 * MIB),
                BudgetLimit::bytes(3_600_000, 100 * MIB),
            ],
            per_ip: vec![BudgetLimit::bytes(3_600_000, 250 * MIB)],
            counterparty_overrides: Vec::new(),
        }
    }
}

/// A budget scope a usage snapshot can name (#2247 observability).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BudgetScope {
    /// App-wide inbound.
    AppInbound,
    /// App-wide outbound.
    AppOutbound,
    /// One endpoint's (default or override) budget.
    Peer(EndpointId),
    /// One directly observed IP's budget.
    Ip(IpAddr),
}

/// Estimated usage of one budget window inside its rolling span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BudgetUsage {
    /// Estimated bytes charged inside the rolling window.
    pub bytes: u64,
    /// Estimated messages charged inside the rolling window.
    pub messages: u64,
}

/// Sliding-window counter pair for one [`BudgetLimit`]: the current
/// fixed window's counts plus the previous window's, weighted by
/// the fraction of the previous window still inside the rolling
/// span. All arithmetic is saturating / u128 where a product could
/// overflow.
#[derive(Clone, Copy, Debug)]
struct WindowCounter {
    window_ms: u64,
    cur_start_ms: u64,
    cur_bytes: u64,
    cur_messages: u64,
    prev_bytes: u64,
    prev_messages: u64,
}

impl WindowCounter {
    fn new(limit: &BudgetLimit, now_ms: u64) -> Self {
        Self {
            window_ms: limit.window_ms.max(1),
            cur_start_ms: now_ms,
            cur_bytes: 0,
            cur_messages: 0,
            prev_bytes: 0,
            prev_messages: 0,
        }
    }

    fn roll(&mut self, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.cur_start_ms);
        if elapsed >= self.window_ms {
            if elapsed >= self.window_ms.saturating_mul(2) {
                self.prev_bytes = 0;
                self.prev_messages = 0;
                self.cur_start_ms = now_ms;
            } else {
                self.prev_bytes = self.cur_bytes;
                self.prev_messages = self.cur_messages;
                self.cur_start_ms += self.window_ms;
            }
            self.cur_bytes = 0;
            self.cur_messages = 0;
        }
    }

    /// Previous window's contribution: its count weighted by the
    /// fraction of it still inside the rolling span.
    fn prev_weighted(&self, prev: u64, now_ms: u64) -> u64 {
        let elapsed = now_ms.saturating_sub(self.cur_start_ms);
        let remaining = self.window_ms.saturating_sub(elapsed);
        let weighted = u128::from(prev) * u128::from(remaining) / u128::from(self.window_ms);
        u64::try_from(weighted).unwrap_or(u64::MAX)
    }

    /// This counter as it would look after `roll(now_ms)`, without
    /// mutating: the stored counter only rolls on the mutating charge
    /// paths, so an estimate taken from the unrolled state keeps
    /// reporting the last charged window in full forever (#4683).
    /// Rolling a copy keeps the read path `&self`; a second roll at
    /// the same `now_ms` is a no-op, so `can_admit` results on an
    /// already-rolled counter are unchanged.
    fn rolled(&self, now_ms: u64) -> Self {
        let mut rolled = *self;
        rolled.roll(now_ms);
        rolled
    }

    fn estimate_bytes(&self, now_ms: u64) -> u64 {
        let rolled = self.rolled(now_ms);
        rolled
            .cur_bytes
            .saturating_add(rolled.prev_weighted(rolled.prev_bytes, now_ms))
    }

    fn estimate_messages(&self, now_ms: u64) -> u64 {
        let rolled = self.rolled(now_ms);
        rolled
            .cur_messages
            .saturating_add(rolled.prev_weighted(rolled.prev_messages, now_ms))
    }

    /// Whether admitting `bytes` more bytes / one more message
    /// stays within `limit`.
    fn can_admit(&self, limit: &BudgetLimit, bytes: u64, now_ms: u64) -> bool {
        let bytes_ok = limit.bytes_per_window == 0
            || self.estimate_bytes(now_ms).saturating_add(bytes) <= limit.bytes_per_window;
        let messages_ok = limit.messages_per_window == 0
            || self.estimate_messages(now_ms).saturating_add(1) <= limit.messages_per_window;
        bytes_ok && messages_ok
    }

    /// Milliseconds until the current fixed window rolls; the
    /// earliest moment a refused charge could start succeeding.
    fn retry_after_ms(&self, now_ms: u64) -> u64 {
        let elapsed = now_ms.saturating_sub(self.cur_start_ms);
        self.window_ms.saturating_sub(elapsed).max(1)
    }

    fn record(&mut self, bytes: u64) {
        self.cur_bytes = self.cur_bytes.saturating_add(bytes);
        self.cur_messages = self.cur_messages.saturating_add(1);
    }

    /// True when both fixed windows are empty, so the state carries
    /// no information and may be dropped.
    fn is_empty(&self) -> bool {
        self.cur_bytes == 0
            && self.cur_messages == 0
            && self.prev_bytes == 0
            && self.prev_messages == 0
    }

    /// True when, rolled to `now_ms`, this counter would be empty:
    /// both windows have aged out, or the one window that would
    /// survive a roll is the empty current one. Non-mutating.
    fn aged_out(&self, now_ms: u64) -> bool {
        let elapsed = now_ms.saturating_sub(self.cur_start_ms);
        if elapsed >= self.window_ms.saturating_mul(2) {
            return true;
        }
        if elapsed >= self.window_ms {
            return self.cur_bytes == 0 && self.cur_messages == 0;
        }
        self.is_empty()
    }
}

/// Key for one budget scope's window counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BudgetKey {
    AppInbound,
    AppOutbound,
    Peer(EndpointId),
    Ip(IpAddr),
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

/// Counters split by the direction traffic crossed the evaluator in.
///
/// Traffic admitted through a check that knows its direction lands in
/// `inbound` or `outbound`; traffic admitted through a check that does
/// not (the pre-#4918 surfaces) lands in `unattributed` rather than
/// being guessed at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectionalCounters {
    /// Frames and bytes admitted in the inbound direction.
    pub inbound: Counters,
    /// Frames and bytes admitted in the outbound direction.
    pub outbound: Counters,
    /// Frames and bytes admitted without a known direction.
    pub unattributed: Counters,
}

impl DirectionalCounters {
    /// The aggregate counters across every direction.
    #[must_use]
    pub fn total(&self) -> Counters {
        Counters {
            frames: self
                .inbound
                .frames
                .saturating_add(self.outbound.frames)
                .saturating_add(self.unattributed.frames),
            bytes: self
                .inbound
                .bytes
                .saturating_add(self.outbound.bytes)
                .saturating_add(self.unattributed.bytes),
        }
    }

    fn for_direction(&mut self, direction: Option<Direction>) -> &mut Counters {
        match direction {
            Some(Direction::Inbound) => &mut self.inbound,
            Some(Direction::Outbound) => &mut self.outbound,
            None => &mut self.unattributed,
        }
    }
}

/// One registry row: the per-direction counters plus the last time
/// traffic was recorded for the pair, which is what
/// [`CounterRegistry::prune`] ages out on.
#[derive(Clone, Copy, Debug, Default)]
struct CounterEntry {
    counters: DirectionalCounters,
    last_seen_ms: u64,
}

/// Per-(local node, remote endpoint) counters with a snapshot API
/// for the observability surface.
#[derive(Clone, Debug, Default)]
pub struct CounterRegistry {
    counters: HashMap<(NodeId, EndpointId), CounterEntry>,
}

impl CounterRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `frames` admitted frames and `bytes` admitted bytes
    /// for traffic between `local` and `remote`, stamped at
    /// `now_ms` for pruning. `direction` names the direction the
    /// traffic crossed in, or `None` when the admitting check cannot
    /// tell; those charges are kept as `unattributed`.
    pub fn record(
        &mut self,
        local: NodeId,
        remote: EndpointId,
        frames: u64,
        bytes: u64,
        direction: Option<Direction>,
        now_ms: u64,
    ) {
        let entry = self.counters.entry((local, remote)).or_default();
        let counters = entry.counters.for_direction(direction);
        counters.frames = counters.frames.saturating_add(frames);
        counters.bytes = counters.bytes.saturating_add(bytes);
        entry.last_seen_ms = entry.last_seen_ms.max(now_ms);
    }

    /// The aggregate counters for one pair — every direction summed —
    /// if any traffic was recorded.
    #[must_use]
    pub fn get(&self, local: &NodeId, remote: &EndpointId) -> Option<Counters> {
        self.counters
            .get(&(*local, *remote))
            .map(|e| e.counters.total())
    }

    /// All counters aggregated across directions, sorted by (local
    /// node, remote endpoint) so snapshots are deterministic.
    #[must_use]
    pub fn snapshot(&self) -> Vec<((NodeId, EndpointId), Counters)> {
        let mut rows: Vec<_> = self
            .counters
            .iter()
            .map(|(k, e)| (*k, e.counters.total()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// All counters split by direction, copied without sorting.
    ///
    /// For callers holding a lock on the registry's owner who must keep
    /// the hold short: copy under the lock, then sort the copy after
    /// release (see [`CounterRegistry::snapshot_directional`]).
    #[must_use]
    pub fn copied_directional(&self) -> Vec<((NodeId, EndpointId), DirectionalCounters)> {
        self.counters
            .iter()
            .map(|(k, e)| (*k, e.counters))
            .collect()
    }

    /// All counters split by direction, sorted by (local node, remote
    /// endpoint) so snapshots are deterministic.
    #[must_use]
    pub fn snapshot_directional(&self) -> Vec<((NodeId, EndpointId), DirectionalCounters)> {
        let mut rows = self.copied_directional();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// Drops rows whose last recorded traffic is at least
    /// `retention_ms` before `now_ms`, returning how many rows
    /// were dropped. This is the registry's bound on retained
    /// state: without it, every distinct endpoint ever seen would
    /// keep a row forever. Snapshot before pruning if lifetime
    /// totals are needed.
    pub fn prune(&mut self, now_ms: u64, retention_ms: u64) -> usize {
        let before = self.counters.len();
        self.counters
            .retain(|_, e| now_ms.saturating_sub(e.last_seen_ms) < retention_ms);
        before - self.counters.len()
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

impl EndpointState {
    /// Whether evicting this state changes nothing: no open
    /// connections, and every bucket back at full capacity, so a
    /// future first check sees exactly the state a fresh endpoint
    /// would get.
    fn is_idle_and_full(&mut self, now_ms: u64) -> bool {
        self.connections.is_empty()
            && self.authorized_frames.is_full(now_ms)
            && self.authorized_bytes.is_full(now_ms)
            && self.preauth_frames.is_full(now_ms)
            && self.preauth_bytes.is_full(now_ms)
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
    /// The #2247 rolling-window budgets, when this evaluator's
    /// owner configured them. `None` keeps the pre-#2247 behaviour
    /// exactly: token buckets only, no budget scopes charged.
    budgets: Option<BudgetPolicy>,
    budget_state: HashMap<BudgetKey, Vec<WindowCounter>>,
    /// The operator block list (#2245), when the evaluator's owner
    /// attached one: [`AbuseEvaluator::open_connection`] refuses a
    /// blocked endpoint before any bucket or cap is consulted. `None`
    /// keeps the pre-#2245 behaviour exactly.
    blocks: Option<Arc<dyn BlockList>>,
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
            budgets: None,
            budget_state: HashMap::new(),
            blocks: None,
        }
    }

    /// Like [`AbuseEvaluator::new`], with the operator block list
    /// (#2245) attached: a new connection from an endpoint blocked by
    /// key is refused with [`Refusal::blocked_by_operator`], and so is
    /// a connection whose directly observed IP is blocked, when the
    /// listener supplies one to
    /// [`AbuseEvaluator::open_connection_with_direct_ip`]. The list
    /// is consulted live on every open, so a block or an unblock lands
    /// on the next connection. Frame and byte checks are unchanged —
    /// a block refuses new sessions; it does not retroactively charge
    /// or drop a session that is already open.
    #[must_use]
    pub fn with_block_list(mut self, blocks: Arc<dyn BlockList>) -> Self {
        self.blocks = Some(blocks);
        self
    }

    /// Like [`AbuseEvaluator::new`], with the #2247 rolling-window
    /// budgets configured: charges through
    /// [`AbuseEvaluator::check_budget`] and
    /// [`AbuseEvaluator::check_frame_and_bytes_budgeted`] evaluate
    /// the app-wide, per-endpoint (or per-counterparty override),
    /// and direct-IP scopes in addition to the token buckets.
    #[must_use]
    pub fn with_budgets(policy: AbusePolicy, local_node: NodeId, budgets: BudgetPolicy) -> Self {
        let mut evaluator = Self::new(policy, local_node);
        evaluator.budgets = Some(budgets);
        evaluator
    }

    /// The configured #2247 budget policy, if any.
    #[must_use]
    pub fn budgets(&self) -> Option<&BudgetPolicy> {
        self.budgets.as_ref()
    }

    /// The budget scopes one charge touches, in check order:
    /// app-wide for the direction, the endpoint's own scope, and
    /// the direct-IP scope when an IP was directly observed. A
    /// relayed or otherwise obscured IP is never supplied here —
    /// callers pass `None`, and no IP scope is charged.
    fn budget_scopes(
        &self,
        direction: Direction,
        endpoint: EndpointId,
        direct_ip: Option<IpAddr>,
    ) -> Vec<(BudgetKey, BudgetScope, Vec<BudgetLimit>)> {
        let Some(policy) = &self.budgets else {
            return Vec::new();
        };
        let (app_key, app_scope, app_limits) = match direction {
            Direction::Inbound => (
                BudgetKey::AppInbound,
                BudgetScope::AppInbound,
                policy.app_inbound.clone(),
            ),
            Direction::Outbound => (
                BudgetKey::AppOutbound,
                BudgetScope::AppOutbound,
                policy.app_outbound.clone(),
            ),
        };
        let mut scopes = vec![
            (app_key, app_scope, app_limits),
            (
                BudgetKey::Peer(endpoint),
                BudgetScope::Peer(endpoint),
                policy.limits_for_peer(&endpoint).to_vec(),
            ),
        ];
        if let Some(ip) = direct_ip {
            scopes.push((
                BudgetKey::Ip(ip),
                BudgetScope::Ip(ip),
                policy.per_ip.clone(),
            ));
        }
        scopes
    }

    /// The refusal one budget charge would produce, if any —
    /// evaluating every scope without recording anything. Window
    /// counters are rolled to `now_ms`, which changes no admission
    /// outcome (a rolled counter estimates the same usage).
    fn budget_refusal(
        &mut self,
        direction: Direction,
        endpoint: EndpointId,
        direct_ip: Option<IpAddr>,
        bytes: u64,
        now_ms: u64,
    ) -> Option<Refusal> {
        let scopes = self.budget_scopes(direction, endpoint, direct_ip);
        let mut retry_ms = 0_u64;
        let mut refused = false;
        for (key, _scope, limits) in &scopes {
            let counters = self.budget_state.entry(*key).or_insert_with(|| {
                limits
                    .iter()
                    .map(|l| WindowCounter::new(l, now_ms))
                    .collect()
            });
            for (counter, limit) in counters.iter_mut().zip(limits.iter()) {
                counter.roll(now_ms);
                if !counter.can_admit(limit, bytes, now_ms) {
                    refused = true;
                    retry_ms = retry_ms.max(counter.retry_after_ms(now_ms));
                }
            }
        }
        refused.then(|| Refusal::new(RefusalReason::BudgetExhausted, retry_ms))
    }

    /// Records one admitted budget charge in every scope.
    fn record_budget(
        &mut self,
        direction: Direction,
        endpoint: EndpointId,
        direct_ip: Option<IpAddr>,
        bytes: u64,
        now_ms: u64,
    ) {
        let scopes = self.budget_scopes(direction, endpoint, direct_ip);
        for (key, _scope, limits) in &scopes {
            let counters = self.budget_state.entry(*key).or_insert_with(|| {
                limits
                    .iter()
                    .map(|l| WindowCounter::new(l, now_ms))
                    .collect()
            });
            for counter in counters.iter_mut() {
                counter.roll(now_ms);
                counter.record(bytes);
            }
        }
    }

    /// Admits `bytes` bytes (one message) against the #2247
    /// rolling-window budgets alone: the app-wide scope for
    /// `direction`, the endpoint's per-peer or counterparty-override
    /// scope, and — only when `direct_ip` is `Some`, i.e. the IP was
    /// directly observed on a direct connection — the per-IP scope.
    /// Either every scope admits the charge and it is recorded in
    /// all of them, or none records it.
    ///
    /// # Errors
    ///
    /// Returns a [`RefusalReason::BudgetExhausted`] refusal with the
    /// bounded retry delay of the slowest-refilling failing window
    /// when any scope is exhausted; returns `Ok(())` trivially when
    /// no budget policy is configured.
    pub fn check_budget(
        &mut self,
        direction: Direction,
        endpoint: EndpointId,
        direct_ip: Option<IpAddr>,
        bytes: u64,
        now_ms: u64,
    ) -> Result<(), Refusal> {
        if let Some(refusal) = self.budget_refusal(direction, endpoint, direct_ip, bytes, now_ms) {
            return Err(refusal);
        }
        self.record_budget(direction, endpoint, direct_ip, bytes, now_ms);
        Ok(())
    }

    /// Atomically admits one frame and its byte cost against both
    /// the token buckets (as [`AbuseEvaluator::check_frame_and_bytes`])
    /// and the #2247 budgets (as [`AbuseEvaluator::check_budget`]).
    /// Nothing is charged — no bucket token, no budget window, no
    /// counter — unless every check admits the frame.
    ///
    /// # Errors
    ///
    /// Returns the frame-rate refusal first, then the byte-bucket
    /// refusal, then the budget refusal, matching the precedence of
    /// the checks.
    pub fn check_frame_and_bytes_budgeted(
        &mut self,
        endpoint: EndpointId,
        authorized: bool,
        bytes: u64,
        direction: Direction,
        direct_ip: Option<IpAddr>,
        now_ms: u64,
    ) -> Result<(), Refusal> {
        let (frame_retry, byte_retry) = {
            let state = self.state(endpoint, now_ms);
            let (frames, byte_bucket) = if authorized {
                (&mut state.authorized_frames, &mut state.authorized_bytes)
            } else {
                (&mut state.preauth_frames, &mut state.preauth_bytes)
            };
            let frame_retry =
                (!frames.can_take(1, now_ms)).then(|| frames.retry_after_ms(1, now_ms));
            let byte_retry = (!byte_bucket.can_take(bytes, now_ms))
                .then(|| byte_bucket.retry_after_ms(bytes, now_ms));
            (frame_retry, byte_retry)
        };
        if let Some(retry) = frame_retry {
            return Err(Refusal::new(RefusalReason::RateLimited, retry));
        }
        if let Some(retry) = byte_retry {
            return Err(Refusal::new(RefusalReason::BudgetExhausted, retry));
        }
        if let Some(refusal) = self.budget_refusal(direction, endpoint, direct_ip, bytes, now_ms) {
            return Err(refusal);
        }
        {
            let state = self.state(endpoint, now_ms);
            let (frames, byte_bucket) = if authorized {
                (&mut state.authorized_frames, &mut state.authorized_bytes)
            } else {
                (&mut state.preauth_frames, &mut state.preauth_bytes)
            };
            let frame_taken = frames.try_take(1, now_ms);
            let bytes_taken = byte_bucket.try_take(bytes, now_ms);
            debug_assert!(frame_taken && bytes_taken);
        }
        self.record_budget(direction, endpoint, direct_ip, bytes, now_ms);
        self.registry.record(
            self.local_node,
            endpoint,
            1,
            bytes,
            Some(direction),
            now_ms,
        );
        Ok(())
    }

    /// Estimated #2247 budget usage per scope and window at
    /// `now_ms`, sorted by scope for deterministic snapshots — the
    /// operator-visibility surface for budgets, alongside
    /// [`AbuseEvaluator::counters`]. Empty when no budget policy is
    /// configured or nothing has been charged yet.
    #[must_use]
    pub fn budget_usage(&self, now_ms: u64) -> Vec<(BudgetScope, BudgetLimit, BudgetUsage)> {
        let Some(policy) = &self.budgets else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for (key, counters) in &self.budget_state {
            let (scope, limits): (BudgetScope, &[BudgetLimit]) = match key {
                BudgetKey::AppInbound => (BudgetScope::AppInbound, &policy.app_inbound),
                BudgetKey::AppOutbound => (BudgetScope::AppOutbound, &policy.app_outbound),
                BudgetKey::Peer(endpoint) => (
                    BudgetScope::Peer(*endpoint),
                    policy.limits_for_peer(endpoint),
                ),
                BudgetKey::Ip(ip) => (BudgetScope::Ip(*ip), &policy.per_ip),
            };
            for (counter, limit) in counters.iter().zip(limits.iter()) {
                rows.push((
                    scope,
                    *limit,
                    BudgetUsage {
                        bytes: counter.estimate_bytes(now_ms),
                        messages: counter.estimate_messages(now_ms),
                    },
                ));
            }
        }
        rows.sort_by_key(|(scope, limit, _)| (format!("{scope:?}"), limit.window_ms));
        rows
    }

    /// Drops budget scopes whose windows have fully aged out (both
    /// fixed windows empty after rolling to `now_ms`), returning how
    /// many scopes were dropped. A dropped scope's next charge
    /// behaves exactly as a never-seen scope's.
    pub fn prune_budget_state(&mut self, now_ms: u64) -> usize {
        let before = self.budget_state.len();
        self.budget_state
            .retain(|_, counters| counters.iter().any(|c| !c.aged_out(now_ms)));
        before - self.budget_state.len()
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
    /// Returns a [`RefusalReason::BlockedByOperator`] refusal when the
    /// operator block list attached with
    /// [`AbuseEvaluator::with_block_list`] blocks `endpoint`, and a
    /// [`RefusalReason::RateLimited`] refusal when the endpoint already
    /// holds the policy's maximum connections.
    pub fn open_connection(
        &mut self,
        endpoint: EndpointId,
        now_ms: u64,
    ) -> Result<ConnectionId, Refusal> {
        self.open_connection_inner(endpoint, None, now_ms)
    }

    /// Like [`AbuseEvaluator::open_connection`], for a listener that
    /// directly observed the connection's remote IP (#2245): when
    /// `direct_ip` is `Some` and the attached block list blocks that
    /// IP, the connection is refused with
    /// [`Refusal::blocked_by_operator`] before any bucket or cap is
    /// consulted — the listener-only, best-effort IP enforcement the
    /// issue asks for. A relayed or otherwise obscured connection
    /// passes `None` and is judged exactly as by
    /// [`AbuseEvaluator::open_connection`]: an IP block never refuses
    /// a peer whose IP was not directly observed.
    ///
    /// # Errors
    ///
    /// The same refusals as [`AbuseEvaluator::open_connection`], plus
    /// a [`RefusalReason::BlockedByOperator`] refusal when `direct_ip`
    /// names a blocked IP.
    pub fn open_connection_with_direct_ip(
        &mut self,
        endpoint: EndpointId,
        direct_ip: Option<IpAddr>,
        now_ms: u64,
    ) -> Result<ConnectionId, Refusal> {
        self.open_connection_inner(endpoint, direct_ip, now_ms)
    }

    fn open_connection_inner(
        &mut self,
        endpoint: EndpointId,
        direct_ip: Option<IpAddr>,
        now_ms: u64,
    ) -> Result<ConnectionId, Refusal> {
        if self
            .blocks
            .as_ref()
            .is_some_and(|blocks| blocks.peer_blocked(&endpoint))
        {
            return Err(Refusal::blocked_by_operator());
        }
        if let (Some(blocks), Some(ip)) = (&self.blocks, direct_ip) {
            if blocks.ip_blocked(&ip) {
                return Err(Refusal::blocked_by_operator());
            }
        }
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

    #[cfg(test)]
    pub(crate) fn active_connection_count(&self, endpoint: &EndpointId) -> usize {
        self.endpoints
            .get(endpoint)
            .map_or(0, |state| state.connections.len())
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
            self.registry
                .record(self.local_node, endpoint, 1, 0, None, now_ms);
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
            self.registry
                .record(self.local_node, endpoint, 0, bytes, None, now_ms);
            Ok(())
        } else {
            let retry = bucket.retry_after_ms(bytes, now_ms);
            Err(Refusal::new(RefusalReason::BudgetExhausted, retry))
        }
    }

    /// Atomically admits one frame and its byte cost. Neither bucket
    /// is charged, and no counter is recorded, unless both budgets
    /// can admit the frame.
    ///
    /// # Errors
    ///
    /// Returns the frame-rate refusal first when both budgets are
    /// exhausted; otherwise returns the byte-budget refusal.
    pub fn check_frame_and_bytes(
        &mut self,
        endpoint: EndpointId,
        authorized: bool,
        bytes: u64,
        now_ms: u64,
    ) -> Result<(), Refusal> {
        let (frame_retry, byte_retry) = {
            let state = self.state(endpoint, now_ms);
            let (frames, byte_bucket) = if authorized {
                (&mut state.authorized_frames, &mut state.authorized_bytes)
            } else {
                (&mut state.preauth_frames, &mut state.preauth_bytes)
            };
            let frame_retry =
                (!frames.can_take(1, now_ms)).then(|| frames.retry_after_ms(1, now_ms));
            let byte_retry = (!byte_bucket.can_take(bytes, now_ms))
                .then(|| byte_bucket.retry_after_ms(bytes, now_ms));
            (frame_retry, byte_retry)
        };

        if let Some(retry) = frame_retry {
            return Err(Refusal::new(RefusalReason::RateLimited, retry));
        }
        if let Some(retry) = byte_retry {
            return Err(Refusal::new(RefusalReason::BudgetExhausted, retry));
        }

        {
            let state = self.state(endpoint, now_ms);
            let (frames, byte_bucket) = if authorized {
                (&mut state.authorized_frames, &mut state.authorized_bytes)
            } else {
                (&mut state.preauth_frames, &mut state.preauth_bytes)
            };
            // No other operation can mutate these buckets between the
            // checks above and these debits: the evaluator is mutably
            // borrowed for this whole call.
            let frame_taken = frames.try_take(1, now_ms);
            let bytes_taken = byte_bucket.try_take(bytes, now_ms);
            debug_assert!(frame_taken && bytes_taken);
        }
        self.registry
            .record(self.local_node, endpoint, 1, bytes, None, now_ms);
        Ok(())
    }

    /// The configured abuse policy — the rate and burst the token
    /// buckets enforce, shown alongside the counters on the operator
    /// surface.
    #[must_use]
    pub fn policy(&self) -> &AbusePolicy {
        &self.policy
    }

    /// The counter registry this evaluator records into.
    #[must_use]
    pub fn counters(&self) -> &CounterRegistry {
        &self.registry
    }

    /// Evicts endpoint states that carry no information - no open
    /// connections and every bucket fully refilled - returning how
    /// many were evicted. An evicted endpoint's next check behaves
    /// exactly as a never-seen endpoint's, so callers may run this
    /// on any schedule to bound retained state; endpoints with
    /// open connections or partially drained buckets are kept.
    pub fn prune_idle_endpoints(&mut self, now_ms: u64) -> usize {
        let before = self.endpoints.len();
        self.endpoints.retain(|_, s| !s.is_idle_and_full(now_ms));
        before - self.endpoints.len()
    }

    /// Drops counter rows idle for at least `retention_ms`,
    /// returning how many were dropped; see
    /// [`CounterRegistry::prune`].
    pub fn prune_counters(&mut self, now_ms: u64, retention_ms: u64) -> usize {
        self.registry.prune(now_ms, retention_ms)
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
    fn a_blocked_endpoint_is_refused_a_new_connection() {
        // #2245 session admission: with a block list attached, a blocked
        // endpoint's new connection is refused as an operator block —
        // permanent, no retry offered, and no connection registered.
        let blocks = crate::pull_policy::testing::stub_blocks(&[endpoint(7)], &[]);
        let mut eval = evaluator().with_block_list(blocks);
        let refused = eval
            .open_connection(endpoint(7), 0)
            .expect_err("a blocked endpoint was admitted");
        assert_eq!(refused, Refusal::blocked_by_operator());
        assert_eq!(refused.reason, RefusalReason::BlockedByOperator);
        assert_eq!(refused.retry_after_ms, None);
        assert_eq!(eval.active_connection_count(&endpoint(7)), 0);
        eval.open_connection(endpoint(8), 0)
            .expect("an unblocked endpoint still connects");
    }

    #[test]
    fn a_blocked_direct_ip_is_refused_a_new_connection() {
        // #2245 listener seam: with a block list attached, a connection
        // whose directly observed IP is blocked is refused as an
        // operator block — and no connection is registered. An
        // unblocked direct IP still connects, and a relayed
        // connection (no IP observed) is unaffected by the IP block.
        let blocked_ip: IpAddr = "203.0.113.7".parse().expect("test ip");
        let other_ip: IpAddr = "203.0.113.8".parse().expect("test ip");
        let blocks = crate::pull_policy::testing::stub_blocks_with_ips(&[], &[], &[blocked_ip]);
        let mut eval = evaluator().with_block_list(blocks);
        let refused = eval
            .open_connection_with_direct_ip(endpoint(7), Some(blocked_ip), 0)
            .expect_err("a blocked direct IP was admitted");
        assert_eq!(refused, Refusal::blocked_by_operator());
        assert_eq!(refused.reason, RefusalReason::BlockedByOperator);
        assert_eq!(refused.retry_after_ms, None);
        assert_eq!(eval.active_connection_count(&endpoint(7)), 0);
        eval.open_connection_with_direct_ip(endpoint(7), Some(other_ip), 0)
            .expect("an unblocked direct IP still connects");
        eval.open_connection_with_direct_ip(endpoint(8), None, 0)
            .expect("a relayed connection is unaffected by IP blocks");
        eval.open_connection(endpoint(9), 0)
            .expect("the pre-#2245 entry point is unchanged");
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

    #[test]
    fn prune_evicts_only_idle_fully_refilled_endpoints() {
        let mut eval = evaluator();
        let idle = endpoint(20);
        let connected = endpoint(21);
        let drained = endpoint(22);
        // idle: traffic at t=0, no open connection. After enough
        // time every bucket refills, so the state carries nothing.
        eval.check_frame(idle, true, 0).expect("frame");
        // connected: an open connection pins the state.
        let conn = eval.open_connection(connected, 0).expect("conn");
        // drained: burst spent at t=0 and not yet refilled.
        for _ in 0..180 {
            eval.check_frame(drained, true, 0).expect("burst");
        }
        // At t=0 nothing is prunable: idle's frame bucket is one
        // token short of full, drained is empty, connected is open.
        assert_eq!(eval.prune_idle_endpoints(0), 0);
        // By 3 s idle's one spent token has refilled, so it is
        // evictable. Keep drained non-full by spending one of its
        // refilled tokens just before the prune.
        eval.check_frame(drained, true, 2_999).expect("one more");
        assert_eq!(eval.prune_idle_endpoints(3_000), 1);
        // The connected endpoint survives until its connection
        // closes and its buckets refill.
        eval.close_connection(&connected, conn);
        assert_eq!(eval.prune_idle_endpoints(3_000), 1);
        // An evicted endpoint behaves exactly like a fresh one:
        // its full authorized burst is available again.
        for _ in 0..180 {
            eval.check_frame(idle, true, 3_000).expect("fresh burst");
        }
        assert!(eval.check_frame(idle, true, 3_000).is_err());
    }

    #[test]
    fn prune_counters_drops_rows_idle_past_retention() {
        let mut eval = evaluator();
        let old = endpoint(23);
        let fresh = endpoint(24);
        eval.check_frame(old, true, 0).expect("frame");
        eval.check_frame(fresh, true, 10_000).expect("frame");
        // Retention 5 s at t=10 s: old (idle 10 s) drops, fresh stays.
        assert_eq!(eval.prune_counters(10_000, 5_000), 1);
        assert_eq!(eval.counters().get(&[1; 32], &old), None);
        assert_eq!(
            eval.counters().get(&[1; 32], &fresh),
            Some(Counters {
                frames: 1,
                bytes: 0
            })
        );
        // Nothing left to drop on a repeat prune at the same time.
        assert_eq!(eval.prune_counters(10_000, 5_000), 0);
    }

    #[test]
    fn directional_counters_split_by_direction_and_aggregate() {
        let mut eval = evaluator();
        let ep = endpoint(25);
        eval.registry.record([1; 32], ep, 1, 100, Some(Direction::Inbound), 0);
        eval.registry
            .record([1; 32], ep, 2, 300, Some(Direction::Outbound), 0);
        eval.registry.record([1; 32], ep, 3, 500, None, 0);

        let rows = eval.counters().snapshot_directional();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, ([1; 32], ep));
        assert_eq!(
            rows[0].1,
            DirectionalCounters {
                inbound: Counters {
                    frames: 1,
                    bytes: 100,
                },
                outbound: Counters {
                    frames: 2,
                    bytes: 300,
                },
                unattributed: Counters {
                    frames: 3,
                    bytes: 500,
                },
            }
        );

        let aggregate = Counters {
            frames: 6,
            bytes: 900,
        };
        assert_eq!(eval.counters().get(&[1; 32], &ep), Some(aggregate));
        assert_eq!(eval.counters().snapshot(), vec![(([1; 32], ep), aggregate)]);
    }

    #[test]
    fn directional_snapshot_is_empty_without_traffic() {
        let eval = evaluator();
        assert!(eval.counters().snapshot_directional().is_empty());
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn endpoint(seed: u8) -> EndpointId {
        [seed; 32]
    }

    fn policy_with(per_peer_bytes: u64) -> BudgetPolicy {
        BudgetPolicy {
            app_inbound: vec![BudgetLimit::bytes(3_600_000, u64::MAX / 4)],
            app_outbound: vec![BudgetLimit::bytes(3_600_000, u64::MAX / 4)],
            per_peer: vec![BudgetLimit::bytes(3_600_000, per_peer_bytes)],
            per_ip: vec![BudgetLimit::bytes(3_600_000, u64::MAX / 4)],
            counterparty_overrides: Vec::new(),
        }
    }

    fn budgeted(policy: BudgetPolicy) -> AbuseEvaluator {
        AbuseEvaluator::with_budgets(AbusePolicy::default(), [1; 32], policy)
    }

    #[test]
    fn budget_defaults_match_issue_2247() {
        let policy = BudgetPolicy::default();
        let mib: u64 = 1024 * 1024;
        let gib: u64 = 1024 * mib;
        assert_eq!(
            policy.app_inbound,
            vec![
                BudgetLimit::bytes(1_000, 32 * mib),
                BudgetLimit::bytes(3_600_000, 20 * gib),
            ]
        );
        assert_eq!(
            policy.app_outbound,
            vec![
                BudgetLimit::bytes(1_000, 64 * mib),
                BudgetLimit::bytes(3_600_000, 40 * gib),
            ]
        );
        assert_eq!(
            policy.per_peer,
            vec![
                BudgetLimit::bytes(60_000, 8 * mib),
                BudgetLimit::bytes(3_600_000, 100 * mib),
            ]
        );
        assert_eq!(
            policy.per_ip,
            vec![BudgetLimit::bytes(3_600_000, 250 * mib)]
        );
        assert!(policy.counterparty_overrides.is_empty());
    }

    #[test]
    fn per_peer_budget_exhaustion_refuses_with_bounded_retry() {
        let mut eval = budgeted(policy_with(1_000));
        let ep = endpoint(7);
        eval.check_budget(Direction::Inbound, ep, None, 600, 0)
            .expect("within budget");
        eval.check_budget(Direction::Inbound, ep, None, 400, 0)
            .expect("exactly at budget");
        let refusal = eval
            .check_budget(Direction::Inbound, ep, None, 1, 0)
            .expect_err("over budget");
        assert_eq!(refusal.reason, RefusalReason::BudgetExhausted);
        let retry = refusal.retry_after_ms.expect("temporary refusal has retry");
        assert!((RETRY_FLOOR_MS..=RETRY_CAP_MS).contains(&retry));
        // A different peer is unaffected by this peer's exhaustion.
        eval.check_budget(Direction::Inbound, endpoint(8), None, 1_000, 0)
            .expect("other peer unaffected");
        // Refused charges are not recorded.
        let usage = eval.budget_usage(0);
        let peer_row = usage
            .iter()
            .find(|(scope, _, _)| *scope == BudgetScope::Peer(ep))
            .expect("peer usage row");
        assert_eq!(peer_row.2.bytes, 1_000);
    }

    #[test]
    fn counterparty_override_replaces_default_in_both_directions() {
        let mut policy = policy_with(1_000);
        let capped = endpoint(9);
        let raised = endpoint(10);
        policy.counterparty_overrides = vec![
            (
                capped,
                vec![BudgetLimit {
                    window_ms: 86_400_000,
                    bytes_per_window: 100,
                    messages_per_window: 0,
                }],
            ),
            (raised, vec![BudgetLimit::bytes(3_600_000, 5_000)]),
        ];
        let mut eval = budgeted(policy);
        // The capped peer hits its 100-byte/day override, not the
        // 1_000-byte default.
        eval.check_budget(Direction::Inbound, capped, None, 100, 0)
            .expect("at override cap");
        assert!(
            eval.check_budget(Direction::Inbound, capped, None, 1, 0)
                .is_err()
        );
        // The raised peer may exceed the default per-peer budget.
        eval.check_budget(Direction::Inbound, raised, None, 3_000, 0)
            .expect("override raises the cap");
    }

    #[test]
    fn override_message_cap_counts_messages_not_bytes() {
        let mut policy = policy_with(u64::MAX / 4);
        let ep = endpoint(11);
        policy.counterparty_overrides = vec![(
            ep,
            vec![BudgetLimit {
                window_ms: 60_000,
                bytes_per_window: 0,
                messages_per_window: 2,
            }],
        )];
        let mut eval = budgeted(policy);
        eval.check_budget(Direction::Inbound, ep, None, 10, 0)
            .expect("message 1");
        eval.check_budget(Direction::Inbound, ep, None, 10, 0)
            .expect("message 2");
        let refusal = eval
            .check_budget(Direction::Inbound, ep, None, 10, 0)
            .expect_err("message 3 refused");
        assert_eq!(refusal.reason, RefusalReason::BudgetExhausted);
    }

    #[test]
    fn direct_ip_scope_is_shared_and_relayed_traffic_skips_it() {
        let mut policy = policy_with(u64::MAX / 4);
        policy.per_ip = vec![BudgetLimit::bytes(3_600_000, 1_000)];
        let mut eval = budgeted(policy);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        eval.check_budget(Direction::Inbound, endpoint(1), Some(ip), 700, 0)
            .expect("first endpoint on the ip");
        // A second endpoint behind the same direct IP shares its budget.
        assert!(
            eval.check_budget(Direction::Inbound, endpoint(2), Some(ip), 400, 0)
                .is_err()
        );
        // Relayed traffic (no directly observed IP) charges no IP scope.
        eval.check_budget(Direction::Inbound, endpoint(2), None, 400, 0)
            .expect("no ip observed, no ip scope");
        let usage = eval.budget_usage(0);
        let ip_row = usage
            .iter()
            .find(|(scope, _, _)| *scope == BudgetScope::Ip(ip))
            .expect("ip usage row");
        assert_eq!(ip_row.2.bytes, 700);
    }

    #[test]
    fn app_wide_scope_spans_peers_and_directions_are_separate() {
        let mut policy = policy_with(u64::MAX / 4);
        policy.app_inbound = vec![BudgetLimit::bytes(3_600_000, 1_000)];
        let mut eval = budgeted(policy);
        eval.check_budget(Direction::Inbound, endpoint(1), None, 800, 0)
            .expect("first peer inbound");
        assert!(
            eval.check_budget(Direction::Inbound, endpoint(2), None, 300, 0)
                .is_err()
        );
        // Outbound has its own app-wide scope.
        eval.check_budget(Direction::Outbound, endpoint(2), None, 300, 0)
            .expect("outbound separate");
    }

    #[test]
    fn budget_recovers_after_the_window_rolls() {
        let mut policy = policy_with(0);
        policy.per_peer = vec![BudgetLimit::bytes(1_000, 100)];
        let mut eval = budgeted(policy);
        let ep = endpoint(3);
        eval.check_budget(Direction::Inbound, ep, None, 100, 0)
            .expect("full window");
        assert!(
            eval.check_budget(Direction::Inbound, ep, None, 1, 0)
                .is_err()
        );
        // Two windows later the usage has fully aged out.
        eval.check_budget(Direction::Inbound, ep, None, 100, 2_000)
            .expect("recovered after roll");
        // Aged-out scopes prune away.
        assert!(eval.prune_budget_state(10_000) >= 1);
    }

    #[test]
    fn budget_usage_rolls_the_estimate_on_the_read_path() {
        // #4683: with no intervening charge, budget_usage must report
        // the rolled sliding-window estimate, not the last charged
        // window in full forever.
        let mut policy = policy_with(0);
        policy.per_peer = vec![BudgetLimit {
            window_ms: 1_000,
            bytes_per_window: 1_000,
            messages_per_window: 10,
        }];
        let mut eval = budgeted(policy);
        let ep = endpoint(11);
        eval.check_budget(Direction::Inbound, ep, None, 100, 0)
            .expect("first charge in the window");
        eval.check_budget(Direction::Inbound, ep, None, 100, 0)
            .expect("second charge in the window");
        let usage_at = |eval: &AbuseEvaluator, now_ms: u64| {
            let usage = eval.budget_usage(now_ms);
            let row = usage
                .iter()
                .find(|(scope, _, _)| *scope == BudgetScope::Peer(ep))
                .expect("peer usage row");
            (row.2.bytes, row.2.messages)
        };
        // In-window: the full charges count.
        assert_eq!(usage_at(&eval, 0), (200, 2));
        // A window and a half later, still with no intervening
        // charge: the charges are the previous window now, weighted
        // by the half of it still inside the rolling span.
        assert_eq!(usage_at(&eval, 1_500), (100, 1));
        // Two or more windows later they have fully aged out, and
        // admission at the same instant agrees with the report.
        assert_eq!(usage_at(&eval, 2_000), (0, 0));
        assert_eq!(usage_at(&eval, 100_000), (0, 0));
        eval.check_budget(Direction::Inbound, ep, None, 100, 100_000)
            .expect("admission agrees with the aged-out usage");
    }

    #[test]
    fn budgeted_frame_charge_is_atomic() {
        let mut policy = policy_with(0);
        policy.per_peer = vec![BudgetLimit::bytes(3_600_000, 10)];
        let mut eval = budgeted(policy);
        let ep = endpoint(4);
        // One frame of 10 bytes fits the budget exactly.
        eval.check_frame_and_bytes_budgeted(ep, true, 10, Direction::Inbound, None, 0)
            .expect("first frame");
        // The next frame is budget-refused: no frame token is spent
        // and no counter is recorded for it.
        let refusal = eval
            .check_frame_and_bytes_budgeted(ep, true, 1, Direction::Inbound, None, 0)
            .expect_err("budget exhausted");
        assert_eq!(refusal.reason, RefusalReason::BudgetExhausted);
        assert_eq!(
            eval.counters().get(&[1; 32], &ep),
            Some(Counters {
                frames: 1,
                bytes: 10
            })
        );
    }

    #[test]
    fn no_budget_policy_means_no_budget_refusals() {
        let mut eval = AbuseEvaluator::new(AbusePolicy::default(), [1; 32]);
        assert!(eval.budgets().is_none());
        eval.check_budget(
            Direction::Inbound,
            endpoint(5),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            u64::MAX / 2,
            0,
        )
        .expect("unconfigured budgets never refuse");
        assert!(eval.budget_usage(0).is_empty());
    }
}
