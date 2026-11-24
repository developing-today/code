//! Block content model for note revisions (#8218, design record #5839 D1/D2).
//!
//! A note's content is an ordered tree of blocks: typed content nodes
//! (paragraph, heading, list item, to-do, quote, code, attachment
//! reference, table, transclusion) carrying stable block IDs scoped to
//! their note. Blocks are not independent objects — identity, links,
//! permissions and revision lineage all stay at the note (#5839
//! invariant 1) — so this module defines the *content* shape and the
//! anchor rules, not a new object kind.
//!
//! The note substrate (#3814) stores a revision's content as a
//! [`crate::documents::DocumentRef`] blob; a block tree is the decoded
//! form of such a blob (see [`BlockTree::to_bytes`] / [`BlockTree::from_bytes`]).
//! Nothing in this module changes that substrate: it is the schema +
//! resolution layer the substrate's content bytes conform to, and the
//! anchor resolver any projection calls into.
//!
//! First slice (this module): block schema, stable block IDs with
//! tombstoned-ID non-reuse, and D2 anchor resolution. The D3/D4/D5
//! projections (outline, journal, database views) and note-granularity
//! read-authority enforcement are split into named follow-on packets on
//! #8218; see that issue for the split.

use std::collections::{BTreeSet, HashSet};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

/// Media type a [`BlockTree`] is serialized as inside a
/// [`crate::documents::DocumentRef`].
pub const BLOCK_CONTENT_MEDIA_TYPE: &str = "application/vnd.id.note-blocks+postcard";

/// The most bytes a block ID may have.
pub const MAX_BLOCK_ID_BYTES: usize = 64;

/// The most bytes a block's text may have.
pub const MAX_BLOCK_TEXT_BYTES: usize = 65_536;

/// The most nesting depth a block tree may have.
pub const MAX_BLOCK_DEPTH: usize = 64;

/// The most bytes a serialized block tree may have on the wire.
///
/// This is the decode resource bound for [`BlockTree::from_bytes`]: a
/// longer payload is refused before decoding allocates anything
/// proportional to its length prefixes. It is sized as headroom over
/// content, not as a content policy — 64 blocks carrying the maximum
/// text ([`MAX_BLOCK_TEXT_BYTES`] each) encode to just under this
/// bound, and per-block structural overhead (an ID of at most
/// [`MAX_BLOCK_ID_BYTES`] plus a few length/discriminant bytes) is
/// small next to a max-size text. Nesting depth needs no separate
/// wire bound: every level costs at least 4 bytes on the wire (ID
/// length, kind discriminant, text length, children length), so this
/// cap also bounds decode recursion, and [`BlockTree::check`] then
/// enforces the much smaller [`MAX_BLOCK_DEPTH`] on the decoded tree.
pub const MAX_BLOCK_TREE_BYTES: usize = 4_194_304;

/// Why a block value was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BlockError {
    /// A block ID is empty, not NFC, too long, or otherwise invalid.
    #[error("invalid block id: {0}")]
    BlockId(String),
    /// A block's text is too long or not NFC.
    #[error("invalid block text: {0}")]
    Text(String),
    /// Two blocks in one tree share an ID.
    #[error("duplicate block id {0}")]
    DuplicateId(String),
    /// A block ID was retired (tombstoned) and may not be reused.
    #[error("block id {0} is tombstoned and may not be reused")]
    TombstonedId(String),
    /// The tree exceeds the maximum nesting depth.
    #[error("block tree exceeds maximum depth {MAX_BLOCK_DEPTH}")]
    TooDeep,
    /// Serialization or deserialization failed.
    #[error("block content (de)serialization failed: {0}")]
    Codec(String),
    /// An anchor field is invalid.
    #[error("invalid anchor: {0}")]
    Anchor(String),
}

/// A stable block ID, scoped to a single note. IDs are never reused
/// within a note, including after deletion (tombstoned IDs stay taken).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockId(pub String);

impl BlockId {
    /// Checks the rules every block ID meets.
    ///
    /// # Errors
    ///
    /// Returns [`BlockError::BlockId`] when the ID is empty, longer
    /// than [`MAX_BLOCK_ID_BYTES`], or not NFC.
    pub fn check(&self) -> Result<(), BlockError> {
        if self.0.is_empty() {
            return Err(BlockError::BlockId("block id is empty".to_owned()));
        }
        if self.0.len() > MAX_BLOCK_ID_BYTES {
            return Err(BlockError::BlockId(format!(
                "block id is {} bytes, it may be at most {MAX_BLOCK_ID_BYTES}",
                self.0.len()
            )));
        }
        if !is_nfc(&self.0) {
            return Err(BlockError::BlockId("block id is not NFC".to_owned()));
        }
        Ok(())
    }
}

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The kind of a block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockKind {
    /// Plain paragraph.
    Paragraph,
    /// Heading at a given level (1..=6).
    Heading {
        /// Heading level, 1..=6.
        level: u8,
    },
    /// List item.
    ListItem,
    /// To-do item with checked state.
    ToDo {
        /// Whether the item is checked.
        checked: bool,
    },
    /// Quoted block.
    Quote,
    /// Code block, optionally tagged with a language.
    Code {
        /// Language hint, if any.
        language: Option<String>,
    },
    /// Reference to an attached blob/document.
    AttachmentRef {
        /// The referenced document's media type, if known.
        media_type: Option<String>,
    },
    /// Grid/table content.
    Table,
    /// A block that renders another note's block (transclusion).
    Transclusion {
        /// The target note's stable ID.
        target_note: Hash,
        /// The pinned target revision, if any.
        pinned_revision: Option<Hash>,
        /// The target block, if addressing one; `None` means the whole note.
        target_block: Option<BlockId>,
    },
}

/// A typed content node. Blocks form an ordered tree: a block may
/// contain child blocks (for nesting in outlines and lists).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Stable ID of this block, scoped to its note.
    pub id: BlockId,
    /// The block's kind.
    pub kind: BlockKind,
    /// Inline text content of the block.
    pub text: String,
    /// Child blocks.
    pub children: Vec<Block>,
}

impl Block {
    /// Checks the rules a single block meets (id, text, depth handled by
    /// the tree check).
    ///
    /// # Errors
    ///
    /// Returns [`BlockError`] when the ID or text is invalid.
    pub fn check(&self) -> Result<(), BlockError> {
        self.id.check()?;
        if self.text.len() > MAX_BLOCK_TEXT_BYTES {
            return Err(BlockError::Text(format!(
                "block text is {} bytes, it may be at most {MAX_BLOCK_TEXT_BYTES}",
                self.text.len()
            )));
        }
        if !is_nfc(&self.text) && !self.text.is_empty() {
            // Empty text is allowed (e.g. empty paragraph); non-empty must be NFC.
            return Err(BlockError::Text("block text is not NFC".to_owned()));
        }
        // Heading level sanity.
        if let BlockKind::Heading { level } = &self.kind {
            if !(1..=6).contains(level) {
                return Err(BlockError::Text(format!(
                    "heading level {level} is not in 1..=6"
                )));
            }
        }
        Ok(())
    }
}

/// An ordered tree of blocks: the content of one note revision.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BlockTree {
    /// Top-level blocks in order.
    pub blocks: Vec<Block>,
}

impl BlockTree {
    /// Returns every block ID in the tree, in document order.
    #[must_use]
    pub fn all_ids(&self) -> Vec<BlockId> {
        let mut out = Vec::new();
        fn walk(block: &Block, out: &mut Vec<BlockId>) {
            out.push(block.id.clone());
            for child in &block.children {
                walk(child, out);
            }
        }
        for block in &self.blocks {
            walk(block, &mut out);
        }
        out
    }

    /// Finds a block by ID anywhere in the tree.
    #[must_use]
    pub fn find_block(&self, id: &BlockId) -> Option<&Block> {
        fn walk<'a>(block: &'a Block, id: &BlockId) -> Option<&'a Block> {
            if &block.id == id {
                return Some(block);
            }
            for child in &block.children {
                if let Some(found) = walk(child, id) {
                    return Some(found);
                }
            }
            None
        }
        for block in &self.blocks {
            if let Some(found) = walk(block, id) {
                return Some(found);
            }
        }
        None
    }

    /// Checks the rules a block tree meets: valid blocks, unique IDs
    /// within the tree, and depth within the limit.
    ///
    /// # Errors
    ///
    /// Returns [`BlockError`] on the first violation.
    pub fn check(&self) -> Result<(), BlockError> {
        let mut seen: HashSet<BlockId> = HashSet::new();
        fn walk(
            block: &Block,
            depth: usize,
            seen: &mut HashSet<BlockId>,
        ) -> Result<(), BlockError> {
            if depth > MAX_BLOCK_DEPTH {
                return Err(BlockError::TooDeep);
            }
            block.check()?;
            if !seen.insert(block.id.clone()) {
                return Err(BlockError::DuplicateId(block.id.to_string()));
            }
            for child in &block.children {
                walk(child, depth + 1, seen)?;
            }
            Ok(())
        }
        for block in &self.blocks {
            walk(block, 1, &mut seen)?;
        }
        Ok(())
    }

    /// Serializes the tree to bytes (postcard).
    ///
    /// The encoder enforces the same [`MAX_BLOCK_TREE_BYTES`] bound
    /// the decoder refuses on receipt, so a tree this function encodes
    /// is never one [`BlockTree::from_bytes`] would refuse as
    /// oversized.
    ///
    /// # Errors
    ///
    /// Returns [`BlockError::Codec`] on failure, including when the
    /// encoded form exceeds [`MAX_BLOCK_TREE_BYTES`].
    pub fn to_bytes(&self) -> Result<Vec<u8>, BlockError> {
        let bytes =
            postcard::to_allocvec(self).map_err(|err| BlockError::Codec(err.to_string()))?;
        if bytes.len() > MAX_BLOCK_TREE_BYTES {
            return Err(BlockError::Codec(format!(
                "encoded block tree is {} bytes, it may be at most {MAX_BLOCK_TREE_BYTES}",
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    /// Deserializes a tree from bytes (postcard) and validates it.
    ///
    /// The decoded tree must pass [`BlockTree::check`]: serialized
    /// bytes are the persistence boundary for note revisions, so a
    /// decoder that silently accepted trees the module itself
    /// considers invalid (duplicate or invalid IDs, invalid text,
    /// heading levels outside 1..=6, nesting past
    /// [`MAX_BLOCK_DEPTH`]) would make correctness depend on every
    /// reader remembering a second validation call. Payloads longer
    /// than [`MAX_BLOCK_TREE_BYTES`] are refused before decoding.
    ///
    /// # Errors
    ///
    /// Returns [`BlockError::Codec`] when the payload is oversized or
    /// not valid postcard for a block tree, and the specific
    /// [`BlockError`] [`BlockTree::check`] returns when the decoded
    /// tree violates an invariant.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BlockError> {
        if bytes.len() > MAX_BLOCK_TREE_BYTES {
            return Err(BlockError::Codec(format!(
                "block tree payload is {} bytes, it may be at most {MAX_BLOCK_TREE_BYTES}",
                bytes.len()
            )));
        }
        let tree: Self =
            postcard::from_bytes(bytes).map_err(|err| BlockError::Codec(err.to_string()))?;
        tree.check()?;
        Ok(tree)
    }
}

/// A set of retired (tombstoned) block IDs for one note. Tombstoned
/// IDs stay taken: a later revision may not reuse one.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TombstoneSet {
    /// Retired IDs, in sorted order.
    pub retired: BTreeSet<BlockId>,
}

impl TombstoneSet {
    /// Whether `id` is retired.
    #[must_use]
    pub fn contains(&self, id: &BlockId) -> bool {
        self.retired.contains(id)
    }

    /// Retires `id`. Retiring an already-retired ID is a no-op.
    pub fn retire(&mut self, id: BlockId) {
        self.retired.insert(id);
    }

    /// Checks that no ID in `tree` is retired in `self`.
    ///
    /// # Errors
    ///
    /// Returns [`BlockError::TombstonedId`] on the first reuse.
    pub fn check_tree(&self, tree: &BlockTree) -> Result<(), BlockError> {
        for id in tree.all_ids() {
            if self.retired.contains(&id) {
                return Err(BlockError::TombstonedId(id.to_string()));
            }
        }
        Ok(())
    }

    /// Derives the tombstone set for a note from a sequence of revisions'
    /// block trees: any ID that appeared in an earlier revision but is
    /// absent from `latest` is retired. This captures "deleted block IDs
    /// stay taken" (D2) without storing deletions as events.
    #[must_use]
    pub fn derive(previous: &[BlockTree], latest: &BlockTree) -> Self {
        let latest_ids: HashSet<BlockId> = latest.all_ids().into_iter().collect();
        let mut retired = BTreeSet::new();
        for tree in previous {
            for id in tree.all_ids() {
                if !latest_ids.contains(&id) {
                    retired.insert(id);
                }
            }
        }
        Self { retired }
    }
}

/// An anchor addressing a block inside a note: (note ID, optional
/// pinned revision ID, block ID). Extension of #2048's typed references
/// with an intra-note anchor (D2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockAnchor {
    /// The target note's stable ID.
    pub note: Hash,
    /// The pinned target revision, if any. `None` means unpinned.
    pub revision: Option<Hash>,
    /// The block addressed within the note.
    pub block: BlockId,
}

impl BlockAnchor {
    /// Checks the anchor's fields.
    ///
    /// # Errors
    ///
    /// Returns [`BlockError::Anchor`] or [`BlockError::BlockId`] on
    /// violation.
    pub fn check(&self) -> Result<(), BlockError> {
        self.block.check()?;
        // Note existence is validated by the projection, not here.
        Ok(())
    }
}

/// The result of resolving a block anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorResolution {
    /// The anchor resolved to a block.
    Block {
        /// The resolved block ID.
        block: BlockId,
        /// Whether the resolution was exact (pinned) or against the head.
        pinned: bool,
    },
    /// The anchor degraded to the note, with an unresolved-anchor state.
    DegradedToNote {
        /// Whether the degradation is reported as unresolved (always true
        /// for a degraded anchor).
        unresolved: bool,
    },
}

/// Resolves a block anchor given the pinned revision's tree (if a
/// revision is pinned and known) and the current head's tree.
///
/// Pinned anchors are exact and permanent: they resolve only against
/// the pinned revision's tree. Unpinned anchors resolve against the
/// current head and degrade to the note when the block is absent.
#[must_use]
pub fn resolve_anchor(
    anchor: &BlockAnchor,
    pinned_tree: Option<&BlockTree>,
    head_tree: Option<&BlockTree>,
) -> AnchorResolution {
    if let Some(rev) = anchor.revision {
        let _ = rev;
        // Pinned: exact resolution against the pinned revision only.
        if let Some(tree) = pinned_tree {
            if tree.find_block(&anchor.block).is_some() {
                return AnchorResolution::Block {
                    block: anchor.block.clone(),
                    pinned: true,
                };
            }
        }
        return AnchorResolution::DegradedToNote { unresolved: true };
    }
    // Unpinned: resolve against the current head.
    if let Some(tree) = head_tree {
        if tree.find_block(&anchor.block).is_some() {
            return AnchorResolution::Block {
                block: anchor.block.clone(),
                pinned: false,
            };
        }
    }
    AnchorResolution::DegradedToNote { unresolved: true }
}

/// A record that a block moved from one note to another. The move
/// mints a new block ID in the target note; provenance is recorded so
/// anchors to the old pair degrade rather than silently follow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockMove {
    /// The source note's stable ID.
    pub from_note: Hash,
    /// The block ID in the source note.
    pub from_block: BlockId,
    /// The target note's stable ID.
    pub to_note: Hash,
    /// The minted block ID in the target note. Must differ from
    /// `from_block`.
    pub to_block: BlockId,
    /// The revision that records the move.
    pub revision: Hash,
}

impl BlockMove {
    /// Checks the move record's invariants: distinct notes/blocks and
    /// valid IDs.
    ///
    /// # Errors
    ///
    /// Returns [`BlockError`] on violation.
    pub fn check(&self) -> Result<(), BlockError> {
        self.from_block.check()?;
        self.to_block.check()?;
        if self.from_note == self.to_note {
            return Err(BlockError::Anchor("a move must cross notes".to_owned()));
        }
        if self.from_block == self.to_block {
            return Err(BlockError::Anchor(
                "a move must mint a new block id".to_owned(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

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

    #[test]
    fn block_id_rejects_empty_and_overlong() {
        assert!(BlockId(String::new()).check().is_err());
        assert!(BlockId("x".repeat(MAX_BLOCK_ID_BYTES + 1)).check().is_err());
        assert!(id("ok").check().is_ok());
    }

    #[test]
    fn tree_check_rejects_duplicate_ids() {
        let tree = BlockTree {
            blocks: vec![para("a", "x"), para("a", "y")],
        };
        assert!(matches!(tree.check(), Err(BlockError::DuplicateId(_))));
    }

    #[test]
    fn tree_check_accepts_nested_unique_ids() {
        let tree = BlockTree {
            blocks: vec![Block {
                id: id("h"),
                kind: BlockKind::Heading { level: 1 },
                text: "title".to_owned(),
                children: vec![para("p", "body")],
            }],
        };
        assert!(tree.check().is_ok());
        assert_eq!(tree.all_ids(), vec![id("h"), id("p")]);
        assert!(tree.find_block(&id("p")).is_some());
    }

    #[test]
    fn tombstones_reject_reuse() {
        let mut ts = TombstoneSet::default();
        ts.retire(id("gone"));
        let tree = BlockTree {
            blocks: vec![para("gone", "reborn")],
        };
        assert!(matches!(
            ts.check_tree(&tree),
            Err(BlockError::TombstonedId(_))
        ));
        // A tree without the retired id passes.
        let tree2 = BlockTree {
            blocks: vec![para("fresh", "ok")],
        };
        assert!(ts.check_tree(&tree2).is_ok());
    }

    #[test]
    fn derive_tombstones_retires_deleted_ids() {
        let prev = BlockTree {
            blocks: vec![para("a", "x"), para("b", "y")],
        };
        let latest = BlockTree {
            blocks: vec![para("a", "x")],
        };
        let ts = TombstoneSet::derive(&[prev], &latest);
        assert!(ts.contains(&id("b")));
        assert!(!ts.contains(&id("a")));
    }

    #[test]
    fn round_trip_serialization() {
        let tree = BlockTree {
            blocks: vec![Block {
                id: id("h"),
                kind: BlockKind::Heading { level: 2 },
                text: "title".to_owned(),
                children: vec![
                    para("p", "body"),
                    Block {
                        id: id("todo"),
                        kind: BlockKind::ToDo { checked: true },
                        text: "done".to_owned(),
                        children: Vec::new(),
                    },
                ],
            }],
        };
        let bytes = tree.to_bytes().unwrap();
        let back = BlockTree::from_bytes(&bytes).unwrap();
        assert_eq!(tree, back);
    }

    #[test]
    fn from_bytes_rejects_duplicate_ids() {
        let tree = BlockTree {
            blocks: vec![para("a", "x"), para("a", "y")],
        };
        let bytes = tree.to_bytes().unwrap();
        assert!(matches!(
            BlockTree::from_bytes(&bytes),
            Err(BlockError::DuplicateId(_))
        ));
    }

    #[test]
    fn from_bytes_rejects_invalid_ids() {
        // Empty ID.
        let tree = BlockTree {
            blocks: vec![para("", "x")],
        };
        let bytes = tree.to_bytes().unwrap();
        assert!(matches!(
            BlockTree::from_bytes(&bytes),
            Err(BlockError::BlockId(_))
        ));
        // Non-NFC ID ("e" + combining acute U+0301).
        let tree = BlockTree {
            blocks: vec![para("e\u{301}", "x")],
        };
        let bytes = tree.to_bytes().unwrap();
        assert!(matches!(
            BlockTree::from_bytes(&bytes),
            Err(BlockError::BlockId(_))
        ));
    }

    #[test]
    fn from_bytes_rejects_invalid_text() {
        // Oversized text.
        let tree = BlockTree {
            blocks: vec![para("a", &"x".repeat(MAX_BLOCK_TEXT_BYTES + 1))],
        };
        let bytes = tree.to_bytes().unwrap();
        assert!(matches!(
            BlockTree::from_bytes(&bytes),
            Err(BlockError::Text(_))
        ));
        // Non-NFC text.
        let tree = BlockTree {
            blocks: vec![para("a", "e\u{301}")],
        };
        let bytes = tree.to_bytes().unwrap();
        assert!(matches!(
            BlockTree::from_bytes(&bytes),
            Err(BlockError::Text(_))
        ));
    }

    #[test]
    fn from_bytes_rejects_invalid_heading_level() {
        for level in [0u8, 7u8] {
            let tree = BlockTree {
                blocks: vec![Block {
                    id: id("h"),
                    kind: BlockKind::Heading { level },
                    text: "title".to_owned(),
                    children: Vec::new(),
                }],
            };
            let bytes = tree.to_bytes().unwrap();
            assert!(matches!(
                BlockTree::from_bytes(&bytes),
                Err(BlockError::Text(_))
            ));
        }
    }

    #[test]
    fn from_bytes_rejects_excessive_depth() {
        // A chain nested past MAX_BLOCK_DEPTH, with unique IDs.
        let mut block = para("leaf", "x");
        for i in 0..MAX_BLOCK_DEPTH {
            block = Block {
                id: id(&format!("wrap-{i}")),
                kind: BlockKind::Paragraph,
                text: String::new(),
                children: vec![block],
            };
        }
        let tree = BlockTree { blocks: vec![block] };
        assert!(matches!(tree.check(), Err(BlockError::TooDeep)));
        let bytes = tree.to_bytes().unwrap();
        assert!(matches!(
            BlockTree::from_bytes(&bytes),
            Err(BlockError::TooDeep)
        ));
    }

    #[test]
    fn from_bytes_rejects_oversized_payload_before_decoding() {
        let payload = vec![0u8; MAX_BLOCK_TREE_BYTES + 1];
        assert!(matches!(
            BlockTree::from_bytes(&payload),
            Err(BlockError::Codec(_))
        ));
    }

    #[test]
    fn to_bytes_refuses_tree_over_the_wire_bound() {
        // 65 blocks at the maximum text size encode past the bound;
        // the tree itself is valid, so only the wire bound refuses it.
        let blocks = (0..65)
            .map(|i| para(&format!("b{i}"), &"x".repeat(MAX_BLOCK_TEXT_BYTES)))
            .collect();
        let tree = BlockTree { blocks };
        assert!(tree.check().is_ok());
        assert!(matches!(tree.to_bytes(), Err(BlockError::Codec(_))));
    }

    #[test]
    fn pinned_anchor_exact_then_degrades() {
        let tree = BlockTree {
            blocks: vec![para("p", "body")],
        };
        let anchor = BlockAnchor {
            note: hash(1),
            revision: Some(hash(2)),
            block: id("p"),
        };
        // Pinned and present -> exact.
        assert_eq!(
            resolve_anchor(&anchor, Some(&tree), None),
            AnchorResolution::Block {
                block: id("p"),
                pinned: true,
            }
        );
        // Pinned but block absent in pinned revision -> degraded.
        let empty = BlockTree::default();
        assert_eq!(
            resolve_anchor(&anchor, Some(&empty), None),
            AnchorResolution::DegradedToNote { unresolved: true }
        );
    }

    #[test]
    fn unpinned_anchor_degrades_when_missing_at_head() {
        let tree = BlockTree {
            blocks: vec![para("p", "body")],
        };
        let anchor = BlockAnchor {
            note: hash(1),
            revision: None,
            block: id("missing"),
        };
        assert_eq!(
            resolve_anchor(&anchor, None, Some(&tree)),
            AnchorResolution::DegradedToNote { unresolved: true }
        );
        let anchor2 = BlockAnchor {
            note: hash(1),
            revision: None,
            block: id("p"),
        };
        assert_eq!(
            resolve_anchor(&anchor2, None, Some(&tree)),
            AnchorResolution::Block {
                block: id("p"),
                pinned: false,
            }
        );
    }

    #[test]
    fn cross_note_move_mints_new_id() {
        let mv = BlockMove {
            from_note: hash(1),
            from_block: id("orig"),
            to_note: hash(2),
            to_block: id("minted"),
            revision: hash(3),
        };
        assert!(mv.check().is_ok());
        // Reusing the same id is rejected.
        let bad = BlockMove {
            from_note: hash(1),
            from_block: id("orig"),
            to_note: hash(2),
            to_block: id("orig"),
            revision: hash(3),
        };
        assert!(bad.check().is_err());
        // Same note is rejected.
        let bad2 = BlockMove {
            from_note: hash(1),
            from_block: id("a"),
            to_note: hash(1),
            to_block: id("b"),
            revision: hash(3),
        };
        assert!(bad2.check().is_err());
    }

    #[test]
    fn transclusion_carries_anchor() {
        let block = Block {
            id: id("t"),
            kind: BlockKind::Transclusion {
                target_note: hash(9),
                pinned_revision: Some(hash(7)),
                target_block: Some(id("src")),
            },
            text: String::new(),
            children: Vec::new(),
        };
        assert!(block.check().is_ok());
    }
}
