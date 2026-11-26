//! Publish/swap bookkeeping for dynamic user functions (design #5964 §1.2,
//! §1.3, §3.1 to §3.3).
//!
//! [`FunctionRegistry`] is the pure, deterministic state machine behind
//! the per-user function mapping (#5967): per (user, binding name) it
//! keeps the versioned binding expectations, the published versions, the
//! active version, at most one pending candidate awaiting the next tick
//! boundary, and the log of rejected publishes and swaps. It holds no Wasm
//! instance, does no IO and reads no clock; the caller does the real work
//! and reports the outcome:
//!
//! - [`FunctionRegistry::publish`] takes a [`FunctionArtifact`] from the
//!   §2.2 pipeline and the caller's [`InstantiateOutcome`] (fresh
//!   instantiation plus init). It runs the §1.3 check through
//!   [`check_activation`] against the current expectation and, only when
//!   everything succeeded, assigns the next version and makes it the
//!   pending candidate (§3.1). Every failure is recorded as a
//!   [`Rejection`] and leaves the active and pending versions untouched.
//! - [`FunctionRegistry::tick_boundary`] promotes the pending candidate to
//!   active (§3.2). When a version is already active the caller restores
//!   its state blob into the candidate and reports a [`RestoreOutcome`]; a
//!   failed restore rejects the swap and the old version keeps running
//!   (§3.3). A first activation needs no restore.
//! - [`FunctionRegistry::resolve`] answers the mapping: the active
//!   version's coordinates, never a pending one (§1.2, discovery is by the
//!   mapping).
//! - [`FunctionRegistry::set_expectation`] is the explicit, versioned
//!   mapping change (§1.3); it drops a pending candidate that no longer
//!   matches, so activation is never a side effect of a stale check.
//!
//! Version numbers are assigned per (user, name), start at 1, and only a
//! successful publish consumes one: a rejected publish never became a
//! published version, so it is logged with its module hash and reason but
//! no version. Swap-time rejections (superseded, stale expectation, failed
//! restore) name the version they discarded.
//!
//! The real instantiation, init and snapshot/restore wiring through
//! `sandbox.rs`, with one instance (and so one fuel budget) per function
//! version, is the host half in `world_function_host` (`FunctionHost`,
//! under the `sandbox` feature), which owns a registry and drives it with
//! real outcomes.
//!
//! Not here, and arriving with later slices: the per-function budgets
//! inside the world's per-tick budget (§4), draining in-flight calls on the
//! old instance, and persistence of this state.

use std::collections::BTreeMap;

use crate::world::ModuleProvenance;
use crate::world_compile::{
    ActivationRefusal, FunctionArtifact, FunctionBinding, check_activation,
};

/// A function version or expectation version: monotonic per (user, name),
/// starting at 1.
pub type Version = u64;

/// One binding expectation of record (design #5964 §1.3).
///
/// A world that wants a new interface publishes a new expectation first;
/// each one carries its own version so the mapping change is auditable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingExpectation {
    /// Expectation version, monotonic per (user, name).
    pub version: Version,
    /// The binding name and the interface hash a candidate must carry.
    pub binding: FunctionBinding,
}

/// A published function version's coordinates (design #5964 §1.2).
///
/// Immutable once recorded; an edit is a new version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedVersion {
    /// Version, monotonic per (user, name).
    pub version: Version,
    /// BLAKE3 of the module bytes.
    pub module_hash: String,
    /// The interface hash the version was published under.
    pub interface_hash: String,
    /// The compile provenance the artifact carried.
    pub provenance: ModuleProvenance,
    /// The expectation version the publish was checked against.
    pub expectation: Version,
}

/// What the caller's fresh instantiation and init of a candidate did
/// (design #5964 §3.1 step 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstantiateOutcome {
    /// The module instantiated and its init returned.
    Ready,
    /// Instantiation or init trapped.
    Trapped {
        /// The trap, as the host reported it.
        message: String,
    },
}

/// What the host's state handoff into the candidate did (design #5964
/// §3.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The old version's state blob was restored into the candidate.
    Restored,
    /// The restore failed.
    Failed {
        /// The failure, as the host reported it.
        message: String,
    },
}

/// Why a publish or a swap was rejected (design #5964 §1.3, §3.1, §3.3).
///
/// Every reason means the same to the live world: the active version keeps
/// running with its state, and nothing else changed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RejectionReason {
    /// The binding has no expectation yet, so nothing can activate on it.
    #[error(
        "user {user:?}: binding {function:?} has no interface expectation; publish a binding expectation first"
    )]
    NoExpectation {
        /// The publishing user.
        user: String,
        /// The binding name.
        function: String,
    },
    /// The §1.3 activation check refused the candidate; an interface
    /// mismatch carries the expected and the candidate hash.
    #[error(transparent)]
    Refused(#[from] ActivationRefusal),
    /// The candidate's fresh instantiation or init trapped.
    #[error("candidate init trapped: {message}; the published version keeps running")]
    InitTrap {
        /// The trap, as the host reported it.
        message: String,
    },
    /// A newer successful publish replaced this pending candidate before
    /// the tick boundary; it never activated.
    #[error("pending version superseded by version {by} before the tick boundary")]
    Superseded {
        /// The version that replaced it.
        by: Version,
    },
    /// A new expectation no longer admits this pending candidate.
    #[error(
        "pending version dropped: expectation {expectation} expects interface {expected}, the candidate carries {candidate}"
    )]
    StaleExpectation {
        /// The new expectation version.
        expectation: Version,
        /// The interface hash the new expectation expects.
        expected: String,
        /// The candidate's interface hash.
        candidate: String,
    },
    /// The host could not restore the old version's state into the
    /// candidate, so the swap was rejected.
    #[error("state restore failed: {message}; the old version keeps running with its state")]
    RestoreFailed {
        /// The failure, as the host reported it.
        message: String,
    },
}

/// One recorded rejection.
///
/// A rejected publish has no version (only a successful publish consumes
/// one); a rejected swap names the version it discarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    /// The discarded version, when one was assigned.
    pub version: Option<Version>,
    /// The candidate's module hash.
    pub module_hash: String,
    /// The expectation version in force, if any.
    pub expectation: Option<Version>,
    /// Why it was rejected.
    pub reason: RejectionReason,
}

/// A successful publish: the candidate is pending until the next tick
/// boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublishAccepted {
    /// The version assigned to the candidate.
    pub version: Version,
    /// The older pending version this one superseded, if any.
    pub superseded: Option<Version>,
}

/// The versions a same-function swap moves between (design #5964 §3.3).
///
/// The caller snapshots `from` and restores into `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swap<'a> {
    /// The active version, whose state is handed over.
    pub from: &'a PublishedVersion,
    /// The pending candidate, which receives the state.
    pub to: &'a PublishedVersion,
}

/// What a tick boundary did for one binding (design #5964 §3.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundaryOutcome {
    /// No candidate was pending.
    NothingPending,
    /// The candidate became the active version.
    Activated {
        /// The newly active version.
        version: Version,
        /// The version it replaced, or `None` on a first activation.
        previous: Option<Version>,
    },
    /// The swap was rejected and recorded; the previous version stays
    /// active.
    Rejected {
        /// The discarded candidate.
        version: Version,
        /// Why.
        reason: RejectionReason,
    },
}

/// Everything recorded for one (user, binding name).
#[derive(Clone, Debug, Default)]
struct Slot {
    expectations: Vec<BindingExpectation>,
    published: Vec<PublishedVersion>,
    active: Option<Version>,
    pending: Option<Version>,
    rejections: Vec<Rejection>,
}

impl Slot {
    fn expectation(&self) -> Option<&BindingExpectation> {
        self.expectations.last()
    }

    /// Published versions are numbered 1..=n in order, so a version is
    /// its own index plus one.
    fn version(&self, version: Version) -> Option<&PublishedVersion> {
        let index = usize::try_from(version.checked_sub(1)?).ok()?;
        self.published.get(index)
    }

    fn reject(&mut self, version: Option<Version>, module_hash: String, reason: RejectionReason) {
        let expectation = self.expectation().map(|current| current.version);
        self.rejections.push(Rejection {
            version,
            module_hash,
            expectation,
            reason,
        });
    }
}

/// The per-user function mapping's publish/swap state (design #5964 §3).
///
/// Pure and deterministic: iteration order is the key order, and nothing
/// reads a clock or does IO.
#[derive(Clone, Debug, Default)]
pub struct FunctionRegistry {
    slots: BTreeMap<String, BTreeMap<String, Slot>>,
}

impl FunctionRegistry {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
        }
    }

    fn slot(&self, user: &str, name: &str) -> Option<&Slot> {
        self.slots.get(user)?.get(name)
    }

    fn slot_mut(&mut self, user: &str, name: &str) -> &mut Slot {
        self.slots
            .entry(user.to_owned())
            .or_default()
            .entry(name.to_owned())
            .or_default()
    }

    /// Record a new binding expectation for `user` and return its version.
    ///
    /// The binding name is `binding.function`. A pending candidate whose
    /// interface hash the new expectation does not admit is dropped and
    /// recorded as [`RejectionReason::StaleExpectation`]; the active version
    /// keeps running either way. The pending candidate already passed the
    /// full [`check_activation`] at publish, so only its interface hash can
    /// diverge from the new expectation.
    pub fn set_expectation(&mut self, user: &str, binding: FunctionBinding) -> Version {
        let slot = self.slot_mut(user, &binding.function);
        let version = slot.expectation().map_or(1, |current| current.version + 1);
        let expected = binding.interface_hash.clone();
        slot.expectations
            .push(BindingExpectation { version, binding });
        if let Some(pending) = slot.pending
            && let Some(candidate) = slot.version(pending)
            && candidate.interface_hash != expected
        {
            let module_hash = candidate.module_hash.clone();
            let candidate = candidate.interface_hash.clone();
            slot.pending = None;
            slot.reject(
                Some(pending),
                module_hash,
                RejectionReason::StaleExpectation {
                    expectation: version,
                    expected,
                    candidate,
                },
            );
        }
        version
    }

    /// Publish a candidate on (user, name) and make it pending (§3.1).
    ///
    /// Order: the binding must have an expectation, the artifact must pass
    /// [`check_activation`] against it, and the caller's fresh
    /// instantiation and init must have succeeded. Only then is the next
    /// version assigned and made the pending candidate, superseding (and
    /// recording) any older pending one.
    ///
    /// # Errors
    ///
    /// The [`RejectionReason`] — no expectation, an activation refusal
    /// (both hashes on a mismatch), or an init trap. The rejection is
    /// recorded with no version, and the active and pending versions are
    /// untouched.
    pub fn publish(
        &mut self,
        user: &str,
        name: &str,
        artifact: &FunctionArtifact,
        instantiate: InstantiateOutcome,
    ) -> Result<PublishAccepted, RejectionReason> {
        let slot = self.slot_mut(user, name);
        let checked = match slot.expectation() {
            None => Err(RejectionReason::NoExpectation {
                user: user.to_owned(),
                function: name.to_owned(),
            }),
            Some(current) => check_activation(&current.binding, artifact)
                .map(|()| current.version)
                .map_err(RejectionReason::from),
        }
        .and_then(|expectation| match instantiate {
            InstantiateOutcome::Ready => Ok(expectation),
            InstantiateOutcome::Trapped { message } => Err(RejectionReason::InitTrap { message }),
        });
        let expectation = match checked {
            Ok(expectation) => expectation,
            Err(reason) => {
                slot.reject(None, artifact.module_hash.clone(), reason.clone());
                return Err(reason);
            }
        };
        let version = slot.published.len() as Version + 1;
        slot.published.push(PublishedVersion {
            version,
            module_hash: artifact.module_hash.clone(),
            interface_hash: artifact.interface_hash.clone(),
            provenance: artifact.provenance.clone(),
            expectation,
        });
        let superseded = slot.pending.replace(version);
        if let Some(old) = superseded {
            let module_hash = slot
                .version(old)
                .map(|candidate| candidate.module_hash.clone())
                .unwrap_or_default();
            slot.reject(
                Some(old),
                module_hash,
                RejectionReason::Superseded { by: version },
            );
        }
        Ok(PublishAccepted {
            version,
            superseded,
        })
    }

    /// Run the tick boundary for (user, name): promote the pending
    /// candidate to active (§3.2, §3.3).
    ///
    /// When a version is already active, `restore` is called with the
    /// [`Swap`] and reports the host's state handoff; a failed restore
    /// rejects the swap (recorded) and the old version stays active. A
    /// first activation does not call `restore`.
    #[must_use]
    pub fn tick_boundary(
        &mut self,
        user: &str,
        name: &str,
        restore: impl FnOnce(Swap<'_>) -> RestoreOutcome,
    ) -> BoundaryOutcome {
        let Some(slot) = self
            .slots
            .get_mut(user)
            .and_then(|names| names.get_mut(name))
        else {
            return BoundaryOutcome::NothingPending;
        };
        let Some(version) = slot.pending.take() else {
            return BoundaryOutcome::NothingPending;
        };
        let previous = slot.active;
        if let Some(active) = previous
            && let (Some(from), Some(to)) = (slot.version(active), slot.version(version))
            && let RestoreOutcome::Failed { message } = restore(Swap { from, to })
        {
            let module_hash = to.module_hash.clone();
            let reason = RejectionReason::RestoreFailed { message };
            slot.reject(Some(version), module_hash, reason.clone());
            return BoundaryOutcome::Rejected { version, reason };
        }
        slot.active = Some(version);
        BoundaryOutcome::Activated { version, previous }
    }

    /// The active version of (user, name): what the mapping resolves to.
    ///
    /// Never a pending candidate (§1.2, §3.2).
    #[must_use]
    pub fn resolve(&self, user: &str, name: &str) -> Option<&PublishedVersion> {
        let slot = self.slot(user, name)?;
        slot.version(slot.active?)
    }

    /// The pending candidate of (user, name), awaiting the next boundary.
    #[must_use]
    pub fn pending(&self, user: &str, name: &str) -> Option<&PublishedVersion> {
        let slot = self.slot(user, name)?;
        slot.version(slot.pending?)
    }

    /// The current binding expectation of (user, name).
    #[must_use]
    pub fn expectation(&self, user: &str, name: &str) -> Option<&BindingExpectation> {
        self.slot(user, name)?.expectation()
    }

    /// Every binding expectation of (user, name), oldest first.
    #[must_use]
    pub fn expectations(&self, user: &str, name: &str) -> &[BindingExpectation] {
        self.slot(user, name)
            .map_or(&[], |slot| slot.expectations.as_slice())
    }

    /// Every published version of (user, name), in version order.
    #[must_use]
    pub fn published(&self, user: &str, name: &str) -> &[PublishedVersion] {
        self.slot(user, name)
            .map_or(&[], |slot| slot.published.as_slice())
    }

    /// Every recorded rejection of (user, name), oldest first.
    #[must_use]
    pub fn rejections(&self, user: &str, name: &str) -> &[Rejection] {
        self.slot(user, name)
            .map_or(&[], |slot| slot.rejections.as_slice())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world_compile::{FunctionInterface, FunctionShape};

    const USER: &str = "alice";
    const NAME: &str = "echo";

    fn shape(state_schema: &str) -> FunctionShape {
        FunctionShape {
            input: "Str".to_owned(),
            output: "Str".to_owned(),
            state_schema: state_schema.to_owned(),
        }
    }

    fn interface(state_schema: &str) -> FunctionInterface {
        FunctionInterface::new(
            NAME.to_owned(),
            shape(state_schema),
            vec!["Clock".to_owned()],
        )
    }

    /// A candidate artifact under `interface` with distinct module bytes,
    /// built without the compiler.
    fn candidate(interface: FunctionInterface, module: &[u8]) -> FunctionArtifact {
        FunctionArtifact {
            function: interface.function().to_owned(),
            module: module.to_vec(),
            diagnostics: String::new(),
            provenance: ModuleProvenance::Artifact,
            module_hash: crate::world_cache::artifact_hash(module),
            interface_hash: interface.interface_hash(),
            interface,
        }
    }

    fn registry_expecting(interface: &FunctionInterface) -> FunctionRegistry {
        let mut registry = FunctionRegistry::new();
        assert_eq!(
            registry.set_expectation(USER, FunctionBinding::expecting(interface)),
            1
        );
        registry
    }

    fn restored(_: Swap<'_>) -> RestoreOutcome {
        RestoreOutcome::Restored
    }

    #[test]
    fn versions_are_monotonic_and_immutable() {
        // Oracle: design #5964 §1.2 — the version is the publish sequence
        // for (user, name), monotonically increasing and assigned at
        // publish; a version is immutable once published.
        let v1 = interface("echo-state-v1");
        let mut registry = registry_expecting(&v1);
        let first = registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(first.version, 1);
        let snapshot = registry.published(USER, NAME)[0].clone();
        // A rejected publish consumes no version.
        let trapped = InstantiateOutcome::Trapped {
            message: "unreachable".to_owned(),
        };
        registry
            .publish(USER, NAME, &candidate(v1.clone(), b"\0asm-x"), trapped)
            .unwrap_err();
        let second = registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-2"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(second.version, 2);
        let _ = registry.tick_boundary(USER, NAME, |_| RestoreOutcome::Restored);
        let third = registry
            .publish(
                USER,
                NAME,
                &candidate(v1, b"\0asm-3"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(third.version, 3);
        let published = registry.published(USER, NAME);
        assert_eq!(
            published.iter().map(|p| p.version).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(published[0], snapshot);
        assert_eq!(
            published[2].module_hash,
            crate::world_cache::artifact_hash(b"\0asm-3")
        );
        // Another user's versions are numbered independently.
        let mut other = registry.clone();
        other.set_expectation(
            "bob",
            FunctionBinding::expecting(&interface("echo-state-v1")),
        );
        let bob = other
            .publish(
                "bob",
                NAME,
                &candidate(interface("echo-state-v1"), b"\0asm-b"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(bob.version, 1);
    }

    #[test]
    fn pending_never_resolves_before_the_boundary() {
        // Oracle: design #5964 §3.1, §3.2 — a successful publish becomes
        // active only at the next tick boundary, and discovery is by the
        // mapping, which resolves the active version only.
        let v1 = interface("echo-state-v1");
        let mut registry = registry_expecting(&v1);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert!(registry.resolve(USER, NAME).is_none());
        assert_eq!(registry.pending(USER, NAME).unwrap().version, 1);
        let _ = registry.tick_boundary(USER, NAME, |_| RestoreOutcome::Restored);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1, b"\0asm-2"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(registry.resolve(USER, NAME).unwrap().version, 1);
        assert_eq!(registry.pending(USER, NAME).unwrap().version, 2);
    }

    #[test]
    fn boundary_activates_the_pending_candidate() {
        // Oracle: design #5964 §3.2 — at the tick boundary (user, name)
        // rebinds to the new version, whose coordinates the mapping then
        // resolves; with nothing pending the boundary changes nothing.
        let v1 = interface("echo-state-v1");
        let mut registry = registry_expecting(&v1);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(
            registry.tick_boundary(USER, NAME, restored),
            BoundaryOutcome::Activated {
                version: 1,
                previous: None
            }
        );
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-2"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        let mut handed = None;
        let outcome = registry.tick_boundary(USER, NAME, |swap| {
            handed = Some((swap.from.version, swap.to.version));
            RestoreOutcome::Restored
        });
        assert_eq!(
            outcome,
            BoundaryOutcome::Activated {
                version: 2,
                previous: Some(1)
            }
        );
        assert_eq!(handed, Some((1, 2)));
        let active = registry.resolve(USER, NAME).unwrap();
        assert_eq!(active.version, 2);
        assert_eq!(
            active.module_hash,
            crate::world_cache::artifact_hash(b"\0asm-2")
        );
        assert_eq!(active.interface_hash, v1.interface_hash());
        assert!(registry.pending(USER, NAME).is_none());
        assert_eq!(
            registry.tick_boundary(USER, NAME, restored),
            BoundaryOutcome::NothingPending
        );
        assert_eq!(
            registry.tick_boundary(USER, "unknown", restored),
            BoundaryOutcome::NothingPending
        );
    }

    #[test]
    fn mismatch_is_rejected_with_both_hashes_and_active_unchanged() {
        // Oracle: design #5964 §1.3 — a candidate whose interface hash
        // differs from the expectation is not activated, the published
        // version keeps running, and the publish is recorded as rejected
        // with the two hashes.
        let v1 = interface("echo-state-v1");
        let v2 = interface("echo-state-v2");
        let mut registry = registry_expecting(&v1);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        let _ = registry.tick_boundary(USER, NAME, restored);
        let wrong = candidate(v2.clone(), b"\0asm-2");
        let reason = registry
            .publish(USER, NAME, &wrong, InstantiateOutcome::Ready)
            .unwrap_err();
        let expected = RejectionReason::Refused(ActivationRefusal::InterfaceMismatch {
            function: NAME.to_owned(),
            expected: v1.interface_hash(),
            candidate: v2.interface_hash(),
        });
        assert_eq!(reason, expected);
        assert_eq!(
            registry.rejections(USER, NAME),
            [Rejection {
                version: None,
                module_hash: wrong.module_hash,
                expectation: Some(1),
                reason: expected,
            }]
        );
        assert_eq!(registry.resolve(USER, NAME).unwrap().version, 1);
        assert!(registry.pending(USER, NAME).is_none());
        assert_eq!(registry.published(USER, NAME).len(), 1);
    }

    #[test]
    fn init_trap_is_rejected_and_nothing_changes() {
        // Oracle: design #5964 §3.1 step 3 — an instantiation or init trap
        // discards the candidate and the previously published version
        // continues; the rejection is recorded.
        let v1 = interface("echo-state-v1");
        let mut registry = registry_expecting(&v1);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        let _ = registry.tick_boundary(USER, NAME, restored);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-2"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        let trapped = InstantiateOutcome::Trapped {
            message: "init: unreachable".to_owned(),
        };
        let reason = registry
            .publish(USER, NAME, &candidate(v1, b"\0asm-3"), trapped)
            .unwrap_err();
        assert_eq!(
            reason,
            RejectionReason::InitTrap {
                message: "init: unreachable".to_owned()
            }
        );
        assert_eq!(registry.resolve(USER, NAME).unwrap().version, 1);
        // The earlier pending candidate is untouched by the failed publish.
        assert_eq!(registry.pending(USER, NAME).unwrap().version, 2);
        assert_eq!(registry.rejections(USER, NAME).len(), 1);
        assert_eq!(registry.rejections(USER, NAME)[0].version, None);
    }

    #[test]
    fn failed_restore_rejects_the_swap_and_keeps_the_old_version() {
        // Oracle: design #5964 §3.3 — for a new version of the same
        // function the host restores the old state into the fresh instance;
        // a failed restore rejects the swap, the old version continues with
        // its state, and the rejection is recorded.
        let v1 = interface("echo-state-v1");
        let mut registry = registry_expecting(&v1);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        let _ = registry.tick_boundary(USER, NAME, restored);
        let second = candidate(v1, b"\0asm-2");
        registry
            .publish(USER, NAME, &second, InstantiateOutcome::Ready)
            .unwrap();
        let reason = RejectionReason::RestoreFailed {
            message: "schema decode".to_owned(),
        };
        assert_eq!(
            registry.tick_boundary(USER, NAME, |_| RestoreOutcome::Failed {
                message: "schema decode".to_owned()
            }),
            BoundaryOutcome::Rejected {
                version: 2,
                reason: reason.clone()
            }
        );
        assert_eq!(registry.resolve(USER, NAME).unwrap().version, 1);
        assert!(registry.pending(USER, NAME).is_none());
        assert_eq!(
            registry.rejections(USER, NAME),
            [Rejection {
                version: Some(2),
                module_hash: second.module_hash,
                expectation: Some(1),
                reason,
            }]
        );
        assert_eq!(
            registry.tick_boundary(USER, NAME, restored),
            BoundaryOutcome::NothingPending
        );
    }

    #[test]
    fn newer_publish_supersedes_the_pending_candidate() {
        // Oracle: design #5964 §3.1, §3.2 — only one candidate waits for
        // the boundary; a newer successful publish replaces it, and the
        // replaced one is recorded as superseded, never activated.
        let v1 = interface("echo-state-v1");
        let mut registry = registry_expecting(&v1);
        let first = candidate(v1.clone(), b"\0asm-1");
        registry
            .publish(USER, NAME, &first, InstantiateOutcome::Ready)
            .unwrap();
        let accepted = registry
            .publish(
                USER,
                NAME,
                &candidate(v1, b"\0asm-2"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(
            accepted,
            PublishAccepted {
                version: 2,
                superseded: Some(1)
            }
        );
        assert_eq!(
            registry.rejections(USER, NAME),
            [Rejection {
                version: Some(1),
                module_hash: first.module_hash,
                expectation: Some(1),
                reason: RejectionReason::Superseded { by: 2 },
            }]
        );
        assert_eq!(
            registry.tick_boundary(USER, NAME, restored),
            BoundaryOutcome::Activated {
                version: 2,
                previous: None
            }
        );
    }

    #[test]
    fn expectation_change_drops_a_stale_pending_then_a_matching_publish_activates() {
        // Oracle: design #5964 §1.3 — interface evolution is an explicit
        // two-step: a new, versioned binding expectation first, then a
        // candidate carrying the new hash. A pending candidate the new
        // expectation does not admit never activates, and the running
        // version keeps running until the new candidate's boundary.
        let v1 = interface("echo-state-v1");
        let v2 = interface("echo-state-v2");
        let mut registry = registry_expecting(&v1);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1.clone(), b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        let _ = registry.tick_boundary(USER, NAME, restored);
        let stale = candidate(v1.clone(), b"\0asm-2");
        registry
            .publish(USER, NAME, &stale, InstantiateOutcome::Ready)
            .unwrap();
        assert_eq!(
            registry.set_expectation(USER, FunctionBinding::expecting(&v2)),
            2
        );
        assert!(registry.pending(USER, NAME).is_none());
        assert_eq!(
            registry.rejections(USER, NAME),
            [Rejection {
                version: Some(2),
                module_hash: stale.module_hash,
                expectation: Some(2),
                reason: RejectionReason::StaleExpectation {
                    expectation: 2,
                    expected: v2.interface_hash(),
                    candidate: v1.interface_hash(),
                },
            }]
        );
        assert_eq!(
            registry.tick_boundary(USER, NAME, restored),
            BoundaryOutcome::NothingPending
        );
        assert_eq!(registry.resolve(USER, NAME).unwrap().version, 1);
        let accepted = registry
            .publish(
                USER,
                NAME,
                &candidate(v2.clone(), b"\0asm-3"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(accepted.version, 3);
        assert_eq!(registry.published(USER, NAME)[2].expectation, 2);
        assert_eq!(
            registry.tick_boundary(USER, NAME, restored),
            BoundaryOutcome::Activated {
                version: 3,
                previous: Some(1)
            }
        );
        assert_eq!(
            registry.resolve(USER, NAME).unwrap().interface_hash,
            v2.interface_hash()
        );
        assert_eq!(
            registry
                .expectations(USER, NAME)
                .iter()
                .map(|e| e.version)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        // A matching pending candidate survives a re-stated expectation.
        registry
            .publish(
                USER,
                NAME,
                &candidate(v2.clone(), b"\0asm-4"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        assert_eq!(
            registry.set_expectation(USER, FunctionBinding::expecting(&v2)),
            3
        );
        assert_eq!(registry.pending(USER, NAME).unwrap().version, 4);
    }

    #[test]
    fn first_activation_needs_no_restore() {
        // Oracle: design #5964 §3.3 — state handoff is for a new version
        // of a function already running; the first activation has no old
        // instance and so no state to restore.
        let v1 = interface("echo-state-v1");
        let mut registry = registry_expecting(&v1);
        registry
            .publish(
                USER,
                NAME,
                &candidate(v1, b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap();
        let outcome = registry.tick_boundary(USER, NAME, |_| {
            panic!("a first activation must not ask for a state restore")
        });
        assert_eq!(
            outcome,
            BoundaryOutcome::Activated {
                version: 1,
                previous: None
            }
        );
        assert_eq!(registry.resolve(USER, NAME).unwrap().version, 1);
    }

    #[test]
    fn publish_without_an_expectation_is_refused() {
        // Oracle: design #5964 §1.3 — activation needs the interface hash
        // the binding expects; with no expectation of record nothing can
        // activate, and the refusal is recorded.
        let mut registry = FunctionRegistry::new();
        let artifact = candidate(interface("echo-state-v1"), b"\0asm-1");
        let reason = registry
            .publish(USER, NAME, &artifact, InstantiateOutcome::Ready)
            .unwrap_err();
        assert_eq!(
            reason,
            RejectionReason::NoExpectation {
                user: USER.to_owned(),
                function: NAME.to_owned(),
            }
        );
        assert!(registry.published(USER, NAME).is_empty());
        assert!(registry.pending(USER, NAME).is_none());
        assert_eq!(
            registry.rejections(USER, NAME),
            [Rejection {
                version: None,
                module_hash: artifact.module_hash,
                expectation: None,
                reason,
            }]
        );
        assert_eq!(
            registry.tick_boundary(USER, NAME, restored),
            BoundaryOutcome::NothingPending
        );
    }

    #[test]
    fn wrong_binding_name_is_refused() {
        // Oracle: design #5964 §1.3 — the shared activation check runs on
        // publish, so a candidate compiled for another binding is refused.
        let v1 = interface("echo-state-v1");
        let mut registry = registry_expecting(&v1);
        let other = FunctionInterface::new("other".to_owned(), shape("echo-state-v1"), vec![]);
        let reason = registry
            .publish(
                USER,
                NAME,
                &candidate(other, b"\0asm-1"),
                InstantiateOutcome::Ready,
            )
            .unwrap_err();
        assert!(matches!(
            reason,
            RejectionReason::Refused(ActivationRefusal::WrongFunction { .. })
        ));
        assert!(registry.published(USER, NAME).is_empty());
    }
}
