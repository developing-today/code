//! Context hydration contract: safe cross-repository source keys and
//! artifact paths (developing-today/code#9207 slice HYD-02).
//! Pure, persistence-free helpers: no storage, no network, no schema.
//! Identity is canonical `provider://owner/repository/resource_kind/remote_id`;
//! artifact keys are repo-scoped digest-suffixed slugs so the same issue/PR
//! number in two repositories can never collide, and traversal or control
//! input is rejected before it ever touches a path.

use crate::work::contracts::{ContractError, WorkRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Canonical source identity for one linked context source.
///
/// Never derived from a GitHub issue/PR number alone: the repository
/// identity and stable remote ID are part of the key, so `alpha/repo#10`
/// and `beta/repo#10` are distinct even though both display as `#10`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceRef {
    /// `github` or another registered provider class.
    pub provider: String,
    /// Provider account / owner.
    pub owner: String,
    /// Repository name.
    pub repository: String,
    /// `issue`, `pull_request`, `review`, `comment`, ...
    pub resource_kind: String,
    /// Stable remote ID assigned by the provider (not a display number).
    pub remote_id: String,
}

impl SourceRef {
    pub fn new(
        provider: impl Into<String>,
        owner: impl Into<String>,
        repository: impl Into<String>,
        resource_kind: impl Into<String>,
        remote_id: impl Into<String>,
    ) -> Result<Self, SourceError> {
        let provider = provider.into();
        let owner = owner.into();
        let repository = repository.into();
        let resource_kind = resource_kind.into();
        let remote_id = remote_id.into();
        validate_source_segment(&provider, "provider")?;
        validate_source_segment(&owner, "owner")?;
        validate_source_segment(&repository, "repository")?;
        validate_source_segment(&resource_kind, "resource_kind")?;
        validate_source_segment(&remote_id, "remote_id")?;
        Ok(SourceRef {
            provider,
            owner,
            repository,
            resource_kind,
            remote_id,
        })
    }

    /// Canonical stable URI form used as the digest input.
    pub fn canonical_uri(&self) -> String {
        format!(
            "{}://{}/{}/{}/{}",
            self.provider, self.owner, self.repository, self.resource_kind, self.remote_id
        )
    }
}

/// Segment validation: reject path separators, `..`, control characters
/// and anything that could escape the artifact root or be confused across
/// repositories.
fn validate_source_segment(value: &str, field: &str) -> Result<(), SourceError> {
    if value.is_empty() {
        return Err(SourceError::EmptySegment {
            field: field.to_string(),
        });
    }
    if value.contains('/') || value.contains('\\') {
        return Err(SourceError::PathSeparator {
            field: field.to_string(),
            value: value.to_string(),
        });
    }
    if value == ".." || value == "." {
        return Err(SourceError::DotSegment {
            field: field.to_string(),
            value: value.to_string(),
        });
    }
    if value.contains('\0') || value.chars().any(|c| c.is_control()) {
        return Err(SourceError::ControlChar {
            field: field.to_string(),
        });
    }
    Ok(())
}

/// Collision-resistant, traversal-safe artifact key for one source.
///
/// Format: `<kind>-<slug>-<digest>.<ext>` where the digest covers the full
/// canonical source URI so the same PR number in two repositories yields
/// distinct keys, and the slug is a safe rendering of the repository and
/// remote ID only (never titles or user-supplied paths).
pub fn artifact_key(
    source: &SourceRef,
    kind: ArtifactKind,
    extension: &str,
) -> Result<String, SourceError> {
    validate_extension(extension)?;
    let slug = format!(
        "{}-{}",
        slug_repo(&source.repository),
        slug_remote(&source.remote_id),
    );
    let mut hasher = Sha256::new();
    hasher.update(source.canonical_uri().as_bytes());
    let digest = hasher.finalize();
    let hex_suffix = encode_hex_lower(&digest)[..16].to_string();
    Ok(format!(
        "{}-{}-{}.{}",
        kind.as_str(),
        slug,
        hex_suffix,
        extension
    ))
}

/// Kind of artifact emitted for one source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArtifactKind {
    /// Full source record (JSON).
    Record,
    /// Raw diff/patch bytes.
    Diff,
    /// Concise navigation summary (never evidence of completeness).
    Summary,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Record => "record",
            ArtifactKind::Diff => "diff",
            ArtifactKind::Summary => "summary",
        }
    }
}

/// Slug for a repository: lowercase, runs of non-alphanumerics collapsed to
/// a single hyphen, trimmed. Never collapses distinct repositories into one
/// slug — the digest suffix carries uniqueness.
fn slug_repo(repository: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = true;
    for c in repository.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "repo".to_string()
    } else {
        out
    }
}

/// Slug for a stable remote ID: keep digits and alphanumerics only.
fn slug_remote(remote_id: &str) -> String {
    remote_id
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
}

/// Validate an artifact extension: no path separators, no `..`, no control.
fn validate_extension(extension: &str) -> Result<(), SourceError> {
    if extension.is_empty() {
        return Err(SourceError::EmptyExtension);
    }
    if extension.starts_with('.') {
        return Err(SourceError::DotExtension);
    }
    if extension.contains('/')
        || extension.contains('\\')
        || extension.contains('\0')
        || extension.chars().any(|c| c.is_control())
    {
        return Err(SourceError::BadExtension {
            extension: extension.to_string(),
        });
    }
    Ok(())
}

fn encode_hex_lower(bytes: &[u8]) -> String {
    const CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(CHARS[(b >> 4) as usize] as char);
        out.push(CHARS[(b & 0x0f) as usize] as char);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    EmptySegment { field: String },
    PathSeparator { field: String, value: String },
    DotSegment { field: String, value: String },
    ControlChar { field: String },
    EmptyExtension,
    BadExtension { extension: String },
    DotExtension,
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::EmptySegment { field } => {
                write!(f, "empty source segment: {field}")
            }
            SourceError::PathSeparator { field, value } => {
                write!(f, "path separator in source segment {field}: {value:?}")
            }
            SourceError::DotSegment { field, value } => {
                write!(f, "dot segment in source segment {field}: {value:?}")
            }
            SourceError::ControlChar { field } => {
                write!(f, "control character in source segment {field}")
            }
            SourceError::EmptyExtension => write!(f, "empty artifact extension"),
            SourceError::BadExtension { extension } => {
                write!(f, "bad artifact extension: {extension:?}")
            }
            SourceError::DotExtension => write!(f, "dot artifact extension"),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<SourceError> for ContractError {
    fn from(err: SourceError) -> Self {
        ContractError::InvalidSchema {
            detail: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_a() -> SourceRef {
        SourceRef::new("github", "alpha", "repo", "pull_request", "231").unwrap()
    }
    fn repo_b() -> SourceRef {
        SourceRef::new("github", "beta", "repo", "pull_request", "231").unwrap()
    }

    /// HYD-02 oracle: same issue/PR number in two repositories plus `../`,
    /// control and long Unicode titles never collide or escape; distinct
    /// refs stay distinct.
    #[test]
    fn pmo_hyd_artifact_keys_are_repo_scoped_and_traversal_safe() {
        // Same PR number in two repositories -> distinct keys.
        let a = artifact_key(&repo_a(), ArtifactKind::Record, "json").unwrap();
        let b = artifact_key(&repo_b(), ArtifactKind::Record, "json").unwrap();
        assert_ne!(a, b, "same PR number in two repos must not collide");
        assert!(a.starts_with("record-"));
        assert!(a.ends_with(".json"));
        // Digest suffix present and deterministic.
        let a2 = artifact_key(&repo_a(), ArtifactKind::Record, "json").unwrap();
        assert_eq!(a, a2, "same source yields deterministic key");

        // `..` / traversal input is rejected.
        assert!(SourceRef::new("github", "..", "repo", "issue", "5").is_err());
        assert!(SourceRef::new("github", "owner", "../repo", "issue", "5").is_err());
        assert!(SourceRef::new("github", "owner", "repo", "issue", "5/..").is_err());

        // Control characters rejected.
        assert!(SourceRef::new("github", "owner", "repo", "issue", "5\0x").is_err());
        assert!(SourceRef::new("github", "owner", "repo", "issue", "5\x1b").is_err());

        // Empty segments rejected.
        assert!(SourceRef::new("github", "", "repo", "issue", "5").is_err());
        assert!(SourceRef::new("github", "owner", "", "issue", "5").is_err());

        // Long Unicode title-like input does not collapse distinct refs
        // into one slug: the digest suffix carries uniqueness.
        let long_a = SourceRef::new(
            "github",
            "owner",
            "a-long-ünïcode-repo-name-🚀",
            "issue",
            "42",
        )
        .unwrap();
        let long_b = SourceRef::new(
            "github",
            "owner",
            "a-long-unicode-repo-name-🚀",
            "issue",
            "42",
        )
        .unwrap();
        assert_ne!(
            artifact_key(&long_a, ArtifactKind::Record, "json").unwrap(),
            artifact_key(&long_b, ArtifactKind::Record, "json").unwrap(),
            "distinct refs never collapse"
        );

        // Different resource kind / remote id -> distinct keys.
        assert_ne!(
            artifact_key(&repo_a(), ArtifactKind::Record, "json").unwrap(),
            artifact_key(
                &SourceRef::new("github", "alpha", "repo", "issue", "231").unwrap(),
                ArtifactKind::Record,
                "json"
            )
            .unwrap(),
            "different resource kind is distinct"
        );

        // Bad extensions rejected.
        assert!(artifact_key(&repo_a(), ArtifactKind::Record, "../evil").is_err());
        assert!(artifact_key(&repo_a(), ArtifactKind::Record, ".json").is_err());
        assert!(artifact_key(&repo_a(), ArtifactKind::Record, "js\non").is_err());

        // WorkRef pair identity still validates (digest domain framing not
        // affected by source keys).
        let work = WorkRef::new("local", "842").unwrap();
        assert_eq!(work.qualified_uri(), "fleet://local/work/842");
    }
}
