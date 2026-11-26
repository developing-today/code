//! Fleet context propagation build 2/5 — prove it end to end
//! (developing-today/code#14188, decision record for #10357).
//!
//! One context id travels laptop-agent exec -> SSH hop -> queue broker
//! trip, byte-identical at each leg's log, with no model in the loop:
//!
//! 1. **exec leg**: the laptop-agent client's real spec builder
//!    (`command_spec_with_ctx`) puts the record in the spec's `env`;
//!    the command is launched the way the daemon launches it (argv
//!    mode, spec env merged) — here on top of an *empty* environment,
//!    so the record can only have arrived through the spec carrier.
//!    The leg logs `exec <fields>`.
//! 2. **SSH hop**: the exec leg renders its remote line with the same
//!    `shell_prefix` `fleet ssh exec` uses and hands it to `/bin/sh -c`
//!    under a scrubbed environment (what sshd gives the remote login
//!    shell). The remote side logs `ssh <fields>`.
//! 3. **queue broker trip**: the remote side sends a request with the
//!    record as `X-Fleet-Ctx-*` headers (the queue client's direct-leg
//!    carrier) to a broker served by the real `uds_http` server, whose
//!    handler logs the real `ctx_trip_line` the queue and secrets
//!    brokers emit.
//!
//! Each leg is a separate process (this test binary re-executed in a
//! helper mode), so nothing is shared in memory between legs.
#![cfg(all(unix, feature = "laptop-agent", feature = "queue-broker"))]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fleet_config::ctx::Ctx;

const HELPER: &str = "ctx_e2e_helper";
const MODE: &str = "FLEET_CTX_E2E_MODE";
const LOG: &str = "FLEET_CTX_E2E_LOG";
const SOCKET: &str = "FLEET_CTX_E2E_SOCKET";

fn append(log: &Path, line: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .expect("open leg log");
    writeln!(f, "{line}").expect("write leg log");
}

fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn helper_argv() -> Vec<String> {
    let exe = std::env::current_exe().expect("test binary path");
    vec![
        exe.to_string_lossy().into_owned(),
        "--exact".into(),
        HELPER.into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ]
}

/// The legs. A no-op in a normal test run (no mode set).
#[test]
fn ctx_e2e_helper() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let log = PathBuf::from(std::env::var(LOG).expect("leg log path"));
    let socket = std::env::var(SOCKET).expect("broker socket path");
    let ctx = Ctx::from_env();
    match mode.as_str() {
        "exec" => {
            append(&log, &format!("exec {}", ctx.log_fields()));
            // The SSH hop: `fleet ssh exec`'s rendering (context
            // prefix, then the command), run by a shell that inherits
            // nothing — the test plumbing is exported on the line too,
            // as it would have to be over a real ssh.
            let argv: Vec<String> = helper_argv().iter().map(|a| sq(a)).collect();
            let inner = format!(
                "export {MODE}=ssh {LOG}={} {SOCKET}={}; exec {}",
                sq(&log.to_string_lossy()),
                sq(&socket),
                argv.join(" ")
            );
            let line = format!("{}{inner}", ctx.shell_prefix());
            let status = Command::new("/bin/sh")
                .arg("-c")
                .arg(&line)
                .env_clear()
                .status()
                .expect("ssh-hop shell runs");
            assert!(status.success(), "ssh hop failed: {status}");
        }
        "ssh" => {
            append(&log, &format!("ssh {}", ctx.log_fields()));
            // The broker trip, carried as the queue client's direct
            // legs carry it: X-Fleet-Ctx-* header lines.
            let body = b"{}";
            let head = format!(
                "POST /claim HTTP/1.1\r\nHost: localhost\r\nX-Agent-Token: e2e\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                ctx.header_lines(),
                body.len()
            );
            let mut s = UnixStream::connect(&socket).expect("connect broker");
            s.set_read_timeout(Some(Duration::from_secs(30))).ok();
            s.write_all(head.as_bytes()).expect("write head");
            s.write_all(body).expect("write body");
            let mut raw = Vec::new();
            s.read_to_end(&mut raw).expect("read reply");
            let reply = String::from_utf8_lossy(&raw);
            assert!(reply.starts_with("HTTP/1.1 200"), "broker reply: {reply}");
        }
        other => panic!("unknown helper mode {other}"),
    }
}

/// Field rendering after a leg's prefix (`exec `, `ssh `, or the
/// broker's `trip POST /claim status=200 `).
fn fields_after<'a>(log: &'a str, prefix: &str) -> &'a str {
    log.lines()
        .find_map(|l| l.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no {prefix:?} line in leg log:\n{log}"))
}

fn id_of(fields: &str) -> &str {
    fields
        .split(' ')
        .find_map(|f| f.strip_prefix("FLEET_CTX_ID="))
        .unwrap_or_else(|| panic!("no FLEET_CTX_ID in {fields:?}"))
}

#[test]
fn ctx_propagation_end_to_end_exec_ssh_broker_byte_identical() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("fleet-ctx-e2e-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test dir");
    let log = dir.join("legs.log");
    let socket = dir.join("broker.sock");

    // The broker: the real uds_http server, logging the real per-trip
    // context line both brokers emit.
    let listener = fleet_queue::uds_http::bind_socket(&socket)
        .expect("bind broker socket")
        .expect("socket is free");
    let broker_log = log.clone();
    std::thread::spawn(move || {
        fleet_queue::uds_http::serve(listener, move |req| {
            if let Some(line) = fleet_queue::uds_http::ctx_trip_line(req, 200) {
                append(&broker_log, &format!("queue-broker: {line}"));
            }
            (200, b"{}".to_vec())
        });
    });

    // The record a queue-launched session would carry.
    let ctx = Ctx::from_pairs([
        ("FLEET_CTX_ID", "task-22655"),
        ("FLEET_CTX_ORIGIN", "systemd-unit:opencode"),
        ("FLEET_CTX_HOST", "amd"),
        ("FLEET_CTX_TASK_ID", "22655"),
        ("FLEET_CTX_ANCESTRY", "cron/muse/opencode"),
        ("FLEET_CTX_GIT_HEADS", "hatch=0a1b2c,work=9f8e7d"),
    ]);
    let want = ctx.log_fields();

    // Leg 1: the laptop-agent exec spec, launched as the daemon does
    // (argv mode, spec env merged) over an empty environment.
    let argv = helper_argv();
    let spec = fleet_laptop_agent::client::command_spec_with_ctx(&ctx, None, false, &argv);
    let spec_env = spec["env"].as_object().expect("spec carries env");
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .env_clear()
        .envs(
            spec_env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str().expect("string env"))),
        )
        .env(MODE, "exec")
        .env(LOG, &log)
        .env(SOCKET, &socket)
        .status()
        .expect("exec leg runs");
    assert!(status.success(), "exec leg failed: {status}");

    let legs = std::fs::read_to_string(&log).expect("leg log written");
    println!("--- leg log (#14188 proof) ---\n{legs}------------------------------");
    let exec = fields_after(&legs, "exec ");
    let ssh = fields_after(&legs, "ssh ");
    let broker = fields_after(&legs, "queue-broker: trip POST /claim status=200 ");

    // Byte-identical at every leg: the id, and the whole record.
    for (leg, fields) in [("exec", exec), ("ssh", ssh), ("broker", broker)] {
        assert_eq!(id_of(fields).as_bytes(), b"task-22655", "{leg}");
        assert_eq!(fields, want, "{leg} leg record differs");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
