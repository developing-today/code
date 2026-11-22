//! Conformance of the Rust `id://` V1 codec with the stdlib reference
//! codec (slice A9117-uri-reference-pbt, issue #10280; design A-9117.1,
//! S4.12–S4.17).
//!
//! Oracle: **golden** — `part-a-addressing/uri_codec_reference.py
//! self-test` draws 10,000 array IDs and 10,000 content IDs from
//! `random.Random(0x9117_2026)`. This test replays the same draws with a
//! port of `CPython`'s Mersenne Twister and its `randrange`, `choice` and
//! `randbytes`, formats every value with the Rust formatter, and requires
//! the `SHA-256` of the whole corpus to equal the digest the reference pins
//! ([`CORPUS_SHA256`]). Every URI must also parse back to the value that
//! produced it. The reference's readable vectors, content golden and
//! malformed corpus are checked with their exact outcomes.
//!
//! `hosted-worker/test/id-uri-vectors.test.ts` replays the same corpus
//! against the TypeScript edition and pins the same digest.
//!
//! The crate-root export of `id_uri_parse`, `id_uri_format` and
//! `array_address` belongs to slice 9110-02, so until that lands this
//! test includes them by path; `id_shape` is already exported and is
//! re-exported under the name the modules use.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod id_shape {
    pub use id_core::id_shape::*;
}

#[path = "../src/array_address.rs"]
#[allow(dead_code)]
mod array_address;

#[path = "../src/id_uri_parse.rs"]
#[allow(dead_code)]
mod id_uri_parse;

#[path = "../src/id_uri_format.rs"]
#[allow(dead_code)]
mod id_uri_format;

use std::fmt::Write as _;

use array_address::ArrayAddress;
use id_shape::{ContentId, HashProfile};
use id_uri_format::{format_array, format_content, resolve_context};
use id_uri_parse::{UriTarget, parse};
use sha2::{Digest, Sha256};

/// SHA-256 of the reference's deterministic corpus: the 10,000 array
/// URIs then the 10,000 content URIs, each followed by `\n`. The
/// reference self-test asserts the same constant.
const CORPUS_SHA256: &str = "390eaa67a9f94e9034fb8ba77596b14be2765403606b52abfb6c1a4ff9af7fac";

const SEED: u32 = 0x9117_2026;

/// `CPython`'s `random.Random` for an integer seed below `2^32`: `MT19937`
/// seeded by `init_by_array([seed])`, with `getrandbits`, the
/// rejection-sampling `_randbelow`, `randrange`, `choice` and
/// `randbytes` exactly as `CPython` 3.9+ defines them.
struct PyRandom {
    state: [u32; 624],
    index: usize,
}

impl PyRandom {
    fn new(seed: u32) -> Self {
        let mut rng = Self {
            state: [0; 624],
            index: 624,
        };
        rng.init_genrand(19_650_218);
        let key = [seed];
        let (mut i, mut j) = (1_usize, 0_usize);
        for _ in 0..624.max(key.len()) {
            let prev = rng.state[i - 1] ^ (rng.state[i - 1] >> 30);
            rng.state[i] = (rng.state[i] ^ prev.wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(u32::try_from(j).unwrap());
            i += 1;
            j += 1;
            if i >= 624 {
                rng.state[0] = rng.state[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            let prev = rng.state[i - 1] ^ (rng.state[i - 1] >> 30);
            rng.state[i] = (rng.state[i] ^ prev.wrapping_mul(1_566_083_941))
                .wrapping_sub(u32::try_from(i).unwrap());
            i += 1;
            if i >= 624 {
                rng.state[0] = rng.state[623];
                i = 1;
            }
        }
        rng.state[0] = 0x8000_0000;
        rng
    }

    fn init_genrand(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..624 {
            let prev = self.state[i - 1] ^ (self.state[i - 1] >> 30);
            self.state[i] = 1_812_433_253_u32
                .wrapping_mul(prev)
                .wrapping_add(u32::try_from(i).unwrap());
        }
        self.index = 624;
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for k in 0..624 {
                let y = (self.state[k] & 0x8000_0000) | (self.state[(k + 1) % 624] & 0x7fff_ffff);
                let mag = if y & 1 == 0 { 0 } else { 0x9908_b0df };
                self.state[k] = self.state[(k + 397) % 624] ^ (y >> 1) ^ mag;
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `getrandbits(k)` for `k <= 32`; `getrandbits(0)` draws nothing.
    fn bits(&mut self, k: u32) -> u32 {
        if k == 0 {
            0
        } else {
            self.next_u32() >> (32 - k)
        }
    }

    fn below(&mut self, n: usize) -> usize {
        let n = u32::try_from(n).unwrap();
        let k = u32::BITS - n.leading_zeros();
        loop {
            let r = self.bits(k);
            if r < n {
                return usize::try_from(r).unwrap();
            }
        }
    }

    fn randrange(&mut self, start: usize, stop: usize) -> usize {
        start + self.below(stop - start)
    }

    fn choice<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// `getrandbits(8 * n).to_bytes(n, "little")`.
    fn randbytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        let mut remaining = n * 8;
        while remaining > 0 {
            let take = remaining.min(32);
            let word = self.bits(u32::try_from(take).unwrap());
            out.extend_from_slice(&word.to_le_bytes()[..take / 8]);
            remaining -= take;
        }
        out
    }
}

/// One generated array case, as the reference draws it.
fn draw_array(rng: &mut PyRandom) -> (String, ArrayAddress) {
    let prefixes = ["ns", "雪/é", "café%path"];
    let tail: Vec<char> = "abcXYZ012-_".chars().collect();
    let all_bytes: Vec<u8> = (0..=255).collect();
    let snow: Vec<u8> = "雪/é".as_bytes().to_vec();
    let alphabet: [Option<&[u8]>; 7] = [
        None,
        Some(b""),
        Some(b"/"),
        Some(b"%"),
        Some(b"~"),
        Some(&all_bytes),
        Some(&snow),
    ];
    let mut namespace = (*rng.choice(&prefixes)).to_owned();
    for _ in 0..rng.randrange(0, 12) {
        namespace.push(*rng.choice(&tail));
    }
    let count = rng.randrange(0, 10);
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        if rng.randrange(0, 3) == 0 {
            let len = rng.randrange(0, 17);
            slots.push(Some(rng.randbytes(len)));
        } else {
            slots.push(rng.choice(&alphabet).map(<[u8]>::to_vec));
        }
    }
    (namespace, ArrayAddress::new(slots))
}

#[test]
fn deterministic_corpus_matches_the_reference_digest_and_round_trips() {
    let mut rng = PyRandom::new(SEED);
    let mut hasher = Sha256::new();
    for _ in 0..10_000 {
        let (namespace, address) = draw_array(&mut rng);
        let uri = format_array("local", &namespace, &address).unwrap();
        let parsed = parse(&uri).unwrap();
        assert_eq!(parsed.resolver_hint(), "local");
        assert_eq!(parsed.namespace(), namespace);
        assert_eq!(parsed.target(), &UriTarget::Array(address));
        assert_eq!(parsed.format(), uri);
        hasher.update(uri.as_bytes());
        hasher.update(b"\n");
    }
    for _ in 0..10_000 {
        let digest: [u8; 32] = rng.randbytes(32).try_into().unwrap();
        let id = ContentId::new(HashProfile::RawBlake3_256, digest);
        let uri = format_content("local", "main", &id).unwrap();
        let parsed = parse(&uri).unwrap();
        assert_eq!(parsed.target(), &UriTarget::Content(id));
        assert_eq!(parsed.format(), uri);
        hasher.update(uri.as_bytes());
        hasher.update(b"\n");
    }
    let got = hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut out, byte| {
            write!(out, "{byte:02x}").unwrap();
            out
        });
    assert_eq!(got, CORPUS_SHA256);
}

#[test]
fn mersenne_twister_port_matches_cpython() {
    // `random.Random(0x9117_2026)`: first `getrandbits(32)` values and
    // `randbytes(5)` after them, from CPython 3.13.
    let mut rng = PyRandom::new(SEED);
    let first: Vec<u32> = (0..3).map(|_| rng.bits(32)).collect();
    assert_eq!(first, MT_FIRST_WORDS);
    assert_eq!(rng.randbytes(5), MT_THEN_BYTES);
    // `random.Random(5489).getrandbits(32)`: a second seed, so the port is
    // not only right for the one the corpus uses.
    assert_eq!(PyRandom::new(5489).bits(32), MT_5489_FIRST);
}

const MT_FIRST_WORDS: [u32; 3] = [1_688_267_078, 1_539_041_827, 1_619_599_464];
const MT_THEN_BYTES: [u8; 5] = [101, 213, 5, 69, 198];
const MT_5489_FIRST: u32 = 3_382_763_572;

#[test]
fn readable_vectors_and_content_golden_match_the_reference() {
    type Vector<'a> = (&'a str, &'a str, Vec<Option<&'a [u8]>>);
    let vectors: [Vector<'_>; 6] = [
        ("id://local/main/v1/array", "main", vec![]),
        ("id://local/main/v1/array/~", "main", vec![None]),
        ("id://local/main/v1/array/b.", "main", vec![Some(b"")]),
        ("id://local/main/v1/array/b.%2F", "main", vec![Some(b"/")]),
        (
            "id://local/wiki/v1/array/b.us%20president.md",
            "wiki",
            vec![Some(b"us president.md")],
        ),
        ("id://local/%E9%9B%AA%2F%C3%A9/v1/array", "雪/é", vec![]),
    ];
    for (uri, namespace, slots) in vectors {
        let address = ArrayAddress::new(slots.into_iter().map(|s| s.map(<[u8]>::to_vec)).collect());
        let parsed = parse(uri).unwrap();
        assert_eq!(parsed.namespace(), namespace);
        assert_eq!(parsed.target(), &UriTarget::Array(address.clone()));
        assert_eq!(format_array("local", namespace, &address).unwrap(), uri);
    }
    let empty = ContentId::of_bytes(HashProfile::RawBlake3_256, b"").unwrap();
    let golden = "id://local/main/v1/content/raw-blake3-256.\
                  af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    assert_eq!(parse(golden).unwrap().target(), &UriTarget::Content(empty));
    assert_eq!(format_content("local", "main", &empty).unwrap(), golden);
}

#[test]
fn malformed_corpus_and_negative_content_cases_reject() {
    let zeros63 = "0".repeat(63);
    let bad = [
        "id:/local/main/v1/array".to_owned(),
        "id://Local/main/v1/array".to_owned(),
        "id://local/main/v2/array".to_owned(),
        "id://local/main/v1/array/%2F".to_owned(),
        "id://local/main/v1/array/b.%".to_owned(),
        "id://local/main/v1/array/b.%2F/".to_owned(),
        "id://local/main/v1/array/b.%61".to_owned(),
        "id://local/main/v1/array?".to_owned(),
        "id://local/main/v1/array#".to_owned(),
        "id://local/%FF/v1/array".to_owned(),
        "id://local/main/v1/array?x=1".to_owned(),
        "id://local/main/v1/array#x".to_owned(),
        "id://local/../v1/array".to_owned(),
        format!("id://local/main/v1/content/unknown.{}", "0".repeat(64)),
        format!("id://local/main/v1/content/raw-blake3-256.{zeros63}"),
        format!("id://local/main/v1/content/raw-blake3-256.{zeros63}G"),
        format!("id://local/main/v1/content/raw-blake3-256.{zeros63}A"),
    ];
    for uri in &bad {
        assert!(parse(uri).is_err(), "accepted malformed URI {uri:?}");
    }
    // Digest length, case and alphabet; parser profile.
    let good = "ab".repeat(32);
    let content_cases = [
        (good[..62].to_owned(), "invalid_digest_text"),
        (format!("{good}00"), "invalid_digest_text"),
        (good.to_uppercase(), "invalid_digest_text"),
        (format!("{}zz", &good[..62]), "invalid_digest_text"),
    ];
    for (digest, class) in content_cases {
        let uri = format!("id://local/main/v1/content/raw-blake3-256.{digest}");
        assert_eq!(parse(&uri).unwrap_err().class(), class, "{uri}");
    }
    let unknown = format!("id://local/main/v1/content/sha256.{good}");
    assert_eq!(
        parse(&unknown).unwrap_err().class(),
        "unsupported_hash_profile"
    );
    // Formatter profile: only raw-blake3-256 has a URI form.
    let atlas = ContentId::new(HashProfile::AtlasChunkSha256Utf8, [0; 32]);
    assert_eq!(
        format_content("local", "main", &atlas).unwrap_err().class(),
        "unsupported_hash_profile"
    );
    // Resolver context mismatch.
    assert_eq!(
        resolve_context("local", "other").unwrap_err().class(),
        "resolver_context_mismatch"
    );
}
