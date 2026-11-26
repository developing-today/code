//! Viewer-side chronological feed: slice 3 of #7585.
//!
//! Oracle: `doc/2026-11-08_design_id5862.md`, Decision 3. The feed is
//! computed for one viewer from their unmuted subscriptions plus the
//! boards they own; mute applies to that combined set, so a muted board
//! the viewer owns contributes nothing. It is a view, not a stored
//! stream: [`build_feed`] holds nothing between calls and the
//! [`FeedItem`]s it returns are references only (post hash, arrival
//! paths, times), so unfollow, mute, revocation and deletion take effect
//! on the next render with no cleanup pass, and no title, excerpt or
//! thumbnail is ever copied into the feed.
//!
//! Ordering is strictly chronological, newest first. An original post
//! arrives at its post time; a repost arrives at its repost time and
//! carries the attribution chain (original author, original board,
//! reposting board). The same post arriving through several source
//! boards is one item listing every arrival path; the item sits at its
//! newest visible arrival time. Equal times are not a ranking signal:
//! they break by post hash only so the output order is stable
//! (invariant 6).
//!
//! Read authority is checked at render time, every render, through the
//! same [`ReadAuthority`] the discovery shelves use: the viewer must be
//! able to read the post, and each arrival path must come through a
//! board the viewer can read. A path that fails the check is dropped; a
//! post that fails it, or that is left with no readable path, is not in
//! the feed at all — including positions the viewer scrolled past on an
//! earlier render (there is no "you saw it once, so it stays" rule).
//!
//! Playable posts never run inline: they render as their static
//! fallback with an "open on board" path to the post's original board
//! (the #5859 embed contract). Feed is not notification: `notify_mode`
//! plays no part here, and building a feed never creates or changes a
//! subscription (invariant 5) — the store is borrowed read-only.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{ArtifactHash, ReadAuthority, SubscriptionStore, is_public_key};

/// Attribution chain for a repost arrival (Decision 3). The reposting
/// board is the arrival's own `via_board`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepostAttribution {
    /// Canonical Ed25519 key (hex) of the post's original author.
    pub original_author: String,
    /// Board the post was originally made on.
    pub original_board: ArtifactHash,
}

/// One way a post reaches a source board: posted there, or reposted
/// into it. Supplied by the caller from the board graph at render time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedArrivalFacts {
    /// The post this arrival references.
    pub post: ArtifactHash,
    /// Board the post arrives through (its own board for an original
    /// post, the reposting board for a repost).
    pub via_board: ArtifactHash,
    /// Post time for an original, repost time for a repost (Unix
    /// seconds).
    pub arrived_at: u64,
    /// Present when this arrival is a repost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repost: Option<RepostAttribution>,
    /// Whether the post is playable (renders as its static fallback in
    /// the feed).
    #[serde(default)]
    pub playable: bool,
}

/// One arrival path of a rendered feed item.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedArrival {
    /// Board the post arrived through.
    pub via_board: ArtifactHash,
    /// Post or repost time of this path (Unix seconds).
    pub at: u64,
    /// Attribution chain when this path is a repost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repost: Option<RepostAttribution>,
}

/// How a feed item renders (Decision 3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedRender {
    /// A reference to the post, rendered under the #5856 post
    /// semantics (live reference or snapshot). The feed keeps no copy.
    Reference,
    /// A playable post: its static fallback, with an "open on board"
    /// path. Live embeds run only on the board surface itself.
    StaticFallback {
        /// The board where the post runs live.
        open_on_board: ArtifactHash,
    },
}

/// One rendered feed item: a reference to a post plus every readable
/// path it arrived by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedItem {
    /// The referenced post.
    pub post: ArtifactHash,
    /// Feed position time: the newest readable arrival (Unix seconds).
    pub at: u64,
    /// Every readable arrival path, newest first.
    pub arrivals: Vec<FeedArrival>,
    /// How the item renders.
    pub render: FeedRender,
}

/// The boards whose posts feed this viewer: unmuted subscriptions plus
/// owned boards, minus any board the viewer has muted (Decision 3).
fn feed_sources(
    viewer: &str,
    subscriptions: &SubscriptionStore,
    owned_boards: &[ArtifactHash],
) -> BTreeSet<ArtifactHash> {
    let mut sources: BTreeSet<ArtifactHash> = subscriptions
        .subscriptions_for(viewer)
        .into_iter()
        .filter(|subscription| !subscription.muted)
        .map(|subscription| subscription.board.clone())
        .collect();
    for board in owned_boards {
        // Mute exclusions apply to the combined set: a muted
        // subscription to a board the viewer owns removes it too.
        let muted = subscriptions
            .get(viewer, board)
            .is_some_and(|subscription| subscription.muted);
        if !muted {
            sources.insert(board.clone());
        }
    }
    sources
}

/// The board a playable post runs live on: its original board. An
/// original arrival names it directly; when only reposts are visible,
/// their attribution chain names it.
fn original_board(arrivals: &[FeedArrival]) -> Option<ArtifactHash> {
    arrivals
        .iter()
        .find(|arrival| arrival.repost.is_none())
        .map(|arrival| arrival.via_board.clone())
        .or_else(|| {
            arrivals
                .iter()
                .find_map(|arrival| arrival.repost.as_ref())
                .map(|repost| repost.original_board.clone())
        })
}

/// Build one viewer's feed (Decision 3).
///
/// `owned_boards` lists the boards the viewer owns; `arrivals` carries
/// every candidate arrival the caller knows of. Arrivals through boards
/// outside the viewer's feed sources are ignored, and every surviving
/// item passes the render-time read-authority check. Exact duplicate
/// arrivals collapse to one path.
pub fn build_feed(
    viewer: &str,
    subscriptions: &SubscriptionStore,
    owned_boards: &[ArtifactHash],
    arrivals: &[FeedArrivalFacts],
    authority: &dyn ReadAuthority,
) -> Result<Vec<FeedItem>> {
    ensure!(
        is_public_key(viewer),
        "feed viewer is not a canonical Ed25519 key"
    );
    for arrival in arrivals {
        if let Some(repost) = &arrival.repost {
            ensure!(
                is_public_key(&repost.original_author),
                "repost original author is not a canonical Ed25519 key"
            );
        }
    }
    let sources = feed_sources(viewer, subscriptions, owned_boards);

    // Group readable arrival paths by post. The board check runs per
    // path, the post check once per post, both at render time.
    let mut by_post: BTreeMap<&ArtifactHash, (BTreeSet<FeedArrival>, bool)> = BTreeMap::new();
    let mut post_readable: BTreeMap<&ArtifactHash, bool> = BTreeMap::new();
    for arrival in arrivals {
        if !sources.contains(&arrival.via_board)
            || !authority.can_read_board(viewer, &arrival.via_board)
        {
            continue;
        }
        let readable = *post_readable
            .entry(&arrival.post)
            .or_insert_with(|| authority.can_read_post(viewer, &arrival.post));
        if !readable {
            continue;
        }
        let entry = by_post
            .entry(&arrival.post)
            .or_insert_with(|| (BTreeSet::new(), false));
        entry.0.insert(FeedArrival {
            via_board: arrival.via_board.clone(),
            at: arrival.arrived_at,
            repost: arrival.repost.clone(),
        });
        entry.1 |= arrival.playable;
    }

    let mut items: Vec<FeedItem> = by_post
        .into_iter()
        .map(|(post, (paths, playable))| {
            let mut arrivals: Vec<FeedArrival> = paths.into_iter().collect();
            arrivals.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.cmp(b)));
            let at = arrivals.first().map_or(0, |arrival| arrival.at);
            let render = match original_board(&arrivals) {
                Some(open_on_board) if playable => FeedRender::StaticFallback { open_on_board },
                _ => FeedRender::Reference,
            };
            FeedItem {
                post: post.clone(),
                at,
                arrivals,
                render,
            }
        })
        .collect();

    // Chronology honesty: newest first by post/repost time only; equal
    // times break by post hash so the order is stable, nothing more.
    items.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.post.cmp(&b.post)));
    Ok(items)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod feed_tests {
    //! Feed tests naming `doc/2026-11-08_design_id5862.md` Decision 3
    //! and invariants 1-6 as oracle.

    use std::collections::BTreeSet;

    use super::*;
    use crate::boards::NotifyMode;

    fn hash(ch: char) -> ArtifactHash {
        ArtifactHash::new(ch.to_string().repeat(64)).unwrap()
    }

    fn viewer() -> String {
        "e".repeat(64)
    }

    /// Set-based authority stub: exactly the boards/posts in the sets
    /// are readable; everything else is not.
    #[derive(Default)]
    struct StubAuthority {
        boards: BTreeSet<ArtifactHash>,
        posts: BTreeSet<ArtifactHash>,
    }

    impl StubAuthority {
        fn all(boards: &[&ArtifactHash], posts: &[&ArtifactHash]) -> Self {
            Self {
                boards: boards.iter().map(|b| (*b).clone()).collect(),
                posts: posts.iter().map(|p| (*p).clone()).collect(),
            }
        }
    }

    impl ReadAuthority for StubAuthority {
        fn can_read_board(&self, _viewer: &str, board: &ArtifactHash) -> bool {
            self.boards.contains(board)
        }
        fn can_read_post(&self, _viewer: &str, post: &ArtifactHash) -> bool {
            self.posts.contains(post)
        }
    }

    fn original(post: &ArtifactHash, board: &ArtifactHash, at: u64) -> FeedArrivalFacts {
        FeedArrivalFacts {
            post: post.clone(),
            via_board: board.clone(),
            arrived_at: at,
            repost: None,
            playable: false,
        }
    }

    fn repost(
        post: &ArtifactHash,
        via: &ArtifactHash,
        original_board: &ArtifactHash,
        at: u64,
    ) -> FeedArrivalFacts {
        FeedArrivalFacts {
            post: post.clone(),
            via_board: via.clone(),
            arrived_at: at,
            repost: Some(RepostAttribution {
                original_author: viewer(),
                original_board: original_board.clone(),
            }),
            playable: false,
        }
    }

    fn posts_of(items: &[FeedItem]) -> Vec<ArtifactHash> {
        items.iter().map(|item| item.post.clone()).collect()
    }

    #[test]
    fn oracle_sources_are_unmuted_subscriptions_plus_owned_boards() {
        // Oracle Decision 3: the feed draws from the viewer's
        // subscriptions plus boards they own; a board neither followed
        // nor owned contributes nothing even when readable.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (followed, owned, stranger) = (hash('a'), hash('b'), hash('c'));
        store.follow(&viewer, &followed, 1).unwrap();
        let (p1, p2, p3) = (hash('1'), hash('2'), hash('3'));
        let authority = StubAuthority::all(&[&followed, &owned, &stranger], &[&p1, &p2, &p3]);
        let arrivals = vec![
            original(&p1, &followed, 10),
            original(&p2, &owned, 20),
            original(&p3, &stranger, 30),
        ];
        let feed = build_feed(
            &viewer,
            &store,
            std::slice::from_ref(&owned),
            &arrivals,
            &authority,
        )
        .unwrap();
        assert_eq!(posts_of(&feed), vec![p2, p1]);
    }

    #[test]
    fn oracle_mute_applies_to_the_combined_set_including_owned_boards() {
        // Oracle Decision 3 + invariant 3: mute removes the board's
        // posts from the feed while the subscription stays; a muted
        // board the viewer owns contributes nothing either.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (followed, owned) = (hash('a'), hash('b'));
        store.follow(&viewer, &followed, 1).unwrap();
        store.follow(&viewer, &owned, 2).unwrap();
        let (p1, p2) = (hash('1'), hash('2'));
        let authority = StubAuthority::all(&[&followed, &owned], &[&p1, &p2]);
        let arrivals = vec![original(&p1, &followed, 10), original(&p2, &owned, 20)];
        let owned_boards = std::slice::from_ref(&owned);
        let before = build_feed(&viewer, &store, owned_boards, &arrivals, &authority).unwrap();
        assert_eq!(before.len(), 2);
        store.set_muted(&viewer, &followed, true).unwrap();
        store.set_muted(&viewer, &owned, true).unwrap();
        let after = build_feed(&viewer, &store, owned_boards, &arrivals, &authority).unwrap();
        assert!(
            after.is_empty(),
            "muted boards leaked into the feed: {after:?}"
        );
        assert!(store.is_subscribed(&viewer, &followed));
    }

    #[test]
    fn oracle_unfollow_removes_posts_on_next_render() {
        // Oracle invariant 3: unfollow removes the board's posts from
        // the feed, with no cleanup pass.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer, &board, 1).unwrap();
        let post = hash('1');
        let authority = StubAuthority::all(&[&board], &[&post]);
        let arrivals = vec![original(&post, &board, 10)];
        assert_eq!(
            build_feed(&viewer, &store, &[], &arrivals, &authority)
                .unwrap()
                .len(),
            1
        );
        assert!(store.unfollow(&viewer, &board));
        assert!(
            build_feed(&viewer, &store, &[], &arrivals, &authority)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn oracle_order_is_post_time_newest_first_ties_by_hash_only() {
        // Oracle invariant 6: feed order is post time; equal times
        // break by post hash purely for a stable order.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer, &board, 1).unwrap();
        let (p1, p2, p3) = (hash('1'), hash('2'), hash('3'));
        let authority = StubAuthority::all(&[&board], &[&p1, &p2, &p3]);
        let arrivals = vec![
            original(&p3, &board, 50),
            original(&p1, &board, 100),
            original(&p2, &board, 50),
        ];
        let feed = build_feed(&viewer, &store, &[], &arrivals, &authority).unwrap();
        assert_eq!(posts_of(&feed), vec![p1, p2, p3]);
        assert_eq!(
            feed.iter().map(|item| item.at).collect::<Vec<_>>(),
            vec![100, 50, 50]
        );
    }

    #[test]
    fn oracle_repost_surfaces_at_repost_time_with_attribution_chain() {
        // Oracle Decision 3: reposts surface at repost time with the
        // full attribution chain (original author, original board,
        // reposting board).
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (origin, reposter, other) = (hash('a'), hash('b'), hash('c'));
        store.follow(&viewer, &reposter, 1).unwrap();
        store.follow(&viewer, &other, 1).unwrap();
        let (old_post, mid_post) = (hash('1'), hash('2'));
        let authority = StubAuthority::all(&[&origin, &reposter, &other], &[&old_post, &mid_post]);
        let arrivals = vec![
            repost(&old_post, &reposter, &origin, 300),
            original(&mid_post, &other, 200),
        ];
        let feed = build_feed(&viewer, &store, &[], &arrivals, &authority).unwrap();
        assert_eq!(posts_of(&feed), vec![old_post, mid_post]);
        let top = &feed[0];
        assert_eq!(top.at, 300);
        assert_eq!(top.arrivals.len(), 1);
        assert_eq!(top.arrivals[0].via_board, reposter);
        let chain = top.arrivals[0].repost.as_ref().unwrap();
        assert_eq!(chain.original_board, origin);
        assert_eq!(chain.original_author, viewer);
    }

    #[test]
    fn oracle_same_post_via_several_boards_is_one_item_with_every_path() {
        // Oracle Decision 3: the same post arriving via several
        // followed boards appears once, with all arrival paths listed;
        // an exact duplicate arrival is one path.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (origin, reposter) = (hash('a'), hash('b'));
        store.follow(&viewer, &origin, 1).unwrap();
        store.follow(&viewer, &reposter, 1).unwrap();
        let post = hash('1');
        let authority = StubAuthority::all(&[&origin, &reposter], &[&post]);
        let arrivals = vec![
            original(&post, &origin, 100),
            repost(&post, &reposter, &origin, 400),
            original(&post, &origin, 100),
        ];
        let feed = build_feed(&viewer, &store, &[], &arrivals, &authority).unwrap();
        assert_eq!(feed.len(), 1);
        let item = &feed[0];
        assert_eq!(item.at, 400);
        assert_eq!(
            item.arrivals
                .iter()
                .map(|a| (a.via_board.clone(), a.at))
                .collect::<Vec<_>>(),
            vec![(reposter, 400), (origin, 100)]
        );
    }

    #[test]
    fn oracle_authority_is_checked_at_render_time_every_render() {
        // Oracle invariants 2 and 3: authority loss removes the item
        // on the next render — including one already seen — with no
        // residual metadata; an unreadable arrival path is dropped
        // while a readable one keeps the item.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (open, closed) = (hash('a'), hash('b'));
        store.follow(&viewer, &open, 1).unwrap();
        store.follow(&viewer, &closed, 1).unwrap();
        let (kept, lost) = (hash('1'), hash('2'));
        let arrivals = vec![
            original(&kept, &open, 10),
            repost(&kept, &closed, &open, 90),
            original(&lost, &open, 20),
        ];
        let mut authority = StubAuthority::all(&[&open], &[&kept, &lost]);
        let first = build_feed(&viewer, &store, &[], &arrivals, &authority).unwrap();
        assert_eq!(posts_of(&first), vec![lost.clone(), kept.clone()]);
        let kept_item = first.iter().find(|item| item.post == kept).unwrap();
        assert_eq!(kept_item.arrivals.len(), 1, "unreadable path leaked");
        assert_eq!(kept_item.at, 10);

        authority.posts.remove(&lost);
        let second = build_feed(&viewer, &store, &[], &arrivals, &authority).unwrap();
        assert_eq!(posts_of(&second), vec![kept]);
        let rendered = serde_json::to_string(&second).unwrap();
        assert!(
            !rendered.contains(lost.as_str()),
            "revoked post left metadata behind: {rendered}"
        );
    }

    #[test]
    fn oracle_deleted_post_disappears_with_no_cleanup() {
        // Oracle Decision 3: the feed is a view, not a stored stream.
        // A post the caller no longer supplies (withdrawn, deleted,
        // moderated away) is simply absent on the next render.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer, &board, 1).unwrap();
        let (p1, p2) = (hash('1'), hash('2'));
        let authority = StubAuthority::all(&[&board], &[&p1, &p2]);
        let mut arrivals = vec![original(&p1, &board, 10), original(&p2, &board, 20)];
        assert_eq!(
            build_feed(&viewer, &store, &[], &arrivals, &authority)
                .unwrap()
                .len(),
            2
        );
        arrivals.retain(|arrival| arrival.post != p2);
        let feed = build_feed(&viewer, &store, &[], &arrivals, &authority).unwrap();
        assert_eq!(posts_of(&feed), vec![p1]);
    }

    #[test]
    fn oracle_playable_post_renders_static_fallback_with_open_on_board() {
        // Oracle Decision 3: playable posts render in the feed as
        // their static fallback with an "open on board" path to the
        // board where they run live; never inline.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (origin, reposter) = (hash('a'), hash('b'));
        store.follow(&viewer, &reposter, 1).unwrap();
        let (game, text) = (hash('1'), hash('2'));
        let authority = StubAuthority::all(&[&origin, &reposter], &[&game, &text]);
        let mut game_arrival = repost(&game, &reposter, &origin, 50);
        game_arrival.playable = true;
        let arrivals = vec![game_arrival, original(&text, &reposter, 40)];
        let feed = build_feed(&viewer, &store, &[], &arrivals, &authority).unwrap();
        assert_eq!(
            feed[0].render,
            FeedRender::StaticFallback {
                open_on_board: origin
            }
        );
        assert_eq!(feed[1].render, FeedRender::Reference);
    }

    #[test]
    fn oracle_feed_is_not_notification_and_creates_no_subscription() {
        // Oracle Decision 3 + invariant 5: notify_mode off (the
        // default) still feeds posts, and building a feed creates or
        // changes no subscription — not even for an owned board.
        let viewer = viewer();
        let mut store = SubscriptionStore::new();
        let (followed, owned) = (hash('a'), hash('b'));
        store.follow(&viewer, &followed, 1).unwrap();
        assert_eq!(
            store.get(&viewer, &followed).unwrap().notify_mode,
            NotifyMode::Off
        );
        let post = hash('1');
        let authority = StubAuthority::all(&[&followed, &owned], &[&post]);
        let arrivals = vec![original(&post, &followed, 10)];
        let feed = build_feed(
            &viewer,
            &store,
            std::slice::from_ref(&owned),
            &arrivals,
            &authority,
        )
        .unwrap();
        assert_eq!(feed.len(), 1);
        assert_eq!(store.len(), 1);
        assert!(!store.is_subscribed(&viewer, &owned));
    }

    #[test]
    fn oracle_malformed_keys_are_rejected() {
        let store = SubscriptionStore::new();
        let authority = StubAuthority::default();
        assert!(build_feed("not-a-key", &store, &[], &[], &authority).is_err());
        let mut bad = repost(&hash('1'), &hash('a'), &hash('b'), 1);
        if let Some(chain) = bad.repost.as_mut() {
            chain.original_author = "nope".to_owned();
        }
        assert!(build_feed(&viewer(), &store, &[], &[bad], &authority).is_err());
    }
}
