//! The rejection matrix. Each input runs through four checks:
//!
//! 1. cbor-core's STRICT decoder.
//! 2. dcbor's decoder.
//! 3. Decode-then-re-encode: decode with LENIENT, which normalizes instead of refusing, then
//!    require the re-encoded bytes to equal the input.
//! 4. The wrapper in `canonical`, which is check 3 plus the value rules.

use std::panic::{self, AssertUnwindSafe};

use cbor_core::{DecodeOptions, Strictness};

use crate::{canonical, golden};

/// `Ok(())` accepts; `Err` gives the reason for rejecting.
pub type Check = Result<(), String>;

pub struct Row {
    pub name: &'static str,
    pub input: Vec<u8>,
}

pub struct Outcome {
    pub strict: Check,
    pub dcbor: Check,
    pub reencode: Check,
    /// What the lenient decoder produced, re-encoded. Empty if it did not decode.
    pub normalized: Vec<u8>,
    pub wrapper: Check,
}

fn hex(text: &str) -> Vec<u8> {
    hex::decode(text.replace(' ', "")).expect("valid hex literal")
}

pub fn rows() -> Vec<Row> {
    let row = |name: &'static str, text: &str| Row {
        name,
        input: hex(text),
    };
    vec![
        row(
            "map keys length-first, mixed types: {\"a\": 1, 1000: 2}",
            "a2 61 61 01 19 03 e8 02",
        ),
        row(
            "control: text keys, length-first = bytewise: {\"b\": 1, \"aa\": 2}",
            "a2 61 62 01 62 61 61 02",
        ),
        row(
            "map keys out of order: {\"aa\": 1, \"b\": 2}",
            "a2 62 61 61 01 61 62 02",
        ),
        row("integer 5 in a 1-byte argument", "18 05"),
        row("integer 255 in a 2-byte argument", "19 00 ff"),
        row(
            "integer 5 in an 8-byte argument",
            "1b 00 00 00 00 00 00 00 05",
        ),
        row("integer -1 in a 1-byte argument", "38 00"),
        row("text of length 1 with a 1-byte length", "78 01 61"),
        row("float 1.0 as f64", "fb 3f f0 00 00 00 00 00 00"),
        row("float 1.0 as f32", "fa 3f 80 00 00"),
        row("float 1.0 as f16", "f9 3c 00"),
        row("float 1.5 as f32 (fits f16)", "fa 3f c0 00 00"),
        row("float 1.5 as f16", "f9 3e 00"),
        row("float 0.1 as f64", "fb 3f b9 99 99 99 99 99 9a"),
        row("NaN as f16 0x7e00", "f9 7e 00"),
        row("NaN as f16, payload 1", "f9 7e 01"),
        row("-0.0 as f16", "f9 80 00"),
        row("text e + U+0301 (not NFC)", "63 65 cc 81"),
        row("text U+00E9 (NFC, control)", "62 c3 a9"),
        row("duplicate key {\"a\": 1, \"a\": 2}", "a2 61 61 01 61 61 02"),
        row(
            "indefinite byte string (chunks 01, 02)",
            "5f 41 01 41 02 ff",
        ),
        row("indefinite text string (chunks a, b)", "7f 61 61 61 62 ff"),
        row("indefinite array [1, 2]", "9f 01 02 ff"),
        row("indefinite map {\"a\": 1}", "bf 61 61 01 ff"),
        row("simple value 23 (undefined)", "f7"),
        row("simple value 16 (unassigned)", "f0"),
        row("simple value false (control)", "f4"),
        row("tag 1000 (unknown meaning) around 1", "d9 03 e8 01"),
        row(
            "bignum tag 2, 4-byte payload with a leading zero",
            "c2 44 00 00 00 01",
        ),
        Row {
            name: "p2panda header, seq 0 as 18 00, validly signed",
            input: golden::non_canonical_header(),
        },
    ]
}

/// Runs all four checks on `input`.
pub fn evaluate(input: &[u8]) -> Outcome {
    let strict = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .decode_owned(input)
        .map(drop)
        .map_err(|err| format!("{err:?}"));
    let (reencode, normalized) = match DecodeOptions::new()
        .strictness(Strictness::LENIENT)
        .decode_owned(input)
    {
        Ok(value) => {
            let bytes = value.encode();
            let check = if bytes == input {
                Ok(())
            } else {
                Err("re-encoding differs".into())
            };
            (check, bytes)
        }
        Err(err) => (Err(format!("decode error {err:?}")), Vec::new()),
    };
    Outcome {
        strict,
        dcbor: dcbor_roundtrip(input).map(drop),
        reencode,
        normalized,
        wrapper: canonical::decode(input)
            .map(drop)
            .map_err(|err| format!("{err:?}")),
    }
}

/// dcbor decodes `input` and re-encodes the result. A panic counts as a rejection.
pub fn dcbor_roundtrip(input: &[u8]) -> Result<Vec<u8>, String> {
    let attempt = panic::catch_unwind(AssertUnwindSafe(|| {
        dcbor::CBOR::try_from_data(input).map(|value| value.to_cbor_data())
    }));
    match attempt {
        Ok(result) => result.map_err(|err| format!("{err:?}")),
        Err(_) => Err("panic".into()),
    }
}
