//! B17 golden and property test: signed queries, relay grants and
//! end-to-end responder provenance (#10249, statements S1.58-S1.62).
//!
//! Oracle: **golden** — the preimages are rebuilt here by an encoder
//! written from the slice's documented layout, the signatures are
//! checked against those bytes with Ed25519 directly, and the digests
//! of a fixed scenario are pinned to hex values computed independently
//! (Python `cryptography` Ed25519 and `hashlib` SHA-256 over the same
//! layout). Properties then build random chains, including paths that
//! revisit a peer, and check that every valid chain and response
//! verifies and that any single change to any signed field, a forged
//! responder, a reordered or missing receipt, or a grant past the hop
//! bound is refused.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B17 reaches B20
//! through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b20_request_identity.rs"]
#[allow(dead_code)]
mod b20_request_identity;

#[path = "../src/federation/slices/b17_query_grants.rs"]
mod b17_query_grants;

use b17_query_grants::{
    ChainError, Completeness, ForwardingReceipt, GrantPath, HOP_CEILING, OriginQuery, QueryError,
    RelayGrant, RelayedResponse, ResponseError, SignedResponse, VerifiedResponse, verify_chain,
    verify_response,
};
use b20_request_identity::{PeerKey, QueryBinding, RequestId};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

const NOW: u64 = 500;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn id(seed: u8) -> PeerKey {
    key(seed).verifying_key().to_bytes()
}

fn binding(max_hops: u32) -> QueryBinding {
    QueryBinding {
        query_digest: [9; 32],
        scope: b"public".to_vec(),
        issued_at: 100,
        expires_at: 1000,
        initial_max_hops: max_hops,
    }
}

fn query(max_hops: u32) -> OriginQuery {
    OriginQuery::sign(&key(1), [7; 32], binding(max_hops))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        write!(out, "{b:02x}").unwrap();
        out
    })
}

fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn ed_verifies(signer: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(signer)
        .unwrap()
        .verify_strict(message, &Signature::from_bytes(signature))
        .is_ok()
}

fn response(responder: u8, request_id: RequestId, results: &[&[u8]]) -> SignedResponse {
    let mut response = SignedResponse {
        request_id,
        responder_id: id(responder),
        scope: b"public".to_vec(),
        policy_digest: [4; 32],
        completeness: Completeness::Complete,
        results: results.iter().map(|r| r.to_vec()).collect(),
        generated_at: 480,
        signature: [0; 64],
    };
    response.sign(&key(responder));
    response
}

/// The query relayed by each seed in `relays` in turn, the first being
/// the peer asked, down to `responder`, with each relay's receipt.
fn relayed(query: &OriginQuery, relays: &[u8], responder: u8, expiries: &[u64]) -> RelayedResponse {
    let mut grants: Vec<RelayGrant> = Vec::new();
    for (i, relay) in relays.iter().enumerate() {
        let child = relays.get(i + 1).copied().unwrap_or(responder);
        let grant = RelayGrant::issue(&key(*relay), query, grants.last(), id(child), expiries[i]);
        grants.push(grant);
    }
    let response = response(responder, query.request_id, &[b"r1", b"r22"]);
    let receipts = relays
        .iter()
        .zip(&grants)
        .map(|(relay, grant)| ForwardingReceipt::issue(&key(*relay), grant, &response, 490))
        .collect();
    RelayedResponse {
        response,
        grants,
        receipts,
    }
}

fn standard() -> (OriginQuery, RelayedResponse) {
    let query = query(4);
    let relayed = relayed(&query, &[2, 3], 5, &[900, 800]);
    (query, relayed)
}

// ----- golden -------------------------------------------------------

fn be32(n: usize) -> [u8; 4] {
    u32::try_from(n).unwrap().to_be_bytes()
}

fn expected_origin_preimage(query: &OriginQuery) -> Vec<u8> {
    let b = &query.binding;
    let mut out = b"id-sync/b17-origin-query/v1".to_vec();
    out.extend_from_slice(&query.request_id.0);
    out.extend_from_slice(&query.origin);
    out.extend_from_slice(&query.nonce);
    out.extend_from_slice(&b.query_digest);
    out.extend_from_slice(&be32(b.scope.len()));
    out.extend_from_slice(&b.scope);
    out.extend_from_slice(&b.issued_at.to_be_bytes());
    out.extend_from_slice(&b.expires_at.to_be_bytes());
    out.extend_from_slice(&b.initial_max_hops.to_be_bytes());
    out
}

fn expected_grant_preimage(g: &RelayGrant) -> Vec<u8> {
    let mut out = b"id-sync/b17-relay-grant/v1".to_vec();
    out.extend_from_slice(&g.request_id.0);
    out.extend_from_slice(&g.origin_query_digest);
    if let Some(parent) = g.parent_grant_digest {
        out.push(1);
        out.extend_from_slice(&parent);
    } else {
        out.push(0);
    }
    out.extend_from_slice(&g.relay_id);
    out.extend_from_slice(&g.child_id);
    out.extend_from_slice(&g.hop_index.to_be_bytes());
    out.extend_from_slice(&be32(g.path.len()));
    for peer in &g.path {
        out.extend_from_slice(peer);
    }
    out.extend_from_slice(&g.expires_at.to_be_bytes());
    out
}

fn expected_response_preimage(r: &SignedResponse) -> Vec<u8> {
    let mut out = b"id-sync/b17-response/v1".to_vec();
    out.extend_from_slice(&r.request_id.0);
    out.extend_from_slice(&r.responder_id);
    out.extend_from_slice(&be32(r.scope.len()));
    out.extend_from_slice(&r.scope);
    out.extend_from_slice(&r.policy_digest);
    out.push(match r.completeness {
        Completeness::Unstated => 0,
        Completeness::Partial => 1,
        Completeness::Complete => 2,
    });
    out.extend_from_slice(&be32(r.results.len()));
    for result in &r.results {
        out.extend_from_slice(&be32(result.len()));
        out.extend_from_slice(result);
    }
    out.extend_from_slice(&r.generated_at.to_be_bytes());
    out
}

fn expected_receipt_preimage(r: &ForwardingReceipt) -> Vec<u8> {
    let mut out = b"id-sync/b17-forwarding-receipt/v1".to_vec();
    out.extend_from_slice(&r.request_id.0);
    out.extend_from_slice(&r.relay_id);
    out.extend_from_slice(&r.grant_digest);
    out.extend_from_slice(&r.response_digest);
    out.extend_from_slice(&r.forwarded_at.to_be_bytes());
    out
}

#[test]
fn preimages_follow_the_documented_layout_and_signatures_cover_them() {
    let (query, relayed) = standard();
    let pre = expected_origin_preimage(&query);
    assert_eq!(query.preimage(), pre);
    assert!(ed_verifies(&query.origin, &pre, &query.signature));
    assert_eq!(query.digest(), sha(&[&pre, &query.signature]));

    for grant in &relayed.grants {
        let pre = expected_grant_preimage(grant);
        assert_eq!(grant.preimage(), pre);
        assert!(ed_verifies(&grant.relay_id, &pre, &grant.signature));
        assert_eq!(grant.digest(), sha(&[&pre, &grant.signature]));
    }

    let r = &relayed.response;
    let pre = expected_response_preimage(r);
    assert_eq!(r.preimage(), pre);
    assert!(ed_verifies(&r.responder_id, &pre, &r.signature));
    assert_eq!(r.digest(), sha(&[&pre, &r.signature]));

    for receipt in &relayed.receipts {
        let pre = expected_receipt_preimage(receipt);
        assert_eq!(receipt.preimage(), pre);
        assert!(ed_verifies(&receipt.relay_id, &pre, &receipt.signature));
        assert_eq!(receipt.digest(), sha(&[&pre, &receipt.signature]));
    }
}

#[test]
fn golden_digests_of_the_standard_scenario() {
    let (query, relayed) = standard();
    let golden = [
        hex(&query.request_id.0),
        hex(&query.digest()),
        hex(&relayed.grants[0].digest()),
        hex(&relayed.grants[1].digest()),
        hex(&relayed.response.digest()),
        hex(&relayed.receipts[0].digest()),
        hex(&relayed.receipts[1].digest()),
    ];
    assert_eq!(golden, GOLDEN);
}

const GOLDEN: [&str; 7] = [
    "8cbfed6bcd7b73b5d6a40e8c73912bffcbfb7d00094c84d96cf26ed14aec5582",
    "056f1f3007c083125805fa4de5f29efb611992f057440a0aae2eb99fb6df2468",
    "475730197b96b5b7ecde8bd37b019e38d6250479f529137610662f29a9087c5d",
    "41f313b3bb7fc15faa613e947f73b037edcc39a410bafb50f0dcf55cb8f48a15",
    "2a7ee9fdebe1506b5d583908fc5ea9b4675705412590f3fda364324daec13145",
    "300bb5194859da5de7a58abc3c82a1e5d6e70ed1847c627a5ba30c1f8c61596d",
    "43535659314eb7a0517aa349456362652d48ebf051098a608a09734801df24fa",
];

// ----- the standard scenario ----------------------------------------

#[test]
fn a_relayed_response_verifies_with_its_path() {
    let (query, relayed) = standard();
    let path = verify_chain(&query, &relayed.grants, NOW).unwrap();
    assert_eq!(
        path,
        GrantPath {
            relays: vec![id(2), id(3)],
            holder: Some(id(5)),
            remaining_hops: 1,
        }
    );
    assert!(path.visits(&id(2)) && path.visits(&id(3)) && !path.visits(&id(5)));
    assert_eq!(
        verify_response(&query, &id(2), &relayed, NOW),
        Ok(VerifiedResponse {
            responder: id(5),
            relays: vec![id(2), id(3)],
            response_digest: relayed.response.digest(),
        })
    );
}

#[test]
fn a_direct_response_meets_the_same_standard() {
    let query = query(1);
    let direct = relayed(&query, &[], 2, &[]);
    assert_eq!(
        verify_chain(&query, &[], NOW),
        Ok(GrantPath {
            relays: vec![],
            holder: None,
            remaining_hops: 0,
        })
    );
    assert_eq!(
        verify_response(&query, &id(2), &direct, NOW).map(|v| v.responder),
        Ok(id(2))
    );
    assert_eq!(
        verify_response(&query, &id(3), &direct, NOW),
        Err(ResponseError::ResponderMismatch)
    );
    let mut forged = direct;
    forged.response.signature[0] ^= 1;
    assert_eq!(
        verify_response(&query, &id(2), &forged, NOW),
        Err(ResponseError::BadResponderSignature)
    );
}

// ----- origin query -------------------------------------------------

#[test]
fn the_origin_query_is_checked_before_anything_else() {
    let good = query(4);
    assert_eq!(good.verify(NOW), Ok(()));
    assert_eq!(good.verify(999), Ok(()));
    assert_eq!(good.verify(1000), Err(QueryError::Expired));

    let mut changed = good.clone();
    changed.binding.scope = b"private".to_vec();
    assert_eq!(changed.verify(NOW), Err(QueryError::RequestIdMismatch));

    // Re-deriving the id does not help without the origin's key.
    changed.request_id = RequestId::derive(&changed.origin, &changed.nonce, &changed.binding);
    assert_eq!(changed.verify(NOW), Err(QueryError::BadSignature));

    // Another key signing in the origin's name fails too.
    let mut stolen = OriginQuery::sign(&key(6), good.nonce, good.binding.clone());
    stolen.origin = good.origin;
    stolen.request_id = RequestId::derive(&stolen.origin, &stolen.nonce, &stolen.binding);
    assert_eq!(stolen.verify(NOW), Err(QueryError::BadSignature));

    assert_eq!(query(0).verify(NOW), Err(QueryError::HopBound));
    assert_eq!(query(HOP_CEILING).verify(NOW), Ok(()));
    assert_eq!(
        query(HOP_CEILING + 1).verify(NOW),
        Err(QueryError::HopBound)
    );

    let (_, relayed) = standard();
    assert_eq!(
        verify_response(&query(0), &id(2), &relayed, NOW),
        Err(ResponseError::Chain(ChainError::Query(
            QueryError::HopBound
        )))
    );
}

// ----- grant chain --------------------------------------------------

#[test]
fn the_hop_bound_stops_the_chain() {
    // Four hops: the asked peer and up to three relays below it.
    let q = query(4);
    let ok = relayed(&q, &[2, 3, 4], 5, &[900, 900, 900]);
    assert_eq!(verify_chain(&q, &ok.grants, NOW).unwrap().remaining_hops, 0);
    let over = relayed(&q, &[2, 3, 4, 6], 5, &[900; 4]);
    assert_eq!(
        verify_chain(&q, &over.grants, NOW),
        Err(ChainError::HopBoundExceeded { index: 3 })
    );
    // A one-hop query admits no relay at all.
    let q = query(1);
    let one = relayed(&q, &[2], 5, &[900]);
    assert_eq!(
        verify_chain(&q, &one.grants, NOW),
        Err(ChainError::HopBoundExceeded { index: 0 })
    );
}

#[test]
fn a_grant_can_only_narrow_its_expiry() {
    let q = query(4);
    let equal = relayed(&q, &[2, 3], 5, &[1000, 1000]);
    assert!(verify_chain(&q, &equal.grants, NOW).is_ok());
    let past_query = relayed(&q, &[2], 5, &[1001]);
    assert_eq!(
        verify_chain(&q, &past_query.grants, NOW),
        Err(ChainError::ExpiryWidened { index: 0 })
    );
    let past_parent = relayed(&q, &[2, 3], 5, &[800, 801]);
    assert_eq!(
        verify_chain(&q, &past_parent.grants, NOW),
        Err(ChainError::ExpiryWidened { index: 1 })
    );
    let (q, std) = standard();
    assert!(verify_chain(&q, &std.grants, 799).is_ok());
    assert_eq!(
        verify_chain(&q, &std.grants, 800),
        Err(ChainError::Expired { index: 1 })
    );
}

#[test]
fn only_the_granted_child_can_relay_onward() {
    let (q, std) = standard();
    // Peer 6 was never granted, but signs a grant chained to grant 0.
    let mut chain = std.grants;
    chain[1] = RelayGrant::issue(&key(6), &q, Some(&chain[0]), id(5), 800);
    assert_eq!(
        verify_chain(&q, &chain, NOW),
        Err(ChainError::RelayNotGranted { index: 1 })
    );
}

#[test]
fn a_relay_cannot_rewrite_the_path_even_with_its_own_signature() {
    let (q, std) = standard();
    let relay = key(3);

    let mut dropped = std.grants.clone();
    dropped[1].path = vec![id(3)];
    dropped[1].sign(&relay);
    assert_eq!(
        verify_chain(&q, &dropped, NOW),
        Err(ChainError::PathMismatch { index: 1 })
    );

    let mut skipped = std.grants.clone();
    skipped[1].hop_index = 1;
    skipped[1].sign(&relay);
    assert_eq!(
        verify_chain(&q, &skipped, NOW),
        Err(ChainError::HopIndex { index: 1 })
    );

    let mut orphan = std.grants.clone();
    orphan[1].parent_grant_digest = None;
    orphan[1].sign(&relay);
    assert_eq!(
        verify_chain(&q, &orphan, NOW),
        Err(ChainError::BrokenLink { index: 1 })
    );

    let mut first = std.grants;
    first[0].parent_grant_digest = Some([0; 32]);
    first[0].sign(&key(2));
    assert_eq!(
        verify_chain(&q, &first, NOW),
        Err(ChainError::BrokenLink { index: 0 })
    );
}

#[test]
fn a_grant_belongs_to_one_query() {
    let (q, std) = standard();
    let other = OriginQuery::sign(&key(1), [8; 32], binding(4));
    assert_eq!(
        verify_chain(&other, &std.grants, NOW),
        Err(ChainError::WrongRequest { index: 0 })
    );

    // Same request id, but a grant naming another origin-query digest.
    let mut swapped = std.grants;
    swapped[0].origin_query_digest = [0; 32];
    swapped[0].sign(&key(2));
    assert_eq!(
        verify_chain(&q, &swapped, NOW),
        Err(ChainError::WrongOriginQuery { index: 0 })
    );
}

#[test]
fn a_path_that_revisits_a_peer_is_validly_signed_evidence() {
    let q = query(5);
    let looped = relayed(&q, &[2, 3, 2], 4, &[900, 900, 900]);
    let path = verify_chain(&q, &looped.grants, NOW).unwrap();
    assert_eq!(path.relays, vec![id(2), id(3), id(2)]);
    assert!(path.visits(&id(2)));
    assert!(verify_response(&q, &id(2), &looped, NOW).is_ok());
}

// ----- response provenance ------------------------------------------

#[test]
fn no_hop_can_answer_in_the_responders_name() {
    let (q, std) = standard();

    // The last relay signs an answer claiming to be the responder.
    let mut forged = std.clone();
    forged.response.results.push(b"injected".to_vec());
    forged.response.sign(&key(3));
    assert_eq!(
        verify_response(&q, &id(2), &forged, NOW),
        Err(ResponseError::BadResponderSignature)
    );

    // The last relay answers honestly in its own name instead.
    let mut own = std.clone();
    own.response = response(3, q.request_id, &[b"r1"]);
    assert_eq!(
        verify_response(&q, &id(2), &own, NOW),
        Err(ResponseError::ResponderMismatch)
    );

    // A relay edits the result bytes it passes back.
    let mut edited = std.clone();
    edited.response.results[0] = b"r9".to_vec();
    assert_eq!(
        verify_response(&q, &id(2), &edited, NOW),
        Err(ResponseError::BadResponderSignature)
    );

    // A chain that does not start at the peer asked.
    assert_eq!(
        verify_response(&q, &id(6), &std, NOW),
        Err(ResponseError::NotTheAskedPeer)
    );
}

#[test]
fn the_response_must_answer_this_query_in_its_scope() {
    let (q, std) = standard();
    let mut other = std.clone();
    other.response.request_id = RequestId([1; 32]);
    other.response.sign(&key(5));
    assert_eq!(
        verify_response(&q, &id(2), &other, NOW),
        Err(ResponseError::WrongRequest)
    );
    let mut wider = std;
    wider.response.scope = b"private".to_vec();
    wider.response.sign(&key(5));
    assert_eq!(
        verify_response(&q, &id(2), &wider, NOW),
        Err(ResponseError::ScopeMismatch)
    );
}

#[test]
fn every_relay_must_receipt_the_unchanged_response() {
    let (q, std) = standard();

    let mut missing = std.clone();
    missing.receipts.pop();
    assert_eq!(
        verify_response(&q, &id(2), &missing, NOW),
        Err(ResponseError::ReceiptCount)
    );

    let mut reordered = std.clone();
    reordered.receipts.swap(0, 1);
    assert_eq!(
        verify_response(&q, &id(2), &reordered, NOW),
        Err(ResponseError::ReceiptMismatch { index: 0 })
    );

    // A receipt for some other response from the same responder.
    let mut stale = std.clone();
    let earlier = response(5, q.request_id, &[b"old"]);
    stale.receipts[1] = ForwardingReceipt::issue(&key(3), &stale.grants[1], &earlier, 490);
    assert_eq!(
        verify_response(&q, &id(2), &stale, NOW),
        Err(ResponseError::ReceiptMismatch { index: 1 })
    );

    let mut wrong_request = std.clone();
    wrong_request.receipts[0].request_id = RequestId([1; 32]);
    wrong_request.receipts[0].sign(&key(2));
    assert_eq!(
        verify_response(&q, &id(2), &wrong_request, NOW),
        Err(ResponseError::ReceiptMismatch { index: 0 })
    );

    let mut wrong_grant = std.clone();
    wrong_grant.receipts[1].grant_digest = std.grants[0].digest();
    wrong_grant.receipts[1].sign(&key(3));
    assert_eq!(
        verify_response(&q, &id(2), &wrong_grant, NOW),
        Err(ResponseError::ReceiptMismatch { index: 1 })
    );

    // Signed by someone other than the relay it names.
    let mut impostor = std.clone();
    impostor.receipts[0].sign(&key(6));
    assert_eq!(
        verify_response(&q, &id(2), &impostor, NOW),
        Err(ResponseError::BadReceiptSignature { index: 0 })
    );

    let mut extra = std;
    extra.receipts.push(extra.receipts[0].clone());
    assert_eq!(
        verify_response(&q, &id(2), &extra, NOW),
        Err(ResponseError::ReceiptCount)
    );
}

// ----- properties ---------------------------------------------------

/// One change to one signed field of one artifact, without re-signing.
#[derive(Debug, Clone, Copy)]
enum Tamper {
    Query(u8),
    Grant(usize, u8),
    Response(u8),
    Receipt(usize, u8),
}

const fn flip(bytes: &mut [u8]) {
    bytes[0] ^= 0x80;
}

fn apply(tamper: Tamper, query: &mut OriginQuery, relayed: &mut RelayedResponse) -> bool {
    match tamper {
        Tamper::Query(field) => match field % 6 {
            0 => flip(&mut query.nonce),
            1 => query.binding.issued_at ^= 1,
            2 => query.binding.query_digest[3] ^= 1,
            3 => flip(&mut query.origin),
            4 => query.binding.expires_at -= 1,
            _ => flip(&mut query.signature),
        },
        Tamper::Grant(i, field) => {
            let Some(g) = relayed.grants.get_mut(i) else {
                return false;
            };
            match field % 8 {
                0 => flip(&mut g.request_id.0),
                1 => flip(&mut g.origin_query_digest),
                2 => flip(&mut g.relay_id),
                3 => flip(&mut g.child_id),
                4 => g.expires_at -= 1,
                5 => match &mut g.parent_grant_digest {
                    Some(parent) => flip(parent),
                    None => g.parent_grant_digest = Some([0; 32]),
                },
                6 => flip(g.path.last_mut().unwrap()),
                _ => flip(&mut g.signature),
            }
        }
        Tamper::Response(field) => {
            let r = &mut relayed.response;
            match field % 8 {
                0 => r.scope.push(b'x'),
                1 => flip(&mut r.policy_digest),
                2 => {
                    r.completeness = match r.completeness {
                        Completeness::Complete => Completeness::Partial,
                        Completeness::Partial | Completeness::Unstated => Completeness::Unstated,
                    };
                }
                3 => r.results.push(Vec::new()),
                4 => r.generated_at += 1,
                5 => flip(&mut r.responder_id),
                6 => flip(&mut r.request_id.0),
                _ => flip(&mut r.signature),
            }
        }
        Tamper::Receipt(i, field) => {
            let Some(r) = relayed.receipts.get_mut(i) else {
                return false;
            };
            match field % 6 {
                0 => flip(&mut r.request_id.0),
                1 => flip(&mut r.relay_id),
                2 => flip(&mut r.grant_digest),
                3 => flip(&mut r.response_digest),
                4 => r.forwarded_at += 1,
                _ => flip(&mut r.signature),
            }
        }
    }
    true
}

fn scenario() -> impl Strategy<Value = (u32, Vec<u8>, u8, Vec<u64>)> {
    (1..=HOP_CEILING).prop_flat_map(|hops| {
        let relays = prop::collection::vec(2u8..7, 0..hops as usize);
        (Just(hops), relays, 2u8..7).prop_flat_map(|(hops, relays, responder)| {
            let n = relays.len();
            (
                Just(hops),
                Just(relays),
                Just(responder),
                prop::collection::vec(501u64..=1000, n),
            )
        })
    })
}

fn monotone(mut expiries: Vec<u64>) -> Vec<u64> {
    expiries.sort_unstable_by(|a, b| b.cmp(a));
    expiries
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    #[test]
    fn every_well_formed_chain_and_response_verifies(
        (hops, relays, responder, expiries) in scenario(),
        nonce in any::<[u8; 32]>(),
    ) {
        let q = OriginQuery::sign(&key(1), nonce, binding(hops));
        let relayed = relayed(&q, &relays, responder, &monotone(expiries));
        let asked = relays.first().copied().unwrap_or(responder);
        let path = verify_chain(&q, &relayed.grants, NOW).unwrap();
        prop_assert_eq!(&path.relays, &relays.iter().map(|r| id(*r)).collect::<Vec<_>>());
        prop_assert_eq!(path.remaining_hops, hops - 1 - u32::try_from(relays.len()).unwrap());
        let verified = verify_response(&q, &id(asked), &relayed, NOW).unwrap();
        prop_assert_eq!(verified.responder, id(responder));
    }

    #[test]
    fn any_unsigned_change_is_refused(
        (hops, relays, responder, expiries) in scenario(),
        target in 0u8..4,
        index in 0usize..8,
        field in any::<u8>(),
    ) {
        let mut q = query(hops);
        let mut relayed = relayed(&q, &relays, responder, &monotone(expiries));
        let asked = id(relays.first().copied().unwrap_or(responder));
        let tamper = match target {
            0 => Tamper::Query(field),
            1 => Tamper::Grant(index, field),
            2 => Tamper::Response(field),
            _ => Tamper::Receipt(index, field),
        };
        prop_assume!(apply(tamper, &mut q, &mut relayed));
        prop_assert!(verify_response(&q, &asked, &relayed, NOW).is_err(), "{tamper:?}");
    }

    #[test]
    fn a_chain_past_the_hop_bound_is_refused(
        hops in 1..=HOP_CEILING,
        extra in 0usize..3,
    ) {
        let q = query(hops);
        let relays: Vec<u8> = (0..hops as usize + extra).map(|i| 2 + u8::try_from(i % 5).unwrap()).collect();
        let relayed = relayed(&q, &relays, 6, &vec![900; relays.len()]);
        let index = hops as usize - 1;
        prop_assert_eq!(
            verify_chain(&q, &relayed.grants, NOW),
            Err(ChainError::HopBoundExceeded { index })
        );
    }
}
