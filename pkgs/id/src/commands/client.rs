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
//! use id::commands::{get_serve_info, create_local_client_endpoint};
//!
//! if let Some(serve_info) = get_serve_info().await {
//!     let (endpoint, addr) = create_local_client_endpoint(&serve_info).await?;
//!
//!     // Connect using the meta protocol
//!     let conn = endpoint.connect(addr, META_ALPN).await?;
//! }
//! ```

use anyhow::Result;
use iroh::{
    address_lookup::MdnsAddressLookup,
    endpoint::{Endpoint, presets},
};
use iroh_base::{EndpointAddr, TransportAddr};

use super::serve::ServeInfo;
use crate::{CLIENT_KEY_FILE, load_or_create_keypair};

/// Creates a client endpoint configured to connect to a local serve.
///
/// The endpoint is configured with:
/// - A client-specific keypair (separate from the serve keypair)
/// - DNS and Pkarr address lookup for remote peer discovery
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
    let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    // Enable relay, DNS lookup, and mDNS so local network discovery works
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(client_key)
        .address_lookup(MdnsAddressLookup::builder())
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
