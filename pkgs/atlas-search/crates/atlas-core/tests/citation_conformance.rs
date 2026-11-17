// lane-scratch: any-c /tmp/lane-any-c-20261117-1112 task 20162
// F03 citation and occurrence conformance fixtures (issue #8526, design #6020).
//
// Test-only, offline conformance corpus plus an independent reference
// resolver for Atlas citation/occurrence resolution. This file is an
// adapter-conformance oracle: it implements no production registry, CAS,
// storage, MCP tool, or live resolution (those belong to #3125), and it
// deliberately does not import the F02 test helper or any production type,
// so this slice ships independently.
//
// Oracle (hand-authored expectations, failing-first):
//   * a successful citation resolves to the exact retained bytes of the
//     named occurrence, at the named (or current) version, at its own
//     source coordinates -- byte-identical content never merges the
//     authority of distinct occurrences;
//   * completeness is only within the enumerated, ingested, retained,
//     permitted snapshot: missing coverage and deleted bytes answer a
//     scoped Unavailable to an authorized caller;
//   * a caller without current rights gets a response indistinguishable
//     from an unknown occurrence id -- no path, no count, no metadata
//     about forbidden occurrences is ever revealed;
//   * current revocation makes previously valid citations unavailable,
//     and a hidden insertion never changes visible counts or metadata.
//
// Negative controls: the deliberately incorrect aliasing resolver (keys
// by content, merging occurrences) and the stale-replay resolver (built
// from a superseded snapshot) must both be REJECTED by the corpus.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use proptest::prelude::*;

// ---------------------------------------------------------------- model

/// FNV-1a 64-bit hash. Fixtures are synthetic; the hash only has to be a
/// deterministic function of the retained bytes so a citation can be
/// checked against them.
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// One explicit fixture record: a single version of a single occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FixtureRecord {
    content_id: String,
    occurrence_id: String,
    version: u64,
    /// Stable source identity; survives a rename.
    source_id: String,
    /// Current source path; changes on rename.
    source_path: String,
    byte_start: usize,
    byte_end: usize,
    /// Retained bytes for this version; None means the bytes are deleted.
    retained: Option<Vec<u8>>,
    /// True when this occurrence is inside the snapshot coverage.
    coverage: bool,
    /// Caller groups currently holding rights on this occurrence.
    rights: BTreeSet<String>,
    /// Hidden records exist in storage but are invisible: they must never
    /// change visible counts, metadata, or resolution results.
    hidden: bool,
}

impl FixtureRecord {
    fn hash(&self) -> Option<u64> {
        self.retained.as_ref().map(|b| hash_bytes(b))
    }
    fn authorized(&self, granted: &BTreeSet<String>) -> bool {
        !self.rights.is_disjoint(granted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    occurrence_id: String,
    /// None asks for the current version; Some pins a historical version.
    version: Option<u64>,
    /// The caller's current authorization (groups held right now).
    granted: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnavailableReason {
    DeletedBytes,
    MissingCoverage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Response {
    Citation {
        occurrence_id: String,
        content_id: String,
        version: u64,
        source_id: String,
        source_path: String,
        byte_start: usize,
        byte_end: usize,
        bytes: Vec<u8>,
        hash: u64,
    },
    /// The occurrence is visible to this caller but cannot be cited.
    Unavailable { reason: UnavailableReason },
    /// Unknown id -- and, indistinguishably, an id this caller may not
    /// see. Carries no path, count, or metadata.
    NotFound,
}

/// The test-local adapter interface every candidate implementation is
/// judged through.
trait CitationAdapter {
    fn resolve(&self, req: &Request) -> Response;
    /// Number of occurrences visible to a caller with these grants.
    fn visible_count(&self, granted: &BTreeSet<String>) -> usize;
    /// Sorted ids of the occurrences visible to this caller.
    fn visible_ids(&self, granted: &BTreeSet<String>) -> Vec<String>;
}

// ------------------------------------------------------- reference impl

/// The independent reference projection: a direct, deliberately simple
/// scan over the explicit fixture records. Correctness by inspection,
/// not by sharing code with any candidate adapter.
#[derive(Clone, Debug)]
struct ReferenceResolver {
    records: Vec<FixtureRecord>,
}

impl ReferenceResolver {
    fn new(records: Vec<FixtureRecord>) -> Self {
        Self { records }
    }

    fn visible_records<'a>(&'a self, granted: &BTreeSet<String>) -> Vec<&'a FixtureRecord> {
        // One entry per occurrence: the current (max) version decides
        // visibility, so version history never inflates counts.
        let mut current: BTreeMap<&str, &FixtureRecord> = BTreeMap::new();
        for r in &self.records {
            if r.hidden {
                continue;
            }
            let e = current.entry(r.occurrence_id.as_str()).or_insert(r);
            if r.version > e.version {
                *e = r;
            }
        }
        current
            .into_values()
            .filter(|r| r.authorized(granted))
            .collect()
    }
}

impl CitationAdapter for ReferenceResolver {
    fn resolve(&self, req: &Request) -> Response {
        let versions: Vec<&FixtureRecord> = self
            .records
            .iter()
            .filter(|r| r.occurrence_id == req.occurrence_id && !r.hidden)
            .collect();
        if versions.is_empty() {
            return Response::NotFound;
        }
        let record = match req.version {
            Some(v) => versions.iter().find(|r| r.version == v).copied(),
            None => versions.iter().max_by_key(|r| r.version).copied(),
        };
        let Some(record) = record else {
            return Response::NotFound;
        };
        // Current authorization decides. An unauthorized caller learns
        // nothing: the answer is the same as for an unknown id.
        if !record.authorized(&req.granted) {
            return Response::NotFound;
        }
        if !record.coverage {
            return Response::Unavailable {
                reason: UnavailableReason::MissingCoverage,
            };
        }
        let Some(retained) = &record.retained else {
            return Response::Unavailable {
                reason: UnavailableReason::DeletedBytes,
            };
        };
        let bytes = retained[record.byte_start..record.byte_end].to_vec();
        Response::Citation {
            occurrence_id: record.occurrence_id.clone(),
            content_id: record.content_id.clone(),
            version: record.version,
            source_id: record.source_id.clone(),
            source_path: record.source_path.clone(),
            byte_start: record.byte_start,
            byte_end: record.byte_end,
            bytes,
            hash: record.hash().expect("retained bytes hash"),
        }
    }

    fn visible_count(&self, granted: &BTreeSet<String>) -> usize {
        self.visible_records(granted).len()
    }

    fn visible_ids(&self, granted: &BTreeSet<String>) -> Vec<String> {
        let mut ids: Vec<String> = self
            .visible_records(granted)
            .iter()
            .map(|r| r.occurrence_id.clone())
            .collect();
        ids.sort();
        ids
    }
}

// ------------------------------------------------- correct fake adapter

/// A small deliberately correct fake: indexes records by
/// (occurrence id, version) up front instead of scanning. Structurally
/// different from the reference projection, held to the same corpus.
struct FakeAdapter {
    by_key: HashMap<(String, u64), FixtureRecord>,
    current_version: HashMap<String, u64>,
}

impl FakeAdapter {
    fn new(records: Vec<FixtureRecord>) -> Self {
        let mut by_key = HashMap::new();
        let mut current_version: HashMap<String, u64> = HashMap::new();
        for r in records {
            if r.hidden {
                continue;
            }
            let cur = current_version.entry(r.occurrence_id.clone()).or_insert(0);
            *cur = (*cur).max(r.version);
            by_key.insert((r.occurrence_id.clone(), r.version), r);
        }
        Self {
            by_key,
            current_version,
        }
    }
}

impl CitationAdapter for FakeAdapter {
    fn resolve(&self, req: &Request) -> Response {
        let version = match req.version {
            Some(v) => v,
            None => match self.current_version.get(&req.occurrence_id) {
                Some(v) => *v,
                None => return Response::NotFound,
            },
        };
        let Some(record) = self.by_key.get(&(req.occurrence_id.clone(), version)) else {
            return Response::NotFound;
        };
        if !record.authorized(&req.granted) {
            return Response::NotFound;
        }
        if !record.coverage {
            return Response::Unavailable {
                reason: UnavailableReason::MissingCoverage,
            };
        }
        let Some(retained) = &record.retained else {
            return Response::Unavailable {
                reason: UnavailableReason::DeletedBytes,
            };
        };
        Response::Citation {
            occurrence_id: record.occurrence_id.clone(),
            content_id: record.content_id.clone(),
            version: record.version,
            source_id: record.source_id.clone(),
            source_path: record.source_path.clone(),
            byte_start: record.byte_start,
            byte_end: record.byte_end,
            bytes: retained[record.byte_start..record.byte_end].to_vec(),
            hash: record.hash().expect("retained bytes hash"),
        }
    }

    fn visible_count(&self, granted: &BTreeSet<String>) -> usize {
        self.visible_ids(granted).len()
    }

    fn visible_ids(&self, granted: &BTreeSet<String>) -> Vec<String> {
        let mut ids: Vec<String> = self
            .current_version
            .iter()
            .filter(|(id, v)| {
                self.by_key
                    .get(&((*id).clone(), **v))
                    .map(|r| r.authorized(granted))
                    .unwrap_or(false)
            })
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }
}

// ------------------------------------------- deliberately incorrect impls

/// INCORRECT on purpose: indexes by content id, so byte-identical
/// occurrences alias onto whichever record was inserted first. The
/// corpus must reject this implementation.
struct AliasingResolver {
    by_content: HashMap<String, FixtureRecord>,
    inner: ReferenceResolver,
}

impl AliasingResolver {
    fn new(records: Vec<FixtureRecord>) -> Self {
        let mut by_content = HashMap::new();
        for r in &records {
            by_content
                .entry(r.content_id.clone())
                .or_insert_with(|| r.clone());
        }
        Self {
            by_content,
            inner: ReferenceResolver::new(records),
        }
    }
}

impl CitationAdapter for AliasingResolver {
    fn resolve(&self, req: &Request) -> Response {
        // The defect: resolve the requested occurrence through the
        // content index, returning the first record with that content.
        let target = self
            .inner
            .records
            .iter()
            .find(|r| r.occurrence_id == req.occurrence_id);
        if let Some(t) = target {
            if let Some(first) = self.by_content.get(&t.content_id) {
                let redirected = Request {
                    occurrence_id: first.occurrence_id.clone(),
                    version: req.version,
                    granted: req.granted.clone(),
                };
                return self.inner.resolve(&redirected);
            }
        }
        self.inner.resolve(req)
    }

    fn visible_count(&self, granted: &BTreeSet<String>) -> usize {
        self.inner.visible_count(granted)
    }

    fn visible_ids(&self, granted: &BTreeSet<String>) -> Vec<String> {
        self.inner.visible_ids(granted)
    }
}

/// INCORRECT on purpose: answers from a superseded snapshot, so revoked
/// rights, deleted bytes, and renames are replayed as if nothing had
/// changed. The corpus must reject this implementation.
struct StaleReplayResolver {
    snapshot: ReferenceResolver,
}

impl CitationAdapter for StaleReplayResolver {
    fn resolve(&self, req: &Request) -> Response {
        self.snapshot.resolve(req)
    }

    fn visible_count(&self, granted: &BTreeSet<String>) -> usize {
        self.snapshot.visible_count(granted)
    }

    fn visible_ids(&self, granted: &BTreeSet<String>) -> Vec<String> {
        self.snapshot.visible_ids(granted)
    }
}

// ------------------------------------------------------------- fixtures

fn grants(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn record(
    content_id: &str,
    occurrence_id: &str,
    version: u64,
    source_id: &str,
    source_path: &str,
    retained: Option<&[u8]>,
    rights: &[&str],
) -> FixtureRecord {
    let retained = retained.map(|b| b.to_vec());
    let len = retained.as_ref().map(|b| b.len()).unwrap_or(0);
    FixtureRecord {
        content_id: content_id.to_string(),
        occurrence_id: occurrence_id.to_string(),
        version,
        source_id: source_id.to_string(),
        source_path: source_path.to_string(),
        byte_start: 0,
        byte_end: len,
        retained,
        coverage: true,
        rights: grants(rights),
        hidden: false,
    }
}

fn citation(
    r: &FixtureRecord,
    bytes: &[u8],
) -> Response {
    Response::Citation {
        occurrence_id: r.occurrence_id.clone(),
        content_id: r.content_id.clone(),
        version: r.version,
        source_id: r.source_id.clone(),
        source_path: r.source_path.clone(),
        byte_start: r.byte_start,
        byte_end: r.byte_end,
        bytes: bytes.to_vec(),
        hash: hash_bytes(bytes),
    }
}

struct Case {
    name: &'static str,
    world: Vec<FixtureRecord>,
    request: Request,
    expected: Response,
}

/// The hand-authored corpus: expected results were written from the
/// issue's failing-first list before the reference resolver existed.
fn hand_authored_cases() -> Vec<Case> {
    let shared: &[u8] = b"identical bytes, two homes";
    let mut cases = Vec::new();

    // Same bytes at multiple locations/rights: each occurrence keeps its
    // own coordinates, and a caller sees only the occurrence it holds
    // rights on.
    let occ_a = record("content-shared", "occ-a", 1, "src-1", "docs/one.txt", Some(shared), &["team-x"]);
    let occ_b = record("content-shared", "occ-b", 1, "src-2", "docs/two.txt", Some(shared), &["team-y"]);
    cases.push(Case {
        name: "same_bytes_first_location",
        world: vec![occ_a.clone(), occ_b.clone()],
        request: Request {
            occurrence_id: "occ-a".into(),
            version: None,
            granted: grants(&["team-x"]),
        },
        expected: citation(&occ_a, shared),
    });
    cases.push(Case {
        name: "same_bytes_second_location",
        world: vec![occ_a.clone(), occ_b.clone()],
        request: Request {
            occurrence_id: "occ-b".into(),
            version: None,
            granted: grants(&["team-y"]),
        },
        expected: citation(&occ_b, shared),
    });
    // A caller with rights on occ-a only must learn nothing about occ-b.
    cases.push(Case {
        name: "same_bytes_forbidden_location_is_not_found",
        world: vec![occ_a.clone(), occ_b.clone()],
        request: Request {
            occurrence_id: "occ-b".into(),
            version: None,
            granted: grants(&["team-x"]),
        },
        expected: Response::NotFound,
    });

    // Pinned historical version: v1 stays citable after v2 lands, and an
    // unpinned request follows the current version.
    let v1 = record("content-doc", "occ-doc", 1, "src-9", "docs/spec.txt", Some(b"version one"), &["team-x"]);
    let v2 = record("content-doc", "occ-doc", 2, "src-9", "docs/spec.txt", Some(b"version two, revised"), &["team-x"]);
    cases.push(Case {
        name: "pinned_historical_version",
        world: vec![v1.clone(), v2.clone()],
        request: Request {
            occurrence_id: "occ-doc".into(),
            version: Some(1),
            granted: grants(&["team-x"]),
        },
        expected: citation(&v1, b"version one"),
    });
    cases.push(Case {
        name: "unpinned_follows_current_version",
        world: vec![v1.clone(), v2.clone()],
        request: Request {
            occurrence_id: "occ-doc".into(),
            version: None,
            granted: grants(&["team-x"]),
        },
        expected: citation(&v2, b"version two, revised"),
    });

    // Source rename: identity and bytes are stable, the citation carries
    // the current path.
    let pre = record("content-rn", "occ-rn", 1, "src-7", "docs/old-name.txt", Some(b"renamed source"), &["team-x"]);
    let post = record("content-rn", "occ-rn", 2, "src-7", "docs/new-name.txt", Some(b"renamed source"), &["team-x"]);
    cases.push(Case {
        name: "source_rename_reports_current_path",
        world: vec![pre, post.clone()],
        request: Request {
            occurrence_id: "occ-rn".into(),
            version: None,
            granted: grants(&["team-x"]),
        },
        expected: citation(&post, b"renamed source"),
    });

    // Missing coverage: visible to an authorized caller as Unavailable,
    // never as a citation.
    let mut nocov = record("content-nc", "occ-nc", 1, "src-3", "docs/uncovered.txt", Some(b"not covered"), &["team-x"]);
    nocov.coverage = false;
    cases.push(Case {
        name: "missing_coverage_is_unavailable",
        world: vec![nocov],
        request: Request {
            occurrence_id: "occ-nc".into(),
            version: None,
            granted: grants(&["team-x"]),
        },
        expected: Response::Unavailable {
            reason: UnavailableReason::MissingCoverage,
        },
    });

    // Deleted bytes: the record remains, the bytes do not.
    let gone = record("content-del", "occ-del", 1, "src-4", "docs/gone.txt", None, &["team-x"]);
    cases.push(Case {
        name: "deleted_bytes_are_unavailable",
        world: vec![gone],
        request: Request {
            occurrence_id: "occ-del".into(),
            version: None,
            granted: grants(&["team-x"]),
        },
        expected: Response::Unavailable {
            reason: UnavailableReason::DeletedBytes,
        },
    });

    // Changed permissions: rights moved from team-x to team-y. The old
    // holder now gets exactly the unknown-id answer.
    let moved = record("content-mv", "occ-mv", 1, "src-5", "docs/moved.txt", Some(b"rights changed"), &["team-y"]);
    cases.push(Case {
        name: "changed_permissions_old_holder_gets_not_found",
        world: vec![moved.clone()],
        request: Request {
            occurrence_id: "occ-mv".into(),
            version: None,
            granted: grants(&["team-x"]),
        },
        expected: Response::NotFound,
    });
    cases.push(Case {
        name: "changed_permissions_new_holder_cites",
        world: vec![moved.clone()],
        request: Request {
            occurrence_id: "occ-mv".into(),
            version: None,
            granted: grants(&["team-y"]),
        },
        expected: citation(&moved, b"rights changed"),
    });

    // Unknown id: the baseline every forbidden answer must match.
    cases.push(Case {
        name: "unknown_occurrence_is_not_found",
        world: vec![occ_a.clone()],
        request: Request {
            occurrence_id: "occ-nope".into(),
            version: None,
            granted: grants(&["team-x"]),
        },
        expected: Response::NotFound,
    });

    cases
}

/// Run a candidate against the hand-authored corpus; return the names
/// of the cases it got wrong. An empty vec means conformance.
fn run_corpus<A: CitationAdapter>(make: impl Fn(Vec<FixtureRecord>) -> A) -> Vec<&'static str> {
    let mut failures = Vec::new();
    for case in hand_authored_cases() {
        let adapter = make(case.world.clone());
        let got = adapter.resolve(&case.request);
        if got != case.expected {
            failures.push(case.name);
        }
    }
    failures
}

// ---------------------------------------------------------------- tests

#[test]
fn reference_resolver_passes_hand_authored_corpus() {
    let failures = run_corpus(ReferenceResolver::new);
    assert!(failures.is_empty(), "reference resolver failed cases: {failures:?}");
}

#[test]
fn fake_adapter_passes_hand_authored_corpus() {
    let failures = run_corpus(FakeAdapter::new);
    assert!(failures.is_empty(), "fake adapter failed cases: {failures:?}");
}

#[test]
fn aliasing_implementation_is_rejected() {
    let failures = run_corpus(|world| AliasingResolver::new(world));
    assert!(
        failures.contains(&"same_bytes_second_location"),
        "aliasing resolver must fail the second-location case, got: {failures:?}"
    );
}

#[test]
fn stale_replay_implementation_is_rejected() {
    // The stale resolver answers the CURRENT world from a snapshot taken
    // before permissions changed, bytes were deleted, and a source was
    // renamed, so build its cases on superseded worlds.
    let stale_worlds: Vec<(&str, Vec<FixtureRecord>, Request, Response)> = {
        let moved_then = record("content-mv", "occ-mv", 1, "src-5", "docs/moved.txt", Some(b"rights changed"), &["team-x"]);
        let moved_now = record("content-mv", "occ-mv", 1, "src-5", "docs/moved.txt", Some(b"rights changed"), &["team-y"]);
        let alive_then = record("content-del", "occ-del", 1, "src-4", "docs/gone.txt", Some(b"was here"), &["team-x"]);
        let gone_now = record("content-del", "occ-del", 1, "src-4", "docs/gone.txt", None, &["team-x"]);
        vec![
            (
                "changed_permissions_old_holder_gets_not_found",
                vec![moved_then],
                Request {
                    occurrence_id: "occ-mv".into(),
                    version: None,
                    granted: grants(&["team-x"]),
                },
                {
                    let _ = &moved_now;
                    Response::NotFound
                },
            ),
            (
                "deleted_bytes_are_unavailable",
                vec![alive_then],
                Request {
                    occurrence_id: "occ-del".into(),
                    version: None,
                    granted: grants(&["team-x"]),
                },
                {
                    let _ = &gone_now;
                    Response::Unavailable {
                        reason: UnavailableReason::DeletedBytes,
                    }
                },
            ),
        ]
    };
    let mut failures = Vec::new();
    for (name, snapshot, request, expected) in stale_worlds {
        let stale = StaleReplayResolver {
            snapshot: ReferenceResolver::new(snapshot),
        };
        if stale.resolve(&request) != expected {
            failures.push(name);
        }
    }
    assert_eq!(
        failures.len(),
        2,
        "stale-replay resolver must be rejected on both superseded cases, got: {failures:?}"
    );
}

#[test]
fn visible_counts_never_leak_forbidden_or_hidden_occurrences() {
    let shared: &[u8] = b"identical bytes, two homes";
    let occ_a = record("content-shared", "occ-a", 1, "src-1", "docs/one.txt", Some(shared), &["team-x"]);
    let occ_b = record("content-shared", "occ-b", 1, "src-2", "docs/two.txt", Some(shared), &["team-y"]);
    let mut hidden = record("content-hid", "occ-hid", 1, "src-6", "docs/hidden.txt", Some(b"hidden"), &["team-x"]);
    hidden.hidden = true;
    let world = vec![occ_a, occ_b, hidden];
    let resolver = ReferenceResolver::new(world);
    // team-x sees exactly occ-a: occ-b is forbidden, occ-hid is hidden.
    assert_eq!(resolver.visible_count(&grants(&["team-x"])), 1);
    assert_eq!(resolver.visible_ids(&grants(&["team-x"])), vec!["occ-a".to_string()]);
    assert_eq!(resolver.visible_count(&grants(&["team-y"])), 1);
    assert_eq!(resolver.visible_count(&grants(&["team-z"])), 0);
}

// ------------------------------------------------------- property tests

fn arb_record(id: usize, bytes: Vec<u8>, rights_seed: u8, flags: u8) -> FixtureRecord {
    let rights: &[&str] = match rights_seed % 3 {
        0 => &["team-x"],
        1 => &["team-y"],
        _ => &["team-x", "team-y"],
    };
    let mut r = record(
        "content-gen",
        &format!("occ-gen-{id}"),
        1,
        "src-gen",
        "docs/gen.txt",
        Some(&bytes),
        rights,
    );
    r.coverage = flags & 1 == 0;
    if flags & 2 != 0 {
        r.retained = None;
        r.byte_end = 0;
    }
    r
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: every successful citation resolves to the exact retained
    /// bytes of the named occurrence at its version and range, and only
    /// ever for a currently authorized, covered, retained record.
    #[test]
    fn successful_citations_match_retained_bytes(
        payloads in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..40), 1..6),
        rights_seed in any::<u8>(),
        flags in any::<u8>(),
        caller in 0usize..2,
    ) {
        let world: Vec<FixtureRecord> = payloads
            .iter()
            .enumerate()
            .map(|(i, b)| arb_record(i, b.clone(), rights_seed.wrapping_add(i as u8), flags.wrapping_add(i as u8)))
            .collect();
        let granted = grants(if caller == 0 { &["team-x"] } else { &["team-y"] });
        let resolver = ReferenceResolver::new(world.clone());
        for rec in &world {
            let got = resolver.resolve(&Request {
                occurrence_id: rec.occurrence_id.clone(),
                version: None,
                granted: granted.clone(),
            });
            if let Response::Citation { occurrence_id, bytes, hash, byte_start, byte_end, .. } = &got {
                prop_assert_eq!(occurrence_id, &rec.occurrence_id);
                let retained = rec.retained.as_ref().expect("citation implies retained bytes");
                prop_assert_eq!(bytes, &retained[*byte_start..*byte_end]);
                prop_assert_eq!(*hash, hash_bytes(bytes));
                prop_assert!(rec.authorized(&granted));
                prop_assert!(rec.coverage);
            }
        }
    }

    /// Byte-identical contents never merge occurrence authority: two
    /// occurrences over the same bytes keep distinct coordinates.
    #[test]
    fn byte_identical_occurrences_never_merge(
        payload in proptest::collection::vec(any::<u8>(), 1..40),
    ) {
        let a = record("content-same", "occ-p", 1, "src-p", "docs/p.txt", Some(&payload), &["team-x"]);
        let b = record("content-same", "occ-q", 1, "src-q", "docs/q.txt", Some(&payload), &["team-x"]);
        let resolver = ReferenceResolver::new(vec![a, b]);
        let granted = grants(&["team-x"]);
        let ra = resolver.resolve(&Request { occurrence_id: "occ-p".into(), version: None, granted: granted.clone() });
        let rb = resolver.resolve(&Request { occurrence_id: "occ-q".into(), version: None, granted });
        match (ra, rb) {
            (Response::Citation { source_path: pa, .. }, Response::Citation { source_path: pb, .. }) => {
                prop_assert_eq!(pa, "docs/p.txt".to_string());
                prop_assert_eq!(pb, "docs/q.txt".to_string());
            }
            other => prop_assert!(false, "expected two citations, got {other:?}"),
        }
    }

    /// Current revocation makes a previously valid citation unavailable,
    /// answered exactly like an unknown id.
    #[test]
    fn revocation_makes_old_citations_unavailable(
        payload in proptest::collection::vec(any::<u8>(), 1..40),
    ) {
        let before = record("content-rv", "occ-rv", 1, "src-r", "docs/r.txt", Some(&payload), &["team-x"]);
        let granted = grants(&["team-x"]);
        let ok = ReferenceResolver::new(vec![before.clone()]).resolve(&Request {
            occurrence_id: "occ-rv".into(),
            version: None,
            granted: granted.clone(),
        });
        let is_citation = matches!(ok, Response::Citation { .. });
        prop_assert!(is_citation);
        let mut after = before;
        after.rights = grants(&["team-y"]);
        let revoked = ReferenceResolver::new(vec![after]).resolve(&Request {
            occurrence_id: "occ-rv".into(),
            version: None,
            granted,
        });
        prop_assert_eq!(revoked, Response::NotFound);
    }

    /// A hidden insertion never changes visible counts, ids, or any
    /// existing resolution result.
    #[test]
    fn hidden_insertion_changes_nothing_visible(
        payload in proptest::collection::vec(any::<u8>(), 1..40),
    ) {
        let base = record("content-h", "occ-h", 1, "src-h", "docs/h.txt", Some(&payload), &["team-x"]);
        let granted = grants(&["team-x"]);
        let before = ReferenceResolver::new(vec![base.clone()]);
        let mut hidden = record("content-h2", "occ-h2", 1, "src-h2", "docs/h2.txt", Some(b"secret"), &["team-x"]);
        hidden.hidden = true;
        let after = ReferenceResolver::new(vec![base, hidden]);
        prop_assert_eq!(before.visible_count(&granted), after.visible_count(&granted));
        prop_assert_eq!(before.visible_ids(&granted), after.visible_ids(&granted));
        let req = Request { occurrence_id: "occ-h".into(), version: None, granted };
        prop_assert_eq!(before.resolve(&req), after.resolve(&req));
    }
}
