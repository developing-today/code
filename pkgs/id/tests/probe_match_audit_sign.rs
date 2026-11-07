// Probe (creator-b): do sign_header / sign_footer refuse shapes that
// MatchLogV0::verify_integrity always rejects? (version, roster, signer
// membership, seed commitment) If signing succeeds, the signature is
// dead on arrival.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use p2panda_core::{Hash, SigningKey};
use id::match_audit::{
    FORMAT_VERSION_V0, FooterV0, HeaderV0, MatchLogV0, MetadataV0, PlayerV0, sign_footer,
    sign_header,
};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}
fn player(k: &SigningKey, role: &str) -> PlayerV0 {
    PlayerV0 { id: *k.verifying_key().as_bytes(), role: role.to_owned() }
}
fn header(players: Vec<PlayerV0>) -> HeaderV0 {
    HeaderV0 {
        version: FORMAT_VERSION_V0,
        match_id: "probe-1".to_owned(),
        world_id: "probe-world".to_owned(),
        program_id: "tic-tac-toe".to_owned(),
        program_hash: Hash::digest(b"program"),
        rules_hash: Hash::digest(b"rules"),
        tick_rate: 1,
        rng_algorithm: "none/v1".to_owned(),
        seed_commitment: Hash::digest(b"seed"),
        initial_state_hash: Hash::digest(b"empty"),
        players,
        start_tick: 0,
    }
}
fn log_for(header: HeaderV0, signers: &[SigningKey], seed_reveal: Vec<u8>) -> MatchLogV0 {
    let header_signatures = signers.iter().map(|k| sign_header(k, &header).unwrap()).collect();
    let metadata = MetadataV0::default();
    let mut footer = FooterV0 {
        final_tick: 0,
        final_state_hash: Hash::digest(b"final"),
        result_claim: vec![],
        seed_reveal,
        signatures: vec![],
    };
    // footer_message binds header hash + a chain hash; use header_hash as stand-in chain hash
    let chain = id::match_audit::header_hash(&header).unwrap();
    footer.signatures = signers.iter().map(|k| sign_footer(k, &header, chain, &metadata, &footer).unwrap()).collect();
    MatchLogV0 { header, header_signatures, entries: vec![], metadata, footer }
}

#[test]
fn probe_sign_dead_on_arrival() {
    let x = key(1);
    let o = key(2);
    let stranger = key(9);
    let good_players = vec![player(&x, "x"), player(&o, "o")];

    // Control: a well-formed log verifies (empty entries, final_tick == start_tick).
    let good = log_for(header(good_players.clone()), &[x.clone(), o.clone()], b"seed".to_vec());
    println!("control verify_integrity: {:?}", good.verify_integrity().is_ok());
    assert!(good.verify_integrity().is_ok());

    // 1. Bad version: sign_header Ok, verify rejects (UnsupportedVersion is first check).
    let mut h = header(good_players.clone());
    h.version = 99;
    println!("sign_header version=99: {:?}", sign_header(&x, &h).is_ok());
    assert!(sign_header(&x, &h).is_ok());
    let bad = log_for(h, &[x.clone(), o.clone()], b"seed".to_vec());
    println!("verify version=99 log: {:?}", bad.verify_integrity().unwrap_err().to_string());
    assert!(bad.verify_integrity().is_err());

    // 2. Empty roster: sign_header Ok, verify rejects (Roster).
    let h = header(vec![]);
    println!("sign_header empty roster: {:?}", sign_header(&x, &h).is_ok());
    assert!(sign_header(&x, &h).is_ok());
    let bad = log_for(h, &[x.clone()], b"seed".to_vec());
    println!("verify empty-roster log: {:?}", bad.verify_integrity().unwrap_err().to_string());
    assert!(bad.verify_integrity().is_err());

    // 3. Duplicate roster: sign_header Ok, verify rejects (Roster).
    let h = header(vec![player(&x, "x"), player(&x, "o")]);
    println!("sign_header duplicate roster: {:?}", sign_header(&x, &h).is_ok());
    assert!(sign_header(&x, &h).is_ok());
    let bad = log_for(h, &[x.clone(), x.clone()], b"seed".to_vec());
    println!("verify duplicate-roster log: {:?}", bad.verify_integrity().unwrap_err().to_string());
    assert!(bad.verify_integrity().is_err());

    // 4. Non-roster signer: sign_header Ok for stranger, verify rejects (signature set).
    let h = header(good_players.clone());
    println!("sign_header stranger signer: {:?}", sign_header(&stranger, &h).is_ok());
    assert!(sign_header(&stranger, &h).is_ok());
    let bad = log_for(h, &[x.clone(), stranger.clone()], b"seed".to_vec());
    println!("verify stranger-header-sig log: {:?}", bad.verify_integrity().unwrap_err().to_string());
    assert!(bad.verify_integrity().is_err());

    // 5. sign_footer: stranger signer Ok; seed reveal mismatching the header
    //    commitment also signs Ok, verify rejects (SeedCommitmentMismatch,
    //    checked before footer signatures).
    let h = header(good_players.clone());
    let metadata = MetadataV0::default();
    let chain = id::match_audit::header_hash(&h).unwrap();
    let footer = FooterV0 {
        final_tick: 0,
        final_state_hash: Hash::digest(b"final"),
        result_claim: vec![],
        seed_reveal: b"wrong-seed".to_vec(),
        signatures: vec![],
    };
    println!("sign_footer stranger: {:?}", sign_footer(&stranger, &h, chain, &metadata, &footer).is_ok());
    assert!(sign_footer(&stranger, &h, chain, &metadata, &footer).is_ok());
    println!("sign_footer seed-mismatch (by roster player): {:?}", sign_footer(&x, &h, chain, &metadata, &footer).is_ok());
    assert!(sign_footer(&x, &h, chain, &metadata, &footer).is_ok());
    let bad = log_for(h, &[x.clone(), o.clone()], b"wrong-seed".to_vec());
    println!("verify seed-mismatch log: {:?}", bad.verify_integrity().unwrap_err().to_string());
    assert!(bad.verify_integrity().is_err());
}
