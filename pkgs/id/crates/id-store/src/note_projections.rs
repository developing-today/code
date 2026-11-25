//! Block-content projections for note revisions (#8218 slice 2, design #5839 D3/D4/D5).
//!
//! The substrate (slice 1, [`crate::note_blocks`]) defines the block
//! schema and anchor rules. This module defines the three projections
//! the #5839 design derives from that substrate, as pure functions over
//! caller-provided note data:
//!
//! * **Outline** (D3): folding, zooming, and hoisting over one note's
//!   block tree. A presentation mode, not a document type.
//! * **Journal** (D4): a date-ordered view over daily notes plus
//!   capture-append to today's note. The journal is derivable from
//!   date claims alone (#5839 invariant 4); there is no journal store.
//! * **Database views** (D5): a saved query plus a presentation over
//!   notes carrying typed properties. A database is derivable from its
//!   definition note plus the notes it queries (#5839 invariant 5);
//!   deleting it deletes no content.
//!
//! Read authority (#5839 §6, ruling #5678) is enforced at the note —
//! the only granularity this model has. Every multi-note projection
//! takes a caller-provided `can_read` predicate and never exposes a
//! note (or its existence metadata, including through transclusion)
//! to a reader the predicate denies. The #5678 resolution order
//! (object → user default → server config → public) is
//! [`crate::read_authority::NoteReader`], which callers pass as that
//! predicate (#12751).
//!
//! Tests in this module name the #5839 design record
//! (`doc/2026-11-10T20-12-00Z_design_id_content_model/`) as their
//! oracle.

use std::collections::{BTreeMap, BTreeSet};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use crate::note_blocks::{AnchorResolution, Block, BlockAnchor, BlockId, BlockTree};

// ---------------------------------------------------------------------------
// Shared input shapes
// ---------------------------------------------------------------------------

/// A calendar date carried as a note-level claim. Daily notes are
/// ordinary notes; this claim is what makes a note a *daily* note
/// (#5839 D4). Year/month/day are validated on [`CalendarDate::check`];
/// no date library is needed for ordering (lexicographic works).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CalendarDate {
    /// Proleptic Gregorian year.
    pub year: u16,
    /// Month, 1..=12.
    pub month: u8,
    /// Day of month, 1..=31 (month-length aware).
    pub day: u8,
}

/// Why a date claim was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DateError {
    /// Month is not in 1..=12.
    #[error("month {0} is not in 1..=12")]
    Month(u8),
    /// Day is not valid for the month (leap-year aware for February).
    #[error("day {0} is not valid for {1}-{2:02}")]
    Day(u8, u16, u8),
}

impl CalendarDate {
    /// Checks month/day validity (leap-year aware for February).
    ///
    /// # Errors
    ///
    /// Returns [`DateError`] when the month or day is out of range.
    pub fn check(&self) -> Result<(), DateError> {
        if !(1..=12).contains(&self.month) {
            return Err(DateError::Month(self.month));
        }
        let max = match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                let leap = self.year % 4 == 0 && (self.year % 100 != 0 || self.year % 400 == 0);
                if leap { 29 } else { 28 }
            }
            _ => unreachable!("month range checked above"),
        };
        if self.day == 0 || self.day > max {
            return Err(DateError::Day(self.day, self.year, self.month));
        }
        Ok(())
    }
}

/// A typed property carried on a note (#5839 D5: columns are typed
/// properties with the same provenance discipline as #2048's other
/// claims).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteProperty {
    /// Property name.
    pub name: String,
    /// Property value.
    pub value: PropertyValue,
}

/// A typed property value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropertyValue {
    /// Text value.
    Text(String),
    /// Integer value.
    Integer(i64),
    /// Boolean flag.
    Flag(bool),
    /// Calendar date value.
    Date(CalendarDate),
}

/// One note revision's worth of projection input: identity plus the
/// content and claims a projection reads. Storage stays on the
/// caller's side; projections never fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedNote {
    /// The note's stable ID.
    pub note: Hash,
    /// The revision this input was read at.
    pub revision: Hash,
    /// The revision's block tree.
    pub tree: BlockTree,
    /// Date claims carried by the note (empty for non-daily notes).
    pub dates: Vec<CalendarDate>,
    /// Typed properties carried by the note.
    pub properties: Vec<NoteProperty>,
}

/// A read-authority predicate over note IDs: `true` means the viewer
/// may read the note. The #5678 resolution order is
/// [`crate::read_authority::NoteReader::may_read`]; pass
/// `&|n: &Hash| reader.may_read(n)`.
pub type CanRead = dyn Fn(&Hash) -> bool;

// ---------------------------------------------------------------------------
// Outline (D3)
// ---------------------------------------------------------------------------

/// One visible row of an outline view: a block plus its depth under
/// the view root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineEntry<'a> {
    /// The block at this row.
    pub block: &'a Block,
    /// Depth under the view root (0 for top-level rows).
    pub depth: usize,
}

/// Lists a tree's blocks in document order with depths.
///
/// This is the unfolded outline: every block appears exactly once.
#[must_use]
pub fn outline_entries(tree: &BlockTree) -> Vec<OutlineEntry<'_>> {
    let mut out = Vec::new();
    fn walk<'a>(block: &'a Block, depth: usize, out: &mut Vec<OutlineEntry<'a>>) {
        out.push(OutlineEntry { block, depth });
        for child in &block.children {
            walk(child, depth + 1, out);
        }
    }
    for block in &tree.blocks {
        walk(block, 0, &mut out);
    }
    out
}

/// Folds a tree: rows under a collapsed block are hidden. `collapsed`
/// holds the IDs of folded blocks; a folded block itself stays
/// visible, its descendants do not.
#[must_use]
pub fn folded_entries<'a>(
    tree: &'a BlockTree,
    collapsed: &BTreeSet<BlockId>,
) -> Vec<OutlineEntry<'a>> {
    let mut out = Vec::new();
    fn walk<'a>(
        block: &'a Block,
        depth: usize,
        hidden: bool,
        collapsed: &BTreeSet<BlockId>,
        out: &mut Vec<OutlineEntry<'a>>,
    ) {
        if hidden {
            return;
        }
        out.push(OutlineEntry { block, depth });
        let hide_children = collapsed.contains(&block.id);
        for child in &block.children {
            walk(child, depth + 1, hidden || hide_children, collapsed, out);
        }
    }
    for block in &tree.blocks {
        walk(block, 0, false, collapsed, &mut out);
    }
    out
}

/// Zooms (hoists) the view to a block: the block becomes the view
/// root and only its subtree is listed. Returns `None` when the ID
/// is absent — hoisting never lands on a different block (#5839
/// invariant 2, applied to focus).
#[must_use]
pub fn hoisted_entries<'a>(tree: &'a BlockTree, focus: &BlockId) -> Option<Vec<OutlineEntry<'a>>> {
    let root = tree.find_block(focus)?;
    let mut out = Vec::new();
    fn walk<'a>(block: &'a Block, depth: usize, out: &mut Vec<OutlineEntry<'a>>) {
        out.push(OutlineEntry { block, depth });
        for child in &block.children {
            walk(child, depth + 1, out);
        }
    }
    walk(root, 0, &mut out);
    Some(out)
}

// ---------------------------------------------------------------------------
// Journal (D4)
// ---------------------------------------------------------------------------

/// One row of a journal view: a daily note in date order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalRow<'a> {
    /// The daily note.
    pub note: &'a ProjectedNote,
    /// The date claim this row is ordered by. A note carrying
    /// several date claims appears once per claim.
    pub date: CalendarDate,
}

/// Builds the journal: every daily note the viewer may read, ordered
/// by date (oldest first), notes unreadable under `can_read`
/// omitted entirely — including their existence metadata (#5839
/// invariant 6).
#[must_use]
pub fn journal_view<'a>(notes: &'a [ProjectedNote], can_read: &CanRead) -> Vec<JournalRow<'a>> {
    let mut rows: Vec<JournalRow<'a>> = notes
        .iter()
        .filter(|n| can_read(&n.note))
        .flat_map(|n| {
            n.dates
                .iter()
                .map(move |d| JournalRow { note: n, date: *d })
        })
        .collect();
    rows.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.note.note.cmp(&b.note.note))
    });
    rows
}

/// Where a capture for `date` goes: the existing daily note, or a
/// signal to create it from the date template (#5839 D4: exactly one
/// daily note per owner+date; contested claims resolve by #2048's
/// name-claim rules, not by merging here).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureTarget<'a> {
    /// Append to this existing daily note.
    Existing {
        /// The daily note to append to.
        note: &'a ProjectedNote,
    },
    /// No daily note carries this date: create it from the template,
    /// then append. The first capture wins; a second note claiming
    /// the same date is a contested claim for #2048's rules.
    Create {
        /// The date to create the daily note for.
        date: CalendarDate,
    },
}

/// Resolves a capture's target among the notes the viewer may read.
/// Unreadable notes never satisfy a capture.
#[must_use]
pub fn capture_target<'a>(
    notes: &'a [ProjectedNote],
    date: CalendarDate,
    can_read: &CanRead,
) -> CaptureTarget<'a> {
    for note in notes {
        if note.dates.contains(&date) && can_read(&note.note) {
            return CaptureTarget::Existing { note };
        }
    }
    CaptureTarget::Create { date }
}

// ---------------------------------------------------------------------------
// Database views (D5)
// ---------------------------------------------------------------------------

/// A predicate over one typed property value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropertyFilter {
    /// Property present with this exact value.
    Equals(PropertyValue),
    /// Text property contains this substring (false for non-text).
    Contains(String),
    /// Integer property strictly greater than this bound (false for non-integer).
    GreaterThan(i64),
    /// Property present with any value.
    Present,
}

impl PropertyFilter {
    /// Evaluates the filter against one value.
    #[must_use]
    pub fn matches(&self, value: &PropertyValue) -> bool {
        match self {
            PropertyFilter::Equals(want) => value == want,
            PropertyFilter::Contains(needle) => match value {
                PropertyValue::Text(haystack) => haystack.contains(needle),
                _ => false,
            },
            PropertyFilter::GreaterThan(bound) => match value {
                PropertyValue::Integer(n) => n > bound,
                _ => false,
            },
            PropertyFilter::Present => true,
        }
    }
}

/// Sort direction for one database column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    /// Ascending.
    Ascending,
    /// Descending.
    Descending,
}

/// A database definition: a saved query plus a presentation (#5839
/// D5). Serializable so the definition itself can live as a note.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseDefinition {
    /// Display name of the view.
    pub name: String,
    /// Every entry must match for a note to appear as a row.
    pub filters: BTreeMap<String, PropertyFilter>,
    /// Columns to project, in order (property names).
    pub columns: Vec<String>,
    /// Optional sort: property name plus direction.
    pub sort: Option<(String, SortDirection)>,
}

/// One database row: a note plus its projected cells. A missing
/// property projects as `None`, never as an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseRow<'a> {
    /// The row's note.
    pub note: &'a ProjectedNote,
    /// One cell per column in definition order.
    pub cells: Vec<Option<&'a PropertyValue>>,
}

/// Builds the database view: readable notes matching every filter,
/// with cells projected per the column list and rows sorted per the
/// sort spec. Unreadable notes are omitted entirely (#5839 invariant
/// 6). The definition note itself is just another note: deleting the
/// definition deletes no content (#5839 invariant 5).
#[must_use]
pub fn database_view<'a>(
    def: &DatabaseDefinition,
    notes: &'a [ProjectedNote],
    can_read: &CanRead,
) -> Vec<DatabaseRow<'a>> {
    let mut rows: Vec<DatabaseRow<'a>> = notes
        .iter()
        .filter(|n| can_read(&n.note))
        .filter(|n| {
            def.filters.iter().all(|(name, filter)| {
                n.properties
                    .iter()
                    .find(|p| &p.name == name)
                    .is_some_and(|p| filter.matches(&p.value))
            })
        })
        .map(|n| DatabaseRow {
            note: n,
            cells: def
                .columns
                .iter()
                .map(|col| {
                    n.properties
                        .iter()
                        .find(|p| &p.name == col)
                        .map(|p| &p.value)
                })
                .collect(),
        })
        .collect();
    if let Some((sort_col, dir)) = &def.sort {
        rows.sort_by(|a, b| {
            let key = |r: &DatabaseRow<'a>| {
                def.columns
                    .iter()
                    .position(|c| c == sort_col)
                    .and_then(|i| r.cells[i].clone())
            };
            let ord = compare_values(key(a), key(b));
            match dir {
                SortDirection::Ascending => ord,
                SortDirection::Descending => ord.reverse(),
            }
        });
    }
    rows
}

/// Orders two optional cells: absent sorts before present; values of
/// different types order by a fixed type rank (deterministic, not
/// meaningful across types).
fn compare_values(a: Option<&PropertyValue>, b: Option<&PropertyValue>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(x), Some(y)) => match (x, y) {
            (PropertyValue::Text(a), PropertyValue::Text(b)) => a.cmp(b),
            (PropertyValue::Integer(a), PropertyValue::Integer(b)) => a.cmp(b),
            (PropertyValue::Flag(a), PropertyValue::Flag(b)) => a.cmp(b),
            (PropertyValue::Date(a), PropertyValue::Date(b)) => a.cmp(b),
            (a, b) => type_rank(a).cmp(&type_rank(b)),
        },
    }
}

/// Fixed rank for cross-type cell ordering.
fn type_rank(v: &PropertyValue) -> u8 {
    match v {
        PropertyValue::Text(_) => 0,
        PropertyValue::Integer(_) => 1,
        PropertyValue::Flag(_) => 2,
        PropertyValue::Date(_) => 3,
    }
}

/// A cell edit: the note-property write a database cell edit means.
/// Editing a cell edits the underlying note's property (#5839 D5);
/// storage applies the returned edit, the view never writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyEdit {
    /// The note whose property changes.
    pub note: Hash,
    /// The property to set.
    pub name: String,
    /// The new value.
    pub value: PropertyValue,
}

/// Describes the cell edit for `(note, column, value)`: the note's
/// property write the database view implies. Returns `None` when the
/// viewer may not read the note.
#[must_use]
pub fn describe_cell_edit(
    note: &ProjectedNote,
    column: &str,
    value: PropertyValue,
    can_read: &CanRead,
) -> Option<PropertyEdit> {
    if !can_read(&note.note) {
        return None;
    }
    Some(PropertyEdit {
        note: note.note,
        name: column.to_owned(),
        value,
    })
}

// ---------------------------------------------------------------------------
// Transclusion rendering (#5839 §6)
// ---------------------------------------------------------------------------

/// What a transclusion block renders as for one viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransclusionRender<'a> {
    /// The source block's content. Borrowed from the caller's tree.
    Content {
        /// The rendered source block.
        block: &'a Block,
        /// Whether the anchor was pinned (exact) or head-resolved.
        pinned: bool,
    },
    /// Degraded to the reference's authored surface: the viewer may
    /// not read the source note, or the anchor did not resolve. Never
    /// the content.
    Unresolved,
}

/// Renders a transclusion block: resolves the anchor with
/// [`crate::note_blocks::resolve_anchor`] and returns the source
/// block only when the viewer may read the source note and the anchor
/// resolves to a block. Anything else degrades to the authored
/// surface in an unresolved state (#5839 §6, invariant 6).
#[must_use]
pub fn render_transclusion<'a>(
    target_note: &Hash,
    pinned_revision: Option<Hash>,
    target_block: Option<&BlockId>,
    pinned_tree: Option<&'a BlockTree>,
    head_tree: Option<&'a BlockTree>,
    source_note: &Hash,
    can_read: &CanRead,
) -> TransclusionRender<'a> {
    if !can_read(source_note) {
        return TransclusionRender::Unresolved;
    }
    let Some(block_id) = target_block else {
        // Whole-note transclusion: the note surface itself is the
        // authored reference; nothing further to resolve here.
        return TransclusionRender::Unresolved;
    };
    let anchor = BlockAnchor {
        note: *target_note,
        revision: pinned_revision,
        block: block_id.clone(),
    };
    match crate::note_blocks::resolve_anchor(&anchor, pinned_tree, head_tree) {
        AnchorResolution::Block { block, pinned } => {
            let tree = if pinned { pinned_tree } else { head_tree };
            match tree.and_then(|t| t.find_block(&block)) {
                Some(found) => TransclusionRender::Content {
                    block: found,
                    pinned,
                },
                None => TransclusionRender::Unresolved,
            }
        }
        AnchorResolution::DegradedToNote { .. } => TransclusionRender::Unresolved,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::note_blocks::{BlockKind, BlockTree};

    // Oracle: the #5839 design record
    // (doc/2026-11-10T20-12-00Z_design_id_content_model/).
    const ORACLE: &str = "doc/2026-11-10T20-12-00Z_design_id_content_model \
        (#5839 content model cluster decision record)";

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn id(s: &str) -> BlockId {
        BlockId(s.to_owned())
    }

    fn para(s: &str, text: &str) -> Block {
        Block {
            id: id(s),
            kind: BlockKind::Paragraph,
            text: text.to_owned(),
            children: Vec::new(),
        }
    }

    fn note(byte: u8, tree: BlockTree) -> ProjectedNote {
        ProjectedNote {
            note: hash(byte),
            revision: hash(byte.wrapping_add(100)),
            tree,
            dates: Vec::new(),
            properties: Vec::new(),
        }
    }

    fn allow_all(_: &Hash) -> bool {
        true
    }

    fn deny_all(_: &Hash) -> bool {
        false
    }

    // -- Outline (D3) ---------------------------------------------------

    #[test]
    fn oracle_is_the_5839_record() {
        assert!(ORACLE.contains("#5839"));
    }

    #[test]
    fn outline_lists_document_order_with_depth() {
        let tree = BlockTree {
            blocks: vec![Block {
                id: id("h"),
                kind: BlockKind::Heading { level: 1 },
                text: "t".to_owned(),
                children: vec![para("p", "b"), para("q", "c")],
            }],
        };
        let rows: Vec<(String, usize)> = outline_entries(&tree)
            .iter()
            .map(|e| (e.block.id.0.clone(), e.depth))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("h".to_owned(), 0),
                ("p".to_owned(), 1),
                ("q".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn fold_hides_descendants_keeps_row() {
        let tree = BlockTree {
            blocks: vec![Block {
                id: id("h"),
                kind: BlockKind::Heading { level: 1 },
                text: "t".to_owned(),
                children: vec![para("p", "b")],
            }],
        };
        let collapsed: BTreeSet<BlockId> = [id("h")].into_iter().collect();
        let rows: Vec<String> = folded_entries(&tree, &collapsed)
            .iter()
            .map(|e| e.block.id.0.clone())
            .collect();
        assert_eq!(rows, vec!["h".to_owned()]);
        // Unfolded, both rows show.
        let open = folded_entries(&tree, &BTreeSet::new());
        assert_eq!(open.len(), 2);
    }

    #[test]
    fn hoist_roots_at_focus_and_misses_get_none() {
        let tree = BlockTree {
            blocks: vec![Block {
                id: id("h"),
                kind: BlockKind::Heading { level: 1 },
                text: "t".to_owned(),
                children: vec![para("p", "b")],
            }],
        };
        let hoisted = hoisted_entries(&tree, &id("h")).unwrap();
        assert_eq!(hoisted.len(), 2);
        assert_eq!(hoisted[0].depth, 0);
        assert_eq!(hoisted[1].depth, 1);
        // Hoisting never lands on a different block (#5839 invariant 2).
        assert!(hoisted_entries(&tree, &id("absent")).is_none());
    }

    // -- Journal (D4) ---------------------------------------------------

    fn dated(byte: u8, y: u16, m: u8, d: u8) -> ProjectedNote {
        let mut n = note(byte, BlockTree::default());
        n.dates.push(CalendarDate {
            year: y,
            month: m,
            day: d,
        });
        n
    }

    #[test]
    fn calendar_dates_validate() {
        let ok = CalendarDate {
            year: 2026,
            month: 2,
            day: 28,
        };
        assert!(ok.check().is_ok());
        assert!(
            CalendarDate {
                year: 2026,
                month: 2,
                day: 29
            }
            .check()
            .is_err()
        );
        assert!(
            CalendarDate {
                year: 2024,
                month: 2,
                day: 29
            }
            .check()
            .is_ok()
        );
        assert!(
            CalendarDate {
                year: 2026,
                month: 13,
                day: 1
            }
            .check()
            .is_err()
        );
    }

    #[test]
    fn journal_orders_by_date_and_omits_unreadable() {
        let notes = vec![dated(1, 2026, 11, 23), dated(2, 2026, 11, 20)];
        // Journal is derivable from date claims alone (#5839 invariant 4).
        let rows = journal_view(&notes, &allow_all);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date.day, 20);
        assert_eq!(rows[1].date.day, 23);
        // No view exposes a note to a denied reader (#5839 invariant 6).
        assert!(journal_view(&notes, &deny_all).is_empty());
        let can_read_first = |h: &Hash| *h == hash(1);
        let rows = journal_view(&notes, &can_read_first);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].note.note, hash(1));
    }

    #[test]
    fn capture_targets_existing_or_create() {
        let notes = vec![dated(1, 2026, 11, 23)];
        let today = CalendarDate {
            year: 2026,
            month: 11,
            day: 23,
        };
        assert!(matches!(
            capture_target(&notes, today, &allow_all),
            CaptureTarget::Existing { .. }
        ));
        let other = CalendarDate {
            year: 2026,
            month: 11,
            day: 24,
        };
        assert!(matches!(
            capture_target(&notes, other, &allow_all),
            CaptureTarget::Create { .. }
        ));
        // An unreadable daily note never satisfies a capture.
        assert!(matches!(
            capture_target(&notes, today, &deny_all),
            CaptureTarget::Create { .. }
        ));
    }

    // -- Database views (D5) --------------------------------------------

    fn propertied(byte: u8, props: Vec<NoteProperty>) -> ProjectedNote {
        let mut n = note(byte, BlockTree::default());
        n.properties = props;
        n
    }

    fn prop(name: &str, value: PropertyValue) -> NoteProperty {
        NoteProperty {
            name: name.to_owned(),
            value,
        }
    }

    #[test]
    fn database_filters_projects_sorts_and_omits_unreadable() {
        let notes = vec![
            propertied(
                1,
                vec![
                    prop("status", PropertyValue::Text("done".to_owned())),
                    prop("prio", PropertyValue::Integer(2)),
                ],
            ),
            propertied(
                2,
                vec![
                    prop("status", PropertyValue::Text("done".to_owned())),
                    prop("prio", PropertyValue::Integer(5)),
                ],
            ),
            propertied(
                3,
                vec![prop("status", PropertyValue::Text("open".to_owned()))],
            ),
        ];
        let def = DatabaseDefinition {
            name: "done by prio".to_owned(),
            filters: [(
                "status".to_owned(),
                PropertyFilter::Equals(PropertyValue::Text("done".to_owned())),
            )]
            .into_iter()
            .collect(),
            columns: vec!["status".to_owned(), "prio".to_owned()],
            sort: Some(("prio".to_owned(), SortDirection::Descending)),
        };
        let rows = database_view(&def, &notes, &allow_all);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].note.note, hash(2));
        assert_eq!(rows[0].cells.len(), 2);
        // Missing property projects as None, never an error.
        let def_all = DatabaseDefinition {
            name: "all".to_owned(),
            filters: BTreeMap::new(),
            columns: vec!["prio".to_owned()],
            sort: None,
        };
        let rows = database_view(&def_all, &notes, &allow_all);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].cells, vec![None]);
        // Unreadable notes are omitted entirely (#5839 invariant 6).
        assert!(database_view(&def, &notes, &deny_all).is_empty());
    }

    #[test]
    fn database_definition_round_trips_as_a_note_payload() {
        // The definition is itself storable as a note (#5839 D5).
        let def = DatabaseDefinition {
            name: "v".to_owned(),
            filters: BTreeMap::new(),
            columns: vec!["a".to_owned()],
            sort: None,
        };
        let bytes = postcard::to_allocvec(&def).unwrap();
        let back: DatabaseDefinition = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(def, back);
    }

    #[test]
    fn cell_edit_describes_note_property_write() {
        let n = propertied(
            1,
            vec![prop("status", PropertyValue::Text("open".to_owned()))],
        );
        // Editing a cell edits the underlying note's property (#5839 D5).
        let edit = describe_cell_edit(
            &n,
            "status",
            PropertyValue::Text("done".to_owned()),
            &allow_all,
        )
        .unwrap();
        assert_eq!(
            edit,
            PropertyEdit {
                note: hash(1),
                name: "status".to_owned(),
                value: PropertyValue::Text("done".to_owned()),
            }
        );
        // No edit is described for an unreadable note.
        assert!(
            describe_cell_edit(&n, "status", PropertyValue::Text("x".to_owned()), &deny_all)
                .is_none()
        );
    }

    // -- Transclusion (§6) -----------------------------------------------

    #[test]
    fn transclusion_renders_only_for_authorized_resolved_readers() {
        let source = BlockTree {
            blocks: vec![para("s", "secret")],
        };
        // Authorized + resolved: content.
        let render = render_transclusion(
            &hash(9),
            None,
            Some(&id("s")),
            None,
            Some(&source),
            &hash(9),
            &allow_all,
        );
        assert!(matches!(
            render,
            TransclusionRender::Content { pinned: false, .. }
        ));
        // Denied reader: degraded, never content (#5839 invariant 6).
        let render = render_transclusion(
            &hash(9),
            None,
            Some(&id("s")),
            None,
            Some(&source),
            &hash(9),
            &deny_all,
        );
        assert!(matches!(render, TransclusionRender::Unresolved));
        // Unresolved anchor: degraded, never a different block
        // (#5839 invariant 2).
        let render = render_transclusion(
            &hash(9),
            None,
            Some(&id("gone")),
            None,
            Some(&source),
            &hash(9),
            &allow_all,
        );
        assert!(matches!(render, TransclusionRender::Unresolved));
    }

    #[test]
    fn pinned_transclusion_is_exact() {
        let pinned = BlockTree {
            blocks: vec![para("s", "v1")],
        };
        let head = BlockTree {
            blocks: vec![para("other", "v2")],
        };
        // Pinned resolves against the pinned tree only (#5839 invariant 2).
        let render = render_transclusion(
            &hash(9),
            Some(hash(7)),
            Some(&id("s")),
            Some(&pinned),
            Some(&head),
            &hash(9),
            &allow_all,
        );
        match render {
            TransclusionRender::Content { block, pinned } => {
                assert!(pinned);
                assert_eq!(block.text, "v1");
            }
            TransclusionRender::Unresolved => panic!("expected content"),
        }
    }
}
