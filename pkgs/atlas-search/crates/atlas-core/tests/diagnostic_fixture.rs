//! F01 — synthetic missing-versus-buried diagnostic fixture and runner
//! seam (developing-today/code#8525, design #6020 F01).
//!
//! This file is a closed synthetic world: an offline fixture pack plus
//! a test-local runner that decides, for one known target document,
//! at which pipeline stage the target was lost (missing-versus-buried)
//! and how complete the corpus enumeration behind that verdict is.
//! It is deliberately isolated: it does not use `EvalData`, the shared
//! evaluator in `atlas_core::eval`, any production API, trace schema,
//! corpus loader, model, or network.
//!
//! The runner accepts: a synthetic snapshot ID, independently
//! enumerated eligible documents, coverage completeness, target ID
//! and relevance, indexed IDs, generated ordered candidate IDs, a
//! served cutoff, context-included IDs, per-stage completion/error
//! state, and an answer-support flag. It returns the stage and the
//! coverage confidence as independent components of one diagnosis.
//!
//! Invariants the diagnosis must never break (the issue's three
//! never-present-as rules):
//! * unknown coverage and intentional (policy) exclusion are never
//!   presented as global absence from the corpus;
//! * a stage timeout/error is never presented as an empty successful
//!   candidate set (or as any verdict the failed stage would have
//!   decided);
//! * a served hit omitted from context packing is never presented as
//!   an answer-support failure — unsupported-answer judgments stay
//!   distinct from retrieval outcomes.
//!
//! Oracles (fleet testing doctrine: every test names its oracle):
//! * **the fixture table** — ten named example fixtures with
//!   hand-written expected diagnoses, kept as data separate from both
//!   decision procedures;
//! * **the reference decision procedure** — a second, independent
//!   implementation (reverse, observation-masking derivation) that
//!   the runner is checked against on generated finite corpora and
//!   on the exhaustive target-fate space;
//! * **the exhaustive-set oracle** — for generated corpora the
//!   expected tuple is whatever the reference derives from the raw
//!   sets; the runner must agree on every generated world.
//!
//! Relevance is fixture context, not a stage: the stage vocabulary is
//! factual (where the target is). Whether a stage is a *miss* depends
//! on relevance — see [`Diagnosis::is_miss`] and the controls: the
//! negative control asserts absence only for a relevant target under
//! complete enumeration.

use std::collections::BTreeSet;

/// A pipeline stage at which a run can time out or error. The rank of
/// a stage is its position in the pipeline: a verdict decided by an
/// earlier stage survives a later stage's failure; a verdict that the
/// failed stage itself would have decided does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineStage {
    Enumeration,
    Indexing,
    CandidateGeneration,
    Serving,
    ContextPacking,
    AnswerSupport,
}

impl PipelineStage {
    fn rank(self) -> usize {
        match self {
            PipelineStage::Enumeration => 0,
            PipelineStage::Indexing => 1,
            PipelineStage::CandidateGeneration => 2,
            PipelineStage::Serving => 3,
            PipelineStage::ContextPacking => 4,
            PipelineStage::AnswerSupport => 5,
        }
    }

    const ALL: [PipelineStage; 6] = [
        PipelineStage::Enumeration,
        PipelineStage::Indexing,
        PipelineStage::CandidateGeneration,
        PipelineStage::Serving,
        PipelineStage::ContextPacking,
        PipelineStage::AnswerSupport,
    ];
}

/// How a stage failed. The kind never changes the diagnosis logic —
/// a timeout and an error are equally "the stage did not complete" —
/// but fixtures name the kind the issue names (timeout/error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Timeout,
    Error,
}

/// Per-stage completion/error state for one supplied run: either the
/// whole pipeline completed, or exactly one stage failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StageFailure {
    stage: PipelineStage,
    kind: FailureKind,
}

/// Where the target's fate was decided — the diagnosis stage.
/// The vocabulary is factual: it says where the target is, not
/// whether that is good or bad (that judgment needs relevance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Fully enumerated and not in the corpus.
    AbsentFromCorpus,
    /// Not enumerated, and enumeration is incomplete: whereabouts
    /// unknown. Never a synonym for absence.
    UnknownCoverage,
    /// Intentionally excluded by policy. Never a synonym for
    /// absence; the fixture for this case is shaped like an ordinary
    /// not-found everywhere except this verdict.
    PolicyExcluded,
    /// Enumerated (stored) but never indexed.
    StoredNotIndexed,
    /// Indexed but generation never produced it as a candidate.
    IndexedNotCandidate,
    /// Generated as a candidate but ranked below the served cutoff.
    CandidateBelowCutoff,
    /// Served but omitted by context packing. A retrieval success
    /// with a packing loss — not an answer-support failure.
    ServedNotInContext,
    /// Delivered in context, but the answer is not supported.
    /// An answer-support failure, distinct from every retrieval
    /// stage above.
    ContextDeliveredUnsupported,
    /// Delivered in context and the answer is supported.
    ServedSupported,
    /// A stage timed out or errored before the target's fate could
    /// be decided; the payload is the failed stage.
    StageError(PipelineStage),
}

/// Coverage confidence, returned independently of the stage: how
/// complete the independent enumeration of eligible documents is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Coverage {
    Complete,
    Incomplete,
}

/// The runner's return: stage and coverage confidence, independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Diagnosis {
    stage: Stage,
    coverage: Coverage,
}

impl Diagnosis {
    /// A diagnosis is a *miss* only for a relevant target lost at a
    /// retrieval stage. Absence of an irrelevant target, a policy
    /// exclusion, and an answer-support failure are not misses.
    fn is_miss(&self, target_relevant: bool) -> bool {
        target_relevant
            && matches!(
                self.stage,
                Stage::StoredNotIndexed
                    | Stage::IndexedNotCandidate
                    | Stage::CandidateBelowCutoff
                    | Stage::ServedNotInContext
            )
    }
}

/// One supplied synthetic run: everything the runner may look at.
/// The world is closed — every set is supplied by the fixture, and
/// structural violations of the closed world are malformed input,
/// not diagnoses.
#[derive(Debug, Clone)]
struct FixtureInput {
    snapshot_id: String,
    /// Independently enumerated eligible documents (the stored
    /// universe of the synthetic snapshot).
    eligible: BTreeSet<String>,
    /// Whether that enumeration is complete.
    coverage_complete: bool,
    target: String,
    target_relevant: bool,
    /// The target is intentionally excluded by policy.
    policy_excluded: bool,
    /// Documents that reached the index. Must be a subset of
    /// `eligible` in a well-formed fixture.
    indexed: BTreeSet<String>,
    /// Generated candidates, in rank order. Must be drawn from
    /// `indexed`, without duplicates.
    candidates: Vec<String>,
    /// How many of the candidates were served (a prefix).
    served_cutoff: usize,
    /// Documents included in the answer context. Must be a subset
    /// of the served prefix.
    context_included: BTreeSet<String>,
    /// Whether the delivered answer is supported (judged on the
    /// delivered context, independently of retrieval).
    answer_supported: bool,
    /// Per-stage completion/error state.
    failure: Option<StageFailure>,
}

impl FixtureInput {
    fn served(&self) -> &[String] {
        &self.candidates[..self.served_cutoff]
    }
}

/// Malformed-input rejection: a structurally impossible fixture is
/// an error, never a diagnosis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Malformed(&'static str);

/// Runner-side validation (set-algebra formulation).
fn validate(input: &FixtureInput) -> Result<(), Malformed> {
    if input.snapshot_id.is_empty() {
        return Err(Malformed("snapshot_id is empty"));
    }
    if !input.indexed.is_subset(&input.eligible) {
        return Err(Malformed("indexed is not a subset of eligible"));
    }
    let mut seen = BTreeSet::new();
    for c in &input.candidates {
        if !seen.insert(c) {
            return Err(Malformed("duplicate candidate"));
        }
        if !input.indexed.contains(c) {
            return Err(Malformed("candidate not in indexed"));
        }
    }
    if input.served_cutoff > input.candidates.len() {
        return Err(Malformed("served_cutoff beyond candidate list"));
    }
    let served: BTreeSet<&String> = input.served().iter().collect();
    if !input.context_included.iter().all(|d| served.contains(d)) {
        return Err(Malformed("context includes an unserved document"));
    }
    Ok(())
}

/// Decision switches for the mutation controls. The production
/// runner uses `Rules::SOUND`; each mutant flips exactly one switch
/// to a deliberately wrong rule, and the mutation test proves the
/// fixture assertions detect the wrong outcome.
#[derive(Debug, Clone, Copy)]
struct Rules {
    /// Treat a candidate-generation failure as an empty successful
    /// candidate set.
    error_is_empty_candidates: bool,
    /// Treat incomplete coverage as proof of absence.
    unknown_is_absent: bool,
    /// Treat a context-packing omission as an answer-support
    /// failure.
    context_omission_is_support_failure: bool,
    /// Treat a policy exclusion as global absence.
    policy_is_absent: bool,
}

impl Rules {
    const SOUND: Rules = Rules {
        error_is_empty_candidates: false,
        unknown_is_absent: false,
        context_omission_is_support_failure: false,
        policy_is_absent: false,
    };
}

/// The runner under test: a forward cascade. It first derives the
/// target's fate and the rank of the stage that decided it, ignoring
/// failures; then it applies the failure rule — a verdict stands
/// only if a strictly earlier stage decided it.
fn diagnose_with(input: &FixtureInput, rules: Rules) -> Result<Diagnosis, Malformed> {
    validate(input)?;
    let coverage = if input.coverage_complete {
        Coverage::Complete
    } else {
        Coverage::Incomplete
    };
    if input.policy_excluded {
        let stage = if rules.policy_is_absent {
            Stage::AbsentFromCorpus
        } else {
            Stage::PolicyExcluded
        };
        return Ok(Diagnosis { stage, coverage });
    }
    let t = &input.target;
    // (verdict, rank of the deciding stage)
    let (stage, rank) = if !input.eligible.contains(t) {
        if input.coverage_complete || rules.unknown_is_absent {
            (Stage::AbsentFromCorpus, 0)
        } else {
            (Stage::UnknownCoverage, 0)
        }
    } else if !input.indexed.contains(t) {
        (Stage::StoredNotIndexed, 1)
    } else if !input.candidates.iter().any(|c| c == t) {
        (Stage::IndexedNotCandidate, 2)
    } else if input.candidates.iter().position(|c| c == t).unwrap() >= input.served_cutoff {
        (Stage::CandidateBelowCutoff, 3)
    } else if !input.context_included.contains(t) {
        if rules.context_omission_is_support_failure {
            (Stage::ContextDeliveredUnsupported, 5)
        } else {
            (Stage::ServedNotInContext, 4)
        }
    } else if !input.answer_supported {
        (Stage::ContextDeliveredUnsupported, 5)
    } else {
        (Stage::ServedSupported, 5)
    };
    if let Some(f) = input.failure {
        let failed_rank = f.stage.rank();
        let masked = failed_rank <= rank
            && !(rules.error_is_empty_candidates && f.stage == PipelineStage::CandidateGeneration);
        if masked {
            let coverage = if f.stage == PipelineStage::Enumeration {
                Coverage::Incomplete
            } else {
                coverage
            };
            return Ok(Diagnosis {
                stage: Stage::StageError(f.stage),
                coverage,
            });
        }
    }
    Ok(Diagnosis { stage, coverage })
}

/// The runner under test, with sound rules.
fn diagnose(input: &FixtureInput) -> Result<Diagnosis, Malformed> {
    diagnose_with(input, Rules::SOUND)
}

/// Reference-side validation, formulated independently of the
/// runner's: sort-and-compare over plain vectors instead of set
/// algebra.
fn reference_validate(input: &FixtureInput) -> Result<(), Malformed> {
    if input.snapshot_id.is_empty() {
        return Err(Malformed("snapshot_id is empty"));
    }
    let mut sorted_candidates = input.candidates.clone();
    sorted_candidates.sort();
    for w in sorted_candidates.windows(2) {
        if w[0] == w[1] {
            return Err(Malformed("duplicate candidate"));
        }
    }
    for c in &input.candidates {
        if !input.indexed.contains(c) {
            return Err(Malformed("candidate not in indexed"));
        }
    }
    for d in &input.indexed {
        if !input.eligible.contains(d) {
            return Err(Malformed("indexed is not a subset of eligible"));
        }
    }
    if input.served_cutoff > input.candidates.len() {
        return Err(Malformed("served_cutoff beyond candidate list"));
    }
    for d in &input.context_included {
        let in_served_prefix = input
            .candidates
            .iter()
            .take(input.served_cutoff)
            .any(|c| c == d);
        if !in_served_prefix {
            return Err(Malformed("context includes an unserved document"));
        }
    }
    Ok(())
}

/// The reference decision procedure: an independent implementation
/// of the same specification, written as a reverse derivation with
/// observation masking. It walks the pipeline from the target's
/// deepest possible stage backwards, and treats every observation
/// at or after a failed stage as untrusted (masked), so a verdict
/// is only ever formed from observations a completed stage made.
/// It shares no code with the runner's forward cascade.
fn reference_diagnose(input: &FixtureInput) -> Result<Diagnosis, Malformed> {
    reference_validate(input)?;
    let coverage = if input.coverage_complete {
        Coverage::Complete
    } else {
        Coverage::Incomplete
    };
    if input.policy_excluded {
        return Ok(Diagnosis {
            stage: Stage::PolicyExcluded,
            coverage,
        });
    }
    let failed = input.failure.map(|f| f.stage);
    // Masked check: the observation made by `stage` is trusted only
    // when no failure happened at or before it.
    let trusted = |stage: PipelineStage| failed.map_or(true, |f| stage.rank() < f.rank());
    let t = &input.target;

    if !trusted(PipelineStage::Enumeration) {
        return Ok(Diagnosis {
            stage: Stage::StageError(PipelineStage::Enumeration),
            coverage: Coverage::Incomplete,
        });
    }
    if !input.eligible.contains(t) {
        let stage = if input.coverage_complete {
            Stage::AbsentFromCorpus
        } else {
            Stage::UnknownCoverage
        };
        return Ok(Diagnosis { stage, coverage });
    }
    if !trusted(PipelineStage::Indexing) {
        return Ok(Diagnosis {
            stage: Stage::StageError(PipelineStage::Indexing),
            coverage,
        });
    }
    if !input.indexed.contains(t) {
        return Ok(Diagnosis {
            stage: Stage::StoredNotIndexed,
            coverage,
        });
    }
    if !trusted(PipelineStage::CandidateGeneration) {
        return Ok(Diagnosis {
            stage: Stage::StageError(PipelineStage::CandidateGeneration),
            coverage,
        });
    }
    let position = input.candidates.iter().position(|c| c == t);
    if position.is_none() {
        return Ok(Diagnosis {
            stage: Stage::IndexedNotCandidate,
            coverage,
        });
    }
    if !trusted(PipelineStage::Serving) {
        return Ok(Diagnosis {
            stage: Stage::StageError(PipelineStage::Serving),
            coverage,
        });
    }
    if position.unwrap() >= input.served_cutoff {
        return Ok(Diagnosis {
            stage: Stage::CandidateBelowCutoff,
            coverage,
        });
    }
    if !trusted(PipelineStage::ContextPacking) {
        return Ok(Diagnosis {
            stage: Stage::StageError(PipelineStage::ContextPacking),
            coverage,
        });
    }
    if !input.context_included.contains(t) {
        return Ok(Diagnosis {
            stage: Stage::ServedNotInContext,
            coverage,
        });
    }
    if !trusted(PipelineStage::AnswerSupport) {
        return Ok(Diagnosis {
            stage: Stage::StageError(PipelineStage::AnswerSupport),
            coverage,
        });
    }
    let stage = if input.answer_supported {
        Stage::ServedSupported
    } else {
        Stage::ContextDeliveredUnsupported
    };
    Ok(Diagnosis { stage, coverage })
}

/// A named example fixture: input plus its hand-written expected
/// diagnosis. The table is data, independent of both procedures.
struct NamedFixture {
    name: &'static str,
    input: FixtureInput,
    expected: Diagnosis,
}

fn base_input() -> FixtureInput {
    FixtureInput {
        snapshot_id: "snap-f01".to_string(),
        eligible: BTreeSet::new(),
        coverage_complete: true,
        target: "t1".to_string(),
        target_relevant: true,
        policy_excluded: false,
        indexed: BTreeSet::new(),
        candidates: Vec::new(),
        served_cutoff: 0,
        context_included: BTreeSet::new(),
        answer_supported: false,
        failure: None,
    }
}

fn set(docs: &[&str]) -> BTreeSet<String> {
    docs.iter().map(|d| d.to_string()).collect()
}

fn vec_of(docs: &[&str]) -> Vec<String> {
    docs.iter().map(|d| d.to_string()).collect()
}

/// The ten example fixtures named by the issue, in the issue's
/// order. Expected values are written by hand from the issue text,
/// not derived from either procedure.
fn fixtures() -> Vec<NamedFixture> {
    let mut v = Vec::new();

    // 1. Incomplete coverage: the target was not enumerated, but
    // enumeration is incomplete — whereabouts unknown, never absent.
    let mut i = base_input();
    i.eligible = set(&["d1", "d2"]);
    i.coverage_complete = false;
    v.push(NamedFixture {
        name: "incomplete_coverage",
        input: i,
        expected: Diagnosis {
            stage: Stage::UnknownCoverage,
            coverage: Coverage::Incomplete,
        },
    });

    // 2. Fully enumerated absent: complete enumeration, target not
    // in the corpus — the negative control's world.
    let mut i = base_input();
    i.eligible = set(&["d1", "d2"]);
    v.push(NamedFixture {
        name: "enumerated_absent",
        input: i,
        expected: Diagnosis {
            stage: Stage::AbsentFromCorpus,
            coverage: Coverage::Complete,
        },
    });

    // 3. Stored but unindexed.
    let mut i = base_input();
    i.eligible = set(&["t1", "d1"]);
    i.indexed = set(&["d1"]);
    v.push(NamedFixture {
        name: "stored_unindexed",
        input: i,
        expected: Diagnosis {
            stage: Stage::StoredNotIndexed,
            coverage: Coverage::Complete,
        },
    });

    // 4. Indexed but not a candidate.
    let mut i = base_input();
    i.eligible = set(&["t1", "d1"]);
    i.indexed = set(&["t1", "d1"]);
    i.candidates = vec_of(&["d1"]);
    i.served_cutoff = 1;
    v.push(NamedFixture {
        name: "indexed_not_candidate",
        input: i,
        expected: Diagnosis {
            stage: Stage::IndexedNotCandidate,
            coverage: Coverage::Complete,
        },
    });

    // 5. Candidate below the served cutoff.
    let mut i = base_input();
    i.eligible = set(&["t1", "d1"]);
    i.indexed = set(&["t1", "d1"]);
    i.candidates = vec_of(&["d1", "t1"]);
    i.served_cutoff = 1;
    v.push(NamedFixture {
        name: "below_cutoff",
        input: i,
        expected: Diagnosis {
            stage: Stage::CandidateBelowCutoff,
            coverage: Coverage::Complete,
        },
    });

    // 6. Served but omitted by context packing.
    let mut i = base_input();
    i.eligible = set(&["t1", "d1"]);
    i.indexed = set(&["t1", "d1"]);
    i.candidates = vec_of(&["t1", "d1"]);
    i.served_cutoff = 2;
    i.context_included = set(&["d1"]);
    v.push(NamedFixture {
        name: "served_not_in_context",
        input: i,
        expected: Diagnosis {
            stage: Stage::ServedNotInContext,
            coverage: Coverage::Complete,
        },
    });

    // 7. Context delivered but the answer is unsupported.
    let mut i = base_input();
    i.eligible = set(&["t1"]);
    i.indexed = set(&["t1"]);
    i.candidates = vec_of(&["t1"]);
    i.served_cutoff = 1;
    i.context_included = set(&["t1"]);
    i.answer_supported = false;
    v.push(NamedFixture {
        name: "delivered_unsupported",
        input: i,
        expected: Diagnosis {
            stage: Stage::ContextDeliveredUnsupported,
            coverage: Coverage::Complete,
        },
    });

    // 8. Served and supported — the positive control's world.
    let mut i = base_input();
    i.eligible = set(&["t1"]);
    i.indexed = set(&["t1"]);
    i.candidates = vec_of(&["t1"]);
    i.served_cutoff = 1;
    i.context_included = set(&["t1"]);
    i.answer_supported = true;
    v.push(NamedFixture {
        name: "served_supported",
        input: i,
        expected: Diagnosis {
            stage: Stage::ServedSupported,
            coverage: Coverage::Complete,
        },
    });

    // 9. Stage timeout: candidate generation timed out. The empty
    // candidate list this leaves behind is not a successful empty
    // generation — the verdict is the error, never
    // IndexedNotCandidate.
    let mut i = base_input();
    i.eligible = set(&["t1", "d1"]);
    i.indexed = set(&["t1", "d1"]);
    i.candidates = Vec::new();
    i.failure = Some(StageFailure {
        stage: PipelineStage::CandidateGeneration,
        kind: FailureKind::Timeout,
    });
    v.push(NamedFixture {
        name: "stage_timeout",
        input: i,
        expected: Diagnosis {
            stage: Stage::StageError(PipelineStage::CandidateGeneration),
            coverage: Coverage::Complete,
        },
    });

    // 10. Policy excluded: intentionally excluded, shaped like an
    // ordinary not-found everywhere except the verdict itself,
    // which is never global absence.
    let mut i = base_input();
    i.eligible = set(&["t1", "d1"]);
    i.indexed = set(&["d1"]);
    i.candidates = vec_of(&["d1"]);
    i.served_cutoff = 1;
    i.policy_excluded = true;
    v.push(NamedFixture {
        name: "policy_excluded",
        input: i,
        expected: Diagnosis {
            stage: Stage::PolicyExcluded,
            coverage: Coverage::Complete,
        },
    });

    v
}

/// Oracle: the fixture table. All ten example fixtures from the
/// issue produce exactly their hand-written expected diagnoses,
/// from both the runner and the reference procedure.
#[test]
fn ten_example_fixtures_match_expected() {
    let table = fixtures();
    assert_eq!(table.len(), 10, "the issue names ten example fixtures");
    for f in &table {
        assert_eq!(
            diagnose(&f.input),
            Ok(f.expected),
            "runner disagrees with the fixture table on {}",
            f.name
        );
        assert_eq!(
            reference_diagnose(&f.input),
            Ok(f.expected),
            "reference disagrees with the fixture table on {}",
            f.name
        );
    }
}

/// Oracle: the fixture table, read against the issue's three
/// never-present-as rules. These are the assertions the mutation
/// controls below exist to prove can fail.
#[test]
fn never_present_as_rules_hold_on_fixtures() {
    for f in fixtures() {
        let d = diagnose(&f.input).expect("fixture is well-formed");
        if !f.input.coverage_complete {
            assert_ne!(
                d.stage,
                Stage::AbsentFromCorpus,
                "{}: unknown coverage presented as absence",
                f.name
            );
        }
        if f.input.policy_excluded {
            assert_ne!(
                d.stage,
                Stage::AbsentFromCorpus,
                "{}: policy exclusion presented as absence",
                f.name
            );
        }
        if let Some(fail) = f.input.failure {
            assert_ne!(
                d.stage,
                Stage::IndexedNotCandidate,
                "{}: stage failure {:?} presented as an empty successful candidate set",
                f.name,
                fail
            );
        }
        if d.stage == Stage::ServedNotInContext {
            assert_ne!(
                d.stage,
                Stage::ContextDeliveredUnsupported,
                "{}: context omission presented as an answer-support failure",
                f.name
            );
        }
    }
}

/// Positive control: a world that explicitly includes the target at
/// every stage must diagnose as served and supported — the fixture
/// machinery can say "found", not only "lost".
#[test]
fn positive_control_target_explicitly_included() {
    let f = &fixtures()[7];
    assert_eq!(f.name, "served_supported");
    let d = diagnose(&f.input).expect("well-formed");
    assert_eq!(d.stage, Stage::ServedSupported);
    assert!(!d.is_miss(f.input.target_relevant));
}

/// Negative control: absence may be asserted only under complete
/// enumeration. The same world with incomplete coverage flips the
/// verdict to unknown — fixtures 1 and 2 are that pair, and this
/// test pins the dependency on the coverage flag alone.
#[test]
fn negative_control_absence_requires_complete_enumeration() {
    let table = fixtures();
    let absent = &table[1];
    assert_eq!(absent.name, "enumerated_absent");
    assert_eq!(
        diagnose(&absent.input).expect("well-formed").stage,
        Stage::AbsentFromCorpus
    );
    let mut incomplete = absent.input.clone();
    incomplete.coverage_complete = false;
    let d = diagnose(&incomplete).expect("well-formed");
    assert_eq!(d.stage, Stage::UnknownCoverage);
    assert_eq!(d.coverage, Coverage::Incomplete);
    assert!(!d.is_miss(incomplete.target_relevant));
}

/// A deterministic xorshift64* generator: the generated corpora are
/// identical on every run and every machine (deterministic replay
/// is part of the acceptance, so no entropy source is used).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn coin(&mut self) -> bool {
        self.next() % 2 == 0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % (n.max(1) as u64)) as usize
    }
}

/// Build one random closed world over a six-document universe.
/// Construction guarantees structural validity (subsets are drawn
/// from their supersets), so every generated world is well-formed.
fn generated_world(rng: &mut Rng) -> FixtureInput {
    let universe: Vec<String> = (0..6).map(|i| format!("d{i}")).collect();
    let mut input = base_input();
    input.snapshot_id = "snap-generated".to_string();
    input.target = universe[rng.below(universe.len())].clone();
    input.coverage_complete = rng.coin();
    input.target_relevant = rng.coin();
    input.policy_excluded = rng.below(8) == 0;
    for d in &universe {
        if rng.coin() {
            input.eligible.insert(d.clone());
        }
    }
    for d in input.eligible.clone() {
        if rng.coin() {
            input.indexed.insert(d);
        }
    }
    let mut cands: Vec<String> = input
        .indexed
        .iter()
        .filter(|_| rng.coin())
        .cloned()
        .collect();
    // Fisher–Yates shuffle driven by the seeded generator.
    for i in (1..cands.len()).rev() {
        let j = rng.below(i + 1);
        cands.swap(i, j);
    }
    input.candidates = cands;
    input.served_cutoff = rng.below(input.candidates.len() + 1);
    for d in input.served().to_vec() {
        if rng.coin() {
            input.context_included.insert(d);
        }
    }
    input.answer_supported = rng.coin();
    input.failure = if rng.below(4) == 0 {
        Some(StageFailure {
            stage: PipelineStage::ALL[rng.below(6)],
            kind: if rng.coin() {
                FailureKind::Timeout
            } else {
                FailureKind::Error
            },
        })
    } else {
        None
    };
    input
}

/// Oracle: the exhaustive-set reference. On 4,000 generated finite
/// corpora the runner's (stage, coverage) tuple equals the
/// reference procedure's, and the never-present-as invariants hold
/// structurally on every generated world.
#[test]
fn runner_matches_reference_on_generated_corpora() {
    let mut rng = Rng(0x8525_6020_F01);
    for case in 0..4000 {
        let input = generated_world(&mut rng);
        let got = diagnose(&input).expect("generated worlds are well-formed");
        let want = reference_diagnose(&input).expect("generated worlds are well-formed");
        assert_eq!(got, want, "case {case}: runner != reference on {input:?}");
        if got.stage == Stage::AbsentFromCorpus {
            assert_eq!(
                got.coverage,
                Coverage::Complete,
                "case {case}: absence without complete coverage"
            );
            assert!(
                !input.policy_excluded,
                "case {case}: policy exclusion presented as absence"
            );
        }
        if let Some(f) = input.failure {
            if f.stage == PipelineStage::Enumeration && !input.policy_excluded {
                assert_eq!(
                    got.stage,
                    Stage::StageError(PipelineStage::Enumeration),
                    "case {case}: enumeration failure not surfaced"
                );
            }
        }
    }
}

/// Oracle: the exhaustive-set reference over the abstract fate
/// space. Every structurally valid combination of the target's
/// per-stage fate is constructed concretely and checked — this is
/// the exhaustive enumeration the generated corpora sample from.
#[test]
fn runner_matches_reference_on_exhaustive_fate_space() {
    // Candidate position of the target: not a candidate, candidate
    // above the cutoff (served), candidate below the cutoff.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Pos {
        None,
        Above,
        Below,
    }
    let mut checked = 0usize;
    for in_eligible in [false, true] {
        for coverage_complete in [false, true] {
            for in_indexed in [false, true] {
                for pos in [Pos::None, Pos::Above, Pos::Below] {
                    for in_context in [false, true] {
                        for supported in [false, true] {
                            for policy in [false, true] {
                                for failure in
                                    [None].into_iter().chain(PipelineStage::ALL.map(Some))
                                {
                                    // Structural validity of the fate
                                    // combination itself.
                                    if in_indexed && !in_eligible {
                                        continue;
                                    }
                                    if pos != Pos::None && !in_indexed {
                                        continue;
                                    }
                                    if in_context && pos != Pos::Above {
                                        continue;
                                    }
                                    let mut input = base_input();
                                    input.coverage_complete = coverage_complete;
                                    input.policy_excluded = policy;
                                    input.answer_supported = supported;
                                    input.failure = failure.map(|stage| StageFailure {
                                        stage,
                                        kind: FailureKind::Error,
                                    });
                                    // Filler "a" is always indexed and
                                    // a candidate, for realism.
                                    input.eligible = set(&["a"]);
                                    input.indexed = set(&["a"]);
                                    if in_eligible {
                                        input.eligible.insert("t1".to_string());
                                    }
                                    if in_indexed {
                                        input.indexed.insert("t1".to_string());
                                    }
                                    input.candidates = match pos {
                                        Pos::None => vec_of(&["a"]),
                                        Pos::Above => vec_of(&["t1", "a"]),
                                        Pos::Below => vec_of(&["a", "t1"]),
                                    };
                                    input.served_cutoff = match pos {
                                        Pos::Above => 2,
                                        _ => 1,
                                    };
                                    if in_context {
                                        input.context_included = set(&["t1"]);
                                    }
                                    let got = diagnose(&input)
                                        .expect("fate-space worlds are well-formed");
                                    let want = reference_diagnose(&input)
                                        .expect("fate-space worlds are well-formed");
                                    assert_eq!(
                                        got, want,
                                        "fate space: eligible={in_eligible} coverage={coverage_complete} indexed={in_indexed} pos={pos:?} context={in_context} supported={supported} policy={policy} failure={failure:?}"
                                    );
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // The valid fate space is exactly 336 combinations at the time
    // of writing (2^6 flag fates x 3 positions x 7 failure states,
    // minus the structurally invalid combinations filtered above).
    assert!(
        checked > 300,
        "the exhaustive fate space must be substantial, checked {checked}"
    );
}

/// Mutation controls: each mutant encodes one deliberately wrong
/// outcome the issue forbids. The fixture assertions must detect
/// every mutant — a mutant that passes the whole table would prove
/// the assertions cannot fail, which is itself a test failure here.
#[test]
fn mutation_controls_detect_wrong_outcomes() {
    let table = fixtures();
    let mutants: [(&str, Rules, &str); 4] = [
        (
            "error_is_empty_candidates",
            Rules {
                error_is_empty_candidates: true,
                ..Rules::SOUND
            },
            "stage_timeout",
        ),
        (
            "unknown_is_absent",
            Rules {
                unknown_is_absent: true,
                ..Rules::SOUND
            },
            "incomplete_coverage",
        ),
        (
            "context_omission_is_support_failure",
            Rules {
                context_omission_is_support_failure: true,
                ..Rules::SOUND
            },
            "served_not_in_context",
        ),
        (
            "policy_is_absent",
            Rules {
                policy_is_absent: true,
                ..Rules::SOUND
            },
            "policy_excluded",
        ),
    ];
    for (mutant_name, rules, target_fixture) in mutants {
        let mut detected = false;
        for f in &table {
            let got = diagnose_with(&f.input, rules).expect("fixture is well-formed");
            if got != f.expected {
                detected = true;
                assert_eq!(
                    f.name, target_fixture,
                    "mutant {mutant_name} should be caught by {target_fixture}, not by {}",
                    f.name
                );
            }
        }
        assert!(
            detected,
            "mutant {mutant_name} passed the whole fixture table — the assertions cannot detect it"
        );
    }
}

/// Deterministic replay: the same worlds diagnose identically on
/// repeated runs, under reconstruction with different set-insertion
/// orders, and the ten fixtures reproduce a fixed golden transcript.
#[test]
fn deterministic_replay() {
    let golden = "incomplete_coverage:UnknownCoverage/Incomplete;\
        enumerated_absent:AbsentFromCorpus/Complete;\
        stored_unindexed:StoredNotIndexed/Complete;\
        indexed_not_candidate:IndexedNotCandidate/Complete;\
        below_cutoff:CandidateBelowCutoff/Complete;\
        served_not_in_context:ServedNotInContext/Complete;\
        delivered_unsupported:ContextDeliveredUnsupported/Complete;\
        served_supported:ServedSupported/Complete;\
        stage_timeout:StageError(CandidateGeneration)/Complete;\
        policy_excluded:PolicyExcluded/Complete";
    let transcript = |table: &[NamedFixture]| {
        table
            .iter()
            .map(|f| {
                let d = diagnose(&f.input).expect("well-formed");
                format!("{}:{:?}/{:?}", f.name, d.stage, d.coverage)
            })
            .collect::<Vec<_>>()
            .join(";")
    };
    let table = fixtures();
    assert_eq!(transcript(&table), golden);
    // Replay: a second, independent pass over freshly built
    // fixtures reproduces the same transcript byte for byte.
    assert_eq!(transcript(&fixtures()), golden);

    // Generated worlds replay identically, and a world rebuilt with
    // its sets inserted in reverse order diagnoses identically
    // (the diagnosis is a function of the sets, not their history).
    let mut rng = Rng(0xF01D);
    for _ in 0..500 {
        let input = generated_world(&mut rng);
        let first = diagnose(&input).expect("well-formed");
        assert_eq!(diagnose(&input).expect("well-formed"), first);
        let mut rebuilt = input.clone();
        rebuilt.eligible = input.eligible.iter().rev().cloned().collect();
        rebuilt.indexed = input.indexed.iter().rev().cloned().collect();
        rebuilt.context_included = input.context_included.iter().rev().cloned().collect();
        assert_eq!(diagnose(&rebuilt).expect("well-formed"), first);
    }
}

/// Malformed-input checks: every structural violation of the
/// closed world is rejected by both procedures, with the same
/// reason — never silently diagnosed.
#[test]
fn malformed_inputs_are_rejected() {
    let mut cases: Vec<(FixtureInput, Malformed)> = Vec::new();

    let mut i = base_input();
    i.snapshot_id = String::new();
    cases.push((i, Malformed("snapshot_id is empty")));

    let mut i = base_input();
    i.eligible = set(&["d1"]);
    i.indexed = set(&["t1"]);
    cases.push((i, Malformed("indexed is not a subset of eligible")));

    let mut i = base_input();
    i.eligible = set(&["t1"]);
    i.indexed = set(&["t1"]);
    i.candidates = vec_of(&["t1", "t1"]);
    cases.push((i, Malformed("duplicate candidate")));

    let mut i = base_input();
    i.eligible = set(&["t1", "d1"]);
    i.indexed = set(&["d1"]);
    i.candidates = vec_of(&["t1"]);
    cases.push((i, Malformed("candidate not in indexed")));

    let mut i = base_input();
    i.eligible = set(&["t1"]);
    i.indexed = set(&["t1"]);
    i.candidates = vec_of(&["t1"]);
    i.served_cutoff = 2;
    cases.push((i, Malformed("served_cutoff beyond candidate list")));

    let mut i = base_input();
    i.eligible = set(&["t1", "d1"]);
    i.indexed = set(&["t1", "d1"]);
    i.candidates = vec_of(&["t1", "d1"]);
    i.served_cutoff = 1;
    i.context_included = set(&["d1"]);
    cases.push((i, Malformed("context includes an unserved document")));

    assert_eq!(cases.len(), 6);
    for (input, want) in &cases {
        assert_eq!(diagnose(input), Err(*want), "runner accepted {input:?}");
        assert_eq!(
            reference_diagnose(input),
            Err(*want),
            "reference accepted {input:?}"
        );
    }
}
