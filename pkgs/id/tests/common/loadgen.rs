//! The shared load & workload generator core (#5891, step 1 of the
//! record's implementation order; step packet #8307).
//!
//! One generator for the honest benchmark workloads and the
//! adversarial DoS profiles, per the design record
//! (`doc/2026-11-08T11-27-00Z_design_shared_workload_generator/`):
//!
//! - **Workloads are data, profiles are parameters.** A [`Workload`]
//!   is a named [`MixTable`] over the operation families; a
//!   [`Profile`] parameterizes pacing and misbehaviour. The same
//!   workload definition runs as a benchmark and as the honest phase
//!   of a DoS scenario; only the profile changes.
//! - **Determinism before realism.** Every run is driven by a
//!   recorded seed; mixes are fixed counts per 1,000 ops, never
//!   time-sampled; payload corpora are generated from the seed,
//!   never fetched. A run is replayable from its header (seed +
//!   workload + profile + scale + surface) or it is a debugging
//!   session, not a measurement.
//! - **Surfaces behind one trait.** [`Target`] is the surface
//!   contract of record §2 (`InProcess` / `LocalSocket` / `Iroh`).
//!   Step 1 ships the scheduler and the `InProcess` surface (the
//!   `world_bench` pattern: direct calls on the core types, provided
//!   by the bench). `LocalSocket` and `Iroh` are steps 3 and 5.
//! - **The generator asserts nothing.** It reports counts, errors
//!   and latencies; verdicts belong to the consumers (#5894
//!   baselines, #5892 scenarios). The measurement header/summary
//!   writer of record §4 is step 2; this step provides the header
//!   fields and the per-family summary the writer will print.
//!
//! This module is `std`-only on purpose: integration tests reach it
//! through `mod common;`, benches through a `#[path]` include, and
//! neither context may grow a dependency for it.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

/// The generator version. A change to a unit, a metric definition,
/// or a workload's mix table is a version event under #5894's
/// baseline versioning: numbers across the event are incomparable.
pub const GENERATOR_VERSION: u64 = 2;

/// SplitMix64: a small deterministic PRNG, seeded per run. Chosen
/// because it is fully specified here — the same seed produces the
/// same stream on every machine, which the determinism invariant
/// (record decision 4) requires of the corpus and the scheduler.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn fill_bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}

/// One operation family a workload's mix is stated over (record §1,
/// §2). `JournalCompact` extends the journal family for the existing
/// `checkpoint_compaction` workload, whose measured operation is a
/// compaction, not an append or a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpKind {
    Identity,
    CapabilityCheck,
    JournalAppend,
    JournalRead,
    JournalCompact,
    WorldSync,
    BlobPut,
    BlobGet,
}

impl OpKind {
    pub const ALL: [OpKind; 8] = [
        OpKind::Identity,
        OpKind::CapabilityCheck,
        OpKind::JournalAppend,
        OpKind::JournalRead,
        OpKind::JournalCompact,
        OpKind::WorldSync,
        OpKind::BlobPut,
        OpKind::BlobGet,
    ];

    pub fn name(self) -> &'static str {
        match self {
            OpKind::Identity => "identity",
            OpKind::CapabilityCheck => "capability_check",
            OpKind::JournalAppend => "journal_append",
            OpKind::JournalRead => "journal_read",
            OpKind::JournalCompact => "journal_compact",
            OpKind::WorldSync => "world_sync",
            OpKind::BlobPut => "blob_put",
            OpKind::BlobGet => "blob_get",
        }
    }
}

/// Why a mix table cannot be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixError {
    /// Counts must sum to exactly 1,000 (the mix is counts per 1,000 ops).
    TotalNotThousand(u32),
    /// The same family appears twice; state its total once.
    DuplicateKind(OpKind),
}

impl fmt::Display for MixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MixError::TotalNotThousand(total) => {
                write!(f, "mix counts sum to {total}, not 1000")
            }
            MixError::DuplicateKind(kind) => {
                write!(f, "mix states {} twice", kind.name())
            }
        }
    }
}

impl std::error::Error for MixError {}

/// A workload's operation mix: counts per 1,000 ops per family
/// (record §1). The op stream of a run is a pure function of the
/// mix: op `i` is decided by `i % 1000` walking the table in
/// declaration order, so the same mix always produces the same
/// stream, with no sampling and no seed dependence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MixTable {
    entries: Vec<(OpKind, u32)>,
}

impl MixTable {
    /// # Errors
    ///
    /// Fails unless the counts sum to exactly 1,000 with no family
    /// stated twice.
    pub fn new(entries: &[(OpKind, u32)]) -> Result<Self, MixError> {
        let mut total = 0u32;
        let mut seen: Vec<OpKind> = Vec::new();
        for &(kind, count) in entries {
            if seen.contains(&kind) {
                return Err(MixError::DuplicateKind(kind));
            }
            seen.push(kind);
            total = total.saturating_add(count);
        }
        if total != 1_000 {
            return Err(MixError::TotalNotThousand(total));
        }
        Ok(Self {
            entries: entries.to_vec(),
        })
    }

    /// A mix that is only this family (the existing `world_bench`
    /// workloads are each a single family, stated honestly).
    pub fn single(kind: OpKind) -> Self {
        Self {
            entries: vec![(kind, 1_000)],
        }
    }

    /// The operation at stream position `index`. Pure: the same
    /// index always yields the same operation.
    pub fn op_at(&self, index: u64) -> OpKind {
        let mut at = index % 1_000;
        for &(kind, count) in &self.entries {
            if at < u64::from(count) {
                return kind;
            }
            at -= u64::from(count);
        }
        // The table sums to 1,000, so some entry always claims the
        // position; the last entry is the unreachable fallback.
        self.entries[self.entries.len() - 1].0
    }

    /// The declared count per 1,000 ops for a family (0 if absent).
    pub fn count(&self, kind: OpKind) -> u32 {
        self.entries
            .iter()
            .find(|&&(k, _)| k == kind)
            .map_or(0, |&(_, c)| c)
    }
}

/// The seeded payload corpus (record decision 4): payloads are
/// generated from the run's seed, never fetched, so a run's traffic
/// is a pure function of (seed, actor, op index).
#[derive(Clone, Copy, Debug)]
pub struct Corpus {
    seed: u64,
}

impl Corpus {
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// The payload bytes for one operation. Same (seed, actor,
    /// index, len) in, same bytes out, on every machine.
    pub fn payload(&self, actor: u32, index: u64, len: usize) -> Vec<u8> {
        let mut rng = SplitMix64::new(
            self.seed ^ (u64::from(actor) << 32) ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15),
        );
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    /// A printable payload of exactly `len` bytes, for surfaces
    /// whose payloads are text (chat bodies, journal lines).
    pub fn text(&self, actor: u32, index: u64, len: usize) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789 ";
        let bytes = self.payload(actor, index, len);
        bytes
            .iter()
            .map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char)
            .collect()
    }
}

/// One virtual actor of a run. Its identity material is derived
/// from the run seed (record §1: "signing keys derived from the
/// seed"): the bytes are deterministic per (seed, index); a surface
/// turns them into its real key type at connect time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actor {
    pub index: u32,
    pub key: [u8; 32],
}

impl Actor {
    pub fn derive(seed: u64, index: u32) -> Self {
        let mut rng = SplitMix64::new(seed ^ 0xAC70_0000_0000_0001 ^ u64::from(index));
        let mut key = [0u8; 32];
        rng.fill_bytes(&mut key);
        Self { index, key }
    }
}

/// How a run's operations are issued (record §1). The modes are
/// never compared: the mode is part of the profile and the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pacing {
    /// Actors issue the next op when the previous completes — the
    /// benchmark mode, measuring service capacity.
    ClosedLoop,
    /// Ops issued at a target rate regardless of completions — the
    /// DoS mode, measuring overload behaviour.
    OpenLoop,
}

impl Pacing {
    pub fn name(self) -> &'static str {
        match self {
            Pacing::ClosedLoop => "closed_loop",
            Pacing::OpenLoop => "open_loop",
        }
    }
}

/// A named misbehaviour parameterization (record §3). Adversarial
/// profiles are modifiers on an honest profile, not workloads of
/// their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    None,
    Churn,
    Slowloris,
    Amplify,
    Bomb,
    Garbage,
    Exhaust,
}

impl Modifier {
    pub fn name(self) -> &'static str {
        match self {
            Modifier::None => "none",
            Modifier::Churn => "churn",
            Modifier::Slowloris => "slowloris",
            Modifier::Amplify => "amplify",
            Modifier::Bomb => "bomb",
            Modifier::Garbage => "garbage",
            Modifier::Exhaust => "exhaust",
        }
    }
}

/// A profile: the parameterization of a workload (record §3) —
/// pacing, and for adversarial profiles the modifier and the target
/// rate. Honest profiles are closed-loop with no modifier; the
/// scales of #5894 (S0/S1/S2) arrive as the run's scale field, not
/// as separate profiles.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub pacing: Pacing,
    pub modifier: Modifier,
    /// The open-loop target rate (ops/sec across the run). Honest
    /// (closed-loop) profiles carry none.
    pub target_rate_per_sec: Option<f64>,
}

impl Profile {
    pub fn honest(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pacing: Pacing::ClosedLoop,
            modifier: Modifier::None,
            target_rate_per_sec: None,
        }
    }

    pub fn adversarial(
        name: impl Into<String>,
        modifier: Modifier,
        target_rate_per_sec: f64,
    ) -> Self {
        Self {
            name: name.into(),
            pacing: Pacing::OpenLoop,
            modifier,
            target_rate_per_sec: Some(target_rate_per_sec),
        }
    }
}

/// The interface surface a run drives (record §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    InProcess,
    LocalSocket,
    Iroh,
}

impl Surface {
    pub fn name(self) -> &'static str {
        match self {
            Surface::InProcess => "in_process",
            Surface::LocalSocket => "local_socket",
            Surface::Iroh => "iroh",
        }
    }
}

/// A workload definition: a name, its mix, and the per-actor shape
/// of a run over it. Phase boundaries are op counts, not wall-clock
/// (record §1): `warm_up_ops` per actor are executed and discarded
/// before the measured ops.
#[derive(Clone, Debug, PartialEq)]
pub struct Workload {
    pub name: String,
    pub mix: MixTable,
    pub ops_per_actor: u64,
    pub warm_up_ops: u64,
    /// The corpus payload size for operations that carry one.
    pub payload_bytes: usize,
}

/// Everything that identifies a run (record decision 4, §4). The
/// server commit and machine class of the §4 header belong to the
/// harness that owns the server process; the fields here are the
/// generator's own.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSpec {
    pub seed: u64,
    pub workload: Workload,
    pub profile: Profile,
    /// The #5894 scale name (S0, S1, S2), as recorded.
    pub scale: String,
    pub surface: Surface,
    pub actors: u32,
}

impl RunSpec {
    /// The run header in the key=value style `world_bench` emits
    /// and `BASELINES.md` parses (record §4). The writer that prints
    /// it as part of a run is step 2; the fields and their order are
    /// fixed here so the header cannot drift between consumers.
    pub fn header_line(&self) -> String {
        format!(
            "loadgen seed={} workload={} profile={} pacing={} modifier={} scale={} surface={} actors={} generator={}",
            self.seed,
            self.workload.name,
            self.profile.name,
            self.profile.pacing.name(),
            self.profile.modifier.name(),
            self.scale,
            self.surface.name(),
            self.actors,
            GENERATOR_VERSION,
        )
    }
}

/// Why an operation or a run failed at the surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpError {
    /// The surface cannot express this operation. Recorded, never
    /// silently downgraded to a different operation (record §2).
    Unsupported(OpKind),
    /// The core scheduler executes closed-loop runs in this step;
    /// open-loop pacing is the DoS consumers' executor work
    /// (record implementation order, step 4).
    UnsupportedPacing,
    /// The operation itself failed; the run records it by class and
    /// continues (record §4: errors are measurement, not verdicts).
    Failed(String),
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpError::Unsupported(kind) => {
                write!(
                    f,
                    "operation {} is unsupported on this surface",
                    kind.name()
                )
            }
            OpError::UnsupportedPacing => {
                write!(f, "this step's scheduler executes closed-loop runs only")
            }
            OpError::Failed(msg) => write!(f, "operation failed: {msg}"),
        }
    }
}

impl std::error::Error for OpError {}

/// The surface contract of record §2, in the synchronous shape the
/// step-1 surfaces use. `connect` is per actor: the surface turns
/// the actor's derived identity material into its real session. A
/// workload names the operations it needs, never the transport; the
/// adapter owns framing, connection handling, and teardown.
pub trait Target<S> {
    /// # Errors
    ///
    /// Fails when the surface cannot establish the actor's session.
    fn connect(&mut self, seed: u64, actor: &Actor) -> Result<S, OpError>;

    /// Execute one operation of the run's stream, returning its
    /// latency. `Err(OpError::Unsupported)` refuses the operation
    /// class for this surface; any other error is recorded as an
    /// operation failure and the run continues.
    ///
    /// # Errors
    ///
    /// As described above.
    fn execute(&mut self, session: &mut S, op: OpKind, payload: &[u8])
    -> Result<Duration, OpError>;

    /// Tear the session down. Called exactly once per successful
    /// `connect`, including after an unsupported operation ended
    /// the actor's stream early.
    ///
    /// # Errors
    ///
    /// Fails when teardown itself fails; the run reports the first
    /// teardown failure after the stream completes.
    fn disconnect(&mut self, session: S) -> Result<(), OpError>;
}

/// Per-family measurement: the summary's raw material (record §4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpStats {
    pub count: u64,
    pub errors: u64,
    pub latencies: Vec<Duration>,
}

impl OpStats {
    /// Sum of the measured operation latencies.
    pub fn total(&self) -> Duration {
        self.latencies.iter().sum()
    }

    /// Mean measured latency, or zero when nothing was measured.
    pub fn mean(&self) -> Duration {
        if self.latencies.is_empty() {
            return Duration::ZERO;
        }
        self.total() / u32::try_from(self.latencies.len()).unwrap_or(u32::MAX)
    }
}

/// The outcome of a run: the header that identifies it and the
/// per-family measurement. Warm-up operations are executed but
/// never appear here (record §1: the warm-up phase is discarded).
#[derive(Clone, Debug, PartialEq)]
pub struct RunSummary {
    pub header: String,
    pub per_kind: BTreeMap<OpKind, OpStats>,
    pub ops: u64,
    pub errors: u64,
}

impl RunSummary {
    pub fn stats(&self, kind: OpKind) -> OpStats {
        self.per_kind.get(&kind).cloned().unwrap_or_default()
    }
}

/// The scheduler: a single-threaded, closed-loop driver over a
/// fixed population of virtual actors (record §1). Each actor
/// connects once, executes `warm_up_ops + ops_per_actor` operations
/// from the workload's mix (op index within the actor decides the
/// family, so every actor runs the same stream), and disconnects.
/// The run is a pure function of the spec and the surface's answers:
/// same seed, same stream, same payloads.
///
/// # Errors
///
/// - [`OpError::UnsupportedPacing`] for an open-loop profile (see
///   the variant: this step's executor is closed-loop).
/// - [`OpError::Unsupported`] the first time the surface refuses an
///   operation family — the run stops rather than downgrading.
/// - A `connect` failure, propagated as-is.
pub fn run<T, S>(target: &mut T, spec: &RunSpec) -> Result<RunSummary, OpError>
where
    T: Target<S>,
{
    if spec.profile.pacing == Pacing::OpenLoop {
        return Err(OpError::UnsupportedPacing);
    }
    let corpus = Corpus::new(spec.seed);
    let mut summary = RunSummary {
        header: spec.header_line(),
        per_kind: BTreeMap::new(),
        ops: 0,
        errors: 0,
    };
    let stream_len = spec.workload.warm_up_ops + spec.workload.ops_per_actor;
    for actor_index in 0..spec.actors {
        let actor = Actor::derive(spec.seed, actor_index);
        let mut session = target.connect(spec.seed, &actor)?;
        let mut outcome: Result<(), OpError> = Ok(());
        for index in 0..stream_len {
            let kind = spec.workload.mix.op_at(index);
            let payload = corpus.payload(actor_index, index, spec.workload.payload_bytes);
            match target.execute(&mut session, kind, &payload) {
                Ok(latency) => {
                    if index >= spec.workload.warm_up_ops {
                        let stats = summary.per_kind.entry(kind).or_default();
                        stats.count += 1;
                        stats.latencies.push(latency);
                        summary.ops += 1;
                    }
                }
                Err(OpError::Unsupported(kind)) => {
                    outcome = Err(OpError::Unsupported(kind));
                    break;
                }
                Err(OpError::Failed(msg)) => {
                    if index >= spec.workload.warm_up_ops {
                        let stats = summary.per_kind.entry(kind).or_default();
                        stats.errors += 1;
                        summary.errors += 1;
                    }
                    let _ = msg;
                }
                Err(other) => {
                    outcome = Err(other);
                    break;
                }
            }
        }
        let teardown = target.disconnect(session);
        outcome?;
        teardown?;
    }
    Ok(summary)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn mix_table_must_sum_to_a_thousand() {
        assert_eq!(
            MixTable::new(&[(OpKind::JournalAppend, 999)]),
            Err(MixError::TotalNotThousand(999))
        );
        assert_eq!(
            MixTable::new(&[(OpKind::JournalAppend, 500), (OpKind::JournalAppend, 500)]),
            Err(MixError::DuplicateKind(OpKind::JournalAppend))
        );
        assert!(MixTable::new(&[(OpKind::JournalAppend, 600), (OpKind::WorldSync, 400)]).is_ok());
    }

    #[test]
    fn mix_stream_is_the_table_counts() {
        // The record's example shape: 60% append, 25% capability
        // check, 15% sync — over one 1,000-op period the stream
        // contains exactly the table's counts, in table order.
        let mix = MixTable::new(&[
            (OpKind::JournalAppend, 600),
            (OpKind::CapabilityCheck, 250),
            (OpKind::WorldSync, 150),
        ])
        .unwrap();
        let mut counts = BTreeMap::new();
        for i in 0..1_000 {
            *counts.entry(mix.op_at(i)).or_insert(0u32) += 1;
        }
        assert_eq!(counts[&OpKind::JournalAppend], 600);
        assert_eq!(counts[&OpKind::CapabilityCheck], 250);
        assert_eq!(counts[&OpKind::WorldSync], 150);
        // The stream is periodic and index-pure.
        assert_eq!(mix.op_at(0), mix.op_at(1_000));
        assert_eq!(mix.op_at(599), OpKind::JournalAppend);
        assert_eq!(mix.op_at(600), OpKind::CapabilityCheck);
        assert_eq!(mix.op_at(850), OpKind::WorldSync);
    }

    #[test]
    fn corpus_is_deterministic_per_seed() {
        // The acceptance invariant of #8307: same seed, same
        // workload sequence — the corpus is part of that sequence.
        let a = Corpus::new(42);
        let b = Corpus::new(42);
        for index in 0..16 {
            assert_eq!(a.payload(3, index, 64), b.payload(3, index, 64));
            assert_eq!(a.text(3, index, 32), b.text(3, index, 32));
        }
        let other = Corpus::new(43);
        assert_ne!(a.payload(3, 0, 64), other.payload(3, 0, 64));
        assert_ne!(a.payload(3, 0, 64), a.payload(4, 0, 64));
        assert_ne!(a.payload(3, 0, 64), a.payload(3, 1, 64));
        assert_eq!(a.payload(3, 0, 17).len(), 17);
        assert_eq!(a.text(3, 0, 17).len(), 17);
    }

    #[test]
    fn actors_derive_deterministically_from_the_seed() {
        assert_eq!(Actor::derive(7, 0), Actor::derive(7, 0));
        assert_ne!(Actor::derive(7, 0), Actor::derive(7, 1));
        assert_ne!(Actor::derive(7, 0), Actor::derive(8, 0));
    }

    /// A target that records the stream it was driven through.
    #[derive(Default)]
    struct RecordingTarget {
        stream: Vec<(u32, OpKind, Vec<u8>)>,
        connects: u32,
        disconnects: u32,
    }

    struct RecordingSession {
        actor: u32,
    }

    impl Target<RecordingSession> for RecordingTarget {
        fn connect(&mut self, _seed: u64, actor: &Actor) -> Result<RecordingSession, OpError> {
            self.connects += 1;
            Ok(RecordingSession { actor: actor.index })
        }

        fn execute(
            &mut self,
            session: &mut RecordingSession,
            op: OpKind,
            payload: &[u8],
        ) -> Result<Duration, OpError> {
            self.stream.push((session.actor, op, payload.to_vec()));
            Ok(Duration::from_micros(10))
        }

        fn disconnect(&mut self, _session: RecordingSession) -> Result<(), OpError> {
            self.disconnects += 1;
            Ok(())
        }
    }

    fn spec(seed: u64) -> RunSpec {
        RunSpec {
            seed,
            workload: Workload {
                name: "mixed".to_owned(),
                mix: MixTable::new(&[(OpKind::JournalAppend, 600), (OpKind::WorldSync, 400)])
                    .unwrap(),
                ops_per_actor: 10,
                warm_up_ops: 2,
                payload_bytes: 8,
            },
            profile: Profile::honest("bench"),
            scale: "S0".to_owned(),
            surface: Surface::InProcess,
            actors: 2,
        }
    }

    #[test]
    fn same_seed_same_stream() {
        // The seeded determinism invariant, end to end through the
        // scheduler: two runs of one spec drive identical streams.
        let mut a = RecordingTarget::default();
        let mut b = RecordingTarget::default();
        let summary_a = run(&mut a, &spec(99)).unwrap();
        let summary_b = run(&mut b, &spec(99)).unwrap();
        assert_eq!(a.stream, b.stream);
        assert_eq!(summary_a, summary_b);
        // Warm-up ops are driven but not measured.
        assert_eq!(a.stream.len(), 2 * 12);
        assert_eq!(summary_a.ops, 2 * 10);
        assert_eq!(a.connects, 2);
        assert_eq!(a.disconnects, 2);
    }

    #[test]
    fn different_seed_different_payloads() {
        let mut a = RecordingTarget::default();
        let mut b = RecordingTarget::default();
        run(&mut a, &spec(1)).unwrap();
        run(&mut b, &spec(2)).unwrap();
        let kinds_a: Vec<OpKind> = a.stream.iter().map(|(_, k, _)| *k).collect();
        let kinds_b: Vec<OpKind> = b.stream.iter().map(|(_, k, _)| *k).collect();
        assert_eq!(kinds_a, kinds_b, "the mix stream itself is seed-free");
        assert_ne!(a.stream, b.stream, "the payloads carry the seed");
    }

    #[test]
    fn unsupported_operation_stops_the_run() {
        struct RefusingTarget;
        struct Session;
        impl Target<Session> for RefusingTarget {
            fn connect(&mut self, _seed: u64, _actor: &Actor) -> Result<Session, OpError> {
                Ok(Session)
            }
            fn execute(
                &mut self,
                _session: &mut Session,
                op: OpKind,
                _payload: &[u8],
            ) -> Result<Duration, OpError> {
                Err(OpError::Unsupported(op))
            }
            fn disconnect(&mut self, _session: Session) -> Result<(), OpError> {
                Ok(())
            }
        }
        let mut target = RefusingTarget;
        assert_eq!(
            run(&mut target, &spec(5)),
            Err(OpError::Unsupported(OpKind::JournalAppend))
        );
    }

    #[test]
    fn open_loop_profiles_are_refused_by_this_scheduler() {
        let mut open = spec(5);
        open.profile = Profile::adversarial("dos", Modifier::Churn, 1_000.0);
        let mut target = RecordingTarget::default();
        assert_eq!(run(&mut target, &open), Err(OpError::UnsupportedPacing));
        assert_eq!(target.connects, 0, "a refused run must not connect");
    }

    #[test]
    fn header_line_carries_the_run_identity() {
        let line = spec(7).header_line();
        for field in [
            "seed=7",
            "workload=mixed",
            "profile=bench",
            "pacing=closed_loop",
            "scale=S0",
            "surface=in_process",
            "actors=2",
            "generator=2",
        ] {
            assert!(line.contains(field), "header {line:?} lacks {field}");
        }
    }
}
