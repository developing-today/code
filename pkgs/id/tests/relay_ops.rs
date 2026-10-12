//! Relay operations (P3.8, N25): the systemd unit and the configs in the relay
//! kit must stop the relay cleanly and keep its metrics off the public
//! interface. The kit lives in the design doc, so the paths are relative to
//! this crate.
//!
//! Each check runs on the kit's real files, and also on mutated copies, so a
//! check that accepts everything fails.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{net::SocketAddr, path::Path};

const KIT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../doc/2026-10-11T03-44-33Z_design_groups_events_delegation/connectivity/relay"
);

const UNIT: &str = "iroh-relay.service";
const CONFIGS: [&str; 2] = ["tunnel.toml", "vps.toml"];

fn kit_file(name: &str) -> String {
    let path = Path::new(KIT).join(name);
    std::fs::read_to_string(&path).expect("the relay kit is readable")
}

/// Returns `text` with `from` replaced by `to`. Fails if `from` is absent, so a
/// mutation that changes nothing cannot pass as a negative test.
fn mutate(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "the kit has no line {from:?}");
    text.replacen(from, to, 1)
}

/// The `(section, key, value)` assignments in a systemd unit, comments skipped.
fn unit_entries(unit: &str) -> Vec<(String, String, String)> {
    let mut section = String::new();
    let mut entries = Vec::new();
    for line in unit.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            name.clone_into(&mut section);
        } else if let Some((key, value)) = line.split_once('=') {
            entries.push((
                section.clone(),
                key.trim().to_owned(),
                value.trim().to_owned(),
            ));
        }
    }
    entries
}

/// Checks the unit: the stop signal is SIGINT, and the config file must exist
/// before the relay starts (no config means silent defaults, S11).
fn check_unit(unit: &str) -> Result<(), String> {
    let entries = unit_entries(unit);
    // systemd takes the last assignment of a key.
    let value = |section: &str, key: &str| {
        entries
            .iter()
            .rev()
            .find(|(s, k, _)| s == section && k == key)
            .map(|(_, _, v)| v.as_str())
    };

    match value("Service", "KillSignal") {
        Some("SIGINT") => {}
        found => {
            return Err(format!(
                "[Service] KillSignal must be SIGINT, found {found:?}"
            ));
        }
    }

    let condition = value("Unit", "ConditionPathExists")
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "[Unit] has no ConditionPathExists for the config".to_owned())?;
    let exec =
        value("Service", "ExecStart").ok_or_else(|| "[Service] has no ExecStart".to_owned())?;
    let tokens: Vec<&str> = exec.split_whitespace().collect();
    let config = tokens
        .windows(2)
        .find(|pair| pair[0] == "--config-path")
        .map(|pair| pair[1]);
    if config != Some(condition) {
        return Err(format!(
            "ConditionPathExists {condition} does not match ExecStart --config-path {config:?}"
        ));
    }
    Ok(())
}

/// Checks a relay config: metrics must be set, and bound to a loopback address.
/// Unset, the relay binds metrics to the HTTP address on port 9090, which is public.
fn check_config(config: &str) -> Result<(), String> {
    let table: toml::Table =
        toml::from_str(config).map_err(|err| format!("not valid TOML: {err}"))?;
    let bind = table
        .get("metrics_bind_addr")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            "metrics_bind_addr is not set, so metrics default to a public port".to_owned()
        })?;
    let addr: SocketAddr = bind
        .parse()
        .map_err(|err| format!("metrics_bind_addr {bind:?} is not an address: {err}"))?;
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(format!("metrics_bind_addr {bind} is not loopback"))
    }
}

#[test]
fn test_kit_unit_file_stops_on_sigint_and_requires_its_config() {
    let result = check_unit(&kit_file(UNIT));
    assert!(result.is_ok(), "{UNIT}: {result:?}");
}

#[test]
fn test_kit_configs_bind_metrics_to_loopback() {
    for name in CONFIGS {
        let result = check_config(&kit_file(name));
        assert!(result.is_ok(), "{name}: {result:?}");
    }
}

#[test]
fn test_unit_check_refuses_sigterm() {
    let unit = mutate(&kit_file(UNIT), "KillSignal=SIGINT", "KillSignal=SIGTERM");
    assert!(check_unit(&unit).is_err());
}

#[test]
fn test_unit_check_refuses_the_default_kill_signal() {
    let unit = mutate(&kit_file(UNIT), "KillSignal=SIGINT\n", "");
    assert!(check_unit(&unit).is_err());
}

#[test]
fn test_unit_check_refuses_a_missing_config_condition() {
    let unit = mutate(
        &kit_file(UNIT),
        "ConditionPathExists=/etc/iroh-relay/relay.toml\n",
        "",
    );
    assert!(check_unit(&unit).is_err());
}

#[test]
fn test_unit_check_refuses_a_condition_on_another_path() {
    let unit = mutate(
        &kit_file(UNIT),
        "ConditionPathExists=/etc/iroh-relay/relay.toml",
        "ConditionPathExists=/etc/iroh-relay/other.toml",
    );
    assert!(check_unit(&unit).is_err());
}

#[test]
fn test_config_check_refuses_public_metrics() {
    let config = kit_file("tunnel.toml");
    let public = mutate(
        &config,
        "metrics_bind_addr = \"127.0.0.1:9090\"",
        "metrics_bind_addr = \"[::]:9090\"",
    );
    assert!(check_config(&public).is_err());
    let any_v4 = mutate(
        &config,
        "metrics_bind_addr = \"127.0.0.1:9090\"",
        "metrics_bind_addr = \"0.0.0.0:9090\"",
    );
    assert!(check_config(&any_v4).is_err());
}

#[test]
fn test_config_check_refuses_unset_metrics() {
    let unset = mutate(
        &kit_file("vps.toml"),
        "metrics_bind_addr = \"127.0.0.1:9090\"\n",
        "",
    );
    assert!(check_config(&unset).is_err());
}

#[test]
fn test_config_check_accepts_loopback_ipv6() {
    let config = mutate(
        &kit_file("vps.toml"),
        "metrics_bind_addr = \"127.0.0.1:9090\"",
        "metrics_bind_addr = \"[::1]:9090\"",
    );
    check_config(&config).unwrap();
}
