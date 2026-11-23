//! Acceptance oracle for #7916 (spike 4/5 for #6300): the host-side
//! policy point — capability ceiling, per-session quota with visible
//! parking, and the signed/hash-chained audit log (design §6, §7,
//! §9.3).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::SigningKey;
use id_caps::agent_delegation::{
    AgentId, DEFAULT_LIFETIME_SECS, Delegation, Role, RoleBinding, verify_join,
};
use id_caps::agent_policy::{
    AgentAction, AuditChainError, AuditLog, Ceiling, ParkEvent, PolicyPoint, PolicyRefusal, Quota,
    RateLimit,
};
use id_caps::world_scopes::WorldScopes;

const NOW: u64 = 1_700_000_000;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

/// A session fixture: owner 7, agent 9, host 3; role "archivist" in
/// world "atlas" with the GUEST ceiling; the delegation carries
/// exactly `scopes`.
struct Fixture {
    host: SigningKey,
    point: PolicyPoint,
}

fn fixture(scopes: WorldScopes, budget: Quota, rate_limits: Vec<RateLimit>) -> Fixture {
    fixture_with_ceiling(scopes, budget, rate_limits, None)
}

fn fixture_with_ceiling(
    scopes: WorldScopes,
    budget: Quota,
    rate_limits: Vec<RateLimit>,
    widen: Option<fn(Ceiling) -> Ceiling>,
) -> Fixture {
    let owner = key(7);
    let agent = key(9);
    let host = key(3);
    let role = Role::new(
        "archivist",
        WorldScopes::GUEST,
        vec!["atlas".to_owned()],
        DEFAULT_LIFETIME_SECS,
    )
    .expect("role within lifetime bounds");
    let binding = RoleBinding::bind(
        &owner,
        "atlas",
        &role,
        AgentId::from_verifying_key(&agent.verifying_key()),
    )
    .expect("role applies to world");
    let delegation = Delegation::issue(&owner, &binding, scopes, NOW, DEFAULT_LIFETIME_SECS)
        .expect("attenuated issue");
    let grant = verify_join(&binding, Some(&delegation), "atlas", NOW).expect("valid chain");
    let mut ceiling = Ceiling::from_grant(&grant);
    if let Some(widen) = widen {
        ceiling = widen(ceiling);
    }
    let point = PolicyPoint::open(&grant, ceiling, budget, rate_limits, &host, NOW)
        .expect("ceiling covers the grant world");
    Fixture { host, point }
}

const fn big_budget() -> Quota {
    Quota {
        frames: 100,
        tool_calls: 100,
        program_invocations: 10,
    }
}

/// Oracle §6.1 + §7: an out-of-ceiling action is refused, audited,
/// and never executed. The delegation carries no CHAT, so `chat.post`
/// is outside the ceiling the grant set at role grant.
#[test]
fn out_of_ceiling_action_is_refused_audited_and_never_executed() {
    let mut fx = fixture(WorldScopes::JOIN, big_budget(), Vec::new());
    let mut executed = 0u32;
    let admission = fx.point.admit(
        &fx.host,
        &AgentAction::new("chat.post", "chat.post"),
        0,
        NOW + 1,
    );
    if admission.decision.is_ok() {
        executed += 1;
    }
    assert_eq!(admission.decision, Err(PolicyRefusal::CeilingTool));
    assert_eq!(executed, 0, "a refused action must never execute");
    assert_eq!(admission.event.event.tool, "chat.post");
    assert_eq!(admission.event.event.outcome, "refused:ceiling_tool");
    assert_eq!(admission.event.event.quota_charged, Quota::ZERO);
    assert_eq!(fx.point.spent(), Quota::ZERO);
}

/// Oracle §7: refusals are never sampled away — a mixed run of
/// actions leaves exactly one audit event per action, plus the join.
#[test]
fn refusals_are_never_sampled_away() {
    let mut fx = fixture(
        WorldScopes::JOIN.union(WorldScopes::CHAT),
        big_budget(),
        Vec::new(),
    );
    let actions = [
        AgentAction::new("screen.read", "screen.read"),
        AgentAction::new("chat.post", "chat.post"),
        AgentAction::new("program.invoke", "program.invoke:delete-all"),
        AgentAction::new("screen.read", "screen.read").with_decided_revision(41),
        AgentAction::new("chat.post", "chat.post"),
        AgentAction::new("audit.mutate", "audit.mutate"),
    ];
    let mut refusals = 0u64;
    for (index, action) in actions.iter().enumerate() {
        let admission = fx.point.admit(&fx.host, action, 42, NOW + 1 + index as u64);
        if admission.decision.is_err() {
            refusals += 1;
        }
    }
    assert_eq!(refusals, 3);
    assert_eq!(fx.point.log().len(), actions.len() + 1);
    let refused_events = fx
        .point
        .log()
        .events()
        .iter()
        .filter(|chained| chained.event.outcome.starts_with("refused:"))
        .count();
    assert_eq!(refused_events, 3);
    fx.point
        .log()
        .verify(&fx.host.verifying_key())
        .expect("chain verifies");
}

/// Oracle §6.2: quota exhaustion parks the agent visibly — the park
/// is a first-class world event, and a parked session refuses
/// everything after it.
#[test]
fn quota_exhaustion_parks_the_agent_visibly() {
    let budget = Quota {
        frames: 2,
        tool_calls: 2,
        program_invocations: 0,
    };
    let mut fx = fixture(WorldScopes::GUEST, budget, Vec::new());
    let read = AgentAction::new("screen.read", "screen.read");
    assert!(fx.point.admit(&fx.host, &read, 0, NOW + 1).decision.is_ok());
    assert!(fx.point.admit(&fx.host, &read, 0, NOW + 2).decision.is_ok());

    let third = fx.point.admit(&fx.host, &read, 0, NOW + 3);
    assert_eq!(third.decision, Err(PolicyRefusal::QuotaExhausted));
    let park: ParkEvent = third.park_event.expect("exhaustion emits the park event");
    assert_eq!(park.kind, "agent_parked");
    assert_eq!(park.reason, "quota_exhausted");
    assert_eq!(park.message, "agent parked, quota exhausted");
    assert_eq!(park.quota_spent.frames, 2);
    assert!(fx.point.is_parked());

    let fourth = fx.point.admit(&fx.host, &read, 0, NOW + 4);
    assert_eq!(fourth.decision, Err(PolicyRefusal::SessionParked));
    assert_eq!(fourth.park_event, None, "the park event fires once");
}

/// Oracle §6.2 + §7: the charge lands before execution and a refusal
/// charges nothing — the ledger only ever moves on admission.
#[test]
fn quota_is_charged_before_execution_and_a_refusal_charges_nothing() {
    let mut fx = fixture(WorldScopes::GUEST, big_budget(), Vec::new());
    let chat = AgentAction::new("chat.post", "chat.post");
    let admission = fx.point.admit(&fx.host, &chat, 0, NOW + 1);
    assert_eq!(
        admission.decision.expect("within ceiling and budget"),
        Quota {
            frames: 1,
            tool_calls: 1,
            program_invocations: 0,
        }
    );
    assert_eq!(fx.point.spent().tool_calls, 1);

    let refused = fx.point.admit(
        &fx.host,
        &AgentAction::new("program.invoke", "program.invoke:unlisted"),
        0,
        NOW + 2,
    );
    assert_eq!(refused.decision, Err(PolicyRefusal::CeilingProgram));
    assert_eq!(fx.point.spent().tool_calls, 1, "a refusal charges nothing");
}

/// Oracle §7: the audit chain verifies end-to-end for a spike
/// session, and any edit to any event — content, hash, or signature —
/// is detected. The agent cannot mutate the log: the only writer is
/// the policy point, and tampering with an exported copy fails
/// verification rather than rewriting history.
#[test]
fn audit_chain_verifies_end_to_end_and_detects_tampering() {
    let mut fx = fixture(WorldScopes::GUEST, big_budget(), Vec::new());
    for index in 0..5u64 {
        fx.point.admit(
            &fx.host,
            &AgentAction::new("screen.read", "screen.read"),
            0,
            NOW + 1 + index,
        );
    }
    let host_key = fx.host.verifying_key();
    fx.point.log().verify(&host_key).expect("chain verifies");

    let mut tampered = fx.point.log().events().to_vec();
    tampered[2].event.outcome = "refused:ceiling_tool".to_owned();
    assert_eq!(
        AuditLog::verify_events(&tampered, &host_key),
        Err(AuditChainError::HashMismatch)
    );

    let mut rehashed = fx.point.log().events().to_vec();
    rehashed[1].signature = [0u8; 64];
    assert_eq!(
        AuditLog::verify_events(&rehashed, &host_key),
        Err(AuditChainError::BadSignature)
    );

    // The non-tools of §4.5 stay non-tools at the policy point:
    // audit mutation and quota adjustment are refused and audited.
    let mutation = fx.point.admit(
        &fx.host,
        &AgentAction::new("audit.mutate", "audit.mutate"),
        0,
        NOW + 10,
    );
    assert_eq!(mutation.decision, Err(PolicyRefusal::CeilingTool));
    assert_eq!(fx.point.log().len(), 7);
    fx.point
        .log()
        .verify(&host_key)
        .expect("chain still verifies");
}

/// Oracle §4.3 + §7: an action decided against a stale screen
/// revision is rejected and audited like a refusal; the same action
/// at the current revision executes.
#[test]
fn stale_input_is_rejected_and_audited() {
    let mut fx = fixture(WorldScopes::GUEST, big_budget(), Vec::new());
    let action = AgentAction::new("chat.post", "chat.post").with_decided_revision(7);
    let stale = fx.point.admit(&fx.host, &action, 8, NOW + 1);
    assert_eq!(stale.decision, Err(PolicyRefusal::StaleInput));
    assert_eq!(stale.event.event.outcome, "refused:stale_input");
    let fresh = fx.point.admit(&fx.host, &action, 7, NOW + 2);
    assert!(fresh.decision.is_ok());
}

/// Oracle §7: the owner summary shows actions, spend, and refusals
/// for the session, in structured form and rendered.
#[test]
fn owner_summary_shows_actions_spend_and_refusals() {
    let mut fx = fixture(WorldScopes::GUEST, big_budget(), Vec::new());
    fx.point.admit(
        &fx.host,
        &AgentAction::new("screen.read", "screen.read"),
        0,
        NOW + 1,
    );
    fx.point.admit(
        &fx.host,
        &AgentAction::new("chat.post", "chat.post"),
        0,
        NOW + 2,
    );
    fx.point.admit(
        &fx.host,
        &AgentAction::new("program.invoke", "program.invoke:unlisted"),
        0,
        NOW + 3,
    );
    let summary = fx.point.summary();
    assert_eq!(summary.executed, 2);
    assert_eq!(summary.refused, 1);
    assert_eq!(summary.refusals.get("ceiling_program"), Some(&1));
    assert_eq!(summary.quota_spent.tool_calls, 2);
    assert!(!summary.parked);
    let rendered = summary.render();
    assert!(rendered.contains("2 actions executed, 1 refused"));
    assert!(rendered.contains("ceiling_program x1"));
}

/// Oracle §4.6 + §6.1 + §6.2: program invocation needs both the
/// base key in the ceiling and the program on the owner's allowlist,
/// and it charges the program axis of the quota.
#[test]
fn program_invoke_needs_the_allowlist_and_charges_the_program_axis() {
    let mut fx = fixture_with_ceiling(
        WorldScopes::GUEST,
        big_budget(),
        Vec::new(),
        Some(|ceiling| ceiling.with_programs(vec!["lamp.toggle".to_owned()])),
    );
    let allowed = fx.point.admit(
        &fx.host,
        &AgentAction::new("program.invoke", "program.invoke:lamp.toggle"),
        0,
        NOW + 1,
    );
    let charge = allowed.decision.expect("allowlisted program admitted");
    assert_eq!(charge.program_invocations, 1);
    assert_eq!(fx.point.spent().program_invocations, 1);

    let unlisted = fx.point.admit(
        &fx.host,
        &AgentAction::new("program.invoke", "program.invoke:door.unlock"),
        0,
        NOW + 2,
    );
    assert_eq!(unlisted.decision, Err(PolicyRefusal::CeilingProgram));
}

/// Oracle §6.1: object classes are part of the ceiling — an action
/// on a class the owner has not allowed is refused even when its
/// tool is in the ceiling.
#[test]
fn object_classes_are_part_of_the_ceiling() {
    let mut fx = fixture_with_ceiling(
        WorldScopes::GUEST,
        big_budget(),
        Vec::new(),
        Some(|ceiling| ceiling.with_object_classes(vec!["note".to_owned()])),
    );
    let note = AgentAction::new("object.inspect", "screen.read").with_object("obj-1", "note");
    assert!(fx.point.admit(&fx.host, &note, 0, NOW + 1).decision.is_ok());
    let secret =
        AgentAction::new("object.inspect", "screen.read").with_object("obj-2", "credential");
    let refused = fx.point.admit(&fx.host, &secret, 0, NOW + 2);
    assert_eq!(refused.decision, Err(PolicyRefusal::CeilingObjectClass));
}

/// Oracle §6.3: the per-tool rate limit refuses beyond the role's
/// limit inside the window, and the window slides.
#[test]
fn chat_post_rate_limit_refuses_beyond_the_role_limit() {
    let limits = vec![RateLimit {
        tool: "chat.post".to_owned(),
        max_calls: 2,
        window_secs: 60,
    }];
    let mut fx = fixture(WorldScopes::GUEST, big_budget(), limits);
    let chat = AgentAction::new("chat.post", "chat.post");
    assert!(fx.point.admit(&fx.host, &chat, 0, NOW + 1).decision.is_ok());
    assert!(fx.point.admit(&fx.host, &chat, 0, NOW + 2).decision.is_ok());
    let third = fx.point.admit(&fx.host, &chat, 0, NOW + 3);
    assert_eq!(third.decision, Err(PolicyRefusal::RateLimited));
    let later = fx.point.admit(&fx.host, &chat, 0, NOW + 62);
    assert!(later.decision.is_ok(), "the window slides open again");
}

/// Oracle §5.3: once the delegation behind the session expires,
/// actions are refused (and audited) until the owner's client
/// renews.
#[test]
fn expired_delegation_refuses_actions() {
    let mut fx = fixture(WorldScopes::GUEST, big_budget(), Vec::new());
    let admission = fx.point.admit(
        &fx.host,
        &AgentAction::new("screen.read", "screen.read"),
        0,
        NOW + DEFAULT_LIFETIME_SECS,
    );
    assert_eq!(admission.decision, Err(PolicyRefusal::DelegationExpired));
    assert_eq!(admission.event.event.outcome, "refused:delegation_expired");
}

/// Oracle §4.1 + §7: the join is audit event #1 of the session.
#[test]
fn the_join_is_the_first_audit_event() {
    let fx = fixture(WorldScopes::GUEST, big_budget(), Vec::new());
    let events = fx.point.log().events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, 0);
    assert_eq!(events[0].event.tool, "world.join");
    assert_eq!(events[0].event.outcome, "granted");
    assert!(events[0].event.proof_ref.is_some());
}

/// Oracle §6/§7 property: for any sequence of actions, the log
/// holds exactly one event per action plus the join, and the chain
/// verifies — refusals are never sampled away, whatever the mix.
#[test]
fn any_action_sequence_leaves_a_complete_verifiable_log() {
    let mut fx = fixture(WorldScopes::GUEST, big_budget(), Vec::new());
    let tools = [
        ("screen.read", "screen.read"),
        ("chat.post", "chat.post"),
        ("world.leave", "presence"),
        ("program.invoke", "program.invoke:nope"),
        ("host.exec", "host.exec"),
    ];
    let mut state = 0x2436_2017u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 16) as usize
    };
    for step in 0..40u64 {
        let (tool, ceiling_key) = tools[next() % tools.len()];
        let decided = u64::from(next() % 3 == 0);
        let action = AgentAction::new(tool, ceiling_key).with_decided_revision(decided);
        fx.point.admit(&fx.host, &action, 1, NOW + 1 + step);
    }
    assert_eq!(fx.point.log().len(), 41);
    fx.point
        .log()
        .verify(&fx.host.verifying_key())
        .expect("chain verifies after a mixed sequence");
}
