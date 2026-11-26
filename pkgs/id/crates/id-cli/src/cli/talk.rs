//! `id talk` subcommand definitions: post on and list an object's talk page, and the
//! thread steps of a conversation group: propose, resolve, reject, withdraw, accept, and
//! read the threads back.

use std::path::PathBuf;

use clap::Subcommand;

/// Subcommands for `id talk`. A talk page is the discussion attached to one object.
///
/// The thread steps (#7849) act in a conversation group, which only its members write to.
/// Each step is one terminal line (`propose`, `resolve`, `reject`, `withdraw`, `accept`)
/// that `threads` reads back. An object is `<object-id>/<author-key>`, a blob is
/// `<hash>/<author-key>[@<hint>]`, and every other ID is 64 hex characters.
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
    /// Propose a change to an object, out of a thread in a conversation group.
    ///
    /// The proposal is pinned to the exact revision it is against, and carries the full
    /// proposed content by blob link. An object writer accepts it with `accept`.
    Propose {
        /// The conversation group's ID.
        group: String,
        /// The object to change, as `<object-id>/<author-key>`.
        object: String,
        /// The exact revision of the object the proposal is against.
        #[arg(long, value_name = "REVISION")]
        base: String,
        /// The full proposed content, as a blob link `<hash>/<author-key>[@<hint>]`.
        #[arg(long, value_name = "BLOB")]
        content: String,
        /// The root event ID of the thread the proposal came out of.
        #[arg(long)]
        thread: String,
        /// This node's earlier proposal against the same object and base that this one replaces.
        #[arg(long, value_name = "PROPOSAL")]
        supersedes: Option<String>,
    },
    /// Resolve a thread: into an edit on an object, a decision, or no action.
    ///
    /// Give exactly one of `--edit` with `--object`, `--decision`, or `--no-action`.
    /// Concurrent resolutions of one thread are all kept, and `threads` shows them side by side.
    Resolve {
        /// The conversation group's ID.
        group: String,
        /// The root event ID of the thread.
        thread: String,
        /// The edit (the resulting revision) the thread produced on `--object`.
        #[arg(
            long,
            value_name = "EDIT",
            requires = "object",
            conflicts_with_all = ["decision", "no_action"]
        )]
        edit: Option<String>,
        /// The object the edit is on, as `<object-id>/<author-key>`.
        #[arg(long, requires = "edit")]
        object: Option<String>,
        /// The decision text, as a blob link `<hash>/<author-key>[@<hint>]`.
        #[arg(long, value_name = "BLOB", conflicts_with = "no_action")]
        decision: Option<String>,
        /// The thread was considered and nothing changes.
        #[arg(long)]
        no_action: bool,
    },
    /// Reject a proposal, as a writer of its object.
    Reject {
        /// The conversation group's ID.
        group: String,
        /// The proposal's event ID.
        proposal: String,
        /// Why, as a blob link `<hash>/<author-key>[@<hint>]`.
        #[arg(long, value_name = "BLOB")]
        because: String,
    },
    /// Withdraw a proposal this node made.
    Withdraw {
        /// The conversation group's ID.
        group: String,
        /// The proposal's event ID.
        proposal: String,
    },
    /// Accept a proposal: revise its object, a note, with an edit that cites the proposal.
    ///
    /// The revision is the acceptance; there is no separate approval. It counts when this
    /// node may write the object.
    Accept {
        /// The conversation group's ID.
        group: String,
        /// The proposal's event ID.
        proposal: String,
        /// The file holding the revision's content, normally the proposal's content.
        file: PathBuf,
        /// The media type of the file, for example `text/markdown`.
        #[arg(long, value_name = "TYPE")]
        media_type: String,
        /// A revision the edit builds on. Repeat for a merge. Defaults to the proposal's base.
        #[arg(long = "parent", value_name = "REVISION")]
        parents: Vec<String>,
    },
    /// Show a group's threads as this node may see them: each thread's resolution and each
    /// proposal's state, concurrent resolutions side by side and stale proposals marked.
    Threads {
        /// The conversation group's ID.
        group: String,
    },
}
