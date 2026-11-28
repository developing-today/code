//! `id claim` subcommand definitions: set, clear, search, scope,
//! review, and list tag claims.

use clap::{Subcommand, ValueEnum};

/// Subcommands for `id claim`. A claim is the event that sets a tag on a subject.
#[derive(Subcommand, Debug)]
pub enum ClaimCommand {
    /// Set a tag on a subject, which is any event ID, such as a document's.
    ///
    /// An explicit `--scope` wins over a `local:` name prefix: the prefix is
    /// shorthand for `--scope local` only when `--scope` is absent, so with
    /// `--scope shared` a name written `local:NAME` is a shared claim whose
    /// name keeps the prefix.
    Set {
        /// The subject's event ID.
        subject: String,
        /// The tag name. A `local:` prefix is shorthand for `--scope local`
        /// when `--scope` is absent.
        name: String,
        /// The tag value. Without one, the tag is bare.
        value: Option<String>,
        /// Whether the claim is shared with peers or stays on this node:
        /// `shared` publishes it to peers, `local` keeps it on this node
        /// only. An explicit `--scope` wins over a `local:` name prefix.
        #[arg(long, value_enum)]
        scope: Option<ClaimScope>,
    },
    /// Clear the live claims with this name on a subject, whoever set them.
    ///
    /// Clearing stops future replication; it cannot unsend copies peers
    /// already hold.
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
    ///
    /// The target scope here is explicit, so it wins over any `local:`
    /// prefix shorthand used when a claim was set. Moving to `local` stops
    /// future replication but cannot unsend copies peers already hold.
    /// Moving to `shared` publishes this node's local claims and cannot be
    /// undone, so it requires `--yes` when a local claim would be published.
    Scope {
        /// The subject's event ID.
        subject: String,
        /// The tag name.
        name: String,
        /// The target scope.
        #[arg(value_enum)]
        scope: ClaimScope,
        /// Confirm publishing local claims as shared. Required when the
        /// move would publish a local claim: publishing cannot be undone.
        #[arg(long)]
        yes: bool,
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
