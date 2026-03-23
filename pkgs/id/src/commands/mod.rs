//! Command implementations for the `id` CLI tool.
//!
//! This module contains all command handlers organized by function:
//!
//! - **[`serve`]**: Server that accepts connections from peers
//! - **[`client`]**: Client endpoint creation for connecting to serve
//! - **[`put`]**: Store blobs (local, remote, stdin, files)
//! - **[`get`]**: Retrieve blobs (local, remote, by name, by hash)
//! - **[`find`]**: Search and retrieve matching blobs
//! - **[`list`]**: List stored blobs
//! - **[`id`]**: Print node identity
//! - **[`peers`]**: Discover and list known peers
//! - **[`repl`]**: Interactive REPL context management
//!
//! # Command Flow
//!
//! Commands follow a consistent pattern for local vs remote operations:
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────────┐
//! │                      Command Entry                            │
//! └───────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//!              ┌───────────────────────────────┐
//!              │   Is first arg a NODE_ID?     │
//!              │      (64 hex chars)           │
//!              └───────────────────────────────┘
//!                    │                  │
//!                   Yes                 No
//!                    │                  │
//!                    ▼                  ▼
//!          ┌─────────────────┐  ┌─────────────────┐
//!          │  Remote Mode    │  │  Local Mode     │
//!          │  - Connect via  │  │  - Check for    │
//!          │    relay/direct │  │    running serve│
//!          │  - Use protocol │  │  - Open store   │
//!          └─────────────────┘  │    directly     │
//!                               └─────────────────┘
//! ```
//!
//! # Examples
//!
//! ```rust,ignore
//! use id::commands::{cmd_put_local_file, cmd_get_local, cmd_list};
//!
//! // Store a file
//! cmd_put_local_file("./data.txt", Some("my-data".to_string())).await?;
//!
//! // List all stored files
//! cmd_list(None, false).await?;
//!
//! // Retrieve the file
//! cmd_get_local("my-data", "./output.txt").await?;
//! ```

pub mod client;
pub mod find;
pub mod get;
pub mod id;
pub mod list;
pub mod peers;
pub mod put;
pub mod repl;
pub mod serve;

pub use client::create_local_client_endpoint;
pub use find::{
    PeekOptions, SearchOptions, cmd_find, cmd_find_matches, cmd_peek, cmd_search, cmd_show,
};
pub use get::{cmd_get_local, cmd_get_multi, cmd_get_one, cmd_get_one_remote, cmd_gethash};
pub use id::cmd_id;
pub use list::{cmd_list, cmd_list_remote};
pub use peers::{PeersOptions, cmd_peers};
pub use put::{
    cmd_put_hash, cmd_put_local_file, cmd_put_local_stdin, cmd_put_multi, cmd_put_one,
    cmd_put_one_remote,
};
pub use repl::{ReplContext, ReplContextInner};
pub use serve::{
    ServeInfo, cmd_serve, create_serve_lock, get_serve_info, is_process_alive, remove_serve_lock,
};
