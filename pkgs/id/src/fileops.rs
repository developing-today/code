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

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_lite::StreamExt;
use iroh_blobs::{Hash, api::Store};
use thiserror::Error;

use crate::tags::{TagStore, TagValue, now_unix};

const ARCHIVE_IDENTITY_TAG_KEYS: [&[u8]; 5] = [
    b"name",
    b"file",
    b"path",
    // Archive references describe the live file's archives, so they
    // stay on the live name when its other metadata moves onto an
    // archive. Letting one travel onto the archive subject orphans
    // every earlier archive from the live file's delete (#4574).
    b"archive.rename",
    b"archive.replace",
];

/// Errors from file operations, distinguishable so callers can map them to
/// protocol errors or HTTP status codes.
#[derive(Debug, Error)]
pub enum FileOpError {
    /// The new name was empty or whitespace.
    #[error("name cannot be empty")]
    EmptyName,
    /// The name is rooted or contains a traversal component.
    #[error("name must be a safe relative path: {0}")]
    InvalidName(String),
    /// The name collides with the namespace used for system archives.
    #[error("name is reserved for archives: {0}")]
    ReservedArchiveName(String),
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
#[derive(Debug, Clone)]
pub struct FileOps<'a> {
    store: &'a Store,
    tags: &'a TagStore,
    blob_wait: Duration,
    /// Node that originated a write, stamped as the file's `origin` tag
    /// (#4973). `None` when the caller does not know its node id: no
    /// origin is stamped and listings show the file's origin as unknown.
    origin: Option<String>,
}

/// Serialises archive-name resolve+claim within this process (#2462).
///
/// On its own, resolving a free archive name is check-then-act: the name
/// comes back *unclaimed* and the caller claims it with a later
/// unconditional `tags().set`, so two concurrent same-second replaces of
/// one name could resolve the same candidate and the second claim would
/// silently overwrite the first replace's archive.
/// [`claim_archive_name`] holds this lock across the whole
/// resolve-and-set, so a name is never handed out unclaimed.
///
/// Scope: this is an in-process lock. It is sound because the `id` server
/// is the single process writing this tag store; a second writer process
/// sharing the store would need store-level compare-and-set instead.
static ARCHIVE_CLAIM_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The archive tag for `name` at `ts`, bumped past any archive tag that
/// already exists: `{name}.archive.{ts}`, then `{name}.archive.{ts + 1}`,
/// and so on until the name is free.
///
/// Archive names carry second-resolution timestamps, so two replaces of
/// one name within the same second would otherwise pick the same tag and
/// the second would silently overwrite the first replace's archive
/// (#1612). The suffix stays all digits: `delete` enumerates a file's
/// archives by exactly that shape, and the web layer parses the suffix
/// as the archive timestamp.
///
/// The returned name is NOT claimed: callers must go through
/// [`claim_archive_name`], which holds [`ARCHIVE_CLAIM_LOCK`]; this
/// helper may only be called with that lock held.
async fn free_archive_name(store: &Store, name: &str, ts: u64) -> Result<String> {
    let mut suffix = ts;
    loop {
        let candidate = format!("{name}.archive.{suffix}");
        if store.tags().get(&candidate).await?.is_none() {
            return Ok(candidate);
        }
        suffix += 1;
    }
}

/// Whether `name` has the archive tag shape used by FileOps.
fn is_archive_tag_name(name: &str) -> bool {
    let Some((file_name, suffix)) = name.rsplit_once(".archive.") else {
        return false;
    };
    !file_name.is_empty() && !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
}

/// Resolve a free archive name for `name` at `ts` and claim it for
/// `hash` in the same critical section (#2462): when this returns, the
/// tag already points at `hash`, and no concurrent claim in this process
/// can receive the same name.
pub(crate) async fn claim_archive_name(
    store: &Store,
    name: &str,
    ts: u64,
    hash: Hash,
) -> Result<String> {
    let _guard = ARCHIVE_CLAIM_LOCK.lock().await;
    let candidate = free_archive_name(store, name, ts).await?;
    store.tags().set(&candidate, hash).await?;
    Ok(candidate)
}

/// Provenance metadata keys recorded at a file's first write (#4973):
/// the origin node and the upload (`created`) timestamp. Listings read
/// a file's origin and upload time from these tags on its own name.
pub(crate) const PROVENANCE_TAG_KEYS: [&[u8]; 2] = [b"origin", b"created"];

/// Snapshot `from`'s recorded provenance tags onto `to` (#5383).
///
/// An archive alias points at the same blob as its source but lives
/// under a new name, so without this copy a listing reports the
/// archive's origin as unknown and origin filtering misses it. Tags
/// are copied by content hash so the recorded values are preserved
/// exactly; a source with no recorded provenance leaves `to` with
/// none — unknown stays unknown, provenance is never invented.
pub(crate) async fn snapshot_provenance_tags(tags: &TagStore, from: &str, to: &str) -> Result<()> {
    for key in PROVENANCE_TAG_KEYS {
        for tag in tags.get_by_key(&tags.global, from.as_bytes(), key).await? {
            tags.set_tag_hash(
                &tags.global,
                to.as_bytes(),
                &tag.key,
                tag.value.as_ref().map(TagValue::as_bytes),
                tag.hash,
                tag.len,
            )
            .await?;
        }
    }
    Ok(())
}

impl<'a> FileOps<'a> {
    /// Create file operations over the given stores.
    pub const fn new(store: &'a Store, tags: &'a TagStore) -> Self {
        Self {
            store,
            tags,
            blob_wait: Duration::ZERO,
            origin: None,
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

    /// Stamp writes with `origin` (a node id) as the file's origin node
    /// (#4973). The stamp is set-if-absent: the node that first wrote a
    /// file stays its origin across later writes and renames, and files
    /// written before origin stamping existed simply have no origin tag.
    #[must_use]
    pub fn with_origin(mut self, origin: String) -> Self {
        self.origin = Some(origin);
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

    /// The single canonical-name rule for every FileOps entry point.
    ///
    /// A name is stored exactly as given, so any name that cannot be
    /// re-specified identically is refused rather than stored under a
    /// look-alike: leading or trailing padding, empty components
    /// (`a//b`, `a/`, `a\`), rooted names, drive prefixes, and `.`/`..`
    /// components all fail with [`FileOpError::InvalidName`]. Forgiving
    /// surfaces (CLI, REPL, web) trim user input before calling in.
    fn validate_name(name: &str) -> Result<(), FileOpError> {
        if name.trim().is_empty() {
            return Err(FileOpError::EmptyName);
        }
        let bytes = name.as_bytes();
        let has_windows_drive_prefix =
            bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
        if name != name.trim()
            || name.starts_with('/')
            || name.starts_with('\\')
            || has_windows_drive_prefix
            || name
                .split(['/', '\\'])
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
        {
            return Err(FileOpError::InvalidName(name.to_owned()));
        }
        if is_archive_tag_name(name) {
            return Err(FileOpError::ReservedArchiveName(name.to_owned()));
        }
        Ok(())
    }

    /// Validate a rename/copy target without normalising it: the target is
    /// stored verbatim, so trimming here would create the padded-value twins
    /// the canonical-name rule exists to prevent.
    fn validate_target(from: &str, to: &str) -> Result<String, FileOpError> {
        Self::validate_name(to)?;
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
        // Archive references belong to the live file: they name the
        // archives a future delete of this file must also remove. The
        // transfer below discards them from this name (and the
        // exclusion list keeps them off the archive), so snapshot them
        // here and restore them onto the live name afterwards -
        // otherwise each replacement orphans every earlier archive.
        let mut archive_refs = Vec::new();
        for key in [&b"archive.rename"[..], &b"archive.replace"[..]] {
            for tag in self
                .tags
                .get_by_key(&self.tags.global, name.as_bytes(), key)
                .await?
            {
                if let Some(value) = tag.value.as_ref().and_then(|value| value.as_str()) {
                    archive_refs.push((key, value.to_owned()));
                }
            }
        }
        let archive = claim_archive_name(self.store, name, ts, existing.hash)
            .await
            .with_context(|| format!("archiving replaced file {name}"))?;
        self.tags
            .transfer_all_tags_except(
                &self.tags.global,
                name.as_bytes(),
                archive.as_bytes(),
                &ARCHIVE_IDENTITY_TAG_KEYS,
            )
            .await
            .with_context(|| format!("moving metadata for archived file {name}"))?;
        self.tags
            .refresh_identity_tags(&self.tags.global, archive.as_bytes())
            .await
            .with_context(|| format!("refreshing identity for archive {archive}"))?;
        for (key, value) in &archive_refs {
            self.tags
                .set_tag(
                    &self.tags.global,
                    name.as_bytes(),
                    key,
                    Some(value.as_bytes()),
                    b"",
                )
                .await
                .with_context(|| format!("restoring archive references for {name}"))?;
        }
        Ok(Some(archive))
    }

    /// Name `hash` as `name`, after verifying the blob is complete in the store.
    ///
    /// Sets the `created` tag only if absent and always refreshes `modified`,
    /// stamps the `origin` node on a genuine first write (no `created` tag
    /// recorded yet) if one is configured, then auto-tags `name`/`file`.
    /// A legacy file that predates origin stamping keeps an unknown origin
    /// when it is overwritten: absence of an `origin` tag alone does not
    /// prove this write is the file's first.
    pub async fn put(&self, name: &str, hash: Hash) -> Result<(), FileOpError> {
        Self::validate_name(name)?;
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
        // Origin is stamped only for a genuine first write: a name with a
        // `created` tag but no `origin` tag is a legacy file whose true
        // origin is unknown and must stay unknown (Codex review, PR #5191).
        let first_write = self
            .tags
            .get_by_key(ns, subject, b"created")
            .await
            .map(|existing| existing.is_empty())
            .unwrap_or(false);
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
        if first_write
            && let Some(origin) = &self.origin
            && let Err(e) = self
                .tags
                .set_if_absent(ns, subject, b"origin", Some(origin.as_bytes()), b"")
                .await
        {
            tracing::warn!("failed to set origin tag for {name}: {e:#}");
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
            match claim_archive_name(self.store, from, ts, hash).await {
                Ok(name) => Some(name),
                Err(e) => {
                    tracing::error!("failed to archive original {from}: {e}");
                    None
                }
            }
        } else {
            None
        };

        // The archive alias keeps the source's recorded provenance
        // (#5383): snapshot it now, before the transfer below moves
        // (and deletes) the source name's tags onto `to`.
        if let Some(archive_name) = &archived_original
            && let Err(e) = snapshot_provenance_tags(self.tags, from, archive_name).await
        {
            tracing::warn!("failed to snapshot provenance {from} -> {archive_name}: {e:#}");
        }

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
            .copy_all_tags_except(
                ns,
                from.as_bytes(),
                to.as_bytes(),
                &[b"archive.rename", b"archive.replace"],
            )
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

        // Renames move the file identity metadata, including these archive
        // references, to the current name. Read them before deleting metadata.
        let mut archives = BTreeSet::new();
        for key in [&b"archive.rename"[..], &b"archive.replace"[..]] {
            for tag in self
                .tags
                .get_by_key(&self.tags.global, name.as_bytes(), key)
                .await?
            {
                if let Some(archive_name) = tag.value.as_ref().and_then(|value| value.as_str()) {
                    if is_archive_tag_name(archive_name) {
                        archives.insert(archive_name.to_owned());
                    } else {
                        tracing::warn!("ignoring malformed archive reference on {name}");
                    }
                }
            }
        }

        // Keep the legacy/current-name scan for archives without metadata.
        // Collect before mutation so a listing failure cannot leave a partial
        // hard delete behind.
        let prefix = format!("{name}.archive.");
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
                archives.insert(tag);
            }
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

        let mut archives_removed = 0;
        for tag in archives {
            match self.store.tags().delete(&tag).await {
                Ok(_) => archives_removed += 1,
                Err(e) => tracing::warn!("failed to delete archive tag {tag}: {e}"),
            }
            // Archives created by rename/copy carry the replaced file's
            // metadata (archive_existing moves it onto the archive subject);
            // remove it too, or it stays searchable after the archive blob
            // tag is gone.
            if let Err(e) = self
                .tags
                .del_all_tags(&self.tags.global, tag.as_bytes())
                .await
            {
                tracing::warn!("failed to delete metadata for archive {tag}: {e:#}");
            }
        }

        Ok(DeleteOutcome {
            existed,
            archives_removed,
            metadata_removed,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod archive_name_tests {
    use iroh_blobs::{Hash, store::mem::MemStore};

    use super::*;

    fn hash(byte: u8) -> Hash {
        Hash::from([byte; 32])
    }

    #[test]
    fn fileops_rejects_archive_shaped_names_but_allows_lookalikes() {
        for name in [
            "report.archive.0",
            "report.archive.1700000000",
            "report.archive.18446744073709551616",
        ] {
            assert!(matches!(
                FileOps::validate_name(name),
                Err(FileOpError::ReservedArchiveName(found)) if found == name
            ));
        }

        for name in ["a.archive.notes", "a.archive.+1", "a.archive.١"] {
            assert!(FileOps::validate_name(name).is_ok(), "{name:?}");
        }
    }

    /// Two replaces of one name within the same second must not share an
    /// archive tag (#1612): the second archive is bumped to the next free
    /// numeric suffix, and both replaced versions stay archived.
    #[tokio::test]
    async fn same_second_archives_never_overwrite() {
        let store: Store = MemStore::new().into();
        let ts = 1_700_000_000_u64;

        let first = claim_archive_name(&store, "doc", ts, hash(1))
            .await
            .unwrap();
        assert_eq!(first, format!("doc.archive.{ts}"));

        let second = claim_archive_name(&store, "doc", ts, hash(2))
            .await
            .unwrap();
        assert_eq!(second, format!("doc.archive.{}", ts + 1));

        // Both archives survive, each still pointing at its own version.
        assert_eq!(
            store.tags().get(&first).await.unwrap().unwrap().hash,
            hash(1)
        );
        assert_eq!(
            store.tags().get(&second).await.unwrap().unwrap().hash,
            hash(2)
        );

        // A third same-second replace bumps past both.
        let third = claim_archive_name(&store, "doc", ts, hash(3))
            .await
            .unwrap();
        assert_eq!(third, format!("doc.archive.{}", ts + 2));
    }

    /// Concurrent same-second claims must never share a name (#2462):
    /// when resolve and claim were separate steps, interleaved resolves
    /// could pick the same candidate and one archive silently overwrote
    /// another. Barrier-synchronised claims must all land on distinct
    /// tags, each still pointing at its own version.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_same_second_claims_never_collide() {
        use std::sync::Arc;

        use tokio::sync::Barrier;

        let store: Store = MemStore::new().into();
        let store = Arc::new(store);
        let ts = 1_700_000_000_u64;
        const CLAIMS: u8 = 8;
        let barrier = Arc::new(Barrier::new(usize::from(CLAIMS)));
        let mut handles = Vec::new();
        for i in 0..CLAIMS {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                let name = claim_archive_name(&store, "doc", ts, hash(i + 1))
                    .await
                    .unwrap();
                (name, hash(i + 1))
            }));
        }
        let mut claimed = Vec::new();
        for handle in handles {
            claimed.push(handle.await.unwrap());
        }
        let mut names: Vec<&str> = claimed.iter().map(|(name, _)| name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), usize::from(CLAIMS), "archive claims collided");
        for (name, expected) in &claimed {
            assert_eq!(
                store.tags().get(name).await.unwrap().unwrap().hash,
                *expected,
                "archive {name} lost its version"
            );
        }
    }

    /// With no collision the archive name is exactly the historic shape.
    #[tokio::test]
    async fn uncontested_archive_name_is_unchanged() {
        let store: Store = MemStore::new().into();
        let name = claim_archive_name(&store, "doc", 42, hash(7))
            .await
            .unwrap();
        assert_eq!(name, "doc.archive.42");
        assert_eq!(
            store.tags().get(&name).await.unwrap().unwrap().hash,
            hash(7)
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod replacement_metadata_tests {
    use crate::local::LocalNode;
    use crate::tags::Tag;

    use super::*;

    async fn put(node: &LocalNode, name: &str, content: &[u8]) {
        let guard = node
            .blobs()
            .add_bytes(content.to_vec())
            .temp_tag()
            .await
            .unwrap();
        node.ops().put(name, guard.hash()).await.unwrap();
    }

    async fn set_tag(node: &LocalNode, subject: &str, key: &str, value: &str) {
        node.tags()
            .set_tag(
                &node.tags().global,
                subject.as_bytes(),
                key.as_bytes(),
                Some(value.as_bytes()),
                b"",
            )
            .await
            .unwrap();
    }

    async fn set_singleton(node: &LocalNode, subject: &str, key: &str, value: &str) {
        node.tags()
            .set_singleton(
                &node.tags().global,
                subject.as_bytes(),
                key.as_bytes(),
                Some(value.as_bytes()),
                b"",
            )
            .await
            .unwrap();
    }

    async fn tags_of(node: &LocalNode, subject: &str) -> Vec<Tag> {
        node.tags()
            .get_tags(&node.tags().global, subject.as_bytes())
            .await
            .unwrap()
    }

    fn values(tags: &[Tag], key: &str) -> Vec<Vec<u8>> {
        tags.iter()
            .filter(|tag| tag.key.as_bytes() == key.as_bytes())
            .filter_map(|tag| tag.value.as_ref().map(|value| value.as_bytes().to_vec()))
            .collect()
    }

    fn has_tag(tags: &[Tag], key: &str, value: &str) -> bool {
        values(tags, key)
            .iter()
            .any(|found| found == value.as_bytes())
    }

    fn assert_single_value(tags: &[Tag], key: &str, value: &str) {
        assert_eq!(
            values(tags, key),
            vec![value.as_bytes().to_vec()],
            "{tags:?}"
        );
    }

    fn assert_archive_identity(tags: &[Tag], archive: &str) {
        assert_single_value(tags, "name", archive);
        assert_single_value(tags, "file", archive.rsplit('/').next().unwrap_or(archive));
        assert!(
            values(tags, "path").is_empty(),
            "archive inherited path: {tags:?}"
        );
    }

    async fn seed_replacement_pair(node: &LocalNode) {
        put(node, "source.txt", b"new content").await;
        put(node, "destination.txt", b"replaced content").await;

        set_tag(node, "source.txt", "shared", "source-value").await;
        set_tag(node, "source.txt", "source-only", "source-metadata").await;
        set_singleton(node, "source.txt", "created", "source-created").await;
        set_singleton(node, "source.txt", "modified", "source-modified").await;

        set_tag(node, "destination.txt", "shared", "replaced-value").await;
        set_tag(
            node,
            "destination.txt",
            "replaced-only",
            "replaced-metadata",
        )
        .await;
        set_tag(
            node,
            "destination.txt",
            "archive.previous",
            "earlier.archive.7",
        )
        .await;
        set_tag(node, "destination.txt", "path", "old/source/path.txt").await;
        set_singleton(node, "destination.txt", "created", "replaced-created").await;
        set_singleton(node, "destination.txt", "modified", "replaced-modified").await;
    }

    fn assert_replaced_metadata_on_archive(tags: &[Tag], archive: &str) {
        assert!(has_tag(tags, "shared", "replaced-value"), "{tags:?}");
        assert!(
            has_tag(tags, "replaced-only", "replaced-metadata"),
            "{tags:?}"
        );
        assert!(
            has_tag(tags, "archive.previous", "earlier.archive.7"),
            "{tags:?}"
        );
        assert_single_value(tags, "created", "replaced-created");
        assert_single_value(tags, "modified", "replaced-modified");
        assert_archive_identity(tags, archive);
        assert!(!has_tag(tags, "name", "destination.txt"), "{tags:?}");
        assert!(!has_tag(tags, "file", "destination.txt"), "{tags:?}");
    }

    #[tokio::test]
    async fn rename_with_archive_preserves_source_provenance_on_archive_alias() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "before.txt", b"original").await;
        set_singleton(&node, "before.txt", "origin", "node-alpha").await;
        set_singleton(&node, "before.txt", "created", "1700000000").await;

        // The source's recorded provenance, exactly as stored.
        let source_tags = tags_of(&node, "before.txt").await;
        assert_single_value(&source_tags, "origin", "node-alpha");
        let source_created = values(&source_tags, "created");
        assert!(!source_created.is_empty(), "{source_tags:?}");

        let rename = node
            .ops()
            .rename("before.txt", "after.txt", true)
            .await
            .unwrap();
        let archive = rename.archived_original.unwrap();

        // #5383: the archive alias carries the source's recorded
        // provenance, not an unknown origin.
        let archive_tags = tags_of(&node, &archive).await;
        assert_single_value(&archive_tags, "origin", "node-alpha");
        assert_eq!(values(&archive_tags, "created"), source_created);

        // The live name keeps its provenance too (tags move with it).
        let live_tags = tags_of(&node, "after.txt").await;
        assert_single_value(&live_tags, "origin", "node-alpha");
        assert_eq!(values(&live_tags, "created"), source_created);

        node.shutdown().await.unwrap();
    }

    async fn rename_onto_existing_moves_replaced_metadata_and_refreshes_archive_identity() {
        let node = LocalNode::open(true).await.unwrap();
        seed_replacement_pair(&node).await;

        let outcome = node
            .ops()
            .rename("source.txt", "destination.txt", false)
            .await
            .unwrap();
        let archive = outcome.archived_replaced.expect("replaced file archived");

        let archived = tags_of(&node, &archive).await;
        assert_replaced_metadata_on_archive(&archived, &archive);

        let destination = tags_of(&node, "destination.txt").await;
        assert!(
            has_tag(&destination, "shared", "source-value"),
            "{destination:?}"
        );
        assert!(
            has_tag(&destination, "source-only", "source-metadata"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "shared", "replaced-value"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "replaced-only", "replaced-metadata"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "archive.previous", "earlier.archive.7"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "path", "old/source/path.txt"),
            "{destination:?}"
        );
        assert_single_value(&destination, "created", "source-created");
        assert_single_value(&destination, "modified", "source-modified");
        assert_single_value(&destination, "name", "destination.txt");
        assert_single_value(&destination, "file", "destination.txt");
        assert!(
            has_tag(&destination, "archive.replace", &archive),
            "{destination:?}"
        );

        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn copy_onto_existing_moves_replaced_metadata_and_keeps_single_fresh_stamps() {
        let node = LocalNode::open(true).await.unwrap();
        seed_replacement_pair(&node).await;

        let outcome = node
            .ops()
            .copy("source.txt", "destination.txt")
            .await
            .unwrap();
        let archive = outcome.archived_replaced.expect("replaced file archived");

        let archived = tags_of(&node, &archive).await;
        assert_replaced_metadata_on_archive(&archived, &archive);

        let destination = tags_of(&node, "destination.txt").await;
        assert!(
            has_tag(&destination, "shared", "source-value"),
            "{destination:?}"
        );
        assert!(
            has_tag(&destination, "source-only", "source-metadata"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "shared", "replaced-value"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "replaced-only", "replaced-metadata"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "archive.previous", "earlier.archive.7"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "path", "old/source/path.txt"),
            "{destination:?}"
        );
        assert_eq!(values(&destination, "created").len(), 1, "{destination:?}");
        assert_eq!(values(&destination, "modified").len(), 1, "{destination:?}");
        assert_single_value(&destination, "name", "destination.txt");
        assert_single_value(&destination, "file", "destination.txt");
        assert!(
            has_tag(&destination, "archive.replace", &archive),
            "{destination:?}"
        );

        let source = tags_of(&node, "source.txt").await;
        assert!(has_tag(&source, "shared", "source-value"), "{source:?}");
        assert!(
            has_tag(&source, "source-only", "source-metadata"),
            "{source:?}"
        );

        node.shutdown().await.unwrap();
    }

    /// A copy must not become a second owner of the source's archives
    /// (#4761): deleting the copy must leave the source's archives intact.
    #[tokio::test]
    async fn copy_does_not_inherit_source_archive_refs() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "source.txt", b"source content").await;
        put(&node, "other.txt", b"other content").await;
        // Renaming onto source.txt with archiving enabled adds both
        // archive.rename and archive.replace refs to the live source.
        let rename_outcome = node
            .ops()
            .rename("other.txt", "source.txt", true)
            .await
            .unwrap();
        let source_archives = [
            rename_outcome
                .archived_original
                .expect("original source archived"),
            rename_outcome
                .archived_replaced
                .expect("replaced source archived"),
        ];
        let source_before = tags_of(&node, "source.txt").await;
        assert!(
            has_tag(&source_before, "archive.rename", &source_archives[0]),
            "{source_before:?}"
        );
        assert!(
            has_tag(&source_before, "archive.replace", &source_archives[1]),
            "{source_before:?}"
        );

        node.ops().copy("source.txt", "copy.txt").await.unwrap();
        let copy_tags = tags_of(&node, "copy.txt").await;
        assert!(
            values(&copy_tags, "archive.rename").is_empty(),
            "copy inherited archive.rename refs: {copy_tags:?}"
        );
        assert!(
            values(&copy_tags, "archive.replace").is_empty(),
            "copy inherited archive.replace refs: {copy_tags:?}"
        );

        // Deleting the copy must not touch the source's archive.
        let delete_outcome = node.ops().delete("copy.txt").await.unwrap();
        assert_eq!(
            delete_outcome.archives_removed, 0,
            "copy delete removed archives"
        );
        for archive in &source_archives {
            assert!(
                !tags_of(&node, archive).await.is_empty(),
                "source archive {archive} removed by copy delete"
            );
        }
        let source_after = tags_of(&node, "source.txt").await;
        assert!(
            has_tag(&source_after, "archive.rename", &source_archives[0]),
            "source lost its archive.rename ref: {source_after:?}"
        );
        assert!(
            has_tag(&source_after, "archive.replace", &source_archives[1]),
            "source lost its archive ref: {source_after:?}"
        );

        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rename_onto_existing_preserves_replaced_tag_payloads() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "source.txt", b"new content").await;
        put(&node, "destination.txt", b"replaced content").await;

        // A tag with a non-empty entry payload, as written for legacy
        // links by migrate_legacy_meta (payload carries the link target).
        let payload: &[u8] = b"hash:0123abcd";
        node.tags()
            .set_tag(
                &node.tags().global,
                b"destination.txt",
                b"link",
                Some(b"target"),
                payload,
            )
            .await
            .unwrap();
        let before = tags_of(&node, "destination.txt").await;
        let before = before
            .iter()
            .find(|tag| tag.key.as_bytes() == b"link")
            .expect("link tag seeded");
        assert_eq!(before.len as usize, payload.len(), "{before:?}");
        let before_hash = before.hash;

        let outcome = node
            .ops()
            .rename("source.txt", "destination.txt", false)
            .await
            .unwrap();
        let archive = outcome.archived_replaced.expect("replaced file archived");

        let archived = tags_of(&node, &archive).await;
        let moved = archived
            .iter()
            .find(|tag| tag.key.as_bytes() == b"link")
            .expect("link tag moved to archive");
        assert_eq!(moved.hash, before_hash, "{moved:?}");
        assert_eq!(moved.len as usize, payload.len(), "{moved:?}");

        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn delete_removes_archived_replaced_metadata() {
        let node = LocalNode::open(true).await.unwrap();
        seed_replacement_pair(&node).await;

        let outcome = node
            .ops()
            .rename("source.txt", "destination.txt", false)
            .await
            .unwrap();
        let archive = outcome.archived_replaced.expect("replaced file archived");
        assert!(
            !tags_of(&node, &archive).await.is_empty(),
            "archive carries the replaced file's metadata before delete"
        );

        let deleted = node.ops().delete("destination.txt").await.unwrap();
        assert_eq!(deleted.archives_removed, 1);
        assert!(
            tags_of(&node, &archive).await.is_empty(),
            "archive metadata removed with the archive"
        );

        node.shutdown().await.unwrap();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod delete_tests {
    use crate::local::LocalNode;

    use super::*;

    async fn put(node: &LocalNode, name: &str, data: &[u8]) {
        let guard = node
            .blobs()
            .add_bytes(data.to_vec())
            .temp_tag()
            .await
            .unwrap();
        node.ops().put(name, guard.hash()).await.unwrap();
    }

    #[tokio::test]
    async fn delete_after_archived_rename_removes_previous_name_archive() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "before", b"original").await;

        let rename = node.ops().rename("before", "after", true).await.unwrap();
        let archive = rename.archived_original.unwrap();
        assert!(node.blobs().tags().get(&archive).await.unwrap().is_some());

        let deleted = node.ops().delete("after").await.unwrap();

        assert_eq!(deleted.archives_removed, 1);
        assert!(node.blobs().tags().get(&archive).await.unwrap().is_none());
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn delete_after_repeated_replacements_removes_all_replaced_archives() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "A", b"original").await;

        put(&node, "replacement-1", b"first replacement").await;
        let first = node
            .ops()
            .rename("replacement-1", "A", false)
            .await
            .unwrap()
            .archived_replaced
            .unwrap();
        put(&node, "replacement-2", b"second replacement").await;
        let second = node
            .ops()
            .rename("replacement-2", "A", false)
            .await
            .unwrap()
            .archived_replaced
            .unwrap();
        assert_ne!(first, second);

        node.ops().rename("A", "B", false).await.unwrap();
        let deleted = node.ops().delete("B").await.unwrap();

        assert_eq!(deleted.archives_removed, 2);
        assert!(node.blobs().tags().get(&first).await.unwrap().is_none());
        assert!(node.blobs().tags().get(&second).await.unwrap().is_none());
        node.shutdown().await.unwrap();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod name_rule_tests {
    use crate::local::LocalNode;

    use super::*;

    async fn put(node: &LocalNode, name: &str, data: &[u8]) {
        let guard = node
            .blobs()
            .add_bytes(data.to_vec())
            .temp_tag()
            .await
            .unwrap();
        node.ops().put(name, guard.hash()).await.unwrap();
    }

    fn assert_invalid(name: &str) {
        assert!(
            matches!(
                FileOps::validate_name(name),
                Err(FileOpError::InvalidName(_))
            ),
            "{name:?} must be refused as InvalidName"
        );
    }

    /// Padding and empty components are refused by the one canonical rule
    /// (#4102 / design #4766 Phase 3), so a name can never be stored in a
    /// form that rename/copy would normalise away.
    #[test]
    fn refuses_padding_and_empty_components() {
        for name in [
            " report.txt",
            "report.txt ",
            "\treport.txt",
            "report.txt\n",
            "a//b",
            "a/",
            "a\\",
            "a//b/c",
            "a/b/",
        ] {
            assert_invalid(name);
        }
    }

    /// The pre-existing refusals stay in place: rooted names, drive
    /// prefixes, and `.`/`..` traversal components.
    #[test]
    fn preserves_traversal_refusals() {
        for name in [
            "/etc/passwd",
            "\\windows",
            "C:file.txt",
            ".",
            "..",
            "a/./b",
            "a/../b",
        ] {
            assert_invalid(name);
        }
        assert!(matches!(
            FileOps::validate_name("   "),
            Err(FileOpError::EmptyName)
        ));
        // Padding is a refusal, but interior spaces in a name are not.
        assert!(FileOps::validate_name("my report.txt").is_ok());
        assert!(FileOps::validate_name("a/b").is_ok());
    }

    /// A padded `put` cannot create an untargetable twin, and a padded
    /// rename/copy target is refused instead of being silently trimmed.
    #[tokio::test]
    async fn padded_names_cannot_create_twins() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "report.txt", b"report").await;

        let guard = node
            .blobs()
            .add_bytes(b"report".to_vec())
            .temp_tag()
            .await
            .unwrap();
        assert!(matches!(
            node.ops().put(" report.txt", guard.hash()).await,
            Err(FileOpError::InvalidName(_))
        ));
        assert!(
            node.blobs()
                .tags()
                .get(" report.txt")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            node.blobs()
                .tags()
                .get("report.txt")
                .await
                .unwrap()
                .is_some()
        );

        assert!(matches!(
            node.ops().rename("report.txt", "other.txt ", false).await,
            Err(FileOpError::InvalidName(_))
        ));
        assert!(
            node.blobs()
                .tags()
                .get("other.txt")
                .await
                .unwrap()
                .is_none()
        );

        assert!(matches!(
            node.ops().copy("report.txt", " copy.txt").await,
            Err(FileOpError::InvalidName(_))
        ));
        assert!(
            node.blobs()
                .tags()
                .get(" copy.txt")
                .await
                .unwrap()
                .is_none()
        );
        assert!(node.blobs().tags().get("copy.txt").await.unwrap().is_none());

        node.shutdown().await.unwrap();
    }

    /// Empty components are refused at put and at rename/copy targets alike.
    #[tokio::test]
    async fn empty_components_refused_across_put_rename_copy() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "a/b", b"body").await;

        let guard = node
            .blobs()
            .add_bytes(b"body".to_vec())
            .temp_tag()
            .await
            .unwrap();
        for name in ["a//b", "a/", "a\\"] {
            assert!(matches!(
                node.ops().put(name, guard.hash()).await,
                Err(FileOpError::InvalidName(_))
            ));
            assert!(matches!(
                node.ops().rename("a/b", name, false).await,
                Err(FileOpError::InvalidName(_))
            ));
            assert!(matches!(
                node.ops().copy("a/b", name).await,
                Err(FileOpError::InvalidName(_))
            ));
            assert!(node.blobs().tags().get(name).await.unwrap().is_none());
        }

        // The canonical source is untouched and ordinary names still work.
        assert!(node.blobs().tags().get("a/b").await.unwrap().is_some());
        node.ops().copy("a/b", "a/c").await.unwrap();
        assert!(node.blobs().tags().get("a/c").await.unwrap().is_some());

        node.shutdown().await.unwrap();
    }
}
