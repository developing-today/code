//! fleet-udp-forward — the CONNECT-UDP client shim (#6109):
//! local UDP in, CONNECT-UDP sessions out, replies back. The
//! piece a real consumer (Moonlight) needs in front of it,
//! since it speaks plain UDP and nothing else.
//!
//! Two ways to name the forwards:
//!
//! * `--target <host> --ports <set>` — port-preserving over a
//!   set (`47998-48010` is the Moonlight shape): local port P
//!   forwards to <host>:P.
//! * `--map <lport>=<host>:<rport>` (repeatable) — explicit
//!   one-off mappings.
//!
//! Everything binds `--bind` (default 127.0.0.1) and talks to
//! the CONNECT-UDP endpoint at `--proxy` (default
//! 127.0.0.1:3132). Config-gated in the only way a shim can
//! be: nothing runs it unless a host deliberately starts it.

use std::net::{IpAddr, SocketAddr};
use std::process::ExitCode;

use fleet_proxy::EXIT_USAGE;
use fleet_proxy::udp_forward::{Forward, parse_map, parse_port_set};

const DEFAULT_PROXY: &str = "127.0.0.1:3132";
const USAGE: &str = "usage: fleet-udp-forward [--proxy <addr>] [--bind <ip>] \
                     (--map <lport>=<host>:<rport>... | --target <host> --ports <set>)";

fn fail(msg: &str) -> ExitCode {
    eprintln!("fleet-udp-forward: {msg}");
    eprintln!("{USAGE}");
    ExitCode::from(EXIT_USAGE as u8)
}

fn take(args: &[String], i: &mut usize, flag: &str) -> Result<String, ExitCode> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| fail(&format!("{flag} needs a value")))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut proxy = DEFAULT_PROXY.to_string();
    let mut bind: IpAddr = IpAddr::from([127, 0, 0, 1]);
    let mut target: Option<String> = None;
    let mut ports: Option<String> = None;
    let mut maps: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--proxy" => match take(&argv, &mut i, "--proxy") {
                Ok(v) => proxy = v,
                Err(c) => return c,
            },
            "--bind" => match take(&argv, &mut i, "--bind") {
                Ok(v) => match v.parse() {
                    Ok(ip) => bind = ip,
                    Err(_) => return fail(&format!("--bind {v:?} is not an IP address")),
                },
                Err(c) => return c,
            },
            "--target" => match take(&argv, &mut i, "--target") {
                Ok(v) => target = Some(v),
                Err(c) => return c,
            },
            "--ports" => match take(&argv, &mut i, "--ports") {
                Ok(v) => ports = Some(v),
                Err(c) => return c,
            },
            "--map" => match take(&argv, &mut i, "--map") {
                Ok(v) => maps.push(v),
                Err(c) => return c,
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => return fail(&format!("unknown argument {other:?}")),
        }
        i += 1;
    }

    let mut forwards: Vec<Forward> = Vec::new();
    for m in &maps {
        match parse_map(m, bind) {
            Ok(f) => forwards.push(f),
            Err(e) => return fail(&e),
        }
    }
    match (target, ports) {
        (Some(host), Some(spec)) => match parse_port_set(&spec) {
            Ok(ps) => {
                for p in ps {
                    forwards.push(Forward {
                        listen: SocketAddr::new(bind, p),
                        target_host: host.clone(),
                        target_port: p,
                    });
                }
            }
            Err(e) => return fail(&e),
        },
        (None, None) => {}
        _ => return fail("--target and --ports must be given together"),
    }
    if forwards.is_empty() {
        return fail("nothing to forward: give --map and/or --target + --ports");
    }
    match fleet_proxy::udp_forward::run_forwards(&proxy, &forwards) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fleet-udp-forward: {e}");
            ExitCode::FAILURE
        }
    }
}
