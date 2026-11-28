use serde::{Deserialize, Serialize};

use super::*;

/// Stable identity root for one public board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Board {
    /// Stable ID chosen by the creator; the signed author's key scopes it.
    pub board_id: String,
    /// Optional source root when this board is a successor fork.
    pub forked_from: Option<ArtifactHash>,
    /// Theme applied by reference, if the board names one. The named
    /// [`Theme`] object must already be in the graph when the board root
    /// is accepted: publish the theme first, then the board that applies
    /// it. Older v1 board bodies written before this field existed decode
    /// as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_ref: Option<ArtifactHash>,
}

/// Versioned board rules. A revision is a new immutable object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    /// Board root hash.
    pub board: ArtifactHash,
    /// One-based revision number within this board.
    pub revision: u64,
    /// Human-readable rule text; no executable content is interpreted.
    pub text: String,
}

/// A named discussion area within a board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forum {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Display name.
    pub name: String,
}

/// A discussion thread, optionally scoped to a forum and rules revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thread {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Optional forum object hash.
    pub forum_ref: Option<ArtifactHash>,
    /// Optional rules revision pinned by this thread.
    pub rules_ref: Option<ArtifactHash>,
    /// Thread title.
    pub title: String,
}

/// Reference-not-content pointer from a board post to a note, per the
/// note->board sharing model decided on #5830 (implementation #7587).
///
/// The post stores ONLY this reference: the note's object id, the home
/// server that serves it, the pinned version id plus that version's
/// content hash, and the sharer attribution. Note content is never
/// copied into the post, so pinned-view semantics and visibility
/// revocation on the home server stay meaningful (later slices render
/// live vs pinned views and resolve cross-server, grant-checked and
/// hash-verified against `pinned_content_hash`). The referenced note
/// is NOT a board-graph dependency: it need not — and ordinarily will
/// not — be present in this graph, so graph insertion never requires
/// it and replication closure never claims it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteRef {
    /// Object id of the shared note on its home server.
    pub object: ArtifactHash,
    /// Home server that serves the note (host identifier text).
    pub home_server: String,
    /// Version id of the note version pinned at share time.
    pub pinned_version: ArtifactHash,
    /// Content hash of the pinned note version; renderers and
    /// resolvers must verify fetched content against it.
    pub pinned_content_hash: ArtifactHash,
    /// Sharer's canonical Ed25519 public key; must match the post's
    /// signer, so a share can only be attributed to its author.
    /// Sharing grants no edit rights and creates no fork.
    pub sharer: String,
}

/// A revision candidate from the note's current local projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteHead {
    /// Immutable revision identifier.
    pub revision: ArtifactHash,
    /// Content hash for the revision's canonical content.
    pub content_hash: ArtifactHash,
    /// Whether this revision descends from the share's pinned revision.
    pub descends_from_pin: bool,
}

/// How an embedded note is presented relative to its immutable share pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteEmbedResolution {
    /// The only current head is still the pinned revision.
    Current(NoteHead),
    /// One current head supersedes the pin; show it live and offer the pin.
    Updated(NoteHead),
    /// Multiple heads exist. Preserve all of them; never choose by arrival order.
    Diverged(Vec<NoteHead>),
    /// The local head is not on the pinned revision's descendant line.
    SupersededLine(Vec<NoteHead>),
    /// No usable current head is available; the pin remains the citation target.
    Unavailable,
}

/// The preferred default projection for a note embed. Viewers may still switch
/// between this view and its counterpart without changing the signed share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteEmbedView {
    /// Follow the authorized current local head (the default when omitted).
    Live,
    /// Show the immutable revision named by `NoteRef::pinned_version`.
    Pinned,
}

/// Resolve the live-view badge and candidates for an embedded note.
///
/// The caller supplies the note subsystem's already-authorized local heads and
/// their ancestry relation to the immutable pin. This function deliberately
/// does not fetch note content or pick a winner among concurrent heads.
pub fn resolve_note_embed(note_ref: &NoteRef, heads: &[NoteHead]) -> NoteEmbedResolution {
    if heads.is_empty() {
        return NoteEmbedResolution::Unavailable;
    }
    if heads.len() > 1 {
        return NoteEmbedResolution::Diverged(heads.to_vec());
    }
    let head = heads[0].clone();
    if head.revision == note_ref.pinned_version {
        NoteEmbedResolution::Current(head)
    } else if head.descends_from_pin {
        NoteEmbedResolution::Updated(head)
    } else {
        NoteEmbedResolution::SupersededLine(vec![head])
    }
}

/// The immutable target selected for an embedded note's presentation.
/// Markup renderers consume this projection and fetch/verify the content
/// against `content_hash`; this module never fetches note content itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteEmbedTarget {
    /// A single authorized live head.
    Live(NoteHead),
    /// All live candidates are retained when the note has concurrent heads.
    LiveCandidates(Vec<NoteHead>),
    /// The version and hash signed into the board post.
    Pinned {
        /// Immutable note version id.
        version: ArtifactHash,
        /// Expected canonical content hash.
        content_hash: ArtifactHash,
    },
}

/// Presentation decision for a note embed, independent of any UI or markup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteEmbedPresentation {
    /// Requested/default view, retained so the UI can expose a view switch.
    pub preferred_view: NoteEmbedView,
    /// View represented by `target`; live-unavailable falls back to pinned.
    pub displayed_view: NoteEmbedView,
    /// Immutable content target(s) the renderer may display.
    pub target: NoteEmbedTarget,
    /// Whether an available live head differs from the post's pinned version.
    pub changed_since_posted: bool,
}

/// Select a live or pinned display target without choosing between concurrent
/// heads. `resolution` must be the result of [`resolve_note_embed`] for this
/// `note_ref`; inconsistent revision/content hashes are still reported as
/// changed, while the caller remains responsible for verifying fetched bytes.
/// If the live view is unavailable, fall back to the signed pin.
pub fn present_note_embed(
    note_ref: &NoteRef,
    preferred_view: Option<NoteEmbedView>,
    resolution: &NoteEmbedResolution,
) -> NoteEmbedPresentation {
    let preferred_view = preferred_view.unwrap_or(NoteEmbedView::Live);
    let heads: Vec<&NoteHead> = match resolution {
        NoteEmbedResolution::Current(head) | NoteEmbedResolution::Updated(head) => vec![head],
        NoteEmbedResolution::Diverged(heads) | NoteEmbedResolution::SupersededLine(heads) => {
            heads.iter().collect()
        }
        NoteEmbedResolution::Unavailable => Vec::new(),
    };
    let changed_since_posted = heads.iter().any(|head| {
        head.revision != note_ref.pinned_version
            || head.content_hash != note_ref.pinned_content_hash
    });

    let pinned = || NoteEmbedTarget::Pinned {
        version: note_ref.pinned_version.clone(),
        content_hash: note_ref.pinned_content_hash.clone(),
    };
    let displayed_view = match (preferred_view, resolution) {
        (NoteEmbedView::Live, NoteEmbedResolution::Unavailable) => NoteEmbedView::Pinned,
        (view, _) => view,
    };
    let target = match (preferred_view, resolution) {
        (NoteEmbedView::Pinned, _) | (NoteEmbedView::Live, NoteEmbedResolution::Unavailable) => {
            pinned()
        }
        (NoteEmbedView::Live, NoteEmbedResolution::Current(head))
        | (NoteEmbedView::Live, NoteEmbedResolution::Updated(head)) => {
            NoteEmbedTarget::Live(head.clone())
        }
        (NoteEmbedView::Live, NoteEmbedResolution::Diverged(heads))
        | (NoteEmbedView::Live, NoteEmbedResolution::SupersededLine(heads)) => {
            NoteEmbedTarget::LiveCandidates(heads.clone())
        }
    };

    NoteEmbedPresentation {
        preferred_view,
        displayed_view,
        target,
        changed_since_posted,
    }
}

/// A passage within the pinned note version that a reply is about.
/// Offsets are byte ranges in the section's canonical text; `quote_hash`
/// is the content hash of that exact range and is used to relocate it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotePassageSelector {
    /// Stable section identifier in the note version.
    pub section_id: String,
    /// Inclusive byte offset in the section text.
    pub start_byte: u64,
    /// Exclusive byte offset in the section text.
    pub end_byte: u64,
    /// SHA-256 of the selected passage text.
    pub quote_hash: ArtifactHash,
}

/// Immutable anchor from a board reply to the note version discussed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteAnchor {
    /// Note identity on its home server.
    pub object: ArtifactHash,
    /// Version discussed by the thread (normally the post's pinned version).
    pub version: ArtifactHash,
    /// `None` means the reply is about the note as a whole.
    pub passage: Option<NotePassageSelector>,
}

/// Passage candidate found in the current note head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotePassage {
    /// Stable section identifier.
    pub section_id: String,
    /// Inclusive byte offset in the section text.
    pub start_byte: u64,
    /// Exclusive byte offset in the section text.
    pub end_byte: u64,
    /// SHA-256 of the candidate passage text.
    pub quote_hash: ArtifactHash,
}

/// Where an anchored reply should be displayed relative to the live head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteAnchorResolution {
    /// The passage remains at its original location.
    Exact(NotePassageSelector),
    /// The same section and passage were found at a new location.
    Relocated(NotePassageSelector),
    /// The passage no longer exists; keep the reply on its pinned version.
    Orphaned,
    /// The reply is anchored to the note as a whole, not a passage.
    WholeNote,
}

/// Resolve a passage without ever substituting different text for its anchor.
pub fn resolve_note_anchor(
    anchor: &NoteAnchor,
    current_passages: &[NotePassage],
) -> NoteAnchorResolution {
    let Some(selector) = &anchor.passage else {
        return NoteAnchorResolution::WholeNote;
    };

    let same_quote = current_passages.iter().filter(|passage| {
        passage.section_id == selector.section_id && passage.quote_hash == selector.quote_hash
    });
    if let Some(exact) = same_quote.clone().find(|passage| {
        passage.start_byte == selector.start_byte && passage.end_byte == selector.end_byte
    }) {
        return NoteAnchorResolution::Exact(NotePassageSelector {
            section_id: exact.section_id.clone(),
            start_byte: exact.start_byte,
            end_byte: exact.end_byte,
            quote_hash: exact.quote_hash.clone(),
        });
    }

    if let Some(relocated) = same_quote.min_by_key(|passage| passage.start_byte) {
        return NoteAnchorResolution::Relocated(NotePassageSelector {
            section_id: relocated.section_id.clone(),
            start_byte: relocated.start_byte,
            end_byte: relocated.end_byte,
            quote_hash: relocated.quote_hash.clone(),
        });
    }

    NoteAnchorResolution::Orphaned
}

/// A display target for a reply anchored to a note passage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteReplyPresentation {
    /// Version whose text the reply should be shown against.
    pub version: ArtifactHash,
    /// `Some` only when the passage is exact or safely relocated.
    pub passage: Option<NotePassageSelector>,
    /// Orphaned replies stay on their original version and are never remapped.
    pub orphaned: bool,
}

/// Project a resolution produced by [`resolve_note_anchor`] into a display
/// version. An orphan always uses the immutable version named by the anchor,
/// never a guessed replacement. The caller must pair the resolution with the
/// same anchor; this projection does not revalidate passage hashes.
pub fn present_note_reply(
    anchor: &NoteAnchor,
    current_version: &ArtifactHash,
    resolution: &NoteAnchorResolution,
) -> NoteReplyPresentation {
    match resolution {
        NoteAnchorResolution::Exact(selector) | NoteAnchorResolution::Relocated(selector) => {
            NoteReplyPresentation {
                version: current_version.clone(),
                passage: Some(selector.clone()),
                orphaned: false,
            }
        }
        NoteAnchorResolution::Orphaned => NoteReplyPresentation {
            version: anchor.version.clone(),
            passage: anchor.passage.clone(),
            orphaned: true,
        },
        NoteAnchorResolution::WholeNote => NoteReplyPresentation {
            version: current_version.clone(),
            passage: None,
            orphaned: false,
        },
    }
}

/// Immutable post or post revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Post {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Thread object hash.
    pub thread: ArtifactHash,
    /// Parent post hash for a reply, if any.
    pub parent_post: Option<ArtifactHash>,
    /// Markdown/plain text body; media bytes are never embedded here.
    pub body: String,
    /// Attachment-record hashes.
    pub attachments: Vec<ArtifactHash>,
    /// Typed hook for a signed game post; it does not imply execution.
    pub embed_ref: Option<GamePostRef>,
    /// Reference-not-content pointer to a shared note, if this post
    /// shares one (#7587). Older v1 post bodies written before this
    /// field existed decode as .
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_ref: Option<NoteRef>,
    /// Passage or whole-note anchor for a reply in a note-sharing thread.
    /// Older v1 post bodies decode with no anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_anchor: Option<NoteAnchor>,
    /// Optional signed default for displaying an attached note; absent means live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_view: Option<NoteEmbedView>,
}

/// Metadata-only reference to a content-addressed attachment blob.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Content-addressed blob hash (not an inline payload).
    pub blob: ArtifactHash,
    /// Declared media type.
    pub media_type: String,
    /// Blob size in bytes.
    pub size_bytes: u64,
}

/// Optional self-authored membership statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Membership {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Member's canonical Ed25519 public key; must match the signer.
    pub member: String,
    /// Optional signed display-name claim; it is not globally unique.
    pub display_name: Option<String>,
}

/// Board-scoped role names. Ownership is always the board-root signer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardRole {
    /// May publish rules/forums and signed moderation records.
    Moderator,
}

/// Owner-signed grant or revocation of a board-scoped role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAttestation {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Subject's canonical Ed25519 public key.
    pub member: String,
    /// Role being granted or revoked.
    pub role: BoardRole,
    /// Whether the role is active after this attestation.
    pub granted: bool,
    /// Previous attestation this record supersedes, if changing a role.
    pub supersedes: Option<ArtifactHash>,
}

/// Supported append-only moderation log actions; these do not enforce a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationActionKind {
    /// Mark a referenced object hidden in a board view.
    Hide,
    /// Reverse a prior hide by adding another log entry.
    Restore,
}

/// Append-only, board-scoped moderation statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModerationAction {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Object in this board's graph being acted upon.
    pub target: ArtifactHash,
    /// Action recorded by the signer.
    pub action: ModerationActionKind,
    /// Optional human-readable explanation.
    pub reason: Option<String>,
}

/// A fork cites a source board and the source graph frontier it starts from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardFork {
    /// Source board root hash.
    pub source_board: ArtifactHash,
    /// New board root hash, whose `forked_from` names `source_board`.
    pub fork_board: ArtifactHash,
    /// Source objects included in the fork's cited starting frontier.
    pub source_frontier: Vec<ArtifactHash>,
}

/// Explicit immutable version link between two objects by the same author.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Supersede {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Earlier object hash.
    pub previous: ArtifactHash,
    /// New object hash that replaces it in a projection.
    pub replacement: ArtifactHash,
}

/// Author-signed deletion request; the original signed object stays intact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tombstone {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Object requested for deletion by its author.
    pub target: ArtifactHash,
    /// Optional explanation.
    pub reason: Option<String>,
}

/// Closed set of board layouts a theme may select. Custom HTML templates
/// are explicitly out of v1: a theme is data (tokens, one layout, asset
/// references), never markup or code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeLayout {
    /// A single vertical list of threads.
    List,
    /// A grid of thread cards.
    Grid,
    /// A dense list with reduced spacing.
    Compact,
}

/// A signed, portable board theme: design tokens, one layout, and
/// content-addressed asset references.
///
/// A theme names no board. It is a reusable signed package, and a board
/// applies one by reference through [`Board::theme_ref`]. That keeps the
/// content-addressed graph acyclic: a theme that named its board could
/// never be referenced from that board's own root hash. `forked_from` is
/// the fork lineage — the theme this one derives from, if any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    /// Human-readable theme name.
    pub name: String,
    /// Design tokens by name (for example `accent`, `surface`); values
    /// are plain data — colors, sizes, font names — never markup.
    pub tokens: BTreeMap<String, String>,
    /// Layout selected from the closed v1 set.
    pub layout: ThemeLayout,
    /// Content-addressed asset blobs (logos, backgrounds); metadata
    /// references only, payloads are never embedded.
    pub assets: Vec<ArtifactHash>,
    /// Theme this one was forked from, if any (fork lineage).
    pub forked_from: Option<ArtifactHash>,
}

/// A signed, opt-in host listing for one board (#3806).
///
/// The board creator signs a listing naming a host (by its Ed25519 key)
/// that replicates the board's signed object set. Listings are versioned
/// per (board, host): the first listing is version 1, and every later
/// listing supersedes the previous one and advances the version by
/// exactly one, so publication and withdrawal both follow the signed
/// version/supersede rules of the graph — never host clocks and never
/// an unsigned host assertion. A withdrawal is a newer listing with
/// `listed == false`. Delisting is host-local in effect: it ends this
/// listing's replication offer, deletes nothing from any graph, and
/// makes no claim of global erasure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardListing {
    /// Board root hash being listed.
    pub board: ArtifactHash,
    /// Canonical Ed25519 hex key of the host offering replication.
    pub host: String,
    /// One-based per-(board, host) listing version.
    pub version: u64,
    /// True publishes the listing; false withdraws it.
    pub listed: bool,
    /// Optional human/transport contact for the host (plain data).
    pub contact: Option<String>,
    /// Previous listing for the same board and host, if any.
    pub supersedes: Option<ArtifactHash>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod note_anchor_tests {
    use super::*;

    fn hash(ch: char) -> ArtifactHash {
        ArtifactHash::new(ch.to_string().repeat(64)).unwrap()
    }

    fn anchor() -> NoteAnchor {
        NoteAnchor {
            object: hash('a'),
            version: hash('b'),
            passage: Some(NotePassageSelector {
                section_id: "intro".into(),
                start_byte: 4,
                end_byte: 12,
                quote_hash: hash('c'),
            }),
        }
    }

    fn passage(start_byte: u64, end_byte: u64, quote: char) -> NotePassage {
        NotePassage {
            section_id: "intro".into(),
            start_byte,
            end_byte,
            quote_hash: hash(quote),
        }
    }

    fn note_head(revision: char, content: char, descends_from_pin: bool) -> NoteHead {
        NoteHead {
            revision: hash(revision),
            content_hash: hash(content),
            descends_from_pin,
        }
    }

    fn note_ref() -> NoteRef {
        NoteRef {
            object: hash('a'),
            home_server: "https://notes.example".into(),
            pinned_version: hash('b'),
            pinned_content_hash: hash('c'),
            sharer: "d".repeat(64),
        }
    }

    #[test]
    fn anchor_resolves_exact_when_quote_stays_at_original_range() {
        assert_eq!(
            resolve_note_anchor(&anchor(), &[passage(4, 12, 'c')]),
            NoteAnchorResolution::Exact(anchor().passage.unwrap())
        );
    }

    #[test]
    fn anchor_resolves_relocated_only_for_same_section_and_quote() {
        assert_eq!(
            resolve_note_anchor(&anchor(), &[passage(20, 28, 'c')]),
            NoteAnchorResolution::Relocated(NotePassageSelector {
                section_id: "intro".into(),
                start_byte: 20,
                end_byte: 28,
                quote_hash: hash('c'),
            })
        );
    }

    #[test]
    fn missing_or_changed_quote_is_orphaned_not_repointed() {
        assert_eq!(
            resolve_note_anchor(&anchor(), &[passage(4, 12, 'd')]),
            NoteAnchorResolution::Orphaned
        );
    }

    #[test]
    fn whole_note_anchor_needs_no_passage_match() {
        let mut whole_note = anchor();
        whole_note.passage = None;
        assert_eq!(
            resolve_note_anchor(&whole_note, &[]),
            NoteAnchorResolution::WholeNote
        );
    }

    #[test]
    fn embed_resolution_is_current_when_head_is_the_pin() {
        let head = note_head('b', 'c', true);
        assert_eq!(
            resolve_note_embed(&note_ref(), std::slice::from_ref(&head)),
            NoteEmbedResolution::Current(head)
        );
    }

    #[test]
    fn embed_resolution_marks_a_descendant_as_updated() {
        let head = note_head('e', 'f', true);
        assert_eq!(
            resolve_note_embed(&note_ref(), std::slice::from_ref(&head)),
            NoteEmbedResolution::Updated(head)
        );
    }

    #[test]
    fn embed_resolution_preserves_all_concurrent_heads() {
        let heads = vec![note_head('e', 'f', true), note_head('0', '1', true)];
        assert_eq!(
            resolve_note_embed(&note_ref(), &heads),
            NoteEmbedResolution::Diverged(heads)
        );
    }

    #[test]
    fn embed_resolution_badges_a_head_off_the_pinned_line() {
        let head = note_head('e', 'f', false);
        assert_eq!(
            resolve_note_embed(&note_ref(), std::slice::from_ref(&head)),
            NoteEmbedResolution::SupersededLine(vec![head])
        );
    }

    #[test]
    fn embed_resolution_uses_unavailable_when_there_is_no_local_head() {
        assert_eq!(
            resolve_note_embed(&note_ref(), &[]),
            NoteEmbedResolution::Unavailable
        );
    }

    #[test]
    fn embed_presentation_defaults_to_live_and_reports_changed_head() {
        let head = note_head('e', 'f', true);
        let presentation = present_note_embed(
            &note_ref(),
            None,
            &NoteEmbedResolution::Updated(head.clone()),
        );
        assert_eq!(presentation.preferred_view, NoteEmbedView::Live);
        assert_eq!(presentation.displayed_view, NoteEmbedView::Live);
        assert_eq!(presentation.target, NoteEmbedTarget::Live(head));
        assert!(presentation.changed_since_posted);
    }

    #[test]
    fn embed_presentation_for_unchanged_head_is_not_badged() {
        let head = note_head('b', 'c', true);
        let presentation = present_note_embed(
            &note_ref(),
            None,
            &NoteEmbedResolution::Current(head.clone()),
        );
        assert_eq!(presentation.target, NoteEmbedTarget::Live(head));
        assert!(!presentation.changed_since_posted);
    }

    #[test]
    fn embed_presentation_can_pin_and_falls_back_to_pin_when_live_is_unavailable() {
        let reference = note_ref();
        let pinned = NoteEmbedTarget::Pinned {
            version: reference.pinned_version.clone(),
            content_hash: reference.pinned_content_hash.clone(),
        };
        assert_eq!(
            present_note_embed(
                &reference,
                Some(NoteEmbedView::Pinned),
                &NoteEmbedResolution::Updated(note_head('e', 'f', true)),
            )
            .target,
            pinned
        );
        let unavailable = present_note_embed(&reference, None, &NoteEmbedResolution::Unavailable);
        assert_eq!(unavailable.target, pinned);
        assert_eq!(unavailable.preferred_view, NoteEmbedView::Live);
        assert_eq!(unavailable.displayed_view, NoteEmbedView::Pinned);
    }

    #[test]
    fn embed_presentation_preserves_all_live_candidates() {
        let heads = vec![note_head('e', 'f', true), note_head('0', '1', true)];
        let presentation = present_note_embed(
            &note_ref(),
            Some(NoteEmbedView::Live),
            &NoteEmbedResolution::Diverged(heads.clone()),
        );
        assert_eq!(presentation.target, NoteEmbedTarget::LiveCandidates(heads));
        assert!(presentation.changed_since_posted);
    }

    #[test]
    fn embed_presentation_keeps_off_line_live_head_and_marks_it_changed() {
        let head = note_head('e', 'f', false);
        let presentation = present_note_embed(
            &note_ref(),
            Some(NoteEmbedView::Live),
            &NoteEmbedResolution::SupersededLine(vec![head.clone()]),
        );
        assert_eq!(
            presentation.target,
            NoteEmbedTarget::LiveCandidates(vec![head])
        );
        assert!(presentation.changed_since_posted);
    }

    #[test]
    fn orphaned_reply_stays_on_its_anchored_version_and_passage() {
        let anchor = anchor();
        let presentation = present_note_reply(&anchor, &hash('e'), &NoteAnchorResolution::Orphaned);
        assert_eq!(presentation.version, anchor.version);
        assert_eq!(presentation.passage, anchor.passage);
        assert!(presentation.orphaned);
    }

    #[test]
    fn exact_reply_uses_the_current_version_at_the_original_passage() {
        let anchor = anchor();
        let current = hash('e');
        let selector = anchor.passage.clone().unwrap();
        let presentation = present_note_reply(
            &anchor,
            &current,
            &NoteAnchorResolution::Exact(selector.clone()),
        );
        assert_eq!(presentation.version, current);
        assert_eq!(presentation.passage, Some(selector));
        assert!(!presentation.orphaned);
    }

    #[test]
    fn resolved_reply_uses_the_current_version_and_resolved_passage() {
        let anchor = anchor();
        let current = hash('e');
        let relocated = NotePassageSelector {
            section_id: "intro".into(),
            start_byte: 20,
            end_byte: 28,
            quote_hash: hash('c'),
        };
        let presentation = present_note_reply(
            &anchor,
            &current,
            &NoteAnchorResolution::Relocated(relocated.clone()),
        );
        assert_eq!(presentation.version, current);
        assert_eq!(presentation.passage, Some(relocated));
        assert!(!presentation.orphaned);
    }

    #[test]
    fn whole_note_reply_uses_the_current_version_without_a_passage() {
        let mut anchor = anchor();
        anchor.passage = None;
        let current = hash('e');
        let presentation = present_note_reply(&anchor, &current, &NoteAnchorResolution::WholeNote);
        assert_eq!(presentation.version, current);
        assert_eq!(presentation.passage, None);
        assert!(!presentation.orphaned);
    }

    #[test]
    fn post_note_view_preference_round_trips_and_absence_means_live() {
        let pinned = NoteEmbedView::Pinned;
        let encoded = serde_json::to_string(&pinned).unwrap();
        assert_eq!(
            serde_json::from_str::<NoteEmbedView>(&encoded).unwrap(),
            pinned
        );

        let post = serde_json::json!({
            "board": "a".repeat(64),
            "thread": "b".repeat(64),
            "parent_post": null,
            "body": "shared",
            "attachments": [],
            "embed_ref": null,
            "note_ref": null,
            "note_anchor": null
        });
        let decoded: Post = serde_json::from_value(post).unwrap();
        assert_eq!(decoded.note_view, None);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod note_projection_properties {
    //! Generated coverage for the note embed and reply-anchor projections:
    //! a resolved passage keeps its section and quote, an orphan keeps
    //! its original version and passage, and a live view keeps every
    //! concurrent head.

    use proptest::prelude::*;

    use super::*;

    fn hash(ch: char) -> ArtifactHash {
        ArtifactHash::new(ch.to_string().repeat(64)).unwrap()
    }

    /// Hashes from a small alphabet, so generated values often collide.
    fn small_hash() -> impl Strategy<Value = ArtifactHash> {
        prop::sample::select(vec!['1', '2', '3']).prop_map(hash)
    }

    fn selector() -> impl Strategy<Value = NotePassageSelector> {
        (
            prop::sample::select(vec!["intro", "body"]),
            0_u64..6,
            0_u64..4,
            small_hash(),
        )
            .prop_map(
                |(section, start_byte, len, quote_hash)| NotePassageSelector {
                    section_id: section.to_owned(),
                    start_byte,
                    end_byte: start_byte + len,
                    quote_hash,
                },
            )
    }

    fn passage() -> impl Strategy<Value = NotePassage> {
        selector().prop_map(|selector| NotePassage {
            section_id: selector.section_id,
            start_byte: selector.start_byte,
            end_byte: selector.end_byte,
            quote_hash: selector.quote_hash,
        })
    }

    fn anchor() -> impl Strategy<Value = NoteAnchor> {
        prop::option::weighted(0.8, selector()).prop_map(|passage| NoteAnchor {
            object: hash('a'),
            version: hash('b'),
            passage,
        })
    }

    fn head() -> impl Strategy<Value = NoteHead> {
        (small_hash(), small_hash(), any::<bool>()).prop_map(
            |(revision, content_hash, descends_from_pin)| NoteHead {
                revision,
                content_hash,
                descends_from_pin,
            },
        )
    }

    fn note_ref() -> NoteRef {
        NoteRef {
            object: hash('a'),
            home_server: "https://notes.example".into(),
            pinned_version: hash('1'),
            pinned_content_hash: hash('1'),
            sharer: "d".repeat(64),
        }
    }

    fn as_selector(passage: &NotePassage) -> NotePassageSelector {
        NotePassageSelector {
            section_id: passage.section_id.clone(),
            start_byte: passage.start_byte,
            end_byte: passage.end_byte,
            quote_hash: passage.quote_hash.clone(),
        }
    }

    proptest! {
        #[test]
        fn anchor_resolution_never_substitutes_other_text(
            anchor in anchor(),
            passages in prop::collection::vec(passage(), 0..8),
        ) {
            let resolution = resolve_note_anchor(&anchor, &passages);
            let Some(wanted) = &anchor.passage else {
                prop_assert_eq!(resolution, NoteAnchorResolution::WholeNote);
                return Ok(());
            };
            let same_quote: Vec<NotePassageSelector> = passages
                .iter()
                .map(as_selector)
                .filter(|p| p.section_id == wanted.section_id && p.quote_hash == wanted.quote_hash)
                .collect();
            match resolution {
                NoteAnchorResolution::Exact(found) => {
                    prop_assert_eq!(&found, wanted);
                    prop_assert!(same_quote.contains(&found));
                }
                NoteAnchorResolution::Relocated(found) => {
                    prop_assert_eq!(&found.section_id, &wanted.section_id);
                    prop_assert_eq!(&found.quote_hash, &wanted.quote_hash);
                    prop_assert!(same_quote.contains(&found));
                    prop_assert!(!same_quote.contains(wanted));
                    let earliest = same_quote.iter().map(|p| p.start_byte).min();
                    prop_assert_eq!(Some(found.start_byte), earliest);
                }
                NoteAnchorResolution::Orphaned => prop_assert!(same_quote.is_empty()),
                NoteAnchorResolution::WholeNote => {
                    prop_assert!(false, "a passage anchor resolved as the whole note");
                }
            }
        }

        #[test]
        fn reply_presentation_keeps_orphans_on_their_version(
            anchor in anchor(),
            passages in prop::collection::vec(passage(), 0..8),
            current in small_hash(),
        ) {
            let resolution = resolve_note_anchor(&anchor, &passages);
            let shown = present_note_reply(&anchor, &current, &resolution);
            match &resolution {
                NoteAnchorResolution::Orphaned => {
                    prop_assert!(shown.orphaned);
                    prop_assert_eq!(&shown.version, &anchor.version);
                    prop_assert_eq!(&shown.passage, &anchor.passage);
                }
                NoteAnchorResolution::Exact(found) | NoteAnchorResolution::Relocated(found) => {
                    prop_assert!(!shown.orphaned);
                    prop_assert_eq!(&shown.version, &current);
                    prop_assert_eq!(shown.passage.as_ref(), Some(found));
                }
                NoteAnchorResolution::WholeNote => {
                    prop_assert!(!shown.orphaned);
                    prop_assert_eq!(&shown.version, &current);
                    prop_assert_eq!(&shown.passage, &None);
                }
            }
        }

        #[test]
        fn embed_presentation_keeps_every_live_head(
            heads in prop::collection::vec(head(), 0..5),
            pinned_view in any::<bool>(),
        ) {
            let note_ref = note_ref();
            let view = if pinned_view { NoteEmbedView::Pinned } else { NoteEmbedView::Live };
            let resolution = resolve_note_embed(&note_ref, &heads);
            let shown = present_note_embed(&note_ref, Some(view), &resolution);

            prop_assert_eq!(shown.preferred_view, view);
            prop_assert_eq!(
                shown.changed_since_posted,
                heads.iter().any(|h| {
                    h.revision != note_ref.pinned_version
                        || h.content_hash != note_ref.pinned_content_hash
                })
            );
            if pinned_view || heads.is_empty() {
                prop_assert_eq!(shown.displayed_view, NoteEmbedView::Pinned);
                prop_assert_eq!(
                    shown.target,
                    NoteEmbedTarget::Pinned {
                        version: note_ref.pinned_version.clone(),
                        content_hash: note_ref.pinned_content_hash.clone(),
                    }
                );
                return Ok(());
            }
            prop_assert_eq!(shown.displayed_view, NoteEmbedView::Live);
            let shown_heads = match shown.target {
                NoteEmbedTarget::Live(head) => vec![head],
                NoteEmbedTarget::LiveCandidates(heads) => heads,
                NoteEmbedTarget::Pinned { .. } => {
                    prop_assert!(false, "an available live view fell back to the pin");
                    unreachable!()
                }
            };
            prop_assert_eq!(shown_heads, heads);
        }
    }
}
