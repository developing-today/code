//! Atomic generation pointer for complete journal-plus-sidecar replica state.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use super::{
    ReplicationCursor,
    replica_control::{
        ReplicaRefreshPayload, ReplicaRefreshRecord, now_unix_secs,
    },
};

/// Root-relative pointer to the last complete generation.
pub const CURRENT_REPLICA_FILE: &str = "replica-current";
const GENERATIONS_DIR: &str = "replica-generations";

/// Resolve the last atomically published state, or the legacy flat directory
/// before the first Phase 5 generation is installed.
pub fn active_replica_dir(root: &Path) -> Result<PathBuf> {
    let pointer = root.join(CURRENT_REPLICA_FILE);
    if !pointer.exists() {
        return Ok(root.to_path_buf());
    }
    let name = fs::read_to_string(&pointer).with_context(|| format!("read {}", pointer.display()))?;
    let name = name.trim();
    ensure!(
        name.starts_with("state-")
            && name.len() <= 160
            && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "replica state pointer is malformed"
    );
    let path = root.join(GENERATIONS_DIR).join(name);
    ensure!(path.is_dir(), "published replica generation is missing");
    Ok(path)
}

/// Create a private working generation copied from the currently published
/// state. It is not visible to serve/open until [`publish_generation`].
pub fn create_working_generation(root: &Path, nonce: &[u8; 16]) -> Result<PathBuf> {
    fs::create_dir_all(root.join(GENERATIONS_DIR))
        .with_context(|| format!("create replica generations under {}", root.display()))?;
    let name = format!("staging-{}", hex(nonce));
    let working = root.join(GENERATIONS_DIR).join(name);
    fs::create_dir(&working).with_context(|| format!("create {}", working.display()))?;
    let active = active_replica_dir(root)?;
    if (active.as_path() != root && active.is_dir()) || active.join("journal.jsonl").is_file() {
        copy_directory(&active, &working)?;
    }
    Ok(working)
}

/// Install authenticated module/directory sidecars into an unpublished
/// working generation. The generation remains invisible until publication.
pub fn install_refresh(
    state_dir: &Path,
    source: [u8; 32],
    world: &str,
    cursor: ReplicationCursor,
    record: &ReplicaRefreshRecord,
) -> Result<ReplicaRefreshPayload> {
    let payload = record.verify_decode(source, world, cursor)?;
    payload
        .admission_lease
        .verify_at(source, world, cursor, now_unix_secs()?)?;

    match (&payload.module_hash, &payload.module_bytes) {
        (Some(expected), Some(bytes)) => {
            let stored = crate::world_store::ModuleDir::new(state_dir.join("modules")).save(bytes)?;
            ensure!(&stored == expected, "refreshed module hash does not match the journal");
        }
        (None, None) => {}
        _ => anyhow::bail!("refreshed module hash and bytes are inconsistent"),
    }

    let mut directory_bytes = Vec::new();
    for fact in &payload.directory_facts {
        directory_bytes.extend(serde_json::to_vec(fact).context("encode replica directory fact")?);
        directory_bytes.push(b'\n');
    }
    let directory_path = state_dir.join("directory.jsonl");
    let directory_tmp = state_dir.join("directory.jsonl.refreshing");
    write_synced(&directory_tmp, &directory_bytes)?;
    crate::directory::Directory::replay_read_only(&directory_tmp)
        .context("validate refreshed directory facts")?;
    fs::rename(&directory_tmp, &directory_path)
        .with_context(|| format!("install {}", directory_path.display()))?;
    sync_dir(state_dir)?;

    let manifest_bytes = postcard::to_allocvec(record).context("encode signed refresh manifest")?;
    atomic_write(&state_dir.join("replica-refresh.bin"), &manifest_bytes)?;
    Ok(payload)
}

/// Load the signed refresh record and ensure its sidecars match its payload.
pub fn verify_refresh(
    state_dir: &Path,
    source: [u8; 32],
    world: &str,
    cursor: ReplicationCursor,
    require_unexpired_lease: bool,
) -> Result<ReplicaRefreshPayload> {
    let bytes = fs::read(state_dir.join("replica-refresh.bin"))
        .context("read replica refresh manifest")?;
    ensure!(
        bytes.len() <= super::replica_control::MAX_REPLICA_REFRESH_BYTES + 1024 * 1024,
        "replica refresh manifest exceeds bound"
    );
    let record: ReplicaRefreshRecord =
        postcard::from_bytes(&bytes).context("decode replica refresh manifest")?;
    let payload = record.verify_decode(source, world, cursor)?;
    if require_unexpired_lease {
        payload.admission_lease.verify_at(source, world, cursor, now_unix_secs()?)?;
    } else {
        payload.admission_lease.verify_signature(source, world, cursor)?;
    }
    let expected_directory = encode_directory(&payload.directory_facts)?;
    let actual_directory = fs::read(state_dir.join("directory.jsonl"))
        .context("read replica directory sidecar")?;
    ensure!(
        actual_directory == expected_directory,
        "replica directory sidecar differs from its signed refresh"
    );
    match (&payload.module_hash, &payload.module_bytes) {
        (Some(hash), Some(bytes)) => {
            ensure!(
                crate::world_session::module_hash(bytes).as_str() == hash,
                "signed module bytes do not match active module hash"
            );
            let stored = crate::world_store::ModuleDir::new(state_dir.join("modules")).load(hash)?;
            ensure!(stored.as_slice() == bytes.as_slice(), "stored module differs from signed refresh");
        }
        (None, None) => {}
        _ => anyhow::bail!("signed module hash and bytes are inconsistent"),
    }
    Ok(payload)
}

/// Atomically publish a complete verified generation. The working directory
/// must already contain a committed journal and a verified refresh manifest.
pub fn publish_generation(
    root: &Path,
    working: &Path,
    cursor_ordinal: u64,
    nonce: &[u8; 16],
) -> Result<PathBuf> {
    let generations = root.join(GENERATIONS_DIR);
    ensure!(
        working.parent() == Some(generations.as_path()),
        "working replica generation is outside its state directory"
    );
    let name = format!("state-{cursor_ordinal}-{}", hex(nonce));
    let destination = generations.join(&name);
    fs::rename(working, &destination)
        .with_context(|| format!("publish generation {}", destination.display()))?;
    File::open(&generations)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("sync {}", generations.display()))?;

    let temporary = root.join(format!("{CURRENT_REPLICA_FILE}.{}.tmp", hex(nonce)));
    {
        let mut file = File::create(&temporary)
            .with_context(|| format!("create {}", temporary.display()))?;
        file.write_all(name.as_bytes())
            .with_context(|| format!("write {}", temporary.display()))?;
        file.write_all(b"\n")?;
        file.sync_all()
            .with_context(|| format!("sync {}", temporary.display()))?;
    }
    fs::rename(&temporary, root.join(CURRENT_REPLICA_FILE))
        .with_context(|| format!("install {}", root.join(CURRENT_REPLICA_FILE).display()))?;
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("sync replica root {}", root.display()))?;
    Ok(destination)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)
            .with_context(|| format!("stat {}", source_path.display()))?;
        if metadata.file_type().is_symlink() {
            anyhow::bail!("replica state contains a symlink: {}", source_path.display());
        }
        if metadata.is_dir() {
            fs::create_dir(&destination_path)
                .with_context(|| format!("create {}", destination_path.display()))?;
            copy_directory(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!("copy {} to {}", source_path.display(), destination_path.display())
            })?;
        } else {
            anyhow::bail!("replica state has a non-file entry: {}", source_path.display());
        }
    }
    Ok(())
}

fn encode_directory(facts: &[crate::directory::DirectoryEntry]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for fact in facts {
        bytes.extend(serde_json::to_vec(fact).context("encode replica directory fact")?);
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("sidecar path has no parent")?;
    let temporary = path.with_extension("tmp");
    write_synced(&temporary, bytes)?;
    fs::rename(&temporary, path).with_context(|| format!("install {}", path.display()))?;
    sync_dir(parent)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes).with_context(|| format!("write {}", path.display()))?;
    file.sync_all().with_context(|| format!("sync {}", path.display()))?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("sync directory {}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    #[test]
    fn unpublished_generation_is_invisible_until_pointer_swap() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path()).unwrap();
        fs::write(root.path().join("journal.jsonl"), b"old\n").unwrap();
        let working = create_working_generation(root.path(), &[1; 16]).unwrap();
        fs::write(working.join("journal.jsonl"), b"new\n").unwrap();

        assert_eq!(
            fs::read(active_replica_dir(root.path()).unwrap().join("journal.jsonl")).unwrap(),
            b"old\n"
        );

        let published = publish_generation(root.path(), &working, 3, &[2; 16]).unwrap();
        assert_eq!(active_replica_dir(root.path()).unwrap(), published);
        assert_eq!(fs::read(published.join("journal.jsonl")).unwrap(), b"new\n");
    }

    #[test]
    fn restart_keeps_last_complete_generation_after_staged_refresh_damage() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("journal.jsonl"), b"verified-before-outage\n").unwrap();
        fs::write(root.path().join("complete.marker"), b"complete").unwrap();
        let working = create_working_generation(root.path(), &[4; 16]).unwrap();
        fs::write(working.join("journal.jsonl"), b"new-cursor\n").unwrap();
        fs::write(working.join("replica-refresh.bin"), b"partial sidecar").unwrap();

        // A follower crash before publication leaves the old generation
        // selected; startup does not discover or serve the damaged staging dir.
        let reopened = active_replica_dir(root.path()).unwrap();
        assert_eq!(
            fs::read(reopened.join("journal.jsonl")).unwrap(),
            b"verified-before-outage\n"
        );
        assert_eq!(fs::read(reopened.join("complete.marker")).unwrap(), b"complete");
    }

    #[test]
    fn authenticated_sidecars_reject_tampering_before_state_publication() {
        let root = tempfile::tempdir().unwrap();
        let key = SigningKey::from_bytes(&[5; 32]);
        let source = key.verifying_key().to_bytes();
        let cursor = super::super::chain_base_cursor("lobby", &source, 1);
        let lease = super::super::SignedAdmissionLease::issue("lobby", source, cursor, now_unix_secs().unwrap(), &key).unwrap();
        let payload = ReplicaRefreshPayload {
            module_hash: None,
            module_bytes: None,
            directory_facts: Vec::new(),
            admission_lease: lease,
        };
        let payload_bytes = postcard::to_allocvec(&payload).unwrap();
        let (header, _) = super::super::sign_refresh_transfer(
            "lobby", source, [8; 16], cursor, &payload_bytes, &key,
        )
        .unwrap();
        let record = ReplicaRefreshRecord { header, payload_bytes };

        install_refresh(root.path(), source, "lobby", cursor, &record).unwrap();
        assert!(verify_refresh(root.path(), source, "lobby", cursor, true).is_ok());
        fs::write(root.path().join("directory.jsonl"), b"tampered\n").unwrap();
        assert!(verify_refresh(root.path(), source, "lobby", cursor, true).is_err());
    }
}
