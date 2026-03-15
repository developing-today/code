//! Client endpoint creation for connecting to local serve

use anyhow::Result;
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::Endpoint,
};
use iroh_base::{EndpointAddr, TransportAddr};

use crate::{CLIENT_KEY_FILE, load_or_create_keypair};
use super::serve::ServeInfo;

/// Create a client endpoint configured to connect to the local serve
pub async fn create_local_client_endpoint(serve_info: &ServeInfo) -> Result<(Endpoint, EndpointAddr)> {
    let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    // Enable relay and DNS lookup so @NODE_ID targeting works for remote peers
    let endpoint = Endpoint::builder()
        .secret_key(client_key)
        .address_lookup(PkarrPublisher::n0_dns())
        .address_lookup(DnsAddressLookup::n0_dns())
        .bind()
        .await?;

    // Build EndpointAddr with known socket addresses to bypass DNS discovery
    // Prefer IPv4 localhost for reliability on systems with IPv6 issues
    let addrs: Vec<_> = serve_info
        .addrs
        .iter()
        .filter(|addr| addr.is_ipv4())
        .map(|addr| TransportAddr::Ip(*addr))
        .collect();

    // Fall back to all addresses if no IPv4 found
    let addrs = if addrs.is_empty() {
        serve_info
            .addrs
            .iter()
            .map(|addr| TransportAddr::Ip(*addr))
            .collect()
    } else {
        addrs
    };

    let endpoint_addr = EndpointAddr::from_parts(serve_info.node_id, addrs);

    Ok((endpoint, endpoint_addr))
}
