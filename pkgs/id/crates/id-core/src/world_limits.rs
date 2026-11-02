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
}

impl WorldExecutionProfile {
    /// Validate values and resolve them beneath the server's hard limits.
    ///
    /// A request may lower a bound or inherit it, but can never raise the
    /// server ceiling. The Wasm module-size cap is always taken from `hard`.
    ///
    /// # Errors
    ///
    /// A requested bound of zero or a value that cannot be represented on
    /// this host.
    pub fn resolve(self, hard: RuntimeLimits) -> Result<RuntimeLimits> {
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
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

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
