//! Delegation chains and two-key control transfer (slice
//! A-GRAPH-9132-01, issue #10239; design part A §11, statements S5.56,
//! S5.57, S5.61, S5.64).
//!
//! Oracle: **model**. The proptest at the end builds random grant
//! forests over a few keys (seed 0 owns the space), admits some of them,
//! revokes some, and asks random requests at random times, depths and
//! skews. An independent abstract authority-chain model, written over
//! plain integers and byte strings rather than the module's types,
//! decides each request; the module must return the same outcome and
//! failure class. Targeted tests pin down the cases the generator is
//! unlikely to hit on its own: replay, revocation signers, stale
//! frontiers, the depth budget, canonical payloads and transfers.
//!
//! `graph/mod.rs` does not export these modules yet (slice 9110-09 wires
//! them), so this test includes them, and the modules they build on, by
//! path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::type_complexity
)]

#[path = "../src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../src/signed_envelope.rs"]
#[allow(dead_code)]
mod signed_envelope;

#[path = "../src/graph/term.rs"]
#[allow(dead_code)]
mod term;

#[path = "../src/graph/space_binding.rs"]
#[allow(dead_code)]
mod space_binding;

#[path = "../src/graph/delegation.rs"]
mod delegation;

use std::collections::HashSet;

use canonical_cbor::{Value, decode_canonical, encode_canonical};
use delegation::{
    Authority, Clock, DelegationError, DelegationSet, GRANT_DOMAIN, Grant, Permission, Permissions,
    Profile, REVOCATION_DOMAIN, Request, Scope, TRANSFER_ACCEPT_DOMAIN, TRANSFER_OFFER_DOMAIN,
    TransferStatement, sign_revocation, verify_transfer,
};
use ed25519_dalek::SigningKey;
use hash_profile::ProfileDispatch;
use id_shape::HashProfile;
use proptest::prelude::*;
use signed_envelope::{Body, Envelope, EnvelopeId, PayloadId, SUITE_ED25519};
use space_binding::SpaceAddress;
use term::Term;

const SIGNER: &str = "tester";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public(seed: u8) -> [u8; 32] {
    key(seed).verifying_key().to_bytes()
}

fn space(seed: u8) -> SpaceAddress {
    SpaceAddress::for_key(public(seed)).unwrap()
}

fn dispatch() -> ProfileDispatch {
    ProfileDispatch::default()
}

fn step(b: u8) -> Term {
    Term::typed(1, vec![b])
}

fn path(bytes: &[u8]) -> Vec<Term> {
    bytes.iter().copied().map(step).collect()
}

fn scope(prefixes: &[&[u8]]) -> Scope {
    Scope::new(prefixes.iter().map(|p| path(p)).collect()).unwrap()
}

const NOW: Clock = Clock {
    verifier_utc: 1_000,
    frontier_age_secs: 0,
};

/// Signs any payload under any domain, so tests can build records the
/// module would never make.
fn record(domain: &str, context: &str, payload: Vec<u8>, signer: &SigningKey) -> Envelope {
    let body = Body {
        domain: domain.to_owned(),
        context: context.to_owned(),
        subject: None,
        graph: None,
        predecessor: None,
        signer_id: SIGNER.to_owned(),
        key: signer.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(HashProfile::RawBlake3_256, &payload).unwrap(),
        payload_length: payload.len() as u64,
    };
    Envelope::sign(body, payload, signer).unwrap()
}

struct GrantSpec {
    space: u8,
    grantor: u8,
    grantee: u8,
    scope: Scope,
    perms: Permissions,
    window: (u64, u64),
    nonce: u8,
    pred: Option<EnvelopeId>,
}

impl GrantSpec {
    fn new(grantor: u8, grantee: u8, pred: Option<EnvelopeId>) -> Self {
        Self {
            space: 0,
            grantor,
            grantee,
            scope: scope(&[&[]]),
            perms: Permissions::from_bits(Permissions::ALL_BITS).unwrap(),
            window: (0, 2_000),
            nonce: 1,
            pred,
        }
    }

    fn grant(&self) -> Grant {
        Grant {
            space: space(self.space),
            grantor_key: public(self.grantor),
            grantee_key: public(self.grantee),
            scope: self.scope.clone(),
            permissions: self.perms,
            not_before: self.window.0,
            expires: self.window.1,
            nonce: vec![self.nonce],
            predecessor: self.pred,
        }
    }

    fn sign(&self) -> Envelope {
        self.grant().sign(SIGNER, &key(self.grantor)).unwrap()
    }
}

fn admit(set: &mut DelegationSet, spec: &GrantSpec) -> EnvelopeId {
    set.admit_grant(&spec.sign(), &dispatch()).unwrap()
}

fn request(seed: u8, at: &[u8], permission: Permission) -> Request {
    Request {
        space: space(0),
        key: public(seed),
        path: path(at),
        permission,
    }
}

// ---------------------------------------------------------------------
// Targeted cases.

#[test]
fn owner_and_a_two_link_chain_authorize() {
    let mut set = DelegationSet::new();
    let root = admit(&mut set, &GrantSpec::new(0, 1, None));
    let mut leaf = GrantSpec::new(1, 2, Some(root));
    leaf.perms = Permissions::of(&[Permission::Read, Permission::Append]);
    leaf.scope = scope(&[&[7]]);
    let leaf = admit(&mut set, &leaf);

    let p = Profile::INITIAL;
    assert_eq!(
        set.authorize(&request(0, &[9], Permission::Moderate), None, NOW, &p),
        Ok(Authority::Owner)
    );
    assert_eq!(
        set.authorize(
            &request(2, &[7, 3], Permission::Append),
            Some(&leaf),
            NOW,
            &p
        ),
        Ok(Authority::Delegated {
            depth: 2,
            stale: false
        })
    );
    assert_eq!(
        set.authorize(&request(2, &[8], Permission::Append), Some(&leaf), NOW, &p),
        Err(DelegationError::NotAuthorized("path outside the grant"))
    );
    assert_eq!(
        set.authorize(&request(2, &[7], Permission::Edit), Some(&leaf), NOW, &p),
        Err(DelegationError::NotAuthorized("permission not granted"))
    );
    assert_eq!(
        set.authorize(&request(3, &[7], Permission::Read), Some(&leaf), NOW, &p),
        Err(DelegationError::NotAuthorized("grant is for another key"))
    );
    assert_eq!(
        set.authorize(&request(2, &[7], Permission::Read), None, NOW, &p),
        Err(DelegationError::NotAuthorized("no grant presented"))
    );
}

#[test]
fn authority_does_not_pass_on_without_delegate() {
    let mut set = DelegationSet::new();
    let mut root = GrantSpec::new(0, 1, None);
    root.perms = Permissions::of(&[Permission::Read, Permission::Append]);
    let root = admit(&mut set, &root);
    let mut leaf = GrantSpec::new(1, 2, Some(root));
    leaf.perms = Permissions::of(&[Permission::Read]);
    let leaf = admit(&mut set, &leaf);
    assert_eq!(
        set.authorize(
            &request(2, &[], Permission::Read),
            Some(&leaf),
            NOW,
            &Profile::INITIAL
        ),
        Err(DelegationError::NotDelegable)
    );
}

#[test]
fn a_child_cannot_exceed_its_parent() {
    let mut set = DelegationSet::new();
    let mut root = GrantSpec::new(0, 1, None);
    root.perms = Permissions::of(&[Permission::Read, Permission::Delegate]);
    root.scope = scope(&[&[1]]);
    let root = admit(&mut set, &root);

    let mut wider_perms = GrantSpec::new(1, 2, Some(root));
    wider_perms.perms = Permissions::of(&[Permission::Read, Permission::Edit]);
    wider_perms.scope = scope(&[&[1, 2]]);
    let wider_perms = admit(&mut set, &wider_perms);

    let mut wider_scope = GrantSpec::new(1, 2, Some(root));
    wider_scope.perms = Permissions::of(&[Permission::Read]);
    wider_scope.scope = scope(&[&[1, 2], &[3]]);
    wider_scope.nonce = 2;
    let wider_scope = admit(&mut set, &wider_scope);

    let mut not_chained = GrantSpec::new(3, 2, Some(root));
    not_chained.perms = Permissions::of(&[Permission::Read]);
    let not_chained = admit(&mut set, &not_chained);

    let p = Profile::INITIAL;
    let read = request(2, &[1, 2], Permission::Read);
    assert_eq!(
        set.authorize(&read, Some(&wider_perms), NOW, &p),
        Err(DelegationError::PermissionExceeded)
    );
    assert_eq!(
        set.authorize(&read, Some(&wider_scope), NOW, &p),
        Err(DelegationError::ScopeExceeded)
    );
    assert_eq!(
        set.authorize(&read, Some(&not_chained), NOW, &p),
        Err(DelegationError::BrokenChain)
    );
}

#[test]
fn a_root_must_come_from_the_space_key() {
    let mut set = DelegationSet::new();
    let root = admit(&mut set, &GrantSpec::new(1, 2, None));
    assert_eq!(
        set.authorize(
            &request(2, &[], Permission::Read),
            Some(&root),
            NOW,
            &Profile::INITIAL
        ),
        Err(DelegationError::NotAuthorized(
            "root not granted by the space"
        ))
    );
    let mut elsewhere = GrantSpec::new(5, 2, None);
    elsewhere.space = 5;
    let elsewhere = admit(&mut set, &elsewhere);
    assert_eq!(
        set.authorize(
            &request(2, &[], Permission::Read),
            Some(&elsewhere),
            NOW,
            &Profile::INITIAL
        ),
        Err(DelegationError::WrongSpace)
    );
}

#[test]
fn a_missing_parent_and_a_repeated_grantor_fail_closed() {
    let mut set = DelegationSet::new();
    let root_spec = GrantSpec::new(0, 1, None);
    let root_id = root_spec.sign().id(HashProfile::RawBlake3_256).unwrap();
    let orphan = admit(&mut set, &GrantSpec::new(1, 2, Some(root_id)));
    let p = Profile::INITIAL;
    assert_eq!(
        set.authorize(&request(2, &[], Permission::Read), Some(&orphan), NOW, &p),
        Err(DelegationError::MissingLink)
    );
    assert_eq!(
        set.authorize(&request(2, &[], Permission::Read), Some(&root_id), NOW, &p),
        Err(DelegationError::MissingLink)
    );

    let root = admit(&mut set, &root_spec);
    assert_eq!(root, root_id);
    let back = admit(&mut set, &GrantSpec::new(1, 2, Some(root)));
    let mut again = GrantSpec::new(2, 1, Some(back));
    again.nonce = 9;
    let again = admit(&mut set, &again);
    let mut leaf = GrantSpec::new(1, 3, Some(again));
    leaf.nonce = 10;
    let leaf = admit(&mut set, &leaf);
    assert_eq!(
        set.authorize(&request(3, &[], Permission::Read), Some(&leaf), NOW, &p),
        Err(DelegationError::DelegationCycle)
    );
    // A key cannot authorize itself through a grant it issued.
    let mut own = GrantSpec::new(2, 2, Some(back));
    own.nonce = 11;
    let own = admit(&mut set, &own);
    assert_eq!(
        set.authorize(&request(2, &[], Permission::Read), Some(&own), NOW, &p),
        Err(DelegationError::DelegationCycle)
    );
}

#[test]
fn the_depth_budget_is_exact() {
    let mut set = DelegationSet::new();
    let mut tip = admit(&mut set, &GrantSpec::new(0, 1, None));
    // Keys 1..=16 each grant to the next: 16 grants end at key 16.
    for seed in 1..16u8 {
        tip = admit(&mut set, &GrantSpec::new(seed, seed + 1, Some(tip)));
    }
    let p = Profile::INITIAL;
    assert_eq!(
        set.authorize(&request(16, &[], Permission::Read), Some(&tip), NOW, &p),
        Ok(Authority::Delegated {
            depth: 16,
            stale: false
        })
    );
    let deeper = admit(&mut set, &GrantSpec::new(16, 17, Some(tip)));
    assert_eq!(
        set.authorize(&request(17, &[], Permission::Read), Some(&deeper), NOW, &p),
        Err(DelegationError::DelegationChainTooDeep)
    );
}

#[test]
fn windows_allow_exactly_the_skew() {
    let mut set = DelegationSet::new();
    let mut spec = GrantSpec::new(0, 1, None);
    spec.window = (1_300, 1_400);
    let early = admit(&mut set, &spec);
    let p = Profile::INITIAL;
    let at = |utc| Clock {
        verifier_utc: utc,
        frontier_age_secs: 0,
    };
    let read = request(1, &[], Permission::Read);
    assert_eq!(
        set.authorize(&read, Some(&early), at(999), &p),
        Err(DelegationError::NotYetValid)
    );
    assert!(set.authorize(&read, Some(&early), at(1_000), &p).is_ok());
    assert!(set.authorize(&read, Some(&early), at(1_700), &p).is_ok());
    assert_eq!(
        set.authorize(&read, Some(&early), at(1_701), &p),
        Err(DelegationError::Expired)
    );
}

#[test]
fn a_stale_frontier_blocks_writes_and_marks_reads() {
    let mut set = DelegationSet::new();
    let root = admit(&mut set, &GrantSpec::new(0, 1, None));
    let p = Profile::INITIAL;
    let old = Clock {
        verifier_utc: 1_000,
        frontier_age_secs: p.max_revocation_staleness_secs + 1,
    };
    let edge = Clock {
        verifier_utc: 1_000,
        frontier_age_secs: p.max_revocation_staleness_secs,
    };
    for permission in [
        Permission::Append,
        Permission::Edit,
        Permission::Moderate,
        Permission::Delegate,
    ] {
        assert_eq!(
            set.authorize(&request(1, &[], permission), Some(&root), old, &p),
            Err(DelegationError::StaleFrontier)
        );
        assert!(
            set.authorize(&request(1, &[], permission), Some(&root), edge, &p)
                .is_ok()
        );
    }
    assert_eq!(
        set.authorize(&request(1, &[], Permission::Read), Some(&root), old, &p),
        Ok(Authority::Delegated {
            depth: 1,
            stale: true
        })
    );
    // The space's own key is never blocked by a stale frontier.
    assert_eq!(
        set.authorize(&request(0, &[], Permission::Edit), None, old, &p),
        Ok(Authority::Owner)
    );
}

#[test]
fn nonces_replay_idempotently_and_conflict_otherwise() {
    let mut set = DelegationSet::new();
    let spec = GrantSpec::new(0, 1, None);
    let first = admit(&mut set, &spec);
    assert_eq!(set.admit_grant(&spec.sign(), &dispatch()), Ok(first));
    let mut other = GrantSpec::new(0, 2, None);
    other.nonce = spec.nonce;
    assert_eq!(
        set.admit_grant(&other.sign(), &dispatch()),
        Err(DelegationError::ReplayConflict)
    );
    // The same nonce from another grantor is a separate slot.
    let mut elsewhere = GrantSpec::new(1, 2, Some(first));
    elsewhere.nonce = spec.nonce;
    assert!(set.admit_grant(&elsewhere.sign(), &dispatch()).is_ok());
    assert_eq!(set.grant(&first), Some(&spec.grant()));
}

#[test]
fn grants_must_be_signed_by_the_grantor_for_this_purpose() {
    let spec = GrantSpec::new(0, 1, None);
    assert_eq!(
        spec.grant().sign(SIGNER, &key(3)),
        Err(DelegationError::WrongSigner)
    );
    let payload = spec.grant().payload_bytes().unwrap();
    let mut set = DelegationSet::new();
    let forged = record(GRANT_DOMAIN, "delegation/v1", payload.clone(), &key(3));
    assert_eq!(
        set.admit_grant(&forged, &dispatch()),
        Err(DelegationError::WrongSigner)
    );
    let wrong_domain = record("id/other", "delegation/v1", payload.clone(), &key(0));
    assert_eq!(
        set.admit_grant(&wrong_domain, &dispatch())
            .map_err(|e| e.class()),
        Err("wrong_domain")
    );
    let wrong_profile = record(GRANT_DOMAIN, "delegation/v2", payload.clone(), &key(0));
    assert_eq!(
        set.admit_grant(&wrong_profile, &dispatch())
            .map_err(|e| e.class()),
        Err("wrong_domain")
    );
    let mut tampered = spec.sign();
    tampered.payload[3] ^= 1;
    assert_eq!(
        set.admit_grant(&tampered, &dispatch())
            .map_err(|e| e.class()),
        Err("bad_signature")
    );
    let revocation = sign_revocation(
        record(GRANT_DOMAIN, "delegation/v1", payload, &key(0))
            .id(HashProfile::RawBlake3_256)
            .unwrap(),
        SIGNER,
        &key(0),
    )
    .unwrap();
    assert_eq!(
        set.admit_grant(&revocation, &dispatch())
            .map_err(|e| e.class()),
        Err("wrong_domain")
    );
}

fn grant_value(spec: &GrantSpec) -> Vec<Value> {
    let Value::Array(items) = decode_canonical(&spec.grant().payload_bytes().unwrap()).unwrap()
    else {
        panic!("grant payload is an array");
    };
    items
}

fn signed_payload(items: Vec<Value>) -> Envelope {
    record(
        GRANT_DOMAIN,
        "delegation/v1",
        encode_canonical(&Value::Array(items)).unwrap(),
        &key(0),
    )
}

#[test]
fn grant_payloads_are_canonical_and_bounded() {
    let mut spec = GrantSpec::new(0, 1, None);
    spec.scope = scope(&[&[2], &[1, 5], &[1, 5]]);
    let grant = spec.grant();
    assert_eq!(grant.scope.prefixes().len(), 2);
    assert_eq!(
        Grant::from_payload(&grant.payload_bytes().unwrap()),
        Ok(grant)
    );

    let malformed = |items: Vec<Value>| {
        let mut set = DelegationSet::new();
        set.admit_grant(&signed_payload(items), &dispatch())
            .map(|_| ())
            .map_err(|e| e.class())
    };
    // Scope prefixes out of canonical order.
    let mut items = grant_value(&spec);
    let Value::Array(prefixes) = &mut items[4] else {
        panic!("scope is an array");
    };
    prefixes.reverse();
    assert_eq!(malformed(items), Err("malformed"));
    // Unknown permission bits.
    let mut items = grant_value(&spec);
    items[5] = Value::UInt(32);
    assert_eq!(malformed(items), Err("malformed"));
    let mut items = grant_value(&spec);
    items[5] = Value::UInt(256 + 1);
    assert_eq!(malformed(items), Err("malformed"));
    // A window that ends before it starts.
    let mut items = grant_value(&spec);
    items[6] = Value::UInt(5);
    items[7] = Value::UInt(4);
    assert_eq!(malformed(items), Err("malformed"));
    // A one-second window is fine.
    assert_eq!(window_ok(&spec, 4, 4), Ok(()));
    // Empty, oversize and non-bytes nonces.
    for nonce in [
        Value::Bytes(vec![]),
        Value::Bytes(vec![0; 65]),
        Value::UInt(1),
    ] {
        let mut items = grant_value(&spec);
        items[8] = nonce;
        assert_eq!(malformed(items), Err("malformed"));
    }
    let mut items = grant_value(&spec);
    items[8] = Value::Bytes(vec![0; 64]);
    assert_eq!(malformed(items), Ok(()));
    // Version, length, keys, space and predecessor shapes.
    let mut items = grant_value(&spec);
    items[0] = Value::UInt(2);
    assert_eq!(malformed(items), Err("malformed"));
    let mut items = grant_value(&spec);
    items.push(Value::Null);
    assert_eq!(malformed(items), Err("malformed"));
    let mut items = grant_value(&spec);
    items[3] = Value::Bytes(vec![1; 31]);
    assert_eq!(malformed(items), Err("malformed"));
    let mut items = grant_value(&spec);
    items[1] = Value::Bytes(vec![1; 8]);
    assert_eq!(malformed(items), Err("malformed"));
    let mut items = grant_value(&spec);
    items[9] = Value::Bytes(vec![1; 8]);
    assert_eq!(malformed(items), Err("malformed"));
    assert_eq!(
        Scope::new(vec![vec![step(0)]; 65]).map_err(|e| e.class()),
        Err("malformed")
    );
    let most: Vec<Vec<Term>> = (0..64u8).map(|i| vec![step(i)]).collect();
    assert_eq!(Scope::new(most).unwrap().prefixes().len(), 64);
}

fn window_ok(spec: &GrantSpec, nb: u64, exp: u64) -> Result<(), &'static str> {
    let mut items = grant_value(spec);
    items[6] = Value::UInt(nb);
    items[7] = Value::UInt(exp);
    let mut set = DelegationSet::new();
    set.admit_grant(&signed_payload(items), &dispatch())
        .map(|_| ())
        .map_err(|e| e.class())
}

#[test]
fn revocations_need_the_grantor_or_the_space_key() {
    let mut set = DelegationSet::new();
    let root = admit(&mut set, &GrantSpec::new(0, 1, None));
    let leaf = admit(&mut set, &GrantSpec::new(1, 2, Some(root)));
    let mut sibling = GrantSpec::new(1, 3, Some(root));
    sibling.nonce = 2;
    let sibling = admit(&mut set, &sibling);
    let p = Profile::INITIAL;
    let read = |seed| request(seed, &[], Permission::Read);

    let by_stranger = sign_revocation(leaf, SIGNER, &key(4)).unwrap();
    assert_eq!(
        set.admit_revocation(&by_stranger, &dispatch()),
        Err(DelegationError::WrongSigner)
    );
    let by_grantee = sign_revocation(leaf, SIGNER, &key(2)).unwrap();
    assert_eq!(
        set.admit_revocation(&by_grantee, &dispatch()),
        Err(DelegationError::WrongSigner)
    );
    let unknown = sign_revocation(
        GrantSpec::new(0, 4, None)
            .sign()
            .id(HashProfile::RawBlake3_256)
            .unwrap(),
        SIGNER,
        &key(0),
    )
    .unwrap();
    assert_eq!(
        set.admit_revocation(&unknown, &dispatch()),
        Err(DelegationError::UnknownGrant)
    );
    assert!(set.authorize(&read(2), Some(&leaf), NOW, &p).is_ok());

    let by_grantor = sign_revocation(leaf, SIGNER, &key(1)).unwrap();
    assert_eq!(set.admit_revocation(&by_grantor, &dispatch()), Ok(leaf));
    assert_eq!(
        set.authorize(&read(2), Some(&leaf), NOW, &p),
        Err(DelegationError::Revoked)
    );
    assert!(set.authorize(&read(3), Some(&sibling), NOW, &p).is_ok());

    // The space's key revokes a grant it did not issue directly, and
    // revoking a parent ends every chain under it.
    let by_owner = sign_revocation(sibling, SIGNER, &key(0)).unwrap();
    assert_eq!(set.admit_revocation(&by_owner, &dispatch()), Ok(sibling));
    let by_owner_root = sign_revocation(root, SIGNER, &key(0)).unwrap();
    assert_eq!(set.admit_revocation(&by_owner_root, &dispatch()), Ok(root));
    assert_eq!(
        set.authorize(&read(1), Some(&root), NOW, &p),
        Err(DelegationError::Revoked)
    );

    let grant_as_revocation = GrantSpec::new(0, 1, None).sign();
    assert_eq!(
        set.admit_revocation(&grant_as_revocation, &dispatch())
            .map_err(|e| e.class()),
        Err("wrong_domain")
    );
    let bad_payload = record(
        REVOCATION_DOMAIN,
        "delegation/v1",
        encode_canonical(&Value::Array(vec![
            Value::UInt(1),
            Value::Bytes(vec![0; 4]),
        ]))
        .unwrap(),
        &key(0),
    );
    assert_eq!(
        set.admit_revocation(&bad_payload, &dispatch())
            .map_err(|e| e.class()),
        Err("malformed")
    );
}

fn statement() -> TransferStatement {
    TransferStatement {
        space: space(0),
        old_key: public(0),
        new_key: public(1),
        scope: scope(&[&[]]),
        nonce: vec![7],
        challenge: b"challenge".to_vec(),
        expires: 1_100,
    }
}

#[test]
fn a_transfer_needs_both_keys_over_identical_bytes() {
    let s = statement();
    let offer = s.sign_offer(SIGNER, &key(0)).unwrap();
    let accept = s.sign_acceptance(SIGNER, &key(1)).unwrap();
    let p = Profile::INITIAL;
    let d = dispatch();
    let verified = verify_transfer(&offer, &accept, &public(0), NOW, &p, &d).unwrap();
    assert_eq!(verified, s);
    // The address stays the one the old key named.
    assert_eq!(verified.space, space(0));
    assert_eq!(verified.space.key(), &public(0));

    assert_eq!(
        s.sign_offer(SIGNER, &key(1)),
        Err(DelegationError::WrongSigner)
    );
    assert_eq!(
        s.sign_acceptance(SIGNER, &key(0)),
        Err(DelegationError::WrongSigner)
    );
    // The new key alone, or the old key alone, is not enough.
    let forged_offer = record(
        TRANSFER_OFFER_DOMAIN,
        "delegation/v1",
        s.payload_bytes().unwrap(),
        &key(1),
    );
    assert_eq!(
        verify_transfer(&forged_offer, &accept, &public(0), NOW, &p, &d),
        Err(DelegationError::WrongSigner)
    );
    let forged_accept = record(
        TRANSFER_ACCEPT_DOMAIN,
        "delegation/v1",
        s.payload_bytes().unwrap(),
        &key(0),
    );
    assert_eq!(
        verify_transfer(&offer, &forged_accept, &public(0), NOW, &p, &d),
        Err(DelegationError::WrongSigner)
    );
    // The offer and the acceptance are not interchangeable.
    assert_eq!(
        verify_transfer(&accept, &offer, &public(0), NOW, &p, &d).map_err(|e| e.class()),
        Err("wrong_domain")
    );
    // Any differing field breaks the pairing.
    for change in 0..5 {
        let mut other = s.clone();
        match change {
            0 => other.challenge = b"other".to_vec(),
            1 => other.nonce = vec![8],
            2 => other.expires += 1,
            3 => other.scope = scope(&[&[1]]),
            _ => other.space = space(2),
        }
        let accept = other.sign_acceptance(SIGNER, &key(1)).unwrap();
        assert_eq!(
            verify_transfer(&offer, &accept, &public(0), NOW, &p, &d),
            Err(DelegationError::TransferMismatch)
        );
    }
    // Only the current controller can transfer.
    assert_eq!(
        verify_transfer(&offer, &accept, &public(2), NOW, &p, &d),
        Err(DelegationError::WrongSigner)
    );
    // Expiry honours the skew allowance exactly.
    let late = |utc| Clock {
        verifier_utc: utc,
        frontier_age_secs: 0,
    };
    assert!(verify_transfer(&offer, &accept, &public(0), late(1_400), &p, &d).is_ok());
    assert_eq!(
        verify_transfer(&offer, &accept, &public(0), late(1_401), &p, &d),
        Err(DelegationError::Expired)
    );
    // A transfer to the same key is refused.
    let mut same = s;
    same.new_key = public(0);
    let offer = same.sign_offer(SIGNER, &key(0)).unwrap();
    let accept = same.sign_acceptance(SIGNER, &key(0)).unwrap();
    assert_eq!(
        verify_transfer(&offer, &accept, &public(0), NOW, &p, &d),
        Err(DelegationError::SameKey)
    );
}

#[test]
fn transfer_payloads_are_checked() {
    let s = statement();
    let Value::Array(items) = decode_canonical(&s.payload_bytes().unwrap()).unwrap() else {
        panic!("statement is an array");
    };
    let p = Profile::INITIAL;
    let check = |items: Vec<Value>| {
        let bytes = encode_canonical(&Value::Array(items)).unwrap();
        let offer = record(
            TRANSFER_OFFER_DOMAIN,
            "delegation/v1",
            bytes.clone(),
            &key(0),
        );
        let accept = record(TRANSFER_ACCEPT_DOMAIN, "delegation/v1", bytes, &key(1));
        verify_transfer(&offer, &accept, &public(0), NOW, &p, &dispatch()).map_err(|e| e.class())
    };
    assert!(check(items.clone()).is_ok());
    let mut purpose = items.clone();
    purpose[1] = Value::Text("id/other".to_owned());
    assert_eq!(check(purpose), Err("malformed"));
    let mut challenge = items.clone();
    challenge[7] = Value::UInt(1);
    assert_eq!(check(challenge), Err("malformed"));
    let mut short = items;
    short.pop();
    assert_eq!(check(short), Err("malformed"));
}

#[test]
fn error_classes_are_stable() {
    let all = [
        (DelegationError::Malformed("x"), "malformed"),
        (DelegationError::WrongDomain(String::new()), "wrong_domain"),
        (DelegationError::WrongSigner, "wrong_signer"),
        (DelegationError::ReplayConflict, "replay_conflict"),
        (DelegationError::UnknownGrant, "unknown_grant"),
        (DelegationError::NotAuthorized("x"), "not_authorized"),
        (DelegationError::WrongSpace, "wrong_space"),
        (DelegationError::Revoked, "revoked"),
        (DelegationError::NotYetValid, "not_yet_valid"),
        (DelegationError::Expired, "expired"),
        (DelegationError::BrokenChain, "broken_chain"),
        (DelegationError::NotDelegable, "not_delegable"),
        (DelegationError::PermissionExceeded, "permission_exceeded"),
        (DelegationError::ScopeExceeded, "scope_exceeded"),
        (DelegationError::MissingLink, "missing_link"),
        (DelegationError::DelegationCycle, "delegation_cycle"),
        (
            DelegationError::DelegationChainTooDeep,
            "delegation_chain_too_deep",
        ),
        (DelegationError::StaleFrontier, "stale_frontier"),
        (DelegationError::TransferMismatch, "transfer_mismatch"),
        (DelegationError::SameKey, "same_key"),
    ];
    let mut seen = HashSet::new();
    for (error, class) in all {
        assert_eq!(error.class(), class);
        assert!(seen.insert(class));
        assert!(!error.to_string().is_empty());
    }
}

// ---------------------------------------------------------------------
// Independent model: grants are plain records over key seeds, scopes are
// sets of byte-string prefixes, and permissions are bitmasks.

const OWNER: u8 = 0;
const KEYS: u8 = 5;

#[derive(Debug, Clone)]
struct MGrant {
    other_space: bool,
    grantor: u8,
    grantee: u8,
    prefixes: Vec<Vec<u8>>,
    perms: u8,
    nb: u64,
    exp: u64,
    pred: Option<usize>,
    admitted: bool,
    revoked: bool,
}

#[derive(Debug, Clone)]
struct MRequest {
    key: u8,
    leaf: Option<usize>,
    perm: u8,
    path: Vec<u8>,
    utc: u64,
    age: u64,
    max_depth: usize,
    skew: u64,
    stale_after: u64,
}

const READ: u8 = 1;
const DELEGATE: u8 = 16;

fn m_covers(prefixes: &[Vec<u8>], path: &[u8]) -> bool {
    prefixes
        .iter()
        .any(|p| p.len() <= path.len() && path[..p.len()] == p[..])
}

/// The model's decision, as `Ok((depth, stale))` or a failure class.
fn model(grants: &[MGrant], r: &MRequest) -> Result<Option<(usize, bool)>, &'static str> {
    if r.key == OWNER {
        return Ok(None);
    }
    let stale = r.age > r.stale_after;
    if stale && r.perm != READ {
        return Err("stale_frontier");
    }
    let Some(leaf) = r.leaf else {
        return Err("not_authorized");
    };
    // The grant indices from leaf to root.
    let mut chain = vec![leaf];
    while let Some(pred) = grants[*chain.last().unwrap()].pred {
        chain.push(pred);
    }
    let mut used: Vec<u8> = vec![r.key];
    for (pos, &i) in chain.iter().enumerate() {
        let g = &grants[i];
        if !g.admitted {
            return Err("missing_link");
        }
        if pos >= r.max_depth {
            return Err("delegation_chain_too_deep");
        }
        if used.contains(&g.grantor) {
            return Err("delegation_cycle");
        }
        used.push(g.grantor);
        if g.other_space {
            return Err("wrong_space");
        }
        if g.revoked {
            return Err("revoked");
        }
        if g.nb > r.utc + r.skew {
            return Err("not_yet_valid");
        }
        if g.exp + r.skew < r.utc {
            return Err("expired");
        }
        if pos == 0 {
            if g.grantee != r.key || g.perms & r.perm == 0 || !m_covers(&g.prefixes, &r.path) {
                return Err("not_authorized");
            }
        } else {
            let c = &grants[chain[pos - 1]];
            if g.grantee != c.grantor {
                return Err("broken_chain");
            }
            if g.perms & DELEGATE == 0 {
                return Err("not_delegable");
            }
            if c.perms & !g.perms != 0 {
                return Err("permission_exceeded");
            }
            if !c.prefixes.iter().all(|p| m_covers(&g.prefixes, p)) {
                return Err("scope_exceeded");
            }
        }
    }
    let root = &grants[*chain.last().unwrap()];
    if root.grantor == OWNER {
        Ok(Some((chain.len(), stale)))
    } else {
        Err("not_authorized")
    }
}

fn prefix() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(0..2u8, 0..3)
}

fn m_grant(index: usize) -> impl Strategy<Value = MGrant> {
    (
        prop::bool::weighted(0.05),
        prop_oneof![3 => Just(OWNER), 7 => 1..KEYS],
        0..KEYS,
        prop::collection::vec(prefix(), 0..3),
        prop_oneof![4 => Just(31u8), 6 => 0..32u8],
        (0..20u64, 0..20u64),
        prop_oneof![
            1 => Just(None),
            if index == 0 { 0 } else { 4 } => (0..index.max(1)).prop_map(Some),
        ],
        prop::bool::weighted(0.9),
        prop::bool::weighted(0.08),
    )
        .prop_map(
            |(other_space, grantor, grantee, prefixes, perms, (a, b), pred, admitted, revoked)| {
                MGrant {
                    other_space,
                    grantor,
                    grantee,
                    prefixes,
                    perms,
                    nb: a.min(b),
                    exp: a.max(b),
                    pred,
                    admitted,
                    revoked,
                }
            },
        )
}

fn forest() -> impl Strategy<Value = Vec<MGrant>> {
    (1..9usize).prop_flat_map(|n| (0..n).map(m_grant).collect::<Vec<_>>())
}

fn m_request(n: usize) -> impl Strategy<Value = MRequest> {
    (
        prop_oneof![1 => Just(OWNER), 9 => 1..KEYS],
        prop_oneof![1 => Just(None), 9 => (0..n).prop_map(Some)],
        prop::sample::select(vec![1u8, 2, 4, 8, 16]),
        prop::collection::vec(0..2u8, 0..4),
        0..25u64,
        prop_oneof![9 => Just(0u64), 1 => 0..20u64],
        1..6usize,
        0..3u64,
        5..15u64,
    )
        .prop_map(
            |(key, leaf, perm, path, utc, age, max_depth, skew, stale_after)| MRequest {
                key,
                leaf,
                perm,
                path,
                utc,
                age,
                max_depth,
                skew,
                stale_after,
            },
        )
}

const fn permission(bit: u8) -> Permission {
    match bit {
        1 => Permission::Read,
        2 => Permission::Append,
        4 => Permission::Edit,
        8 => Permission::Moderate,
        _ => Permission::Delegate,
    }
}

/// Builds the module's set from the model's records.
fn realize(grants: &[MGrant]) -> (DelegationSet, Vec<EnvelopeId>) {
    let mut set = DelegationSet::new();
    let mut ids: Vec<EnvelopeId> = Vec::new();
    for (i, g) in grants.iter().enumerate() {
        let spec = GrantSpec {
            space: if g.other_space { KEYS } else { OWNER },
            grantor: g.grantor,
            grantee: g.grantee,
            scope: Scope::new(g.prefixes.iter().map(|p| path(p)).collect()).unwrap(),
            perms: Permissions::from_bits(g.perms).unwrap(),
            window: (g.nb, g.exp),
            nonce: i as u8,
            pred: g.pred.map(|p| ids[p]),
        };
        let envelope = spec.sign();
        let id = envelope.id(HashProfile::RawBlake3_256).unwrap();
        if g.admitted {
            assert_eq!(set.admit_grant(&envelope, &dispatch()), Ok(id));
        }
        ids.push(id);
    }
    for (g, id) in grants.iter().zip(&ids) {
        if g.admitted && g.revoked {
            let revocation = sign_revocation(*id, SIGNER, &key(g.grantor)).unwrap();
            assert_eq!(set.admit_revocation(&revocation, &dispatch()), Ok(*id));
        }
    }
    (set, ids)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn authorization_agrees_with_the_model(
        (grants, requests) in forest().prop_flat_map(|g| {
            let n = g.len();
            (Just(g), prop::collection::vec(m_request(n), 1..6))
        })
    ) {
        let mut grants = grants;
        // A revocation can only name an admitted grant.
        for g in &mut grants {
            g.revoked &= g.admitted;
        }
        let (set, ids) = realize(&grants);
        for r in &requests {
            let request = Request {
                space: space(OWNER),
                key: public(r.key),
                path: path(&r.path),
                permission: permission(r.perm),
            };
            let clock = Clock { verifier_utc: r.utc, frontier_age_secs: r.age };
            let profile = Profile {
                max_depth: r.max_depth,
                clock_skew_secs: r.skew,
                max_revocation_staleness_secs: r.stale_after,
            };
            let got = set
                .authorize(&request, r.leaf.map(|i| &ids[i]), clock, &profile)
                .map(|a| match a {
                    Authority::Owner => None,
                    Authority::Delegated { depth, stale } => Some((depth, stale)),
                })
                .map_err(|e| e.class());
            prop_assert_eq!(got, model(&grants, r), "request {:?}", r);
        }
    }

    #[test]
    fn scope_containment_agrees_with_the_model(
        parent in prop::collection::vec(prefix(), 0..4),
        child in prop::collection::vec(prefix(), 0..4),
        probe in prop::collection::vec(0..2u8, 0..4),
    ) {
        let p = Scope::new(parent.iter().map(|x| path(x)).collect()).unwrap();
        let c = Scope::new(child.iter().map(|x| path(x)).collect()).unwrap();
        prop_assert_eq!(p.covers_path(&path(&probe)), m_covers(&parent, &probe));
        let contained = child.iter().all(|x| m_covers(&parent, x));
        prop_assert_eq!(p.contains(&c), contained);
        // Containment means every path the child covers, the parent covers.
        if contained && m_covers(&child, &probe) {
            prop_assert!(m_covers(&parent, &probe));
        }
    }

    #[test]
    fn grants_round_trip_through_their_payload(
        g in m_grant(0),
        nonce in prop::collection::vec(any::<u8>(), 1..=64),
    ) {
        let grant = Grant {
            space: space(OWNER),
            grantor_key: public(g.grantor),
            grantee_key: public(g.grantee),
            scope: Scope::new(g.prefixes.iter().map(|p| path(p)).collect()).unwrap(),
            permissions: Permissions::from_bits(g.perms).unwrap(),
            not_before: g.nb,
            expires: g.exp,
            nonce,
            predecessor: None,
        };
        let bytes = grant.payload_bytes().unwrap();
        prop_assert_eq!(Grant::from_payload(&bytes), Ok(grant));
    }
}
