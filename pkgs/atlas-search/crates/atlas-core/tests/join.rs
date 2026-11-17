//! Invariant J1 property suite for the §4.1 assembler (design record
//! `doc/2026-11-10T22-56-00Z_design_atlas_delta_detection_join/` §4.2,
//! developing-today/code#9979).
//!
//! Oracle: the version text itself. For random texts, random paragraph
//! extents and random span cuts — including adversarial overlaps
//! (nested spans, identical content at different offsets, spans
//! crossing paragraph boundaries) — assembly must emit exactly
//! `V[P.start .. P.end)`, byte for byte: no byte twice, none missing,
//! none invented.
//!
//! A second, search-level statement: a paragraph returned for a hit
//! contains the hit's matched text exactly once per occurrence of that
//! text in the paragraph itself — repetition in the source is the
//! source, not doubling.

use atlas_core::join::{
    Hit, Paragraph, RenderRequest, RenderResult, Span, SpanOccurrence, SpanTable, VersionCatalog,
    VersionRecord, assemble,
};
use proptest::prelude::*;

/// Text shaped like real paragraph-bearing bytes: printable ASCII,
/// spaces and newlines, no multi-byte sequences (anchors are bytes, so
/// this is the general case; UTF-8 is a subset of it).
fn text_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop_oneof![
            (b'a'..=b'z').prop_map(|b| b),
            Just(b' '),
            Just(b'\n'),
            Just(b'.'),
        ],
        0..256,
    )
}

/// A paragraph extent inside `len`, sometimes empty.
fn paragraph_strategy(len: usize) -> impl Strategy<Value = Paragraph> {
    (0..=len)
        .prop_flat_map(move |start| (Just(start), start..=len))
        .prop_map(|(start, end)| Paragraph::new(start, end))
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        .. ProptestConfig::default()
    })]

    /// J1 with random spans over random paragraph extents.
    #[test]
    fn j1_random_spans_never_double_or_drop_bytes(
        (text, paragraph, seed) in text_strategy().prop_flat_map(|text| {
            let len = text.len();
            (Just(text), paragraph_strategy(len), any::<u64>())
        }),
    ) {
        let spans = random_spans(seed, text.len(), (seed % 9) as usize);
        let assembled = assemble(&text, paragraph, &spans).expect("assembly");
        prop_assert_eq!(assembled.bytes.as_slice(), &text[paragraph.start..paragraph.end]);
        assert_highlights_well_formed(&assembled, paragraph, &text);
    }
}

/// Deterministic pseudo-random span sets (no OS entropy in proptest
/// strategies keeps failures reproducible).
fn random_spans(seed: u64, len: usize, count: usize) -> Vec<SpanOccurrence> {
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    let mut next = |bound: usize| -> usize {
        if bound == 0 {
            return 0;
        }
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % bound
    };
    let mut spans = Vec::with_capacity(count);
    for id in 0..count {
        let start = next(len + 1);
        let end = start + next(len - start.min(len) + 1);
        spans.push(SpanOccurrence::new(id as u64, 1, start, end));
    }
    spans
}

fn assert_highlights_well_formed(
    assembled: &atlas_core::join::Assembled,
    paragraph: Paragraph,
    text: &[u8],
) {
    let mut previous_end = 0;
    for range in &assembled.highlights {
        assert!(range.start < range.end, "empty highlight {range:?}");
        assert!(
            range.end <= paragraph.len(),
            "highlight {range:?} escapes paragraph of {} bytes",
            paragraph.len()
        );
        assert!(
            range.start >= previous_end,
            "highlights not sorted/merged: {range:?} after end {previous_end}"
        );
        previous_end = range.end;
    }
    // Assembly is byte-exact regardless of highlight merging.
    assert_eq!(
        assembled.bytes.as_slice(),
        &text[paragraph.start..paragraph.end]
    );
}

mod adversarial {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

        /// Nested spans, exact duplicates, and identical content at
        /// different offsets: the adversarial overlaps named in §4.2.
        #[test]
        fn j1_adversarial_overlaps(text in text_strategy()) {
            let len = text.len();
            let mut spans: Vec<SpanOccurrence> = Vec::new();
            let mut id = 0u64;
            let push = |start: usize, end: usize, spans: &mut Vec<SpanOccurrence>, id: &mut u64| {
                let end = end.min(len).max(start.min(len));
                spans.push(SpanOccurrence::new(*id, 1, start.min(len), end));
                *id += 1;
            };

            if len > 0 {
                // One long span over everything, plus nested sub-spans.
                push(0, len, &mut spans, &mut id);
                let third = len / 3;
                push(third / 2, third * 2, &mut spans, &mut id);
                push(third, third * 2, &mut spans, &mut id);
                // Exact duplicate of the previous span.
                push(third, third * 2, &mut spans, &mut id);
                // Identical content at a different offset (same length).
                push(third * 2, (third * 3).min(len), &mut spans, &mut id);
                // Spans crossing both paragraph boundaries below.
                push(0, len, &mut spans, &mut id);
                push(third / 2, len - third / 2, &mut spans, &mut id);
                // Tail and head touches.
                push(len - 1, len, &mut spans, &mut id);
                push(0, 1, &mut spans, &mut id);
                // Empty span: contributes a highlight but no bytes.
                push(third, third, &mut spans, &mut id);
            }

            let paragraph = Paragraph::new(len / 4, len - len / 4);
            let assembled = assemble(&text, paragraph, &spans).expect("assembly");
            assert_highlights_well_formed(&assembled, paragraph, &text);
            prop_assert_eq!(
                assembled.bytes.as_slice(),
                &text[paragraph.start..paragraph.end]
            );
        }

        /// Many small random spans, dense overlap — a stress case.
        #[test]
        fn j1_dense_random_spans(text in text_strategy(), seed in any::<u64>()) {
            let len = text.len();
            let spans = random_spans(seed, len, 24);
            let paragraph = Paragraph::new(len / 5, len - len / 5);
            let assembled = assemble(&text, paragraph, &spans).expect("assembly");
            assert_highlights_well_formed(&assembled, paragraph, &text);
            prop_assert_eq!(
                assembled.bytes.as_slice(),
                &text[paragraph.start..paragraph.end]
            );
        }

        /// The search-level statement: source repetition survives
        /// assembly (the rendering repeats a line exactly as often as
        /// the paragraph does), and nothing is added.
        #[test]
        fn source_repetition_is_preserved_never_invented(
            line in "[a-z]{1,12}",
            repeats in 1usize..6,
        ) {
            let mut text = Vec::new();
            for _ in 0..repeats {
                text.extend_from_slice(line.as_bytes());
                text.push(b'\n');
            }
            let paragraph = Paragraph::whole(&text);
            let mut spans = Vec::new();
            for id in 0..repeats {
                let start = id * (line.len() + 1);
                spans.push(SpanOccurrence::new(id as u64, 1, start, start + line.len()));
            }
            let assembled = assemble(&text, paragraph, &spans).expect("assembly");
            prop_assert_eq!(assembled.bytes.as_slice(), text.as_slice());
            // Rendering contains the line exactly `repeats` times, the
            // same count as the source paragraph.
            let rendered = assembled.text();
            prop_assert_eq!(rendered.matches(line.as_str()).count(), repeats);
        }
    }
}

/// Cross-version rendering: the mapped range comes from the occurrence
/// row, so it works even when the offsets differ between versions.
#[test]
fn occurrence_mapping_is_the_only_cross_version_route() {
    let v1 = b"prefix-content-that-matches";
    let v2 = b"a-much-longer-prefix-content-that-matches";

    let mut table = SpanTable::new();
    let content = b"content-that-matches";
    table.land(Span::live(1, content.to_vec()), 1, 7, v1.len());
    table.add_occurrence(SpanOccurrence::new(1, 2, 21, v2.len()));

    let mut catalog = VersionCatalog::new();
    catalog.insert(VersionRecord::live(1, v1.to_vec()));
    catalog.insert(VersionRecord::live(2, v2.to_vec()));

    let request = RenderRequest {
        hit: Hit {
            span_id: 1,
            version_id: 1,
        },
        target_version: 2,
        paragraph: Paragraph::whole(v2),
    };
    let RenderResult::Paragraph {
        assembled, matched, ..
    } = atlas_core::join::render(&table, &catalog, &request).unwrap()
    else {
        panic!("expected paragraph render");
    };
    assert_eq!(assembled.bytes, v2);
    assert_eq!(matched, Some(21..v2.len()));
    // Offset arithmetic would have produced 7..29 — the wrong content.
    assert_ne!(matched, Some(7..7 + content.len()));
}
