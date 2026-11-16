//! Client endpoint creation for connecting to local serve.
//!
//! This module provides utilities for creating client endpoints that
//! connect to a running local serve instance. It handles:
//!
//! - Loading or creating client keypairs
//! - Building endpoint addresses with known socket addresses
//! - IPv4/IPv6 address selection
//!
//! # Usage
//!
//! ```rust,ignore
//! use id_cli::commands::{get_serve_info, create_local_client_endpoint};
//!
//! if let Some(serve_info) = get_serve_info().await {
//!     let (endpoint, addr) = create_local_client_endpoint(&serve_info).await?;
//!
//!     // Connect using the meta protocol
//!     let conn = endpoint.connect(addr, META_ALPN).await?;
//! }
//! ```

use anyhow::Result;
use iroh::endpoint::{Endpoint, RelayMode, presets};
use iroh_base::{EndpointAddr, TransportAddr};

use super::serve::ServeInfo;
use crate::{CLIENT_KEY_FILE, load_or_create_keypair};

/// Creates a client endpoint configured to connect to a local serve.
///
/// The endpoint is configured with:
/// - A client-specific keypair (separate from the serve keypair)
/// - No relay and no address lookup: the serve address is known, so the
///   connection is purely local
/// - Known socket addresses from the serve lock file
///
/// # Arguments
///
/// * `serve_info` - Information about the running serve instance
///
/// # Returns
///
/// A tuple of:
/// - `Endpoint`: The QUIC endpoint for making connections
/// - `EndpointAddr`: The address of the serve, pre-populated with socket addresses
///
/// # Address Selection
///
/// IPv4 addresses are preferred over IPv6 for reliability on systems
/// with IPv6 configuration issues. If no IPv4 addresses are available,
/// all addresses from the serve info are used.
///
/// # Example
///
/// ```rust,ignore
/// let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
///
/// // Connect to meta protocol
/// let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
///
/// // Connect to blobs protocol
/// let blobs_conn = endpoint.connect(endpoint_addr, BLOBS_ALPN).await?;
/// ```
pub async fn create_local_client_endpoint(
    serve_info: &ServeInfo,
) -> Result<(Endpoint, EndpointAddr)> {
    create_local_endpoint_with_key(serve_info, CLIENT_KEY_FILE).await
}

/// Creates an endpoint that connects to a local serve as its
/// designated local operator (#2147 operator endpoint).
///
/// Publication control is operator-only: the serve handler answers
/// publish/unpublish/list requests only from the serving node
/// itself or the local operator identity serve designated -- this
/// machine's client identity. The local publication CLI therefore
/// presents the client keypair: an iroh endpoint cannot connect to
/// itself ("Connecting to ourself is not supported"), so the
/// node's own key cannot be the transport identity of a local
/// operator connection (learned in the slice-3b acceptance walk,
/// 2026-11-14). Holding either local key file is the operator
/// credential -- the design's trade-off, chosen over widening the
/// remote write policy to listener and route changes.
pub async fn create_local_operator_endpoint(
    serve_info: &ServeInfo,
) -> Result<(Endpoint, EndpointAddr)> {
    create_local_endpoint_with_key(serve_info, CLIENT_KEY_FILE).await
}

async fn create_local_endpoint_with_key(
    serve_info: &ServeInfo,
    key_file: &str,
) -> Result<(Endpoint, EndpointAddr)> {
    let client_key = load_or_create_keypair(key_file).await?;
    // The server's loopback addresses come from the lock file, so no relay and
    // no address lookup (DNS, pkarr, mDNS) are needed. Using the full N0 preset
    // here made every CLI command probe relays and publish this client's info
    // to n0's DNS, which added seconds of latency and an outbound dependency
    // to what is a purely local connection.
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(client_key)
        .bind()
        .await?;

    // Build EndpointAddr with known socket addresses to bypass DNS discovery
    // Prefer IPv4 localhost for reliability on systems with IPv6 issues
    let all_addrs = serve_info.socket_addrs();
    let addrs: Vec<_> = all_addrs
        .iter()
        .filter(|addr| addr.is_ipv4())
        .map(|addr| TransportAddr::Ip(*addr))
        .collect();

    // Fall back to all addresses if no IPv4 found
    let addrs = if addrs.is_empty() {
        all_addrs
            .iter()
            .map(|addr| TransportAddr::Ip(*addr))
            .collect()
    } else {
        addrs
    };

    let node_id = serve_info
        .endpoint_id()
        .ok_or_else(|| anyhow::anyhow!("invalid node_id in lock file"))?;
    let endpoint_addr = EndpointAddr::from_parts(node_id, addrs);

    Ok((endpoint, endpoint_addr))
}
