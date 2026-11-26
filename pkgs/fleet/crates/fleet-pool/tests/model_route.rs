//! The model-route boundary's regression cases (#14723), bound
//! against the #13693 RC-1..RC-4 rules the design (#13752 §3.5
//! step 3) names for this adopter:
//!
//! - RC-1/RC-2: a reset-less wall must still gate, and re-admits
//!   only through the bounded inferred expiry + probe path.
//! - RC-3: walls bind at the cap's scope, not the reader's shape —
//!   a quota wall on one alias provider gates its cap siblings.
//! - RC-4: an all-walled pool returns no selection; the walk stops.
//!
//! Plus the boundary's own contract: recorded outcomes are
//! consumed, selections are returned, and a served failure advances
//! only when its classifier says so.

use fleet_pool::model_route::{ModelRoute, RouteLeg, RouteOutcome};
use fleet_pool::MemberState;

fn leg(provider: &str, model: &str) -> RouteLeg {
    RouteLeg::new(provider, model)
}

fn quota_no_reset() -> RouteOutcome {
    RouteOutcome::Quota {
        reset_hint: None,
        evidence: "quota-class error, no reset recorded".to_owned(),
    }
}

#[test]
fn rc1_a_reset_less_quota_wall_still_gates() {
    // Oracle (#13693 RC-1/RC-2): a wall with no recorded reset must
    // still gate — the leg is not re-selected while the wall holds.
    let mut route = ModelRoute::new(
        "workhorse",
        vec![leg("openai-plan", "gpt-x"), leg("anthropic", "claude-x")],
        &[],
    );
    let fx = route
        .record(&leg("openai-plan", "gpt-x"), &quota_no_reset(), 0)
        .expect("a quota outcome has an effect");
    assert!(fx.advance, "the walk advances past a quota-walled leg");
    assert_eq!(
        route.pool().state_of(&leg("openai-plan", "gpt-x").node_id(), 0),
        MemberState::Walled
    );
    // Still gated just inside the inferred TTL (300s default).
    assert_eq!(route.select(299), Some(leg("anthropic", "claude-x")));
    assert_eq!(route.select(299), Some(leg("anthropic", "claude-x")));
}

#[test]
fn rc2_a_reset_less_wall_readmits_through_expiry_and_probe() {
    // Oracle (#13693 RC-2): the reset-less wall expires at the
    // inferred bound; the leg re-enters through probing and is
    // re-seated (demoted) only when its probe passes.
    let mut route = ModelRoute::new("workhorse", vec![leg("openai-plan", "gpt-x")], &[]);
    route.record(&leg("openai-plan", "gpt-x"), &quota_no_reset(), 0);
    assert_eq!(route.select(10), None, "gated while the wall holds");

    route.tick(300, &mut |_: &fleet_pool::NodeId| true);
    assert_eq!(
        route.pool().state_of(&leg("openai-plan", "gpt-x").node_id(), 300),
        MemberState::Demoted,
        "a passed probe re-seats the leg at demoted weight"
    );
    assert_eq!(route.select(300), Some(leg("openai-plan", "gpt-x")));
}

#[test]
fn rc3_a_quota_wall_binds_at_the_cap_scope_not_the_reader_shape() {
    // Oracle (#13693 RC-3): plan and free aliases draw on one cap,
    // declared as a cap group. A quota wall recorded against the
    // plan alias gates the free alias too — and no wider: an
    // unrelated provider stays selectable.
    let mut route = ModelRoute::new(
        "workhorse",
        vec![
            leg("openai-plan", "gpt-x"),
            leg("openai-free", "gpt-x"),
            leg("anthropic", "claude-x"),
        ],
        &[("openai-main", &["openai-plan", "openai-free"])],
    );
    route.record(&leg("openai-plan", "gpt-x"), &quota_no_reset(), 0);
    assert_eq!(
        route.pool().state_of(&leg("openai-free", "gpt-x").node_id(), 0),
        MemberState::Walled,
        "the cap sibling is gated by the same wall"
    );
    assert_eq!(route.select(0), Some(leg("anthropic", "claude-x")));
}

#[test]
fn rc4_an_all_walled_pool_returns_no_selection() {
    // Oracle (#13693 RC-4): when every leg is walled the boundary
    // returns no selection — the walk stops instead of hitting
    // walled legs indefinitely.
    let mut route = ModelRoute::new(
        "workhorse",
        vec![leg("openai-plan", "gpt-x"), leg("anthropic", "claude-x")],
        &[],
    );
    route.record(&leg("openai-plan", "gpt-x"), &quota_no_reset(), 0);
    route.record(&leg("anthropic", "claude-x"), &quota_no_reset(), 0);
    assert_eq!(route.select(0), None);
}

#[test]
fn boundary_consumes_outcomes_and_returns_selections_in_order() {
    // The boundary contract: with no outcomes recorded, selection
    // is the first declared leg; a transport failure advances the
    // next selection to the following leg; a served call records a
    // success and the leg stays first choice.
    let mut route = ModelRoute::new(
        "workhorse",
        vec![leg("openai-plan", "gpt-x"), leg("anthropic", "claude-x")],
        &[],
    );
    assert_eq!(route.select(0), Some(leg("openai-plan", "gpt-x")));

    let fx = route
        .record(
            &leg("openai-plan", "gpt-x"),
            &RouteOutcome::Transport {
                evidence: "connect timeout".to_owned(),
            },
            1,
        )
        .expect("a transport outcome has an effect");
    assert!(fx.advance);
    assert_eq!(route.select(1), Some(leg("anthropic", "claude-x")));

    route.record(&leg("anthropic", "claude-x"), &RouteOutcome::Served, 2);
    // The served call ended that walk; the next walk starts again
    // in declaration order — the demoted first leg is still
    // eligible (demotion is not a wall).
    assert_eq!(route.select(2), Some(leg("openai-plan", "gpt-x")));
}

#[test]
fn boundary_served_failure_advances_only_when_its_classifier_says_so() {
    let mut route = ModelRoute::new(
        "workhorse",
        vec![leg("openai-plan", "gpt-x"), leg("anthropic", "claude-x")],
        &[],
    );
    let fx = route
        .record(
            &leg("openai-plan", "gpt-x"),
            &RouteOutcome::ServedFailure {
                advance: false,
                evidence: "guard refusal: a served result".to_owned(),
            },
            0,
        )
        .expect("a served failure has an effect");
    assert!(!fx.advance, "a terminal served failure must not advance");
}
