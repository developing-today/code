//! `id compact`: one-shot retention pass over the event store (LIST 6–15, #2056).
//!
//! The missing production caller for [`LocalEvents::compact`](id_events::local_events::LocalEvents::compact)
//! (#5534, #2036 §4.6, criterion 8): reclaims caller-declared live-class bytes under a
//! [`CompactionPolicy`](id_events::event_store::CompactionPolicy), leaving durable kinds,
//! checkpoints and each author's retained recent history untouched. A compacted event keeps
//! its presence, applied verdict and causal edges as a residue.
//!
//! The policy here is the node-wide default the design leaves open per channel (§7): the
//! live kind is [`CHAT_MESSAGE_KIND`], and `retain_recent` defaults to 100. Retention is
//! opt-in, never inferred: a kind the policy does not declare live is retained.
//!
//! The store directory admits one opener, so this command fails clearly when `id serve`
//! holds the store instead of waiting on its lock.
//!
//! # Output
//!
//! Prints `compact reclaimed=<N> retain_recent=<N> live_kind=<KIND>`, or one JSON object
//! with `command` first.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::json;

use crate::commands::doc::{open_events, print_action};
use id_events::event_store::CompactionPolicy;
use id_events::local_events::LocalEvents;
use id_store::chat::CHAT_MESSAGE_KIND;

/// How many of each author's most recent applied events are always kept when the
/// caller passes no `--retain-recent`.
pub const DEFAULT_RETAIN_RECENT: u32 = 100;

/// The retention declaration for one `id compact` run: which live class to reclaim
/// and how much recent history to keep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactOptions {
    /// How many of an author's most recent applied events are always retained.
    pub retain_recent: u32,
}

/// The [`CompactionPolicy`] for `options`: the chat channel family is the live class,
/// and durable kinds stay retained whatever this says (the store's durable-kind table).
#[must_use]
pub fn policy_for(options: &CompactOptions) -> CompactionPolicy {
    CompactionPolicy {
        live_kinds: [CHAT_MESSAGE_KIND.to_owned()].into_iter().collect(),
        retain_recent: options.retain_recent,
    }
}

/// Reclaims live-class bytes in `events` under `options`, returning how many events
/// lost their bytes. Durable kinds, checkpoints and retained recent history are never
/// touched; a compacted event keeps its presence, verdict and causal edges.
///
/// # Errors
///
/// Fails if the store is unusable or the compaction write fails; then the store is
/// unchanged (the pass is one transaction).
pub fn compact_store(events: &mut LocalEvents, options: &CompactOptions) -> Result<usize> {
    events.compact(&policy_for(options))
}

/// Runs an `id compact` command: opens the event store and reclaims live-class bytes.
///
/// # Errors
///
/// Fails if the store cannot be opened — for example while `id serve` holds its
/// single-opener lock — or the compaction write fails.
pub async fn cmd_compact(path: Option<PathBuf>, json: bool, retain_recent: u32) -> Result<()> {
    let options = CompactOptions { retain_recent };
    let mut events = open_events(path)
        .await
        .context("open the event store (it is single-opener: stop `id serve` first)")?;
    let reclaimed = compact_store(&mut events, &options)?;
    print_action(
        json,
        "compact",
        vec![
            ("reclaimed", json!(reclaimed)),
            ("retain_recent", json!(options.retain_recent)),
            ("live_kind", json!(CHAT_MESSAGE_KIND)),
        ],
    );
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use p2panda_core::SigningKey;

    #[test]
    fn test_policy_for_declares_chat_live_and_maps_retain_recent() {
        let options = CompactOptions { retain_recent: 100 };
        let policy = policy_for(&options);
        assert!(policy.live_kinds.contains(CHAT_MESSAGE_KIND));
        assert_eq!(policy.live_kinds.len(), 1);
        assert_eq!(policy.retain_recent, 100);
        assert_eq!(
            policy_for(&CompactOptions { retain_recent: 7 }).retain_recent,
            7
        );
        assert_eq!(DEFAULT_RETAIN_RECENT, 100);
    }

    fn test_events() -> (tempfile::TempDir, LocalEvents) {
        let dir = tempfile::TempDir::new().unwrap();
        let events = LocalEvents::open(dir.path(), SigningKey::from_bytes(&[9; 32])).unwrap();
        (dir, events)
    }

    #[test]
    fn test_compact_on_document_only_store_reclaims_nothing_and_keeps_docs() {
        let (_dir, mut events) = test_events();
        let published = events
            .publish("text/plain", b"durable content", &[])
            .unwrap();
        let options = CompactOptions { retain_recent: 0 };
        let reclaimed = compact_store(&mut events, &options).unwrap();
        assert_eq!(reclaimed, 0, "durable document kinds never compact");
        // Oracle: the public read API. The document's history still resolves to its
        // one `document.put`, and the author's head is still that event.
        let history = events.history(published.id).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id(), published.id);
        let author = events.author();
        let head = events
            .frontier()
            .unwrap()
            .into_iter()
            .find(|head| head.author == author)
            .expect("the author's head");
        assert_eq!(head.event, published.id);
    }
}
