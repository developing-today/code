//! Execution bounds for world programs, set by the server's admin.
//!
//! A Wasm guest runs under fuel, memory and payload caps; a native worker
//! runs under address-space and CPU rlimits and a per-call deadline. The
//! defaults are conservative. Raising them is an admin decision, and the
//! values are validated before the server starts.

use std::time::Duration;

use anyhow::{Result, ensure};

use crate::cli::WorldRuntimeArgs;
#[cfg(feature = "sandbox")]
use crate::sandbox::SandboxLimits;
use crate::world_native::NativeLimits;
#[cfg(feature = "sandbox")]
use crate::world_session::MAX_WORLD_MODULE_BYTES;

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
    pub fn from_args(args: &WorldRuntimeArgs) -> Result<Self> {
        ensure!(args.world_fuel > 0, "--world-fuel must be at least 1");
        ensure!(
            (1..=2048).contains(&args.world_memory_mib),
            "--world-memory-mib must be 1..=2048"
        );
        ensure!(
            (1..=1024).contains(&args.world_message_kib),
            "--world-message-kib must be 1..=1024"
        );
        ensure!(
            (16..=65_536).contains(&args.world_native_memory_mib),
            "--world-native-memory-mib must be 16..=65536"
        );
        ensure!(
            args.world_native_cpu_secs > 0,
            "--world-native-cpu-secs must be at least 1"
        );
        ensure!(
            args.world_native_deadline_ms > 0,
            "--world-native-deadline-ms must be at least 1"
        );
        Ok(Self {
            #[cfg(feature = "sandbox")]
            sandbox: SandboxLimits {
                module_bytes: MAX_WORLD_MODULE_BYTES,
                fuel: args.world_fuel,
                memory_bytes: usize::try_from(args.world_memory_mib * MIB)?,
                message_bytes: usize::try_from(args.world_message_kib * KIB)?,
            },
            native: NativeLimits {
                memory_bytes: args.world_native_memory_mib * MIB,
                cpu_seconds: args.world_native_cpu_secs,
                call_deadline: Duration::from_millis(args.world_native_deadline_ms),
            },
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;

    fn parse(extra: &[&str]) -> Result<RuntimeLimits> {
        let mut argv = vec!["id", "serve", "--world", "--world-admin-token", "t"];
        argv.extend_from_slice(extra);
        let cli = Cli::try_parse_from(argv).unwrap();
        let Some(Command::Serve { world_runtime, .. }) = cli.command else {
            panic!("expected serve");
        };
        RuntimeLimits::from_args(&world_runtime)
    }

    #[test]
    fn flag_defaults_match_the_built_in_limits() {
        let limits = parse(&[]).unwrap();
        let defaults = RuntimeLimits::default();
        assert_eq!(limits.native.memory_bytes, defaults.native.memory_bytes);
        assert_eq!(limits.native.cpu_seconds, defaults.native.cpu_seconds);
        assert_eq!(limits.native.call_deadline, defaults.native.call_deadline);
        #[cfg(feature = "sandbox")]
        {
            assert_eq!(limits.sandbox.fuel, defaults.sandbox.fuel);
            assert_eq!(limits.sandbox.memory_bytes, defaults.sandbox.memory_bytes);
            assert_eq!(limits.sandbox.message_bytes, defaults.sandbox.message_bytes);
            assert_eq!(limits.sandbox.module_bytes, defaults.sandbox.module_bytes);
        }
    }

    #[test]
    fn flags_override_each_bound() {
        let limits = parse(&[
            "--world-fuel",
            "5000",
            "--world-memory-mib",
            "32",
            "--world-message-kib",
            "64",
            "--world-native-memory-mib",
            "64",
            "--world-native-cpu-secs",
            "3",
            "--world-native-deadline-ms",
            "250",
        ])
        .unwrap();
        assert_eq!(limits.native.memory_bytes, 64 * MIB);
        assert_eq!(limits.native.cpu_seconds, 3);
        assert_eq!(limits.native.call_deadline, Duration::from_millis(250));
        #[cfg(feature = "sandbox")]
        {
            assert_eq!(limits.sandbox.fuel, 5000);
            assert_eq!(
                limits.sandbox.memory_bytes,
                usize::try_from(32 * MIB).unwrap()
            );
            assert_eq!(
                limits.sandbox.message_bytes,
                usize::try_from(64 * KIB).unwrap()
            );
        }
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        assert!(parse(&["--world-fuel", "0"]).is_err());
        assert!(parse(&["--world-memory-mib", "0"]).is_err());
        assert!(parse(&["--world-memory-mib", "4096"]).is_err());
        assert!(parse(&["--world-message-kib", "2048"]).is_err());
        assert!(parse(&["--world-native-memory-mib", "8"]).is_err());
        assert!(parse(&["--world-native-cpu-secs", "0"]).is_err());
        assert!(parse(&["--world-native-deadline-ms", "0"]).is_err());
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
