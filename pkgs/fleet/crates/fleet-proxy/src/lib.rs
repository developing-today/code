//! fleet-proxy — the session proxy domain (design note §4, epic
//! developing-today/code#5412, step 3: #5431).
//!
//! Two halves of one program, succeeding the Python trio
//! (laptop-agent-broker.py, laptop-proxy-connect.py, and the
//! client autostart habits around them):
//!
//! * **serve** (feature `serve`) — the broker: a Unix-socket server
//!   at the historical path holding per-port pools of warm CONNECT
//!   tunnels to the laptop's services, so a client call skips the
//!   ~1.8 s tunnel handshake. Auto-started by clients when the
//!   socket is missing; exits after an idle hour (client requests
//!   only — keepalive never resets the timer).
//! * **connect / init** — the client: ensure the broker, speak the
//!   hello (fleet-proxy-proto), regenerate the PATH shim dir from
//!   the negotiated services map, and the stdio relay that SSH's
//!   ProxyCommand will eventually point at.
//!
//! The wire is fleet-proxy-proto's and nobody else's: 8-byte
//! big-endian length-prefixed JSON, requests in today's shape. The
//! broker also serves the pre-hello legacy dialect (proto 0 — the
//! Python clients) by peeking at the first frame, and the client
//! falls back to it when a legacy broker answers its hello with a
//! response frame; the migration needs both directions alive at
//! once (§7: additive first, flips one at a time).
//!
//! The broker persists nothing: the routing table is config
//! (fleet-config), pooled tunnels are memory, token files are read
//! per request and never stored or logged.

pub mod client;
pub mod http;
pub mod raw_splice;
pub mod raw_stream;
pub mod shims;
pub mod stdio;
pub mod tunnel;

#[cfg(feature = "serve")]
pub mod serve;

#[cfg(feature = "serve")]
pub mod connect_udp;

#[cfg(feature = "serve")]
pub mod udp_forward;

/// Exit code for "the transport is down" — repo-queue's established
/// 3 (§3's collision table: 3 means repair the transport, 69 means
/// wrong build). The proxy client surface honors the same split.
pub const EXIT_TRANSPORT: i32 = 3;
/// Exit code for usage errors (the workspace-wide clap convention).
pub const EXIT_USAGE: i32 = 2;
/// Exit code when the peer's protocol is outside the window
/// (EX_UNAVAILABLE — the capability stub's code, D3).
pub const EXIT_UNAVAILABLE: i32 = 69;
