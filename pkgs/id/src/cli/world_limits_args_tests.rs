//! `RuntimeLimits::from_args` flag tests (see the `mod` declaration in
//! [`super`] for the history).

use std::time::Duration;

use anyhow::Result;
use clap::Parser;

use super::{Cli, Command, ServeArgs};
use crate::world_limits::RuntimeLimits;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

fn parse(extra: &[&str]) -> Result<RuntimeLimits> {
    let mut argv = vec!["id", "serve", "--world", "--world-admin-token", "t"];
    argv.extend_from_slice(extra);
    let cli = Cli::try_parse_from(argv).unwrap();
    let Some(Command::Serve(args)) = cli.command else {
        panic!("expected serve");
    };
    let ServeArgs { world_runtime, .. } = args;
    RuntimeLimits::from_args(&world_runtime)
}

#[test]
fn flag_defaults_match_the_built_in_limits() {
    let limits = parse(&[]).unwrap();
    let defaults = RuntimeLimits::default();
    assert_eq!(limits.native.memory_bytes, defaults.native.memory_bytes);
    assert_eq!(limits.native.cpu_seconds, defaults.native.cpu_seconds);
    assert_eq!(limits.native.call_deadline, defaults.native.call_deadline);
    assert_eq!(
        limits.total_memory_bytes, defaults.total_memory_bytes,
        "--world-total-memory-mib default must match the built-in budget"
    );
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
        "--world-total-memory-mib",
        "128",
    ])
    .unwrap();
    assert_eq!(limits.native.memory_bytes, 64 * MIB);
    assert_eq!(limits.native.cpu_seconds, 3);
    assert_eq!(limits.native.call_deadline, Duration::from_millis(250));
    assert_eq!(limits.total_memory_bytes, 128 * MIB);
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
    assert!(parse(&["--world-total-memory-mib", "0"]).is_err());
    assert!(parse(&["--world-total-memory-mib", "131072"]).is_err());
}
