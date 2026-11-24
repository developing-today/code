//! Staged config activation and rollback by revision (slice 9111-05,
//! #10217; parent #9111; design
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`,
//! sections "Configuration data model and precedence" and "Failure,
//! recovery, and version rules"; statements S7.1, S7.10, S7.14).
//!
//! The design rule, in one paragraph: "Activation is two-phase: validate
//! against all selected adapter capabilities and peer protocol
//! constraints, then commit a revision; on failure retain previous
//! revision. Rollback restores prior config, not already-completed
//! external effects." This module is the state machine that enforces
//! it for one configuration identity:
//!
//! - [`Rollout::stage`] is phase one. The document is resolved (every
//!   hard invariant of slice 9111-01 applies), the resolved config is
//!   checked against the selected adapters' capabilities through a
//!   [`CapabilityValidator`], and its digest and provenance are
//!   computed. Any failure — invalid document, capability mismatch, an
//!   injected validator failure — rejects the staging and leaves the
//!   active and retained revisions exactly as they were: a bad
//!   adapter/config never activates.
//! - [`Rollout::commit`] is phase two. The staged revision becomes
//!   active and the outgoing active revision is retained as the
//!   rollback target. Staging is single-slot: a second stage is
//!   rejected until the first is committed or aborted, so the revision
//!   that was validated is the revision that activates.
//! - Revisions only move forward through the front door. Staging a
//!   revision that does not strictly advance the active revision — or
//!   one belonging to a different config identity — is rejected;
//!   returning to an earlier revision is exclusively
//!   [`Rollout::rollback`]'s job.
//! - [`Rollout::rollback`] restores the retained previous revision
//!   exactly (the same resolved config and digest that were activated
//!   earlier) and retains the superseded revision in its place, so the
//!   operation is symmetric. Rollback restores configuration only: it
//!   never claims to undo external effects. Effects are recorded
//!   against the revision that was active when they happened
//!   ([`Rollout::record_effect`]); the [`RollbackOutcome`] names the
//!   superseded revision's irreversible effects as *not undone* and
//!   lists the compensating operations they require as explicit
//!   follow-ups — no rollback fiction.
//!
//! This is a leaf state machine: it executes no effects, talks to no
//! adapter, and performs no I/O. Executors drive it and act on its
//! records.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{ConfigDocument, ConfigError, ResolvedConfig, SyncScope, Tier};

/// The digest of a resolved config: lowercase hex SHA-256 over its
/// canonical JSON encoding ([`ResolvedConfig::canonical_json`]). The
/// canonical encoding is deterministic, so the digest identifies the
/// exact resolved revision — values, ceilings, and provenance inputs
/// alike.
///
/// # Errors
/// Returns [`ConfigError::Parse`] if the canonical encoding fails
/// (not expected for these types, but never panics).
pub fn resolved_digest(config: &ResolvedConfig) -> Result<String, ConfigError> {
    let canonical = config.canonical_json()?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// What one selected adapter can do, in the vocabulary of the resolved
/// config. Activation validates the resolved config against every
/// selected adapter: an adapter that cannot carry the config is a
/// capability mismatch, rejected before any operation side effects
/// (design: "Capability mismatch and invalid config are rejected
/// before operation side effects").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterCapabilities {
    /// Stable adapter identity, named in validation failures.
    pub adapter_id: String,
    /// Protocol versions this adapter can speak, in the same canonical
    /// string form the config allowlist uses (e.g. `id/2`). At least
    /// one must intersect the resolved config's allowlist.
    pub protocol_versions: Vec<String>,
    /// Sync scopes this adapter can carry. The resolved behavioral
    /// sync scope must be among them.
    pub sync_scopes: Vec<SyncScope>,
    /// Placement tiers this adapter can serve. The resolved behavioral
    /// tier must be among them.
    pub tiers: Vec<Tier>,
    /// The largest single payload this adapter can carry, in bytes.
    /// The resolved config's `max_bytes` ceiling — the widest payload
    /// any operation may present — must fit within it.
    pub max_payload_bytes: u64,
}

/// One capability requirement a resolved config failed, naming the
/// adapter, the requirement, and the specifics. Validation is
/// fail-closed: the first failure rejects the staging.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("adapter {adapter} does not satisfy {requirement}: {detail}")]
pub struct CapabilityFailure {
    /// The adapter that cannot carry the resolved config.
    pub adapter: String,
    /// The requirement that failed: `adapter-identity`,
    /// `protocol-version`, `sync-scope`, `tier`, or `payload-ceiling`
    /// for the built-in [`AdapterSet`] checks.
    pub requirement: &'static str,
    /// What the config requires versus what the adapter offers.
    pub detail: String,
}

/// The capability-validation seam of phase one. Implementations check
/// a resolved config against the selected adapters (and any peer
/// protocol constraints of the deployment) and reject with a
/// [`CapabilityFailure`]. The seam is how executors — and tests —
/// inject validation failures: a rejected validation retains the
/// previous revision and activates nothing.
pub trait CapabilityValidator {
    /// Check `config` against every selected capability.
    ///
    /// # Errors
    /// Returns the first [`CapabilityFailure`] that rejects the config.
    fn validate(&self, config: &ResolvedConfig) -> Result<(), CapabilityFailure>;
}

impl<F> CapabilityValidator for F
where
    F: Fn(&ResolvedConfig) -> Result<(), CapabilityFailure>,
{
    fn validate(&self, config: &ResolvedConfig) -> Result<(), CapabilityFailure> {
        self(config)
    }
}

/// The built-in [`CapabilityValidator`]: a fixed set of selected
/// adapters, each checked in adapter-id order against the resolved
/// config's protocol allowlist, sync scope, tier, and payload ceiling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdapterSet {
    adapters: Vec<AdapterCapabilities>,
}

impl AdapterSet {
    /// Build the set from the selected adapters. The adapters are
    /// sorted by id so validation order — and therefore which failure
    /// is reported first — is deterministic.
    #[must_use]
    pub fn new(mut adapters: Vec<AdapterCapabilities>) -> Self {
        adapters.sort_by(|a, b| a.adapter_id.cmp(&b.adapter_id));
        Self { adapters }
    }

    /// The selected adapters, in validation order.
    #[must_use]
    pub fn adapters(&self) -> &[AdapterCapabilities] {
        &self.adapters
    }
}

impl CapabilityValidator for AdapterSet {
    fn validate(&self, config: &ResolvedConfig) -> Result<(), CapabilityFailure> {
        for adapter in &self.adapters {
            validate_adapter(adapter, config)?;
        }
        Ok(())
    }
}

fn validate_adapter(
    adapter: &AdapterCapabilities,
    config: &ResolvedConfig,
) -> Result<(), CapabilityFailure> {
    let fail = |requirement: &'static str, detail: String| CapabilityFailure {
        adapter: adapter.adapter_id.clone(),
        requirement,
        detail,
    };
    if adapter.adapter_id.trim().is_empty() {
        return Err(fail(
            "adapter-identity",
            "adapter_id must not be empty".to_owned(),
        ));
    }
    if !adapter
        .protocol_versions
        .iter()
        .any(|version| config.protocol_versions.contains(version))
    {
        return Err(fail(
            "protocol-version",
            format!(
                "config allowlist {:?} shares no protocol version with adapter's {:?}",
                config.protocol_versions, adapter.protocol_versions
            ),
        ));
    }
    if !adapter.sync_scopes.contains(&config.behavior.sync_scope) {
        return Err(fail(
            "sync-scope",
            format!(
                "config resolves sync scope {:?}, adapter supports {:?}",
                config.behavior.sync_scope, adapter.sync_scopes
            ),
        ));
    }
    if !adapter.tiers.contains(&config.behavior.tier) {
        return Err(fail(
            "tier",
            format!(
                "config resolves tier {:?}, adapter supports {:?}",
                config.behavior.tier, adapter.tiers
            ),
        ));
    }
    if adapter.max_payload_bytes < config.ceilings.max_bytes {
        return Err(fail(
            "payload-ceiling",
            format!(
                "config ceiling max_bytes is {}, adapter carries at most {}",
                config.ceilings.max_bytes, adapter.max_payload_bytes
            ),
        ));
    }
    Ok(())
}

/// A resolved configuration revision together with its digest: the
/// unit the rollout machine stages, activates, retains, and restores.
/// The record exposes the resolved digest and provenance the design
/// requires activation to surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRecord {
    /// The resolved configuration this record pins.
    pub config: ResolvedConfig,
    /// [`resolved_digest`] of `config`, computed at staging time.
    pub digest: String,
}

impl RevisionRecord {
    /// The resolved configuration.
    #[must_use]
    pub const fn config(&self) -> &ResolvedConfig {
        &self.config
    }

    /// The revision's digest (hex SHA-256 of the canonical resolved
    /// encoding).
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The configuration identity this revision belongs to.
    #[must_use]
    pub fn config_id(&self) -> &str {
        &self.config.config_id
    }

    /// The monotonic revision number.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.config.revision
    }

    /// Which plane supplied each resolved behavioral field, carried
    /// from the resolved config's provenance record.
    #[must_use]
    pub const fn provenance(&self) -> &BTreeMap<String, String> {
        &self.config.provenance
    }
}

/// An external effect that happened while a revision was active,
/// recorded so a later rollback can report it honestly. Effects are
/// facts about the outside world, not configuration: rollback never
/// undoes them. An irreversible effect may name the explicit
/// compensating operation it requires instead (the issue's rollback
/// contract: "external side effects require explicit compensating
/// operations").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEffect {
    /// Stable effect identity, unique per active revision.
    pub effect_id: String,
    /// What the effect was, in operator terms.
    pub description: String,
    /// Whether the effect can be undone at all. `false` marks an
    /// irreversible effect: a rollback will name it as not undone.
    pub reversible: bool,
    /// The explicit compensating operation this effect requires if it
    /// is irreversible and the operator wants its impact countered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compensating_operation: Option<String>,
}

/// The honest report of a [`Rollout::rollback`]: which revision was
/// restored, which was superseded, and what the rollback does *not*
/// do — the superseded revision's external effects, partitioned by
/// reversibility, with the compensating operations the irreversible
/// ones require.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackOutcome {
    /// The revision now active: the exact retained previous record.
    pub restored: RevisionRecord,
    /// The revision that was active until the rollback; it is now the
    /// retained previous revision.
    pub superseded: RevisionRecord,
    /// Irreversible effects recorded under the superseded revision.
    /// These are **not undone** by the rollback; they are named so no
    /// reader can mistake a config restore for an effect undo.
    pub irreversible_effects: Vec<ExternalEffect>,
    /// The compensating operations declared by the irreversible
    /// effects, in recording order. Irreversible effects that declared
    /// none contribute nothing here but still appear in
    /// `irreversible_effects`.
    pub compensations_required: Vec<String>,
    /// Reversible effects recorded under the superseded revision. The
    /// machine does not undo them itself; they are listed so the
    /// operator/runtime can.
    pub reversible_effects: Vec<ExternalEffect>,
}

/// Why a rollout operation was rejected. Every variant is a refusal
/// that leaves the machine's state unchanged.
#[derive(Debug, Error)]
pub enum RolloutError {
    /// The document failed resolution (a hard invariant of the config
    /// contract rejects it).
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Capability validation rejected the resolved config.
    #[error(transparent)]
    Capability(#[from] CapabilityFailure),
    /// A revision is already staged; commit or abort it first, so the
    /// validated revision is the one that activates.
    #[error("revision {revision} is already staged; commit or abort it before staging another")]
    AlreadyStaged {
        /// The revision currently staged.
        revision: u64,
    },
    /// [`Rollout::commit`] was called with nothing staged.
    #[error("no revision is staged")]
    NothingStaged,
    /// An operation needed an active revision and there is none.
    #[error("no configuration revision is active")]
    NoActiveRevision,
    /// [`Rollout::rollback`] was called with no retained previous
    /// revision.
    #[error("no previous revision is retained for rollback")]
    NoPreviousRevision,
    /// The staged document belongs to a different configuration than
    /// the active one; one rollout machine serves one config identity.
    #[error(
        "staged revision belongs to config {candidate:?}, but the active configuration is {active:?}"
    )]
    ConfigIdMismatch {
        /// The active configuration's identity.
        active: String,
        /// The staged document's identity.
        candidate: String,
    },
    /// The staged revision does not strictly advance the active
    /// revision. Returning to an earlier revision is rollback's job,
    /// never a silent front-door downgrade.
    #[error(
        "revision {candidate} does not advance the active revision {active}; use rollback to return to an earlier revision"
    )]
    RevisionNotAdvanced {
        /// The active revision.
        active: u64,
        /// The staged document's revision.
        candidate: u64,
    },
    /// An external effect was recorded with an empty id.
    #[error("external effect id must not be empty")]
    EmptyEffectId,
    /// An external effect id was recorded twice under one revision.
    #[error("external effect {0:?} is already recorded for the active revision")]
    DuplicateEffect(String),
}

#[derive(Debug)]
struct EffectRecord {
    revision: u64,
    effect: ExternalEffect,
}

/// The rollout state machine for one configuration identity: at most
/// one active revision, one retained previous revision (the rollback
/// target), and one staged revision awaiting commit.
#[derive(Debug, Default)]
pub struct Rollout {
    active: Option<RevisionRecord>,
    previous: Option<RevisionRecord>,
    staged: Option<RevisionRecord>,
    effects: Vec<EffectRecord>,
}

impl Rollout {
    /// A fresh machine: nothing active, nothing retained, nothing
    /// staged.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently active revision, if any.
    #[must_use]
    pub const fn active(&self) -> Option<&RevisionRecord> {
        self.active.as_ref()
    }

    /// The retained previous revision — the rollback target — if any.
    #[must_use]
    pub const fn previous(&self) -> Option<&RevisionRecord> {
        self.previous.as_ref()
    }

    /// The staged revision awaiting commit, if any.
    #[must_use]
    pub const fn staged(&self) -> Option<&RevisionRecord> {
        self.staged.as_ref()
    }

    /// Phase one of activation: resolve `document`, check the resolved
    /// config against `validator`, and stage the resulting revision.
    /// On any failure the machine is unchanged — the previous revision
    /// is retained and nothing activates.
    ///
    /// # Errors
    /// Returns [`RolloutError::AlreadyStaged`] if a revision is staged,
    /// [`RolloutError::Config`] if the document fails resolution,
    /// [`RolloutError::ConfigIdMismatch`] or
    /// [`RolloutError::RevisionNotAdvanced`] if the document does not
    /// belong to — or does not advance — the active configuration, and
    /// [`RolloutError::Capability`] if capability validation rejects
    /// the resolved config.
    pub fn stage(
        &mut self,
        document: &ConfigDocument,
        validator: &dyn CapabilityValidator,
    ) -> Result<RevisionRecord, RolloutError> {
        if let Some(staged) = &self.staged {
            return Err(RolloutError::AlreadyStaged {
                revision: staged.revision(),
            });
        }
        let resolved = document.resolve()?;
        if let Some(active) = &self.active {
            if active.config_id() != resolved.config_id {
                return Err(RolloutError::ConfigIdMismatch {
                    active: active.config_id().to_owned(),
                    candidate: resolved.config_id.clone(),
                });
            }
            if resolved.revision <= active.revision() {
                return Err(RolloutError::RevisionNotAdvanced {
                    active: active.revision(),
                    candidate: resolved.revision,
                });
            }
        }
        let digest = resolved_digest(&resolved)?;
        validator.validate(&resolved)?;
        let record = RevisionRecord {
            config: resolved,
            digest,
        };
        self.staged = Some(record.clone());
        Ok(record)
    }

    /// Discard the staged revision, if any, without activating it.
    /// Returns the discarded record.
    pub fn abort_staged(&mut self) -> Option<RevisionRecord> {
        self.staged.take()
    }

    /// Phase two of activation: the staged revision becomes active
    /// and the outgoing active revision is retained as the rollback
    /// target. Returns the newly active record.
    ///
    /// # Errors
    /// Returns [`RolloutError::NothingStaged`] if no revision is
    /// staged.
    pub fn commit(&mut self) -> Result<RevisionRecord, RolloutError> {
        let staged = self.staged.take().ok_or(RolloutError::NothingStaged)?;
        self.previous = self.active.replace(staged);
        // `replace` has just installed the staged record, so the active
        // slot is occupied; the error arm is unreachable in practice and
        // exists only to keep the no-panic posture.
        self.active.clone().ok_or(RolloutError::NoActiveRevision)
    }

    /// Record an external effect against the currently active
    /// revision, so a later rollback can report it honestly.
    ///
    /// # Errors
    /// Returns [`RolloutError::NoActiveRevision`] if nothing is
    /// active, [`RolloutError::EmptyEffectId`] for an empty effect id,
    /// and [`RolloutError::DuplicateEffect`] if the id is already
    /// recorded under the active revision.
    pub fn record_effect(&mut self, effect: ExternalEffect) -> Result<(), RolloutError> {
        let Some(active) = &self.active else {
            return Err(RolloutError::NoActiveRevision);
        };
        if effect.effect_id.trim().is_empty() {
            return Err(RolloutError::EmptyEffectId);
        }
        let revision = active.revision();
        if self.effects.iter().any(|record| {
            record.revision == revision && record.effect.effect_id == effect.effect_id
        }) {
            return Err(RolloutError::DuplicateEffect(effect.effect_id));
        }
        self.effects.push(EffectRecord { revision, effect });
        Ok(())
    }

    /// Restore the retained previous revision exactly, retaining the
    /// superseded revision in its place. The outcome names the
    /// superseded revision's external effects: irreversible ones are
    /// not undone, and their declared compensating operations are
    /// listed as required follow-ups.
    ///
    /// # Errors
    /// Returns [`RolloutError::NoPreviousRevision`] if no previous
    /// revision is retained.
    pub fn rollback(&mut self) -> Result<RollbackOutcome, RolloutError> {
        let Some(restored) = self.previous.take() else {
            return Err(RolloutError::NoPreviousRevision);
        };
        let Some(superseded) = self.active.replace(restored.clone()) else {
            // Unreachable in practice: a retained previous revision is
            // only ever set alongside an active one. Restore the state
            // rather than panic.
            self.previous = self.active.take();
            return Err(RolloutError::NoActiveRevision);
        };
        self.previous = Some(superseded.clone());

        let mut irreversible_effects = Vec::new();
        let mut compensations_required = Vec::new();
        let mut reversible_effects = Vec::new();
        for record in &self.effects {
            if record.revision != superseded.revision() {
                continue;
            }
            if record.effect.reversible {
                reversible_effects.push(record.effect.clone());
            } else {
                if let Some(operation) = &record.effect.compensating_operation {
                    compensations_required.push(operation.clone());
                }
                irreversible_effects.push(record.effect.clone());
            }
        }

        Ok(RollbackOutcome {
            restored,
            superseded,
            irreversible_effects,
            compensations_required,
            reversible_effects,
        })
    }
}
