//! Deployment inventory and admission for residency-scoped worlds (#3398 design step 2,
//! issue #5529).
//!
//! Design: `doc/2026-11-01_design_id3398.md` §2 (storage/processing planes) and §7 step 2.
//! Step 1 ([`crate::residency`]) defined the policy vocabulary and the pure evaluator; this
//! module adds the deployment side step 2 names:
//!
//! - **Deployment configuration**: an [`Inventory`] records every resource that can retain
//!   or process scoped content — journal, blob store, replicas, backups, caches, outbox
//!   queues, telemetry stores and execution hosts — each bound to its [`Plane`].
//! - **Resource-bound evidence**: each record carries the [`Evidence`] offered for that
//!   exact resource, checked by the step-1 evaluator (trusted issuer, signature, revocation,
//!   freshness, binding, policy) at admission time.
//! - **Admission**: world create, open and restore, and managed work submission, evaluate
//!   the inventory before the world or the work is allowed to touch those resources.
//!
//! Two step-2 rules are enforced here, not left to callers:
//!
//! - **A category that is neither recorded nor explicitly declared absent is
//!   unsupported.** Unknown copies deny: a scoped world whose deployment has, say, no
//!   backup record and no declaration that backups do not exist is denied, because its
//!   placement cannot be established (§6: unsupported or unconfirmed copies are denied).
//! - **Unscoped content is untouched.** [`Scope::Unscoped`] admits exactly as before this
//!   step existed; the inventory gates scoped worlds only.
//!
//! Transport is not a deployment property: a world's lifecycle touches storage,
//! processing and metadata resources, so admission evaluates those three planes of the
//! caller's policy. Route policy for transport hops is steps 3–4 of the design.
//! Nothing here performs I/O or changes an existing route's behaviour; the enforcement
//! points call these functions before create/open/restore/submission, in the same pure
//! style as step 1.

use std::collections::{BTreeMap, BTreeSet};

use crate::residency::{
    Decision, DenyReason, Evidence, EvidenceVerifier, Hop, Plane, Policy, ResourceId, Scope,
    evaluate,
};

/// A class of deployment resource that can retain or process content. Every class maps
/// to exactly one residency plane; classes are the unit of the inventory's completeness
/// rule (a class is recorded, declared absent, or unsupported).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    /// The world's journal / event store (storage).
    Journal,
    /// Module and blob storage (storage).
    BlobStore,
    /// A replica of the journal or blobs (storage).
    Replica,
    /// Backups and recovery copies, including point-in-time recovery (storage).
    Backup,
    /// Caches that can retain content (storage).
    Cache,
    /// Queues and outboxes that can retain content in flight (storage).
    Outbox,
    /// Logs, traces, metrics and other telemetry stores (metadata).
    Telemetry,
    /// A host that executes requests, world turns or managed work (processing).
    ExecutionHost,
}

impl ResourceKind {
    /// Every resource class a deployment inventory accounts for.
    pub const ALL: [ResourceKind; 8] = [
        ResourceKind::Journal,
        ResourceKind::BlobStore,
        ResourceKind::Replica,
        ResourceKind::Backup,
        ResourceKind::Cache,
        ResourceKind::Outbox,
        ResourceKind::Telemetry,
        ResourceKind::ExecutionHost,
    ];

    /// The residency plane this resource class belongs to.
    pub const fn plane(self) -> Plane {
        match self {
            ResourceKind::Journal
            | ResourceKind::BlobStore
            | ResourceKind::Replica
            | ResourceKind::Backup
            | ResourceKind::Cache
            | ResourceKind::Outbox => Plane::Storage,
            ResourceKind::Telemetry => Plane::Metadata,
            ResourceKind::ExecutionHost => Plane::Processing,
        }
    }
}

/// One deployment resource, as configured by the operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRecord {
    /// The resource's identity, as named in policy allow-lists and evidence.
    pub id: ResourceId,
    /// What class of resource this is.
    pub kind: ResourceKind,
}

/// The deployment configuration for residency admission: the resource records, the
/// evidence bound to each resource, and the resource classes the operator explicitly
/// declares do not exist in this deployment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    /// Every resource that can retain or process scoped content in this deployment.
    pub resources: Vec<ResourceRecord>,
    /// Placement evidence, keyed by the resource it was issued for. A record with no
    /// entry here has no evidence and cannot admit a scoped world.
    pub evidence: BTreeMap<ResourceId, Evidence>,
    /// Resource classes the operator declares absent (e.g. "this deployment keeps no
    /// backups"). A class that is neither recorded nor declared absent is unsupported
    /// and denies scoped admission.
    pub declared_absent: BTreeSet<ResourceKind>,
}

impl Inventory {
    /// An empty inventory: nothing recorded, nothing declared absent.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one deployment resource.
    pub fn add_resource(mut self, id: &str, kind: ResourceKind) -> Self {
        self.resources.push(ResourceRecord {
            id: ResourceId(id.into()),
            kind,
        });
        self
    }

    /// Bind placement evidence to the resource it names.
    pub fn add_evidence(mut self, evidence: Evidence) -> Self {
        self.evidence.insert(evidence.resource.clone(), evidence);
        self
    }

    /// Declare that a resource class does not exist in this deployment.
    pub fn declare_absent(mut self, kind: ResourceKind) -> Self {
        self.declared_absent.insert(kind);
        self
    }

    /// The record for one resource, if the deployment has it.
    fn record(&self, id: &ResourceId) -> Option<&ResourceRecord> {
        self.resources.iter().find(|r| &r.id == id)
    }

    /// Resource classes whose placement cannot be established: neither recorded nor
    /// declared absent. Each maps to a `PlaneUnknown` denial for its plane — from the
    /// policy's point of view that plane's placement is unknown.
    fn unsupported(&self) -> Vec<DenyReason> {
        let mut reasons = Vec::new();
        for kind in ResourceKind::ALL {
            let recorded = self.resources.iter().any(|r| r.kind == kind);
            if !recorded && !self.declared_absent.contains(&kind) {
                let reason = DenyReason::PlaneUnknown(kind.plane());
                if !reasons.contains(&reason) {
                    reasons.push(reason);
                }
            }
        }
        reasons
    }

    /// One hop per recorded resource, carrying the evidence bound to it (or none).
    fn hops(&self) -> Vec<Hop<'_>> {
        self.resources
            .iter()
            .map(|r| Hop {
                plane: r.kind.plane(),
                resource: r.id.clone(),
                evidence: self.evidence.get(&r.id),
            })
            .collect()
    }
}

/// The planes a world-lifecycle admission evaluates: creating, opening or
/// restoring a world touches storage, processing and metadata resources.
/// Transport is a property of a path, not of a deployment; steps 3–4 evaluate it.
const WORLD_PLANES: [Plane; 3] = [Plane::Storage, Plane::Processing, Plane::Metadata];

/// The planes a work-submission admission evaluates: the execution host
/// (processing) and the journal the work is recorded in (storage).
const WORK_PLANES: [Plane; 2] = [Plane::Storage, Plane::Processing];

/// Evaluate admission for a scoped scope against a set of hops, under the given
/// planes of the scope's policy. Unscoped admits without consulting the inventory.
fn admit(
    scope: &Scope,
    planes: &[Plane],
    hops: &[Hop<'_>],
    extra_reasons: Vec<DenyReason>,
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> Decision {
    let Scope::Scoped(policy) = scope else {
        return Decision::Allow;
    };
    let mut policy: Policy = policy.clone();
    policy.required.retain(|p| planes.contains(p));
    let mut reasons = extra_reasons;
    if let Decision::Deny(mut found) = evaluate(&Scope::Scoped(policy), hops, now, verifier) {
        reasons.append(&mut found);
    }
    if reasons.is_empty() {
        Decision::Allow
    } else {
        Decision::Deny(reasons)
    }
}

/// Admit a world at one of its lifecycle points — create, open, or restore. All three
/// share one gate: every recorded deployment resource becomes a hop, every resource
/// class must be recorded or declared absent, and the step-1 evaluator checks the
/// evidence for each hop. A scoped world is denied when any of that fails; in
/// particular a restore onto a deployment whose backup evidence has expired is denied
/// rather than silently landing the recovered copy in an unestablished store.
fn admit_world(
    inventory: &Inventory,
    scope: &Scope,
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> Decision {
    let unsupported = inventory.unsupported();
    admit(
        scope,
        &WORLD_PLANES,
        &inventory.hops(),
        unsupported,
        now,
        verifier,
    )
}

/// Admit creating a scoped world on this deployment. See [`admit_world`].
pub fn admit_world_create(
    inventory: &Inventory,
    scope: &Scope,
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> Decision {
    admit_world(inventory, scope, now, verifier)
}

/// Admit opening an existing scoped world on this deployment. See [`admit_world`].
pub fn admit_world_open(
    inventory: &Inventory,
    scope: &Scope,
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> Decision {
    admit_world(inventory, scope, now, verifier)
}

/// Admit restoring a scoped world (backup/recovery) onto this deployment.
/// See [`admit_world`].
pub fn admit_world_restore(
    inventory: &Inventory,
    scope: &Scope,
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> Decision {
    admit_world(inventory, scope, now, verifier)
}

/// Admit submitting managed work to an execution host. A storage pin alone is
/// insufficient (design §2): the host itself must be a recorded execution host with
/// current, trusted processing evidence, and the journal the work will be recorded in
/// must admit on the storage plane. A host the deployment does not record is
/// unsupported — its placement cannot be established — and denies.
pub fn admit_work_submission(
    inventory: &Inventory,
    scope: &Scope,
    host: &ResourceId,
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> Decision {
    if matches!(scope, Scope::Unscoped) {
        return Decision::Allow;
    }
    let mut extra = Vec::new();
    let mut hops: Vec<Hop<'_>> = Vec::new();
    match inventory.record(host) {
        Some(record) if record.kind == ResourceKind::ExecutionHost => {
            hops.push(Hop {
                plane: Plane::Processing,
                resource: record.id.clone(),
                evidence: inventory.evidence.get(&record.id),
            });
        }
        _ => extra.push(DenyReason::PlaneUnknown(Plane::Processing)),
    }
    for record in inventory
        .resources
        .iter()
        .filter(|r| r.kind == ResourceKind::Journal)
    {
        hops.push(Hop {
            plane: Plane::Storage,
            resource: record.id.clone(),
            evidence: inventory.evidence.get(&record.id),
        });
    }
    admit(scope, &WORK_PLANES, &hops, extra, now, verifier)
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use crate::residency::{Jurisdiction, Policy};

    struct V {
        sig_ok: bool,
        revoked: bool,
    }
    impl EvidenceVerifier for V {
        fn signature_valid(&self, _: &Evidence) -> bool {
            self.sig_ok
        }
        fn revoked(&self, _: &Evidence) -> bool {
            self.revoked
        }
    }
    const OK: V = V {
        sig_ok: true,
        revoked: false,
    };
    const NOW: u64 = 50;

    /// A policy permitting the deployment's resources in "eu" on the three
    /// deployment planes; transport is required by the policy too, and must
    /// not block a deployment admission.
    fn policy() -> Policy {
        let mut p = Policy::new(["root".to_string()]);
        for (plane, resources) in [
            (Plane::Storage, &["journal", "blobs", "backup"][..]),
            (Plane::Processing, &["host"][..]),
            (Plane::Metadata, &["telemetry"][..]),
        ] {
            p = p
                .allow(plane, &["eu"], resources)
                .trust_evidence_source(plane, "control-plane");
        }
        p
    }

    fn ev(plane: Plane, res: &str) -> Evidence {
        Evidence {
            issuer: "root".into(),
            resource: ResourceId(res.into()),
            plane,
            jurisdiction: Jurisdiction("eu".into()),
            source: "control-plane".into(),
            expires_at: 100,
            serial: "1".into(),
            signature: vec![],
        }
    }

    /// A complete deployment: journal, blobs and backup on storage, a
    /// telemetry store, one execution host; replica/cache/outbox declared
    /// absent; current evidence bound to every record.
    fn inventory() -> Inventory {
        Inventory::new()
            .add_resource("journal", ResourceKind::Journal)
            .add_resource("blobs", ResourceKind::BlobStore)
            .add_resource("backup", ResourceKind::Backup)
            .add_resource("telemetry", ResourceKind::Telemetry)
            .add_resource("host", ResourceKind::ExecutionHost)
            .declare_absent(ResourceKind::Replica)
            .declare_absent(ResourceKind::Cache)
            .declare_absent(ResourceKind::Outbox)
            .add_evidence(ev(Plane::Storage, "journal"))
            .add_evidence(ev(Plane::Storage, "blobs"))
            .add_evidence(ev(Plane::Storage, "backup"))
            .add_evidence(ev(Plane::Metadata, "telemetry"))
            .add_evidence(ev(Plane::Processing, "host"))
    }

    fn scoped() -> Scope {
        Scope::Scoped(policy())
    }

    #[test]
    fn unscoped_world_admits_with_no_inventory() {
        let empty = Inventory::new();
        assert!(admit_world_create(&empty, &Scope::Unscoped, NOW, &OK).is_allow());
        assert!(admit_world_open(&empty, &Scope::Unscoped, NOW, &OK).is_allow());
        assert!(admit_world_restore(&empty, &Scope::Unscoped, NOW, &OK).is_allow());
        assert!(
            admit_work_submission(
                &empty,
                &Scope::Unscoped,
                &ResourceId("any".into()),
                NOW,
                &OK
            )
            .is_allow()
        );
    }

    #[test]
    fn complete_inventory_admits_create_open_restore() {
        let inv = inventory();
        assert!(admit_world_create(&inv, &scoped(), NOW, &OK).is_allow());
        assert!(admit_world_open(&inv, &scoped(), NOW, &OK).is_allow());
        assert!(admit_world_restore(&inv, &scoped(), NOW, &OK).is_allow());
    }

    #[test]
    fn missing_category_is_unsupported_and_denies() {
        // No backup record and no declaration that backups do not exist:
        // the deployment's recovery copies are unaccounted for.
        let mut inv = inventory();
        inv.resources.retain(|r| r.kind != ResourceKind::Backup);
        inv.evidence.remove(&ResourceId("backup".into()));
        let d = admit_world_open(&inv, &scoped(), NOW, &OK);
        assert!(!d.is_allow());
        let Decision::Deny(reasons) = d else { panic!() };
        assert!(reasons.contains(&DenyReason::PlaneUnknown(Plane::Storage)));
    }

    #[test]
    fn declared_absent_category_admits() {
        let mut inv = inventory();
        inv.resources.retain(|r| r.kind != ResourceKind::Backup);
        inv.evidence.remove(&ResourceId("backup".into()));
        inv.declared_absent.insert(ResourceKind::Backup);
        assert!(admit_world_open(&inv, &scoped(), NOW, &OK).is_allow());
    }

    #[test]
    fn recorded_resource_without_evidence_denies() {
        // A replica exists but has no placement evidence: an unknown copy.
        let inv = inventory().add_resource("replica-1", ResourceKind::Replica);
        let d = admit_world_open(&inv, &scoped(), NOW, &OK);
        let Decision::Deny(reasons) = d else { panic!() };
        assert!(reasons.contains(&DenyReason::NoEvidence(Plane::Storage)));
    }

    #[test]
    fn expired_backup_evidence_denies_restore() {
        let mut inv = inventory();
        inv.evidence
            .get_mut(&ResourceId("backup".into()))
            .unwrap()
            .expires_at = NOW;
        let d = admit_world_restore(&inv, &scoped(), NOW, &OK);
        let Decision::Deny(reasons) = d else { panic!() };
        assert!(reasons.contains(&DenyReason::Expired(Plane::Storage)));
    }

    #[test]
    fn evidence_bound_to_another_resource_denies() {
        let mut inv = inventory();
        inv.evidence
            .get_mut(&ResourceId("backup".into()))
            .unwrap()
            .resource = ResourceId("blobs".into());
        let d = admit_world_create(&inv, &scoped(), NOW, &OK);
        let Decision::Deny(reasons) = d else { panic!() };
        assert!(reasons.contains(&DenyReason::BindingMismatch(Plane::Storage)));
    }

    #[test]
    fn out_of_policy_jurisdiction_denies() {
        let mut inv = inventory();
        inv.evidence
            .get_mut(&ResourceId("telemetry".into()))
            .unwrap()
            .jurisdiction = Jurisdiction("us".into());
        let d = admit_world_open(&inv, &scoped(), NOW, &OK);
        let Decision::Deny(reasons) = d else { panic!() };
        assert!(reasons.contains(&DenyReason::OutOfPolicy(Plane::Metadata)));
    }

    #[test]
    fn work_submission_to_recorded_host_admits() {
        let inv = inventory();
        assert!(
            admit_work_submission(&inv, &scoped(), &ResourceId("host".into()), NOW, &OK).is_allow()
        );
    }

    #[test]
    fn work_submission_to_unknown_host_denies() {
        // The host is not in the deployment inventory at all: unsupported,
        // its placement cannot be established.
        let inv = inventory();
        let d = admit_work_submission(&inv, &scoped(), &ResourceId("ghost".into()), NOW, &OK);
        let Decision::Deny(reasons) = d else { panic!() };
        assert!(reasons.contains(&DenyReason::PlaneUnknown(Plane::Processing)));
    }

    #[test]
    fn work_submission_to_a_store_is_not_processing_evidence() {
        // A storage resource named as the host does not satisfy the
        // processing plane: a storage pin alone is insufficient.
        let inv = inventory();
        let d = admit_work_submission(&inv, &scoped(), &ResourceId("journal".into()), NOW, &OK);
        assert!(!d.is_allow());
    }

    #[test]
    fn work_submission_host_without_evidence_denies() {
        let mut inv = inventory();
        inv.evidence.remove(&ResourceId("host".into()));
        let d = admit_work_submission(&inv, &scoped(), &ResourceId("host".into()), NOW, &OK);
        let Decision::Deny(reasons) = d else { panic!() };
        assert!(reasons.contains(&DenyReason::NoEvidence(Plane::Processing)));
    }

    #[test]
    fn revoked_evidence_denies() {
        let inv = inventory();
        let v = V {
            sig_ok: true,
            revoked: true,
        };
        assert!(!admit_world_open(&inv, &scoped(), NOW, &v).is_allow());
    }
}
