//! Behaviour of groups and friendship. Each rule has a test. The property tests check that the
//! final state does not depend on the order the events arrive in.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use p2panda_core::Hash;
use proptest::prelude::*;

use super::policy::{Delegation, Join, OnMemberRemoved, Policy, Removal, RoleRules, Template};
use super::{
    Body, Create, Delegate, DeviceOp, Event, Key, MemberOp, Outcome, Pair, Projection, Refusal,
    Role, fold, friendship_id,
};
use crate::rejection::Reason;

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

/// The friendship group of `a` and `b` at `counter`.
fn pair_group(a: u8, b: u8, counter: u32) -> Hash {
    friendship_id(&Pair::of(key(a), key(b)).unwrap(), counter)
}

/// A group created from `template` by `author`, with `members` besides the author.
fn create(
    name: &str,
    author: u8,
    after_names: &[&str],
    template: Template,
    policy: Policy,
    members: &[(u8, Role)],
    admins_on_create: bool,
) -> Event {
    let members = members.iter().map(|(n, role)| (key(*n), *role)).collect();
    let body = Body::Create(Create {
        template,
        policy,
        members,
        admins_on_create,
    });
    event(name, author, after_names, None, body)
}

/// A normal group of a custom policy, created by `author`.
fn custom(name: &str, author: u8, members: &[(u8, Role)], policy: Policy) -> Event {
    create(name, author, &[], Template::Custom, policy, members, false)
}

/// A friendship request from `from` to `to`, naming the friendship at `counter`.
fn request(
    name: &str,
    from: u8,
    to: u8,
    counter: u32,
    after_names: &[&str],
    admins: bool,
) -> Event {
    let pair = Pair::of(key(from), key(to));
    let group = pair.map(|pair| friendship_id(&pair, counter));
    let body = Body::Create(Create {
        template: Template::Friendship,
        policy: Policy::friendship(),
        members: vec![(key(to), Role::Manage)],
        admins_on_create: admins,
    });
    event(name, from, after_names, group, body)
}

fn join(name: &str, by: u8, group: Hash, after_names: &[&str]) -> Event {
    event(name, by, after_names, Some(group), Body::Join)
}

fn policy(name: &str, by: u8, group: Hash, record: Policy, after_names: &[&str]) -> Event {
    event(name, by, after_names, Some(group), Body::Policy(record))
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

fn remove(name: &str, by: u8, group: Hash, who: u8, after_names: &[&str]) -> Event {
    member(
        name,
        by,
        group,
        MemberOp::Remove {
            member: key(who),
            successor: None,
        },
        after_names,
    )
}

fn set_role(name: &str, by: u8, group: Hash, who: u8, role: Role, after_names: &[&str]) -> Event {
    member(
        name,
        by,
        group,
        MemberOp::SetRole {
            member: key(who),
            role,
        },
        after_names,
    )
}

fn project(events: &[Event]) -> Projection {
    fold(events).unwrap()
}

fn outcome(p: &Projection, name: &str) -> Outcome {
    p.outcome(&id(name)).cloned().unwrap()
}

fn role_of(p: &Projection, group: Hash, n: u8) -> Option<Role> {
    p.group(&group)?.members.get(&key(n)).copied()
}

/// A deterministic shuffle, so a failing case can be replayed from its seed.
fn shuffled<T: Clone>(items: &[T], mut seed: u64) -> Vec<T> {
    let mut out = items.to_vec();
    for i in (1..out.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let j = usize::try_from(seed % (i as u64 + 1)).unwrap();
        out.swap(i, j);
    }
    out
}

// ----- Creation and roles -----

#[test]
fn test_anyone_may_create_a_normal_group_and_the_creator_is_its_admin() {
    let events = [custom("g", 3, &[(1, Role::Read)], Policy::default())];
    let p = project(&events);
    assert_eq!(outcome(&p, "g"), Outcome::Applied);
    assert_eq!(role_of(&p, id("g"), 3), Some(Role::Admin));
    assert_eq!(role_of(&p, id("g"), 1), Some(Role::Read));
}

#[test]
fn test_a_friendship_has_no_admin_by_default() {
    let g = pair_group(1, 2, 0);
    let p = project(&[
        request("r0", 1, 2, 0, &[], false),
        join("a0", 2, g, &["r0"]),
    ]);
    assert_eq!(role_of(&p, g, 1), Some(Role::Manage));
    assert_eq!(role_of(&p, g, 2), Some(Role::Manage));
    assert!(p.friends(&key(1), &key(2)));
}

#[test]
fn test_admins_on_create_makes_both_friends_admin() {
    let g = pair_group(1, 2, 0);
    let p = project(&[request("r0", 1, 2, 0, &[], true), join("a0", 2, g, &["r0"])]);
    assert_eq!(role_of(&p, g, 1), Some(Role::Admin));
    assert_eq!(role_of(&p, g, 2), Some(Role::Admin));
}

#[test]
fn test_either_admin_friend_may_change_the_other_friends_role() {
    let g = pair_group(1, 2, 0);
    let base = [request("r0", 1, 2, 0, &[], true), join("a0", 2, g, &["r0"])];
    let mut ann_demotes_bo = base.to_vec();
    ann_demotes_bo.push(set_role("s1", 1, g, 2, Role::Read, &["a0"]));
    let p = project(&ann_demotes_bo);
    assert_eq!(outcome(&p, "s1"), Outcome::Applied);
    assert_eq!(role_of(&p, g, 2), Some(Role::Read));

    let mut bo_demotes_ann = base.to_vec();
    bo_demotes_ann.push(set_role("s2", 2, g, 1, Role::Manage, &["a0"]));
    let p = project(&bo_demotes_ann);
    assert_eq!(outcome(&p, "s2"), Outcome::Applied);
    assert_eq!(role_of(&p, g, 1), Some(Role::Manage));
}

#[test]
fn test_a_non_admin_may_not_change_a_role() {
    let g = id("g");
    let p = project(&[
        custom(
            "g",
            1,
            &[(2, Role::Manage), (3, Role::Read)],
            Policy::default(),
        ),
        set_role("s0", 2, g, 3, Role::Write, &["g"]),
    ]);
    assert_eq!(outcome(&p, "s0"), Outcome::Refused(Refusal::NotAdmin));
    assert_eq!(role_of(&p, g, 3), Some(Role::Read));
}

#[test]
fn test_a_rule_other_than_admin_for_roles_is_refused() {
    let manager_rule = Policy {
        roles: RoleRules {
            change: Role::Manage,
        },
        ..Policy::default()
    };
    let p = project(&[custom("g", 1, &[], manager_rule)]);
    assert_eq!(outcome(&p, "g"), Outcome::Refused(Refusal::BadPolicy));
}

// ----- Policy -----

#[test]
fn test_a_non_admin_may_not_change_the_policy_of_a_normal_group_or_a_friendship() {
    let g = id("g");
    let p = project(&[
        custom("g", 1, &[(2, Role::Manage)], Policy::default()),
        policy("p0", 2, g, Policy::default(), &["g"]),
    ]);
    assert_eq!(outcome(&p, "p0"), Outcome::Refused(Refusal::NotAdmin));

    let f = pair_group(1, 2, 0);
    let capped = Policy {
        member_cap: Some(3),
        ..Policy::friendship()
    };
    let p = project(&[
        request("r0", 1, 2, 0, &[], false),
        join("a0", 2, f, &["r0"]),
        policy("p0", 1, f, capped, &["a0"]),
    ]);
    assert_eq!(outcome(&p, "p0"), Outcome::Refused(Refusal::NotAdmin));
    assert_eq!(p.group(&f).unwrap().policy, Policy::friendship());
}

#[test]
fn test_an_admin_changes_the_member_cap_and_not_below_the_members() {
    let g = id("g");
    let raise = Policy {
        member_cap: Some(3),
        ..Policy::default()
    };
    let lower = Policy {
        member_cap: Some(1),
        ..Policy::default()
    };
    let p = project(&[
        custom("g", 1, &[(2, Role::Read)], Policy::default()),
        policy("p0", 1, g, raise, &["g"]),
        policy("p1", 1, g, lower, &["p0"]),
    ]);
    assert_eq!(outcome(&p, "p0"), Outcome::Applied);
    assert_eq!(p.group(&g).unwrap().policy.member_cap, Some(3));
    assert_eq!(
        outcome(&p, "p1"),
        Outcome::Refused(Refusal::OverCap { cap: 1 })
    );
}

// ----- Member cap -----

#[test]
fn test_the_member_cap_refuses_the_add_past_it() {
    let g = id("g");
    let capped = Policy {
        member_cap: Some(2),
        ..Policy::default()
    };
    let p = project(&[
        custom("g", 1, &[], capped),
        add("a1", 1, g, 2, Role::Read, &["g"]),
        add("a2", 1, g, 3, Role::Read, &["a1"]),
    ]);
    assert_eq!(outcome(&p, "a1"), Outcome::Applied);
    assert_eq!(
        outcome(&p, "a2"),
        Outcome::Refused(Refusal::OverCap { cap: 2 })
    );
    assert_eq!(p.group(&g).unwrap().members.len(), 2);
}

#[test]
fn test_concurrent_adds_past_the_cap_keep_the_lowest_event_ids() {
    // F2 (S2 friendship). Two concurrent adds and one free place: the add with the lower event ID
    // wins, in every order.
    let g = id("g");
    let capped = Policy {
        member_cap: Some(3),
        ..Policy::default()
    };
    let events = [
        custom("g", 1, &[(2, Role::Read)], capped),
        add("a1", 1, g, 3, Role::Read, &["g"]),
        add("a2", 1, g, 4, Role::Read, &["g"]),
    ];
    let (low, high, low_key) = if id("a1") < id("a2") {
        ("a1", "a2", 3)
    } else {
        ("a2", "a1", 4)
    };
    let p = project(&events);
    assert_eq!(outcome(&p, low), Outcome::Applied);
    assert_eq!(
        outcome(&p, high),
        Outcome::Refused(Refusal::OverCap { cap: 3 })
    );
    let members: Vec<Key> = p.group(&g).unwrap().members.keys().copied().collect();
    assert_eq!(members, vec![key(1), key(2), key(low_key)]);

    let mut reversed = events.to_vec();
    reversed.reverse();
    assert_eq!(project(&reversed), p);
}

// ----- Templates -----

#[test]
fn test_a_template_admits_only_the_policy_it_names() {
    let capped = Policy {
        member_cap: Some(2),
        ..Policy::default()
    };
    let p = project(&[
        create("c1", 1, &[], Template::Channel, capped, &[], false),
        create(
            "c2",
            1,
            &[],
            Template::Private,
            Policy::default(),
            &[],
            false,
        ),
        create(
            "c3",
            1,
            &[],
            Template::Friendship,
            Policy {
                member_cap: Some(3),
                ..Policy::friendship()
            },
            &[(2, Role::Manage)],
            false,
        ),
        create(
            "c4",
            1,
            &[],
            Template::Private,
            Policy {
                join: Join::Invite,
                ..Policy::default()
            },
            &[(2, Role::Read)],
            false,
        ),
    ]);
    assert_eq!(
        outcome(&p, "c1"),
        Outcome::Refused(Refusal::BadTemplate(Template::Channel))
    );
    assert_eq!(
        outcome(&p, "c2"),
        Outcome::Refused(Refusal::BadTemplate(Template::Private))
    );
    assert_eq!(
        outcome(&p, "c3"),
        Outcome::Refused(Refusal::BadTemplate(Template::Friendship))
    );
    assert_eq!(outcome(&p, "c4"), Outcome::Applied);
}

#[test]
fn test_admins_on_create_is_for_friendships_only() {
    let p = project(&[create(
        "g",
        1,
        &[],
        Template::Channel,
        Policy::default(),
        &[(2, Role::Read)],
        true,
    )]);
    assert_eq!(
        outcome(&p, "g"),
        Outcome::Refused(Refusal::BadTemplate(Template::Channel))
    );
}

#[test]
fn test_a_join_is_for_a_friendship_and_a_normal_group_refuses_it() {
    let g = id("g");
    let mutual = Policy {
        join: Join::MutualRequest,
        ..Policy::default()
    };
    let p = project(&[
        custom("g", 1, &[(2, Role::Read)], mutual),
        join("j0", 2, g, &["g"]),
    ]);
    assert_eq!(
        outcome(&p, "j0"),
        Outcome::Refused(Refusal::JoinNotSupported)
    );
}

#[test]
fn test_a_removal_on_a_policy_that_continues_does_not_dissolve() {
    let g = id("g");
    let p = project(&[
        custom("g", 1, &[(2, Role::Read)], Policy::default()),
        remove("x0", 1, g, 2, &["g"]),
    ]);
    assert_eq!(outcome(&p, "x0"), Outcome::Applied);
    assert_eq!(p.group(&g).unwrap().dissolved_by, None);
    assert_eq!(
        Policy::default().on_member_removed,
        OnMemberRemoved::Continue
    );
}

// ----- Rejection reasons -----

#[test]
fn test_a_rule_refusal_gives_the_voided_reason() {
    let g = id("g");
    let p = project(&[
        custom(
            "g",
            1,
            &[(2, Role::Write), (3, Role::Read)],
            Policy::default(),
        ),
        add("a0", 2, g, 4, Role::Read, &["g"]),
    ]);
    assert_eq!(outcome(&p, "a0"), Outcome::Refused(Refusal::NotManager));
    assert_eq!(outcome(&p, "a0").reason(), Some(Reason::Voided));
}

#[test]
fn test_a_pending_friend_refusal_gives_the_waiting_reason() {
    let g = id("g");
    let f = pair_group(1, 2, 0);
    let p = project(&[
        custom("g", 1, &[], Policy::default()),
        add("a0", 3, g, 4, Role::Read, &["g"]),
        request("r0", 1, 2, 0, &[], false),
        add("a1", 2, f, 3, Role::Read, &["r0"]),
    ]);
    assert_eq!(outcome(&p, "a1"), Outcome::Refused(Refusal::NotActive));
    assert_eq!(outcome(&p, "a1").reason(), Some(Reason::Waiting));
}

#[test]
fn test_a_refusal_in_a_dissolved_group_gives_the_frozen_reason() {
    let g = pair_group(1, 2, 0);
    let p = project(&[
        request("r0", 1, 2, 0, &[], false),
        join("a0", 2, g, &["r0"]),
        remove("x", 1, g, 2, &["a0"]),
        remove("y", 2, g, 1, &["x"]),
    ]);
    assert_eq!(outcome(&p, "y"), Outcome::Refused(Refusal::Dissolved));
    assert_eq!(outcome(&p, "y").reason(), Some(Reason::Frozen));
}

// ----- Membership rules -----

#[test]
fn test_a_non_member_may_not_add_and_a_pending_friend_may_not_act() {
    let g = id("g");
    let f = pair_group(1, 2, 0);
    let p = project(&[
        custom("g", 1, &[], Policy::default()),
        add("a0", 3, g, 4, Role::Read, &["g"]),
        request("r0", 1, 2, 0, &[], false),
        add("a1", 2, f, 3, Role::Read, &["r0"]),
    ]);
    assert_eq!(outcome(&p, "a0"), Outcome::Refused(Refusal::NotActive));
    assert_eq!(outcome(&p, "a1"), Outcome::Refused(Refusal::NotActive));
}

#[test]
fn test_a_non_manager_may_not_add_or_remove_a_member() {
    let g = id("g");
    let p = project(&[
        custom(
            "g",
            1,
            &[(2, Role::Write), (3, Role::Read)],
            Policy::default(),
        ),
        add("a0", 2, g, 4, Role::Read, &["g"]),
        remove("x0", 2, g, 3, &["g"]),
    ]);
    assert_eq!(outcome(&p, "a0"), Outcome::Refused(Refusal::NotManager));
    assert_eq!(outcome(&p, "x0"), Outcome::Refused(Refusal::NotManager));
}

#[test]
fn test_a_delegated_manage_may_add_and_remove_a_member() {
    let g = id("g");
    let p = project(&[
        custom(
            "g",
            1,
            &[(2, Role::Write), (3, Role::Read)],
            Policy::default(),
        ),
        delegate("d0", 1, g, 2, Role::Manage, false, &["g"]),
        add("a0", 2, g, 4, Role::Read, &["d0"]),
        remove("x0", 2, g, 3, &["a0"]),
    ]);
    assert_eq!(outcome(&p, "d0"), Outcome::Applied);
    assert_eq!(
        outcome(&p, "a0"),
        Outcome::Applied,
        "a Write member holding a delegated Manage chain may add"
    );
    assert_eq!(
        outcome(&p, "x0"),
        Outcome::Applied,
        "and may remove another non-Admin member"
    );
    assert_eq!(role_of(&p, g, 4), Some(Role::Read));
    assert_eq!(role_of(&p, g, 3), None);
}

#[test]
fn test_a_delegated_manage_may_not_add_or_remove_an_admin() {
    let g = id("g");
    let p = project(&[
        custom(
            "g",
            1,
            &[(2, Role::Write), (3, Role::Admin)],
            Policy::default(),
        ),
        delegate("d0", 1, g, 2, Role::Manage, false, &["g"]),
        add("a0", 2, g, 4, Role::Admin, &["d0"]),
        remove("x0", 2, g, 3, &["d0"]),
    ]);
    assert_eq!(outcome(&p, "a0"), Outcome::Refused(Refusal::NotAdmin));
    assert_eq!(outcome(&p, "x0"), Outcome::Refused(Refusal::NotAdmin));
}

#[test]
fn test_a_cut_manage_chain_refuses_the_add_as_no_chain() {
    let g = id("g");
    let p = project(&[
        custom(
            "g",
            1,
            &[(2, Role::Write), (3, Role::Read)],
            Policy::default(),
        ),
        delegate("d0", 1, g, 2, Role::Manage, false, &["g"]),
        veto("v0", 1, g, "d0", &["d0"]),
        add("a0", 2, g, 4, Role::Read, &["v0"]),
        remove("x0", 2, g, 3, &["v0"]),
    ]);
    assert_eq!(outcome(&p, "v0"), Outcome::Applied);
    assert_eq!(outcome(&p, "a0"), Outcome::Refused(Refusal::NoChain));
    assert_eq!(outcome(&p, "x0"), Outcome::Refused(Refusal::NoChain));
}

#[test]
fn test_a_delegated_add_concurrent_with_a_strong_removal_of_the_delegatee_is_void() {
    let g = id("g");
    let p = project(&[
        custom(
            "g",
            1,
            &[(2, Role::Write)],
            with_removal(Removal::Strong, None),
        ),
        delegate("d0", 1, g, 2, Role::Manage, false, &["g"]),
        add("a0", 2, g, 4, Role::Read, &["d0"]),
        remove("r0", 1, g, 2, &["d0"]),
    ]);
    assert_eq!(outcome(&p, "r0"), Outcome::Applied);
    assert_eq!(
        role_of(&p, g, 4),
        None,
        "strong remove voids the removed member's concurrent operations, delegated or not"
    );
}

#[test]
fn test_a_descendant_of_a_voided_delegated_add_takes_no_effect() {
    // Member 2's delegated add makes member 4 a manager; member 4 adds member 5.
    // A concurrent strong removal of member 2 voids the first add, and the
    // descendant add must fall with it — the fold must still project.
    let g = id("g");
    let p = project(&[
        custom(
            "g",
            1,
            &[(2, Role::Write)],
            with_removal(Removal::Strong, None),
        ),
        delegate("d0", 1, g, 2, Role::Manage, false, &["g"]),
        add("a0", 2, g, 4, Role::Manage, &["d0"]),
        add("a1", 4, g, 5, Role::Read, &["a0"]),
        remove("r0", 1, g, 2, &["d0"]),
    ]);
    assert_eq!(outcome(&p, "r0"), Outcome::Applied);
    assert_eq!(role_of(&p, g, 4), None);
    assert_eq!(role_of(&p, g, 5), None);
}

#[test]
fn test_a_read_scoped_device_of_a_manager_may_not_add_a_member() {
    let g = id("g");
    let p = project(&[
        custom("g", 1, &[(2, Role::Manage)], Policy::default()),
        device_add("dv0", 2, g, 50, Role::Read, &["g"]),
        add("a0", 50, g, 3, Role::Read, &["dv0"]),
    ]);
    assert_eq!(outcome(&p, "dv0"), Outcome::Applied);
    assert_eq!(
        outcome(&p, "a0"),
        Outcome::Refused(Refusal::NotManager),
        "the device's Read scope caps it, however its account stands"
    );
    assert_eq!(role_of(&p, g, 3), None);
}

#[test]
fn test_a_manage_scoped_device_of_a_manager_may_add_a_member() {
    let g = id("g");
    let p = project(&[
        custom("g", 1, &[(2, Role::Manage)], Policy::default()),
        device_add("dv0", 2, g, 50, Role::Manage, &["g"]),
        add("a0", 50, g, 3, Role::Read, &["dv0"]),
    ]);
    assert_eq!(outcome(&p, "a0"), Outcome::Applied);
    assert_eq!(role_of(&p, g, 3), Some(Role::Read));
}

#[test]
fn test_only_an_invitee_may_accept_a_friendship_request() {
    let g = pair_group(1, 2, 0);
    let p = project(&[
        request("r0", 1, 2, 0, &[], false),
        join("j0", 3, g, &["r0"]),
    ]);
    assert_eq!(outcome(&p, "j0"), Outcome::Refused(Refusal::NotInvited));
    assert!(!p.friends(&key(1), &key(2)));
}

#[test]
fn test_a_friendship_request_must_name_its_pair_and_counter() {
    let wrong_group = Event {
        id: id("r0"),
        author: key(1),
        after: vec![],
        group: Some(id("not a friendship")),
        body: Body::Create(Create {
            template: Template::Friendship,
            policy: Policy::friendship(),
            members: vec![(key(2), Role::Manage)],
            admins_on_create: false,
        }),
    };
    let p = project(&[wrong_group]);
    assert_eq!(
        outcome(&p, "r0"),
        Outcome::Refused(Refusal::WrongFriendshipId)
    );

    // A request for counter 1 that does not see a dissolve is refused.
    let g = pair_group(1, 2, 0);
    let p = project(&[
        request("r0", 1, 2, 0, &[], false),
        join("a0", 2, g, &["r0"]),
        request("r9", 1, 2, 1, &["a0"], false),
    ]);
    assert_eq!(
        outcome(&p, "r9"),
        Outcome::Refused(Refusal::WrongFriendshipId)
    );
}

// ----- Friendship -----

#[test]
fn test_a_removal_by_either_side_dissolves_the_friendship() {
    let g = pair_group(1, 2, 0);
    for (by, other) in [(1u8, 2u8), (2, 1)] {
        let p = project(&[
            request("r0", 1, 2, 0, &[], false),
            join("a0", 2, g, &["r0"]),
            remove("x", by, g, other, &["a0"]),
        ]);
        assert_eq!(outcome(&p, "x"), Outcome::Applied);
        assert!(!p.friends(&key(1), &key(2)));
        assert_eq!(p.group(&g).unwrap().dissolved_by, Some(id("x")));
    }
}

#[test]
fn test_a_removal_concurrent_with_the_accept_dissolves_it_in_every_order() {
    // F3 (S2 friendship). The removal and the accept are concurrent. In every order the
    // friendship ends.
    let g = pair_group(1, 2, 0);
    let events = [
        request("r0", 1, 2, 0, &[], false),
        join("a0", 2, g, &["r0"]),
        remove("x", 1, g, 2, &["r0"]),
    ];
    let p = project(&events);
    assert_eq!(outcome(&p, "x"), Outcome::Applied);
    assert!(!p.friends(&key(1), &key(2)));
    assert!(p.group(&g).unwrap().dissolved_by.is_some());

    let mut reversed = events.to_vec();
    reversed.reverse();
    assert_eq!(project(&reversed), p);
}

#[test]
fn test_mutual_removal_dissolves_the_friendship_once() {
    // F4 (S2 friendship). Both sides remove. One removal dissolves the friendship, and the other
    // finds it already dissolved.
    let g = pair_group(1, 2, 0);
    let p = project(&[
        request("r0", 1, 2, 0, &[], false),
        join("a0", 2, g, &["r0"]),
        remove("x", 1, g, 2, &["a0"]),
        remove("y", 2, g, 1, &["a0"]),
    ]);
    // Which removal comes first is decided by event ID. The other is refused as dissolved.
    let applied = ["x", "y"]
        .iter()
        .filter(|name| outcome(&p, name) == Outcome::Applied)
        .count();
    assert_eq!(applied, 1);
    for name in ["x", "y"] {
        let result = outcome(&p, name);
        assert!(
            result == Outcome::Applied || result == Outcome::Refused(Refusal::Dissolved),
            "{name} was {result:?}"
        );
    }
    assert!(!p.friends(&key(1), &key(2)));
}

#[test]
fn test_a_dissolved_friendship_is_not_revived_by_its_own_accept_in_any_order() {
    // F5 (S2 friendship). The accept is concurrent with the removal that dissolves the friendship.
    // Whichever comes first, the friendship ends. The accept's outcome is decided by the order
    // (removal and voiding are the voiding track's rules).
    let g = pair_group(1, 2, 0);
    let events = [
        request("r0", 1, 2, 0, &[], false),
        remove("x", 1, g, 2, &["r0"]),
        join("a0", 2, g, &["r0"]),
    ];
    let p = project(&events);
    assert!(!p.friends(&key(1), &key(2)));
    assert!(p.group(&g).unwrap().dissolved_by.is_some());
    let mut reversed = events.to_vec();
    reversed.reverse();
    assert_eq!(project(&reversed), p);
}

#[test]
fn test_re_friending_creates_a_new_group_with_the_counter_incremented() {
    // F5, the other half (S2 friendship). After a dissolve, a new request is open, and accepting
    // it makes a new friendship in a new group.
    let f0 = pair_group(1, 2, 0);
    let f1 = pair_group(1, 2, 1);
    assert_ne!(f0, f1);
    let p = project(&[
        request("r0", 1, 2, 0, &[], false),
        join("a0", 2, f0, &["r0"]),
        remove("x", 1, f0, 2, &["a0"]),
        request("r2", 1, 2, 1, &["x"], false),
        join("a3", 2, f1, &["r2"]),
    ]);
    assert_eq!(outcome(&p, "r2"), Outcome::Applied);
    assert_eq!(outcome(&p, "a3"), Outcome::Applied);
    assert!(p.friends(&key(1), &key(2)));
    assert_eq!(p.group(&f0).unwrap().dissolved_by, Some(id("x")));
}

#[test]
fn test_concurrent_requests_for_one_pair_collapse_into_one_friendship() {
    // The design's collapse (§5.4): a request in each direction is one friendship.
    let g = pair_group(1, 2, 0);
    let events = [
        request("r0", 1, 2, 0, &[], false),
        request("r1", 2, 1, 0, &[], false),
    ];
    let p = project(&events);
    assert_eq!(outcome(&p, "r0"), Outcome::Applied);
    assert_eq!(outcome(&p, "r1"), Outcome::Applied);
    let friendships = p
        .groups()
        .filter(|(_, view)| view.friendship == Pair::of(key(1), key(2)))
        .count();
    assert_eq!(friendships, 1);
    assert!(p.friends(&key(1), &key(2)));
    assert_eq!(p.group(&g).unwrap().present.len(), 2);

    let mut reversed = events.to_vec();
    reversed.reverse();
    assert_eq!(project(&reversed), p);
}

#[test]
fn test_a_friendship_has_no_cap_beyond_its_two_and_no_third_member() {
    let g = pair_group(1, 2, 0);
    let p = project(&[
        request("r0", 1, 2, 0, &[], false),
        join("a0", 2, g, &["r0"]),
        add("a1", 1, g, 3, Role::Read, &["a0"]),
    ]);
    assert_eq!(
        outcome(&p, "a1"),
        Outcome::Refused(Refusal::OverCap { cap: 2 })
    );
}

// ----- Delegation (P2.5, design §7) -----

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

fn veto(name: &str, by: u8, group: Hash, grant: &str, after_names: &[&str]) -> Event {
    event(
        name,
        by,
        after_names,
        Some(group),
        Body::Veto { grant: id(grant) },
    )
}

fn remove_to(
    name: &str,
    by: u8,
    group: Hash,
    who: u8,
    successor: Option<u8>,
    after_names: &[&str],
) -> Event {
    member(
        name,
        by,
        group,
        MemberOp::Remove {
            member: key(who),
            successor: successor.map(key),
        },
        after_names,
    )
}

fn with_mode(mode: Option<Delegation>) -> Policy {
    Policy {
        delegation: mode,
        ..Policy::default()
    }
}

/// A group by 1, with 2 and 3 as Write members and a chain 1 -> 2 -> 3 of grants that may be
/// passed on. A Write member is not a manager, so its own grants need the chain. The mode tests
/// probe with a grant from 3 to 4, and the grants from 2 are the ones a removal of 2 touches.
fn chain_base(mode: Option<Delegation>) -> Vec<Event> {
    let g = id("g");
    vec![
        custom("g", 1, &[], with_mode(mode)),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        add("a1", 1, g, 3, Role::Write, &["a0"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a1"]),
        delegate("d1", 2, g, 3, Role::Write, true, &["d0"]),
    ]
}

#[test]
fn test_a_group_without_a_delegation_setting_cascades() {
    assert_eq!(
        Policy::default().delegation_mode(),
        Delegation::Cascade,
        "cascade is the default"
    );
    assert_eq!(
        with_mode(Some(Delegation::KeepPrior)).delegation_mode(),
        Delegation::KeepPrior
    );
}

fn with_removal(removal: Removal, mode: Option<Delegation>) -> Policy {
    Policy {
        removal,
        delegation: mode,
        ..Policy::default()
    }
}

fn device_add(
    name: &str,
    by: u8,
    group: Hash,
    device: u8,
    scope: Role,
    after_names: &[&str],
) -> Event {
    event(
        name,
        by,
        after_names,
        Some(group),
        Body::Device(DeviceOp::Add {
            device: key(device),
            scope,
        }),
    )
}

/// Design §6.4's week-late case, adapted to weak remove (§6.3): Joe (2) grants Write to Dan (3)
/// without having seen his own removal. `keep_prior` makes the grant a fact about the past, so the
/// removal's strength alone decides whether the concurrent grant stands.
fn late_grant_events(removal: Removal) -> Vec<Event> {
    let g = id("g");
    vec![
        custom(
            "g",
            1,
            &[],
            with_removal(removal, Some(Delegation::KeepPrior)),
        ),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        add("a1", 1, g, 3, Role::Write, &["a0"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a1"]),
        delegate("d1", 2, g, 3, Role::Write, true, &["d0"]),
        remove("r0", 1, g, 2, &["a0"]),
    ]
}

#[test]
fn test_a_concurrent_grant_stands_under_weak_remove() {
    let weak = project(&late_grant_events(Removal::Weak));
    assert_eq!(outcome(&weak, "d1"), Outcome::Applied);
    assert!(
        weak.grant_in_force(&id("d1")),
        "weak remove stands a grant concurrent with it"
    );
    assert!(role_of(&weak, id("g"), 2).is_none(), "2 is removed");
}

#[test]
fn test_a_concurrent_grant_is_void_under_strong_remove() {
    let strong = project(&late_grant_events(Removal::Strong));
    assert!(
        !strong.grant_in_force(&id("d1")),
        "strong remove voids a grant concurrent with it, whether the fold refuses it at the head or \
         applies it and voids the link"
    );
}

#[test]
fn test_weak_remove_still_invalidates_events_after_the_removal() {
    let g = id("g");
    let mut events = late_grant_events(Removal::Weak);
    events.push(delegate("d2", 2, g, 4, Role::Read, false, &["r0"]));
    let weak = project(&events);
    assert_eq!(outcome(&weak, "r0"), Outcome::Applied);
    assert_eq!(
        outcome(&weak, "d2"),
        Outcome::Refused(Refusal::NotActive),
        "an event by the removed member in the removal's causal future is invalid"
    );
}

/// Joe (2) authorizes device 50 without having seen his own removal, and is then re-added. The
/// device grant is concurrent with the removal, so only the removal's strength decides it.
fn late_device_events(removal: Removal) -> Vec<Event> {
    let g = id("g");
    vec![
        custom("g", 1, &[], with_removal(removal, None)),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        device_add("dv0", 2, g, 50, Role::Write, &["a0"]),
        remove("r0", 1, g, 2, &["a0"]),
        add("a1", 1, g, 2, Role::Write, &["r0"]),
    ]
}

#[test]
fn test_a_concurrent_device_grant_stands_under_weak_remove() {
    let g = id("g");
    let weak = project(&late_device_events(Removal::Weak));
    assert!(
        weak.active_devices(&g, &key(2)).contains(&key(50)),
        "weak remove stands a device grant concurrent with it"
    );
}

#[test]
fn test_a_concurrent_device_grant_is_void_under_strong_remove() {
    let g = id("g");
    let strong = project(&late_device_events(Removal::Strong));
    assert!(
        strong.active_devices(&g, &key(2)).is_empty(),
        "strong remove voids a device grant concurrent with it"
    );
}

/// A device-authored grant is judged through the device's account: Joe (2) authorizes device
/// 50, the device grants Write to Dan (3), and Joe's removal is concurrent with both. Under
/// weak remove the account's standing at the grant's position resolves the device, so the
/// grant stands (design 6.3); the device-resolution gate must not read head membership.
#[test]
fn test_a_device_authored_grant_stands_under_weak_remove() {
    let g = id("g");
    let events = vec![
        custom(
            "g",
            1,
            &[],
            with_removal(Removal::Weak, Some(Delegation::KeepPrior)),
        ),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        add("a1", 1, g, 3, Role::Write, &["a0"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a1"]),
        device_add("dv0", 2, g, 50, Role::Write, &["d0"]),
        remove("r0", 1, g, 2, &["a0"]),
        delegate("dv1", 50, g, 3, Role::Write, false, &["dv0", "d0"]),
    ];
    let weak = project(&events);
    assert_eq!(outcome(&weak, "dv1"), Outcome::Applied);
    assert!(
        weak.grant_in_force(&id("dv1")),
        "weak remove stands a device-authored grant concurrent with the account's removal"
    );
}

/// An Admin authorizes an Admin-scoped device without having seen their own removal. Under
/// weak remove the Admin status at the grant's position is what counts, so the device grant
/// stands (design 6.3); the current record has already struck the author from its admins.
#[test]
fn test_an_admin_scoped_device_grant_stands_under_weak_remove() {
    let g = id("g");
    let events = vec![
        custom("g", 1, &[], with_removal(Removal::Weak, None)),
        add("a0", 1, g, 2, Role::Admin, &["g"]),
        device_add("dv0", 2, g, 50, Role::Admin, &["a0"]),
        remove("r0", 1, g, 2, &["a0"]),
        add("a1", 1, g, 2, Role::Write, &["r0"]),
    ];
    let weak = project(&events);
    assert_eq!(outcome(&weak, "dv0"), Outcome::Applied);
    assert!(
        weak.active_devices(&g, &key(2)).contains(&key(50)),
        "weak remove stands an Admin-scoped device grant concurrent with the Admin's removal"
    );
}

#[test]
fn test_keep_prior_lets_a_grant_made_before_the_removal_stand() {
    let g = id("g");
    let mut events = chain_base(Some(Delegation::KeepPrior));
    events.push(remove_to("x0", 1, g, 2, None, &["d1"]));
    events.push(delegate("d2", 3, g, 4, Role::Read, false, &["x0"]));
    let p = project(&events);
    assert_eq!(outcome(&p, "x0"), Outcome::Applied);
    assert_eq!(outcome(&p, "d2"), Outcome::Applied, "d1 was made before x0");
    assert!(p.grant_in_force(&id("d1")));
}

#[test]
fn test_cascade_removes_the_grants_the_removed_member_made() {
    let g = id("g");
    let mut events = chain_base(None);
    events.push(remove_to("x0", 1, g, 2, None, &["d1"]));
    events.push(delegate("d2", 3, g, 4, Role::Read, false, &["x0"]));
    let p = project(&events);
    assert_eq!(outcome(&p, "x0"), Outcome::Applied);
    assert_eq!(outcome(&p, "d2"), Outcome::Refused(Refusal::NoChain));
    assert!(!p.grant_in_force(&id("d1")));
    assert!(p.grant_in_force(&id("d0")), "the owner's grant to 2 stands");
}

#[test]
fn test_cascade_keeps_a_grant_for_an_action_concurrent_with_the_removal() {
    // d2 names only d1, so it is concurrent with x0. At its position 2 still holds Write.
    let g = id("g");
    let mut events = chain_base(None);
    events.push(remove_to("x0", 1, g, 2, None, &["d1"]));
    events.push(delegate("d2", 3, g, 4, Role::Read, false, &["d1"]));
    let p = project(&events);
    assert_eq!(outcome(&p, "d2"), Outcome::Applied);
}

#[test]
fn test_transfer_hands_the_removed_members_grants_to_the_successor() {
    let g = id("g");
    let mut events = chain_base(Some(Delegation::Transfer));
    events.push(add("a2", 1, g, 5, Role::Write, &["a1"]));
    events.push(delegate("d2", 1, g, 5, Role::Write, true, &["a2"]));
    events.push(remove_to("x0", 1, g, 2, Some(5), &["d1", "d2"]));
    events.push(delegate("d3", 3, g, 4, Role::Read, false, &["x0"]));
    let p = project(&events);
    assert_eq!(outcome(&p, "d3"), Outcome::Applied, "5 reissues d1");
    assert!(
        !p.grant_in_force(&id("d1")),
        "the original grant is superseded"
    );
}

#[test]
fn test_transfer_without_a_successor_ends_the_grants() {
    let g = id("g");
    let mut events = chain_base(Some(Delegation::Transfer));
    events.push(remove_to("x0", 1, g, 2, None, &["d1"]));
    events.push(delegate("d2", 3, g, 4, Role::Read, false, &["x0"]));
    let p = project(&events);
    assert_eq!(outcome(&p, "d2"), Outcome::Refused(Refusal::NoChain));
}

#[test]
fn test_a_successor_must_hold_the_scope_to_take_over() {
    // 5 is granted Write by the owner, but its membership is Read, so it cannot hold Write.
    let g = id("g");
    let mut events = chain_base(Some(Delegation::Transfer));
    events.push(add("a2", 1, g, 5, Role::Read, &["a1"]));
    events.push(delegate("d2", 1, g, 5, Role::Write, true, &["a2"]));
    events.push(remove_to("x0", 1, g, 2, Some(5), &["d1", "d2"]));
    events.push(delegate("d3", 3, g, 4, Role::Read, false, &["x0"]));
    let p = project(&events);
    assert_eq!(outcome(&p, "d3"), Outcome::Refused(Refusal::NoChain));
}

#[test]
fn test_a_removed_member_may_not_grant_after_its_removal() {
    let g = id("g");
    let mut events = chain_base(None);
    events.push(remove_to("x0", 1, g, 2, None, &["d1"]));
    events.push(delegate("d2", 2, g, 4, Role::Read, false, &["x0"]));
    let p = project(&events);
    assert!(matches!(outcome(&p, "d2"), Outcome::Refused(_)));
    assert!(!p.grant_in_force(&id("d2")));
}

#[test]
fn test_a_grant_may_not_be_wider_than_the_grantors_scope() {
    let g = id("g");
    let p = project(&[
        custom("g", 1, &[], Policy::default()),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a0"]),
        delegate("d1", 2, g, 3, Role::Manage, true, &["d0"]),
        delegate("d2", 2, g, 3, Role::Read, false, &["d0"]),
    ]);
    assert_eq!(outcome(&p, "d1"), Outcome::Refused(Refusal::Attenuated));
    assert_eq!(outcome(&p, "d2"), Outcome::Applied, "Read is within Write");
}

#[test]
fn test_a_grant_that_cannot_be_redelegated_ends_the_chain() {
    let g = id("g");
    let p = project(&[
        custom("g", 1, &[], Policy::default()),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        delegate("d0", 1, g, 2, Role::Write, false, &["a0"]),
        delegate("d1", 2, g, 3, Role::Read, false, &["d0"]),
    ]);
    assert_eq!(outcome(&p, "d1"), Outcome::Refused(Refusal::NoChain));
    assert!(p.grant_in_force(&id("d0")));
}

#[test]
fn test_chain_search_accepts_a_valid_chain_and_refuses_a_broken_one() {
    let g = id("g");
    let members = |events: &mut Vec<Event>| {
        events.push(custom("g", 1, &[], Policy::default()));
        events.push(add("a0", 1, g, 2, Role::Write, &["g"]));
        events.push(add("a1", 1, g, 3, Role::Write, &["a0"]));
        events.push(add("a2", 1, g, 4, Role::Write, &["a1"]));
    };
    // 1 -> 2 -> 3 -> 4 -> 5. Every link may be passed on, so 4 can grant to 5.
    let mut valid = Vec::new();
    members(&mut valid);
    valid.push(delegate("d0", 1, g, 2, Role::Write, true, &["a2"]));
    valid.push(delegate("d1", 2, g, 3, Role::Write, true, &["d0"]));
    valid.push(delegate("d2", 3, g, 4, Role::Write, true, &["d1"]));
    valid.push(delegate("d3", 4, g, 5, Role::Read, false, &["d2"]));
    let p = project(&valid);
    assert_eq!(outcome(&p, "d3"), Outcome::Applied, "the chain holds");

    // The last link cannot be passed on, so 4 cannot grant.
    let mut last = Vec::new();
    members(&mut last);
    last.push(delegate("d0", 1, g, 2, Role::Write, true, &["a2"]));
    last.push(delegate("d1", 2, g, 3, Role::Write, true, &["d0"]));
    last.push(delegate("d2", 3, g, 4, Role::Write, false, &["d1"]));
    last.push(delegate("d3", 4, g, 5, Role::Read, false, &["d2"]));
    let p = project(&last);
    assert_eq!(outcome(&p, "d3"), Outcome::Refused(Refusal::NoChain));

    // The middle link cannot be passed on: 3 cannot grant, so 4 has no chain.
    let mut middle = Vec::new();
    members(&mut middle);
    middle.push(delegate("d0", 1, g, 2, Role::Write, true, &["a2"]));
    middle.push(delegate("d1", 2, g, 3, Role::Write, false, &["d0"]));
    middle.push(delegate("d2", 3, g, 4, Role::Write, true, &["d1"]));
    let p = project(&middle);
    assert_eq!(outcome(&p, "d2"), Outcome::Refused(Refusal::NoChain));
}

#[test]
fn test_a_manager_added_by_another_member_is_a_root_and_its_grants_are_valid() {
    // Design §7.2: roots are the owners or managers at the action's position. 2 is a manager
    // added by 1, so it is a root there: it may manage, and its grant is valid.
    let g = id("g");
    let p = project(&[
        custom("g", 1, &[], Policy::default()),
        add("a0", 1, g, 2, Role::Manage, &["g"]),
        add("a1", 2, g, 4, Role::Read, &["a0"]),
        delegate("d0", 2, g, 3, Role::Write, true, &["a1"]),
    ]);
    assert_eq!(outcome(&p, "a1"), Outcome::Applied);
    assert_eq!(outcome(&p, "d0"), Outcome::Applied);
    assert!(p.grant_in_force(&id("d0")));
}

#[test]
fn test_a_manager_may_veto_a_grant_and_the_grants_below_it_fall() {
    let g = id("g");
    let mut events = chain_base(None);
    events.push(add("a2", 1, g, 4, Role::Manage, &["a1"]));
    events.push(veto("v0", 4, g, "d0", &["d1", "a2"]));
    events.push(delegate("d2", 3, g, 5, Role::Read, false, &["v0"]));
    events.push(delegate("d3", 3, g, 6, Role::Read, false, &["d1"]));
    events.push(delegate("d4", 4, g, 7, Role::Read, false, &["v0"]));
    let p = project(&events);
    assert_eq!(outcome(&p, "v0"), Outcome::Applied);
    assert_eq!(
        outcome(&p, "d2"),
        Outcome::Refused(Refusal::NoChain),
        "3 reaches the owner only through d0"
    );
    assert_eq!(
        outcome(&p, "d3"),
        Outcome::Applied,
        "before the veto, d0 stood"
    );
    assert_eq!(
        outcome(&p, "d4"),
        Outcome::Applied,
        "4 is a manager, a root at its position"
    );
    assert!(!p.grant_in_force(&id("d0")));
}

#[test]
fn test_a_veto_needs_manage_a_grant_it_has_seen() {
    let g = id("g");
    let p = project(&[
        custom("g", 1, &[], Policy::default()),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        add("a1", 1, g, 3, Role::Manage, &["a0"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a1"]),
        delegate("d5", 3, g, 4, Role::Read, false, &["a1"]),
        veto("v0", 2, g, "d0", &["d0"]),
        veto("v1", 3, g, "d0", &["d0"]),
        veto("v2", 3, g, "d5", &["d0"]),
        veto("v3", 3, g, "d9", &["d0"]),
    ]);
    assert_eq!(outcome(&p, "v0"), Outcome::Refused(Refusal::NotManager));
    assert_eq!(outcome(&p, "v1"), Outcome::Applied);
    assert_eq!(
        outcome(&p, "v2"),
        Outcome::Refused(Refusal::UnknownGrant),
        "d5 is concurrent with d0, so the veto has not seen it"
    );
    assert_eq!(
        outcome(&p, "v3"),
        Outcome::Refused(Refusal::UnknownGrant),
        "d9 does not exist"
    );
}

/// The S2 spike's delegation DAG (`spikes/shared/s2-auth/src/delegation.rs`), with the owner 1,
/// Joe 2, Dan 3, Jim 4, Zed 5, Yan 6, Quinn 7, Eve 8, Kim 9, Lou 10 and Xav 11. Joe is removed by
/// s9. `alt` adds Dan's grant to Jim (s16). `successor` is named by the removal.
fn s2_events(alt: bool, mode: Option<Delegation>, successor: Option<u8>) -> Vec<Event> {
    let g = id("s0");
    let mut events = vec![
        custom("s0", 1, &[], with_mode(mode)),
        add("s1", 1, g, 2, Role::Manage, &["s0"]),
        add("s2", 1, g, 3, Role::Manage, &["s1"]),
        delegate("s3", 1, g, 2, Role::Manage, true, &["s1"]),
        delegate("s4", 1, g, 3, Role::Manage, true, &["s2"]),
        add("s5", 2, g, 4, Role::Manage, &["s3"]),
        delegate("s6", 2, g, 4, Role::Manage, true, &["s5"]),
        add("s7", 1, g, 7, Role::Manage, &["s2"]),
        delegate("s8", 2, g, 7, Role::Manage, true, &["s5"]),
    ];
    let mut removal_after = vec!["s6", "s4"];
    if alt {
        events.push(delegate("s16", 3, g, 4, Role::Manage, true, &["s4"]));
        removal_after.push("s16");
    }
    events.push(remove_to("s9", 1, g, 2, successor, &removal_after));
    events.push(add("s10", 4, g, 5, Role::Manage, &["s9"]));
    events.push(add("s11", 4, g, 6, Role::Manage, &["s6"]));
    events.push(add("s12", 7, g, 8, Role::Read, &["s7", "s8"]));
    events.push(add("s13", 1, g, 9, Role::Write, &["s2"]));
    events.push(delegate("s14", 9, g, 10, Role::Manage, true, &["s13"]));
    events.push(delegate("s15", 2, g, 11, Role::Manage, true, &["s9"]));
    events
}

#[test]
fn test_s2_delegation_scenarios_keep_their_results() {
    // (alt chain, mode, successor, Joe to Jim stands at the head). Jim (4) and Quinn (7) are
    // managers, so they are roots at their positions (design §7.2). Their later actions apply in
    // every mode. S2 had them vetoed, because it made only the owner a root. The mode still
    // decides the grant from Joe, which is the table's last column.
    let g = id("s0");
    let runs = [
        (false, None, None, false),
        (true, None, None, false),
        (false, Some(Delegation::KeepPrior), None, true),
        (false, Some(Delegation::Transfer), Some(3), false),
    ];
    for (alt, mode, successor, joe_to_jim_stands) in runs {
        let name = format!("alt {alt}, mode {mode:?}, successor {successor:?}");
        let p = project(&s2_events(alt, mode, successor));
        assert_eq!(
            outcome(&p, "s10"),
            Outcome::Applied,
            "{name}: Jim's later add"
        );
        assert_eq!(outcome(&p, "s11"), Outcome::Applied, "{name}: concurrent");
        assert_eq!(outcome(&p, "s12"), Outcome::Applied, "{name}: Quinn's add");
        assert!(!p.grant_in_force(&id("s8")), "{name}: void");
        assert_eq!(
            p.grant_in_force(&id("s6")),
            joe_to_jim_stands,
            "{name}: Joe to Jim"
        );
        let members = &p.group(&g).unwrap().members;
        assert!(!members.contains_key(&key(2)), "{name}: Joe is removed");
        assert!(members.contains_key(&key(5)), "{name}: Zed");
    }
    let p = project(&s2_events(false, None, None));
    assert_eq!(outcome(&p, "s14"), Outcome::Refused(Refusal::Attenuated));
    assert!(matches!(outcome(&p, "s15"), Outcome::Refused(_)));
    assert!(p.grant_in_force(&id("s3")));
}

/// The causal past of each event, the event included, from the `after` links.
fn pasts(events: &[Event]) -> BTreeMap<Hash, BTreeSet<Hash>> {
    let by_id: BTreeMap<Hash, &Event> = events.iter().map(|e| (e.id, e)).collect();
    events
        .iter()
        .map(|e| {
            let mut past = BTreeSet::new();
            let mut stack = vec![e.id];
            while let Some(next) = stack.pop() {
                if past.insert(next)
                    && let Some(event) = by_id.get(&next)
                {
                    stack.extend(event.after.iter().copied());
                }
            }
            (e.id, past)
        })
        .collect()
}

/// Random delegation scripts: groups in four modes, managers, grants, removals with an optional
/// successor, and vetoes. Each step is (kind, author, target, scope, pick, mask).
fn delegation_script() -> impl Strategy<Value = Vec<(u8, u8, u8, u8, u8, u8)>> {
    prop::collection::vec(
        (0u8..8, 1u8..=4, 1u8..=4, 0u8..4, any::<u8>(), any::<u8>()),
        1..24,
    )
}

fn delegation_build(script: &[(u8, u8, u8, u8, u8, u8)]) -> Vec<Event> {
    const ROLES: [Role; 4] = [Role::Pull, Role::Read, Role::Write, Role::Manage];
    const MODES: [Option<Delegation>; 4] = [
        None,
        Some(Delegation::Cascade),
        Some(Delegation::KeepPrior),
        Some(Delegation::Transfer),
    ];
    let mut events: Vec<Event> = Vec::new();
    let mut groups: Vec<Hash> = Vec::new();
    for (i, &(raw, author, target, scope, pick, mask)) in script.iter().enumerate() {
        // The first event creates a group. After it, adds and grants are the common steps.
        let kind = match raw {
            _ if i == 0 => 0,
            0 => 0,
            1 | 2 => 1,
            3..=5 => 2,
            6 => 3,
            _ => 4,
        };
        let name = format!("e{i}");
        let after_hashes: Vec<Hash> = events
            .iter()
            .enumerate()
            .filter(|(j, _)| (mask >> (j % 8)) & 1 == 1)
            .map(|(_, earlier)| earlier.id)
            .rev()
            .take(3)
            .collect();
        let group = groups
            .get(usize::from(pick) % groups.len().max(1))
            .copied()
            .unwrap_or_else(|| id("none"));
        let grants: Vec<Hash> = events
            .iter()
            .filter(|e| matches!(e.body, Body::Delegate(_)))
            .map(|e| e.id)
            .collect();
        let grant = grants
            .get(usize::from(pick) % grants.len().max(1))
            .copied()
            .unwrap_or_else(|| id("none"));
        let role = ROLES[usize::from(scope)];
        let (group, body) = match kind {
            0 => {
                let policy = with_mode(MODES[usize::from(pick % 4)]);
                let body = Body::Create(Create {
                    template: Template::Custom,
                    policy,
                    members: Vec::new(),
                    admins_on_create: false,
                });
                (None, body)
            }
            1 => (
                Some(group),
                Body::Member(MemberOp::Add {
                    member: key(target),
                    role,
                }),
            ),
            2 => (
                Some(group),
                Body::Delegate(Delegate {
                    grantee: key(target),
                    scope: role,
                    can_redelegate: mask % 2 == 0,
                }),
            ),
            3 => (
                Some(group),
                Body::Member(MemberOp::Remove {
                    member: key(target),
                    successor: (mask % 3 == 0).then(|| key(pick % 5 + 1)),
                }),
            ),
            _ => (Some(group), Body::Veto { grant }),
        };
        let created = Event {
            id: id(&name),
            author: key(author),
            after: after_hashes,
            group,
            body,
        };
        if matches!(created.body, Body::Create(_)) {
            groups.push(created.id);
        }
        events.push(created);
    }
    events
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_delegation_outcomes_do_not_depend_on_the_arrival_order(
        script in delegation_script(),
        seed in any::<u64>(),
    ) {
        let events = delegation_build(&script);
        let expected = project(&events);
        let shuffled_state = fold(&shuffled(&events, seed.max(1))).unwrap();
        prop_assert_eq!(&shuffled_state, &expected);
        let mut reversed = events;
        reversed.reverse();
        prop_assert_eq!(&fold(&reversed).unwrap(), &expected);
    }

    #[test]
    fn prop_no_grant_survives_its_grantors_strong_removal_unless_the_mode_keeps_it(
        script in delegation_script(),
    ) {
        let events = delegation_build(&script);
        let p = project(&events);
        let pasts = pasts(&events);
        let grants = events.iter().filter(|e| {
            matches!(e.body, Body::Delegate(_)) && p.outcome(&e.id) == Some(&Outcome::Applied)
        });
        for grant in grants {
            let (Some(group), Body::Delegate(delegate)) = (grant.group, &grant.body) else {
                continue;
            };
            let view = p.group(&group).unwrap();
            let grantor = grant.author;
            let removals: Vec<&Event> = events
                .iter()
                .filter(|e| {
                    e.group == Some(group)
                        && matches!(e.body, Body::Member(MemberOp::Remove { member, .. }) if member == grantor)
                        && p.takes_effect(&e.id)
                })
                .collect();
            let vetoed = events.iter().any(|v| {
                matches!(v.body, Body::Veto { grant: target } if target == grant.id)
                    && p.takes_effect(&v.id)
            });
            let in_force = p.grant_in_force(&grant.id);
            if vetoed {
                prop_assert!(!in_force, "a vetoed grant is not in force");
            }
            let concurrent = removals.iter().any(|r| {
                !pasts[&grant.id].contains(&r.id) && !pasts[&r.id].contains(&grant.id)
            });
            if concurrent {
                prop_assert!(!in_force, "a grant concurrent with its grantor's removal is void");
            }
            match view.policy.delegation_mode() {
                Delegation::Cascade => {
                    let holds_at_head = view
                        .members
                        .get(&grantor)
                        .is_some_and(|role| *role >= delegate.scope);
                    if !removals.is_empty() && !holds_at_head {
                        prop_assert!(!in_force, "cascade ends a grant its grantor no longer holds");
                    }
                }
                Delegation::Transfer => {
                    if !removals.is_empty() {
                        prop_assert!(!in_force, "transfer supersedes the grantor's grants");
                    }
                }
                Delegation::KeepPrior => {
                    let before_every_removal =
                        removals.iter().all(|r| pasts[&r.id].contains(&grant.id));
                    if !vetoed && before_every_removal {
                        prop_assert!(in_force, "keep_prior keeps a grant made before the removal");
                    }
                }
            }
        }
    }
}

// ----- Properties over arrival order -----

/// A random set of group events. Each step picks a kind, the actors, a group among those
/// already named, and an `after` set drawn from the earlier events.
fn script() -> impl Strategy<Value = Vec<(u8, u8, u8, u8, u8, u8)>> {
    prop::collection::vec(
        (0u8..7, 1u8..=4, 1u8..=4, 0u8..5, 0u8..4, any::<u8>()),
        1..16,
    )
}

fn build(script: &[(u8, u8, u8, u8, u8, u8)]) -> Vec<Event> {
    const ROLES: [Role; 5] = [
        Role::Pull,
        Role::Read,
        Role::Write,
        Role::Manage,
        Role::Admin,
    ];
    let mut events: Vec<Event> = Vec::new();
    // The groups named so far, so that most events name a group that exists.
    let mut groups: Vec<Hash> = Vec::new();
    for (i, &(kind, author, target, role, pick, mask)) in script.iter().enumerate() {
        let name = format!("e{i}");
        let after_hashes: Vec<Hash> = events
            .iter()
            .enumerate()
            .filter(|(j, _)| (mask >> (j % 8)) & 1 == 1)
            .map(|(_, earlier)| earlier.id)
            .rev()
            .take(3)
            .collect();
        let group = groups
            .get(usize::from(pick) % groups.len().max(1))
            .copied()
            .unwrap_or_else(|| id("none"));
        let role = ROLES[usize::from(role)];
        let body_and_group = match kind {
            0 => {
                let cap = if target % 2 == 0 {
                    None
                } else {
                    Some(usize::from(target))
                };
                let policy = Policy {
                    member_cap: cap,
                    ..Policy::default()
                };
                let members = if target == author {
                    vec![]
                } else {
                    vec![(key(target), role)]
                };
                let body = Body::Create(Create {
                    template: Template::Custom,
                    policy,
                    members,
                    admins_on_create: false,
                });
                (None, body)
            }
            1 => {
                let counter = u32::from(pick % 4 == 0);
                let group =
                    Pair::of(key(author), key(target)).map(|pair| friendship_id(&pair, counter));
                let body = Body::Create(Create {
                    template: Template::Friendship,
                    policy: Policy::friendship(),
                    members: vec![(key(target), Role::Manage)],
                    admins_on_create: mask % 2 == 1,
                });
                (group, body)
            }
            2 => (Some(group), Body::Join),
            3 => {
                let policy = Policy {
                    member_cap: if target % 2 == 0 {
                        None
                    } else {
                        Some(usize::from(target))
                    },
                    ..Policy::default()
                };
                (Some(group), Body::Policy(policy))
            }
            4 => (
                Some(group),
                Body::Member(MemberOp::Add {
                    member: key(target),
                    role,
                }),
            ),
            5 => (
                Some(group),
                Body::Member(MemberOp::Remove {
                    member: key(target),
                    successor: None,
                }),
            ),
            _ => (
                Some(group),
                Body::Member(MemberOp::SetRole {
                    member: key(target),
                    role,
                }),
            ),
        };
        let (group, body) = body_and_group;
        let created = Event {
            id: id(&name),
            author: key(author),
            after: after_hashes,
            group,
            body,
        };
        if matches!(created.body, Body::Create(_)) {
            groups.push(created.group.unwrap_or(created.id));
        }
        events.push(created);
    }
    events
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_the_final_state_does_not_depend_on_the_order_the_events_arrive_in(
        script in script(),
        seed in any::<u64>(),
    ) {
        let events = build(&script);
        let expected = project(&events);
        let shuffle = shuffled(&events, seed.max(1));
        let shuffled_state = fold(&shuffle).unwrap();
        prop_assert_eq!(&shuffled_state, &expected);
        let mut reversed = events;
        reversed.reverse();
        prop_assert_eq!(fold(&reversed).unwrap(), expected);
    }

    #[test]
    fn prop_no_group_is_ever_over_its_cap_and_a_friendship_is_active_only_when_both_are_present(
        script in script(),
    ) {
        let p = project(&build(&script));
        for (_, view) in p.groups() {
            if let Some(cap) = view.policy.member_cap {
                prop_assert!(view.members.len() <= cap);
            }
            if view.is_active()
                && let Some(pair) = view.friendship
            {
                prop_assert!(pair.members().iter().all(|k| view.present.contains(k)));
            }
        }
    }
}

// ----- Review of Phase 2 [P2-review] -----

#[test]
fn test_a_manager_who_is_not_an_admin_may_not_remove_an_admin() {
    // Design §5.2: Manage adds and removes members at or below its own level. Admin is above it.
    // An Admin may remove anyone, and any member may leave.
    for remover in [3, 4] {
        let events = vec![
            custom(
                "g",
                1,
                &[(2, Role::Admin), (3, Role::Manage), (4, Role::Manage)],
                Policy::default(),
            ),
            remove("r", remover, id("g"), 2, &["g"]),
        ];
        let p = project(&events);
        assert_eq!(
            outcome(&p, "r"),
            Outcome::Refused(Refusal::NotAdmin),
            "manager {remover} removing an Admin"
        );
        assert_eq!(role_of(&p, id("g"), 2), Some(Role::Admin));
    }
    let events = vec![
        custom(
            "g",
            1,
            &[(2, Role::Admin), (3, Role::Manage)],
            Policy::default(),
        ),
        remove("r", 2, id("g"), 3, &["g"]),
    ];
    let p = project(&events);
    assert_eq!(outcome(&p, "r"), Outcome::Applied);
    assert_eq!(role_of(&p, id("g"), 3), None);
}

#[test]
fn test_a_grant_by_a_member_who_was_re_added_stands_in_every_mode() {
    // 2 is removed and re-added. Its later grant is made by a member who is valid at that
    // position, so it stands. Under transfer it must not be treated as the removed grantor's.
    for mode in [
        Delegation::KeepPrior,
        Delegation::Cascade,
        Delegation::Transfer,
    ] {
        let policy = Policy {
            delegation: Some(mode),
            ..Policy::default()
        };
        let events = vec![
            custom("g", 1, &[(2, Role::Manage), (3, Role::Manage)], policy),
            remove("r", 1, id("g"), 2, &["g"]),
            add("a", 1, id("g"), 2, Role::Manage, &["r"]),
            delegate("d", 2, id("g"), 4, Role::Read, false, &["a"]),
        ];
        let p = project(&events);
        assert_eq!(outcome(&p, "d"), Outcome::Applied, "{mode:?}");
        assert!(
            p.grant_in_force(&id("d")),
            "{mode:?}: a grant made after the re-add stands"
        );
    }
}

/// Removal-sorts-first variant of the device-authored grant case: the removal's ID
/// (s0) is smaller than the device grant's (dv1), so canonical order applies the
/// removal BEFORE the grant is judged. Under weak remove the grant still stands:
/// its authority is judged at its own causal position (design §6.3).
#[test]
fn test_a_device_authored_grant_stands_under_weak_remove_when_the_removal_sorts_first() {
    let g = id("g");
    let events = vec![
        custom(
            "g",
            1,
            &[],
            with_removal(Removal::Weak, Some(Delegation::KeepPrior)),
        ),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        add("a1", 1, g, 3, Role::Write, &["a0"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a1"]),
        device_add("dv0", 2, g, 50, Role::Write, &["d0"]),
        remove("s0", 1, g, 2, &["a0"]),
        delegate("dv1", 50, g, 3, Role::Write, false, &["dv0", "d0"]),
    ];
    let weak = project(&events);
    assert_eq!(outcome(&weak, "dv1"), Outcome::Applied);
    assert!(
        weak.grant_in_force(&id("dv1")),
        "weak remove stands a device-authored grant even when the removal sorts first"
    );
}

/// Removal-sorts-first variant of the Admin-scoped device grant case: the
/// removal (s0) is applied before the device grant (dv0) in canonical order.
/// Under weak remove the author's Admin status at the grant's causal position
/// is what counts, so the grant stands (design §6.3).
#[test]
fn test_an_admin_scoped_device_grant_stands_under_weak_remove_when_the_removal_sorts_first() {
    let g = id("g");
    let events = vec![
        custom("g", 1, &[], with_removal(Removal::Weak, None)),
        add("a0", 1, g, 2, Role::Admin, &["g"]),
        device_add("dv0", 2, g, 50, Role::Admin, &["a0"]),
        remove("s0", 1, g, 2, &["a0"]),
        add("a1", 1, g, 2, Role::Write, &["s0"]),
    ];
    let weak = project(&events);
    assert_eq!(outcome(&weak, "dv0"), Outcome::Applied);
    assert!(
        weak.active_devices(&g, &key(2)).contains(&key(50)),
        "weak remove stands an Admin-scoped device grant even when the removal sorts first"
    );
}

/// Creation-assigned Admin variant of the removal-sorts-first case: the Admin
/// was named in the group Create (no MemberOp::Add ever names them), so the
/// causal Admin replay in admin_for must seed from the creation event itself.
/// The removal (s0) sorts before the device grant (dv0); under weak remove the
/// grant stands (design 6.3).
#[test]
fn test_an_admin_scoped_device_grant_stands_under_weak_remove_for_a_creation_admin() {
    let g = id("g");
    let events = vec![
        custom(
            "g",
            1,
            &[(2, Role::Admin)],
            with_removal(Removal::Weak, None),
        ),
        device_add("dv0", 2, g, 50, Role::Admin, &["g"]),
        remove("s0", 1, g, 2, &["g"]),
        add("a1", 1, g, 2, Role::Write, &["s0"]),
    ];
    let weak = project(&events);
    assert_eq!(outcome(&weak, "dv0"), Outcome::Applied);
    assert!(
        weak.active_devices(&g, &key(2)).contains(&key(50)),
        "weak remove stands an Admin-scoped device grant for an Admin assigned at creation"
    );
}

/// The Admin-scoped device grant above without a re-add: the grant is judged
/// Applied at its causal position (design §6.3). The account still ends
/// removed, so it has no active devices at the head — that is the documented
/// head-view cascade, not a refusal of the grant.
#[test]
fn test_an_admin_scoped_device_grant_is_applied_under_weak_remove_without_a_readd() {
    let g = id("g");
    let events = vec![
        custom("g", 1, &[], with_removal(Removal::Weak, None)),
        add("a0", 1, g, 2, Role::Admin, &["g"]),
        device_add("dv0", 2, g, 50, Role::Admin, &["a0"]),
        remove("s0", 1, g, 2, &["a0"]),
    ];
    let weak = project(&events);
    assert_eq!(outcome(&weak, "dv0"), Outcome::Applied);
    assert!(
        weak.active_devices(&g, &key(2)).is_empty(),
        "a removed account has no active devices at the head"
    );
}

/// The device-authored delegation case with the removal authored after both
/// member adds (a wider causal past, still concurrent with the delegation).
#[test]
fn test_a_device_authored_grant_stands_under_weak_remove_with_a_wide_removal_past() {
    let g = id("g");
    let events = vec![
        custom(
            "g",
            1,
            &[],
            with_removal(Removal::Weak, Some(Delegation::KeepPrior)),
        ),
        add("a0", 1, g, 2, Role::Write, &["g"]),
        add("a1", 1, g, 3, Role::Write, &["a0"]),
        delegate("d0", 1, g, 2, Role::Write, true, &["a1"]),
        device_add("dv0", 2, g, 50, Role::Write, &["d0"]),
        remove("s0", 1, g, 2, &["a0", "a1"]),
        delegate("dv1", 50, g, 3, Role::Write, false, &["dv0", "d0"]),
    ];
    let weak = project(&events);
    assert_eq!(outcome(&weak, "dv1"), Outcome::Applied);
    assert!(weak.grant_in_force(&id("dv1")));
}
