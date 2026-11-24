//! Bundle directories: the on-disk form of root sets, issuer sets
//! and leaf deliveries (spec §7).
//!
//! A bundle is a directory with a canonical `manifest.json`
//! (§7.1) plus the artifacts it indexes. The manifest's file index
//! carries a SHA-256 per file; loading a bundle verifies every
//! indexed file, so a tampered or truncated bundle fails loudly
//! instead of issuing from corrupt state. Private key files are
//! written `0600` under a `0700` directory and are never included
//! in any public artifact.

use crate::error::{CertError, Result};
use crate::model::{BundleManifest, BundleType, FileRecord, MANIFEST_SCHEMA};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// SHA-256 of `bytes`, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Encode DER bytes as a PEM block with 64-column lines.
pub fn pem_encode(label: &str, der: &[u8]) -> String {
    let b64 = base64_encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// Decode every PEM block in `text` into (label, DER) pairs.
/// Non-PEM text between blocks is ignored.
pub fn pem_decode_all(text: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(begin) = rest.find("-----BEGIN ") {
        let after = &rest[begin + 11..];
        let label_end = after
            .find("-----")
            .ok_or_else(|| CertError::manifest("malformed PEM: unterminated BEGIN line"))?;
        let label = after[..label_end].trim().to_string();
        let body_start = begin + 11 + label_end + 5;
        let end_marker = format!("-----END {label}-----");
        let body_end = rest[body_start..]
            .find(&end_marker)
            .ok_or_else(|| CertError::manifest(format!("malformed PEM: no END for {label}")))?
            + body_start;
        let der = base64_decode(&rest[body_start..body_end])?;
        out.push((label, der));
        rest = &rest[body_end + end_marker.len()..];
    }
    Ok(out)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0] as u32,
            *chunk.get(1).unwrap_or(&0) as u32,
            *chunk.get(2).unwrap_or(&0) as u32,
        ];
        let n = (b[0] << 16) | (b[1] << 8) | b[2];
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let mut vals = Vec::new();
    for c in text.chars() {
        match c {
            'A'..='Z' => vals.push(c as u32 - 'A' as u32),
            'a'..='z' => vals.push(c as u32 - 'a' as u32 + 26),
            '0'..='9' => vals.push(c as u32 - '0' as u32 + 52),
            '+' => vals.push(62),
            '/' => vals.push(63),
            '=' => break,
            c if c.is_whitespace() => {}
            other => {
                return Err(CertError::manifest(format!(
                    "invalid base64 character '{other}'"
                )));
            }
        }
    }
    let mut out = Vec::with_capacity(vals.len() * 3 / 4);
    for chunk in vals.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let n = (chunk[0] << 18)
            | (chunk[1] << 12)
            | (*chunk.get(2).unwrap_or(&0) << 6)
            | *chunk.get(3).unwrap_or(&0);
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

/// One artifact to write into a bundle.
#[derive(Debug, Clone)]
pub struct Artifact {
    /// Bundle-relative path (`public/...` or `private/...`).
    pub rel_path: String,
    /// File-index kind: `certificate-pem`, `private-key-pkcs8`,
    /// `trust-bundle-pem`, `chain-pem`.
    pub kind: String,
    /// `public` | `private`.
    pub visibility: String,
    pub bytes: Vec<u8>,
}

/// Write a complete bundle: artifacts first (so their checksums can
/// be indexed), then the manifest. The target directory must not
/// already contain a manifest — overwriting a CA bundle is a
/// ceremony, not a default.
pub fn write_bundle(
    dir: &Path,
    mut manifest: BundleManifest,
    artifacts: &[Artifact],
) -> Result<BundleManifest> {
    if dir.join("manifest.json").exists() {
        return Err(CertError::policy(format!(
            "bundle already exists at {} (manifest.json present); refusing to overwrite a CA bundle",
            dir.display()
        )));
    }
    std::fs::create_dir_all(dir)?;
    let mut records = Vec::new();
    for art in artifacts {
        if art.rel_path.contains("..") {
            return Err(CertError::usage(format!(
                "artifact path '{}' escapes the bundle",
                art.rel_path
            )));
        }
        let path = dir.join(&art.rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            if art.visibility == "private" {
                set_dir_private(parent);
            }
        }
        std::fs::write(&path, &art.bytes)?;
        if art.visibility == "private" {
            set_file_private(&path);
        }
        records.push(FileRecord {
            path: art.rel_path.clone(),
            kind: art.kind.clone(),
            sha256: sha256_hex(&art.bytes),
            bytes: art.bytes.len() as u64,
            visibility: art.visibility.clone(),
        });
    }
    records.sort_by(|a, b| a.path.cmp(&b.path));
    manifest.files = records;
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(dir.join("manifest.json"), manifest_json + "\n")?;
    Ok(manifest)
}

#[cfg(unix)]
fn set_file_private(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_file_private(_path: &Path) {}

#[cfg(unix)]
fn set_dir_private(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_private(_path: &Path) {}

/// A loaded, checksum-verified bundle.
#[derive(Debug)]
pub struct Bundle {
    pub dir: PathBuf,
    pub manifest: BundleManifest,
}

/// Load a bundle and verify it: schema, file presence, and every
/// indexed file's SHA-256 (§7.1 read mode: strict verification).
pub fn load_bundle(dir: &Path) -> Result<Bundle> {
    let manifest_path = dir.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        CertError::manifest(format!(
            "no readable manifest.json in {}: {e}",
            dir.display()
        ))
    })?;
    let manifest: BundleManifest = serde_json::from_str(&text)?;
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(CertError::manifest(format!(
            "unsupported manifest schema '{}' (this build reads {MANIFEST_SCHEMA})",
            manifest.schema
        )));
    }
    let mut failures = Vec::new();
    for file in &manifest.files {
        let path = dir.join(&file.path);
        match std::fs::read(&path) {
            Ok(bytes) => {
                if sha256_hex(&bytes) != file.sha256 {
                    failures.push(format!("{}: checksum mismatch", file.path));
                } else if bytes.len() as u64 != file.bytes {
                    failures.push(format!("{}: size mismatch", file.path));
                }
            }
            Err(e) => failures.push(format!("{}: unreadable ({e})", file.path)),
        }
    }
    if !failures.is_empty() {
        return Err(CertError::manifest(format!(
            "bundle at {} fails verification: {}",
            dir.display(),
            failures.join("; ")
        )));
    }
    Ok(Bundle {
        dir: dir.to_path_buf(),
        manifest,
    })
}

/// Read a bundle's manifest without checksum verification (for
/// listing damaged bundles honestly). Returns `None` when the
/// directory is not a bundle.
pub fn read_manifest_unverified(dir: &Path) -> Option<BundleManifest> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// List bundles under a parent directory: the parent itself when it
/// is a bundle, plus each immediate subdirectory that is one.
/// Optionally filtered by bundle type.
pub fn list_bundles(parent: &Path, want: Option<BundleType>) -> Vec<(PathBuf, BundleManifest)> {
    let mut out = Vec::new();
    let consider = |dir: &Path, out: &mut Vec<(PathBuf, BundleManifest)>| {
        if let Some(m) = read_manifest_unverified(dir) {
            if want.is_none() || want == Some(m.bundle_type) {
                out.push((dir.to_path_buf(), m));
            }
        }
    };
    consider(parent, &mut out);
    if let Ok(entries) = std::fs::read_dir(parent) {
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for d in dirs {
            consider(&d, &mut out);
        }
    }
    out
}

/// Read one indexed artifact's bytes from a loaded bundle.
pub fn read_artifact(bundle: &Bundle, rel_path: &str) -> Result<Vec<u8>> {
    Ok(std::fs::read(bundle.dir.join(rel_path))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_round_trip() {
        let der = vec![0x30, 0x82, 0x01, 0x00, 0xff, 0x10, 0x42];
        let pem = pem_encode("CERTIFICATE", &der);
        let blocks = pem_decode_all(&pem).unwrap();
        assert_eq!(blocks, vec![("CERTIFICATE".to_string(), der)]);
        // Multi-block and noise tolerance.
        let two = format!("noise\n{pem}{pem}");
        assert_eq!(pem_decode_all(&two).unwrap().len(), 2);
    }

    #[test]
    fn base64_vectors() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_decode("TWFu").unwrap(), b"Man");
        assert_eq!(base64_decode("TW\nE=").unwrap(), b"Ma");
        assert!(base64_decode("T!Fu").is_err());
    }
}
