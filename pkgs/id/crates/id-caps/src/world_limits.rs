//! Execution bounds for world programs, set by the server's admin.
//!
//! A Wasm guest runs under fuel, memory and payload caps; a native worker
//! runs under address-space and CPU rlimits and a per-call deadline. The
//! defaults are conservative. Raising them is an admin decision, and the
//! values are validated before the server starts.

use std::time::Duration;

use anyhow::{Result, ensure};

/// Largest Wasm world module that may be installed (16 MiB).
///
/// The single definition lives here since the Phase 1 crate split
/// (#3447): `world_session` re-exports it at its historical path.
pub const MAX_WORLD_MODULE_BYTES: usize = 16 * 1024 * 1024;

/// Default server hard ceiling for one world's total logical memory
/// budget (64 MiB, #557 Phase 1).
///
/// This is a proposed policy value, not a measured result (design
/// `doc/2026-11-01_design_id557.md`, Q1): four times the default Wasm
/// linear-memory ceiling, leaving 48 MiB of logical-charge room for the
/// module and other charges. The budget is a deterministic logical-charge
/// ceiling over allocations owned by one world — program reservation,
/// persistent logical state, session/queued charges, and transient
/// persistence peaks — never an RSS guarantee: allocator metadata,
/// thread stacks, and process-shared runtime caches are outside the
/// accounting model. Phase 1 established the validated, persisted
/// contract; Phase 2 (below) adds the ledger and enforces it at world
/// restore/open. Runtime growth, program, and transient-peak charge
/// sites remain later phases.
pub const DEFAULT_WORLD_TOTAL_MEMORY_BYTES: u64 = 64 * 1024 * 1024;

/// Fixed logical charge for a world's own core state — the
/// `WorldCore` struct, its maps/deques, and the actor's fixed
/// collections — independent of journal size (#557 Phase 2). A policy
/// constant, not an allocator measurement.
pub const WORLD_MEMORY_BASE_BYTES: u64 = 4 * 1024;

/// Fixed structural allowance charged per materialized journal entry,
/// on top of its serialized length (#557 Phase 2): the entry enum, its
/// owned strings/vectors' headers, and map nodes. A policy constant,
/// not an allocator measurement.
pub const JOURNAL_ENTRY_OVERHEAD_BYTES: u64 = 64;

/// Logical charge for one materialized journal entry: its serialized
/// payload length plus the fixed structural allowance.
#[must_use]
pub fn journal_entry_charge(serialized_len: usize) -> u64 {
    u64::try_from(serialized_len)
        .unwrap_or(u64::MAX)
        .saturating_add(JOURNAL_ENTRY_OVERHEAD_BYTES)
}

/// A refused charge against a world's total memory budget (#557).
///
/// Typed and recoverable, and deliberately distinct from a guest trap,
/// an open-world ceiling refusal, and a storage failure: the world is
/// unharmed, nothing was charged, and a retry can succeed after
/// charges are released or the configured budget is raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldMemoryBudgetRefusal {
    /// The world's effective budget, in logical bytes.
    pub limit_bytes: u64,
    /// Logical bytes already charged when the charge was refused.
    pub charged_bytes: u64,
    /// Logical bytes the refused charge asked for.
    pub requested_bytes: u64,
}

impl std::fmt::Display for WorldMemoryBudgetRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "world is at its memory budget: {} charged + {} requested              exceeds the {}-byte logical budget",
            self.charged_bytes, self.requested_bytes, self.limit_bytes
        )
    }
}

impl std::error::Error for WorldMemoryBudgetRefusal {}

/// Per-world logical-charge ledger for the total memory budget
/// (#557 Phase 2; design `doc/2026-11-01_design_id557.md`).
///
/// Deterministic accounting, not an RSS gauge: every charge is a
/// documented logical size (a serialized length, a reservation
/// ceiling, or a fixed policy allowance). The invariant is
/// `charged_bytes <= limit_bytes` at every observable point; a charge
/// that would break it is refused whole — it charges nothing and
/// bumps `refusal_count` — so a refused operation leaves the ledger
/// exactly as it was and can be retried after a release.
#[derive(Debug, Clone)]
pub struct WorldMemoryBudget {
    limit_bytes: u64,
    charged_bytes: u64,
    high_water_bytes: u64,
    refusal_count: u64,
}

impl WorldMemoryBudget {
    /// A fresh ledger with nothing charged.
    #[must_use]
    pub const fn new(limit_bytes: u64) -> Self {
        Self {
            limit_bytes,
            charged_bytes: 0,
            high_water_bytes: 0,
            refusal_count: 0,
        }
    }

    /// The effective budget this ledger enforces, in logical bytes.
    #[must_use]
    pub const fn limit_bytes(&self) -> u64 {
        self.limit_bytes
    }

    /// Set the effective budget (a profile resolved after construction
    /// can only have lowered it; enforcement is against the new limit
    /// from the next charge on).
    pub const fn set_limit_bytes(&mut self, limit_bytes: u64) {
        self.limit_bytes = limit_bytes;
    }

    /// Logical bytes currently charged.
    #[must_use]
    pub const fn charged_bytes(&self) -> u64 {
        self.charged_bytes
    }

    /// Highest charged total ever reached under this ledger.
    #[must_use]
    pub const fn high_water_bytes(&self) -> u64 {
        self.high_water_bytes
    }

    /// How many charges this ledger has refused.
    #[must_use]
    pub const fn refusal_count(&self) -> u64 {
        self.refusal_count
    }

    /// Charge `bytes` against the budget, or refuse the whole charge.
    ///
    /// # Errors
    ///
    /// Returns [`WorldMemoryBudgetRefusal`] — charging nothing — when
    /// the charge would take the total past the limit (or overflow).
    pub fn try_charge(&mut self, bytes: u64) -> Result<(), WorldMemoryBudgetRefusal> {
        match self.charged_bytes.checked_add(bytes) {
            Some(total) if total <= self.limit_bytes => {
                self.charged_bytes = total;
                self.high_water_bytes = self.high_water_bytes.max(total);
                Ok(())
            }
            _ => {
                self.refusal_count += 1;
                Err(WorldMemoryBudgetRefusal {
                    limit_bytes: self.limit_bytes,
                    charged_bytes: self.charged_bytes,
                    requested_bytes: bytes,
                })
            }
        }
    }

    /// Release a previous charge. Saturates at zero: releasing more
    /// than is charged is a caller bug the ledger absorbs rather than
    /// underflowing into free budget.
    pub const fn release(&mut self, bytes: u64) {
        self.charged_bytes = self.charged_bytes.saturating_sub(bytes);
    }
}

impl Default for WorldMemoryBudget {
    fn default() -> Self {
        Self::new(DEFAULT_WORLD_TOTAL_MEMORY_BYTES)
    }
}

/// Resource bounds applied to each guest invocation.
///
/// The definition lives here since the Phase 1 crate split (#3447):
/// the `sandbox` module re-exports it at its historical path.
#[derive(Clone, Copy, Debug)]
pub struct SandboxLimits {
    /// Maximum compiled Wasm module size, in bytes.
    pub module_bytes: usize,
    /// Maximum Wasm fuel for an invocation.
    pub fuel: u64,
    /// Maximum linear memory, in bytes.
    pub memory_bytes: usize,
    /// Maximum input or output payload, in bytes.
    pub message_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            module_bytes: 16 * 1024 * 1024,
            fuel: 10_000_000,
            memory_bytes: 16 * 1024 * 1024,
            message_bytes: 1024 * 1024,
        }
    }
}

/// Resource bounds for one native worker.
///
/// The definition lives here since the Phase 1 crate split (#3447):
/// the `world_native` module re-exports it at its historical path.
#[derive(Clone, Copy, Debug)]
pub struct NativeLimits {
    /// Address-space cap (`RLIMIT_AS`).
    pub memory_bytes: u64,
    /// CPU cap (`RLIMIT_CPU`).
    pub cpu_seconds: u64,
    /// Wall-clock budget per call.
    pub call_deadline: Duration,
}

impl Default for NativeLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 256 * 1024 * 1024,
            cpu_seconds: 10,
            // Generous: the host machine may be loaded while a world is
            // starting; correctness does not depend on snappy guests.
            call_deadline: Duration::from_secs(10),
        }
    }
}

/// The raw world-runtime flag values [`RuntimeLimits::from_args`]
/// validates.
///
/// The CLI's `WorldRuntimeArgs` (in the binary crate, which depends on
/// this crate and so cannot be named here) implements this trait since
/// the Phase 1 crate split (#3447); `from_args` keeps its exact
/// validation and semantics.
pub trait WorldLimitArgs {
    /// Wasm fuel (instruction budget) for one world call.
    fn world_fuel(&self) -> u64;
    /// Linear memory for a Wasm world program, in MiB.
    fn world_memory_mib(&self) -> u64;
    /// Largest Wasm payload (input or output), in KiB.
    fn world_message_kib(&self) -> u64;
    /// Address-space cap for a native worker, in MiB.
    fn world_native_memory_mib(&self) -> u64;
    /// CPU seconds a native worker may use in total.
    fn world_native_cpu_secs(&self) -> u64;
    /// Wall-clock budget for one native worker call, in milliseconds.
    fn world_native_deadline_ms(&self) -> u64;
    /// Total per-world logical memory budget, in MiB.
    fn world_total_memory_mib(&self) -> u64;
}

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

/// Bounds applied to every world program the server runs.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeLimits {
    /// Bounds for a Wasm guest: fuel, memory and payload sizes.
    #[cfg(feature = "sandbox")]
    pub sandbox: SandboxLimits,
    /// Bounds for a native worker: address space, CPU and call deadline.
    pub native: NativeLimits,
    /// Server hard ceiling for one world's total logical memory budget, in
    /// bytes (#557 Phase 1). A per-world request can only lower it, via
    /// [`WorldExecutionProfile::total_memory_bytes`].
    pub total_memory_bytes: u64,
}

/// Per-world resource limits requested beneath the server's hard ceilings.
///
/// Missing fields inherit the current server ceiling. A requested value above
/// that ceiling is clamped during [`Self::resolve`]. This type changes no
/// running instance and does not grant capabilities or enable native code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WorldExecutionProfile {
    /// Requested Wasm fuel limit; `None` inherits the server ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_fuel: Option<u64>,
    /// Requested Wasm linear memory in bytes; `None` inherits the server ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_memory_bytes: Option<u64>,
    /// Requested Wasm input/output payload limit in bytes; `None` inherits the server ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_message_bytes: Option<u64>,
    /// Requested native address-space cap in bytes; `None` inherits the server ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_memory_bytes: Option<u64>,
    /// Requested native CPU cap in seconds; `None` inherits the server ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_cpu_seconds: Option<u64>,
    /// Requested native per-call wall-clock deadline in milliseconds; `None` inherits the server ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_call_deadline_ms: Option<u64>,
    /// Requested total per-world logical memory budget in bytes; `None`
    /// inherits the server ceiling (#557 Phase 1). The budget is a
    /// deterministic logical-charge ceiling, not an RSS cap; see
    /// [`DEFAULT_WORLD_TOTAL_MEMORY_BYTES`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_memory_bytes: Option<u64>,
}

impl WorldExecutionProfile {
    /// Validate the requested bounds on their own.
    ///
    /// Every bound that is set must be positive and at or above its floor:
    /// Wasm memory at least one Wasm page, native address space at least
    /// 16 MiB, and every other bound nonzero. A bound left as `None`
    /// inherits the server ceiling and is always valid.
    ///
    /// This is the single source of truth for the bound checks:
    /// [`Self::resolve`] runs it against the live profile at apply time,
    /// and the world manifest validator (`WorldManifest::validate`, in the
    /// parent `id` crate) runs it at parse time, so a manifest can never
    /// accept bounds that apply would later refuse (#3015).
    ///
    /// # Errors
    ///
    /// Names the first offending bound.
    pub fn validate(&self) -> Result<()> {
        ensure!(self.wasm_fuel != Some(0), "world Wasm fuel must be nonzero");
        ensure!(
            self.wasm_memory_bytes.is_none_or(|bytes| bytes >= 64 * KIB),
            "world Wasm memory limit must be at least one Wasm page"
        );
        ensure!(
            self.wasm_message_bytes != Some(0),
            "world Wasm message limit must be nonzero"
        );
        ensure!(
            self.native_memory_bytes
                .is_none_or(|bytes| bytes >= 16 * MIB),
            "world native memory limit must be at least 16 MiB"
        );
        ensure!(
            self.native_cpu_seconds != Some(0),
            "world native CPU limit must be nonzero"
        );
        ensure!(
            self.native_call_deadline_ms != Some(0),
            "world native call deadline must be nonzero"
        );
        ensure!(
            self.total_memory_bytes != Some(0),
            "world total memory budget must be nonzero"
        );
        Ok(())
    }

    /// Resolve these requested bounds beneath the server's hard limits.
    ///
    /// Every bound may lower the server ceiling or inherit it, but never
    /// raise it: a set value is clamped to the corresponding `hard` limit
    /// and a `None` value takes the ceiling. The Wasm module-size cap is
    /// always taken from `hard` (the profile cannot request it).
    ///
    /// Calls [`Self::validate`] first, so an invalid requested bound is
    /// refused here exactly as the world manifest validator refuses it at
    /// parse time (#3015).
    ///
    /// # Errors
    ///
    /// Propagates [`Self::validate`]'s errors, and fails when a resolved
    /// bound cannot be represented on this host.
    pub fn resolve(self, hard: RuntimeLimits) -> Result<RuntimeLimits> {
        self.validate()?;

        // Phase 1 (#557): resolve the total budget only. Program ceilings
        // keep their existing beneath-the-server-ceiling semantics; clamping
        // them beneath the budget lands with the enforcement charge sites
        // (Phase 3), so a configured budget never silently shrinks guest
        // limits before the ledger exists to account for it.
        let total_memory_bytes = self
            .total_memory_bytes
            .map_or(hard.total_memory_bytes, |value| {
                value.min(hard.total_memory_bytes)
            });
        #[cfg(feature = "sandbox")]
        let sandbox = {
            let hard_memory = u64::try_from(hard.sandbox.memory_bytes)?;
            let hard_message = u64::try_from(hard.sandbox.message_bytes)?;
            SandboxLimits {
                module_bytes: hard.sandbox.module_bytes,
                fuel: self
                    .wasm_fuel
                    .map_or(hard.sandbox.fuel, |value| value.min(hard.sandbox.fuel)),
                memory_bytes: usize::try_from(
                    self.wasm_memory_bytes
                        .map_or(hard_memory, |value| value.min(hard_memory)),
                )?,
                message_bytes: usize::try_from(
                    self.wasm_message_bytes
                        .map_or(hard_message, |value| value.min(hard_message)),
                )?,
            }
        };

        let native = NativeLimits {
            memory_bytes: self
                .native_memory_bytes
                .map_or(hard.native.memory_bytes, |value| {
                    value.min(hard.native.memory_bytes)
                }),
            cpu_seconds: self
                .native_cpu_seconds
                .map_or(hard.native.cpu_seconds, |value| {
                    value.min(hard.native.cpu_seconds)
                }),
            call_deadline: self
                .native_call_deadline_ms
                .map_or(hard.native.call_deadline, |millis| {
                    Duration::from_millis(millis).min(hard.native.call_deadline)
                }),
        };

        Ok(RuntimeLimits {
            #[cfg(feature = "sandbox")]
            sandbox,
            native,
            total_memory_bytes,
        })
    }
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            #[cfg(feature = "sandbox")]
            sandbox: SandboxLimits {
                module_bytes: MAX_WORLD_MODULE_BYTES,
                ..SandboxLimits::default()
            },
            native: NativeLimits::default(),
            total_memory_bytes: DEFAULT_WORLD_TOTAL_MEMORY_BYTES,
        }
    }
}

impl RuntimeLimits {
    /// Validate the admin's flag values into limits.
    pub fn from_args<A: WorldLimitArgs>(args: &A) -> Result<Self> {
        ensure!(args.world_fuel() > 0, "--world-fuel must be at least 1");
        ensure!(
            (1..=2048).contains(&args.world_memory_mib()),
            "--world-memory-mib must be 1..=2048"
        );
        ensure!(
            (1..=1024).contains(&args.world_message_kib()),
            "--world-message-kib must be 1..=1024"
        );
        ensure!(
            (16..=65_536).contains(&args.world_native_memory_mib()),
            "--world-native-memory-mib must be 16..=65536"
        );
        ensure!(
            args.world_native_cpu_secs() > 0,
            "--world-native-cpu-secs must be at least 1"
        );
        ensure!(
            args.world_native_deadline_ms() > 0,
            "--world-native-deadline-ms must be at least 1"
        );
        ensure!(
            (1..=65_536).contains(&args.world_total_memory_mib()),
            "--world-total-memory-mib must be 1..=65536"
        );
        Ok(Self {
            #[cfg(feature = "sandbox")]
            sandbox: SandboxLimits {
                module_bytes: MAX_WORLD_MODULE_BYTES,
                fuel: args.world_fuel(),
                memory_bytes: usize::try_from(args.world_memory_mib() * MIB)?,
                message_bytes: usize::try_from(args.world_message_kib() * KIB)?,
            },
            native: NativeLimits {
                memory_bytes: args.world_native_memory_mib() * MIB,
                cpu_seconds: args.world_native_cpu_secs(),
                call_deadline: Duration::from_millis(args.world_native_deadline_ms()),
            },
            total_memory_bytes: args.world_total_memory_mib() * MIB,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_charges_up_to_the_exact_limit_and_refuses_past_it() {
        let mut budget = WorldMemoryBudget::new(100);
        budget.try_charge(60).unwrap();
        budget.try_charge(40).unwrap();
        assert_eq!(budget.charged_bytes(), 100);
        assert_eq!(budget.high_water_bytes(), 100);
        let refusal = budget.try_charge(1).unwrap_err();
        assert_eq!(
            refusal,
            WorldMemoryBudgetRefusal {
                limit_bytes: 100,
                charged_bytes: 100,
                requested_bytes: 1,
            }
        );
    }

    #[test]
    fn a_refused_charge_charges_nothing_and_counts_the_refusal() {
        let mut budget = WorldMemoryBudget::new(10);
        budget.try_charge(10).unwrap();
        assert!(budget.try_charge(1).is_err());
        assert_eq!(budget.charged_bytes(), 10, "refusal must be atomic");
        assert_eq!(budget.refusal_count(), 1);
        // After a release the same charge succeeds: refusal is
        // recoverable, not poisoning.
        budget.release(1);
        budget.try_charge(1).unwrap();
        assert_eq!(budget.charged_bytes(), 10);
        assert_eq!(budget.high_water_bytes(), 10);
    }

    #[test]
    fn an_overflowing_charge_is_refused_not_wrapped() {
        let mut budget = WorldMemoryBudget::new(u64::MAX);
        budget.try_charge(u64::MAX).unwrap();
        assert!(budget.try_charge(1).is_err());
        assert_eq!(budget.charged_bytes(), u64::MAX);
    }

    #[test]
    fn releasing_more_than_charged_saturates_at_zero() {
        let mut budget = WorldMemoryBudget::new(10);
        budget.try_charge(4).unwrap();
        budget.release(9);
        assert_eq!(budget.charged_bytes(), 0);
    }

    #[test]
    fn a_journal_entry_charges_its_serialized_length_plus_the_allowance() {
        assert_eq!(journal_entry_charge(0), JOURNAL_ENTRY_OVERHEAD_BYTES);
        assert_eq!(
            journal_entry_charge(100),
            100 + JOURNAL_ENTRY_OVERHEAD_BYTES
        );
    }

    #[test]
    fn an_empty_world_profile_inherits_every_server_ceiling() {
        let hard = RuntimeLimits::default();
        let effective = WorldExecutionProfile::default().resolve(hard).unwrap();
        assert_eq!(effective.native.memory_bytes, hard.native.memory_bytes);
        assert_eq!(effective.native.cpu_seconds, hard.native.cpu_seconds);
        assert_eq!(effective.native.call_deadline, hard.native.call_deadline);
        #[cfg(feature = "sandbox")]
        {
            assert_eq!(effective.sandbox.module_bytes, hard.sandbox.module_bytes);
            assert_eq!(effective.sandbox.fuel, hard.sandbox.fuel);
            assert_eq!(effective.sandbox.memory_bytes, hard.sandbox.memory_bytes);
            assert_eq!(effective.sandbox.message_bytes, hard.sandbox.message_bytes);
        }
    }

    #[test]
    fn a_missing_profile_deserializes_to_inherited_limits() {
        let profile: WorldExecutionProfile = serde_json::from_str("{}").unwrap();
        assert_eq!(profile, WorldExecutionProfile::default());
        assert_eq!(serde_json::to_string(&profile).unwrap(), "{}");
    }

    #[test]
    #[cfg(feature = "sandbox")]
    fn a_world_profile_can_lower_but_never_raise_server_ceilings() {
        let hard = RuntimeLimits::default();
        let effective = WorldExecutionProfile {
            wasm_fuel: Some(hard.sandbox.fuel / 2),
            wasm_memory_bytes: Some((hard.sandbox.memory_bytes / 2) as u64),
            wasm_message_bytes: Some((hard.sandbox.message_bytes / 2) as u64),
            native_memory_bytes: Some(hard.native.memory_bytes / 2),
            native_cpu_seconds: Some(hard.native.cpu_seconds / 2),
            native_call_deadline_ms: Some(
                u64::try_from(hard.native.call_deadline.as_millis() / 2).unwrap(),
            ),
            total_memory_bytes: None,
        }
        .resolve(hard)
        .unwrap();
        assert_eq!(effective.native.memory_bytes, hard.native.memory_bytes / 2);
        assert_eq!(effective.native.cpu_seconds, hard.native.cpu_seconds / 2);
        assert!(effective.native.call_deadline < hard.native.call_deadline);
        #[cfg(feature = "sandbox")]
        {
            assert_eq!(effective.sandbox.fuel, hard.sandbox.fuel / 2);
            assert_eq!(
                effective.sandbox.memory_bytes,
                hard.sandbox.memory_bytes / 2
            );
            assert_eq!(
                effective.sandbox.message_bytes,
                hard.sandbox.message_bytes / 2
            );
            assert_eq!(effective.sandbox.module_bytes, hard.sandbox.module_bytes);
        }
    }

    #[test]
    #[cfg(feature = "sandbox")]
    fn profile_values_above_server_ceilings_are_clamped() {
        let hard = RuntimeLimits::default();
        let effective = WorldExecutionProfile {
            wasm_fuel: Some(u64::MAX),
            wasm_memory_bytes: Some(u64::MAX),
            wasm_message_bytes: Some(u64::MAX),
            native_memory_bytes: Some(u64::MAX),
            native_cpu_seconds: Some(u64::MAX),
            native_call_deadline_ms: Some(u64::MAX),
            total_memory_bytes: None,
        }
        .resolve(hard)
        .unwrap();
        assert_eq!(effective.native.memory_bytes, hard.native.memory_bytes);
        assert_eq!(effective.native.cpu_seconds, hard.native.cpu_seconds);
        assert_eq!(effective.native.call_deadline, hard.native.call_deadline);
        #[cfg(feature = "sandbox")]
        {
            assert_eq!(effective.sandbox.fuel, hard.sandbox.fuel);
            assert_eq!(effective.sandbox.memory_bytes, hard.sandbox.memory_bytes);
            assert_eq!(effective.sandbox.message_bytes, hard.sandbox.message_bytes);
            assert_eq!(effective.sandbox.module_bytes, hard.sandbox.module_bytes);
        }
    }

    #[test]
    fn a_zero_total_budget_request_is_refused() {
        assert!(
            WorldExecutionProfile {
                total_memory_bytes: Some(0),
                ..Default::default()
            }
            .resolve(RuntimeLimits::default())
            .is_err()
        );
    }

    #[test]
    fn the_default_server_ceiling_is_the_proposed_64mib_budget() {
        assert_eq!(
            RuntimeLimits::default().total_memory_bytes,
            DEFAULT_WORLD_TOTAL_MEMORY_BYTES
        );
        assert_eq!(
            DEFAULT_WORLD_TOTAL_MEMORY_BYTES,
            64 * 1024 * 1024,
            "the 64 MiB default is the design's proposed policy value (Q1)"
        );
    }

    #[test]
    fn an_empty_profile_inherits_the_server_total_budget() {
        let hard = RuntimeLimits::default();
        let effective = WorldExecutionProfile::default().resolve(hard).unwrap();
        assert_eq!(effective.total_memory_bytes, hard.total_memory_bytes);
    }

    #[test]
    fn a_total_budget_request_can_lower_but_never_raise_the_ceiling() {
        let hard = RuntimeLimits::default();
        let lowered = WorldExecutionProfile {
            total_memory_bytes: Some(hard.total_memory_bytes / 2),
            ..Default::default()
        }
        .resolve(hard)
        .unwrap();
        assert_eq!(lowered.total_memory_bytes, hard.total_memory_bytes / 2);
        let raised = WorldExecutionProfile {
            total_memory_bytes: Some(u64::MAX),
            ..Default::default()
        }
        .resolve(hard)
        .unwrap();
        assert_eq!(raised.total_memory_bytes, hard.total_memory_bytes);
    }

    #[test]
    fn a_budget_request_leaves_program_ceilings_untouched_in_phase_1() {
        // Phase 1 establishes the validated contract only: no charge site
        // may claim enforcement yet, so program ceilings keep their
        // beneath-the-server-ceiling semantics until Phase 3.
        let hard = RuntimeLimits::default();
        let effective = WorldExecutionProfile {
            total_memory_bytes: Some(hard.total_memory_bytes / 2),
            ..Default::default()
        }
        .resolve(hard)
        .unwrap();
        assert_eq!(effective.native.memory_bytes, hard.native.memory_bytes);
        assert_eq!(effective.native.cpu_seconds, hard.native.cpu_seconds);
        assert_eq!(effective.native.call_deadline, hard.native.call_deadline);
        #[cfg(feature = "sandbox")]
        {
            assert_eq!(effective.sandbox.fuel, hard.sandbox.fuel);
            assert_eq!(effective.sandbox.memory_bytes, hard.sandbox.memory_bytes);
            assert_eq!(effective.sandbox.message_bytes, hard.sandbox.message_bytes);
        }
    }

    #[test]
    fn the_total_budget_round_trips_and_stays_absent_when_unset() {
        let profile = WorldExecutionProfile {
            total_memory_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("total_memory_bytes"));
        let back: WorldExecutionProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
        let inherited: WorldExecutionProfile = serde_json::from_str("{}").unwrap();
        assert_eq!(inherited.total_memory_bytes, None);
    }

    #[test]
    fn zero_and_too_small_memory_requests_are_refused() {
        assert!(
            WorldExecutionProfile {
                wasm_fuel: Some(0),
                ..Default::default()
            }
            .resolve(RuntimeLimits::default())
            .is_err()
        );
        assert!(
            WorldExecutionProfile {
                wasm_memory_bytes: Some(1),
                ..Default::default()
            }
            .resolve(RuntimeLimits::default())
            .is_err()
        );
        assert!(
            WorldExecutionProfile {
                wasm_message_bytes: Some(0),
                ..Default::default()
            }
            .resolve(RuntimeLimits::default())
            .is_err()
        );
        assert!(
            WorldExecutionProfile {
                native_memory_bytes: Some(1),
                ..Default::default()
            }
            .resolve(RuntimeLimits::default())
            .is_err()
        );
        assert!(
            WorldExecutionProfile {
                native_cpu_seconds: Some(0),
                ..Default::default()
            }
            .resolve(RuntimeLimits::default())
            .is_err()
        );
        assert!(
            WorldExecutionProfile {
                native_call_deadline_ms: Some(0),
                ..Default::default()
            }
            .resolve(RuntimeLimits::default())
            .is_err()
        );
    }
}
