//! `fleet proxy connect --stdio` — the ProxyCommand relay:
//! laptop-proxy-connect.py's job, fleet-shaped. Opens a tunnel to
//! the target (CONNECT through the egress proxy, or direct) and
//! splices stdin/stdout onto it until either side closes.
//!
//! The proxy credential travels inside the CONNECT request (from
//! the runtime env) — never in this process's argument list, which
//! is what killed the retired socat-based command (code#948).
//! Relay errors after the splice starts are silent exits, like the
//! Python's: a ProxyCommand must not chatter onto the SSH stream.

use std::io::Write;

use fleet_config::Config;

use crate::client::ClientError;
use crate::tunnel::{connect_tunnel, egress_from_env};

/// Relay stdin/stdout over a tunnel to `host:port`. Returns the
/// exit code (0 on a clean splice, 3 when the tunnel itself could
/// not be opened — with the reason on stderr, where SSH shows it).
pub fn relay_stdio(config: &Config, host: &str, port: u16) -> Result<i32, ClientError> {
    let mut tunnel = if config.proxy_direct() {
        connect_tunnel(None, host, port)
    } else {
        let egress = egress_from_env(config.egress_proxy_port()).map_err(ClientError::Transport)?;
        connect_tunnel(Some(&egress), host, port)
    }
    .map_err(|e| ClientError::Transport(format!("tunnel to {host}:{port}: {e}")))?;

    let mut inbound = tunnel
        .try_clone()
        .map_err(|e| ClientError::Transport(format!("tunnel clone: {e}")))?;
    // stdin → tunnel, on a thread; half-close the tunnel write
    // side at stdin EOF so the peer sees a clean end.
    let up = std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let _ = std::io::copy(&mut stdin, &mut inbound);
        let _ = inbound.shutdown(std::net::Shutdown::Write);
    });
    // tunnel → stdout on this thread.
    let mut stdout = std::io::stdout().lock();
    let _ = std::io::copy(&mut tunnel, &mut stdout);
    let _ = stdout.flush();
    drop(tunnel);
    let _ = up.join();
    Ok(0)
}
