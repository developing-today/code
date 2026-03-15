//! Find and search command handlers for locating blobs by pattern matching.
//!
//! This module provides fuzzy search capabilities for the blob store, allowing
//! users to locate files by partial name or hash matches. It supports both
//! local and remote searching.
//!
//! # Commands
//!
//! - **find**: Search and retrieve matching files (outputs content by default)
//! - **search**: Search and list matches (metadata only, optionally retrieve)
//!
//! # Match Types
//!
//! The search algorithm recognizes three types of matches, in priority order:
//!
//! 1. **Exact**: Query exactly matches the name or hash
//! 2. **Prefix**: Name or hash starts with the query
//! 3. **Contains**: Name or hash contains the query anywhere
//!
//! # Output Formats
//!
//! Both commands support multiple output formats via `--format`:
//!
//! - **tag**: Shows query tag with each match
//! - **group**: Groups matches by query
//! - **union**: Default format, shows all matches with query suffix
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────┐
//! │                    cmd_find / cmd_search                       │
//! │            (CLI handlers with output formatting)               │
//! └────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌────────────────────────────────────────────────────────────────┐
//! │                     cmd_find_matches                           │
//! │     (core search logic, works locally or via remote node)      │
//! └────────────────────────────────────────────────────────────────┘
//!                    │                        │
//!                    ▼                        ▼
//!            ┌──────────────┐         ┌──────────────┐
//!            │ Local Store  │         │ Remote Node  │
//!            │ (tags list)  │         │ (MetaRequest)│
//!            └──────────────┘         └──────────────┘
//! ```
//!
//! # Examples
//!
//! Find files matching "config":
//! ```bash
//! id find config
//! ```
//!
//! Search for multiple patterns, output all to a directory:
//! ```bash
//! id search "*.json" "*.toml" --all --dir ./configs
//! ```
//!
//! Search on a remote node:
//! ```bash
//! id find config --node <NODE_ID>
//! ```

use anyhow::{Result, bail};
use futures_lite::StreamExt;
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::{Endpoint, RelayMode},
};
use iroh_base::EndpointId;

use crate::{
    CLIENT_KEY_FILE, META_ALPN,
    FindMatch, MetaRequest, MetaResponse, TaggedMatch,
    load_or_create_keypair, open_store,
    print_match_cli, print_matches_cli, match_kind,
    cmd_get_one, cmd_get_one_remote,
};

/// Find files matching queries and output their content.
///
/// This is the main handler for the `id find` command. It searches for blobs
/// matching the given queries and outputs their content. By default, it outputs
/// to stdout; with `--file` or `--dir`, it saves to files.
///
/// # Behavior
///
/// - **Single match**: Outputs directly to stdout or file
/// - **Multiple matches**: Lists all matches with details, uses first match
/// - **`--all` mode**: Outputs all matching files (deduplicated by hash+name)
///
/// # Arguments
///
/// * `queries` - One or more search patterns to match against names/hashes
/// * `prefer_name` - If true, prioritize name matches over hash matches
/// * `to_stdout` - If true, always output content to stdout
/// * `all` - If true, output all matches (not just the first)
/// * `dir` - Optional directory to save all matching files
/// * `format` - Output format: "tag", "group", or "union"
/// * `node` - Optional remote node ID to search on
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Errors
///
/// Returns an error if no matches are found for any query.
///
/// # Example
///
/// ```bash
/// id find config              # Find and output first match
/// id find config --all        # Output all matches to stdout
/// id find "*.json" --dir ./   # Save all JSON files to current directory
/// ```
pub async fn cmd_find(
    queries: Vec<String>,
    prefer_name: bool,
    to_stdout: bool,
    all: bool,
    dir: Option<String>,
    format: &str,
    node: Option<String>,
    no_relay: bool,
) -> Result<()> {
    // Collect matches for all queries
    let mut all_matches: Vec<TaggedMatch> = Vec::new();
    for query in &queries {
        let matches = cmd_find_matches(query, prefer_name, node.clone(), no_relay).await?;
        for m in matches {
            all_matches.push(TaggedMatch {
                query: query.clone(),
                hash: m.hash,
                name: m.name,
                kind: m.kind,
                is_hash_match: m.is_hash_match,
            });
        }
    }

    if all_matches.is_empty() {
        bail!("no matches found for: {}", queries.join(", "));
    }

    // --all mode: output all matches
    if all {
        if let Some(ref dir_path) = dir {
            std::fs::create_dir_all(dir_path)?;
            // Deduplicate by hash+name for file output
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    let output_path = format!("{}/{}", dir_path, m.name);
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, &output_path, no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, &output_path, false, false).await?;
                    }
                    print_match_cli(m, format);
                }
            }
        } else {
            // Output all to stdout (concatenated)
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, "-", no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, "-", false, false).await?;
                    }
                }
            }
        }
        return Ok(());
    }

    // Single match or first match mode
    if all_matches.len() == 1 {
        let m = &all_matches[0];
        let output = if to_stdout { "-" } else { &m.name };
        if node.is_some() {
            let node_id: EndpointId = node.as_ref().unwrap().parse()?;
            cmd_get_one_remote(node_id, &m.name, output, no_relay).await?;
        } else {
            cmd_get_one(&m.name, output, false, false).await?;
        }
    } else {
        // Multiple matches - print them and use first one
        eprintln!("found {} matches (using first):", all_matches.len());
        print_matches_cli(&all_matches, format);
        let m = &all_matches[0];
        let output = if to_stdout { "-" } else { &m.name };
        if let Some(node_str) = node {
            let node_id: EndpointId = node_str.parse()?;
            cmd_get_one_remote(node_id, &m.name, output, no_relay).await?;
        } else {
            cmd_get_one(&m.name, output, false, false).await?;
        }
    }
    Ok(())
}

/// Search for files matching queries and list the results.
///
/// This is the main handler for the `id search` command. Unlike `find`, it
/// defaults to listing matches (metadata only) without outputting file content.
/// Use `--all` to also retrieve the matching files.
///
/// # Behavior
///
/// - **Default**: Lists all matches with hash, name, match type, and query
/// - **`--all` mode**: Lists matches and also outputs file content
/// - **`--file`**: After listing, save the first match to a file
///
/// # Arguments
///
/// * `queries` - One or more search patterns to match against names/hashes
/// * `prefer_name` - If true, prioritize name matches over hash matches
/// * `all` - If true, also output file content for all matches
/// * `dir` - Optional directory to save all matching files
/// * `format` - Output format: "tag", "group", or "union"
/// * `node` - Optional remote node ID to search on
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Example
///
/// ```bash
/// id search config            # List all matches
/// id search config --all      # List and output all matches
/// id search "*.json" --dir ./ # List and save all JSON files
/// ```
pub async fn cmd_search(
    queries: Vec<String>,
    prefer_name: bool,
    all: bool,
    dir: Option<String>,
    format: &str,
    node: Option<String>,
    no_relay: bool,
) -> Result<()> {
    // Collect matches for all queries
    let mut all_matches: Vec<TaggedMatch> = Vec::new();
    for query in &queries {
        let matches = cmd_find_matches(query, prefer_name, node.clone(), no_relay).await?;
        for m in matches {
            all_matches.push(TaggedMatch {
                query: query.clone(),
                hash: m.hash,
                name: m.name,
                kind: m.kind,
                is_hash_match: m.is_hash_match,
            });
        }
    }

    if all_matches.is_empty() {
        println!("no matches found for: {}", queries.join(", "));
        return Ok(());
    }

    // --all mode: output all files (like find --all)
    if all {
        if let Some(ref dir_path) = dir {
            std::fs::create_dir_all(dir_path)?;
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    let output_path = format!("{}/{}", dir_path, m.name);
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, &output_path, no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, &output_path, false, false).await?;
                    }
                    print_match_cli(m, format);
                }
            }
        } else {
            // Output all to stdout (concatenated)
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, "-", no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, "-", false, false).await?;
                    }
                }
            }
        }
        return Ok(());
    }

    // Default: just list matches
    print_matches_cli(&all_matches, format);
    Ok(())
}

/// Get matching entries for a query, either locally or from a remote node.
///
/// This is the core search function used by both `cmd_find` and `cmd_search`.
/// It performs case-insensitive matching against both tag names and blob hashes.
///
/// # Search Algorithm
///
/// For each stored blob:
/// 1. Convert name to lowercase
/// 2. Check if query matches name (exact → prefix → contains)
/// 3. If no name match, check if query matches hash string
/// 4. Collect all matches with their match type and source (name vs hash)
///
/// Results are sorted by:
/// 1. Match kind (Exact > Prefix > Contains)
/// 2. Match source (name vs hash, controlled by `prefer_name`)
///
/// # Arguments
///
/// * `query` - The search pattern (case-insensitive)
/// * `prefer_name` - If true, name matches sort before hash matches
/// * `node` - Optional remote node ID; if None, search locally
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Returns
///
/// A vector of [`FindMatch`] entries, sorted by relevance. Each entry contains:
/// - `hash`: The blob's content hash
/// - `name`: The blob's tag name
/// - `kind`: Match type (Exact, Prefix, or Contains)
/// - `is_hash_match`: Whether the query matched the hash (vs name)
///
/// # Remote Protocol
///
/// When `node` is Some, sends `MetaRequest::Find { query, prefer_name }`
/// and receives `MetaResponse::Find { matches }`.
pub async fn cmd_find_matches(
    query: &str,
    prefer_name: bool,
    node: Option<String>,
    no_relay: bool,
) -> Result<Vec<FindMatch>> {
    if let Some(node_str) = node {
        let node_id: EndpointId = node_str.parse()?;
        let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
        let mut builder = Endpoint::builder()
            .secret_key(client_key)
            .address_lookup(PkarrPublisher::n0_dns())
            .address_lookup(DnsAddressLookup::n0_dns());
        if no_relay {
            builder = builder.relay_mode(RelayMode::Disabled);
        }
        let endpoint = builder.bind().await?;

        let meta_conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Find {
            query: query.to_string(),
            prefer_name,
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        match resp {
            MetaResponse::Find { matches } => Ok(matches),
            _ => bail!("unexpected response"),
        }
    } else {
        // Local search
        let store = open_store(false).await?;
        let store_handle = store.as_store();
        let mut matches = Vec::new();
        let query_lower = query.to_lowercase();

        if let Ok(mut list) = store_handle.tags().list().await {
            while let Some(item) = list.next().await {
                if let Ok(item) = item {
                    let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                    let hash_str = item.hash.to_string();
                    let name_lower = name.to_lowercase();

                    if let Some(kind) = match_kind(&name_lower, &query_lower) {
                        matches.push(FindMatch {
                            hash: item.hash,
                            name: name.clone(),
                            kind,
                            is_hash_match: false,
                        });
                    } else if let Some(kind) = match_kind(&hash_str, &query_lower) {
                        matches.push(FindMatch {
                            hash: item.hash,
                            name,
                            kind,
                            is_hash_match: true,
                        });
                    }
                }
            }
        }

        matches.sort_by(|a, b| match a.kind.cmp(&b.kind) {
            std::cmp::Ordering::Equal => {
                if prefer_name {
                    a.is_hash_match.cmp(&b.is_hash_match)
                } else {
                    b.is_hash_match.cmp(&a.is_hash_match)
                }
            }
            other => other,
        });

        store.shutdown().await?;
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MatchKind;

    #[test]
    fn test_match_kind_exact() {
        assert_eq!(match_kind("hello", "hello"), Some(MatchKind::Exact));
    }

    #[test]
    fn test_match_kind_prefix() {
        assert_eq!(match_kind("hello world", "hello"), Some(MatchKind::Prefix));
    }

    #[test]
    fn test_match_kind_contains() {
        assert_eq!(match_kind("say hello to me", "hello"), Some(MatchKind::Contains));
    }

    #[test]
    fn test_match_kind_no_match() {
        assert_eq!(match_kind("goodbye", "hello"), None);
    }
}
