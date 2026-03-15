//! Server command and lock file management.
//!
//! This module implements the `serve` command which starts a persistent
//! server that accepts connections from peers for blob storage and retrieval.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     Serve Process                           │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐      │
//! │  │  Endpoint   │    │   Router    │    │   Store     │      │
//! │  │  (QUIC)     │───►│             │───►│ (blobs/tags)│      │
//! │  └─────────────┘    └─────────────┘    └─────────────┘      │
//! │         │                  │                                 │
//! │         │           ┌──────┴──────┐                          │
//! │         │           │             │                          │
//! │         │     ┌─────▼─────┐ ┌─────▼─────┐                    │
//! │         │     │MetaProtocol│ │BlobsProtocol│                    │
//! │         │     │ /id/meta/1 │ │ /iroh/blobs │                    │
//! │         │     └───────────┘ └───────────┘                    │
//! │         │                                                    │
//! │         ▼                                                    │
//! │  Lock File: .id-serve-lock                                   │
//! │  - Node ID, PID, Socket addresses                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Lock File Protocol
//!
//! The serve process creates a lock file (`.id-serve-lock`) containing:
//! 1. Node ID (line 1)
//! 2. Process ID (line 2)
//! 3. Socket addresses (remaining lines)
//!
//! Other processes (REPL, CLI commands) check this file to determine
//! if a local serve is running and how to connect to it.
//!
//! # Examples
//!
//! ```bash
//! # Start persistent server
//! id serve
//!
//! # Start ephemeral server (in-memory)
//! id serve --ephemeral
//!
//! # Start without relay servers
//! id serve --no-relay
//! ```

use anyhow::Result;
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::{Endpoint, RelayMode},
    protocol::Router,
};
use iroh_base::EndpointId;
use iroh_blobs::{ALPN as BLOBS_ALPN, BlobsProtocol};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::fs as afs;
use tracing::info;

use crate::protocol::MetaProtocol;
use crate::store::{load_or_create_keypair, open_store};
use crate::{KEY_FILE, META_ALPN, SERVE_LOCK, STORE_PATH};

/// Information about a running serve instance.
///
/// Retrieved from the lock file by [`get_serve_info`] to enable
/// other processes to connect to the local serve.
///
/// # Fields
///
/// - `node_id`: The public identity of the serve node
/// - `addrs`: Local socket addresses where the serve is listening
#[derive(Debug, Clone)]
pub struct ServeInfo {
    /// The public node ID derived from the serve's keypair.
    pub node_id: EndpointId,
    /// Socket addresses the serve is bound to.
    pub addrs: Vec<SocketAddr>,
}

/// Checks if a serve instance is running and returns its connection info.
///
/// Reads the lock file, verifies the PID is still alive, and returns
/// the serve info needed to connect. Returns `None` if:
/// - Lock file doesn't exist
/// - Lock file is malformed
/// - Referenced process is no longer running (stale lock)
///
/// # Lock File Format
///
/// ```text
/// <node_id>
/// <pid>
/// <socket_addr_1>
/// <socket_addr_2>
/// ...
/// ```
///
/// # Example
///
/// ```rust,ignore
/// if let Some(info) = get_serve_info().await {
///     println!("Serve running: {}", info.node_id);
///     // Connect to info.addrs...
/// } else {
///     println!("No serve running");
/// }
/// ```
pub async fn get_serve_info() -> Option<ServeInfo> {
    let contents = afs::read_to_string(SERVE_LOCK).await.ok()?;
    let mut lines = contents.lines();
    let node_id_str = lines.next()?;
    let pid_str = lines.next()?;
    let pid: u32 = pid_str.parse().ok()?;

    // Check if process is still alive
    if !is_process_alive(pid) {
        // Stale lock file - remove it
        let _ = afs::remove_file(SERVE_LOCK).await;
        return None;
    }

    let node_id: EndpointId = node_id_str.parse().ok()?;

    // Parse socket addresses (remaining lines)
    let addrs: Vec<SocketAddr> = lines.filter_map(|line| line.parse().ok()).collect();

    Some(ServeInfo { node_id, addrs })
}

/// Checks if a process with the given PID is still running.
///
/// Uses platform-specific methods:
/// - Unix: `kill(pid, 0)` which checks existence without sending a signal
/// - Other: Always returns `true` (conservative fallback)
///
/// # Arguments
///
/// * `pid` - The process ID to check
///
/// # Returns
///
/// `true` if the process exists, `false` otherwise.
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill -0 checks existence without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, just assume it's alive if we have a PID
        let _ = pid;
        true
    }
}

/// Creates the serve lock file with connection information.
///
/// Writes the node ID, current process ID, and socket addresses
/// to the lock file so other processes can discover and connect.
///
/// # Arguments
///
/// * `node_id` - The serve node's public identity
/// * `addrs` - Socket addresses the serve is listening on
///
/// # Errors
///
/// Returns an error if the lock file cannot be written.
pub async fn create_serve_lock(node_id: &EndpointId, addrs: &[SocketAddr]) -> Result<()> {
    let pid = std::process::id();
    let mut contents = format!("{}\n{}", node_id, pid);
    for addr in addrs {
        contents.push_str(&format!("\n{}", addr));
    }
    afs::write(SERVE_LOCK, contents).await?;
    Ok(())
}

/// Removes the serve lock file.
///
/// Called during graceful shutdown to indicate the serve is no longer running.
/// Errors are silently ignored (file may already be removed).
pub async fn remove_serve_lock() -> Result<()> {
    let _ = afs::remove_file(SERVE_LOCK).await;
    Ok(())
}

/// Starts the serve process.
///
/// Initializes the Iroh endpoint, blob store, and protocol handlers,
/// then waits for incoming connections until interrupted with Ctrl+C.
///
/// # Arguments
///
/// * `ephemeral` - If `true`, use in-memory storage (lost on exit)
/// * `no_relay` - If `true`, disable relay servers (direct connections only)
///
/// # Behavior
///
/// 1. Loads or creates the node keypair
/// 2. Opens the blob store (persistent or ephemeral)
/// 3. Creates the Iroh endpoint with DNS/Pkarr address lookup
/// 4. Registers MetaProtocol and BlobsProtocol handlers
/// 5. Creates the lock file for discovery
/// 6. Waits for Ctrl+C
/// 7. Cleans up and exits
///
/// # Output
///
/// Prints the node ID and mode to stdout. Status messages go to stderr.
///
/// # Example
///
/// ```rust,ignore
/// // Start a persistent server
/// cmd_serve(false, false).await?;
/// ```
pub async fn cmd_serve(ephemeral: bool, no_relay: bool) -> Result<()> {
    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public().into();
    info!("serve: {}", node_id);

    let store = open_store(ephemeral).await?;
    let store_handle = store.as_store();

    let mut builder = Endpoint::builder()
        .secret_key(key.clone())
        .address_lookup(PkarrPublisher::n0_dns())
        .address_lookup(DnsAddressLookup::n0_dns());
    if no_relay {
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    let endpoint = builder.bind().await?;

    let meta = MetaProtocol::new(&store_handle);
    let blobs = BlobsProtocol::new(&store_handle, None);

    let router = Router::builder(endpoint)
        .accept(META_ALPN, meta)
        .accept(BLOBS_ALPN, blobs)
        .spawn();

    let serve_node_id = router.endpoint().id();
    let bound_addrs = router.endpoint().bound_sockets();
    let local_addrs: Vec<SocketAddr> = bound_addrs
        .iter()
        .map(|addr| match addr {
            SocketAddr::V4(v4) if v4.ip().is_unspecified() => {
                SocketAddr::new(Ipv4Addr::LOCALHOST.into(), v4.port())
            }
            SocketAddr::V6(v6) if v6.ip().is_unspecified() => {
                SocketAddr::new(Ipv6Addr::LOCALHOST.into(), v6.port())
            }
            other => *other,
        })
        .collect();
    create_serve_lock(&serve_node_id, &local_addrs).await?;

    println!("node: {}", serve_node_id);
    if ephemeral {
        println!("mode: ephemeral (in-memory)");
    } else {
        println!("mode: persistent ({})", STORE_PATH);
    }
    if no_relay {
        println!("relay: disabled");
    }

    tokio::signal::ctrl_c().await?;
    remove_serve_lock().await?;
    router.shutdown().await?;
    store.shutdown().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_process_alive_current_process() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn test_is_process_alive_nonexistent() {
        // Use a very high PID that's unlikely to exist
        // Note: On non-Unix this always returns true
        #[cfg(unix)]
        {
            assert!(!is_process_alive(999999999));
        }
    }

    #[test]
    fn test_is_process_alive_pid_1() {
        // PID 1 (init) should exist on Unix systems, but may not be visible
        // in containerized environments where the container has its own PID namespace
        #[cfg(unix)]
        {
            // Just check that the function doesn't panic - the result depends on environment
            let _ = is_process_alive(1);
        }
    }

    #[test]
    fn test_serve_info_struct() {
        use iroh_base::SecretKey;
        
        let key = SecretKey::generate(&mut rand::rng());
        let node_id = key.public();
        let addrs = vec![
            "127.0.0.1:8080".parse().unwrap(),
            "[::1]:8080".parse().unwrap(),
        ];
        
        let info = ServeInfo {
            node_id,
            addrs: addrs.clone(),
        };
        
        assert_eq!(info.node_id, node_id);
        assert_eq!(info.addrs.len(), 2);
        assert_eq!(info.addrs[0].to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn test_serve_info_clone() {
        use iroh_base::SecretKey;
        
        let key = SecretKey::generate(&mut rand::rng());
        let node_id = key.public();
        let info = ServeInfo {
            node_id,
            addrs: vec!["127.0.0.1:8080".parse().unwrap()],
        };
        
        let cloned = info.clone();
        assert_eq!(cloned.node_id, info.node_id);
        assert_eq!(cloned.addrs, info.addrs);
    }

    // Integration tests for lock file functions require file system access
    // and are tested via the integration test suite
}
