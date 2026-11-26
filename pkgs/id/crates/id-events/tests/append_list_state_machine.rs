//! Model-based oracle for A-GRAPH-9127-02 (#10237).
//!
//! The model says invalid signatures never enter either state; open attributes
//! still hold non-group submissions in Pending; approval rechecks current
//! membership and revocation; idempotency keys conflict on different bytes;
//! and accepted order is append-only.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use id_core::graph::append_list::{AdmissionOutcome, AttributeSubmission, PendingId};
use id_core::graph::attribute::{AdmissionRule, AppendMode};
use id_events::graph_admission::{AdmissionError, AdmissionPolicy, AppendAdmission};

fn policy(mode: AppendMode, members: &[&str]) -> AdmissionPolicy {
    AdmissionPolicy {
        mode,
        rule: AdmissionRule::Members,
        members: members.iter().map(|value| (*value).to_owned()).collect(),
        admins: BTreeSet::new(),
        revoked: BTreeSet::new(),
        epoch: 1,
    }
}

fn submission(id: &str, key: &str, bytes: &[u8], signer: &str) -> AttributeSubmission {
    AttributeSubmission {
        id: id.to_owned(),
        idempotency_key: key.to_owned(),
        bytes: bytes.to_vec(),
        signer: signer.to_owned(),
        signature_valid: true,
    }
}

#[test]
fn invalid_signatures_never_enter_pending_or_accepted() {
    let mut admission = AppendAdmission::default();
    let mut bad = submission("bad", "k1", b"bad", "alice");
    bad.signature_valid = false;

    assert_eq!(
        admission
            .submit(bad, policy(AppendMode::Open, &[]))
            .unwrap_err(),
        AdmissionError::InvalidSignature
    );
    assert!(admission.pending().is_empty());
    assert_eq!(admission.accepted(), [] as [std::string::String; 0]);
}

#[test]
fn open_non_member_waits_but_member_can_be_accepted_directly() {
    let mut admission = AppendAdmission::default();
    let pending = admission
        .submit(
            submission("p", "p", b"pending", "bob"),
            policy(AppendMode::Open, &["alice"]),
        )
        .unwrap();
    assert_eq!(pending, AdmissionOutcome::Pending(PendingId::from("p")));

    let accepted = admission
        .submit(
            submission("a", "a", b"accepted", "alice"),
            policy(AppendMode::Open, &["alice"]),
        )
        .unwrap();
    assert_eq!(accepted, AdmissionOutcome::Accepted);
    assert_eq!(admission.accepted(), &["a"]);
}

#[test]
fn approval_rechecks_membership_and_revocation() {
    let mut admission = AppendAdmission::default();
    admission
        .submit(
            submission("p", "p", b"pending", "bob"),
            policy(AppendMode::Open, &["alice"]),
        )
        .unwrap();

    let mut removed = policy(AppendMode::Open, &["alice"]);
    assert_eq!(
        admission.approve("p", &removed).unwrap_err(),
        AdmissionError::NotEligible
    );

    removed.members.insert("bob".to_owned());
    removed.revoked.insert("bob".to_owned());
    assert_eq!(
        admission.approve("p", &removed).unwrap_err(),
        AdmissionError::Revoked
    );
}

#[test]
fn idempotency_conflict_and_order_are_deterministic() {
    let mut admission = AppendAdmission::default();
    let open = policy(AppendMode::Open, &["alice"]);
    admission
        .submit(submission("one", "same", b"one", "alice"), open.clone())
        .unwrap();
    assert_eq!(
        admission
            .submit(submission("two", "same", b"two", "alice"), open.clone())
            .unwrap_err(),
        AdmissionError::IdempotencyConflict
    );
    admission
        .submit(submission("three", "three", b"three", "alice"), open)
        .unwrap();
    assert_eq!(admission.accepted(), &["one", "three"]);
}
