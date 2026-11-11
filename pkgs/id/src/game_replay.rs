//! Deterministic replay of asynchronous game lineages (#6160).
//!
//! A [`GamePost`](crate::boards::GamePost) pins the exact package bytes a
//! game is played under. This module folds one signed lineage of
//! [`GameTurn`](crate::boards::GameTurn) objects through those bytes in the
//! existing sandbox: the post's genesis string is restored as the opening
//! frontier, each turn's action is applied by the participant number the
//! post's participant list assigns its signer, and the resulting snapshot
//! is the next frontier. A turn that names any frontier other than the one
//! replay has actually reached is rejected as stale, so a valid signed
//! lineage always replays to the same state from the same inputs.
//!
//! Replay is per lineage: competing turns that build on the same
//! predecessor are forks, each replayed on its own — replaying one branch
//! never erases or rewrites another.

use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

use crate::boards::{ArtifactHash, BoardGraph, BoardObject, GameTurn};
use crate::sandbox::{Sandbox, SandboxLimits};
use crate::world::hex_encode;

/// The state one replayed lineage reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameReplay {
    /// Package snapshot string after the last applied turn (the genesis
    /// frontier when the lineage is empty).
    pub frontier: String,
    /// The package's `records` output at the final state, when it has one.
    pub records: Option<String>,
    /// Number of turns applied.
    pub turns_applied: usize,
}

/// The hashes of the turns that directly build on `prev_turn` within one
/// game (`None` selects opening turns). More than one child is a fork:
/// every competing branch stays visible here.
pub fn game_turn_children(
    graph: &BoardGraph,
    game: &ArtifactHash,
    prev_turn: Option<&ArtifactHash>,
) -> Vec<ArtifactHash> {
    graph
        .turns_for_game(game)
        .into_iter()
        .filter(|(_, turn)| turn.prev_turn.as_ref() == prev_turn)
        .map(|(hash, _)| hash.clone())
        .collect()
}

/// Replay `lineage` (turn hashes in play order) for the game post named by
/// `game`, running the exact `package_bytes` the post pins.
///
/// Oracle: replay is a pure function of (genesis, package bytes, lineage) —
/// the same inputs reach the same frontier and records on every run, and a
/// lineage whose turns name a signer, package, predecessor, index, or
/// frontier that does not match the post or the running state is rejected.
pub fn replay_game_lineage(
    graph: &BoardGraph,
    game: &ArtifactHash,
    lineage: &[ArtifactHash],
    package_bytes: &[u8],
    seed: u64,
) -> Result<GameReplay> {
    let game_object = graph.get(game).context("game post is missing")?;
    let BoardObject::GamePost(post) = &game_object.object else {
        anyhow::bail!("reference is not a game post");
    };
    let package_hash = ArtifactHash::new(hex_encode(&Sha256::digest(package_bytes)))?;
    ensure!(
        package_hash == post.package.hash,
        "package bytes do not match the game post pin"
    );
    let genesis = post
        .genesis
        .as_str()
        .context("game genesis is not a package snapshot string")?;

    let sandbox = Sandbox::compile(package_bytes, SandboxLimits::default())?;
    let mut world = sandbox.instantiate(seed)?;
    world.restore(genesis.as_bytes())?;
    let mut frontier = snapshot_string(&mut world)?;
    ensure!(
        frontier == genesis,
        "game genesis does not restore to itself under the pinned package"
    );

    let mut previous: Option<&ArtifactHash> = None;
    for (position, turn_hash) in lineage.iter().enumerate() {
        let turn_object = graph.get(turn_hash).context("lineage turn is missing")?;
        let BoardObject::GameTurn(turn) = &turn_object.object else {
            anyhow::bail!("lineage reference is not a game turn");
        };
        ensure!(turn.game == *game, "lineage turn belongs to another game");
        ensure!(
            turn.package == post.package,
            "lineage turn package does not match the game post pin"
        );
        ensure!(
            turn.prev_turn.as_ref() == previous,
            "lineage turn does not build on the previous lineage turn"
        );
        ensure!(
            turn.index == u64::try_from(position + 1).unwrap_or(u64::MAX),
            "lineage turn index is out of sequence"
        );
        ensure!(
            turn.prev_frontier == frontier,
            "game turn names frontier {}, replay is at {frontier}",
            turn.prev_frontier
        );
        let participant = participant_number(post, turn, &turn_object.artifact.author)?;
        world.update(participant, turn.action.as_bytes())?;
        frontier = snapshot_string(&mut world)?;
        previous = Some(turn_hash);
    }

    let records = world
        .records()?
        .map(|bytes| String::from_utf8(bytes).context("package records are not UTF-8"))
        .transpose()?;
    Ok(GameReplay {
        frontier,
        records,
        turns_applied: lineage.len(),
    })
}

/// The 1-based sandbox participant number of a turn's signer: the position
/// of the (author, role) assignment in the game post's participant list.
fn participant_number(
    post: &crate::boards::GamePost,
    turn: &GameTurn,
    author: &str,
) -> Result<u64> {
    let position = post
        .participants
        .iter()
        .position(|participant| participant.author == author && participant.role == turn.role)
        .context("game turn signer is not assigned the turn role")?;
    u64::try_from(position + 1).map_err(Into::into)
}

fn snapshot_string(world: &mut crate::sandbox::WorldInstance) -> Result<String> {
    let bytes = world
        .snapshot()?
        .context("pinned package exposes no state snapshot")?;
    String::from_utf8(bytes).context("package snapshot is not UTF-8")
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::boards::{
        Board, BoardObject, GameAcceptance, GameMode, GamePackageRef, GameParticipant, GamePost,
        SignedBoardObject,
    };
    use crate::world::hex_encode;

    const PACKAGE_WASM: &[u8] = include_bytes!("../apps/tic-tac-toe/program/tic-tac-toe.wasm");

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn author_of(key: &SigningKey) -> String {
        hex_encode(&key.verifying_key().to_bytes())
    }

    struct Fixture {
        graph: BoardGraph,
        game: ArtifactHash,
        package: GamePackageRef,
        x_key: SigningKey,
        o_key: SigningKey,
    }

    fn fixture() -> Fixture {
        let x_key = key(7);
        let o_key = key(9);
        let mut graph = BoardGraph::default();
        let board = SignedBoardObject::sign(
            &x_key,
            1,
            "",
            BoardObject::Board(Board {
                board_id: "retro-games".to_owned(),
                forked_from: None,
                theme_ref: None,
            }),
        )
        .unwrap();
        let board_hash = graph.insert(board).unwrap();
        let package = GamePackageRef {
            hash: ArtifactHash::new(hex_encode(&Sha256::digest(PACKAGE_WASM))).unwrap(),
            version: "0.1.0".to_owned(),
        };
        let post = SignedBoardObject::sign(
            &x_key,
            1,
            "",
            BoardObject::GamePost(GamePost {
                board: board_hash.clone(),
                package: package.clone(),
                mode: GameMode::AsyncTurns,
                configuration: serde_json::json!({"board_size": 3}),
                participants: vec![
                    GameParticipant {
                        author: author_of(&x_key),
                        role: "x".to_owned(),
                    },
                    GameParticipant {
                        author: author_of(&o_key),
                        role: "o".to_owned(),
                    },
                ],
                genesis: serde_json::json!("---------"),
            }),
        )
        .unwrap();
        let game = graph.insert(post).unwrap();
        for (signer, role) in [(&x_key, "x"), (&o_key, "o")] {
            let acceptance = SignedBoardObject::sign(
                signer,
                1,
                "",
                BoardObject::GameAcceptance(GameAcceptance {
                    board: board_hash.clone(),
                    game: game.clone(),
                    role: role.to_owned(),
                }),
            )
            .unwrap();
            graph.insert(acceptance).unwrap();
        }
        Fixture {
            graph,
            game,
            package,
            x_key,
            o_key,
        }
    }

    fn turn(
        fixture: &Fixture,
        signer: &SigningKey,
        role: &str,
        index: u64,
        prev_turn: Option<ArtifactHash>,
        prev_frontier: &str,
        action: &str,
    ) -> SignedBoardObject {
        SignedBoardObject::sign(
            signer,
            1,
            "",
            BoardObject::GameTurn(GameTurn {
                board: fixture
                    .graph
                    .get(&fixture.game)
                    .map(|object| match &object.object {
                        BoardObject::GamePost(post) => post.board.clone(),
                        _ => unreachable!(),
                    })
                    .unwrap(),
                game: fixture.game.clone(),
                package: fixture.package.clone(),
                role: role.to_owned(),
                index,
                prev_turn,
                prev_frontier: prev_frontier.to_owned(),
                action: action.to_owned(),
            }),
        )
        .unwrap()
    }

    /// Play the standard five-move line: X takes the top row.
    fn standard_lineage(fixture: &mut Fixture) -> Vec<ArtifactHash> {
        let moves = [
            (&fixture.x_key, "x", "---------", "0"),
            (&fixture.o_key, "o", "X--------", "3"),
            (&fixture.x_key, "x", "X--O-----", "1"),
            (&fixture.o_key, "o", "XX-OO----", "4"),
        ];
        // The fourth move's frontier is computed from the third move's
        // result; keep the table honest by deriving each frontier via
        // replay of the prefix instead of trusting the literals past
        // move two.
        let mut lineage: Vec<ArtifactHash> = Vec::new();
        for (index, (signer, role, frontier, action)) in moves.iter().enumerate() {
            let frontier = if index < 2 {
                frontier.to_string()
            } else {
                replay_game_lineage(&fixture.graph, &fixture.game, &lineage, PACKAGE_WASM, 7)
                    .unwrap()
                    .frontier
            };
            let signed = turn(
                fixture,
                signer,
                role,
                u64::try_from(index + 1).unwrap(),
                lineage.last().cloned(),
                &frontier,
                action,
            );
            lineage.push(fixture.graph.insert(signed).unwrap());
        }
        let frontier =
            replay_game_lineage(&fixture.graph, &fixture.game, &lineage, PACKAGE_WASM, 7)
                .unwrap()
                .frontier;
        let signed = turn(
            fixture,
            &fixture.x_key,
            "x",
            5,
            lineage.last().cloned(),
            &frontier,
            "2",
        );
        lineage.push(fixture.graph.insert(signed).unwrap());
        lineage
    }

    #[test]
    fn signed_lineage_replays_deterministically_to_the_same_state() {
        // Oracle: the checked-in tic-tac-toe package itself — X playing
        // 0,1,2 against O playing 3,4 wins the top row, and replaying
        // the signed lineage twice reaches byte-identical state.
        let mut fixture = fixture();
        let lineage = standard_lineage(&mut fixture);
        let first =
            replay_game_lineage(&fixture.graph, &fixture.game, &lineage, PACKAGE_WASM, 7).unwrap();
        assert_eq!(first.turns_applied, 5);
        assert_eq!(first.frontier, "XXXOO----");
        assert_eq!(
            first.records.as_deref(),
            Some(r#"{"board":"XXXOO----","plays":5,"winner":"X"}"#)
        );
        let second =
            replay_game_lineage(&fixture.graph, &fixture.game, &lineage, PACKAGE_WASM, 7).unwrap();
        assert_eq!(first, second, "same inputs must replay to the same state");
    }

    #[test]
    fn replay_rejects_wrong_package_stale_frontier_and_broken_chain() {
        let mut fixture = fixture();
        let lineage = standard_lineage(&mut fixture);

        // Package bytes that do not match the post's pin are refused.
        let error =
            replay_game_lineage(&fixture.graph, &fixture.game, &lineage, b"wasm?", 7).unwrap_err();
        assert!(error.to_string().contains("do not match"), "{error}");

        // A turn naming a frontier replay has already passed is stale:
        // the graph keeps the signed object (it is well-formed), replay
        // is where the frontier is judged.
        let stale = turn(
            &fixture,
            &fixture.o_key,
            "o",
            2,
            Some(lineage[0].clone()),
            "---------",
            "4",
        );
        let stale_hash = fixture.graph.insert(stale).unwrap();
        let error = replay_game_lineage(
            &fixture.graph,
            &fixture.game,
            &[lineage[0].clone(), stale_hash],
            PACKAGE_WASM,
            7,
        )
        .unwrap_err();
        assert!(error.to_string().contains("frontier"), "{error}");

        // Skipping a turn breaks the predecessor chain.
        let error = replay_game_lineage(
            &fixture.graph,
            &fixture.game,
            &[lineage[1].clone()],
            PACKAGE_WASM,
            7,
        )
        .unwrap_err();
        assert!(error.to_string().contains("previous"), "{error}");
    }

    #[test]
    fn competing_turns_stay_visible_as_forks_and_replay_independently() {
        // Oracle: two signed replies to the same frontier are different
        // boards under the package's own rules (O at 3 vs O at 4), and
        // both remain in the graph as children of the shared turn.
        let mut fixture = fixture();
        let opening = turn(&fixture, &fixture.x_key, "x", 1, None, "---------", "0");
        let opening_hash = fixture.graph.insert(opening).unwrap();
        let mut children = Vec::new();
        for action in ["3", "4"] {
            let reply = turn(
                &fixture,
                &fixture.o_key,
                "o",
                2,
                Some(opening_hash.clone()),
                "X--------",
                action,
            );
            children.push(fixture.graph.insert(reply).unwrap());
        }
        let visible = game_turn_children(&fixture.graph, &fixture.game, Some(&opening_hash));
        assert_eq!(visible.len(), 2, "both competing turns stay visible");
        let left = replay_game_lineage(
            &fixture.graph,
            &fixture.game,
            &[opening_hash.clone(), children[0].clone()],
            PACKAGE_WASM,
            7,
        )
        .unwrap();
        let right = replay_game_lineage(
            &fixture.graph,
            &fixture.game,
            &[opening_hash, children[1].clone()],
            PACKAGE_WASM,
            7,
        )
        .unwrap();
        assert_eq!(left.frontier, "X--O-----");
        assert_eq!(right.frontier, "X---O----");
    }
}
