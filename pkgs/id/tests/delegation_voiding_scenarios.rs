//! Delegation-grant voiding scenarios reaching `Engine::void` (#7071; design §6.2/§6.3/§7.3).
//!
//! The event-store voiding suites (`voiding_scenarios` / `voiding_properties`) exercise
//! authority voiding for membership operations and contain no delegation grants, so nothing
//! in them reaches the groups engine's grant-voiding path (`Engine::void` in
//! `src/groups/delegation.rs`): a grant is void when a settled removal of its grantor is
//! concurrent with it — neither event in the other's causal past — and the group's removal
//! policy is strong. Weak remove stands the grant, and only the authority check decides
//! later events.
//!
//! These scenarios drive the groups fold directly: a grant is issued, its grantor is
//! removed, and the grantee's onward operation is judged at a position that has seen both
//! the grant and the removal. The group's delegation mode is `KeepPrior`, so no removal
//! disposition can cut the concurrent grant (the removal event's own mode only governs
//! grants in its causal future): `Engine::void` alone decides whether it stands.
//!
//! The planted-bug check (`planted-bug-check.sh`, mutation `delegation-voiding`) inverts
//! the strong-removal gate at `Engine::void` and requires this suite to fail.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::groups::{
    Body, Create, Delegate, Delegation, Event, Key, MemberOp, Outcome, Policy, Projection, Refusal,
    Removal, RemoveMode, Role, Template, fold,
};
use p2panda_core::Hash;

fn key(n: u8) -> Key {
    [n; 32]
}

fn id(name: &str) -> Hash {
    Hash::digest(name.as_bytes())
}

fn after(names: &[&str]) -> Vec<Hash> {
    names.iter().map(|name| id(name)).collect()
}

fn event(name: &str, author: u8, after_names: &[&str], group: Option<Hash>, body: Body) -> Event {
    Event {
        id: id(name),
        author: key(author),
        after: after(after_names),
        group,
        body,
    }
}

/// A group whose removal strength is the variable under test. `KeepPrior` delegation keeps
/// every other disposition from cutting a concurrent grant, isolating `Engine::void`.
fn policy(removal: Removal) -> Policy {
    Policy {
        removal,
        delegation: Some(Delegation::KeepPrior),
        ..Policy::default()
    }
}

fn custom(name: &str, author: u8, record: Policy) -> Event {
    let body = Body::Create(Create {
        template: Template::Custom,
        policy: record,
        members: vec![],
        admins_on_create: false,
    });
    event(name, author, &[], None, body)
}

fn member(name: &str, by: u8, group: Hash, op: MemberOp, after_names: &[&str]) -> Event {
    event(name, by, after_names, Some(group), Body::Member(op))
}

fn add(name: &str, by: u8, group: Hash, who: u8, role: Role, after_names: &[&str]) -> Event {
    member(
        name,
        by,
        group,
        MemberOp::Add {
            member: key(who),
            role,
        },
        after_names,
    )
}

fn remove_with_mode(
    name: &str,
    by: u8,
    group: Hash,
    who: u8,
    mode: RemoveMode,
    after_names: &[&str],
) -> Event {
    member(
        name,
        by,
        group,
        MemberOp::Remove {
            member: key(who),
            mode,
        },
        after_names,
    )
}

fn delegate(
    name: &str,
    by: u8,
    group: Hash,
    to: u8,
    scope: Role,
    redelegate: bool,
    after_names: &[&str],
) -> Event {
    let body = Body::Delegate(Delegate {
        grantee: key(to),
        scope,
        can_redelegate: redelegate,
    });
    event(name, by, after_names, Some(group), body)
}

fn project(events: &[Event]) -> Projection {
    fold(events).unwrap()
}

fn outcome(p: &Projection, name: &str) -> Outcome {
    p.outcome(&id(name)).cloned().unwrap()
}

/// The shared shape of the concurrent scenarios: 1 (Admin) adds 2, 3 and 4, grants Write to
/// 2, and 2 grants Write on to 3 (`d1`). The removal of 2 (`r0`) has seen only `a0`, so it
/// is concurrent with `d1`: neither is in the other's causal past. The probe `d2` is 3
/// granting Read to 4 from a position that has seen both `d1` and `r0`, so it is judged
/// through whatever is left of the chain 1 -> 2 -> 3.
fn concurrent_grant_events(removal: Removal) -> Vec<Event> {
    let g = id("g");
    vec![
        custom("g", 1, policy(removal)),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        add("a1", 1, g, 3, Role::Write, &["a0"]),
        add("a2", 1, g, 4, Role::Read, &["a1"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a2"]),
        delegate("d1", 2, g, 3, Role::Write, true, &["d0"]),
        remove_with_mode("r0", 1, g, 2, RemoveMode::Revoke, &["a0"]),
        delegate("d2", 3, g, 4, Role::Read, false, &["d1", "r0"]),
    ]
}

#[test]
fn test_a_concurrent_grant_is_void_under_strong_remove() {
    let events = concurrent_grant_events(Removal::Strong);
    let p = project(&events);
    assert!(
        !p.grant_in_force(&id("d1")),
        "strong remove voids a grant concurrent with the grantor's removal"
    );
    assert_eq!(
        outcome(&p, "d2"),
        Outcome::Refused(Refusal::NoChain),
        "with the grant voided, the grantee's onward operation has no chain"
    );

    let mut reversed = events.clone();
    reversed.reverse();
    let p = project(&reversed);
    assert!(!p.grant_in_force(&id("d1")));
    assert_eq!(outcome(&p, "d2"), Outcome::Refused(Refusal::NoChain));
}

#[test]
fn test_a_concurrent_grant_stands_under_weak_remove() {
    let events = concurrent_grant_events(Removal::Weak);
    let p = project(&events);
    assert!(
        p.grant_in_force(&id("d1")),
        "weak remove stands a grant concurrent with it"
    );
    assert_eq!(
        outcome(&p, "d2"),
        Outcome::Applied,
        "with the grant standing, the grantee's onward operation is judged through it"
    );

    let mut reversed = events.clone();
    reversed.reverse();
    let p = project(&reversed);
    assert!(p.grant_in_force(&id("d1")));
    assert_eq!(outcome(&p, "d2"), Outcome::Applied);
}

/// The causal-past half of `Engine::void`: a removal that has *seen* the grant is not
/// concurrent with it, so it cannot void it — the removal's own mode disposes of the grant
/// instead. Here the mode is `KeepPrior`, so the grant stands even under strong remove, and
/// the grantee's onward operation still applies. A mutation that drops the causal-past
/// condition at `Engine::void` voids this grant and fails this test.
#[test]
fn test_a_grant_the_removal_has_seen_is_not_void_under_strong_remove() {
    let g = id("g");
    let events = vec![
        custom("g", 1, policy(Removal::Strong)),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        add("a1", 1, g, 3, Role::Write, &["a0"]),
        add("a2", 1, g, 4, Role::Read, &["a1"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a2"]),
        delegate("d1", 2, g, 3, Role::Write, true, &["d0"]),
        remove_with_mode("r0", 1, g, 2, RemoveMode::KeepPrior, &["d1"]),
        delegate("d2", 3, g, 4, Role::Read, false, &["d1", "r0"]),
    ];
    let p = project(&events);
    assert!(
        p.grant_in_force(&id("d1")),
        "a removal in the grant's causal future does not void it; its KeepPrior mode retains it"
    );
    assert_eq!(outcome(&p, "d2"), Outcome::Applied);
}
