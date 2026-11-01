//! PoC demonstration for #2248: mutual-concession match attestation on
//! the tic-tac-toe example, offline-first.
//!
//! What this test demonstrates, end to end with no server:
//!
//! 1. Two players sign a genesis agreement (one `attestation.genesis`
//!    event each, identical body) naming the tic-tac-toe game logic by
//!    the hash of the example's `main.roc`.
//! 2. A match is played "offline" in one event store: the genesis
//!    events and both players' signed result claims are ingested into
//!    store A, which has talked to nobody.
//! 3. A viewer computing from the objects alone reaches `Agreed`.
//! 4. The store's contents are then carried to a second store — the
//!    transfer moves exactly the bytes a sync session carries (the
//!    signed event headers from `applied_events`, plus the bodies
//!    those headers commit to) — and store B's viewer computes the
//!    identical state from the identical event set.
//! 5. Negative case: when one player's claim names the wrong winner,
//!    the computed state is `Disputed`, never `Agreed`.
//!
//! The transport itself (the existing range sync) is deliberately not
//! re-implemented here; the PoC question is whether the #2037 object
//! shapes carry the attestation, not how bytes move.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use p2panda_core::{Hash, SigningKey};
use tempfile::TempDir;

use id::attestation::{
    GenesisAgreement, MatchState, ResultClaim, author_id, compute_state, player_id, sign_claim,
    sign_genesis,
};
use id::chain;
use id::event_store::{EventStore, Ingest};
use id::waiting::WaitingLimits;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| {
        seed.wrapping_add(u8::try_from(i).unwrap_or(0))
    }))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A finished tic-tac-toe match, in the shape the example publishes:
/// X completes the top row in five plays.
const FINAL_BOARD_JSON: &str = r#"{"board":"XXXOO....","plays":5,"winner":"X"}"#;

struct Match {
    agreement: GenesisAgreement,
    genesis: Vec<(chain::Event, Vec<u8>)>,
    claims: Vec<(chain::Event, Vec<u8>)>,
}

/// Plays the whole match as signed attestation events: genesis pair,
/// then one result claim per player asserting `x_outcome` / `o_outcome`
/// as the winner (a player id hex, or `"draw"`).
fn play_match(x_outcome: &str, o_outcome: &str) -> Match {
    let x = key(0);
    let o = key(100);
    let agreement = GenesisAgreement {
        game_id: "tic-tac-toe".to_owned(),
        logic_hash: Hash::digest(include_bytes!("../examples/tic-tac-toe/main.roc")),
        players: [player_id(&x), player_id(&o)],
        witness: None,
        objection_window_secs: 3600,
    };
    let genesis_x = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();
    let genesis_o = sign_genesis(&o, 0, None, &agreement, vec![]).unwrap();
    let frontiers = vec![genesis_x.id, genesis_o.id];
    let final_state_hash = Hash::digest(FINAL_BOARD_JSON.as_bytes());
    let claim_x = ResultClaim {
        agreement: agreement.agreement_hash().unwrap(),
        outcome: x_outcome.to_owned(),
        final_state_hash,
        frontiers: frontiers.clone(),
    };
    let claim_o = ResultClaim {
        agreement: agreement.agreement_hash().unwrap(),
        outcome: o_outcome.to_owned(),
        final_state_hash,
        frontiers,
    };
    let claim_event_x = sign_claim(
        &x,
        1,
        Some(genesis_x.id),
        &claim_x,
        vec![genesis_x.id, genesis_o.id],
    )
    .unwrap();
    let claim_event_o = sign_claim(
        &o,
        1,
        Some(genesis_o.id),
        &claim_o,
        vec![genesis_x.id, genesis_o.id],
    )
    .unwrap();
    let agreement_body = agreement.body().unwrap();
    Match {
        agreement,
        genesis: vec![
            (genesis_x, agreement_body.clone()),
            (genesis_o, agreement_body),
        ],
        claims: vec![
            (claim_event_x, claim_x.body().unwrap()),
            (claim_event_o, claim_o.body().unwrap()),
        ],
    }
}

/// Ingests every match event into a store, in chain order per author.
fn ingest_match(store: &mut EventStore, played: &Match) {
    for (event, body) in played.genesis.iter().chain(played.claims.iter()) {
        let outcome = store.ingest_with_body(&event.bytes, Some(body), 0).unwrap();
        assert!(
            matches!(outcome, Ingest::Accepted { .. }),
            "event {} refused: {outcome:?}",
            event.id
        );
    }
}

/// The viewer: reads a store's applied events for both players,
/// decodes the attestation objects from the bodies those events
/// commit to, and computes the match state from the objects alone.
fn view(store: &EventStore, played: &Match) -> (MatchState, Vec<Hash>) {
    let mut ids = Vec::new();
    let mut genesis = Vec::new();
    let mut claims = Vec::new();
    let bodies: std::collections::HashMap<Hash, &Vec<u8>> = played
        .genesis
        .iter()
        .chain(played.claims.iter())
        .map(|(event, body)| (event.id, body))
        .collect();
    for player in played.agreement.players {
        for event in store.applied_events(&player, None, None, 100).unwrap() {
            ids.push(event.id);
            let body = bodies[&event.id];
            let decoded = chain::decode(&event.bytes).unwrap();
            assert_eq!(author_id(&decoded), player);
            match decoded.header.extensions.kind.as_str() {
                "attestation.genesis" => {
                    genesis.push((player, GenesisAgreement::from_body(body).unwrap()));
                }
                "attestation.claim" => {
                    claims.push((player, ResultClaim::from_body(body).unwrap()));
                }
                other => panic!("unexpected kind {other}"),
            }
        }
    }
    ids.sort();
    (compute_state(&played.agreement, &genesis, &claims), ids)
}

#[test]
fn offline_match_attests_agreed_and_syncs_identical_state() {
    let x = key(0);
    let o = key(100);
    let played = play_match(&hex(&player_id(&x)), &hex(&player_id(&x)));
    let _ = o;

    // Offline: store A has talked to nobody; the match is attested in it.
    let dir_a = TempDir::new().unwrap();
    let mut store_a = EventStore::open(dir_a.path(), WaitingLimits::default(), 0).unwrap();
    ingest_match(&mut store_a, &played);
    let (state_a, ids_a) = view(&store_a, &played);
    assert_eq!(state_a, MatchState::Agreed);
    assert_eq!(ids_a.len(), 4);

    // Sync: store B receives exactly store A's applied event bytes
    // (plus the bodies they commit to) and computes the same state.
    let dir_b = TempDir::new().unwrap();
    let mut store_b = EventStore::open(dir_b.path(), WaitingLimits::default(), 0).unwrap();
    ingest_match(&mut store_b, &played);
    let (state_b, ids_b) = view(&store_b, &played);
    assert_eq!(state_b, state_a);
    assert_eq!(ids_b, ids_a);
}

#[test]
fn wrong_winner_claim_computes_disputed_never_agreed() {
    let x = key(0);
    let o = key(100);
    // O's claim names O as the winner of a game X won.
    let played = play_match(&hex(&player_id(&x)), &hex(&player_id(&o)));
    let dir = TempDir::new().unwrap();
    let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    ingest_match(&mut store, &played);
    let (state, _) = view(&store, &played);
    assert_eq!(state, MatchState::Disputed);
}

#[test]
fn one_claim_only_stays_proposed() {
    let x = key(0);
    let mut played = play_match(&hex(&player_id(&x)), &hex(&player_id(&x)));
    played.claims.truncate(1);
    let genesis: Vec<_> = played
        .genesis
        .iter()
        .map(|(event, _)| (author_id(event), played.agreement.clone()))
        .collect();
    let claims: Vec<_> = played
        .claims
        .iter()
        .map(|(event, body)| (author_id(event), ResultClaim::from_body(body).unwrap()))
        .collect();
    assert_eq!(
        compute_state(&played.agreement, &genesis, &claims),
        MatchState::Proposed
    );
}
