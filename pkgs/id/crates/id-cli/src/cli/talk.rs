//! `id talk` subcommand definitions: post on and list an object's talk page.

use clap::Subcommand;

/// Subcommands for `id talk`. A talk page is the discussion attached to one object.
#[derive(Subcommand, Debug)]
pub enum TalkCommand {
    /// Post on an object's talk page, signed by this node.
    ///
    /// A post is never edited: to correct one, post again with `--supersedes`
    /// naming it. The page then marks the earlier post superseded and keeps it.
    Post {
        /// The object's event ID, such as a document's or a note's.
        object: String,
        /// The post text.
        body: String,
        /// The ID of the post this one replies to, on the same page.
        #[arg(long)]
        reply_to: Option<String>,
        /// The ID of this node's earlier post on the same page that this one corrects.
        #[arg(long)]
        supersedes: Option<String>,
    },
    /// List every post on an object's talk page, oldest first.
    List {
        /// The object's event ID.
        object: String,
    },
}
