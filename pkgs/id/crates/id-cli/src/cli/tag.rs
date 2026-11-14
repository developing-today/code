//! `id tag` subcommand definitions: set, delete, list, search.

use std::path::PathBuf;

use clap::Subcommand;

/// Subcommands for `id tag`.
///
/// Manage metadata tags (key/value pairs) attached to files.
/// Tags are stored in iroh-docs CRDT documents and support
/// conflict-free replication across peers.
#[derive(Subcommand, Debug)]
pub enum TagCommand {
    /// Set a metadata tag on a file.
    ///
    /// Creates or adds a key/value pair on the specified file.
    /// Multiple values can exist per key.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id tag set README.md priority high
    /// id tag set README.md pinned
    /// ```
    #[command(alias = "add")]
    Set {
        /// The file to tag.
        file: String,
        /// The tag key.
        key: String,
        /// Optional tag value (UTF-8 text).
        #[arg(conflicts_with_all = ["value_hex", "value_file"])]
        value: Option<String>,
        /// Tag value as hex-encoded raw bytes (binary-safe).
        #[arg(long, value_name = "HEX", conflicts_with = "value_file")]
        value_hex: Option<String>,
        /// Read the raw tag value from a file (`-` for stdin).
        #[arg(long, value_name = "PATH")]
        value_file: Option<PathBuf>,
    },
    /// Delete a metadata tag from a file.
    ///
    /// With a value, removes exactly that `(key, value)` tag. Without one,
    /// removes **every** tag with that key (all values, and a value-less
    /// key-only tag).
    ///
    /// # Examples
    ///
    /// ```bash
    /// id tag del README.md priority high   # only priority=high
    /// id tag del README.md priority        # priority=* and bare "priority"
    /// ```
    #[command(aliases = ["rm", "remove", "rem", "delete", "unset"])]
    Del {
        /// The file to untag.
        file: String,
        /// The tag key to remove.
        key: String,
        /// Specific value to remove (UTF-8 text). Omit to remove all values.
        #[arg(conflicts_with = "value_hex")]
        value: Option<String>,
        /// Specific value to remove, as hex-encoded raw bytes.
        #[arg(long, value_name = "HEX")]
        value_hex: Option<String>,
    },
    /// List metadata tags.
    ///
    /// Lists all tags, or tags for a specific file.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id tag list
    /// id tag list README.md
    /// ```
    #[command(alias = "ls")]
    List {
        /// File to list tags for (omit for all).
        file: Option<String>,
        /// Show non-UTF-8 values as hex (`0x...`) instead of `<binary N bytes>`.
        #[arg(long)]
        hex: bool,
        /// Show non-UTF-8 values as lossy text instead of `<binary N bytes>`.
        #[arg(long)]
        binary: bool,
        /// Don't truncate long values (default: truncate at 256 bytes).
        #[arg(long)]
        no_truncate: bool,
    },
    /// Search metadata tags using structured query syntax.
    ///
    /// Query syntax:
    /// - `key:` — filter by key name
    /// - `:value` — filter by value
    /// - `key:value` — filter by exact key-value pair
    /// - `"literal"` — search all fields for literal text
    /// - `bare` — case-insensitive search across subject/key/value
    ///
    /// Quoted strings can be used in key:value position:
    /// `"key:":":value"` matches key `key:` with value `:value`.
    ///
    /// Multiple terms are `ANDed` together.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id tag search name:
    /// id tag search name:myfile.txt
    /// id tag search :high
    /// id tag search priority
    /// ```
    #[command(alias = "find")]
    Search {
        /// Search query terms.
        #[arg(num_args = 1.., required = true)]
        query: Vec<String>,
        /// Show non-UTF-8 values as hex (`0x...`) instead of `<binary N bytes>`.
        #[arg(long)]
        hex: bool,
        /// Show non-UTF-8 values as lossy text instead of `<binary N bytes>`.
        #[arg(long)]
        binary: bool,
        /// Don't truncate long values (default: truncate at 256 bytes).
        #[arg(long)]
        no_truncate: bool,
    },
}
