//! `id claim` subcommand definitions: set, clear, search, scope,
//! review, and list tag claims.

use clap::{Subcommand, ValueEnum};

/// Subcommands for `id claim`. A claim is the event that sets a tag on a subject.
#[derive(Subcommand, Debug)]
pub enum ClaimCommand {
    /// Set a tag on a subject, which is any event ID, such as a document's.
    Set {
        /// The subject's event ID.
        subject: String,
        /// The tag name.
        name: String,
        /// The tag value. Without one, the tag is bare.
        value: Option<String>,
        /// Whether the claim is shared with peers or stays on this node.
        #[arg(long, value_enum, default_value = "shared")]
        scope: ClaimScope,
    },
    /// Clear the live claims with this name on a subject, whoever set them.
    Clear {
        /// The subject's event ID.
        subject: String,
        /// The tag name.
        name: String,
    },
    /// List the live claims with this name, and value if given, on any subject.
    Search {
        /// The tag name.
        name: String,
        /// The tag value to match as a whole. Without one, any value and a bare tag match.
        value: Option<String>,
    },
    /// Change the scope of this node's claims with this name on a subject.
    Scope {
        /// The subject's event ID.
        subject: String,
        /// The tag name.
        name: String,
        /// The target scope.
        #[arg(value_enum)]
        scope: ClaimScope,
    },
    /// Review every live claim visible to this node, with its scope.
    Review,
    /// List the live claims on a subject.
    List {
        /// The subject's event ID.
        subject: String,
    },
}

/// Where a tag claim lives: shared with peers, or on this node only (#1793).
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClaimScope {
    /// Published to the shared claim set and replicated, the v1 default.
    #[default]
    Shared,
    /// Kept on this node only; never published or synced.
    Local,
}
