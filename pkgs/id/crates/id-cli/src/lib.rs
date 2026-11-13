//! The id command-line application layer: `cli` (argument parsing),
//! `commands` (subcommand implementations), `repl`, and
//! `server_config`, plus the support modules they share (`local`,
//! `protocol`, `meta_client`, `helpers`, `chat_presence`,
//! `app_manifest`, `match_audit`, `aliases`, `publications`).
//!
//! Extracted from the facade in split step 7b (#4411). The facade
//! re-exports these modules at their historical `id::` paths and
//! keeps `main.rs`; this crate depends only downward on the member
//! crates (id-core, id-caps, id-store, id-events, id-groups,
//! id-sync, id-worlds, id-directory, and id-web under `web`).

use anyhow::Result;
use std::path::PathBuf;

pub mod aliases;
pub mod app_manifest;
pub mod chat_presence;
pub mod cli;
pub mod commands;
pub mod helpers;
pub mod local;
pub mod match_audit;
pub mod meta_client;
mod peer_announcement;
pub mod protocol;
pub mod publications;
pub mod publications_runtime;
pub mod repl;
pub mod server_config;

#[cfg(test)]
mod fileops_tests;

// Root convenience re-exports, mirroring the layer the facade root
// provided when these modules lived there: moved code imports
// several of these as `crate::<name>`.
pub use id_core::{CLIENT_KEY_FILE, KEY_FILE, SERVE_LOCK};
pub use id_core::{is_format_char, is_node_id};
pub use id_store::STORE_PATH;
pub use id_sync::protocol::META_ALPN;

pub use crate::cli::{Cli, Command, KeyCommand, TagCommand};
pub use crate::commands::{
    PeekOptions, PeersOptions, ReplContext, ReplContextInner, SearchOptions, ServeInfo,
    ServeOptions, cmd_find, cmd_find_matches, cmd_get_local, cmd_get_multi, cmd_get_one,
    cmd_get_one_remote, cmd_gethash, cmd_id, cmd_list, cmd_list_remote, cmd_migrate_tags, cmd_peek,
    cmd_peers, cmd_put_hash, cmd_put_local_file, cmd_put_local_stdin, cmd_put_multi, cmd_put_one,
    cmd_put_one_remote, cmd_search, cmd_serve, cmd_show, cmd_tag, create_local_client_endpoint,
    create_serve_lock, get_serve_info, is_process_alive, remove_serve_lock,
};
pub use crate::helpers::{
    download_output_path, parse_get_spec, parse_put_spec, print_match_cli, print_match_repl,
    print_matches_cli,
};
pub use crate::protocol::match_kind;
pub use crate::protocol::{
    FindMatch, MatchKind, MetaProtocol, MetaRequest, MetaResponse, TaggedMatch,
};
pub use crate::repl::run_repl;
pub use id_core::discovery::{
    ANNOUNCE_INTERVAL, DEFAULT_TOPIC, DEFAULT_TOPIC_SECRET, Defaults, MAX_PEER_NAME_BYTES,
    MAX_PEERS, PeerAnnouncement, PeerDiscovery, PeerInfo, ResolvedConfig, STALE_CHECK_INTERVAL,
    STALE_THRESHOLD, defaults, parse_defaults, resolve_config,
};
pub use id_store::store::{StoreType, load_or_create_keypair, open_store};

// Root helpers moved from the facade root in step 7b: they serve
// the command layer (get/put/repl) and are re-exported by the facade
// at their historical `id::` paths.

/// Parses items from stdin, splitting on common delimiters.
///
/// Reads all content from stdin and splits it on newlines, tabs, or commas.
/// Empty items and whitespace-only items are filtered out. This is useful
/// for batch operations where file lists are piped in.
///
/// # Returns
///
/// A vector of trimmed, non-empty strings.
///
/// # Example
///
/// ```bash
/// # From command line:
/// echo "file1.txt,file2.txt,file3.txt" | id put --stdin
/// ```
///
/// ```rust,no_run
/// use id_cli::parse_stdin_items;
///
/// // If stdin contains "a.txt\nb.txt\tc.txt,d.txt"
/// let items = parse_stdin_items().unwrap();
/// // items = ["a.txt", "b.txt", "c.txt", "d.txt"]
/// ```
pub fn parse_stdin_items() -> Result<Vec<String>> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    Ok(input
        .split(['\n', '\t', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

/// Reads input data from a file path or stdin.
///
/// If the input is "-", reads from stdin. Otherwise, reads from the specified
/// file path. This is a common pattern for CLI tools that accept either a
/// file or piped input.
///
/// # Arguments
///
/// * `input` - Either "-" for stdin, or a file path
///
/// # Returns
///
/// The raw bytes read from the source.
///
/// # Example
///
/// ```rust,ignore
/// use id_cli::read_input;
///
/// # async fn example() -> anyhow::Result<()> {
/// // Read from file
/// let data = read_input("myfile.txt").await?;
///
/// // Read from stdin (when input is "-")
/// let stdin_data = read_input("-").await?;
/// # Ok(())
/// # }
/// ```
pub async fn read_input(input: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    use tokio::fs as afs;

    if input == "-" {
        let mut data = Vec::new();
        std::io::stdin().read_to_end(&mut data)?;
        Ok(data)
    } else {
        Ok(afs::read(input).await?)
    }
}

/// Exports a blob to a file or stdout.
///
/// Retrieves the blob data by hash from the store and writes it to the
/// specified output. If output is "-", writes to stdout. Otherwise,
/// writes to a file at the given path (converting to absolute if needed).
///
/// # Arguments
///
/// * `store` - The blob store to read from
/// * `hash` - The hash of the blob to export
/// * `output` - Either "-" for stdout, or a file path
///
/// # Example
///
/// ```rust,ignore
/// use id_cli::{open_store, export_blob};
/// use iroh_blobs::Hash;
///
/// # async fn example() -> anyhow::Result<()> {
/// let store_type = open_store(false).await?;
/// let store = store_type.as_store();
///
/// // Export to file
/// let hash: Hash = "abc123...".parse()?;
/// export_blob(&store, hash, "output.txt").await?;
///
/// // Export to stdout
/// export_blob(&store, hash, "-").await?;
/// # Ok(())
/// # }
/// ```
pub async fn export_blob(
    store: &iroh_blobs::api::Store,
    hash: iroh_blobs::Hash,
    output: &str,
) -> Result<()> {
    use std::io::Write;

    if output == "-" {
        let data = store.blobs().get_bytes(hash).await?;
        std::io::stdout().write_all(&data)?;
    } else {
        let path = to_absolute(&PathBuf::from(output))?;
        store.blobs().export(hash, &path).await?;
        eprintln!("exported: {} -> {}", hash, path.display());
    }
    Ok(())
}

/// Converts a potentially relative path to an absolute path.
///
/// If the path is already absolute, it is returned unchanged. Otherwise,
/// it is joined with the current working directory.
///
/// # Arguments
///
/// * `path` - The path to convert
///
/// # Returns
///
/// The absolute path, or an error if the current directory cannot be determined.
///
/// # Example
///
/// ```rust
/// use std::path::PathBuf;
/// use id_cli::to_absolute;
///
/// let abs = to_absolute(&PathBuf::from("/already/absolute")).unwrap();
/// assert_eq!(abs, PathBuf::from("/already/absolute"));
///
/// // Relative paths are joined with the current directory
/// let rel = to_absolute(&PathBuf::from("relative/path")).unwrap();
/// assert!(rel.is_absolute());
/// ```
pub fn to_absolute(path: &PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.clone())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
