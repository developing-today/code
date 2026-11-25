//! Terminal parity for thread resolution and change proposals (#7849 item 3,
//! terminal half; decision record `doc/2026-11-08_decision_id5874_thread_resolution.md`
//! §5 invariant 5).
//!
//! Every step of the loop is one line of text a terminal client can type and
//! read back, and no step needs a GUI:
//!
//! ```text
//! propose <object> base <revision> content <blob> thread <thread> [supersedes <proposal>]
//! resolve <thread> edit <object> <edit>
//! resolve <thread> decision <blob>
//! resolve <thread> no-action
//! reject <proposal> because <blob>
//! withdraw <proposal>
//! accept <proposal>
//! ```
//!
//! An event ID or revision is 64 hex characters. An object is
//! `<object-id>/<author-key>`. A blob is `<hash>/<author-key>`, optionally
//! followed by `@<hint>` (a hint with no whitespace).
//!
//! [`parse_command`] turns a line into a [`ThreadCommand`] whose events pass
//! the same field checks the fold applies, and [`format_command`] writes the
//! line back, so `parse_command(&format_command(c)?)` returns `c`.
//!
//! `accept` is not a thread event. The record makes acceptance the edit itself,
//! signed by an object writer and citing the proposal (§3), so
//! [`ThreadCommand::Accept`] names the proposal for the object's own editor to
//! cite; that editor plumbing is #7849 item 2.
//!
//! [`render_projection`] reads a [`ThreadProjection`] back as text: each
//! thread's resolution (concurrent heads side by side, none chosen), then each
//! proposal with its derived state and what closed it, and a stale marker when
//! the caller knows the object's heads. Render the viewer's projection
//! ([`ThreadProjection::visible_to`]), never the full one, so the text keeps
//! the no-leak rule.

use std::fmt::Write as _;

use p2panda_core::Hash;

use crate::chat::{ChatLink, ChatLinkKind};
use crate::chat_anchor::ObjectRef;
use crate::thread_resolution::{
    ChatProposal, ChatProposalRejection, ChatProposalWithdrawal, ChatResolution, ProposalRecord,
    ProposalState, ResolutionKind, ResolutionRecord, ThreadEvent, ThreadEventError,
    ThreadProjection,
};
use id_core::waiting::AuthorKey;

/// One line a terminal client types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadCommand {
    /// A thread event to sign and publish.
    Event(ThreadEvent),
    /// Accept a proposal: the object's editor makes the edit citing it.
    Accept {
        /// The proposal the edit cites.
        proposal: Hash,
    },
}

/// Why a line is not a command, or a command has no text form.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ThreadTextError {
    /// The line does not follow the grammar.
    #[error("{0}")]
    Syntax(String),
    /// The line parses but the event breaks a field rule.
    #[error(transparent)]
    Event(#[from] ThreadEventError),
}

fn syntax(message: impl Into<String>) -> ThreadTextError {
    ThreadTextError::Syntax(message.into())
}

/// The grammar, one command per line, for help text.
pub const USAGE: &str = "\
propose <object> base <revision> content <blob> thread <thread> [supersedes <proposal>]
resolve <thread> edit <object> <edit>
resolve <thread> decision <blob>
resolve <thread> no-action
reject <proposal> because <blob>
withdraw <proposal>
accept <proposal>";

/// Parses one command line.
///
/// # Errors
///
/// Returns [`ThreadTextError::Syntax`] when the line does not follow
/// [`USAGE`], and [`ThreadTextError::Event`] when the event it names breaks a
/// field rule.
pub fn parse_command(line: &str) -> Result<ThreadCommand, ThreadTextError> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let command = match tokens.as_slice() {
        ["resolve", thread, "edit", object, edit] => {
            ThreadCommand::Event(ThreadEvent::Resolution(ChatResolution::edit(
                parse_hash(thread, "thread")?,
                parse_object(object)?,
                parse_hash(edit, "edit")?,
            )))
        }
        ["resolve", thread, "decision", summary] => ThreadCommand::Event(ThreadEvent::Resolution(
            ChatResolution::decision(parse_hash(thread, "thread")?, parse_blob(summary)?)?,
        )),
        ["resolve", thread, "no-action"] => ThreadCommand::Event(ThreadEvent::Resolution(
            ChatResolution::no_action(parse_hash(thread, "thread")?),
        )),
        ["resolve", ..] => {
            return Err(syntax(
                "resolve takes a thread, then `edit <object> <edit>`, `decision <blob>` or `no-action`",
            ));
        }
        [
            "propose",
            object,
            "base",
            base,
            "content",
            content,
            "thread",
            thread,
            rest @ ..,
        ] => {
            let supersedes = match rest {
                [] => None,
                ["supersedes", proposal] => Some(parse_hash(proposal, "superseded proposal")?),
                _ => {
                    return Err(syntax(
                        "propose ends with `supersedes <proposal>` or nothing",
                    ));
                }
            };
            ThreadCommand::Event(ThreadEvent::Proposal(ChatProposal {
                object: parse_object(object)?,
                base: parse_hash(base, "base revision")?,
                content: parse_blob(content)?,
                thread: parse_hash(thread, "thread")?,
                supersedes,
            }))
        }
        ["propose", ..] => {
            return Err(syntax(
                "propose takes `<object> base <revision> content <blob> thread <thread>`",
            ));
        }
        ["reject", proposal, "because", reason] => {
            ThreadCommand::Event(ThreadEvent::Rejection(ChatProposalRejection {
                proposal: parse_hash(proposal, "proposal")?,
                reason: parse_blob(reason)?,
            }))
        }
        ["reject", ..] => return Err(syntax("reject takes `<proposal> because <blob>`")),
        ["withdraw", proposal] => {
            ThreadCommand::Event(ThreadEvent::Withdrawal(ChatProposalWithdrawal {
                proposal: parse_hash(proposal, "proposal")?,
            }))
        }
        ["withdraw", ..] => return Err(syntax("withdraw takes one proposal")),
        ["accept", proposal] => ThreadCommand::Accept {
            proposal: parse_hash(proposal, "proposal")?,
        },
        ["accept", ..] => return Err(syntax("accept takes one proposal")),
        [] => return Err(syntax("the line is empty")),
        [word, ..] => {
            return Err(syntax(format!(
                "{word:?} is not a command; the commands are propose, resolve, reject, withdraw and accept"
            )));
        }
    };
    if let ThreadCommand::Event(event) = &command {
        event.check()?;
    }
    Ok(command)
}

/// Writes a command as the line [`parse_command`] reads.
///
/// # Errors
///
/// Returns [`ThreadTextError::Event`] when the event breaks a field rule, and
/// [`ThreadTextError::Syntax`] when a blob's hint holds whitespace, which a
/// line has no place for.
pub fn format_command(command: &ThreadCommand) -> Result<String, ThreadTextError> {
    let event = match command {
        ThreadCommand::Accept { proposal } => return Ok(format!("accept {}", hex_of(proposal))),
        ThreadCommand::Event(event) => event,
    };
    event.check()?;
    Ok(match event {
        ThreadEvent::Resolution(resolution) => {
            let thread = hex_of(&resolution.thread);
            match (
                resolution.kind,
                &resolution.object,
                &resolution.edit,
                &resolution.summary,
            ) {
                (ResolutionKind::Edit, Some(object), Some(edit), None) => {
                    format!(
                        "resolve {thread} edit {} {}",
                        object_text(object),
                        hex_of(edit)
                    )
                }
                (ResolutionKind::Decision, None, None, Some(summary)) => {
                    format!("resolve {thread} decision {}", blob_text(summary)?)
                }
                _ => format!("resolve {thread} no-action"),
            }
        }
        ThreadEvent::Proposal(proposal) => {
            let mut line = format!(
                "propose {} base {} content {} thread {}",
                object_text(&proposal.object),
                hex_of(&proposal.base),
                blob_text(&proposal.content)?,
                hex_of(&proposal.thread),
            );
            if let Some(supersedes) = &proposal.supersedes {
                let _ = write!(line, " supersedes {}", hex_of(supersedes));
            }
            line
        }
        ThreadEvent::Rejection(rejection) => format!(
            "reject {} because {}",
            hex_of(&rejection.proposal),
            blob_text(&rejection.reason)?
        ),
        ThreadEvent::Withdrawal(withdrawal) => format!("withdraw {}", hex_of(&withdrawal.proposal)),
    })
}

/// Reads a projection back as text, one line per resolution head and per
/// proposal. `object_heads` answers an object's current heads when the caller
/// knows them; a proposal still open against a base that is no longer a head
/// is marked stale (an observation, not a state: a writer may still accept it).
pub fn render_projection(
    projection: &ThreadProjection,
    object_heads: &dyn Fn(&ObjectRef) -> Option<Vec<Hash>>,
) -> String {
    let mut out = String::new();
    for entry in &projection.resolutions {
        let thread = hex_of(&entry.thread);
        if let [head] = entry.heads.as_slice() {
            let _ = writeln!(out, "thread {thread} resolved: {}", resolution_text(head));
        } else {
            let _ = writeln!(
                out,
                "thread {thread} has {} concurrent resolutions, none chosen:",
                entry.heads.len()
            );
            for head in &entry.heads {
                let _ = writeln!(out, "  {}", resolution_text(head));
            }
        }
    }
    for record in &projection.proposals {
        let _ = writeln!(out, "{}", proposal_text(record, object_heads));
    }
    if !projection.refused.is_empty() {
        let refused: Vec<String> = projection.refused.iter().map(hex_of).collect();
        let _ = writeln!(out, "refused events: {}", refused.join(" "));
    }
    out
}

fn resolution_text(record: &ResolutionRecord) -> String {
    let resolution = &record.resolution;
    let what = match (
        resolution.kind,
        &resolution.object,
        &resolution.edit,
        &resolution.summary,
    ) {
        (ResolutionKind::Edit, Some(object), Some(edit), _) => {
            format!("edit {} on object {}", hex_of(edit), object_text(object))
        }
        (ResolutionKind::Decision, _, _, Some(summary)) => {
            format!("decision, summary {}", blob_text_lossy(summary))
        }
        _ => "no action".to_owned(),
    };
    format!(
        "{what}, by {} (event {})",
        hex::encode(record.author),
        hex_of(&record.id)
    )
}

fn proposal_text(
    record: &ProposalRecord,
    object_heads: &dyn Fn(&ObjectRef) -> Option<Vec<Hash>>,
) -> String {
    let proposal = &record.proposal;
    let mut line = format!(
        "proposal {} on object {} at base {} from thread {}, by {}: ",
        hex_of(&record.id),
        object_text(&proposal.object),
        hex_of(&proposal.base),
        hex_of(&proposal.thread),
        hex::encode(record.author),
    );
    let ids = |ids: &[Hash]| ids.iter().map(hex_of).collect::<Vec<_>>().join(" ");
    match record.state {
        ProposalState::Proposed => line.push_str("open"),
        ProposalState::Accepted => {
            let _ = write!(line, "accepted by edit {}", ids(&record.accepted_by));
        }
        ProposalState::Rejected => {
            let rejections: Vec<String> = record
                .rejected_by
                .iter()
                .map(|(id, reason)| format!("{} (reason {})", hex_of(id), blob_text_lossy(reason)))
                .collect();
            let _ = write!(line, "rejected by {}", rejections.join(", "));
        }
        ProposalState::Withdrawn => {
            let _ = write!(line, "withdrawn by {}", ids(&record.withdrawn_by));
        }
        ProposalState::Superseded => {
            let _ = write!(line, "superseded by {}", ids(&record.superseded_by));
        }
    }
    if record.state == ProposalState::Proposed
        && object_heads(&proposal.object).is_some_and(|heads| record.is_stale(&heads))
    {
        line.push_str(" (stale: its base is no longer a head of the object)");
    }
    line
}

fn hex_of(hash: &Hash) -> String {
    hex::encode(hash.as_bytes())
}

fn object_text(object: &ObjectRef) -> String {
    format!(
        "{}/{}",
        hex_of(&object.object_id),
        hex::encode(object.author)
    )
}

fn blob_text(link: &ChatLink) -> Result<String, ThreadTextError> {
    if link
        .hint
        .as_deref()
        .is_some_and(|hint| hint.is_empty() || hint.chars().any(char::is_whitespace))
    {
        return Err(syntax(
            "a blob hint that is empty or holds whitespace has no place on a command line",
        ));
    }
    Ok(blob_text_lossy(link))
}

fn blob_text_lossy(link: &ChatLink) -> String {
    let mut text = format!("{}/{}", hex_of(&link.hash), hex::encode(link.author));
    if let Some(hint) = &link.hint {
        let _ = write!(text, "@{hint}");
    }
    text
}

fn parse_bytes(token: &str, what: &str) -> Result<[u8; 32], ThreadTextError> {
    hex::decode(token)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| {
            syntax(format!(
                "the {what} must be 64 hex characters, not {token:?}"
            ))
        })
}

fn parse_hash(token: &str, what: &str) -> Result<Hash, ThreadTextError> {
    parse_bytes(token, what).map(Hash::from_bytes)
}

fn parse_pair(token: &str, what: &str) -> Result<(Hash, AuthorKey), ThreadTextError> {
    let (id, author) = token.split_once('/').ok_or_else(|| {
        syntax(format!(
            "the {what} must be `<id>/<author-key>`, not {token:?}"
        ))
    })?;
    Ok((parse_hash(id, what)?, parse_bytes(author, "author key")?))
}

fn parse_object(token: &str) -> Result<ObjectRef, ThreadTextError> {
    let (id, author) = parse_pair(token, "object")?;
    Ok(ObjectRef::new(id, author))
}

fn parse_blob(token: &str) -> Result<ChatLink, ThreadTextError> {
    let (pair, hint) = match token.split_once('@') {
        Some((pair, hint)) if !hint.is_empty() => (pair, Some(hint.to_owned())),
        Some(_) => return Err(syntax("a blob hint after `@` must not be empty")),
        None => (token, None),
    };
    let (hash, author) = parse_pair(pair, "blob")?;
    Ok(ChatLink {
        kind: ChatLinkKind::Blob,
        hash,
        author,
        hint,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::thread_resolution::{
        AcceptingEdit, ThreadAuthority, ThreadEventRecord, project_threads,
    };

    fn hash(seed: u8) -> Hash {
        Hash::digest([seed])
    }

    fn author(seed: u8) -> AuthorKey {
        [seed; 32]
    }

    fn object(seed: u8) -> ObjectRef {
        ObjectRef::new(hash(100 + seed), author(100 + seed))
    }

    fn blob(seed: u8, hint: Option<&str>) -> ChatLink {
        ChatLink {
            kind: ChatLinkKind::Blob,
            hash: hash(200 + seed),
            author: author(1),
            hint: hint.map(str::to_owned),
        }
    }

    struct Open;

    impl ThreadAuthority for Open {
        fn may_resolve(&self, _thread: &Hash, _author: &AuthorKey) -> bool {
            true
        }
        fn may_write(&self, _object: &ObjectRef, _author: &AuthorKey) -> bool {
            true
        }
    }

    fn record(id: u8, by: u8, event: ThreadEvent) -> ThreadEventRecord {
        ThreadEventRecord {
            id: hash(id),
            author: author(by),
            after: Vec::new(),
            backlink: None,
            event,
        }
    }

    fn round_trip(command: &ThreadCommand) {
        let line = format_command(command).unwrap();
        assert_eq!(parse_command(&line).unwrap(), *command, "line: {line}");
    }

    #[test]
    fn every_step_of_the_loop_is_one_line_that_reads_back() {
        let proposal = ChatProposal {
            object: object(1),
            base: hash(2),
            content: blob(3, Some("peer-hint")),
            thread: hash(4),
            supersedes: Some(hash(5)),
        };
        let commands = [
            ThreadCommand::Event(ThreadEvent::Proposal(proposal.clone())),
            ThreadCommand::Event(ThreadEvent::Proposal(ChatProposal {
                supersedes: None,
                content: blob(3, None),
                ..proposal
            })),
            ThreadCommand::Event(ThreadEvent::Resolution(ChatResolution::edit(
                hash(4),
                object(1),
                hash(6),
            ))),
            ThreadCommand::Event(ThreadEvent::Resolution(
                ChatResolution::decision(hash(4), blob(7, None)).unwrap(),
            )),
            ThreadCommand::Event(ThreadEvent::Resolution(ChatResolution::no_action(hash(4)))),
            ThreadCommand::Event(ThreadEvent::Rejection(ChatProposalRejection {
                proposal: hash(8),
                reason: blob(9, None),
            })),
            ThreadCommand::Event(ThreadEvent::Withdrawal(ChatProposalWithdrawal {
                proposal: hash(8),
            })),
            ThreadCommand::Accept { proposal: hash(8) },
        ];
        for command in &commands {
            round_trip(command);
        }
    }

    #[test]
    fn the_line_names_its_parts_in_order() {
        let line = format_command(&ThreadCommand::Event(ThreadEvent::Resolution(
            ChatResolution::no_action(hash(4)),
        )))
        .unwrap();
        assert_eq!(line, format!("resolve {} no-action", hex_of(&hash(4))));
        let accept = format_command(&ThreadCommand::Accept { proposal: hash(8) }).unwrap();
        assert_eq!(accept, format!("accept {}", hex_of(&hash(8))));
    }

    #[test]
    fn malformed_lines_say_what_is_wrong() {
        let thread = hex_of(&hash(4));
        let bad = [
            String::new(),
            "approve something".to_owned(),
            format!("resolve {thread}"),
            format!("resolve {thread} edit"),
            "resolve nothex no-action".to_owned(),
            format!("resolve {} no-action", &thread[..10]),
            format!("resolve {thread} decision {thread}"),
            format!("resolve {thread} decision {thread}/{thread}@"),
            format!("reject {thread}"),
            format!("withdraw {thread} {thread}"),
            "accept".to_owned(),
            format!(
                "propose {thread}/{thread} base {thread} content {thread}/{thread} thread {thread} extra"
            ),
        ];
        for line in &bad {
            assert!(
                matches!(parse_command(line), Err(ThreadTextError::Syntax(_))),
                "{line:?} should be a syntax error"
            );
        }
    }

    #[test]
    fn a_line_that_breaks_a_field_rule_is_refused_like_the_event() {
        let long_hint = "h".repeat(crate::chat::MAX_HINT_BYTES + 1);
        let thread = hex_of(&hash(4));
        let line = format!("resolve {thread} decision {thread}/{thread}@{long_hint}");
        assert!(matches!(
            parse_command(&line),
            Err(ThreadTextError::Event(_))
        ));
    }

    #[test]
    fn a_hint_with_whitespace_has_no_text_form() {
        let command = ThreadCommand::Event(ThreadEvent::Rejection(ChatProposalRejection {
            proposal: hash(8),
            reason: blob(9, Some("two words")),
        }));
        assert!(matches!(
            format_command(&command),
            Err(ThreadTextError::Syntax(_))
        ));
    }

    #[test]
    fn the_projection_reads_back_with_states_concurrency_and_staleness() {
        let open = ChatProposal {
            object: object(1),
            base: hash(2),
            content: blob(3, None),
            thread: hash(4),
            supersedes: None,
        };
        let accepted = ChatProposal {
            base: hash(12),
            ..open.clone()
        };
        let rejected = ChatProposal {
            base: hash(22),
            ..open.clone()
        };
        let events = vec![
            record(30, 2, ThreadEvent::Proposal(open)),
            record(31, 2, ThreadEvent::Proposal(accepted)),
            record(32, 2, ThreadEvent::Proposal(rejected)),
            record(
                33,
                3,
                ThreadEvent::Rejection(ChatProposalRejection {
                    proposal: hash(32),
                    reason: blob(9, None),
                }),
            ),
            record(
                40,
                3,
                ThreadEvent::Resolution(ChatResolution::edit(hash(4), object(1), hash(50))),
            ),
            record(
                41,
                5,
                ThreadEvent::Resolution(ChatResolution::no_action(hash(4))),
            ),
            record(
                42,
                3,
                ThreadEvent::Resolution(ChatResolution::no_action(hash(60))),
            ),
        ];
        let edits = vec![AcceptingEdit {
            id: hash(50),
            author: author(3),
            object: object(1),
            proposal: hash(31),
        }];
        let mut reversed = events.clone();
        reversed.reverse();
        let projection = project_threads(events, edits.clone(), &Open);
        let heads = vec![hash(12)];
        let text = render_projection(&projection, &|_| Some(heads.clone()));

        assert!(text.contains(&format!(
            "thread {} has 2 concurrent resolutions, none chosen:",
            hex_of(&hash(4))
        )));
        assert!(text.contains(&format!(
            "thread {} resolved: no action, by {}",
            hex_of(&hash(60)),
            hex::encode(author(3))
        )));
        assert!(text.contains(&format!(
            "edit {} on object {}",
            hex_of(&hash(50)),
            object_text(&object(1))
        )));
        assert!(text.contains(&format!(
            "proposal {} on object {} at base {}",
            hex_of(&hash(30)),
            object_text(&object(1)),
            hex_of(&hash(2))
        )));
        assert!(text.contains(": open (stale: its base is no longer a head of the object)"));
        assert!(text.contains(&format!("accepted by edit {}", hex_of(&hash(50)))));
        assert!(text.contains(&format!("rejected by {} (reason", hex_of(&hash(33)))));
        assert!(!text.contains("refused"));

        // Without heads, nothing is called stale.
        let unknown = render_projection(&projection, &|_| None);
        assert!(!unknown.contains("stale"));
        // The same event set renders the same text in any arrival order.
        let again = project_threads(reversed, edits, &Open);
        assert_eq!(text, render_projection(&again, &|_| Some(heads.clone())));
    }

    #[test]
    fn the_viewers_projection_renders_nothing_it_cannot_read() {
        let events = vec![
            record(
                30,
                2,
                ThreadEvent::Proposal(ChatProposal {
                    object: object(1),
                    base: hash(2),
                    content: blob(3, None),
                    thread: hash(4),
                    supersedes: None,
                }),
            ),
            record(
                41,
                5,
                ThreadEvent::Resolution(ChatResolution::no_action(hash(4))),
            ),
        ];
        let projection = project_threads(events, Vec::new(), &Open);
        let hidden = hash(4);
        let view = projection.visible_to(&|thread| *thread != hidden, &|_| true);
        let text = render_projection(&view, &|_| None);
        assert!(text.is_empty(), "{text}");
    }

    fn arb_hash() -> impl Strategy<Value = Hash> {
        any::<[u8; 32]>().prop_map(Hash::from_bytes)
    }

    fn arb_blob() -> impl Strategy<Value = ChatLink> {
        (
            arb_hash(),
            any::<[u8; 32]>(),
            proptest::option::of("[a-z0-9.:-]{1,40}"),
        )
            .prop_map(|(hash, author, hint)| ChatLink {
                kind: ChatLinkKind::Blob,
                hash,
                author,
                hint,
            })
    }

    fn arb_command() -> impl Strategy<Value = ThreadCommand> {
        let object = (arb_hash(), any::<[u8; 32]>()).prop_map(|(id, by)| ObjectRef::new(id, by));
        prop_oneof![
            (
                object.clone(),
                arb_hash(),
                arb_blob(),
                arb_hash(),
                proptest::option::of(arb_hash())
            )
                .prop_map(|(object, base, content, thread, supersedes)| {
                    ThreadCommand::Event(ThreadEvent::Proposal(ChatProposal {
                        object,
                        base,
                        content,
                        thread,
                        supersedes,
                    }))
                }),
            (arb_hash(), object, arb_hash()).prop_map(|(thread, object, edit)| {
                ThreadCommand::Event(ThreadEvent::Resolution(ChatResolution::edit(
                    thread, object, edit,
                )))
            }),
            (arb_hash(), arb_blob()).prop_map(|(thread, summary)| {
                ThreadCommand::Event(ThreadEvent::Resolution(
                    ChatResolution::decision(thread, summary).unwrap(),
                ))
            }),
            arb_hash().prop_map(|thread| {
                ThreadCommand::Event(ThreadEvent::Resolution(ChatResolution::no_action(thread)))
            }),
            (arb_hash(), arb_blob()).prop_map(|(proposal, reason)| {
                ThreadCommand::Event(ThreadEvent::Rejection(ChatProposalRejection {
                    proposal,
                    reason,
                }))
            }),
            arb_hash().prop_map(|proposal| {
                ThreadCommand::Event(ThreadEvent::Withdrawal(ChatProposalWithdrawal { proposal }))
            }),
            arb_hash().prop_map(|proposal| ThreadCommand::Accept { proposal }),
        ]
    }

    proptest! {
        #[test]
        fn any_command_reads_back_as_itself(command in arb_command()) {
            let line = format_command(&command).unwrap();
            prop_assert_eq!(parse_command(&line).unwrap(), command);
        }
    }
}
