//! S5 assertions. The expected values are the ones recorded by `cargo run -p s5-cbor` (see
//! RESULTS.md), so a change in a decoder, in dcbor, or in the golden encoding fails here.

use cbor_core::Value;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use p2panda_core::Header;
use s5_cbor::{canonical, fuzz, golden, matrix};

/// [cbor-core STRICT, dcbor, re-encode, wrapper] accept (true) or reject (false).
fn verdicts(out: &matrix::Outcome) -> [bool; 4] {
    [
        out.strict.is_ok(),
        out.dcbor.is_ok(),
        out.reencode.is_ok(),
        out.wrapper.is_ok(),
    ]
}

#[test]
fn rejection_matrix_matches_the_recorded_run() {
    // In the order of `matrix::rows()`.
    let expected = [
        [false, false, false, false], // map keys length-first, mixed types
        [true, true, true, true],     // control: text keys, length-first = bytewise
        [false, false, false, false], // map keys out of order
        [false, false, false, false], // integer 5, 1-byte argument
        [false, false, false, false], // integer 255, 2-byte argument
        [false, false, false, false], // integer 5, 8-byte argument
        [false, false, false, false], // integer -1, 1-byte argument
        [false, false, false, false], // text length 1, 1-byte length
        [false, false, false, false], // float 1.0 as f64
        [false, false, false, false], // float 1.0 as f32
        [true, false, true, false],   // float 1.0 as f16
        [false, false, false, false], // float 1.5 as f32
        [true, true, true, false],    // float 1.5 as f16
        [true, true, true, false],    // float 0.1 as f64
        [true, true, true, false],    // NaN as f16 0x7e00
        [true, false, true, false],   // NaN as f16, payload 1
        [true, false, true, false],   // -0.0 as f16
        [true, false, true, false],   // text e + U+0301 (not NFC)
        [true, true, true, true],     // text U+00E9 (NFC)
        [false, false, false, false], // duplicate key
        [false, false, false, false], // indefinite byte string
        [false, false, false, false], // indefinite text string
        [false, false, false, false], // indefinite array
        [false, false, false, false], // indefinite map
        [true, false, true, false],   // simple value 23 (undefined)
        [true, false, true, false],   // simple value 16
        [true, true, true, true],     // simple value false
        [true, true, true, false],    // tag 1000
        [false, true, false, false],  // bignum tag 2 with a leading zero
        [false, false, false, false], // validly signed header with seq as 18 00
    ];
    let rows = matrix::rows();
    assert_eq!(rows.len(), expected.len());
    for (row, want) in rows.iter().zip(expected) {
        assert_eq!(
            verdicts(&matrix::evaluate(&row.input)),
            want,
            "{}",
            row.name
        );
    }
}

#[test]
fn golden_body_vectors_match_the_recorded_bytes_and_dcbor() {
    let recorded = [
        "a3646b696e646c67726f75702e706f6c6963796567726f7570626731656c6576656c03",
        "a364626c6f624300ff10646861736858203ae7d805f6789a6402acb70ad4096a85a56bf6804eaf25c0493ac697548d30b565656d70747940",
        "83582017762fddd969a453925d65717ac3eea21320b66b54342fde15128d6caf21215f582010e5cf3d3c8a4f9f3468c8cc58eea84892a22fdadbc1acb22410190044c1d5535820ea7aa1fc9efdbe106dbb70369a75e9671fa29d52bd55536711bf197477b8f021",
        "a36161f6617a646c617374656f75746572a26374616741ab65696e6e6572830121a16464656570f5",
    ];
    let vectors = golden::vectors();
    assert_eq!(vectors.len(), recorded.len());
    for (vector, want) in vectors.iter().zip(recorded) {
        assert_eq!(hex::encode(&vector.cbor_core), want, "{}", vector.name);
        assert!(vector.agree(), "dcbor disagrees on {}", vector.name);
        let back = matrix::dcbor_roundtrip(&vector.cbor_core);
        assert_eq!(
            back.as_deref(),
            Ok(vector.cbor_core.as_slice()),
            "{}",
            vector.name
        );
    }
}

#[test]
fn golden_header_vector_matches_the_recorded_bytes() {
    let h = golden::header_vector();
    assert_eq!(
        hex::encode(&h.bytes),
        "8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b85840be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d00",
    );
    assert_eq!(
        hex::encode(&h.signed),
        "8501582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d00",
    );
    assert_eq!(
        hex::encode(h.operation_id),
        "90acd5f7c498ccb93dc33ce41f49d166140bef38c1f85cb95940f311b6e7f212",
    );
    assert_eq!(
        hex::encode(h.signature),
        "be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f",
    );
    assert_eq!(h.dcbor_header, h.bytes, "dcbor header");
    assert_eq!(h.dcbor_signed, h.signed, "dcbor signed part");
    assert_eq!(
        matrix::dcbor_roundtrip(&h.bytes).as_deref(),
        Ok(h.bytes.as_slice())
    );
    assert!(Header::<()>::decode(&h.bytes).is_ok());
    let key = canonical::admit_key(&h.key).expect("golden key is admitted");
    assert!(canonical::verify(&key, &h.signed, &h.signature).is_ok());
}

#[test]
fn validly_signed_non_canonical_header_is_refused() {
    let bytes = golden::non_canonical_header();
    let err = Header::<()>::decode(&bytes).expect_err("p2panda must refuse 18 00");
    assert!(format!("{err:?}").contains("NonDeterministic"), "{err:?}");
}

#[test]
fn keys_are_admitted_only_when_they_are_not_of_small_order() {
    let h = golden::header_vector();
    assert!(canonical::admit_key(&h.key).is_some());
    let forgeries = golden::forgeries(&h);
    let find = |name: &str| forgeries.iter().find(|f| f.name == name).expect(name);
    assert!(canonical::admit_key(&find("key = identity (small order)").key).is_none());
    assert!(canonical::admit_key(&find("key = order-2 point (y = -1)").key).is_none());
}

#[test]
fn verify_strict_refuses_non_canonical_s_and_small_order_keys() {
    let h = golden::header_vector();
    let forgeries = golden::forgeries(&h);
    let find = |name: &str| forgeries.iter().find(|f| f.name == name).expect(name);
    let golden_key = VerifyingKey::from_bytes(&h.key).expect("golden key");

    for name in [
        "S + L (non-canonical)",
        "S = L",
        "S = L + 1",
        "S = 2^256 - 1",
    ] {
        let f = find(name);
        assert!(
            canonical::verify(&golden_key, &h.signed, &f.signature).is_err(),
            "{name}"
        );
        let forged = canonical::encode(&Value::from(vec![
            Value::Unsigned(1),
            Value::from(f.key.to_vec()),
            Value::from(f.signature.to_vec()),
            Value::Unsigned(h.body.len() as u64),
            Value::from(h.header.payload_hash.expect("body").as_bytes().to_vec()),
            Value::Unsigned(0),
        ]));
        assert!(
            Header::<()>::decode(&forged).is_err(),
            "p2panda accepted {name}"
        );
    }

    // The identity key with R = identity and S = 0 passes the plain equation for any message.
    // Only verify_strict's small-order rule refuses it.
    let identity = find("key = identity (small order)");
    let key = VerifyingKey::from_bytes(&identity.key).expect("on curve");
    let sig = Signature::from_bytes(&identity.signature);
    assert!(
        key.verify(&h.signed, &sig).is_ok(),
        "plain verify accepts it"
    );
    assert!(canonical::verify(&key, &h.signed, &identity.signature).is_err());

    let order_two = find("key = order-2 point (y = -1)");
    let key = VerifyingKey::from_bytes(&order_two.key).expect("on curve");
    assert!(canonical::verify(&key, &h.signed, &order_two.signature).is_err());
}

#[test]
fn wrapper_refuses_non_nfc_text_in_map_keys_too() {
    let decomposed =
        std::collections::BTreeMap::from([(Value::from("e\u{301}"), Value::Unsigned(1))]);
    let refused = canonical::decode(&Value::from(decomposed).encode());
    assert!(
        matches!(refused, Err(canonical::Error::NotNfc)),
        "{refused:?}"
    );
    let precomposed =
        std::collections::BTreeMap::from([(Value::from("\u{e9}"), Value::Unsigned(1))]);
    assert!(canonical::decode(&Value::from(precomposed).encode()).is_ok());
}

#[test]
fn bytewise_and_length_first_orders_differ_only_across_major_types() {
    fn sorted_both_ways(mut keys: Vec<Vec<u8>>) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let mut bytewise = keys.clone();
        bytewise.sort();
        keys.sort_by_key(|key| (key.len(), key.clone()));
        (bytewise, keys)
    }
    let lengths = [0usize, 1, 2, 22, 23, 24, 25, 255, 256, 300];
    let texts: Vec<Vec<u8>> = lengths
        .iter()
        .map(|&n| Value::from("x".repeat(n)).encode())
        .collect();
    let bytes: Vec<Vec<u8>> = lengths
        .iter()
        .map(|&n| Value::from(vec![7u8; n]).encode())
        .collect();
    let uints: Vec<Vec<u8>> = [
        0u64,
        1,
        23,
        24,
        255,
        256,
        65_535,
        65_536,
        u64::from(u32::MAX) + 1,
    ]
    .iter()
    .map(|&n| Value::Unsigned(n).encode())
    .collect();
    let negs: Vec<Vec<u8>> = [0u64, 1, 23, 24, 255, 256, 65_535, 65_536]
        .iter()
        .map(|&n| Value::Negative(n).encode())
        .collect();
    for group in [texts, bytes, uints, negs] {
        let (bytewise, length_first) = sorted_both_ways(group);
        assert_eq!(
            bytewise, length_first,
            "one major type: the two orders should agree"
        );
    }

    // Mixed types disagree: text "a" is shorter but its first byte is larger than 1000's.
    let short_text = Value::from("a").encode();
    let big_int = Value::Unsigned(1000).encode();
    assert!(big_int < short_text, "bytewise: 1000 first");
    assert!(
        short_text.len() < big_int.len(),
        "length-first: \"a\" first"
    );
}

#[test]
fn decode_then_reencode_holds_over_10k_values_and_160k_mutants() {
    let s = fuzz::run(20_261_011, 10_000, 16);
    assert_eq!(s.values, 10_000);
    assert_eq!(s.mutants, 160_000);
    assert_eq!(
        s.reencode_mismatch, 0,
        "STRICT accepted bytes that re-encode differently"
    );
    assert_eq!(
        s.strict_vs_reencode_disagree, 0,
        "STRICT and re-encode disagree"
    );
    assert_eq!(s.dcbor_reencode_differs, 0);
    assert_eq!(
        s.wrapper_accepted_dcbor_refused, 0,
        "dcbor refuses a wrapper-accepted value"
    );
    assert_eq!(s.panics, 0);
    assert_eq!(s.baseline_strict_refused, s.baseline_refused_bignum);
    assert_eq!(
        (s.baseline_strict_ok, s.baseline_strict_refused),
        (9_991, 9)
    );
    assert_eq!(s.strict_accepted, 30_365);
    assert_eq!(s.wrapper_accepted, 21_003);
    assert_eq!(s.dcbor_only, 31);
    assert_eq!(s.dcbor_only_bignum, 31);
}
