//! Golden vectors, the p2panda-core header vector, and the forgeries for the signature checks.
//!
//! Every vector is built twice from the same logical values: once with cbor-core (the candidate)
//! and once with dcbor (the cross-check). The signing key is a fixed test seed (bytes 00..1f),
//! not a real key.

use std::collections::BTreeMap;

use cbor_core::Value;
use dcbor::{ByteString, CBOR};
use p2panda_core::{Header, SigningKey};

use crate::canonical;

/// Test-only signing key: the 32 bytes 00 through 1f.
pub fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| i as u8))
}

/// One golden vector: the same logical value encoded by cbor-core and by dcbor.
pub struct Vector {
    pub name: &'static str,
    pub cbor_core: Vec<u8>,
    pub dcbor: Vec<u8>,
}

impl Vector {
    pub fn agree(&self) -> bool {
        self.cbor_core == self.dcbor
    }
}

/// The five vectors. The header vector is built separately by `header_vector`.
pub fn vectors() -> Vec<Vector> {
    [
        ("plain body map", body_map()),
        ("body with byte strings", body_bytes()),
        ("array of 32-byte hashes", hash_array()),
        ("nested map", nested()),
    ]
    .into_iter()
    .map(|(name, (value, cbor))| Vector {
        name,
        cbor_core: canonical::encode(&value),
        dcbor: cbor.to_cbor_data(),
    })
    .collect()
}

/// `{"kind": "group.policy", "group": "g1", "level": 3}`
fn body_map() -> (Value<'static>, CBOR) {
    let value = Value::from(BTreeMap::from([
        (Value::from("kind"), Value::from("group.policy")),
        (Value::from("group"), Value::from("g1")),
        (Value::from("level"), Value::Unsigned(3)),
    ]));
    let mut map = dcbor::Map::new();
    map.insert("kind", "group.policy");
    map.insert("group", "g1");
    map.insert("level", 3u64);
    (value, CBOR::from(map))
}

/// `{"blob": h'00ff10', "empty": h'', "hash": h'<blake3("x")>'}`
fn body_bytes() -> (Value<'static>, CBOR) {
    let blob = vec![0x00, 0xff, 0x10];
    let hash = blake3::hash(b"x").as_bytes().to_vec();
    let value = Value::from(BTreeMap::from([
        (Value::from("blob"), Value::from(blob.clone())),
        (Value::from("empty"), Value::from(Vec::<u8>::new())),
        (Value::from("hash"), Value::from(hash.clone())),
    ]));
    let mut map = dcbor::Map::new();
    map.insert("blob", ByteString::from(blob));
    map.insert("empty", ByteString::from(Vec::<u8>::new()));
    map.insert("hash", ByteString::from(hash));
    (value, CBOR::from(map))
}

/// `[blake3("a"), blake3("b"), blake3("c")]`, the shape of an `after` frontier.
fn hash_array() -> (Value<'static>, CBOR) {
    let hashes: Vec<Vec<u8>> = [b"a", b"b", b"c"]
        .iter()
        .map(|s| blake3::hash(*s).as_bytes().to_vec())
        .collect();
    let value = Value::from(
        hashes
            .iter()
            .map(|h| Value::from(h.clone()))
            .collect::<Vec<_>>(),
    );
    let cbor = CBOR::from(
        hashes
            .into_iter()
            .map(|h| CBOR::from(ByteString::from(h)))
            .collect::<Vec<_>>(),
    );
    (value, cbor)
}

/// `{"a": null, "outer": {"inner": [1, -2, {"deep": true}], "tag": h'ab'}, "z": "last"}`
fn nested() -> (Value<'static>, CBOR) {
    let deep = Value::from(BTreeMap::from([(Value::from("deep"), Value::from(true))]));
    let inner = Value::from(vec![Value::Unsigned(1), Value::Negative(1), deep]);
    let outer = Value::from(BTreeMap::from([
        (Value::from("inner"), inner),
        (Value::from("tag"), Value::from(vec![0xabu8])),
    ]));
    let value = Value::from(BTreeMap::from([
        (Value::from("a"), Value::null()),
        (Value::from("outer"), outer),
        (Value::from("z"), Value::from("last")),
    ]));

    let mut deep = dcbor::Map::new();
    deep.insert("deep", true);
    let inner = CBOR::from(vec![CBOR::from(1u64), CBOR::from(-2i64), CBOR::from(deep)]);
    let mut outer = dcbor::Map::new();
    outer.insert("inner", inner);
    outer.insert("tag", ByteString::from(vec![0xabu8]));
    let mut root = dcbor::Map::new();
    root.insert("a", CBOR::null());
    root.insert("outer", outer);
    root.insert("z", "last");
    (value, CBOR::from(root))
}

/// The golden p2panda-core header: seq 0, the plain body map as its body, the fixed key.
pub struct HeaderVector {
    pub header: Header<()>,
    pub key: [u8; 32],
    pub body: Vec<u8>,
    /// The full encoded header, signature included.
    pub bytes: Vec<u8>,
    /// The bytes the signature covers: the header array with the signature field removed.
    pub signed: Vec<u8>,
    /// BLAKE3 over `bytes`: the operation ID.
    pub operation_id: [u8; 32],
    pub signature: [u8; 64],
    /// dcbor encodings of the same logical header and of the signed part.
    pub dcbor_header: Vec<u8>,
    pub dcbor_signed: Vec<u8>,
}

pub fn header_vector() -> HeaderVector {
    let body = canonical::encode(&body_map().0);
    let key = signing_key();
    let header = Header::<()>::builder()
        .body(&body)
        .seq_num(0)
        .build(&key, ());
    let bytes = header.encode();
    let hash = header
        .payload_hash
        .expect("body is non-empty")
        .as_bytes()
        .to_vec();
    let size = body.len() as u64;
    let pk = *key.verifying_key().as_bytes();
    let signature = header.signature.to_bytes();

    let signed = signed_bytes(&bytes);
    let dcbor_header = dcbor_array(&[
        CBOR::from(1u64),
        ByteString::from(pk.to_vec()).into(),
        ByteString::from(signature.to_vec()).into(),
        CBOR::from(size),
        ByteString::from(hash.clone()).into(),
        CBOR::from(0u64),
    ]);
    let dcbor_signed = dcbor_array(&[
        CBOR::from(1u64),
        ByteString::from(pk.to_vec()).into(),
        CBOR::from(size),
        ByteString::from(hash).into(),
        CBOR::from(0u64),
    ]);
    HeaderVector {
        operation_id: *blake3::hash(&bytes).as_bytes(),
        header,
        key: pk,
        body,
        bytes,
        signed,
        signature,
        dcbor_header,
        dcbor_signed,
    }
}

fn dcbor_array(items: &[CBOR]) -> Vec<u8> {
    CBOR::from(items.to_vec()).to_cbor_data()
}

/// The header array without its signature field (index 2), re-encoded canonically.
pub fn signed_bytes(header_bytes: &[u8]) -> Vec<u8> {
    let mut items = canonical::decode(header_bytes)
        .expect("header bytes are canonical")
        .into_array()
        .expect("a header is an array");
    items.remove(2);
    canonical::encode(&Value::from(items))
}

/// A header with seq 0 written as `18 00`, signed over those non-canonical bytes. The signature
/// is valid, so only the decoder can refuse it.
pub fn non_canonical_header() -> Vec<u8> {
    let golden = header_vector();
    let key = signing_key();
    let pk = golden.key.to_vec();
    let size = golden.body.len() as u64;
    let hash = golden
        .header
        .payload_hash
        .expect("body")
        .as_bytes()
        .to_vec();
    let item = |v: Value<'static>| v.encode();

    // Array of five: version, key, size, hash, then seq as 18 00 instead of 00.
    let mut unsigned = vec![0x85];
    unsigned.extend(item(Value::Unsigned(1)));
    unsigned.extend(item(Value::from(pk.clone())));
    unsigned.extend(item(Value::Unsigned(size)));
    unsigned.extend(item(Value::from(hash.clone())));
    unsigned.extend([0x18, 0x00]);
    let signature = key.sign(&unsigned).to_bytes();

    // Array of six, with the signature in place.
    let mut full = vec![0x86];
    full.extend(item(Value::Unsigned(1)));
    full.extend(item(Value::from(pk)));
    full.extend(item(Value::from(signature.to_vec())));
    full.extend(item(Value::Unsigned(size)));
    full.extend(item(Value::from(hash)));
    full.extend([0x18, 0x00]);
    full
}

/// The group order L = 2^252 + 27742317777372353535851937790883648493, little-endian.
pub const GROUP_ORDER_LE: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

/// Little-endian addition of two 256-bit values that does not overflow.
pub fn add_le(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for i in 0..32 {
        let sum = a[i] as u16 + b[i] as u16 + carry;
        out[i] = sum as u8;
        carry = sum >> 8;
    }
    assert_eq!(carry, 0, "sum overflowed 256 bits");
    out
}

/// A key and signature to test against the golden signed bytes.
pub struct Forgery {
    pub name: &'static str,
    pub key: [u8; 32],
    pub signature: [u8; 64],
}

/// The golden pair, S at or above L, and two small-order keys.
pub fn forgeries(golden: &HeaderVector) -> Vec<Forgery> {
    let r: [u8; 32] = golden.signature[..32].try_into().expect("R is 32 bytes");
    let s: [u8; 32] = golden.signature[32..].try_into().expect("S is 32 bytes");
    let with_s = |s: [u8; 32]| {
        let mut sig = [0u8; 64];
        sig[..32].copy_from_slice(&r);
        sig[32..].copy_from_slice(&s);
        sig
    };
    let mut one = [0u8; 32];
    one[0] = 1;
    let identity_point = one;
    let mut order_two = [0xffu8; 32];
    order_two[0] = 0xec;
    order_two[31] = 0x7f;
    // R = identity and S = 0. With A = identity the check reduces to R = identity, which holds
    // for any message, so only the small-order rule can refuse it.
    let identity_sig = {
        let mut sig = [0u8; 64];
        sig[..32].copy_from_slice(&one);
        sig
    };
    vec![
        Forgery {
            name: "golden signature",
            key: golden.key,
            signature: golden.signature,
        },
        Forgery {
            name: "S + L (non-canonical)",
            key: golden.key,
            signature: with_s(add_le(&s, &GROUP_ORDER_LE)),
        },
        Forgery {
            name: "S = L",
            key: golden.key,
            signature: with_s(GROUP_ORDER_LE),
        },
        Forgery {
            name: "S = L + 1",
            key: golden.key,
            signature: with_s(add_le(&GROUP_ORDER_LE, &one)),
        },
        Forgery {
            name: "S = 2^256 - 1",
            key: golden.key,
            signature: with_s([0xff; 32]),
        },
        Forgery {
            name: "key = identity (small order)",
            key: identity_point,
            signature: identity_sig,
        },
        Forgery {
            name: "key = order-2 point (y = -1)",
            key: order_two,
            signature: identity_sig,
        },
    ]
}
