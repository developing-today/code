//! File-level operations shared by the CLI, REPL, meta protocol and web UI.
//!
//! A "file" in `id` is a *name* (an iroh-blobs tag) pointing at an immutable
//! blob hash, plus a set of metadata tags in the [`TagStore`]. Operations that
//! touch both halves — `put`, `rename`, `copy`, `delete` — used to be
//! implemented separately in the web routes, the meta protocol and the REPL,
//! and they drifted apart (for example: copy over the meta protocol did not
//! copy metadata, and delete left metadata behind). This module is the single
//! implementation; every entry point calls it.
//!
//! # Semantics
//!
//! - [`FileOps::put`] verifies that the blob is present **before** creating the
//!   name, so a failed upload can never leave a dangling reference.
//! - [`FileOps::rename`] and [`FileOps::copy`] archive a file they replace
//!   (`<name>.archive.<unix-ts>`) and record `archive.replace` / `archive.rename`
//!   metadata. Metadata moves (rename) or is duplicated (copy) with the file,
//!   and the `name`/`file` identity tags are refreshed to the new name.
//! - [`FileOps::delete`] is a *hard* delete: the name, its metadata and its
//!   archive tags are removed. The blob bytes stay until garbage collected.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_lite::StreamExt;
use iroh_blobs::{Hash, api::Store};
use thiserror::Error;

use crate::tags::{TagStore, now_unix};

/// Errors from file operations, distinguishable so callers can map them to
/// protocol errors or HTTP status codes.
#[derive(Debug, Error)]
pub enum FileOpError {
    /// The new name was empty or whitespace.
    #[error("name cannot be empty")]
    EmptyName,
    /// Source and destination are the same name.
    #[error("new name must differ from the current name")]
    SameName,
    /// The named file does not exist.
    #[error("file not found: {0}")]
    NotFound(String),
    /// The blob to be named is not (completely) in the store.
    #[error("blob {0} is not present in the store (upload the content before naming it)")]
    BlobMissing(Hash),
    /// Any other storage failure.
    #[error(transparent)]
    Store(#[from] anyhow::Error),
}

/// Whether an error message from a server means "the blob is not (yet) there".
///
/// Clients use this to retry a `Put` while an upload is still being finalized.
pub fn is_blob_missing_message(message: &str) -> bool {
    message.contains("is not present in the store")
}

/// Result of [`FileOps::rename`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameOutcome {
    /// Hash of the renamed content.
    pub hash: Hash,
    /// Archive tag created for the original name, if any.
    pub archived_original: Option<String>,
    /// Archive tag created for a file that was replaced at the destination.
    pub archived_replaced: Option<String>,
}

/// Result of [`FileOps::copy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyOutcome {
    /// Hash of the copied content.
    pub hash: Hash,
    /// Archive tag created for a file that was replaced at the destination.
    pub archived_replaced: Option<String>,
}

/// Result of [`FileOps::delete`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteOutcome {
    /// Whether the name existed.
    pub existed: bool,
    /// Number of archive tags removed.
    pub archives_removed: usize,
    /// Number of metadata tags removed.
    pub metadata_removed: usize,
}

/// File operations over a blob [`Store`] and a [`TagStore`].
#[derive(Debug, Clone, Copy)]
pub struct FileOps<'a> {
    store: &'a Store,
    tags: &'a TagStore,
    blob_wait: Duration,
}

impl<'a> FileOps<'a> {
    /// Create file operations over the given stores.
    pub const fn new(store: &'a Store, tags: &'a TagStore) -> Self {
        Self {
            store,
            tags,
            blob_wait: Duration::ZERO,
        }
    }

    /// How long [`FileOps::put`] waits for a blob that is still being
    /// uploaded to become complete before declaring it missing.
    ///
    /// A pushing client can finish *sending* slightly before the server has
    /// finished *storing*, so a `Put` that immediately follows a push may
    /// arrive first. Servers set a few seconds here; local puts keep the
    /// default of zero, since their blob is already complete.
    #[must_use]
    pub const fn with_blob_wait(mut self, wait: Duration) -> Self {
        self.blob_wait = wait;
        self
    }

    /// Whether the blob is complete, waiting up to `blob_wait` for it.
    async fn blob_present(&self, hash: Hash) -> Result<bool> {
        let deadline = Instant::now() + self.blob_wait;
        loop {
            if self
                .store
                .blobs()
                .has(hash)
                .await
                .map_err(anyhow::Error::from)?
            {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn validate_target(from: &str, to: &str) -> Result<String, FileOpError> {
        let to = to.trim();
        if to.is_empty() {
            return Err(FileOpError::EmptyName);
        }
        if from == to {
            return Err(FileOpError::SameName);
        }
        Ok(to.to_owned())
    }

    /// Archive whatever currently lives at `name`, returning the archive tag.
    async fn archive_existing(&self, name: &str, ts: u64) -> Result<Option<String>> {
        let Some(existing) = self.store.tags().get(name).await? else {
            return Ok(None);
        };
        let archive = format!("{name}.archive.{ts}");
        self.store
            .tags()
            .set(&archive, existing.hash)
            .await
            .with_context(|| format!("archiving replaced file {name}"))?;
        Ok(Some(archive))
    }

    /// Name `hash` as `name`, after verifying the blob is complete in the store.
    ///
    /// Sets the `created` tag only if absent and always refreshes `modified`,
    /// then auto-tags `name`/`file`.
    pub async fn put(&self, name: &str, hash: Hash) -> Result<(), FileOpError> {
        if name.trim().is_empty() {
            return Err(FileOpError::EmptyName);
        }
        if !self.blob_present(hash).await? {
            return Err(FileOpError::BlobMissing(hash));
        }
        self.store
            .tags()
            .set(name, hash)
            .await
            .map_err(anyhow::Error::from)?;
        self.stamp(name, false).await;
        Ok(())
    }

    /// Record `created` (if absent or `reset_created`), `modified`, and identity tags.
    ///
    /// Metadata bookkeeping failures are logged, not fatal: the name already
    /// points at the right content.
    async fn stamp(&self, name: &str, reset_created: bool) {
        let ns = &self.tags.global;
        let now = now_unix().to_string();
        let subject = name.as_bytes();
        let created = if reset_created {
            self.tags
                .set_singleton(ns, subject, b"created", Some(now.as_bytes()), b"")
                .await
        } else {
            self.tags
                .set_if_absent(ns, subject, b"created", Some(now.as_bytes()), b"")
                .await
                .map(|_| ())
        };
        if let Err(e) = created {
            tracing::warn!("failed to set created tag for {name}: {e:#}");
        }
        if let Err(e) = self
            .tags
            .set_singleton(ns, subject, b"modified", Some(now.as_bytes()), b"")
            .await
        {
            tracing::warn!("failed to set modified tag for {name}: {e:#}");
        }
        if let Err(e) = self.tags.auto_tag(ns, subject, None).await {
            tracing::warn!("failed to auto-tag {name}: {e:#}");
        }
    }

    /// Rename `from` to `to`.
    ///
    /// A file already at `to` is archived first. When `archive` is true the
    /// original name is also archived. Metadata follows the file.
    pub async fn rename(
        &self,
        from: &str,
        to: &str,
        archive: bool,
    ) -> Result<RenameOutcome, FileOpError> {
        let to = Self::validate_target(from, to)?;
        let hash = self
            .store
            .tags()
            .get(from)
            .await
            .map_err(anyhow::Error::from)?
            .ok_or_else(|| FileOpError::NotFound(from.to_owned()))?
            .hash;
        let ts = now_unix();

        let archived_replaced = self.archive_existing(&to, ts).await?;
        self.store
            .tags()
            .set(&to, hash)
            .await
            .map_err(anyhow::Error::from)?;

        let archived_original = if archive {
            let name = format!("{from}.archive.{ts}");
            match self.store.tags().set(&name, hash).await {
                Ok(()) => Some(name),
                Err(e) => {
                    tracing::error!("failed to archive original {from}: {e}");
                    None
                }
            }
        } else {
            None
        };

        self.store
            .tags()
            .delete(from)
            .await
            .map_err(anyhow::Error::from)?;

        let ns = &self.tags.global;
        if let Err(e) = self
            .tags
            .transfer_all_tags(ns, from.as_bytes(), to.as_bytes())
            .await
        {
            tracing::warn!("failed to transfer tags {from} -> {to}: {e:#}");
        }
        if let Err(e) = self.tags.refresh_identity_tags(ns, to.as_bytes()).await {
            tracing::warn!("failed to refresh identity tags for {to}: {e:#}");
        }
        for (key, value) in [
            ("archive.rename", &archived_original),
            ("archive.replace", &archived_replaced),
        ] {
            if let Some(archive_name) = value
                && let Err(e) = self
                    .tags
                    .set_tag(
                        ns,
                        to.as_bytes(),
                        key.as_bytes(),
                        Some(archive_name.as_bytes()),
                        b"",
                    )
                    .await
            {
                tracing::warn!("failed to set {key} tag on {to}: {e:#}");
            }
        }

        Ok(RenameOutcome {
            hash,
            archived_original,
            archived_replaced,
        })
    }

    /// Copy `from` to `to` (a second name for the same content).
    ///
    /// A file already at `to` is archived first. Metadata is duplicated, the
    /// identity tags are refreshed, and the copy gets its own `created` and
    /// `modified` timestamps.
    pub async fn copy(&self, from: &str, to: &str) -> Result<CopyOutcome, FileOpError> {
        let to = Self::validate_target(from, to)?;
        let hash = self
            .store
            .tags()
            .get(from)
            .await
            .map_err(anyhow::Error::from)?
            .ok_or_else(|| FileOpError::NotFound(from.to_owned()))?
            .hash;
        let ts = now_unix();

        let archived_replaced = self.archive_existing(&to, ts).await?;
        self.store
            .tags()
            .set(&to, hash)
            .await
            .map_err(anyhow::Error::from)?;

        let ns = &self.tags.global;
        if let Err(e) = self
            .tags
            .copy_all_tags(ns, from.as_bytes(), to.as_bytes())
            .await
        {
            tracing::warn!("failed to copy tags {from} -> {to}: {e:#}");
        }
        if let Err(e) = self.tags.refresh_identity_tags(ns, to.as_bytes()).await {
            tracing::warn!("failed to refresh identity tags for {to}: {e:#}");
        }
        self.stamp(&to, true).await;
        if let Some(archive_name) = &archived_replaced
            && let Err(e) = self
                .tags
                .set_tag(
                    ns,
                    to.as_bytes(),
                    b"archive.replace",
                    Some(archive_name.as_bytes()),
                    b"",
                )
                .await
        {
            tracing::warn!("failed to set archive.replace tag on {to}: {e:#}");
        }

        Ok(CopyOutcome {
            hash,
            archived_replaced,
        })
    }

    /// Hard-delete `name`: the name itself, its metadata, and its archive tags.
    pub async fn delete(&self, name: &str) -> Result<DeleteOutcome, FileOpError> {
        if name.trim().is_empty() {
            return Err(FileOpError::EmptyName);
        }
        let existed = self
            .store
            .tags()
            .get(name)
            .await
            .map_err(anyhow::Error::from)?
            .is_some();
        self.store
            .tags()
            .delete(name)
            .await
            .map_err(anyhow::Error::from)?;

        let metadata_removed = self
            .tags
            .del_all_tags(&self.tags.global, name.as_bytes())
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("failed to delete metadata for {name}: {e:#}");
                0
            });

        // Archive tags are named "<name>.archive.<unix-ts>"; match the numeric
        // suffix exactly so unrelated files that merely share a prefix survive.
        let prefix = format!("{name}.archive.");
        let mut archives = Vec::new();
        let mut list = self
            .store
            .tags()
            .list()
            .await
            .map_err(anyhow::Error::from)?;
        while let Some(item) = list.next().await {
            let item = item.map_err(anyhow::Error::from)?;
            let tag = String::from_utf8_lossy(item.name.as_ref()).into_owned();
            if let Some(rest) = tag.strip_prefix(&prefix)
                && !rest.is_empty()
                && rest.bytes().all(|b| b.is_ascii_digit())
            {
                archives.push(tag);
            }
        }
        let mut archives_removed = 0;
        for tag in archives {
            match self.store.tags().delete(&tag).await {
                Ok(_) => archives_removed += 1,
                Err(e) => tracing::warn!("failed to delete archive tag {tag}: {e}"),
            }
        }

        Ok(DeleteOutcome {
            existed,
            archives_removed,
            metadata_removed,
        })
    }
}
