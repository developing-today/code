//! Oracle tests for slice 9111-05 (#10217): staged config activation and
//! rollback by revision.
//!
//! The oracle is the issue's model statement: *a bad adapter/config never
//! activates; rollback selects the exact previous revision and does not
//! claim to undo external effects.* The verifier shape is the rollout
//! state machine driven with injected validation failures (a closure
//! validator that always fails, adapters missing one capability each)
//! and irreversible-effect markers recorded under the active revision.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;

use id_config::rollout::{
    AdapterCapabilities, AdapterSet, CapabilityFailure, ExternalEffect, Rollout, RolloutError,
};
use id_config::{
    Behavior, CONFIG_SCHEMA_VERSION, Ceilings, ConfigDocument, Fanout, FixedGates, Mode, SyncScope,
    Tier,
};
use sha2::{Digest as _, Sha256};

fn document(revision: u64) -> ConfigDocument {
    ConfigDocument {
        schema_version: CONFIG_SCHEMA_VERSION,
        config_id: "node-config".to_owned(),
        revision,
        mode: Mode::HostedTenant,
        protocol_versions: vec!["id/2".to_owned()],
        ceilings: Ceilings {
            max_hops: 5,
            max_bytes: 4096,
            max_time_ms: 1000,
            max_fan_out: 10,
        },
        gates: FixedGates::default(),
        behavior: Behavior {
            max_hops: Some(2),
            sync_scope: None,
            tier: None,
        },
        overrides: vec![],
        extensions: BTreeMap::new(),
        fanout: Fanout::default(),
        peers: Vec::new(),
        storage: id_config::Storage::default(),
    }
}

fn capable_adapter() -> AdapterCapabilities {
    AdapterCapabilities {
        adapter_id: "store-redb".to_owned(),
        protocol_versions: vec!["id/2".to_owned()],
        sync_scopes: vec![SyncScope::Metadata, SyncScope::Full],
        tiers: vec![Tier::Cold, Tier::Warm, Tier::Hot],
        max_payload_bytes: 8192,
    }
}

fn capable_set() -> AdapterSet {
    AdapterSet::new(vec![capable_adapter()])
}

/// The independent digest oracle: SHA-256 over the resolved config's
/// canonical JSON, computed here from the canonical bytes directly.
fn expected_digest(doc: &ConfigDocument) -> String {
    let resolved = doc.resolve().expect("test document resolves");
    let canonical = resolved.canonical_json().expect("canonical json");
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn activate(rollout: &mut Rollout, revision: u64) {
    rollout
        .stage(&document(revision), &capable_set())
        .expect("stage succeeds");
    rollout.commit().expect("commit succeeds");
}

#[test]
fn valid_config_activates_with_digest_and_provenance_exposed() {
    let mut rollout = Rollout::new();
    let staged = rollout
        .stage(&document(1), &capable_set())
        .expect("capable adapter validates");
    assert_eq!(staged.digest(), expected_digest(&document(1)));
    assert_eq!(staged.revision(), 1);
    assert_eq!(staged.config_id(), "node-config");
    // Provenance is exposed with the record: the document supplied
    // max_hops, the mode bundle supplied the rest.
    assert_eq!(
        staged.provenance().get("max_hops").map(String::as_str),
        Some("document")
    );
    assert_eq!(
        staged.provenance().get("sync_scope").map(String::as_str),
        Some("mode:hosted-tenant")
    );

    let active = rollout.commit().expect("commit activates the stage");
    assert_eq!(active, staged);
    assert_eq!(rollout.active(), Some(&staged));
    assert_eq!(rollout.previous(), None);
    assert_eq!(rollout.staged(), None);
}

#[test]
fn capability_mismatch_never_activates() {
    // One adapter per missing capability: each must fail staging with the
    // named requirement, and none may leave anything staged or active.
    let mut no_protocol = capable_adapter();
    no_protocol.protocol_versions = vec!["id/1".to_owned()];
    let mut no_scope = capable_adapter();
    no_scope.sync_scopes = vec![SyncScope::Metadata];
    let mut no_tier = capable_adapter();
    no_tier.tiers = vec![Tier::Cold];
    let mut small_payload = capable_adapter();
    small_payload.max_payload_bytes = 1024;

    for (adapter, requirement) in [
        (no_protocol, "protocol-version"),
        (no_scope, "sync-scope"),
        (no_tier, "tier"),
        (small_payload, "payload-ceiling"),
    ] {
        let mut rollout = Rollout::new();
        let set = AdapterSet::new(vec![adapter]);
        let error = rollout
            .stage(&document(1), &set)
            .expect_err("incapable adapter must reject staging");
        let RolloutError::Capability(failure) = error else {
            panic!("expected capability failure, got {error:?}");
        };
        assert_eq!(failure.adapter, "store-redb");
        assert_eq!(failure.requirement, requirement);
        assert_eq!(rollout.active(), None);
        assert_eq!(rollout.staged(), None);
        assert!(matches!(rollout.commit(), Err(RolloutError::NothingStaged)));
    }
}

#[test]
fn invalid_config_never_activates() {
    let mut doc = document(1);
    doc.gates.validation = false;
    let mut rollout = Rollout::new();
    let error = rollout
        .stage(&doc, &capable_set())
        .expect_err("a document disabling a fixed gate must reject");
    assert!(matches!(
        error,
        RolloutError::Config(id_config::ConfigError::FixedGateDisabled("validation"))
    ));
    assert_eq!(rollout.active(), None);
    assert_eq!(rollout.staged(), None);
}

#[test]
fn injected_validator_failure_retains_previous_revision() {
    let mut rollout = Rollout::new();
    activate(&mut rollout, 1);
    let active_before = rollout.active().expect("rev 1 active").clone();

    let always_fail = |_config: &id_config::ResolvedConfig| -> Result<(), CapabilityFailure> {
        Err(CapabilityFailure {
            adapter: "injected".to_owned(),
            requirement: "injected-failure",
            detail: "test validator rejects everything".to_owned(),
        })
    };
    let error = rollout
        .stage(&document(2), &always_fail)
        .expect_err("injected failure must reject staging");
    assert!(matches!(error, RolloutError::Capability(_)));
    // On failure the previous revision is retained, exactly.
    assert_eq!(rollout.active(), Some(&active_before));
    assert_eq!(rollout.staged(), None);
    assert!(matches!(rollout.commit(), Err(RolloutError::NothingStaged)));
}

#[test]
fn rollback_selects_the_exact_previous_revision() {
    let mut rollout = Rollout::new();
    activate(&mut rollout, 1);
    let rev1 = rollout.active().expect("rev 1 active").clone();
    activate(&mut rollout, 2);
    let rev2 = rollout.active().expect("rev 2 active").clone();
    assert_eq!(rollout.previous(), Some(&rev1));

    let outcome = rollout.rollback().expect("rollback succeeds");
    assert_eq!(outcome.restored, rev1);
    assert_eq!(outcome.superseded, rev2);
    // The restored active config is byte-for-byte the earlier activation:
    // same resolved config, same digest.
    assert_eq!(rollout.active(), Some(&rev1));
    assert_eq!(
        rollout.active().expect("active").digest(),
        expected_digest(&document(1))
    );
    // The superseded revision is retained, so rolling forward again is
    // the same explicit operation in the other direction.
    let forward = rollout.rollback().expect("roll forward succeeds");
    assert_eq!(forward.restored, rev2);
    assert_eq!(rollout.active(), Some(&rev2));
}

#[test]
fn rollback_names_irreversible_effects_and_never_claims_to_undo_them() {
    let mut rollout = Rollout::new();
    activate(&mut rollout, 1);
    activate(&mut rollout, 2);

    rollout
        .record_effect(ExternalEffect {
            effect_id: "announce-1".to_owned(),
            description: "federation announcement sent to peers".to_owned(),
            reversible: false,
            compensating_operation: Some("send a signed retraction".to_owned()),
        })
        .expect("record irreversible effect with compensation");
    rollout
        .record_effect(ExternalEffect {
            effect_id: "mint-1".to_owned(),
            description: "externally witnessed token minted".to_owned(),
            reversible: false,
            compensating_operation: None,
        })
        .expect("record irreversible effect without compensation");
    rollout
        .record_effect(ExternalEffect {
            effect_id: "cache-1".to_owned(),
            description: "local cache warmed".to_owned(),
            reversible: true,
            compensating_operation: None,
        })
        .expect("record reversible effect");

    let outcome = rollout.rollback().expect("rollback succeeds");
    assert_eq!(outcome.restored.revision(), 1);
    // Both irreversible effects are named as not undone; only the one
    // that declared a compensating operation appears in the
    // compensations list. The reversible effect is reported separately.
    let irreversible_ids: Vec<&str> = outcome
        .irreversible_effects
        .iter()
        .map(|effect| effect.effect_id.as_str())
        .collect();
    assert_eq!(irreversible_ids, ["announce-1", "mint-1"]);
    assert_eq!(
        outcome.compensations_required,
        ["send a signed retraction".to_owned()]
    );
    let reversible_ids: Vec<&str> = outcome
        .reversible_effects
        .iter()
        .map(|effect| effect.effect_id.as_str())
        .collect();
    assert_eq!(reversible_ids, ["cache-1"]);
}

#[test]
fn effects_are_charged_to_the_revision_that_was_active() {
    let mut rollout = Rollout::new();
    activate(&mut rollout, 1);
    rollout
        .record_effect(ExternalEffect {
            effect_id: "under-rev-1".to_owned(),
            description: "effect while revision 1 was active".to_owned(),
            reversible: false,
            compensating_operation: None,
        })
        .expect("record under rev 1");
    activate(&mut rollout, 2);

    // Rolling back from rev 2 reports only rev 2's effects (none); the
    // rev 1 effect belongs to the revision being restored, not the one
    // being superseded.
    let outcome = rollout.rollback().expect("rollback succeeds");
    assert!(outcome.irreversible_effects.is_empty());
    assert!(outcome.reversible_effects.is_empty());
    assert!(outcome.compensations_required.is_empty());
}

#[test]
fn staging_requires_advancing_revision_of_the_same_config() {
    let mut rollout = Rollout::new();
    activate(&mut rollout, 2);

    let error = rollout
        .stage(&document(2), &capable_set())
        .expect_err("re-staging the active revision must reject");
    assert!(matches!(
        error,
        RolloutError::RevisionNotAdvanced {
            active: 2,
            candidate: 2
        }
    ));
    let error = rollout
        .stage(&document(1), &capable_set())
        .expect_err("staging an older revision must reject");
    assert!(matches!(
        error,
        RolloutError::RevisionNotAdvanced {
            active: 2,
            candidate: 1
        }
    ));

    let mut other = document(3);
    other.config_id = "other-config".to_owned();
    let error = rollout
        .stage(&other, &capable_set())
        .expect_err("a different config identity must reject");
    assert!(matches!(error, RolloutError::ConfigIdMismatch { .. }));
    // None of the rejected stagings disturbed the active revision.
    assert_eq!(rollout.active().expect("active").revision(), 2);
    assert_eq!(rollout.staged(), None);
}

#[test]
fn state_machine_guards() {
    let mut rollout = Rollout::new();
    assert!(matches!(rollout.commit(), Err(RolloutError::NothingStaged)));
    assert!(matches!(
        rollout.rollback(),
        Err(RolloutError::NoPreviousRevision)
    ));
    assert!(matches!(
        rollout.record_effect(ExternalEffect {
            effect_id: "orphan".to_owned(),
            description: "no revision is active".to_owned(),
            reversible: true,
            compensating_operation: None,
        }),
        Err(RolloutError::NoActiveRevision)
    ));

    // Double staging is rejected; aborting clears the stage.
    rollout.stage(&document(1), &capable_set()).expect("stage");
    let error = rollout
        .stage(&document(1), &capable_set())
        .expect_err("second stage must reject");
    assert!(matches!(error, RolloutError::AlreadyStaged { revision: 1 }));
    assert_eq!(
        rollout.abort_staged().map(|record| record.revision()),
        Some(1)
    );
    assert_eq!(rollout.staged(), None);

    // One activation with no predecessor still has nothing to roll back to.
    activate(&mut rollout, 1);
    assert!(matches!(
        rollout.rollback(),
        Err(RolloutError::NoPreviousRevision)
    ));

    // Duplicate effect ids under one revision are rejected.
    let effect = ExternalEffect {
        effect_id: "dup".to_owned(),
        description: "recorded twice".to_owned(),
        reversible: true,
        compensating_operation: None,
    };
    rollout.record_effect(effect.clone()).expect("first record");
    assert!(matches!(
        rollout.record_effect(effect),
        Err(RolloutError::DuplicateEffect(_))
    ));
}

#[test]
fn digest_is_deterministic_and_revision_sensitive() {
    let mut rollout = Rollout::new();
    let first = rollout
        .stage(&document(7), &capable_set())
        .expect("stage")
        .digest()
        .to_owned();
    rollout.abort_staged();
    let second = rollout
        .stage(&document(7), &capable_set())
        .expect("stage again")
        .digest()
        .to_owned();
    assert_eq!(first, second);
    rollout.abort_staged();
    let other = rollout
        .stage(&document(8), &capable_set())
        .expect("stage different revision")
        .digest()
        .to_owned();
    assert_ne!(first, other);
}
