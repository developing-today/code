//! scheme-serde: welcome, update and re-add sizes for four builds of p2panda-encryption at rev
//! 3e84cac (stock, both byte fields, the inner field only, the outer field only). Also checks how
//! serde reads a byte string and an integer array, and runs a Value-level transcoder from the stock
//! layout to the outer-only layout. Method and results are in RESULTS.md.
#![allow(dead_code, unused_imports)]

use std::fmt::Debug;
use std::fs;
use std::path::Path;

use cbor_core::Value;
use p2panda_core::cbor::{decode_cbor, encode_cbor};
use serde::{Deserialize, Serialize};
use serde_bytes::{ByteBuf, Bytes};

mod stock {
    use p2panda_encryption as enc;

    type Dgm = enc::data_scheme::test_utils::dgm::TestDgm<
        enc::test_utils::MemberId,
        enc::test_utils::MessageId,
    >;

    include!("harness.rs");
}

mod both {
    use p2panda_encryption_both as enc;

    type Dgm = enc::data_scheme::test_utils::dgm::TestDgm<
        enc::test_utils::MemberId,
        enc::test_utils::MessageId,
    >;

    include!("harness.rs");
}

mod inner {
    use p2panda_encryption_inner as enc;

    type Dgm = enc::data_scheme::test_utils::dgm::TestDgm<
        enc::test_utils::MemberId,
        enc::test_utils::MessageId,
    >;

    include!("harness.rs");
}

mod outer {
    use p2panda_encryption_outer as enc;

    type Dgm = enc::data_scheme::test_utils::dgm::TestDgm<
        enc::test_utils::MemberId,
        enc::test_utils::MessageId,
    >;

    include!("harness.rs");
}

// Path of the X3DH ciphertext's byte field in the CBOR of a PreKey welcome. The welcome is the
// direct message: `content` -> `Welcome` -> `ciphertext` (TwoPartyMessage) -> `ciphertext`
// (TwoPartyCiphertext) -> `PreKey` (X3dhCiphertext) -> `ciphertext`.
const X3DH_PATH: [&str; 6] = [
    "content",
    "Welcome",
    "ciphertext",
    "ciphertext",
    "PreKey",
    "ciphertext",
];

fn navigate<'v, 'a>(mut value: &'v mut Value<'a>, path: &[&str]) -> &'v mut Value<'a> {
    for key in path {
        value = value.get_mut(*key).expect("a key on the path");
    }
    value
}

// Stock layout to outer-only layout: the integer array at the X3DH path becomes a byte string.
// Nothing else changes, so the inner layer (inside the AEAD) stays as stock.
fn to_wire(stock_bytes: &[u8]) -> Vec<u8> {
    let mut value = Value::decode_owned(stock_bytes).expect("decode the stock welcome");
    let slot = navigate(&mut value, &X3DH_PATH);
    let bytes: Vec<u8> = slot
        .as_array()
        .expect("an integer array")
        .iter()
        .map(|item| item.to_u8().expect("a byte"))
        .collect();
    *slot = Value::byte_string(bytes);
    value.encode()
}

// The reverse: the byte string at the X3DH path becomes an integer array, which the stock decoder
// reads.
fn from_wire(wire: &[u8]) -> Vec<u8> {
    let mut value = Value::decode_owned(wire).expect("decode the wire welcome");
    let slot = navigate(&mut value, &X3DH_PATH);
    let items: Vec<Value> = slot
        .as_bytes()
        .expect("a byte string")
        .iter()
        .map(|byte| Value::from(*byte))
        .collect();
    *slot = Value::array(items);
    value.encode()
}

// What serde reads from each encoding. `EitherForm` is an untagged enum that accepts both.
#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum EitherForm {
    Bytes(ByteBuf),
    Seq(Vec<u8>),
}

#[derive(Serialize)]
struct Envelope<'a> {
    v: u8,
    body: &'a Bytes,
}

fn read<T: for<'a> Deserialize<'a> + Debug>(bytes: &[u8]) -> String {
    match decode_cbor::<T, _>(bytes) {
        Ok(value) => format!("ok {value:?}"),
        Err(err) => format!("err: {err}"),
    }
}

fn serde_read_check() {
    let data: Vec<u8> = vec![1, 2, 3, 200];
    let as_array = encode_cbor(&data).expect("encode the array form");
    let as_bytes = encode_cbor(&Bytes::new(&data[..])).expect("encode the byte-string form");
    println!("## serde read check (one byte field, 4 bytes)");
    println!("Vec<u8> encodes to {:02x?} ({} B)", as_array, as_array.len());
    println!("serde_bytes encodes to {:02x?} ({} B)", as_bytes, as_bytes.len());
    println!("read Vec<u8> from the array form: {}", read::<Vec<u8>>(&as_array));
    println!("read Vec<u8> from the byte-string form: {}", read::<Vec<u8>>(&as_bytes));
    println!("read ByteBuf from the array form: {}", read::<ByteBuf>(&as_array));
    println!("read ByteBuf from the byte-string form: {}", read::<ByteBuf>(&as_bytes));
    println!("read EitherForm from the array form: {}", read::<EitherForm>(&as_array));
    println!("read EitherForm from the byte-string form: {}", read::<EitherForm>(&as_bytes));
    println!();
}

// Bytes added around a body of `len` bytes by a version tag: an array of (version, body) and a map
// of {v, body}. The body header is counted, so the figure depends on the body length.
fn envelope_overhead(len: usize) -> (usize, usize) {
    let body = vec![0u8; len];
    let body = Bytes::new(&body[..]);
    let array = encode_cbor(&(1u8, body)).expect("encode the array envelope").len() - len;
    let map = encode_cbor(&Envelope { v: 1, body })
        .expect("encode the map envelope")
        .len()
        - len;
    (array, map)
}

fn cross_checks(seed: u8, out_dir: &Path, s: &stock::Dumps, b: &both::Dumps, i: &inner::Dumps, o: &outer::Dumps) {
    println!("## cross-decode, seed {seed} (decode_cbor of the whole direct message; the inner layer is inside the AEAD and is not read)");
    println!("stock K=1 welcome read by both: {:?}", both::decode_welcome(&s.k1_welcome));
    println!("both K=1 welcome read by stock: {:?}", stock::decode_welcome(&b.k1_welcome));
    println!("stock K=1 welcome read by inner: {:?}", inner::decode_welcome(&s.k1_welcome));
    println!("stock K=1 welcome read by outer: {:?}", outer::decode_welcome(&s.k1_welcome));
    println!("inner K=1 welcome read by stock: {:?}", stock::decode_welcome(&i.k1_welcome));
    println!("outer K=1 welcome read by stock: {:?}", stock::decode_welcome(&o.k1_welcome));
    println!("stock re-add welcome read by both: {:?}", both::decode_welcome(&s.k1000_readd));
    println!("both re-add welcome read by stock: {:?}", stock::decode_welcome(&b.k1000_readd));
    println!("stock K=1000 bytes are canonical (decode then encode is identity): {}", Value::decode_owned(&s.k1000_welcome).map(|v| v.encode() == s.k1000_welcome).unwrap_or(false));
    println!("stock K=1 welcome: stock build re-encodes it byte-exact: {}", stock::reencode_welcome(&s.k1_welcome).map(|v| v == s.k1_welcome).unwrap_or(false));
    println!("stock K=1 welcome: Value decode then encode is identity: {}", Value::decode_owned(&s.k1_welcome).map(|v| v.encode() == s.k1_welcome).unwrap_or(false));
    println!("outer K=1 welcome (outer layout, not transcoded): outer build re-encodes it byte-exact: {}", outer::reencode_welcome(&o.k1_welcome).map(|v| v == o.k1_welcome).unwrap_or(false));
    println!("both K=1 welcome: both build re-encodes it byte-exact: {}", both::reencode_welcome(&b.k1_welcome).map(|v| v == b.k1_welcome).unwrap_or(false));

    println!("## transcoder: stock PreKey welcome to the outer-only layout (seed {seed})");
    for (label, bytes) in [("K=1", &s.k1_welcome), ("K=1000", &s.k1000_welcome)] {
        let wire = to_wire(bytes);
        let back = from_wire(&wire);
        println!("{label}: stock {} B -> wire {} B", bytes.len(), wire.len());
        println!("{label}: from_wire(to_wire(x)) == x: {}", back == *bytes);
        println!("{label}: stock reads the round trip: {:?}", stock::decode_welcome(&back));
        println!("{label}: outer build reads the wire form: {:?}", outer::decode_welcome(&wire));
        let exact = outer::reencode_welcome(&wire).map(|v| v == wire).unwrap_or(false);
        println!("{label}: outer build re-encodes the wire form byte-exact: {exact}");
        if label == "K=1000" {
            fs::write(out_dir.join(format!("transcoded-outer-s{seed}-welcome-k1000.bin")), &wire).expect("write transcoded dump");
            let (arr, map) = envelope_overhead(wire.len());
            println!("{label}: version tag overhead on the wire form: array (version, body) +{arr} B, map {{v, body}} +{map} B");
        }
    }
    let (arr, map) = envelope_overhead(b.k1000_welcome.len());
    println!("both K=1000 welcome {} B: version tag overhead: array +{arr} B, map +{map} B", b.k1000_welcome.len());
    println!();
}

fn main() {
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("out");
    fs::create_dir_all(&out_dir).expect("create out/");
    serde_read_check();
    for seed in 1..=3u8 {
        let s = stock::run("stock", seed, &out_dir);
        let b = both::run("both", seed, &out_dir);
        let i = inner::run("inner", seed, &out_dir);
        let o = outer::run("outer", seed, &out_dir);
        cross_checks(seed, &out_dir, &s, &b, &i, &o);
    }
}
