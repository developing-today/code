//! Put command handlers - store blobs with optional naming.
//!
//! This module implements various ways to store content:
//!
//! - **Local files**: Store files from the filesystem
//! - **Stdin content**: Store data piped from stdin
//! - **Remote storage**: Push content to remote nodes
//! - **Hash-only storage**: Store without creating named tags
//!
//! # Storage Flow
//!
//! ```text
//! Input (file/stdin)
//!        │
//!        ▼
//! ┌──────────────┐
//! │ Add to local │
//! │    store     │
//! └──────────────┘
//!        │
//!        ├─── If local serve running ───┐
//!        │                               ▼
//!        │                    ┌───────────────────┐
//!        │                    │ Push blob to serve│
//!        │                    │ Create tag via    │
//!        │                    │ meta protocol     │
//!        │                    └───────────────────┘
//!        │
//!        └─── If remote node ───────────┐
//!                                        ▼
//!                             ┌───────────────────┐
//!                             │ Push blob to remote│
//!                             │ Create tag via     │
//!                             │ meta protocol      │
//!                             └───────────────────┘
//! ```
//!
//! # Examples
//!
//! ```bash
//! # Store a file
//! id put file.txt
//!
//! # Store with custom name
//! id put file.txt:config
//!
//! # Store from stdin
//! echo "hello" | id put --content greeting
//!
//! # Store to remote
//! id put NODE_ID file.txt
//! ```

use anyhow::{Context, Result, bail};
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::{Endpoint, RelayMode},
};
use iroh_base::EndpointId;
use iroh_blobs::{
    ALPN as BLOBS_ALPN, BlobFormat,
    api::blobs::AddBytesOptions,
    protocol::{ChunkRanges, ChunkRangesSeq, PushRequest},
};
use std::io::IsTerminal;
use std::path::PathBuf;
use tokio::fs as afs;

use crate::{
    CLIENT_KEY_FILE, META_ALPN, MetaRequest, MetaResponse, create_local_client_endpoint,
    get_serve_info, is_node_id, load_or_create_keypair, open_store, parse_put_spec,
    parse_stdin_items, read_input,
};

/// Stores content by hash only, without creating a named tag.
///
/// Useful when you only need the content-addressed hash and don't
/// need a human-readable name to reference it.
///
/// # Arguments
///
/// * `source` - File path to store, or `"-"` to read from stdin
///
/// # Output
///
/// Prints the hash to stdout.
///
/// # Example
///
/// ```rust,ignore
/// cmd_put_hash("data.bin").await?;
/// // Prints: abc123...def456
/// ```
pub async fn cmd_put_hash(source: &str) -> Result<()> {
    let data = if source == "-" {
        read_input("-").await?
    } else {
        afs::read(source).await?
    };

    if let Some(serve_info) = get_serve_info().await {
        // Store in local ephemeral store, push blob to serve
        let store = open_store(true).await?;
        let store_handle = store.as_store();

        let added = store_handle
            .add_bytes_with_opts(AddBytesOptions {
                data: data.into(),
                format: BlobFormat::Raw,
            })
            .await?;
        let hash = added.hash;

        let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

        let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
        let push_request =
            PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
        store_handle
            .remote()
            .execute_push(blobs_conn.clone(), push_request)
            .await?;
        blobs_conn.close(0u32.into(), b"done");

        println!("{hash}");
        store.shutdown().await?;
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        let added = store_handle
            .add_bytes_with_opts(AddBytesOptions {
                data: data.into(),
                format: BlobFormat::Raw,
            })
            .await?;

        println!("{}", added.hash);
        store.shutdown().await?;
    }
    Ok(())
}

/// Stores a local file with an optional custom name.
///
/// If no custom name is provided, the filename is used as the tag name.
///
/// # Arguments
///
/// * `path` - Path to the file to store
/// * `custom_name` - Optional tag name (defaults to filename)
///
/// # Output
///
/// Prints `stored: <name> -> <hash>` to stderr.
pub async fn cmd_put_local_file(path: &str, custom_name: Option<String>) -> Result<()> {
    let path = PathBuf::from(path);
    let filename = custom_name.unwrap_or_else(|| {
        path.file_name()
            .map_or_else(|| "unnamed".to_owned(), |s| s.to_string_lossy().to_string())
    });
    let data = afs::read(&path).await?;

    if let Some(serve_info) = get_serve_info().await {
        let store = open_store(true).await?;
        let store_handle = store.as_store();

        let added = store_handle
            .add_bytes_with_opts(AddBytesOptions {
                data: data.into(),
                format: BlobFormat::Raw,
            })
            .await?;
        let hash = added.hash;

        let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

        let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Put {
            filename: filename.clone(),
            hash,
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        match resp {
            MetaResponse::Put { success: true } => {
                let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                let push_request =
                    PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                store_handle
                    .remote()
                    .execute_push(blobs_conn.clone(), push_request)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");
                eprintln!("stored: {filename} -> {hash}");
                store.shutdown().await?;
            }
            MetaResponse::Put { success: false } => bail!("server rejected"),
            _ => bail!("unexpected response"),
        }
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        let added = store_handle
            .add_bytes_with_opts(AddBytesOptions {
                data: data.into(),
                format: BlobFormat::Raw,
            })
            .await?;

        store_handle.tags().set(&filename, added.hash).await?;
        eprintln!("stored: {} -> {}", filename, added.hash);
        store.shutdown().await?;
    }
    Ok(())
}

/// Stores content from stdin with a given name.
///
/// # Arguments
///
/// * `name` - The tag name for the stored content
///
/// # Output
///
/// Prints `stored: <name> -> <hash>` to stderr.
pub async fn cmd_put_local_stdin(name: &str) -> Result<()> {
    let data = read_input("-").await?;

    if let Some(serve_info) = get_serve_info().await {
        let store = open_store(true).await?;
        let store_handle = store.as_store();

        let added = store_handle
            .add_bytes_with_opts(AddBytesOptions {
                data: data.into(),
                format: BlobFormat::Raw,
            })
            .await?;
        let hash = added.hash;

        let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

        let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Put {
            filename: name.to_owned(),
            hash,
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        match resp {
            MetaResponse::Put { success: true } => {
                let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                let push_request =
                    PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                store_handle
                    .remote()
                    .execute_push(blobs_conn.clone(), push_request)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");
                eprintln!("stored: {name} -> {hash}");
                store.shutdown().await?;
            }
            MetaResponse::Put { success: false } => bail!("server rejected"),
            _ => bail!("unexpected response"),
        }
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        let added = store_handle
            .add_bytes_with_opts(AddBytesOptions {
                data: data.into(),
                format: BlobFormat::Raw,
            })
            .await?;

        store_handle.tags().set(name, added.hash).await?;
        eprintln!("stored: {} -> {}", name, added.hash);
        store.shutdown().await?;
    }
    Ok(())
}

/// Stores a single file locally (used by multi-put).
///
/// # Arguments
///
/// * `path` - Path to the file
/// * `name` - Optional custom tag name
/// * `hash_only` - If true, store without creating a tag
pub async fn cmd_put_one(path: &str, name: Option<&str>, hash_only: bool) -> Result<()> {
    if hash_only {
        cmd_put_hash(path).await
    } else {
        cmd_put_local_file(path, name.map(ToOwned::to_owned)).await
    }
}

/// Stores a single file on a remote node.
///
/// Adds the content to a local ephemeral store, creates the tag on
/// the remote via the meta protocol, then pushes the blob content.
///
/// # Arguments
///
/// * `server_node_id` - The remote node's identity
/// * `path` - Path to the file
/// * `name` - Optional custom tag name
/// * `no_relay` - Disable relay servers
///
/// # Output
///
/// Prints `uploaded: <name> -> <hash>` to stdout.
pub async fn cmd_put_one_remote(
    server_node_id: EndpointId,
    path: &str,
    name: Option<&str>,
    no_relay: bool,
) -> Result<()> {
    let path_buf = PathBuf::from(path);
    let filename = if let Some(n) = name {
        n.to_owned()
    } else {
        path_buf
            .file_name()
            .context("invalid filename")?
            .to_string_lossy()
            .to_string()
    };

    let store = open_store(true).await?;
    let store_handle = store.as_store();

    let data = afs::read(&path_buf).await?;
    let added = store_handle
        .add_bytes_with_opts(AddBytesOptions {
            data: data.into(),
            format: BlobFormat::Raw,
        })
        .await?;
    let hash = added.hash;

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
    let req = postcard::to_allocvec(&MetaRequest::Put {
        filename: filename.clone(),
        hash,
    })?;
    send.write_all(&req).await?;
    send.finish()?;
    let resp_buf = recv.read_to_end(64 * 1024).await?;
    let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
    meta_conn.close(0u32.into(), b"done");

    match resp {
        MetaResponse::Put { success: true } => {
            let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
            let push_request =
                PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
            store_handle
                .remote()
                .execute_push(blobs_conn.clone(), push_request)
                .await?;
            blobs_conn.close(0u32.into(), b"done");
            println!("uploaded: {filename} -> {hash}");
            store.shutdown().await?;
        }
        MetaResponse::Put { success: false } => bail!("server rejected"),
        _ => bail!("unexpected response"),
    }
    Ok(())
}

/// Stores multiple files (local or remote).
///
/// This is the main entry point for the `put` command. It handles:
/// - Content mode (stdin as content)
/// - Remote targeting (first arg is `NODE_ID`)
/// - Multiple file specs
/// - Stdin path reading
///
/// # Arguments
///
/// * `files` - File specs (`path` or `path:name`)
/// * `content_mode` - Read content from stdin
/// * `from_stdin` - Read file paths from stdin
/// * `hash_only` - Store without creating tags
/// * `no_relay` - Disable relay servers
///
/// # Errors
///
/// Collects errors from individual puts and reports them all at the end.
pub async fn cmd_put_multi(
    files: Vec<String>,
    content_mode: bool,
    from_stdin: bool,
    hash_only: bool,
    no_relay: bool,
) -> Result<()> {
    // Content mode: read stdin as content, store with given name
    if content_mode {
        if files.len() != 1 {
            bail!("--content requires exactly one name argument");
        }
        let name = &files[0];
        if hash_only {
            return cmd_put_hash("-").await;
        }
        return cmd_put_local_stdin(name).await;
    }

    let mut items = files;

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

    // Auto-detect stdin content: if exactly one arg (the name) and stdin is piped
    if items.len() == 1 && !std::io::stdin().is_terminal() && !from_stdin {
        // Check if the item looks like a file path that exists
        let path = PathBuf::from(&items[0]);
        if !path.exists() {
            // Doesn't exist as a file, treat as name and read content from stdin
            let name = &items[0];
            if hash_only {
                return cmd_put_hash("-").await;
            }
            return cmd_put_local_stdin(name).await;
        }
    }

    if items.is_empty() {
        bail!("no files provided");
    }

    let mut errors = Vec::new();
    for spec in &items {
        let (path, name) = parse_put_spec(spec);
        let result = if let Some(node_id) = remote_node {
            cmd_put_one_remote(node_id, path, name, no_relay).await
        } else {
            cmd_put_one(path, name, hash_only).await
        };
        if let Err(e) = result {
            errors.push(format!("{spec}: {e}"));
        }
    }

    if !errors.is_empty() {
        bail!("some puts failed:\n{}", errors.join("\n"));
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
    fn test_parse_put_spec_integration() {
        // Simple path
        let (path, name) = parse_put_spec("file.txt");
        assert_eq!(path, "file.txt");
        assert_eq!(name, None);

        // Path with custom name
        let (path, name) = parse_put_spec("file.txt:custom");
        assert_eq!(path, "file.txt");
        assert_eq!(name, Some("custom"));
    }

    #[tokio::test]
    async fn test_cmd_put_hash_nonexistent_file() {
        let result = cmd_put_hash("/nonexistent/path/file.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cmd_put_local_file_nonexistent() {
        let result = cmd_put_local_file("/nonexistent/path/file.txt", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cmd_put_multi_empty_no_files() {
        let result = cmd_put_multi(vec![], false, false, false, false).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no files provided")
        );
    }

    #[tokio::test]
    async fn test_cmd_put_multi_content_requires_one_name() {
        // Content mode with no args
        let result = cmd_put_multi(vec![], true, false, false, false).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("--content requires exactly one name argument")
        );

        // Content mode with multiple args
        let result = cmd_put_multi(vec!["a".into(), "b".into()], true, false, false, false).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("--content requires exactly one name argument")
        );
    }

    #[tokio::test]
    async fn test_cmd_put_one_nonexistent() {
        let result = cmd_put_one("/nonexistent/file.txt", None, false).await;
        assert!(result.is_err());
    }
}
