//! Relay operations (P3.8, N25, P7.1): the systemd unit and the configs in the
//! relay kit must stop the relay cleanly and keep its metrics off the public
//! interface, in both modes the kit supports.
//!
//! `relay.kit.toml` has one table per mode. `tunnel` is a Cloudflare Tunnel
//! (no inbound port, plain HTTP on loopback). `vps` is a public address (TLS on
//! 80 and 443, QUIC address discovery). A mode's unit is the shared
//! `iroh-relay.service` plus the mode's drop-in, if it has one. The kit lives in
//! the design doc, so the paths are relative to this crate.
//!
//! Each check runs on the kit's real files, and also on mutated copies, so a
//! check that accepts everything fails.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{net::SocketAddr, path::Path};

const KIT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../doc/2026-10-11T03-44-33Z_design_groups_events_delegation/connectivity/relay"
);

const KIT_FILE: &str = "relay.kit.toml";
const UNIT: &str = "iroh-relay.service";
/// What both modes must run. The config path is where the unit's
/// `ConditionPathExists` looks, so a missing config stops the start.
const EXEC_START: &str = "/usr/local/bin/iroh-relay --config-path /etc/iroh-relay/relay.toml";

fn kit_file(name: &str) -> String {
    let path = Path::new(KIT).join(name);
    std::fs::read_to_string(&path).expect("the relay kit is readable")
}

/// One mode from `relay.kit.toml`.
#[derive(Debug)]
struct Mode {
    name: String,
    /// The committed relay config for this mode, in the kit.
    config: String,
    hostname: String,
    http_bind_addr: String,
    /// Set only when the mode serves TLS on a public port.
    https_bind_addr: Option<String>,
    metrics_bind_addr: String,
    /// QUIC address discovery on 7842/udp.
    qad: bool,
    /// The unit drop-in this mode installs, if it needs one.
    unit_dropin: Option<String>,
}

impl Mode {
    /// Whether the mode binds a port below 1024, which needs `CAP_NET_BIND_SERVICE`.
    fn binds_privileged_port(&self) -> bool {
        [Some(&self.http_bind_addr), self.https_bind_addr.as_ref()]
            .into_iter()
            .flatten()
            .any(|bind| {
                bind.parse::<SocketAddr>()
                    .expect("a kit bind address")
                    .port()
                    < 1024
            })
    }
}

/// Every mode in `relay.kit.toml`, in name order.
fn kit_modes() -> Vec<Mode> {
    let kit: toml::Table = toml::from_str(&kit_file(KIT_FILE)).expect("the kit is TOML");
    let modes = kit
        .get("modes")
        .and_then(toml::Value::as_table)
        .expect("the kit has a [modes] table");
    modes
        .iter()
        .map(|(name, value)| {
            let table = value.as_table().expect("a mode is a table");
            let text = |key: &str| {
                table
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            };
            Mode {
                name: name.clone(),
                config: text("config").expect("a mode names its config"),
                hostname: text("hostname").expect("a mode has a hostname"),
                http_bind_addr: text("http_bind_addr").expect("a mode has an http bind"),
                https_bind_addr: text("https_bind_addr"),
                metrics_bind_addr: text("metrics_bind_addr").expect("a mode has metrics"),
                qad: table
                    .get("qad")
                    .and_then(toml::Value::as_bool)
                    .expect("a mode says whether QAD is on"),
                unit_dropin: text("unit_dropin"),
            }
        })
        .collect()
}

/// The unit a host gets for `mode`: the shared unit, then the mode's drop-in.
/// systemd reads a drop-in's keys as part of the unit's sections.
fn rendered_unit(mode: &Mode) -> String {
    let mut unit = kit_file(UNIT);
    if let Some(dropin) = &mode.unit_dropin {
        unit.push_str(&kit_file(dropin));
    }
    unit
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

/// The value systemd takes for `key` in `section`: the last assignment wins.
fn unit_value(unit: &str, section: &str, key: &str) -> Option<String> {
    unit_entries(unit)
        .into_iter()
        .rev()
        .find(|(s, k, _)| s == section && k == key)
        .map(|(_, _, value)| value)
}

/// Checks the unit: the stop signal is SIGINT, and the config file must exist
/// before the relay starts (no config means silent defaults, S11).
fn check_unit(unit: &str) -> Result<(), String> {
    match unit_value(unit, "Service", "KillSignal").as_deref() {
        Some("SIGINT") => {}
        found => {
            return Err(format!(
                "[Service] KillSignal must be SIGINT, found {found:?}"
            ));
        }
    }

    let condition = unit_value(unit, "Unit", "ConditionPathExists")
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "[Unit] has no ConditionPathExists for the config".to_owned())?;
    let exec = unit_value(unit, "Service", "ExecStart")
        .ok_or_else(|| "[Service] has no ExecStart".to_owned())?;
    let tokens: Vec<&str> = exec.split_whitespace().collect();
    let config = tokens
        .windows(2)
        .find(|pair| pair[0] == "--config-path")
        .map(|pair| pair[1]);
    if config != Some(condition.as_str()) {
        return Err(format!(
            "ConditionPathExists {condition} does not match ExecStart --config-path {config:?}"
        ));
    }
    Ok(())
}

/// Checks a mode's rendered unit: the shared checks, the exact `ExecStart`, and the
/// capability. The unit grants `CAP_NET_BIND_SERVICE` if and only if the mode binds
/// a port below 1024, so tunnel mode (no privileged port) grants nothing.
fn check_mode_unit(unit: &str, mode: &Mode) -> Result<(), String> {
    check_unit(unit)?;
    let exec = unit_value(unit, "Service", "ExecStart");
    if exec.as_deref() != Some(EXEC_START) {
        return Err(format!(
            "[Service] ExecStart must be {EXEC_START:?}, found {exec:?}"
        ));
    }
    let grants = unit_entries(unit).iter().any(|(_, key, value)| {
        matches!(
            key.as_str(),
            "AmbientCapabilities" | "CapabilityBoundingSet"
        ) && value
            .split_whitespace()
            .any(|cap| cap == "CAP_NET_BIND_SERVICE")
    });
    let needed = mode.binds_privileged_port();
    if grants != needed {
        return Err(format!(
            "{}: binds a port below 1024 is {needed}, but the unit grants CAP_NET_BIND_SERVICE is {grants}",
            mode.name
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

/// Checks a mode's relay config against the kit's values for that mode: the
/// same binds, the same QAD setting, and TLS only where the kit serves it. A mode
/// with no public port must bind loopback only, which is the no-inbound-port rule.
fn check_mode_config(config: &str, mode: &Mode) -> Result<(), String> {
    check_config(config)?;
    let table: toml::Table =
        toml::from_str(config).map_err(|err| format!("not valid TOML: {err}"))?;
    let text = |key: &str| table.get(key).and_then(toml::Value::as_str);

    if text("metrics_bind_addr") != Some(mode.metrics_bind_addr.as_str()) {
        return Err(format!(
            "metrics_bind_addr must be {}",
            mode.metrics_bind_addr
        ));
    }
    if text("http_bind_addr") != Some(mode.http_bind_addr.as_str()) {
        return Err(format!("http_bind_addr must be {}", mode.http_bind_addr));
    }
    let qad = table
        .get("enable_quic_addr_discovery")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    if qad != mode.qad {
        return Err(format!("enable_quic_addr_discovery must be {}", mode.qad));
    }

    match (table.get("tls"), &mode.https_bind_addr) {
        (None, None) => {}
        (Some(tls), Some(bind)) => {
            let tls = tls.as_table().ok_or("[tls] is not a table")?;
            if tls.get("https_bind_addr").and_then(toml::Value::as_str) != Some(bind.as_str()) {
                return Err(format!("[tls] https_bind_addr must be {bind}"));
            }
            if tls.get("hostname").and_then(toml::Value::as_str) != Some(mode.hostname.as_str()) {
                return Err(format!("[tls] hostname must be {}", mode.hostname));
            }
        }
        (Some(_), None) => {
            return Err("the mode serves no TLS, but the config has [tls]".to_owned());
        }
        (None, Some(bind)) => {
            return Err(format!(
                "the mode serves TLS on {bind}, but the config has no [tls]"
            ));
        }
    }

    if mode.https_bind_addr.is_none() {
        for (key, value) in &table {
            if !key.ends_with("_bind_addr") {
                continue;
            }
            let bind = value
                .as_str()
                .ok_or_else(|| format!("{key} is not a string"))?;
            let addr: SocketAddr = bind
                .parse()
                .map_err(|err| format!("{key} {bind:?} is not an address: {err}"))?;
            if !addr.ip().is_loopback() {
                return Err(format!("the mode has no public port, but {key} is {bind}"));
            }
        }
    }
    Ok(())
}

fn mode(name: &str) -> Mode {
    kit_modes()
        .into_iter()
        .find(|mode| mode.name == name)
        .expect("the kit has the mode")
}

#[test]
fn test_kit_has_the_tunnel_and_vps_modes() {
    let names: Vec<String> = kit_modes().into_iter().map(|mode| mode.name).collect();
    assert_eq!(names, ["tunnel", "vps"]);
}

#[test]
fn test_both_modes_render_the_same_exec_start() {
    for mode in kit_modes() {
        let exec = unit_value(&rendered_unit(&mode), "Service", "ExecStart");
        assert_eq!(exec.as_deref(), Some(EXEC_START), "{}", mode.name);
    }
}

#[test]
fn test_both_modes_render_a_unit_that_stops_on_sigint_and_requires_its_config() {
    for mode in kit_modes() {
        let result = check_unit(&rendered_unit(&mode));
        assert!(result.is_ok(), "{}: {result:?}", mode.name);
    }
}

#[test]
fn test_each_mode_unit_grants_the_capability_its_ports_need() {
    for mode in kit_modes() {
        let result = check_mode_unit(&rendered_unit(&mode), &mode);
        assert!(result.is_ok(), "{}: {result:?}", mode.name);
    }
}

#[test]
fn test_both_modes_bind_metrics_to_loopback() {
    for mode in kit_modes() {
        let config = kit_file(&mode.config);
        let result = check_config(&config);
        assert!(result.is_ok(), "{}: {result:?}", mode.config);
    }
}

#[test]
fn test_each_committed_config_matches_its_mode() {
    for mode in kit_modes() {
        let result = check_mode_config(&kit_file(&mode.config), &mode);
        assert!(result.is_ok(), "{}: {result:?}", mode.config);
    }
}

#[test]
fn test_tunnel_mode_has_no_inbound_port() {
    let tunnel = mode("tunnel");
    assert!(tunnel.https_bind_addr.is_none(), "no TLS listener");
    assert!(!tunnel.qad, "no UDP for QAD");
    assert!(!tunnel.binds_privileged_port(), "no port below 1024");
    let unit = rendered_unit(&tunnel);
    assert!(!unit.contains("CAP_NET_BIND_SERVICE"), "{unit}");
    let result = check_mode_config(&kit_file(&tunnel.config), &tunnel);
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn test_vps_mode_serves_tls_and_qad_on_public_ports() {
    let vps = mode("vps");
    assert_eq!(vps.https_bind_addr.as_deref(), Some("[::]:443"));
    assert!(vps.qad, "QAD on 7842/udp");
    assert!(vps.binds_privileged_port(), "ports 80 and 443");
    let unit = rendered_unit(&vps);
    assert!(
        unit.contains("AmbientCapabilities=CAP_NET_BIND_SERVICE"),
        "{unit}"
    );
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
fn test_mode_unit_check_refuses_dev_mode_in_place_of_the_config() {
    let vps = mode("vps");
    let unit = mutate(
        &rendered_unit(&vps),
        "--config-path /etc/iroh-relay/relay.toml",
        "--dev",
    );
    assert!(check_mode_unit(&unit, &vps).is_err());
}

#[test]
fn test_tunnel_unit_check_refuses_a_capability() {
    let tunnel = mode("tunnel");
    let unit = mutate(
        &rendered_unit(&tunnel),
        "[Service]\n",
        "[Service]\nAmbientCapabilities=CAP_NET_BIND_SERVICE\n",
    );
    assert!(check_mode_unit(&unit, &tunnel).is_err());
}

#[test]
fn test_vps_unit_check_refuses_a_missing_capability() {
    let vps = mode("vps");
    let unit = mutate(
        &rendered_unit(&vps),
        "AmbientCapabilities=CAP_NET_BIND_SERVICE",
        "AmbientCapabilities=CAP_CHOWN",
    );
    let unit = mutate(
        &unit,
        "CapabilityBoundingSet=CAP_NET_BIND_SERVICE",
        "CapabilityBoundingSet=CAP_CHOWN",
    );
    assert!(check_mode_unit(&unit, &vps).is_err());
}

#[test]
fn test_tunnel_config_check_refuses_a_public_bind() {
    let tunnel = mode("tunnel");
    let config = mutate(
        &kit_file(&tunnel.config),
        "http_bind_addr = \"127.0.0.1:3340\"",
        "http_bind_addr = \"[::]:3340\"",
    );
    assert!(check_mode_config(&config, &tunnel).is_err());
}

#[test]
fn test_tunnel_config_check_refuses_tls() {
    let tunnel = mode("tunnel");
    let config = format!(
        "{}\n[tls]\nhttps_bind_addr = \"[::]:443\"\nhostname = \"relay.example.org\"\n",
        kit_file(&tunnel.config)
    );
    assert!(check_mode_config(&config, &tunnel).is_err());
}

#[test]
fn test_vps_config_check_refuses_a_missing_tls_table() {
    let vps = mode("vps");
    let config = mutate(&kit_file(&vps.config), "[tls]\n", "");
    assert!(check_mode_config(&config, &vps).is_err());
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
