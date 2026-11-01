//! Signed, content-addressed board graph objects.
//!
//! A board object is the typed body of the existing [`SignedArtifact`] envelope,
//! not a second signature or storage protocol. Each object is immutable; edits
//! and deletions add [`BoardObject::Supersede`] and [`BoardObject::Tombstone`]
//! records. [`BoardGraph`] checks signatures, canonical references, board scope,
//! and the authority rules needed to accept an object. It does not render a view,
//! enforce moderation, or fetch attachment/game payloads.

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
        }
    }

    fn board_ref(&self) -> Option<&ArtifactHash> {
        match self {
            Self::Board(_) | Self::BoardFork(_) => None,
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
                    self.require_scoped(parent, &value.board, "post")?;
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

    fn has_role(&self, board: &ArtifactHash, member: &str, role: BoardRole) -> bool {
        let attestations: Vec<_> = self
            .objects
            .iter()
            .filter_map(|(hash, signed)| match &signed.object {
                BoardObject::RoleAttestation(value)
                    if &value.board == board && value.member == member && value.role == role =>
                {
                    Some((hash, value))
                }
                _ => None,
            })
            .collect();
        let superseded: BTreeSet<_> = attestations
            .iter()
            .filter_map(|(_, value)| value.supersedes.as_ref())
            .collect();
        attestations
            .iter()
            .any(|(hash, value)| !superseded.contains(hash) && value.granted)
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
}
