//! Rule R tests, named after the guardian recovery design's §7 list.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::keyset::{Log, checkpoint_final_heads_with_recovery, fold, fold_with_recovery};

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
    owner_identity().1
}

fn owner_identity() -> (Log, KeysetState) {
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
fn recovery_rejects_a_successor_admin_key_already_in_the_pinned_fold() {
    let head = owner_head();
    let reused_key = OWNER;
    let request = AnnexRecord::sign(
        &key(reused_key),
        head.head_hash.clone(),
        AnnexEntry::RecoveryRequest {
            identity_id: head.identity_id.clone(),
            keyset_head_hash: head.head_hash.clone(),
            successor_admin_key: public(reused_key),
            successor_device_key: public(SUCCESSOR_DEVICE),
            created_at: CREATED_AT,
            reason: "lost every device".to_owned(),
        },
    )
    .unwrap();
    let approvals = [
        approval(&head, &request, 11, 11),
        approval(&head, &request, 12, 12),
    ];
    let mut approval_hashes: Vec<String> = approvals.iter().map(|a| a.hash.clone()).collect();
    approval_hashes.sort();
    let rotation = KeysetRecord::sign(
        &key(reused_key),
        head.records_folded + 10,
        head.head_hash.clone(),
        KeysetEntry::RecoveryRotation {
            request_id: request.hash.clone(),
            approval_hashes,
            successor_admin_key: public(reused_key),
            successor_device_key: public(SUCCESSOR_DEVICE),
        },
        None,
    )
    .unwrap();

    let error = check(&head, &request, &approvals, &[], &rotation).unwrap_err();
    assert!(error.to_string().contains("already present"), "{error:#}");
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
    // Seen in the same second as the rotation (#11773): the relay's
    // whole-second stamp cannot order them, so the cancellation wins,
    // whichever of the two reached the relay first.
    let same_second = [Seen {
        record: &by_device,
        first_seen: rotation_seen,
    }];
    let error = check(&head, &request, &approvals, &same_second, &rotation).unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error:#}");
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

// Folding a rotation (the design's split, item 2).

/// The owner's log with a passing recovery rotation in it: the log, its
/// genesis hash, the pinned head, the annex and the rotation.
fn recovered() -> (Log, String, KeysetState, Vec<AnnexRecord>, KeysetRecord) {
    let (mut log, head) = owner_identity();
    let genesis = log
        .records()
        .find(|record| record.prev_hash.is_empty())
        .unwrap()
        .hash
        .clone();
    let request = request(&head, "lost");
    let approvals = [
        approval(&head, &request, 11, 11),
        approval(&head, &request, 12, 12),
    ];
    let rotation = rotation(&head, &request, &[&approvals[0], &approvals[1]]);
    log.insert(rotation.clone()).unwrap();
    let [first, second] = approvals;
    (log, genesis, head, vec![request, first, second], rotation)
}

/// Folds with every annex record first seen at its creation and the
/// rotation first seen at `rotation_seen`.
fn fold_recovering(
    log: &Log,
    genesis: &str,
    annex: &[AnnexRecord],
    rotation: &KeysetRecord,
    rotation_seen: u64,
    finals: &BTreeSet<String>,
) -> KeysetState {
    let seen: Vec<Seen<'_>> = annex
        .iter()
        .map(|record| Seen {
            record,
            first_seen: CREATED_AT,
        })
        .collect();
    let folds = guardian_folds();
    let first_seen = BTreeMap::from([(rotation.hash.clone(), rotation_seen)]);
    let evidence = RecoveryEvidence {
        annex: &seen,
        guardian_folds: &folds,
        keyset_first_seen: &first_seen,
    };
    fold_with_recovery(log, genesis, finals, &evidence).unwrap()
}

const AFTER_DELAY: u64 = CREATED_AT + RECOVERY_DELAY_SECONDS;

#[test]
fn a_recovery_rotation_folds_with_its_evidence() {
    let (mut log, genesis, head, annex, rotation) = recovered();
    let none = BTreeSet::new();
    let after = fold_recovering(&log, &genesis, &annex, &rotation, AFTER_DELAY, &none);
    let successor = Successor {
        admin_key: public(SUCCESSOR),
        device_key: public(SUCCESSOR_DEVICE),
    };
    assert_eq!(after, apply_rotation(&head, &rotation, &successor).unwrap());
    // Without the evidence nothing folds it.
    assert_eq!(fold(&log, &genesis).unwrap(), head);
    // The successor's records continue it; the old admin's do not.
    let next = KeysetRecord::sign(
        &key(SUCCESSOR),
        rotation.lamport + 1,
        rotation.hash.clone(),
        KeysetEntry::IdpBindingBound {
            binding_id: "after".to_owned(),
        },
        None,
    )
    .unwrap();
    let stale = KeysetRecord::sign(
        &key(OWNER),
        rotation.lamport + 1,
        rotation.hash.clone(),
        KeysetEntry::IdpBindingBound {
            binding_id: "stale".to_owned(),
        },
        None,
    )
    .unwrap();
    log.insert(next.clone()).unwrap();
    log.insert(stale).unwrap();
    let folded = fold_recovering(&log, &genesis, &annex, &rotation, AFTER_DELAY, &none);
    assert_eq!(folded.head_hash, next.hash);
}

#[test]
fn a_rotation_folds_only_once_rule_r_holds() {
    let (log, genesis, head, annex, rotation) = recovered();
    let none = BTreeSet::new();
    let early = fold_recovering(&log, &genesis, &annex, &rotation, AFTER_DELAY - 1, &none);
    assert_eq!(early, head);
    let no_request = fold_recovering(&log, &genesis, &annex[1..], &rotation, AFTER_DELAY, &none);
    assert_eq!(no_request, head);
    let mut cancelled = annex.clone();
    cancelled.push(cancellation(
        &head,
        &annex[0],
        OWNER_DEVICE,
        CANCELLED_BY_SURVIVING_KEY,
    ));
    let state = fold_recovering(&log, &genesis, &cancelled, &rotation, AFTER_DELAY, &none);
    assert_eq!(state, head);
}

#[test]
fn a_surviving_admin_continuation_makes_the_recovery_moot() {
    let (mut log, genesis, head, annex, rotation) = recovered();
    // The owner's own sibling sorts after the rotation and is revoked by
    // it, yet a surviving key acting at the pinned head wins.
    let sibling = KeysetRecord::sign(
        &key(OWNER),
        rotation.lamport + 50,
        head.head_hash,
        KeysetEntry::IdpBindingBound {
            binding_id: "alive".to_owned(),
        },
        None,
    )
    .unwrap();
    log.insert(sibling.clone()).unwrap();
    let state = fold_recovering(
        &log,
        &genesis,
        &annex,
        &rotation,
        AFTER_DELAY,
        &BTreeSet::new(),
    );
    assert_eq!(state.head_hash, sibling.hash);
    assert!(state.is_admin(&public(OWNER)));
    assert!(!state.is_admin(&public(SUCCESSOR)));
}

#[test]
fn a_checkpointed_recovery_outranks_a_late_surviving_sibling() {
    let (mut log, genesis, head, annex, rotation) = recovered();
    let none = BTreeSet::new();
    let recovered = fold_recovering(&log, &genesis, &annex, &rotation, AFTER_DELAY, &none);
    let checkpoint = KeysetRecord::sign(
        &key(SUCCESSOR_DEVICE),
        rotation.lamport + 1,
        rotation.hash.clone(),
        KeysetEntry::Checkpoint {
            state_hash: recovered.state_hash().unwrap(),
            record_count: recovered.records_folded,
        },
        None,
    )
    .unwrap();
    log.insert(checkpoint).unwrap();
    let seen: Vec<Seen<'_>> = annex
        .iter()
        .map(|record| Seen {
            record,
            first_seen: CREATED_AT,
        })
        .collect();
    let folds = guardian_folds();
    let first_seen = BTreeMap::from([(rotation.hash.clone(), AFTER_DELAY)]);
    let evidence = RecoveryEvidence {
        annex: &seen,
        guardian_folds: &folds,
        keyset_first_seen: &first_seen,
    };
    let finals = checkpoint_final_heads_with_recovery(&log, &genesis, &none, &evidence).unwrap();
    assert!(finals.contains(&rotation.hash));
    let sibling = KeysetRecord::sign(
        &key(OWNER),
        rotation.lamport + 50,
        head.head_hash,
        KeysetEntry::IdpBindingBound {
            binding_id: "late".to_owned(),
        },
        None,
    )
    .unwrap();
    log.insert(sibling.clone()).unwrap();
    let pinned = fold_recovering(&log, &genesis, &annex, &rotation, AFTER_DELAY, &finals);
    assert_eq!(pinned, recovered);
    let unpinned = fold_recovering(&log, &genesis, &annex, &rotation, AFTER_DELAY, &none);
    assert_eq!(unpinned.head_hash, sibling.hash);
}

// Delayed guardian removals (Decision 6).

const SOCKPUPPETS: [u8; 2] = [21, 22];

/// The owner's identity after an eviction: guardians 11 and 12 removed,
/// sockpuppets 21 and 22 added, 2-of-3 kept. Returns the log, its
/// genesis hash, the head and the eviction record's hash.
fn evicted() -> (Log, String, KeysetState, String) {
    let (log, head) = identity(
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
            (guardian_set(&[13, 21, 22], 2), None),
        ],
    );
    let genesis = log
        .records()
        .find(|record| record.prev_hash.is_empty())
        .unwrap()
        .hash
        .clone();
    let eviction = head.pending_guardian_set.clone().unwrap().record_hash;
    (log, genesis, head, eviction)
}

/// Rule R through [`RecoveryEvidence`], with the request first seen at
/// `request_seen`, the eviction at `CREATED_AT` and the rotation once
/// the request's delay has passed.
fn evidence_check(
    head: &KeysetState,
    eviction: &str,
    request_seen: u64,
    signers: &[u8],
) -> Result<Successor> {
    let request = request(head, "lost");
    let approvals: Vec<AnnexRecord> = signers
        .iter()
        .map(|seed| approval(head, &request, *seed, *seed))
        .collect();
    let refs: Vec<&AnnexRecord> = approvals.iter().collect();
    let rotation = rotation(head, &request, &refs);
    let mut annex = vec![Seen {
        record: &request,
        first_seen: request_seen,
    }];
    annex.extend(approvals.iter().map(|record| Seen {
        record,
        first_seen: request_seen,
    }));
    let mut folds = guardian_folds();
    for seed in SOCKPUPPETS {
        folds.insert(public(seed), identity(seed, vec![]).1);
    }
    let first_seen = BTreeMap::from([
        (eviction.to_owned(), CREATED_AT),
        (rotation.hash.clone(), request_seen + RECOVERY_DELAY_SECONDS),
    ]);
    RecoveryEvidence {
        annex: &annex,
        guardian_folds: &folds,
        keyset_first_seen: &first_seen,
    }
    .check(head, &rotation)
}

#[test]
fn guardian_eviction_attack_fails_within_delay() {
    let (_, _, head, eviction) = evicted();
    // The removal is pending: the honest set is still in force.
    let honest: Vec<String> = GUARDIANS.iter().map(|seed| public(*seed)).collect();
    assert_eq!(head.guardian_set.as_ref().unwrap().guardians.len(), 3);
    assert!(honest.iter().all(|guardian| {
        head.guardian_set
            .as_ref()
            .unwrap()
            .guardians
            .contains(guardian)
    }));
    // Sockpuppet approvals inside the window recover nothing.
    let inside = CREATED_AT + RECOVERY_DELAY_SECONDS - 1;
    assert!(evidence_check(&head, &eviction, inside, &SOCKPUPPETS).is_err());
    // The removed-but-pending guardians still count.
    assert!(evidence_check(&head, &eviction, inside, &[11, 12]).is_ok());
}

#[test]
fn pending_guardian_removal_takes_effect_after_delay() {
    let (mut log, genesis, head, eviction) = evicted();
    let matured = CREATED_AT + RECOVERY_DELAY_SECONDS;
    // Rule R: a request first seen once the delay has run counts the
    // new set and no longer the removed guardians.
    assert!(evidence_check(&head, &eviction, matured, &SOCKPUPPETS).is_ok());
    assert!(evidence_check(&head, &eviction, matured, &[11, 12]).is_err());
    // The fold: the next record, first seen after the delay, folds onto
    // the new set; without times the change never matures.
    let next = KeysetRecord::sign(
        &key(OWNER),
        head.records_folded + 1,
        head.head_hash,
        KeysetEntry::IdpBindingBound {
            binding_id: "later".to_owned(),
        },
        None,
    )
    .unwrap();
    log.insert(next.clone()).unwrap();
    let fold_at = |seen: u64| {
        let first_seen =
            BTreeMap::from([(eviction.clone(), CREATED_AT), (next.hash.clone(), seen)]);
        let folds = BTreeMap::new();
        let evidence = RecoveryEvidence {
            annex: &[],
            guardian_folds: &folds,
            keyset_first_seen: &first_seen,
        };
        fold_with_recovery(&log, &genesis, &BTreeSet::new(), &evidence).unwrap()
    };
    let early = fold_at(matured - 1);
    assert!(early.pending_guardian_set.is_some());
    let late = fold_at(matured);
    assert!(late.pending_guardian_set.is_none());
    let mut evicted_set = vec![public(13), public(21), public(22)];
    evicted_set.sort();
    assert_eq!(late.guardian_set.unwrap().guardians, evicted_set);
    let untimed = fold(&log, &genesis).unwrap();
    assert!(untimed.pending_guardian_set.is_some());
}

#[test]
fn adding_folds_at_once_and_restoring_cancels_a_pending_removal() {
    let (_, added) = identity(
        OWNER,
        vec![
            (guardian_set(&GUARDIANS, 2), None),
            (guardian_set(&[11, 12, 13, 14], 3), None),
        ],
    );
    assert_eq!(added.guardian_set.unwrap().guardians.len(), 4);
    assert!(added.pending_guardian_set.is_none());
    let (_, restored) = identity(
        OWNER,
        vec![
            (guardian_set(&GUARDIANS, 2), None),
            (guardian_set(&[11], 1), None),
            (guardian_set(&GUARDIANS, 2), None),
        ],
    );
    assert!(restored.pending_guardian_set.is_none());
    assert_eq!(restored.guardian_set, owner_head().guardian_set);
}

// The directory relay (server role: relay and timestamp only).

mod relay {
    use super::*;
    use crate::recovery_relay::{RecoveryRelay, RelayedRecord};

    fn store() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery-relay.jsonl");
        (dir, path)
    }

    #[test]
    fn a_first_seen_time_is_attested_once_and_survives_a_restart() {
        let (_dir, path) = store();
        let request = request(&owner_head(), "lost");
        let mut relay = RecoveryRelay::open(&path).unwrap();
        let first = relay
            .relay(RelayedRecord::Annex(request.clone()), CREATED_AT + 5)
            .unwrap();
        assert_eq!((first.first_seen, first.new), (CREATED_AT + 5, true));
        // Sending it again cannot move the time, so it cannot shorten
        // the delay.
        let again = relay
            .relay(RelayedRecord::Annex(request.clone()), AFTER_DELAY)
            .unwrap();
        assert_eq!((again.first_seen, again.new), (CREATED_AT + 5, false));
        let reopened = RecoveryRelay::open(&path).unwrap();
        assert_eq!(reopened.first_seen(&request.hash), Some(CREATED_AT + 5));
    }

    #[test]
    fn a_request_is_never_attested_before_it_was_created() {
        let (_dir, path) = store();
        let request = request(&owner_head(), "lost");
        let mut relay = RecoveryRelay::open(&path).unwrap();
        let seen = relay
            .relay(RelayedRecord::Annex(request), CREATED_AT - 500)
            .unwrap();
        assert_eq!(seen.first_seen, CREATED_AT);
    }

    #[test]
    fn the_relay_refuses_forged_records() {
        let (_dir, path) = store();
        let head = owner_head();
        let request = request(&head, "lost");
        let mut forged = approval(&head, &request, 11, 11);
        if let AnnexEntry::RecoveryApproval { guardian, .. } = &mut forged.entry {
            *guardian = public(12);
        }
        let mut relay = RecoveryRelay::open(&path).unwrap();
        assert!(
            relay
                .relay(RelayedRecord::Annex(forged.clone()), CREATED_AT)
                .is_err()
        );
        let honest = approval(&head, &request, 11, 11);
        let mut rotation = rotation(&head, &request, &[&honest]);
        rotation.lamport += 1;
        assert!(
            relay
                .relay(RelayedRecord::Keyset(rotation), CREATED_AT)
                .is_err()
        );
        assert_eq!(relay.first_seen(&forged.hash), None);
        // A record tampered on disk is refused when the store opens.
        relay
            .relay(RelayedRecord::Annex(request), CREATED_AT)
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("\"lost\"", "\"found\"")).unwrap();
        assert!(RecoveryRelay::open(&path).is_err());
    }

    /// Relays the recovered identity's annex at `CREATED_AT` and its
    /// rotation at `rotation_seen`, then folds from what the relay holds.
    fn fold_from_relay(rotation_seen: u64) -> (KeysetState, KeysetState, KeysetRecord) {
        let (log, genesis, head, annex, rotation) = recovered();
        let (_dir, path) = store();
        let mut relay = RecoveryRelay::open(&path).unwrap();
        for record in annex {
            relay
                .relay(RelayedRecord::Annex(record), CREATED_AT)
                .unwrap();
        }
        relay
            .relay(RelayedRecord::Keyset(rotation.clone()), rotation_seen)
            .unwrap();
        assert!(relay.annex(&public(GUARDIANS[0])).is_empty());
        let annex = relay.annex(&public(OWNER));
        let keyset_first_seen = relay.keyset_first_seen();
        let folds = guardian_folds();
        let evidence = RecoveryEvidence {
            annex: &annex,
            guardian_folds: &folds,
            keyset_first_seen: &keyset_first_seen,
        };
        let folded = fold_with_recovery(&log, &genesis, &BTreeSet::new(), &evidence).unwrap();
        (head, folded, rotation)
    }

    #[test]
    fn relayed_evidence_folds_a_recovery_only_after_the_delay() {
        let (head, folded, rotation) = fold_from_relay(AFTER_DELAY);
        let successor = Successor {
            admin_key: public(SUCCESSOR),
            device_key: public(SUCCESSOR_DEVICE),
        };
        assert_eq!(
            folded,
            apply_rotation(&head, &rotation, &successor).unwrap()
        );
        let (head, early, _) = fold_from_relay(AFTER_DELAY - 1);
        assert_eq!(early, head);
    }
}

// The client flow: request, approve, cancel, wait, rotate.

mod flow {
    use super::*;
    use crate::recovery_flow::{self, Progress};
    use crate::recovery_relay::{RecoveryRelay, RelayedRecord};

    struct Run {
        _dir: tempfile::TempDir,
        relay: RecoveryRelay,
        log: Log,
        genesis: String,
        head: KeysetState,
        request: AnnexRecord,
    }

    /// The owner's identity with a flow-built request relayed at
    /// `CREATED_AT`.
    fn started() -> Run {
        let (log, head) = owner_identity();
        let genesis = log
            .records()
            .find(|record| record.prev_hash.is_empty())
            .unwrap()
            .hash
            .clone();
        let request = recovery_flow::request(
            &key(SUCCESSOR),
            &public(SUCCESSOR_DEVICE),
            &head,
            CREATED_AT,
            "lost every device",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut relay = RecoveryRelay::open(dir.path().join("relay.jsonl")).unwrap();
        relay
            .relay(RelayedRecord::Annex(request.clone()), CREATED_AT)
            .unwrap();
        Run {
            _dir: dir,
            relay,
            log,
            genesis,
            head,
            request,
        }
    }

    impl Run {
        fn approve(&mut self, guardian: u8) {
            let approval =
                recovery_flow::approve(&key(guardian), &public(guardian), &self.request, 2_000)
                    .unwrap();
            self.relay
                .relay(RelayedRecord::Annex(approval), CREATED_AT + 1)
                .unwrap();
        }

        fn progress(&self, now: u64) -> Progress {
            let annex = self.relay.annex(&public(OWNER));
            let first_seen = self.relay.keyset_first_seen();
            let folds = guardian_folds();
            let evidence = RecoveryEvidence {
                annex: &annex,
                guardian_folds: &folds,
                keyset_first_seen: &first_seen,
            };
            recovery_flow::progress(&evidence, &self.head, &self.request.hash, now).unwrap()
        }
    }

    #[test]
    fn a_flow_built_recovery_waits_for_approvals_and_the_delay_then_folds() {
        let mut run = started();
        assert_eq!(
            run.progress(CREATED_AT),
            Progress::Waiting {
                approvals: 0,
                threshold: 2,
                ready_at: AFTER_DELAY,
            }
        );
        run.approve(11);
        run.approve(11);
        // A non-guardian's approval never counts.
        run.approve(21);
        assert!(matches!(
            run.progress(AFTER_DELAY),
            Progress::Waiting { approvals: 1, .. }
        ));
        run.approve(12);
        assert!(matches!(
            run.progress(AFTER_DELAY - 1),
            Progress::Waiting { approvals: 2, .. }
        ));
        let approval_hashes = match run.progress(AFTER_DELAY) {
            Progress::Ready { approval_hashes } => Some(approval_hashes),
            _ => None,
        }
        .expect("the recovery is ready");
        assert_eq!(approval_hashes.len(), 2);
        // Only the successor can complete it.
        assert!(
            recovery_flow::rotate(&key(OWNER), &run.request, approval_hashes.clone(), 99).is_err()
        );
        let rotation = recovery_flow::rotate(
            &key(SUCCESSOR),
            &run.request,
            approval_hashes,
            run.head.records_folded + 10,
        )
        .unwrap();
        run.relay
            .relay(RelayedRecord::Keyset(rotation.clone()), AFTER_DELAY)
            .unwrap();
        run.log.insert(rotation).unwrap();
        let annex = run.relay.annex(&public(OWNER));
        let first_seen = run.relay.keyset_first_seen();
        let folds = guardian_folds();
        let evidence = RecoveryEvidence {
            annex: &annex,
            guardian_folds: &folds,
            keyset_first_seen: &first_seen,
        };
        let folded =
            fold_with_recovery(&run.log, &run.genesis, &BTreeSet::new(), &evidence).unwrap();
        assert!(folded.is_admin(&public(SUCCESSOR)));
        assert!(!folded.is_admin(&public(OWNER)));
    }

    #[test]
    fn a_surviving_device_cancels_a_flow_built_request() {
        let mut run = started();
        run.approve(11);
        run.approve(12);
        let cancel = recovery_flow::cancel(&key(OWNER_DEVICE), &run.request).unwrap();
        run.relay
            .relay(RelayedRecord::Annex(cancel), CREATED_AT + 10)
            .unwrap();
        assert!(matches!(
            run.progress(CREATED_AT + 9),
            Progress::Waiting { .. }
        ));
        assert_eq!(
            run.progress(AFTER_DELAY),
            Progress::Cancelled {
                by: public(OWNER_DEVICE)
            }
        );
    }
}

mod view {
    use super::*;
    use crate::recovery_flow::{self, Progress};
    use crate::recovery_relay::{RecoveryRelay, RelayedRecord};
    use crate::recovery_view::{RelayView, Status};

    /// A relay holding the owner's and every guardian's keyset, all
    /// first seen at time 0, and nothing else.
    fn published() -> (tempfile::TempDir, RecoveryRelay) {
        let dir = tempfile::tempdir().unwrap();
        let mut relay = RecoveryRelay::open(dir.path().join("relay.jsonl")).unwrap();
        let mut logs = vec![owner_identity().0];
        logs.extend(GUARDIANS.iter().map(|seed| identity(*seed, vec![]).0));
        for log in logs {
            for record in log.records() {
                relay
                    .relay(RelayedRecord::Keyset(record.clone()), 0)
                    .unwrap();
            }
        }
        (dir, relay)
    }

    fn relay_annex(relay: &mut RecoveryRelay, record: AnnexRecord, now: u64) {
        relay.relay(RelayedRecord::Annex(record), now).unwrap();
    }

    #[test]
    fn a_relay_view_carries_a_recovery_from_request_to_completion() {
        let (_dir, mut relay) = published();
        let head = RelayView::new(&relay, &public(OWNER))
            .unwrap()
            .head()
            .unwrap();
        assert_eq!(head, owner_head());
        let request = recovery_flow::request(
            &key(SUCCESSOR),
            &public(SUCCESSOR_DEVICE),
            &head,
            CREATED_AT,
            "lost every device",
        )
        .unwrap();
        relay_annex(&mut relay, request.clone(), CREATED_AT);
        for guardian in [11, 12] {
            let approval =
                recovery_flow::approve(&key(guardian), &public(guardian), &request, 2_000).unwrap();
            relay_annex(&mut relay, approval, CREATED_AT + 1);
        }
        let view = RelayView::new(&relay, &public(OWNER)).unwrap();
        assert!(matches!(
            view.status(&request.hash, AFTER_DELAY - 1).unwrap(),
            Status::Open(Progress::Waiting { approvals: 2, .. })
        ));
        let approval_hashes = match view.status(&request.hash, AFTER_DELAY).unwrap() {
            Status::Open(Progress::Ready { approval_hashes }) => Some(approval_hashes),
            _ => None,
        }
        .expect("the recovery is ready");
        let lamport = view.next_lamport().unwrap();
        assert_eq!(lamport, 4);
        let rotation =
            recovery_flow::rotate(&key(SUCCESSOR), &request, approval_hashes, lamport).unwrap();
        relay
            .relay(RelayedRecord::Keyset(rotation), AFTER_DELAY)
            .unwrap();
        let view = RelayView::new(&relay, &public(OWNER)).unwrap();
        assert_eq!(
            view.status(&request.hash, AFTER_DELAY).unwrap(),
            Status::Completed
        );
        let recovered = view.head().unwrap();
        assert!(recovered.is_admin(&public(SUCCESSOR)));
        assert!(recovered.is_device(&public(SUCCESSOR_DEVICE)));
    }

    #[test]
    fn a_request_is_moot_once_the_keyset_moves_on() {
        let (_dir, mut relay) = published();
        let head = owner_head();
        let request = recovery_flow::request(
            &key(SUCCESSOR),
            &public(SUCCESSOR_DEVICE),
            &head,
            CREATED_AT,
            "lost every device",
        )
        .unwrap();
        relay_annex(&mut relay, request.clone(), CREATED_AT);
        // A surviving admin key keeps using the keyset.
        let next = KeysetRecord::sign(
            &key(OWNER),
            4,
            head.head_hash,
            KeysetEntry::DeviceRevoked {
                device_key: public(OWNER_DEVICE),
                reason: "lost".to_owned(),
            },
            None,
        )
        .unwrap();
        relay
            .relay(RelayedRecord::Keyset(next), CREATED_AT + 5)
            .unwrap();
        let view = RelayView::new(&relay, &public(OWNER)).unwrap();
        assert_eq!(
            view.status(&request.hash, AFTER_DELAY).unwrap(),
            Status::Moot
        );
        assert_eq!(view.next_lamport().unwrap(), 5);
    }

    #[test]
    fn a_relay_view_needs_the_identitys_genesis_and_its_requests() {
        let (_dir, relay) = published();
        assert!(RelayView::new(&relay, &public(SUCCESSOR)).is_err());
        let view = RelayView::new(&relay, &public(OWNER)).unwrap();
        assert!(view.status(&"0".repeat(64), CREATED_AT).is_err());
        assert!(view.request(&"0".repeat(64)).is_none());
    }
}
