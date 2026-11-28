//! The stress generator's output lifecycle (#13181): staging,
//! identity verification, recovery, and atomic publication for the
//! `stress_gen` bench profile's fixture directories.
//!
//! Shared by the bench (through a `#[path]` include) and the
//! integration tests (through `mod common;`). Unlike `loadgen`
//! this module is not std-only: it manipulates the generator's
//! on-disk JSON formats, so it uses the `serde_json` / `anyhow`
//! stack the bench and the test crates already link.
//!
//! The rules, in order:
//!
//! - Generation materializes into a staging sibling of the output
//!   root, marked with an ownership marker before anything else is
//!   written. A leftover staging directory is recovered (deleted)
//!   only when its marker verifies it as this generator's.
//! - A completed output root is replaced only after its manifest
//!   verifies as a stress manifest of the current schema. A
//!   directory whose contents cannot be verified — a foreign
//!   manifest, a missing manifest, an unmarked staging directory —
//!   is never deleted; the run refuses and names the cleanup
//!   action instead.
//! - Publication writes the manifest atomically inside the staging
//!   directory (temp file, sync, rename) and then renames the
//!   staging directory into place, so the output root only ever
//!   holds a complete generation or nothing at all.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

/// File name of the ownership marker a stress generation writes
/// into its staging directory before anything else. The marker is
/// what makes an interrupted generation recognizable as ours.
pub const STRESS_STAGING_MARKER: &str = ".stress-gen-staging.json";

/// The manifest `type` every completed stress generation carries;
/// the identity a previous output root must prove before a new
/// generation may replace it.
pub const STRESS_MANIFEST_TYPE: &str = "stress-manifest";

/// The staging directory a stress generation materializes into
/// before publishing: a sibling of the output root named after it,
/// so the publish rename stays on one filesystem.
pub fn stress_staging_path(out: &Path) -> Result<PathBuf> {
    let name = out
        .file_name()
        .context("stress output root has no final path component")?;
    let mut staging = name.to_os_string();
    staging.push(".partial");
    Ok(out.with_file_name(staging))
}

/// Whether `dir`'s manifest identifies it as a completed stress
/// generation of this generator: the manifest must parse and carry
/// the stress manifest type and schema version. Anything else —
/// missing, unparseable, foreign — is not ours to delete.
pub fn stress_dir_is_completed_generation(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    value.get("type").and_then(serde_json::Value::as_str) == Some(STRESS_MANIFEST_TYPE)
        && value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            == Some(1)
}

/// Whether `dir` is a staging directory left by an interrupted
/// stress generation: its ownership marker must parse and name
/// this generator. A staging path without a verifiable marker is
/// foreign content, not a recoverable leftover.
pub fn stress_dir_is_staging(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join(STRESS_STAGING_MARKER)) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    value.get("type").and_then(serde_json::Value::as_str) == Some("stress-gen-staging")
}

/// Prepare a stress generation: recover the staging directory an
/// interrupted run left behind (verified ours via its ownership
/// marker), refuse unverified content at either path with the
/// cleanup action named, and create a fresh staging directory
/// carrying its marker. The completed output root, if any, is left
/// untouched here — it is replaced only at publish time, after the
/// new generation succeeds, so a failed regeneration never strands
/// a previously good fixture.
pub fn prepare_stress_output(
    out: &Path,
    seed: u64,
    tier_name: &str,
    generator_version: u64,
) -> Result<PathBuf> {
    let staging = stress_staging_path(out)?;
    if staging.exists() {
        ensure!(
            stress_dir_is_staging(&staging),
            "stress staging directory {staging:?} exists but its ownership marker is missing \
             or foreign; refusing to delete unverified content — inspect it and remove it \
             manually if it is a leftover stress generation"
        );
        std::fs::remove_dir_all(&staging)
            .with_context(|| format!("remove interrupted stress staging directory {staging:?}"))?;
    }
    if out.exists() {
        ensure!(
            stress_dir_is_completed_generation(out),
            "stress output root {out:?} exists but does not hold a verified stress manifest \
             (type {STRESS_MANIFEST_TYPE:?}, schema_version 1); refusing to overwrite \
             unverified content — if it is a partial generation from an interrupted run, \
             remove the directory and re-run"
        );
    }
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("create stress staging directory {staging:?}"))?;
    let marker = serde_json::json!({
        "type": "stress-gen-staging",
        "generator_version": generator_version,
        "seed": seed,
        "tier": tier_name,
    });
    std::fs::write(
        staging.join(STRESS_STAGING_MARKER),
        format!("{}\n", serde_json::to_string(&marker)?),
    )
    .context("write stress staging ownership marker")?;
    Ok(staging)
}

/// Publish a completed staging directory as the output root: the
/// manifest is written atomically (temp file, sync, rename within
/// the staging directory, so no reader can observe a partial
/// manifest), the staging marker is dropped, the previous
/// completed generation — its identity re-verified here — is
/// removed, and the staging directory is renamed into place.
pub fn publish_stress_output(
    out: &Path,
    staging: &Path,
    manifest: &serde_json::Value,
) -> Result<PathBuf> {
    let manifest_tmp = staging.join("manifest.json.tmp");
    std::fs::write(
        &manifest_tmp,
        format!("{}\n", serde_json::to_string_pretty(manifest)?),
    )
    .context("write stress manifest temp file")?;
    std::fs::File::open(&manifest_tmp)
        .context("open stress manifest temp file")?
        .sync_all()
        .context("sync stress manifest temp file")?;
    std::fs::rename(&manifest_tmp, staging.join("manifest.json"))
        .context("publish stress manifest within staging directory")?;
    std::fs::remove_file(staging.join(STRESS_STAGING_MARKER))
        .context("remove stress staging ownership marker")?;
    if out.exists() {
        ensure!(
            stress_dir_is_completed_generation(out),
            "stress output root {out:?} no longer holds a verified stress manifest; \
             refusing to replace unverified content"
        );
        std::fs::remove_dir_all(out)
            .with_context(|| format!("remove previous stress generation {out:?}"))?;
    }
    std::fs::rename(staging, out)
        .with_context(|| format!("publish stress generation {staging:?} -> {out:?}"))?;
    Ok(out.join("manifest.json"))
}
