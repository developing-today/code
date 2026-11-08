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
//!
//! The pump loops are hand-rolled read/write — deliberately NOT
//! `std::io::copy`: on Linux that lowers to splice(2), and under
//! this sandbox's runtime splice between a socket and a pipe
//! stalls mid-stream (observed live, #5431: the SSH handshake
//! froze after a few KB in either direction, while the Python
//! relay's recv/sendall sailed through the same path). Read/write
//! is the Python's exact syscall shape; keep it.

use std::io::{Read, Write};

use fleet_config::Config;

use crate::client::ClientError;
use crate::tunnel::{connect_tunnel, egress_from_env};

/// Move bytes until EOF or error, in small explicit read/write
/// steps (see the module docs for why not `io::copy`). Errors end
/// the pump silently — the relay's contract.
fn pump<R: Read, W: Write>(src: &mut R, dst: &mut W) {
    let mut buf = [0u8; 16384];
    loop {
        match src.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if dst.write_all(&buf[..n]).is_err() {
                    break;
                }
                if dst.flush().is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

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
        pump(&mut stdin, &mut inbound);
        let _ = inbound.shutdown(std::net::Shutdown::Write);
    });
    // tunnel → stdout on this thread.
    let mut stdout = std::io::stdout().lock();
    pump(&mut tunnel, &mut stdout);
    // The tunnel is done, so the relay is done — do NOT join the
    // upload thread (#5518): it may be blocked reading stdin
    // with stdin still open (an SSH ProxyCommand whose local side
    // has not hung up), and it holds the socket clone, so joining
    // here wedges the process until stdin EOFs. Shut the socket
    // down so a late stdin burst fails the upload pump fast, then
    // detach: process exit reaps the thread, exactly like the
    // Python relay exiting when either side finished.
    let _ = tunnel.shutdown(std::net::Shutdown::Both);
    drop(tunnel);
    drop(up);
    Ok(0)
}
