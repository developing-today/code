//! `id talk`: post on and list an object's talk page (#5834 slice 2, #14270), and take the
//! thread steps of a conversation group (#7849).
//!
//! This module parses the arguments, acts on the event store, and prints. The rules are the
//! library's ([`id_events::local_events::TalkEntry`], [`id_store::talk_page`],
//! [`id_store::thread_resolution`]): visibility is the only gate on posting, and a correction
//! is a new post that supersedes the old one.
//!
//! A thread step is one terminal line ([`id_store::thread_text`]). Each subcommand writes its
//! arguments as that line and parses it with [`parse_command`], so the event it signs passes
//! the fold's own field checks, and only a member of the group can write one. `accept` is the
//! note revision that cites the proposal. `threads` prints this node's view of the group
//! ([`id_events::local_events::LocalEvents::thread_view`]) with [`render_projection`]: a
//! proposal still open against a revision that is no longer a head of its note is marked stale.
//!
//! # Output
//!
//! `post` prints `post object=<ID> post=<POST> author=<KEY> event=<ID> ...`. `list` prints one
//! line per post with the same fields and no command word, or one JSON array. A post another
//! post supersedes carries `superseded_by=<POST>`; it is listed, never dropped. Post IDs are
//! the record's content IDs in text form, which `--reply-to` and `--supersedes` take back.
//!
//! `propose`, `resolve`, `reject` and `withdraw` print `<step> group=<ID> kind=<KIND>
//! event=<ID> line=<LINE>`, where the line is the step's terminal line. `accept` prints
//! `accept group=<ID> proposal=<ID> note=<ID> revision=<ID> matches_proposal=<BOOL>
//! line=<LINE>`, where `matches_proposal` says whether the revision's bytes are the
//! proposal's content. `threads` prints the rendered lines, or one JSON array of them.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use p2panda_core::Hash;
use serde_json::{Value, json};

use crate::cli::TalkCommand;
use crate::commands::doc::{open_events, print_action, print_rows};
use crate::commands::group::hex;
use id_core::id_shape::{ContentId, Id};
use id_events::local_events::{LocalEvents, TalkEntry};
use id_groups::groups::log::hash_from_hex;
use id_store::chat_anchor::ObjectRef;
use id_store::thread_text::{ThreadCommand, format_command, parse_command, render_projection};

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

/// One argument as a word of a thread line. A word holds no whitespace, so an argument can
/// never add words of its own to the line.
fn word<'a>(text: &'a str, what: &str) -> Result<&'a str> {
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        bail!("the {what} must be one word with no whitespace, not {text:?}");
    }
    Ok(text)
}

/// Parses a thread line, signs its event as this node in `group`, and prints the step.
///
/// # Errors
///
/// Fails if the line breaks the grammar or a field rule, names no thread event, or the
/// library refuses the write (this node is not a member of the group, for instance).
fn send_step(
    events: &mut LocalEvents,
    json: bool,
    step: &str,
    group: &str,
    line: &str,
) -> Result<()> {
    let group = hash_from_hex(group)?;
    let command = parse_command(line)?;
    let line = format_command(&command)?;
    let ThreadCommand::Event(event) = command else {
        bail!("{step} names no thread event");
    };
    let kind = event.kind();
    let id = events.send_thread_event(group, *event)?;
    print_action(
        json,
        step,
        vec![
            ("group", json!(group.to_hex())),
            ("kind", json!(kind)),
            ("event", json!(id.to_hex())),
            ("line", json!(line)),
        ],
    );
    Ok(())
}

/// The thread line of `propose`.
fn propose_line(
    object: &str,
    base: &str,
    content: &str,
    thread: &str,
    supersedes: Option<&str>,
) -> Result<String> {
    let mut line = format!(
        "propose {} base {} content {} thread {}",
        word(object, "object")?,
        word(base, "base revision")?,
        word(content, "content")?,
        word(thread, "thread")?,
    );
    if let Some(supersedes) = supersedes {
        line.push_str(" supersedes ");
        line.push_str(word(supersedes, "superseded proposal")?);
    }
    Ok(line)
}

/// The thread line of `resolve`: one outcome of the three.
fn resolve_line(
    thread: &str,
    edit: Option<&str>,
    object: Option<&str>,
    decision: Option<&str>,
    no_action: bool,
) -> Result<String> {
    let thread = word(thread, "thread")?;
    let outcome = match (edit, object, decision, no_action) {
        (Some(edit), Some(object), None, false) => {
            format!("edit {} {}", word(object, "object")?, word(edit, "edit")?)
        }
        (None, None, Some(summary), false) => format!("decision {}", word(summary, "decision")?),
        (None, None, None, true) => "no-action".to_owned(),
        _ => bail!(
            "resolve takes exactly one of `--edit <EDIT> --object <OBJECT>`, `--decision <BLOB>` or `--no-action`"
        ),
    };
    Ok(format!("resolve {thread} {outcome}"))
}

/// Accepts `proposal` in `group`: revises its note with `content`, citing it, on `parents`
/// or, with none, on the proposal's base. Prints the step.
///
/// # Errors
///
/// Fails if this node sees no such proposal in the group, a parent is malformed or not a
/// revision of the note, or the library refuses the revision.
fn accept_step(
    events: &mut LocalEvents,
    json: bool,
    group: &str,
    proposal: &str,
    content: &[u8],
    media_type: &str,
    parents: &[String],
) -> Result<()> {
    let group = hash_from_hex(group)?;
    let command = parse_command(&format!("accept {}", word(proposal, "proposal")?))?;
    let line = format_command(&command)?;
    let ThreadCommand::Accept { proposal } = command else {
        bail!("accept names no proposal");
    };
    let view = events.thread_view(group)?;
    let record = view
        .proposals
        .iter()
        .find(|record| record.id == proposal)
        .ok_or_else(|| {
            anyhow!(
                "no proposal {} in the group's threads that this node can see",
                proposal.to_hex()
            )
        })?;
    let note = record.proposal.object.object_id;
    let matches_proposal = record.proposal.content.verify_blob(content).is_ok();
    let parents = if parents.is_empty() {
        vec![record.proposal.base]
    } else {
        parents
            .iter()
            .map(|parent| hash_from_hex(parent))
            .collect::<Result<Vec<Hash>>>()?
    };
    let revision = events.accept_proposal(note, media_type, content, &parents, Some(proposal))?;
    print_action(
        json,
        "accept",
        vec![
            ("group", json!(group.to_hex())),
            ("proposal", json!(proposal.to_hex())),
            ("note", json!(note.to_hex())),
            ("revision", json!(revision.to_hex())),
            ("matches_proposal", json!(matches_proposal)),
            ("line", json!(line)),
        ],
    );
    Ok(())
}

/// Prints this node's view of `group`'s threads: the rendered lines, or one JSON array of
/// them. A proposal on a note is marked stale against the note's current heads.
///
/// # Errors
///
/// Fails if this node is not a member of the group, or the store cannot be read.
fn threads_step(events: &LocalEvents, json: bool, group: &str) -> Result<()> {
    let group = hash_from_hex(group)?;
    let view = events.thread_view(group)?;
    let heads: BTreeMap<ObjectRef, Vec<Hash>> = events
        .notes()?
        .into_iter()
        .filter_map(|note| {
            let object = note.object()?;
            Some((object, note.heads))
        })
        .collect();
    let object_heads = |object: &ObjectRef| heads.get(object).cloned();
    let text = render_projection(&view, &object_heads);
    if json {
        let lines: Vec<Value> = text.lines().map(|line| json!(line)).collect();
        println!("{}", Value::Array(lines));
    } else {
        print!("{text}");
    }
    Ok(())
}

/// Runs an `id talk` command.
///
/// # Errors
///
/// Fails if an argument is malformed, this node cannot see the object or is not a member of the
/// group, the store cannot be read or written, or the library refuses the post or the step.
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
        TalkCommand::Propose {
            group,
            object,
            base,
            content,
            thread,
            supersedes,
        } => {
            let line = propose_line(&object, &base, &content, &thread, supersedes.as_deref())?;
            send_step(&mut events, json, "propose", &group, &line)?;
        }
        TalkCommand::Resolve {
            group,
            thread,
            edit,
            object,
            decision,
            no_action,
        } => {
            let line = resolve_line(
                &thread,
                edit.as_deref(),
                object.as_deref(),
                decision.as_deref(),
                no_action,
            )?;
            send_step(&mut events, json, "resolve", &group, &line)?;
        }
        TalkCommand::Reject {
            group,
            proposal,
            because,
        } => {
            let line = format!(
                "reject {} because {}",
                word(&proposal, "proposal")?,
                word(&because, "reason")?
            );
            send_step(&mut events, json, "reject", &group, &line)?;
        }
        TalkCommand::Withdraw { group, proposal } => {
            let line = format!("withdraw {}", word(&proposal, "proposal")?);
            send_step(&mut events, json, "withdraw", &group, &line)?;
        }
        TalkCommand::Accept {
            group,
            proposal,
            file,
            media_type,
            parents,
        } => {
            let content =
                std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
            accept_step(
                &mut events,
                json,
                &group,
                &proposal,
                &content,
                &media_type,
                &parents,
            )?;
        }
        TalkCommand::Threads { group } => threads_step(&events, json, &group)?,
    }
    Ok(())
}
