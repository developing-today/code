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
}
