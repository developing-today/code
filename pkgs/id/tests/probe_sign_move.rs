// Probe (creator-b, this run): does sign_move refuse the values the
// acceptance side can never accept? WorldCore::new refuses an empty /
// whitespace-only world ID (core.rs), and prepare_input_signed refuses
// empty input BEFORE MoveChain::verify (core.rs) -- so a move signed
// for a blank world or with an empty input is dead on arrival.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use id::world::signed::{MoveChain, sign_move};
use id::world::{WorldCore, WorldLimits};
use p2panda_core::SigningKey;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

#[test]
fn probe_sign_move_domain() {
    let key = key(7);
    let author = *key.verifying_key().as_bytes();

    // Acceptance-side oracles (the domain sign_move must match):
    assert!(WorldCore::new("", WorldLimits::default()).is_err());
    assert!(WorldCore::new("   ", WorldLimits::default()).is_err());
    assert!(WorldCore::new("ttt", WorldLimits::default()).is_ok());

    // Control: a normal move signs and verifies.
    let good = sign_move(&key, "ttt", 0, None, b"cell-0").unwrap();
    assert!(MoveChain::new()
        .verify(&good.bytes, "ttt", b"cell-0", &author)
        .is_ok());

    // Case A: blank world ids sign Ok and even verify against the same
    // id -- yet no WorldCore can ever bear one.
    for bad_world in ["", " ", "   ", "\t"] {
        let ev = sign_move(&key, bad_world, 0, None, b"cell-0");
        println!("sign_move world_id={bad_world:?}: ok={}", ev.is_ok());
        let ev = ev.unwrap();
        let v = MoveChain::new().verify(&ev.bytes, bad_world, b"cell-0", &author);
        println!("  MoveChain::verify(same world_id): ok={}", v.is_ok());
        assert!(v.is_ok());
    }

    // Case B: an empty input signs Ok, but MoveChain::verify refuses
    // the result -- the header carries no payload hash for an empty
    // body, so the move fails verify's commitment check ("does not
    // commit to the submitted input bytes"). And the production path
    // refuses empty input even earlier (core.rs:1938, before verify).
    let ev = sign_move(&key, "ttt", 0, None, b"").unwrap();
    println!("sign_move empty input: payload_hash={:?}", ev.header.payload_hash);
    let v = MoveChain::new().verify(&ev.bytes, "ttt", b"", &author);
    println!("sign_move empty input: ok=true, verify result: {v:?}");
    assert!(v.is_err());
}
