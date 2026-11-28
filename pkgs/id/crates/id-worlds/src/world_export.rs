//! World snapshot/export for custody (#10695, packet P3 of the hosted-worlds
//! v1 design `doc/2026-11-16_design_id6283_hosted_worlds_v1.md` §5).
//!
//! An export is one world as a self-describing file of signed F12.13
//! replication frames (#806): a JSON manifest frame, then exactly the
//! snapshot batch a replication session would send — the journal's exact
//! entry bytes, the directory facts and the active module, closed by a
//! signed terminal frame. Every frame is built by the same code that
//! serves replicas ([`crate::world_replication_net`]), so an export is
//! never a second format to keep in step.
//!
//! [`import_world`] restores one: every frame is verified under a source
//! key the operator pins, staged in a scratch directory beside the
//! destination, and renamed into place only after the terminal frame
//! commits the whole batch. A tampered, truncated or wrongly signed file
//! leaves nothing behind. The restored directory is an ordinary world
//! directory (`journal.jsonl`, `directory.jsonl`, `modules/`), byte for
//! byte the journal the exporting host held. Encryption at rest stays the
//! deployment's property: the file is readable by any host.
//!
//! ## Incremental sets and pruning (#13747 slice 1)
//!
//! A full export is set zero of a custody chain. [`export_incremental_set`]
//! writes a later set: only the journal entries at and after a base ordinal,
//! as tail frames chained from the cursor covering them, with the same
//! sidecars and terminal. [`verify_export_set`] verifies one set file under
//! the pinned source without trusting its sender or its manifest, and
//! [`check_set_chain`] proves ordered sets link into one continuous chain
//! (each set starts where the previous committed, ordinal and digest).
//! [`prune_export_sets`] is the retention policy: whole set files covered
//! by a newer snapshot may be deleted; a set file is never edited, and the
//! live chain (newest snapshot forward) is never a pruning candidate.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

use crate::frames::{read_frame_from, write_frame_to};
use crate::world::{
    DurableReceiver, REPLICATION_ENVELOPE_VERSION, ReceiverProgress, ReplicationCursor,
    ReplicationKind, SignedReplicationEnvelope, advance_cursor, chain_base_cursor, frame_digest,
    payload_digest,
};
use crate::world_replication_net::{
    FrameSink, fresh_session_nonce, gather_sidecars, send_incremental, send_snapshot,
};
use crate::world_session::{WorldService, decode_hex, encode_hex};

/// Which set of a world's custody chain one export file holds (#13747
/// slice 1). A full export is a snapshot from the chain base; later sets
/// are incrementals chained to the set covering their base ordinal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportSetKind {
    /// A full journal image from the chain base (`base_ordinal` 0).
    #[default]
    Snapshot,
    /// Only the entries at and after `base_ordinal`, chained to the set
    /// that covers them.
    Incremental,
}

/// The format named by every export manifest this module writes.
pub const EXPORT_FORMAT: &str = "id-world-export/1";

/// The first frame of an export: what the signed frames after it hold.
/// It is not signed itself; [`import_world`] checks every claim it makes
/// against the signed frames, so a forged manifest restores nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportManifest {
    /// [`EXPORT_FORMAT`].
    pub format: String,
    /// The world's ID, as its journal's `Created` header names it.
    pub world: String,
    /// Lowercase hex of the Ed25519 public key that signed every frame.
    pub source: String,
    /// The journal header version.
    pub header_version: u32,
    /// Journal entries covered through this set: the set's own entry
    /// count for a snapshot, `base_ordinal` plus the new entries for an
    /// incremental. The next set in a chain starts here.
    pub entries: u64,
    /// Lowercase hex of the blake3 hash of the exact journal bytes this
    /// export was taken from. Unlike the entry count, this binds the
    /// manifest to the journal's content: two different journals (for
    /// example after checkpoint compaction) can share an entry count but
    /// never this digest. Unlike the export envelope, it is stable
    /// across exports of the same journal — the per-export nonce is not
    /// part of it. Absent in exports written before this field existed.
    #[serde(default)]
    pub journal_digest: String,
    /// Lowercase hex of the session nonce every frame is bound to.
    pub nonce: String,
    /// Which set of the custody chain this file holds. Absent — a P3
    /// `id-world-export/1` file — means a full snapshot.
    #[serde(default)]
    pub set_kind: ExportSetKind,
    /// Entry ordinal this set starts from: 0 for a snapshot, the covered
    /// entry count of the previous set for an incremental. Absent means 0.
    #[serde(default)]
    pub base_ordinal: u64,
}

/// Writes frames length-prefixed, as the replication transport does.
struct WriterSink<'a, W>(&'a mut W);

impl<W: AsyncWrite + Unpin + Send> FrameSink for WriterSink<'_, W> {
    async fn put(&mut self, frame: &[u8]) -> Result<()> {
        write_frame_to(self.0, frame).await
    }
}

/// Export the world `service` holds to `out`, signed by `signing_key`.
/// The world keeps serving: the snapshot is the same consistent image a
/// replica receives at the start of a session.
///
/// # Errors
///
/// Fails when the world cannot produce a replication snapshot (for
/// instance a journal entry too large for one frame), when its active
/// module cannot be loaded, and on any write error.
pub async fn export_world<W: AsyncWrite + Unpin + Send>(
    service: &WorldService,
    signing_key: &SigningKey,
    out: &mut W,
) -> Result<ExportManifest> {
    let (world_id, snapshot_source, _updates) = service.world().replication_feed().await?;
    let snapshot = tokio::task::spawn_blocking(move || snapshot_source.read())
        .await
        .context("export snapshot read task")??;
    let sidecars = gather_sidecars(service, &snapshot.bytes).await?;
    let nonce = fresh_session_nonce();
    let source = *signing_key.verifying_key().as_bytes();
    let entries = snapshot.bytes.iter().filter(|byte| **byte == b'\n').count();
    let manifest = ExportManifest {
        format: EXPORT_FORMAT.to_owned(),
        world: world_id.clone(),
        source: encode_hex(&source),
        header_version: snapshot.header_version,
        entries: u64::try_from(entries).unwrap_or(u64::MAX),
        journal_digest: encode_hex(blake3::hash(&snapshot.bytes).as_bytes()),
        nonce: encode_hex(&nonce),
        set_kind: ExportSetKind::Snapshot,
        base_ordinal: 0,
    };
    write_frame_to(out, &serde_json::to_vec(&manifest)?).await?;
    let mut sink = WriterSink(&mut *out);
    send_snapshot(
        &mut sink,
        signing_key,
        &world_id,
        source,
        nonce,
        snapshot,
        &sidecars,
        [0; 32],
    )
    .await?;
    out.flush().await?;
    Ok(manifest)
}

/// Export the world `service` holds to the file `path`, written beside it
/// first and renamed into place once complete and fsynced.
///
/// # Errors
///
/// Refuses an existing `path`; otherwise as [`export_world`].
pub async fn export_world_to_file(
    service: &WorldService,
    signing_key: &SigningKey,
    path: &Path,
) -> Result<ExportManifest> {
    export_world_to_file_with_dir_sync(service, signing_key, path, &sync_export_parent_dir).await
}

/// Make the rename of an exported file durable by syncing its parent
/// directory (#16179), mirroring `HostFile::save`: without it, a power
/// loss can lose the renamed file even though the export returned `Ok`.
/// Best-effort no-op off unix, mirroring the `#[cfg(unix)]` shape in
/// `host.rs`.
///
/// # Errors
///
/// The parent directory cannot be opened, or its sync fails.
fn sync_export_parent_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .with_context(|| format!("fsync export directory {}", parent.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// [`export_world_to_file`] with the parent-directory sync step
/// supplied, split out so tests can fault-inject a sync failure (the
/// same shape as the #16058/#16131/#16140 durability tests).
async fn export_world_to_file_with_dir_sync(
    service: &WorldService,
    signing_key: &SigningKey,
    path: &Path,
    sync_dir: &(dyn Fn(&Path) -> Result<()> + Send + Sync),
) -> Result<ExportManifest> {
    if tokio::fs::symlink_metadata(path).await.is_ok() {
        bail!("{} already exists: export to a new file", path.display());
    }
    let partial = sibling(path, "partial");
    let result = async {
        let mut file = tokio::fs::File::create(&partial)
            .await
            .with_context(|| format!("creating {}", partial.display()))?;
        let manifest = export_world(service, signing_key, &mut file).await?;
        file.sync_all()
            .await
            .with_context(|| format!("fsync {}", partial.display()))?;
        drop(file);
        tokio::fs::rename(&partial, path)
            .await
            .with_context(|| format!("renaming {} to {}", partial.display(), path.display()))?;
        // Make the rename itself durable before reporting success: the
        // directory entry is synced after the rename, and a sync failure
        // is propagated rather than acking a non-durable export (#16179).
        sync_dir(path)?;
        Ok(manifest)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&partial).await;
    }
    result
}

/// Export only the journal entries at and after `base_ordinal` (#13747
/// slice 1): the tail of the same consistent snapshot [`export_world`]
/// reads, as tail frames chained from the cursor covering `base_ordinal`,
/// with the current sidecars and a terminal. `base_ordinal` is normally
/// the `entries` of the previous set in the custody chain; exporting past
/// the journal's end is refused, while exporting exactly at it writes a
/// valid empty incremental (no journal frames, sidecars and terminal only).
///
/// # Errors
///
/// Fails when `base_ordinal` is beyond the journal's entry count, when the
/// world cannot produce a replication snapshot, and on any write error.
pub async fn export_incremental_set<W: AsyncWrite + Unpin + Send>(
    service: &WorldService,
    signing_key: &SigningKey,
    base_ordinal: u64,
    out: &mut W,
) -> Result<ExportManifest> {
    let (world_id, snapshot_source, _updates) = service.world().replication_feed().await?;
    let snapshot = tokio::task::spawn_blocking(move || snapshot_source.read())
        .await
        .context("export snapshot read task")??;
    let source = *signing_key.verifying_key().as_bytes();
    let tail_offset = tail_offset_at(&snapshot.bytes, base_ordinal)?;
    let mut base = chain_base_cursor(&world_id, &source, snapshot.header_version);
    for line in snapshot.bytes[..tail_offset].split_inclusive(|byte| *byte == b'\n') {
        base = advance_cursor(base, line);
    }
    let sidecars = gather_sidecars(service, &snapshot.bytes).await?;
    let nonce = fresh_session_nonce();
    let total = snapshot.bytes.iter().filter(|byte| **byte == b'\n').count();
    let manifest = ExportManifest {
        format: EXPORT_FORMAT.to_owned(),
        world: world_id.clone(),
        source: encode_hex(&source),
        header_version: snapshot.header_version,
        entries: u64::try_from(total).unwrap_or(u64::MAX),
        journal_digest: encode_hex(blake3::hash(&snapshot.bytes).as_bytes()),
        nonce: encode_hex(&nonce),
        set_kind: ExportSetKind::Incremental,
        base_ordinal,
    };
    write_frame_to(out, &serde_json::to_vec(&manifest)?).await?;
    let mut sink = WriterSink(&mut *out);
    send_incremental(
        &mut sink,
        signing_key,
        &world_id,
        source,
        nonce,
        snapshot.header_version,
        base,
        &snapshot.bytes[tail_offset..],
        &sidecars,
        [0; 32],
    )
    .await?;
    out.flush().await?;
    Ok(manifest)
}

/// Byte offset of the first journal entry at `base_ordinal`: the journal
/// is entry lines, so skipping the covered prefix is exact — no entry is
/// ever split or rewritten.
///
/// # Errors
///
/// Fails when `base_ordinal` names more entries than `journal` holds.
fn tail_offset_at(journal: &[u8], base_ordinal: u64) -> Result<usize> {
    let mut offset = 0usize;
    let mut seen = 0u64;
    for line in journal.split_inclusive(|byte| *byte == b'\n') {
        if seen == base_ordinal {
            break;
        }
        offset += line.len();
        seen += 1;
    }
    if seen != base_ordinal {
        bail!("base ordinal {base_ordinal} is beyond the journal's {seen} entries");
    }
    Ok(offset)
}

/// What [`verify_export_set`] established for one set file: the set's own
/// frames verify under the pinned source and form one continuous run, from
/// `cursor_before` to `cursor_after`. Chaining sets together is
/// [`check_set_chain`]'s job.
#[derive(Clone, Debug)]
pub struct VerifiedExportSet {
    /// The set's manifest, checked against its signed frames.
    pub manifest: ExportManifest,
    /// Chain position the set's first frame continues from.
    pub cursor_before: ReplicationCursor,
    /// Chain position the set's terminal frame committed.
    pub cursor_after: ReplicationCursor,
}

/// Verify one export set file under `pinned_source` without trusting its
/// sender or its manifest (#13747 slice 1): the manifest's format, signer,
/// world and header are checked, then every frame must decode, verify
/// under the pinned key, sit in the manifest's session, chain digests and
/// cursors to the previous frame, and close with exactly one terminal frame
/// committing `entries`. A snapshot set must open from the chain base; an
/// incremental set must open at `base_ordinal`.
///
/// # Errors
///
/// Fails on any refusal above; a failed set verifies nothing.
pub async fn verify_export_set<R: AsyncRead + Unpin>(
    input: &mut R,
    pinned_source: [u8; 32],
) -> Result<VerifiedExportSet> {
    let first = read_frame_from(input)
        .await?
        .context("the export is empty")?;
    let manifest: ExportManifest =
        serde_json::from_slice(&first).context("reading the export manifest")?;
    if manifest.format != EXPORT_FORMAT {
        bail!(
            "export format {:?} is not {EXPORT_FORMAT:?}",
            manifest.format
        );
    }
    if decode_hex(&manifest.source).as_deref() != Some(&pinned_source[..]) {
        bail!(
            "the export says it was signed by {}, not the pinned source {}",
            manifest.source,
            encode_hex(&pinned_source)
        );
    }
    let nonce: [u8; 16] = decode_hex(&manifest.nonce)
        .and_then(|bytes| bytes.try_into().ok())
        .context("the export manifest's nonce is not 16 bytes of hex")?;
    let mut running: Option<ReplicationCursor> = None;
    let mut first_before = None;
    let mut prev_digest = [0; 32];
    let mut committed = None;
    while let Some(frame) = read_frame_from(input).await? {
        if committed.is_some() {
            bail!("the export has frames after its terminal frame");
        }
        let signed = SignedReplicationEnvelope::decode_frame(&frame)
            .context("malformed replication frame")?;
        signed
            .verify_signature()
            .map_err(|error| anyhow::anyhow!("export frame refused: {error}"))?;
        let envelope = &signed.envelope;
        if envelope.source != pinned_source {
            bail!("export frame is not signed by the pinned source");
        }
        if envelope.session_nonce != nonce {
            bail!("export frame is bound to another session");
        }
        if envelope.version != REPLICATION_ENVELOPE_VERSION {
            bail!("export frame names an unsupported envelope version");
        }
        if envelope.world_id != manifest.world {
            bail!("export frame names another world");
        }
        if envelope.header_version != manifest.header_version {
            bail!("export frame names another journal header version");
        }
        if envelope.prev_frame_digest != prev_digest {
            bail!("export frame breaks the frame chain");
        }
        if envelope.payload_digest != payload_digest(&envelope.payload) {
            bail!("export frame's payload digest does not match its payload");
        }
        match running {
            None => match envelope.kind {
                ReplicationKind::Snapshot => {
                    if manifest.set_kind != ExportSetKind::Snapshot || manifest.base_ordinal != 0 {
                        bail!("a snapshot frame cannot open an incremental set");
                    }
                    if envelope.cursor_before
                        != chain_base_cursor(
                            &manifest.world,
                            &pinned_source,
                            manifest.header_version,
                        )
                    {
                        bail!("a snapshot frame must open from the chain base");
                    }
                }
                ReplicationKind::Tail => {
                    if manifest.set_kind != ExportSetKind::Incremental {
                        bail!("a tail frame cannot open a snapshot set");
                    }
                    if envelope.cursor_before.ordinal != manifest.base_ordinal {
                        bail!(
                            "the incremental set opens at {}, its manifest says {}",
                            envelope.cursor_before.ordinal,
                            manifest.base_ordinal
                        );
                    }
                }
                // An empty incremental set carries no journal frames: its
                // sidecars open the batch at the base cursor.
                ReplicationKind::Directory | ReplicationKind::Module => {
                    if manifest.set_kind != ExportSetKind::Incremental {
                        bail!("sidecars cannot open a snapshot set");
                    }
                    if envelope.cursor_before != envelope.cursor_after
                        || envelope.cursor_before.ordinal != manifest.base_ordinal
                    {
                        bail!("the incremental set does not open at its base ordinal");
                    }
                }
                _ => bail!("the export's first signed frame must open the chain"),
            },
            Some(cursor) => {
                if envelope.cursor_before != cursor {
                    bail!("export frame breaks the chain");
                }
            }
        }
        if envelope.kind == ReplicationKind::Terminal {
            if !envelope.payload.is_empty() {
                bail!("the terminal frame must carry no payload");
            }
            committed = Some(envelope.cursor_after);
        }
        if first_before.is_none() {
            first_before = Some(envelope.cursor_before);
        }
        running = Some(envelope.cursor_after);
        prev_digest = frame_digest(&frame);
    }
    let after = committed.context("the export ends before its terminal frame")?;
    if after.ordinal != manifest.entries {
        bail!(
            "the export restored {} journal entries, its manifest says {}",
            after.ordinal,
            manifest.entries
        );
    }
    let before = first_before.context("the export ends before its terminal frame")?;
    Ok(VerifiedExportSet {
        manifest,
        cursor_before: before,
        cursor_after: after,
    })
}

/// Prove ordered verified sets link into one continuous custody chain
/// (#13747 slice 1): the first set is a snapshot from the chain base, every
/// set names the same world, source and journal header, and each set opens
/// exactly where the previous committed — ordinal and digest, so no entry
/// can be dropped, reordered or spliced between files without breaking the
/// link.
///
/// # Errors
///
/// Fails on the first broken link; an empty chain proves nothing.
pub fn check_set_chain(sets: &[VerifiedExportSet]) -> Result<()> {
    let Some(first) = sets.first() else {
        bail!("no export sets to chain");
    };
    if first.manifest.set_kind != ExportSetKind::Snapshot || first.manifest.base_ordinal != 0 {
        bail!("a custody chain must start with a snapshot set");
    }
    for pair in sets.windows(2) {
        let (previous, next) = (&pair[0], &pair[1]);
        if next.manifest.world != previous.manifest.world
            || next.manifest.source != previous.manifest.source
            || next.manifest.header_version != previous.manifest.header_version
        {
            bail!("chained sets must name one world, source and journal header");
        }
        if next.manifest.base_ordinal != previous.manifest.entries {
            bail!(
                "the next set starts at {}, the previous covers through {}",
                next.manifest.base_ordinal,
                previous.manifest.entries
            );
        }
        if next.cursor_before != previous.cursor_after {
            bail!("chained sets do not link: the chain digest breaks between files");
        }
    }
    Ok(())
}

/// One custody set file in a backup directory: what an operator lists when
/// deciding what may go. `entries` is the set's covered-through count —
/// the manifest's `entries` — so a set covers `[base_ordinal, entries)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustodySet {
    /// File name, for the operator's delete call.
    pub name: String,
    /// Snapshot or incremental, from the set's manifest.
    pub set_kind: ExportSetKind,
    /// Entry ordinal the set starts from, from the set's manifest.
    pub base_ordinal: u64,
    /// Entry ordinal the set covers through, from the set's manifest.
    pub entries: u64,
}

/// Decide which whole set files a retention pass may delete (#13747 slice 1).
///
/// Snapshots are full images from the chain base, so a snapshot covers a
/// set exactly when the set's covered-through count falls within the
/// snapshot's. A set file may go only when a retained newer snapshot
/// covers its whole entry range — and then the whole file goes: the return
/// is file names only, so a kept set's journal is never edited. The live
/// chain (the newest snapshot and every set after it) is never a
/// candidate, and the previous `keep_snapshots - 1` snapshot-led chains
/// are retained as rollback points. With no snapshot among `sets`,
/// coverage cannot be proved and nothing is deletable. Malformed entries
/// (`entries` below their own `base_ordinal`) are never auto-deleted
/// either.
#[must_use]
pub fn prune_export_sets(sets: &[CustodySet], keep_snapshots: usize) -> Vec<String> {
    let mut snapshots: Vec<&CustodySet> = sets
        .iter()
        .filter(|set| {
            set.set_kind == ExportSetKind::Snapshot
                && set.base_ordinal == 0
                && set.entries >= set.base_ordinal
        })
        .collect();
    if snapshots.is_empty() {
        return Vec::new();
    }
    snapshots.sort_by(|a, b| a.entries.cmp(&b.entries));
    let cutoff = snapshots.len().saturating_sub(keep_snapshots.max(1));
    let Some(oldest_kept) = snapshots.get(cutoff) else {
        return Vec::new();
    };
    let threshold = oldest_kept.entries;
    let mut deletable: Vec<String> = sets
        .iter()
        .filter(|set| set.entries >= set.base_ordinal)
        .filter(|set| set.entries <= threshold)
        .filter(|set| {
            !(set.set_kind == ExportSetKind::Snapshot
                && set.base_ordinal == 0
                && set.entries >= threshold)
        })
        .map(|set| set.name.clone())
        .collect();
    deletable.sort();
    deletable
}

/// Restore an export read from `input` into the new world directory
/// `dest`, verifying every frame under `pinned_source`.
///
/// # Errors
///
/// Refuses an existing `dest`, a manifest of another format, a manifest
/// whose signer is not `pinned_source`, any frame that does not verify,
/// an export that ends before its terminal frame, frames after it, and a
/// restored journal whose length is not the manifest's. Nothing is left
/// at `dest` or beside it when it fails.
pub async fn import_world<R: AsyncRead + Unpin>(
    input: &mut R,
    dest: &Path,
    pinned_source: [u8; 32],
) -> Result<ExportManifest> {
    if tokio::fs::symlink_metadata(dest).await.is_ok() {
        bail!(
            "{} already exists: restore into a new world directory",
            dest.display()
        );
    }
    let manifest = read_manifest(input).await?;
    if decode_hex(&manifest.source).as_deref() != Some(&pinned_source[..]) {
        bail!(
            "the export says it was signed by {}, not the pinned source {}",
            manifest.source,
            encode_hex(&pinned_source)
        );
    }
    let nonce: [u8; 16] = decode_hex(&manifest.nonce)
        .and_then(|bytes| bytes.try_into().ok())
        .context("the export manifest's nonce is not 16 bytes of hex")?;
    let staging = sibling(dest, "importing");
    if tokio::fs::symlink_metadata(&staging).await.is_ok() {
        bail!(
            "{} exists: an earlier restore was interrupted; remove it first",
            staging.display()
        );
    }
    let result = receive(input, &staging, &manifest, pinned_source, nonce).await;
    match result {
        Ok(()) => {
            tokio::fs::rename(&staging, dest)
                .await
                .with_context(|| format!("renaming {} to {}", staging.display(), dest.display()))?;
            Ok(manifest)
        }
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            Err(error)
        }
    }
}

/// Read the manifest at the head of the export file `path`, verifying
/// nothing: what the export claims to be, so a caller can choose where to
/// restore it. [`import_world`] checks every claim against the signed
/// frames.
///
/// # Errors
///
/// Fails when `path` cannot be read or its first frame is not a manifest
/// of [`EXPORT_FORMAT`].
pub async fn read_export_manifest(path: &Path) -> Result<ExportManifest> {
    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let mut reader = tokio::io::BufReader::new(file);
    read_manifest(&mut reader).await
}

async fn read_manifest<R: AsyncRead + Unpin>(input: &mut R) -> Result<ExportManifest> {
    let first = read_frame_from(input)
        .await?
        .context("the export is empty")?;
    let manifest: ExportManifest =
        serde_json::from_slice(&first).context("reading the export manifest")?;
    if manifest.format != EXPORT_FORMAT {
        bail!(
            "export format {:?} is not {EXPORT_FORMAT:?}",
            manifest.format
        );
    }
    Ok(manifest)
}

/// Restore the export file at `path` into `dest`, as [`import_world`].
///
/// # Errors
///
/// As [`import_world`], and when `path` cannot be opened.
pub async fn import_world_from_file(
    path: &Path,
    dest: &Path,
    pinned_source: [u8; 32],
) -> Result<ExportManifest> {
    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let mut reader = tokio::io::BufReader::new(file);
    import_world(&mut reader, dest, pinned_source).await
}

async fn receive<R: AsyncRead + Unpin>(
    input: &mut R,
    staging: &Path,
    manifest: &ExportManifest,
    pinned_source: [u8; 32],
    nonce: [u8; 16],
) -> Result<()> {
    let mut receiver = DurableReceiver::open(staging, &manifest.world, pinned_source)
        .map_err(|error| anyhow::anyhow!("opening {}: {error}", staging.display()))?;
    receiver.begin_session(nonce);
    let mut committed = None;
    while let Some(frame) = read_frame_from(input).await? {
        if committed.is_some() {
            bail!("the export has frames after its terminal frame");
        }
        match receiver
            .accept_frame(&frame)
            .map_err(|error| anyhow::anyhow!("export frame refused: {error}"))?
        {
            ReceiverProgress::Committed { cursor } => committed = Some(cursor),
            ReceiverProgress::Staged { .. } => {}
        }
    }
    let cursor = committed.context("the export ends before its terminal frame")?;
    if cursor.ordinal != manifest.entries {
        bail!(
            "the export restored {} journal entries, its manifest says {}",
            cursor.ordinal,
            manifest.entries
        );
    }
    Ok(())
}

/// `<parent>/.<name>.<suffix>`: a scratch path beside `path`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{suffix}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world::{WorldLimits, WorldScopes};
    use crate::world_limits::RuntimeLimits;
    use crate::world_store::open_world;
    use tempfile::TempDir;

    async fn busy_world(dir: &Path) -> WorldService {
        let (service, _) = open_world(
            dir,
            "w",
            WorldLimits::default(),
            RuntimeLimits::default(),
            |world| WorldService::new(world, None),
        )
        .await
        .unwrap();
        let (_, ann) = service
            .world()
            .issue("ann", WorldScopes::GUEST)
            .await
            .unwrap();
        service.world().chat(ann.clone(), "hello").await.unwrap();
        service.world().chat(ann, "again").await.unwrap();
        service
    }

    // #16179 oracle: the durable ack requires the parent-directory sync
    // after the rename — the injected sync observes the renamed file
    // already in place, and is called exactly once with the export path.
    #[tokio::test]
    async fn an_export_syncs_the_parent_directory_after_the_rename() {
        let root = TempDir::new().unwrap();
        let world_dir = root.path().join("w");
        let service = busy_world(&world_dir).await;
        let key = SigningKey::from_bytes(&[9; 32]);
        let file = root.path().join("w.export");
        let synced = std::sync::Mutex::new(Vec::new());
        let recording_sync = |path: &Path| -> Result<()> {
            assert!(
                path.exists(),
                "directory sync ran before the rename landed"
            );
            synced.lock().unwrap().push(path.to_path_buf());
            Ok(())
        };
        let manifest =
            export_world_to_file_with_dir_sync(&service, &key, &file, &recording_sync)
                .await
                .unwrap();
        service.world().shutdown().await.unwrap();
        assert_eq!(manifest.world, "w");
        assert_eq!(*synced.lock().unwrap(), vec![file.clone()]);
        assert!(file.exists());
    }

    // #16179 oracle: a parent-directory sync failure is propagated, not
    // a successful export — the caller must not treat the export as
    // crash-durable when its rename was never made durable.
    #[tokio::test]
    async fn an_export_propagates_a_parent_directory_sync_failure() {
        let root = TempDir::new().unwrap();
        let world_dir = root.path().join("w");
        let service = busy_world(&world_dir).await;
        let key = SigningKey::from_bytes(&[9; 32]);
        let file = root.path().join("w.export");
        let failing_sync = |_: &Path| -> Result<()> {
            bail!("injected directory sync failure")
        };
        let error = export_world_to_file_with_dir_sync(&service, &key, &file, &failing_sync)
            .await
            .unwrap_err();
        service.world().shutdown().await.unwrap();
        assert!(
            format!("{error:#}").contains("injected directory sync failure"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn an_export_restores_byte_for_byte_and_reopens() {
        let root = TempDir::new().unwrap();
        let world_dir = root.path().join("w");
        let service = busy_world(&world_dir).await;
        let key = SigningKey::from_bytes(&[9; 32]);
        let file = root.path().join("w.export");
        let manifest = export_world_to_file(&service, &key, &file).await.unwrap();
        service.world().shutdown().await.unwrap();
        assert_eq!(manifest.format, EXPORT_FORMAT);
        assert_eq!(manifest.world, "w");
        assert!(manifest.entries >= 3, "{manifest:?}");
        assert!(!sibling(&file, "partial").exists());
        // Exporting over an existing file is refused.
        assert!(export_world_to_file(&service, &key, &file).await.is_err());

        let other = TempDir::new().unwrap();
        let dest = other.path().join("w");
        let source = *key.verifying_key().as_bytes();
        let restored = import_world_from_file(&file, &dest, source).await.unwrap();
        assert_eq!(restored, manifest);
        assert_eq!(
            std::fs::read(dest.join("journal.jsonl")).unwrap(),
            std::fs::read(world_dir.join("journal.jsonl")).unwrap()
        );
        assert!(!sibling(&dest, "importing").exists());

        // The restored world reopens to the same place as the original.
        let mut reports = Vec::new();
        for dir in [&world_dir, &dest] {
            let (reopened, report) = open_world(
                dir,
                "w",
                WorldLimits::default(),
                RuntimeLimits::default(),
                |world| WorldService::new(world, None),
            )
            .await
            .unwrap();
            reopened.world().shutdown().await.unwrap();
            reports.push((report.sequence, report.journal_entries));
        }
        assert_eq!(reports[0], reports[1]);
        assert!(reports[0].0 >= 2, "{reports:?}");

        // A second restore to the same place is refused.
        assert!(import_world_from_file(&file, &dest, source).await.is_err());
    }

    #[tokio::test]
    async fn journal_digest_binds_the_exact_journal_not_the_envelope() {
        // Oracle: the digest names journal content, so re-exporting an
        // unchanged journal (fresh nonce, new envelope) keeps it, and
        // any state change changes it — even though a later compaction
        // could bring the entry count back to the same value.
        let root = TempDir::new().unwrap();
        let service = busy_world(&root.path().join("w")).await;
        let key = SigningKey::from_bytes(&[9; 32]);
        let mut first = Vec::new();
        let one = export_world(&service, &key, &mut first).await.unwrap();
        let mut second = Vec::new();
        let two = export_world(&service, &key, &mut second).await.unwrap();
        assert_ne!(one.nonce, two.nonce);
        assert_eq!(one.journal_digest, two.journal_digest);
        assert_eq!(one.entries, two.entries);
        assert!(!one.journal_digest.is_empty());

        let (_, ann) = service
            .world()
            .issue("ann2", WorldScopes::GUEST)
            .await
            .unwrap();
        service.world().chat(ann, "changed").await.unwrap();
        let mut third = Vec::new();
        let three = export_world(&service, &key, &mut third).await.unwrap();
        assert_ne!(one.journal_digest, three.journal_digest);
        service.world().shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_wrong_key_a_flipped_byte_or_a_cut_file_restores_nothing() {
        let root = TempDir::new().unwrap();
        let service = busy_world(&root.path().join("w")).await;
        let key = SigningKey::from_bytes(&[9; 32]);
        let mut bytes = Vec::new();
        export_world(&service, &key, &mut bytes).await.unwrap();
        service.world().shutdown().await.unwrap();
        let source = *key.verifying_key().as_bytes();
        let dest_root = TempDir::new().unwrap();
        let dest = dest_root.path().join("w");

        let stranger = *SigningKey::from_bytes(&[3; 32]).verifying_key().as_bytes();
        let refused = import_world(&mut bytes.as_slice(), &dest, stranger)
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("pinned source"), "{refused}");

        let mut flipped = bytes.clone();
        let at = flipped.len() - 40;
        flipped[at] ^= 0x01;
        assert!(
            import_world(&mut flipped.as_slice(), &dest, source)
                .await
                .is_err()
        );

        let cut = &bytes[..bytes.len() - 10];
        assert!(import_world(&mut &cut[..], &dest, source).await.is_err());

        let mut doubled = bytes.clone();
        doubled.extend_from_slice(&bytes);
        assert!(
            import_world(&mut doubled.as_slice(), &dest, source)
                .await
                .is_err()
        );

        let names: Vec<_> = std::fs::read_dir(dest_root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(names.is_empty(), "{names:?}");

        import_world(&mut bytes.as_slice(), &dest, source)
            .await
            .unwrap();
    }

    async fn snapshot_plus_incremental() -> (Vec<u8>, Vec<u8>, [u8; 32]) {
        let root = TempDir::new().unwrap();
        let service = busy_world(&root.path().join("w")).await;
        let key = SigningKey::from_bytes(&[9; 32]);
        let source = *key.verifying_key().as_bytes();
        let mut full = Vec::new();
        let base = export_world(&service, &key, &mut full).await.unwrap();
        assert_eq!(base.set_kind, ExportSetKind::Snapshot);
        assert_eq!(base.base_ordinal, 0);
        let (_, ann) = service
            .world()
            .issue("ann", WorldScopes::GUEST)
            .await
            .unwrap();
        service
            .world()
            .chat(ann.clone(), "after the snapshot")
            .await
            .unwrap();
        service.world().chat(ann, "one more").await.unwrap();
        let mut inc = Vec::new();
        let incremental = export_incremental_set(&service, &key, base.entries, &mut inc)
            .await
            .unwrap();
        assert_eq!(incremental.set_kind, ExportSetKind::Incremental);
        assert_eq!(incremental.base_ordinal, base.entries);
        assert!(
            incremental.entries > base.entries,
            "{incremental:?} covers nothing new"
        );
        // Past the end is refused; exactly at the end is a valid empty set.
        assert!(
            export_incremental_set(&service, &key, incremental.entries + 1, &mut Vec::new())
                .await
                .is_err()
        );
        let mut empty = Vec::new();
        let at_end = export_incremental_set(&service, &key, incremental.entries, &mut empty)
            .await
            .unwrap();
        assert_eq!(at_end.base_ordinal, at_end.entries);
        let verified_empty = verify_export_set(&mut empty.as_slice(), source)
            .await
            .unwrap();
        assert_eq!(verified_empty.cursor_before, verified_empty.cursor_after);
        service.world().shutdown().await.unwrap();
        (full, inc, source)
    }

    #[tokio::test]
    async fn incremental_set_chains_to_its_snapshot() {
        let (full, inc, source) = snapshot_plus_incremental().await;
        let base = verify_export_set(&mut full.as_slice(), source)
            .await
            .unwrap();
        assert_eq!(base.manifest.set_kind, ExportSetKind::Snapshot);
        let incremental = verify_export_set(&mut inc.as_slice(), source)
            .await
            .unwrap();
        assert_eq!(incremental.manifest.set_kind, ExportSetKind::Incremental);
        assert_eq!(incremental.cursor_before, base.cursor_after);
        check_set_chain(&[base, incremental]).unwrap();
        // The full snapshot still restores on its own: the new fields
        // change nothing for import.
        let other = TempDir::new().unwrap();
        let dest = other.path().join("w");
        import_world(&mut full.as_slice(), &dest, source)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tampered_incremental_set_verifies_nothing() {
        let (full, inc, source) = snapshot_plus_incremental().await;
        let mut flipped = inc.clone();
        let at = flipped.len() - 40;
        flipped[at] ^= 0x01;
        assert!(
            verify_export_set(&mut flipped.as_slice(), source)
                .await
                .is_err()
        );
        let cut = &inc[..inc.len() - 10];
        assert!(verify_export_set(&mut &cut[..], source).await.is_err());
        let stranger = *SigningKey::from_bytes(&[3; 32]).verifying_key().as_bytes();
        assert!(
            verify_export_set(&mut inc.as_slice(), stranger)
                .await
                .is_err()
        );
        // An incremental set cannot start a chain, and an empty chain
        // proves nothing.
        let incremental = verify_export_set(&mut inc.as_slice(), source)
            .await
            .unwrap();
        assert!(check_set_chain(std::slice::from_ref(&incremental)).is_err());
        assert!(check_set_chain(&[]).is_err());
        // A forged base ordinal breaks the link even when every frame
        // verifies.
        let base = verify_export_set(&mut full.as_slice(), source)
            .await
            .unwrap();
        let mut doctored = incremental;
        doctored.manifest.base_ordinal += 1;
        assert!(check_set_chain(&[base, doctored]).is_err());
    }

    #[test]
    fn set_chain_checks_digests_not_just_ordinals() {
        let manifest = |set_kind, base_ordinal, entries| ExportManifest {
            format: EXPORT_FORMAT.to_owned(),
            world: "w".to_owned(),
            source: "9".repeat(64),
            header_version: 1,
            entries,
            journal_digest: "0".repeat(64),
            nonce: "0".repeat(32),
            set_kind,
            base_ordinal,
        };
        let previous = VerifiedExportSet {
            manifest: manifest(ExportSetKind::Snapshot, 0, 5),
            cursor_before: crate::world::chain_base_cursor("w", &[9; 32], 1),
            cursor_after: crate::world::ReplicationCursor {
                ordinal: 5,
                digest: [1; 32],
            },
        };
        let linked = VerifiedExportSet {
            manifest: manifest(ExportSetKind::Incremental, 5, 7),
            cursor_before: crate::world::ReplicationCursor {
                ordinal: 5,
                digest: [1; 32],
            },
            cursor_after: crate::world::ReplicationCursor {
                ordinal: 7,
                digest: [3; 32],
            },
        };
        check_set_chain(std::slice::from_ref(&previous)).unwrap();
        check_set_chain(&[previous.clone(), linked.clone()]).unwrap();
        // Same ordinals, different digest: a spliced middle the ordinal
        // check alone would miss.
        let spliced = VerifiedExportSet {
            cursor_before: crate::world::ReplicationCursor {
                ordinal: 5,
                digest: [2; 32],
            },
            ..linked
        };
        assert!(check_set_chain(&[previous, spliced]).is_err());
    }

    #[test]
    fn prune_removes_only_whole_sets_covered_by_a_newer_snapshot() {
        let set =
            |name: &str, set_kind: ExportSetKind, base_ordinal: u64, entries: u64| CustodySet {
                name: name.to_owned(),
                set_kind,
                base_ordinal,
                entries,
            };
        let sets = vec![
            set("set-001.export", ExportSetKind::Snapshot, 0, 10),
            set("set-002.export", ExportSetKind::Incremental, 10, 14),
            set("set-003.export", ExportSetKind::Snapshot, 0, 20),
            set("set-004.export", ExportSetKind::Incremental, 20, 22),
        ];
        // keep=1: the older chain is fully covered by the newer snapshot.
        assert_eq!(
            prune_export_sets(&sets, 1),
            vec!["set-001.export".to_owned(), "set-002.export".to_owned()]
        );
        // keep=2: the previous snapshot-led chain stays as a rollback point.
        assert!(prune_export_sets(&sets, 2).is_empty());
        // keep=0 still keeps one snapshot: pruning never leaves no base.
        assert_eq!(prune_export_sets(&sets, 0).len(), 2);
        // The live chain is never a candidate, whatever the keep count.
        for keep in [1, 2, 10] {
            let deletable = prune_export_sets(&sets, keep);
            assert!(!deletable.contains(&"set-003.export".to_owned()));
            assert!(!deletable.contains(&"set-004.export".to_owned()));
        }
        // Without a snapshot coverage cannot be proved: delete nothing.
        let no_snapshot = vec![set("set-002.export", ExportSetKind::Incremental, 10, 14)];
        assert!(prune_export_sets(&no_snapshot, 1).is_empty());
        assert!(prune_export_sets(&[], 1).is_empty());
        // Malformed entries are never auto-deleted.
        let mut with_bad = sets.clone();
        with_bad.push(set("set-000.export", ExportSetKind::Incremental, 99, 5));
        assert!(!prune_export_sets(&with_bad, 1).contains(&"set-000.export".to_owned()));
    }
}
