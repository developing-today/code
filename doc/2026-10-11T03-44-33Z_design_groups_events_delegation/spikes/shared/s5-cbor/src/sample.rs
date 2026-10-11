//! Seeded test data: random canonical values and random byte mutations of their encodings.

use std::collections::BTreeMap;

use cbor_core::Value;

/// SplitMix64. Seeded, so every run of the spike sees the same values and mutations.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n`. `n` must be non-zero.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }

    /// An integer whose bit width is itself random, so every CBOR argument size gets exercised.
    fn magnitude(&mut self) -> u64 {
        match self.below(65) {
            0 => 0,
            bits => self.next_u64() >> (64 - bits),
        }
    }
}

const TEXTS: &[&str] = &[
    "",
    "a",
    "kind",
    "group.policy",
    "\u{e9}",   // precomposed e-acute, NFC
    "e\u{301}", // e and combining acute, not NFC
    "\u{212b}", // angstrom sign, not NFC (NFC is U+00C5)
    "\u{1f600}",
    "日本語",
    "a text string that runs well past twenty-four bytes",
];

const INTERESTING_BYTES: &[u8] = &[
    0x00, 0x01, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1f, 0x20, 0x38, 0x39, 0x3b, 0x41, 0x58, 0x5f, 0x60,
    0x61, 0x77, 0x78, 0x79, 0x7f, 0x80, 0x81, 0x9f, 0xa1, 0xa2, 0xbf, 0xc0, 0xc2, 0xd8, 0xd9, 0xe0,
    0xf4, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xff,
];

/// A random canonical value. Containers nest at most `depth` levels deep.
pub fn value(rng: &mut Rng, depth: u32) -> Value<'static> {
    let roll = rng.below(100);
    let inner = depth.saturating_sub(1);
    match roll {
        0..=17 => Value::Unsigned(rng.magnitude()),
        18..=24 => Value::Negative(rng.magnitude()),
        25..=36 => Value::from(bytes(rng)),
        37..=50 => Value::from(text(rng)),
        51..=58 if rng.below(2) == 0 => Value::from(rng.below(2) == 0),
        51..=58 => Value::null(),
        59..=62 => Value::from(float(rng)),
        63..=64 => Value::simple_value(rng.pick(&[0, 16, 23])),
        65..=70 if depth > 0 => Value::tag(tag_number(rng), value(rng, inner)),
        71..=85 if depth > 0 => {
            let items: Vec<Value<'static>> = (0..rng.below(5)).map(|_| value(rng, inner)).collect();
            Value::from(items)
        }
        86..=99 if depth > 0 => map(rng, inner),
        _ => Value::Unsigned(rng.magnitude()),
    }
}

fn map(rng: &mut Rng, depth: u32) -> Value<'static> {
    let mut pairs = BTreeMap::new();
    for _ in 0..rng.below(5) {
        let key = match rng.below(4) {
            0 => Value::from(text(rng)),
            1 => Value::Unsigned(rng.below(40)),
            2 => Value::Negative(rng.below(40)),
            _ => Value::from(bytes(rng)),
        };
        pairs.insert(key, value(rng, depth));
    }
    Value::from(pairs)
}

fn bytes(rng: &mut Rng) -> Vec<u8> {
    let len = match rng.below(3) {
        0 => rng.pick(&[0, 1, 23, 24, 25, 256]),
        _ => rng.below(40) as usize,
    };
    (0..len).map(|_| rng.next_u64() as u8).collect()
}

fn text(rng: &mut Rng) -> String {
    (0..1 + rng.below(3)).map(|_| rng.pick(TEXTS)).collect()
}

fn tag_number(rng: &mut Rng) -> u64 {
    rng.pick(&[0, 1, 2, 3, 24, 42, 1000, 55799])
}

fn float(rng: &mut Rng) -> f64 {
    const FIXED: [f64; 8] = [1.0, -0.0, 0.5, 1.5, 0.1, 65504.0, f64::INFINITY, f64::NAN];
    match rng.below(3) {
        0 => FIXED[rng.below(FIXED.len() as u64) as usize],
        1 => f64::from_bits(0x7ff8_0000_0000_0001), // NaN with a payload
        _ => f64::from_bits(rng.next_u64()),
    }
}

/// A random byte mutation of `input`: one to three edits (bit flip, byte replace, insert,
/// delete, or truncate), each at a random position, often with an interesting byte.
pub fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..1 + rng.below(3) {
        let byte = if rng.below(2) == 0 {
            rng.pick(INTERESTING_BYTES)
        } else {
            rng.next_u64() as u8
        };
        if out.is_empty() {
            out.push(byte);
            continue;
        }
        let i = rng.below(out.len() as u64) as usize;
        match rng.below(5) {
            0 => out[i] ^= 1 << rng.below(8),
            1 => out[i] = byte,
            2 => out.insert(i, byte),
            3 => {
                out.remove(i);
            }
            _ => out.truncate(i),
        }
    }
    out
}
