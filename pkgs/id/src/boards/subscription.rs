//! Explicit per-viewer board subscriptions: slice 1 of #7585.
//!
//! Oracle: `doc/2026-11-08_design_id5862.md`, Decision 1. A subscription
//! is the subscriber's private data — `{subscriber, board, created_at,
//! notify_mode, muted}` — created only by an explicit follow action.
//! Nothing else creates one: not posting, reposting, membership, or
//! viewing. Unfollow deletes it and takes effect immediately; mute keeps
//! the follow but removes the board's posts from the feed and silences
//! notifications. `notify_mode` defaults to off. Subscribing never grants
//! read authority and surfaces nothing without it (that check lives on
//! the discovery/feed surfaces, later slices — this store just never
//! widens anything itself).
//!
//! This store is host-local and private by API shape: the only reads are
//! a per-pair lookup and a per-subscriber listing, so there is no surface
//! that enumerates who follows what. It holds no caches or materialised
//! copies, so unfollow/mute take effect on the next read by construction.

use std::collections::BTreeMap;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{ArtifactHash, is_public_key};

/// Per-subscription notification preference (Decision 1).
///
/// Following a board puts its posts in the feed and nothing more.
/// Notifications are a separate opt-in; the default is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyMode {
    /// No notifications (the default for every new subscription).
    #[default]
    Off,
    /// Batched notification delivery.
    Digest,
    /// Notify as posts arrive.
    Immediate,
}

/// One explicit follow: the subscriber's private subscription object.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardSubscription {
    /// Subscriber's canonical Ed25519 public key (hex).
    pub subscriber: String,
    /// Board root hash being followed.
    pub board: ArtifactHash,
    /// When the follow happened (caller-supplied Unix seconds).
    pub created_at: u64,
    /// Notification preference; off unless the subscriber opts in.
    #[serde(default)]
    pub notify_mode: NotifyMode,
    /// Muted subscriptions keep the follow but leave the feed.
    #[serde(default)]
    pub muted: bool,
}

/// Redacted on purpose: the subscriber key is the follower's private
/// identity (invariant 4), and `Debug` output lands in logs — it must
/// never become an enumeration surface the API shape forbids (#13452).
impl std::fmt::Debug for BoardSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoardSubscription")
            .field("subscriber", &"<redacted>")
            .field("board", &self.board)
            .field("created_at", &self.created_at)
            .field("notify_mode", &self.notify_mode)
            .field("muted", &self.muted)
            .finish()
    }
}

impl BoardSubscription {
    /// Build the subscription an explicit follow creates.
    pub fn new(subscriber: &str, board: &ArtifactHash, created_at: u64) -> Result<Self> {
        ensure!(
            is_public_key(subscriber),
            "subscription subscriber is not a canonical Ed25519 key"
        );
        Ok(Self {
            subscriber: subscriber.to_owned(),
            board: board.clone(),
            created_at,
            notify_mode: NotifyMode::default(),
            muted: false,
        })
    }
}

/// Host-local, private store of explicit board subscriptions.
///
/// Keyed by `(subscriber, board)`: one subscription per viewer per
/// board, at most. The ONLY writer is [`SubscriptionStore::follow`];
/// reads never insert. `Debug` is redacted for the same reason the
/// reads are per-subscriber: it renders a count, never the keyed map
/// (#13452).
#[derive(Clone, Default)]
pub struct SubscriptionStore {
    subscriptions: BTreeMap<(String, ArtifactHash), BoardSubscription>,
}

/// Redacted on purpose: the derived form would dump every subscriber
/// key with every board they follow — the whole follower graph — into
/// any log line that formats the store (#13452).
impl std::fmt::Debug for SubscriptionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionStore")
            .field("subscription_count", &self.subscriptions.len())
            .finish()
    }
}

impl SubscriptionStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of subscriptions held (all subscribers).
    pub fn len(&self) -> usize {
        self.subscriptions.len()
    }

    /// Whether the store holds no subscriptions.
    pub fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
    }

    /// Explicit follow: create the subscriber's subscription for `board`.
    ///
    /// Fails when the subscriber key is malformed or a subscription for
    /// this pair already exists — a follow never silently overwrites
    /// (the original `created_at` and preferences survive).
    pub fn follow(
        &mut self,
        subscriber: &str,
        board: &ArtifactHash,
        created_at: u64,
    ) -> Result<&BoardSubscription> {
        let subscription = BoardSubscription::new(subscriber, board, created_at)?;
        let key = (subscription.subscriber.clone(), subscription.board.clone());
        ensure!(
            !self.subscriptions.contains_key(&key),
            "already following this board; unfollow first"
        );
        Ok(self.subscriptions.entry(key).or_insert(subscription))
    }

    /// Explicit unfollow: delete the subscription, effective immediately.
    /// Returns true when a subscription was removed.
    pub fn unfollow(&mut self, subscriber: &str, board: &ArtifactHash) -> bool {
        self.subscriptions
            .remove(&(subscriber.to_owned(), board.clone()))
            .is_some()
    }

    /// Look up one viewer's subscription for one board, if any.
    pub fn get(&self, subscriber: &str, board: &ArtifactHash) -> Option<&BoardSubscription> {
        self.subscriptions
            .get(&(subscriber.to_owned(), board.clone()))
    }

    /// Whether this viewer currently follows this board.
    pub fn is_subscribed(&self, subscriber: &str, board: &ArtifactHash) -> bool {
        self.get(subscriber, board).is_some()
    }

    /// Change notification preference for an existing subscription.
    pub fn set_notify_mode(
        &mut self,
        subscriber: &str,
        board: &ArtifactHash,
        mode: NotifyMode,
    ) -> Result<&BoardSubscription> {
        let subscription = self
            .subscriptions
            .get_mut(&(subscriber.to_owned(), board.clone()))
            .ok_or_else(|| anyhow::anyhow!("no subscription for this viewer and board"))?;
        subscription.notify_mode = mode;
        Ok(subscription)
    }

    /// Mute or unmute an existing subscription. Muting keeps the follow
    /// (and the board in the viewer's board list) while its posts leave
    /// the feed and its notifications stop.
    pub fn set_muted(
        &mut self,
        subscriber: &str,
        board: &ArtifactHash,
        muted: bool,
    ) -> Result<&BoardSubscription> {
        let subscription = self
            .subscriptions
            .get_mut(&(subscriber.to_owned(), board.clone()))
            .ok_or_else(|| anyhow::anyhow!("no subscription for this viewer and board"))?;
        subscription.muted = muted;
        Ok(subscription)
    }

    /// Every board one viewer follows, in board-hash order.
    ///
    /// Deliberately per-subscriber: there is no cross-subscriber
    /// enumeration, so follower identity stays private (invariant 4).
    pub fn subscriptions_for(&self, subscriber: &str) -> Vec<&BoardSubscription> {
        self.subscriptions
            .iter()
            .filter(|((owner, _), _)| owner == subscriber)
            .map(|(_, subscription)| subscription)
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod subscription_tests {
    //! Lifecycle tests naming `doc/2026-11-08_design_id5862.md` as oracle.

    use super::*;

    fn hash(ch: char) -> ArtifactHash {
        ArtifactHash::new(ch.to_string().repeat(64)).unwrap()
    }

    fn viewer() -> String {
        "e".repeat(64)
    }

    fn other_viewer() -> String {
        "f".repeat(64)
    }

    #[test]
    fn oracle_follow_creates_private_subscription_with_notify_off_by_default() {
        // Oracle doc/2026-11-08_design_id5862.md Decision 1: explicit
        // follow creates {subscriber, board, created_at, notify_mode,
        // muted}; notify defaults to off; the object is the
        // subscriber's private data.
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        let subscription = store.follow(&viewer(), &board, 1_700_000_000).unwrap();
        assert_eq!(subscription.subscriber, viewer());
        assert_eq!(subscription.board, board);
        assert_eq!(subscription.created_at, 1_700_000_000);
        assert_eq!(subscription.notify_mode, NotifyMode::Off);
        assert!(!subscription.muted);
        assert!(store.is_subscribed(&viewer(), &board));
    }

    #[test]
    fn oracle_no_silent_subscriptions_only_follow_creates() {
        // Oracle Decision 1 + invariant 5: nothing but an explicit
        // follow creates a subscription — reads never insert.
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        assert!(!store.is_subscribed(&viewer(), &board));
        assert!(store.get(&viewer(), &board).is_none());
        assert_eq!(store.subscriptions_for(&viewer()), [] as [&BoardSubscription; 0]);
        assert!(!store.unfollow(&viewer(), &board));
        assert!(store.is_empty());
        assert!(store.set_muted(&viewer(), &board, true).is_err());
        assert!(
            store
                .set_notify_mode(&viewer(), &board, NotifyMode::Immediate)
                .is_err()
        );
        assert!(store.is_empty());
    }

    #[test]
    fn oracle_unfollow_is_immediate_and_leaves_no_residue() {
        // Oracle Decision 1 + invariant 3: unfollow deletes the
        // subscription and takes effect on every surface at once.
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer(), &board, 1).unwrap();
        assert!(store.unfollow(&viewer(), &board));
        assert!(!store.is_subscribed(&viewer(), &board));
        assert!(store.get(&viewer(), &board).is_none());
        assert_eq!(store.subscriptions_for(&viewer()), [] as [&BoardSubscription; 0]);
        assert!(!store.unfollow(&viewer(), &board));
    }

    #[test]
    fn oracle_mute_keeps_follow_but_flags_muted() {
        // Oracle Decision 1: mute is subscription state, not a separate
        // system — the follow stays while its posts leave the feed.
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer(), &board, 1).unwrap();
        store.set_muted(&viewer(), &board, true).unwrap();
        let subscription = store.get(&viewer(), &board).unwrap();
        assert!(subscription.muted);
        assert!(store.is_subscribed(&viewer(), &board));
        store.set_muted(&viewer(), &board, false).unwrap();
        assert!(!store.get(&viewer(), &board).unwrap().muted);
    }

    #[test]
    fn oracle_notify_mode_changes_after_follow() {
        // Oracle Decision 1: notify_mode is set at follow time and
        // changeable after; the default for every new subscription is off.
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer(), &board, 1).unwrap();
        store
            .set_notify_mode(&viewer(), &board, NotifyMode::Digest)
            .unwrap();
        assert_eq!(
            store.get(&viewer(), &board).unwrap().notify_mode,
            NotifyMode::Digest
        );
        store
            .set_notify_mode(&viewer(), &board, NotifyMode::Immediate)
            .unwrap();
        assert_eq!(
            store.get(&viewer(), &board).unwrap().notify_mode,
            NotifyMode::Immediate
        );
    }

    #[test]
    fn oracle_subscriber_identity_is_private() {
        // Oracle invariant 4: follower identity is visible to no one
        // but the follower. Another viewer's reads see nothing, and
        // the store offers no cross-subscriber enumeration.
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer(), &board, 1).unwrap();
        assert!(!store.is_subscribed(&other_viewer(), &board));
        assert!(store.get(&other_viewer(), &board).is_none());
        assert_eq!(store.subscriptions_for(&other_viewer()), [] as [&BoardSubscription; 0]);
        assert_eq!(store.subscriptions_for(&viewer()).len(), 1);
    }

    #[test]
    fn oracle_debug_output_never_renders_subscriber_keys() {
        // Oracle invariant 4, via #13452: `Debug` is a log surface, so
        // it must not render subscriber keys either — neither for one
        // subscription nor for the store (whose derived form dumped
        // the entire follower graph).
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer(), &board, 1).unwrap();
        store.follow(&other_viewer(), &board, 1).unwrap();
        let store_rendered = format!("{store:?}");
        assert!(
            !store_rendered.contains(&viewer()),
            "store Debug leaks subscriber key: {store_rendered}"
        );
        assert!(
            !store_rendered.contains(&other_viewer()),
            "store Debug leaks subscriber key: {store_rendered}"
        );
        let subscription = store.get(&viewer(), &board).unwrap();
        let subscription_rendered = format!("{subscription:?}");
        assert!(
            !subscription_rendered.contains(&viewer()),
            "subscription Debug leaks subscriber key: {subscription_rendered}"
        );
    }

    #[test]
    fn oracle_duplicate_follow_is_rejected_not_overwritten() {
        // A follow never silently replaces: the original created_at and
        // preferences survive a repeated follow attempt.
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        store.follow(&viewer(), &board, 1).unwrap();
        store.set_muted(&viewer(), &board, true).unwrap();
        assert!(store.follow(&viewer(), &board, 2).is_err());
        let kept = store.get(&viewer(), &board).unwrap();
        assert_eq!(kept.created_at, 1);
        assert!(kept.muted);
    }

    #[test]
    fn oracle_malformed_subscriber_key_is_rejected() {
        let mut store = SubscriptionStore::new();
        let board = hash('a');
        assert!(store.follow("not-a-key", &board, 1).is_err());
        assert!(store.follow("", &board, 1).is_err());
        assert!(store.is_empty());
    }

    #[test]
    fn oracle_subscription_round_trips_with_off_by_default() {
        let subscription = BoardSubscription::new(&viewer(), &hash('a'), 7).unwrap();
        let encoded = serde_json::to_string(&subscription).unwrap();
        assert_eq!(
            serde_json::from_str::<BoardSubscription>(&encoded).unwrap(),
            subscription
        );
        // Bodies written before notify_mode/muted existed decode as off/unmuted.
        let legacy = serde_json::json!({
            "subscriber": viewer(),
            "board": "a".repeat(64),
            "created_at": 7,
        });
        let decoded: BoardSubscription = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.notify_mode, NotifyMode::Off);
        assert!(!decoded.muted);
    }
}
