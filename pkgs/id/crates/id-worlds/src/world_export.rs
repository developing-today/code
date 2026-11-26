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

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

use crate::frames::{read_frame_from, write_frame_to};
use crate::world::{DurableReceiver, ReceiverProgress};
use crate::world_replication_net::{
    FrameSink, fresh_session_nonce, gather_sidecars, send_snapshot,
};
use crate::world_session::{WorldService, decode_hex, encode_hex};

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
    /// Journal entries in the export, the `Created` header included.
    pub entries: u64,
    /// Lowercase hex of the session nonce every frame is bound to.
    pub nonce: String,
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
        nonce: encode_hex(&nonce),
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
        Ok(manifest)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&partial).await;
    }
    result
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
}
