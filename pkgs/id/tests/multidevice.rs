//! Multi-device capability delegation and revocation integration tests (P8.3).
//!
//! Checks:
//! - Account authorizes Device 1 and Device 2 via `device.add`. Both can act with the account's authority.
//! - Revoking Device 1 via `device.revoke` severs Device 1's authority while Device 2 remains valid.
//! - Removing the account revokes both devices.
//! - Device count helper accurately returns the active device count.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use p2panda_core::Hash;
use tempfile::TempDir;

use id::groups::log::GroupLog;
use id::groups::{
    Body, Create, DEVICE_ADD_KIND, DEVICE_REVOKE_KIND, DeviceOp, Event, Key, MemberOp, Outcome,
    Policy, Refusal, Role, Template, fold,
};

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

fn create_group(name: &str, author: u8, members: &[(u8, Role)]) -> Event {
    let members = members.iter().map(|(n, role)| (key(*n), *role)).collect();
    let body = Body::Create(Create {
        template: Template::Custom,
        policy: Policy::default(),
        members,
        admins_on_create: false,
    });
    event(name, author, &[], None, body)
}

fn device_add(
    name: &str,
    account: u8,
    group: Hash,
    device: u8,
    scope: Role,
    after_names: &[&str],
) -> Event {
    event(
        name,
        account,
        after_names,
        Some(group),
        Body::Device(DeviceOp::Add {
            device: key(device),
            scope,
        }),
    )
}

fn device_revoke(name: &str, account: u8, group: Hash, device: u8, after_names: &[&str]) -> Event {
    event(
        name,
        account,
        after_names,
        Some(group),
        Body::Device(DeviceOp::Revoke {
            device: key(device),
        }),
    )
}

fn member_add(
    name: &str,
    by: u8,
    group: Hash,
    member: u8,
    role: Role,
    after_names: &[&str],
) -> Event {
    event(
        name,
        by,
        after_names,
        Some(group),
        Body::Member(MemberOp::Add {
            member: key(member),
            role,
        }),
    )
}

fn member_remove(name: &str, by: u8, group: Hash, member: u8, after_names: &[&str]) -> Event {
    event(
        name,
        by,
        after_names,
        Some(group),
        Body::Member(MemberOp::Remove {
            member: key(member),
            successor: None,
        }),
    )
}

#[test]
fn test_kind_constants_match_specification() {
    assert_eq!(DEVICE_ADD_KIND, "device.add");
    assert_eq!(DEVICE_REVOKE_KIND, "device.revoke");
}

#[test]
fn test_devices_act_with_account_authority() {
    let g = id("g");
    let events = vec![
        create_group("g", 1, &[]),
        device_add("dev1_add", 1, g, 11, Role::Manage, &["g"]),
        device_add("dev2_add", 1, g, 12, Role::Manage, &["dev1_add"]),
        // Device 1 (key 11) acts with Account 1's authority to add Member 2
        member_add("add_m2", 11, g, 2, Role::Write, &["dev2_add"]),
        // Device 2 (key 12) acts with Account 1's authority to add Member 3
        member_add("add_m3", 12, g, 3, Role::Write, &["add_m2"]),
    ];

    let p = fold(&events).expect("fold succeeds");
    assert_eq!(p.outcome(&id("dev1_add")), Some(&Outcome::Applied));
    assert_eq!(p.outcome(&id("dev2_add")), Some(&Outcome::Applied));
    assert_eq!(
        p.outcome(&id("add_m2")),
        Some(&Outcome::Applied),
        "Device 1 acts with Account 1's authority"
    );
    assert_eq!(
        p.outcome(&id("add_m3")),
        Some(&Outcome::Applied),
        "Device 2 acts with Account 1's authority"
    );

    let view = p.group(&g).expect("group exists");
    assert!(view.members.contains_key(&key(2)));
    assert!(view.members.contains_key(&key(3)));
}

#[test]
fn test_revoking_device_severs_authority_while_other_remains_valid() {
    let g = id("g");
    let events = vec![
        create_group("g", 1, &[]),
        device_add("dev1_add", 1, g, 11, Role::Manage, &["g"]),
        device_add("dev2_add", 1, g, 12, Role::Manage, &["dev1_add"]),
        // Revoke Device 1
        device_revoke("dev1_rev", 1, g, 11, &["dev2_add"]),
        // Device 1 attempts to act after revocation -> refused
        member_add("add_m4_by_dev1", 11, g, 4, Role::Write, &["dev1_rev"]),
        // Device 2 remains valid and acts -> applied
        member_add("add_m5_by_dev2", 12, g, 5, Role::Write, &["dev1_rev"]),
    ];

    let p = fold(&events).expect("fold succeeds");
    assert_eq!(p.outcome(&id("dev1_rev")), Some(&Outcome::Applied));
    assert_eq!(
        p.outcome(&id("add_m4_by_dev1")),
        Some(&Outcome::Refused(Refusal::NotActive)),
        "Revoking Device 1 severs Device 1's authority"
    );
    assert_eq!(
        p.outcome(&id("add_m5_by_dev2")),
        Some(&Outcome::Applied),
        "Device 2 remains valid and authorized"
    );

    let view = p.group(&g).expect("group exists");
    assert!(!view.members.contains_key(&key(4)));
    assert!(view.members.contains_key(&key(5)));
}

#[test]
fn test_removing_account_revokes_both_devices() {
    let g = id("g");
    // Founder 99 creates group and adds Account 1 as Manage
    let events = vec![
        create_group("g", 99, &[(1, Role::Manage)]),
        device_add("dev1_add", 1, g, 11, Role::Manage, &["g"]),
        device_add("dev2_add", 1, g, 12, Role::Manage, &["dev1_add"]),
        // Device 1 successfully adds member 2 before removal
        member_add("add_m2", 11, g, 2, Role::Write, &["dev2_add"]),
        // Founder removes Account 1
        member_remove("rem_acct1", 99, g, 1, &["add_m2"]),
        // Device 1 attempts to act after account removal -> refused
        member_add("add_m3_by_dev1", 11, g, 3, Role::Write, &["rem_acct1"]),
        // Device 2 attempts to act after account removal -> refused
        member_add("add_m4_by_dev2", 12, g, 4, Role::Write, &["rem_acct1"]),
    ];

    let p = fold(&events).expect("fold succeeds");
    assert_eq!(p.outcome(&id("add_m2")), Some(&Outcome::Applied));
    assert_eq!(p.outcome(&id("rem_acct1")), Some(&Outcome::Applied));
    assert_eq!(
        p.outcome(&id("add_m3_by_dev1")),
        Some(&Outcome::Refused(Refusal::NotActive)),
        "Removing account revokes Device 1"
    );
    assert_eq!(
        p.outcome(&id("add_m4_by_dev2")),
        Some(&Outcome::Refused(Refusal::NotActive)),
        "Removing account revokes Device 2"
    );

    let view = p.group(&g).expect("group exists");
    assert!(!view.members.contains_key(&key(1)));
    assert!(view.members.contains_key(&key(2)));
    assert!(!view.members.contains_key(&key(3)));
    assert!(!view.members.contains_key(&key(4)));
}

#[test]
fn test_device_count_helper_accurately_returns_active_device_count() {
    let g = id("g");
    // Baseline: no devices authorized yet
    let base = vec![create_group("g", 1, &[(2, Role::Manage)])];
    let p0 = fold(&base).expect("fold succeeds");
    assert_eq!(p0.active_device_count(&g, &key(1)), 0);
    assert_eq!(p0.group_device_count(&g), 0);

    // Account 1 authorizes Device 1 and Device 2; Account 2 authorizes Device 3
    let mut step1 = base;
    step1.push(device_add("dev1_add", 1, g, 11, Role::Manage, &["g"]));
    step1.push(device_add(
        "dev2_add",
        1,
        g,
        12,
        Role::Manage,
        &["dev1_add"],
    ));
    step1.push(device_add(
        "dev3_add",
        2,
        g,
        21,
        Role::Manage,
        &["dev2_add"],
    ));

    let p1 = fold(&step1).expect("fold succeeds");
    assert_eq!(
        p1.active_device_count(&g, &key(1)),
        2,
        "Account 1 has 2 active devices"
    );
    assert_eq!(
        p1.active_device_count(&g, &key(2)),
        1,
        "Account 2 has 1 active device"
    );
    assert_eq!(
        p1.group_device_count(&g),
        3,
        "Group has 3 active devices in total"
    );

    // Account 1 revokes Device 1
    let mut step2 = step1;
    step2.push(device_revoke("dev1_rev", 1, g, 11, &["dev3_add"]));
    let p2 = fold(&step2).expect("fold succeeds");
    assert_eq!(
        p2.active_device_count(&g, &key(1)),
        1,
        "Account 1 now has 1 active device (Device 2)"
    );
    assert_eq!(
        p2.group_device_count(&g),
        2,
        "Group now has 2 active devices"
    );

    // Account 1 leaves (removes itself) -> cascade severs Account 1's remaining device (Device 2)
    let mut step3 = step2;
    step3.push(member_remove("rem_acct1", 1, g, 1, &["dev1_rev"]));
    let p3 = fold(&step3).expect("fold succeeds");
    assert_eq!(
        p3.active_device_count(&g, &key(1)),
        0,
        "Removed account has 0 active devices"
    );
    assert_eq!(
        p3.active_device_count(&g, &key(2)),
        1,
        "Account 2 still has 1 active device"
    );
    assert_eq!(
        p3.group_device_count(&g),
        1,
        "Group has 1 active device remaining"
    );
}

#[test]
fn test_group_log_multidevice_integration() {
    let dir = TempDir::new().expect("tempdir");
    let log_path = dir.path().join("groups.jsonl");
    let mut log = GroupLog::open(&log_path).expect("open log");

    let author_acct = key(1);
    let dev1 = key(11);
    let dev2 = key(12);

    let create_action = log
        .create(
            author_acct,
            id::groups::actions::NewGroup::Normal {
                max: None,
                admins_on_create: false,
            },
        )
        .expect("create group");
    let group = create_action.group;

    assert_eq!(log.active_device_count(&group, &author_acct).unwrap(), 0);
    assert_eq!(log.group_device_count(&group).unwrap(), 0);

    // Authorize Device 1 and Device 2
    let add1 = log
        .device_add(author_acct, group, dev1, Role::Manage)
        .expect("device_add dev1");
    assert_eq!(add1.outcome, Outcome::Applied);

    let add2 = log
        .device_add(author_acct, group, dev2, Role::Manage)
        .expect("device_add dev2");
    assert_eq!(add2.outcome, Outcome::Applied);

    assert_eq!(log.active_device_count(&group, &author_acct).unwrap(), 2);
    assert_eq!(log.group_device_count(&group).unwrap(), 2);

    // Device 1 adds Member 2
    let add_m2 = log
        .add(dev1, group, key(2), Role::Write)
        .expect("dev1 add member");
    assert_eq!(
        add_m2.outcome,
        Outcome::Applied,
        "Device 1 acts with account authority"
    );

    // Revoke Device 1
    let rev1 = log
        .device_revoke(author_acct, group, dev1)
        .expect("revoke dev1");
    assert_eq!(rev1.outcome, Outcome::Applied);

    assert_eq!(log.active_device_count(&group, &author_acct).unwrap(), 1);
    assert_eq!(log.group_device_count(&group).unwrap(), 1);

    // Device 1 attempting to act is now refused
    let add_m3_by_dev1 = log
        .add(dev1, group, key(3), Role::Write)
        .expect("dev1 add member after revoke");
    assert_eq!(
        add_m3_by_dev1.outcome,
        Outcome::Refused(Refusal::NotActive),
        "Revoked device authority is severed"
    );

    // Device 2 can still act
    let add_m4_by_dev2 = log
        .add(dev2, group, key(4), Role::Write)
        .expect("dev2 add member");
    assert_eq!(
        add_m4_by_dev2.outcome,
        Outcome::Applied,
        "Device 2 remains valid"
    );

    let view = log.view(group).expect("view group");
    assert!(view.members.contains_key(&key(2)));
    assert!(!view.members.contains_key(&key(3)));
    assert!(view.members.contains_key(&key(4)));
}
