//! `id doc` subcommand definitions: publish, events, frontier, history.

use std::path::PathBuf;

use clap::Subcommand;

/// Subcommands for `id doc`. A document is a `document.put` event over the content's hash.
#[derive(Subcommand, Debug)]
pub enum DocCommand {
    /// Publish a file as a document, and set a tag for each `--tag`.
    ///
    /// Prints `publish document=<ID>`, the document's event ID. The file's bytes are hashed, and
    /// the event names their hash and size. The bytes are not stored in the event.
    Publish {
        /// The file to publish.
        file: PathBuf,
        /// The media type of the file, for example `text/markdown`.
        #[arg(long, value_name = "TYPE")]
        media_type: String,
        /// A tag to set on the document, as `name` or `name=value`. Repeat for more.
        #[arg(long = "tag", value_name = "NAME[=VALUE]")]
        tags: Vec<String>,
    },
    /// List the tags and removal on a document, in causal order.
    Events {
        /// The document's event ID, as `publish` printed it.
        document: String,
    },
    /// List the head of each author's chain: the latest event each author has recorded.
    Frontier,
    /// List a document's events in causal order, its `document.put` first.
    History {
        /// The document's event ID, as `publish` printed it.
        document: String,
    },
}
