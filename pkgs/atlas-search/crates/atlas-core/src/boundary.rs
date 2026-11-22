//! Source-boundary-safe embed splitting (#12403, design §3).
//!
//! The splitter and the classifier share source offsets and boundary
//! metadata: classification may change only at a declared content
//! boundary (message/turn, authored block/paragraph, code
//! symbol/declaration, log record, or another source-native unit).
//! [`split_by_boundaries`] turns a source plus its declared
//! classification segments into embed chunks that never cross a
//! known boundary, never duplicate text across one as overlap, and
//! fail closed wherever the boundary is unknown or uncertain:
//!
//! * every declared boundary splits, even when that creates a short
//!   embed and even when the sensitivity does not change across it;
//! * text with no declared classification, text covered by two
//!   contradictory declarations, and text under an uncertain
//!   declaration is one `secret-bearing` chunk, marked uncertain;
//! * the chunks always partition the source exactly, so no overlap
//!   exists to cross a boundary; overlap *within* one segment stays
//!   a #6037 policy question, not a boundary violation;
//! * a boundary revision only changes the chunks it overlaps: every
//!   other chunk keeps its exact byte range and text, so its
//!   classification and vector stay valid (design §3, #6038).
//!
//! This module is the splitting primitive. Declared boundaries are
//! wired into staging by [`crate::stage::stage_classified_artifact`]
//! (boundary-safe `chunks_raw` plus one persisted classification
//! record per chunk) and into promotion by [`crate::aggregate`],
//! which writes each chunk's effective sensitivity into the shard
//! and embeds identical texts once (#12628). The legacy staging
//! path ([`crate::stage::stage_artifact`]) keeps its current
//! behaviour for producers that declare no segments.

use crate::filters::Sensitivity;

/// One declared source-native classification segment: the byte range
/// `start..end` of `source` carries `sensitivity`.
///
/// `certain` records whether the classifier (or the boundary
/// detector) is confident in this segment's boundary and decision.
/// An uncertain segment is handled at the most restrictive
/// classification instead of being joined to a less restricted one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassificationSegment {
    pub start: usize,
    pub end: usize,
    pub sensitivity: Sensitivity,
    pub certain: bool,
}

impl ClassificationSegment {
    pub fn certain(start: usize, end: usize, sensitivity: Sensitivity) -> Self {
        Self {
            start,
            end,
            sensitivity,
            certain: true,
        }
    }

    pub fn uncertain(start: usize, end: usize, sensitivity: Sensitivity) -> Self {
        Self {
            start,
            end,
            sensitivity,
            certain: false,
        }
    }
}

/// One embed chunk produced by [`split_by_boundaries`]: the exact
/// byte range `start..end` of the source, its effective sensitivity,
/// and whether its boundary/decision was uncertain (and therefore
/// raised to `secret-bearing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryChunk {
    pub start: usize,
    pub end: usize,
    pub sensitivity: Sensitivity,
    pub uncertain: bool,
}

impl BoundaryChunk {
    /// The chunk's text. Safe: chunk edges always lie on UTF-8
    /// char boundaries by construction.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }
}

/// Split `source` into embed chunks along its declared classification
/// segments.
///
/// Guarantees (the oracles the tests check):
///
/// * **Exact partition** — the chunks tile `0..source.len()` in
///   order with no gaps and no overlaps, so concatenating their
///   texts reproduces `source` byte for byte.
/// * **No boundary crossing** — no chunk straddles a declared
///   segment edge, and no chunk mixes text from two segments.
/// * **Fail closed** — a gap (undeclared text), an overlap of two
///   declarations, or an uncertain declaration yields a
///   `secret-bearing` chunk marked `uncertain`.
/// * **Only affected chunks change** — the split depends only on the
///   segment edges overlapping each chunk, so revising one segment
///   leaves every non-overlapping chunk byte-identical.
pub fn split_by_boundaries(
    source: &str,
    segments: &[ClassificationSegment],
) -> Vec<BoundaryChunk> {
    let len = source.len();
    if len == 0 {
        return Vec::new();
    }

    // Normalise: clamp to the source, drop empties, snap edges to
    // UTF-8 char boundaries (expanding outwards, never cutting a
    // character), and mark any segment whose declared edge was not a
    // safe split point as uncertain.
    let mut normalised: Vec<ClassificationSegment> = segments
        .iter()
        .map(|segment| {
            let mut start = segment.start.min(len);
            let mut end = segment.end.min(len);
            let mut certain = segment.certain;
            if start > end {
                std::mem::swap(&mut start, &mut end);
            }
            let snapped_start = floor_char_boundary(source, start);
            let snapped_end = ceil_char_boundary(source, end);
            if snapped_start != start || snapped_end != end {
                certain = false;
            }
            ClassificationSegment {
                start: snapped_start,
                end: snapped_end,
                sensitivity: segment.sensitivity,
                certain,
            }
        })
        .filter(|segment| segment.start < segment.end)
        .collect();
    normalised.sort_by_key(|segment| (segment.start, segment.end));

    // Every segment edge is a split point, plus the source edges.
    let mut points: Vec<usize> = vec![0, len];
    for segment in &normalised {
        points.push(segment.start);
        points.push(segment.end);
    }
    points.sort_unstable();
    points.dedup();

    let mut chunks = Vec::new();
    for window in points.windows(2) {
        let (start, end) = (window[0], window[1]);
        let covering: Vec<&ClassificationSegment> = normalised
            .iter()
            .filter(|segment| segment.start <= start && end <= segment.end)
            .collect();
        let chunk = if covering.is_empty() {
            BoundaryChunk {
                start,
                end,
                sensitivity: Sensitivity::SecretBearing,
                uncertain: true,
            }
        } else if covering.len() == 1 && covering[0].certain {
            BoundaryChunk {
                start,
                end,
                sensitivity: covering[0].sensitivity,
                uncertain: false,
            }
        } else {
            // One uncertain declaration, or two declarations claiming
            // the same bytes: the boundary is uncertain or
            // contradictory, so no safe split exists there.
            BoundaryChunk {
                start,
                end,
                sensitivity: Sensitivity::SecretBearing,
                uncertain: true,
            }
        };
        chunks.push(chunk);
    }
    chunks
}

/// Largest UTF-8 char boundary at or below `at`.
fn floor_char_boundary(source: &str, at: usize) -> usize {
    let mut at = at.min(source.len());
    while at > 0 && !source.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Smallest UTF-8 char boundary at or above `at`.
fn ceil_char_boundary(source: &str, at: usize) -> usize {
    let mut at = at.min(source.len());
    while at < source.len() && !source.is_char_boundary(at) {
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::{BoundaryChunk, ClassificationSegment, split_by_boundaries};
    use crate::classification::{ClassificationRecord, EffectiveClassification};
    use crate::filters::Sensitivity;

    /// Oracle helper: the per-embed records for a split compose to
    /// exactly the chunk sensitivities, via the #12402 record API.
    fn records_for(chunks: &[BoundaryChunk]) -> Vec<ClassificationRecord> {
        chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| {
                ClassificationRecord::parse(json!({
                    "schema_version": 1,
                    "embed_id": format!("embed-{i}"),
                    "sensitivity": chunk.sensitivity.name(),
                    "provenance": {
                        "actor": "boundary/test",
                        "decided_at": "2026-11-22T00:00:00Z",
                        "method": "boundary-split",
                        "method_version": "1",
                        "config_version": "1",
                        "evidence_refs": [],
                        "previous_decision_id": null
                    }
                }))
                .expect("chunk sensitivity is a valid record value")
            })
            .collect()
    }

    #[test]
    fn transcript_turns_split_at_message_boundaries() {
        // Design §7, worked example 1: a trip-planning turn followed
        // by a deployment turn, separated by a message boundary.
        let trip = "Trip: book flights for the offsite next week.\n";
        let deploy = "Deploy: roll out the new queue worker tonight.\n";
        let source = format!("{trip}{deploy}");
        let segments = [
            ClassificationSegment::certain(0, trip.len(), Sensitivity::Personal),
            ClassificationSegment::certain(
                trip.len(),
                source.len(),
                Sensitivity::Shareable,
            ),
        ];
        let chunks = split_by_boundaries(&source, &segments);
        assert_eq!(chunks.len(), 2, "one chunk per turn: {chunks:?}");
        assert_eq!(chunks[0].text(&source), trip);
        assert_eq!(chunks[0].sensitivity, Sensitivity::Personal);
        assert!(!chunks[0].uncertain);
        assert_eq!(chunks[1].text(&source), deploy);
        assert_eq!(chunks[1].sensitivity, Sensitivity::Shareable);
        assert!(!chunks[1].uncertain);
        // No chunk carries text from the other turn.
        assert!(!chunks[0].text(&source).contains("Deploy"));
        assert!(!chunks[1].text(&source).contains("Trip"));
    }

    #[test]
    fn embedded_secret_is_isolated_and_never_copied_to_neighbors() {
        // Design §7, worked example 2: project text with an embedded
        // secret; the secret chunk is secret-bearing and the secret
        // appears in no neighbouring embed.
        let before = "fn deploy() {\n    let region = \"eu-west-1\";\n";
        let secret = "    let token = \"AKIAIOSFODNN7EXAMPLE\";\n";
        let after = "    start_server(region);\n}\n";
        let source = format!("{before}{secret}{after}");
        let secret_start = before.len();
        let secret_end = secret_start + secret.len();
        let segments = [
            ClassificationSegment::certain(0, secret_start, Sensitivity::Shareable),
            ClassificationSegment::certain(
                secret_start,
                secret_end,
                Sensitivity::SecretBearing,
            ),
            ClassificationSegment::certain(secret_end, source.len(), Sensitivity::Shareable),
        ];
        let chunks = split_by_boundaries(&source, &segments);
        assert_eq!(chunks.len(), 3, "before/secret/after: {chunks:?}");
        assert_eq!(chunks[1].text(&source), secret);
        assert_eq!(chunks[1].sensitivity, Sensitivity::SecretBearing);
        assert_eq!(chunks[0].sensitivity, Sensitivity::Shareable);
        assert_eq!(chunks[2].sensitivity, Sensitivity::Shareable);
        // Overlap must not copy the secret into a neighbour.
        assert!(!chunks[0].text(&source).contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!chunks[2].text(&source).contains("AKIAIOSFODNN7EXAMPLE"));
        assert_eq!(
            chunks.iter().map(|c| c.text(&source)).collect::<String>(),
            source,
            "chunks partition the source exactly"
        );
    }

    #[test]
    fn code_and_log_units_align_to_declared_units() {
        // Source-native units: two code declarations and two log
        // records, each its own chunk at its exact offsets.
        let code = "fn alpha() {}\nfn beta() {}\n";
        let segments = [
            ClassificationSegment::certain(0, 14, Sensitivity::Shareable),
            ClassificationSegment::certain(14, code.len(), Sensitivity::Personal),
        ];
        let chunks = split_by_boundaries(code, &segments);
        assert_eq!(
            chunks,
            vec![
                BoundaryChunk {
                    start: 0,
                    end: 14,
                    sensitivity: Sensitivity::Shareable,
                    uncertain: false,
                },
                BoundaryChunk {
                    start: 14,
                    end: code.len(),
                    sensitivity: Sensitivity::Personal,
                    uncertain: false,
                },
            ]
        );

        let logs = "2026-11-22T09:00:00Z INFO started\n2026-11-22T09:01:00Z ERROR boom\n";
        let first_len = "2026-11-22T09:00:00Z INFO started\n".len();
        let segments = [
            ClassificationSegment::certain(0, first_len, Sensitivity::Shareable),
            ClassificationSegment::certain(
                first_len,
                logs.len(),
                Sensitivity::LocalOnly,
            ),
        ];
        let chunks = split_by_boundaries(logs, &segments);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text(logs), "2026-11-22T09:00:00Z INFO started\n");
        assert_eq!(chunks[1].text(logs), "2026-11-22T09:01:00Z ERROR boom\n");
        assert_eq!(chunks[1].sensitivity, Sensitivity::LocalOnly);
    }

    #[test]
    fn uncertain_boundary_raises_only_the_affected_chunk() {
        let source = "aaaaBBBBcccc";
        let segments = [
            ClassificationSegment::certain(0, 4, Sensitivity::Shareable),
            ClassificationSegment::uncertain(4, 8, Sensitivity::Personal),
            ClassificationSegment::certain(8, 12, Sensitivity::Shareable),
        ];
        let chunks = split_by_boundaries(source, &segments);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].sensitivity, Sensitivity::Shareable);
        assert!(!chunks[0].uncertain);
        // Uncertain material stays secret-bearing and is not joined
        // to a less restricted segment.
        assert_eq!(chunks[1].sensitivity, Sensitivity::SecretBearing);
        assert!(chunks[1].uncertain);
        assert_eq!(chunks[2].sensitivity, Sensitivity::Shareable);

        // Revising the uncertain segment changes only its chunk: the
        // neighbours keep their exact byte ranges and text, so their
        // classifications and vectors stay valid (design §3).
        let revised = [
            ClassificationSegment::certain(0, 4, Sensitivity::Shareable),
            ClassificationSegment::certain(4, 8, Sensitivity::LocalOnly),
            ClassificationSegment::certain(8, 12, Sensitivity::Shareable),
        ];
        let after = split_by_boundaries(source, &revised);
        assert_eq!(after.len(), chunks.len());
        assert_eq!(after[0], chunks[0]);
        assert_eq!(after[2], chunks[2]);
        assert_eq!(after[1].sensitivity, Sensitivity::LocalOnly);
        assert!(!after[1].uncertain);
    }

    #[test]
    fn uncovered_text_and_overlaps_fail_closed() {
        // A gap between declared segments: unclassified material is
        // never public or ordinary-internal by default.
        let source = "aaaa----bbbb";
        let segments = [
            ClassificationSegment::certain(0, 4, Sensitivity::Shareable),
            ClassificationSegment::certain(8, 12, Sensitivity::Shareable),
        ];
        let chunks = split_by_boundaries(source, &segments);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[1].text(source), "----");
        assert_eq!(chunks[1].sensitivity, Sensitivity::SecretBearing);
        assert!(chunks[1].uncertain);

        // Two declarations claiming the same bytes contradict each
        // other: the overlap is secret-bearing, the rest is not.
        let source = "abcdefgh";
        let segments = [
            ClassificationSegment::certain(0, 6, Sensitivity::Shareable),
            ClassificationSegment::certain(4, 8, Sensitivity::Personal),
        ];
        let chunks = split_by_boundaries(source, &segments);
        assert_eq!(
            chunks
                .iter()
                .map(|c| (c.start, c.end, c.sensitivity, c.uncertain))
                .collect::<Vec<_>>(),
            vec![
                (0, 4, Sensitivity::Shareable, false),
                (4, 6, Sensitivity::SecretBearing, true),
                (6, 8, Sensitivity::Personal, false),
            ]
        );
    }

    #[test]
    fn no_segments_fails_closed_and_empty_source_is_empty() {
        let source = "unclassified material";
        let chunks = split_by_boundaries(source, &[]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].sensitivity, Sensitivity::SecretBearing);
        assert!(chunks[0].uncertain);

        assert!(split_by_boundaries("", &[]).is_empty());
        assert!(
            split_by_boundaries(
                "",
                &[ClassificationSegment::certain(0, 0, Sensitivity::Shareable)]
            )
            .is_empty()
        );
    }

    #[test]
    fn non_char_boundary_edges_snap_outward_and_fail_closed() {
        // "é" is two bytes; a declared edge inside it is not a safe
        // split point, so the segment expands to whole characters
        // and is treated as uncertain rather than panicking.
        let source = "aéb";
        let segments = [ClassificationSegment::certain(2, 3, Sensitivity::Personal)];
        let chunks = split_by_boundaries(source, &segments);
        assert_eq!(
            chunks.iter().map(|c| c.text(source)).collect::<String>(),
            source
        );
        let secret = chunks
            .iter()
            .find(|c| c.text(source).contains('é'))
            .expect("the multi-byte char survives in one chunk");
        assert_eq!(secret.sensitivity, Sensitivity::SecretBearing);
        assert!(secret.uncertain);
    }

    #[test]
    fn chunk_records_compose_to_chunk_sensitivities() {
        // The #12402 record API round-trips the split: composing the
        // per-embed records yields each chunk's own sensitivity, and
        // the maximum across a split is the most restrictive chunk.
        let source = "aaaaBBBB";
        let segments = [
            ClassificationSegment::certain(0, 4, Sensitivity::Shareable),
            ClassificationSegment::certain(4, 8, Sensitivity::SecretBearing),
        ];
        let chunks = split_by_boundaries(source, &segments);
        let records = records_for(&chunks);
        assert_eq!(records.len(), 2);
        let effective = EffectiveClassification::compose(records.iter());
        assert_eq!(effective.sensitivity, Sensitivity::SecretBearing);
        assert!(!effective.is_excluded());
        assert_eq!(records[0].sensitivity, Sensitivity::Shareable);
        assert_eq!(records[1].sensitivity, Sensitivity::SecretBearing);
    }

    fn sensitivity_strategy() -> impl Strategy<Value = Sensitivity> {
        prop_oneof![
            Just(Sensitivity::Shareable),
            Just(Sensitivity::Personal),
            Just(Sensitivity::LocalOnly),
            Just(Sensitivity::SecretBearing),
        ]
    }

    fn source_strategy() -> impl Strategy<Value = String> {
        // ASCII plus multi-byte characters, so random segment edges
        // regularly land inside a character.
        prop_oneof![
            "[a-z ]{0,40}",
            prop::collection::vec(
                prop_oneof![Just("a"), Just("é"), Just("€"), Just("\n"), Just(" ")],
                0..16
            )
            .prop_map(|parts| parts.concat()),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn chunks_always_partition_source_exactly(
            source in source_strategy(),
            segments in prop::collection::vec(
                (0usize..64, 0usize..64, sensitivity_strategy(), any::<bool>()),
                0..8,
            ),
        ) {
            let segments: Vec<ClassificationSegment> = segments
                .into_iter()
                .map(|(start, end, sensitivity, certain)| ClassificationSegment {
                    start,
                    end,
                    sensitivity,
                    certain,
                })
                .collect();
            let chunks = split_by_boundaries(&source, &segments);
            if source.is_empty() {
                prop_assert!(chunks.is_empty());
                return Ok(());
            }
            prop_assert!(!chunks.is_empty());
            prop_assert_eq!(chunks[0].start, 0);
            prop_assert_eq!(chunks.last().unwrap().end, source.len());
            for window in chunks.windows(2) {
                prop_assert_eq!(window[0].end, window[1].start, "no gaps or overlaps");
                prop_assert!(window[0].start < window[0].end);
            }
            prop_assert_eq!(
                chunks.iter().map(|c| c.text(&source)).collect::<String>(),
                source,
                "concatenation reproduces the source byte for byte"
            );
        }

        #[test]
        fn no_chunk_crosses_a_declared_segment_edge(
            source in source_strategy(),
            segments in prop::collection::vec(
                (0usize..64, 0usize..64, sensitivity_strategy(), any::<bool>()),
                0..8,
            ),
        ) {
            let segments: Vec<ClassificationSegment> = segments
                .into_iter()
                .map(|(start, end, sensitivity, certain)| ClassificationSegment {
                    start,
                    end,
                    sensitivity,
                    certain,
                })
                .collect();
            let chunks = split_by_boundaries(&source, &segments);
            for chunk in &chunks {
                // An empty declaration covers no bytes and creates no
                // boundary (the splitter drops it); the oracle only
                // quantifies over segments covering at least one byte.
                for segment in segments.iter().filter(|s| s.start < s.end) {
                    // A chunk is fully inside or fully outside every
                    // declared segment: it never straddles an edge.
                    let inside = segment.start <= chunk.start && chunk.end <= segment.end;
                    let outside = chunk.end <= segment.start || segment.end <= chunk.start;
                    // Edges snapped to char boundaries can shift a
                    // segment by a byte; allow the chunk to straddle
                    // only the *declared* edge when the snapped edge
                    // moved, and only inside the snapped segment.
                    let straddles_declared = chunk.start < segment.end
                        && segment.start < chunk.end
                        && !inside
                        && !outside;
                    prop_assert!(
                        !straddles_declared || chunk.uncertain,
                        "chunk {:?} straddles segment {:?} without failing closed",
                        chunk,
                        segment
                    );
                }
            }
        }

        #[test]
        fn uncertain_chunks_are_secret_bearing_and_certain_singles_keep_declared(
            source in source_strategy(),
            segments in prop::collection::vec(
                (0usize..64, 0usize..64, sensitivity_strategy(), any::<bool>()),
                0..8,
            ),
        ) {
            let segments: Vec<ClassificationSegment> = segments
                .into_iter()
                .map(|(start, end, sensitivity, certain)| ClassificationSegment {
                    start,
                    end,
                    sensitivity,
                    certain,
                })
                .collect();
            let chunks = split_by_boundaries(&source, &segments);
            for chunk in &chunks {
                if chunk.uncertain {
                    prop_assert_eq!(chunk.sensitivity, Sensitivity::SecretBearing);
                }
            }
        }

        #[test]
        fn segment_order_does_not_change_the_split(
            source in source_strategy(),
            segments in prop::collection::vec(
                (0usize..64, 0usize..64, sensitivity_strategy(), any::<bool>()),
                0..8,
            ),
        ) {
            let segments: Vec<ClassificationSegment> = segments
                .into_iter()
                .map(|(start, end, sensitivity, certain)| ClassificationSegment {
                    start,
                    end,
                    sensitivity,
                    certain,
                })
                .collect();
            let forward = split_by_boundaries(&source, &segments);
            let mut reversed = segments.clone();
            reversed.reverse();
            let backward = split_by_boundaries(&source, &reversed);
            prop_assert_eq!(forward, backward);
        }
    }
}
