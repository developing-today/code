//! The #13717 route-form walker's acceptance cases, bound against
//! `fleet-pool` (design #13752 §3.5 step 1: "its acceptance cases
//! become the crate's first oracle tests").
//!
//! The walker is tier 0 of the laptop route-form matrix: the forms
//! T1 (commander/fleet), T2 (http/fleet), and T3 (ssh/fleet) are
//! members under one host group, attempted in order. The walk
//! advances only when a form fails to *serve* — never on a served
//! result, whatever its exit code — and the serving form is named
//! in every result, which here means: the decision log's selection
//! entries name each attempted form, and the last one names the
//! form that served.

use fleet_pool::{
    AttemptKind, Candidate, Classification, MemberState, NodeId, Pool, ServedDisposition,
    Strategy as PoolStrategy,
};
use std::collections::HashSet;

fn form_pool() -> Pool {
    let mut pool = Pool::new();
    pool.add_group("host", None);
    pool.add_member("t1", Some(&NodeId::new("host")));
    pool.add_member("t2", Some(&NodeId::new("host")));
    pool.add_member("t3", Some(&NodeId::new("host")));
    pool
}

fn serve_failure(member: &NodeId, evidence: &str) -> Classification {
    Classification {
        kind: AttemptKind::ServeFailure,
        served_disposition: ServedDisposition::Hold,
        advance: true,
        evidence_scope: member.clone(),
        reset_hint: None,
        evidence: evidence.to_owned(),
    }
}

fn served_failure(member: &NodeId, advance: bool, evidence: &str) -> Classification {
    Classification {
        kind: AttemptKind::ServedFailure,
        served_disposition: ServedDisposition::Hold,
        advance,
        evidence_scope: member.clone(),
        reset_hint: None,
        evidence: evidence.to_owned(),
    }
}

/// What a form does when the walker attempts it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FormResult {
    /// The form served a result (any exit code).
    ServedOk,
    /// The form answered with a failure the classifier judges
    /// terminal for the walk (e.g. a guard refusal, a command that
    /// ran and failed): a served result — the walk stops here.
    ServedTerminal,
    /// The form answered with a failure whose classifier says the
    /// walk may advance.
    ServedAdvance,
    /// Transport/auth/connect/timeout: the form never answered.
    FailedToServe,
}

/// The walker's own order: declaration order over the forms it has
/// not attempted yet this walk. The pool supplies availability
/// (walled forms never become candidates), state, advance
/// verdicts, and the log; the cursor is the walker's.
#[derive(Default)]
struct WalkOrder {
    attempted: HashSet<NodeId>,
}

impl PoolStrategy for WalkOrder {
    fn name(&self) -> &str {
        "walker-ordered"
    }

    fn select(&mut self, candidates: &[Candidate]) -> Option<usize> {
        candidates
            .iter()
            .position(|c| !self.attempted.contains(&c.id))
    }
}

struct WalkOutcome {
    serving: Option<NodeId>,
    attempts: Vec<NodeId>,
}

/// One tier-0 walk over the pool at `now`.
fn walk(pool: &mut Pool, script: &dyn Fn(&NodeId) -> FormResult, now: u64) -> WalkOutcome {
    let mut order = WalkOrder::default();
    let mut attempts = Vec::new();
    loop {
        let Some(form) = pool.select(&mut order, &|_| None, now) else {
            return WalkOutcome {
                serving: None,
                attempts,
            };
        };
        order.attempted.insert(form.clone());
        attempts.push(form.clone());
        match script(&form) {
            FormResult::ServedOk => {
                pool.record_success(&form, now);
                return WalkOutcome {
                    serving: Some(form),
                    attempts,
                };
            }
            FormResult::ServedTerminal => {
                let fx =
                    pool.record_outcome(&form, &served_failure(&form, false, "served result"), now);
                assert!(!fx.advance, "a terminal served result must not advance");
                return WalkOutcome {
                    serving: Some(form),
                    attempts,
                };
            }
            FormResult::ServedAdvance => {
                let fx =
                    pool.record_outcome(&form, &served_failure(&form, true, "served failure"), now);
                assert!(fx.advance, "the classifier said advance");
            }
            FormResult::FailedToServe => {
                let fx = pool.record_outcome(&form, &serve_failure(&form, "failed to serve"), now);
                assert!(fx.advance, "a failure to serve always advances");
            }
        }
    }
}

fn script_for(serving: &str) -> impl Fn(&NodeId) -> FormResult {
    let serving = serving.to_owned();
    move |form: &NodeId| {
        if form.as_str() == serving {
            FormResult::ServedOk
        } else {
            FormResult::FailedToServe
        }
    }
}

#[test]
fn walker_first_form_serves_on_a_healthy_estate() {
    // #13717 acceptance shape: tier 0 serves via T1 and reports it.
    let mut pool = form_pool();
    let out = walk(&mut pool, &script_for("t1"), 0);
    assert_eq!(out.serving, Some(NodeId::new("t1")));
    assert_eq!(out.attempts, vec![NodeId::new("t1")]);
}

#[test]
fn walker_advances_only_when_a_form_fails_to_serve() {
    let mut pool = form_pool();
    let out = walk(&mut pool, &script_for("t2"), 0);
    assert_eq!(out.attempts, vec![NodeId::new("t1"), NodeId::new("t2")]);
    assert_eq!(out.serving, Some(NodeId::new("t2")));
}

#[test]
fn walker_never_advances_on_a_served_result_whatever_its_exit_code() {
    // The remote command ran and exited non-zero: a served result.
    // Tier 0 must not route around it to the next form.
    let mut pool = form_pool();
    let out = walk(&mut pool, &|_| FormResult::ServedTerminal, 0);
    assert_eq!(out.serving, Some(NodeId::new("t1")));
    assert_eq!(out.attempts, vec![NodeId::new("t1")]);
}

#[test]
fn walker_treats_a_guard_refusal_as_a_served_result() {
    // The T2 contract in #13717: the daemon's primary-checkout
    // guard refusal is a *served* result — tier 0 must not route
    // around it. Same mechanics as any terminal served result,
    // bound explicitly because the contract names it.
    let mut pool = form_pool();
    let out = walk(
        &mut pool,
        &|form: &NodeId| {
            if form.as_str() == "t2" {
                FormResult::ServedTerminal
            } else {
                FormResult::FailedToServe
            }
        },
        0,
    );
    assert_eq!(out.attempts, vec![NodeId::new("t1"), NodeId::new("t2")]);
    assert_eq!(out.serving, Some(NodeId::new("t2")));
}

#[test]
fn walker_advances_on_a_served_failure_only_when_its_classifier_says_so() {
    // The other side of the rule: a served failure whose
    // classifier judges the form done advances the walk.
    let mut pool = form_pool();
    let out = walk(
        &mut pool,
        &|form: &NodeId| match form.as_str() {
            "t1" => FormResult::ServedAdvance,
            _ => FormResult::ServedOk,
        },
        0,
    );
    assert_eq!(out.attempts, vec![NodeId::new("t1"), NodeId::new("t2")]);
    assert_eq!(out.serving, Some(NodeId::new("t2")));
}

#[test]
fn walker_exhausts_when_no_form_serves() {
    let mut pool = form_pool();
    let out = walk(&mut pool, &|_| FormResult::FailedToServe, 0);
    assert_eq!(out.serving, None);
    assert_eq!(out.attempts.len(), 3);
    // The walk's end is itself a logged decision: the final
    // selection found no available form.
    let log = pool.decisions();
    let last = log.last().expect("the walk logged its decisions");
    assert_eq!(last.member, None);
}

#[test]
fn walker_skips_a_walled_form_on_later_walks() {
    let mut pool = form_pool();
    // Two walks in which t1 fails to serve: the second strike
    // walls it.
    for now in 0..2 {
        let out = walk(&mut pool, &script_for("t2"), now);
        assert_eq!(out.serving, Some(NodeId::new("t2")));
    }
    let t1 = NodeId::new("t1");
    assert_eq!(pool.state_of(&t1, 2), MemberState::Walled);

    // The next walk never attempts t1.
    let out = walk(&mut pool, &script_for("t2"), 2);
    assert_eq!(out.attempts, vec![NodeId::new("t2")]);
}

#[test]
fn walker_names_the_serving_form_in_the_log() {
    // Form visibility (#13717): every attempt is a logged
    // selection naming its form, and the serving form is the last
    // selection the walk made.
    let mut pool = form_pool();
    let out = walk(&mut pool, &script_for("t3"), 0);
    assert_eq!(out.serving, Some(NodeId::new("t3")));

    let selections: Vec<Option<NodeId>> = pool
        .decisions()
        .iter()
        .filter(|d| d.kind == fleet_pool::DecisionKind::Selection)
        .map(|d| d.member.clone())
        .collect();
    assert_eq!(
        selections,
        vec![
            Some(NodeId::new("t1")),
            Some(NodeId::new("t2")),
            Some(NodeId::new("t3")),
        ]
    );
    assert!(
        pool.decisions()
            .iter()
            .filter(|d| d.kind == fleet_pool::DecisionKind::Selection)
            .all(|d| d.strategy == "walker-ordered")
    );
}
