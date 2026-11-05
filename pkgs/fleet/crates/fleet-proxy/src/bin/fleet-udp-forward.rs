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
//! The scalar flags (`--proxy`, `--bind`, `--target`, `--ports`)
//! may each be given at most once: a repeat is a usage error,
//! never a silent last-wins overwrite (#6139). `--map` is the
//! one repeatable flag.
//!
//! Everything binds `--bind` (default 127.0.0.1) and talks to
//! the CONNECT-UDP endpoint at `--proxy` (default
//! 127.0.0.1:3131). Config-gated in the only way a shim can
//! be: nothing runs it unless a host deliberately starts it.

use std::net::{IpAddr, SocketAddr};
use std::process::ExitCode;

use fleet_proxy::EXIT_USAGE;
use fleet_proxy::udp_forward::{Forward, parse_map, parse_port_set};

const DEFAULT_PROXY: &str = "127.0.0.1:3131";
const USAGE: &str = "usage: fleet-udp-forward [--proxy <addr>] [--bind <ip>] \
                     (--map <lport>=<host>:<rport>... | --target <host> --ports <set>)";

fn fail(msg: &str) -> ExitCode {
    eprintln!("fleet-udp-forward: {msg}");
    eprintln!("{USAGE}");
    ExitCode::from(EXIT_USAGE as u8)
}

fn take(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// One parsed invocation. `proxy`/`bind` stay `Option` so `main`
/// applies the defaults; `help` records a `-h`/`--help` seen
/// before any error, which `main` answers with the usage text.
#[derive(Debug)]
struct ParsedArgs {
    proxy: Option<String>,
    bind: Option<IpAddr>,
    target: Option<String>,
    ports: Option<String>,
    maps: Vec<String>,
    help: bool,
}

/// Pure core of argument parsing, returning the usage error as a
/// string instead of exiting so the rejection shapes are
/// unit-testable (mirrors fleet-queue's `parse_args_inner`, #5939).
///
/// Duplicate scalar flags are usage errors, never silent
/// last-wins: repeating a scalar flag used to retarget an
/// invocation (e.g. `--target a --target b` forwarded to `b`,
/// the first value silently discarded, and a duplicate
/// `--proxy` silently re-pointed the CONNECT-UDP endpoint —
/// #6139). `--map` is repeatable by design and accumulates.
/// `-h`/`--help` short-circuits exactly where it appears, as
/// the inline loop did before this extraction.
fn parse_args_inner(argv: &[String]) -> Result<ParsedArgs, String> {
    let mut proxy: Option<String> = None;
    let mut bind: Option<IpAddr> = None;
    let mut target: Option<String> = None;
    let mut ports: Option<String> = None;
    let mut maps: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--proxy" => {
                let v = take(argv, &mut i, "--proxy")?;
                if proxy.replace(v).is_some() {
                    return Err("argument --proxy: duplicate flag".to_string());
                }
            }
            "--bind" => {
                let v = take(argv, &mut i, "--bind")?;
                let ip: IpAddr = v
                    .parse()
                    .map_err(|_| format!("--bind {v:?} is not an IP address"))?;
                if bind.replace(ip).is_some() {
                    return Err("argument --bind: duplicate flag".to_string());
                }
            }
            "--target" => {
                let v = take(argv, &mut i, "--target")?;
                if target.replace(v).is_some() {
                    return Err("argument --target: duplicate flag".to_string());
                }
            }
            "--ports" => {
                let v = take(argv, &mut i, "--ports")?;
                if ports.replace(v).is_some() {
                    return Err("argument --ports: duplicate flag".to_string());
                }
            }
            "--map" => maps.push(take(argv, &mut i, "--map")?),
            "-h" | "--help" => {
                return Ok(ParsedArgs {
                    proxy,
                    bind,
                    target,
                    ports,
                    maps,
                    help: true,
                });
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    Ok(ParsedArgs {
        proxy,
        bind,
        target,
        ports,
        maps,
        help: false,
    })
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse_args_inner(&argv) {
        Ok(p) => p,
        Err(msg) => return fail(&msg),
    };
    if parsed.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let proxy = parsed.proxy.unwrap_or_else(|| DEFAULT_PROXY.to_string());
    let bind = parsed.bind.unwrap_or_else(|| IpAddr::from([127, 0, 0, 1]));

    let mut forwards: Vec<Forward> = Vec::new();
    for m in &parsed.maps {
        match parse_map(m, bind) {
            Ok(f) => forwards.push(f),
            Err(e) => return fail(&e),
        }
    }
    match (parsed.target, parsed.ports) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    fn parse(parts: &[&str]) -> Result<ParsedArgs, String> {
        parse_args_inner(&argv(parts))
    }

    #[test]
    fn duplicate_target_is_a_usage_error() {
        let err = parse(&[
            "--target",
            "first.example",
            "--target",
            "second.example",
            "--ports",
            "47998",
        ])
        .expect_err("duplicate --target must be rejected");
        assert!(err.contains("--target"), "{err}");
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn duplicate_proxy_is_a_usage_error() {
        let err = parse(&[
            "--proxy",
            "127.0.0.1:1111",
            "--proxy",
            "127.0.0.1:2222",
            "--map",
            "47998=127.0.0.1:47999",
        ])
        .expect_err("duplicate --proxy must be rejected");
        assert!(err.contains("--proxy"), "{err}");
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn duplicate_bind_is_a_usage_error() {
        let err = parse(&["--bind", "127.0.0.1", "--bind", "127.0.0.2"])
            .expect_err("duplicate --bind must be rejected");
        assert!(err.contains("--bind"), "{err}");
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn duplicate_ports_is_a_usage_error() {
        let err = parse(&[
            "--target",
            "host.example",
            "--ports",
            "47998",
            "--ports",
            "47999",
        ])
        .expect_err("duplicate --ports must be rejected");
        assert!(err.contains("--ports"), "{err}");
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn duplicate_flag_is_rejected_in_any_position() {
        // The duplicate need not be adjacent: a repeatable --map
        // between the two occurrences does not excuse the repeat.
        let err = parse(&[
            "--proxy",
            "127.0.0.1:1111",
            "--map",
            "47998=127.0.0.1:47999",
            "--proxy",
            "127.0.0.1:2222",
        ])
        .expect_err("duplicate --proxy must be rejected in any position");
        assert!(err.contains("--proxy"), "{err}");
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn repeated_map_accumulates() {
        let p = parse(&[
            "--map",
            "47998=127.0.0.1:47999",
            "--map",
            "48000=127.0.0.1:48001",
        ])
        .expect("repeated --map is repeatable by design");
        assert_eq!(p.maps.len(), 2);
        assert!(!p.help);
    }

    #[test]
    fn single_value_invocation_parses_as_before() {
        let p = parse(&[
            "--proxy",
            "127.0.0.1:9999",
            "--bind",
            "127.0.0.2",
            "--target",
            "host.example",
            "--ports",
            "47998-48000",
        ])
        .expect("single-occurrence invocation parses");
        assert_eq!(p.proxy.as_deref(), Some("127.0.0.1:9999"));
        assert_eq!(p.bind, Some(IpAddr::from([127, 0, 0, 2])));
        assert_eq!(p.target.as_deref(), Some("host.example"));
        assert_eq!(p.ports.as_deref(), Some("47998-48000"));
        assert!(p.maps.is_empty());
        assert!(!p.help);
    }

    #[test]
    fn defaults_are_left_unset_for_main_to_apply() {
        let p = parse(&["--map", "47998=127.0.0.1:47999"]).expect("map-only invocation parses");
        assert_eq!(p.proxy, None);
        assert_eq!(p.bind, None);
    }

    #[test]
    fn help_short_circuits_where_it_appears() {
        // --help first: the rest of the line is never parsed,
        // exactly as the early return behaved before.
        let p = parse(&["--help", "--target", "a", "--target", "b"])
            .expect("help before any duplicate still parses as help");
        assert!(p.help);
        let p = parse(&["--target", "a", "--help"]).expect("help after valid flags parses");
        assert!(p.help);
    }

    #[test]
    fn missing_value_and_unknown_argument_still_fail() {
        let err = parse(&["--target"]).expect_err("a flag with no value must be rejected");
        assert!(err.contains("--target"), "{err}");
        let err = parse(&["--bogus"]).expect_err("an unknown argument must be rejected");
        assert!(err.contains("--bogus"), "{err}");
    }

    #[test]
    fn invalid_bind_address_still_fails() {
        let err = parse(&["--bind", "not-an-ip"]).expect_err("a non-IP --bind must be rejected");
        assert!(err.contains("--bind"), "{err}");
    }
}
