//! The `fetch-model` verb (step 5, code#5355; design section 4,
//! Q1 as ruled): provision the pinned Snowflake model snapshot
//! into the state dir, outside the nix store.
//!
//! The model is data, not code. `~/.local/state/atlas-search/
//! model/<rev>/` holds exactly the two files the embedder loads
//! (`tokenizer.json`, `onnx/model.onnx`), each verified against
//! the sha256 constants recorded in [`MODEL_FILES`] — the same
//! hashes the Gate A fixtures record (`fixtures/embed-probes.json`
//! provenance). Downloads land in a sibling temp dir and the
//! completed set renames into place in one move; a snapshot that
//! already verifies is a no-op, and a hash mismatch at any point
//! refuses the install loudly (this verb + the loud embedder-init
//! failure are the design's mitigation for the state-dir choice).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The pinned HuggingFace repository (design section 4).
pub const MODEL_REPO: &str = "Snowflake/snowflake-arctic-embed-m-v2.0";
/// The pinned HuggingFace revision (fixtures/README.md; Gate A).
pub const MODEL_REV: &str = "95c2741480856aa9666782eb4afe11959938017f";
/// The pinned file set: (path within the snapshot, sha256). Only
/// the files atlas-embed loads are provisioned.
pub const MODEL_FILES: [(&str, &str); 2] = [
    (
        "tokenizer.json",
        "f1cc44ad7faaeec47241864835473fd5403f2da94673f3f764a77ebcb0a803ec",
    ),
    (
        "onnx/model.onnx",
        "c0c53d7f49a2db60761b92b7bbf5be87a7b3cf5d92dbbd7f1b5028bd5a40aa39",
    ),
];

/// `~/.local/state/atlas-search` — the same state-root convention
/// `init`/`serve` use for `service.json`.
fn home_dir() -> PathBuf {
    // Home-resolution contract (issues #5640 / #5762): a missing or
    // empty HOME is an error naming HOME, never a substitute
    // directory - the old fallback to a literal "~" silently
    // resolved state/config/token paths relative to the CWD.
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => panic!("HOME is not set or is empty: atlas-search requires a non-empty HOME to resolve its state/config paths (issue #5640)"),
    }
}

pub fn state_root() -> PathBuf {
    home_dir().join(".local/state/atlas-search")
}

/// The provisioned snapshot dir for the pinned revision.
pub fn model_dir() -> PathBuf {
    state_root().join("model").join(MODEL_REV)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// Every file in `files` present in `dir` with a matching hash.
fn verify_dir_with(dir: &Path, files: &[(&str, &str)]) -> Result<bool, String> {
    for (rel, want) in files {
        let p = dir.join(rel);
        if !p.is_file() {
            return Ok(false);
        }
        if sha256_file(&p)? != *want {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The pinned snapshot in `dir` verifies, file for file.
fn verify_dir(dir: &Path) -> Result<bool, String> {
    verify_dir_with(dir, &MODEL_FILES)
}

fn download(url: &str, dest: &Path, want: &str, rel: &str) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let mut resp = ureq::get(url)
        .call()
        .map_err(|e| format!("download of {rel} failed ({url}): {e}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!(
            "download of {rel} failed: HTTP {status} from {url}"
        ));
    }
    let mut out =
        fs::File::create(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut reader = resp.body_mut().as_reader();
    let mut h = Sha256::new();
    let mut buf = [0u8; 1 << 20];
    let mut total: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download of {rel} failed mid-body: {e}"))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n])
            .map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
        total += n as u64;
    }
    out.flush()
        .map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    let got = format!("{:x}", h.finalize());
    if got != want {
        return Err(format!(
            "sha256 mismatch for {rel}: downloaded {got}, pinned {want} — refusing to install"
        ));
    }
    println!("atlas-search: fetched {rel} ({total} bytes, sha256 verified)");
    Ok(())
}

/// Mutual exclusion for provisioning runs. Two `fetch-model`
/// processes sharing the staging dir interleave writes into the
/// same files (observed live during step-5 verification, where
/// orphaned runs raced): the mkdir is the atomic test-and-set,
/// and the guard removes it on exit. A killed run leaves the
/// lock behind; the error tells the operator how to clear it.
struct FetchLock(PathBuf);

impl Drop for FetchLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn acquire_fetch_lock(model_root: &Path) -> Result<FetchLock, String> {
    let lock = model_root.join(".fetch-model.lock");
    match fs::create_dir(&lock) {
        Ok(()) => Ok(FetchLock(lock)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
            "another fetch-model run is in progress (lock {} exists);              if no run is active, a previous run was killed              mid-provision — remove the lock dir and retry",
            lock.display()
        )),
        Err(e) => Err(format!(
            "cannot take the fetch-model lock {}: {e}",
            lock.display()
        )),
    }
}

/// Provision the pinned model into the state dir. Idempotent:
/// a snapshot that verifies is left untouched.
pub fn fetch_model() -> Result<(), String> {
    let target = model_dir();
    let model_root = state_root().join("model");
    fs::create_dir_all(&model_root)
        .map_err(|e| format!("cannot create {}: {e}", model_root.display()))?;
    let _lock = acquire_fetch_lock(&model_root)?;
    if verify_dir(&target)? {
        println!(
            "atlas-search: model {MODEL_REV} already provisioned at {} (sha256 verified); nothing to do",
            target.display()
        );
        return Ok(());
    }
    let tmp = model_root.join(format!(".fetch-{MODEL_REV}"));
    if tmp.exists() {
        fs::remove_dir_all(&tmp)
            .map_err(|e| format!("cannot clear stale {}: {e}", tmp.display()))?;
    }
    fs::create_dir_all(&tmp).map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
    let result = (|| {
        for (rel, want) in MODEL_FILES {
            let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REV}/{rel}");
            download(&url, &tmp.join(rel), want, rel)?;
        }
        Ok::<(), String>(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    // Atomic swap: the completed temp dir renames into place; a
    // pre-existing (failing) snapshot is moved aside first and
    // dropped once the new one is in.
    let aside = model_root.join(format!(".previous-{MODEL_REV}"));
    if aside.exists() {
        fs::remove_dir_all(&aside).map_err(|e| format!("cannot clear {}: {e}", aside.display()))?;
    }
    let had_target = target.exists();
    if had_target {
        fs::rename(&target, &aside)
            .map_err(|e| format!("cannot move {} aside: {e}", target.display()))?;
    }
    if let Err(e) = fs::rename(&tmp, &target) {
        if had_target {
            let _ = fs::rename(&aside, &target);
        }
        return Err(format!("cannot install {}: {e}", target.display()));
    }
    if had_target {
        let _ = fs::remove_dir_all(&aside);
    }
    println!(
        "atlas-search: model {MODEL_REV} provisioned at {}",
        target.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_file_matches_known_vector() {
        // FIPS 180-4 example: sha256("abc").
        let dir = std::env::temp_dir().join(format!("atlas-fetch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("abc");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_dir_with_toy_files() {
        let dir = std::env::temp_dir().join(format!("atlas-fetch-test2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let files: [(&str, &str); 1] = [(
            "a.txt",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )];
        // Missing file: does not verify.
        assert!(!verify_dir_with(&dir, &files).unwrap());
        // Wrong bytes: does not verify.
        std::fs::write(dir.join("a.txt"), b"abd").unwrap();
        assert!(!verify_dir_with(&dir, &files).unwrap());
        // Exact bytes: verifies.
        std::fs::write(dir.join("a.txt"), b"abc").unwrap();
        assert!(verify_dir_with(&dir, &files).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fetch_lock_excludes_second_holder() {
        let dir = std::env::temp_dir().join(format!("atlas-fetch-test3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("model");
        std::fs::create_dir_all(&root).unwrap();
        let first = acquire_fetch_lock(&root).unwrap();
        // A second holder is refused while the first lives.
        assert!(acquire_fetch_lock(&root).is_err());
        drop(first);
        // And the lock frees on drop.
        assert!(acquire_fetch_lock(&root).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_dir_ends_with_pinned_rev() {
        assert!(model_dir().ends_with(MODEL_REV));
    }
}
