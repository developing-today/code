//! Signed, content-addressed board graph objects.
//!
//! A board object is the typed body of the existing [`SignedArtifact`] envelope,
//! not a second signature or storage protocol. Each object is immutable; edits
//! and deletions add [`BoardObject::Supersede`] and [`BoardObject::Tombstone`]
//! records. [`BoardGraph`] checks signatures, canonical references, board scope,
//! and the authority rules needed to accept an object. [`BoardObject::Theme`]
//! adds signed, portable themes a board applies by reference
//! ([`Board::theme_ref`]), and [`BoardGraph::compose_view`] projects the
//! three-layer view (personal filter / board view / host view) under the
//! transparency rule: hidden is never erased. It does not render markup,
//! serve content, or fetch attachment/game payloads.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::artifact::SignedArtifact;

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

/// Typed reference to a signed game-post object; game execution is out of scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GamePostRef {
    /// Hash of the immutable signed game-post object.
    pub object: ArtifactHash,
}

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
                    ensure!(
                        source != &signed.hash()?,
                        "a theme cannot fork from itself"
                    );
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
        // projection, whatever it once said.
        let mut superseded: BTreeSet<ArtifactHash> = BTreeSet::new();
        for signed in self.objects.values() {
            if let BoardObject::Supersede(value) = &signed.object {
                if &value.board == board {
                    superseded.insert(value.previous.clone());
                }
            }
        }

        // Current moderator frontier: a role attestation that another
        // attestation supersedes is history, not authority. Following
        // the signed supersedes links (never arrival order) keeps this
        // identical on every peer holding the same set.
        let mut attestation_superseded: BTreeSet<ArtifactHash> = BTreeSet::new();
        for signed in self.objects.values() {
            if let BoardObject::RoleAttestation(value) = &signed.object {
                if &value.board == board {
                    if let Some(previous) = &value.supersedes {
                        attestation_superseded.insert(previous.clone());
                    }
                }
            }
        }
        let current_moderators: BTreeSet<String> = self
            .objects
            .iter()
            .filter(|(hash, _)| !attestation_superseded.contains(hash))
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
        let has_authority = |author: &str| {
            author == owner || current_moderators.contains(author)
        };

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
                    if superseded.contains(hash)
                        || !has_authority(signed.artifact.author.as_str())
                    {
                        continue;
                    }
                    match value.action {
                        ModerationActionKind::Hide => {
                            *hide_balance.entry(value.target.clone()).or_default() += 1;
                            hide_source.insert(
                                value.target.clone(),
                                (hash.clone(), value.reason.clone()),
                            );
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
                    let (action, reason) =
                        tombstone.cloned().unwrap_or((hash.clone(), None));
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

/// Which layer of a composed board view hid an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewLayer {
    /// The viewer's own filter: personal hides and muted authors.
    Personal,
    /// The board's own signed record: moderation log and tombstones.
    Board,
    /// The serving host's policy set.
    Host,
}

/// One object a composed view hid, and the layer that hid it.
///
/// The object itself stays in the graph — hidden is not erased — so
/// this entry (plus the public moderation log for board-layer hides)
/// is how a viewer, or an auditor, can tell that something is missing
/// from the projection and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HiddenObject {
    /// Hash of the hidden object.
    pub object: ArtifactHash,
    /// Layer that hid it (outermost applicable: board, host, personal).
    pub layer: ViewLayer,
    /// Moderation action or tombstone that hid it, for board-layer
    /// hides; `None` for host and personal hides, which are not signed
    /// graph records.
    pub action: Option<ArtifactHash>,
    /// Recorded reason, when the hiding record carries one.
    pub reason: Option<String>,
}

/// Caller-supplied outer layers for [`BoardGraph::compose_view`].
///
/// Host and personal filters are serving/viewing state, not signed
/// graph objects — a host's policy and a viewer's mutes are theirs to
/// keep — so they are inputs to the projection, never inserted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewLayers {
    /// Objects this viewer has personally hidden.
    pub personal_hidden: BTreeSet<ArtifactHash>,
    /// Authors (canonical Ed25519 public keys) this viewer has muted.
    pub muted_authors: BTreeSet<String>,
    /// Objects the serving host will not serve.
    pub host_hidden: BTreeSet<ArtifactHash>,
}

/// A composed three-layer view of one board. This is a projection of
/// the graph, not a copy of it: every hash names an object that stays
/// in the graph, hidden ones included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardView {
    /// Board root hash this view was composed for.
    pub board: ArtifactHash,
    /// Theme the board applies by reference, resolved from the graph.
    pub theme: Option<Theme>,
    /// Visible current objects: the board root and its content objects,
    /// in deterministic content-hash order.
    pub objects: Vec<ArtifactHash>,
    /// Objects hidden by one of the three layers, with the layer and
    /// (for board-layer hides) the record that hid them.
    pub hidden: Vec<HiddenObject>,
    /// Every moderation action recorded for this board, in
    /// deterministic content-hash order. The moderation log is public:
    /// it is listed whether or not each entry currently hides anything.
    pub moderation_log: Vec<ArtifactHash>,
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

fn validate_token_name(value: &str) -> Result<()> {
    ensure!(
        (1..=64).contains(&value.len())
            && value.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b"-_.".contains(&byte)),
        "theme token name must be 1-64 lower-case ASCII letters, digits, '-', '_' or '.'"
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
mod tests {
    use ed25519_dalek::SigningKey;

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
        assert!(SignedBoardObject::sign(
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
        .is_err());

        let mut empty_value = BTreeMap::new();
        empty_value.insert("accent".to_owned(), String::new());
        assert!(SignedBoardObject::sign(
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
        .is_err());

        let asset = ArtifactHash::new("c".repeat(64)).unwrap();
        assert!(SignedBoardObject::sign(
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
        .is_err());
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
        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
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
        let thread_hash = graph.insert(thread(&owner, root_hash.clone(), "General")).unwrap();
        let post_hash = graph
            .insert(post(&author, root_hash.clone(), thread_hash, "buy my stuff"))
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

        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
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
        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
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
        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
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
        let thread_hash = graph.insert(thread(&owner, root_hash.clone(), "General")).unwrap();
        let alice_post = graph
            .insert(post(&alice, root_hash.clone(), thread_hash.clone(), "alice"))
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

        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
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
        let thread_hash = graph.insert(thread(&owner, root_hash.clone(), "General")).unwrap();
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
        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
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
        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
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
        let thread_hash = graph.insert(thread(&owner, root_hash.clone(), "General")).unwrap();
        let first = graph
            .insert(post(&author, root_hash.clone(), thread_hash.clone(), "first"))
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
        let first_tombstone = graph.insert(tombstone(first.clone(), "wrong thread")).unwrap();
        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
        assert_eq!(view.hidden.len(), 1);
        assert_eq!(view.hidden[0].object, first);
        assert_eq!(view.hidden[0].reason.as_deref(), Some("wrong thread"));

        let second_tombstone =
            graph.insert(tombstone(second.clone(), "duplicate")).unwrap();
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
        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
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
        let root_hash = graph.insert(themed_board(&author, theme_hash.clone())).unwrap();

        let view = graph.compose_view(&root_hash, &ViewLayers::default()).unwrap();
        assert!(view.theme.is_some());

        let mut layers = ViewLayers::default();
        layers.host_hidden.insert(theme_hash);
        let view = graph.compose_view(&root_hash, &layers).unwrap();
        assert!(view.theme.is_none());
        assert_eq!(view.hidden.len(), 1);
        assert_eq!(view.hidden[0].layer, ViewLayer::Host);
    }

}