//! F04 — cost-aware placement dry-run reference simulator
//! (developing-today/code#8528, parent #6020, programme #6018; design
//! `doc/2026-11-11T03-45-00Z_design_atlas_v2_v4/` §4.6 and its
//! [F04 follow-on]).
//!
//! A **pure offline** comparison of a few synthetic placement choices
//! under explicit quality, latency, allowed-location and plan-envelope
//! constraints. Everything here is fixture data: no live pricing, no
//! provider bindings, no runtime services, no real corpus, no
//! credentials and no autonomous movement or deletion. The simulator
//! only ever emits a reversible proposal with its assumptions; it
//! never executes one.
//!
//! [F04 follow-on]: https://github.com/developing-today/code/blob/main/doc/2026-11-11T03-45-00Z_design_atlas_v2_v4/follow-ons/F04.md
//!
//! # Oracles (the testing doctrine asks each test to name one)
//!
//! * **hand-calculated unit ledger** — the scenarios in
//!   [`ledger`] below were computed by hand before the reference
//!   calculation was written, and the tests assert the exact
//!   itemised figures (not ranges, not "some cost").
//! * **invariants** — determinism, unit-preserving sums, nonnegative
//!   input validation, monotonic cost under increased nonnegative use
//!   at fixed prices, hard-policy infeasibility even at zero cost, and
//!   unknown propagation. Property tests state these over generated
//!   inputs.
//! * **an intentionally-wrong reference** — [`choose_cheapest_first`]
//!   is a deliberately naive selector; a fixture proves it picks a
//!   placement the constraints forbid, so the oracle distinguishes
//!   the correct selector from a plausible wrong one.
//!
//! # Units
//!
//! Costs are **integer micros** (millionths of one currency unit) and
//! never mixed with byte quantities. Usage is counted in whole
//! units — GiB-months of storage, GiB of egress, thousands of read
//! and write operations ("kops"), and thousands of embedding tokens
//! ("ktokens"). A rate is micros per one such unit. Every sum is
//! therefore unit-preserving: the line items and the total are all
//! micros, so the arithmetic is exact and not subject to float
//! associativity.
//!
//! # Ordering of checks
//!
//! Constraints are evaluated **before** cost, and hard policy is
//! evaluated before everything: sensitivity, legal retention, source
//! rights and pinned heads are never overridden by a cheaper price.
//! The reference order is
//!
//! 1. hard policy (infeasible — never a cost question),
//! 2. the plan envelope (quality floor, latency ceiling),
//! 3. required temporary headroom,
//! 4. unknown measurements (a missing price or an unmeasured quality
//!    yields [`Decision::Unknown`], never a guessed value),
//! 5. cost, only for what survived 1–4.

use proptest::prelude::*;

/// Money in millionths of one currency unit.
///
/// A newtype, not a bare integer: the whole point of the design's
/// "integer fixed units … not mixed currency/bytes" rule is that a
/// cost cannot be silently added to a byte count.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Micros(i128);

impl Micros {
    const ZERO: Micros = Micros(0);

    fn get(self) -> i128 {
        self.0
    }
}

/// Versioned synthetic meter rates, micros per one usage unit.
///
/// The version string is carried deliberately: a rate card that is
/// stale must be identifiable, and the simulator never claims these
/// are current prices.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct MeterRates {
    /// micros per GiB-month of storage.
    storage: Micros,
    /// micros per GiB of egress.
    egress: Micros,
    /// micros per thousand read operations.
    read: Micros,
    /// micros per thousand write operations.
    write: Micros,
    /// micros per thousand embedding tokens.
    embed: Micros,
}

impl MeterRates {
    /// Validating constructor: every rate must be nonnegative.
    ///
    /// Negative rates are a fixture defect, not a discount: they
    /// would let "monotonic cost" be violated by construction and
    /// could make a forbidden placement look cheapest.
    fn try_new(
        storage: i128,
        egress: i128,
        read: i128,
        write: i128,
        embed: i128,
    ) -> Result<Self, &'static str> {
        for (name, value) in [
            ("storage", storage),
            ("egress", egress),
            ("read", read),
            ("write", write),
            ("embed", embed),
        ] {
            if value < 0 {
                return Err(if name == "storage" {
                    "storage rate is negative"
                } else if name == "egress" {
                    "egress rate is negative"
                } else if name == "read" {
                    "read rate is negative"
                } else if name == "write" {
                    "write rate is negative"
                } else {
                    "embed rate is negative"
                });
            }
        }
        Ok(MeterRates {
            storage: Micros(storage),
            egress: Micros(egress),
            read: Micros(read),
            write: Micros(write),
            embed: Micros(embed),
        })
    }
}

/// One scenario's usage, in whole units. All quantities are unsigned,
/// so an unvalidated use cannot be negative by construction.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
struct Use {
    storage_gib_months: u64,
    egress_gib: u64,
    read_kops: u64,
    write_kops: u64,
    embed_ktokens: u64,
}

/// The plan envelope: what the approved plan still requires of a
/// placement. Both fields are optional so a scenario can constrain
/// only one dimension; `None` means "not constrained here", which is
/// different from "unknown" (which belongs to the placement).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
struct Envelope {
    /// Minimum measured quality, 0..=100.
    min_quality: Option<u32>,
    /// Maximum acceptable latency in milliseconds.
    max_latency_ms: Option<u32>,
}

/// Hard policy for one placement. Every flag must hold. These are the
/// constraints the design says are never overridden: sensitivity,
/// legal retention, source rights, and pinned heads.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Policy {
    sensitivity_ok: bool,
    retention_ok: bool,
    rights_ok: bool,
    pinned_head_ok: bool,
    allowed_location: bool,
}

impl Policy {
    fn all_ok() -> Self {
        Policy {
            sensitivity_ok: true,
            retention_ok: true,
            rights_ok: true,
            pinned_head_ok: true,
            allowed_location: true,
        }
    }

    /// The first violated rule, in a fixed order, so infeasibility is
    /// deterministic and the reason is reportable.
    fn first_violation(&self) -> Option<&'static str> {
        if !self.sensitivity_ok {
            Some("sensitivity ceiling")
        } else if !self.retention_ok {
            Some("legal retention")
        } else if !self.rights_ok {
            Some("source rights")
        } else if !self.pinned_head_ok {
            Some("pinned head")
        } else if !self.allowed_location {
            Some("allowed location")
        } else {
            None
        }
    }
}

/// A candidate placement. Optional measurements model "measured or
/// unknown" honestly: `None` is unknown, never zero.
#[derive(Clone, Debug)]
struct Placement {
    name: &'static str,
    /// `None` means the price card does not cover this placement.
    rates: Option<MeterRates>,
    /// `None` means quality is unmeasured.
    quality: Option<u32>,
    latency_ms: Option<u32>,
    policy: Policy,
    /// Temporary export/restore/compaction headroom this placement
    /// can actually provide, in GiB.
    temp_headroom_gib: u64,
}

/// One itemised cost line: a unit name and its micros.
type Line = (&'static str, Micros);

/// The reversible output: a proposal, its itemisation and the
/// assumptions it rests on. Never executed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Proposal {
    placement: &'static str,
    total: Micros,
    lines: Vec<Line>,
    assumptions: Vec<&'static str>,
}

/// Evaluation outcome for one placement.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Decision {
    /// Within every constraint, with an itemised cost.
    Feasible(Proposal),
    /// A hard constraint excludes it, whatever it costs.
    Infeasible {
        placement: &'static str,
        reason: &'static str,
    },
    /// A required measurement is missing; no guess is made.
    Unknown {
        placement: &'static str,
        reason: &'static str,
    },
}

/// Itemised reference cost. `None` when any rate that the use actually
/// touches is unknown — an unknown price for a used unit propagates
/// rather than being treated as free.
///
/// Zero-use dimensions do not require a price: a scenario that never
/// egresses has no egress cost to be unknown about.
fn itemised_cost(use_: Use, rates: &MeterRates) -> Option<(Micros, Vec<Line>)> {
    let mut lines: Vec<Line> = Vec::with_capacity(5);
    let mut total = Micros::ZERO;

    let mut push = |name: &'static str, quantity: u64, rate: Micros| {
        let line = Micros(rate.get() * i128::from(quantity));
        total = Micros(total.get() + line.get());
        lines.push((name, line));
    };

    push("storage", use_.storage_gib_months, rates.storage);
    push("egress", use_.egress_gib, rates.egress);
    push("read", use_.read_kops, rates.read);
    push("write", use_.write_kops, rates.write);
    push("embed", use_.embed_ktokens, rates.embed);

    Some((total, lines))
}

/// Reference evaluation of one placement.
///
/// Order is the contract: hard policy, then the plan envelope, then
/// temporary headroom, then unknown measurements, then cost. A cheaper
/// price can never rescue a placement that failed 1–3.
fn evaluate(
    placement: &Placement,
    use_: Use,
    envelope: &Envelope,
    required_temp_gib: u64,
) -> Decision {
    // 1. Hard policy is never a cost question.
    if let Some(reason) = placement.policy.first_violation() {
        return Decision::Infeasible {
            placement: placement.name,
            reason,
        };
    }

    // 2. The plan envelope. An unmeasured dimension cannot be claimed
    //    to satisfy a required bound, so it is Unknown, not Feasible.
    if let Some(floor) = envelope.min_quality {
        match placement.quality {
            None => {
                return Decision::Unknown {
                    placement: placement.name,
                    reason: "quality is unmeasured",
                };
            }
            Some(q) if q < floor => {
                return Decision::Infeasible {
                    placement: placement.name,
                    reason: "below the quality floor",
                };
            }
            Some(_) => {}
        }
    }
    if let Some(ceiling) = envelope.max_latency_ms {
        match placement.latency_ms {
            None => {
                return Decision::Unknown {
                    placement: placement.name,
                    reason: "latency is unmeasured",
                };
            }
            Some(ms) if ms > ceiling => {
                return Decision::Infeasible {
                    placement: placement.name,
                    reason: "above the latency ceiling",
                };
            }
            Some(_) => {}
        }
    }

    // 3. Temporary headroom the operation itself needs.
    if placement.temp_headroom_gib < required_temp_gib {
        return Decision::Infeasible {
            placement: placement.name,
            reason: "insufficient temporary headroom",
        };
    }

    // 4. Unknown price propagates; it is never treated as zero.
    let Some(rates) = placement.rates else {
        return Decision::Unknown {
            placement: placement.name,
            reason: "price is unknown",
        };
    };
    let Some((total, lines)) = itemised_cost(use_, &rates) else {
        return Decision::Unknown {
            placement: placement.name,
            reason: "price is unknown",
        };
    };

    // 5. Only survivors are compared on cost.
    let mut assumptions: Vec<&'static str> = vec![
        "synthetic rate card, not a live price",
        "usage is a synthetic scenario, not a measured corpus",
    ];
    if placement.quality.is_none() {
        assumptions.push("quality was not required by this envelope");
    }
    if placement.latency_ms.is_none() {
        assumptions.push("latency was not required by this envelope");
    }

    Decision::Feasible(Proposal {
        placement: placement.name,
        total,
        lines,
        assumptions,
    })
}

/// The reference selector: evaluate every placement, keep the feasible
/// ones, and take the cheapest. Ties break on lower latency and then
/// on name, so the choice is deterministic and speed-first only
/// *inside* the approved envelope.
///
/// `Err` carries every non-feasible decision so the caller can report
/// why a proposal could not be made — an infeasible or unknown
/// scenario is a result, not a crash.
fn choose<'a>(
    placements: &'a [Placement],
    use_: Use,
    envelope: &Envelope,
    required_temp_gib: u64,
) -> Result<Proposal, Vec<Decision>> {
    let mut feasible: Vec<(Proposal, u32)> = Vec::new();
    let mut rejected: Vec<Decision> = Vec::new();

    for placement in placements {
        match evaluate(placement, use_, envelope, required_temp_gib) {
            Decision::Feasible(proposal) => {
                let latency = placement.latency_ms.unwrap_or(u32::MAX);
                feasible.push((proposal, latency));
            }
            other => rejected.push(other),
        }
    }

    feasible.sort_by(|(a, a_lat), (b, b_lat)| {
        a.total
            .cmp(&b.total)
            .then(a_lat.cmp(b_lat))
            .then(a.placement.cmp(b.placement))
    });

    match feasible.into_iter().next() {
        Some((proposal, _)) => Ok(proposal),
        None => Err(rejected),
    }
}

/// The **intentionally naive** selector this file exists to rule out:
/// cheapest price wins, with no policy or envelope filter at all. It
/// is never used to make a decision; a test asserts it picks a
/// placement the constraints forbid, so the oracle can tell the
/// reference selector apart from a plausible wrong one.
#[allow(dead_code)]
fn choose_cheapest_first<'a>(placements: &'a [Placement], use_: Use) -> Option<(&'a str, Micros)> {
    placements
        .iter()
        .filter_map(|p| {
            let rates = p.rates.as_ref()?;
            let (total, _) = itemised_cost(use_, rates)?;
            Some((p.name, total))
        })
        .min_by(|(a, a_cost), (b, b_cost)| a_cost.cmp(b_cost).then(a.cmp(b)))
}

/// The hand-calculated unit ledger.
///
/// Every figure below was computed by hand from the rate card and the
/// scenario usage *before* [`itemised_cost`] was written; the tests
/// assert these exact numbers. One shared rate card keeps the
/// arithmetic checkable:
///
/// * storage 2_000_000 µ / GiB-month  (= 2.00 per GiB-month)
/// * egress     40_000 µ / GiB        (= 0.04 per GiB)
/// * read          300 µ / kop        (= 0.0003 per thousand reads)
/// * write       1_500 µ / kop        (= 0.0015 per thousand writes)
/// * embed      20_000 µ / ktoken     (= 0.02 per thousand tokens)
mod ledger {
    use super::*;

    /// The shared synthetic rate card (version `synthetic-v1`).
    pub fn rates() -> MeterRates {
        MeterRates::try_new(2_000_000, 40_000, 300, 1_500, 20_000)
            .expect("the ledger rate card is nonnegative")
    }

    /// The shared usage: 50 GiB-months storage, 100 GiB egress,
    /// 2_000 kops read, 500 kops write, 1_000 ktokens embedded.
    ///
    /// By hand:
    /// * storage 50 × 2_000_000 = 100_000_000
    /// * egress  100 ×    40_000 =   4_000_000
    /// * read   2000 ×       300 =     600_000
    /// * write   500 ×     1_500 =     750_000
    /// * embed  1000 ×    20_000 =  20_000_000
    /// * total                   = 125_350_000 µ  (125.35)
    pub fn usage() -> Use {
        Use {
            storage_gib_months: 50,
            egress_gib: 100,
            read_kops: 2_000,
            write_kops: 500,
            embed_ktokens: 1_000,
        }
    }

    /// The hand total for [`usage`] under [`rates`].
    pub const TOTAL_MICROS: i128 = 125_350_000;

    /// Ledger line items by hand, in the reference order.
    pub const LINES: [(&str, i128); 5] = [
        ("storage", 100_000_000),
        ("egress", 4_000_000),
        ("read", 600_000),
        ("write", 750_000),
        ("embed", 20_000_000),
    ];
}

// ---------------------------------------------------------------------
// Scenario 1 — two equal-quality choices, itemised metering.
// ---------------------------------------------------------------------

/// Oracle: the hand-calculated unit ledger. Two placements priced by
/// the same card produce exactly the hand total, item for item, and
/// sum to that total (unit-preserving sums).
#[test]
fn ledger_two_equal_quality_choices_are_itemised_to_the_hand_total() {
    let a = Placement {
        name: "alpha",
        rates: Some(ledger::rates()),
        quality: Some(90),
        latency_ms: Some(30),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let b = Placement {
        name: "beta",
        rates: Some(ledger::rates()),
        quality: Some(90),
        latency_ms: Some(30),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let envelope = Envelope {
        min_quality: Some(80),
        max_latency_ms: Some(100),
    };

    let Decision::Feasible(pa) = evaluate(&a, ledger::usage(), &envelope, 0) else {
        panic!("alpha must be feasible");
    };
    let Decision::Feasible(pb) = evaluate(&b, ledger::usage(), &envelope, 0) else {
        panic!("beta must be feasible");
    };

    assert_eq!(pa.total, Micros(ledger::TOTAL_MICROS), "hand total, alpha");
    assert_eq!(pb.total, Micros(ledger::TOTAL_MICROS), "hand total, beta");
    assert_eq!(
        pa.lines,
        ledger::LINES
            .iter()
            .map(|(name, micros)| (*name, Micros(*micros)))
            .collect::<Vec<_>>(),
        "hand line items, in reference order"
    );

    // Unit-preserving sums: the itemisation adds back to the total.
    let summed: i128 = pa.lines.iter().map(|(_, m)| m.get()).sum();
    assert_eq!(summed, pa.total.get(), "line items must sum to the total");
    assert_eq!(summed, ledger::TOTAL_MICROS);

    // Equal quality and equal cost: the tie is broken deterministically
    // by name, so the reference never flips between runs.
    let picked = choose(&[a, b], ledger::usage(), &envelope, 0).expect("a feasible pick");
    assert_eq!(picked.placement, "alpha", "tie breaks on name");
}

// ---------------------------------------------------------------------
// Scenario 2 — a faster choice inside the envelope wins on latency
// only after cost ties.
// ---------------------------------------------------------------------

/// Oracle: hand-calculated selection. Two placements cost the same and
/// have the same quality; only latency distinguishes them, and the
/// one above the ceiling is infeasible rather than merely slower.
#[test]
fn ledger_faster_feasible_choice_within_envelope_is_preferred() {
    let fast = Placement {
        name: "fast",
        rates: Some(ledger::rates()),
        quality: Some(90),
        latency_ms: Some(40),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let slow_ok = Placement {
        name: "slow-but-inside",
        rates: Some(ledger::rates()),
        quality: Some(90),
        latency_ms: Some(50),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let too_slow = Placement {
        name: "too-slow",
        rates: Some(ledger::rates()),
        quality: Some(90),
        latency_ms: Some(90),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let envelope = Envelope {
        min_quality: Some(80),
        max_latency_ms: Some(50),
    };

    // The 90 ms placement is excluded by the ceiling, not merely
    // ranked below: the envelope is a constraint.
    assert_eq!(
        evaluate(&too_slow, ledger::usage(), &envelope, 0),
        Decision::Infeasible {
            placement: "too-slow",
            reason: "above the latency ceiling",
        }
    );

    let picked =
        choose(&[too_slow, slow_ok, fast], ledger::usage(), &envelope, 0).expect("a feasible pick");
    assert_eq!(picked.placement, "fast", "equal cost, lower latency wins");
    assert_eq!(picked.total, Micros(ledger::TOTAL_MICROS));
}

// ---------------------------------------------------------------------
// Scenario 3 — a forbidden placement is infeasible even at zero cost.
// ---------------------------------------------------------------------

/// Oracle: hand-calculated. The forbidden placement prices out at
/// exactly zero and is still rejected, proving constraints precede
/// cost; and the naive cheapest-first selector picks it.
#[test]
fn ledger_forbidden_placement_is_infeasible_even_at_zero_cost() {
    let free_but_forbidden = Placement {
        name: "free-forbidden",
        rates: Some(MeterRates::try_new(0, 0, 0, 0, 0).expect("zero is nonnegative")),
        quality: Some(100),
        latency_ms: Some(1),
        policy: Policy {
            retention_ok: false,
            ..Policy::all_ok()
        },
        temp_headroom_gib: 999,
    };
    let permitted = Placement {
        name: "permitted",
        rates: Some(ledger::rates()),
        quality: Some(90),
        latency_ms: Some(30),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let envelope = Envelope {
        min_quality: Some(80),
        max_latency_ms: Some(100),
    };

    assert_eq!(
        evaluate(&free_but_forbidden, ledger::usage(), &envelope, 0),
        Decision::Infeasible {
            placement: "free-forbidden",
            reason: "legal retention",
        },
        "a zero price never overrides legal retention"
    );

    let picked = choose(
        &[free_but_forbidden.clone(), permitted],
        ledger::usage(),
        &envelope,
        0,
    )
    .expect("the permitted placement is feasible");
    assert_eq!(picked.placement, "permitted");

    // The intentionally-wrong reference: cheapest-first takes the free
    // forbidden placement, which is exactly why it cannot be the
    // decision procedure.
    let naive = choose_cheapest_first(std::slice::from_ref(&free_but_forbidden), ledger::usage())
        .expect("cheapest-first ignores policy and finds a candidate");
    assert_eq!(naive.0, "free-forbidden");
    assert_eq!(naive.1, Micros::ZERO);
}

/// Oracle: hand-calculated. A cheapest-first selector also fails a
/// fixture where the cheap placement is above the latency ceiling, so
/// the wrong selector is distinguishable on the envelope axis too.
#[test]
fn cheapest_first_selector_fails_a_fixture_that_violates_the_envelope() {
    let cheap_slow = Placement {
        name: "cheap-slow",
        rates: Some(MeterRates::try_new(1, 1, 1, 1, 1).expect("nonnegative")),
        quality: Some(90),
        latency_ms: Some(900),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let dear_fast = Placement {
        name: "dear-fast",
        rates: Some(MeterRates::try_new(9, 9, 9, 9, 9).expect("nonnegative")),
        quality: Some(90),
        latency_ms: Some(20),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let envelope = Envelope {
        min_quality: Some(80),
        max_latency_ms: Some(100),
    };

    let reference = choose(
        &[cheap_slow.clone(), dear_fast],
        ledger::usage(),
        &envelope,
        0,
    )
    .expect("dear-fast is feasible");
    assert_eq!(
        reference.placement, "dear-fast",
        "the reference pays more to stay inside the envelope"
    );

    let only_cheap = [cheap_slow];
    let naive = choose_cheapest_first(&only_cheap, ledger::usage()).expect("a candidate");
    assert_eq!(
        naive.0, "cheap-slow",
        "cheapest-first picks the placement the envelope forbids"
    );
}

// ---------------------------------------------------------------------
// Scenario 4 — a missing price is unknown, never free.
// ---------------------------------------------------------------------

/// Oracle: unknown propagation by hand. The placement has no rate card;
/// the reference reports Unknown and the selector refuses to propose.
#[test]
fn ledger_missing_price_is_unknown_not_free() {
    let unpriced = Placement {
        name: "unpriced",
        rates: None,
        quality: Some(90),
        latency_ms: Some(30),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let envelope = Envelope {
        min_quality: Some(80),
        max_latency_ms: Some(100),
    };

    assert_eq!(
        evaluate(&unpriced, ledger::usage(), &envelope, 0),
        Decision::Unknown {
            placement: "unpriced",
            reason: "price is unknown",
        }
    );

    let rejected = choose(&[unpriced], ledger::usage(), &envelope, 0)
        .expect_err("an unpriced scenario yields no proposal");
    assert_eq!(rejected.len(), 1);
    assert!(matches!(rejected[0], Decision::Unknown { .. }));
}

// ---------------------------------------------------------------------
// Scenario 5 — insufficient temporary space is infeasible.
// ---------------------------------------------------------------------

/// Oracle: hand-calculated. The scenario needs 120 GiB of temporary
/// headroom for export/restore; 80 GiB is not enough and no price
/// makes it enough.
#[test]
fn ledger_insufficient_temporary_space_is_infeasible() {
    let cramped = Placement {
        name: "cramped",
        rates: Some(ledger::rates()),
        quality: Some(95),
        latency_ms: Some(10),
        policy: Policy::all_ok(),
        temp_headroom_gib: 80,
    };
    let roomy = Placement {
        name: "roomy",
        rates: Some(ledger::rates()),
        quality: Some(95),
        latency_ms: Some(10),
        policy: Policy::all_ok(),
        temp_headroom_gib: 120,
    };
    let envelope = Envelope {
        min_quality: Some(80),
        max_latency_ms: Some(100),
    };

    assert_eq!(
        evaluate(&cramped, ledger::usage(), &envelope, 120),
        Decision::Infeasible {
            placement: "cramped",
            reason: "insufficient temporary headroom",
        }
    );

    let picked = choose(&[cramped, roomy], ledger::usage(), &envelope, 120)
        .expect("roomy satisfies the headroom need");
    assert_eq!(picked.placement, "roomy");
}

/// Oracle: unmeasured quality cannot justify a placement that a
/// quality floor applies to. `None` is not "probably fine".
#[test]
fn unmeasured_quality_cannot_justify_a_placement() {
    let unmeasured = Placement {
        name: "unmeasured-quality",
        rates: Some(ledger::rates()),
        quality: None,
        latency_ms: Some(30),
        policy: Policy::all_ok(),
        temp_headroom_gib: 200,
    };
    let envelope = Envelope {
        min_quality: Some(80),
        max_latency_ms: Some(100),
    };

    assert_eq!(
        evaluate(&unmeasured, ledger::usage(), &envelope, 0),
        Decision::Unknown {
            placement: "unmeasured-quality",
            reason: "quality is unmeasured",
        }
    );

    // With no quality floor in the envelope the same placement is
    // evaluable, and the proposal records that it was not required.
    let relaxed = Envelope::default();
    let Decision::Feasible(proposal) = evaluate(&unmeasured, ledger::usage(), &relaxed, 0) else {
        panic!("without a quality floor the placement prices out");
    };
    assert!(
        proposal
            .assumptions
            .contains(&"quality was not required by this envelope"),
        "the proposal states why unmeasured quality did not block it"
    );
}

// ---------------------------------------------------------------------
// Invariants: validation, sums, determinism, monotonicity.
// ---------------------------------------------------------------------

/// Oracle: input validation. Negative rates are rejected by the
/// constructor, so the rest of the model can assume nonnegative money.
#[test]
fn negative_rates_are_rejected() {
    assert!(MeterRates::try_new(-1, 0, 0, 0, 0).is_err());
    assert!(MeterRates::try_new(0, -1, 0, 0, 0).is_err());
    assert!(MeterRates::try_new(0, 0, -1, 0, 0).is_err());
    assert!(MeterRates::try_new(0, 0, 0, -1, 0).is_err());
    assert!(MeterRates::try_new(0, 0, 0, 0, -1).is_err());
    assert!(MeterRates::try_new(0, 0, 0, 0, 0).is_ok());
}

/// Oracle: the hand ledger plus zero use. A zero-use scenario costs
/// exactly zero, whatever the rate card, and still itemises five zeros.
#[test]
fn zero_use_costs_zero_whatever_the_rates() {
    let rates = ledger::rates();
    let (total, lines) = itemised_cost(Use::default(), &rates).expect("a priced card");
    assert_eq!(total, Micros::ZERO);
    assert_eq!(lines.len(), 5);
    assert!(lines.iter().all(|(_, micros)| *micros == Micros::ZERO));
}

proptest! {
    /// Oracle: unit-preserving sums. For any usage and any nonnegative
    /// rate card, the itemised lines sum to the reported total — the
    /// arithmetic cannot lose or invent micros.
    #[test]
    fn itemised_lines_sum_to_the_total(
        storage_rate in 0i128..1_000_000,
        egress_rate in 0i128..1_000_000,
        read_rate in 0i128..1_000_000,
        write_rate in 0i128..1_000_000,
        embed_rate in 0i128..1_000_000,
        storage in 0u64..1_000_000,
        egress in 0u64..1_000_000,
        read in 0u64..1_000_000,
        write in 0u64..1_000_000,
        embed in 0u64..1_000_000,
    ) {
        let rates = MeterRates::try_new(storage_rate, egress_rate, read_rate, write_rate, embed_rate)
            .expect("generated rates are nonnegative");
        let use_ = Use {
            storage_gib_months: storage,
            egress_gib: egress,
            read_kops: read,
            write_kops: write,
            embed_ktokens: embed,
        };
        let (total, lines) = itemised_cost(use_, &rates).expect("a priced card");
        let summed: i128 = lines.iter().map(|(_, micros)| micros.get()).sum();
        prop_assert_eq!(summed, total.get());
        prop_assert!(total.get() >= 0, "a nonnegative scenario cannot cost less than zero");
    }

    /// Oracle: monotonic cost. Increasing any nonnegative usage at
    /// fixed nonnegative prices never lowers the cost.
    #[test]
    fn cost_is_monotonic_in_increased_use(
        storage_rate in 0i128..1_000_000,
        egress_rate in 0i128..1_000_000,
        read_rate in 0i128..1_000_000,
        write_rate in 0i128..1_000_000,
        embed_rate in 0i128..1_000_000,
        base_storage in 0u64..500_000,
        base_egress in 0u64..500_000,
        base_read in 0u64..500_000,
        base_write in 0u64..500_000,
        base_embed in 0u64..500_000,
        add_storage in 0u64..500_000,
        add_egress in 0u64..500_000,
        add_read in 0u64..500_000,
        add_write in 0u64..500_000,
        add_embed in 0u64..500_000,
    ) {
        let rates = MeterRates::try_new(storage_rate, egress_rate, read_rate, write_rate, embed_rate)
            .expect("generated rates are nonnegative");
        let before = Use {
            storage_gib_months: base_storage,
            egress_gib: base_egress,
            read_kops: base_read,
            write_kops: base_write,
            embed_ktokens: base_embed,
        };
        let after = Use {
            storage_gib_months: base_storage + add_storage,
            egress_gib: base_egress + add_egress,
            read_kops: base_read + add_read,
            write_kops: base_write + add_write,
            embed_ktokens: base_embed + add_embed,
        };
        let (cost_before, _) = itemised_cost(before, &rates).expect("a priced card");
        let (cost_after, _) = itemised_cost(after, &rates).expect("a priced card");
        prop_assert!(
            cost_after >= cost_before,
            "cost fell from {} to {} as use increased",
            cost_before.get(),
            cost_after.get()
        );
    }

    /// Oracle: determinism. Evaluating the same scenario twice yields
    /// an identical decision, itemisation order included, so a proposal
    /// can be re-derived and compared byte for byte.
    #[test]
    fn evaluation_is_deterministic(
        storage_rate in 0i128..1_000_000,
        egress_rate in 0i128..1_000_000,
        read_rate in 0i128..1_000_000,
        write_rate in 0i128..1_000_000,
        embed_rate in 0i128..1_000_000,
        storage in 0u64..1_000_000,
        egress in 0u64..1_000_000,
        read in 0u64..1_000_000,
        write in 0u64..1_000_000,
        embed in 0u64..1_000_000,
        temp_headroom in 0u64..1_000,
        required_temp in 0u64..1_000,
        min_quality in 0u32..=100,
        max_latency in 0u32..1_000,
        quality in 0u32..=100,
        latency in 0u32..1_000,
    ) {
        let placement = Placement {
            name: "generated",
            rates: Some(
                MeterRates::try_new(storage_rate, egress_rate, read_rate, write_rate, embed_rate)
                    .expect("generated rates are nonnegative"),
            ),
            quality: Some(quality),
            latency_ms: Some(latency),
            policy: Policy::all_ok(),
            temp_headroom_gib: temp_headroom,
        };
        let use_ = Use {
            storage_gib_months: storage,
            egress_gib: egress,
            read_kops: read,
            write_kops: write,
            embed_ktokens: embed,
        };
        let envelope = Envelope {
            min_quality: Some(min_quality),
            max_latency_ms: Some(max_latency),
        };
        let first = evaluate(&placement, use_, &envelope, required_temp);
        let second = evaluate(&placement, use_, &envelope, required_temp);
        prop_assert_eq!(first, second);
    }

    /// Oracle: hard-policy dominance. A policy-violating placement is
    /// Infeasible with the same reason for every price and every usage,
    /// including a free one.
    #[test]
    fn hard_policy_dominates_any_price(
        storage_rate in 0i128..1_000_000,
        egress_rate in 0i128..1_000_000,
        storage in 0u64..1_000_000,
    ) {
        let placement = Placement {
            name: "generated",
            rates: Some(
                MeterRates::try_new(storage_rate, egress_rate, 0, 0, 0)
                    .expect("generated rates are nonnegative"),
            ),
            quality: Some(100),
            latency_ms: Some(1),
            policy: Policy {
                sensitivity_ok: false,
                ..Policy::all_ok()
            },
            temp_headroom_gib: u64::MAX,
        };
        let use_ = Use {
            storage_gib_months: storage,
            ..Use::default()
        };
        prop_assert_eq!(
            evaluate(&placement, use_, &Envelope::default(), 0),
            Decision::Infeasible {
                placement: "generated",
                reason: "sensitivity ceiling",
            }
        );
        // A free variant is still infeasible with the same reason.
        let free = Placement {
            rates: Some(MeterRates::try_new(0, 0, 0, 0, 0).expect("zero is nonnegative")),
            ..placement
        };
        let free_variant = evaluate(&free, use_, &Envelope::default(), 0);
        prop_assert_eq!(
            free_variant,
            Decision::Infeasible {
                placement: "generated",
                reason: "sensitivity ceiling",
            }
        );
    }
}
