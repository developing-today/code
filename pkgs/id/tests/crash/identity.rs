//! The crash gate for the identity store (plan step P1.10). Built with the `web` feature.
//!
//! The store's own redb operations run on the fault model: creating the store, which makes its
//! first commit, then `create` and `update` of identities, through `id::web::crash_gate`. Client
//! IDs are fixed, so the storage-call trace is the same on every run and the corpus can name a
//! crash point by its call number.
//!
//! Creation also changes the directory, which the fault model does not see. The store builds its
//! file under a temporary name, makes the first commit, renames the file into place, and syncs the
//! directory, all before it returns. So a power cut inside creation leaves the disk image at the
//! temporary name and nothing at the store's name. A cut after creation leaves the image at the
//! store's name. Each image is written to a real file and reopened through `crash_gate::open_path`,
//! the path `IdentityStore::new` takes, and the records are checked against the model.
//!
//! The in-place control writes the creation images at the store's name, as if creation wrote
//! there directly. That is the failure the temporary name removes, so the control must fail.

use crate::corpus::{self, Expect, Header};
use crate::faultfs::{FaultFs, Plan, Trace, splitmix};
use crate::harness::{Outcome, Verdict, classify, panic_text};
use crate::sweep::{Job, describe, jobs_for, report, run_jobs_with, sample_events};
use id::web::crash_gate::{self, ClientIdentity, Store};
use redb::Database;
use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

/// The store's secret key in every run.
const SECRET: [u8; 32] = [7u8; 32];

/// Identities the workload creates. Every other one is renamed once, after it is created.
const CREATES: u64 = 40;

/// The default run crashes at every `SAMPLE_STRIDE`th storage call, and at every creation call.
const SAMPLE_STRIDE: u64 = 20;

/// How many failing runs an assertion message lists.
const LISTED: usize = 20;

const CORPUS_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/crash/identity_corpus.tsv"
);
const CORPUS: &str = include_str!("identity_corpus.tsv");

const PREAMBLE: &str = "\
# Crash corpus for the identity store (tests/crash/identity.rs). Replayed by identity::corpus_replays_with_recorded_verdicts.
# Each line is one crash point of the identity workload: the plan that cuts power there, and the verdict its run must keep.
# expect `ok` accepts either valid prefix (ok_acked or ok_acked_plus_one). Any other value must match.
# trace_fnv1a is FNV-1a over (kind, offset, length) of every storage call of the clean run.
# Regenerate, from pkgs/id: cargo test --release --features web --test crash write_corpus_identity -- --ignored
# Each sweep failure is added by hand as a fix-<commit> row, with the verdict the fixed code gives.
";

/// One identity as the gate compares it: its name, its creation time, and its last update time.
type Entry = (Option<String>, u64, u64);

/// Every identity in a store, by client ID.
type Snapshot = BTreeMap<String, Entry>;

/// One step after creation: a new identity, or a new name for an existing one.
#[derive(Clone, Debug)]
enum Op {
    Create(ClientIdentity),
    Rename(ClientIdentity),
}

/// The client ID of identity `i`. The IDs are fixed but not in creation order, so the B-tree sees
/// keys out of order.
fn client_id(i: u64) -> String {
    format!("{:016x}{:016x}", splitmix(i), splitmix(i ^ 0xFFFF_0000))
}

const fn created_at(i: u64) -> u64 {
    1_700_000_000 + i
}

/// A name for identity `i` in round `round`. Some names are absent, and their lengths vary, so
/// the values differ in size.
fn name(i: u64, round: u64) -> Option<String> {
    if (i + round) % 5 == 4 {
        None
    } else {
        let padding = "x".repeat(((i * 7 + round * 3) % 40) as usize);
        Some(format!("user-{i:02}-r{round}-{padding}"))
    }
}

/// The workload's steps after creation: each identity is created, and every other one is renamed.
fn ops() -> Vec<Op> {
    let mut ops = Vec::new();
    for i in 0..CREATES {
        ops.push(Op::Create(ClientIdentity {
            client_id: client_id(i),
            name: name(i, 0),
            created_at: created_at(i),
            updated_at: created_at(i),
        }));
        if i % 2 == 1 {
            let j = i / 2;
            ops.push(Op::Rename(ClientIdentity {
                client_id: client_id(j),
                name: name(j, 1),
                created_at: created_at(j),
                updated_at: 1_700_100_000 + i,
            }));
        }
    }
    ops
}

fn apply(model: &mut Snapshot, op: &Op) {
    match op {
        Op::Create(identity) | Op::Rename(identity) => {
            model.insert(
                identity.client_id.clone(),
                (
                    identity.name.clone(),
                    identity.created_at,
                    identity.updated_at,
                ),
            );
        }
    }
}

fn commit(store: &Store, op: &Op) -> anyhow::Result<()> {
    match op {
        Op::Create(identity) => store.create(identity),
        Op::Rename(identity) => store.update(identity),
    }
}

/// `models()[n]` is the store after `n` steps. Step 0 is before creation, and step 1 is the store
/// as creation leaves it, which is empty. Step `k + 2` is the state after the `k`th op.
fn models() -> Vec<Snapshot> {
    let mut out = vec![Snapshot::new(), Snapshot::new()];
    let mut model = Snapshot::new();
    for op in ops() {
        apply(&mut model, &op);
        out.push(model.clone());
    }
    out
}

fn snapshot(records: Vec<ClientIdentity>) -> Snapshot {
    records
        .into_iter()
        .map(|r| (r.client_id, (r.name, r.created_at, r.updated_at)))
        .collect()
}

/// Creates the store on `fs`, then runs every op. `acked` counts the steps that returned, creation
/// included. `made` is the number of storage calls creation made.
fn run_workload(fs: &FaultFs, acked: &mut u64, made: &mut u64) -> Result<(), String> {
    let db = Database::builder()
        .create_with_backend(fs.clone())
        .map_err(|e| format!("create: {e}"))?;
    let store = crash_gate::open_database(db, &SECRET).map_err(|e| format!("open: {e:#}"))?;
    if fs.cut() {
        // The process died inside creation. Nothing is acknowledged.
        return Ok(());
    }
    *made = fs.trace().len() as u64;
    *acked = 1;
    for (k, op) in ops().iter().enumerate() {
        let result = commit(&store, op);
        if fs.cut() {
            // The process died inside this call. The op is not acknowledged.
            return Ok(());
        }
        result.map_err(|e| format!("op {k}: {e:#}"))?;
        *acked = k as u64 + 2;
    }
    drop(store);
    Ok(())
}

/// The disk image after creation returns, with nothing else done.
fn creation_image() -> Vec<u8> {
    let fs = FaultFs::new(Vec::new(), None, false);
    let db = Database::builder()
        .create_with_backend(fs.clone())
        .expect("create the database");
    let store = crash_gate::open_database(db, &SECRET).expect("create the store");
    let image = fs.disk_image();
    drop(store);
    image
}

/// Where a run's disk image is written for the reopen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    /// As the store leaves it: a cut inside creation leaves the image at the temporary name.
    AsStored,
    /// The control: the image of a cut inside creation goes at the store's name.
    InPlace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// The temporary name a new store is built under.
    Temporary,
    /// The store's own name.
    Store,
}

/// One run: its outcome, the storage-call trace, and how many calls creation made.
struct Run {
    outcome: Outcome,
    trace: Trace,
    made: u64,
}

/// Runs the workload with one plan, reopens the disk image, and classifies the result. `created`
/// is the creation length of the clean run: a cut at or before it lands inside creation.
fn run_identity(
    plan: Option<Plan>,
    lying: bool,
    models: &[Snapshot],
    created: u64,
    placement: Placement,
) -> Run {
    let fs = FaultFs::new(Vec::new(), plan, lying);
    let mut acked = 0u64;
    let mut made = 0u64;
    let attempt = panic::catch_unwind(AssertUnwindSafe(|| {
        run_workload(&fs, &mut acked, &mut made)
    }));
    let trace = fs.trace();
    let in_creation = plan.is_some_and(|p| p.event <= created);
    let outcome = match attempt {
        Err(payload) if !fs.cut() => Outcome {
            verdict: Verdict::Panic,
            acked,
            repairs: 0,
            detail: format!(
                "panic before the power cut: {}",
                panic_text(payload.as_ref())
            ),
        },
        Ok(Err(e)) if !fs.cut() => Outcome {
            verdict: Verdict::NoCrashError,
            acked,
            repairs: 0,
            detail: e,
        },
        // A cut (or a clean finish) leaves a disk image to reopen. A failure after the cut is the
        // dead process failing, not a finding.
        _ => {
            let target = match (in_creation, placement) {
                (true, Placement::AsStored) => Target::Temporary,
                _ => Target::Store,
            };
            let image = fs.disk_image();
            match panic::catch_unwind(AssertUnwindSafe(|| verify(&image, target, acked, models))) {
                Ok(outcome) => outcome,
                Err(payload) => Outcome {
                    verdict: Verdict::Panic,
                    acked,
                    repairs: 0,
                    detail: format!(
                        "panic while reopening the image: {}",
                        panic_text(payload.as_ref())
                    ),
                },
            }
        }
    };
    Run {
        outcome,
        trace,
        made,
    }
}

/// Writes the image where the store would have left it, then reopens it.
fn verify(image: &[u8], target: Target, acked: u64, models: &[Snapshot]) -> Outcome {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("identity.redb");
    let file = match target {
        Target::Temporary => crash_gate::creating_path(&path),
        Target::Store => path.clone(),
    };
    std::fs::write(&file, image).expect("write the disk image");
    check_reopened(&path, acked, models)
}

/// Reopens the store at `path` through the production open, and checks it. It must open and hold a
/// valid prefix of the workload. Then the file the store left is checked in memory: it must pass an
/// integrity check, and take new commits that survive another reopen.
fn check_reopened(path: &Path, acked: u64, models: &[Snapshot]) -> Outcome {
    let outcome = |verdict, detail: String| Outcome {
        verdict,
        acked,
        repairs: 0,
        detail,
    };
    let store = match crash_gate::open_path(path, &SECRET) {
        Ok(store) => store,
        Err(e) => return outcome(Verdict::OpenError, format!("open: {e:#}")),
    };
    let state = match store.records() {
        Ok(records) => snapshot(records),
        Err(e) => return outcome(Verdict::OpenError, format!("records: {e:#}")),
    };
    drop(store);
    let verdict = classify(&state, models, acked);
    if verdict.is_failure() {
        return outcome(verdict, format!("{} identities reopened", state.len()));
    }
    // The fsyncs of a commit dominate a run, so the checks after the open run on a copy in memory.
    let image = match std::fs::read(path) {
        Ok(image) => image,
        Err(e) => return outcome(Verdict::OpenError, format!("read back: {e}")),
    };
    if let Err(e) = integrity(image.clone()) {
        return outcome(Verdict::Integrity, e);
    }
    if let Err(e) = post_recovery(image, &state) {
        return outcome(Verdict::PostRecovery, e);
    }
    outcome(verdict, String::new())
}

/// Checks the integrity of a database image in memory.
fn integrity(image: Vec<u8>) -> Result<(), String> {
    let mut db = Database::builder()
        .create_with_backend(FaultFs::new(image, None, false))
        .map_err(|e| format!("integrity open: {e}"))?;
    match db.check_integrity() {
        Ok(true) => Ok(()),
        Ok(false) => Err("check_integrity returned false".into()),
        Err(e) => Err(format!("check_integrity: {e}")),
    }
}

/// Creates one identity and renames another on the reopened store, then reopens it again. The
/// recovered records and both new commits must survive. Runs on a copy of the image in memory.
fn post_recovery(image: Vec<u8>, recovered: &Snapshot) -> Result<(), String> {
    let mut want = recovered.clone();
    let fresh = ClientIdentity {
        client_id: "post-recovery".into(),
        name: Some("after the cut".into()),
        created_at: 1,
        updated_at: 1,
    };
    want.insert(
        fresh.client_id.clone(),
        (fresh.name.clone(), fresh.created_at, fresh.updated_at),
    );
    let backing = FaultFs::new(image, None, false);
    let db = Database::builder()
        .create_with_backend(backing.clone())
        .map_err(|e| format!("reopen: {e}"))?;
    let store = crash_gate::open_database(db, &SECRET).map_err(|e| format!("reopen: {e:#}"))?;
    store
        .create(&fresh)
        .map_err(|e| format!("create after recovery: {e:#}"))?;
    if let Some((id, (_, created, _))) = recovered.iter().next() {
        let renamed = ClientIdentity {
            client_id: id.clone(),
            name: Some("renamed after the cut".into()),
            created_at: *created,
            updated_at: 2,
        };
        store
            .update(&renamed)
            .map_err(|e| format!("update after recovery: {e:#}"))?;
        want.insert(id.clone(), (renamed.name, *created, 2));
    }
    drop(store);
    let db = Database::builder()
        .create_with_backend(FaultFs::new(backing.disk_image(), None, false))
        .map_err(|e| format!("second reopen: {e}"))?;
    let again =
        crash_gate::open_database(db, &SECRET).map_err(|e| format!("second reopen: {e:#}"))?;
    let got = snapshot(again.records().map_err(|e| format!("records: {e:#}"))?);
    if got == want {
        Ok(())
    } else {
        Err("state after the post-recovery commits and a clean reopen differs".into())
    }
}

/// The clean run's trace, and how many of its calls creation made.
fn shape() -> (Trace, u64) {
    let models = models();
    let run = run_identity(None, false, &models, 0, Placement::AsStored);
    assert_eq!(
        run.outcome.verdict,
        Verdict::OkAcked,
        "{}",
        run.outcome.detail
    );
    assert_eq!(run.outcome.acked, models.len() as u64 - 1);
    assert!(run.made > 0 && run.made < run.trace.len() as u64);
    (run.trace, run.made)
}

/// Every creation call. These are the first-creation crash points.
fn creation_events(created: u64) -> Vec<u64> {
    (1..=created).collect()
}

/// Every creation call, every `SAMPLE_STRIDE`th call, and the last one.
fn sampled_events(trace: &Trace, created: u64) -> Vec<u64> {
    let mut events = sample_events(trace.len() as u64, SAMPLE_STRIDE);
    events.extend(creation_events(created));
    events.sort_unstable();
    events.dedup();
    events
}

/// Runs every plan of the given crash points. Returns the jobs, their outcomes, and the indices of
/// the failing runs.
fn sweep(
    label: &str,
    trace: &Trace,
    created: u64,
    events: &[u64],
    lying: bool,
    placement: Placement,
) -> (Vec<Job>, Vec<Outcome>, Vec<usize>) {
    let models = models();
    let jobs = jobs_for(trace, events, lying);
    let outcomes = run_jobs_with(&jobs, |job| {
        run_identity(Some(job.plan), job.lying, &models, created, placement).outcome
    });
    let failing = report(label, &jobs, &outcomes);
    (jobs, outcomes, failing)
}

fn listing(jobs: &[Job], outcomes: &[Outcome], failing: &[usize]) -> String {
    failing
        .iter()
        .take(LISTED)
        .map(|&i| describe(&jobs[i], &outcomes[i]))
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_control_fails(outcomes: &[Outcome], failing: &[usize]) {
    assert!(
        !failing.is_empty(),
        "the lying-sync control produced no failures: the gate cannot see lost durability"
    );
    assert!(
        failing.iter().any(|&i| matches!(
            outcomes[i].verdict,
            Verdict::LostCommitted | Verdict::OpenError
        )),
        "the control failed, but never with lost_committed or open_error"
    );
}

#[test]
fn clean_run_reopens_with_every_step() {
    let models = models();
    let run = run_identity(None, false, &models, 0, Placement::AsStored);
    assert_eq!(
        run.outcome.verdict,
        Verdict::OkAcked,
        "{}",
        run.outcome.detail
    );
    assert_eq!(run.outcome.acked, models.len() as u64 - 1);
    assert!(run.made > 0 && run.made < run.trace.len() as u64);
}

/// Every crash point of creation, under every plan, must reopen to an empty store. This is the
/// first-creation guarantee: no crash leaves a file the store refuses to open.
#[test]
fn first_creation_has_no_failures() {
    let (trace, created) = shape();
    let events = creation_events(created);
    let (jobs, outcomes, failing) = sweep(
        "first creation",
        &trace,
        created,
        &events,
        false,
        Placement::AsStored,
    );
    assert!(
        failing.is_empty(),
        "{} failing runs:\n{}",
        failing.len(),
        listing(&jobs, &outcomes, &failing)
    );
}

/// After creation returns, the rename may or may not have reached the disk. Both states must open
/// to an empty store, and the temporary name must be gone afterwards.
#[test]
fn first_creation_rename_states_open() {
    let image = creation_image();
    let models = models();

    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("identity.redb");
    std::fs::write(&path, &image).expect("write the image at the store's name");
    let outcome = check_reopened(&path, 1, &models);
    assert_eq!(
        outcome.verdict,
        Verdict::OkAcked,
        "rename survived: {}",
        outcome.detail
    );
    drop(dir);

    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("identity.redb");
    std::fs::write(crash_gate::creating_path(&path), &image)
        .expect("write the image at the temporary name");
    let outcome = check_reopened(&path, 1, &models);
    assert_eq!(
        outcome.verdict,
        Verdict::OkAcked,
        "rename did not survive: {}",
        outcome.detail
    );
    assert!(!crash_gate::creating_path(&path).exists());
}

/// The control. With creation writing in place, some crash points leave a file at the store's name
/// that redb refuses to open. The gate must see those, or the first-creation guarantee is untested.
#[test]
fn in_place_creation_control_fails() {
    let (trace, created) = shape();
    let events = creation_events(created);
    let (_, outcomes, failing) = sweep(
        "control (in-place creation)",
        &trace,
        created,
        &events,
        false,
        Placement::InPlace,
    );
    assert!(
        !failing.is_empty(),
        "the in-place control produced no failures: the gate cannot see a torn first creation"
    );
    assert!(
        failing
            .iter()
            .all(|&i| outcomes[i].verdict == Verdict::OpenError),
        "the in-place control failed with something other than open_error"
    );
}

#[test]
fn sampled_sweep_has_no_failures() {
    let (trace, created) = shape();
    let events = sampled_events(&trace, created);
    let (jobs, outcomes, failing) = sweep(
        "sampled sweep",
        &trace,
        created,
        &events,
        false,
        Placement::AsStored,
    );
    assert!(
        failing.is_empty(),
        "{} failing runs:\n{}",
        failing.len(),
        listing(&jobs, &outcomes, &failing)
    );
}

#[test]
fn lying_sync_control_produces_failures() {
    let (trace, created) = shape();
    let events = sampled_events(&trace, created);
    let (_, outcomes, failing) = sweep(
        "control (lying sync)",
        &trace,
        created,
        &events,
        true,
        Placement::AsStored,
    );
    assert_control_fails(&outcomes, &failing);
}

#[test]
fn corpus_replays_with_recorded_verdicts() {
    let (trace, created) = shape();
    let models = models();
    let (header, rows) = corpus::parse(CORPUS).expect("identity_corpus.tsv parses");
    assert_eq!(
        header,
        Header {
            storage_calls: trace.len() as u64,
            trace_fnv1a: corpus::trace_fnv1a(&trace),
        },
        "the identity workload's storage-call trace changed. Check the change, then regenerate identity_corpus.tsv (see its header)"
    );
    let mut problems = Vec::new();
    for row in &rows {
        let actual = (row.job.plan.event as usize)
            .checked_sub(1)
            .and_then(|i| trace.get(i))
            .map(|call| call.0);
        if actual != Some(row.kind) {
            let actual = actual.map_or("no such call", |kind| kind.name());
            problems.push(format!(
                "{}: event {} is {actual}, not {}",
                row.name,
                row.job.plan.event,
                row.kind.name()
            ));
        }
    }
    let jobs: Vec<Job> = rows.iter().map(|row| row.job).collect();
    let outcomes = run_jobs_with(&jobs, |job| {
        run_identity(
            Some(job.plan),
            job.lying,
            &models,
            created,
            Placement::AsStored,
        )
        .outcome
    });
    for (row, outcome) in rows.iter().zip(&outcomes) {
        if !row.expect.accepts(outcome.verdict) {
            problems.push(format!(
                "{}: got {} ({}), expected {}",
                row.name,
                outcome.verdict.name(),
                outcome.detail,
                row.expect.name()
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{} corpus rows changed:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
#[ignore = "the full identity gate: every storage call under every plan. `just crash-gate` runs it in release"]
fn full_sweep_has_no_failures() {
    let (trace, created) = shape();
    let events: Vec<u64> = (1..=trace.len() as u64).collect();
    let (jobs, outcomes, failing) = sweep(
        "full identity sweep",
        &trace,
        created,
        &events,
        false,
        Placement::AsStored,
    );
    assert!(
        failing.is_empty(),
        "{} failing runs:\n{}",
        failing.len(),
        listing(&jobs, &outcomes, &failing)
    );
}

#[test]
#[ignore = "the full identity control: every storage call under a lying sync. `just crash-gate` runs it in release"]
fn full_lying_sync_control_produces_failures() {
    let (trace, created) = shape();
    let events: Vec<u64> = (1..=trace.len() as u64).collect();
    let (_, outcomes, failing) = sweep(
        "full identity control (lying sync)",
        &trace,
        created,
        &events,
        true,
        Placement::AsStored,
    );
    assert_control_fails(&outcomes, &failing);
}

/// Rewrites `identity_corpus.tsv`: every creation call and every faithful point that ends in the
/// plus-one state, plus two failing rows per class from the lying sync. The `fix-` rows already in
/// the file are kept. A faithful failure stops it, because such a failure needs a fix first.
#[test]
#[ignore = "rewrites identity_corpus.tsv. Run by hand, in release, after a deliberate workload change"]
fn write_corpus_identity() {
    let (trace, created) = shape();
    let all: Vec<u64> = (1..=trace.len() as u64).collect();

    let (jobs, outcomes, failing) = sweep(
        "faithful sweep",
        &trace,
        created,
        &all,
        false,
        Placement::AsStored,
    );
    assert!(
        failing.is_empty(),
        "a faithful run failed. Fix it, then add its row by hand as fix-<commit>:\n{}",
        listing(&jobs, &outcomes, &failing)
    );
    let mut rows = Vec::new();
    for (job, outcome) in jobs.iter().zip(&outcomes) {
        let (prefix, expect) = match outcome.verdict {
            Verdict::OkAcked if job.plan.event <= created => {
                ("creation", Expect::Exact(outcome.verdict))
            }
            Verdict::OkAckedPlusOne => ("plus-one", Expect::Valid),
            _ => continue,
        };
        rows.push(crate::corpus_row(
            crate::plan_name(prefix, job),
            *job,
            &trace,
            expect,
        ));
    }

    let (cjobs, coutcomes, _) = sweep(
        "control sweep",
        &trace,
        created,
        &all,
        true,
        Placement::AsStored,
    );
    let mut taken: BTreeMap<&str, usize> = BTreeMap::new();
    for (job, outcome) in cjobs.iter().zip(&coutcomes) {
        let v = outcome.verdict;
        let expect = match v {
            Verdict::OkAckedPlusOne => Expect::Valid,
            _ if v.is_failure() => Expect::Exact(v),
            _ => continue,
        };
        let count = taken.entry(v.name()).or_default();
        if *count < 2 {
            *count += 1;
            let name = format!("control-{}-{count}", v.name());
            rows.push(crate::corpus_row(name, *job, &trace, expect));
        }
    }

    let existing = std::fs::read_to_string(CORPUS_PATH).unwrap_or_default();
    if let Ok((_, kept)) = corpus::parse(&existing) {
        rows.extend(kept.into_iter().filter(|row| row.name.starts_with("fix-")));
    }

    let header = Header {
        storage_calls: trace.len() as u64,
        trace_fnv1a: corpus::trace_fnv1a(&trace),
    };
    std::fs::write(CORPUS_PATH, corpus::format_with(PREAMBLE, &header, &rows))
        .expect("write identity_corpus.tsv");
    println!("wrote {} rows to {CORPUS_PATH}", rows.len());
}
