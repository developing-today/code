//! List command - enumerate stored blobs.
//!
//! This module implements listing all tags (named blobs) in a store,
//! either locally or on a remote node.
//!
//! # Output Format
//!
//! Each line contains:
//! ```text
//! <hash>\t<name>
//! ```
//!
//! Where:
//! - `hash` is the 64-character BLAKE3 content hash
//! - `name` is the tag name assigned to the blob
//!
//! # Examples
//!
//! ```bash
//! # List local store
//! id list
//!
//! # List remote node
//! id list abc123...def456
//! ```

use anyhow::{bail, Result};
use futures_lite::StreamExt;
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::{Endpoint, RelayMode},
};
use iroh_base::EndpointId;

use crate::commands::client::create_local_client_endpoint;
use crate::commands::serve::get_serve_info;
use crate::protocol::{MetaRequest, MetaResponse};
use crate::store::{load_or_create_keypair, open_store};
use crate::{is_node_id, CLIENT_KEY_FILE, META_ALPN};

/// Lists all stored files (local or remote).
///
/// # Mode Selection
///
/// 1. If `node` is provided: connect to that remote node
/// 2. If local serve is running: connect to it via lock file
/// 3. Otherwise: open the store directly
///
/// # Arguments
///
/// * `node` - Optional remote node ID (64 hex characters)
/// * `no_relay` - Disable relay servers for remote connections
///
/// # Output
///
/// Prints each tag as `hash\tname` to stdout.
/// Prints `(no files stored)` if the store is empty.
///
/// # Errors
///
/// Returns an error if:
/// - The node ID is invalid
/// - Connection to remote fails
/// - Store operations fail
pub async fn cmd_list(node: Option<String>, no_relay: bool) -> Result<()> {
    // Remote list
    if let Some(node_id_str) = node {
        if !is_node_id(&node_id_str) {
            bail!("invalid node ID: must be 64 hex characters");
        }
        let server_node_id: EndpointId = node_id_str.parse()?;
        return cmd_list_remote(server_node_id, no_relay).await;
    }

    // Local list
    if let Some(serve_info) = get_serve_info().await {
        let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

        let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::List)?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(1024 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        match resp {
            MetaResponse::List { items } => {
                if items.is_empty() {
                    println!("(no files stored)");
                } else {
                    for (hash, name) in items {
                        println!("{}\t{}", hash, name);
                    }
                }
            }
            _ => bail!("unexpected response"),
        }
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        let mut list = store_handle.tags().list().await?;
        let mut count = 0;
        while let Some(item) = list.next().await {
            let item = item?;
            let name = String::from_utf8_lossy(item.name.as_ref());
            println!("{}\t{}", item.hash, name);
            count += 1;
        }
        if count == 0 {
            println!("(no files stored)");
        }
        store.shutdown().await?;
    }
    Ok(())
}

/// Lists files on a remote node.
///
/// Connects to the specified node via QUIC and requests a list
/// of all stored tags using the meta protocol.
///
/// # Arguments
///
/// * `server_node_id` - The remote node's public identity
/// * `no_relay` - Disable relay servers (direct connection only)
///
/// # Output
///
/// Same format as [`cmd_list`].
pub async fn cmd_list_remote(server_node_id: EndpointId, no_relay: bool) -> Result<()> {
    let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    let mut builder = Endpoint::builder()
        .secret_key(client_key)
        .address_lookup(PkarrPublisher::n0_dns())
        .address_lookup(DnsAddressLookup::n0_dns());
    if no_relay {
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    let endpoint = builder.bind().await?;

    let meta_conn = endpoint.connect(server_node_id, META_ALPN).await?;
    let (mut send, mut recv) = meta_conn.open_bi().await?;
    let req = postcard::to_allocvec(&MetaRequest::List)?;
    send.write_all(&req).await?;
    send.finish()?;
    let resp_buf = recv.read_to_end(1024 * 1024).await?;
    let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
    meta_conn.close(0u32.into(), b"done");

    match resp {
        MetaResponse::List { items } => {
            if items.is_empty() {
                println!("(no files stored)");
            } else {
                for (hash, name) in items {
                    println!("{}\t{}", hash, name);
                }
            }
        }
        _ => bail!("unexpected response"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_node_id_validation() {
        // Valid node ID (64 hex chars)
        assert!(is_node_id("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
        
        // Invalid: too short
        assert!(!is_node_id("0123456789abcdef"));
        
        // Invalid: non-hex chars
        assert!(!is_node_id("ghijklmnopqrstuv0123456789abcdef0123456789abcdef0123456789abcdef"));
    }
}
