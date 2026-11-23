//! Reference-oracle tests for slice 9111-02 (#10193).
//!
//! The independent oracle applies the documented layer order by hand and
//! checks that the resolver agrees while preserving authority and resource
//! ceilings.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use id_config::{
    Behavior, CeilingOverride, Ceilings, LayerOverride, Mode, ResolutionInput, ResolveError,
    SyncScope, Tier, resolve,
};

fn ceilings(max_hops: u32) -> Ceilings {
    Ceilings {
        max_hops,
        max_bytes: 1_000,
        max_time_ms: 1_000,
        max_fan_out: 10,
    }
}

fn layer(name: &str, behavior: Behavior) -> LayerOverride {
    LayerOverride {
        name: name.to_owned(),
        behavior,
        ceilings: CeilingOverride::default(),
    }
}

fn base() -> ResolutionInput {
    ResolutionInput {
        deployment: Behavior {
            max_hops: Some(2),
            sync_scope: Some(SyncScope::Metadata),
            tier: Some(Tier::Cold),
        },
        mode: Mode::HostedTenant,
        tenant: None,
        peer: None,
        link: None,
        storage: None,
        operation: None,
        resource_ceiling: ceilings(5),
        authority_sync_scopes: vec![SyncScope::Metadata],
    }
}

#[test]
fn later_scopes_replace_behavior_but_never_widen_resources() {
    let mut input = base();
    input.tenant = Some(layer(
        "tenant:alpha",
        Behavior {
            max_hops: Some(3),
            sync_scope: Some(SyncScope::Metadata),
            tier: Some(Tier::Warm),
        },
    ));
    input.peer = Some(layer(
        "peer:beta",
        Behavior {
            max_hops: Some(1),
            sync_scope: None,
            tier: None,
        },
    ));
    input.storage = Some(layer(
        "storage:archive",
        Behavior {
            max_hops: None,
            sync_scope: None,
            tier: Some(Tier::Hot),
        },
    ));

    let resolved = resolve(&input).unwrap();
    // Independent reference: mode defaults replace deployment, tenant then
    // replaces tier, peer narrows hops, and storage replaces tier last.
    assert_eq!(resolved.behavior.max_hops, 1);
    assert_eq!(resolved.behavior.sync_scope, SyncScope::Metadata);
    assert_eq!(resolved.behavior.tier, Tier::Hot);
    assert_eq!(resolved.provenance["max_hops"], "override:peer:beta");
    assert_eq!(resolved.provenance["tier"], "override:storage:archive");
    assert_eq!(resolved.ceilings.max_hops, 5);
}

#[test]
fn behavioral_values_can_loosen_inside_authority_and_resource_ceilings() {
    let mut input = base();
    input.authority_sync_scopes = vec![SyncScope::Metadata, SyncScope::Full];
    input.operation = Some(layer(
        "operation:sync",
        Behavior {
            max_hops: Some(4),
            sync_scope: Some(SyncScope::Full),
            tier: Some(Tier::Hot),
        },
    ));

    let resolved = resolve(&input).unwrap();
    assert_eq!(resolved.behavior.max_hops, 4);
    assert_eq!(resolved.behavior.sync_scope, SyncScope::Full);
    assert_eq!(resolved.behavior.tier, Tier::Hot);
    assert_eq!(resolved.provenance["sync_scope"], "override:operation:sync");
}

#[test]
fn authority_ceiling_rejects_a_final_out_of_scope_override() {
    let mut input = base();
    input.operation = Some(layer(
        "operation:sync",
        Behavior {
            max_hops: None,
            sync_scope: Some(SyncScope::Full),
            tier: None,
        },
    ));

    assert!(matches!(
        resolve(&input),
        Err(ResolveError::AuthorityExceeded {
            field: "sync_scope",
            ..
        })
    ));
}

#[test]
fn a_scope_cannot_widen_the_current_resource_ceiling() {
    let mut input = base();
    input.peer = Some(LayerOverride {
        name: "peer:beta".to_owned(),
        behavior: Behavior::default(),
        ceilings: CeilingOverride {
            max_hops: Some(6),
            ..CeilingOverride::default()
        },
    });

    assert!(matches!(
        resolve(&input),
        Err(ResolveError::CeilingWidened {
            field: "max_hops",
            ..
        })
    ));
}
