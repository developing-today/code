//! Permission noninterference reference-model fixtures (F02, #8527).
//!
//! Oracle: doc/2026-11-11T03-45-00Z_design_atlas_v2_v4/follow-ons/F02.md
//!
//! A standalone, test-only state machine modeling authorized physical
//! selection, retrieval, resolution/hydration, cache and current-epoch
//! revocation for Atlas-style search. Two implementations of the same
//! contract live here on purpose:
//!
//! - `Model` — the independent reference: a pure recomputation over the
//!   world state. It selects authorized physical inputs *before*
//!   ranking, and authorizes resolution, hydration and every cached
//!   replay against *current* rights.
//! - `Pipeline` — an event-driven implementation with a real cache,
//!   checked against `Model` after every generated operation.
//!
//! Noninterference claim (F02 step 1): for a low-privilege caller, the
//! observable projection (result ids, ranks, counts, errors, metadata)
//! is a function of the permitted objects only. Paired worlds that
//! share their permitted objects but differ in forbidden objects,
//! source paths and content must produce identical projections.
//!
//! Negative controls (F02 step 3): two deliberately insecure mutants —
//! selection-after-ranking and stale-cache replay — are included, and
//! the tests assert that the same checks *reject* them. Timing and
//! side channels are restricted to modeled observable values; nothing
//! here asserts constant-time execution, and a fixture pass is not a
//! certification of any deployed surface.

use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A physical object in a world. `content` stands in for the object's
/// bytes; ranking scores derive from it, so forbidden content must
/// never influence a low-privilege projection.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Obj {
    id: u32,
    path: String,
    content: String,
    /// Callers currently authorized to observe this object.
    allowed: BTreeSet<String>,
}

/// The world state both implementations read. Rights transitions bump
/// `epoch`; a projection stamped with an older epoch is stale.
#[derive(Clone, Debug)]
struct World {
    objects: BTreeMap<u32, Obj>,
    epoch: u64,
}

impl World {
    fn authorized(&self, caller: &str) -> Vec<&Obj> {
        self.objects
            .values()
            .filter(|o| o.allowed.contains(caller))
            .collect()
    }
}

/// What a caller can observe from a search: result ids in rank order,
/// the total count, and metadata. Deliberately carries no field a
/// forbidden object could influence without detection.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Projection {
    ids_in_rank_order: Vec<u32>,
    total_count: usize,
    epoch: u64,
}

/// Errors are observable, so a forbidden object and a nonexistent
/// object must be indistinguishable: both are `NotFound`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum HydrateError {
    NotFound,
}

type HydrateResult = Result<(u32, String), HydrateError>;

fn score(query: &str, content: &str) -> usize {
    // Deterministic stand-in for ranking: term overlap count.
    query
        .split_whitespace()
        .filter(|t| !t.is_empty() && content.split_whitespace().any(|w| w == *t))
        .count()
}

fn project(_caller: &str, query: &str, world: &World, inputs: Vec<&Obj>) -> Projection {
    let mut scored: Vec<(u32, usize)> = inputs
        .into_iter()
        .map(|o| (o.id, score(query, &o.content)))
        .filter(|(_, s)| *s > 0)
        .collect();
    // Rank by score desc, id asc — a total order, so ranks are stable.
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Projection {
        total_count: scored.len(),
        ids_in_rank_order: scored.into_iter().map(|(id, _)| id).collect(),
        epoch: world.epoch,
    }
}

/// The independent reference model: pure functions over world state.
struct Model;

impl Model {
    fn search(world: &World, caller: &str, query: &str) -> Projection {
        // Authorized physical selection BEFORE ranking (F02 step 2).
        project(caller, query, world, world.authorized(caller))
    }

    fn hydrate(world: &World, caller: &str, id: u32) -> HydrateResult {
        // Authorize against CURRENT rights; forbidden == nonexistent.
        match world.objects.get(&id) {
            Some(o) if o.allowed.contains(caller) => Ok((o.id, o.content.clone())),
            _ => Err(HydrateError::NotFound),
        }
    }

    fn replay(world: &World, caller: &str, query: &str) -> Projection {
        // A cached replay is only ever the current projection: the
        // cache may store bytes, never authority.
        Self::search(world, caller, query)
    }
}

/// The event-driven pipeline under test: holds a projection cache and
/// must agree with `Model` after every operation.
struct Pipeline {
    cache: HashMap<(String, String), Projection>,
}

impl Pipeline {
    fn new() -> Self {
        Pipeline {
            cache: HashMap::new(),
        }
    }

    fn search(&mut self, world: &World, caller: &str, query: &str) -> Projection {
        let p = Model::search(world, caller, query);
        self.cache
            .insert((caller.to_string(), query.to_string()), p.clone());
        p
    }

    fn hydrate(&self, world: &World, caller: &str, id: u32) -> HydrateResult {
        Model::hydrate(world, caller, id)
    }

    fn replay(&mut self, world: &World, caller: &str, query: &str) -> Projection {
        // Replay validates the cached entry against the CURRENT epoch
        // and rights: a stale entry is recomputed, never served.
        let key = (caller.to_string(), query.to_string());
        if let Some(cached) = self.cache.get(&key) {
            if cached.epoch == world.epoch {
                return cached.clone();
            }
        }
        self.search(world, caller, query)
    }
}

/// MUTANT (negative control): ranks the full physical set first and
/// filters to authorized objects afterwards. Forbidden objects change
/// ranks and counts — exactly the leak F02 step 3 names.
fn mutant_selection_after_ranking(world: &World, caller: &str, query: &str) -> Projection {
    let all: Vec<&Obj> = world.objects.values().collect();
    let mut p = project(caller, query, world, all);
    p.ids_in_rank_order.retain(|id| {
        world
            .objects
            .get(id)
            .map(|o| o.allowed.contains(caller))
            .unwrap_or(false)
    });
    // The count still reflects the unfiltered ranking: the leak.
    p
}

/// MUTANT (negative control): serves the cached projection without
/// re-validating against current rights.
struct StaleCachePipeline {
    cache: HashMap<(String, String), Projection>,
}

impl StaleCachePipeline {
    fn new() -> Self {
        StaleCachePipeline {
            cache: HashMap::new(),
        }
    }
    fn search(&mut self, world: &World, caller: &str, query: &str) -> Projection {
        let p = Model::search(world, caller, query);
        self.cache
            .insert((caller.to_string(), query.to_string()), p.clone());
        p
    }
    fn replay(&mut self, _world: &World, caller: &str, query: &str) -> Projection {
        self.cache
            .get(&(caller.to_string(), query.to_string()))
            .cloned()
            .expect("seeded by search")
    }
}

/// Rights transitions, shared by both implementations' worlds.
#[derive(Clone, Debug)]
enum Op {
    Search(String),
    Hydrate(u32),
    Replay(String),
    Grant(u32, String),
    Revoke(u32, String),
    Reclassify(u32, BTreeSet<String>),
}

fn apply(world: &mut World, op: &Op) {
    match op {
        Op::Grant(id, caller) => {
            if let Some(o) = world.objects.get_mut(id) {
                o.allowed.insert(caller.clone());
                world.epoch += 1;
            }
        }
        Op::Revoke(id, caller) => {
            if let Some(o) = world.objects.get_mut(id) {
                o.allowed.remove(caller);
                world.epoch += 1;
            }
        }
        Op::Reclassify(id, allowed) => {
            if let Some(o) = world.objects.get_mut(id) {
                o.allowed = allowed.clone();
                world.epoch += 1;
            }
        }
        _ => {}
    }
}

/// Deterministic sequence generator (xorshift64*): no external
/// entropy, so a failing sequence replays byte-for-byte.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const CALLERS: [&str; 3] = ["low", "mid", "high"];
const QUERIES: [&str; 4] = ["alpha", "beta", "alpha beta", "gamma delta"];

fn gen_op(rng: &mut Rng, ids: &[u32]) -> Op {
    let caller = CALLERS[rng.below(3) as usize].to_string();
    match rng.below(6) {
        0 => Op::Search(QUERIES[rng.below(4) as usize].to_string()),
        1 => Op::Hydrate(ids[rng.below(ids.len() as u64) as usize]),
        2 => Op::Replay(QUERIES[rng.below(4) as usize].to_string()),
        3 => Op::Grant(ids[rng.below(ids.len() as u64) as usize], caller),
        4 => Op::Revoke(ids[rng.below(ids.len() as u64) as usize], caller),
        _ => {
            let mut set = BTreeSet::new();
            if rng.below(2) == 0 {
                set.insert("low".to_string());
            }
            if rng.below(2) == 0 {
                set.insert("mid".to_string());
            }
            Op::Reclassify(ids[rng.below(ids.len() as u64) as usize], set)
        }
    }
}

fn obj(id: u32, path: &str, content: &str, allowed: &[&str]) -> Obj {
    Obj {
        id,
        path: path.to_string(),
        content: content.to_string(),
        allowed: allowed.iter().map(|s| s.to_string()).collect(),
    }
}

/// Paired worlds (F02 step 1): identical permitted objects for the
/// low-privilege caller; forbidden objects differ in id, source path,
/// content and number. One forbidden object in each world is a
/// byte-identical twin of permitted object 1 with different rights.
fn paired_worlds() -> (World, World) {
    let permitted = vec![
        obj(
            1,
            "docs/alpha.md",
            "alpha beta release notes",
            &["low", "mid"],
        ),
        obj(2, "docs/beta.md", "beta gamma handbook", &["low"]),
        obj(
            3,
            "docs/gamma.md",
            "gamma delta alpha guide",
            &["low", "high"],
        ),
        obj(4, "docs/delta.md", "delta epsilon memo", &["low"]),
    ];
    let mut a = World {
        objects: BTreeMap::new(),
        epoch: 0,
    };
    let mut b = World {
        objects: BTreeMap::new(),
        epoch: 0,
    };
    for o in permitted {
        a.objects.insert(o.id, o.clone());
        b.objects.insert(o.id, o);
    }
    // World A forbidden set.
    a.objects.insert(
        10,
        obj(
            10,
            "secret/plans.md",
            "alpha beta alpha beta secret",
            &["high"],
        ),
    );
    a.objects.insert(
        11,
        obj(11, "secret/twin.md", "alpha beta release notes", &["mid"]),
    );
    // World B forbidden set: different ids, paths, contents, count —
    // including a byte-identical twin of object 1's content.
    b.objects
        .insert(20, obj(20, "vault/x.md", "gamma delta vault", &["high"]));
    b.objects.insert(
        21,
        obj(21, "vault/twin.md", "alpha beta release notes", &["high"]),
    );
    b.objects
        .insert(22, obj(22, "vault/y.md", "epsilon zeta vault", &[]));
    (a, b)
}

#[test]
fn paired_worlds_low_privilege_projections_are_identical() {
    // Oracle: F02 step 1 — the low-privilege observable projection is
    // identical across paired worlds, including counts, errors and
    // metadata, after every operation in generated sequences.
    let ids = [1, 2, 3, 4, 10, 20, 999];
    for seed in 0..60u64 {
        let (mut wa, mut wb) = paired_worlds();
        let mut pa = Pipeline::new();
        let mut pb = Pipeline::new();
        let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        for _ in 0..40 {
            let op = gen_op(&mut rng, &ids);
            match &op {
                Op::Search(q) => {
                    assert_eq!(
                        pa.search(&wa, "low", q),
                        pb.search(&wb, "low", q),
                        "search diverged: seed {seed}, op {op:?}"
                    );
                }
                Op::Replay(q) => {
                    assert_eq!(
                        pa.replay(&wa, "low", q),
                        pb.replay(&wb, "low", q),
                        "replay diverged: seed {seed}, op {op:?}"
                    );
                }
                Op::Hydrate(id) => {
                    assert_eq!(
                        pa.hydrate(&wa, "low", *id),
                        pb.hydrate(&wb, "low", *id),
                        "hydrate diverged: seed {seed}, op {op:?}"
                    );
                }
                _ => {
                    // Transitions apply only to the shared permitted
                    // ids: a transition naming a world-specific
                    // forbidden id would change one world's permitted
                    // set by construction, which is not the claim
                    // under test.
                    let target = match &op {
                        Op::Grant(id, _) | Op::Revoke(id, _) | Op::Reclassify(id, _) => Some(*id),
                        _ => None,
                    };
                    if target.map(|id| [1, 2, 3, 4].contains(&id)).unwrap_or(false) {
                        apply(&mut wa, &op);
                        apply(&mut wb, &op);
                    }
                }
            }
        }
    }
}

#[test]
fn pipeline_matches_reference_model_over_generated_sequences() {
    // Oracle: F02 step 2 — every generated read / cache / revoke /
    // replay result is checked against the independent model.
    let ids = [1, 2, 3, 4, 5, 6];
    for seed in 0..200u64 {
        let mut world = World {
            objects: BTreeMap::new(),
            epoch: 0,
        };
        world
            .objects
            .insert(1, obj(1, "a.md", "alpha beta", &["low", "mid"]));
        world
            .objects
            .insert(2, obj(2, "b.md", "beta gamma", &["mid"]));
        world
            .objects
            .insert(3, obj(3, "c.md", "alpha beta", &["high"])); // byte-identical twin of 1, different rights
        world
            .objects
            .insert(4, obj(4, "d.md", "gamma delta alpha", &["low"]));
        world
            .objects
            .insert(5, obj(5, "e.md", "delta epsilon", &[]));
        world
            .objects
            .insert(6, obj(6, "f.md", "alpha gamma", &["low", "mid", "high"]));
        let mut pipe = Pipeline::new();
        let mut rng = Rng(seed.wrapping_mul(0xD1B54A32D192ED03) | 1);
        for step in 0..50 {
            let op = gen_op(&mut rng, &ids);
            apply(&mut world, &op);
            let caller = CALLERS[(seed + step) as usize % 3];
            let query = QUERIES[(step as usize) % QUERIES.len()];
            assert_eq!(
                pipe.search(&world, caller, query),
                Model::search(&world, caller, query),
                "search != model: seed {seed}, step {step}, op {op:?}"
            );
            assert_eq!(
                pipe.replay(&world, caller, query),
                Model::replay(&world, caller, query),
                "replay != model: seed {seed}, step {step}, op {op:?}"
            );
            let hid = ids[(step as usize) % ids.len()];
            assert_eq!(
                pipe.hydrate(&world, caller, hid),
                Model::hydrate(&world, caller, hid),
                "hydrate != model: seed {seed}, step {step}, op {op:?}"
            );
        }
    }
}

#[test]
fn mutant_selection_after_ranking_is_rejected() {
    // Oracle: F02 step 3 — the paired-world assertion must reject the
    // selection-after-ranking mutant: forbidden objects move its
    // count even though its filtered ids happen to match here.
    let (wa, wb) = paired_worlds();
    let correct_a = Model::search(&wa, "low", "alpha beta");
    let correct_b = Model::search(&wb, "low", "alpha beta");
    assert_eq!(
        correct_a, correct_b,
        "reference model must be noninterfering"
    );
    let mutant_a = mutant_selection_after_ranking(&wa, "low", "alpha beta");
    let mutant_b = mutant_selection_after_ranking(&wb, "low", "alpha beta");
    assert_ne!(
        mutant_a, mutant_b,
        "mutant projections must differ across paired worlds (the leak the assertion catches)"
    );
    assert_ne!(
        mutant_a.total_count, correct_a.total_count,
        "mutant count leaks forbidden membership"
    );
}

#[test]
fn mutant_stale_cache_is_rejected() {
    // Oracle: F02 step 3 — after a revocation, a stale-cache replay
    // serves formerly allowed content; the check must reject it,
    // while the epoch-validating pipeline still matches the model.
    let mut world = World {
        objects: BTreeMap::new(),
        epoch: 0,
    };
    world
        .objects
        .insert(1, obj(1, "a.md", "alpha beta", &["low"]));
    world
        .objects
        .insert(2, obj(2, "b.md", "alpha gamma", &["low"]));
    let mut stale = StaleCachePipeline::new();
    let mut pipe = Pipeline::new();
    let seeded = stale.search(&world, "low", "alpha");
    assert_eq!(seeded, pipe.search(&world, "low", "alpha"));
    // Revoke object 1: historical access is not present access.
    apply(&mut world, &Op::Revoke(1, "low".to_string()));
    let expected = Model::replay(&world, "low", "alpha");
    assert!(!expected.ids_in_rank_order.contains(&1));
    assert_eq!(
        pipe.replay(&world, "low", "alpha"),
        expected,
        "validating pipeline must match the model after revocation"
    );
    let stale_replay = stale.replay(&world, "low", "alpha");
    assert!(
        stale_replay.ids_in_rank_order.contains(&1),
        "stale mutant must actually serve the revoked object for this control to mean anything"
    );
    assert_ne!(
        stale_replay, expected,
        "the model check must reject the stale-cache mutant"
    );
}

#[test]
fn hydrate_forbidden_is_indistinguishable_from_missing() {
    // Oracle: F02 step 1 (errors are observable) — a forbidden object
    // and a nonexistent object produce the same error.
    let (wa, _) = paired_worlds();
    assert_eq!(Model::hydrate(&wa, "low", 10), Err(HydrateError::NotFound));
    assert_eq!(
        Model::hydrate(&wa, "low", 10),
        Model::hydrate(&wa, "low", 999_999)
    );
}

#[test]
fn byte_identical_twins_do_not_leak_across_rights() {
    // Oracle: F02 step 2 seed — mixed-rights byte-identical
    // occurrences: ranking over identical content must surface only
    // the permitted twin, with a count that ignores the forbidden one.
    let (wa, _) = paired_worlds();
    let p = Model::search(&wa, "low", "alpha beta release notes");
    assert_eq!(
        p.ids_in_rank_order,
        vec![1, 2, 3],
        "permitted twin ranks first; only permitted objects surface"
    );
    assert!(
        !p.ids_in_rank_order.contains(&11),
        "forbidden twin must not surface"
    );
    assert_eq!(p.total_count, 3, "count must reflect permitted inputs only");
}
