//! Waiting events: events that have arrived but cannot be applied yet.
//!
//! An event waits when a dependency has not arrived: a backlink, an `after`
//! hash, or an auth dependency. The pool holds the event until every dependency
//! has arrived, then releases it. Three caps bound what a peer can make a node
//! hold: at most [`WaitingLimits::per_author`] waiting events per author, at
//! most [`WaitingLimits::total`] waiting events across all authors, and a drop
//! once an event has waited [`WaitingLimits::max_age_ms`]. The total cap is
//! the one that binds an attacker: author keys are self-certifying, so minting
//! authors is free and a per-author cap alone bounds nothing. A dropped event
//! is not acknowledged, and sync sends it again once its dependency exists.
//!
//! The pool does no I/O and takes the clock as an explicit `now_ms`, so tests
//! use a fake clock. The event store holds one pool and mirrors it into the
//! `waiting` table (P1.11). [`WaitingPool::restore`] rebuilds the pool from those
//! rows on open. The pool keeps its own set of arrived keys, for the filter in
//! [`WaitingPool::insert`]. The store does not feed that set: it releases
//! waiters through [`WaitingPool::release`], which records nothing, and its
//! own applied table is its duplicate and replay record — so the set cannot
//! grow with every event the store ever applies (#1730).
//!
//! The caller's loop:
//!
//! 1. Call [`WaitingPool::insert`] for each event it cannot apply yet, then take
//!    [`WaitingPool::take_ready`] straight away. The events whose dependencies
//!    had all arrived are in that list. Apply them.
//! 2. After applying an event, call [`WaitingPool::arrived`] with its key. It
//!    returns the events that were waiting only on that key. Apply each one, and
//!    call `arrived` for its key in turn, so that causal order holds.
//! 3. Call [`WaitingPool::expire`] regularly and log what it returns.
//!
//! The pool returns events, not keys, so `E` must carry its own key for step 2.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::Hash;

/// An author's public key: the 32 bytes of its Ed25519 key.
pub type AuthorKey = [u8; 32];

/// Waiting events allowed per author, unless a world sets its own.
pub const DEFAULT_PER_AUTHOR: usize = 256;

/// Waiting events allowed across all authors, unless a world sets its own:
/// 32 authors' worth of the per-author cap. The per-author cap shapes one
/// author's share; this cap is the node's actual memory and disk bound,
/// because author keys are free to mint.
pub const DEFAULT_TOTAL: usize = 32 * DEFAULT_PER_AUTHOR;

const MS_PER_DAY: u64 = 24 * 60 * 60 * 1_000;

/// Age after which a waiting event is dropped, unless a world sets its own: 7 days.
pub const DEFAULT_MAX_AGE_MS: u64 = 7 * MS_PER_DAY;

/// The caps a world sets on its waiting events.
///
/// [`WaitingLimits::default`] gives the accepted values: 256 waiting events per
/// author, 8,192 across all authors, and a 7-day age cap. A world sets its own
/// with [`WaitingLimits::new`] and [`WaitingLimits::with_total`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitingLimits {
    per_author: usize,
    total: usize,
    max_age_ms: u64,
}

impl WaitingLimits {
    /// Caps for one world. `per_author` is the most waiting events one author may
    /// hold, and `max_age_ms` is how long an event may wait before it expires.
    /// The total across all authors is [`DEFAULT_TOTAL`]; set it with
    /// [`with_total`](Self::with_total).
    ///
    /// Zero is accepted. A `per_author` of zero refuses every event that would
    /// wait, and a `max_age_ms` of zero expires events at the next call.
    pub const fn new(per_author: usize, max_age_ms: u64) -> Self {
        Self {
            per_author,
            total: DEFAULT_TOTAL,
            max_age_ms,
        }
    }

    /// The same caps with a different total: the most waiting events the pool
    /// may hold across all authors. A total of zero refuses every event that
    /// would wait.
    pub const fn with_total(self, total: usize) -> Self {
        Self { total, ..self }
    }

    /// The most waiting events one author may hold.
    pub const fn per_author(self) -> usize {
        self.per_author
    }

    /// The most waiting events the pool may hold across all authors.
    pub const fn total(self) -> usize {
        self.total
    }

    /// How long an event may wait before it expires, in milliseconds.
    pub const fn max_age_ms(self) -> u64 {
        self.max_age_ms
    }

    /// Whether an event that arrived at `arrived_at_ms` is due at `now_ms`, that is, it has
    /// waited [`max_age_ms`](Self::max_age_ms) or longer. This is the rule
    /// [`WaitingPool::expire`] applies, for a store that filters its rows on open. A clock
    /// that is behind the arrival time has not made the event due.
    pub const fn is_due(self, arrived_at_ms: u64, now_ms: u64) -> bool {
        match now_ms.checked_sub(arrived_at_ms) {
            Some(waited) => waited >= self.max_age_ms,
            None => false,
        }
    }
}

impl Default for WaitingLimits {
    fn default() -> Self {
        Self::new(DEFAULT_PER_AUTHOR, DEFAULT_MAX_AGE_MS)
    }
}

/// Why [`WaitingPool::insert`] refused an event. The event is dropped and not
/// acknowledged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WaitRefusal {
    /// The author already holds the per-author cap of waiting events.
    #[error("the author already holds the maximum number of waiting events")]
    AuthorFull,
    /// The pool already holds the total cap of waiting events across all
    /// authors. Distinct from [`AuthorFull`](Self::AuthorFull) so a caller can
    /// tell a Sybil-shaped flood (many authors, pool full) from one author
    /// over its own share; the event store's ingest surfaces both as its one
    /// waiting-full refusal, because the remedy for the sender is the same.
    #[error("the pool already holds the maximum number of waiting events")]
    PoolFull,
}

/// One held event and the dependencies it still waits for.
#[derive(Debug)]
struct Waiting<K, E> {
    author: AuthorKey,
    key: K,
    arrived_at: u64,
    missing: HashSet<K>,
    event: E,
}

/// Events held until their dependencies arrive.
///
/// `K` is an event's key (its hash), and `E` is the event the pool stores. The
/// pool releases events in arrival order: the order `insert` accepted them.
#[derive(Debug)]
pub struct WaitingPool<K, E> {
    limits: WaitingLimits,
    /// The sequence number the next accepted event gets. Sequence numbers give
    /// arrival order.
    next_seq: u64,
    /// Held events by sequence number.
    waiting: BTreeMap<u64, Waiting<K, E>>,
    /// The sequence number of each held event, by its key.
    by_key: HashMap<K, u64>,
    /// The sequence numbers of the held events that still miss each dependency.
    by_dep: HashMap<K, BTreeSet<u64>>,
    /// Held events by (arrival time, sequence number), for the age cap.
    by_time: BTreeSet<(u64, u64)>,
    /// The sequence numbers of each author's held events. An author with none has no
    /// entry.
    held: HashMap<AuthorKey, BTreeSet<u64>>,
    /// Keys that have arrived, and stay arrived. A dependency in this set is already satisfied.
    /// Only the standalone [`insert`](Self::insert)/[`park`](Self::park) protocol
    /// records here, through [`arrived`](Self::arrived). The event store never
    /// does: it releases through [`release`](Self::release) and tracks applied
    /// keys in its own table, so this set stays empty on the store path (#1730).
    arrived: HashSet<K>,
    /// Events whose dependencies had all arrived when they were inserted.
    ready: Vec<E>,
    /// The keys of the events in `ready`. The ready queue is the one
    /// state a key can be in that is neither held (`by_key`) nor
    /// arrived, and without this index the duplicate guard in
    /// `insert`/`park` could not see it: the same key inserted twice
    /// before `take_ready` was handed out twice (#2001).
    ready_keys: HashSet<K>,
    /// Events dropped by the age cap, for `expire` to return.
    expired: Vec<(AuthorKey, E)>,
}

impl<K, E> WaitingPool<K, E>
where
    K: Clone + Eq + Hash,
{
    /// An empty pool under `limits`.
    pub fn new(limits: WaitingLimits) -> Self {
        Self {
            limits,
            next_seq: 0,
            waiting: BTreeMap::new(),
            by_key: HashMap::new(),
            by_dep: HashMap::new(),
            by_time: BTreeSet::new(),
            held: HashMap::new(),
            arrived: HashSet::new(),
            ready: Vec::new(),
            ready_keys: HashSet::new(),
            expired: Vec::new(),
        }
    }

    /// The number of events held, across all authors.
    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    /// Whether the pool holds no events.
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    /// The number of events `author` has held. Never above the per-author cap.
    pub fn waiting_count(&self, author: AuthorKey) -> usize {
        self.held.get(&author).map_or(0, BTreeSet::len)
    }

    /// Hold `event` until each of `deps` has arrived.
    ///
    /// `key` is the event's own key, and `deps` are the keys it needs. A
    /// dependency that has already arrived is not waited for. If every dependency
    /// has arrived, the event is not held: it is returned by
    /// [`take_ready`](Self::take_ready), and it takes no slot under the cap. An
    /// event whose key is already held, has arrived, or is ready is a duplicate,
    /// and it is ignored.
    ///
    /// Events due at `now_ms` expire before the insert, so a refusal changes
    /// nothing except that expiry.
    ///
    /// # Errors
    ///
    /// [`WaitRefusal::AuthorFull`] when the event would wait and its author
    /// already holds the per-author cap, or [`WaitRefusal::PoolFull`] when the
    /// pool already holds the total cap across all authors.
    pub fn insert(
        &mut self,
        author: AuthorKey,
        key: K,
        deps: impl IntoIterator<Item = K>,
        event: E,
        now_ms: u64,
    ) -> Result<(), WaitRefusal> {
        self.expire_due(now_ms);
        if self.arrived.contains(&key)
            || self.by_key.contains_key(&key)
            || self.ready_keys.contains(&key)
        {
            return Ok(());
        }
        let missing: HashSet<K> = deps
            .into_iter()
            .filter(|dep| !self.arrived.contains(dep))
            .collect();
        if missing.is_empty() {
            self.ready_keys.insert(key);
            self.ready.push(event);
            return Ok(());
        }
        self.hold(author, key, missing, event, now_ms)
    }

    /// Hold `event` for good: it waits for no dependency, so no arrival releases it,
    /// and only the age cap drops it. The caller parks an event that can never be
    /// applied, but must still see when another event conflicts with it.
    ///
    /// An event whose key is already held, has arrived, or is ready is ignored,
    /// as for [`insert`](Self::insert).
    ///
    /// # Errors
    ///
    /// [`WaitRefusal::AuthorFull`] when the author already holds the per-author
    /// cap, or [`WaitRefusal::PoolFull`] when the pool already holds the total
    /// cap across all authors.
    pub fn park(
        &mut self,
        author: AuthorKey,
        key: K,
        event: E,
        now_ms: u64,
    ) -> Result<(), WaitRefusal> {
        self.expire_due(now_ms);
        if self.arrived.contains(&key)
            || self.by_key.contains_key(&key)
            || self.ready_keys.contains(&key)
        {
            return Ok(());
        }
        self.hold(author, key, HashSet::new(), event, now_ms)
    }

    /// Whether an event with this key is held: waiting, or parked.
    pub fn contains(&self, key: &K) -> bool {
        self.by_key.contains_key(key)
    }

    /// Holds an event that arrived at `arrived_at`. The store uses it for every hold: when it
    /// restores its rows on open, and when it holds an event it has decided, which keeps the
    /// event's first arrival time.
    ///
    /// Unlike [`insert`](Self::insert) and [`park`](Self::park), nothing expires first, and an
    /// empty `missing` does not release the event: it is held for good, as a parked event is.
    /// The caller holds the events in the order they arrived, so they are released in that
    /// order. An event whose key is already held is ignored.
    ///
    /// # Errors
    ///
    /// [`WaitRefusal::AuthorFull`] when the author already holds the per-author
    /// cap, or [`WaitRefusal::PoolFull`] when the pool already holds the total
    /// cap across all authors. On restore-on-open the store drops a refused
    /// row's event, exactly as it drops an expired one, so a store written
    /// under a larger cap still opens under a smaller one: the earliest
    /// arrivals are kept and the excess is dropped, never wedged.
    pub fn restore(
        &mut self,
        author: AuthorKey,
        key: K,
        missing: impl IntoIterator<Item = K>,
        event: E,
        arrived_at: u64,
    ) -> Result<(), WaitRefusal> {
        if self.by_key.contains_key(&key) {
            return Ok(());
        }
        let missing: HashSet<K> = missing.into_iter().collect();
        self.hold(author, key, missing, event, arrived_at)
    }

    /// The held events of `author`, in sequence order of arrival. Parked events are
    /// included.
    pub fn held_by(&self, author: &AuthorKey) -> impl Iterator<Item = &E> {
        self.held
            .get(author)
            .into_iter()
            .flatten()
            .filter_map(|seq| self.waiting.get(seq))
            .map(|entry| &entry.event)
    }

    /// Stores `event` under `key`, waiting for `missing`, under the caps.
    /// The total cap is checked first: it is the node's resource bound, and
    /// when the pool is full every further hold is refused whatever author it
    /// names.
    fn hold(
        &mut self,
        author: AuthorKey,
        key: K,
        missing: HashSet<K>,
        event: E,
        now_ms: u64,
    ) -> Result<(), WaitRefusal> {
        if self.len() >= self.limits.total() {
            return Err(WaitRefusal::PoolFull);
        }
        if self.waiting_count(author) >= self.limits.per_author() {
            return Err(WaitRefusal::AuthorFull);
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        for dep in &missing {
            self.by_dep.entry(dep.clone()).or_default().insert(seq);
        }
        self.by_time.insert((now_ms, seq));
        self.by_key.insert(key.clone(), seq);
        self.held.entry(author).or_default().insert(seq);
        self.waiting.insert(
            seq,
            Waiting {
                author,
                key,
                arrived_at: now_ms,
                missing,
                event,
            },
        );
        Ok(())
    }

    /// Record that `key` has arrived, and release the events that were waiting
    /// only for it.
    ///
    /// Returns the released events in arrival order. Each one has all its
    /// dependencies but has not arrived itself: apply it, then call `arrived` with
    /// its key, which can release more events. Events due at `now_ms` expire first,
    /// so an expired event is never returned. If `key` was itself held, that copy
    /// is dropped without being returned, because the caller now holds the event.
    pub fn arrived(&mut self, key: K, now_ms: u64) -> Vec<E> {
        let released = self.release(key.clone(), now_ms);
        self.arrived.insert(key);
        released
    }

    /// Release the events that were waiting only for `key`, exactly as
    /// [`arrived`](Self::arrived) does — the held copy of `key` is dropped and
    /// events due at `now_ms` expire first — but record nothing: `key` is not
    /// added to the arrived set, so a later [`insert`](Self::insert) does not
    /// treat it as a satisfied dependency or as a duplicate.
    ///
    /// This is the event store's path (#1730). The store calls it for every
    /// applied event and every fork proof, and its own applied table already
    /// recognises duplicates and replays; recording each key here as well
    /// grew this pool by one set entry per event ever applied, in a pool no
    /// store caller ever read back.
    pub fn release(&mut self, key: K, now_ms: u64) -> Vec<E> {
        self.expire_due(now_ms);
        if let Some(seq) = self.by_key.get(&key).copied() {
            self.remove(seq);
        }
        let waiters = self.by_dep.remove(&key).unwrap_or_default();
        let mut released = Vec::new();
        for seq in waiters {
            let Some(entry) = self.waiting.get_mut(&seq) else {
                continue;
            };
            entry.missing.remove(&key);
            if !entry.missing.is_empty() {
                continue;
            }
            if let Some(done) = self.remove(seq) {
                released.push(done.event);
            }
        }
        released
    }

    /// Drop every held event that has waited `max_age_ms` by `now_ms`, and return
    /// them as `(author, event)` in arrival order, so the caller can log them.
    ///
    /// The cap is inclusive. An event is dropped when `now_ms - arrived_at` reaches
    /// `max_age_ms`, and one millisecond earlier it is kept. Each event is returned
    /// once.
    pub fn expire(&mut self, now_ms: u64) -> Vec<(AuthorKey, E)> {
        self.expire_due(now_ms);
        std::mem::take(&mut self.expired)
    }

    /// The events whose dependencies had all arrived when they were inserted, in
    /// insert order. Take them right after each `insert`, before the next call.
    pub fn take_ready(&mut self) -> Vec<E> {
        self.ready_keys.clear();
        std::mem::take(&mut self.ready)
    }

    /// Move the events that have reached the age cap at `now_ms` into `expired`.
    /// Every operation that takes a clock calls this first, so counts and releases
    /// never include an expired event.
    fn expire_due(&mut self, now_ms: u64) {
        let Some(cutoff) = now_ms.checked_sub(self.limits.max_age_ms()) else {
            return;
        };
        let due: Vec<u64> = self
            .by_time
            .range(..=(cutoff, u64::MAX))
            .map(|&(_, seq)| seq)
            .collect();
        for seq in due {
            if let Some(entry) = self.remove(seq) {
                self.expired.push((entry.author, entry.event));
            }
        }
    }

    /// Take a held event out of every index, and return it.
    fn remove(&mut self, seq: u64) -> Option<Waiting<K, E>> {
        let entry = self.waiting.remove(&seq)?;
        self.by_time.remove(&(entry.arrived_at, seq));
        self.by_key.remove(&entry.key);
        for dep in &entry.missing {
            if let Some(waiters) = self.by_dep.get_mut(dep) {
                waiters.remove(&seq);
                if waiters.is_empty() {
                    self.by_dep.remove(dep);
                }
            }
        }
        if let Some(seqs) = self.held.get_mut(&entry.author) {
            seqs.remove(&seq);
            if seqs.is_empty() {
                self.held.remove(&entry.author);
            }
        }
        Some(entry)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    type Pool = WaitingPool<u32, &'static str>;

    const WEEK_MS: u64 = 7 * MS_PER_DAY;

    fn author(n: u8) -> AuthorKey {
        [n; 32]
    }

    fn pool() -> Pool {
        WaitingPool::new(WaitingLimits::default())
    }

    #[test]
    fn test_defaults_are_256_per_author_and_seven_days() {
        let limits = WaitingLimits::default();
        assert_eq!(limits.per_author(), 256);
        assert_eq!(limits.total(), DEFAULT_TOTAL);
        assert_eq!(limits.max_age_ms(), 604_800_000);
    }

    #[test]
    fn test_limits_are_set_per_world() {
        let limits = WaitingLimits::new(2, 1_000);
        assert_eq!((limits.per_author(), limits.max_age_ms()), (2, 1_000));
        assert_eq!(limits.total(), DEFAULT_TOTAL);
        assert_eq!(limits.with_total(7).total(), 7);
        let mut pool = WaitingPool::new(limits);
        pool.insert(author(1), 1, [10], "a", 0).unwrap();
        pool.insert(author(1), 2, [10], "b", 0).unwrap();
        assert_eq!(
            pool.insert(author(1), 3, [10], "c", 0),
            Err(WaitRefusal::AuthorFull)
        );
        assert!(pool.expire(999).is_empty());
        assert_eq!(pool.expire(1_000).len(), 2);
    }

    #[test]
    fn test_per_author_cap_admits_256_and_refuses_the_257th() {
        let mut pool = pool();
        let a = author(1);
        for key in 0..255 {
            pool.insert(a, key, [1_000 + key], "held", 0).unwrap();
        }
        assert_eq!(pool.waiting_count(a), 255);
        pool.insert(a, 255, [1_255], "held", 0).unwrap();
        assert_eq!(pool.waiting_count(a), 256);

        assert_eq!(
            pool.insert(a, 256, [1_256], "refused", 0),
            Err(WaitRefusal::AuthorFull)
        );
        assert_eq!(pool.waiting_count(a), 256);
        assert_eq!(pool.len(), 256);
        assert!(
            pool.arrived(1_256, 0).is_empty(),
            "a refused event is not held"
        );

        pool.insert(author(2), 300, [2_000], "other author", 0)
            .unwrap();
        assert_eq!(pool.waiting_count(author(2)), 1);
    }

    #[test]
    fn test_expiry_is_inclusive_at_max_age_and_exclusive_one_ms_before() {
        let mut pool = pool();
        pool.insert(author(1), 1, [9], "e1", 1_000).unwrap();
        assert!(pool.expire(1_000 + WEEK_MS - 1).is_empty());
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.expire(1_000 + WEEK_MS), vec![(author(1), "e1")]);
        assert_eq!(pool.waiting_count(author(1)), 0);
        assert!(pool.is_empty());
        assert!(pool.expire(1_000 + WEEK_MS).is_empty(), "reported once");
    }

    #[test]
    fn test_an_expired_event_is_not_released_by_its_dependency() {
        let mut pool = pool();
        pool.insert(author(1), 1, [9], "e1", 0).unwrap();
        assert!(pool.arrived(9, WEEK_MS).is_empty());
        assert_eq!(pool.expire(WEEK_MS), vec![(author(1), "e1")]);
    }

    #[test]
    fn test_dependencies_arriving_out_of_order_release_in_arrival_order() {
        let mut pool = pool();
        pool.insert(author(1), 3, [10], "c", 0).unwrap();
        pool.insert(author(1), 2, [11], "b", 0).unwrap();
        pool.insert(author(1), 1, [10], "a", 0).unwrap();
        pool.insert(author(1), 4, [12], "d", 0).unwrap();
        assert_eq!(pool.arrived(11, 0), vec!["b"]);
        assert_eq!(pool.arrived(10, 0), vec!["c", "a"]);
        assert_eq!(pool.arrived(12, 0), vec!["d"]);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_a_chain_releases_one_link_per_arrival() {
        let mut pool = pool();
        pool.insert(author(1), 1, [2], "a", 0).unwrap();
        pool.insert(author(1), 2, [3], "b", 0).unwrap();
        assert_eq!(pool.arrived(3, 0), vec!["b"]);
        assert_eq!(pool.arrived(2, 0), vec!["a"]);
    }

    #[test]
    fn test_an_event_with_two_deps_waits_for_both() {
        let mut pool = pool();
        pool.insert(author(1), 1, [10, 11], "e", 0).unwrap();
        assert!(pool.arrived(10, 0).is_empty());
        assert_eq!(pool.waiting_count(author(1)), 1);
        assert_eq!(pool.arrived(11, 0), vec!["e"]);
        assert_eq!(pool.waiting_count(author(1)), 0);

        pool.insert(author(1), 2, [20, 21], "f", 0).unwrap();
        assert!(pool.arrived(21, 0).is_empty());
        assert_eq!(pool.arrived(20, 0), vec!["f"]);
    }

    #[test]
    fn test_a_repeated_dep_counts_once() {
        let mut pool = pool();
        pool.insert(author(1), 1, [30, 30], "g", 0).unwrap();
        assert_eq!(pool.arrived(30, 0), vec!["g"]);
    }

    #[test]
    fn test_an_event_whose_deps_arrived_is_ready_at_once() {
        let mut pool = pool();
        pool.arrived(10, 0);
        pool.insert(author(1), 1, [10], "ready", 0).unwrap();
        assert_eq!(pool.take_ready(), vec!["ready"]);
        assert_eq!(pool.waiting_count(author(1)), 0);
        assert!(pool.is_empty());
        assert!(pool.take_ready().is_empty());
    }

    #[test]
    fn test_an_event_with_no_deps_is_ready_at_once() {
        let mut pool = pool();
        pool.insert(author(1), 1, [], "free", 0).unwrap();
        assert_eq!(pool.take_ready(), vec!["free"]);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_a_ready_event_is_not_refused_by_a_full_author() {
        let mut pool = WaitingPool::new(WaitingLimits::new(1, WEEK_MS));
        pool.insert(author(1), 1, [10], "held", 0).unwrap();
        pool.arrived(20, 0);
        pool.insert(author(1), 2, [20], "ready", 0).unwrap();
        assert_eq!(pool.take_ready(), vec!["ready"]);
        assert_eq!(pool.waiting_count(author(1)), 1);
        assert_eq!(
            pool.insert(author(1), 3, [30], "refused", 0),
            Err(WaitRefusal::AuthorFull)
        );
    }

    #[test]
    fn test_a_partly_arrived_event_waits_for_the_rest() {
        let mut pool = pool();
        pool.arrived(10, 0);
        pool.insert(author(1), 1, [10, 11], "e", 0).unwrap();
        assert!(pool.take_ready().is_empty());
        assert_eq!(pool.arrived(11, 0), vec!["e"]);
    }

    #[test]
    fn test_a_duplicate_key_is_ignored() {
        let mut pool = pool();
        pool.insert(author(1), 1, [10], "first", 0).unwrap();
        pool.insert(author(1), 1, [10], "second", 0).unwrap();
        assert_eq!(pool.waiting_count(author(1)), 1);
        assert_eq!(pool.arrived(10, 0), vec!["first"]);

        pool.arrived(1, 0);
        pool.insert(author(1), 1, [10], "again", 0).unwrap();
        assert!(pool.take_ready().is_empty());
        assert!(pool.is_empty());
    }

    #[test]
    fn test_a_duplicate_key_in_the_ready_queue_is_ignored() {
        // #2001: an event whose dependencies have all arrived sits
        // in the ready queue, recorded in neither by_key nor
        // arrived, so the duplicate guard missed it and a second
        // insert pushed a second copy for take_ready to hand out.
        let mut pool = pool();
        pool.arrived(10, 0);
        pool.insert(author(1), 1, [10], "first", 0).unwrap();
        pool.insert(author(1), 1, [10], "second", 0).unwrap();
        assert_eq!(pool.take_ready(), vec!["first"]);

        // The no-deps form is ready immediately and guards the same.
        pool.insert(author(1), 2, [], "one", 0).unwrap();
        pool.insert(author(1), 2, [], "two", 0).unwrap();
        assert_eq!(pool.take_ready(), vec!["one"]);

        // Parking a ready key is a duplicate too: it must not create
        // a held copy of an event that is about to be applied.
        pool.insert(author(1), 3, [], "ready", 0).unwrap();
        pool.park(author(1), 3, "parked", 0).unwrap();
        assert!(!pool.contains(&3));
        assert_eq!(pool.take_ready(), vec!["ready"]);
    }

    #[test]
    fn test_arrival_of_a_held_event_drops_the_held_copy_without_releasing_it() {
        let mut pool = pool();
        pool.insert(author(1), 1, [10], "held", 0).unwrap();
        assert!(pool.arrived(1, 0).is_empty());
        assert_eq!(pool.waiting_count(author(1)), 0);
        assert!(pool.arrived(10, 0).is_empty());
    }

    #[test]
    fn test_counts_stay_exact_across_insert_release_and_expiry() {
        let mut pool = pool();
        pool.insert(author(1), 1, [10], "a", 0).unwrap();
        pool.insert(author(1), 2, [11], "b", 100).unwrap();
        pool.insert(author(2), 3, [12], "c", 100).unwrap();
        assert_eq!(pool.waiting_count(author(1)), 2);
        assert_eq!(pool.waiting_count(author(2)), 1);
        assert_eq!(pool.len(), 3);

        assert_eq!(pool.arrived(10, 200), vec!["a"]);
        assert_eq!(pool.waiting_count(author(1)), 1);
        assert_eq!(pool.len(), 2);

        assert_eq!(
            pool.expire(100 + WEEK_MS),
            vec![(author(1), "b"), (author(2), "c")]
        );
        assert_eq!(pool.waiting_count(author(1)), 0);
        assert_eq!(pool.waiting_count(author(2)), 0);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_restore_holds_a_parked_event_and_counts_its_age_from_arrival() {
        let mut pool = pool();
        pool.restore(author(1), 1, [], "parked", 500).unwrap();
        assert!(
            pool.take_ready().is_empty(),
            "an empty missing set does not release the event"
        );
        assert!(pool.contains(&1));
        assert_eq!(pool.waiting_count(author(1)), 1);
        assert!(pool.expire(500 + WEEK_MS - 1).is_empty());
        assert_eq!(pool.expire(500 + WEEK_MS), vec![(author(1), "parked")]);
    }

    #[test]
    fn test_restore_keeps_the_cap_and_ignores_a_key_already_held() {
        let mut pool = WaitingPool::new(WaitingLimits::new(1, WEEK_MS));
        pool.restore(author(1), 1, [10], "first", 0).unwrap();
        pool.restore(author(1), 1, [10], "again", 0).unwrap();
        assert_eq!(pool.waiting_count(author(1)), 1);
        assert_eq!(
            pool.restore(author(1), 2, [11], "refused", 0),
            Err(WaitRefusal::AuthorFull)
        );
        assert!(!pool.contains(&2));
        assert_eq!(pool.arrived(10, 0), vec!["first"]);
    }

    #[test]
    fn test_is_due_agrees_with_expire_at_every_boundary() {
        for max_age_ms in [0, 5, 10] {
            for arrived_at in [0, 3, 10] {
                for now_ms in [0, 4, 9, 10, 11, 15, 20] {
                    let limits = WaitingLimits::new(8, max_age_ms);
                    let mut pool = WaitingPool::new(limits);
                    pool.insert(author(1), 1, [9], "e", arrived_at).unwrap();
                    let dropped = !pool.expire(now_ms).is_empty();
                    assert_eq!(
                        dropped,
                        limits.is_due(arrived_at, now_ms),
                        "max age {max_age_ms}, arrived {arrived_at}, now {now_ms}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_total_cap_refuses_across_authors_below_the_per_author_cap() {
        // The (total+1)-th held event is refused even though every author is
        // far below the per-author cap: minting authors does not mint room.
        let limits = WaitingLimits::new(256, WEEK_MS).with_total(3);
        let mut pool = WaitingPool::new(limits);
        for n in 1..=3u8 {
            pool.insert(author(n), u32::from(n), [100], "held", 0)
                .unwrap();
            assert_eq!(pool.waiting_count(author(n)), 1);
        }
        assert_eq!(pool.len(), 3);
        assert_eq!(
            pool.insert(author(4), 4, [100], "refused", 0),
            Err(WaitRefusal::PoolFull)
        );
        assert_eq!(pool.len(), 3);
        assert!(!pool.contains(&4), "a refused event is not held");
    }

    #[test]
    fn test_total_cap_bounds_park_and_restore_too() {
        // Every path into the pool funnels through the one hold, so the total
        // cap binds parked and restored events as it binds inserts.
        let limits = WaitingLimits::new(256, WEEK_MS).with_total(1);
        let mut pool = WaitingPool::new(limits);
        pool.park(author(1), 1, "parked", 0).unwrap();
        assert_eq!(
            pool.insert(author(2), 2, [100], "refused", 0),
            Err(WaitRefusal::PoolFull)
        );
        assert_eq!(
            pool.restore(author(2), 2, [100], "refused", 0),
            Err(WaitRefusal::PoolFull)
        );
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_restore_over_a_lowered_total_keeps_the_earliest_arrivals() {
        // Rows written under a larger total, restored under a smaller one:
        // the earliest arrivals are held and the excess is refused, so the
        // store that owns the rows can drop them and open.
        let limits = WaitingLimits::new(256, WEEK_MS).with_total(2);
        let mut pool = WaitingPool::new(limits);
        pool.restore(author(1), 1, [100], "first", 0).unwrap();
        pool.restore(author(2), 2, [100], "second", 1).unwrap();
        assert_eq!(
            pool.restore(author(3), 3, [100], "excess", 2),
            Err(WaitRefusal::PoolFull)
        );
        assert_eq!(pool.len(), 2);
        assert!(pool.contains(&1) && pool.contains(&2) && !pool.contains(&3));
    }

    #[test]
    fn test_release_releases_waiters_without_recording_the_key() {
        let mut pool: WaitingPool<u32, &str> = WaitingPool::new(WaitingLimits::default());
        let author = [7u8; 32];
        pool.insert(author, 10, [20], "a", 0).unwrap();
        // release frees the waiter exactly as arrived does...
        assert_eq!(pool.release(20, 0), vec!["a"]);
        // ...but the key was not recorded: an event depending on it still
        // waits, and the key itself is not treated as a duplicate (#1730).
        pool.insert(author, 11, [20], "b", 0).unwrap();
        assert!(pool.take_ready().is_empty());
        assert_eq!(pool.len(), 1);
        pool.insert(author, 20, [], "c", 0).unwrap();
        assert_eq!(pool.take_ready(), vec!["c"]);
    }

    #[test]
    fn test_release_drops_the_held_copy_like_arrived() {
        let mut pool: WaitingPool<u32, &str> = WaitingPool::new(WaitingLimits::default());
        let author = [7u8; 32];
        pool.insert(author, 10, [20], "a", 0).unwrap();
        assert!(pool.release(10, 0).is_empty());
        assert!(pool.is_empty());
    }
}
