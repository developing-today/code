//! Probe: world_fixture::decode duplicate JSON keys in header/event lines.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::world_fixture::decode;

#[test]
fn probe_fixture_dup_keys() {
    // Control: a minimal valid fixture decodes.
    let good = "{\"fixture\":1,\"world\":\"w\",\"module_hash\":null,\"current_sequence\":0,\"complete\":true,\"records\":null}\n";
    let fixture = decode(good).unwrap();
    assert_eq!(fixture.header.world_id, "w");

    // Case 1: duplicate header field - two different worlds, last silently wins.
    let dup_world = "{\"fixture\":1,\"world\":\"first\",\"world\":\"second\",\"module_hash\":null,\"current_sequence\":0,\"complete\":true,\"records\":null}\n";
    let res = decode(dup_world);
    println!("dup world ok? {:?} -> {:?}", res.is_ok(), res.as_ref().map(|f| f.header.world_id.clone()));
    assert!(res.is_ok(), "BUG-CHECK: duplicate header field accepted");

    // Case 2: duplicate version field - unsupported first version hidden by supported second.
    let dup_version = "{\"fixture\":99,\"fixture\":1,\"world\":\"w\",\"module_hash\":null,\"current_sequence\":0,\"complete\":true,\"records\":null}\n";
    let res = decode(dup_version);
    println!("dup version ok? {}", res.is_ok());
    assert!(res.is_ok(), "BUG-CHECK: duplicate version field accepted");

    // Case 3: duplicate event sequence - out-of-window first value hidden by in-window second.
    let dup_seq = "{\"fixture\":1,\"world\":\"w\",\"module_hash\":null,\"current_sequence\":1,\"complete\":true,\"records\":null}\n{\"sequence\":99,\"sequence\":1,\"participant_id\":3,\"kind\":\"chat\",\"text\":\"hi\"}\n";
    let res = decode(dup_seq);
    println!("dup seq ok? {}", res.is_ok());
    assert!(res.is_ok(), "BUG-CHECK: duplicate event field accepted");
}
