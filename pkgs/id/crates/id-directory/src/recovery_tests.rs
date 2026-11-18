//! Rule R tests, named after the guardian recovery design's §7 list.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::keyset::{Log, fold};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public(seed: u8) -> String {
    hex_encode(&key(seed).verifying_key().to_bytes())
}

const OWNER: u8 = 1;
const OWNER_DEVICE: u8 = 2;
const GUARDIANS: [u8; 3] = [11, 12, 13];
const SUCCESSOR: u8 = 50;
const SUCCESSOR_DEVICE: u8 = 51;
const CREATED_AT: u64 = 1_000;

/// A fold of an identity whose genesis key is `seed`, plus `extra`
/// records appended by that key in order.
fn identity(seed: u8, extra: Vec<(KeysetEntry, Option<SigningKey>)>) -> (Log, KeysetState) {
    let admin = key(seed);
    let genesis = KeysetRecord::sign(
        &admin,
        1,
        "",
        KeysetEntry::Genesis {
            admin_key: public(seed),
            device_key: public(seed),
            label: "first".to_owned(),
        },
        None,
    )
    .unwrap();
    let mut log = Log::default();
    log.insert(genesis.clone()).unwrap();
    let mut prev = genesis.clone();
    for (lamport, (entry, counter)) in (2..).zip(extra) {
        let record =
            KeysetRecord::sign(&admin, lamport, prev.hash.clone(), entry, counter.as_ref())
                .unwrap();
        log.insert(record.clone()).unwrap();
        prev = record;
    }
    let state = fold(&log, &genesis.hash).unwrap();
    (log, state)
}

fn guardian_set(seeds: &[u8], threshold: u32) -> KeysetEntry {
    let mut guardians: Vec<String> = seeds.iter().map(|seed| public(*seed)).collect();
    guardians.sort();
    KeysetEntry::GuardianSetUpdated {
        guardians,
        threshold,
    }
}

/// The owner's identity: a second device, then guardians 2-of-3.
fn owner_head() -> KeysetState {
    identity(
        OWNER,
        vec![
            (
                KeysetEntry::DeviceEnrolled {
                    device_key: public(OWNER_DEVICE),
                    label: "phone".to_owned(),
                },
                Some(key(OWNER_DEVICE)),
            ),
            (guardian_set(&GUARDIANS, 2), None),
        ],
    )
    .1
}

fn guardian_folds() -> BTreeMap<String, KeysetState> {
    GUARDIANS
        .iter()
        .map(|seed| (public(*seed), identity(*seed, vec![]).1))
        .collect()
}

fn request(head: &KeysetState, reason: &str) -> AnnexRecord {
    AnnexRecord::sign(
        &key(SUCCESSOR),
        head.head_hash.clone(),
        AnnexEntry::RecoveryRequest {
            identity_id: head.identity_id.clone(),
            keyset_head_hash: head.head_hash.clone(),
            successor_admin_key: public(SUCCESSOR),
            successor_device_key: public(SUCCESSOR_DEVICE),
            created_at: CREATED_AT,
            reason: reason.to_owned(),
        },
    )
    .unwrap()
}

/// An approval signed by `signer` on behalf of guardian identity
/// `guardian`.
fn approval(head: &KeysetState, request: &AnnexRecord, signer: u8, guardian: u8) -> AnnexRecord {
    approval_at(head, request, signer, guardian, 2_000)
}

fn approval_at(
    head: &KeysetState,
    request: &AnnexRecord,
    signer: u8,
    guardian: u8,
    approved_at: u64,
) -> AnnexRecord {
    AnnexRecord::sign(
        &key(signer),
        head.head_hash.clone(),
        AnnexEntry::RecoveryApproval {
            identity_id: head.identity_id.clone(),
            request_id: request.hash.clone(),
            guardian: public(guardian),
            approved_at,
        },
    )
    .unwrap()
}

fn cancellation(head: &KeysetState, request: &AnnexRecord, signer: u8, basis: &str) -> AnnexRecord {
    AnnexRecord::sign(
        &key(signer),
        head.head_hash.clone(),
        AnnexEntry::RecoveryCancellation {
            identity_id: head.identity_id.clone(),
            request_id: request.hash.clone(),
            basis: basis.to_owned(),
        },
    )
    .unwrap()
}

fn rotation(head: &KeysetState, request: &AnnexRecord, approvals: &[&AnnexRecord]) -> KeysetRecord {
    let mut approval_hashes: Vec<String> = approvals.iter().map(|a| a.hash.clone()).collect();
    approval_hashes.sort();
    approval_hashes.dedup();
    KeysetRecord::sign(
        &key(SUCCESSOR),
        head.records_folded + 10,
        head.head_hash.clone(),
        KeysetEntry::RecoveryRotation {
            request_id: request.hash.clone(),
            approval_hashes,
            successor_admin_key: public(SUCCESSOR),
            successor_device_key: public(SUCCESSOR_DEVICE),
        },
        None,
    )
    .unwrap()
}

/// Runs Rule R with the rotation first seen just after the delay.
fn check(
    head: &KeysetState,
    request: &AnnexRecord,
    approvals: &[AnnexRecord],
    cancellations: &[Seen<'_>],
    rotation: &KeysetRecord,
) -> Result<Successor> {
    check_at(
        head,
        request,
        CREATED_AT,
        approvals,
        cancellations,
        rotation,
        CREATED_AT + RECOVERY_DELAY_SECONDS,
        &BTreeSet::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn check_at(
    head: &KeysetState,
    request: &AnnexRecord,
    request_seen: u64,
    approvals: &[AnnexRecord],
    cancellations: &[Seen<'_>],
    rotation: &KeysetRecord,
    rotation_seen: u64,
    consumed: &BTreeSet<String>,
) -> Result<Successor> {
    let folds = guardian_folds();
    check_rotation(&RotationCheck {
        head,
        request: Seen {
            record: request,
            first_seen: request_seen,
        },
        approvals,
        guardian_folds: &folds,
        cancellations,
        consumed,
        rotation,
        rotation_first_seen: rotation_seen,
    })
}

#[test]
fn guardian_set_update_folds_and_pins_threshold() {
    let head = owner_head();
    let set = head.guardian_set.as_ref().unwrap();
    assert_eq!(set.threshold, 2);
    assert_eq!(set.guardians.len(), 3);
    let admin = key(OWNER);
    let sign = |entry| KeysetRecord::sign(&admin, 9, head.head_hash.clone(), entry, None);
    assert!(sign(guardian_set(&GUARDIANS, 0)).is_err(), "threshold 0");
    assert!(sign(guardian_set(&GUARDIANS, 4)).is_err(), "m > n");
    let mut unsorted: Vec<String> = GUARDIANS.iter().map(|seed| public(*seed)).collect();
    unsorted.sort();
    unsorted.reverse();
    assert!(
        sign(KeysetEntry::GuardianSetUpdated {
            guardians: unsorted,
            threshold: 2,
        })
        .is_err(),
        "a set out of canonical order"
    );
}

#[test]
fn a_passing_recovery_names_its_successor() {
    let head = owner_head();
    let request = request(&head, "lost every device");
    let approvals = [
        approval(&head, &request, 11, 11),
        approval(&head, &request, 13, 13),
    ];
    let rotation = rotation(&head, &request, &[&approvals[0], &approvals[1]]);
    assert_eq!(
        check(&head, &request, &approvals, &[], &rotation).unwrap(),
        Successor {
            admin_key: public(SUCCESSOR),
            device_key: public(SUCCESSOR_DEVICE),
        }
    );
}

#[test]
fn recovery_request_id_is_content_addressed_and_single_use() {
    let head = owner_head();
    let first = request(&head, "lost every device");
    assert_eq!(first.request_id(), Some(first.hash.as_str()));
    assert_ne!(request(&head, "another reason").hash, first.hash);
    let approvals = [
        approval(&head, &first, 11, 11),
        approval(&head, &first, 12, 12),
    ];
    let rotation = rotation(&head, &first, &[&approvals[0], &approvals[1]]);
    let consumed = BTreeSet::from([first.hash.clone()]);
    let error = check_at(
        &head,
        &first,
        CREATED_AT,
        &approvals,
        &[],
        &rotation,
        CREATED_AT + RECOVERY_DELAY_SECONDS,
        &consumed,
    )
    .unwrap_err();
    assert!(error.to_string().contains("consumed"), "{error:#}");
    // A request must be signed by the successor it names.
    let mut forged = first.clone();
    forged.author = public(OWNER);
    assert!(forged.verify().is_err());
}

#[test]
fn approvals_below_threshold_do_not_rotate() {
    let head = owner_head();
    let request = request(&head, "lost");
    let approvals = [approval(&head, &request, 11, 11)];
    let rotation = rotation(&head, &request, &[&approvals[0]]);
    let error = check(&head, &request, &approvals, &[], &rotation).unwrap_err();
    assert!(error.to_string().contains("threshold"), "{error:#}");
}

#[test]
fn duplicate_guardian_approvals_count_once() {
    let head = owner_head();
    let request = request(&head, "lost");
    let approvals = [
        approval_at(&head, &request, 11, 11, 2_000),
        approval_at(&head, &request, 11, 11, 2_001),
    ];
    let rotation = rotation(&head, &request, &[&approvals[0], &approvals[1]]);
    assert!(check(&head, &request, &approvals, &[], &rotation).is_err());
}

#[test]
fn approval_from_non_guardian_or_device_key_rejected() {
    let head = owner_head();
    let request = request(&head, "lost");
    // Guardian 12's own device key (not its admin key) signs for 12.
    let device_seed = 60;
    let (_, guardian_with_device) = identity(
        12,
        vec![(
            KeysetEntry::DeviceEnrolled {
                device_key: public(device_seed),
                label: "tablet".to_owned(),
            },
            Some(key(device_seed)),
        )],
    );
    let mut folds = guardian_folds();
    folds.insert(public(12), guardian_with_device);
    let good = approval(&head, &request, 11, 11);
    let by_device = approval(&head, &request, device_seed, 12);
    // Identity 70 is nobody's guardian.
    let outsider = approval(&head, &request, 70, 70);
    for bad in [&by_device, &outsider] {
        let approvals = [good.clone(), bad.clone()];
        let rotation = rotation(&head, &request, &[&good, bad]);
        let result = check_rotation(&RotationCheck {
            head: &head,
            request: Seen {
                record: &request,
                first_seen: CREATED_AT,
            },
            approvals: &approvals,
            guardian_folds: &folds,
            cancellations: &[],
            consumed: &BTreeSet::new(),
            rotation: &rotation,
            rotation_first_seen: CREATED_AT + RECOVERY_DELAY_SECONDS,
        });
        assert!(result.is_err());
    }
}

#[test]
fn recovery_rotation_revokes_all_prior_keys() {
    let head = owner_head();
    let request = request(&head, "lost");
    let approvals = [
        approval(&head, &request, 11, 11),
        approval(&head, &request, 12, 12),
    ];
    let rotation = rotation(&head, &request, &[&approvals[0], &approvals[1]]);
    let successor = check(&head, &request, &approvals, &[], &rotation).unwrap();
    let after = apply_rotation(&head, &rotation, &successor).unwrap();
    assert!(after.is_admin(&public(SUCCESSOR)));
    assert!(after.is_device(&public(SUCCESSOR_DEVICE)));
    assert!(!after.is_admin(&public(OWNER)));
    assert!(!after.is_device(&public(OWNER)));
    assert!(!after.is_device(&public(OWNER_DEVICE)));
    assert_eq!(after.identity_id, head.identity_id);
    assert_eq!(after.guardian_set, head.guardian_set);
    assert_eq!(after.head_hash, rotation.hash);
}

#[test]
fn plain_fold_never_takes_a_recovery_rotation() {
    let (mut log, head) = identity(OWNER, vec![(guardian_set(&GUARDIANS, 2), None)]);
    let request = request(&head, "lost");
    let approvals = [
        approval(&head, &request, 11, 11),
        approval(&head, &request, 12, 12),
    ];
    let rotation = rotation(&head, &request, &[&approvals[0], &approvals[1]]);
    log.insert(rotation).unwrap();
    let genesis = log
        .records()
        .find(|record| record.prev_hash.is_empty())
        .unwrap();
    assert_eq!(fold(&log, &genesis.hash).unwrap(), head);
}

#[test]
fn cancellation_by_single_device_key_aborts_recovery() {
    let head = owner_head();
    let request = request(&head, "lost");
    let approvals = [
        approval(&head, &request, 11, 11),
        approval(&head, &request, 12, 12),
    ];
    let rotation = rotation(&head, &request, &[&approvals[0], &approvals[1]]);
    let rotation_seen = CREATED_AT + RECOVERY_DELAY_SECONDS;
    let by_device = cancellation(&head, &request, OWNER_DEVICE, CANCELLED_BY_SURVIVING_KEY);
    let before = [Seen {
        record: &by_device,
        first_seen: rotation_seen - 1,
    }];
    let error = check(&head, &request, &approvals, &before, &rotation).unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error:#}");
    // Seen only after the rotation: the window had closed.
    let after = [Seen {
        record: &by_device,
        first_seen: rotation_seen + 1,
    }];
    assert!(check(&head, &request, &approvals, &after, &rotation).is_ok());
    // A guardian cannot cancel, and the guardian-process basis is
    // reserved, not honoured.
    let by_guardian = cancellation(&head, &request, 11, CANCELLED_BY_SURVIVING_KEY);
    let by_process = cancellation(&head, &request, OWNER, CANCELLED_BY_GUARDIAN_PROCESS);
    for record in [&by_guardian, &by_process] {
        let seen = [Seen {
            record,
            first_seen: CREATED_AT,
        }];
        assert!(check(&head, &request, &approvals, &seen, &rotation).is_ok());
    }
}

#[test]
fn rotation_before_delay_rejected() {
    let head = owner_head();
    let request = request(&head, "lost");
    let approvals = [
        approval(&head, &request, 11, 11),
        approval(&head, &request, 12, 12),
    ];
    let rotation = rotation(&head, &request, &[&approvals[0], &approvals[1]]);
    let none = BTreeSet::new();
    let early = check_at(
        &head,
        &request,
        CREATED_AT,
        &approvals,
        &[],
        &rotation,
        CREATED_AT + RECOVERY_DELAY_SECONDS - 1,
        &none,
    );
    assert!(early.is_err());
    // A relay that backdates the request's first sighting is caught.
    let backdated = check_at(
        &head,
        &request,
        CREATED_AT - 1,
        &approvals,
        &[],
        &rotation,
        CREATED_AT + 1,
        &none,
    );
    assert!(backdated.unwrap_err().to_string().contains("backdated"));
    // A relay that withholds only lengthens the wait.
    let late = check_at(
        &head,
        &request,
        CREATED_AT + 500,
        &approvals,
        &[],
        &rotation,
        CREATED_AT + 500 + RECOVERY_DELAY_SECONDS,
        &none,
    );
    assert!(late.is_ok());
}

#[test]
fn guardians_added_after_the_pinned_head_do_not_count() {
    let head = owner_head();
    let request = request(&head, "lost");
    // Guardian 14 joins later; the request still weighs the pinned set.
    let latecomer = approval(&head, &request, 14, 14);
    let honest = approval(&head, &request, 11, 11);
    let approvals = [honest.clone(), latecomer.clone()];
    let rotation = rotation(&head, &request, &[&honest, &latecomer]);
    let mut folds = guardian_folds();
    folds.insert(public(14), identity(14, vec![]).1);
    let result = check_rotation(&RotationCheck {
        head: &head,
        request: Seen {
            record: &request,
            first_seen: CREATED_AT,
        },
        approvals: &approvals,
        guardian_folds: &folds,
        cancellations: &[],
        consumed: &BTreeSet::new(),
        rotation: &rotation,
        rotation_first_seen: CREATED_AT + RECOVERY_DELAY_SECONDS,
    });
    assert!(result.is_err());
}

#[test]
fn server_cannot_forge_approval_or_shorten_delay() {
    let head = owner_head();
    let request = request(&head, "lost");
    let honest = approval(&head, &request, 11, 11);
    // The relay rewrites an approval to name another guardian.
    let mut forged = approval(&head, &request, 12, 12);
    if let AnnexEntry::RecoveryApproval { guardian, .. } = &mut forged.entry {
        *guardian = public(13);
    }
    let approvals = [honest.clone(), forged.clone()];
    let rotation = rotation(&head, &request, &[&honest, &forged]);
    assert!(check(&head, &request, &approvals, &[], &rotation).is_err());
    // The relay alters the request's created_at to start the clock
    // early: the signature no longer verifies.
    let mut rewound = request.clone();
    if let AnnexEntry::RecoveryRequest { created_at, .. } = &mut rewound.entry {
        *created_at = 0;
    }
    assert!(rewound.verify().is_err());
}

#[test]
fn recovery_notifications_cover_all_bound_devices_and_guardians() {
    let head = owner_head();
    let mut expected: BTreeSet<String> = [OWNER, OWNER_DEVICE].iter().map(|s| public(*s)).collect();
    expected.extend(GUARDIANS.iter().map(|seed| public(*seed)));
    assert_eq!(notify_targets(&head), expected);
}

#[test]
fn shamir_material_is_never_accepted() {
    let head = owner_head();
    let request = request(&head, "lost");
    let mut value = serde_json::to_value(&request).unwrap();
    value["entry"]["share"] = serde_json::json!("01:deadbeef");
    assert!(serde_json::from_value::<AnnexRecord>(value).is_err());
    let mut outer = serde_json::to_value(&request).unwrap();
    outer["key_share"] = serde_json::json!("deadbeef");
    assert!(serde_json::from_value::<AnnexRecord>(outer).is_err());
    let round_trip: AnnexRecord =
        serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
    round_trip.verify().unwrap();
}
