//! `id writer-sets` subcommand definitions: show, expand, and discover
//! writer sets stored in the event store.

use clap::Subcommand;

/// The actions of `id writer-sets` (writer-set spec §§4–5; issue #6690).
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum WriterSetCommand {
    /// Show one stored writer set: its ID, version, owner, scope, and the
    /// counts of its admission entries and exclusions.
    Show {
        /// The publishing event's ID.
        event: String,
    },
    /// Expand one stored writer set at its publishing frontier: whether the
    /// expansion is public, and the explicit authors it admits after
    /// exclusions.
    Expand {
        /// The publishing event's ID.
        event: String,
        /// The resolution snapshot to pin the projection under, as hex.
        /// Without one, the zero hash names the store's current resolved
        /// state.
        #[arg(long)]
        resolution: Option<String>,
        /// Expand retroactively: only entries marked include-history
        /// contribute (exclusions always apply).
        #[arg(long)]
        retro: bool,
    },
    /// Discover stored writer sets by scope: one row per set, with the
    /// writers it expands to. Without `--scope`, every stored set matches.
    Discover {
        /// The scope (subject) to match, exactly.
        #[arg(long)]
        scope: Option<String>,
        /// The resolution snapshot to pin each expansion under, as hex.
        /// Without one, the zero hash names the store's current resolved
        /// state.
        #[arg(long)]
        resolution: Option<String>,
    },
}
