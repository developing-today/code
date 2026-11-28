//! fleet-connect-udp — the standalone CONNECT-UDP endpoint
//! (RFC 9298, #6109), for hosts where the session broker
//! (`fleet proxy serve`) doesn't run: the laptop serves its
//! fleet roles as `fleet serve <role>` units, so the endpoint
//! gets its own handler process there.
//!
//! Dark by construction, twice over: it refuses to start
//! unless the fleet config it loads opts in with
//! `[proxy] connect_udp = true` (fleet-config), and the
//! endpoint binds whatever `connect_udp_listen` says — the
//! default, and the only sanctioned posture pre-cutover, is
//! loopback. Which config loads is fleet-config's own
//! resolution ($FLEET_CONFIG, else ~/.config/fleet/fleet.toml),
//! so a host can stage this endpoint behind a dedicated config
//! file without touching the shared one.

use std::process::ExitCode;

fn main() -> ExitCode {
    // The argument surface comes first (#6134): --help/-h print
    // the usage and exit 0, and any other argument is a usage
    // error — both without loading config or binding anything,
    // so an accidental argument vector can never start the
    // endpoint. Bare invocation is unchanged.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match fleet_proxy::connect_udp::classify_bin_args(&argv) {
        fleet_proxy::connect_udp::BinArgs::Help => {
            println!("{}", fleet_proxy::connect_udp::CONNECT_UDP_USAGE);
            return ExitCode::SUCCESS;
        }
        fleet_proxy::connect_udp::BinArgs::UsageError(arg) => {
            eprintln!("fleet-connect-udp: unknown argument {arg:?}");
            eprintln!("{}", fleet_proxy::connect_udp::CONNECT_UDP_USAGE);
            return ExitCode::from(fleet_proxy::EXIT_USAGE as u8);
        }
        fleet_proxy::connect_udp::BinArgs::Run => {}
    }
    let config = match fleet_config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fleet-connect-udp: cannot load fleet config: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !config.connect_udp_enabled() {
        eprintln!(
            "fleet-connect-udp: [proxy] connect_udp is not enabled in the loaded \
             fleet config — the endpoint stays dark. Enable it deliberately, \
             loopback only, in the config this process loads."
        );
        return ExitCode::from(fleet_proxy::EXIT_UNAVAILABLE as u8);
    }
    match fleet_proxy::connect_udp::run(&config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fleet-connect-udp: {e}");
            ExitCode::FAILURE
        }
    }
}
