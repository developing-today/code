//! Board view builder — topic/comment tree projection (#13096;
//! design #9128 §5–§7, `doc/2026-11-23_design_id9128_social_app_reference_model.md`).
//!
//! A board is an object with an append-list attribute `topics`; an
//! accepted submission becomes a topic, and each topic carries an
//! append-list attribute `comments`. The builder projects those two
//! append-lists plus the #9130 tombstone state into the thread tree
//! view:
//!
//! - Tree shape comes from each comment's `parent_ref` (the topic or
//!   another comment); order at every level comes from append sequence
//!   (#9127 server order). The builder consumes the output of
//!   [`AppendListServer::read`](crate::append_list::AppendListServer::read),
//!   so pending submissions never appear: `read` never returns them
//!   (#9127 §6).
//! - Tombstoned entries are filtered per the view's [`TombstoneSet`]
//!   without renumbering: survivors keep their [`Entry::seq`], and
//!   sibling order is untouched. A tombstoned entry hides its whole
//!   subtree — its descendants are reachable only through it, and the
//!   tombstone is a filter, never a rewrite (#9130). Hiding the
//!   subtree rather than reparenting its children is this slice's
//!   recorded decision (FDR-0013); a placeholder/reparent variant is a
//!   follow-up if the owner wants it.
//! - Every rendered subtree carries a [`BoardViewToken`] for its
//!   inputs, so a collapsed subtree re-checks via the §6.3
//!   nothing-new path instead of refetching ([`check_freshness`]).
//!   A token's frontier is the high-water mark of the prefix actually
//!   rendered in this page: a page that renders a list whole mints a
//!   token whose head is the list's head, and a page that truncates a
//!   list mints a token whose head is the last rendered sequence, so
//!   the remainder is a §6.3 delta from that head — never a refetch.
//! - Pages are bounded per §7: max display depth 32, max 2,000 visited
//!   nodes per page, continued by [`BoardCursor`]. The 8 MiB response
//!   cap is a transport concern — the builder holds hashes, not bodies,
//!   so it bounds nodes, not bytes.
//!
//! The builder is pure projection over already-verified inputs: it
//! verifies nothing, signs nothing, and never fails. Unknown
//! `parent_ref`s (a parent that is neither the topic nor an entry of
//! the list, e.g. not yet arrived) attach at the top level so the
//! comment stays visible; a `parent_ref` cycle renders as a
//! back-reference instead of recursing (§7, cycle-safe traversal).
//!
//! Freshness tokens here are the board view's own token type. The
//! general token type and frontier comparison of #13094 are a sibling
//! slice in flight; when it lands, this module's token should be
//! rewired onto it (the frontier shapes are the same: per-input head
//! sequence plus tombstone-set revision).

use std::collections::{BTreeMap, BTreeSet};

use blake3::Hasher;
use ed25519_dalek::VerifyingKey;

use crate::append_list::{ContentHash, Entry};

/// §7 defaults: max display depth, max visited nodes per page.
pub const DEFAULT_MAX_DEPTH: usize = 32;
/// §7 default: max visited nodes per page.
pub const DEFAULT_MAX_NODES_PER_PAGE: usize = 2_000;

/// Bounds for one board view page (§7). Policy may tighten these,
/// never silently loosen them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardViewConfig {
    /// Max display depth. A topic is depth 0; its direct comments are
    /// depth 1. Nodes at this depth render with their children folded.
    pub max_depth: usize,
    /// Max visited nodes (topics plus comments) in one page.
    pub max_nodes_per_page: usize,
    /// Nodes per page; clamped to `max_nodes_per_page`.
    pub page_size: usize,
}

impl Default for BoardViewConfig {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_nodes_per_page: DEFAULT_MAX_NODES_PER_PAGE,
            page_size: DEFAULT_MAX_NODES_PER_PAGE,
        }
    }
}

impl BoardViewConfig {
    /// A config with the §7 defaults and this page size, clamped to the
    /// node budget.
    #[must_use]
    pub fn with_page_size(page_size: usize) -> Self {
        let mut config = Self::default();
        config.page_size = page_size.min(config.max_nodes_per_page).max(1);
        config
    }
}

/// The view's tombstone state (#9130): which entries are hidden in
/// this view, and the revision of that set. Every hide/show bumps the
/// revision, so a token minted against one revision checks `resync`
/// against another (§6.3: a tombstone-set rewrite is not a delta).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TombstoneSet {
    hidden: BTreeSet<ContentHash>,
    revision: u64,
}

impl TombstoneSet {
    /// An empty set at revision 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Hides `hash` in the view and bumps the revision. Hiding an
    /// already-hidden entry is a no-op and does not bump.
    pub fn hide(&mut self, hash: ContentHash) {
        if self.hidden.insert(hash) {
            self.revision += 1;
        }
    }

    /// Shows `hash` again and bumps the revision. Showing a visible
    /// entry is a no-op and does not bump.
    pub fn show(&mut self, hash: ContentHash) {
        if self.hidden.remove(&hash) {
            self.revision += 1;
        }
    }

    /// Whether `hash` is hidden in this view.
    #[must_use]
    pub fn is_hidden(&self, hash: ContentHash) -> bool {
        self.hidden.contains(&hash)
    }

    /// The revision of this set.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// One input's high-water mark in a freshness frontier (§6.2): the
/// accepted head sequence of the input prefix the token certifies,
/// plus the tombstone-set revision that filters it. `head_seq` is
/// `None` when the certified prefix is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrontierEntry {
    /// The input list's canonical id (a hash of its list path).
    pub list: ContentHash,
    /// Sequence of the list's last accepted entry, if any.
    pub head_seq: Option<u64>,
    /// The tombstone-set revision that filters this list.
    pub tombstone_revision: u64,
}

/// A freshness token for one rendered subtree (§6.1): the canonical
/// view definition, the input frontier at issue time, and the issue
/// time. The server signature of §6.1 lives with the issuing server;
/// this pure-logic type carries the checkable contents, and the caller
/// wraps it with its signature when serving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardViewToken {
    /// Hash of the canonical view definition: board + projection +
    /// policy revision.
    pub view_id: ContentHash,
    /// The view's input frontier at issue time.
    pub frontier: Vec<FrontierEntry>,
    /// Server time of the check that issued this token, in
    /// milliseconds since the Unix epoch (caller-supplied; the builder
    /// reads no clock).
    pub issued_at: u64,
}

impl BoardViewToken {
    /// The input frontier this token was issued against.
    #[must_use]
    pub fn frontier(&self) -> &[FrontierEntry] {
        &self.frontier
    }
}

/// The canonical view definition (§6.1 `view_id`) for the board view
/// itself: board plus policy revision.
#[must_use]
pub fn board_view_id(board: &ContentHash, policy_revision: u64) -> ContentHash {
    view_id(board, "board", board, policy_revision)
}

/// The canonical view definition for one topic's subtree.
#[must_use]
pub fn topic_view_id(board: &ContentHash, topic: &ContentHash, policy_revision: u64) -> ContentHash {
    view_id(board, "topic", topic, policy_revision)
}

/// The canonical view definition for one comment's subtree.
#[must_use]
pub fn comment_view_id(
    board: &ContentHash,
    comment: &ContentHash,
    policy_revision: u64,
) -> ContentHash {
    view_id(board, "comment", comment, policy_revision)
}

/// The §6.3 verdict for a token against the current frontier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckVerdict {
    /// The frontier is unchanged: the holder's view is current.
    NothingNew,
    /// The frontier advanced. `after` is the token's frontier: read
    /// each input list with `append-read` after its recorded head
    /// sequence to fetch the delta, never the unchanged prefix.
    Delta {
        /// The token's frontier; read after each entry's head sequence.
        after: Vec<FrontierEntry>,
    },
    /// The token's frontier is no longer reachable (view definition
    /// changed, tombstone set rewritten, or a frontier moved
    /// backwards): refetch the view once and take a fresh token.
    Resync,
}

/// Checks a token against the current frontier (§6.3).
///
/// The caller verifies the token belongs to the view being polled by
/// passing that view's canonical id ([`board_view_id`],
/// [`topic_view_id`] or [`comment_view_id`]); a mismatch is
/// [`Resync`], as is a frontier-shape mismatch, a tombstone revision
/// change, or any input head moving backwards. Every component equal
/// is [`NothingNew`]; otherwise every head is at or past the token's
/// and at least one advanced, which is [`Delta`] from the token's
/// head sequences.
#[must_use]
pub fn check_freshness(
    token: &BoardViewToken,
    view_id: &ContentHash,
    current: &[FrontierEntry],
) -> CheckVerdict {
    if token.view_id != *view_id || token.frontier.len() != current.len() {
        return CheckVerdict::Resync;
    }
    let mut advanced = false;
    for (old, now) in token.frontier.iter().zip(current.iter()) {
        if old.list != now.list || old.tombstone_revision != now.tombstone_revision {
            return CheckVerdict::Resync;
        }
        match (old.head_seq, now.head_seq) {
            (a, b) if a == b => {}
            (a, b) if b > a => advanced = true,
            _ => return CheckVerdict::Resync,
        }
    }
    if advanced {
        CheckVerdict::Delta {
            after: token.frontier.clone(),
        }
    } else {
        CheckVerdict::NothingNew
    }
}

/// A position in the topic list a page resumes from: the sequence of
/// the next unvisited topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BoardCursor {
    /// Sequence of the next topic to render.
    pub next_topic_seq: u64,
}

/// One rendered comment in the thread tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentNode {
    /// The entry's append sequence, unchanged by tombstone filtering.
    pub seq: u64,
    /// The comment object's content hash.
    pub hash: ContentHash,
    /// The key that signed the comment.
    pub submitter: VerifyingKey,
    /// Rendered children, in append order.
    pub children: Vec<CommentNode>,
    /// Freshness token for this subtree's inputs: the topic's
    /// comments list frontier. Expanding a folded subtree is the
    /// §6.3 check against this token, not a blind refetch.
    pub token: BoardViewToken,
    /// True when children were cut by the depth or node budget:
    /// children exist but are not rendered here.
    pub folded: bool,
    /// Set when this node is a back-reference to an ancestor in a
    /// `parent_ref` cycle (§7): the cycle is displayed, not recursed
    /// into. The node carries the ancestor's hash and no children.
    pub back_reference_to: Option<ContentHash>,
}

/// One rendered topic: the accepted submission plus its comment tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicNode {
    /// The entry's append sequence, unchanged by tombstone filtering.
    pub seq: u64,
    /// The topic object's content hash.
    pub hash: ContentHash,
    /// The key that signed the topic.
    pub submitter: VerifyingKey,
    /// Top-level comments, in append order.
    pub comments: Vec<CommentNode>,
    /// Freshness token for this topic's inputs: its comments list
    /// frontier over the rendered prefix.
    pub token: BoardViewToken,
    /// True when the topic has comments beyond those rendered here
    /// (cut by the node budget): the remainder is a §6.3 delta from
    /// the token's head sequence.
    pub folded: bool,
}

/// One page of a board view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardView {
    /// The board object's content hash.
    pub board: ContentHash,
    /// Rendered topics in append order.
    pub topics: Vec<TopicNode>,
    /// Freshness token for the board's inputs: the topics list
    /// frontier.
    pub token: BoardViewToken,
    /// The cursor for the next page, or `None` when this page holds
    /// the last topic.
    pub next_cursor: Option<BoardCursor>,
}

/// Canonical id of the topics list for `board`.
fn topics_list_id(board: &ContentHash) -> ContentHash {
    let mut hasher = Hasher::new();
    hasher.update(b"id-board-view/1/topics\0");
    hasher.update(&board.0);
    ContentHash(*hasher.finalize().as_bytes())
}

/// Canonical id of `topic`'s comments list.
fn comments_list_id(topic: &ContentHash) -> ContentHash {
    let mut hasher = Hasher::new();
    hasher.update(b"id-board-view/1/comments\0");
    hasher.update(&topic.0);
    ContentHash(*hasher.finalize().as_bytes())
}

/// Hash of the canonical view definition (§6.1 `view_id`).
fn view_id(board: &ContentHash, kind: &str, root: &ContentHash, policy_revision: u64) -> ContentHash {
    let mut hasher = Hasher::new();
    hasher.update(b"id-board-view/1\0");
    hasher.update(&board.0);
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(&root.0);
    hasher.update(&policy_revision.to_le_bytes());
    ContentHash(*hasher.finalize().as_bytes())
}

fn mint_token(
    view_id: ContentHash,
    list: ContentHash,
    head: Option<u64>,
    tombstone_revision: u64,
    issued_at: u64,
) -> BoardViewToken {
    BoardViewToken {
        view_id,
        frontier: vec![FrontierEntry {
            list,
            head_seq: head,
            tombstone_revision,
        }],
        issued_at,
    }
}

/// Builds one page of a board view (§5).
///
/// - `topics`: the board's full topics list in server order, as
///   returned by `append-read` (pending never appears).
/// - `comments`: the full comments list for a topic hash, in server
///   order.
/// - `parent_of`: a comment hash's `parent_ref` — the topic or another
///   comment — or `None` for a top-level comment. Unknown parents
///   attach at the top level.
/// - `tombstones`: the view's tombstone state; hidden entries (and
///   their subtrees) are filtered without renumbering.
/// - `policy_revision`: the view-policy revision bound into every
///   token's `view_id`.
/// - `issued_at`: server time of this build, in milliseconds since the
///   Unix epoch, recorded on every minted token.
/// - `cursor`: resume from this topic sequence; `None` starts at the
///   first topic.
///
/// Topics render whole and in order; a page stops before the node
/// budget would be exceeded, and a single topic larger than the budget
/// renders truncated with its remainder folded. The cursor names the
/// next unvisited topic, so pages chain without overlap or gaps; a
/// truncated topic's remainder is not on the cursor path — it expands
/// via that topic's token (§6.3 delta from the token's head).
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn build_board_view(
    board: &ContentHash,
    topics: &[Entry],
    comments: &dyn Fn(&ContentHash) -> Vec<Entry>,
    parent_of: &dyn Fn(&ContentHash) -> Option<ContentHash>,
    tombstones: &TombstoneSet,
    policy_revision: u64,
    issued_at: u64,
    config: &BoardViewConfig,
    cursor: Option<&BoardCursor>,
) -> BoardView {
    let page_size = config.page_size.min(config.max_nodes_per_page).max(1);
    let max_depth = config.max_depth.max(1);

    let start = cursor.map_or(0, |cursor| cursor.next_topic_seq);
    let mut rendered: Vec<TopicNode> = Vec::new();
    let mut used = 0usize;
    let mut next_cursor = None;

    for topic in topics.iter().filter(|topic| topic.seq >= start) {
        if tombstones.is_hidden(topic.hash) {
            continue;
        }
        // Whole topics only, unless the page is still empty: then the
        // topic renders truncated rather than the page coming back
        // empty.
        let entries = comments(&topic.hash);
        let visible: Vec<&Entry> = entries
            .iter()
            .filter(|entry| !tombstones.is_hidden(entry.hash))
            .collect();
        let topic_nodes = 1 + visible.len();
        if !rendered.is_empty() && used + topic_nodes > page_size {
            next_cursor = Some(BoardCursor {
                next_topic_seq: topic.seq,
            });
            break;
        }
        let budget = page_size.saturating_sub(used);
        let node = render_topic(
            board,
            topic,
            &entries,
            &visible,
            parent_of,
            tombstones,
            policy_revision,
            issued_at,
            max_depth,
            budget,
        );
        used += node_size(&node);
        rendered.push(node);
        if used >= page_size {
            // There may be a next topic after this one.
            let after = topics
                .iter()
                .filter(|t| t.seq > topic.seq && !tombstones.is_hidden(t.hash))
                .map(|t| t.seq)
                .min();
            next_cursor = after.map(|seq| BoardCursor { next_topic_seq: seq });
            break;
        }
    }

    // The board token certifies the rendered prefix: its head is the
    // last rendered topic's sequence, so a page that ends mid-list
    // still yields a correct §6.3 delta for the remainder. A page
    // that renders the list whole has the list's head here.
    let board_token = mint_token(
        board_view_id(board, policy_revision),
        topics_list_id(board),
        rendered.last().map(|topic| topic.seq),
        tombstones.revision(),
        issued_at,
    );

    BoardView {
        board: *board,
        topics: rendered,
        token: board_token,
        next_cursor,
    }
}

fn node_size(node: &TopicNode) -> usize {
    1 + node.comments.iter().map(comment_size).sum::<usize>()
}

fn comment_size(node: &CommentNode) -> usize {
    1 + node.children.iter().map(comment_size).sum::<usize>()
}

/// Renders one topic and its comment tree, bounded by `budget` nodes
/// (the topic itself plus its comments). `full` is the topic's whole
/// comments list: the token's head sequence is taken from it, because
/// tombstoned entries still occupy their sequences. `visible` is the
/// filtered view of the same list.
#[allow(clippy::too_many_arguments)]
fn render_topic(
    board: &ContentHash,
    topic: &Entry,
    full: &[Entry],
    visible: &[&Entry],
    parent_of: &dyn Fn(&ContentHash) -> Option<ContentHash>,
    tombstones: &TombstoneSet,
    policy_revision: u64,
    issued_at: u64,
    max_depth: usize,
    budget: usize,
) -> TopicNode {
    let list = comments_list_id(&topic.hash);

    // Children keyed by parent hash, in append order. A child of a
    // tombstoned entry is keyed under a hash no rendered node ever
    // expands, so the whole hidden subtree drops out of the walk.
    let mut by_parent: BTreeMap<ContentHash, Vec<&Entry>> = BTreeMap::new();
    let in_list: BTreeSet<ContentHash> = full.iter().map(|entry| entry.hash).collect();
    for entry in visible {
        if let Some(parent) = parent_of(entry.hash) {
            by_parent.entry(parent).or_default().push(entry);
        }
    }
    // Roots: no parent, the topic itself as parent, or a parent that
    // is not an entry of this list at all (dangling — not yet
    // arrived). A parent that is in the list but not visible is a
    // tombstoned ancestor: the entry stays hidden with it.
    let mut roots: Vec<&Entry> = Vec::new();
    for entry in visible {
        match parent_of(entry.hash) {
            None => roots.push(entry),
            Some(parent) if parent == topic.hash => roots.push(entry),
            Some(parent) if !in_list.contains(&parent) => roots.push(entry),
            Some(_) => {}
        }
    }

    let mut state = RenderState {
        board,
        by_parent: &by_parent,
        tombstones,
        policy_revision,
        issued_at,
        list,
        max_depth,
        remaining: budget.saturating_sub(1),
        visited: BTreeSet::new(),
    };
    let mut comments = Vec::new();
    let mut folded = false;
    for root in roots {
        if state.remaining == 0 {
            folded = true;
            break;
        }
        if let Some(node) = render_comment(root, &mut state, 1, &mut Vec::new()) {
            comments.push(node);
        } else {
            folded = true;
            break;
        }
    }
    // Entries never reached from a root belong to a `parent_ref`
    // cycle (or a chain into one) disconnected from the topic.
    // Emit them top-level in append order; the walk renders the cycle
    // as back-references, so nothing is silently lost.
    for entry in visible {
        if state.remaining == 0 {
            folded = true;
            break;
        }
        if !state.visited.contains(&entry.hash) {
            // The same hash may name two entries (two submitters of
            // one object, #9127 §8.5); the visited set is by hash, so
            // a duplicate renders once, under its earliest sequence.
            if let Some(node) = render_comment(entry, &mut state, 1, &mut Vec::new()) {
                comments.push(node);
            } else {
                folded = true;
                break;
            }
        }
    }
    // The topic token certifies the rendered prefix: its head is the
    // highest rendered comment sequence, so a truncated topic still
    // yields a correct §6.3 delta for its remainder.
    let head = comments.iter().map(comment_max_seq).max();
    let topic_token = mint_token(
        topic_view_id(board, &topic.hash, policy_revision),
        list,
        head,
        tombstones.revision(),
        issued_at,
    );

    TopicNode {
        seq: topic.seq,
        hash: topic.hash,
        submitter: topic.submitter,
        comments,
        token: topic_token,
        folded,
    }
}

/// Highest append sequence rendered in a comment subtree.
fn comment_max_seq(node: &CommentNode) -> u64 {
    node.children
        .iter()
        .map(comment_max_seq)
        .max()
        .map_or(node.seq, |child| child.max(node.seq))
}

/// Mutable walk state for one topic's tree render.
struct RenderState<'a> {
    board: &'a ContentHash,
    by_parent: &'a BTreeMap<ContentHash, Vec<&'a Entry>>,
    tombstones: &'a TombstoneSet,
    policy_revision: u64,
    issued_at: u64,
    list: ContentHash,
    max_depth: usize,
    remaining: usize,
    visited: BTreeSet<ContentHash>,
}

impl RenderState<'_> {
    /// The token for the subtree rooted at `root`: the topic's
    /// comments list frontier over the rendered prefix (`head` is the
    /// highest sequence rendered in this subtree), bound to this
    /// subtree's view definition.
    fn subtree_token(&self, root: &ContentHash, head: Option<u64>) -> BoardViewToken {
        mint_token(
            comment_view_id(self.board, root, self.policy_revision),
            self.list,
            head,
            self.tombstones.revision(),
            self.issued_at,
        )
    }
}

/// Renders one comment and its subtree. Returns `None` when the node
/// budget is exhausted. `path` holds the ancestor hashes on the
/// current walk, for cycle detection.
fn render_comment(
    entry: &Entry,
    state: &mut RenderState<'_>,
    depth: usize,
    path: &mut Vec<ContentHash>,
) -> Option<CommentNode> {
    if state.remaining == 0 {
        return None;
    }
    if path.contains(&entry.hash) {
        // A `parent_ref` cycle: display the back-reference, do not
        // recurse (§7).
        state.remaining -= 1;
        state.visited.insert(entry.hash);
        return Some(CommentNode {
            seq: entry.seq,
            hash: entry.hash,
            submitter: entry.submitter,
            children: Vec::new(),
            token: state.subtree_token(&entry.hash, Some(entry.seq)),
            folded: false,
            back_reference_to: Some(entry.hash),
        });
    }
    state.remaining -= 1;
    state.visited.insert(entry.hash);
    let mut children = Vec::new();
    let mut folded = false;
    if depth < state.max_depth {
        path.push(entry.hash);
        if let Some(siblings) = state.by_parent.get(&entry.hash) {
            for child in siblings {
                if state.remaining == 0 {
                    folded = true;
                    break;
                }
                if let Some(node) = render_comment(child, state, depth + 1, path) {
                    children.push(node);
                } else {
                    folded = true;
                    break;
                }
            }
        }
        path.pop();
    } else if state.by_parent.contains_key(&entry.hash) {
        // At the depth cap with children below: fold, don't drop.
        folded = true;
    }
    let head = Some(
        children
            .iter()
            .map(comment_max_seq)
            .max()
            .map_or(entry.seq, |child| child.max(entry.seq)),
    );
    Some(CommentNode {
        seq: entry.seq,
        hash: entry.hash,
        submitter: entry.submitter,
        children,
        token: state.subtree_token(&entry.hash, head),
        folded,
        back_reference_to: None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeMap;

    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;
    use crate::append_list::{AppendListServer, AttributeConfig, SubmitOutcome};

    const TOPICS: &str = "board/topics";
    const COMMENTS_A: &str = "topic-a/comments";
    const COMMENTS_B: &str = "topic-b/comments";

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn board_id() -> ContentHash {
        ContentHash::of(b"board-object")
    }

    /// Owner 1, member 2; outsiders are any other seed.
    fn opened_server() -> AppendListServer {
        let mut server = AppendListServer::new();
        for attr in [TOPICS, COMMENTS_A, COMMENTS_B] {
            server
                .create_append_list(
                    attr,
                    key(1).verifying_key(),
                    &AttributeConfig {
                        members: vec![key(2).verifying_key()],
                        ..AttributeConfig::default()
                    },
                )
                .unwrap();
            server.open(attr, &key(1).verifying_key()).unwrap();
        }
        server
    }

    fn submit(server: &mut AppendListServer, attr: &str, object: &[u8]) -> u64 {
        let signer = key(2);
        let hash = ContentHash::of(object);
        let signature = signer.sign(&hash.0);
        match server.submit(attr, object, &hash, &signer.verifying_key(), &signature) {
            Ok(SubmitOutcome::Appended(seq)) => seq,
            other => panic!("member submit of {attr} failed: {other:?}"),
        }
    }

    fn submit_outsider(server: &mut AppendListServer, attr: &str, object: &[u8]) -> SubmitOutcome {
        let signer = key(5);
        let hash = ContentHash::of(object);
        let signature = signer.sign(&hash.0);
        server
            .submit(attr, object, &hash, &signer.verifying_key(), &signature)
            .unwrap()
    }

    fn read(server: &AppendListServer, attr: &str) -> Vec<Entry> {
        server.read(attr, None, None).unwrap().to_vec()
    }

    /// The standard fixture: topics a and b; under a, comments c1..c5
    /// with c1, c2 top-level, c3 and c4 under c1, c5 under c3.
    struct Fixture {
        server: AppendListServer,
        topic_a: ContentHash,
        topic_b: ContentHash,
        comment: BTreeMap<&'static str, ContentHash>,
        parents: BTreeMap<ContentHash, ContentHash>,
    }

    fn fixture() -> Fixture {
        let mut server = opened_server();
        submit(&mut server, TOPICS, b"topic-a");
        submit(&mut server, TOPICS, b"topic-b");
        for name in ["c1", "c2", "c3", "c4", "c5"] {
            submit(&mut server, COMMENTS_A, name.as_bytes());
        }
        let topic_a = ContentHash::of(b"topic-a");
        let topic_b = ContentHash::of(b"topic-b");
        let comment: BTreeMap<&'static str, ContentHash> = ["c1", "c2", "c3", "c4", "c5"]
            .iter()
            .map(|name| (*name, ContentHash::of(name.as_bytes())))
            .collect();
        let mut parents = BTreeMap::new();
        parents.insert(comment["c1"], topic_a);
        parents.insert(comment["c2"], topic_a);
        parents.insert(comment["c3"], comment["c1"]);
        parents.insert(comment["c4"], comment["c1"]);
        parents.insert(comment["c5"], comment["c3"]);
        Fixture {
            server,
            topic_a,
            topic_b,
            comment,
            parents,
        }
    }

    fn view(fixture: &Fixture, tombstones: &TombstoneSet, config: &BoardViewConfig) -> BoardView {
        let topics = read(&fixture.server, TOPICS);
        let attr_of = |hash: &ContentHash| {
            if *hash == fixture.topic_a {
                COMMENTS_A
            } else {
                COMMENTS_B
            }
        };
        build_board_view(
            &board_id(),
            &topics,
            &|hash| read(&fixture.server, attr_of(hash)),
            &|hash| fixture.parents.get(hash).copied(),
            tombstones,
            1,
            1_000,
            config,
            None,
        )
    }

    /// Sequences of a comment subtree in pre-order.
    fn seqs(node: &CommentNode) -> Vec<u64> {
        let mut out = vec![node.seq];
        for child in &node.children {
            out.extend(seqs(child));
        }
        out
    }

    #[test]
    fn nested_comments_render_in_append_order_at_each_level() {
        let fixture = fixture();
        let view = view(&fixture, &TombstoneSet::new(), &BoardViewConfig::default());
        assert_eq!(view.topics.len(), 2);
        assert_eq!(view.topics[0].seq, 0);
        assert_eq!(view.topics[1].seq, 1);
        let topic = &view.topics[0];
        assert!(!topic.folded);
        // Top level: c1 (seq 0), c2 (seq 1).
        assert_eq!(
            topic.comments.iter().map(|c| c.seq).collect::<Vec<_>>(),
            vec![0, 1]
        );
        // Under c1: c3 (seq 2), c4 (seq 3), in append order.
        assert_eq!(
            topic.comments[0]
                .children
                .iter()
                .map(|c| c.seq)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        // Under c3: c5 (seq 4).
        assert_eq!(
            topic.comments[0].children[0]
                .children
                .iter()
                .map(|c| c.seq)
                .collect::<Vec<_>>(),
            vec![4]
        );
        assert_eq!(seqs(&topic.comments[0]), vec![0, 2, 4, 3]);
        assert_eq!(view.next_cursor, None);
    }

    #[test]
    fn tombstoned_comment_hides_its_subtree_without_renumbering() {
        let fixture = fixture();
        let mut tombstones = TombstoneSet::new();
        tombstones.hide(fixture.comment["c3"]);
        let view = view(&fixture, &tombstones, &BoardViewConfig::default());
        let topic = &view.topics[0];
        // c3 and its child c5 are gone; c1 keeps seq 0 with only c4
        // (seq 3), c2 keeps seq 1. No sequence moved.
        assert_eq!(seqs(&topic.comments[0]), vec![0, 3]);
        assert_eq!(topic.comments[1].seq, 1);
        assert_eq!(view.topics[1].seq, 1);

        // Hiding a top-level comment hides its whole subtree.
        let mut tombstones = TombstoneSet::new();
        tombstones.hide(fixture.comment["c1"]);
        let view = view(&fixture, &tombstones, &BoardViewConfig::default());
        assert_eq!(
            view.topics[0]
                .comments
                .iter()
                .map(|c| c.seq)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn folded_subtree_token_checks_nothing_new_then_delta() {
        let mut fixture = fixture();
        let tombstones = TombstoneSet::new();
        let first = view(&fixture, &tombstones, &BoardViewConfig::default());
        let token = first.topics[0].token.clone();
        // Unchanged lists: the rebuilt view mints the same frontier.
        let same = view(&fixture, &tombstones, &BoardViewConfig::default());
        assert_eq!(
            check_freshness(
                &token,
                &topic_view_id(&board_id(), &fixture.topic_a, 1),
                same.topics[0].token.frontier()
            ),
            CheckVerdict::NothingNew
        );
        // A new comment lands: the frontier advances past the token.
        submit(&mut fixture.server, COMMENTS_A, b"c6");
        let mut parents = fixture.parents.clone();
        parents.insert(ContentHash::of(b"c6"), fixture.topic_a);
        fixture.parents = parents;
        let after = view(&fixture, &tombstones, &BoardViewConfig::default());
        assert_eq!(
            check_freshness(
                &token,
                &topic_view_id(&board_id(), &fixture.topic_a, 1),
                after.topics[0].token.frontier()
            ),
            CheckVerdict::Delta {
                after: token.frontier.clone(),
            }
        );
        // The delta starts after the token's head sequence (4).
        assert_eq!(token.frontier[0].head_seq, Some(4));
        // A tombstone change is a resync, not a delta.
        let mut changed = TombstoneSet::new();
        changed.hide(fixture.comment["c2"]);
        let resync = view(&fixture, &changed, &BoardViewConfig::default());
        assert_eq!(
            check_freshness(
                &token,
                &topic_view_id(&board_id(), &fixture.topic_a, 1),
                resync.topics[0].token.frontier()
            ),
            CheckVerdict::Resync
        );
        // A token for another view is a resync too.
        assert_eq!(
            check_freshness(
                &token,
                &topic_view_id(&board_id(), &fixture.topic_b, 1),
                same.topics[0].token.frontier()
            ),
            CheckVerdict::Resync
        );
    }

    #[test]
    fn pending_submissions_never_appear() {
        let mut fixture = fixture();
        assert!(matches!(
            submit_outsider(&mut fixture.server, TOPICS, b"topic-pending"),
            SubmitOutcome::Pending(_)
        ));
        assert!(matches!(
            submit_outsider(&mut fixture.server, COMMENTS_A, b"c-pending"),
            SubmitOutcome::Pending(_)
        ));
        let view = view(&fixture, &TombstoneSet::new(), &BoardViewConfig::default());
        assert_eq!(view.topics.len(), 2);
        assert_eq!(view.topics[0].comments.len(), 2);
        assert_eq!(view.token.frontier[0].head_seq, Some(1));
    }

    #[test]
    fn pages_are_bounded_and_continued_by_cursor() {
        let mut server = opened_server();
        for n in 0..5 {
            submit(&mut server, TOPICS, format!("t{n}").as_bytes());
        }
        let topics = read(&server, TOPICS);
        let config = BoardViewConfig::with_page_size(2);
        let mut seen = Vec::new();
        let mut cursor = None;
        for _ in 0..6 {
            let page = build_board_view(
                &board_id(),
                &topics,
                &|_| Vec::new(),
                &|_| None,
                &TombstoneSet::new(),
                1,
                1_000,
                &config,
                cursor.as_ref(),
            );
            assert!(page.topics.len() <= 2);
            seen.extend(page.topics.iter().map(|t| t.seq));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, vec![0, 1, 2, 3, 4]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn parent_ref_cycles_render_as_back_references() {
        let mut server = opened_server();
        submit(&mut server, TOPICS, b"topic-a");
        submit(&mut server, COMMENTS_A, b"x");
        submit(&mut server, COMMENTS_A, b"y");
        let x = ContentHash::of(b"x");
        let y = ContentHash::of(b"y");
        let mut parents = BTreeMap::new();
        parents.insert(x, y);
        parents.insert(y, x);
        let topics = read(&server, TOPICS);
        let view = build_board_view(
            &board_id(),
            &topics,
            &|_| read(&server, COMMENTS_A),
            &|hash| parents.get(hash).copied(),
            &TombstoneSet::new(),
            1,
            1_000,
            &BoardViewConfig::default(),
            None,
        );
        // The orphan cycle is emitted top-level; the walk terminates
        // with a back-reference instead of recursing forever.
        let top = &view.topics[0].comments[0];
        assert_eq!(top.hash, x);
        assert_eq!(top.children.len(), 1);
        assert_eq!(top.children[0].hash, y);
        assert_eq!(top.children[0].children.len(), 1);
        assert_eq!(
            top.children[0].children[0].back_reference_to,
            Some(x)
        );
    }

    #[test]
    fn depth_beyond_the_cap_folds_with_a_token() {
        let mut server = opened_server();
        submit(&mut server, TOPICS, b"topic-a");
        // A chain c0 <- c1 <- ... <- c4, five deep.
        let mut parents = BTreeMap::new();
        let topic_a = ContentHash::of(b"topic-a");
        let mut previous = topic_a;
        for n in 0..5 {
            let name = format!("c{n}");
            submit(&mut server, COMMENTS_A, name.as_bytes());
            let hash = ContentHash::of(name.as_bytes());
            parents.insert(hash, previous);
            previous = hash;
        }
        let topics = read(&server, TOPICS);
        let config = BoardViewConfig {
            max_depth: 2,
            ..BoardViewConfig::default()
        };
        let view = build_board_view(
            &board_id(),
            &topics,
            &|_| read(&server, COMMENTS_A),
            &|hash| parents.get(hash).copied(),
            &TombstoneSet::new(),
            1,
            1_000,
            &config,
            None,
        );
        let top = &view.topics[0].comments[0];
        assert_eq!(top.seq, 0);
        assert!(!top.folded);
        let second = &top.children[0];
        assert_eq!(second.seq, 1);
        // At the depth cap: rendered, but its children are folded.
        assert!(second.folded);
        assert!(second.children.is_empty());
        assert_eq!(second.token.frontier[0].head_seq, Some(4));
    }

    #[test]
    fn node_budget_truncates_a_large_topic_and_folds_the_remainder() {
        let fixture = fixture();
        let config = BoardViewConfig::with_page_size(3);
        let view = view(&fixture, &TombstoneSet::new(), &config);
        // 1 topic node + 2 comment nodes fill the page of 3.
        let topic = &view.topics[0];
        assert!(topic.folded);
        assert_eq!(topic.comments.len(), 2);
        assert_eq!(topic.token.frontier[0].head_seq, Some(1));
        // The remainder is a delta from the token's head.
        assert_eq!(
            check_freshness(
                &topic.token,
                &topic_view_id(&board_id(), &fixture.topic_a, 1),
                view.topics[0].token.frontier()
            ),
            CheckVerdict::NothingNew
        );
        assert_eq!(view.next_cursor, None);
    }

    mod properties {
        use proptest::prelude::*;

        use super::*;

        /// Builds entries directly (no server): one comment per
        /// parent slot, with parents resolved against the full hash
        /// set so forward references and cycles are possible.
        fn entries(parents: &[Option<usize>]) -> (Vec<Entry>, BTreeMap<ContentHash, ContentHash>) {
            let signer = key(7);
            let topic = ContentHash::of(b"topic-a");
            let count = parents.len();
            let hashes: Vec<ContentHash> = (0..count)
                .map(|index| ContentHash::of(format!("c{index}").as_bytes()))
                .collect();
            let mut list = Vec::new();
            let mut map = BTreeMap::new();
            for (index, parent) in parents.iter().enumerate() {
                let hash = hashes[index];
                list.push(Entry {
                    seq: index as u64,
                    hash,
                    submitter: signer.verifying_key(),
                    signature: signer.sign(&hash.0),
                });
                if let Some(parent) = parent {
                    let parent_hash = if *parent == usize::MAX || *parent >= count {
                        topic
                    } else {
                        hashes[*parent]
                    };
                    map.insert(hash, parent_hash);
                }
            }
            (list, map)
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]

            /// Oracle: the rendered tree is always a subsequence of the
            /// input in append order — no entry moves, no sequence is
            /// renumbered, no tombstoned entry renders, depth and node
            /// counts stay within the §7 bounds, and the build is
            /// deterministic.
            #[test]
            fn projection_keeps_order_hides_tombstones_and_stays_bounded(
                parents in proptest::collection::vec(
                    prop_oneof![Just(None), Just(Some(usize::MAX)), (0usize..8).prop_map(Some)],
                    0..9,
                ),
                hidden in proptest::collection::vec(0usize..8, 0..3),
            ) {
                let count = parents.len();
                let (list, map) = entries(&parents);
                let mut tombstones = TombstoneSet::new();
                for index in &hidden {
                    if *index < count {
                        tombstones.hide(ContentHash::of(format!("c{index}").as_bytes()));
                    }
                }
                let config = BoardViewConfig { max_depth: 3, ..BoardViewConfig::default() };
                let topic = Entry {
                    seq: 0,
                    hash: ContentHash::of(b"topic-a"),
                    submitter: key(7).verifying_key(),
                    signature: key(7).sign(&ContentHash::of(b"topic-a").0),
                };
                let build = |tombstones: &TombstoneSet| {
                    build_board_view(
                        &board_id(),
                        std::slice::from_ref(&topic),
                        &|_| list.clone(),
                        &|hash| map.get(hash).copied(),
                        tombstones,
                        1,
                        1_000,
                        &config,
                        None,
                    )
                };
                let first = build(&tombstones);
                let second = build(&tombstones);
                prop_assert_eq!(&first, &second);

                // Collect rendered sequences in pre-order.
                fn collect(node: &CommentNode, out: &mut Vec<u64>) {
                    out.push(node.seq);
                    for child in &node.children {
                        collect(child, out);
                    }
                }
                let mut rendered = Vec::new();
                for comment in &first.topics[0].comments {
                    collect(comment, &mut rendered);
                }
                // Every rendered sequence is a real input entry, shown
                // once: no moves, no renumbering, no duplicates.
                let seen: BTreeSet<u64> = rendered.iter().copied().collect();
                prop_assert_eq!(seen.len(), rendered.len());
                prop_assert!(rendered.iter().all(|seq| (*seq as usize) < count));
                // Siblings stay in append order at every level.
                fn siblings_ordered(nodes: &[CommentNode]) -> bool {
                    nodes.windows(2).all(|pair| pair[0].seq < pair[1].seq)
                        && nodes.iter().all(|node| siblings_ordered(&node.children))
                }
                prop_assert!(siblings_ordered(&first.topics[0].comments));
                // No tombstoned entry renders, at any depth.
                for seq in &rendered {
                    let hash = ContentHash::of(format!("c{seq}").as_bytes());
                    prop_assert!(!tombstones.is_hidden(hash));
                }
                // Depth and node bounds hold.
                fn depth(node: &CommentNode) -> usize {
                    1 + node.children.iter().map(depth).max().unwrap_or(0)
                }
                for comment in &first.topics[0].comments {
                    prop_assert!(depth(comment) <= 3);
                }
                prop_assert!(rendered.len() + 1 <= config.max_nodes_per_page);
            }
        }
    }
}
