//! §4.1 join/reconstruction assembler and §4.3 cold/retired rendering
//! (design record
//! `doc/2026-11-10T22-56-00Z_design_atlas_delta_detection_join/`,
//! developing-today/code#9979, the #6028 follow-on (d)).
//!
//! The span table (#6035 shape) records *occurrences*: a row
//! `(span_id, version_id, start, end)` says "span `span_id` appears in
//! version `version_id` at these byte offsets". Spans are indexes into
//! immutable version text, never replacements for it: assembly always
//! emits `V`'s own bytes, and the spans only decide what is highlighted
//! and what is a hit.
//!
//! §4.1 assembly of one paragraph `P` of version `V`:
//!
//! 1. collect every occurrence in `V` whose range intersects `P`
//!    (cross-version rendering supplies `V`'s occurrence rows for the
//!    hit's `span_id` — mapping is by occurrence, never offset
//!    arithmetic on the source version);
//! 2. sort by `start`, walk with a coverage cursor `c = P.start`, fill
//!    each gap from `V`'s text, and emit the clipped intersection
//!    `max(S.start, c) .. min(S.end, P.end)` — intersecting spans are
//!    clipped to the paragraph extent *before* emission, so assembly
//!    can never write past `P.end`;
//! 3. fill the tail `c .. P.end` from version text.
//!
//! Invariant J1 (§4.2): the assembled bytes are exactly
//! `V[P.start .. P.end)`, byte for byte, no byte twice, none missing,
//! none invented, however the spans overlap. The property suite lives in
//! `tests/join.rs`; the oracle is the version text itself.
//!
//! §4.3: cold versions assemble from their own fetched text — coldness
//! is latency, never correctness, and a missing fetch is an explicit
//! [`RenderResult::ColdUnavailable`], never a neighbouring version's
//! text. Retired versions render a [`RenderResult::Tombstone`] from the
//! span's stored snippet, with the version id and retirement marker; a
//! live successor occurrence is *offered* separately and never
//! substituted. Orphaned spans behave as retired when every version
//! referencing them is retired, and as normal spans while any live
//! version references them.

use std::ops::Range;

use crate::error::{Error, Result};

/// A paragraph extent in one version's byte offsets. Paragraph
/// boundaries are a property of the version text (#6037's cutter
/// records them at ingest); a span crossing a boundary contributes to
/// each paragraph it touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paragraph {
    pub start: usize,
    pub end: usize,
}

impl Paragraph {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The whole text as one paragraph (the common single-block case).
    pub fn whole(text: &[u8]) -> Self {
        Self {
            start: 0,
            end: text.len(),
        }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// One span-table occurrence row: span `span_id` appears in
/// `version_id` at bytes `start .. end` (#6035 shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanOccurrence {
    pub span_id: u64,
    pub version_id: u64,
    pub start: usize,
    pub end: usize,
}

impl SpanOccurrence {
    pub fn new(span_id: u64, version_id: u64, start: usize, end: usize) -> Self {
        Self {
            span_id,
            version_id,
            start,
            end,
        }
    }

    fn intersects(&self, paragraph: Paragraph) -> bool {
        self.start < paragraph.end && self.end > paragraph.start
    }
}

/// A span row: the embedded content plus its lifecycle state. The
/// stored snippet is captured at ingest exactly for the §4.3 tombstone
/// render; `tombstone` is the retention record's retirement marker on
/// the span itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub id: u64,
    pub snippet: Vec<u8>,
    pub tombstone: bool,
}

impl Span {
    pub fn live(id: u64, snippet: impl Into<Vec<u8>>) -> Self {
        Self {
            id,
            snippet: snippet.into(),
            tombstone: false,
        }
    }

    pub fn tombstoned(id: u64, snippet: impl Into<Vec<u8>>) -> Self {
        Self {
            id,
            snippet: snippet.into(),
            tombstone: true,
        }
    }
}

/// In-memory view of the #6035 span table: spans plus occurrence rows.
#[derive(Debug, Default, Clone)]
pub struct SpanTable {
    spans: Vec<Span>,
    occurrences: Vec<SpanOccurrence>,
}

impl SpanTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_span(&mut self, span: Span) {
        self.spans.push(span);
    }

    pub fn add_occurrence(&mut self, occurrence: SpanOccurrence) {
        self.occurrences.push(occurrence);
    }

    /// Convenience: one span row plus its occurrence in one version.
    pub fn land(&mut self, span: Span, version_id: u64, start: usize, end: usize) {
        let id = span.id;
        self.add_span(span);
        self.add_occurrence(SpanOccurrence::new(id, version_id, start, end));
    }

    pub fn span(&self, span_id: u64) -> Option<&Span> {
        self.spans.iter().find(|s| s.id == span_id)
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    pub fn occurrences(&self) -> &[SpanOccurrence] {
        &self.occurrences
    }

    /// Every occurrence of `version_id`, in table order.
    pub fn occurrences_in(&self, version_id: u64) -> impl Iterator<Item = &SpanOccurrence> {
        self.occurrences
            .iter()
            .filter(move |o| o.version_id == version_id)
    }

    /// The occurrence of `span_id` in `version_id`, if the span is
    /// referenced there. This — and only this — is how a hit is mapped
    /// onto another version: no offset arithmetic across versions.
    pub fn occurrence(&self, span_id: u64, version_id: u64) -> Option<&SpanOccurrence> {
        self.occurrences
            .iter()
            .find(|o| o.span_id == span_id && o.version_id == version_id)
    }

    /// Versions that reference `span_id`.
    pub fn versions_referencing(&self, span_id: u64) -> impl Iterator<Item = u64> + '_ {
        self.occurrences
            .iter()
            .filter(move |o| o.span_id == span_id)
            .map(|o| o.version_id)
    }

    /// An orphan is a span referenced by no live version (§3). Orphans
    /// render as retired-version spans; live references render normally.
    pub fn is_orphan(&self, span_id: u64, catalog: &VersionCatalog) -> bool {
        self.versions_referencing(span_id).all(|version_id| {
            catalog
                .get(version_id)
                .is_none_or(|v| v.state == VersionState::Retired)
        })
    }
}

/// Residency of a version (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionState {
    /// The landed object itself; its text is resident.
    Live,
    /// Retained but not resident: text is fetched from cold storage.
    Cold,
    /// Past retention: renders tombstones only, never paragraphs.
    Retired,
}

/// One version record. `text` is the version's *own* text: always
/// present for a [`Live`](VersionState::Live) version, present for a
/// cold version once fetched, and absent for a retired version (and for
/// a cold version whose fetch has not happened yet — assembly then
/// fails explicitly rather than borrowing a neighbour's text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRecord {
    pub id: u64,
    pub state: VersionState,
    pub text: Option<Vec<u8>>,
}

impl VersionRecord {
    pub fn live(id: u64, text: impl Into<Vec<u8>>) -> Self {
        Self {
            id,
            state: VersionState::Live,
            text: Some(text.into()),
        }
    }

    pub fn cold(id: u64, fetched_text: Option<Vec<u8>>) -> Self {
        Self {
            id,
            state: VersionState::Cold,
            text: fetched_text,
        }
    }

    pub fn retired(id: u64) -> Self {
        Self {
            id,
            state: VersionState::Retired,
            text: None,
        }
    }
}

/// The versions a render may reference, by id.
#[derive(Debug, Default, Clone)]
pub struct VersionCatalog {
    versions: Vec<VersionRecord>,
}

impl VersionCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, record: VersionRecord) {
        self.versions.push(record);
    }

    pub fn get(&self, version_id: u64) -> Option<&VersionRecord> {
        self.versions.iter().find(|v| v.id == version_id)
    }
}

/// The output of §4.1 assembly: the paragraph bytes plus the
/// paragraph-relative byte ranges the spans highlight.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assembled {
    /// Exactly `V[P.start .. P.end)` (invariant J1).
    pub bytes: Vec<u8>,
    /// Highlight ranges, paragraph-relative, sorted, clipped to
    /// `0 .. P.len()`, with overlaps and adjacency merged. The union is
    /// a subset of the paragraph: highlights never point outside it.
    pub highlights: Vec<Range<usize>>,
}

impl Assembled {
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }

    pub fn is_highlighted(&self, offset: usize) -> bool {
        self.highlights.iter().any(|range| range.contains(&offset))
    }
}

/// Assemble one paragraph from a version's own text and its
/// occurrence rows (§4.1).
///
/// `occurrences` may contain rows for other versions and rows outside
/// `paragraph`; non-intersecting rows are ignored. Spans are clipped to
/// the paragraph extent before emission.
pub fn assemble(
    version_text: &[u8],
    paragraph: Paragraph,
    occurrences: &[SpanOccurrence],
) -> Result<Assembled> {
    if paragraph.start > paragraph.end {
        return Err(Error::fatal(format!(
            "paragraph extent is inverted: {}..{}",
            paragraph.start, paragraph.end
        )));
    }
    if paragraph.end > version_text.len() {
        return Err(Error::fatal(format!(
            "paragraph extent {}..{} exceeds version text length {}",
            paragraph.start,
            paragraph.end,
            version_text.len()
        )));
    }

    let mut intersecting: Vec<&SpanOccurrence> = occurrences
        .iter()
        .filter(|o| o.intersects(paragraph))
        .collect();
    // Deterministic walk: by start, longest first on ties, so nested
    // spans resolve identically on every run.
    intersecting.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));

    let mut bytes = Vec::with_capacity(paragraph.len());
    let mut highlights: Vec<Range<usize>> = Vec::new();
    let mut cursor = paragraph.start;

    for occurrence in intersecting {
        let span_start = occurrence.start.max(paragraph.start);
        let span_end = occurrence.end.min(paragraph.end);
        let emit_start = span_start.max(cursor);
        if emit_start >= span_end {
            // Entirely behind the cursor (overlap already emitted) or
            // clipped away by the paragraph extent.
            continue;
        }
        if emit_start > cursor {
            // Gap: the version text is the ground truth.
            bytes.extend_from_slice(&version_text[cursor..emit_start]);
        }
        bytes.extend_from_slice(&version_text[emit_start..span_end]);
        push_highlight(
            &mut highlights,
            emit_start - paragraph.start..span_end - paragraph.start,
        );
        cursor = span_end;
    }

    if cursor < paragraph.end {
        bytes.extend_from_slice(&version_text[cursor..paragraph.end]);
    }

    // J1 as an assertion, not an aspiration: any deviation is a bug in
    // this function, not in the caller.
    let expected = &version_text[paragraph.start..paragraph.end];
    if bytes != expected {
        return Err(Error::backend(format!(
            "assembly violated J1 for paragraph {}..{}: emitted {} bytes, expected {}",
            paragraph.start,
            paragraph.end,
            bytes.len(),
            expected.len()
        )));
    }

    Ok(Assembled { bytes, highlights })
}

/// Assemble `paragraph` of `version_id` from the table and the
/// version's text.
pub fn assemble_version(
    table: &SpanTable,
    version_id: u64,
    version_text: &[u8],
    paragraph: Paragraph,
) -> Result<Assembled> {
    let occurrences: Vec<SpanOccurrence> = table.occurrences_in(version_id).copied().collect();
    assemble(version_text, paragraph, &occurrences)
}

fn push_highlight(highlights: &mut Vec<Range<usize>>, range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = highlights.last_mut() {
        if range.start <= last.end {
            last.end = last.end.max(range.end);
            return;
        }
    }
    highlights.push(range);
}

/// A search hit: the span that matched and the version its span
/// belongs to. A hit returned for an older version may be rendered
/// against a newer version when that version carries an occurrence of
/// the same span (cross-version rendering, §4.1 step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub span_id: u64,
    pub version_id: u64,
}

/// What to render: one paragraph of one target version, for one hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRequest {
    pub hit: Hit,
    /// The version being rendered against (the hit's own version, or a
    /// version that carries an occurrence of the hit's span).
    pub target_version: u64,
    pub paragraph: Paragraph,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderResult {
    /// A normal paragraph render (§4.1) of a live or cold-with-text
    /// version. `matched` is the hit span's clipped range, relative to
    /// the paragraph, when the target version carries it.
    Paragraph {
        version_id: u64,
        state: VersionState,
        assembled: Assembled,
        matched: Option<Range<usize>>,
    },
    /// §4.3 tombstone: the hit resolved only through a retired version.
    /// The stored snippet is shown with the version id and retirement
    /// marker; a live successor occurrence is offered separately.
    Tombstone {
        version_id: u64,
        span_id: u64,
        snippet: Vec<u8>,
        retired: bool,
        successor: Option<Successor>,
    },
    /// A cold version whose text has not been fetched. Explicit: the
    /// renderer must never substitute another version's text.
    ColdUnavailable { version_id: u64 },
    /// The span has no occurrence row in the target version — mapping
    /// across versions is by occurrence only, never offset arithmetic.
    NoOccurrence { span_id: u64, version_id: u64 },
    /// The target version is not in the catalog at all.
    UnknownVersion { version_id: u64 },
    /// The span row (and therefore its stored snippet) is missing.
    UnknownSpan { span_id: u64 },
}

/// A live occurrence of the hit's span in a successor version. Offered
/// with the tombstone, always labelled by version — never substituted
/// for the retired paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Successor {
    pub version_id: u64,
    pub start: usize,
    pub end: usize,
}

/// Render one hit request under §4.1/§4.3.
pub fn render(
    table: &SpanTable,
    catalog: &VersionCatalog,
    request: &RenderRequest,
) -> Result<RenderResult> {
    let target_version = request.target_version;
    let version = match catalog.get(target_version) {
        Some(version) => version,
        None => {
            return Ok(RenderResult::UnknownVersion {
                version_id: target_version,
            });
        }
    };

    let span_id = request.hit.span_id;
    let occurrence = match table.occurrence(span_id, target_version) {
        Some(occurrence) => occurrence,
        None => {
            return Ok(RenderResult::NoOccurrence {
                span_id,
                version_id: target_version,
            });
        }
    };

    if version.state == VersionState::Retired {
        let span = match table.span(span_id) {
            Some(span) => span,
            None => return Ok(RenderResult::UnknownSpan { span_id }),
        };
        let successor = find_successor(table, catalog, span_id, target_version);
        return Ok(RenderResult::Tombstone {
            version_id: target_version,
            span_id,
            snippet: span.snippet.clone(),
            retired: true,
            successor,
        });
    }

    let text = match version.text.as_deref() {
        Some(text) => text,
        None => {
            // Live versions always carry their text; reaching here means
            // a cold version whose fetch has not happened. Never borrow a
            // neighbouring version's text (that would be substitution).
            return Ok(RenderResult::ColdUnavailable {
                version_id: target_version,
            });
        }
    };

    let assembled = assemble_version(table, target_version, text, request.paragraph)?;
    let matched = clip_to_paragraph(occurrence.start, occurrence.end, request.paragraph);
    Ok(RenderResult::Paragraph {
        version_id: target_version,
        state: version.state,
        assembled,
        matched,
    })
}

fn clip_to_paragraph(start: usize, end: usize, paragraph: Paragraph) -> Option<Range<usize>> {
    let start = start.max(paragraph.start);
    let end = end.min(paragraph.end);
    if start >= end {
        return None;
    }
    Some(start - paragraph.start..end - paragraph.start)
}

fn find_successor(
    table: &SpanTable,
    catalog: &VersionCatalog,
    span_id: u64,
    retired_version: u64,
) -> Option<Successor> {
    table
        .occurrences()
        .iter()
        .filter(|o| o.span_id == span_id && o.version_id != retired_version)
        .find(|o| {
            catalog
                .get(o.version_id)
                .is_some_and(|v| v.state != VersionState::Retired)
        })
        .map(|o| Successor {
            version_id: o.version_id,
            start: o.start,
            end: o.end,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occ(span_id: u64, version: u64, start: usize, end: usize) -> SpanOccurrence {
        SpanOccurrence::new(span_id, version, start, end)
    }

    #[test]
    fn worked_example_17_241_case() {
        // §5: V2 = shared head (17 B) | new literal (225 B) | shared
        // tail. Three spans supply the paragraph; the render equals V2.
        let head = b"HEADER-17-BYTES!\n"; // 17 bytes
        assert_eq!(head.len(), 17);
        let new = vec![b'x'; 225];
        let tail = b"\ntail paragraph bytes shared with V1\n";
        let mut v2 = Vec::new();
        v2.extend_from_slice(head);
        v2.extend_from_slice(&new);
        v2.extend_from_slice(tail);

        let mut table = SpanTable::new();
        table.land(Span::live(1, head.to_vec()), 1, 0, head.len());
        table.add_occurrence(occ(1, 2, 0, head.len()));
        table.land(
            Span::live(2, new.clone()),
            2,
            head.len(),
            head.len() + new.len(),
        );
        let tail_start = head.len() + new.len();
        table.land(Span::live(3, tail.to_vec()), 1, 40, 40 + tail.len());
        table.add_occurrence(occ(3, 2, tail_start, v2.len()));

        let paragraph = Paragraph::whole(&v2);
        let assembled = assemble_version(&table, 2, &v2, paragraph).unwrap();
        assert_eq!(assembled.bytes, v2);
        // The three spans are contiguous, so their merged highlight is
        // the whole paragraph — and the bytes are exact: head, new
        // literal and tail each appear exactly once even though three
        // spans supplied them (the J1 non-doubling statement).
        assert_eq!(assembled.highlights, vec![0..v2.len()]);
        assert_eq!(assembled.bytes.iter().filter(|b| **b == b'x').count(), 225);
    }

    #[test]
    fn nested_and_duplicate_spans_do_not_double() {
        let text = b"abcdefghij";
        let occurrences = vec![
            occ(1, 7, 0, 10),
            occ(2, 7, 2, 5),
            occ(3, 7, 2, 5),
            occ(4, 7, 0, 3),
            occ(5, 7, 8, 10),
        ];
        let assembled = assemble(text, Paragraph::whole(text), &occurrences).unwrap();
        assert_eq!(assembled.bytes, text);
        assert_eq!(assembled.highlights, vec![0..10]);
    }

    #[test]
    fn spans_crossing_paragraph_boundaries_are_clipped() {
        let text = b"para one\npara two\npara three";
        let p2 = Paragraph::new(9, 18);
        let occurrences = vec![
            occ(1, 7, 0, 13),  // crosses into p2 from the left
            occ(2, 7, 15, 26), // crosses out of p2 to the right
        ];
        let assembled = assemble(text, p2, &occurrences).unwrap();
        assert_eq!(assembled.bytes, &text[p2.start..p2.end]);
        // Clipped: nothing outside the paragraph extent is emitted or
        // highlighted; the uncovered gap in the middle is filled from
        // version text (not from either span).
        assert_eq!(assembled.highlights, vec![0..4, 6..9]);
        assert_eq!(&assembled.bytes[4..6], &text[13..15]);
        for range in &assembled.highlights {
            assert!(range.end <= p2.len());
        }
    }

    #[test]
    fn gap_fill_comes_from_version_text() {
        let text = b"0123456789";
        let occurrences = vec![occ(1, 1, 2, 4), occ(2, 1, 7, 9)];
        let assembled = assemble(text, Paragraph::whole(text), &occurrences).unwrap();
        assert_eq!(assembled.bytes, text);
        assert_eq!(assembled.highlights, vec![2..4, 7..9]);
    }

    #[test]
    fn inverted_and_out_of_range_paragraphs_error() {
        let text = b"abc";
        assert!(assemble(text, Paragraph::new(2, 1), &[]).is_err());
        assert!(assemble(text, Paragraph::new(0, 4), &[]).is_err());
    }

    #[test]
    fn empty_paragraph_assembles_empty() {
        let text = b"abc";
        let assembled = assemble(text, Paragraph::new(1, 1), &[occ(1, 1, 0, 3)]).unwrap();
        assert!(assembled.bytes.is_empty());
        assert!(assembled.highlights.is_empty());
    }

    #[test]
    fn cross_version_rendering_maps_by_occurrence_not_offsets() {
        let v1 = b"AAAAcontent-that-matches";
        let v2 = b"BBBBBBBBcontent-that-matches";
        let mut table = SpanTable::new();
        table.land(
            Span::live(1, b"content-that-matches".to_vec()),
            1,
            4,
            v1.len(),
        );
        table.add_occurrence(occ(1, 2, 8, v2.len()));

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
        let rendered = render(&table, &catalog, &request).unwrap();
        let RenderResult::Paragraph {
            assembled, matched, ..
        } = rendered
        else {
            panic!("expected a paragraph render, got {rendered:?}");
        };
        assert_eq!(assembled.bytes, v2);
        // The occurrence row's target offsets — 8.., not the source's 4..
        assert_eq!(matched, Some(8..v2.len()));

        // No occurrence in the target version: never offset arithmetic.
        catalog.insert(VersionRecord::live(3, v2.to_vec()));
        let request = RenderRequest {
            target_version: 3,
            ..request
        };
        let rendered = render(&table, &catalog, &request).unwrap();
        assert_eq!(
            rendered,
            RenderResult::NoOccurrence {
                span_id: 1,
                version_id: 3,
            }
        );
    }

    #[test]
    fn cold_version_assembles_from_its_own_text() {
        let text = b"cold paragraph text";
        let mut table = SpanTable::new();
        table.land(Span::live(1, b"paragraph".to_vec()), 5, 5, 14);
        let mut catalog = VersionCatalog::new();
        catalog.insert(VersionRecord::cold(5, Some(text.to_vec())));
        // A neighbouring live version whose text must never be used.
        catalog.insert(VersionRecord::live(6, b"WRONG neighbour text".to_vec()));

        let request = RenderRequest {
            hit: Hit {
                span_id: 1,
                version_id: 5,
            },
            target_version: 5,
            paragraph: Paragraph::whole(text),
        };
        let rendered = render(&table, &catalog, &request).unwrap();
        let RenderResult::Paragraph {
            assembled, state, ..
        } = rendered
        else {
            panic!("expected paragraph render");
        };
        assert_eq!(state, VersionState::Cold);
        assert_eq!(assembled.bytes, text);
    }

    #[test]
    fn unfetched_cold_version_is_explicit_not_substituted() {
        let mut table = SpanTable::new();
        table.land(Span::live(1, b"span".to_vec()), 5, 0, 4);
        let mut catalog = VersionCatalog::new();
        catalog.insert(VersionRecord::cold(5, None));
        catalog.insert(VersionRecord::live(6, b"neighbour text".to_vec()));

        let request = RenderRequest {
            hit: Hit {
                span_id: 1,
                version_id: 5,
            },
            target_version: 5,
            paragraph: Paragraph::new(0, 4),
        };
        let rendered = render(&table, &catalog, &request).unwrap();
        assert_eq!(rendered, RenderResult::ColdUnavailable { version_id: 5 });
    }

    #[test]
    fn retired_version_renders_tombstone_from_stored_snippet() {
        let snippet = b"the matched span at ingest".to_vec();
        let mut table = SpanTable::new();
        table.land(Span::tombstoned(1, snippet.clone()), 5, 0, snippet.len());
        let mut catalog = VersionCatalog::new();
        catalog.insert(VersionRecord::retired(5));
        // Even with a live successor carrying the same span, the retired
        // version's render is its own tombstone.
        catalog.insert(VersionRecord::live(6, b"successor text".to_vec()));
        table.add_occurrence(occ(1, 6, 3, 3 + snippet.len()));

        let request = RenderRequest {
            hit: Hit {
                span_id: 1,
                version_id: 5,
            },
            target_version: 5,
            paragraph: Paragraph::new(0, snippet.len()),
        };
        let rendered = render(&table, &catalog, &request).unwrap();
        let RenderResult::Tombstone {
            snippet: rendered_snippet,
            retired,
            successor,
            version_id,
            ..
        } = rendered
        else {
            panic!("expected tombstone render");
        };
        assert_eq!(version_id, 5);
        assert!(retired);
        assert_eq!(rendered_snippet, snippet);
        // The successor is offered, labelled — never substituted.
        assert_eq!(
            successor,
            Some(Successor {
                version_id: 6,
                start: 3,
                end: 3 + snippet.len(),
            })
        );
    }

    #[test]
    fn orphaned_span_behaves_as_retired_only() {
        let snippet = b"orphan snippet".to_vec();
        let mut table = SpanTable::new();
        table.land(Span::tombstoned(1, snippet.clone()), 5, 0, snippet.len());
        let mut catalog = VersionCatalog::new();
        catalog.insert(VersionRecord::retired(5));

        assert!(table.is_orphan(1, &catalog));

        // While a live version references it, it is not an orphan and
        // renders as a normal paragraph.
        let live_text = b"x orphan snippet y".to_vec();
        table.add_occurrence(occ(1, 6, 2, 2 + snippet.len()));
        catalog.insert(VersionRecord::live(6, live_text.clone()));
        assert!(!table.is_orphan(1, &catalog));

        let request = RenderRequest {
            hit: Hit {
                span_id: 1,
                version_id: 6,
            },
            target_version: 6,
            paragraph: Paragraph::whole(&live_text),
        };
        let rendered = render(&table, &catalog, &request).unwrap();
        let RenderResult::Paragraph { assembled, .. } = rendered else {
            panic!("expected paragraph render for a live reference");
        };
        assert_eq!(assembled.bytes, live_text);
    }

    #[test]
    fn missing_span_row_reports_unknown_span() {
        let mut table = SpanTable::new();
        table.add_occurrence(occ(9, 5, 0, 3));
        let mut catalog = VersionCatalog::new();
        catalog.insert(VersionRecord::retired(5));
        let request = RenderRequest {
            hit: Hit {
                span_id: 9,
                version_id: 5,
            },
            target_version: 5,
            paragraph: Paragraph::new(0, 3),
        };
        assert_eq!(
            render(&table, &catalog, &request).unwrap(),
            RenderResult::UnknownSpan { span_id: 9 }
        );
    }

    #[test]
    fn unknown_version_reports_explicitly() {
        let request = RenderRequest {
            hit: Hit {
                span_id: 1,
                version_id: 42,
            },
            target_version: 42,
            paragraph: Paragraph::new(0, 1),
        };
        assert_eq!(
            render(&SpanTable::new(), &VersionCatalog::new(), &request).unwrap(),
            RenderResult::UnknownVersion { version_id: 42 }
        );
    }
}
