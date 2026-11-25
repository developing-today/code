//! `fleet kv`: identity-scoped key/value bytes in this host's local store
//! (code#13314, design: doc/2026-11-23T12-45-00Z_design_fleet_kv).
//!
//! The dispatcher calls [`preflight`] before session resolution so a
//! malformed command line touches nothing; [`run`] then opens the store at
//! `<state dir>/kv/local.db` as the launch identity recorded for the bound
//! session, failing closed with `identity_unavailable` when there is none.

use fleet_config::Config;
use fleet_kv::{LocalStore, cli, launch};

/// Answer help, usage errors and central-plane refusals before anything
/// else runs. `Some(code)` means the invocation is finished.
pub fn preflight(args: &[String]) -> Option<i32> {
    cli::preflight(args, &mut std::io::stdout(), &mut std::io::stderr())
}

pub fn run(args: &[String]) -> i32 {
    let state_dir = Config::state_dir();
    let session = std::env::var("FLEET_SESSION_ID").ok();
    cli::run_opening(
        args,
        || {
            let identity = launch::load(&state_dir, session.as_deref())
                .map_err(|error| (error.code(), error.to_string()))?;
            let path = launch::database_path(&state_dir);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|error| ("store_unavailable", error.to_string()))?;
            }
            let store = LocalStore::open(&path)
                .map_err(|error| ("store_unavailable", error.to_string()))?;
            Ok((store, identity))
        },
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr(),
    )
}
