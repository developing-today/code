//! `id walk` subcommand definitions: the #9106 walking skeleton,
//! driven by hand (#12232 step B3).

use std::path::PathBuf;

use clap::Subcommand;

/// Subcommands for `id walk`.
///
/// One stored node per directory; the five steps of the
/// walking-skeleton path as commands: `init` a node, `write` a
/// signed record at an `id://` address, `serve` the node so a peer
/// can sync with it, `sync` to the peer, `read` the record back
/// from the other node's store, and `reports` for the conflict a
/// diverged write opens (a `write` naming both sides as parents
/// resolves it).
#[derive(Subcommand, Debug)]
pub enum WalkCommand {
    /// Create a walk node in a directory (an existing node keeps
    /// its records; its recorded name must match).
    Init {
        /// The node directory.
        dir: PathBuf,
        /// The node's name: the resolver name in the addresses it
        /// mints, and the source label on what it produces.
        #[arg(long)]
        name: String,
    },
    /// Write a signed record (a note) on a node; prints the
    /// record's `id://` address and event id.
    Write {
        /// The node directory.
        dir: PathBuf,
        /// The demo writer signing the record: `alice` or `bob`.
        #[arg(long = "as")]
        writer: String,
        /// The note's text.
        #[arg(long)]
        text: String,
        /// A parent version (hex event id) the record follows;
        /// repeat to name several — two or more parents resolve
        /// the conflict between them. Default: the node's current
        /// heads for the demo graph.
        #[arg(long)]
        parent: Vec<String>,
    },
    /// Read the record an `id://` address names, from a node's
    /// store. The address may have been minted by any node: the
    /// address's target names the record, and the serving node
    /// must hold the version.
    Read {
        /// The node directory.
        dir: PathBuf,
        /// The address, as printed by `walk write`.
        address: String,
    },
    /// Show a node's heads and conflict reports for the demo
    /// graph.
    Reports {
        /// The node directory.
        dir: PathBuf,
    },
    /// Serve a node: answer federation exchanges from its
    /// directory until interrupted, persisting what sessions
    /// admit. Prints the peer token a `walk sync` dials.
    Serve {
        /// The node directory.
        dir: PathBuf,
    },
    /// Run one exchange from a node to a serving peer, and
    /// persist what the node admits.
    Sync {
        /// The node directory.
        dir: PathBuf,
        /// The peer token printed by the peer's `walk serve`
        /// (`<endpoint-id>@<socket-address>`).
        #[arg(long)]
        peer: String,
    },
}
