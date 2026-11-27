//! Versioned hydration manifest and result envelope — HYD-01
//! (developing-today/code#9207, program #9196).
//!
//! This module is the offline manifest slice of the PM orchestration
//! design (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/components/9207-context-hydration-incremental-activity.md`,
//! §2.2): the versioned `fleet.context-manifest/v1` shape, per-source
//! completeness aggregation, and the exact
//! `fleet.context-manifest/v1` digest framing with self-field
//! exclusion. It is pure types and validation: no I/O, no queue, no
//! state — a rejection is a typed [`ContractError`] that mutates
//! nothing.
//!
//! Wire rules (from `../architecture.md` via `contracts.rs`): field
//! names are snake_case; work identity is the `(authority_id,
//! work_id)` string pair ([`WorkRef`]); the manifest digest omits only
//! `integrity.manifest_digest` before RFC 8785 canonicalization,
//! hashes ASCII(domain) + one NUL byte + canonical UTF-8, and emits
//! `sha256:` plus lowercase hex. Hashing a manifest including its own
//! digest is not the contract.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contracts::{
    ContractError, DIGEST_DOMAIN_CONTEXT_MANIFEST, WorkRef, digest, is_digest_string,
};

/// Wire schema identifier of the hydration manifest.
pub const CONTEXT_MANIFEST_SCHEMA_VERSION: &str = "fleet.context-manifest/v1";

/// Aggregate manifest status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestStatus {
    Complete,
    Partial,
    Failed,
}

impl ManifestStatus {
    fn as_str(self) -> &'static str {
        match self {
            ManifestStatus::Complete => "complete",
            ManifestStatus::Partial => "partial",
            ManifestStatus::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "complete" => Some(ManifestStatus::Complete),
            "partial" => Some(ManifestStatus::Partial),
            "failed" => Some(ManifestStatus::Failed),
            _ => None,
        }
    }
}

/// Per-source completeness status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Complete,
    Partial,
    Unavailable,
    Forbidden,
    NotFound,
    Truncated,
}

impl SourceStatus {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "complete" => Some(SourceStatus::Complete),
            "partial" => Some(SourceStatus::Partial),
            "unavailable" => Some(SourceStatus::Unavailable),
            "forbidden" => Some(SourceStatus::Forbidden),
            "not_found" => Some(SourceStatus::NotFound),
            "truncated" => Some(SourceStatus::Truncated),
            _ => None,
        }
    }
}

/// One stored artifact reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

impl ArtifactRef {
    fn validate(&self) -> Result<(), ContractError> {
        if self.path.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "artifact_ref.path must not be empty".to_string(),
            });
        }
        if !is_digest_string(&self.sha256) {
            return Err(ContractError::InvalidSchema {
                detail: format!(
                    "artifact_ref.sha256 is not a wire digest: {:?}",
                    self.sha256
                ),
            });
        }
        Ok(())
    }
}

/// One hydrated source entry (HYD-01 minimal projection: identity,
/// endpoint, completeness, and artifact refs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntry {
    pub source_ref: String,
    pub endpoint: String,
    pub status: SourceStatus,
    pub artifact_refs: Vec<ArtifactRef>,
}

impl SourceEntry {
    fn validate(&self) -> Result<(), ContractError> {
        if self.source_ref.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "source_ref must not be empty".to_string(),
            });
        }
        if self.endpoint.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "endpoint must not be empty".to_string(),
            });
        }
        for artifact in &self.artifact_refs {
            artifact.validate()?;
        }
        Ok(())
    }
}

/// Manifest integrity block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIntegrity {
    pub algorithm: String,
    pub manifest_digest: String,
}

/// Versioned hydration manifest (HYD-01 shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextManifest {
    pub schema_version: String,
    pub work: WorkRef,
    pub generation_id: String,
    pub created_at: String,
    pub status: ManifestStatus,
    pub sources: Vec<SourceEntry>,
    pub integrity: ManifestIntegrity,
}

impl ContextManifest {
    /// Validate shapes and completeness aggregation: `complete`
    /// requires every source `complete`; any missing, forbidden,
    /// unavailable, truncated, or partial source forces `partial` (or
    /// `failed` when no generation could be published). A manifest
    /// that calls itself `complete` while a required source is not
    /// complete is rejected.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != CONTEXT_MANIFEST_SCHEMA_VERSION {
            return Err(ContractError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        if ManifestStatus::parse(self.status.as_str()).is_none() {
            return Err(ContractError::InvalidSchema {
                detail: format!("unknown manifest status: {:?}", self.status.as_str()),
            });
        }
        self.work.validate()?;
        if self.generation_id.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "generation_id must not be empty".to_string(),
            });
        }
        if self.created_at.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "created_at must not be empty".to_string(),
            });
        }
        if self.integrity.algorithm != "sha256" {
            return Err(ContractError::InvalidSchema {
                detail: format!(
                    "integrity.algorithm must be sha256, got {:?}",
                    self.integrity.algorithm
                ),
            });
        }
        if !is_digest_string(&self.integrity.manifest_digest) {
            return Err(ContractError::InvalidSchema {
                detail: "integrity.manifest_digest is not a wire digest".to_string(),
            });
        }
        for source in &self.sources {
            if SourceStatus::parse(match source.status {
                SourceStatus::Complete => "complete",
                SourceStatus::Partial => "partial",
                SourceStatus::Unavailable => "unavailable",
                SourceStatus::Forbidden => "forbidden",
                SourceStatus::NotFound => "not_found",
                SourceStatus::Truncated => "truncated",
            })
            .is_none()
            {
                return Err(ContractError::InvalidSchema {
                    detail: "unknown source status".to_string(),
                });
            }
            source.validate()?;
        }
        let all_complete = self
            .sources
            .iter()
            .all(|s| s.status == SourceStatus::Complete);
        match self.status {
            ManifestStatus::Complete if !all_complete => Err(ContractError::InvalidSchema {
                detail: "status is complete but a required source is not complete".to_string(),
            }),
            ManifestStatus::Partial if all_complete && !self.sources.is_empty() => {
                Err(ContractError::InvalidSchema {
                    detail: "status is partial but every source is complete".to_string(),
                })
            }
            _ => Ok(()),
        }
    }

    /// Canonical JSON of the manifest with `integrity.manifest_digest`
    /// omitted — the exact bytes the digest covers.
    fn digest_dto(&self) -> Result<Value, ContractError> {
        let mut value = serde_json::to_value(self).map_err(|err| ContractError::InvalidSchema {
            detail: format!("manifest is not JSON-serializable: {err}"),
        })?;
        if let Some(integrity) = value.get_mut("integrity") {
            if let Some(map) = integrity.as_object_mut() {
                map.remove("manifest_digest");
            }
        }
        Ok(value)
    }

    /// Compute the manifest digest under `fleet.context-manifest/v1`.
    pub fn compute_digest(&self) -> Result<String, ContractError> {
        digest(DIGEST_DOMAIN_CONTEXT_MANIFEST, &self.digest_dto()?)
    }

    /// Verify `integrity.manifest_digest` matches the recomputed digest.
    pub fn verify_digest(&self) -> Result<(), ContractError> {
        let expected = self.compute_digest()?;
        if expected == self.integrity.manifest_digest {
            Ok(())
        } else {
            Err(ContractError::InvalidSchema {
                detail: format!(
                    "manifest digest mismatch: expected {expected}, got {}",
                    self.integrity.manifest_digest
                ),
            })
        }
    }

    /// Parse JSON text and validate (schema version, shapes,
    /// completeness aggregation). Digest verification is a separate
    /// [`ContextManifest::verify_digest`] step so framing errors and
    /// digest mismatches stay distinct.
    pub fn from_json(text: &str) -> Result<Self, ContractError> {
        let manifest: Self =
            serde_json::from_str(text).map_err(|err| ContractError::InvalidSchema {
                detail: format!("manifest does not match {CONTEXT_MANIFEST_SCHEMA_VERSION}: {err}"),
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Serialize to JSON text.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("manifest serialization is infallible")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_artifact() -> ArtifactRef {
        ArtifactRef {
            path: "pull-123-456-789.json".to_string(),
            sha256: format!("sha256:{}", "a".repeat(64)),
            bytes: 123,
        }
    }

    fn fixture_manifest(status: ManifestStatus, source_status: SourceStatus) -> ContextManifest {
        let mut manifest = ContextManifest {
            schema_version: CONTEXT_MANIFEST_SCHEMA_VERSION.to_string(),
            work: WorkRef::new("local", "842").unwrap(),
            generation_id: "g-fixture-1".to_string(),
            created_at: "2026-11-26T00:00:00Z".to_string(),
            status,
            sources: vec![SourceEntry {
                source_ref: "github://owner/repo/pulls/231".to_string(),
                endpoint: "pull-request-detail".to_string(),
                status: source_status,
                artifact_refs: vec![fixture_artifact()],
            }],
            integrity: ManifestIntegrity {
                algorithm: "sha256".to_string(),
                manifest_digest: format!("sha256:{}", "0".repeat(64)),
            },
        };
        manifest.integrity.manifest_digest = manifest.compute_digest().unwrap();
        manifest
    }

    /// HYD-01 oracle: a missing/unavailable required source can never
    /// be published as `complete`; it aggregates to `partial`.
    #[test]
    fn pmo_hyd_manifest_missing_required_source_is_partial() {
        // Unavailable source claimed complete: rejected.
        let bad = fixture_manifest(ManifestStatus::Complete, SourceStatus::Unavailable);
        assert!(matches!(
            bad.validate(),
            Err(ContractError::InvalidSchema { .. })
        ));

        // Same source reported partial: accepted.
        let partial = fixture_manifest(ManifestStatus::Partial, SourceStatus::Unavailable);
        partial.validate().unwrap();
        partial.verify_digest().unwrap();

        // All sources complete claimed complete: accepted.
        let complete = fixture_manifest(ManifestStatus::Complete, SourceStatus::Complete);
        complete.validate().unwrap();
        complete.verify_digest().unwrap();

        // Digest covers the DTO without the digest field itself:
        // recomputing over the full JSON (digest included) differs.
        let full_value = serde_json::to_value(&complete).unwrap();
        let naive = digest(DIGEST_DOMAIN_CONTEXT_MANIFEST, &full_value).unwrap();
        assert_ne!(naive, complete.integrity.manifest_digest);
    }

    /// HYD-01 oracle: unknown schema versions are typed
    /// `UnsupportedVersion`, never coerced; digest framing is
    /// independent of the envelope.
    #[test]
    fn pmo_hyd_manifest_rejects_unknown_schema_version() {
        let mut manifest = fixture_manifest(ManifestStatus::Partial, SourceStatus::Unavailable);
        manifest.schema_version = "fleet.context-manifest/v999".to_string();
        assert!(matches!(
            manifest.validate(),
            Err(ContractError::UnsupportedVersion { .. })
        ));

        // from_json surfaces the same typed rejection.
        let mut value = serde_json::to_value(&fixture_manifest(
            ManifestStatus::Partial,
            SourceStatus::Unavailable,
        ))
        .unwrap();
        value["schema_version"] = Value::String("fleet.context-manifest/v999".to_string());
        let text = serde_json::to_string(&value).unwrap();
        assert!(matches!(
            ContextManifest::from_json(&text),
            Err(ContractError::UnsupportedVersion { .. })
        ));
    }
}
