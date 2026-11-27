//! Per-function Wasm instances behind the publish/swap state machine
//! (design #5964 §3.1 to §3.3, §4).
//!
//! [`FunctionHost`] is the host half of [`FunctionRegistry`]: it owns one
//! [`WorldInstance`] per active function version and at most one per
//! pending candidate, does the real instantiation, init and state handoff
//! through `sandbox.rs`, and reports the outcomes to the registry it owns.
//! All registry changes go through the host, so the instances it holds
//! always match the registry's active and pending versions.
//!
//! - [`FunctionHost::publish`] checks the candidate against the binding
//!   expectation first; a refused candidate is recorded and never
//!   instantiated. Otherwise the module is compiled and instantiated fresh
//!   and its init runs (§3.1 step 1). A compile, instantiation or init
//!   failure is reported as [`InstantiateOutcome::Trapped`]; on success
//!   the fresh instance is kept as the pending candidate's instance,
//!   replacing (and dropping) the instance of any candidate it superseded.
//! - [`FunctionHost::tick_boundary`] promotes the candidate (§3.2). When a
//!   version is already active, the host snapshots the old instance
//!   (`plaza_snapshot`) and restores the blob into the candidate
//!   (`plaza_restore`, §3.3). On activation the candidate instance becomes
//!   the active one and the old instance is dropped; a failed snapshot or
//!   restore rejects the swap, the candidate instance is dropped and the
//!   old instance stays active with its state.
//! - An old version that exports no snapshot/restore pair holds no state
//!   the host can hand over, so the candidate keeps its fresh init. A
//!   candidate that cannot restore a blob the old version produced is
//!   rejected: state is never dropped silently.
//!
//! Isolation (§4): every function version runs in its own Wasmtime store,
//! built from the artifact's own module bytes under the host's per-function
//! [`SandboxLimits`], so each instance has its own fuel budget and memory
//! limit per guest call; no function shares a store or a fuel pool with
//! another function.
//!
//! Not here, and arriving with later slices: crash recovery after a
//! trapping call (the sandbox already poisons an instance whose call
//! trapped; re-instantiating from the last good version
//! and its last good snapshot is §4's next step), draining in-flight calls
//! before the old instance is dropped (calls are synchronous here, so none
//! are in flight at a boundary), per-function limits and the world's
//! per-tick ceiling over them (#5962), and persistence.

use std::collections::BTreeMap;

use crate::sandbox::{Sandbox, SandboxLimits, WorldInstance};
use crate::world_compile::{FunctionArtifact, FunctionBinding, check_activation};
use crate::world_functions::{
    BoundaryOutcome, FunctionRegistry, InstantiateOutcome, PublishAccepted, RejectionReason,
    RestoreOutcome, Swap, Version,
};

/// The instances held for one (user, binding name).
#[derive(Debug, Default)]
struct Instances {
    /// The active version and its instance.
    active: Option<(Version, WorldInstance)>,
    /// The pending candidate and its freshly initialized instance.
    pending: Option<(Version, WorldInstance)>,
}

/// Per-function instances driving a [`FunctionRegistry`] with real
/// outcomes (design #5964 §3, §4).
#[derive(Debug)]
pub struct FunctionHost {
    limits: SandboxLimits,
    registry: FunctionRegistry,
    instances: BTreeMap<String, BTreeMap<String, Instances>>,
}

impl FunctionHost {
    /// An empty host whose function instances each run under `limits`.
    ///
    /// The limits apply per instance and per guest call: every function
    /// version gets its own store, so one function's fuel and memory are
    /// never another's.
    #[must_use]
    pub const fn new(limits: SandboxLimits) -> Self {
        Self {
            limits,
            registry: FunctionRegistry::new(),
            instances: BTreeMap::new(),
        }
    }

    /// The per-function limits every new instance runs under.
    #[must_use]
    pub const fn limits(&self) -> SandboxLimits {
        self.limits
    }

    /// The publish/swap state the host drives.
    #[must_use]
    pub const fn registry(&self) -> &FunctionRegistry {
        &self.registry
    }

    fn slot(&self, user: &str, name: &str) -> Option<&Instances> {
        self.instances.get(user)?.get(name)
    }

    fn slot_mut(&mut self, user: &str, name: &str) -> &mut Instances {
        self.instances
            .entry(user.to_owned())
            .or_default()
            .entry(name.to_owned())
            .or_default()
    }

    /// Record a new binding expectation for `user` and return its version.
    ///
    /// Forwards to [`FunctionRegistry::set_expectation`]; when the new
    /// expectation drops the pending candidate, its instance is dropped
    /// with it.
    pub fn set_expectation(&mut self, user: &str, binding: FunctionBinding) -> Version {
        let name = binding.function.clone();
        let version = self.registry.set_expectation(user, binding);
        let pending = self
            .registry
            .pending(user, &name)
            .map(|candidate| candidate.version);
        if let Some(slot) = self
            .instances
            .get_mut(user)
            .and_then(|names| names.get_mut(&name))
            && slot.pending.as_ref().map(|(version, _)| *version) != pending
        {
            slot.pending = None;
        }
        version
    }

    /// Publish a candidate on (user, name) and keep its fresh instance as
    /// the pending candidate's (§3.1).
    ///
    /// A candidate the binding expectation refuses is recorded and never
    /// instantiated. Otherwise the module is compiled under the host's
    /// per-function limits, instantiated with `seed`, and its init runs;
    /// the outcome goes to [`FunctionRegistry::publish`]. On success the
    /// instance becomes the pending candidate's, replacing the instance of
    /// any candidate it superseded.
    ///
    /// # Errors
    ///
    /// The registry's [`RejectionReason`]: no expectation, an activation
    /// refusal, or [`RejectionReason::InitTrap`] carrying the compile,
    /// instantiation or init failure. The fresh instance, if any, is
    /// dropped, and the active and pending instances are untouched.
    pub fn publish(
        &mut self,
        user: &str,
        name: &str,
        artifact: &FunctionArtifact,
        seed: u64,
    ) -> Result<PublishAccepted, RejectionReason> {
        let admitted = self
            .registry
            .expectation(user, name)
            .is_some_and(|current| check_activation(&current.binding, artifact).is_ok());
        if !admitted {
            // The registry runs the same expectation and activation check
            // before it reads the outcome, so it records the refusal with
            // its precise reason; the outcome only guarantees the candidate
            // can never be accepted without an instance.
            let skipped = InstantiateOutcome::Trapped {
                message: "not instantiated: the binding expectation refused the candidate"
                    .to_owned(),
            };
            return self.registry.publish(user, name, artifact, skipped);
        }
        let (outcome, instance) = match self.instantiate(artifact, seed) {
            Ok(instance) => (InstantiateOutcome::Ready, Some(instance)),
            Err(error) => (
                InstantiateOutcome::Trapped {
                    message: format!("{error:#}"),
                },
                None,
            ),
        };
        let accepted = self.registry.publish(user, name, artifact, outcome)?;
        self.slot_mut(user, name).pending = instance.map(|instance| (accepted.version, instance));
        Ok(accepted)
    }

    /// Compile `artifact`'s module under the per-function limits and
    /// instantiate it fresh, running its init.
    fn instantiate(&self, artifact: &FunctionArtifact, seed: u64) -> anyhow::Result<WorldInstance> {
        Sandbox::compile(&artifact.module, self.limits)?.instantiate(seed)
    }

    /// Run the tick boundary for (user, name) with the real state handoff
    /// (§3.2, §3.3).
    ///
    /// When a version is already active, its instance is snapshotted and
    /// the blob restored into the candidate's instance. On activation the
    /// candidate instance becomes the active one and the old instance is
    /// dropped; on a rejected swap the candidate instance is dropped and
    /// the old instance stays active with its state.
    #[must_use]
    pub fn tick_boundary(&mut self, user: &str, name: &str) -> BoundaryOutcome {
        let Self {
            registry,
            instances,
            ..
        } = self;
        let mut unknown = Instances::default();
        let Instances { active, pending } = instances
            .get_mut(user)
            .and_then(|names| names.get_mut(name))
            .unwrap_or(&mut unknown);
        let mut candidate = pending.take();
        let outcome = registry.tick_boundary(user, name, |swap| {
            hand_over(swap, active.as_mut(), candidate.as_mut())
        });
        if matches!(outcome, BoundaryOutcome::Activated { .. }) {
            *active = candidate;
        }
        outcome
    }

    /// Call the active version of (user, name) with `input` and return its
    /// rendered output. A pending candidate is not callable until a tick
    /// boundary activates it. A guest trap is returned by the sandbox and
    /// poisons only this function instance.
    ///
    /// # Errors
    ///
    /// Returns an error when no active instance exists or when the guest
    /// update or view traps or otherwise fails.
    pub fn call(&mut self, user: &str, name: &str, input: &[u8]) -> anyhow::Result<Vec<u8>> {
        let instance = self
            .active_instance_mut(user, name)
            .ok_or_else(|| anyhow::anyhow!("no active function instance for ({user}, {name})"))?;
        instance.update(0, input)?;
        instance.view()
    }

    /// The active version's instance of (user, name), if any.
    #[must_use]
    pub fn active_instance(&self, user: &str, name: &str) -> Option<&WorldInstance> {
        self.slot(user, name)?
            .active
            .as_ref()
            .map(|(_, instance)| instance)
    }

    /// The active version's instance of (user, name), mutably: the seam a
    /// later call path drives.
    #[must_use]
    pub fn active_instance_mut(&mut self, user: &str, name: &str) -> Option<&mut WorldInstance> {
        self.instances
            .get_mut(user)?
            .get_mut(name)?
            .active
            .as_mut()
            .map(|(_, instance)| instance)
    }

    /// The pending candidate's instance of (user, name), if any.
    #[must_use]
    pub fn pending_instance(&self, user: &str, name: &str) -> Option<&WorldInstance> {
        self.slot(user, name)?
            .pending
            .as_ref()
            .map(|(_, instance)| instance)
    }
}

/// Snapshot `from`'s instance and restore the blob into `to`'s (§3.3).
fn hand_over(
    swap: Swap<'_>,
    from: Option<&mut (Version, WorldInstance)>,
    to: Option<&mut (Version, WorldInstance)>,
) -> RestoreOutcome {
    let (Some((from_version, old)), Some((to_version, fresh))) = (from, to) else {
        return RestoreOutcome::Failed {
            message: format!(
                "the host holds no instance for version {} or version {}",
                swap.from.version, swap.to.version
            ),
        };
    };
    if *from_version != swap.from.version || *to_version != swap.to.version {
        return RestoreOutcome::Failed {
            message: format!(
                "the host holds versions {from_version} and {to_version}, the swap moves {} to {}",
                swap.from.version, swap.to.version
            ),
        };
    }
    match old.snapshot() {
        // No snapshot/restore pair on the old version: it holds no state
        // the host can hand over, so the candidate keeps its fresh init.
        Ok(None) => RestoreOutcome::Restored,
        Ok(Some(state)) => match fresh.restore(&state) {
            Ok(()) => RestoreOutcome::Restored,
            Err(error) => RestoreOutcome::Failed {
                message: format!("restore into version {} failed: {error:#}", swap.to.version),
            },
        },
        Err(error) => RestoreOutcome::Failed {
            message: format!(
                "snapshot of version {} failed: {error:#}",
                swap.from.version
            ),
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::sandbox::GuestCall;
    use crate::world::ModuleProvenance;
    use crate::world_compile::{ActivationRefusal, FunctionInterface, FunctionShape};

    const USER: &str = "alice";
    const NAME: &str = "counter";

    fn interface(state_schema: &str) -> FunctionInterface {
        FunctionInterface::new(
            NAME.to_owned(),
            FunctionShape {
                input: "Str".to_owned(),
                output: "U32".to_owned(),
                state_schema: state_schema.to_owned(),
            },
            vec![],
        )
    }

    /// How a counter guest misbehaves, if at all.
    #[derive(Clone, Copy)]
    enum Fault {
        Healthy,
        InitTraps,
        RestoreTraps,
        UpdateTraps,
        UseInputByte,
    }

    /// A counter guest built without the Roc compiler: the model is a
    /// little-endian `u32` at address 16, init stores the seed, every
    /// update adds `step`, and snapshot/restore move the 4-byte counter.
    fn counter(step: u32, fault: Fault) -> Vec<u8> {
        let init_fault = if matches!(fault, Fault::InitTraps) {
            "unreachable"
        } else {
            ""
        };
        let restore_fault = if matches!(fault, Fault::RestoreTraps) {
            "unreachable"
        } else {
            ""
        };
        let update_fault = if matches!(fault, Fault::UpdateTraps) {
            "unreachable"
        } else {
            ""
        };
        let input_step = if matches!(fault, Fault::UseInputByte) {
            "(i32.load8_u (local.get $input))"
        } else {
            "(i32.const 0)"
        };
        let wat = format!(
            r#"
            (module
              (memory (export "memory") 1 1)
              (global $out (mut i32) (i32.const 0))
              (global $heap (mut i32) (i32.const 4096))
              (func (export "plaza_alloc") (param $n i32) (result i32)
                (local $p i32)
                (local.set $p (global.get $heap))
                (global.set $heap (i32.add (global.get $heap) (local.get $n)))
                (local.get $p))
              (func (export "plaza_free") (param i32 i32))
              (func (export "plaza_init") (param $seed i64) (result i32)
                {init_fault}
                (i32.store (i32.const 16) (i32.wrap_i64 (local.get $seed)))
                (i32.const 16))
              (func (export "plaza_update") (param $m i32) (param i32) (param $input i32) (param i32) (result i32)
                {update_fault}
                (i32.store (local.get $m)
                  (i32.add (i32.load (local.get $m))
                    (i32.add (i32.const {step}) {input_step})))
                (local.get $m))
              (func (export "plaza_view") (param $m i32) (result i32)
                (global.set $out (i32.const 4))
                (local.get $m))
              (func (export "plaza_out_len") (result i32) (global.get $out))
              (func (export "plaza_snapshot") (param $m i32) (result i32)
                (global.set $out (i32.const 4))
                (local.get $m))
              (func (export "plaza_restore") (param $ptr i32) (param $len i32) (result i32)
                {restore_fault}
                (if (i32.ne (local.get $len) (i32.const 4)) (then unreachable))
                (i32.store (i32.const 16) (i32.load (local.get $ptr)))
                (i32.const 16)))
            "#
        );
        wat::parse_str(wat).unwrap()
    }

    /// A candidate artifact under `interface`, built without the compiler.
    fn artifact(interface: FunctionInterface, module: Vec<u8>) -> FunctionArtifact {
        FunctionArtifact {
            function: interface.function().to_owned(),
            module_hash: crate::world_cache::artifact_hash(&module),
            module,
            diagnostics: String::new(),
            provenance: ModuleProvenance::Artifact,
            interface_hash: interface.interface_hash(),
            interface,
        }
    }

    fn host_expecting(interface: &FunctionInterface) -> FunctionHost {
        let mut host = FunctionHost::new(SandboxLimits::default());
        assert_eq!(
            host.set_expectation(USER, FunctionBinding::expecting(interface)),
            1
        );
        host
    }

    /// The active instance's counter, read through its state snapshot.
    fn state(host: &mut FunctionHost) -> u32 {
        let blob = host
            .active_instance_mut(USER, NAME)
            .expect("an active instance")
            .snapshot()
            .unwrap()
            .expect("the counter exports a snapshot");
        u32::from_le_bytes(blob.try_into().expect("a 4-byte counter"))
    }

    fn bump(host: &mut FunctionHost, times: usize) {
        let instance = host.active_instance_mut(USER, NAME).unwrap();
        for _ in 0..times {
            instance.update(0, b"tick").unwrap();
        }
    }

    /// A host with version 1 of the counter (step 1, seed 0) active and
    /// bumped to `value`.
    fn running(value: usize) -> FunctionHost {
        let v1 = interface("counter-u32");
        let mut host = host_expecting(&v1);
        host.publish(USER, NAME, &artifact(v1, counter(1, Fault::Healthy)), 0)
            .unwrap();
        assert_eq!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::Activated {
                version: 1,
                previous: None
            }
        );
        bump(&mut host, value);
        host
    }

    #[test]
    fn first_publish_activates_without_restore() {
        // Oracle: design #5964 §3.1, §3.3 — the candidate is instantiated
        // fresh and its init runs at publish; it becomes active at the next
        // tick boundary, and a first activation has no old instance and so
        // no state restore.
        let v1 = interface("counter-u32");
        let mut host = host_expecting(&v1);
        let accepted = host
            .publish(USER, NAME, &artifact(v1, counter(1, Fault::Healthy)), 5)
            .unwrap();
        assert_eq!(accepted.version, 1);
        assert!(host.pending_instance(USER, NAME).is_some());
        assert!(host.active_instance(USER, NAME).is_none());
        assert!(host.registry().resolve(USER, NAME).is_none());
        assert_eq!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::Activated {
                version: 1,
                previous: None
            }
        );
        assert!(host.pending_instance(USER, NAME).is_none());
        let active = host.active_instance(USER, NAME).unwrap();
        assert_eq!(active.metrics().get(GuestCall::Init).calls, 1);
        assert_eq!(active.metrics().get(GuestCall::Restore).calls, 0);
        assert_eq!(state(&mut host), 5);
        assert_eq!(host.registry().resolve(USER, NAME).unwrap().version, 1);
        assert_eq!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::NothingPending
        );
    }

    #[test]
    fn call_requires_an_active_instance_and_returns_its_view() {
        // Oracle: design #5964 §1.1 and §3.1 — calls go to the active
        // mapping, and a published candidate is invisible until the tick
        // boundary. The guest adds its one-byte input to its state and the
        // host returns the guest's rendered state.
        let v1 = interface("counter-u32");
        let mut host = host_expecting(&v1);
        assert_eq!(
            host.call(USER, NAME, b"A").unwrap_err().to_string(),
            "no active function instance for (alice, counter)"
        );
        host.publish(
            USER,
            NAME,
            &artifact(v1, counter(1, Fault::UseInputByte)),
            7,
        )
        .unwrap();
        assert_eq!(
            host.call(USER, NAME, b"A").unwrap_err().to_string(),
            "no active function instance for (alice, counter)"
        );
        assert!(matches!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::Activated { version: 1, .. }
        ));
        assert_eq!(host.call(USER, NAME, b"A").unwrap(), 73_u32.to_le_bytes());
    }

    #[test]
    fn trapping_call_is_contained_to_its_user_instance() {
        // Oracle: design #5964 §4 — a trapping function loses its call and
        // poisons only its own instance; another user's active instance can
        // still answer calls.
        let v1 = interface("counter-u32");
        let mut host = host_expecting(&v1);
        host.set_expectation("bob", FunctionBinding::expecting(&v1));
        host.publish(
            USER,
            NAME,
            &artifact(v1.clone(), counter(1, Fault::UpdateTraps)),
            3,
        )
        .unwrap();
        host.publish(
            "bob",
            NAME,
            &artifact(v1, counter(1, Fault::Healthy)),
            3,
        )
        .unwrap();
        assert!(matches!(host.tick_boundary(USER, NAME), BoundaryOutcome::Activated { .. }));
        assert!(matches!(host.tick_boundary("bob", NAME), BoundaryOutcome::Activated { .. }));

        let error = host.call(USER, NAME, b"event").unwrap_err().to_string();
        assert!(error.contains("plaza_update"), "{error}");
        assert!(host.call("bob", NAME, b"event").is_ok());
        assert!(host
            .call(USER, NAME, b"event")
            .unwrap_err()
            .to_string()
            .contains("poisoned"));
        assert_eq!(host.registry().resolve(USER, NAME).unwrap().version, 1);
        assert_eq!(host.registry().resolve("bob", NAME).unwrap().version, 1);
    }

    #[test]
    fn new_version_carries_state_across_the_boundary() {
        // Oracle: design #5964 §3.3 — for a new version of the same
        // function the host snapshots the old instance's state and
        // restores it into the fresh instance; from the boundary on the
        // mapping resolves to the new version's instance.
        let mut host = running(3);
        let v1 = interface("counter-u32");
        let accepted = host
            .publish(USER, NAME, &artifact(v1, counter(10, Fault::Healthy)), 0)
            .unwrap();
        assert_eq!(accepted.version, 2);
        // Before the boundary the old instance keeps serving its state.
        assert_eq!(state(&mut host), 3);
        assert_eq!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::Activated {
                version: 2,
                previous: Some(1)
            }
        );
        assert_eq!(
            host.active_instance(USER, NAME)
                .unwrap()
                .metrics()
                .get(GuestCall::Restore)
                .calls,
            1
        );
        assert_eq!(state(&mut host), 3);
        // The new version's code runs on the carried state.
        bump(&mut host, 1);
        assert_eq!(state(&mut host), 13);
        assert_eq!(host.registry().resolve(USER, NAME).unwrap().version, 2);
        assert!(host.pending_instance(USER, NAME).is_none());
    }

    #[test]
    fn init_trap_is_rejected_and_nothing_changes() {
        // Oracle: design #5964 §3.1 step 3, §4 — an instantiation or init
        // trap discards the candidate, the previously published version
        // keeps running with its state, and the rejection is recorded.
        let mut host = running(2);
        let v1 = interface("counter-u32");
        let reason = host
            .publish(USER, NAME, &artifact(v1, counter(10, Fault::InitTraps)), 0)
            .unwrap_err();
        let RejectionReason::InitTrap { message } = &reason else {
            panic!("expected an init trap, got {reason:?}");
        };
        assert!(message.contains("plaza_init"), "{message}");
        assert!(host.pending_instance(USER, NAME).is_none());
        assert!(host.registry().pending(USER, NAME).is_none());
        assert_eq!(host.registry().resolve(USER, NAME).unwrap().version, 1);
        assert_eq!(host.registry().rejections(USER, NAME).len(), 1);
        assert_eq!(state(&mut host), 2);
        assert_eq!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::NothingPending
        );
        assert_eq!(state(&mut host), 2);
    }

    #[test]
    fn failed_restore_rejects_the_swap_and_keeps_the_old_instance() {
        // Oracle: design #5964 §3.3 — a failed restore rejects the swap:
        // the old version continues with its state intact, and the
        // rejection is recorded.
        let mut host = running(4);
        let v1 = interface("counter-u32");
        let accepted = host
            .publish(
                USER,
                NAME,
                &artifact(v1, counter(10, Fault::RestoreTraps)),
                0,
            )
            .unwrap();
        assert_eq!(accepted.version, 2);
        let outcome = host.tick_boundary(USER, NAME);
        let BoundaryOutcome::Rejected {
            version: 2,
            reason: RejectionReason::RestoreFailed { message },
        } = &outcome
        else {
            panic!("expected a rejected swap, got {outcome:?}");
        };
        assert!(message.contains("plaza_restore"), "{message}");
        assert!(host.pending_instance(USER, NAME).is_none());
        assert_eq!(host.registry().resolve(USER, NAME).unwrap().version, 1);
        assert_eq!(host.registry().rejections(USER, NAME)[0].version, Some(2));
        // The old instance still holds its state and still runs.
        assert_eq!(state(&mut host), 4);
        bump(&mut host, 1);
        assert_eq!(state(&mut host), 5);
        assert_eq!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::NothingPending
        );
    }

    #[test]
    fn interface_mismatch_is_refused_and_never_instantiated() {
        // Oracle: design #5964 §1.3, §3.1 — a candidate whose interface
        // hash differs from the binding expectation is not activated and
        // the published version keeps running; the refusal comes before
        // any instance exists.
        let mut host = running(1);
        let v1 = interface("counter-u32");
        let v2 = interface("counter-u64");
        // Init would trap if the module were ever instantiated.
        let reason = host
            .publish(
                USER,
                NAME,
                &artifact(v2.clone(), counter(10, Fault::InitTraps)),
                0,
            )
            .unwrap_err();
        assert_eq!(
            reason,
            RejectionReason::Refused(ActivationRefusal::InterfaceMismatch {
                function: NAME.to_owned(),
                expected: v1.interface_hash(),
                candidate: v2.interface_hash(),
            })
        );
        assert!(host.pending_instance(USER, NAME).is_none());
        assert_eq!(host.registry().resolve(USER, NAME).unwrap().version, 1);
        assert_eq!(state(&mut host), 1);
        // With no expectation at all, nothing is instantiated either.
        let mut empty = FunctionHost::new(SandboxLimits::default());
        let reason = empty
            .publish(USER, NAME, &artifact(v1, counter(1, Fault::Healthy)), 0)
            .unwrap_err();
        assert!(matches!(reason, RejectionReason::NoExpectation { .. }));
        assert!(empty.pending_instance(USER, NAME).is_none());
    }

    #[test]
    fn superseded_and_stale_candidates_drop_their_instances() {
        // Oracle: design #5964 §1.3, §3.1 — only one candidate waits for
        // the boundary, and a new expectation that no longer admits the
        // pending candidate drops it; a dropped candidate's instance goes
        // with it and never activates.
        let v1 = interface("counter-u32");
        let mut host = host_expecting(&v1);
        host.publish(
            USER,
            NAME,
            &artifact(v1.clone(), counter(1, Fault::Healthy)),
            1,
        )
        .unwrap();
        let accepted = host
            .publish(
                USER,
                NAME,
                &artifact(v1.clone(), counter(2, Fault::Healthy)),
                2,
            )
            .unwrap();
        assert_eq!(accepted.superseded, Some(1));
        assert_eq!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::Activated {
                version: 2,
                previous: None
            }
        );
        assert_eq!(state(&mut host), 2);
        host.publish(USER, NAME, &artifact(v1, counter(3, Fault::Healthy)), 0)
            .unwrap();
        assert!(host.pending_instance(USER, NAME).is_some());
        let v2 = interface("counter-u64");
        assert_eq!(
            host.set_expectation(USER, FunctionBinding::expecting(&v2)),
            2
        );
        assert!(host.pending_instance(USER, NAME).is_none());
        assert_eq!(
            host.tick_boundary(USER, NAME),
            BoundaryOutcome::NothingPending
        );
        assert_eq!(state(&mut host), 2);
    }
}
