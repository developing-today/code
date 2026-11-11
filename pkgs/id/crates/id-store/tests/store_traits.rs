//! The store trait surface against the real event store.
//!
//! These tests lived inside `id_store::store_traits` until step 2a of the
//! crate split (#4405) made the shared handle generic over
//! [`id_store::store_traits::EventStoreBackend`]: a crate's unit tests are
//! compiled against a second instance of the crate, so the facade's
//! backend implementation (for its `EventStore`) is invisible to them.
//! Integration tests link the normal build — one instance — so the suite
//! sits here, unchanged in what it checks.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::pin::Pin;

use futures_lite::{Stream, StreamExt};
use p2panda_core::{AnyOperation, Hash, SeqNum, SigningKey, VerifyingKey};
use p2panda_store::logs::{LogEntry, LogStore};
use p2panda_store::topics::TopicStore;

use id::event_store::EventStore;
use id_core::waiting::{AuthorKey, WaitingLimits};
use id_store::chain::{ChainExt, Event, sign};
use id_store::event_types::{BlockKind, Ingest, StoreError};
use id_store::store_traits::{ChainLog, LogEntries, SharedEventStore, TraitStoreError};


/// One `[[package]]` entry of the lock: its name and the names it
/// depends on. Dependency entries are `"name"`, `"name version"` or
/// `"name version (source)"`; the name is the first word.
fn lock_packages(lock: &str) -> Vec<(String, Vec<String>)> {
    let mut packages = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    let mut in_deps = false;
    for raw in lock.lines() {
        let line = raw.trim();
        if line.starts_with("[[") {
            in_deps = false;
            if let Some(package) = current.take() {
                packages.push(package);
            }
        } else if current.is_none() {
            if let Some(rest) = line.strip_prefix("name = \"") {
                current = Some((rest.trim_end_matches('"').to_owned(), Vec::new()));
            }
        } else if line == "dependencies = [" {
            in_deps = true;
        } else if in_deps && line == "]" {
            in_deps = false;
        } else if in_deps {
            if let Some(entry) = line.strip_suffix(',') {
                let entry = entry.trim_matches('"');
                if let Some(name) = entry.split(' ').next() {
                    current.as_mut().unwrap().1.push(name.to_owned());
                }
            }
        }
    }
    if let Some(package) = current.take() {
        packages.push(package);
    }
    packages
}

/// Whether a manifest section header opens a dev-dependency section
/// (`Some(true)`), a shipped one — normal or build (`Some(false)`) — or
/// neither. Plain, target-specific and per-crate subtable spellings all
/// count: `[dev-dependencies]`, `[target.'cfg(..)'.dependencies]`,
/// `[dependencies.rusqlite]`.
fn dep_section_dev(section: &str) -> Option<bool> {
    let tail = section.rsplit('.').next().unwrap_or(section);
    match tail {
        "dev-dependencies" => Some(true),
        "dependencies" | "build-dependencies" => Some(false),
        _ if section.starts_with("dev-dependencies.") => Some(true),
        _ if section.starts_with("dependencies.")
            || section.starts_with("build-dependencies.") =>
        {
            Some(false)
        }
        _ => None,
    }
}

/// The crate names a manifest declares in dependency sections of one
/// class (`dev` selects dev-dependencies, otherwise the shipped ones).
/// A `package = "..."` rename counts under the real name.
fn manifest_dep_names(manifest: &str, dev: bool) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_class = false;
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section = line.trim_matches(|c| c == '[' || c == ']');
            in_class = dep_section_dev(section) == Some(dev);
            if in_class {
                // A per-crate subtable names the crate in the header:
                // `[dependencies.rusqlite]`.
                if let Some((_, name)) = section.rsplit_once('.')
                    && !matches!(
                        name,
                        "dependencies" | "build-dependencies" | "dev-dependencies"
                    )
                {
                    names.push(name.to_owned());
                }
            }
            continue;
        }
        if !in_class {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim().trim_matches('"');
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            {
                names.push(key.to_owned());
            }
        }
        if let Some((_, rest)) = line.split_once("package = \"")
            && let Some(name) = rest.split('"').next()
        {
            names.push(name.to_owned());
        }
    }
    names
}

/// The `[package]` name of a manifest.
fn manifest_package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && let Some(rest) = line.strip_prefix("name = \"") {
            return Some(rest.trim_end_matches('"').to_owned());
        }
    }
    None
}

/// The shipped default build has no SQLite, with `LogSync` wired in. p2panda-sync's store
/// dependency defaults to SQLite, so the vendored copy turns that default off
/// (`vendor/README.md`).
///
/// Cargo.lock records optional dependencies even when they are inactive, so its raw package
/// edges cannot prove what ships. Keep checking for unexpected SQLite-family edges (allowing
/// p2panda-store's optional `sqlx` edge), then ask Cargo for the shipped default-feature graph.
/// The root manifest must still declare SQLite names only as dev-dependencies. This catches
/// a shipped SQLite path, a root manifest regression, a missing p2panda-sync package, or the
/// vendored line losing its `default-features = false`.
#[test]
fn test_the_lock_has_no_sqlite_with_logsync_wired() {
    const SQLITE: [&str; 4] = ["sqlx", "sqlite", "rusqlite", "libsqlite3-sys"];
    // This guard covers the facade package (the no-SQLite check reads the shipped
    // graph of `id`), whose directory is the workspace root, ../.. from this crate.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let root_dir = format!("{manifest_dir}/../..");
    let lock = std::fs::read_to_string(format!("{root_dir}/Cargo.lock")).unwrap();
    let packages = lock_packages(&lock);
    assert!(
        packages.iter().any(|(name, _)| name == "p2panda-sync"),
        "Cargo.lock does not name p2panda-sync: LogSync is not wired in"
    );

    let manifest = std::fs::read_to_string(format!("{root_dir}/Cargo.toml")).unwrap();
    let root = manifest_package_name(&manifest).unwrap();
    // This package may declare SQLite names only as dev-dependencies.
    let shipped = manifest_dep_names(&manifest, false);
    for name in SQLITE {
        assert!(
            !shipped.iter().any(|declared| declared.as_str() == name),
            "Cargo.toml declares {name} outside dev-dependencies: SQLite is back in the \
             shipped build (see vendor/README.md)"
        );
    }
    // Cargo.lock includes optional dependency edges. p2panda-store declares optional `sqlx`,
    // which is inactive in the shipped default-feature graph checked below.
    for (name, deps) in &packages {
        for dep in deps {
            if SQLITE.contains(&dep.as_str()) {
                assert!(
                    SQLITE.contains(&name.as_str())
                        || name == &root
                        || (name == "p2panda-store" && dep == "sqlx")
                        // Cargo.lock records sqlx-sqlite's optional backend edge even
                        // when it is inactive; cargo tree below checks the shipped graph.
                        || (name == "sqlx-sqlite" && dep == "libsqlite3-sys"),
                    "{name} depends on {dep}: SQLite is back in the shipped build (see \
                     vendor/README.md)"
                );
            }
        }
    }

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let tree = std::process::Command::new(cargo)
        .current_dir(manifest_dir)
        .args([
            "tree",
            "--locked",
            "--package",
            "id",
            "--edges",
            "normal",
            "--no-default-features",
            "--features",
            "sandbox,world",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .output()
        .expect("run cargo tree for the shipped default feature set");
    assert!(
        tree.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&tree.stderr)
    );
    let active_packages = String::from_utf8(tree.stdout).unwrap();
    for package in active_packages
        .lines()
        .filter_map(|line| line.split_whitespace().next())
    {
        assert!(
            !SQLITE.contains(&package),
            "{package} is active in the shipped default-feature graph (see vendor/README.md)"
        );
    }

    let vendored =
        std::fs::read_to_string(format!("{root_dir}/vendor/p2panda-sync/Cargo.toml"))
            .unwrap();
    let store_line = vendored
        .lines()
        .find(|line| line.starts_with("p2panda-store = {"))
        .unwrap();
    assert!(
        store_line.contains("default-features = false"),
        "the vendored p2panda-store keeps its SQLite default: {store_line}"
    );
}

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

fn author_of(key: &SigningKey) -> VerifyingKey {
    key.verifying_key()
}

fn group(n: u8) -> Hash {
    Hash::digest([n])
}

/// One signed event of `signer`'s chain, with an optional body.
fn event(
    signer: &SigningKey,
    seq: SeqNum,
    backlink: Option<Hash>,
    group: Option<Hash>,
    kind: &str,
    body: Option<&[u8]>,
) -> Event {
    let ext = ChainExt {
        kind: kind.to_owned(),
        group,
        after: Vec::new(),
        tag: None,
        document: None,
        removes: None,
        note: None,
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
    };
    sign(signer, seq, backlink, body, ext).unwrap()
}

/// The first `count` events of `signer`'s chain, all in `group`, each naming the one before.
fn chain_of(signer: &SigningKey, count: SeqNum, group: Option<Hash>) -> Vec<Event> {
    let mut events: Vec<Event> = Vec::new();
    for seq in 0..count {
        let backlink = events.last().map(|event| event.id);
        events.push(event(signer, seq, backlink, group, "message", None));
    }
    events
}

fn open() -> (tempfile::TempDir, SharedEventStore<EventStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, SharedEventStore::new(store))
}

fn ingest_all(store: &SharedEventStore<EventStore>, events: &[Event]) {
    for event in events {
        let outcome = store.ingest(&event.bytes, None, 0).unwrap();
        assert_eq!(
            outcome,
            Ingest::Accepted {
                id: event.id,
                released: Vec::new(),
            }
        );
    }
}

/// The `(count, bytes)` that the trait must report for `events`: header bytes plus the
/// declared body sizes.
fn sized(events: &[Event]) -> (u32, u32) {
    let count = u32::try_from(events.len()).unwrap();
    let bytes = events
        .iter()
        .map(|event| u32::try_from(event.bytes.len()).unwrap() + event.header.payload_size)
        .sum();
    (count, bytes)
}

async fn drain<S>(mut stream: S) -> Vec<LogEntry<AnyOperation, ChainLog>>
where
    S: Stream<Item = Result<LogEntry<AnyOperation, ChainLog>, TraitStoreError>> + Unpin,
{
    let mut entries = Vec::new();
    while let Some(entry) = stream.next().await {
        entries.push(entry.unwrap());
    }
    entries
}

async fn entries_of(
    store: &SharedEventStore<EventStore>,
    who: &VerifyingKey,
    after: Option<SeqNum>,
    until: Option<SeqNum>,
) -> Vec<LogEntry<AnyOperation, ChainLog>> {
    drain(store.log_entries(who, &ChainLog, after, until).unwrap()).await
}

/// The streamed entry is the stored event, in the form the sync protocol sends.
fn assert_is_event(entry: &LogEntry<AnyOperation, ChainLog>, event: &Event) {
    assert_eq!(entry.log_id, ChainLog);
    assert_eq!(entry.bytes, event.bytes);
    assert_eq!(entry.entry.hash, event.id);
    assert_eq!(entry.entry.header.seq_num, event.header.seq_num);
    assert_eq!(entry.entry.header.verifying_key, event.header.verifying_key);
    assert_eq!(entry.entry.header.backlink, event.header.backlink);
    assert!(
        entry.entry.body.is_none(),
        "bodies are not sent with events"
    );
}

#[tokio::test]
async fn test_an_empty_store_has_no_log_for_any_author() {
    let (_dir, store) = open();
    let who = author_of(&key(1));
    assert_eq!(
        store.get_latest_entry_tx(&who, &ChainLog).await.unwrap(),
        None
    );
    assert_eq!(
        store.get_log_heights(&who, &[ChainLog]).await.unwrap(),
        None
    );
    assert_eq!(
        store
            .get_log_size(&who, &ChainLog, None, None)
            .await
            .unwrap(),
        None
    );
    assert!(entries_of(&store, &who, None, None).await.is_empty());
    assert_eq!(store.resolve(&group(1)).await.unwrap(), BTreeMap::new());
    store.check_invariants().unwrap();
}

#[tokio::test]
async fn test_the_latest_entry_and_heights_follow_the_chain() {
    let (_dir, store) = open();
    let (alice, bob) = (key(2), key(3));
    let (who_alice, who_bob) = (author_of(&alice), author_of(&bob));
    let alice_events = chain_of(&alice, 5, Some(group(1)));
    let bob_events = chain_of(&bob, 2, Some(group(2)));
    ingest_all(&store, &alice_events);
    ingest_all(&store, &bob_events);

    let latest = store
        .get_latest_entry_tx(&who_alice, &ChainLog)
        .await
        .unwrap()
        .expect("alice has an entry");
    assert_eq!(latest.hash, alice_events[4].id);
    assert_eq!(latest.header.seq_num, 4);
    assert_eq!(latest.header.backlink, Some(alice_events[3].id));

    assert_eq!(
        store
            .get_log_heights(&who_alice, &[ChainLog])
            .await
            .unwrap(),
        Some(BTreeMap::from([(ChainLog, 4)]))
    );
    assert_eq!(
        store.get_log_heights(&who_bob, &[ChainLog]).await.unwrap(),
        Some(BTreeMap::from([(ChainLog, 1)]))
    );
    // Asking for no logs asks for nothing, and asking twice for one log names it once.
    assert_eq!(store.get_log_heights(&who_alice, &[]).await.unwrap(), None);
    assert_eq!(
        store
            .get_log_heights(&who_alice, &[ChainLog, ChainLog])
            .await
            .unwrap(),
        Some(BTreeMap::from([(ChainLog, 4)]))
    );

    assert_eq!(
        store.resolve(&group(1)).await.unwrap(),
        BTreeMap::from([(who_alice, vec![ChainLog])])
    );
    assert_eq!(
        store.resolve(&group(2)).await.unwrap(),
        BTreeMap::from([(who_bob, vec![ChainLog])])
    );
    store.check_invariants().unwrap();
}

#[tokio::test]
async fn test_the_log_size_counts_headers_and_declared_bodies() {
    let (_dir, store) = open();
    let alice = key(4);
    let who = author_of(&alice);
    let mut events: Vec<Event> = Vec::new();
    for seq in 0..5 {
        let backlink = events.last().map(|event| event.id);
        // Every body is non-empty, and the lengths differ, so the sum is not a count.
        let body = vec![7u8; 1 + 10 * usize::try_from(seq).unwrap()];
        events.push(event(
            &alice,
            seq,
            backlink,
            Some(group(1)),
            "message",
            Some(&body),
        ));
    }
    ingest_all(&store, &events);

    assert_eq!(
        store
            .get_log_size(&who, &ChainLog, None, None)
            .await
            .unwrap(),
        Some(sized(&events))
    );
    assert_eq!(
        store
            .get_log_size(&who, &ChainLog, Some(0), Some(3))
            .await
            .unwrap(),
        Some(sized(&events[1..=3]))
    );
    assert_eq!(
        store
            .get_log_size(&who, &ChainLog, None, Some(100))
            .await
            .unwrap(),
        Some(sized(&events))
    );
    // An empty range of an author who has events is zero, not absent.
    assert_eq!(
        store
            .get_log_size(&who, &ChainLog, Some(4), None)
            .await
            .unwrap(),
        Some((0, 0))
    );
    assert_eq!(
        store
            .get_log_size(&author_of(&key(9)), &ChainLog, None, None)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn test_log_entries_stream_each_position_once_in_order() {
    let (_dir, store) = open();
    let alice = key(5);
    let who = author_of(&alice);
    let events = chain_of(&alice, 5, None);
    ingest_all(&store, &events);
    let stream =
        |after, until| LogEntries::new(store.clone(), *who.as_bytes(), after, until, 2);

    // A page of two makes the stream cross page boundaries.
    let all = drain(stream(None, None)).await;
    assert_eq!(all.len(), events.len());
    for (entry, event) in all.iter().zip(&events) {
        assert_is_event(entry, event);
    }
    let middle = drain(stream(Some(1), Some(3))).await;
    assert_eq!(middle.len(), 2, "after 1 is exclusive, until 3 inclusive");
    assert_is_event(&middle[0], &events[2]);
    assert_is_event(&middle[1], &events[3]);
    assert!(drain(stream(Some(4), None)).await.is_empty());
    let first = drain(stream(None, Some(0))).await;
    assert_eq!(first.len(), 1);
    assert_is_event(&first[0], &events[0]);
    assert_eq!(entries_of(&store, &who, Some(1), Some(3)).await.len(), 2);
}

#[tokio::test]
async fn test_a_log_longer_than_one_page_is_streamed_whole() {
    let (_dir, store) = open();
    let alice = key(6);
    let who = author_of(&alice);
    // Three pages: 64, 64 and 3.
    let events = chain_of(&alice, 131, None);
    ingest_all(&store, &events);

    let entries = entries_of(&store, &who, None, None).await;
    assert_eq!(entries.len(), events.len());
    for (entry, event) in entries.iter().zip(&events) {
        assert_is_event(entry, event);
    }
    assert_eq!(
        store
            .get_log_size(&who, &ChainLog, None, None)
            .await
            .unwrap()
            .map(|(count, _bytes)| count),
        Some(131)
    );
    store.check_invariants().unwrap();
}

#[tokio::test]
async fn test_a_topic_resolves_to_the_authors_applied_in_it() {
    let (_dir, store) = open();
    let (alice, bob, carol) = (key(7), key(8), key(9));
    let (topic, other_topic) = (group(1), group(2));
    ingest_all(&store, &chain_of(&alice, 3, Some(topic)));
    ingest_all(&store, &chain_of(&bob, 1, Some(topic)));
    ingest_all(&store, &chain_of(&carol, 2, Some(other_topic)));
    let (who_alice, who_bob, who_carol) =
        (author_of(&alice), author_of(&bob), author_of(&carol));

    assert_eq!(
        store.resolve(&topic).await.unwrap(),
        BTreeMap::from([(who_alice, vec![ChainLog]), (who_bob, vec![ChainLog])])
    );
    assert_eq!(
        store.resolve(&other_topic).await.unwrap(),
        BTreeMap::from([(who_carol, vec![ChainLog])])
    );
    assert!(store.resolve(&group(3)).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_after_a_fork_only_the_applied_prefix_is_visible() {
    let (_dir, store) = open();
    let (alice, bob) = (key(10), key(11));
    let (topic, rival_topic) = (group(4), group(5));
    let who = author_of(&alice);
    // Alice: seq 0 outside any group, then seq 1 and seq 2 in `topic`.
    let first = event(&alice, 0, None, None, "message", None);
    let second = event(&alice, 1, Some(first.id), Some(topic), "message", None);
    let third = event(&alice, 2, Some(second.id), Some(topic), "message", None);
    ingest_all(&store, std::slice::from_ref(&first));
    ingest_all(&store, &[second.clone(), third.clone()]);
    // Alice's rival at seq 1: the same backlink as `second`, so it conflicts with it.
    let rival = event(&alice, 1, Some(first.id), Some(rival_topic), "rival", None);
    assert_eq!(
        store.ingest(&rival.bytes, None, 0).unwrap(),
        Ingest::Fork {
            author: *who.as_bytes(),
            seq: 1,
        }
    );
    ingest_all(&store, &chain_of(&bob, 1, Some(topic)));

    // Only seq 0 is applied. Seq 1 and seq 2 are stored, but they are not part of the log.
    let latest = store
        .get_latest_entry_tx(&who, &ChainLog)
        .await
        .unwrap()
        .expect("seq 0 is applied");
    assert_eq!(latest.hash, first.id);
    assert_eq!(
        store.get_log_heights(&who, &[ChainLog]).await.unwrap(),
        Some(BTreeMap::from([(ChainLog, 0)]))
    );
    assert_eq!(
        store
            .get_log_size(&who, &ChainLog, None, None)
            .await
            .unwrap(),
        Some(sized(std::slice::from_ref(&first)))
    );
    let entries = entries_of(&store, &who, None, None).await;
    assert_eq!(entries.len(), 1);
    assert_is_event(&entries[0], &first);
    // A bound past the applied prefix does not reveal the events stored beyond the fork.
    assert_eq!(
        store
            .get_log_size(&who, &ChainLog, None, Some(100))
            .await
            .unwrap(),
        Some(sized(std::slice::from_ref(&first)))
    );
    assert_eq!(entries_of(&store, &who, None, Some(100)).await.len(), 1);
    // Alice's events in `topic` are past the fork, so she is not in it. Bob still is.
    assert_eq!(
        store.resolve(&topic).await.unwrap(),
        BTreeMap::from([(author_of(&bob), vec![ChainLog])])
    );
    store.check_invariants().unwrap();
}

#[tokio::test]
async fn test_the_methods_outside_the_sync_path_refuse_with_a_reason() {
    let (_dir, store) = open();
    let who = author_of(&key(12));
    let topic = group(6);
    // `get_latest_entry_tx` is the real read path now (no transaction permit on redb), so it
    // serves the empty store as `None`, not an error.
    assert_eq!(
        store.get_latest_entry_tx(&who, &ChainLog).await.unwrap(),
        None
    );
    assert!(matches!(
        store.prune_entries_tx(&who, &ChainLog, &0).await,
        Err(TraitStoreError::Unsupported(_))
    ));
    assert!(matches!(
        store.associate_tx(&topic, &who, &ChainLog).await,
        Err(TraitStoreError::Unsupported(_))
    ));
    assert!(matches!(
        store.remove_tx(&topic, &who, &ChainLog).await,
        Err(TraitStoreError::Unsupported(_))
    ));
    assert!(matches!(
        store.resolve_topics(&who, &ChainLog).await,
        Err(TraitStoreError::Unsupported(_))
    ));
    assert!(matches!(
        store.topics().await,
        Err(TraitStoreError::Unsupported(_))
    ));
}

#[tokio::test]
async fn test_clones_share_one_store() {
    let (_dir, store) = open();
    let other = store.clone();
    let alice = key(13);
    ingest_all(&store, &chain_of(&alice, 2, None));
    assert_eq!(
        other
            .get_log_heights(&author_of(&alice), &[ChainLog])
            .await
            .unwrap(),
        Some(BTreeMap::from([(ChainLog, 1)]))
    );
}

#[tokio::test]
#[allow(clippy::panic)]
async fn test_a_poisoned_lock_is_reported_not_unwrapped() {
    let (_dir, store) = open();
    let poisoner = store.clone();
    let panicked = std::thread::spawn(move || {
        let _guard = poisoner.write_guard_for_test();
        panic!("poisoning the store lock on purpose");
    })
    .join();
    assert!(panicked.is_err());
    let who = author_of(&key(14));
    assert!(matches!(
        store.get_log_heights(&who, &[ChainLog]).await,
        Err(TraitStoreError::Poisoned)
    ));
    assert!(matches!(
        store.ingest(&[], None, 0),
        Err(TraitStoreError::Poisoned)
    ));
}
