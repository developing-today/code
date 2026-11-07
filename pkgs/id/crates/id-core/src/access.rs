//! Write-access control shared by the meta and blobs protocols.
//!
//! `id serve` is reachable by any peer that knows (or discovers) its node ID.
//! Reading names, hashes and metadata is public by design, but **mutating**
//! operations (`put`, `delete`, `rename`, `copy`, tag writes, blob pushes) are
//! gated by an [`AccessPolicy`]:
//!
//! - The server's own key and the data directory's *client* key are trusted
//!   automatically, so the local CLI and REPL keep working with no setup.
//! - Other nodes must be listed explicitly (`--allow-node <ID>` or the
//!   `.iroh-allowed` file in the data directory).
//! - `--open-writes` disables the check entirely (anyone can write). It exists
//!   for demos and trusted LANs and is loudly reported at startup.
//!
//! The same policy is applied to the iroh-blobs *push* path (via
//! [`blobs_events`]), because iroh-blobs rejects pushes unless an event
//! handler enables them.
//!
//! # Design note
//!
//! This is deliberately a flat allowlist, not a capability system. The
//! platform design in `doc/` (ocap tokens, attenuable delegation) supersedes
//! it; the allowlist is the minimal, safe default for the existing CLI.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, RwLock},
};

use iroh_base::EndpointId;
use iroh_blobs::provider::events::{
    AbortReason, ConnectMode, EventMask, EventSender, ProviderMessage, RequestMode,
};
use tracing::{debug, warn};

/// File (in the data directory) listing additional writer node IDs, one per line.
pub const ALLOWED_NODES_FILE: &str = ".iroh-allowed";

#[derive(Debug, Default)]
struct Inner {
    open_writes: bool,
    writers: RwLock<HashSet<EndpointId>>,
}

/// Decides which remote nodes may perform mutating operations.
///
/// Cheap to clone; all clones share state.
#[derive(Debug, Clone, Default)]
pub struct AccessPolicy {
    inner: Arc<Inner>,
}

impl AccessPolicy {
    /// A policy where only the given nodes may write.
    pub fn restricted(writers: impl IntoIterator<Item = EndpointId>) -> Self {
        Self {
            inner: Arc::new(Inner {
                open_writes: false,
                writers: RwLock::new(writers.into_iter().collect()),
            }),
        }
    }

    /// A policy where every connected node may write. Use with care.
    pub fn open() -> Self {
        Self {
            inner: Arc::new(Inner {
                open_writes: true,
                writers: RwLock::new(HashSet::new()),
            }),
        }
    }

    /// Whether `id` may perform mutating operations.
    pub fn can_write(&self, id: &EndpointId) -> bool {
        if self.inner.open_writes {
            return true;
        }
        self.inner.writers.read().is_ok_and(|w| w.contains(id))
    }

    /// Whether the policy lets everyone write.
    pub fn is_open(&self) -> bool {
        self.inner.open_writes
    }

    /// Add a writer at runtime.
    pub fn allow(&self, id: EndpointId) {
        if let Ok(mut w) = self.inner.writers.write() {
            w.insert(id);
        }
    }

    /// Snapshot of explicitly allowed writers (sorted for stable output).
    pub fn writers(&self) -> Vec<EndpointId> {
        let mut v: Vec<EndpointId> = self
            .inner
            .writers
            .read()
            .map(|w| w.iter().copied().collect())
            .unwrap_or_default();
        v.sort_by_key(ToString::to_string);
        v
    }
}

/// Parse a node-ID list: one ID per line, `#` comments and blank lines ignored.
pub fn parse_allowed_nodes(text: &str) -> Vec<EndpointId> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .filter_map(|l| match l.parse::<EndpointId>() {
            Ok(id) => Some(id),
            Err(e) => {
                warn!("ignoring invalid node id {l:?} in allow list: {e}");
                None
            }
        })
        .collect()
}

/// Load the optional allow-list file from `dir`. Missing file means empty list.
///
/// A file that is present but cannot be read as UTF-8 text (saved as
/// UTF-16 by an editor, replaced by a directory, permission denied, ...)
/// also yields an empty list — the policy stays fail-closed — but it is
/// NOT silent: a warning names the file and the read failure, so the
/// operator can tell a broken allow list from a missing one. Collapsing
/// both cases into a silent empty list (#6677) left allowed peers denied
/// with no diagnostic pointing at the file they wrote.
pub fn load_allowed_nodes(dir: &Path) -> Vec<EndpointId> {
    let path = dir.join(ALLOWED_NODES_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_allowed_nodes(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => {
            warn!(
                "ignoring unreadable allow list {}: {err}; no nodes from it will be allowed to write",
                path.display()
            );
            Vec::new()
        }
    }
}

/// Build an iroh-blobs [`EventSender`] that enforces `policy` on push requests.
///
/// iroh-blobs disables pushes unless an event handler enables them. This
/// enables pushes in *intercept* mode and answers each one using the identity
/// of the connection that sent it. Reads (get/observe) are unaffected.
///
/// The returned sender must be passed to `BlobsProtocol::new`. A background
/// task is spawned on the current Tokio runtime and ends when the sender is
/// dropped.
pub fn blobs_events(policy: AccessPolicy) -> EventSender {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderMessage>(64);
    let mask = EventMask {
        connected: ConnectMode::Notify,
        push: RequestMode::Intercept,
        ..EventMask::DEFAULT
    };
    tokio::spawn(async move {
        let mut peers: HashMap<u64, Option<EndpointId>> = HashMap::new();
        while let Some(msg) = rx.recv().await {
            match msg {
                ProviderMessage::ClientConnectedNotify(m) => {
                    peers.insert(m.connection_id, m.endpoint_id);
                }
                ProviderMessage::ConnectionClosed(m) => {
                    peers.remove(&m.connection_id);
                }
                ProviderMessage::PushRequestReceived(m) => {
                    let remote = peers.get(&m.connection_id).copied().flatten();
                    let allowed = remote.is_some_and(|id| policy.can_write(&id));
                    if !allowed {
                        debug!("denied blob push from {remote:?}");
                    }
                    let verdict = if allowed {
                        Ok(())
                    } else {
                        Err(AbortReason::Permission)
                    };
                    let _ = m.tx.send(verdict).await;
                }
                _ => {}
            }
        }
    });
    EventSender::new(tx, mask)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use iroh_base::SecretKey;

    fn id(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }

    #[test]
    fn restricted_policy_only_allows_listed_nodes() {
        let p = AccessPolicy::restricted([id(1)]);
        assert!(p.can_write(&id(1)));
        assert!(!p.can_write(&id(2)));
        assert!(!p.is_open());
    }

    #[test]
    fn open_policy_allows_everyone() {
        let p = AccessPolicy::open();
        assert!(p.can_write(&id(9)));
        assert!(p.is_open());
    }

    #[test]
    fn allow_adds_writer_to_all_clones() {
        let p = AccessPolicy::restricted([]);
        let q = p.clone();
        assert!(!q.can_write(&id(3)));
        p.allow(id(3));
        assert!(q.can_write(&id(3)));
        assert_eq!(q.writers(), vec![id(3)]);
    }

    #[test]
    fn default_policy_denies_everyone() {
        let p = AccessPolicy::default();
        assert!(!p.can_write(&id(1)));
    }

    #[test]
    fn parse_allowed_nodes_skips_comments_blanks_and_garbage() {
        let a = id(4);
        let b = id(5);
        let text = format!("# owners\n{a}  # laptop\n\nnot-a-node-id\n{b}\n");
        assert_eq!(parse_allowed_nodes(&text), vec![a, b]);
    }

    #[test]
    fn load_allowed_nodes_missing_file_is_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(load_allowed_nodes(dir.path()).is_empty());
    }

    #[test]
    fn load_allowed_nodes_reads_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(ALLOWED_NODES_FILE), format!("{}\n", id(7))).unwrap();
        assert_eq!(load_allowed_nodes(dir.path()), vec![id(7)]);
    }

    /// Minimal `tracing` subscriber that records event fields, so tests
    /// can assert a diagnostic was (or was not) emitted (#6677).
    #[derive(Default)]
    struct CapturedEvents {
        lines: std::sync::Mutex<Vec<String>>,
    }

    struct CaptureSubscriber(std::sync::Arc<CapturedEvents>);

    impl tracing::Subscriber for CaptureSubscriber {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Visitor(String);
            impl tracing::field::Visit for Visitor {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    use std::fmt::Write;
                    let _ = write!(self.0, "{}={value:?} ", field.name());
                }
            }
            let mut visitor = Visitor(String::new());
            event.record(&mut visitor);
            self.0.lines.lock().unwrap().push(visitor.0);
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    fn captured_load(dir: &Path) -> (Vec<EndpointId>, Vec<String>) {
        let captured = std::sync::Arc::new(CapturedEvents::default());
        let loaded = tracing::subscriber::with_default(
            CaptureSubscriber(captured.clone()),
            || load_allowed_nodes(dir),
        );
        let lines = captured.lines.lock().unwrap().clone();
        (loaded, lines)
    }

    #[test]
    fn load_allowed_nodes_missing_file_is_empty_and_silent() {
        let dir = tempfile::TempDir::new().unwrap();
        let (loaded, lines) = captured_load(dir.path());
        assert!(loaded.is_empty());
        assert!(lines.is_empty(), "missing file must not warn: {lines:?}");
    }

    #[test]
    fn load_allowed_nodes_directory_in_place_of_file_warns_and_stays_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(ALLOWED_NODES_FILE)).unwrap();
        let (loaded, lines) = captured_load(dir.path());
        assert!(loaded.is_empty(), "fail-closed: a directory grants nothing");
        assert!(
            lines.iter().any(|l| l.contains(ALLOWED_NODES_FILE)),
            "unreadable file must be named in a diagnostic: {lines:?}"
        );
    }

    #[test]
    fn load_allowed_nodes_non_utf8_warns_and_stays_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        // UTF-16LE with BOM, as a Windows editor would save the file.
        std::fs::write(
            dir.path().join(ALLOWED_NODES_FILE),
            [0xFF, 0xFE, 0x61, 0x00, 0x0A, 0x00],
        )
        .unwrap();
        let (loaded, lines) = captured_load(dir.path());
        assert!(loaded.is_empty(), "fail-closed: non-UTF-8 grants nothing");
        assert!(
            lines.iter().any(|l| l.contains(ALLOWED_NODES_FILE)),
            "unreadable file must be named in a diagnostic: {lines:?}"
        );
    }
}
