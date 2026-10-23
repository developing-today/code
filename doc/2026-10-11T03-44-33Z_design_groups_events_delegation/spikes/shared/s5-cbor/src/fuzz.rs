//! Decode-then-re-encode over generated canonical values and their random byte mutations.

use std::collections::BTreeMap;

use cbor_core::{DecodeOptions, Strictness, Value};

use crate::{canonical, matrix, sample};

#[derive(Debug, Default)]
pub struct Stats {
    pub values: u64,
    /// Generated encodings that STRICT accepts and that re-encode identically.
    pub baseline_strict_ok: u64,
    /// Generated encodings that STRICT refuses. `baseline_refused_bignum` counts those whose
    /// value holds a bignum tag (2 or 3), which cbor-core's encoder does not normalize.
    pub baseline_strict_refused: u64,
    pub baseline_refused_bignum: u64,
    pub baseline_refused_examples: Vec<String>,
    /// Generated values that also pass the wrapper's value rules.
    pub baseline_signable: u64,
    /// Generated encodings that dcbor accepts.
    pub baseline_dcbor_ok: u64,
    pub mutants: u64,
    pub strict_accepted: u64,
    /// STRICT-accepted mutants whose re-encoding differs from the input. Must be zero.
    pub reencode_mismatch: u64,
    /// Mutants whose decode-then-re-encode check (LENIENT) passes.
    pub reencode_identical: u64,
    /// Mutants where STRICT's verdict differs from the re-encode check's verdict. Must be zero.
    pub strict_vs_reencode_disagree: u64,
    pub strict_rejected_by: BTreeMap<String, u64>,
    /// STRICT-accepted mutants that the wrapper's value rules refuse, by rule.
    pub rule_refused: BTreeMap<String, u64>,
    pub wrapper_accepted: u64,
    pub both_accept: u64,
    pub both_reject: u64,
    /// cbor-core accepts, dcbor refuses, by dcbor's reason.
    pub dcbor_refuses: BTreeMap<String, u64>,
    /// dcbor accepts, cbor-core refuses. `dcbor_only_bignum` counts those that hold a bignum tag.
    pub dcbor_only: u64,
    pub dcbor_only_bignum: u64,
    pub dcbor_only_examples: Vec<String>,
    /// dcbor accepts but re-encodes to different bytes. Must be zero.
    pub dcbor_reencode_differs: u64,
    /// The wrapper accepts and dcbor refuses. Must be zero for the interop claim.
    pub wrapper_accepted_dcbor_refused: u64,
    /// Panics from either decoder. Must be zero.
    pub panics: u64,
}

pub fn run(seed: u64, values: u64, mutants_per_value: u64) -> Stats {
    let mut rng = sample::Rng::new(seed);
    let mut stats = Stats::default();
    for _ in 0..values {
        let value = sample::value(&mut rng, 3);
        let encoded = value.encode();
        stats.values += 1;
        if strict_roundtrip(&encoded) {
            stats.baseline_strict_ok += 1;
        } else {
            stats.baseline_strict_refused += 1;
            let reason = DecodeOptions::new()
                .strictness(Strictness::STRICT)
                .decode_owned(&encoded)
                .err();
            if reason == Some(cbor_core::Error::NonDeterministic) && has_bignum(&value) {
                stats.baseline_refused_bignum += 1;
            }
            if stats.baseline_refused_examples.len() < 3 {
                stats.baseline_refused_examples.push(hex::encode(&encoded));
            }
        }
        if canonical::decode(&encoded).is_ok() {
            stats.baseline_signable += 1;
        }
        if matrix::dcbor_roundtrip(&encoded).is_ok() {
            stats.baseline_dcbor_ok += 1;
        }
        for _ in 0..mutants_per_value {
            let mutant = sample::mutate(&mut rng, &encoded);
            stats.mutants += 1;
            check(&mutant, &mut stats);
        }
    }
    stats
}

/// STRICT accepts `bytes`, and encoding the result gives back `bytes`.
fn strict_roundtrip(bytes: &[u8]) -> bool {
    DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .decode_owned(bytes)
        .is_ok_and(|value| value.encode() == bytes)
}

/// The decode-then-re-encode check with LENIENT decoding. Normalizing decoders are still
/// caught: the normalized bytes differ from the input.
fn reencode_identical(bytes: &[u8]) -> bool {
    DecodeOptions::new()
        .strictness(Strictness::LENIENT)
        .decode_owned(bytes)
        .is_ok_and(|value| value.encode() == bytes)
}

fn check(mutant: &[u8], stats: &mut Stats) {
    let strict = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .decode_owned(mutant);
    let strict_ok = match &strict {
        Ok(value) => {
            stats.strict_accepted += 1;
            if value.encode() != mutant {
                stats.reencode_mismatch += 1;
            }
            true
        }
        Err(err) => {
            *stats
                .strict_rejected_by
                .entry(format!("{err:?}"))
                .or_default() += 1;
            false
        }
    };

    let reencode_ok = reencode_identical(mutant);
    if reencode_ok {
        stats.reencode_identical += 1;
    }
    if strict_ok != reencode_ok {
        stats.strict_vs_reencode_disagree += 1;
    }

    let wrapper = canonical::decode(mutant);
    match &wrapper {
        Ok(_) => stats.wrapper_accepted += 1,
        Err(canonical::Error::Decode(_) | canonical::Error::NotCanonical) => {}
        Err(rule) => *stats.rule_refused.entry(rule_name(rule)).or_default() += 1,
    }

    let dcbor = matrix::dcbor_roundtrip(mutant);
    match (strict_ok, &dcbor) {
        (true, Ok(bytes)) => {
            stats.both_accept += 1;
            if bytes.as_slice() != mutant {
                stats.dcbor_reencode_differs += 1;
            }
        }
        (false, Err(_)) => stats.both_reject += 1,
        (true, Err(reason)) => {
            let name = reason.split('(').next().unwrap_or_default().to_string();
            *stats.dcbor_refuses.entry(name).or_default() += 1;
        }
        (false, Ok(_)) => {
            stats.dcbor_only += 1;
            if dcbor::CBOR::try_from_data(mutant).is_ok_and(|value| dcbor_has_bignum(&value)) {
                stats.dcbor_only_bignum += 1;
            }
            if stats.dcbor_only_examples.len() < 5 {
                stats.dcbor_only_examples.push(hex::encode(mutant));
            }
        }
    }
    if wrapper.is_ok() && dcbor.is_err() {
        stats.wrapper_accepted_dcbor_refused += 1;
    }
    if matches!(&dcbor, Err(reason) if reason == "panic") {
        stats.panics += 1;
    }
}

/// True if a generated value holds a bignum tag (2 or 3) anywhere.
fn has_bignum(value: &Value) -> bool {
    match value {
        Value::Tag(number, content) => matches!(number, 2 | 3) || has_bignum(content),
        Value::Array(items) => items.iter().any(has_bignum),
        Value::Map(map) => map.iter().any(|(k, v)| has_bignum(k) || has_bignum(v)),
        _ => false,
    }
}

/// True if a dcbor-decoded value holds a bignum tag (2 or 3) anywhere.
fn dcbor_has_bignum(value: &dcbor::CBOR) -> bool {
    match value.as_case() {
        dcbor::CBORCase::Tagged(tag, content) => {
            matches!(tag.value(), 2 | 3) || dcbor_has_bignum(content)
        }
        dcbor::CBORCase::Array(items) => items.iter().any(dcbor_has_bignum),
        dcbor::CBORCase::Map(map) => map
            .iter()
            .any(|(k, v)| dcbor_has_bignum(k) || dcbor_has_bignum(v)),
        _ => false,
    }
}

fn rule_name(rule: &canonical::Error) -> String {
    match rule {
        canonical::Error::NotNfc => "text not NFC".into(),
        canonical::Error::Float => "float".into(),
        canonical::Error::SimpleValue(_) => "simple value other than false/true/null".into(),
        canonical::Error::Tag(_) => "tag".into(),
        other => format!("{other:?}"),
    }
}
