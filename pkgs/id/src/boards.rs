//! Signed, content-addressed board graph objects.
//!
//! A board object is the typed body of the existing [`SignedArtifact`] envelope,
//! not a second signature or storage protocol. Each object is immutable; edits
//! and deletions add [`BoardObject::Supersede`] and [`BoardObject::Tombstone`]
//! records. [`BoardGraph`] checks signatures, canonical references, board scope,
//! and the authority rules needed to accept an object. [`BoardObject::Theme`]
//! Federation (#3806) adds signed, opt-in [`BoardObject::BoardListing`]
//! records and [`BoardGraph::replicate_board_from`]: a listing names a
//! host that replicates the board's signed object set, replication
//! converges two native hosts on the same set with no primary-host
//! authority, and [`BoardGraph::replica_status_against`] makes a lagging
//! replica explicit instead of silently presenting it as current.
//! [`HostServingPolicy`] is the host-local serving layer: delisting a
//! board stops that host serving it and never deletes signed objects
//! or claims global erasure. It also adds signed, portable themes a board applies by reference
//! ([`Board::theme_ref`]), and [`BoardGraph::compose_view`] projects the
//! three-layer view (personal filter / board view / host view) under the
//! transparency rule: hidden is never erased. It does not render markup, serve
//! content, host live game sessions, or fetch attachment/game payloads.
//! Signed async game turns (#6160) are the one execution-adjacent exception:
//! [`BoardGraph::replay_game_lineage`] folds a verified turn lineage through
//! the exact package bytes the game post pins, under the sandbox limits, so
//! every peer recomputes the same state from the same signed inputs.
//! Duplicate `Rules` objects for the same board and revision are legal signed
//! history. [`BoardGraph::resolve_rules`] selects the greater content hash
//! within a revision; current rules are the highest revision, using that same
//! tie-break, so every peer projects the same signed set identically.
//! ## Kind-by-gate inventory (#4114, Phase 4 audit of design #4113)
//!
//! The checklist the next object kind is reviewed against. Every
//! [`BoardObject`] variant is dispositioned against the design's five
//! gates: (1) shape — `BoardObject::validate_shape` constrains every
//! field that carries a rule; (2) references at insert —
//! `BoardGraph::validate_links` validates every graph-reference field
//! (existence + board scope via `require_scoped` / `require_board`,
//! kind checked where a kind rule exists); (3) chain / resolution
//! posture; (4) projection effect in [`BoardGraph::compose_view`] (and
//! [`BoardGraph::latest_listings`] where relevant) under the
//! active-records rule — a record contributes effects only while
//! itself active (not superseded through an active supersede record);
//! (5) replication membership + closure in
//! [`BoardGraph::board_object_set`]. "Deliberate" cells are documented
//! divergences, not findings. Audited at the #4114 baseline; the
//! [`BoardObject::GamePost`] variant postdates the #4114 variant list
//! and is inventoried here as a fifteenth row.
//!
//! | Variant | 1 Shape | 2 References at insert | 3 Chain / resolution | 4 Projection | 5 Replication |
//! |---|---|---|---|---|
//! | `Board` | `board_id` 1–128 lower-case ASCII id chars | `forked_from` → `require_board`; `theme_ref` → exists + `Theme` kind | Identity root, not versioned: `Supersede`/`Tombstone` reject the `board` kind | Root always in its own view; resolved `theme` returned subject to host/personal suppression | Seed of its own set; closure pulls `forked_from` ancestor + `theme_ref`, recursively |
//! | `Rules` | `revision > 0`; text 1–16 KiB, no NUL | `board` → `require_moderator` (root exists + author is owner or moderator) | Duplicate `(board, revision)` objects are legal signed history (deliberate, #3964); [`BoardGraph::resolve_rules`] picks the highest revision, greater content hash within a revision, among **active** objects only (the active-only half was the audit finding below, fixed here) | Content object: shown while active; superseded rules are excluded from the view like any content | Seeded by scope; closure pulls `board` + the author's moderator grants |
//! | `Forum` | Name 1–16 KiB, no NUL | `board` → `require_moderator` | Not versioned; edits are new objects + `Supersede` | Content object: shown while active | Seeded by scope; closure pulls `board` + moderator grants |
//! | `Thread` | Title 1–16 KiB, no NUL | `board` → `require_board`; `forum_ref` → scoped `forum`; `rules_ref` → scoped `rules` (pinned exact hash) | Not versioned; edits via `Supersede` | Content object: shown while active | Seeded by scope; closure pulls `board`, `forum_ref`, `rules_ref` |
//! | `Post` | Body 0–16 KiB, no NUL; `attachments` ≤ 128 and unique | `board` → `require_board`; `thread` → scoped `thread`; `parent_post` → scoped `post` + same thread; each `attachments` → scoped `attachment`; `embed_ref.object` → scoped `game_post` (#3963) | Not versioned; edits via `Supersede` | Content object: shown while active | Seeded by scope; closure pulls `board`, `thread`, `parent_post`, `attachments`, `embed_ref` target |
//! | `Attachment` | Media type 1–16 KiB, no NUL, ≤ 128 bytes; `size_bytes` carries no bound (deliberate: declared metadata only — the payload is never fetched by this module, so no gate depends on it) | `board` → `require_board`; `blob` is an external content hash, not a graph reference (deliberate: metadata-only, payload never embedded) | Not versioned | Content object: shown while active | Seeded by scope; closure pulls `board` (`blob` is external) |
//! | `Membership` | `member` canonical Ed25519 hex; optional display name 1–16 KiB, no NUL | `board` → `require_board`; signer must be `member` (self-authored) | Not versioned; duplicate statements by one member are separate signed history — no uniqueness rule (deliberate: no design rule orders them) | Content object: shown while active | Seeded by scope; closure pulls `board` |
//! | `RoleAttestation` | `member` canonical Ed25519 hex; `role` is the closed [`BoardRole`] set | `board` → `require_board` + signer is the board creator; `supersedes` → scoped `role_attestation` + same member and role | Versioned by its own `supersedes` chain; the current frontier resolves at projection (fixed point, #4003) | Not view content per se: drives the moderator frontier — an attestation has effect only while active; `has_role` at insert is existence-based by deliberate design (see its doc: arrival-order independence) | Seeded by scope; closure pulls `board` + `supersedes`; also pulled as a moderator grant by `Rules`/`Forum`/`ModerationAction` closures |
//! | `ModerationAction` | Optional reason 0–16 KiB, no NUL; `action` is the closed kind set | `board` → `require_moderator`; `target` → scoped, any kind (deliberate: moderation may target any object kind; the board root cannot be a target because its `board_ref` is `None`) | Append-only log, not versioned; hide/restore effect is a signed count balance, order-independent | Never view content: listed in the public `moderation_log` always; hides only while the record is active **and** its author holds authority on the current attestation frontier | Seeded by scope; closure pulls `board`, `target`, + moderator grants |
//! | `BoardFork` | `source_frontier` ≤ 128 and unique | `source_board`/`fork_board` → `require_board`; fork root must cite `source_board` in `forked_from`; signer is the fork creator; each frontier entry exists and is scoped to the source board (the source root itself excepted) | Provenance record, not versioned | Not view content (`board_ref` is `None`): its effect is provenance/replication, not projection | Seeded for the **fork** board (`fork_board` match — #4081); closure pulls `source_board`, `fork_board`, `source_frontier`, and frontier objects' own dependencies recursively |
//! | `Supersede` | `previous != replacement` | `previous`/`replacement` → scoped, any kind, then: same kind, same author as signer, neither is a board root, and the edge must not close a cycle (all successors traversed) | The versioning mechanism itself | Never view content: only **active** supersede records (frontier fixed point) mark their `previous` superseded | Seeded by scope; closure pulls `board`, `previous`, `replacement` |
//! | `Tombstone` | Optional reason 0–16 KiB, no NUL | `target` → scoped, any kind, not a board root; signer must be the target's author | Not versioned; a tombstone's own effect ends if it is superseded | Never view content: an active tombstone hides its target at the board layer, with its reason reported | Seeded by scope; closure pulls `board`, `target` |
//! | `Theme` | Name 1–16 KiB, no NUL; ≤ 64 tokens, token names 1–64 id chars, values 1–512 bytes no NUL; `assets` ≤ 128 and unique | `forked_from` → exists + `Theme` kind + not self; `assets` are external content hashes, not graph references (deliberate, as `Attachment::blob`) | Not versioned; fork lineage only — boards pin the exact theme hash | Not view content: resolved into [`BoardView::theme`] for boards that apply it; a suppressed theme is reported in `hidden` | Names no board (`board_ref` is `None`): pulled into a set only via a board's `theme_ref` closure or a theme fork chain; closure pulls its own `forked_from` |
//! | `GamePost` | Package version 1–16 KiB, no NUL, ≤ 128 bytes; 1–32 participants, each a canonical key + 1–64 id-char role, `(author, role)` unique; `configuration`/`genesis` ≤ 16 KiB serialized | `board` → `require_board`; `package.hash` is an external package pin, not a graph reference (deliberate: data, never executed or fetched) | Not versioned; edits via `Supersede` | Content object: shown while active | Seeded by scope; closure pulls `board` |
//! | `BoardListing` | `host` canonical Ed25519 hex; `version > 0`; optional contact 1–16 KiB, no NUL | `board` → `require_board` + signer is the board creator; `supersedes` → scoped `board_listing` + same host + same author + `version` advances by exactly one | **Chain head unique at insert**: a first listing is version 1 and duplicates for `(board, host)` are refused (#4109); successors must supersede the current listing | Content object: shown while active; [`BoardGraph::latest_listings`] picks the greatest `(version, hash)` per host and `active_listings` keeps `listed` ones — a withdrawal is a newer unlisted version | Seeded by scope; closure pulls `board` + `supersedes` |
//!
//! Finding from this audit (fixed here, regression test
//! `superseded_rules_are_not_resolved`): [`BoardGraph::resolve_rules`]
//! previously considered every signed `Rules` object for a board,
//! including one an active `Supersede` record had superseded — so
//! when the superseded object had the greater content hash within
//! its revision, resolution returned the superseded text while
//! `compose_view` hid that same object. Resolution now excludes
//! objects superseded through an active supersede record — the same
//! frontier [`BoardGraph::compose_view`] uses, shared via
//! `BoardGraph::superseded_objects`. (Verified by source inspection
//! at the audit baseline: `resolve_rules` had no supersede filter at
//! all; a local probe run was started but the laptop build did not
//! finish inside the audit shift, so the regression test's verdict
//! is carried by CI.)

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, de};
use sha2::{Digest, Sha256};

use crate::artifact::SignedArtifact;

mod game;
pub use game::*;
mod records;
pub use records::*;
mod projection;
pub use projection::*;

/// Signed-artifact kind used for all typed board objects in v1.
pub const BOARD_OBJECT_KIND: &str = "board_object.v1";
/// Largest text field accepted by a board object.
pub const MAX_BOARD_TEXT_BYTES: usize = 16 * 1024;
/// Largest number of content references accepted in one object.
pub const MAX_BOARD_REFERENCES: usize = 128;
/// Largest number of design tokens accepted in one theme.
pub const MAX_THEME_TOKENS: usize = 64;
/// Largest design-token value accepted in a theme.
pub const MAX_THEME_TOKEN_VALUE_BYTES: usize = 512;
/// Largest number of assigned participants in one game post.
pub const MAX_GAME_PARTICIPANTS: usize = 32;
/// Largest serialized configuration or genesis state accepted by a game post.
pub const MAX_GAME_DATA_BYTES: usize = 16 * 1024;

/// A canonical lower-case SHA-256 artifact/content hash.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ArtifactHash(String);

impl ArtifactHash {
    /// Parse a canonical 32-byte lower-case hexadecimal hash.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        ensure!(
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "artifact reference must be 64 lower-case hexadecimal characters"
        );
        Ok(Self(value))
    }

    /// Return the canonical hexadecimal representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ArtifactHash {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// One typed body carried by a [`SignedBoardObject`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum BoardObject {
    /// Board identity root.
    Board(Board),
    /// Versioned rules.
    Rules(Rules),
    /// Discussion forum.
    Forum(Forum),
    /// Discussion thread.
    Thread(Thread),
    /// Immutable post.
    Post(Post),
    /// Metadata-only attachment.
    Attachment(Attachment),
    /// Self-authored membership statement.
    Membership(Membership),
    /// Owner-issued role grant or revocation.
    RoleAttestation(RoleAttestation),
    /// Append-only moderation log entry.
    ModerationAction(ModerationAction),
    /// Fork provenance record.
    BoardFork(BoardFork),
    /// Explicit version/supersede relation.
    Supersede(Supersede),
    /// Author deletion request.
    Tombstone(Tombstone),
    /// Signed portable theme.
    Theme(Theme),
    /// Package-pinned playable game post metadata.
    GamePost(GamePost),
    /// Signed challenge acceptance for a game post.
    GameChallengeAcceptance(GameChallengeAcceptance),
    /// Signed asynchronous game turn.
    GameTurn(GameTurn),
    /// Signed save point in a game line.
    GameSavePoint(GameSavePoint),
    /// Signed handoff of a saved game line to a named recipient.
    GameHandoff(GameHandoff),
    /// Signed acceptance of a game handoff by its named recipient.
    GameHandoffAcceptance(GameHandoffAcceptance),
    /// Signed game fork from a cited save point.
    GameFork(GameFork),
    /// Signed opt-in host listing / withdrawal.
    BoardListing(BoardListing),
}

impl BoardObject {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Board(_) => "board",
            Self::Rules(_) => "rules",
            Self::Forum(_) => "forum",
            Self::Thread(_) => "thread",
            Self::Post(_) => "post",
            Self::Attachment(_) => "attachment",
            Self::Membership(_) => "membership",
            Self::RoleAttestation(_) => "role_attestation",
            Self::ModerationAction(_) => "moderation_action",
            Self::BoardFork(_) => "board_fork",
            Self::Supersede(_) => "supersede",
            Self::Tombstone(_) => "tombstone",
            Self::Theme(_) => "theme",
            Self::GamePost(_) => "game_post",
            Self::GameChallengeAcceptance(_) => "game_challenge_acceptance",
            Self::GameTurn(_) => "game_turn",
            Self::GameSavePoint(_) => "game_save_point",
            Self::GameHandoff(_) => "game_handoff",
            Self::GameHandoffAcceptance(_) => "game_handoff_acceptance",
            Self::GameFork(_) => "game_fork",
            Self::BoardListing(_) => "board_listing",
        }
    }

    fn board_ref(&self) -> Option<&ArtifactHash> {
        match self {
            Self::Board(_) | Self::BoardFork(_) | Self::Theme(_) => None,
            Self::Rules(value) => Some(&value.board),
            Self::Forum(value) => Some(&value.board),
            Self::Thread(value) => Some(&value.board),
            Self::Post(value) => Some(&value.board),
            Self::Attachment(value) => Some(&value.board),
            Self::Membership(value) => Some(&value.board),
            Self::RoleAttestation(value) => Some(&value.board),
            Self::ModerationAction(value) => Some(&value.board),
            Self::Supersede(value) => Some(&value.board),
            Self::Tombstone(value) => Some(&value.board),
            Self::BoardListing(value) => Some(&value.board),
            Self::GamePost(value) => Some(&value.board),
            Self::GameChallengeAcceptance(value) => Some(&value.board),
            Self::GameTurn(value) => Some(&value.board),
            Self::GameSavePoint(value) => Some(&value.board),
            Self::GameHandoff(value) => Some(&value.board),
            Self::GameHandoffAcceptance(value) => Some(&value.board),
            Self::GameFork(value) => Some(&value.board),
        }
    }

    fn validate_shape(&self) -> Result<()> {
        match self {
            Self::Board(value) => validate_board_id(&value.board_id)?,
            Self::Rules(value) => {
                ensure!(value.revision > 0, "rules revision numbers start at 1");
                validate_text("rules text", &value.text, false)?;
            }
            Self::Forum(value) => validate_text("forum name", &value.name, false)?,
            Self::Thread(value) => validate_text("thread title", &value.title, false)?,
            Self::Post(value) => {
                validate_text("post body", &value.body, true)?;
                ensure!(
                    value.attachments.len() <= MAX_BOARD_REFERENCES,
                    "a post names at most {MAX_BOARD_REFERENCES} attachments"
                );
                let unique: BTreeSet<_> = value.attachments.iter().collect();
                ensure!(
                    unique.len() == value.attachments.len(),
                    "duplicate attachment reference"
                );
            }
            Self::Attachment(value) => {
                validate_text("attachment media type", &value.media_type, false)?;
                ensure!(
                    value.media_type.len() <= 128,
                    "media type is at most 128 bytes"
                );
            }
            Self::Membership(value) => {
                ensure!(
                    is_public_key(&value.member),
                    "membership key is not canonical Ed25519 hex"
                );
                if let Some(name) = &value.display_name {
                    validate_text("display name", name, false)?;
                }
            }
            Self::RoleAttestation(value) => {
                ensure!(
                    is_public_key(&value.member),
                    "role subject is not canonical Ed25519 hex"
                );
            }
            Self::ModerationAction(value) => {
                if let Some(reason) = &value.reason {
                    validate_text("moderation reason", reason, true)?;
                }
            }
            Self::BoardFork(value) => {
                ensure!(
                    value.source_frontier.len() <= MAX_BOARD_REFERENCES,
                    "a fork cites at most {MAX_BOARD_REFERENCES} source objects"
                );
                let unique: BTreeSet<_> = value.source_frontier.iter().collect();
                ensure!(
                    unique.len() == value.source_frontier.len(),
                    "duplicate source frontier reference"
                );
            }
            Self::Theme(value) => {
                validate_text("theme name", &value.name, false)?;
                ensure!(
                    value.tokens.len() <= MAX_THEME_TOKENS,
                    "a theme names at most {MAX_THEME_TOKENS} tokens"
                );
                for (name, token) in &value.tokens {
                    validate_token_name(name)?;
                    ensure!(
                        !token.is_empty()
                            && token.len() <= MAX_THEME_TOKEN_VALUE_BYTES
                            && !token.contains('\0'),
                        "theme token value must be 1 to {MAX_THEME_TOKEN_VALUE_BYTES} bytes without NUL"
                    );
                }
                ensure!(
                    value.assets.len() <= MAX_BOARD_REFERENCES,
                    "a theme names at most {MAX_BOARD_REFERENCES} assets"
                );
                let unique: BTreeSet<_> = value.assets.iter().collect();
                ensure!(
                    unique.len() == value.assets.len(),
                    "duplicate theme asset reference"
                );
            }
            Self::GamePost(value) => {
                validate_text("game package version", &value.package.version, false)?;
                ensure!(
                    value.package.version.len() <= 128,
                    "game package version is at most 128 bytes"
                );
                validate_game_participants("game post", &value.participants)?;
                validate_game_data("game configuration", &value.configuration)?;
                validate_game_data("game genesis state", &value.genesis)?;
            }
            Self::GameChallengeAcceptance(_) => {}
            Self::GameSavePoint(value) => {
                validate_text("game package version", &value.package.version, false)?;
                ensure!(
                    value.package.version.len() <= 128,
                    "game package version is at most 128 bytes"
                );
            }
            Self::GameHandoff(value) => {
                ensure!(
                    is_public_key(&value.recipient),
                    "game handoff recipient is not a canonical Ed25519 key"
                );
                validate_game_data("game handoff terms", &value.terms)?;
            }
            Self::GameHandoffAcceptance(_) => {}
            Self::GameFork(value) => {
                validate_game_participants("game fork", &value.participants)?;
            }
            Self::GameTurn(value) => {
                validate_game_role(&value.role)?;
                validate_game_data("game turn action", &value.action)?;
            }
            Self::BoardListing(value) => {
                ensure!(
                    is_public_key(&value.host),
                    "listing host is not canonical Ed25519 hex"
                );
                ensure!(value.version > 0, "listing version numbers start at 1");
                if let Some(contact) = &value.contact {
                    validate_text("listing contact", contact, false)?;
                }
            }
            Self::Supersede(value) => ensure!(
                value.previous != value.replacement,
                "an object cannot supersede itself"
            ),
            Self::Tombstone(value) => {
                if let Some(reason) = &value.reason {
                    validate_text("tombstone reason", reason, true)?;
                }
            }
        }
        Ok(())
    }
}

/// A typed board object carried and signed by the existing artifact protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedBoardObject {
    /// Existing signed artifact envelope.
    pub artifact: SignedArtifact,
    /// Strictly decoded, typed board object.
    pub object: BoardObject,
}

impl SignedBoardObject {
    /// Sign a board object using the existing per-author artifact chain.
    pub fn sign(key: &SigningKey, seq: u64, prev: &str, object: BoardObject) -> Result<Self> {
        object.validate_shape()?;
        let body = serde_json::to_value(&object)?;
        let artifact = SignedArtifact::sign(key, seq, prev, BOARD_OBJECT_KIND, body)?;
        Self::from_artifact(artifact)
    }

    /// Verify and strictly decode an existing signed artifact as a board object.
    pub fn from_artifact(artifact: SignedArtifact) -> Result<Self> {
        ensure!(
            artifact.kind == BOARD_OBJECT_KIND,
            "artifact is not a v1 board object"
        );
        artifact.verify()?;
        let object: BoardObject = serde_json::from_value(artifact.body.clone())
            .context("board object body is malformed")?;
        object.validate_shape()?;
        ensure!(
            serde_json::to_value(&object)? == artifact.body,
            "board object body is not in its canonical typed shape"
        );
        Ok(Self { artifact, object })
    }

    /// Verify the signature and typed body again.
    pub fn verify(&self) -> Result<()> {
        let checked = Self::from_artifact(self.artifact.clone())?;
        ensure!(
            checked.object == self.object,
            "typed board object differs from signed body"
        );
        Ok(())
    }

    /// Return the SHA-256 content hash that names this immutable artifact.
    pub fn hash(&self) -> Result<ArtifactHash> {
        ArtifactHash::new(self.artifact.hash.clone())
    }
}

/// Verified set of board objects, enforcing references and board-scoped authority.
#[derive(Clone, Debug, Default)]
pub struct BoardGraph {
    objects: BTreeMap<ArtifactHash, SignedBoardObject>,
}

impl BoardGraph {
    /// Verify and add an object after its referenced graph facts are present.
    ///
    /// The graph keeps every signed version; it never overwrites prior history.
    pub fn insert(&mut self, object: SignedBoardObject) -> Result<ArtifactHash> {
        object.verify()?;
        let hash = object.hash()?;
        ensure!(
            !self.objects.contains_key(&hash),
            "board object is already present"
        );
        self.validate_links(&object)?;
        self.objects.insert(hash.clone(), object);
        Ok(hash)
    }

    /// Look up an already verified immutable object by its content hash.
    pub fn get(&self, hash: &ArtifactHash) -> Option<&SignedBoardObject> {
        self.objects.get(hash)
    }

    /// Number of immutable objects accepted into this graph.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the graph has no accepted objects.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Resolve rules for one board, returning the selected content hash and body.
    ///
    /// `Some(revision)` selects that exact revision; `None` selects current
    /// rules, meaning the highest revision present. Duplicate `(board,
    /// revision)` objects are legal signed history: within a revision, the
    /// greater content hash wins. The same tie-break selects current rules
    /// when multiple objects share its highest revision, independent of
    /// insertion order. Objects superseded through an active supersede
    /// record are not eligible — the active-records rule (#4114) — so
    /// resolution follows the same frontier as [`Self::compose_view`].
    /// A thread's `rules_ref` remains pinned to its exact
    /// referenced hash; this projection does not rewrite or reinterpret it.
    pub fn resolve_rules(
        &self,
        board: &ArtifactHash,
        revision: Option<u64>,
    ) -> Option<(&ArtifactHash, &Rules)> {
        let superseded = self.superseded_objects(board);
        self.objects
            .iter()
            .filter_map(|(hash, signed)| {
                let BoardObject::Rules(rules) = &signed.object else {
                    return None;
                };
                (rules.board == *board
                    && !superseded.contains(hash)
                    && revision.is_none_or(|revision| rules.revision == revision))
                .then_some((hash, rules))
            })
            .max_by(|(left_hash, left), (right_hash, right)| {
                left.revision
                    .cmp(&right.revision)
                    .then_with(|| left_hash.cmp(right_hash))
            })
    }

    /// Every object for `board` that an active supersede record has
    /// superseded. A supersede record can itself be superseded, so the
    /// active records are a frontier, resolved to a fixed point: a
    /// record stays active while no active record names it as its
    /// previous version. (Supersede cycles cannot exist — insertion
    /// rejects an edge that would close one — so this terminates
    /// with the true frontier.) Only active records contribute their
    /// edges to the superseded set. Shared by [`Self::compose_view`]
    /// and [`Self::resolve_rules`] so every projection follows the
    /// same active-records rule (#4114).
    fn superseded_objects(&self, board: &ArtifactHash) -> BTreeSet<ArtifactHash> {
        let supersede_records: Vec<(ArtifactHash, ArtifactHash)> = self
            .objects
            .iter()
            .filter_map(|(hash, signed)| match &signed.object {
                BoardObject::Supersede(value) if &value.board == board => {
                    Some((hash.clone(), value.previous.clone()))
                }
                _ => None,
            })
            .collect();
        let mut active_records: BTreeSet<ArtifactHash> = supersede_records
            .iter()
            .map(|(hash, _)| hash.clone())
            .collect();
        loop {
            let named: BTreeSet<ArtifactHash> = supersede_records
                .iter()
                .filter(|(hash, _)| active_records.contains(hash))
                .map(|(_, previous)| previous.clone())
                .collect();
            let next: BTreeSet<ArtifactHash> = active_records.difference(&named).cloned().collect();
            if next == active_records {
                break;
            }
            active_records = next;
        }
        supersede_records
            .iter()
            .filter(|(hash, _)| active_records.contains(hash))
            .map(|(_, previous)| previous.clone())
            .collect()
    }

    fn validate_links(&self, signed: &SignedBoardObject) -> Result<()> {
        let author = signed.artifact.author.as_str();
        match &signed.object {
            BoardObject::Board(board) => {
                if let Some(source) = &board.forked_from {
                    self.require_board(source)?;
                }
                if let Some(theme) = &board.theme_ref {
                    let theme_object = self
                        .objects
                        .get(theme)
                        .context("board theme reference is missing")?;
                    ensure!(
                        matches!(theme_object.object, BoardObject::Theme(_)),
                        "board theme reference is not a theme object"
                    );
                }
            }
            BoardObject::Rules(value) => {
                self.require_moderator(&value.board, author)?;
            }
            BoardObject::Forum(value) => {
                self.require_moderator(&value.board, author)?;
            }
            BoardObject::Thread(value) => {
                self.require_board(&value.board)?;
                if let Some(forum) = &value.forum_ref {
                    self.require_scoped(forum, &value.board, "forum")?;
                }
                if let Some(rules) = &value.rules_ref {
                    self.require_scoped(rules, &value.board, "rules")?;
                }
            }
            BoardObject::Post(value) => {
                self.require_board(&value.board)?;
                self.require_scoped(&value.thread, &value.board, "thread")?;
                if let Some(parent) = &value.parent_post {
                    let parent_object = self.require_scoped(parent, &value.board, "post")?;
                    let BoardObject::Post(parent_post) = &parent_object.object else {
                        unreachable!("require_scoped checked post kind")
                    };
                    ensure!(
                        parent_post.thread == value.thread,
                        "a reply's parent post must belong to the same thread"
                    );
                }
                for attachment in &value.attachments {
                    self.require_scoped(attachment, &value.board, "attachment")?;
                }
                if let Some(embed_ref) = &value.embed_ref {
                    self.require_scoped(&embed_ref.object, &value.board, "game_post")?;
                }
            }
            BoardObject::GamePost(value) => {
                self.require_board(&value.board)?;
            }
            BoardObject::GameChallengeAcceptance(value) => {
                let post = self.require_scoped(&value.game_post, &value.board, "game_post")?;
                let BoardObject::GamePost(post_body) = &post.object else {
                    unreachable!("require_scoped checked game post kind")
                };
                ensure!(
                    post.artifact.author != author,
                    "a game challenge cannot be accepted by the game post author"
                );
                ensure!(
                    post_body
                        .participants
                        .iter()
                        .any(|participant| participant.author == author),
                    "a game challenge can be accepted only by an assigned participant"
                );
            }
            BoardObject::GameTurn(value) => {
                let post = self.require_scoped(&value.game_post, &value.board, "game_post")?;
                let BoardObject::GamePost(post_body) = &post.object else {
                    unreachable!("require_scoped checked game post kind")
                };
                ensure!(
                    post_body.mode == GameMode::AsyncTurns,
                    "game turns require the async-turns play mode"
                );
                // Authorization: who may sign this turn. A base-line
                // turn is signed by the participant its index names
                // in the game post's rotation — or, on a line an
                // accepted handoff governs, by that handoff's
                // recipient under the seat role rotation assigns
                // (the recipient's leg: the acceptance is what
                // begins it, design #2054 section 4). A fork turn is
                // signed under the fork instance's own participants.
                match &value.fork_ref {
                    Some(fork_hash) => {
                        let fork_object =
                            self.require_scoped(fork_hash, &value.board, "game_fork")?;
                        let BoardObject::GameFork(fork_body) = &fork_object.object else {
                            unreachable!("require_scoped checked game fork kind")
                        };
                        ensure!(
                            fork_body.game_post == value.game_post,
                            "a game turn's fork belongs to a different game"
                        );
                        let save_object = self
                            .objects
                            .get(&fork_body.save_point)
                            .context("game fork save point reference is missing")?;
                        let BoardObject::GameSavePoint(save_body) = &save_object.object else {
                            bail!("game fork save point reference is not a save point object");
                        };
                        let frontier_object = self
                            .objects
                            .get(&save_body.frontier)
                            .context("game fork frontier reference is missing")?;
                        let BoardObject::GameTurn(frontier_body) = &frontier_object.object else {
                            bail!("game fork frontier reference is not a game turn");
                        };
                        let previous_hash = value.prev_turn.as_ref().context(
                            "a fork turn builds on the fork frontier or a turn of the same fork",
                        )?;
                        if previous_hash != &save_body.frontier {
                            let previous_object = self
                                .objects
                                .get(previous_hash)
                                .context("game turn's prior turn is missing")?;
                            let BoardObject::GameTurn(previous_body) = &previous_object.object
                            else {
                                bail!("game turn's prior turn is not a game turn");
                            };
                            ensure!(
                                previous_body.fork_ref.as_ref() == Some(fork_hash),
                                "a fork turn's prior turn belongs to a different fork instance"
                            );
                        }
                        ensure!(
                            value.turn_index > frontier_body.turn_index,
                            "a fork turn's index does not follow the fork frontier"
                        );
                        // Fork seating is phase-aligned to the
                        // absolute turn index: the move count inside
                        // the saved state is part of what the fork
                        // replays from, so a fork turn's seat is the
                        // one its absolute index names. Resetting
                        // the rotation at the frontier would
                        // attribute the saved state's next move to
                        // the wrong seat (Codex P1 on PR #8552).
                        let position = value.turn_index;
                        let acting = &fork_body.participants[usize::try_from(position)
                            .unwrap_or(usize::MAX)
                            % fork_body.participants.len()];
                        let rotation_authorized =
                            acting.author == author && acting.role == value.role;
                        // Handoffs govern fork instances too: the
                        // recipient of an accepted handoff of a
                        // fork-line save may sign fork turns under
                        // the scheduled seat's role, exactly as on
                        // the base line (Codex P2 on PR #8552).
                        let leg_authorized = !rotation_authorized
                            && acting.role == value.role
                            && fork_body
                                .participants
                                .iter()
                                .any(|participant| participant.author == author)
                            && self.leg_recipients(value).contains(author);
                        ensure!(
                            rotation_authorized || leg_authorized,
                            "a fork turn must be signed by the fork participant its fork-turn position names, under that participant's assigned role, or by an accepted handoff recipient under that seat role on the handed-off fork line"
                        );
                    }
                    None => {
                        let acting = &post_body.participants[usize::try_from(value.turn_index)
                            .unwrap_or(usize::MAX)
                            % post_body.participants.len()];
                        let rotation_authorized =
                            acting.author == author && acting.role == value.role;
                        let leg_authorized = !rotation_authorized
                            && acting.role == value.role
                            && post_body
                                .participants
                                .iter()
                                .any(|participant| participant.author == author)
                            && self.leg_recipients(value).contains(author);
                        ensure!(
                            rotation_authorized || leg_authorized,
                            "a game turn must be signed by the participant whose turn its index names, under that participant's assigned role, or by an accepted handoff recipient under that seat role on the handed-off line"
                        );
                    }
                }
                ensure!(
                    self.objects.values().any(|signed| matches!(
                        &signed.object,
                        BoardObject::GameChallengeAcceptance(acceptance)
                            if acceptance.game_post == value.game_post
                    )),
                    "game turns require a signed challenge acceptance first"
                );
                match &value.prev_turn {
                    None => ensure!(value.turn_index == 0, "an opening game turn has index 0"),
                    Some(previous) => {
                        let previous_object =
                            self.require_scoped(previous, &value.board, "game_turn")?;
                        let BoardObject::GameTurn(previous_body) = &previous_object.object else {
                            unreachable!("require_scoped checked game turn kind")
                        };
                        ensure!(
                            previous_body.game_post == value.game_post,
                            "a game turn's prior turn belongs to a different game"
                        );
                        // Instance continuity: a base-line turn
                        // builds only on base-line turns. Building
                        // on a fork turn without naming the fork
                        // would continue the fork instance's state
                        // while silently switching authorization
                        // back to the original roster; crossing
                        // instances takes the explicit fork path
                        // (the fork branch above validates fork
                        // turns' predecessors against their fork)
                        // (Codex P2 on PR #8552).
                        ensure!(
                            value.fork_ref.is_some() || previous_body.fork_ref.is_none(),
                            "a base-line game turn cannot build on a fork turn: instance transitions use an explicit game fork"
                        );
                        ensure!(
                            previous_body.turn_index.checked_add(1) == Some(value.turn_index),
                            "a game turn's index must follow its prior turn"
                        );
                    }
                }
            }
            BoardObject::GameSavePoint(value) => {
                let post = self.require_scoped(&value.game_post, &value.board, "game_post")?;
                let BoardObject::GamePost(post_body) = &post.object else {
                    unreachable!("require_scoped checked game post kind")
                };
                ensure!(
                    value.package == post_body.package,
                    "a game save point must name the game post's pinned package and version"
                );
                let frontier_object =
                    self.require_scoped(&value.frontier, &value.board, "game_turn")?;
                let BoardObject::GameTurn(frontier_body) = &frontier_object.object else {
                    unreachable!("require_scoped checked game turn kind")
                };
                ensure!(
                    frontier_body.game_post == value.game_post,
                    "a game save point's frontier turn belongs to a different game"
                );
                // A save on a fork line is authorized against the
                // fork's participants — the seating of the instance
                // being saved — so a fork's players can save (and
                // fork again) without an original participant's
                // intervention; a base-line save uses the post's
                // participants.
                let save_participants: &[GameParticipant] =
                    if let Some(fork_hash) = &frontier_body.fork_ref {
                        let Some(fork_object) = self.get(fork_hash) else {
                            bail!("a game save point's frontier fork is missing");
                        };
                        let BoardObject::GameFork(fork_body) = &fork_object.object else {
                            bail!("a game save point's frontier fork is not a game fork object");
                        };
                        ensure!(
                            fork_body.game_post == value.game_post,
                            "a game save point's frontier fork belongs to a different game"
                        );
                        &fork_body.participants
                    } else {
                        &post_body.participants
                    };
                ensure!(
                    save_participants
                        .iter()
                        .any(|participant| participant.author == author),
                    "a game save point can be signed only by an assigned participant"
                );
            }
            BoardObject::GameHandoff(value) => {
                let post = self.require_scoped(&value.game_post, &value.board, "game_post")?;
                let BoardObject::GamePost(post_body) = &post.object else {
                    unreachable!("require_scoped checked game post kind")
                };
                let save_object =
                    self.require_scoped(&value.save_point, &value.board, "game_save_point")?;
                let BoardObject::GameSavePoint(save_body) = &save_object.object else {
                    unreachable!("require_scoped checked game save point kind")
                };
                ensure!(
                    save_body.game_post == value.game_post,
                    "a game handoff's save point belongs to a different game"
                );
                ensure!(
                    save_object.artifact.author == author,
                    "a game handoff must be signed by the save point's author"
                );
                ensure!(
                    value.recipient != author,
                    "a game handoff cannot name its own author as recipient"
                );
                // The recipient must belong to the instance the
                // save point belongs to — the fork's participants
                // for a fork-line save, the post's for a base-line
                // save — mirroring save-point authorization: a
                // handoff into an instance the recipient cannot
                // play in is meaningless, and a fork's own players
                // must be reachable without the original game's
                // roster (Codex P2 on PR #8552).
                let frontier_object = self
                    .objects
                    .get(&save_body.frontier)
                    .context("game handoff save point frontier reference is missing")?;
                let BoardObject::GameTurn(frontier_body) = &frontier_object.object else {
                    bail!("game handoff save point frontier reference is not a game turn");
                };
                let instance_participants: &[GameParticipant] = if let Some(fork_hash) =
                    &frontier_body.fork_ref
                {
                    let Some(fork_object) = self.objects.get(fork_hash) else {
                        bail!("game handoff save point frontier fork is missing");
                    };
                    let BoardObject::GameFork(fork_body) = &fork_object.object else {
                        bail!("game handoff save point frontier fork is not a game fork object");
                    };
                    &fork_body.participants
                } else {
                    &post_body.participants
                };
                ensure!(
                    instance_participants
                        .iter()
                        .any(|participant| participant.author == value.recipient),
                    "a game handoff recipient must be an assigned participant of the line the save point belongs to"
                );
            }
            BoardObject::GameHandoffAcceptance(value) => {
                let handoff_object =
                    self.require_scoped(&value.handoff, &value.board, "game_handoff")?;
                let BoardObject::GameHandoff(handoff_body) = &handoff_object.object else {
                    unreachable!("require_scoped checked game handoff kind")
                };
                ensure!(
                    handoff_body.game_post == value.game_post,
                    "a game handoff acceptance's handoff belongs to a different game"
                );
                ensure!(
                    handoff_body.recipient == author,
                    "a game handoff can be accepted only by its named recipient"
                );
            }
            BoardObject::GameFork(value) => {
                let save_object =
                    self.require_scoped(&value.save_point, &value.board, "game_save_point")?;
                let BoardObject::GameSavePoint(save_body) = &save_object.object else {
                    unreachable!("require_scoped checked game save point kind")
                };
                ensure!(
                    save_body.game_post == value.game_post,
                    "a game fork's save point belongs to a different game"
                );
                // The fork continues the saved state, whose move
                // count already fixes which seat moves next: the
                // fork's assignments must preserve the saved
                // instance's seat layout — the same number of
                // seats, with the same role at each seat — or the
                // absolute-index seating would attribute saved-state
                // moves to the wrong role (a one-seat fork of a
                // two-seat game would let its only participant sign
                // every index while replay executes the moves under
                // alternating seats). Authors may change freely;
                // seats and their roles may not (Codex P1 on
                // PR #8552).
                let frontier_object = self
                    .objects
                    .get(&save_body.frontier)
                    .context("game fork frontier reference is missing")?;
                let BoardObject::GameTurn(frontier_body) = &frontier_object.object else {
                    bail!("game fork frontier reference is not a game turn");
                };
                let source_participants: &[GameParticipant] = if let Some(source_fork_hash) =
                    &frontier_body.fork_ref
                {
                    let source_fork_object = self
                        .objects
                        .get(source_fork_hash)
                        .context("game fork source fork reference is missing")?;
                    let BoardObject::GameFork(source_fork_body) = &source_fork_object.object else {
                        bail!("game fork source fork reference is not a game fork object");
                    };
                    &source_fork_body.participants
                } else {
                    let post = self.require_scoped(&value.game_post, &value.board, "game_post")?;
                    let BoardObject::GamePost(post_body) = &post.object else {
                        unreachable!("require_scoped checked game post kind")
                    };
                    &post_body.participants
                };
                ensure!(
                    value.participants.len() == source_participants.len()
                        && value
                            .participants
                            .iter()
                            .zip(source_participants.iter())
                            .all(|(fork_seat, source_seat)| fork_seat.role == source_seat.role),
                    "a game fork must preserve the saved instance's seat layout: the same number of seats with the same role at each seat (authors may change)"
                );
                ensure!(
                    value
                        .participants
                        .iter()
                        .any(|participant| participant.author == author),
                    "a game fork must assign its own author a role in the fork instance"
                );
            }
            BoardObject::Attachment(value) => {
                self.require_board(&value.board)?;
            }
            BoardObject::Membership(value) => {
                self.require_board(&value.board)?;
                ensure!(
                    value.member == author,
                    "membership must be signed by its subject"
                );
            }
            BoardObject::RoleAttestation(value) => {
                let board = self.require_board(&value.board)?;
                ensure!(
                    board.artifact.author == author,
                    "only the board creator may grant or revoke roles"
                );
                if let Some(previous) = &value.supersedes {
                    let prior = self.require_scoped(previous, &value.board, "role_attestation")?;
                    let BoardObject::RoleAttestation(prior) = &prior.object else {
                        unreachable!("require_scoped checked role attestation kind")
                    };
                    ensure!(
                        prior.member == value.member && prior.role == value.role,
                        "role attestation can supersede only the same subject and role"
                    );
                }
            }
            BoardObject::ModerationAction(value) => {
                self.require_moderator(&value.board, author)?;
                self.require_scoped(&value.target, &value.board, "")?;
            }
            BoardObject::BoardFork(value) => {
                let source = self.require_board(&value.source_board)?;
                let fork = self.require_board(&value.fork_board)?;
                let BoardObject::Board(fork_body) = &fork.object else {
                    unreachable!("require_board checked board kind")
                };
                ensure!(
                    fork_body.forked_from.as_ref() == Some(&value.source_board),
                    "fork root does not cite source board"
                );
                ensure!(
                    fork.artifact.author == author,
                    "fork record must be signed by the new board creator"
                );
                for reference in &value.source_frontier {
                    if reference == &value.source_board {
                        continue;
                    }
                    let item = self
                        .objects
                        .get(reference)
                        .context("fork frontier reference is missing")?;
                    ensure!(
                        item.object.board_ref() == Some(&value.source_board),
                        "fork frontier crosses board scope"
                    );
                }
                ensure!(
                    source.object.kind_name() == "board",
                    "fork source is not a board root"
                );
            }
            BoardObject::Supersede(value) => {
                let previous = self.require_scoped(&value.previous, &value.board, "")?;
                let replacement = self.require_scoped(&value.replacement, &value.board, "")?;
                ensure!(
                    previous.object.kind_name() == replacement.object.kind_name(),
                    "superseded objects must have the same type"
                );
                ensure!(
                    previous.artifact.author == author && replacement.artifact.author == author,
                    "a supersede record must be signed by the author of both versions"
                );
                ensure!(
                    previous.object.kind_name() != "board",
                    "a board identity root cannot be superseded"
                );
                // Reject an edge that would close a cycle: the previous
                // object must not be reachable from the replacement through
                // ANY existing supersede path. A version may have several
                // replacements, so traverse every successor, not just one.
                let mut stack = vec![value.replacement.clone()];
                let mut visited = BTreeSet::new();
                while let Some(current) = stack.pop() {
                    if !visited.insert(current.clone()) {
                        continue;
                    }
                    for next in self.supersede_replacements(&current) {
                        ensure!(
                            next != value.previous,
                            "supersede relationship would create a cycle"
                        );
                        stack.push(next);
                    }
                }
            }
            BoardObject::Theme(value) => {
                if let Some(source) = &value.forked_from {
                    ensure!(source != &signed.hash()?, "a theme cannot fork from itself");
                    let source_object = self
                        .objects
                        .get(source)
                        .context("theme fork lineage reference is missing")?;
                    ensure!(
                        matches!(source_object.object, BoardObject::Theme(_)),
                        "theme fork lineage reference is not a theme object"
                    );
                }
            }
            BoardObject::BoardListing(value) => {
                let root = self.require_board(&value.board)?;
                ensure!(
                    root.artifact.author == author,
                    "a board listing must be signed by the board creator"
                );
                if let Some(previous_hash) = &value.supersedes {
                    let previous =
                        self.require_scoped(previous_hash, &value.board, "board_listing")?;
                    let BoardObject::BoardListing(previous_listing) = &previous.object else {
                        unreachable!("require_scoped checked board_listing kind")
                    };
                    ensure!(
                        previous_listing.host == value.host,
                        "a listing can supersede only a listing for the same host"
                    );
                    ensure!(
                        previous.artifact.author == author,
                        "a listing can supersede only the same author's listing"
                    );
                    ensure!(
                        value.version == previous_listing.version + 1,
                        "listing version must advance by exactly one from the superseded listing"
                    );
                } else {
                    ensure!(
                        value.version == 1,
                        "the first listing for a host starts at version 1"
                    );
                    let duplicate_first = self.objects.values().any(|signed| {
                        matches!(&signed.object,
                            BoardObject::BoardListing(existing)
                                if existing.board == value.board
                                    && existing.host == value.host)
                    });
                    ensure!(
                        !duplicate_first,
                        "a first listing already exists for this host"
                    );
                }
            }
            BoardObject::Tombstone(value) => {
                let target = self.require_scoped(&value.target, &value.board, "")?;
                ensure!(
                    target.object.kind_name() != "board",
                    "a board identity root cannot be tombstoned"
                );
                ensure!(
                    target.artifact.author == author,
                    "a tombstone must be signed by the target author"
                );
            }
        }
        Ok(())
    }

    /// Every replacement named by an existing supersede record for
    /// `previous` (a version may have been superseded more than once).
    fn supersede_replacements(&self, previous: &ArtifactHash) -> Vec<ArtifactHash> {
        self.objects
            .values()
            .filter_map(|signed| match &signed.object {
                BoardObject::Supersede(value) if &value.previous == previous => {
                    Some(value.replacement.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn require_board(&self, reference: &ArtifactHash) -> Result<&SignedBoardObject> {
        let object = self
            .objects
            .get(reference)
            .context("board root reference is missing")?;
        ensure!(
            matches!(object.object, BoardObject::Board(_)),
            "reference is not a board root"
        );
        Ok(object)
    }

    /// The recipients whose accepted handoffs authorize legs on a
    /// prospective turn's chain, within the turn's own instance:
    /// the union of the named recipients of every accepted handoff
    /// whose save point's frontier lies on the turn's prior chain
    /// and belongs to the same instance (the base line, or the
    /// same fork) as the turn.
    ///
    /// The union — never only the deepest handoff — is what makes
    /// authorization monotone in the evidence: accepting a later
    /// handoff can add a recipient but can never revoke a leg an
    /// earlier accepted handoff began, so the same signed set
    /// converges to the same verdicts in any arrival order
    /// (Codex P1 on PR #8552). Empty when no accepted handoff
    /// touches the chain, in which case plain rotation authorizes
    /// the turn.
    fn leg_recipients(&self, turn: &GameTurn) -> BTreeSet<String> {
        // The chain's hashes, frontier-first.
        let mut chain = BTreeSet::new();
        let mut cursor = turn.prev_turn.clone();
        while let Some(hash) = cursor {
            let Some(previous) = self.objects.get(&hash) else {
                break;
            };
            let BoardObject::GameTurn(previous_body) = &previous.object else {
                break;
            };
            chain.insert(hash);
            cursor = previous_body.prev_turn.clone();
        }
        let mut recipients = BTreeSet::new();
        if chain.is_empty() {
            return recipients;
        }
        for (save_hash, save) in self.game_save_points(&turn.game_post) {
            if !chain.contains(&save.frontier) {
                continue;
            }
            // Instance scoping: only a handoff whose save frontier
            // belongs to the prospective turn's own instance
            // governs it. A fork's chain runs back through the
            // source line behind its frontier, but the fork is an
            // independent instance — a source-line handoff must
            // not override the fork's own assignments (Codex P2
            // on PR #8552). Base-line turns only ever chain
            // base-line frontiers, so this is a no-op for them.
            let Some(frontier_object) = self.objects.get(&save.frontier) else {
                continue;
            };
            let BoardObject::GameTurn(frontier_body) = &frontier_object.object else {
                continue;
            };
            if frontier_body.fork_ref != turn.fork_ref {
                continue;
            }
            for (handoff_hash, handoff) in self.game_handoffs(&turn.game_post) {
                if handoff.save_point == save_hash
                    && !self.game_handoff_acceptances(&handoff_hash).is_empty()
                {
                    recipients.insert(handoff.recipient.clone());
                }
            }
        }
        recipients
    }

    fn require_scoped(
        &self,
        reference: &ArtifactHash,
        board: &ArtifactHash,
        expected_kind: &str,
    ) -> Result<&SignedBoardObject> {
        let object = self
            .objects
            .get(reference)
            .context("object reference is missing")?;
        ensure!(
            expected_kind.is_empty() || object.object.kind_name() == expected_kind,
            "reference must name a {expected_kind} object"
        );
        ensure!(
            object.object.board_ref() == Some(board),
            "object reference crosses board scope"
        );
        Ok(object)
    }

    fn require_moderator(&self, board: &ArtifactHash, author: &str) -> Result<()> {
        let root = self.require_board(board)?;
        if root.artifact.author == author || self.has_role(board, author, BoardRole::Moderator) {
            return Ok(());
        }
        bail!("author has no moderator authority for this board")
    }

    /// Whether a grant of `role` to `member` exists in this graph.
    ///
    /// Acceptance is based on grant *existence*, not on the current role
    /// frontier. A signed object carries no causal pointer to the grant or
    /// revocation that orders it, so a frontier check makes acceptance
    /// depend on arrival order: the same signed set (grant, a moderator's
    /// forum, revocation) would reconstruct on one peer and fail on
    /// another that received the revocation first. With existence, every
    /// peer that holds the grant reaches the same verdict in any order.
    /// Revocations stay in the graph as signed history; the view/serving
    /// layers (which decide what is served *now*) enforce them — this
    /// substrate only records and validates objects.
    fn has_role(&self, board: &ArtifactHash, member: &str, role: BoardRole) -> bool {
        self.objects.values().any(|signed| match &signed.object {
            BoardObject::RoleAttestation(value) => {
                &value.board == board
                    && value.member == member
                    && value.role == role
                    && value.granted
            }
            _ => false,
        })
    }

    /// The signed challenge acceptances recorded for one game post, in
    /// content-hash order.
    pub fn game_acceptances(&self, game_post: &ArtifactHash) -> Vec<&SignedBoardObject> {
        self.objects
            .values()
            .filter(|signed| {
                matches!(
                    &signed.object,
                    BoardObject::GameChallengeAcceptance(acceptance)
                        if &acceptance.game_post == game_post
                )
            })
            .collect()
    }

    /// Every accepted turn of one game post, ordered by (turn index,
    /// content hash) so every peer enumerates the same set identically.
    /// Competing branches are all present; see [`Self::game_heads`].
    pub fn game_turns(&self, game_post: &ArtifactHash) -> Vec<(ArtifactHash, &GameTurn)> {
        let mut turns: Vec<(ArtifactHash, &GameTurn)> = self
            .objects
            .iter()
            .filter_map(|(hash, signed)| match &signed.object {
                BoardObject::GameTurn(turn) if &turn.game_post == game_post => {
                    Some((hash.clone(), turn))
                }
                _ => None,
            })
            .collect();
        turns.sort_by(|(left_hash, left), (right_hash, right)| {
            left.turn_index
                .cmp(&right.turn_index)
                .then_with(|| left_hash.cmp(right_hash))
        });
        turns
    }

    /// The heads of one game: accepted turns no other turn builds on.
    /// A game with more than one head has forked — competing valid turns
    /// on the same frontier stay visible as separate heads instead of
    /// one erasing the other.
    pub fn game_heads(&self, game_post: &ArtifactHash) -> Vec<ArtifactHash> {
        let turns = self.game_turns(game_post);
        let built_on: BTreeSet<&ArtifactHash> = turns
            .iter()
            .filter_map(|(_, turn)| turn.prev_turn.as_ref())
            .collect();
        turns
            .into_iter()
            .filter(|(hash, _)| !built_on.contains(hash))
            .map(|(hash, _)| hash)
            .collect()
    }

    /// The signed save points recorded for one game post, in
    /// content-hash order.
    pub fn game_save_points(
        &self,
        game_post: &ArtifactHash,
    ) -> Vec<(ArtifactHash, &GameSavePoint)> {
        self.objects
            .iter()
            .filter_map(|(hash, signed)| match &signed.object {
                BoardObject::GameSavePoint(save) if &save.game_post == game_post => {
                    Some((hash.clone(), save))
                }
                _ => None,
            })
            .collect()
    }

    /// The signed handoffs recorded for one game post, in
    /// content-hash order. Competing handoffs of one save point are
    /// all present; see [`Self::game_main_line_head`] for how an
    /// accepted one is selected.
    pub fn game_handoffs(&self, game_post: &ArtifactHash) -> Vec<(ArtifactHash, &GameHandoff)> {
        self.objects
            .iter()
            .filter_map(|(hash, signed)| match &signed.object {
                BoardObject::GameHandoff(handoff) if &handoff.game_post == game_post => {
                    Some((hash.clone(), handoff))
                }
                _ => None,
            })
            .collect()
    }

    /// The signed acceptances recorded for one handoff, in
    /// content-hash order.
    pub fn game_handoff_acceptances(&self, handoff: &ArtifactHash) -> Vec<&SignedBoardObject> {
        self.objects
            .values()
            .filter(|signed| {
                matches!(
                    &signed.object,
                    BoardObject::GameHandoffAcceptance(acceptance)
                        if &acceptance.handoff == handoff
                )
            })
            .collect()
    }

    /// The signed game forks recorded for one game post, in
    /// content-hash order. Concurrent forks citing the same save
    /// point are all present.
    pub fn game_forks(&self, game_post: &ArtifactHash) -> Vec<(ArtifactHash, &GameFork)> {
        self.objects
            .iter()
            .filter_map(|(hash, signed)| match &signed.object {
                BoardObject::GameFork(fork) if &fork.game_post == game_post => {
                    Some((hash.clone(), fork))
                }
                _ => None,
            })
            .collect()
    }

    /// The frontier of the social main line of one game: the
    /// frontier turn of the furthest-along save point that an
    /// accepted handoff cites.
    ///
    /// Handoffs are how a line changes hands socially: the saver
    /// names a recipient, the recipient accepts, and that accepted
    /// chain — not arrival order — selects the main line. Among
    /// accepted save points the one whose frontier has the highest
    /// turn index wins; ties break to the greater frontier hash,
    /// the same tie-break current-rules selection documents.
    /// Unaccepted handoffs and their save points stay in the graph
    /// as alternatives; this query only reads. `None` when no
    /// handoff of this game has been accepted.
    pub fn game_main_line_head(&self, game_post: &ArtifactHash) -> Option<ArtifactHash> {
        let mut best: Option<(u64, ArtifactHash)> = None;
        for (save_hash, save) in self.game_save_points(game_post) {
            let accepted = self
                .game_handoffs(game_post)
                .iter()
                .any(|(handoff_hash, handoff)| {
                    handoff.save_point == save_hash
                        && !self.game_handoff_acceptances(handoff_hash).is_empty()
                });
            if !accepted {
                continue;
            }
            let Some(frontier_object) = self.objects.get(&save.frontier) else {
                continue;
            };
            let BoardObject::GameTurn(frontier_body) = &frontier_object.object else {
                continue;
            };
            let candidate = (frontier_body.turn_index, save.frontier.clone());
            if best.as_ref().map_or(true, |current| candidate > *current) {
                best = Some(candidate);
            }
        }
        best.map(|(_, frontier)| frontier)
    }

    /// One lineage of a game, from its opening turn to `head`, in play
    /// order. Graph acceptance already binds each turn to its prior
    /// turn, so a lineage is a walk of `prev_turn` references; this
    /// re-checks game scope and index contiguity so the slice handed
    /// to replay is self-verified.
    pub fn game_lineage(
        &self,
        game_post: &ArtifactHash,
        head: &ArtifactHash,
    ) -> Result<Vec<(ArtifactHash, &GameTurn)>> {
        let mut lineage = Vec::new();
        let mut cursor = Some(head.clone());
        while let Some(hash) = cursor {
            let signed = self
                .objects
                .get(&hash)
                .context("game lineage reference is missing")?;
            let BoardObject::GameTurn(turn) = &signed.object else {
                bail!("game lineage reference is not a game turn");
            };
            ensure!(
                &turn.game_post == game_post,
                "game lineage crosses into a different game"
            );
            cursor = turn.prev_turn.clone();
            lineage.push((hash, turn));
        }
        lineage.reverse();
        for (expected, (_, turn)) in lineage.iter().enumerate() {
            ensure!(
                usize::try_from(turn.turn_index) == Ok(expected),
                "game lineage indices are not contiguous from 0"
            );
        }
        Ok(lineage)
    }

    /// Deterministically replay one lineage of a game through the exact
    /// package bytes its game post pins, under the sandbox limits.
    ///
    /// The bytes are accepted only when their SHA-256 equals the pinned
    /// package hash — a different build is a different game. The fold
    /// starts from the post's genesis state and applies each turn's
    /// action in lineage order, attributing it to the participant the
    /// game post assigns to the turn's role (participant IDs are the
    /// one-based positions in the post's participant list, as in the
    /// tic-tac-toe package). Player evidence comes only from the signed
    /// turns; nothing is inferred from host-authored journal events.
    /// Returns the package's state snapshot after the final turn: the
    /// same signed lineage and the same bytes always produce the same
    /// snapshot.
    ///
    /// Transition validity (Codex P1 on PR #8327): every turn must
    /// actually transition the package state. A package may ignore an
    /// action it considers invalid (an occupied cell, a move after
    /// the game is won, an out-of-range cell in the pinned tic-tac-toe
    /// package) and return its model unchanged; counting such a call
    /// as a consumed turn would desynchronize the signed turn order
    /// from the executed move order — the next participant's action
    /// would execute as the previous player's move. A lineage whose
    /// turn leaves the state snapshot unchanged is therefore refused:
    /// signed roles can never drift from executed moves.
    ///
    /// Genesis alignment (Codex P2 on PR #8327): the signed-turn
    /// schedule is defined from the package's initial state — graph
    /// validation binds turn index 0 to the first participant. A
    /// genesis that restores to any other state is a mid-game state,
    /// and which participant it hands the move to is package-internal
    /// (the pinned tic-tac-toe package derives it from the restored
    /// mark count) with no generic read through the sandbox
    /// interface. Replay therefore refuses a genesis that does not
    /// restore to the package's initial state rather than execute a
    /// signed turn under a possibly wrong participant.
    #[cfg(feature = "sandbox")]
    pub fn replay_game_lineage(
        &self,
        game_post: &ArtifactHash,
        head: &ArtifactHash,
        package_bytes: &[u8],
    ) -> Result<Vec<u8>> {
        let post = self
            .objects
            .get(game_post)
            .context("game post reference is missing")?;
        let BoardObject::GamePost(post_body) = &post.object else {
            bail!("reference is not a game post");
        };
        ensure!(
            sha256_hex(package_bytes) == post_body.package.hash.as_str(),
            "package bytes do not match the game post's pinned package hash"
        );
        ensure!(
            !self.game_acceptances(game_post).is_empty(),
            "the game has no signed challenge acceptance"
        );
        let lineage = self.game_lineage(game_post, head)?;
        let sandbox = crate::sandbox::Sandbox::compile(
            package_bytes,
            crate::sandbox::SandboxLimits::default(),
        )?;
        // The package's own initial state: the one state the
        // signed-turn schedule (participants by turn index, from 0)
        // is defined from.
        let mut fresh_instance = sandbox.instantiate(0)?;
        let initial_snapshot = fresh_instance.snapshot()?;
        let mut instance = sandbox.instantiate(0)?;
        let genesis_bytes = match &post_body.genesis {
            serde_json::Value::String(text) => text.clone().into_bytes(),
            other => serde_json::to_vec(other)?,
        };
        if !genesis_bytes.is_empty() {
            instance.restore(&genesis_bytes)?;
        }
        let mut previous_snapshot = instance.snapshot()?;
        if !genesis_bytes.is_empty() {
            if let (Some(initial), Some(restored)) = (&initial_snapshot, &previous_snapshot) {
                ensure!(
                    initial == restored,
                    "game genesis state is not the package's initial state: \
                     replay cannot align the signed turn schedule with a restored mid-game state"
                );
            }
        }
        for (hash, turn) in &lineage {
            let signed = self
                .objects
                .get(hash)
                .context("game lineage reference is missing")?;
            let fork_participants;
            let participants: &[GameParticipant] = match &turn.fork_ref {
                Some(fork_hash) => {
                    let fork_object = self
                        .objects
                        .get(fork_hash)
                        .context("game fork reference is missing")?;
                    let BoardObject::GameFork(fork_body) = &fork_object.object else {
                        bail!("game fork reference is not a game fork object");
                    };
                    fork_participants = fork_body.participants.clone();
                    &fork_participants
                }
                None => &post_body.participants,
            };
            // The participant ID the package executes under. For a
            // handoff leg turn — on the base line or inside a fork
            // instance — this is the scheduled seat
            // (the seat the turn's index names in rotation): the
            // recipient substitutes into that seat, so resolving by
            // the recipient's own assignment — or by first role
            // match, ambiguous when participants share a role —
            // would execute the move under the wrong player
            // (Codex P2 on PR #8552). Ordinary turns resolve to the
            // same seat through their signer, so the leg lookup
            // only runs when the direct resolution is not the seat.
            let seat = usize::try_from(turn.turn_index).unwrap_or(usize::MAX) % participants.len();
            let direct = participants.iter().position(|participant| {
                participant.author == signed.artifact.author && participant.role == turn.role
            });
            let position = match direct {
                Some(found) if found == seat => found,
                _ if participants[seat].role == turn.role
                    && self.leg_recipients(turn).contains(&signed.artifact.author) =>
                {
                    seat
                }
                Some(found) => found,
                None if turn.fork_ref.is_some() => {
                    bail!("game turn signer is not assigned in the fork instance");
                }
                None => {
                    bail!("game turn signer is not assigned in the game post");
                }
            };
            let participant = u64::try_from(position + 1)?;
            let action_bytes = match &turn.action {
                serde_json::Value::String(text) => text.clone().into_bytes(),
                other => serde_json::to_vec(other)?,
            };
            instance.update(participant, &action_bytes)?;
            let current_snapshot = instance.snapshot()?;
            if let (Some(before), Some(after)) = (&previous_snapshot, &current_snapshot) {
                ensure!(
                    before != after,
                    "game turn {} was ignored by the package (state unchanged):                      replayed roles would drift from executed moves",
                    turn.turn_index
                );
            }
            previous_snapshot = current_snapshot;
        }
        previous_snapshot.context("game package exposes no state snapshot")
    }

    /// Every hash in one board's replication set: the board root plus
    /// every object scoped to it, plus the fork provenance record when
    /// this board is a fork (the [`BoardFork`] record names no board in
    /// `board`, only `source_board`/`fork_board`, so scope alone would
    /// miss it and fork provenance would never replicate), plus the
    /// dependency closure a fresh graph needs before
    /// [`Self::validate_links`] will accept those objects — the root's
    /// `theme_ref` theme and `forked_from` ancestor root, recursively
    /// (an ancestor's own theme and fork lineage, a theme's fork
    /// lineage, and a fork record's source root, fork root, and cited
    /// source frontier). Themes, ancestor roots, and fork records name
    /// no board (`board_ref` is `None`), so a scope-only filter would
    /// export a set that can never be accepted elsewhere:
    /// themed and forked boards would stall replication forever
    /// (Codex P1 on PR #4048). This set — not a server-side copy — is
    /// what hosts replicate and converge on.
    pub fn board_object_set(&self, board: &ArtifactHash) -> BTreeSet<ArtifactHash> {
        let mut set: BTreeSet<ArtifactHash> = self
            .objects
            .iter()
            .filter(|(hash, signed)| {
                *hash == board
                    || signed.object.board_ref() == Some(board)
                    || matches!(&signed.object, BoardObject::BoardFork(fork) if &fork.fork_board == board)
            })
            .map(|(hash, _)| hash.clone())
            .collect();
        let mut frontier: Vec<ArtifactHash> = set.iter().cloned().collect();
        while let Some(hash) = frontier.pop() {
            let Some(signed) = self.objects.get(&hash) else {
                continue;
            };
            // The closure follows the acceptance dependencies of every
            // object kind -- exactly the references validate_links
            // requires to already be present -- not just roots, themes,
            // and fork records. A frontier-cited source object (a post,
            // say) is scoped to the *source* board, so the seed never
            // contains its thread, that thread's forum/rules, its parent
            // post, or its attachments; without traversing those, the
            // export can never be accepted by a fresh graph and
            // replication stalls (Codex P1 on PR #4545). Moderator-gated
            // kinds (rules, forums, moderation actions) additionally
            // depend on the grant that gives their author authority.
            let author = signed.artifact.author.clone();
            let moderator_grants = |board: &ArtifactHash| -> Vec<ArtifactHash> {
                self.objects
                    .iter()
                    .filter_map(|(grant_hash, grant)| match &grant.object {
                        BoardObject::RoleAttestation(value)
                            if &value.board == board
                                && value.member == author
                                && value.role == BoardRole::Moderator
                                && value.granted =>
                        {
                            Some(grant_hash.clone())
                        }
                        _ => None,
                    })
                    .collect()
            };
            let dependencies: Vec<ArtifactHash> = match &signed.object {
                BoardObject::Board(body) => body
                    .forked_from
                    .iter()
                    .chain(body.theme_ref.iter())
                    .cloned()
                    .collect(),
                BoardObject::Theme(body) => body.forked_from.iter().cloned().collect(),
                BoardObject::BoardFork(body) => body
                    .source_frontier
                    .iter()
                    .chain([&body.source_board, &body.fork_board])
                    .cloned()
                    .collect(),
                BoardObject::Rules(body) => std::iter::once(body.board.clone())
                    .chain(moderator_grants(&body.board))
                    .collect(),
                BoardObject::Forum(body) => std::iter::once(body.board.clone())
                    .chain(moderator_grants(&body.board))
                    .collect(),
                BoardObject::Thread(body) => std::iter::once(body.board.clone())
                    .chain(body.forum_ref.iter().cloned())
                    .chain(body.rules_ref.iter().cloned())
                    .collect(),
                BoardObject::Post(body) => std::iter::once(body.board.clone())
                    .chain(std::iter::once(body.thread.clone()))
                    .chain(body.parent_post.iter().cloned())
                    .chain(body.attachments.iter().cloned())
                    .chain(body.embed_ref.iter().map(|embed| embed.object.clone()))
                    .collect(),
                BoardObject::Attachment(body) => vec![body.board.clone()],
                BoardObject::Membership(body) => vec![body.board.clone()],
                BoardObject::RoleAttestation(body) => std::iter::once(body.board.clone())
                    .chain(body.supersedes.iter().cloned())
                    .collect(),
                BoardObject::ModerationAction(body) => std::iter::once(body.board.clone())
                    .chain(std::iter::once(body.target.clone()))
                    .chain(moderator_grants(&body.board))
                    .collect(),
                BoardObject::Supersede(body) => vec![
                    body.board.clone(),
                    body.previous.clone(),
                    body.replacement.clone(),
                ],
                BoardObject::Tombstone(body) => {
                    vec![body.board.clone(), body.target.clone()]
                }
                BoardObject::BoardListing(body) => std::iter::once(body.board.clone())
                    .chain(body.supersedes.iter().cloned())
                    .collect(),
                BoardObject::GamePost(body) => vec![body.board.clone()],
                BoardObject::GameChallengeAcceptance(body) => {
                    vec![body.board.clone(), body.game_post.clone()]
                }
                BoardObject::GameSavePoint(body) => vec![
                    body.board.clone(),
                    body.game_post.clone(),
                    body.frontier.clone(),
                ],
                BoardObject::GameHandoff(body) => vec![
                    body.board.clone(),
                    body.game_post.clone(),
                    body.save_point.clone(),
                ],
                BoardObject::GameHandoffAcceptance(body) => vec![
                    body.board.clone(),
                    body.game_post.clone(),
                    body.handoff.clone(),
                ],
                BoardObject::GameFork(body) => vec![
                    body.board.clone(),
                    body.game_post.clone(),
                    body.save_point.clone(),
                ],
                BoardObject::GameTurn(body) => {
                    let mut dependencies = vec![body.board.clone(), body.game_post.clone()];
                    dependencies.extend(body.prev_turn.iter().cloned());
                    dependencies.extend(body.fork_ref.iter().cloned());
                    dependencies.extend(self.objects.iter().filter_map(|(hash, signed)| {
                        match &signed.object {
                            BoardObject::GameChallengeAcceptance(acceptance)
                                if acceptance.game_post == body.game_post =>
                            {
                                Some(hash.clone())
                            }
                            _ => None,
                        }
                    }));
                    // A turn on a handed-off line is authorized by
                    // the game's save points, handoffs, and handoff
                    // acceptances; without them a replica holding
                    // the turn could not re-verify it (Codex P2 on
                    // PR #8552). They travel with every turn of the
                    // game, like its challenge acceptances.
                    dependencies.extend(self.objects.iter().filter_map(|(hash, signed)| {
                        match &signed.object {
                            BoardObject::GameSavePoint(save)
                                if save.game_post == body.game_post =>
                            {
                                Some(hash.clone())
                            }
                            BoardObject::GameHandoff(handoff)
                                if handoff.game_post == body.game_post =>
                            {
                                Some(hash.clone())
                            }
                            BoardObject::GameHandoffAcceptance(acceptance)
                                if acceptance.game_post == body.game_post =>
                            {
                                Some(hash.clone())
                            }
                            _ => None,
                        }
                    }));
                    dependencies
                }
            };
            for dependency in dependencies {
                if self.objects.contains_key(&dependency) && set.insert(dependency.clone()) {
                    frontier.push(dependency.clone());
                }
            }
        }
        set
    }

    /// The board's replication set as signed objects, in content-hash
    /// order. Any host can serve this export; accepting it needs only
    /// the graph's own signature and scope checks.
    pub fn export_board(&self, board: &ArtifactHash) -> Result<Vec<SignedBoardObject>> {
        self.require_board(board)?;
        Ok(self
            .board_object_set(board)
            .iter()
            .filter_map(|hash| self.objects.get(hash))
            .cloned()
            .collect())
    }

    /// Replicate one board's signed object set from another host's
    /// graph into this one. Objects already present are skipped, and
    /// the rest are accepted in dependency order by repeated passes, so
    /// the two hosts converge on the identical signed set regardless of
    /// arrival order and with no primary-host authority: every object
    /// still passes this graph's signature, scope, and authority checks.
    pub fn replicate_board_from(
        &mut self,
        source: &BoardGraph,
        board: &ArtifactHash,
    ) -> Result<ReplicationReport> {
        let exported = source.export_board(board)?;
        let already_present = exported
            .iter()
            .filter(|object| {
                object
                    .hash()
                    .map(|hash| self.objects.contains_key(&hash))
                    .unwrap_or(false)
            })
            .count();
        let mut pending: Vec<SignedBoardObject> = exported
            .into_iter()
            .filter(|object| {
                object
                    .hash()
                    .map(|hash| !self.objects.contains_key(&hash))
                    .unwrap_or(false)
            })
            .collect();
        let mut transferred = Vec::new();
        while !pending.is_empty() {
            let mut progressed = false;
            let mut remaining = Vec::new();
            for object in pending {
                match self.insert(object.clone()) {
                    Ok(hash) => {
                        transferred.push(hash);
                        progressed = true;
                    }
                    Err(_) => remaining.push(object),
                }
            }
            pending = remaining;
            ensure!(
                progressed,
                "board replication stalled: {} signed objects could not be accepted (a referenced object is missing from the source set or fails verification)",
                pending.len()
            );
        }
        Ok(ReplicationReport {
            board: board.clone(),
            transferred,
            already_present,
            status: self.replica_status_against(board, source),
        })
    }

    /// How this graph's replica of `board` compares to a source graph.
    /// A replica is [`ReplicaState::Complete`] only while it holds the
    /// source's whole board set; a replica behind on a signed listing
    /// version, or missing objects, is reported explicitly as
    /// [`ReplicaState::Stale`] or [`ReplicaState::Incomplete`] — a
    /// lagging replica is never silently presented as current, and a
    /// newer signed object is never overwritten by an older replica
    /// (the graph is append-only and keeps every signed version).
    pub fn replica_status_against(
        &self,
        board: &ArtifactHash,
        source: &BoardGraph,
    ) -> ReplicaStatus {
        let expected = source.board_object_set(board);
        let mut missing: Vec<ArtifactHash> = expected
            .difference(&self.objects.keys().cloned().collect::<BTreeSet<_>>())
            .cloned()
            .collect();
        if expected.is_empty() {
            missing = vec![board.clone()];
        }
        let local_listings = self.latest_listings(board);
        let mut listing_version_lag = BTreeMap::new();
        for (host, (_, source_listing)) in source.latest_listings(board) {
            let local_version = local_listings
                .get(&host)
                .map(|(_, listing)| listing.version)
                .unwrap_or(0);
            if source_listing.version > local_version {
                listing_version_lag.insert(host, (local_version, source_listing.version));
            }
        }
        let present_objects = expected.len().saturating_sub(missing.len());
        let state = if missing.is_empty() {
            ReplicaState::Complete
        } else if !listing_version_lag.is_empty() {
            ReplicaState::Stale
        } else {
            ReplicaState::Incomplete
        };
        ReplicaStatus {
            board: board.clone(),
            state,
            expected_objects: expected.len(),
            present_objects,
            missing,
            listing_version_lag,
        }
    }

    /// The newest signed listing per host for `board`. Versions are the
    /// signed per-host sequence; when two listings share a version (a
    /// signed fork), the greater content hash wins — a deterministic
    /// rule every peer computes identically, never a host clock.
    pub fn latest_listings(
        &self,
        board: &ArtifactHash,
    ) -> BTreeMap<String, (ArtifactHash, BoardListing)> {
        let mut latest: BTreeMap<String, (ArtifactHash, BoardListing)> = BTreeMap::new();
        for (hash, signed) in &self.objects {
            let BoardObject::BoardListing(listing) = &signed.object else {
                continue;
            };
            if &listing.board != board {
                continue;
            }
            let replace = match latest.get(&listing.host) {
                None => true,
                Some((current_hash, current)) => {
                    (listing.version, hash) > (current.version, current_hash)
                }
            };
            if replace {
                latest.insert(listing.host.clone(), (hash.clone(), listing.clone()));
            }
        }
        latest
    }

    /// Hosts whose newest signed listing for `board` is a publication.
    /// A withdrawal is a newer listing with `listed == false`, so
    /// delisting takes effect by signed version, not by host assertion.
    pub fn active_listings(
        &self,
        board: &ArtifactHash,
    ) -> BTreeMap<String, (ArtifactHash, BoardListing)> {
        self.latest_listings(board)
            .into_iter()
            .filter(|(_, (_, listing))| listing.listed)
            .collect()
    }

    /// Compose the three-layer view of one board: the board layer
    /// (signed moderation log + author tombstones + the board's theme),
    /// the host layer (hashes the serving host will not serve), and the
    /// personal layer (one viewer's own hides and muted authors).
    ///
    /// Transparency rule — hidden is not erased: a hidden object stays
    /// in the graph, is omitted from [`BoardView::objects`], and is
    /// reported in [`BoardView::hidden`] with the layer that hid it and,
    /// for board-layer hides, the moderation action (or tombstone) that
    /// did it. Every moderation action for the board is listed in
    /// [`BoardView::moderation_log`] whether or not it currently hides
    /// anything: the moderation log is public.
    ///
    /// Board-layer hide state uses the same order-independent existence
    /// philosophy as role grants: a target is hidden while its signed
    /// `Hide` records outnumber its signed `Restore` records, so every
    /// peer holding the same log composes the same view in any arrival
    /// order. When several layers hide one object, the reported layer
    /// is the outermost applicable one: board, then host, then personal.
    /// Superseded versions are not hidden and not shown — a projection
    /// shows the replacement; the history stays in the graph.
    ///
    /// Two enforcement duties the substrate deliberately delegates to
    /// this layer (see [`BoardGraph::has_role`]) are discharged here:
    /// a moderation record has an effect only while its author holds
    /// authority — the board owner, or a moderator whose grant is on
    /// the current attestation frontier (a grant superseded by a
    /// revocation no longer empowers; the frontier follows the signed
    /// `supersedes` links, not arrival order) — and a superseded
    /// moderation or tombstone record has no effect. Revoked or
    /// superseded records stay in the public moderation log either
    /// way. The board's theme is itself subject to the host and
    /// personal layers: a suppressed theme is not returned, and the
    /// suppression is reported in `hidden` like any other.
    pub fn compose_view(&self, board: &ArtifactHash, layers: &ViewLayers) -> Result<BoardView> {
        let root = self.require_board(board)?;
        let BoardObject::Board(root_body) = &root.object else {
            unreachable!("require_board checked board kind")
        };
        let mut theme_hidden = None;
        let theme = match &root_body.theme_ref {
            Some(reference) => {
                let theme_object = self
                    .objects
                    .get(reference)
                    .context("board theme reference is missing")?;
                let BoardObject::Theme(theme) = &theme_object.object else {
                    bail!("board theme reference is not a theme object")
                };
                if layers.host_hidden.contains(reference) {
                    theme_hidden = Some(HiddenObject {
                        object: reference.clone(),
                        layer: ViewLayer::Host,
                        action: None,
                        reason: None,
                    });
                    None
                } else if layers.personal_hidden.contains(reference)
                    || layers.muted_authors.contains(&theme_object.artifact.author)
                {
                    theme_hidden = Some(HiddenObject {
                        object: reference.clone(),
                        layer: ViewLayer::Personal,
                        action: None,
                        reason: None,
                    });
                    None
                } else {
                    Some(theme.clone())
                }
            }
            None => None,
        };

        // Supersede relations first: a superseded record (a moderation
        // action, a tombstone, any content object) has no effect in the
        // projection, whatever it once said. The active frontier is
        // resolved by `superseded_objects`, shared with `resolve_rules`.
        let superseded: BTreeSet<ArtifactHash> = self.superseded_objects(board);

        // Current moderator frontier: a role attestation that another
        // attestation supersedes is history, not authority. Following
        // the signed supersedes links (never arrival order) keeps this
        // identical on every peer holding the same set. An attestation
        // that is itself superseded — by an active generic supersede
        // above, or by an active attestation's supersedes edge — has no
        // effect, so its own supersedes edge does not apply either
        // (#4003): superseding a revocation reinstates the grant it
        // revoked. Like the generic frontier, this resolves to a fixed
        // point over the active attestations.
        let attestation_edges: Vec<(ArtifactHash, Option<ArtifactHash>)> = self
            .objects
            .iter()
            .filter_map(|(hash, signed)| match &signed.object {
                BoardObject::RoleAttestation(value) if &value.board == board => {
                    Some((hash.clone(), value.supersedes.clone()))
                }
                _ => None,
            })
            .collect();
        let eligible_attestations: BTreeSet<ArtifactHash> = attestation_edges
            .iter()
            .map(|(hash, _)| hash.clone())
            .filter(|hash| !superseded.contains(hash))
            .collect();
        // Recompute the active set from the eligible set on every pass,
        // never by only shrinking the previous pass: in a chain
        // grant -> revocation -> grant -> revocation, the middle
        // revocation must reactivate once the grant superseding it is
        // itself superseded, and a shrinking-only loop can never bring
        // it back (Codex P1 on PR #4512). Supersedes edges always name
        // an already-inserted record, so the graph is acyclic and this
        // settles within one pass per chain link; the pass cap only
        // bounds a graph that somehow broke that rule.
        let mut active_attestations: BTreeSet<ArtifactHash> = eligible_attestations.clone();
        let attestation_superseded: BTreeSet<ArtifactHash> = {
            let mut named: BTreeSet<ArtifactHash> = BTreeSet::new();
            for _ in 0..=attestation_edges.len() {
                named = attestation_edges
                    .iter()
                    .filter(|(hash, _)| active_attestations.contains(hash))
                    .filter_map(|(_, previous)| previous.clone())
                    .collect();
                let next: BTreeSet<ArtifactHash> =
                    eligible_attestations.difference(&named).cloned().collect();
                if next == active_attestations {
                    break;
                }
                active_attestations = next;
            }
            named
        };
        let current_moderators: BTreeSet<String> = self
            .objects
            .iter()
            .filter(|(hash, _)| {
                !attestation_superseded.contains(hash) && !superseded.contains(hash)
            })
            .filter_map(|(_, signed)| match &signed.object {
                BoardObject::RoleAttestation(value)
                    if &value.board == board
                        && value.role == BoardRole::Moderator
                        && value.granted =>
                {
                    Some(value.member.clone())
                }
                _ => None,
            })
            .collect();
        let owner = root.artifact.author.as_str();
        let has_authority = |author: &str| author == owner || current_moderators.contains(author);

        // Board layer facts, gathered in deterministic (content-hash)
        // order from the map.
        let mut moderation_log = Vec::new();
        let mut hide_balance: BTreeMap<ArtifactHash, i64> = BTreeMap::new();
        let mut hide_source: BTreeMap<ArtifactHash, (ArtifactHash, Option<String>)> =
            BTreeMap::new();
        let mut tombstones: BTreeMap<ArtifactHash, (ArtifactHash, Option<String>)> =
            BTreeMap::new();
        for (hash, signed) in &self.objects {
            match &signed.object {
                BoardObject::ModerationAction(value) if &value.board == board => {
                    // The log is public and complete: every action is
                    // listed, including ones that no longer (or never
                    // did) carry authority. Only empowered, current
                    // records change the projection.
                    moderation_log.push(hash.clone());
                    if superseded.contains(hash) || !has_authority(signed.artifact.author.as_str())
                    {
                        continue;
                    }
                    match value.action {
                        ModerationActionKind::Hide => {
                            *hide_balance.entry(value.target.clone()).or_default() += 1;
                            hide_source
                                .insert(value.target.clone(), (hash.clone(), value.reason.clone()));
                        }
                        ModerationActionKind::Restore => {
                            *hide_balance.entry(value.target.clone()).or_default() -= 1;
                        }
                    }
                }
                BoardObject::Tombstone(value) if &value.board == board => {
                    if !superseded.contains(hash) {
                        tombstones
                            .entry(value.target.clone())
                            .or_insert_with(|| (hash.clone(), value.reason.clone()));
                    }
                }
                _ => {}
            }
        }

        let mut objects = Vec::new();
        let mut hidden = Vec::new();
        if let Some(entry) = theme_hidden {
            hidden.push(entry);
        }
        for (hash, signed) in &self.objects {
            if hash != board && signed.object.board_ref() != Some(board) {
                continue;
            }
            // Log and relation records are not view content: the log is
            // reported as `moderation_log`, and supersede/tombstone
            // records act on their targets instead of being shown.
            if matches!(
                signed.object,
                BoardObject::ModerationAction(_)
                    | BoardObject::Supersede(_)
                    | BoardObject::Tombstone(_)
            ) {
                continue;
            }
            if superseded.contains(hash) {
                continue;
            }
            let board_hide = hide_balance.get(hash).copied().unwrap_or(0) > 0;
            let tombstone = tombstones.get(hash);
            if board_hide || tombstone.is_some() {
                let (action, reason) = if board_hide {
                    let (action, reason) = hide_source
                        .get(hash)
                        .cloned()
                        .unwrap_or((hash.clone(), None));
                    (Some(action), reason)
                } else {
                    let (action, reason) = tombstone.cloned().unwrap_or((hash.clone(), None));
                    (Some(action), reason)
                };
                hidden.push(HiddenObject {
                    object: hash.clone(),
                    layer: ViewLayer::Board,
                    action,
                    reason,
                });
            } else if layers.host_hidden.contains(hash) {
                hidden.push(HiddenObject {
                    object: hash.clone(),
                    layer: ViewLayer::Host,
                    action: None,
                    reason: None,
                });
            } else if layers.personal_hidden.contains(hash)
                || layers.muted_authors.contains(&signed.artifact.author)
            {
                hidden.push(HiddenObject {
                    object: hash.clone(),
                    layer: ViewLayer::Personal,
                    action: None,
                    reason: None,
                });
            } else {
                objects.push(hash.clone());
            }
        }

        Ok(BoardView {
            board: board.clone(),
            theme,
            objects,
            hidden,
            moderation_log,
        })
    }
}

fn validate_board_id(value: &str) -> Result<()> {
    ensure!(
        (1..=128).contains(&value.len())
            && value.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b"-_.".contains(&byte)),
        "board ID must be 1-128 lower-case ASCII letters, digits, '-', '_' or '.'"
    );
    Ok(())
}

fn validate_text(name: &str, value: &str, allow_empty: bool) -> Result<()> {
    ensure!(
        (allow_empty || !value.is_empty()) && value.len() <= MAX_BOARD_TEXT_BYTES,
        "{name} must be {} to {MAX_BOARD_TEXT_BYTES} bytes",
        if allow_empty { "0" } else { "1" }
    );
    ensure!(!value.contains('\0'), "{name} must not contain NUL");
    Ok(())
}

struct SerializedSizeLimit {
    limit: usize,
    written: usize,
}

impl Write for SerializedSizeLimit {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.written);
        let written = buffer.len().min(remaining);
        self.written += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Lower-case hex SHA-256 of `bytes`, the canonical package-hash form.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_game_participants(name: &str, participants: &[GameParticipant]) -> Result<()> {
    ensure!(
        !participants.is_empty() && participants.len() <= MAX_GAME_PARTICIPANTS,
        "a {name} assigns 1 to {MAX_GAME_PARTICIPANTS} participants"
    );
    let mut participant_roles = BTreeSet::new();
    for participant in participants {
        ensure!(
            is_public_key(&participant.author),
            "game participant is not a canonical Ed25519 key"
        );
        validate_game_role(&participant.role)?;
        ensure!(
            participant_roles.insert((participant.author.as_str(), participant.role.as_str())),
            "duplicate game participant (author, role) assignment"
        );
    }
    Ok(())
}

fn validate_game_data(name: &str, value: &serde_json::Value) -> Result<()> {
    let mut writer = SerializedSizeLimit {
        limit: MAX_GAME_DATA_BYTES,
        written: 0,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(()),
        Err(_) if writer.written == MAX_GAME_DATA_BYTES => {
            bail!("{name} is at most {MAX_GAME_DATA_BYTES} serialized bytes")
        }
        Err(error) => Err(error.into()),
    }
}

fn validate_token_name(value: &str) -> Result<()> {
    validate_ascii_identifier("theme token name", value, 64)
}

fn validate_game_role(value: &str) -> Result<()> {
    validate_ascii_identifier("game role identifier", value, 64)
}

fn validate_ascii_identifier(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    ensure!(
        (1..=max_bytes).contains(&value.len())
            && value.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b"-_.".contains(&byte)),
        "{name} must be 1-{max_bytes} lower-case ASCII letters, digits, '-', '_' or '.'"
    );
    Ok(())
}

fn is_public_key(value: &str) -> bool {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return false;
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = (pair[0] as char).to_digit(16);
        let low = (pair[1] as char).to_digit(16);
        let (Some(high), Some(low)) = (high, low) else {
            return false;
        };
        bytes[index] = ((high << 4) | low) as u8;
    }
    VerifyingKey::from_bytes(&bytes).is_ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use ed25519_dalek::SigningKey;
    use proptest::prelude::*;

    use super::*;
    use crate::world::hex_encode;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn board(key: &SigningKey) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::Board(Board {
                board_id: "retro-games".to_owned(),
                forked_from: None,
                theme_ref: None,
            }),
        )
        .unwrap()
    }

    fn post(
        key: &SigningKey,
        board: ArtifactHash,
        thread: ArtifactHash,
        body: &str,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::Post(Post {
                board,
                thread,
                parent_post: None,
                body: body.to_owned(),
                attachments: Vec::new(),
                embed_ref: None,
            }),
        )
        .unwrap()
    }

    fn post_with_embed_ref(
        key: &SigningKey,
        board: ArtifactHash,
        thread: ArtifactHash,
        embed_ref: ArtifactHash,
        body: &str,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::Post(Post {
                board,
                thread,
                parent_post: None,
                body: body.to_owned(),
                attachments: Vec::new(),
                embed_ref: Some(GamePostRef { object: embed_ref }),
            }),
        )
        .unwrap()
    }

    fn game_post_body(
        board: ArtifactHash,
        version: String,
        configuration: serde_json::Value,
        participants: Vec<GameParticipant>,
        genesis: serde_json::Value,
    ) -> GamePost {
        GamePost {
            board,
            package: GamePackageRef {
                hash: ArtifactHash::new("e".repeat(64)).unwrap(),
                version,
            },
            mode: GameMode::AsyncTurns,
            configuration,
            participants,
            genesis,
        }
    }

    fn game_post_object(
        author: &SigningKey,
        opponent: &SigningKey,
        board: ArtifactHash,
    ) -> SignedBoardObject {
        let participants = vec![
            GameParticipant {
                author: hex_encode(&author.verifying_key().to_bytes()),
                role: "x".to_owned(),
            },
            GameParticipant {
                author: hex_encode(&opponent.verifying_key().to_bytes()),
                role: "o".to_owned(),
            },
        ];
        SignedBoardObject::sign(
            author,
            1,
            "",
            BoardObject::GamePost(game_post_body(
                board,
                "0.1.0".to_owned(),
                serde_json::json!({"board_size": 3}),
                participants,
                serde_json::json!("---------"),
            )),
        )
        .unwrap()
    }

    fn participant(index: usize) -> GameParticipant {
        let author = key(u8::try_from(index).unwrap());
        GameParticipant {
            author: hex_encode(&author.verifying_key().to_bytes()),
            role: format!("role-{index}"),
        }
    }

    fn invalid_game_post_is_rejected_by_builder_and_receipt(
        author: &SigningKey,
        game: GamePost,
    ) -> bool {
        let typed = BoardObject::GamePost(game.clone());
        let raw = serde_json::to_value(&typed).unwrap();
        let artifact = SignedArtifact::sign(author, 1, "", BOARD_OBJECT_KIND, raw).unwrap();
        SignedBoardObject::sign(author, 1, "", typed).is_err()
            && SignedBoardObject::from_artifact(artifact).is_err()
    }

    fn theme(key: &SigningKey, seq: u64, forked_from: Option<ArtifactHash>) -> SignedBoardObject {
        let mut tokens = BTreeMap::new();
        tokens.insert("accent".to_owned(), "#cc5500".to_owned());
        tokens.insert("surface".to_owned(), "#101418".to_owned());
        SignedBoardObject::sign(
            key,
            seq,
            "",
            BoardObject::Theme(Theme {
                name: "Ember".to_owned(),
                tokens,
                layout: ThemeLayout::Grid,
                assets: Vec::new(),
                forked_from,
            }),
        )
        .unwrap()
    }

    fn themed_board(key: &SigningKey, theme_ref: ArtifactHash) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::Board(Board {
                board_id: "themed-board".to_owned(),
                forked_from: None,
                theme_ref: Some(theme_ref),
            }),
        )
        .unwrap()
    }

    fn thread(key: &SigningKey, board: ArtifactHash, title: &str) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::Thread(Thread {
                board,
                forum_ref: None,
                rules_ref: None,
                title: title.to_owned(),
            }),
        )
        .unwrap()
    }

    fn moderation(
        key: &SigningKey,
        seq: u64,
        board: ArtifactHash,
        target: ArtifactHash,
        action: ModerationActionKind,
        reason: Option<&str>,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            seq,
            "",
            BoardObject::ModerationAction(ModerationAction {
                board,
                target,
                action,
                reason: reason.map(str::to_owned),
            }),
        )
        .unwrap()
    }

    #[test]
    fn signed_typed_objects_round_trip_and_reject_forgery() {
        let signer = key(1);
        let signed = board(&signer);
        signed.verify().unwrap();
        let decoded = SignedBoardObject::from_artifact(signed.artifact.clone()).unwrap();
        assert_eq!(decoded, signed);

        let mut forged = signed.artifact;
        forged.body["data"]["board_id"] = serde_json::Value::String("changed".to_owned());
        assert!(SignedBoardObject::from_artifact(forged).is_err());
    }

    #[test]
    fn references_must_be_canonical_hashes() {
        assert!(serde_json::from_str::<ArtifactHash>(&format!("\"{}\"", "A".repeat(64))).is_err());
        assert!(serde_json::from_str::<ArtifactHash>("\"not-a-hash\"").is_err());
        assert!(ArtifactHash::new("a".repeat(64)).is_ok());
    }

    #[test]
    fn graph_rejects_missing_or_cross_board_references() {
        let creator = key(2);
        let other = key(3);
        let mut graph = BoardGraph::default();
        let root = board(&creator);
        let root_hash = graph.insert(root).unwrap();
        let missing_thread = ArtifactHash::new("a".repeat(64)).unwrap();
        assert!(
            graph
                .insert(post(&creator, root_hash.clone(), missing_thread, "hello"))
                .is_err()
        );

        let foreign_root = graph.insert(board(&other)).unwrap();
        let foreign_thread = SignedBoardObject::sign(
            &other,
            2,
            "",
            BoardObject::Thread(Thread {
                board: foreign_root.clone(),
                forum_ref: None,
                rules_ref: None,
                title: "Other board".to_owned(),
            }),
        )
        .unwrap();
        let foreign_thread = graph.insert(foreign_thread).unwrap();
        assert!(
            graph
                .insert(post(&creator, root_hash, foreign_thread, "cross-board"))
                .is_err()
        );
    }

    #[test]
    fn signed_game_post_is_verified_and_post_embed_ref_is_board_scoped() {
        let creator = key(6);
        let other = key(7);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&creator)).unwrap();
        let thread_hash = graph
            .insert(thread(&creator, root_hash.clone(), "main"))
            .unwrap();

        let signed_game = game_post_object(&creator, &other, root_hash.clone());
        signed_game.verify().unwrap();
        let decoded = SignedBoardObject::from_artifact(signed_game.artifact.clone()).unwrap();
        assert_eq!(decoded, signed_game);

        let mut tampered = signed_game.artifact.clone();
        tampered.body["data"]["package"]["version"] = serde_json::Value::String("9.9.9".to_owned());
        assert!(SignedBoardObject::from_artifact(tampered).is_err());

        let missing = ArtifactHash::new("a".repeat(64)).unwrap();
        assert!(
            graph
                .insert(post_with_embed_ref(
                    &creator,
                    root_hash.clone(),
                    thread_hash.clone(),
                    missing,
                    "dangling embed",
                ))
                .is_err()
        );

        assert!(
            graph
                .insert(post_with_embed_ref(
                    &creator,
                    root_hash.clone(),
                    thread_hash.clone(),
                    thread_hash.clone(),
                    "wrong kind embed",
                ))
                .is_err()
        );

        let ordinary_post = graph
            .insert(post(
                &creator,
                root_hash.clone(),
                thread_hash.clone(),
                "not a game post",
            ))
            .unwrap();
        assert!(
            graph
                .insert(post_with_embed_ref(
                    &creator,
                    root_hash.clone(),
                    thread_hash.clone(),
                    ordinary_post,
                    "wrong object type",
                ))
                .is_err()
        );

        let foreign_root = graph.insert(board(&other)).unwrap();
        let foreign_game = graph
            .insert(game_post_object(&other, &creator, foreign_root.clone()))
            .unwrap();
        assert!(
            graph
                .insert(post_with_embed_ref(
                    &creator,
                    root_hash.clone(),
                    thread_hash.clone(),
                    foreign_game,
                    "cross-board embed",
                ))
                .is_err()
        );

        let local_game = graph.insert(signed_game).unwrap();
        let embedded_post = graph
            .insert(post_with_embed_ref(
                &creator,
                root_hash.clone(),
                thread_hash,
                local_game.clone(),
                "valid embed",
            ))
            .unwrap();

        let mut replica = BoardGraph::default();
        let report = replica.replicate_board_from(&graph, &root_hash).unwrap();
        assert_eq!(report.status.state, ReplicaState::Complete);
        assert!(replica.get(&local_game).is_some());
        let view = replica
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(view.objects.contains(&local_game));
        assert!(view.objects.contains(&embedded_post));
    }

    proptest! {
        #[test]
        fn rules_resolution_is_independent_of_insertion_order(
            additional in prop::collection::vec((1_u64..=3, "[a-z0-9]{0,8}"), 0..6)
        ) {
            let owner = key(41);
            let mut first_graph = BoardGraph::default();
            let board_hash = first_graph.insert(board(&owner)).unwrap();
            let mut reverse_graph = BoardGraph::default();
            prop_assert_eq!(reverse_graph.insert(board(&owner)).unwrap(), board_hash.clone());

            // Always include two different revision-1 rules so the regression
            // property covers the adopted legal-duplicate case.
            let rule_data = [(1_u64, "first".to_owned()), (1_u64, "second".to_owned())]
                .into_iter()
                .chain(additional.into_iter().map(|(revision, text)| (revision, text)));
            let mut rules = Vec::new();
            for (index, (revision, text)) in rule_data.enumerate() {
                let signed = SignedBoardObject::sign(
                    &owner,
                    index as u64 + 2,
                    "",
                    BoardObject::Rules(Rules {
                        board: board_hash.clone(),
                        revision,
                        text: format!("{index}: {text}"),
                    }),
                )
                .unwrap();
                rules.push((signed.hash().unwrap(), revision, signed));
            }

            for (_, _, signed) in &rules {
                first_graph.insert(signed.clone()).unwrap();
            }
            for (_, _, signed) in rules.iter().rev() {
                reverse_graph.insert(signed.clone()).unwrap();
            }

            let expected_current = rules
                .iter()
                .max_by(|(left_hash, left_revision, _), (right_hash, right_revision, _)| {
                    left_revision
                        .cmp(right_revision)
                        .then_with(|| left_hash.cmp(right_hash))
                })
                .map(|(hash, _, _)| hash)
                .unwrap();
            prop_assert_eq!(
                first_graph.resolve_rules(&board_hash, None).map(|(hash, _)| hash),
                Some(expected_current)
            );
            prop_assert_eq!(
                reverse_graph.resolve_rules(&board_hash, None).map(|(hash, _)| hash),
                Some(expected_current)
            );

            for revision in 1..=3 {
                let expected_revision = rules
                    .iter()
                    .filter(|(_, candidate_revision, _)| *candidate_revision == revision)
                    .map(|(hash, _, _)| hash)
                    .max();
                prop_assert_eq!(
                    first_graph.resolve_rules(&board_hash, Some(revision)).map(|(hash, _)| hash),
                    expected_revision
                );
                prop_assert_eq!(
                    reverse_graph.resolve_rules(&board_hash, Some(revision)).map(|(hash, _)| hash),
                    expected_revision
                );
            }

            // A thread keeps the exact hash it names, even if that is not the
            // graph's selected winner for the same revision.
            let pinned_hash = rules
                .iter()
                .filter(|(_, revision, _)| *revision == 1)
                .map(|(hash, _, _)| hash)
                .min()
                .unwrap()
                .clone();
            let pinned_thread = SignedBoardObject::sign(
                &owner,
                u64::MAX,
                "",
                BoardObject::Thread(Thread {
                    board: board_hash.clone(),
                    forum_ref: None,
                    rules_ref: Some(pinned_hash.clone()),
                    title: "pinned rules".to_owned(),
                }),
            )
            .unwrap();
            let thread_hash = pinned_thread.hash().unwrap();
            first_graph.insert(pinned_thread.clone()).unwrap();
            reverse_graph.insert(pinned_thread).unwrap();
            for graph in [&first_graph, &reverse_graph] {
                let BoardObject::Thread(thread) = &graph.get(&thread_hash).unwrap().object else {
                    prop_assert!(false, "inserted thread hash did not resolve to a thread");
                    return Ok(());
                };
                prop_assert_eq!(thread.rules_ref.as_ref(), Some(&pinned_hash));
            }
        }
    }

    #[test]
    fn only_board_owner_can_issue_roles_and_moderation() {
        let owner = key(4);
        let moderator = key(5);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let moderator_key = hex_encode(&moderator.verifying_key().to_bytes());
        let grant = SignedBoardObject::sign(
            &owner,
            2,
            "",
            BoardObject::RoleAttestation(RoleAttestation {
                board: root_hash.clone(),
                member: moderator_key.clone(),
                role: BoardRole::Moderator,
                granted: true,
                supersedes: None,
            }),
        )
        .unwrap();
        graph.insert(grant).unwrap();

        let rules = SignedBoardObject::sign(
            &moderator,
            1,
            "",
            BoardObject::Rules(Rules {
                board: root_hash.clone(),
                revision: 1,
                text: "Be kind".to_owned(),
            }),
        )
        .unwrap();
        graph.insert(rules).unwrap();

        let forged_grant = SignedBoardObject::sign(
            &moderator,
            2,
            "",
            BoardObject::RoleAttestation(RoleAttestation {
                board: root_hash,
                member: hex_encode(&owner.verifying_key().to_bytes()),
                role: BoardRole::Moderator,
                granted: true,
                supersedes: None,
            }),
        )
        .unwrap();
        assert!(graph.insert(forged_grant).is_err());
    }

    #[test]
    fn supersede_and_tombstone_are_author_signed_append_only_records() {
        let author = key(6);
        let mut graph = BoardGraph::default();
        let root = board(&author);
        let root_hash = graph.insert(root.clone()).unwrap();
        let thread = SignedBoardObject::sign(
            &author,
            2,
            &root.artifact.hash,
            BoardObject::Thread(Thread {
                board: root_hash.clone(),
                forum_ref: None,
                rules_ref: None,
                title: "Introductions".to_owned(),
            }),
        )
        .unwrap();
        let thread_hash = graph.insert(thread).unwrap();
        let first = post(&author, root_hash.clone(), thread_hash.clone(), "first");
        let first_hash = graph.insert(first.clone()).unwrap();
        let second = post(&author, root_hash.clone(), thread_hash, "edited");
        let second_hash = graph.insert(second).unwrap();
        let supersede = SignedBoardObject::sign(
            &author,
            4,
            &first.artifact.hash,
            BoardObject::Supersede(Supersede {
                board: root_hash.clone(),
                previous: first_hash.clone(),
                replacement: second_hash,
            }),
        )
        .unwrap();
        graph.insert(supersede).unwrap();
        let tombstone = SignedBoardObject::sign(
            &author,
            5,
            "",
            BoardObject::Tombstone(Tombstone {
                board: root_hash,
                target: first_hash,
                reason: Some("author deletion".to_owned()),
            }),
        )
        .unwrap();
        graph.insert(tombstone).unwrap();
        assert_eq!(graph.len(), 6);
    }

    #[test]
    fn reply_parent_must_belong_to_the_same_thread() {
        let author = key(7);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&author)).unwrap();
        let thread = |title: &str, seq: u64| {
            SignedBoardObject::sign(
                &author,
                seq,
                "",
                BoardObject::Thread(Thread {
                    board: root_hash.clone(),
                    forum_ref: None,
                    rules_ref: None,
                    title: title.to_owned(),
                }),
            )
            .unwrap()
        };
        let first_thread = graph.insert(thread("First", 2)).unwrap();
        let second_thread = graph.insert(thread("Second", 3)).unwrap();
        let parent = graph
            .insert(post(
                &author,
                root_hash.clone(),
                first_thread.clone(),
                "parent",
            ))
            .unwrap();
        let cross_thread_reply = SignedBoardObject::sign(
            &author,
            4,
            "",
            BoardObject::Post(Post {
                board: root_hash.clone(),
                thread: second_thread,
                parent_post: Some(parent.clone()),
                body: "cross-thread reply".to_owned(),
                attachments: Vec::new(),
                embed_ref: None,
            }),
        )
        .unwrap();
        assert!(graph.insert(cross_thread_reply).is_err());

        let same_thread_reply = SignedBoardObject::sign(
            &author,
            5,
            "",
            BoardObject::Post(Post {
                board: root_hash,
                thread: first_thread,
                parent_post: Some(parent),
                body: "same-thread reply".to_owned(),
                attachments: Vec::new(),
                embed_ref: None,
            }),
        )
        .unwrap();
        graph.insert(same_thread_reply).unwrap();
    }

    #[test]
    fn supersede_cycles_are_rejected() {
        let author = key(8);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&author)).unwrap();
        let thread = SignedBoardObject::sign(
            &author,
            2,
            "",
            BoardObject::Thread(Thread {
                board: root_hash.clone(),
                forum_ref: None,
                rules_ref: None,
                title: "Versions".to_owned(),
            }),
        )
        .unwrap();
        let thread_hash = graph.insert(thread).unwrap();
        let first_hash = graph
            .insert(post(&author, root_hash.clone(), thread_hash.clone(), "v1"))
            .unwrap();
        let second_hash = graph
            .insert(post(&author, root_hash.clone(), thread_hash.clone(), "v2"))
            .unwrap();
        let supersede = |seq: u64, previous: ArtifactHash, replacement: ArtifactHash| {
            SignedBoardObject::sign(
                &author,
                seq,
                "",
                BoardObject::Supersede(Supersede {
                    board: root_hash.clone(),
                    previous,
                    replacement,
                }),
            )
            .unwrap()
        };
        graph
            .insert(supersede(3, first_hash.clone(), second_hash.clone()))
            .unwrap();
        assert!(
            graph
                .insert(supersede(4, second_hash.clone(), first_hash.clone()))
                .is_err(),
            "a supersede edge closing a cycle must be rejected"
        );

        // Multi-successor reachability: first is superseded by both second
        // and third; an edge third -> first would close first -> third ->
        // first and must be rejected whichever successor sorts first.
        let third_hash = graph
            .insert(post(&author, root_hash.clone(), thread_hash, "v3"))
            .unwrap();
        graph
            .insert(supersede(5, first_hash.clone(), third_hash.clone()))
            .unwrap();
        assert!(
            graph.insert(supersede(6, third_hash, first_hash)).is_err(),
            "cycle detection must traverse every supersede successor"
        );
    }

    #[test]
    fn moderator_acceptance_does_not_depend_on_revocation_arrival_order() {
        let owner = key(9);
        let moderator = key(10);
        let moderator_key = hex_encode(&moderator.verifying_key().to_bytes());
        let build = |revoke_before_forum: bool| {
            let mut graph = BoardGraph::default();
            let root_hash = graph.insert(board(&owner)).unwrap();
            let grant = SignedBoardObject::sign(
                &owner,
                2,
                "",
                BoardObject::RoleAttestation(RoleAttestation {
                    board: root_hash.clone(),
                    member: moderator_key.clone(),
                    role: BoardRole::Moderator,
                    granted: true,
                    supersedes: None,
                }),
            )
            .unwrap();
            let grant_hash = graph.insert(grant).unwrap();
            let revocation = SignedBoardObject::sign(
                &owner,
                3,
                "",
                BoardObject::RoleAttestation(RoleAttestation {
                    board: root_hash.clone(),
                    member: moderator_key.clone(),
                    role: BoardRole::Moderator,
                    granted: false,
                    supersedes: Some(grant_hash),
                }),
            )
            .unwrap();
            let forum = SignedBoardObject::sign(
                &moderator,
                1,
                "",
                BoardObject::Forum(Forum {
                    board: root_hash,
                    name: "General".to_owned(),
                }),
            )
            .unwrap();
            if revoke_before_forum {
                graph.insert(revocation).unwrap();
                graph.insert(forum).unwrap();
            } else {
                graph.insert(forum).unwrap();
                graph.insert(revocation).unwrap();
            }
            graph.len()
        };
        // The Codex P1 scenario: both arrival orders reconstruct the graph.
        assert_eq!(build(true), 4);
        assert_eq!(build(false), 4);

        // Without any grant in the graph, moderator objects still reject.
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let ungranted_forum = SignedBoardObject::sign(
            &moderator,
            1,
            "",
            BoardObject::Forum(Forum {
                board: root_hash,
                name: "General".to_owned(),
            }),
        )
        .unwrap();
        assert!(graph.insert(ungranted_forum).is_err());
    }
    #[test]
    fn theme_fork_lineage_must_name_an_existing_theme() {
        let author = key(20);
        let mut graph = BoardGraph::default();
        let base = graph.insert(theme(&author, 1, None)).unwrap();
        graph.insert(theme(&author, 2, Some(base))).unwrap();

        let missing = ArtifactHash::new("b".repeat(64)).unwrap();
        assert!(graph.insert(theme(&author, 3, Some(missing))).is_err());

        let root_hash = graph.insert(board(&author)).unwrap();
        assert!(graph.insert(theme(&author, 4, Some(root_hash))).is_err());
    }

    #[test]
    fn theme_shape_rejects_bad_tokens_and_duplicate_assets() {
        let author = key(21);
        let mut bad_name = BTreeMap::new();
        bad_name.insert("Accent".to_owned(), "#ffffff".to_owned());
        assert!(
            SignedBoardObject::sign(
                &author,
                1,
                "",
                BoardObject::Theme(Theme {
                    name: "Bad".to_owned(),
                    tokens: bad_name,
                    layout: ThemeLayout::List,
                    assets: Vec::new(),
                    forked_from: None,
                }),
            )
            .is_err()
        );

        let mut empty_value = BTreeMap::new();
        empty_value.insert("accent".to_owned(), String::new());
        assert!(
            SignedBoardObject::sign(
                &author,
                1,
                "",
                BoardObject::Theme(Theme {
                    name: "Bad".to_owned(),
                    tokens: empty_value,
                    layout: ThemeLayout::List,
                    assets: Vec::new(),
                    forked_from: None,
                }),
            )
            .is_err()
        );

        let asset = ArtifactHash::new("c".repeat(64)).unwrap();
        assert!(
            SignedBoardObject::sign(
                &author,
                1,
                "",
                BoardObject::Theme(Theme {
                    name: "Bad".to_owned(),
                    tokens: BTreeMap::new(),
                    layout: ThemeLayout::Compact,
                    assets: vec![asset.clone(), asset],
                    forked_from: None,
                }),
            )
            .is_err()
        );
    }

    #[test]
    fn board_theme_ref_applies_a_theme_by_reference() {
        let author = key(22);
        let mut graph = BoardGraph::default();
        let theme_signed = theme(&author, 1, None);
        let theme_hash = graph.insert(theme_signed.clone()).unwrap();

        // A board naming a theme that is not in the graph is rejected...
        let missing = ArtifactHash::new("d".repeat(64)).unwrap();
        assert!(graph.insert(themed_board(&author, missing)).is_err());
        // ...as is a board whose theme_ref names a non-theme object.
        let other_board = graph.insert(board(&author)).unwrap();
        assert!(graph.insert(themed_board(&author, other_board)).is_err());

        let root_hash = graph.insert(themed_board(&author, theme_hash)).unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        let BoardObject::Theme(expected) = theme_signed.object else {
            unreachable!("theme helper builds a theme")
        };
        assert_eq!(view.theme, Some(expected));
        assert!(view.objects.contains(&root_hash));
        assert!(view.hidden.is_empty());

        // Board bodies written before theme_ref existed still decode.
        let legacy: Board =
            serde_json::from_str(r#"{"board_id":"legacy","forked_from":null}"#).unwrap();
        assert_eq!(legacy.theme_ref, None);
    }

    #[test]
    fn layered_view_hides_transparently_and_restore_unhides() {
        let owner = key(23);
        let author = key(24);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let thread_hash = graph
            .insert(thread(&owner, root_hash.clone(), "General"))
            .unwrap();
        let post_hash = graph
            .insert(post(
                &author,
                root_hash.clone(),
                thread_hash,
                "buy my stuff",
            ))
            .unwrap();

        let hide = moderation(
            &owner,
            2,
            root_hash.clone(),
            post_hash.clone(),
            ModerationActionKind::Hide,
            Some("spam"),
        );
        let hide_hash = graph.insert(hide).unwrap();

        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(!view.objects.contains(&post_hash));
        assert_eq!(view.moderation_log, vec![hide_hash.clone()]);
        assert_eq!(view.hidden.len(), 1);
        assert_eq!(view.hidden[0].object, post_hash);
        assert_eq!(view.hidden[0].layer, ViewLayer::Board);
        assert_eq!(view.hidden[0].action, Some(hide_hash));
        assert_eq!(view.hidden[0].reason.as_deref(), Some("spam"));
        // Hidden is not erased: the object is still in the graph.
        assert!(graph.get(&post_hash).is_some());

        let restore = moderation(
            &owner,
            3,
            root_hash.clone(),
            post_hash.clone(),
            ModerationActionKind::Restore,
            None,
        );
        graph.insert(restore).unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(view.objects.contains(&post_hash));
        assert!(view.hidden.is_empty());
        assert_eq!(view.moderation_log.len(), 2);

        // A second hide outnumbers the restore and hides it again.
        let rehide = moderation(
            &owner,
            4,
            root_hash.clone(),
            post_hash.clone(),
            ModerationActionKind::Hide,
            Some("spam again"),
        );
        graph.insert(rehide).unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert_eq!(view.hidden.len(), 1);
        assert_eq!(view.hidden[0].reason.as_deref(), Some("spam again"));
    }

    #[test]
    fn personal_and_host_layers_hide_without_a_board_record() {
        let owner = key(25);
        let alice = key(26);
        let bob = key(27);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let thread_hash = graph
            .insert(thread(&owner, root_hash.clone(), "General"))
            .unwrap();
        let alice_post = graph
            .insert(post(
                &alice,
                root_hash.clone(),
                thread_hash.clone(),
                "alice",
            ))
            .unwrap();
        let bob_post = graph
            .insert(post(&bob, root_hash.clone(), thread_hash, "bob"))
            .unwrap();

        let mut layers = ViewLayers::default();
        layers
            .muted_authors
            .insert(hex_encode(&alice.verifying_key().to_bytes()));
        layers.host_hidden.insert(bob_post.clone());
        let view = graph.compose_view(&root_hash, &layers).unwrap();
        assert!(view.moderation_log.is_empty());
        assert_eq!(view.hidden.len(), 2);
        let by_object: BTreeMap<_, _> = view
            .hidden
            .iter()
            .map(|entry| (entry.object.clone(), entry.layer))
            .collect();
        assert_eq!(by_object[&alice_post], ViewLayer::Personal);
        assert_eq!(by_object[&bob_post], ViewLayer::Host);
        assert!(!view.objects.contains(&alice_post));
        assert!(!view.objects.contains(&bob_post));

        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(view.objects.contains(&alice_post));
        assert!(view.objects.contains(&bob_post));
        assert!(view.hidden.is_empty());
    }

    #[test]
    fn view_enforces_moderator_revocation_but_keeps_the_public_log() {
        let owner = key(30);
        let moderator = key(31);
        let author = key(32);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let thread_hash = graph
            .insert(thread(&owner, root_hash.clone(), "General"))
            .unwrap();
        let post_hash = graph
            .insert(post(&author, root_hash.clone(), thread_hash, "hello"))
            .unwrap();
        let grant = SignedBoardObject::sign(
            &owner,
            2,
            "",
            BoardObject::RoleAttestation(RoleAttestation {
                board: root_hash.clone(),
                member: hex_encode(&moderator.verifying_key().to_bytes()),
                role: BoardRole::Moderator,
                granted: true,
                supersedes: None,
            }),
        )
        .unwrap();
        let grant_hash = graph.insert(grant).unwrap();
        let hide_hash = graph
            .insert(moderation(
                &moderator,
                1,
                root_hash.clone(),
                post_hash.clone(),
                ModerationActionKind::Hide,
                Some("spam"),
            ))
            .unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert_eq!(view.hidden.len(), 1);
        assert_eq!(view.hidden[0].object, post_hash);

        let revoke = SignedBoardObject::sign(
            &owner,
            3,
            "",
            BoardObject::RoleAttestation(RoleAttestation {
                board: root_hash.clone(),
                member: hex_encode(&moderator.verifying_key().to_bytes()),
                role: BoardRole::Moderator,
                granted: false,
                supersedes: Some(grant_hash),
            }),
        )
        .unwrap();
        graph.insert(revoke).unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        // The revoked moderator's hide no longer has an effect...
        assert!(view.objects.contains(&post_hash));
        assert!(view.hidden.is_empty());
        // ...but it stays in the public moderation log.
        assert_eq!(view.moderation_log, vec![hide_hash]);
    }

    #[test]
    fn superseded_tombstone_loses_effect_and_keeps_its_signed_reason() {
        let owner = key(33);
        let author = key(34);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let thread_hash = graph
            .insert(thread(&owner, root_hash.clone(), "General"))
            .unwrap();
        let first = graph
            .insert(post(
                &author,
                root_hash.clone(),
                thread_hash.clone(),
                "first",
            ))
            .unwrap();
        let second = graph
            .insert(post(&author, root_hash.clone(), thread_hash, "second"))
            .unwrap();
        let tombstone = |target: ArtifactHash, reason: &str| {
            SignedBoardObject::sign(
                &author,
                2,
                "",
                BoardObject::Tombstone(Tombstone {
                    board: root_hash.clone(),
                    target,
                    reason: Some(reason.to_owned()),
                }),
            )
            .unwrap()
        };
        let first_tombstone = graph
            .insert(tombstone(first.clone(), "wrong thread"))
            .unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert_eq!(view.hidden.len(), 1);
        assert_eq!(view.hidden[0].object, first);
        assert_eq!(view.hidden[0].reason.as_deref(), Some("wrong thread"));

        let second_tombstone = graph
            .insert(tombstone(second.clone(), "duplicate"))
            .unwrap();
        let supersede = SignedBoardObject::sign(
            &author,
            3,
            "",
            BoardObject::Supersede(Supersede {
                board: root_hash.clone(),
                previous: first_tombstone,
                replacement: second_tombstone,
            }),
        )
        .unwrap();
        graph.insert(supersede).unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(view.objects.contains(&first));
        assert_eq!(view.hidden.len(), 1);
        assert_eq!(view.hidden[0].object, second);
        assert_eq!(view.hidden[0].reason.as_deref(), Some("duplicate"));
    }

    #[test]
    fn host_layer_can_suppress_the_board_theme() {
        let author = key(35);
        let mut graph = BoardGraph::default();
        let theme_hash = graph.insert(theme(&author, 1, None)).unwrap();
        let root_hash = graph
            .insert(themed_board(&author, theme_hash.clone()))
            .unwrap();

        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(view.theme.is_some());

        let mut layers = ViewLayers::default();
        layers.host_hidden.insert(theme_hash);
        let view = graph.compose_view(&root_hash, &layers).unwrap();
        assert!(view.theme.is_none());
        assert_eq!(view.hidden.len(), 1);
        assert_eq!(view.hidden[0].layer, ViewLayer::Host);
    }

    #[test]
    fn generic_supersede_of_a_grant_also_revokes_authority() {
        let owner = key(36);
        let moderator = key(37);
        let author = key(38);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let thread_hash = graph
            .insert(thread(&owner, root_hash.clone(), "General"))
            .unwrap();
        let post_hash = graph
            .insert(post(&author, root_hash.clone(), thread_hash, "hello"))
            .unwrap();
        let attestation = |granted: bool| {
            SignedBoardObject::sign(
                &owner,
                2,
                "",
                BoardObject::RoleAttestation(RoleAttestation {
                    board: root_hash.clone(),
                    member: hex_encode(&moderator.verifying_key().to_bytes()),
                    role: BoardRole::Moderator,
                    granted,
                    supersedes: None,
                }),
            )
            .unwrap()
        };
        let grant_hash = graph.insert(attestation(true)).unwrap();
        graph
            .insert(moderation(
                &moderator,
                1,
                root_hash.clone(),
                post_hash.clone(),
                ModerationActionKind::Hide,
                Some("spam"),
            ))
            .unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert_eq!(view.hidden.len(), 1);

        // Revocation by generic Supersede record (not the attestation's
        // own supersedes field) must end the authority just the same.
        let revoke_hash = graph.insert(attestation(false)).unwrap();
        let supersede = SignedBoardObject::sign(
            &owner,
            3,
            "",
            BoardObject::Supersede(Supersede {
                board: root_hash.clone(),
                previous: grant_hash,
                replacement: revoke_hash,
            }),
        )
        .unwrap();
        graph.insert(supersede).unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(view.objects.contains(&post_hash));
        assert!(view.hidden.is_empty());
        assert_eq!(view.moderation_log.len(), 1);
    }

    #[test]
    fn superseded_revocation_no_longer_revokes() {
        // #4003: a revocation attestation that is itself superseded has
        // no effect, so the grant it revoked is authority again.
        let owner = key(41);
        let moderator = key(42);
        let author = key(43);
        let other = key(44);
        let moderator_key = hex_encode(&moderator.verifying_key().to_bytes());
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let thread_hash = graph
            .insert(thread(&owner, root_hash.clone(), "General"))
            .unwrap();
        let post_hash = graph
            .insert(post(&author, root_hash.clone(), thread_hash, "hello"))
            .unwrap();
        let attestation =
            |seq: u64, member: String, granted: bool, supersedes: Option<ArtifactHash>| {
                SignedBoardObject::sign(
                    &owner,
                    seq,
                    "",
                    BoardObject::RoleAttestation(RoleAttestation {
                        board: root_hash.clone(),
                        member,
                        role: BoardRole::Moderator,
                        granted,
                        supersedes,
                    }),
                )
                .unwrap()
            };
        let grant_hash = graph
            .insert(attestation(2, moderator_key.clone(), true, None))
            .unwrap();
        graph
            .insert(moderation(
                &moderator,
                1,
                root_hash.clone(),
                post_hash.clone(),
                ModerationActionKind::Hide,
                Some("spam"),
            ))
            .unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert_eq!(view.hidden.len(), 1);

        // Revoke: the moderator's Hide stops applying.
        let revocation_hash = graph
            .insert(attestation(
                3,
                moderator_key.clone(),
                false,
                Some(grant_hash.clone()),
            ))
            .unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(view.objects.contains(&post_hash));
        assert!(view.hidden.is_empty());

        // Nullify the revocation with a generic Supersede whose
        // replacement is an unrelated attestation: the original grant
        // is the current frontier again and the Hide applies once more.
        let other_grant_hash = graph
            .insert(attestation(
                4,
                hex_encode(&other.verifying_key().to_bytes()),
                true,
                None,
            ))
            .unwrap();
        let supersede = SignedBoardObject::sign(
            &owner,
            5,
            "",
            BoardObject::Supersede(Supersede {
                board: root_hash.clone(),
                previous: revocation_hash,
                replacement: other_grant_hash,
            }),
        )
        .unwrap();
        graph.insert(supersede).unwrap();
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert_eq!(view.hidden.len(), 1);
        assert!(!view.objects.contains(&post_hash));
    }

    #[test]
    fn attestation_chain_frontier_reinstates_and_re_revokes() {
        // Chain grant -> revocation -> grant -> revocation, each
        // attestation superseding its predecessor: only the last
        // revocation is the frontier, the first revocation reactivates
        // once the second grant falls, and the first grant stays
        // revoked (Codex P1 on PR #4512).
        let owner = key(45);
        let moderator = key(46);
        let author = key(47);
        let moderator_key = hex_encode(&moderator.verifying_key().to_bytes());
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let thread_hash = graph
            .insert(thread(&owner, root_hash.clone(), "General"))
            .unwrap();
        let post_hash = graph
            .insert(post(&author, root_hash.clone(), thread_hash, "hello"))
            .unwrap();
        let attestation = |seq: u64, granted: bool, supersedes: Option<ArtifactHash>| {
            SignedBoardObject::sign(
                &owner,
                seq,
                "",
                BoardObject::RoleAttestation(RoleAttestation {
                    board: root_hash.clone(),
                    member: moderator_key.clone(),
                    role: BoardRole::Moderator,
                    granted,
                    supersedes,
                }),
            )
            .unwrap()
        };
        let first_grant = graph.insert(attestation(2, true, None)).unwrap();
        graph
            .insert(moderation(
                &moderator,
                1,
                root_hash.clone(),
                post_hash.clone(),
                ModerationActionKind::Hide,
                Some("spam"),
            ))
            .unwrap();
        let hidden = |graph: &BoardGraph| {
            graph
                .compose_view(&root_hash, &ViewLayers::default())
                .unwrap()
                .hidden
                .len()
        };
        assert_eq!(hidden(&graph), 1);

        let first_revocation = graph
            .insert(attestation(3, false, Some(first_grant)))
            .unwrap();
        assert_eq!(hidden(&graph), 0);

        let second_grant = graph
            .insert(attestation(4, true, Some(first_revocation)))
            .unwrap();
        assert_eq!(hidden(&graph), 1);

        graph
            .insert(attestation(5, false, Some(second_grant)))
            .unwrap();
        assert_eq!(hidden(&graph), 0);
        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        assert!(view.objects.contains(&post_hash));
    }

    #[test]
    fn superseded_supersede_record_has_no_effect() {
        let owner = key(39);
        let author = key(40);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&owner)).unwrap();
        let thread_hash = graph
            .insert(thread(&owner, root_hash.clone(), "General"))
            .unwrap();
        let mut posts = Vec::new();
        for body in ["a", "b", "c", "d"] {
            posts.push(
                graph
                    .insert(post(&author, root_hash.clone(), thread_hash.clone(), body))
                    .unwrap(),
            );
        }
        let supersede = |seq: u64, previous: ArtifactHash, replacement: ArtifactHash| {
            SignedBoardObject::sign(
                &author,
                seq,
                "",
                BoardObject::Supersede(Supersede {
                    board: root_hash.clone(),
                    previous,
                    replacement,
                }),
            )
            .unwrap()
        };
        let first = graph
            .insert(supersede(2, posts[0].clone(), posts[1].clone()))
            .unwrap();
        let second = graph
            .insert(supersede(3, posts[2].clone(), posts[3].clone()))
            .unwrap();
        graph.insert(supersede(4, first, second)).unwrap();

        let view = graph
            .compose_view(&root_hash, &ViewLayers::default())
            .unwrap();
        // The first supersede was itself superseded: post A is current
        // again. The second supersede is active: post C is replaced.
        assert!(view.objects.contains(&posts[0]));
        assert!(view.objects.contains(&posts[1]));
        assert!(!view.objects.contains(&posts[2]));
        assert!(view.objects.contains(&posts[3]));
    }

    fn listing(
        key: &SigningKey,
        seq: u64,
        board: ArtifactHash,
        host: &SigningKey,
        version: u64,
        listed: bool,
        supersedes: Option<ArtifactHash>,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            seq,
            "",
            BoardObject::BoardListing(BoardListing {
                board,
                host: hex_encode(&host.verifying_key().to_bytes()),
                version,
                listed,
                contact: Some("iroh://host.example".to_owned()),
                supersedes,
            }),
        )
        .unwrap()
    }

    #[test]
    fn listing_publish_and_withdraw_follow_signed_versions() {
        let owner = key(7);
        let host = key(9);
        let host_hex = hex_encode(&host.verifying_key().to_bytes());
        let mut graph = BoardGraph::default();
        let root = graph.insert(board(&owner)).unwrap();

        // Only the board creator may list the board.
        let foreign = listing(&key(8), 1, root.clone(), &host, 1, true, None);
        assert!(graph.insert(foreign).is_err());
        // The first listing for a host starts at version 1.
        let starts_at_two = listing(&owner, 2, root.clone(), &host, 2, true, None);
        assert!(graph.insert(starts_at_two).is_err());

        let v1 = graph
            .insert(listing(&owner, 1, root.clone(), &host, 1, true, None))
            .unwrap();
        assert!(graph.active_listings(&root).contains_key(&host_hex));

        // A version may not skip: v3 superseding v1 is rejected.
        let skip = listing(&owner, 3, root.clone(), &host, 3, false, Some(v1.clone()));
        assert!(graph.insert(skip).is_err());

        // Withdrawal is the next signed version with listed == false.
        graph
            .insert(listing(&owner, 2, root.clone(), &host, 2, false, Some(v1)))
            .unwrap();
        assert!(graph.active_listings(&root).is_empty());
        let (_, latest) = &graph.latest_listings(&root)[&host_hex];
        assert_eq!(latest.version, 2);
        assert!(!latest.listed);
    }

    #[test]
    fn second_first_listing_for_same_host_is_rejected() {
        let owner = key(7);
        let host = key(9);
        let other_host = key(10);
        let mut graph = BoardGraph::default();
        let root = graph.insert(board(&owner)).unwrap();

        // The first listing for a host is accepted ...
        graph
            .insert(listing(&owner, 1, root.clone(), &host, 1, true, None))
            .unwrap();

        // ... but a second chain-less v1 for the same host is rejected,
        // whether the copy publishes or withdraws.
        for (seq, listed) in [(2, true), (3, false)] {
            let second_first = listing(&owner, seq, root.clone(), &host, 1, listed, None);
            let err = graph.insert(second_first).unwrap_err();
            assert!(
                err.to_string()
                    .contains("a first listing already exists for this host"),
                "unexpected error: {err}"
            );
        }

        // A first listing for a different host on the same board is fine.
        graph
            .insert(listing(
                &owner,
                4,
                root.clone(),
                &other_host,
                1,
                false,
                None,
            ))
            .unwrap();
    }

    #[test]
    fn two_hosts_converge_via_replication() {
        let owner = key(7);
        let host = key(9);
        let mut source = BoardGraph::default();
        let root = source.insert(board(&owner)).unwrap();
        let thread = source
            .insert(thread(&owner, root.clone(), "welcome"))
            .unwrap();
        source
            .insert(post(&owner, root.clone(), thread, "hello"))
            .unwrap();
        source
            .insert(listing(&owner, 1, root.clone(), &host, 1, true, None))
            .unwrap();

        let mut replica = BoardGraph::default();
        let report = replica.replicate_board_from(&source, &root).unwrap();
        assert_eq!(report.status.state, ReplicaState::Complete);
        assert_eq!(
            report.transferred.len(),
            source.board_object_set(&root).len()
        );
        assert_eq!(
            replica.board_object_set(&root),
            source.board_object_set(&root)
        );

        // A second pass is a no-op: the hosts have converged.
        let again = replica.replicate_board_from(&source, &root).unwrap();
        assert!(again.transferred.is_empty());
        assert_eq!(again.status.state, ReplicaState::Complete);
    }

    #[test]
    fn themed_board_replicates_with_its_theme() {
        // Codex P1 on PR #4048: a theme names no board, so a
        // scope-only export dropped it and the themed board root
        // could never validate in a fresh graph.
        let owner = key(7);
        let mut source = BoardGraph::default();
        let theme_hash = source.insert(theme(&owner, 1, None)).unwrap();
        let root = source
            .insert(themed_board(&owner, theme_hash.clone()))
            .unwrap();

        let mut replica = BoardGraph::default();
        let report = replica.replicate_board_from(&source, &root).unwrap();
        assert_eq!(report.status.state, ReplicaState::Complete);
        assert!(report.transferred.contains(&theme_hash));
        assert_eq!(
            replica.board_object_set(&root),
            source.board_object_set(&root)
        );
    }

    #[test]
    fn theme_fork_lineage_replicates_with_themed_board() {
        // The closure follows a theme's own fork lineage, not just
        // the theme the board names directly.
        let owner = key(7);
        let mut source = BoardGraph::default();
        let base_theme = source.insert(theme(&owner, 1, None)).unwrap();
        let derived_theme = source
            .insert(theme(&owner, 2, Some(base_theme.clone())))
            .unwrap();
        let root = source
            .insert(themed_board(&owner, derived_theme.clone()))
            .unwrap();

        let mut replica = BoardGraph::default();
        let report = replica.replicate_board_from(&source, &root).unwrap();
        assert_eq!(report.status.state, ReplicaState::Complete);
        assert!(report.transferred.contains(&derived_theme));
        assert!(report.transferred.contains(&base_theme));
    }

    #[test]
    fn forked_board_replicates_with_ancestor_root() {
        // A forked board's root names its source root in
        // `forked_from`; the ancestor root (and its own theme, by
        // recursion) must travel with the fork's replication set.
        let owner = key(7);
        let mut source = BoardGraph::default();
        let theme_hash = source.insert(theme(&owner, 1, None)).unwrap();
        let ancestor = source
            .insert(themed_board(&owner, theme_hash.clone()))
            .unwrap();
        let fork = SignedBoardObject::sign(
            &owner,
            1,
            "",
            BoardObject::Board(Board {
                board_id: "forked-board".to_owned(),
                forked_from: Some(ancestor.clone()),
                theme_ref: None,
            }),
        )
        .unwrap();
        let fork_root = source.insert(fork).unwrap();

        let mut replica = BoardGraph::default();
        let report = replica.replicate_board_from(&source, &fork_root).unwrap();
        assert_eq!(report.status.state, ReplicaState::Complete);
        assert!(report.transferred.contains(&ancestor));
        assert!(report.transferred.contains(&theme_hash));
        assert_eq!(
            replica.board_object_set(&fork_root),
            source.board_object_set(&fork_root)
        );
    }

    #[test]
    fn fork_record_is_in_fork_board_replication_set() {
        // #4081: a BoardFork record names no `board`, so the scope-only
        // seed left it in no board's replication set and fork provenance
        // never replicated. The record belongs to the fork board's set
        // (it is signed by the fork board's creator), and the closure
        // pulls both roots plus the cited source frontier.
        let owner = key(7);
        let forker = key(8);
        let mut source = BoardGraph::default();
        let ancestor = source.insert(board(&owner)).unwrap();
        let frontier_thread = source
            .insert(thread(&owner, ancestor.clone(), "fork frontier"))
            .unwrap();
        let fork_root = source
            .insert(
                SignedBoardObject::sign(
                    &forker,
                    1,
                    "",
                    BoardObject::Board(Board {
                        board_id: "forked-board".to_owned(),
                        forked_from: Some(ancestor.clone()),
                        theme_ref: None,
                    }),
                )
                .unwrap(),
            )
            .unwrap();
        let fork_record = source
            .insert(
                SignedBoardObject::sign(
                    &forker,
                    1,
                    "",
                    BoardObject::BoardFork(BoardFork {
                        source_board: ancestor.clone(),
                        fork_board: fork_root.clone(),
                        source_frontier: vec![ancestor.clone(), frontier_thread.clone()],
                    }),
                )
                .unwrap(),
            )
            .unwrap();

        let fork_set = source.board_object_set(&fork_root);
        assert!(fork_set.contains(&fork_record));
        assert!(fork_set.contains(&fork_root));
        assert!(fork_set.contains(&ancestor));
        assert!(fork_set.contains(&frontier_thread));
        // The provenance record travels with the fork board, not the
        // source board: source replicas must not require an object
        // their board's creator never signed.
        assert!(!source.board_object_set(&ancestor).contains(&fork_record));
    }

    #[test]
    fn fork_record_replicates_end_to_end_and_gates_completeness() {
        // #4081: replicate_board_from into a fresh graph accepts and
        // transfers the fork record, and a replica missing it reports
        // Incomplete — never a silent Complete.
        let owner = key(7);
        let forker = key(8);
        let ancestor_signed = board(&owner);
        // Content hashes are deterministic over the signed artifact, so
        // the thread can name the ancestor root before either is
        // inserted.
        let frontier_signed = thread(&owner, ancestor_signed.hash().unwrap(), "fork frontier");
        let mut source = BoardGraph::default();
        let ancestor = source.insert(ancestor_signed.clone()).unwrap();
        let frontier_thread = source.insert(frontier_signed.clone()).unwrap();
        let fork_root_signed = SignedBoardObject::sign(
            &forker,
            1,
            "",
            BoardObject::Board(Board {
                board_id: "forked-board".to_owned(),
                forked_from: Some(ancestor.clone()),
                theme_ref: None,
            }),
        )
        .unwrap();
        let fork_root = source.insert(fork_root_signed.clone()).unwrap();
        let fork_record_signed = SignedBoardObject::sign(
            &forker,
            1,
            "",
            BoardObject::BoardFork(BoardFork {
                source_board: ancestor.clone(),
                fork_board: fork_root.clone(),
                source_frontier: vec![ancestor.clone(), frontier_thread.clone()],
            }),
        )
        .unwrap();
        let fork_record = source.insert(fork_record_signed.clone()).unwrap();

        let mut replica = BoardGraph::default();
        let report = replica.replicate_board_from(&source, &fork_root).unwrap();
        assert_eq!(report.status.state, ReplicaState::Complete);
        assert!(report.transferred.contains(&fork_record));
        assert_eq!(
            replica.board_object_set(&fork_root),
            source.board_object_set(&fork_root)
        );

        let mut partial = BoardGraph::default();
        partial.insert(ancestor_signed).unwrap();
        partial.insert(frontier_signed).unwrap();
        partial.insert(fork_root_signed).unwrap();
        let status = partial.replica_status_against(&fork_root, &source);
        assert_eq!(status.state, ReplicaState::Incomplete);
        assert!(status.missing.contains(&fork_record));
        assert!(status.present_objects < status.expected_objects);
    }

    #[test]
    fn fork_frontier_post_pulls_its_acceptance_dependencies() {
        // Codex P1 on PR #4545: when a fork record cites a source-board
        // post in its frontier without also listing the post's thread,
        // the closure must still pull the thread -- and the thread's
        // forum and rules, and the moderator grant behind those -- or a
        // fresh replica rejects the post, then the fork record, and
        // replication stalls instead of completing.
        let owner = key(7);
        let moderator = key(11);
        let moderator_hex = hex_encode(&moderator.verifying_key().to_bytes());
        let forker = key(8);
        let mut source = BoardGraph::default();
        let root = source.insert(board(&owner)).unwrap();
        let grant = source
            .insert(
                SignedBoardObject::sign(
                    &owner,
                    2,
                    "",
                    BoardObject::RoleAttestation(RoleAttestation {
                        board: root.clone(),
                        member: moderator_hex,
                        role: BoardRole::Moderator,
                        granted: true,
                        supersedes: None,
                    }),
                )
                .unwrap(),
            )
            .unwrap();
        let rules = source
            .insert(
                SignedBoardObject::sign(
                    &moderator,
                    1,
                    "",
                    BoardObject::Rules(Rules {
                        board: root.clone(),
                        revision: 1,
                        text: "be kind".to_owned(),
                    }),
                )
                .unwrap(),
            )
            .unwrap();
        let forum = source
            .insert(
                SignedBoardObject::sign(
                    &moderator,
                    2,
                    "",
                    BoardObject::Forum(Forum {
                        board: root.clone(),
                        name: "general".to_owned(),
                    }),
                )
                .unwrap(),
            )
            .unwrap();
        let thread = source
            .insert(
                SignedBoardObject::sign(
                    &owner,
                    2,
                    "",
                    BoardObject::Thread(Thread {
                        board: root.clone(),
                        forum_ref: Some(forum.clone()),
                        rules_ref: Some(rules.clone()),
                        title: "cited thread".to_owned(),
                    }),
                )
                .unwrap(),
            )
            .unwrap();
        let cited_post = source
            .insert(post(
                &owner,
                root.clone(),
                thread.clone(),
                "cited by the fork",
            ))
            .unwrap();
        let fork_root = source
            .insert(
                SignedBoardObject::sign(
                    &forker,
                    1,
                    "",
                    BoardObject::Board(Board {
                        board_id: "forked-board".to_owned(),
                        forked_from: Some(root.clone()),
                        theme_ref: None,
                    }),
                )
                .unwrap(),
            )
            .unwrap();
        let fork_record = source
            .insert(
                SignedBoardObject::sign(
                    &forker,
                    1,
                    "",
                    BoardObject::BoardFork(BoardFork {
                        source_board: root.clone(),
                        fork_board: fork_root.clone(),
                        source_frontier: vec![cited_post.clone()],
                    }),
                )
                .unwrap(),
            )
            .unwrap();

        let fork_set = source.board_object_set(&fork_root);
        for expected in [
            &fork_record,
            &fork_root,
            &root,
            &cited_post,
            &thread,
            &forum,
            &rules,
            &grant,
        ] {
            assert!(fork_set.contains(expected), "missing {expected:?}");
        }

        let mut replica = BoardGraph::default();
        let report = replica.replicate_board_from(&source, &fork_root).unwrap();
        assert_eq!(report.status.state, ReplicaState::Complete);
        assert_eq!(
            replica.board_object_set(&fork_root),
            source.board_object_set(&fork_root)
        );
    }

    #[test]
    fn stale_replica_is_explicit_not_silent() {
        let owner = key(7);
        let host = key(9);
        let host_hex = hex_encode(&host.verifying_key().to_bytes());
        let mut source = BoardGraph::default();
        let root = source.insert(board(&owner)).unwrap();
        let v1 = source
            .insert(listing(&owner, 1, root.clone(), &host, 1, true, None))
            .unwrap();

        let mut replica = BoardGraph::default();
        replica.replicate_board_from(&source, &root).unwrap();

        // The source advances: a withdrawal and new content the replica
        // has not seen. The replica must report itself stale, naming
        // exactly what it is missing and the listing-version lag.
        source
            .insert(listing(&owner, 2, root.clone(), &host, 2, false, Some(v1)))
            .unwrap();
        let status = replica.replica_status_against(&root, &source);
        assert_eq!(status.state, ReplicaState::Stale);
        assert_eq!(status.listing_version_lag[&host_hex], (1, 2));
        assert_eq!(status.missing.len(), 1);
        assert!(status.present_objects < status.expected_objects);
    }

    #[test]
    fn delisting_is_host_local_and_never_global_erasure() {
        let owner = key(7);
        let mut graph = BoardGraph::default();
        let root = graph.insert(board(&owner)).unwrap();
        let before = graph.len();

        let mut policy = HostServingPolicy::default();
        assert!(policy.serves_board(&root));
        assert!(policy.delist_board(&root));
        assert!(!policy.serves_board(&root));
        // The signed graph is untouched: delisting removed nothing,
        // and another host's policy still serves the board.
        assert_eq!(graph.len(), before);
        assert!(HostServingPolicy::default().serves_board(&root));
    }

    proptest! {
        /// Oracle: every generated in-bound GamePost must pass the builder,
        /// signature verification and strict typed decode with its body intact.
        #[test]
        fn generated_valid_game_posts_round_trip(
            seed in any::<u8>(),
            participant_count in 1usize..=MAX_GAME_PARTICIPANTS,
            version_len in 1usize..=128,
            configuration in proptest::collection::vec(any::<i16>(), 0..32),
            genesis in proptest::collection::vec(any::<bool>(), 0..64),
        ) {
            let game = game_post_body(
                ArtifactHash::new("d".repeat(64)).unwrap(),
                "v".repeat(version_len),
                serde_json::json!(configuration),
                (0..participant_count).map(participant).collect(),
                serde_json::json!(genesis),
            );
            let signed = SignedBoardObject::sign(
                &key(seed),
                1,
                "",
                BoardObject::GamePost(game.clone()),
            ).unwrap();
            prop_assert!(signed.verify().is_ok());
            prop_assert_eq!(&signed.object, &BoardObject::GamePost(game));
            prop_assert_eq!(
                SignedBoardObject::from_artifact(signed.artifact.clone()).unwrap(),
                signed,
            );
        }

        /// Oracle: generated invalid metadata variants are refused both by the
        /// builder and by typed receipt of a valid SignedArtifact envelope.
        #[test]
        fn generated_invalid_game_post_variants_are_rejected(
            seed in any::<u8>(),
            invalid_variant in 0u8..8,
        ) {
            let author = key(seed);
            let mut invalid = game_post_body(
                ArtifactHash::new("d".repeat(64)).unwrap(),
                "0.1.0".to_owned(),
                serde_json::json!({"seed": seed}),
                vec![participant(1), participant(2)],
                serde_json::json!([seed]),
            );
            match invalid_variant {
                0 => invalid.participants.clear(),
                1 => invalid.participants = (0..=MAX_GAME_PARTICIPANTS).map(participant).collect(),
                2 => {
                    let first = invalid.participants[0].clone();
                    invalid.participants[1] = first;
                }
                3 => invalid.participants[0].author = "not-an-ed25519-key".to_owned(),
                4 => invalid.participants[0].role = "Player One".to_owned(),
                5 => invalid.package.version = "v".repeat(129),
                6 => {
                    invalid.configuration = serde_json::Value::String(
                        "c".repeat(MAX_GAME_DATA_BYTES - 1),
                    );
                }
                _ => {
                    invalid.genesis = serde_json::Value::String(
                        "g".repeat(MAX_GAME_DATA_BYTES - 1),
                    );
                }
            }
            prop_assert!(invalid_game_post_is_rejected_by_builder_and_receipt(
                &author,
                invalid,
            ));
        }
    }

    const TIC_TAC_TOE_WASM: &[u8] = include_bytes!("../apps/tic-tac-toe/program/tic-tac-toe.wasm");

    fn game_acceptance_object(
        key: &SigningKey,
        seq: u64,
        board: ArtifactHash,
        game_post: ArtifactHash,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            seq,
            "",
            BoardObject::GameChallengeAcceptance(GameChallengeAcceptance { board, game_post }),
        )
        .unwrap()
    }

    fn game_turn_object(
        key: &SigningKey,
        board: ArtifactHash,
        game_post: ArtifactHash,
        role: &str,
        turn_index: u64,
        prev_turn: Option<ArtifactHash>,
        action: &str,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::GameTurn(GameTurn {
                board,
                game_post,
                role: role.to_owned(),
                turn_index,
                prev_turn,
                fork_ref: None,
                action: serde_json::Value::String(action.to_owned()),
            }),
        )
        .unwrap()
    }

    fn wasm_game_post_object(
        author: &SigningKey,
        opponent: &SigningKey,
        board: ArtifactHash,
    ) -> SignedBoardObject {
        wasm_game_post_object_with_genesis(author, opponent, board, "---------")
    }

    fn wasm_game_post_object_with_genesis(
        author: &SigningKey,
        opponent: &SigningKey,
        board: ArtifactHash,
        genesis: &str,
    ) -> SignedBoardObject {
        let participants = vec![
            GameParticipant {
                author: hex_encode(&author.verifying_key().to_bytes()),
                role: "x".to_owned(),
            },
            GameParticipant {
                author: hex_encode(&opponent.verifying_key().to_bytes()),
                role: "o".to_owned(),
            },
        ];
        SignedBoardObject::sign(
            author,
            1,
            "",
            BoardObject::GamePost(GamePost {
                board,
                package: GamePackageRef {
                    hash: ArtifactHash::new(sha256_hex(TIC_TAC_TOE_WASM)).unwrap(),
                    version: "0.1.0".to_owned(),
                },
                mode: GameMode::AsyncTurns,
                configuration: serde_json::json!({"board_size": 3}),
                participants,
                genesis: serde_json::json!(genesis),
            }),
        )
        .unwrap()
    }

    #[test]
    fn game_challenge_acceptance_gates_turns_and_forks_stay_visible() {
        let creator = key(41);
        let opponent = key(42);
        let stranger = key(43);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&creator)).unwrap();
        let game_hash = graph
            .insert(game_post_object(&creator, &opponent, root_hash.clone()))
            .unwrap();

        // No turns before a signed challenge acceptance.
        assert!(
            graph
                .insert(game_turn_object(
                    &creator,
                    root_hash.clone(),
                    game_hash.clone(),
                    "x",
                    0,
                    None,
                    "0",
                ))
                .is_err()
        );
        // The poster cannot accept their own challenge, and a
        // non-participant cannot accept it either.
        assert!(
            graph
                .insert(game_acceptance_object(
                    &creator,
                    1,
                    root_hash.clone(),
                    game_hash.clone(),
                ))
                .is_err()
        );
        assert!(
            graph
                .insert(game_acceptance_object(
                    &stranger,
                    1,
                    root_hash.clone(),
                    game_hash.clone(),
                ))
                .is_err()
        );
        graph
            .insert(game_acceptance_object(
                &opponent,
                1,
                root_hash.clone(),
                game_hash.clone(),
            ))
            .unwrap();
        assert_eq!(graph.game_acceptances(&game_hash).len(), 1);
        // A re-signed acceptance from the same participant is kept as
        // signed history, and peers that receive the two records in
        // opposite orders converge on the same set (Codex P1 on PR
        // #8327: a first-arrival-wins rejection stalls replication).
        let acceptance_a =
            game_acceptance_object(&opponent, 1, root_hash.clone(), game_hash.clone());
        let acceptance_b =
            game_acceptance_object(&opponent, 2, root_hash.clone(), game_hash.clone());
        let mut peer = BoardGraph::default();
        let peer_root = peer.insert(board(&creator)).unwrap();
        assert_eq!(peer_root, root_hash);
        peer.insert(game_post_object(&creator, &opponent, root_hash.clone()))
            .unwrap();
        peer.insert(acceptance_b).unwrap();
        peer.insert(acceptance_a).unwrap();
        graph
            .insert(game_acceptance_object(
                &opponent,
                2,
                root_hash.clone(),
                game_hash.clone(),
            ))
            .unwrap();
        assert_eq!(graph.game_acceptances(&game_hash).len(), 2);
        assert_eq!(peer.game_acceptances(&game_hash).len(), 2);

        // A turn signed by anyone but the author assigned to its role
        // is refused (here the opponent signing for role "x").
        assert!(
            graph
                .insert(game_turn_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    "x",
                    0,
                    None,
                    "0",
                ))
                .is_err()
        );
        // A turn scoped to a different board than its game post is refused.
        let foreign_root = graph.insert(board(&stranger)).unwrap();
        assert!(
            graph
                .insert(game_turn_object(
                    &creator,
                    foreign_root,
                    game_hash.clone(),
                    "x",
                    0,
                    None,
                    "0",
                ))
                .is_err()
        );

        let turn0 = graph
            .insert(game_turn_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                "x",
                0,
                None,
                "0",
            ))
            .unwrap();
        // An index that skips ahead of its prior turn is refused.
        assert!(
            graph
                .insert(game_turn_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    "o",
                    2,
                    Some(turn0.clone()),
                    "4",
                ))
                .is_err()
        );
        let turn1 = graph
            .insert(game_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                "o",
                1,
                Some(turn0.clone()),
                "4",
            ))
            .unwrap();
        // A competing valid turn on the same frontier is a fork: it is
        // accepted and stays visible next to the first branch.
        let fork1 = graph
            .insert(game_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                "o",
                1,
                Some(turn0.clone()),
                "3",
            ))
            .unwrap();
        let heads = graph.game_heads(&game_hash);
        assert_eq!(heads.len(), 2);
        assert!(heads.contains(&turn1));
        assert!(heads.contains(&fork1));
        assert_eq!(graph.game_turns(&game_hash).len(), 3);
        let lineage = graph.game_lineage(&game_hash, &turn1).unwrap();
        assert_eq!(lineage.len(), 2);
        assert_eq!(lineage[0].0, turn0);
        assert_eq!(lineage[1].0, turn1);

        // A turn chaining onto another game's turn is refused.
        let other_participants = vec![
            GameParticipant {
                author: hex_encode(&creator.verifying_key().to_bytes()),
                role: "x".to_owned(),
            },
            GameParticipant {
                author: hex_encode(&opponent.verifying_key().to_bytes()),
                role: "o".to_owned(),
            },
        ];
        let other_game = graph
            .insert(
                SignedBoardObject::sign(
                    &creator,
                    1,
                    "",
                    BoardObject::GamePost(game_post_body(
                        root_hash.clone(),
                        "0.2.0".to_owned(),
                        serde_json::json!({"board_size": 3}),
                        other_participants,
                        serde_json::json!("---------"),
                    )),
                )
                .unwrap(),
            )
            .unwrap();
        assert_ne!(other_game, game_hash);
        graph
            .insert(game_acceptance_object(
                &opponent,
                3,
                root_hash.clone(),
                other_game.clone(),
            ))
            .unwrap();
        assert!(
            graph
                .insert(game_turn_object(
                    &creator,
                    root_hash.clone(),
                    other_game,
                    "x",
                    1,
                    Some(turn0.clone()),
                    "1",
                ))
                .is_err()
        );

        // Tampering with a signed turn breaks receipt, like every
        // other board object.
        let turn_object = graph.get(&turn1).unwrap().clone();
        let mut tampered = turn_object.artifact.clone();
        tampered.body["data"]["action"] = serde_json::Value::String("8".to_owned());
        assert!(SignedBoardObject::from_artifact(tampered).is_err());
    }

    #[test]
    fn game_lineage_replays_deterministically_through_the_pinned_package() {
        let creator = key(51);
        let opponent = key(52);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&creator)).unwrap();
        let game_hash = graph
            .insert(wasm_game_post_object(
                &creator,
                &opponent,
                root_hash.clone(),
            ))
            .unwrap();

        // Replay refuses a game with no signed challenge acceptance.
        assert!(
            graph
                .replay_game_lineage(&game_hash, &game_hash, TIC_TAC_TOE_WASM)
                .is_err()
        );
        graph
            .insert(game_acceptance_object(
                &opponent,
                1,
                root_hash.clone(),
                game_hash.clone(),
            ))
            .unwrap();

        // X takes the top row: 0, 1, 2 against O at 4 and 3.
        let moves = [
            (&creator, "x", "0"),
            (&opponent, "o", "4"),
            (&creator, "x", "1"),
            (&opponent, "o", "3"),
            (&creator, "x", "2"),
        ];
        let mut prev: Option<ArtifactHash> = None;
        for (index, (player, role, action)) in moves.iter().enumerate() {
            prev = Some(
                graph
                    .insert(game_turn_object(
                        player,
                        root_hash.clone(),
                        game_hash.clone(),
                        role,
                        u64::try_from(index).unwrap(),
                        prev,
                        action,
                    ))
                    .unwrap(),
            );
        }
        let head = prev.unwrap();
        let state = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        assert_eq!(state, b"XXXOO----");
        // The same signed lineage and the same bytes replay to the
        // same state, every time.
        let replayed = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        assert_eq!(replayed, state);

        // Bytes that do not hash to the pinned package are refused:
        // a different build is a different game.
        assert!(
            graph
                .replay_game_lineage(&game_hash, &head, b"not the pinned package")
                .is_err()
        );
    }

    /// Build a wasm tic-tac-toe graph with an accepted challenge and
    /// one alternating lineage over `actions` (even indices are X,
    /// odd are O), returning the graph, the game hash, and the head.
    fn wasm_game_lineage(actions: &[&str]) -> (BoardGraph, ArtifactHash, ArtifactHash) {
        wasm_game_lineage_with_genesis(actions, "---------")
    }

    fn wasm_game_lineage_with_genesis(
        actions: &[&str],
        genesis: &str,
    ) -> (BoardGraph, ArtifactHash, ArtifactHash) {
        let creator = key(71);
        let opponent = key(72);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&creator)).unwrap();
        let game_hash = graph
            .insert(wasm_game_post_object_with_genesis(
                &creator,
                &opponent,
                root_hash.clone(),
                genesis,
            ))
            .unwrap();
        graph
            .insert(game_acceptance_object(
                &opponent,
                1,
                root_hash.clone(),
                game_hash.clone(),
            ))
            .unwrap();
        let mut prev: Option<ArtifactHash> = None;
        for (index, action) in actions.iter().enumerate() {
            let (player, role) = if index % 2 == 0 {
                (&creator, "x")
            } else {
                (&opponent, "o")
            };
            prev = Some(
                graph
                    .insert(game_turn_object(
                        player,
                        root_hash.clone(),
                        game_hash.clone(),
                        role,
                        u64::try_from(index).unwrap(),
                        prev,
                        action,
                    ))
                    .unwrap(),
            );
        }
        (graph, game_hash, prev.unwrap())
    }

    fn game_save_point_object(
        key: &SigningKey,
        board: ArtifactHash,
        game_post: ArtifactHash,
        package: GamePackageRef,
        frontier: ArtifactHash,
        state_hash: ArtifactHash,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::GameSavePoint(GameSavePoint {
                board,
                game_post,
                package,
                frontier,
                state_hash,
            }),
        )
        .unwrap()
    }

    fn game_handoff_object(
        key: &SigningKey,
        board: ArtifactHash,
        game_post: ArtifactHash,
        save_point: ArtifactHash,
        recipient: String,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::GameHandoff(GameHandoff {
                board,
                game_post,
                save_point,
                recipient,
                terms: serde_json::json!({"mode": "succession"}),
            }),
        )
        .unwrap()
    }

    fn game_handoff_acceptance_object(
        key: &SigningKey,
        board: ArtifactHash,
        game_post: ArtifactHash,
        handoff: ArtifactHash,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::GameHandoffAcceptance(GameHandoffAcceptance {
                board,
                game_post,
                handoff,
            }),
        )
        .unwrap()
    }

    fn game_fork_object(
        key: &SigningKey,
        board: ArtifactHash,
        game_post: ArtifactHash,
        save_point: ArtifactHash,
        participants: Vec<GameParticipant>,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::GameFork(GameFork {
                board,
                game_post,
                save_point,
                participants,
            }),
        )
        .unwrap()
    }

    fn game_fork_turn_object(
        key: &SigningKey,
        board: ArtifactHash,
        game_post: ArtifactHash,
        fork_ref: ArtifactHash,
        role: &str,
        turn_index: u64,
        prev_turn: Option<ArtifactHash>,
        action: &str,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            key,
            1,
            "",
            BoardObject::GameTurn(GameTurn {
                board,
                game_post,
                role: role.to_owned(),
                turn_index,
                prev_turn,
                fork_ref: Some(fork_ref),
                action: serde_json::Value::String(action.to_owned()),
            }),
        )
        .unwrap()
    }

    fn fork_participants(first: &SigningKey, second: &SigningKey) -> Vec<GameParticipant> {
        vec![
            GameParticipant {
                author: hex_encode(&first.verifying_key().to_bytes()),
                role: "x".to_owned(),
            },
            GameParticipant {
                author: hex_encode(&second.verifying_key().to_bytes()),
                role: "o".to_owned(),
            },
        ]
    }

    /// Build a three-participant game (roles x / o / reserve) with an
    /// accepted challenge and two turns played (x, then o), returning
    /// the graph, the board and game hashes, both turn hashes, the
    /// game post body, and the three participant keys.
    fn three_participant_game() -> (
        BoardGraph,
        ArtifactHash,
        ArtifactHash,
        ArtifactHash,
        ArtifactHash,
        GamePost,
        SigningKey,
        SigningKey,
        SigningKey,
    ) {
        let creator = key(81);
        let opponent = key(82);
        let third = key(83);
        let mut graph = BoardGraph::default();
        let root_hash = graph.insert(board(&creator)).unwrap();
        let participants = vec![
            GameParticipant {
                author: hex_encode(&creator.verifying_key().to_bytes()),
                role: "x".to_owned(),
            },
            GameParticipant {
                author: hex_encode(&opponent.verifying_key().to_bytes()),
                role: "o".to_owned(),
            },
            GameParticipant {
                author: hex_encode(&third.verifying_key().to_bytes()),
                role: "reserve".to_owned(),
            },
        ];
        let post_body = game_post_body(
            root_hash.clone(),
            "0.1.0".to_owned(),
            serde_json::json!({"board_size": 3}),
            participants,
            serde_json::json!("---------"),
        );
        let game_hash = graph
            .insert(
                SignedBoardObject::sign(&creator, 1, "", BoardObject::GamePost(post_body.clone()))
                    .unwrap(),
            )
            .unwrap();
        graph
            .insert(game_acceptance_object(
                &opponent,
                1,
                root_hash.clone(),
                game_hash.clone(),
            ))
            .unwrap();
        let turn0 = graph
            .insert(game_turn_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                "x",
                0,
                None,
                "0",
            ))
            .unwrap();
        let turn1 = graph
            .insert(game_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                "o",
                1,
                Some(turn0.clone()),
                "4",
            ))
            .unwrap();
        (
            graph, root_hash, game_hash, turn0, turn1, post_body, creator, opponent, third,
        )
    }

    #[test]
    fn game_save_handoff_accept_and_continue_selects_main_line() {
        // Oracle: issue #6161 acceptance — a tic-tac-toe line can be
        // saved, handed to a named recipient, accepted by that
        // recipient, and continued; the accepted chain selects the
        // social main line and handoff alternatives are kept.
        let (mut graph, root_hash, game_hash, _turn0, turn1, post_body, creator, opponent, third) =
            three_participant_game();
        let state_hash = ArtifactHash::new("a".repeat(64)).unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                post_body.package.clone(),
                turn1.clone(),
                state_hash,
            ))
            .unwrap();
        assert_eq!(graph.game_save_points(&game_hash).len(), 1);

        // The saver hands the line to the third participant, and —
        // as a competing alternative that must be kept — to the
        // opponent as well.
        let handoff_hash = graph
            .insert(game_handoff_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                save_hash.clone(),
                hex_encode(&third.verifying_key().to_bytes()),
            ))
            .unwrap();
        graph
            .insert(game_handoff_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                save_hash.clone(),
                hex_encode(&opponent.verifying_key().to_bytes()),
            ))
            .unwrap();
        assert_eq!(graph.game_handoffs(&game_hash).len(), 2);
        assert_eq!(graph.game_main_line_head(&game_hash), None);

        // Only the accepted handoff selects the main line.
        graph
            .insert(game_handoff_acceptance_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                handoff_hash.clone(),
            ))
            .unwrap();
        assert_eq!(graph.game_handoff_acceptances(&handoff_hash).len(), 1);
        assert_eq!(graph.game_main_line_head(&game_hash), Some(turn1.clone()));

        // The recipient continues the line: turn 2 belongs to the
        // third participant by rotation.
        let turn2 = graph
            .insert(game_turn_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                "reserve",
                2,
                Some(turn1),
                "1",
            ))
            .unwrap();
        assert_eq!(graph.game_heads(&game_hash), vec![turn2]);
    }

    #[test]
    fn game_save_point_handoff_and_fork_rejections() {
        // Oracle: issue #6161 acceptance — bad signer, package /
        // version mismatch, bad frontier, and unresolvable
        // save-point references are each rejected.
        let (mut graph, root_hash, game_hash, _turn0, turn1, post_body, creator, opponent, third) =
            three_participant_game();
        let outsider = key(89);
        let state_hash = ArtifactHash::new("a".repeat(64)).unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                post_body.package.clone(),
                turn1.clone(),
                state_hash.clone(),
            ))
            .unwrap();

        // Bad signer: a non-participant cannot save the line.
        assert!(
            graph
                .insert(game_save_point_object(
                    &outsider,
                    root_hash.clone(),
                    game_hash.clone(),
                    post_body.package.clone(),
                    turn1.clone(),
                    ArtifactHash::new("b".repeat(64)).unwrap(),
                ))
                .is_err()
        );

        // Package mismatch: a different version is a different game.
        let err = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                GamePackageRef {
                    hash: post_body.package.hash.clone(),
                    version: "9.9.9".to_owned(),
                },
                turn1.clone(),
                ArtifactHash::new("b".repeat(64)).unwrap(),
            ))
            .unwrap_err();
        assert!(err.to_string().contains("pinned package"), "{err}");

        // Package mismatch: a different package hash, same version.
        assert!(
            graph
                .insert(game_save_point_object(
                    &creator,
                    root_hash.clone(),
                    game_hash.clone(),
                    GamePackageRef {
                        hash: ArtifactHash::new("b".repeat(64)).unwrap(),
                        version: post_body.package.version.clone(),
                    },
                    turn1.clone(),
                    ArtifactHash::new("b".repeat(64)).unwrap(),
                ))
                .is_err()
        );

        // Bad frontier: the cited frontier is not a game turn.
        assert!(
            graph
                .insert(game_save_point_object(
                    &creator,
                    root_hash.clone(),
                    game_hash.clone(),
                    post_body.package.clone(),
                    game_hash.clone(),
                    ArtifactHash::new("b".repeat(64)).unwrap(),
                ))
                .is_err()
        );

        // Bad frontier: the cited frontier does not resolve at all.
        assert!(
            graph
                .insert(game_save_point_object(
                    &creator,
                    root_hash.clone(),
                    game_hash.clone(),
                    post_body.package.clone(),
                    ArtifactHash::new("c".repeat(64)).unwrap(),
                    ArtifactHash::new("b".repeat(64)).unwrap(),
                ))
                .is_err()
        );

        // A handoff must be signed by the save point's author.
        assert!(
            graph
                .insert(game_handoff_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    save_hash.clone(),
                    hex_encode(&third.verifying_key().to_bytes()),
                ))
                .is_err()
        );

        // A handoff citing an unresolvable save point is rejected.
        assert!(
            graph
                .insert(game_handoff_object(
                    &creator,
                    root_hash.clone(),
                    game_hash.clone(),
                    ArtifactHash::new("d".repeat(64)).unwrap(),
                    hex_encode(&third.verifying_key().to_bytes()),
                ))
                .is_err()
        );

        // A valid handoff, accepted by the wrong participant, is
        // rejected: only the named recipient may accept.
        let handoff_hash = graph
            .insert(game_handoff_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                save_hash.clone(),
                hex_encode(&third.verifying_key().to_bytes()),
            ))
            .unwrap();
        assert!(
            graph
                .insert(game_handoff_acceptance_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    handoff_hash,
                ))
                .is_err()
        );

        // A fork citing an unresolvable save point is rejected.
        assert!(
            graph
                .insert(game_fork_object(
                    &outsider,
                    root_hash.clone(),
                    game_hash.clone(),
                    ArtifactHash::new("f".repeat(64)).unwrap(),
                    fork_participants(&outsider, &creator),
                ))
                .is_err()
        );
    }

    #[test]
    fn game_fork_from_save_point_replays_both_lines_independently() {
        // Oracle: issue #6161 acceptance + design #2054 section 4
        // and Codex P1 on PR #8552 — a third author can fork from a
        // save point into an alternate instance with its own
        // participant resolution; both source and fork replay
        // independently with attribution intact; valid concurrent
        // forks remain visible.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0", "4"]);
        let creator = key(71);
        let opponent = key(72);
        let third = key(73);
        let fourth = key(74);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();

        // The saver records the real replayed state hash at the
        // frontier, recomputed from the same lineage and bytes.
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package,
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();

        // Two third authors fork from the same save point, each
        // seating an instance of their own; both records visible.
        let fork_hash = graph
            .insert(game_fork_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                save_hash.clone(),
                fork_participants(&third, &opponent),
            ))
            .unwrap();
        graph
            .insert(game_fork_object(
                &fourth,
                root_hash.clone(),
                game_hash.clone(),
                save_hash.clone(),
                fork_participants(&fourth, &opponent),
            ))
            .unwrap();
        let forks = graph.game_forks(&game_hash);
        assert_eq!(forks.len(), 2);
        assert!(forks.iter().all(|(_, fork)| fork.save_point == save_hash));

        // The forker cannot sign turns in the base line, and a fork
        // turn signed out of the fork's own rotation is rejected.
        assert!(
            graph
                .insert(game_turn_object(
                    &third,
                    root_hash.clone(),
                    game_hash.clone(),
                    "x",
                    2,
                    Some(head.clone()),
                    "1",
                ))
                .is_err()
        );
        assert!(
            graph
                .insert(game_fork_turn_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    fork_hash.clone(),
                    "o",
                    2,
                    Some(head.clone()),
                    "3",
                ))
                .is_err()
        );

        // The fork instance proceeds under its own participants:
        // fork position 0 is the forker (as x), position 1 the
        // opponent (as o).
        let fork_turn1 = graph
            .insert(game_fork_turn_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash.clone(),
                "x",
                2,
                Some(head.clone()),
                "1",
            ))
            .unwrap();
        let fork_head = graph
            .insert(game_fork_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash.clone(),
                "o",
                3,
                Some(fork_turn1),
                "8",
            ))
            .unwrap();

        // The source line continues in parallel under the game
        // post's own rotation, unchanged by the fork.
        let source_head = graph
            .insert(game_turn_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                "x",
                2,
                Some(head),
                "2",
            ))
            .unwrap();
        let heads = graph.game_heads(&game_hash);
        assert_eq!(heads.len(), 2);
        assert!(heads.contains(&fork_head) && heads.contains(&source_head));

        // Both lines replay independently and deterministically,
        // to different states.
        let fork_state = graph
            .replay_game_lineage(&game_hash, &fork_head, TIC_TAC_TOE_WASM)
            .unwrap();
        let source_state = graph
            .replay_game_lineage(&game_hash, &source_head, TIC_TAC_TOE_WASM)
            .unwrap();
        assert_ne!(fork_state, source_state);
        assert_eq!(
            graph
                .replay_game_lineage(&game_hash, &fork_head, TIC_TAC_TOE_WASM)
                .unwrap(),
            fork_state
        );

        // Attribution is intact: the fork lineage is the source
        // line's signed turns plus the fork instance's turns, and
        // the fork record cites a save point authored by the
        // source player.
        let lineage = graph.game_lineage(&game_hash, &fork_head).unwrap();
        let authors: Vec<String> = lineage
            .iter()
            .map(|(hash, _)| graph.get(hash).unwrap().artifact.author.clone())
            .collect();
        assert_eq!(
            authors,
            vec![
                hex_encode(&creator.verifying_key().to_bytes()),
                hex_encode(&opponent.verifying_key().to_bytes()),
                hex_encode(&third.verifying_key().to_bytes()),
                hex_encode(&opponent.verifying_key().to_bytes()),
            ]
        );
        assert_eq!(
            graph.get(&save_hash).unwrap().artifact.author,
            hex_encode(&creator.verifying_key().to_bytes())
        );
    }

    #[test]
    fn game_handoff_acceptance_begins_the_recipients_leg() {
        // Oracle: design #2054 section 4 + Codex P1 on PR #8552 —
        // after turn 1 of a two-player game the creator hands the
        // save to the opponent; once the opponent accepts, the
        // opponent begins their leg: they may sign the next turn
        // under the seat role rotation assigns (here the creator's
        // x seat), and the line replays with the recipient
        // substituted into that seat. Before the acceptance, the
        // same signature is rejected by rotation alone.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0", "4"]);
        let creator = key(71);
        let opponent = key(72);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package,
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();

        // Before any handoff, the opponent signing turn 2 under
        // the creator's seat role is rejected.
        assert!(
            graph
                .insert(game_turn_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    "x",
                    2,
                    Some(head.clone()),
                    "1",
                ))
                .is_err()
        );

        let handoff_hash = graph
            .insert(game_handoff_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                save_hash.clone(),
                hex_encode(&opponent.verifying_key().to_bytes()),
            ))
            .unwrap();
        graph
            .insert(game_handoff_acceptance_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                handoff_hash,
            ))
            .unwrap();

        // The leg plays the seat, not the recipient's old seat:
        // under their own role the signature is still rejected.
        assert!(
            graph
                .insert(game_turn_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    "o",
                    2,
                    Some(head.clone()),
                    "1",
                ))
                .is_err()
        );
        let leg_head = graph
            .insert(game_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                "x",
                2,
                Some(head.clone()),
                "1",
            ))
            .unwrap();

        // The leg replays deterministically with the recipient in
        // the seat, and the accepted save's frontier is the social
        // main line's head.
        let leg_state = graph
            .replay_game_lineage(&game_hash, &leg_head, TIC_TAC_TOE_WASM)
            .unwrap();
        assert_eq!(
            graph
                .replay_game_lineage(&game_hash, &leg_head, TIC_TAC_TOE_WASM)
                .unwrap(),
            leg_state
        );
        assert_eq!(graph.game_main_line_head(&game_hash), Some(head));
    }

    #[test]
    fn game_handoff_chain_keeps_earlier_legs_authorized() {
        // Oracle: Codex P1 on PR #8552 — authorization is monotone
        // in the evidence. A hands the turn-1 save to B; B legs
        // turn 2 (x seat) and plays turn 3 (own o seat, rotation),
        // saves turn 2, and hands that save back to A, who accepts.
        // B's turn 4 under the x seat must stay authorized by the
        // earlier handoff even though a deeper accepted save now
        // names A — otherwise the same signed set would verdict
        // differently by arrival order.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0", "4"]);
        let creator = key(71);
        let opponent = key(72);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save1 = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package.clone(),
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        let handoff1 = graph
            .insert(game_handoff_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                save1,
                hex_encode(&opponent.verifying_key().to_bytes()),
            ))
            .unwrap();
        graph
            .insert(game_handoff_acceptance_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                handoff1,
            ))
            .unwrap();
        let turn2 = graph
            .insert(game_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                "x",
                2,
                Some(head),
                "1",
            ))
            .unwrap();
        // B saves turn 2 and hands that save back to A; A accepts.
        let snapshot2 = graph
            .replay_game_lineage(&game_hash, &turn2, TIC_TAC_TOE_WASM)
            .unwrap();
        let save2 = graph
            .insert(game_save_point_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                package,
                turn2.clone(),
                ArtifactHash::new(sha256_hex(&snapshot2)).unwrap(),
            ))
            .unwrap();
        let handoff2 = graph
            .insert(game_handoff_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                save2,
                hex_encode(&creator.verifying_key().to_bytes()),
            ))
            .unwrap();
        graph
            .insert(game_handoff_acceptance_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                handoff2,
            ))
            .unwrap();
        let turn3 = graph
            .insert(game_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                "o",
                3,
                Some(turn2),
                "8",
            ))
            .unwrap();
        // The contested turn: B signs turn 4 under the x seat,
        // authorized by the FIRST handoff; the deeper accepted
        // save naming A must not revoke it.
        let turn4 = graph
            .insert(game_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                "x",
                4,
                Some(turn3),
                "2",
            ))
            .unwrap();
        graph
            .replay_game_lineage(&game_hash, &turn4, TIC_TAC_TOE_WASM)
            .unwrap();
    }

    #[test]
    fn game_fork_preserves_the_saved_states_seat_phase() {
        // Oracle: Codex P1 on PR #8552 — a save taken mid-rotation
        // (after turn 0: X has moved, the saved state says O moves
        // next) forks with seating phase-aligned to the absolute
        // turn index: fork turn 1 belongs to fork participant 1
        // (the o seat), not to a reset participant 0.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0"]);
        let creator = key(71);
        let opponent = key(72);
        let third = key(73);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package,
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        let fork_hash = graph
            .insert(game_fork_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                save_hash,
                fork_participants(&third, &opponent),
            ))
            .unwrap();
        // Turn 1 is the o seat by absolute phase: the forker
        // signing it as x is rejected even though the forker
        // holds position 0 of the fork's own list.
        assert!(
            graph
                .insert(game_fork_turn_object(
                    &third,
                    root_hash.clone(),
                    game_hash.clone(),
                    fork_hash.clone(),
                    "x",
                    1,
                    Some(head.clone()),
                    "1",
                ))
                .is_err()
        );
        let fork_turn = graph
            .insert(game_fork_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash.clone(),
                "o",
                1,
                Some(head),
                "4",
            ))
            .unwrap();
        // Turn 2 returns to position 0: the forker, as x.
        let fork_head = graph
            .insert(game_fork_turn_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash,
                "x",
                2,
                Some(fork_turn),
                "1",
            ))
            .unwrap();
        let state = graph
            .replay_game_lineage(&game_hash, &fork_head, TIC_TAC_TOE_WASM)
            .unwrap();
        assert_eq!(
            graph
                .replay_game_lineage(&game_hash, &fork_head, TIC_TAC_TOE_WASM)
                .unwrap(),
            state
        );
    }

    #[test]
    fn game_save_point_on_a_fork_line_uses_fork_participants() {
        // Oracle: Codex P2 on PR #8552 — a fork's own participants
        // can save their continuation (so it can be forked again)
        // without an original participant; conversely a base-line
        // participant who holds no fork seat cannot save the fork
        // line.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0"]);
        let creator = key(71);
        let opponent = key(72);
        let third = key(73);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package.clone(),
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        let fork_hash = graph
            .insert(game_fork_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                save_hash,
                fork_participants(&third, &opponent),
            ))
            .unwrap();
        let fork_turn = graph
            .insert(game_fork_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash.clone(),
                "o",
                1,
                Some(head),
                "4",
            ))
            .unwrap();
        let fork_head = graph
            .insert(game_fork_turn_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash,
                "x",
                2,
                Some(fork_turn),
                "1",
            ))
            .unwrap();
        let fork_snapshot = graph
            .replay_game_lineage(&game_hash, &fork_head, TIC_TAC_TOE_WASM)
            .unwrap();
        let fork_state = ArtifactHash::new(sha256_hex(&fork_snapshot)).unwrap();
        // The fork's own participant saves the fork line.
        graph
            .insert(game_save_point_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                package.clone(),
                fork_head.clone(),
                fork_state.clone(),
            ))
            .unwrap();
        // The base-line creator holds no fork seat: rejected.
        assert!(
            graph
                .insert(game_save_point_object(
                    &creator,
                    root_hash.clone(),
                    game_hash.clone(),
                    package,
                    fork_head,
                    fork_state,
                ))
                .is_err()
        );
    }

    #[test]
    fn game_fork_must_preserve_the_saved_seat_layout() {
        // Oracle: Codex P1 on PR #8552 — a fork that changes the
        // seat count defeats phase-aligned seating: a one-seat
        // fork [C:x] of a two-seat game would authorize C at every
        // index while replay executes odd moves as O. Fork
        // assignments must keep the saved instance's seats and
        // per-seat roles; only authors may change.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0"]);
        let creator = key(71);
        let third = key(73);
        let fourth = key(74);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package,
                head,
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        let layout = fork_participants(&third, &fourth);
        // One seat instead of two: rejected.
        assert!(
            graph
                .insert(game_fork_object(
                    &third,
                    root_hash.clone(),
                    game_hash.clone(),
                    save_hash.clone(),
                    layout.iter().take(1).cloned().collect(),
                ))
                .is_err()
        );
        // Two seats, roles swapped between seats: rejected.
        let mut swapped = layout.clone();
        swapped.reverse();
        assert!(
            graph
                .insert(game_fork_object(
                    &fourth,
                    root_hash.clone(),
                    game_hash.clone(),
                    save_hash.clone(),
                    swapped,
                ))
                .is_err()
        );
        // Same seats and roles, different authors: accepted.
        graph
            .insert(game_fork_object(
                &third, root_hash, game_hash, save_hash, layout,
            ))
            .unwrap();
    }

    #[test]
    fn game_handoff_leg_inside_a_fork_instance() {
        // Oracle: Codex P2 on PR #8552 — accepted handoffs govern
        // fork instances too. Fork [C:x, B:o] (B also an original
        // participant): B plays fork turn 1, C saves it, hands the
        // save to B; once B accepts, B may sign fork turn 2 under
        // the x seat, and replay executes it as X (participant 1).
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0"]);
        let creator = key(71);
        let opponent = key(72);
        let third = key(73);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package.clone(),
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        let fork_hash = graph
            .insert(game_fork_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                save_hash,
                fork_participants(&third, &opponent),
            ))
            .unwrap();
        let fork_turn1 = graph
            .insert(game_fork_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash.clone(),
                "o",
                1,
                Some(head),
                "4",
            ))
            .unwrap();
        let fork_snapshot = graph
            .replay_game_lineage(&game_hash, &fork_turn1, TIC_TAC_TOE_WASM)
            .unwrap();
        let fork_save = graph
            .insert(game_save_point_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                package.clone(),
                fork_turn1.clone(),
                ArtifactHash::new(sha256_hex(&fork_snapshot)).unwrap(),
            ))
            .unwrap();
        let handoff = graph
            .insert(game_handoff_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                fork_save,
                hex_encode(&opponent.verifying_key().to_bytes()),
            ))
            .unwrap();
        // Before acceptance the leg does not exist: B signing the
        // x seat is rejected.
        assert!(
            graph
                .insert(game_fork_turn_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    fork_hash.clone(),
                    "x",
                    2,
                    Some(fork_turn1.clone()),
                    "1",
                ))
                .is_err()
        );
        graph
            .insert(game_handoff_acceptance_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                handoff,
            ))
            .unwrap();
        let fork_turn2 = graph
            .insert(game_fork_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash,
                "x",
                2,
                Some(fork_turn1),
                "1",
            ))
            .unwrap();
        graph
            .replay_game_lineage(&game_hash, &fork_turn2, TIC_TAC_TOE_WASM)
            .unwrap();
    }

    #[test]
    fn game_handoff_recipient_resolves_against_the_saved_instance() {
        // Oracle: Codex P2 on PR #8552 — for an independently
        // seated fork [C:x, D:o] of [A:x, B:o], C can hand a
        // fork-line save to D (a fork participant), while B — an
        // original participant with no fork seat — is not a valid
        // recipient for that save; symmetrically a base-line save
        // cannot be handed to a non-participant of the post.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0"]);
        let creator = key(71);
        let opponent = key(72);
        let third = key(73);
        let fourth = key(74);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package.clone(),
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        // Base-line save to a non-participant of the post: rejected.
        assert!(
            graph
                .insert(game_handoff_object(
                    &creator,
                    root_hash.clone(),
                    game_hash.clone(),
                    save_hash.clone(),
                    hex_encode(&third.verifying_key().to_bytes()),
                ))
                .is_err()
        );
        let fork_hash = graph
            .insert(game_fork_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                save_hash,
                fork_participants(&third, &fourth),
            ))
            .unwrap();
        let fork_turn = graph
            .insert(game_fork_turn_object(
                &fourth,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash.clone(),
                "o",
                1,
                Some(head),
                "4",
            ))
            .unwrap();
        let fork_snapshot = graph
            .replay_game_lineage(&game_hash, &fork_turn, TIC_TAC_TOE_WASM)
            .unwrap();
        let fork_save = graph
            .insert(game_save_point_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                package,
                fork_turn,
                ArtifactHash::new(sha256_hex(&fork_snapshot)).unwrap(),
            ))
            .unwrap();
        // Fork-line save to the fork's other participant: accepted.
        graph
            .insert(game_handoff_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                fork_save.clone(),
                hex_encode(&fourth.verifying_key().to_bytes()),
            ))
            .unwrap();
        // Fork-line save to an original participant with no fork
        // seat: rejected.
        assert!(
            graph
                .insert(game_handoff_object(
                    &third,
                    root_hash.clone(),
                    game_hash.clone(),
                    fork_save,
                    hex_encode(&opponent.verifying_key().to_bytes()),
                ))
                .is_err()
        );
    }

    #[test]
    fn game_source_line_handoff_does_not_govern_a_fork() {
        // Oracle: Codex P2 on PR #8552 — instance scoping of leg
        // authorization. A-to-B handoff accepted on the source
        // line; a fork [C:x, B:o] cites that save. B must not sign
        // fork turn 2 as X on the source handoff's authority: the
        // fork seat belongs to C, and the fork has no handoff of
        // its own.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0", "4"]);
        let creator = key(71);
        let opponent = key(72);
        let third = key(73);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package,
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        let handoff = graph
            .insert(game_handoff_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                save_hash.clone(),
                hex_encode(&opponent.verifying_key().to_bytes()),
            ))
            .unwrap();
        graph
            .insert(game_handoff_acceptance_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                handoff,
            ))
            .unwrap();
        let fork_hash = graph
            .insert(game_fork_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                save_hash,
                fork_participants(&third, &opponent),
            ))
            .unwrap();
        assert!(
            graph
                .insert(game_fork_turn_object(
                    &opponent,
                    root_hash.clone(),
                    game_hash.clone(),
                    fork_hash.clone(),
                    "x",
                    2,
                    Some(head.clone()),
                    "1",
                ))
                .is_err()
        );
        let fork_turn = graph
            .insert(game_fork_turn_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash,
                "x",
                2,
                Some(head),
                "1",
            ))
            .unwrap();
        graph
            .replay_game_lineage(&game_hash, &fork_turn, TIC_TAC_TOE_WASM)
            .unwrap();
    }

    #[test]
    fn game_base_turn_cannot_build_on_a_fork_turn() {
        // Oracle: Codex P2 on PR #8552 — omitting fork_ref must
        // not let an original participant append a base-line turn
        // directly onto a fork turn, continuing the fork's state
        // under the original roster without an explicit fork or
        // handoff.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0"]);
        let creator = key(71);
        let third = key(73);
        let fourth = key(74);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package,
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        let fork_hash = graph
            .insert(game_fork_object(
                &third,
                root_hash.clone(),
                game_hash.clone(),
                save_hash,
                fork_participants(&third, &fourth),
            ))
            .unwrap();
        let fork_turn = graph
            .insert(game_fork_turn_object(
                &fourth,
                root_hash.clone(),
                game_hash.clone(),
                fork_hash,
                "o",
                1,
                Some(head),
                "4",
            ))
            .unwrap();
        // The original creator appending turn 2 as a base-line
        // turn on top of the fork turn: rejected.
        assert!(
            graph
                .insert(game_turn_object(
                    &creator,
                    root_hash,
                    game_hash,
                    "x",
                    2,
                    Some(fork_turn),
                    "1",
                ))
                .is_err()
        );
    }

    #[test]
    fn game_leg_evidence_travels_with_a_board_fork_citation() {
        // Oracle: Codex P2 on PR #8552 — a BoardFork citing a
        // handoff-authorized turn in its source frontier pulls the
        // save point, handoff, and handoff acceptance that
        // authorize it into the fork board's replication set, so
        // replication into a fresh graph can insert the cited
        // turn instead of stalling on missing evidence.
        let (mut graph, game_hash, head) = wasm_game_lineage(&["0", "4"]);
        let creator = key(71);
        let opponent = key(72);
        let forker = key(93);
        let BoardObject::GamePost(post_body) = &graph.get(&game_hash).unwrap().object else {
            panic!("game post body expected");
        };
        let root_hash = post_body.board.clone();
        let package = post_body.package.clone();
        let snapshot = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap();
        let save_hash = graph
            .insert(game_save_point_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                package,
                head.clone(),
                ArtifactHash::new(sha256_hex(&snapshot)).unwrap(),
            ))
            .unwrap();
        let handoff_hash = graph
            .insert(game_handoff_object(
                &creator,
                root_hash.clone(),
                game_hash.clone(),
                save_hash.clone(),
                hex_encode(&opponent.verifying_key().to_bytes()),
            ))
            .unwrap();
        let acceptance_hash = graph
            .insert(game_handoff_acceptance_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                handoff_hash.clone(),
            ))
            .unwrap();
        let leg_turn = graph
            .insert(game_turn_object(
                &opponent,
                root_hash.clone(),
                game_hash.clone(),
                "x",
                2,
                Some(head),
                "1",
            ))
            .unwrap();
        let fork_root = graph
            .insert(
                SignedBoardObject::sign(
                    &forker,
                    1,
                    "",
                    BoardObject::Board(Board {
                        board_id: "fork-of-game".to_owned(),
                        forked_from: Some(root_hash.clone()),
                        theme_ref: None,
                    }),
                )
                .unwrap(),
            )
            .unwrap();
        let fork_record = graph
            .insert(
                SignedBoardObject::sign(
                    &forker,
                    1,
                    "",
                    BoardObject::BoardFork(BoardFork {
                        source_board: root_hash.clone(),
                        fork_board: fork_root.clone(),
                        source_frontier: vec![root_hash, leg_turn.clone()],
                    }),
                )
                .unwrap(),
            )
            .unwrap();
        let fork_set = graph.board_object_set(&fork_root);
        assert!(fork_set.contains(&fork_record));
        assert!(fork_set.contains(&leg_turn));
        assert!(fork_set.contains(&save_hash));
        assert!(fork_set.contains(&handoff_hash));
        assert!(fork_set.contains(&acceptance_hash));
        let mut replica = BoardGraph::default();
        let report = replica.replicate_board_from(&graph, &fork_root).unwrap();
        assert_eq!(report.status.state, ReplicaState::Complete);
        assert!(replica.get(&leg_turn).is_some());
    }

    #[test]
    fn game_replay_rejects_actions_the_package_ignores() {
        // Codex P1 on PR #8327, its exact example: X signs turn 0
        // with the out-of-range action "9" (the package ignores it,
        // plays stays 0), then O signs turn 1 with "4". Before the
        // transition-validity check, replay consumed both turns and
        // O's action placed an X — signed roles drifted from
        // executed moves.
        let (graph, game_hash, head) = wasm_game_lineage(&["9", "4"]);
        let error = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap_err();
        assert!(
            error.to_string().contains("was ignored by the package"),
            "{error:#}"
        );

        // An occupied-cell action is likewise a no-op for the
        // package: X at 0, then O at 0 again.
        let (graph, game_hash, head) = wasm_game_lineage(&["0", "0"]);
        assert!(
            graph
                .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
                .is_err()
        );

        // A turn signed after the game is already won is ignored by
        // the package and must not extend the replayed lineage.
        let (graph, game_hash, head) = wasm_game_lineage(&["0", "4", "1", "3", "2", "5"]);
        assert!(
            graph
                .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
                .is_err()
        );

        // Control: the same lineage without the post-win turn
        // replays to the winning board.
        let (graph, game_hash, head) = wasm_game_lineage(&["0", "4", "1", "3", "2"]);
        assert_eq!(
            graph
                .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
                .unwrap(),
            b"XXXOO----"
        );
    }

    #[test]
    fn game_replay_rejects_genesis_that_is_not_the_package_initial_state() {
        // Codex P2 on PR #8327, its exact example: genesis "X--------"
        // restores with plays = 1, so the package hands the next move
        // to O — but graph validation requires turn 0 to be signed by
        // the X participant, whose action "1" would then place O's
        // mark. Replay refuses the lineage instead of accepting a
        // move executed under the wrong signer.
        let (graph, game_hash, head) = wasm_game_lineage_with_genesis(&["1"], "X--------");
        let error = graph
            .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("genesis state is not the package's initial state"),
            "{error:#}"
        );

        // A mid-game genesis whose mark parity happens to agree with
        // the opening signer is refused too: alignment of a restored
        // state with the signed schedule is package-internal and
        // cannot be verified through the sandbox interface, so no
        // non-initial genesis is replayed under a signed schedule
        // defined from the initial state.
        let (graph, game_hash, head) = wasm_game_lineage_with_genesis(&["2"], "XX-------");
        assert!(
            graph
                .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
                .is_err()
        );

        // Control: the initial-state genesis replays the same signed
        // opening turn to X's mark in cell 1.
        let (graph, game_hash, head) = wasm_game_lineage_with_genesis(&["1"], "---------");
        assert_eq!(
            graph
                .replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM)
                .unwrap(),
            b"-X-------"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]

        /// Invariant (oracle: `tic_tac_toe_oracle`, plus the
        /// package's initial snapshot "---------"): for any genesis
        /// state and any short alternating signed lineage, replay
        /// accepts only when the genesis restores to the package's
        /// initial state and the oracle can execute every action
        /// from it; a non-initial genesis is always refused, so a
        /// restored mid-game state can never execute a signed turn
        /// under the wrong participant.
        #[test]
        fn game_replay_aligns_genesis_with_the_signed_schedule(
            genesis in proptest::sample::select(vec![
                "---------", "X--------", "O--------", "XO-------",
                "XX-------", "XOX------", "XXXOO----", "XOXOXOXOX",
            ]),
            actions in proptest::collection::vec(
                proptest::sample::select(vec![
                    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
                ]),
                1..=4,
            ),
        ) {
            let owned: Vec<String> = actions.iter().map(|a| a.to_string()).collect();
            let refs: Vec<&str> = actions.clone();
            let (graph, game_hash, head) = wasm_game_lineage_with_genesis(&refs, genesis);
            let replayed = graph.replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM);
            if genesis != "---------" {
                prop_assert!(replayed.is_err());
            } else {
                match tic_tac_toe_oracle(&owned) {
                    Ok(expected) => {
                        let state = replayed.unwrap();
                        prop_assert_eq!(String::from_utf8(state).unwrap(), expected);
                    }
                    Err(()) => {
                        prop_assert!(replayed.is_err());
                    }
                }
            }
        }
    }

    /// The tic-tac-toe rules, restated as the property oracle: a
    /// lineage is executable exactly when every action, at its
    /// position, names an empty in-range cell in a game not yet won;
    /// the final board is then fully determined. Oracle for
    /// `game_replay_matches_ttt_oracle_for_arbitrary_action_sequences`.
    fn tic_tac_toe_oracle(actions: &[String]) -> Result<String, ()> {
        let mut board = [b'-'; 9];
        let mut plays = 0usize;
        let mut won = false;
        for action in actions {
            let cell: usize = action.parse().map_err(|_| ())?;
            if won || cell > 8 || board[cell] != b'-' {
                return Err(());
            }
            board[cell] = if plays % 2 == 0 { b'X' } else { b'O' };
            plays += 1;
            let lines = [
                [0, 1, 2],
                [3, 4, 5],
                [6, 7, 8],
                [0, 3, 6],
                [1, 4, 7],
                [2, 5, 8],
                [0, 4, 8],
                [2, 4, 6],
            ];
            won = lines.iter().any(|line| {
                line.iter()
                    .all(|&i| board[i] == board[line[0]] && board[i] != b'-')
            });
        }
        Ok(String::from_utf8(board.to_vec()).unwrap())
    }

    proptest! {
        // Each case compiles the pinned wasm package in a fresh
        // sandbox (~seconds), so the case count is kept small; the
        // deterministic test above pins the exact Codex example.
        #![proptest_config(ProptestConfig::with_cases(20))]

        /// Invariant (testing doctrine, oracle: `tic_tac_toe_oracle`):
        /// for any alternating signed action sequence, replay accepts
        /// the lineage iff the oracle can execute every action, and
        /// the replayed board equals the oracle's board — signed
        /// roles never drift from executed moves, whatever invalid or
        /// no-op actions the sequence contains.
        #[test]
        fn game_replay_matches_ttt_oracle_for_arbitrary_action_sequences(
            actions in proptest::collection::vec(
                proptest::sample::select(vec![
                    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "x", "",
                ]),
                1..=9,
            ),
        ) {
            let owned: Vec<String> = actions.iter().map(|a| a.to_string()).collect();
            let refs: Vec<&str> = actions.clone();
            let (graph, game_hash, head) = wasm_game_lineage(&refs);
            let replayed = graph.replay_game_lineage(&game_hash, &head, TIC_TAC_TOE_WASM);
            match tic_tac_toe_oracle(&owned) {
                Ok(expected) => {
                    let state = replayed.unwrap();
                    prop_assert_eq!(String::from_utf8(state).unwrap(), expected);
                }
                Err(()) => {
                    prop_assert!(replayed.is_err());
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// Invariant (oracle: the generated universe's signed object
        /// set, built independently of either peer): for generated
        /// counts of re-signed challenge acceptances, main-lineage
        /// lengths, and an optional competing fork turn, two peers
        /// that start from different subsets of the universe —
        /// different acceptance arrival orders and opposite sides of
        /// the fork — converge after cross-replication on exactly
        /// the universe's object set, with the same game heads and
        /// the same acceptance and turn counts; a fresh third peer
        /// replicating from a converged peer lands on the same set
        /// and heads. (Codex P1 on PR #8327: replication convergence
        /// for game graphs needs generated coverage, not only the
        /// fixed two-acceptance case above.)
        #[test]
        fn game_graph_replication_converges_for_generated_forks_and_acceptances(
            extra_acceptances in 0u64..=2,
            main_len in 1usize..=5,
            fork in proptest::option::of(0usize..5),
            reverse_acceptance_order in proptest::bool::ANY,
        ) {
            let creator = key(81);
            let opponent = key(82);
            // Build the universe as signed objects first, so both
            // peers can start from different subsets of the same set.
            let root_obj = board(&creator);
            let root_hash = root_obj.hash().unwrap();
            let post_obj = game_post_object(&creator, &opponent, root_hash.clone());
            let game_hash = post_obj.hash().unwrap();
            let acceptance_count = 1 + extra_acceptances;
            let mut acceptances: Vec<SignedBoardObject> = (0..acceptance_count)
                .map(|i| {
                    game_acceptance_object(
                        &opponent,
                        i + 1,
                        root_hash.clone(),
                        game_hash.clone(),
                    )
                })
                .collect();
            if reverse_acceptance_order {
                acceptances.reverse();
            }
            // Main lineage: alternating signed turns over distinct
            // actions. The fork (when generated) is a competing turn
            // at `fork % main_len` with a different action.
            let cells = ["0", "4", "1", "3", "2"];
            let mut main_turns: Vec<SignedBoardObject> = Vec::new();
            let mut prev: Option<ArtifactHash> = None;
            for index in 0..main_len {
                let (player, role) = if index % 2 == 0 {
                    (&creator, "x")
                } else {
                    (&opponent, "o")
                };
                let turn = game_turn_object(
                    player,
                    root_hash.clone(),
                    game_hash.clone(),
                    role,
                    u64::try_from(index).unwrap(),
                    prev.clone(),
                    cells[index],
                );
                prev = Some(turn.hash().unwrap());
                main_turns.push(turn);
            }
            let fork_index = fork.map(|f| f % main_len);
            let fork_turn = fork_index.map(|index| {
                let (player, role) = if index % 2 == 0 {
                    (&creator, "x")
                } else {
                    (&opponent, "o")
                };
                let prev = if index == 0 {
                    None
                } else {
                    Some(main_turns[index - 1].hash().unwrap())
                };
                game_turn_object(
                    player,
                    root_hash.clone(),
                    game_hash.clone(),
                    role,
                    u64::try_from(index).unwrap(),
                    prev,
                    "8",
                )
            });

            // The universe (the oracle): every generated object.
            let mut universe = BoardGraph::default();
            universe.insert(root_obj.clone()).unwrap();
            universe.insert(post_obj.clone()).unwrap();
            for acceptance in &acceptances {
                universe.insert(acceptance.clone()).unwrap();
            }
            for turn in &main_turns {
                universe.insert(turn.clone()).unwrap();
            }
            if let Some(fork_turn) = &fork_turn {
                universe.insert(fork_turn.clone()).unwrap();
            }
            let universe_set = universe.board_object_set(&root_hash);
            let expected_heads = if fork_turn.is_some() { 2 } else { 1 };

            // Peer A: the first-arriving acceptance only, plus the
            // whole main lineage.
            let mut peer_a = BoardGraph::default();
            peer_a.insert(root_obj.clone()).unwrap();
            peer_a.insert(post_obj.clone()).unwrap();
            peer_a.insert(acceptances[0].clone()).unwrap();
            for turn in &main_turns {
                peer_a.insert(turn.clone()).unwrap();
            }
            // Peer B: every acceptance (in its own arrival order),
            // the shared prefix the fork builds on, and the fork.
            let mut peer_b = BoardGraph::default();
            peer_b.insert(root_obj.clone()).unwrap();
            peer_b.insert(post_obj.clone()).unwrap();
            for acceptance in &acceptances {
                peer_b.insert(acceptance.clone()).unwrap();
            }
            if let (Some(index), Some(fork_turn)) = (fork_index, &fork_turn) {
                for turn in &main_turns[..index] {
                    peer_b.insert(turn.clone()).unwrap();
                }
                peer_b.insert(fork_turn.clone()).unwrap();
            }

            // Cross-replicate both ways: both peers must land on the
            // universe set, with identical heads, acceptances, turns.
            peer_a.replicate_board_from(&peer_b, &root_hash).unwrap();
            peer_b.replicate_board_from(&peer_a, &root_hash).unwrap();
            for peer in [&peer_a, &peer_b] {
                prop_assert_eq!(peer.board_object_set(&root_hash), universe_set.clone());
                prop_assert_eq!(peer.game_heads(&game_hash).len(), expected_heads);
                prop_assert_eq!(
                    peer.game_acceptances(&game_hash).len(),
                    usize::try_from(acceptance_count).unwrap()
                );
                prop_assert_eq!(
                    peer.game_turns(&game_hash).len(),
                    main_len + usize::from(fork_turn.is_some())
                );
            }
            prop_assert_eq!(
                peer_a.game_heads(&game_hash),
                peer_b.game_heads(&game_hash)
            );

            // A fresh peer replicating from a converged peer lands
            // on the same set and heads.
            let mut peer_c = BoardGraph::default();
            peer_c.replicate_board_from(&peer_a, &root_hash).unwrap();
            prop_assert_eq!(peer_c.board_object_set(&root_hash), universe_set);
            prop_assert_eq!(
                peer_c.game_heads(&game_hash),
                peer_a.game_heads(&game_hash)
            );
        }
    }

    #[test]
    fn superseded_rules_are_not_resolved() {
        let owner = key(77);
        let mut graph = BoardGraph::default();
        let board_hash = graph.insert(board(&owner)).unwrap();
        let rules_a = SignedBoardObject::sign(
            &owner,
            2,
            "",
            BoardObject::Rules(Rules {
                board: board_hash.clone(),
                revision: 1,
                text: "alpha rules".to_owned(),
            }),
        )
        .unwrap();
        let rules_b = SignedBoardObject::sign(
            &owner,
            3,
            "",
            BoardObject::Rules(Rules {
                board: board_hash.clone(),
                revision: 1,
                text: "beta rules".to_owned(),
            }),
        )
        .unwrap();
        let hash_a = graph.insert(rules_a).unwrap();
        let hash_b = graph.insert(rules_b).unwrap();
        // Supersede the greater-hash rules object with the lesser-hash one.
        let (previous, replacement) = if hash_a > hash_b {
            (hash_a.clone(), hash_b.clone())
        } else {
            (hash_b.clone(), hash_a.clone())
        };
        let supersede = SignedBoardObject::sign(
            &owner,
            4,
            "",
            BoardObject::Supersede(Supersede {
                board: board_hash.clone(),
                previous: previous.clone(),
                replacement: replacement.clone(),
            }),
        )
        .unwrap();
        graph.insert(supersede).unwrap();
        let (resolved, _) = graph.resolve_rules(&board_hash, Some(1)).unwrap();
        // #4114 audit finding: resolution must follow the active-records
        // rule — the superseded (greater-hash) object must not win
        // over its replacement.
        assert_eq!(
            *resolved, replacement,
            "resolve_rules picked the superseded rules object"
        );
    }

    #[test]
    fn game_post_accepts_exact_limits_and_rejects_empty_version() {
        let author = key(60);
        let maximum = game_post_body(
            ArtifactHash::new("d".repeat(64)).unwrap(),
            "v".repeat(128),
            serde_json::Value::String("c".repeat(MAX_GAME_DATA_BYTES - 2)),
            (0..MAX_GAME_PARTICIPANTS).map(participant).collect(),
            serde_json::Value::String("g".repeat(MAX_GAME_DATA_BYTES - 2)),
        );
        let signed =
            SignedBoardObject::sign(&author, 1, "", BoardObject::GamePost(maximum.clone()))
                .unwrap();
        assert_eq!(
            SignedBoardObject::from_artifact(signed.artifact.clone()).unwrap(),
            signed
        );

        let mut duplicate = maximum.clone();
        duplicate.participants[1] = duplicate.participants[0].clone();
        let duplicate_error =
            SignedBoardObject::sign(&author, 2, "", BoardObject::GamePost(duplicate)).unwrap_err();
        assert!(
            duplicate_error
                .to_string()
                .contains("duplicate game participant (author, role) assignment")
        );

        let mut invalid_role = maximum.clone();
        invalid_role.participants[0].role = "Player One".to_owned();
        let role_error =
            SignedBoardObject::sign(&author, 3, "", BoardObject::GamePost(invalid_role))
                .unwrap_err();
        assert!(role_error.to_string().contains("game role identifier"));

        let mut empty_version = maximum;
        empty_version.package.version.clear();
        assert!(invalid_game_post_is_rejected_by_builder_and_receipt(
            &author,
            empty_version,
        ));
    }
}
