//! Canonical encodings. STRICT decoding accepts each encoding and re-encoding gives the same bytes.
//! Op IDs are BLAKE3 of these bytes, so a non-canonical encoding would give one op two IDs.

use std::collections::BTreeSet;

use cbor_core::{DecodeOptions, Strictness};
use s8_tags::{Device, Dot, Entry, Replica, Rng, encode_op, encode_state};

fn device(i: usize) -> Device {
    let mut d = [0u8; 32];
    d[0] = u8::try_from(i + 1).expect("few devices");
    d
}

fn strict_round_trip(bytes: &[u8]) -> bool {
    DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .decode_owned(bytes)
        .is_ok_and(|value| value.encode() == bytes)
}

/// Edge cases: a NUL in a name, an empty name, an empty value, a NUL subject byte, and no value.
fn edge_entry(rng: &mut Rng) -> Entry {
    let mut subject = [0u8; 32];
    subject[rng.below(32)] = u8::try_from(rng.below(3)).expect("small");
    Entry {
        subject,
        name: ["", "a\0b", "ab"][rng.below(3)].to_owned(),
        value: [None, Some(""), Some("x\0y")][rng.below(3)].map(str::to_owned),
    }
}

#[test]
fn every_op_and_state_encoding_is_strictly_canonical() {
    let mut ops_checked = 0;
    let mut states_checked = 0;
    for seed in 0..50 {
        let mut rng = Rng::new(seed);
        let mut reps: Vec<Replica> = (0..3).map(|i| Replica::new(device(i))).collect();
        for _ in 0..80 {
            let r = rng.below(3);
            let op = if rng.below(10) < 6 {
                reps[r].set(edge_entry(&mut rng))
            } else {
                let live: Vec<Dot> = reps[r].state.live.keys().copied().collect();
                let names: BTreeSet<Dot> = if live.is_empty() {
                    BTreeSet::new()
                } else {
                    (0..rng.below(3)).map(|_| live[rng.below(live.len())]).collect()
                };
                reps[r].clear(names)
            };
            assert!(strict_round_trip(&encode_op(&op)), "op not canonical, seed {seed}");
            ops_checked += 1;
            let other = rng.below(3);
            if other != r {
                let src = reps[r].clone();
                reps[other].merge(&src);
            }
            for rep in &reps {
                assert!(strict_round_trip(&encode_state(&rep.state)), "state not canonical, seed {seed}");
                states_checked += 1;
            }
        }
    }
    assert!(ops_checked > 0 && states_checked > 0);
    println!("checked {ops_checked} ops and {states_checked} states");
}
