//! Get command handlers for retrieving blobs by name or hash.
//!
//! This module provides the implementation for the `get` command, which retrieves
//! blobs from either the local store or a remote peer node. It supports multiple
//! retrieval strategies:
//!
//! - **By name**: Look up a blob by its human-readable tag name
//! - **By hash**: Directly fetch a blob using its content hash (64 hex characters)
//! - **Local**: Retrieve from the local blob store
//! - **Remote**: Fetch from a connected serve instance or remote peer node
//!
//! # Architecture
//!
//! The get command operates in different modes depending on context:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      cmd_get_multi                              │
//! │  (orchestrates multi-item fetching, handles stdin input)        │
//! └─────────────────────────────────────────────────────────────────┘
//!                              │
//!          ┌───────────────────┼───────────────────┐
//!          │                   │                   │
//!          ▼                   ▼                   ▼
//! ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
//! │   cmd_get_one   │ │cmd_get_one_remote│ │   cmd_gethash   │
//! │ (local by name) │ │ (from peer node) │ │ (by hash only)  │
//! └─────────────────┘ └─────────────────┘ └─────────────────┘
//!          │                   │                   │
//!          ▼                   ▼                   ▼
//! ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
//! │  cmd_get_local  │ │  MetaProtocol   │ │  BLOBS_ALPN     │
//! │ (name → hash)   │ │   + BLOBS_ALPN  │ │  (direct fetch) │
//! └─────────────────┘ └─────────────────┘ └─────────────────┘
//! ```
//!
//! # Output Destinations
//!
//! All get functions accept an `output` parameter:
//! - `"-"` writes to stdout
//! - Any other string writes to a file at that path
//!
//! # Examples
//!
//! Retrieve a file by name:
//! ```bash
//! id get config.json
//! ```
//!
//! Retrieve by hash to stdout:
//! ```bash
//! id get abc123...def456 -o -
//! ```
//!
//! Fetch from a remote peer:
//! ```bash
//! id get <NODE_ID> config.json
//! ```

use anyhow::{Context, Result, bail};
use iroh::endpoint::{Endpoint, RelayMode, presets};
use iroh_base::EndpointId;
use iroh_blobs::{ALPN as BLOBS_ALPN, Hash};

use crate::{
    CLIENT_KEY_FILE, META_ALPN, MetaRequest, MetaResponse, create_local_client_endpoint,
    export_blob, get_serve_info, is_node_id, load_or_create_keypair, open_store, parse_get_spec,
    parse_stdin_items,
};

/// Retrieve a blob by its content hash and export to the specified output.
///
/// This function performs a direct hash-based lookup, bypassing the tag/name system.
/// It's useful when you have the exact content hash and want to retrieve the data
/// without knowing its name.
///
/// # Arguments
///
/// * `hash_str` - The blob's content hash as a 64-character hex string
/// * `output` - Destination path, or `"-"` for stdout
///
/// # Errors
///
/// Returns an error if:
/// - `hash_str` is not exactly 64 hexadecimal characters
/// - The blob cannot be found locally or fetched from the serve instance
/// - Writing to the output destination fails
///
/// # Example
///
/// ```bash
/// id gethash abc123...def456 output.bin
/// id gethash abc123...def456 -  # to stdout
/// ```
pub async fn cmd_gethash(hash_str: &str, output: &str) -> Result<()> {
    // Validate hash format before parsing (64 hex chars)
    if hash_str.len() != 64 || !hash_str.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid hash: expected 64 hex characters");
    }
    let hash: Hash = hash_str.parse().context("invalid hash")?;

    if let Some(serve_info) = get_serve_info().await {
        let store = open_store(true).await?;
        let store_handle = store.as_store();

        let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

        let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
        store_handle
            .remote()
            .fetch(blobs_conn.clone(), hash)
            .await?;
        blobs_conn.close(0u32.into(), b"done");

        export_blob(&store_handle, hash, output).await?;
        endpoint.close().await;
        store.shutdown().await?;
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        export_blob(&store_handle, hash, output).await?;
        store.shutdown().await?;
    }
    Ok(())
}

/// Retrieve a blob by its tag name from the local store.
///
/// This function looks up a blob by name (tag) rather than hash. If a local
/// serve instance is running, it queries the server; otherwise, it performs
/// a direct local store lookup.
///
/// # Protocol Flow (when serve is running)
///
/// 1. Connect to local serve via `META_ALPN`
/// 2. Send `MetaRequest::Get { filename }` to resolve name → hash
/// 3. Receive `MetaResponse::Get { hash }` with the content hash
/// 4. Connect via `BLOBS_ALPN` and fetch the blob data
/// 5. Export to the output destination
///
/// # Arguments
///
/// * `name` - The tag name to look up
/// * `output` - Destination path, or `"-"` for stdout
///
/// # Errors
///
/// Returns an error if:
/// - The name is not found in the store
/// - Connection to serve fails
/// - Writing to the output destination fails
pub async fn cmd_get_local(name: &str, output: &str) -> Result<()> {
    if let Some(serve_info) = get_serve_info().await {
        let store = open_store(true).await?;
        let store_handle = store.as_store();

        let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

        let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Get {
            filename: name.to_owned(),
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        let hash = match resp {
            MetaResponse::Get { hash: Some(h) } => h,
            MetaResponse::Get { hash: None } => {
                endpoint.close().await;
                bail!("file not found");
            }
            _ => {
                endpoint.close().await;
                bail!("unexpected response");
            }
        };

        let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
        store_handle
            .remote()
            .fetch(blobs_conn.clone(), hash)
            .await?;
        blobs_conn.close(0u32.into(), b"done");

        export_blob(&store_handle, hash, output).await?;
        endpoint.close().await;
        store.shutdown().await?;
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        let tag = store_handle
            .tags()
            .get(name)
            .await?
            .context("file not found")?;

        export_blob(&store_handle, tag.hash, output).await?;
        store.shutdown().await?;
    }
    Ok(())
}

/// Retrieve a single item by name or hash, with auto-detection.
///
/// This function implements smart source detection:
///
/// 1. If `--hash` flag is set, treat source as a hash
/// 2. If source looks like a hash (64 hex chars) and `--name-only` is not set,
///    try hash lookup first
/// 3. Fall back to name-based lookup
///
/// This allows natural usage where `id get abc123...` works whether `abc123...`
/// is a name or a hash, trying the most likely interpretation first.
///
/// # Arguments
///
/// * `source` - Name or hash to retrieve
/// * `output` - Destination path, or `"-"` for stdout
/// * `hash_mode` - If true, treat source as a hash (from `--hash` flag)
/// * `name_only` - If true, skip hash detection (from `--name-only` flag)
///
/// # Errors
///
/// Returns an error if the source cannot be found as either a name or hash.
pub async fn cmd_get_one(
    source: &str,
    output: &str,
    hash_mode: bool,
    name_only: bool,
) -> Result<()> {
    let is_valid_hash = source.len() == 64 && source.chars().all(|c| c.is_ascii_hexdigit());

    // If --hash flag, treat as hash lookup
    if hash_mode {
        return cmd_gethash(source, output).await;
    }

    // If it looks like a hash (64 hex chars) and not --name-only, try hash first
    if is_valid_hash
        && !name_only
        && let Ok(hash) = source.parse::<Hash>()
    {
        if let Some(serve_info) = get_serve_info().await {
            let store = open_store(true).await?;
            let store_handle = store.as_store();
            let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
            let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;

            match store_handle.remote().fetch(blobs_conn.clone(), hash).await {
                Ok(_) => {
                    blobs_conn.close(0u32.into(), b"done");
                    export_blob(&store_handle, hash, output).await?;
                    endpoint.close().await;
                    store.shutdown().await?;
                    return Ok(());
                }
                Err(_) => {
                    blobs_conn.close(0u32.into(), b"done");
                }
            }
            endpoint.close().await;
            store.shutdown().await?;
        } else {
            let store = open_store(false).await?;
            let store_handle = store.as_store();
            if store_handle.blobs().has(hash).await? {
                export_blob(&store_handle, hash, output).await?;
                store.shutdown().await?;
                return Ok(());
            }
            store.shutdown().await?;
        }
    }

    // Try as name
    cmd_get_local(source, output).await
}

/// Retrieve a single file from a remote peer node.
///
/// This function establishes a direct connection to a remote peer (identified
/// by their node ID) and fetches a file by name. It uses DNS-based address
/// lookup to resolve the node ID to network addresses.
///
/// # Protocol Flow
///
/// 1. Create a client endpoint with the local keypair
/// 2. Connect to the remote node via `META_ALPN`
/// 3. Send `MetaRequest::Get { filename }` to get the hash
/// 4. Connect via `BLOBS_ALPN` and fetch the blob data
/// 5. Export to the output destination
///
/// # Arguments
///
/// * `server_node_id` - The remote peer's 32-byte node ID
/// * `name` - The tag name to retrieve
/// * `output` - Destination path, or `"-"` for stdout
/// * `no_relay` - If true, disable relay servers (direct connections only)
///
/// # Errors
///
/// Returns an error if:
/// - Cannot connect to the remote node
/// - The file is not found on the remote
/// - Blob transfer fails
pub async fn cmd_get_one_remote(
    server_node_id: EndpointId,
    name: &str,
    output: &str,
    no_relay: bool,
) -> Result<()> {
    let store = open_store(true).await?;
    let store_handle = store.as_store();

    let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    let mut builder = Endpoint::builder(presets::N0).secret_key(client_key);
    if no_relay {
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    let endpoint = builder.bind().await?;

    let meta_conn = endpoint.connect(server_node_id, META_ALPN).await?;
    let (mut send, mut recv) = meta_conn.open_bi().await?;
    let req = postcard::to_allocvec(&MetaRequest::Get {
        filename: name.to_owned(),
    })?;
    send.write_all(&req).await?;
    send.finish()?;
    let resp_buf = recv.read_to_end(64 * 1024).await?;
    let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
    meta_conn.close(0u32.into(), b"done");

    let hash = match resp {
        MetaResponse::Get { hash: Some(h) } => h,
        MetaResponse::Get { hash: None } => {
            endpoint.close().await;
            bail!("file not found on remote");
        }
        _ => {
            endpoint.close().await;
            bail!("unexpected response");
        }
    };

    let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
    store_handle
        .remote()
        .fetch(blobs_conn.clone(), hash)
        .await?;
    blobs_conn.close(0u32.into(), b"done");

    export_blob(&store_handle, hash, output).await?;
    endpoint.close().await;
    store.shutdown().await?;
    Ok(())
}

/// Retrieve multiple items, either locally or from a remote node.
///
/// This is the main entry point for the CLI `get` command when multiple sources
/// are provided. It handles:
///
/// - **Remote fetching**: If the first argument is a valid node ID, all remaining
///   items are fetched from that remote peer
/// - **Stdin input**: With `--stdin`, read additional sources from stdin (one per line)
/// - **Output mapping**: Sources can include `:output` suffix (e.g., `file.txt:local.txt`)
/// - **Stdout mode**: With `--stdout`, all output goes to stdout instead of files
///
/// # Source Format
///
/// Each source can be:
/// - `name` - Retrieve and save to a file with the same name
/// - `name:output` - Retrieve `name` and save to `output`
/// - `hash` - If it looks like a hash (64 hex chars), try hash lookup first
///
/// # Arguments
///
/// * `sources` - List of sources to retrieve
/// * `from_stdin` - If true, also read sources from stdin
/// * `hash_mode` - If true, treat all sources as hashes
/// * `name_only` - If true, never interpret sources as hashes
/// * `to_stdout` - If true, output all data to stdout
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Errors
///
/// Returns an error if:
/// - No sources are provided
/// - Any individual get operation fails (collected and reported at end)
///
/// # Example
///
/// ```bash
/// # Get multiple files
/// id get file1.txt file2.txt file3.txt
///
/// # Get from remote peer
/// id get <NODE_ID> config.json data.json
///
/// # Get with output mapping
/// id get config.json:local-config.json
/// ```
pub async fn cmd_get_multi(
    sources: Vec<String>,
    from_stdin: bool,
    hash_mode: bool,
    name_only: bool,
    to_stdout: bool,
    no_relay: bool,
) -> Result<()> {
    let mut items = sources;

    // Check if first arg is a remote node ID
    let remote_node: Option<EndpointId> = if !items.is_empty() && is_node_id(&items[0]) {
        let node_id: EndpointId = items[0].parse()?;
        items.remove(0);
        Some(node_id)
    } else {
        None
    };

    if from_stdin {
        items.extend(parse_stdin_items()?);
    }

    if items.is_empty() {
        bail!("no sources provided");
    }

    let mut errors = Vec::new();
    for spec in &items {
        let (source, explicit_output) = parse_get_spec(spec);
        // Priority: --stdout flag > explicit :output > source name
        let output = if to_stdout {
            "-"
        } else if let Some(out) = explicit_output {
            out
        } else {
            source
        };
        let result = if let Some(node_id) = remote_node {
            cmd_get_one_remote(node_id, source, output, no_relay).await
        } else {
            cmd_get_one(source, output, hash_mode, name_only).await
        };
        if let Err(e) = result {
            errors.push(format!("{source}: {e}"));
        }
    }

    if !errors.is_empty() {
        bail!("some gets failed:\n{}", errors.join("\n"));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_is_node_id_integration() {
        // Valid node ID
        assert!(is_node_id(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        // Invalid
        assert!(!is_node_id("not_a_node_id"));
    }

    #[test]
    fn test_parse_get_spec_integration() {
        // Simple source
        let (source, output) = parse_get_spec("file.txt");
        assert_eq!(source, "file.txt");
        assert_eq!(output, None);

        // Source with explicit output
        let (source, output) = parse_get_spec("file.txt:output.txt");
        assert_eq!(source, "file.txt");
        assert_eq!(output, Some("output.txt"));
    }

    #[tokio::test]
    async fn test_cmd_gethash_invalid_hash() {
        // Too short
        let result = cmd_gethash("abc", "-").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid hash"));

        // Non-hex chars
        let result = cmd_gethash(
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "-",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cmd_get_multi_empty_no_sources() {
        let result = cmd_get_multi(vec![], false, false, false, false, false).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no sources provided")
        );
    }
}
