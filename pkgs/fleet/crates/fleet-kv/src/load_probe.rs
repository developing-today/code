//! The five-agent load probe (#13316 acceptance 6): a correctness-only
//! fixture, not a latency, throughput or billing claim.
//!
//! Five agents on five hosts each write 1 to 10 keys per simulated minute
//! into one shared ~10k-key namespace, a fifth of them on a small hot set so
//! they collide. Each sweeps every minute and, by policy, rebases every key
//! that conflicted (re-proposing its own latest value as a new intent). The
//! probe checks that every agent drains, that every agent's final value for
//! every key it wrote is a committed central version, that every losing body
//! is a retrievable conflict, that history has no gaps, and that a polling
//! reader never sees more than the documented 1 s staleness.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::central::{
    self, Address, Namespace, Outcome, REGISTRY_ID, SqlitePrimary, Statement, define_namespace,
};
use crate::read::{CACHE_MAX_AGE_MS, Found, Reader, Source};
use crate::sweep::{SweepReport, Sweeper, rebase};
use crate::sync::pull_namespaces;
use crate::{Identity, LocalStore};

const NAMESPACE: &str = "fleet";
const SCOPE: &str = "fleet-shared";
const KEYS: u32 = 10_000;
const HOT: u32 = 20;
const AGENTS: usize = 5;
const MINUTES: i64 = 12;

/// A small deterministic generator (SplitMix64), so the probe replays.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u32) -> u32 {
        (self.next() % u64::from(bound)) as u32
    }
}

fn key_name(index: u32) -> String {
    format!("k{index:05}")
}

fn address(key: &str) -> Address {
    Address::new(SCOPE, NAMESPACE, key)
}

/// Seeds `KEYS` live keys at version 1 in one transaction.
fn seed(primary: &SqlitePrimary) {
    let numbers = format!(
        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < {})",
        KEYS - 1
    );
    let statements = [
        format!(
            "{numbers} INSERT INTO kv_heads (scope_id, registry_id, namespace, key, version, \
             state, value, writer, written_at_ms) SELECT '{SCOPE}', '{REGISTRY_ID}', \
             '{NAMESPACE}', printf('k%05d', i), 1, 'live', CAST('seed' AS BLOB), 'seed', 0 FROM n"
        ),
        format!(
            "{numbers} INSERT INTO kv_versions (scope_id, registry_id, namespace, key, version, \
             parent_version, state, value, writer, host_incarnation, outbox_id, written_at_ms) \
             SELECT '{SCOPE}', '{REGISTRY_ID}', '{NAMESPACE}', printf('k%05d', i), 1, 0, 'live', \
             CAST('seed' AS BLOB), 'seed', 'seed', printf('seed-%05d', i), 0 FROM n"
        ),
    ];
    let statements: Vec<Statement> = statements
        .into_iter()
        .map(|sql| Statement {
            sql,
            args: Vec::new(),
        })
        .collect();
    primary.transaction_unlisted(&statements).expect("seed");
}

struct Agent {
    store: LocalStore,
    owner: String,
    partition: String,
    /// The value this agent last wrote to each key.
    latest: BTreeMap<String, Vec<u8>>,
}

impl Agent {
    fn identity(&self) -> Identity<'_> {
        Identity::new(&self.owner, "fleet-ctx", &self.partition)
    }

    fn sweep(&self, primary: &SqlitePrimary) -> SweepReport {
        let resolve = |namespace: &str| central::resolve(primary, REGISTRY_ID, namespace);
        let mut sleep = |_: Duration| {};
        Sweeper {
            primary,
            writer: &self.owner,
            resolve: &resolve,
            sleep: &mut sleep,
            blobs: None,
        }
        .sweep(&self.store, &self.identity())
        .expect("sweep")
    }

    /// Sweeps, then rebases every key that conflicted or is blocked.
    /// Returns the number of conflicts this sweep recorded.
    fn sync(&self, primary: &SqlitePrimary, definition: &Namespace) -> usize {
        let report = self.sweep(primary);
        let mut stuck: BTreeSet<String> = report
            .swept
            .iter()
            .filter(|swept| matches!(swept.outcome, Outcome::Conflict { .. }))
            .map(|swept| swept.key.clone())
            .collect();
        stuck.extend(
            report
                .waiting
                .iter()
                .filter(|waiting| waiting.reason == "blocked_by_conflict")
                .map(|waiting| waiting.key.clone()),
        );
        for key in &stuck {
            rebase(
                &self.store,
                &self.identity(),
                primary,
                definition,
                NAMESPACE,
                key,
            )
            .expect("rebase");
        }
        report.conflicts()
    }

    fn pending(&self) -> usize {
        self.latest
            .keys()
            .map(|key| {
                self.store
                    .outbox(&self.identity(), NAMESPACE, key)
                    .expect("outbox")
                    .iter()
                    .filter(|intent| intent.status == "pending")
                    .count()
            })
            .sum()
    }
}

#[test]
fn central_five_agents_converge_on_a_ten_thousand_key_namespace() {
    let primary = SqlitePrimary::in_memory().expect("primary");
    define_namespace(&primary, &Namespace::shared(NAMESPACE, SCOPE)).expect("define");
    seed(&primary);
    let definition = central::resolve(&primary, REGISTRY_ID, NAMESPACE).expect("resolve");
    let agents: Vec<Agent> = (0..AGENTS)
        .map(|index| {
            let store = LocalStore::in_memory().expect("store");
            pull_namespaces(&store, &primary).expect("pull");
            Agent {
                store,
                owner: format!("agent-{index}"),
                partition: format!("session-{index}"),
                latest: BTreeMap::new(),
            }
        })
        .collect();
    let mut agents = agents;
    let mut rng = Rng(13316);
    let mut conflicts = 0;
    let mut writes = 0;
    for minute in 0..MINUTES {
        for agent in agents.iter_mut() {
            for write in 0..=rng.below(10) {
                let index = if rng.below(5) == 0 {
                    rng.below(HOT)
                } else {
                    rng.below(KEYS)
                };
                let key = key_name(index);
                let value = format!("{}-m{minute}-w{write}", agent.owner).into_bytes();
                agent
                    .store
                    .set(&agent.identity(), NAMESPACE, &key, &value)
                    .expect("set");
                agent.latest.insert(key, value);
                writes += 1;
            }
        }
        // Rotate who sweeps first, so no agent always wins the hot keys.
        for offset in 0..AGENTS {
            let agent = &agents[(minute as usize + offset) % AGENTS];
            conflicts += agent.sync(&primary, &definition);
        }
    }
    // Drain: no new writes; every agent keeps sweeping and rebasing.
    let mut rounds = 0;
    while agents.iter().any(|agent| agent.pending() > 0) {
        rounds += 1;
        assert!(rounds <= 2 * AGENTS, "agents did not drain");
        for agent in &agents {
            conflicts += agent.sync(&primary, &definition);
        }
    }
    assert!(writes >= AGENTS * MINUTES as usize, "{writes} writes");
    assert!(conflicts > 0, "the hot set should collide");

    // Every agent's final value for every key it wrote is a committed
    // central version written by that agent.
    for agent in &agents {
        for (key, value) in &agent.latest {
            let history = central::history(&primary, &address(key)).expect("history");
            assert!(
                history
                    .iter()
                    .any(|version| version.writer == agent.owner && &version.value == value),
                "{}'s final value for {key} never committed",
                agent.owner
            );
        }
    }
    // History has no gaps, and each head is its history's last version.
    let touched: BTreeSet<&String> = agents
        .iter()
        .flat_map(|agent| agent.latest.keys())
        .collect();
    let mut recorded_conflicts = 0;
    for key in &touched {
        let history = central::history(&primary, &address(key)).expect("history");
        for (index, version) in history.iter().enumerate() {
            assert_eq!(version.version, index as u64 + 1, "{key}");
            assert_eq!(version.parent_version, index as u64, "{key}");
        }
        let head = central::head(&primary, &address(key))
            .expect("head")
            .expect("present");
        assert_eq!(head.version, history.len() as u64, "{key}");
        recorded_conflicts += central::conflicts(&primary, &address(key))
            .expect("conflicts")
            .len();
    }
    // Every losing body was kept, once.
    assert_eq!(recorded_conflicts, conflicts);
    // The namespace still holds exactly the seeded keys.
    let listing = central::listing(&primary, SCOPE, REGISTRY_ID, NAMESPACE).expect("listing");
    assert_eq!(listing.len(), KEYS as usize);
}

#[test]
fn central_five_agents_polling_sees_at_most_one_second_of_staleness() {
    let primary = SqlitePrimary::in_memory().expect("primary");
    define_namespace(&primary, &Namespace::shared(NAMESPACE, SCOPE)).expect("define");
    seed(&primary);
    let definition = central::resolve(&primary, REGISTRY_ID, NAMESPACE).expect("resolve");
    let writer = Agent {
        store: LocalStore::in_memory().expect("store"),
        owner: "agent-0".into(),
        partition: "session-0".into(),
        latest: BTreeMap::new(),
    };
    pull_namespaces(&writer.store, &primary).expect("pull");
    let observer_store = LocalStore::in_memory().expect("observer");
    pull_namespaces(&observer_store, &primary).expect("pull");
    let observer = Identity::new("observer", "fleet-ctx", "watch");
    let read_at = |now_ms: i64| {
        let mut out = Vec::new();
        let found = Reader {
            store: &observer_store,
            identity: &observer,
            central: Some(&primary),
            blobs: None,
            allow_stale: false,
            now_ms,
        }
        .get(NAMESPACE, "k00007", &mut out)
        .expect("get");
        let Found::Value(Source::Central { version }) = found else {
            panic!("expected a central read, got {found:?}");
        };
        (version, out)
    };
    let list_version = |now_ms: i64| {
        Reader {
            store: &observer_store,
            identity: &observer,
            central: Some(&primary),
            blobs: None,
            allow_stale: false,
            now_ms,
        }
        .list(NAMESPACE, false, false)
        .expect("list")
        .entries
        .into_iter()
        .find(|entry| entry.key == "k00007")
        .expect("listed")
        .version
    };
    // The writer builds on central's seeded head, so its writes commit.
    let based = rebase(
        &writer.store,
        &writer.identity(),
        &primary,
        &definition,
        NAMESPACE,
        "k00007",
    )
    .expect("rebase");
    assert_eq!((based.onto, based.requeued), (1, None));
    let start = 1_000_000;
    assert_eq!(read_at(start).0, 1);
    // Ten commits, polled every 250 ms: a cached read may lag a commit only
    // until the cache is a full second old, and `list` is never stale.
    let mut now = start;
    let mut committed_at: Vec<(i64, u64)> = vec![(0, 1)];
    for round in 0..10 {
        now += 250;
        writer
            .store
            .set(
                &writer.identity(),
                NAMESPACE,
                "k00007",
                format!("r{round}").as_bytes(),
            )
            .expect("set");
        assert_eq!(writer.sync(&primary, &definition), 0);
        let head = central::head(&primary, &address("k00007"))
            .expect("head")
            .expect("present")
            .version;
        committed_at.push((now, head));
        for _ in 0..4 {
            now += 250;
            let (seen, _) = read_at(now);
            // The newest version committed at least 1 s ago must be visible.
            let floor = committed_at
                .iter()
                .filter(|(at, _)| now - at >= CACHE_MAX_AGE_MS)
                .map(|(_, version)| *version)
                .max()
                .unwrap_or(1);
            assert!(
                seen >= floor,
                "at {now} saw v{seen}, but v{floor} is over 1 s old"
            );
            assert_eq!(list_version(now), head, "list is read fresh");
        }
    }
}
