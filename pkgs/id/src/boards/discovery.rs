//! Authority-filtered discovery shelves: slice 2 of #7585.
//!
//! Oracle: `doc/2026-11-08_design_id5862.md`, Decision 2. Discovery
//! shows a viewer exactly three shelves, in this order:
//!
//! 1. **Following** — boards the viewer subscribes to (slice 1's
//!    [`SubscriptionStore`]), most recent activity first.
//! 2. **Shared with you** — boards the viewer's read authority reaches
//!    that they do not follow and that are not public-directory boards.
//! 3. **Public directory** — boards whose owner has opted in with
//!    [`Discoverability::Public`]; the default for every board is
//!    not-public.
//!
//! A board appears on at most one shelf: the first shelf, in the order
//! above, whose criteria it meets. Every entry on every shelf passes a
//! render-time read-authority check through [`ReadAuthority`] — this
//! builder is a render surface, so it consults the authority source on
//! every call and caches nothing. A board the viewer cannot read
//! surfaces nothing, and a post the viewer cannot read never appears
//! even as a teaser: no title, no excerpt, no thumbnail.
//!
//! Ranking inside a shelf is recency only (latest post/share time).
//! There is no engagement ranking, no trending, and no personalised
//! recommendation; ties in activity time are not a ranking signal and
//! break by board hash purely so the output order is stable.
//!
//! Unfollow removes a board from the Following shelf only: a board the
//! viewer can still read may remain on Shared with you or in the public
//! directory, because unfollow is a reading preference, not an
//! authority change. Muting likewise never removes a board's listing;
//! the Following entry just carries the subscription's muted flag.
//!
//! The builder renders only from the [`BoardDiscoveryFacts`] it is
//! given: a followed board with no facts on this host has nothing to
//! render and does not appear. It invents no entries and widens no
//! authority — public discoverability alone never lists a board the
//! viewer may not read.

use std::collections::BTreeMap;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{ArtifactHash, SubscriptionStore, is_public_key};

/// Whether a board's owner has opted it into the public directory.
///
/// Public is an explicit per-board opt-in by the owner; the default
/// for every board is not-public (Decision 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Discoverability {
    /// Not listed in the public directory (the default).
    #[default]
    NotPublic,
    /// Owner opted this board into the public directory.
    Public,
}

/// A candidate latest-post teaser for a shelf entry.
///
/// The preview is only ever *candidate* data: the builder attaches it
/// to an entry solely when the viewer may read that post, so an
/// unreadable post contributes nothing — no title, no excerpt, no
/// thumbnail (Decision 2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryPostPreview {
    /// The post object the teaser points at.
    pub post: ArtifactHash,
    /// Post title as it would render in the teaser.
    pub title: String,
}

/// Per-board facts the discovery builder renders from.
///
/// Supplied by the caller from the board graph and post store at
/// render time; the builder holds no copy between renders, so
/// unfollow, mute, revocation, and deletion take effect on the next
/// build by construction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardDiscoveryFacts {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Owner's public-directory opt-in for this board.
    #[serde(default)]
    pub discoverability: Discoverability,
    /// Latest post time on the board (Unix seconds), if it has posts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_post_at: Option<u64>,
    /// Latest share into the board (Unix seconds), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_share_at: Option<u64>,
    /// The board's latest post, as a candidate teaser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_post: Option<DiscoveryPostPreview>,
}

impl BoardDiscoveryFacts {
    /// Recency key for shelf ranking: the latest post or share time.
    /// A board with neither counts as activity time 0.
    pub fn last_activity_at(&self) -> u64 {
        self.last_post_at.max(self.last_share_at).unwrap_or(0)
    }
}

/// Render-time read-authority source for one build.
///
/// Implementations answer from the live authority state (grants,
/// shares, ownership) at the moment of the call: the check happens at
/// render time, every render, so a lapsed authority removes the entry
/// on the next build with no cleanup pass.
pub trait ReadAuthority {
    /// Whether `viewer` may read the board itself.
    fn can_read_board(&self, viewer: &str, board: &ArtifactHash) -> bool;
    /// Whether `viewer` may read one post (the teaser check).
    fn can_read_post(&self, viewer: &str, post: &ArtifactHash) -> bool;
}

/// One board's rendered shelf entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryEntry {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Latest post/share time used for this shelf's recency ranking.
    pub last_activity_at: u64,
    /// The subscription's muted flag (Following shelf only; muting
    /// never removes the listing). Always false on other shelves.
    #[serde(default)]
    pub muted: bool,
    /// Latest-post teaser, present only when the viewer may read that
    /// post; an unreadable post leaves no teaser behind at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_post: Option<DiscoveryPostPreview>,
}

/// The three discovery shelves, in Decision 2 order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryShelves {
    /// Shelf 1: boards the viewer subscribes to and may read.
    pub following: Vec<DiscoveryEntry>,
    /// Shelf 2: non-public boards the viewer's read authority reaches
    /// that they do not follow.
    pub shared_with_you: Vec<DiscoveryEntry>,
    /// Shelf 3: owner-opted-in public boards the viewer does not
    /// follow and may read.
    pub public_directory: Vec<DiscoveryEntry>,
}

impl DiscoveryShelves {
    /// The shelves in their fixed Decision 2 order: Following, Shared
    /// with you, Public directory.
    pub fn in_order(&self) -> [&Vec<DiscoveryEntry>; 3] {
        [
            &self.following,
            &self.shared_with_you,
            &self.public_directory,
        ]
    }
}

/// Build the three discovery shelves for one viewer (Decision 2).
///
/// `boards` carries one facts record per candidate board; a duplicate
/// record for the same board is ignored after the first. Every
/// candidate passes the render-time authority check before it can
/// appear anywhere, followed or not, public or not.
pub fn build_discovery(
    viewer: &str,
    subscriptions: &SubscriptionStore,
    boards: &[BoardDiscoveryFacts],
    authority: &dyn ReadAuthority,
) -> Result<DiscoveryShelves> {
    ensure!(
        is_public_key(viewer),
        "discovery viewer is not a canonical Ed25519 key"
    );
    let mut facts_by_board: BTreeMap<&ArtifactHash, &BoardDiscoveryFacts> = BTreeMap::new();
    for facts in boards {
        facts_by_board.entry(&facts.board).or_insert(facts);
    }

    let mut shelves = DiscoveryShelves::default();
    for facts in facts_by_board.values() {
        // Render-time read-authority check: no shelf entry exists for
        // a board the viewer cannot read, whatever its discoverability
        // or subscription state.
        if !authority.can_read_board(viewer, &facts.board) {
            continue;
        }
        let subscription = subscriptions.get(viewer, &facts.board);
        let latest_post = facts
            .latest_post
            .clone()
            .filter(|preview| authority.can_read_post(viewer, &preview.post));
        let entry = DiscoveryEntry {
            board: facts.board.clone(),
            last_activity_at: facts.last_activity_at(),
            muted: subscription.map(|s| s.muted).unwrap_or(false),
            latest_post,
        };
        if subscription.is_some() {
            shelves.following.push(entry);
        } else if facts.discoverability == Discoverability::Public {
            shelves.public_directory.push(entry);
        } else {
            shelves.shared_with_you.push(entry);
        }
    }

    // Recency only: newest activity first. Activity ties carry no
    // ranking signal; board hash breaks them for a stable order.
    let by_recency = |a: &DiscoveryEntry, b: &DiscoveryEntry| {
        b.last_activity_at
            .cmp(&a.last_activity_at)
            .then_with(|| a.board.cmp(&b.board))
    };
    shelves.following.sort_by(by_recency);
    shelves.shared_with_you.sort_by(by_recency);
    shelves.public_directory.sort_by(by_recency);
    Ok(shelves)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod discovery_tests {
    //! Shelf tests naming `doc/2026-11-08_design_id5862.md` Decision 2
    //! as oracle.

    use std::collections::BTreeSet;

    use super::*;

    fn hash(ch: char) -> ArtifactHash {
        ArtifactHash::new(ch.to_string().repeat(64)).unwrap()
    }

    fn viewer() -> String {
        "e".repeat(64)
    }

    /// Set-based authority stub: exactly the boards/posts in the sets
    /// are readable by anyone; everything else is not.
    #[derive(Default)]
    struct StubAuthority {
        boards: BTreeSet<ArtifactHash>,
        posts: BTreeSet<ArtifactHash>,
    }

    impl ReadAuthority for StubAuthority {
        fn can_read_board(&self, _viewer: &str, board: &ArtifactHash) -> bool {
            self.boards.contains(board)
        }
        fn can_read_post(&self, _viewer: &str, post: &ArtifactHash) -> bool {
            self.posts.contains(post)
        }
    }

    fn facts(
        board: ArtifactHash,
        discoverability: Discoverability,
        last_post_at: Option<u64>,
        last_share_at: Option<u64>,
    ) -> BoardDiscoveryFacts {
        BoardDiscoveryFacts {
            board,
            discoverability,
            last_post_at,
            last_share_at,
            latest_post: None,
        }
    }

    fn boards_of(entries: &[DiscoveryEntry]) -> Vec<ArtifactHash> {
        entries.iter().map(|e| e.board.clone()).collect()
    }

    #[test]
    fn oracle_exactly_three_shelves_in_order_ranked_by_recency_only() {
        // Oracle Decision 2: exactly three shelves in the order
        // Following / Shared with you / Public directory; ranking
        // inside a shelf is recency only (latest post/share time).
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (old_follow, new_follow) = (hash('a'), hash('b'));
        store.follow(&viewer, &old_follow, 1).unwrap();
        store.follow(&viewer, &new_follow, 2).unwrap();
        let (shared, public) = (hash('c'), hash('d'));
        let authority = StubAuthority {
            boards: [
                old_follow.clone(),
                new_follow.clone(),
                shared.clone(),
                public.clone(),
            ]
            .into_iter()
            .collect(),
            posts: BTreeSet::new(),
        };
        let boards = vec![
            facts(
                old_follow.clone(),
                Discoverability::NotPublic,
                Some(100),
                None,
            ),
            facts(
                new_follow.clone(),
                Discoverability::NotPublic,
                Some(300),
                None,
            ),
            facts(shared.clone(), Discoverability::NotPublic, Some(200), None),
            facts(public.clone(), Discoverability::Public, Some(400), None),
        ];
        let shelves = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        let ordered = shelves.in_order();
        assert_eq!(boards_of(ordered[0]), vec![new_follow, old_follow]);
        assert_eq!(boards_of(ordered[1]), vec![shared]);
        assert_eq!(boards_of(ordered[2]), vec![public]);
    }

    #[test]
    fn oracle_followed_but_unreadable_board_surfaces_nothing() {
        // Oracle Decision 1 + Decision 2: a subscription to a board
        // the subscriber cannot read is allowed to exist but surfaces
        // nothing until read authority exists — it is on no shelf.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer, &board, 1).unwrap();
        let authority = StubAuthority::default();
        let boards = vec![facts(board, Discoverability::NotPublic, Some(100), None)];
        let shelves = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        assert!(shelves.following.is_empty());
        assert!(shelves.shared_with_you.is_empty());
        assert!(shelves.public_directory.is_empty());
    }

    #[test]
    fn oracle_unreadable_post_never_appears_even_as_a_teaser() {
        // Oracle Decision 2: discovery never surfaces a post the
        // viewer cannot read, even as a teaser — no title, no
        // excerpt, no thumbnail. The board entry itself remains.
        let viewer = viewer();
        let store = SubscriptionStore::new();
        let (board, post) = (hash('a'), hash('f'));
        let mut board_facts = facts(board.clone(), Discoverability::NotPublic, Some(100), None);
        board_facts.latest_post = Some(DiscoveryPostPreview {
            post: post.clone(),
            title: "secret title".to_owned(),
        });
        let authority = StubAuthority {
            boards: [board].into_iter().collect(),
            posts: BTreeSet::new(),
        };
        let shelves = build_discovery(&viewer, &store, &[board_facts], &authority).unwrap();
        assert_eq!(shelves.shared_with_you.len(), 1);
        let entry = &shelves.shared_with_you[0];
        assert!(entry.latest_post.is_none());
        let rendered = serde_json::to_string(entry).unwrap();
        assert!(
            !rendered.contains("secret title"),
            "unreadable post title leaked into entry: {rendered}"
        );
    }

    #[test]
    fn oracle_readable_latest_post_renders_as_teaser() {
        // Oracle Decision 2: the teaser ban is about unreadable
        // posts; a readable latest post renders its preview.
        let viewer = viewer();
        let store = SubscriptionStore::new();
        let (board, post) = (hash('a'), hash('f'));
        let mut board_facts = facts(board.clone(), Discoverability::Public, Some(100), None);
        board_facts.latest_post = Some(DiscoveryPostPreview {
            post: post.clone(),
            title: "hello board".to_owned(),
        });
        let authority = StubAuthority {
            boards: [board].into_iter().collect(),
            posts: [post].into_iter().collect(),
        };
        let shelves = build_discovery(&viewer, &store, &[board_facts], &authority).unwrap();
        assert_eq!(shelves.public_directory.len(), 1);
        let preview = shelves.public_directory[0].latest_post.as_ref().unwrap();
        assert_eq!(preview.title, "hello board");
    }

    #[test]
    fn oracle_unfollow_removes_from_following_only() {
        // Oracle Decision 2: unfollowing removes a board from the
        // Following shelf only; a board the viewer can still read may
        // still appear in Shared with you or the public directory.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (shared_board, public_board) = (hash('a'), hash('b'));
        store.follow(&viewer, &shared_board, 1).unwrap();
        store.follow(&viewer, &public_board, 2).unwrap();
        let authority = StubAuthority {
            boards: [shared_board.clone(), public_board.clone()]
                .into_iter()
                .collect(),
            posts: BTreeSet::new(),
        };
        let boards = vec![
            facts(
                shared_board.clone(),
                Discoverability::NotPublic,
                Some(100),
                None,
            ),
            facts(
                public_board.clone(),
                Discoverability::Public,
                Some(100),
                None,
            ),
        ];
        let before = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        assert_eq!(before.following.len(), 2);
        assert!(store.unfollow(&viewer, &shared_board));
        assert!(store.unfollow(&viewer, &public_board));
        let after = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        assert!(after.following.is_empty());
        assert_eq!(boards_of(&after.shared_with_you), vec![shared_board]);
        assert_eq!(boards_of(&after.public_directory), vec![public_board]);
    }

    #[test]
    fn oracle_mute_does_not_remove_a_boards_listing() {
        // Oracle Decision 2: muting does not remove a board's
        // listing. The Following entry stays and carries muted=true.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer, &board, 1).unwrap();
        store.set_muted(&viewer, &board, true).unwrap();
        let authority = StubAuthority {
            boards: [board.clone()].into_iter().collect(),
            posts: BTreeSet::new(),
        };
        let boards = vec![facts(board, Discoverability::NotPublic, Some(100), None)];
        let shelves = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        assert_eq!(shelves.following.len(), 1);
        assert!(shelves.following[0].muted);
    }

    #[test]
    fn oracle_public_directory_is_owner_opt_in_default_not_public() {
        // Oracle Decision 2: public is an explicit per-board opt-in
        // by the owner; the default for every board is not-public, so
        // a default board the viewer can read lands on Shared with
        // you, never in the directory.
        let viewer = viewer();
        let store = SubscriptionStore::new();
        let board = hash('a');
        let authority = StubAuthority {
            boards: [board.clone()].into_iter().collect(),
            posts: BTreeSet::new(),
        };
        let boards = vec![facts(board, Discoverability::default(), Some(100), None)];
        let shelves = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        assert!(shelves.public_directory.is_empty());
        assert_eq!(shelves.shared_with_you.len(), 1);
    }

    #[test]
    fn oracle_public_board_unreadable_to_viewer_is_not_listed() {
        // Oracle Decision 2 + packet acceptance: every shelf entry
        // passes a render-time read-authority check. Opting a board
        // public does not widen this viewer's authority by itself.
        let viewer = viewer();
        let store = SubscriptionStore::new();
        let board = hash('a');
        let authority = StubAuthority::default();
        let boards = vec![facts(board, Discoverability::Public, Some(100), None)];
        let shelves = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        assert!(shelves.public_directory.is_empty());
        assert!(shelves.shared_with_you.is_empty());
        assert!(shelves.following.is_empty());
    }

    #[test]
    fn oracle_recency_uses_latest_share_or_post_and_ties_are_not_a_ranking_signal() {
        // Oracle Decision 2: ranking is the latest post/share time.
        // Equal activity times order by board hash only so the output
        // is stable — no other signal reorders a shelf.
        let viewer = viewer();
        let store = SubscriptionStore::new();
        let (b_board, a_board, shared_late) = (hash('b'), hash('a'), hash('c'));
        let authority = StubAuthority {
            boards: [b_board.clone(), a_board.clone(), shared_late.clone()]
                .into_iter()
                .collect(),
            posts: BTreeSet::new(),
        };
        let boards = vec![
            // Post long ago, shared recently: share time ranks it top.
            facts(
                shared_late.clone(),
                Discoverability::NotPublic,
                Some(10),
                Some(500),
            ),
            facts(b_board.clone(), Discoverability::NotPublic, Some(100), None),
            facts(a_board.clone(), Discoverability::NotPublic, Some(100), None),
        ];
        let shelves = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        assert_eq!(
            boards_of(&shelves.shared_with_you),
            vec![shared_late, a_board, b_board]
        );
    }

    #[test]
    fn oracle_board_appears_on_at_most_one_shelf() {
        // Oracle Decision 2: the shelves are one listing. A followed
        // public board is on Following, not duplicated into the
        // public directory.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer, &board, 1).unwrap();
        let authority = StubAuthority {
            boards: [board.clone()].into_iter().collect(),
            posts: BTreeSet::new(),
        };
        let boards = vec![facts(
            board.clone(),
            Discoverability::Public,
            Some(100),
            None,
        )];
        let shelves = build_discovery(&viewer, &store, &boards, &authority).unwrap();
        assert_eq!(boards_of(&shelves.following), vec![board]);
        assert!(shelves.shared_with_you.is_empty());
        assert!(shelves.public_directory.is_empty());
    }

    #[test]
    fn oracle_malformed_viewer_key_is_rejected() {
        let store = SubscriptionStore::new();
        let authority = StubAuthority::default();
        assert!(build_discovery("not-a-key", &store, &[], &authority).is_err());
        assert!(build_discovery("", &store, &[], &authority).is_err());
    }
}
