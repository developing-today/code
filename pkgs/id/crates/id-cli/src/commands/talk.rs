//! `id talk`: post on and list an object's talk page (#5834 slice 2, #14270).
//!
//! This module parses the arguments, acts on the event store, and prints. The rules are the
//! library's ([`id_events::local_events::TalkEntry`], [`id_store::talk_page`]): visibility is
//! the only gate on posting, and a correction is a new post that supersedes the old one.
//!
//! # Output
//!
//! `post` prints `post object=<ID> post=<POST> author=<KEY> event=<ID> ...`. `list` prints one
//! line per post with the same fields and no command word, or one JSON array. A post another
//! post supersedes carries `superseded_by=<POST>`; it is listed, never dropped. Post IDs are
//! the record's content IDs in text form, which `--reply-to` and `--supersedes` take back.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::cli::TalkCommand;
use crate::commands::doc::{open_events, print_action, print_rows};
use crate::commands::group::hex;
use id_core::id_shape::{ContentId, Id};
use id_events::local_events::TalkEntry;
use id_groups::groups::log::hash_from_hex;

/// Parses a post ID as `--reply-to` and `--supersedes` take it.
fn post_id(text: &str) -> Result<ContentId> {
    match Id::parse_text(text).map_err(|err| anyhow!("{text:?} is not a post ID: {err}"))? {
        Id::Content(id) => Ok(id),
        _ => Err(anyhow!(
            "{text:?} is not a post ID: a post ID is a content ID"
        )),
    }
}

/// A post ID in the text form `--reply-to` and `--supersedes` take.
fn post_text(id: ContentId) -> String {
    Id::Content(id).to_text()
}

/// The fields of one post, as `post` and `list` print it. The body comes last.
fn entry_fields(entry: &TalkEntry) -> Vec<(&'static str, Value)> {
    let post = &entry.post;
    let mut fields = vec![
        ("object", json!(hex_of(post.object_id()))),
        ("post", json!(post_text(entry.post_id))),
        ("author", json!(hex(post.author()))),
        ("authored_at", json!(post.authored_at())),
    ];
    if let Some(parent) = post.parent_post_id() {
        fields.push(("reply_to", json!(post_text(parent))));
    }
    if let Some(earlier) = post.supersedes() {
        fields.push(("supersedes", json!(post_text(earlier))));
    }
    if let Some(later) = entry.superseded_by {
        fields.push(("superseded_by", json!(post_text(later))));
    }
    fields.push(("event", json!(entry.event.to_hex())));
    fields.push(("body", json!(post.body().unwrap_or_default())));
    fields
}

/// Lower-case hex of an object ID's bytes.
fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Runs an `id talk` command.
///
/// # Errors
///
/// Fails if an argument is malformed, this node cannot see the object, the store cannot be read
/// or written, or the library refuses the post.
pub async fn cmd_talk(path: Option<PathBuf>, json: bool, command: TalkCommand) -> Result<()> {
    let mut events = open_events(path).await?;
    match command {
        TalkCommand::Post {
            object,
            body,
            reply_to,
            supersedes,
        } => {
            let object = hash_from_hex(&object)?;
            let reply_to = reply_to.as_deref().map(post_id).transpose()?;
            let supersedes = supersedes.as_deref().map(post_id).transpose()?;
            let entry = events.post_talk(object, reply_to, supersedes, &body)?;
            print_action(json, "post", entry_fields(&entry));
        }
        TalkCommand::List { object } => {
            let object = hash_from_hex(&object)?;
            let rows: Vec<Vec<(&str, Value)>> =
                events.talk_page(object)?.iter().map(entry_fields).collect();
            print_rows(json, &rows);
        }
    }
    Ok(())
}
