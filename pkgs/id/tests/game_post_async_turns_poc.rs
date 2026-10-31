//! PoC (#2266): a game post whose turns pass asynchronously, on the tic-tac-toe world.
//!
//! The #2053 investigation modelled a playable post as a board post that binds a
//! **game package**, a **play mode**, and a **state lineage**. This proves the
//! `async-turns` half of that on the checked-in tic-tac-toe program
//! (`examples/tic-tac-toe/tic-tac-toe.wasm`), with four scope points:
//!
//! 1. a **game-post object** naming the package (hash + version), the mode, the
//!    two players, and `attestation: concession`;
//! 2. a **challenge flow** — post, then a signed acceptance by the second player,
//!    then alternating signed **turn objects**, each naming the state frontier it
//!    builds on, folded by the real `.wasm` guest (no live session, no server);
//! 3. game end emits a pair of signed **result claims**; agreeing claims compute
//!    `agreed`;
//! 4. a **save-point** at a mid-game frontier that a third ID **forks**, both
//!    lines proceeding independently while the fork object preserves attribution.
//!
//! Scope guardrails from the issue: this is a PoC, not the production game-post
//! feature. The objects are plain signed JSON records modelled here; they are
//! *not* wired into the event store, the CLI, or the board UI. The point is the
//! object shapes and the lineage arithmetic, verified end to end against the
//! real guest.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use id::sandbox::{Sandbox, SandboxLimits, WorldInstance};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The tic-tac-toe program, exactly as an app package would ship it.
const PACKAGE_WASM: &[u8] = include_bytes!("../examples/tic-tac-toe/tic-tac-toe.wasm");
/// The package version the posts in this file name. A bump is a new hash, and
/// old posts keep running the version they named.
const PACKAGE_VERSION: &str = "0.1.0";

// ---------------------------------------------------------------------------
// The signed-object envelope
// ---------------------------------------------------------------------------
//
// Every object below is one record of the shape the #2053 design implies: a
// kind, a body, the author key, and a signature over the canonical bytes of
// (kind, body). Two differences from `artifact::SignedArtifact` are deliberate
// and local to the PoC: the chain fields (`seq`, `prev`, `after`) are replaced
// by explicit lineage references inside the body (`instance`, `prev_frontier`),
// because a game turn is ordered by the *game's* frontier, not by the author's
// per-key sequence. Keeping the envelope here rather than reusing the artifact
// type is what makes this a PoC: nothing in production learns a new kind.

/// A signed object: the unit the game-post model is built from.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Signed {
    /// What the body means: `game.post`, `challenge.accept`, `game.turn`, and so on.
    kind: String,
    /// Hex Ed25519 public key of the signer.
    author: String,
    /// Hex SHA-256 over the canonical `(kind, body)` bytes.
    hash: String,
    /// Hex Ed25519 signature over the same bytes.
    signature: String,
    /// The kind-specific fields.
    body: Value,
}

impl Signed {
    /// Sign `body` as `kind` with `key`.
    fn sign(key: &SigningKey, kind: &str, body: Value) -> Self {
        let author = hex(&key.verifying_key().to_bytes());
        let bytes = object_bytes(kind, &body);
        Self {
            kind: kind.to_owned(),
            author,
            hash: hex(&Sha256::digest(&bytes)),
            signature: hex(&key.sign(&bytes).to_bytes()),
            body,
        }
    }

    /// Check the hash and the signature. Returns the signer, so callers can
    /// compare it against the ID the object claims to act for.
    fn verify(&self) -> Result<String, String> {
        let bytes = object_bytes(&self.kind, &self.body);
        if hex(&Sha256::digest(&bytes)) != self.hash {
            return Err("object hash does not match its content".to_owned());
        }
        let key = VerifyingKey::from_bytes(&hex_to_array(&self.author)?)
            .map_err(|e| format!("author is not an Ed25519 key: {e}"))?;
        let signature = Signature::from_bytes(&hex_to_array(&self.signature)?);
        key.verify_strict(&bytes, &signature)
            .map_err(|e| format!("signature does not verify: {e}"))?;
        Ok(self.author.clone())
    }

    /// The body field `name` as a string, refusing a missing or non-string field.
    fn field(&self, name: &str) -> Result<String, String> {
        self.body
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{} has no string field `{name}`", self.kind))
    }
}

/// The bytes an object signs: `{"kind":..,"body":..}` in a fixed field order.
fn object_bytes(kind: &str, body: &Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"kind": kind, "body": body})).expect("a JSON value serializes")
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn hex_to_array<const N: usize>(text: &str) -> Result<[u8; N], String> {
    if text.len() != N * 2 {
        return Err(format!(
            "expected {} hex characters, got {}",
            N * 2,
            text.len()
        ));
    }
    let mut out = [0u8; N];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|e| format!("malformed hex: {e}"))?;
    }
    Ok(out)
}

fn id_of(key: &SigningKey) -> String {
    hex(&key.verifying_key().to_bytes())
}

// ---------------------------------------------------------------------------
// The game package: the real tic-tac-toe guest
// ---------------------------------------------------------------------------

/// The package reference a game post names: content hash plus version.
///
/// The hash is over the module bytes, so a package update is a new hash and a
/// post that names the old one keeps the rules it was played under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PackageRef {
    /// Hex SHA-256 over the module bytes.
    hash: String,
    /// The human-facing version the hash belongs to.
    version: String,
}

impl PackageRef {
    fn of_tic_tac_toe() -> Self {
        Self {
            hash: hex(&Sha256::digest(PACKAGE_WASM)),
            version: PACKAGE_VERSION.to_owned(),
        }
    }
}

/// The game rules, as the fold the journal is replayed through.
struct Package;

impl Package {
    /// The state machine of one instance, instantiated from the package and
    /// rebuilt from a snapshot: the whole point of the `snapshot`/`restore` pair.
    fn open(seed: u64) -> WorldInstance {
        let sandbox =
            Sandbox::compile(PACKAGE_WASM, SandboxLimits::default()).expect("package compiles");
        sandbox.instantiate(seed).expect("guest instantiates")
    }

    /// `records` of an instance, parsed: `{board, plays, winner}`.
    fn records(world: &mut WorldInstance) -> Value {
        let raw = world
            .records()
            .expect("records runs")
            .expect("tic-tac-toe has records");
        serde_json::from_slice(&raw).expect("records are JSON")
    }

    /// The state frontier of an instance: the board string, which is exactly
    /// what `snapshot` returns and what `restore` accepts.
    fn frontier(world: &mut WorldInstance) -> String {
        let raw = world
            .snapshot()
            .expect("snapshot runs")
            .expect("tic-tac-toe has snapshot");
        String::from_utf8(raw).expect("snapshot is UTF-8")
    }

    /// Fold one turn: replay the frontier, apply the move, return the new frontier.
    ///
    /// A turn object is only a claim about a state; the state itself is this
    /// deterministic fold. Anyone holding the journal can recompute it.
    fn apply(frontier: &str, participant: u64, mv: &str) -> String {
        let mut world = Self::open(0);
        world
            .restore(frontier.as_bytes())
            .expect("frontier restores");
        world
            .update(participant, mv.as_bytes())
            .expect("move applies");
        Self::frontier(&mut world)
    }

    /// Replay a whole lineage from genesis, applying each turn's move.
    fn replay(genesis: &str, turns: &[Signed]) -> String {
        let mut frontier = genesis.to_owned();
        for turn in turns {
            let participant = turn
                .body
                .get("participant")
                .and_then(Value::as_u64)
                .expect("a turn names its participant");
            let mv = turn.field("move").unwrap();
            frontier = Self::apply(&frontier, participant, &mv);
        }
        frontier
    }
}

// ---------------------------------------------------------------------------
// Point 1: the game-post object
// ---------------------------------------------------------------------------

/// The post a player publishes to open a game. Signed by the poster.
fn game_post(poster: &SigningKey, opponent: &SigningKey, package: &PackageRef) -> Signed {
    Signed::sign(
        poster,
        "game.post",
        json!({
            "instance": hex(&Sha256::digest(b"instance-1")),
            "package": {"hash": package.hash, "version": package.version},
            "mode": "async-turns",
            "players": [id_of(poster), id_of(opponent)],
            "genesis": "---------",
            "attestation": "concession",
        }),
    )
}

/// The second player takes the challenge. Signed by the opponent.
fn challenge_accept(post: &Signed, opponent: &SigningKey) -> Signed {
    Signed::sign(
        opponent,
        "challenge.accept",
        json!({
            "instance": post.field("instance").unwrap(),
            "post": post.hash,
            "acceptor": id_of(opponent),
        }),
    )
}

// ---------------------------------------------------------------------------
// Point 2: alternating signed turns, offline
// ---------------------------------------------------------------------------

/// One turn. `prev_frontier` is the state the mover saw; a turn that names a
/// frontier other than the current one is a fork, not a move.
fn turn(
    player: &SigningKey,
    post: &Signed,
    player_index: u64,
    turn_no: u64,
    mv: &str,
    prev_frontier: &str,
) -> Signed {
    Signed::sign(
        player,
        "game.turn",
        json!({
            "instance": post.field("instance").unwrap(),
            "turn_no": turn_no,
            "player": player_index,
            "participant": player_index + 1,
            "move": mv,
            "prev_frontier": prev_frontier,
        }),
    )
}

/// Validate `turn` as the next move of `line` by whoever is to move.
///
/// Structural turn enforcement from #2053: the turn must be signed by the
/// player whose turn it is, and it must name the current frontier.
fn accept_turn(
    line: &mut Vec<Signed>,
    candidate: Signed,
    players: &[SigningKey],
) -> Result<(), String> {
    let genesis = "---------";
    let frontier = Package::replay(genesis, line);
    let signer = candidate.verify()?;
    let expected = &players[(line.len() % 2) as usize];
    if signer != id_of(expected) {
        return Err(format!(
            "turn {} is signed by {signer}, but it is {}'s turn",
            line.len() + 1,
            id_of(expected)
        ));
    }
    if candidate.field("prev_frontier")? != frontier {
        return Err(format!(
            "turn {} names frontier {}, the line is at {frontier}",
            line.len() + 1,
            candidate.field("prev_frontier")?
        ));
    }
    line.push(candidate);
    Ok(())
}

// ---------------------------------------------------------------------------
// Point 3: concession — two signed result claims compute `agreed`
// ---------------------------------------------------------------------------

/// What a player says the outcome was, signed.
fn result_claim(player: &SigningKey, post: &Signed, frontier: &str, outcome: &str) -> Signed {
    Signed::sign(
        player,
        "match.result",
        json!({
            "instance": post.field("instance").unwrap(),
            "frontier": frontier,
            "state_hash": hex(&Sha256::digest(frontier.as_bytes())),
            "outcome": outcome,
        }),
    )
}

/// `agreed` when both claims verify, name the same instance and frontier, and
/// say the same outcome. This is the #2037 concession arithmetic in miniature.
fn agreed(post: &Signed, claims: &[Signed]) -> Result<Option<String>, String> {
    if claims.len() != 2 {
        return Ok(None);
    }
    let mut seen = Vec::new();
    for claim in claims {
        claim.verify()?;
        if claim.field("instance")? != post.field("instance")? {
            return Ok(None);
        }
        seen.push((claim.field("frontier")?, claim.field("outcome")?));
    }
    if seen[0].0 != seen[1].0 {
        return Ok(None);
    }
    if seen[0].1 != seen[1].1 {
        return Ok(None);
    }
    Ok(Some(seen[0].1.clone()))
}

// ---------------------------------------------------------------------------
// Point 4: save points and forks
// ---------------------------------------------------------------------------

/// A named, citable state: the unit a continuation starts from.
fn save_point(holder: &SigningKey, post: &Signed, frontier: &str, note: &str) -> Signed {
    Signed::sign(
        holder,
        "save.point",
        json!({
            "instance": post.field("instance").unwrap(),
            "frontier": frontier,
            "state_hash": hex(&Sha256::digest(frontier.as_bytes())),
            "manifest": {"package_version": PACKAGE_VERSION},
            "note": note,
        }),
    )
}

/// A fork: a new instance that starts from `point`, attributing the origin.
fn fork(forker: &SigningKey, point: &Signed, new_instance: &str) -> Signed {
    Signed::sign(
        forker,
        "save.fork",
        json!({
            "save_point": point.hash,
            "instance": new_instance,
            "origin_instance": point.field("instance").unwrap(),
            "origin_author": point.author,
            "origin_frontier": point.field("frontier").unwrap(),
            "forker": id_of(forker),
        }),
    )
}

// ---------------------------------------------------------------------------
// The demonstration
// ---------------------------------------------------------------------------

fn keys() -> (SigningKey, SigningKey, SigningKey) {
    (
        SigningKey::from_bytes(&[1; 32]),
        SigningKey::from_bytes(&[2; 32]),
        SigningKey::from_bytes(&[3; 32]),
    )
}

/// Points 1-3: post, accept, six alternating signed turns, concession result.
#[test]
fn async_turns_play_to_a_conceded_result() {
    let (x, o, _) = keys();
    let package = PackageRef::of_tic_tac_toe();
    let post = game_post(&x, &o, &package);
    // The post itself verifies, and it names the package by hash and version.
    assert_eq!(post.verify().unwrap(), id_of(&x));
    assert_eq!(post.field("mode").unwrap(), "async-turns");
    assert_eq!(post.field("attestation").unwrap(), "concession");
    assert_eq!(package.version, PACKAGE_VERSION);
    assert_eq!(package.hash, hex(&Sha256::digest(PACKAGE_WASM)));

    // The challenge is accepted by the other player, and only by them.
    let accept = challenge_accept(&post, &o);
    assert_eq!(accept.verify().unwrap(), id_of(&o));
    assert_eq!(accept.field("post").unwrap(), post.hash);

    // Five turns, each written as if the two sides met hours apart: every turn
    // carries the frontier its author saw, and folds through the real guest.
    let players = [x.clone(), o.clone()];
    let moves = ["0", "3", "1", "4", "2"];
    let mut line: Vec<Signed> = Vec::new();
    for (index, mv) in moves.iter().enumerate() {
        let frontier = Package::replay("---------", &line);
        let player = &players[index % 2];
        let turn = turn(
            player,
            &post,
            (index % 2) as u64,
            index as u64,
            mv,
            &frontier,
        );
        accept_turn(&mut line, turn, &players).unwrap();
    }

    // The fold of the signed journal is the board: X took the top row.
    let final_frontier = Package::replay("---------", &line);
    assert_eq!(final_frontier, "XXXOO----");

    // A turn out of order is refused: the wrong author for the current frontier.
    let stale = turn(&players[0], &post, 0, 5, "5", "XXXOO----");
    let error = accept_turn(&mut line, stale, &players).unwrap_err();
    assert!(error.contains("it is "), "{error}");

    // A turn that builds on a frontier the line is not at is refused too.
    let forged = turn(&players[1], &post, 1, 5, "5", "---------");
    let error = accept_turn(&mut line, forged, &players).unwrap_err();
    assert!(error.contains("names frontier"), "{error}");

    // Concession: both players sign the same outcome over the same frontier.
    let claims = [
        result_claim(&x, &post, &final_frontier, "X"),
        result_claim(&o, &post, &final_frontier, "X"),
    ];
    assert_eq!(agreed(&post, &claims).unwrap().as_deref(), Some("X"));

    // Disagreement is not `agreed`: the object set records the dispute.
    let disputed = [
        result_claim(&x, &post, &final_frontier, "X"),
        result_claim(&o, &post, &final_frontier, "O"),
    ];
    assert_eq!(agreed(&post, &disputed).unwrap(), None);

    // A claim over a different frontier is not `agreed` either.
    let elsewhere = [
        result_claim(&x, &post, &final_frontier, "X"),
        result_claim(&o, &post, "---------", "X"),
    ];
    assert_eq!(agreed(&post, &elsewhere).unwrap(), None);
}

/// Point 4: a mid-game save point is forked by a third ID; both lines proceed.
#[test]
fn a_mid_game_save_point_forks_into_an_independent_line() {
    let (x, o, third) = keys();
    let package = PackageRef::of_tic_tac_toe();
    let post = game_post(&x, &o, &package);
    let players = [x.clone(), o.clone()];

    // A game that stops after four plies: X 0, O 4, X 1, O 2.
    let mut line: Vec<Signed> = Vec::new();
    for (index, mv) in ["0", "4", "1", "2"].iter().enumerate() {
        let frontier = Package::replay("---------", &line);
        let turn = turn(
            &players[index % 2],
            &post,
            (index % 2) as u64,
            index as u64,
            mv,
            &frontier,
        );
        accept_turn(&mut line, turn, &players).unwrap();
    }
    let mid = Package::replay("---------", &line);
    assert_eq!(mid, "XXO-O----");

    // The holder publishes a save point over that frontier.
    let point = save_point(&x, &post, &mid, "year 10, before the fork");
    assert_eq!(point.verify().unwrap(), id_of(&x));
    assert_eq!(point.field("frontier").unwrap(), mid);

    // A third ID forks: a new instance that starts at the save point's frontier.
    let fork_instance = hex(&Sha256::digest(b"instance-2"));
    let branch = fork(&third, &point, &fork_instance);
    assert_eq!(branch.verify().unwrap(), id_of(&third));
    assert_eq!(branch.field("save_point").unwrap(), point.hash);
    assert_eq!(
        branch.field("origin_instance").unwrap(),
        post.field("instance").unwrap()
    );
    assert_eq!(branch.field("origin_author").unwrap(), id_of(&x));
    assert_eq!(branch.field("origin_frontier").unwrap(), mid);

    // The fork plays on from the same frontier, with its own next mover.
    let forked_players = [x.clone(), third.clone()];
    let mut forked_line: Vec<Signed> = line.clone();
    let next = turn(&forked_players[0], &post, 0, 4, "8", &mid);
    accept_turn(&mut forked_line, next, &forked_players).unwrap();
    let forked_state = Package::replay("---------", &forked_line);
    assert_eq!(forked_state, "XXO-O---X");

    // The original line is untouched: the fork is a separate timeline, not a
    // rewrite. Both lines exist, both replay, and the origin is recorded.
    assert_eq!(Package::replay("---------", &line), mid);
    assert_ne!(forked_state, mid);
}

/// The object shapes, printed as one JSON document so the PR can carry them.
#[test]
fn object_shapes_are_documented() {
    let (x, o, third) = keys();
    let package = PackageRef::of_tic_tac_toe();
    let post = game_post(&x, &o, &package);
    let accept = challenge_accept(&post, &o);
    let first = turn(&x, &post, 0, 0, "0", "---------");
    let point = save_point(&x, &post, "---------", "genesis save point");
    let branch = fork(&third, &point, "instance-2");

    let shapes = json!({
        "package": package,
        "objects": [post, accept, first, point, branch],
    });
    // Every object in the document verifies, and the document round-trips.
    let text = serde_json::to_string_pretty(&shapes).unwrap();
    let parsed: Value = serde_json::from_str(&text).unwrap();
    for object in parsed["objects"].as_array().unwrap() {
        let signed: Signed = serde_json::from_value(object.clone()).unwrap();
        signed.verify().unwrap();
    }
    println!("{text}");
}
