use serde::{Deserialize, Serialize};

use super::ArtifactHash;

/// Typed reference to a signed game-post object; game execution is out of scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GamePostRef {
    /// Hash of the immutable `BoardObject::GamePost` in the board graph.
    pub object: ArtifactHash,
}

/// Immutable package identity pinned by a game post.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GamePackageRef {
    /// SHA-256 hash of the package bytes. A new package build has a new hash.
    pub hash: ArtifactHash,
    /// Human-facing package version associated with the pinned bytes.
    pub version: String,
}

/// Play mode supported by playable posts in v1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameMode {
    /// Players submit signed turns asynchronously against a pinned frontier.
    AsyncTurns,
}

/// One author assigned to a package-defined game role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameParticipant {
    /// Canonical Ed25519 public key of the assigned author.
    pub author: String,
    /// Canonical role identifier understood by the pinned game package.
    pub role: String,
}

/// Signed metadata for a package-pinned playable post.
///
/// This object records the immutable package reference, package-specific
/// configuration and initial state, and the authors assigned to package-defined
/// roles. It is data, not executable code; accepting it does not run the package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GamePost {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Immutable package bytes and their human-facing version.
    pub package: GamePackageRef,
    /// The v1 play mode.
    pub mode: GameMode,
    /// Package-defined configuration, carried as data.
    pub configuration: serde_json::Value,
    /// Authors assigned to package-defined roles.
    pub participants: Vec<GameParticipant>,
    /// Package-specific initial state, carried as data.
    pub genesis: serde_json::Value,
}

/// Signed acceptance of a game post's challenge by one assigned participant.
///
/// The game post's own author cannot accept their own challenge; an
/// acceptance is the record that opens the game for [`GameTurn`] objects.
/// Competing acceptance records from one participant (re-signed envelopes,
/// divergent arrivals) are all kept as signed history: acceptance is
/// effective by existence, never by a first-arrival-wins check, so peers
/// holding the same signed set converge in any arrival order — the same
/// rule [`BoardGraph::has_role`] documents for role grants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameChallengeAcceptance {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Hash of the immutable `BoardObject::GamePost` being accepted.
    pub game_post: ArtifactHash,
}

/// One signed asynchronous turn in a package-pinned game.
///
/// A turn is bound to its game post (and through it to the pinned package
/// hash and version), to the role its signer is assigned in that post, to
/// its index in a lineage, and to the prior turn it builds on (`prev_turn`,
/// `None` only for the opening turn). The acting participant rotates
/// through the game post's participant list by turn index — turn `i` must
/// be signed by `participants[i % participants.len()]` under that
/// participant's role — so a lineage cannot advance on one player's
/// signatures alone. Two valid turns may name the same `prev_turn`: they
/// are competing branches of the game, kept as forks in the graph, never
/// erased or overwritten by acceptance. Two authorization extensions
/// sit on top of rotation, both recorded as signed objects in the
/// graph: a turn in a [`GameFork`] instance names the fork in
/// `fork_ref` and resolves against the fork's own participants, and
/// on a line an accepted [`GameHandoff`] governs, the recipient may
/// sign a base-line turn under the seat role rotation assigns (the
/// recipient's leg — design #2054 section 4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameTurn {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Hash of the immutable `BoardObject::GamePost` this turn belongs to.
    pub game_post: ArtifactHash,
    /// Package-defined role the signer is assigned in the game post.
    pub role: String,
    /// Zero-based position of this turn in its lineage.
    pub turn_index: u64,
    /// The prior turn in this lineage; `None` for the opening turn.
    pub prev_turn: Option<ArtifactHash>,
    /// The [`GameFork`] instance this turn belongs to, when it
    /// continues a fork rather than the game post's base line. A
    /// fork turn builds on the fork's cited save-point frontier or
    /// on another turn of the same fork, and its signer resolves
    /// against the fork's own participant list. Absent (the base
    /// line) in objects written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_ref: Option<ArtifactHash>,
    /// Package-defined action data, carried as data; replay interprets it.
    pub action: serde_json::Value,
}

/// A signed save point in one game line: the pinned package, the
/// frontier turn it saves, and the hash of the replayed state at
/// that frontier.
///
/// A save point is signed by an assigned participant of the line
/// it saves: the post's participants for a base-line frontier, or
/// the fork's own participants when the frontier is a fork turn.
/// post and must name that post's exact pinned package (hash and
/// version) — a save recorded against different package bytes is a
/// save of a different game. The state hash is the saver's signed
/// claim about [`BoardGraph::replay_game_lineage`]'s snapshot at
/// the frontier; peers can recompute it from the same lineage and
/// the same bytes. Save blobs themselves stay content-addressed
/// references: this record carries hashes, never state bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameSavePoint {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Hash of the immutable `BoardObject::GamePost` this save belongs to.
    pub game_post: ArtifactHash,
    /// The pinned package the saved line replays through; must equal
    /// the game post's package reference exactly.
    pub package: GamePackageRef,
    /// Hash of the [`GameTurn`] at the saved frontier.
    pub frontier: ArtifactHash,
    /// SHA-256 hash of the replayed package state at the frontier.
    pub state_hash: ArtifactHash,
}

/// Signed handoff of a saved game line to a named recipient.
///
/// Only the save point's own author may hand it off, and the
/// recipient must be an assigned participant of the game post —
/// turn validation binds continuation turns to the participant
/// list, so a handoff to anyone else could never be continued.
/// Competing handoffs of one save point are all kept as signed
/// history; an accepted handoff selects the social main line (see
/// [`BoardGraph::game_main_line_head`]) and the alternatives are
/// never deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameHandoff {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Hash of the immutable `BoardObject::GamePost` this handoff belongs to.
    pub game_post: ArtifactHash,
    /// Hash of the `BoardObject::GameSavePoint` being handed off.
    pub save_point: ArtifactHash,
    /// Canonical Ed25519 public key of the named recipient.
    pub recipient: String,
    /// Handoff terms, carried as data.
    pub terms: serde_json::Value,
}

/// Signed acceptance of a [`GameHandoff`] by its named recipient.
///
/// Only the recipient the handoff names may accept it. Acceptance
/// is effective by existence, never by a first-arrival-wins check —
/// the same rule [`GameChallengeAcceptance`] documents — so peers
/// holding the same signed set converge in any arrival order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameHandoffAcceptance {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Hash of the immutable `BoardObject::GamePost` this acceptance belongs to.
    pub game_post: ArtifactHash,
    /// Hash of the `BoardObject::GameHandoff` being accepted.
    pub handoff: ArtifactHash,
}

/// A signed fork of a game from a cited save point, by any author.
///
/// The fork record starts an independent continuation: turns that
/// build on the save point's frontier are a branch of the game
/// (see [`GameTurn`]), replayed independently of the source line
/// through [`BoardGraph::replay_game_lineage`]. The citation
/// preserves source-line attribution — the save point, its author,
/// and the lineage behind its frontier stay named in the graph.
/// Board-level [`BoardFork`] is a different feature (whole-board
/// provenance); this is a game-level fork inside one board. The
/// fork carries its own participant list: fork turns resolve their
/// signers against it, not the game post's, so a third-party fork
/// actually proceeds as an alternate instance (design #2054
/// section 4). Seating is phase-aligned to the absolute turn
/// index — a fork turn's seat is `participants[turn_index % len]`,
/// matching the move count already inside the saved state — so a
/// fork opened mid-rotation continues the saved state's rotation
/// instead of restarting it. For the same reason the participant
/// list must preserve the saved instance's seat layout: the same
/// number of seats with the same role at each seat; only the
/// assigned authors may change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameFork {
    /// Board root hash.
    pub board: ArtifactHash,
    /// Hash of the immutable `BoardObject::GamePost` this fork belongs to.
    pub game_post: ArtifactHash,
    /// Hash of the `BoardObject::GameSavePoint` the fork cites.
    pub save_point: ArtifactHash,
    /// Authors assigned to package-defined roles in the fork
    /// instance, in fork-turn rotation order. The fork's own
    /// author must hold one of the assignments: a fork its author
    /// cannot play in does not proceed.
    pub participants: Vec<GameParticipant>,
}
