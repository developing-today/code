//! Host-mode admission around a hub's storage opener (#10693, #10698).
//!
//! `id serve --world-host` serves its worlds directory as a host data
//! root: every open a client asks for is first admitted by the host file
//! (an asleep world is recorded woken, an admin-authorised new world is
//! recorded created, an archived or operator-wake world is refused), and
//! taken back if the open then fails. An idle close is recorded asleep,
//! and the host's pins keep worlds open. [`HostedOpener`] puts that rule
//! around any [`WorldOpener`], so the serving node and the parity suite
//! run the same admission code.

use std::sync::Arc;

use crate::host::HostLifecycle;
use crate::world_hub::{OpenFuture, WorldOpener};

/// A storage opener under host-mode admission.
pub struct HostedOpener<O> {
    storage: O,
    host: Arc<HostLifecycle>,
}

impl<O: WorldOpener> HostedOpener<O> {
    /// Admit every open of `storage` through `host`.
    #[must_use]
    pub const fn new(storage: O, host: Arc<HostLifecycle>) -> Self {
        Self { storage, host }
    }

    /// The host file this opener records into.
    #[must_use]
    pub const fn host(&self) -> &Arc<HostLifecycle> {
        &self.host
    }

    /// The opener beneath the admission rule.
    #[must_use]
    pub const fn storage(&self) -> &O {
        &self.storage
    }
}

impl<O> std::fmt::Debug for HostedOpener<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostedOpener").finish_non_exhaustive()
    }
}

impl<O: WorldOpener> WorldOpener for HostedOpener<O> {
    fn open<'a>(&'a self, name: &'a str, create: bool) -> OpenFuture<'a> {
        Box::pin(async move {
            if !create && !self.storage.exists(name) {
                return Ok(None);
            }
            // Record the wake (or the create) before opening, and take it
            // back if the open fails, so the host file never shows a
            // world live that is not.
            let admission = if create {
                self.host.admit_create(name)?
            } else {
                self.host.admit_wake(name)?
            };
            let opened = self.storage.open(name, create).await;
            if !matches!(opened, Ok(Some(_)))
                && let Err(undo) = self.host.undo_wake(name, admission)
            {
                tracing::warn!(world = name, "host: could not undo wake: {undo:#}");
            }
            opened
        })
    }

    fn exists(&self, name: &str) -> bool {
        self.storage.exists(name)
    }

    fn list(&self) -> Vec<String> {
        self.storage.list()
    }

    fn durable(&self) -> bool {
        self.storage.durable()
    }

    fn stored_bytes(&self, name: &str) -> Option<u64> {
        self.storage.stored_bytes(name)
    }

    fn closed(&self, name: &str) {
        if let Err(error) = self.host.record_closed(name) {
            tracing::warn!(world = name, "host: could not record idle close: {error:#}");
        }
        self.storage.closed(name);
    }

    fn pinned(&self, name: &str) -> bool {
        self.host.pinned(name) || self.storage.pinned(name)
    }
}

/// A plain opener over a data root, as a local host opens worlds: each
/// world is `<root>/<name>`, opened by [`crate::host::open_in_root`]; an
/// admin-authorised create makes the directory first.
#[cfg(test)]
pub(crate) struct RootOpener {
    pub(crate) root: std::path::PathBuf,
}

#[cfg(test)]
impl WorldOpener for RootOpener {
    fn open<'a>(&'a self, name: &'a str, create: bool) -> OpenFuture<'a> {
        Box::pin(async move {
            if !self.exists(name) {
                if !create {
                    return Ok(None);
                }
                std::fs::create_dir_all(self.root.join(name))?;
            }
            let (service, _) = crate::host::open_in_root(&self.root, name).await?;
            Ok(Some(service))
        })
    }

    fn exists(&self, name: &str) -> bool {
        self.root
            .join(name)
            .join(crate::world_storage::JOURNAL_FILE)
            .is_file()
    }

    fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| self.exists(&entry.file_name().to_string_lossy()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        names
    }

    fn durable(&self) -> bool {
        true
    }

    fn stored_bytes(&self, name: &str) -> Option<u64> {
        std::fs::metadata(
            self.root
                .join(name)
                .join(crate::world_storage::JOURNAL_FILE),
        )
        .ok()
        .map(|meta| meta.len())
    }
}
