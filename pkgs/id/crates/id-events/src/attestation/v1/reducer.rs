//! The v1 attestation reducer (#2039 phase 4).
//!
//! [`reduce`] computes a match's [`V1MatchState`] from signature-verified
//! objects alone: the genesis pair, result claims, witness decisions,
//! objections and audit reports, plus verified fork proofs. It is a pure
//! function: it never edits an object, and running it again over the same
//! evidence gives the same answer, whichever peer or registry runs it.
//!
//! The rules follow `doc/2026-11-01_design_id2039.md` §2–§5:
//!
//! - **Admission first.** Only objects that answer this agreement and come
//!   from the right author count: genesis and claims from the two seated
//!   players, decisions from the genesis-named witness on the pinned logic
//!   (no witness-shopping), objections from a seated player naming an object
//!   of this match, audit reports that pass the frozen honest-scope rules.
//!   Objects that name this agreement but fail admission are reported in
//!   [`Reduction::ignored`].
//! - **Evidence closure.** Every admitted object depends on its own chain
//!   position, its causal `after` frontier and the hashes its body names.
//!   A fork proof at or before any of those positions voids the object, and
//!   voidness propagates to every object that names a void one — including
//!   objects authored by somebody else (a witness decision or the other
//!   player's claim that references a now-forked move chain is void too).
//!   The event-store resolver does not propagate forks across authors; this
//!   reducer does, as the design assigns that check to the attestation layer.
//! - **States.** Any void match object makes the match [`V1MatchState::Void`].
//!   Otherwise: no complete genesis pair is `Proposed`; a player claiming two
//!   positions, two witness decisions with different outcomes, any admitted
//!   objection, differing claims, or a witness decision that contradicts a
//!   claim is `Disputed`; two concurring claims are `Agreed` (concession);
//!   witness decisions with no contradicting claim are `Witnessed`
//!   (witnessed-unopposed); anything less is `Proposed`.
//! - **Separate judgment.** An audit report never changes the state: it is
//!   checked for evidence closure and reported, and a registry decides
//!   whether to accept it as an `audit-resolved` basis (phase 6).
//! - **No finality without receipts.** Phase 4 has no receipt/closure
//!   evidence (phase 5), so [`Reduction::finality_proven`] is always `false`
//!   and the reducer never emits [`V1MatchState::Final`]. A self-authored
//!   frontier never proves first receipt.

use std::collections::{HashMap, HashSet};

use p2panda_core::Hash;

use super::audit::AuditReport;
use super::{Objection, V1MatchState, VERSION, WitnessDecision, WitnessEvidence};
use crate::attestation::{GenesisAgreement, PlayerId, ResultClaim, author_id};
use id_store::chain::Event;

/// A decoded, signature-verified object with its chain position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed<T> {
    /// The event's operation id.
    pub id: Hash,
    /// The verified author.
    pub author: PlayerId,
    /// The event's sequence number in its author's chain.
    pub seq: u64,
    /// The event's causal frontier (`ChainExt::after`).
    pub after: Vec<Hash>,
    /// The decoded body.
    pub body: T,
}

impl<T> Signed<T> {
    /// Pairs a decoded event with its decoded body. The caller has already
    /// verified the event (`chain::decode`) and decoded the body with the
    /// object's `from_body`.
    #[must_use]
    pub fn admit(event: &Event, body: T) -> Self {
        Self {
            id: event.id,
            author: author_id(event),
            seq: event.header.seq_num,
            after: event.header.extensions.after.clone(),
            body,
        }
    }
}

/// Where a referenced event sits: its author's chain and sequence number.
/// Supplied by the caller for every referenced event that is not itself one
/// of the attestation objects (moves, signed inputs, frontier events).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventPosition {
    /// The event's operation id.
    pub id: Hash,
    /// The event's author.
    pub author: PlayerId,
    /// The event's sequence number.
    pub seq: u64,
}

/// A verified fork proof: `author` signed two different events at `seq`.
/// Every event of that author at or after `seq` is forked history. The
/// caller verifies the two conflicting signed events before passing a proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForkProof {
    /// The equivocating author.
    pub author: PlayerId,
    /// The earliest sequence number with two different signed events.
    pub seq: u64,
}

/// Everything a reducer run sees. Objects naming other agreements are
/// simply not part of this match and are skipped.
#[derive(Clone, Copy, Debug, Default)]
pub struct Evidence<'a> {
    /// Genesis events, one per player.
    pub genesis: &'a [Signed<GenesisAgreement>],
    /// Result claims.
    pub claims: &'a [Signed<ResultClaim>],
    /// Witness decisions.
    pub decisions: &'a [Signed<WitnessDecision>],
    /// Objections.
    pub objections: &'a [Signed<Objection>],
    /// Audit reports.
    pub audits: &'a [Signed<AuditReport>],
    /// Positions of referenced events that are not attestation objects.
    pub positions: &'a [EventPosition],
    /// Verified fork proofs.
    pub forks: &'a [ForkProof],
}

/// The evidence basis of a non-disputed result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// No result yet, or the result is disputed or void.
    None,
    /// Both players concurred (mutual concession).
    Concession,
    /// The named witness decided on complete evidence.
    WitnessComplete,
    /// The named witness decided on partial evidence only; procedural, not
    /// replay proof.
    WitnessPartial,
}

/// The reducer's answer, with the evidence it could not use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reduction {
    /// The computed state.
    pub state: V1MatchState,
    /// The basis of a non-disputed, non-void result.
    pub basis: Basis,
    /// Always `false` in v1 phase 4: no receipt/closure evidence exists yet,
    /// so no state is protocol-final.
    pub finality_proven: bool,
    /// Admitted match objects voided by fork evidence, in input order.
    pub void: Vec<Hash>,
    /// Admitted audit reports voided by fork evidence. They do not change
    /// the state; a registry must not accept them as a basis.
    pub void_audits: Vec<Hash>,
    /// Admitted audit reports with intact evidence closure.
    pub audits: Vec<Hash>,
    /// Referenced hashes with no known position or object: the reducer
    /// cannot check them against forks until they are synced.
    pub unresolved: Vec<Hash>,
    /// Objects that name this agreement but failed admission (wrong author,
    /// wrong witness or logic, malformed outcome or evidence, dishonest audit
    /// scope), and seated objections whose target is outside this match.
    pub ignored: Vec<Hash>,
}

impl Reduction {
    const fn with_state(state: V1MatchState) -> Self {
        Self {
            state,
            basis: Basis::None,
            finality_proven: false,
            void: Vec::new(),
            void_audits: Vec::new(),
            audits: Vec::new(),
            unresolved: Vec::new(),
            ignored: Vec::new(),
        }
    }
}

fn hex(player: &PlayerId) -> String {
    use std::fmt::Write as _;
    player
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

struct Node {
    id: Hash,
    author: PlayerId,
    seq: u64,
    deps: Vec<Hash>,
}

impl Node {
    fn of<T>(object: &Signed<T>, body_refs: impl IntoIterator<Item = Hash>) -> Self {
        Self {
            id: object.id,
            author: object.author,
            seq: object.seq,
            deps: object.after.iter().copied().chain(body_refs).collect(),
        }
    }
}

fn decision_refs(decision: &WitnessDecision) -> Vec<Hash> {
    match &decision.evidence {
        WitnessEvidence::Complete {
            claim_a, claim_b, ..
        } => vec![*claim_a, *claim_b],
        WitnessEvidence::Partial {
            received,
            non_receipt_statement,
            ..
        } => received
            .iter()
            .copied()
            .chain(std::iter::once(*non_receipt_statement))
            .collect(),
    }
}

/// Computes the v1 match state for `agreement` from `evidence`. See the
/// module documentation for the rules.
#[must_use]
pub fn reduce(agreement: &GenesisAgreement, evidence: &Evidence<'_>) -> Reduction {
    let [first, second] = agreement.players;
    if first == second {
        return Reduction::with_state(V1MatchState::Proposed);
    }
    let Ok(agreement_hash) = agreement.agreement_hash() else {
        return Reduction::with_state(V1MatchState::Proposed);
    };
    let seated = |player: &PlayerId| agreement.players.contains(player);
    let player_hex = agreement.players.map(|player| hex(&player));
    let outcome_ok = |outcome: &str| outcome == "draw" || player_hex.iter().any(|h| h == outcome);
    let mut ignored = Vec::new();

    // Admission.
    let genesis: Vec<&Signed<GenesisAgreement>> = evidence
        .genesis
        .iter()
        .filter(|object| seated(&object.author) && object.body == *agreement)
        .collect();
    let mut claims = Vec::new();
    for object in evidence.claims {
        if object.body.agreement != agreement_hash {
            continue;
        }
        if seated(&object.author) && outcome_ok(&object.body.outcome) {
            claims.push(object);
        } else {
            ignored.push(object.id);
        }
    }
    let mut decisions = Vec::new();
    for object in evidence.decisions {
        let decision = &object.body;
        if decision.agreement != agreement_hash {
            continue;
        }
        let evidence_ok = match &decision.evidence {
            WitnessEvidence::Complete {
                claim_a, claim_b, ..
            } => claim_a != claim_b,
            WitnessEvidence::Partial { missing_from, .. } => seated(missing_from),
        };
        if agreement.witness == Some(object.author)
            && decision.version == VERSION
            && decision.logic_hash == agreement.logic_hash
            && outcome_ok(&decision.outcome)
            && evidence_ok
        {
            decisions.push(object);
        } else {
            ignored.push(object.id);
        }
    }
    let mut audits = Vec::new();
    for object in evidence.audits {
        if object.body.agreement != agreement_hash {
            continue;
        }
        if object.body.validate().is_ok() && object.body.logic_hash == agreement.logic_hash {
            audits.push(object);
        } else {
            ignored.push(object.id);
        }
    }
    let targets: HashSet<Hash> = std::iter::once(agreement_hash)
        .chain(genesis.iter().map(|object| object.id))
        .chain(claims.iter().map(|object| object.id))
        .chain(decisions.iter().map(|object| object.id))
        .chain(audits.iter().map(|object| object.id))
        .collect();
    let mut objections = Vec::new();
    for object in evidence.objections {
        let in_match = targets.contains(&object.body.target);
        if seated(&object.author) && object.body.version == VERSION && in_match {
            objections.push(object);
        } else if in_match || seated(&object.author) {
            // A seated player's objection to something outside this match
            // cannot be attributed to it; a non-player's objection to this
            // match does not count. Both are reported, neither disputes.
            ignored.push(object.id);
        }
    }

    // Evidence closure: the dependency graph of every admitted object.
    let mut nodes: Vec<Node> = Vec::new();
    nodes.extend(
        genesis
            .iter()
            .map(|&object| Node::of(object, std::iter::empty())),
    );
    nodes.extend(
        claims
            .iter()
            .map(|&object| Node::of(object, object.body.frontiers.iter().copied())),
    );
    nodes.extend(
        decisions
            .iter()
            .map(|&object| Node::of(object, decision_refs(&object.body))),
    );
    nodes.extend(objections.iter().map(|&object| {
        Node::of(
            object,
            std::iter::once(object.body.target).chain(object.body.author_frontier.iter().copied()),
        )
    }));
    let audit_start = nodes.len();
    nodes.extend(
        audits
            .iter()
            .map(|&object| Node::of(object, object.body.dependencies())),
    );

    let forked = |author: &PlayerId, seq: u64| {
        evidence
            .forks
            .iter()
            .any(|proof| proof.author == *author && proof.seq <= seq)
    };
    let mut positions: HashMap<Hash, (PlayerId, u64)> = evidence
        .positions
        .iter()
        .map(|position| (position.id, (position.author, position.seq)))
        .collect();
    for node in &nodes {
        positions.insert(node.id, (node.author, node.seq));
    }
    let mut void: HashSet<Hash> = nodes
        .iter()
        .filter(|node| forked(&node.author, node.seq))
        .map(|node| node.id)
        .collect();
    let mut unresolved = Vec::new();
    for node in &nodes {
        for dep in &node.deps {
            match positions.get(dep) {
                Some((author, seq)) if forked(author, *seq) => {
                    void.insert(*dep);
                }
                Some(_) => {}
                None if *dep == agreement_hash => {}
                None => {
                    if !unresolved.contains(dep) {
                        unresolved.push(*dep);
                    }
                }
            }
        }
    }
    loop {
        let before = void.len();
        for node in &nodes {
            if !void.contains(&node.id) && node.deps.iter().any(|dep| void.contains(dep)) {
                void.insert(node.id);
            }
        }
        if void.len() == before {
            break;
        }
    }
    let (match_nodes, audit_nodes) = nodes.split_at(audit_start);
    let void_objects: Vec<Hash> = match_nodes
        .iter()
        .filter(|node| void.contains(&node.id))
        .map(|node| node.id)
        .collect();
    let (void_audits, live_audits): (Vec<Hash>, Vec<Hash>) = audit_nodes
        .iter()
        .map(|node| node.id)
        .partition(|id| void.contains(id));

    let mut result = Reduction {
        void: void_objects,
        void_audits,
        audits: live_audits,
        unresolved,
        ignored,
        ..Reduction::with_state(V1MatchState::Proposed)
    };
    if !result.void.is_empty() {
        result.state = V1MatchState::Void;
        return result;
    }

    // States.
    let genesis_complete = agreement
        .players
        .iter()
        .all(|player| genesis.iter().any(|object| object.author == *player));
    if !genesis_complete {
        return result;
    }
    let mut per_player: [Option<&ResultClaim>; 2] = [None, None];
    for (slot, player) in per_player.iter_mut().zip(agreement.players.iter()) {
        for object in claims.iter().filter(|object| object.author == *player) {
            match slot {
                None => *slot = Some(&object.body),
                Some(seen)
                    if seen.outcome != object.body.outcome
                        || seen.final_state_hash != object.body.final_state_hash =>
                {
                    result.state = V1MatchState::Disputed;
                    return result;
                }
                Some(_) => {}
            }
        }
    }
    let mut decided: Option<&str> = None;
    for object in &decisions {
        match decided {
            None => decided = Some(object.body.outcome.as_str()),
            Some(outcome) if outcome != object.body.outcome => {
                result.state = V1MatchState::Disputed;
                return result;
            }
            Some(_) => {}
        }
    }
    if !objections.is_empty() {
        result.state = V1MatchState::Disputed;
        return result;
    }
    let contradicts = |claim: &ResultClaim| decided.is_some_and(|outcome| outcome != claim.outcome);
    match per_player {
        [Some(a), Some(b)] => {
            if a.outcome != b.outcome || a.final_state_hash != b.final_state_hash || contradicts(a)
            {
                result.state = V1MatchState::Disputed;
            } else {
                result.state = V1MatchState::Agreed;
                result.basis = Basis::Concession;
            }
        }
        [Some(claim), None] | [None, Some(claim)] if contradicts(claim) => {
            result.state = V1MatchState::Disputed;
        }
        _ if decided.is_some() => {
            result.state = V1MatchState::Witnessed;
            result.basis = if decisions
                .iter()
                .any(|object| matches!(object.body.evidence, WitnessEvidence::Complete { .. }))
            {
                Basis::WitnessComplete
            } else {
                Basis::WitnessPartial
            };
        }
        _ => {}
    }
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Basis, EventPosition, Evidence, ForkProof, Signed, hex, reduce};
    use crate::attestation::v1::audit::{
        AuditReport, AuditScope, AuditVerdict, AuditorRole, CheckpointInterval,
    };
    use crate::attestation::v1::{
        Objection, V1MatchState, VERSION, WitnessDecision, WitnessEvidence,
    };
    use crate::attestation::{GenesisAgreement, PlayerId, ResultClaim};
    use p2panda_core::Hash;

    const A: PlayerId = [1; 32];
    const B: PlayerId = [2; 32];
    const W: PlayerId = [3; 32];
    const OUTSIDER: PlayerId = [4; 32];

    fn h(name: &str) -> Hash {
        Hash::digest(name.as_bytes())
    }

    fn agreement() -> GenesisAgreement {
        GenesisAgreement {
            game_id: "tic-tac-toe".to_owned(),
            logic_hash: h("logic"),
            players: [A, B],
            witness: Some(W),
            objection_window_secs: 3600,
        }
    }

    fn signed<T>(name: &str, author: PlayerId, seq: u64, after: &[&str], body: T) -> Signed<T> {
        Signed {
            id: h(name),
            author,
            seq,
            after: after.iter().map(|name| h(name)).collect(),
            body,
        }
    }

    /// A mutable world each table row edits: the genesis pair and move
    /// positions are in place; claims, decisions, objections, audits and
    /// forks are added by the row.
    struct World {
        agreement: GenesisAgreement,
        genesis: Vec<Signed<GenesisAgreement>>,
        claims: Vec<Signed<ResultClaim>>,
        decisions: Vec<Signed<WitnessDecision>>,
        objections: Vec<Signed<Objection>>,
        audits: Vec<Signed<AuditReport>>,
        positions: Vec<EventPosition>,
        forks: Vec<ForkProof>,
    }

    impl World {
        fn new() -> Self {
            let agreement = agreement();
            let genesis = vec![
                signed("genesis a", A, 0, &[], agreement.clone()),
                signed("genesis b", B, 0, &[], agreement.clone()),
            ];
            // Each player's moves at seq 1..=5; the move at seq 5 is the
            // player's frontier at match end.
            let positions = [("a", A), ("b", B)]
                .into_iter()
                .flat_map(|(tag, author)| {
                    (1..=5).map(move |seq| EventPosition {
                        id: h(&format!("move {tag} {seq}")),
                        author,
                        seq,
                    })
                })
                .collect();
            Self {
                agreement,
                genesis,
                claims: Vec::new(),
                decisions: Vec::new(),
                objections: Vec::new(),
                audits: Vec::new(),
                positions,
                forks: Vec::new(),
            }
        }

        fn claim(&mut self, name: &str, author: PlayerId, outcome: &str, state: &str) {
            let body = ResultClaim {
                agreement: self.agreement.agreement_hash().unwrap(),
                outcome: outcome.to_owned(),
                final_state_hash: h(state),
                frontiers: vec![h("move a 5"), h("move b 5")],
            };
            let after: &[&str] = if author == A {
                &["move a 5"]
            } else {
                &["move b 5"]
            };
            self.claims.push(signed(name, author, 6, after, body));
        }

        fn decision(&mut self, name: &str, author: PlayerId, outcome: &str, partial: bool) {
            let evidence = if partial {
                WitnessEvidence::Partial {
                    received: vec![h("claim a")],
                    missing_from: B,
                    deadline_secs: 3600,
                    non_receipt_statement: h("non-receipt"),
                }
            } else {
                WitnessEvidence::Complete {
                    claim_a: h("claim a"),
                    claim_b: h("claim b"),
                    log_hash_a: h("log a"),
                    log_hash_b: h("log b"),
                }
            };
            let body = WitnessDecision {
                version: VERSION,
                agreement: self.agreement.agreement_hash().unwrap(),
                logic_hash: self.agreement.logic_hash,
                outcome: outcome.to_owned(),
                evidence,
            };
            let seq = u64::try_from(self.decisions.len()).unwrap();
            self.decisions.push(signed(name, author, seq, &[], body));
        }

        fn objection(&mut self, name: &str, author: PlayerId, target: Hash) {
            let body = Objection {
                version: VERSION,
                target,
                grounds: "replay differs".to_owned(),
                author_frontier: Vec::new(),
            };
            self.objections.push(signed(name, author, 7, &[], body));
        }

        fn audit(&mut self, name: &str, scope: AuditScope, verdict: AuditVerdict) {
            let body = AuditReport {
                version: VERSION,
                role: AuditorRole::Independent,
                match_id: self.agreement.game_id.clone(),
                agreement: self.agreement.agreement_hash().unwrap(),
                transcript: h("move a 5"),
                evidence: vec![h("claim a"), h("claim b")],
                logic_hash: self.agreement.logic_hash,
                rules: h("rules"),
                runtime: "x86_64-linux".to_owned(),
                tool_version: "match-audit 1".to_owned(),
                scope,
                verdict,
                plausibility: None,
                metadata: Vec::new(),
            };
            self.audits.push(signed(name, OUTSIDER, 0, &[], body));
        }

        fn reduce(&self) -> super::Reduction {
            reduce(
                &self.agreement,
                &Evidence {
                    genesis: &self.genesis,
                    claims: &self.claims,
                    decisions: &self.decisions,
                    objections: &self.objections,
                    audits: &self.audits,
                    positions: &self.positions,
                    forks: &self.forks,
                },
            )
        }
    }

    fn win(player: PlayerId) -> String {
        hex(&player)
    }

    fn concession(world: &mut World) {
        world.claim("claim a", A, &win(A), "final");
        world.claim("claim b", B, &win(A), "final");
    }

    type Row = (&'static str, fn(&mut World), V1MatchState, Basis);

    #[test]
    fn reducer_state_table() {
        let rows: &[Row] = &[
            (
                "no genesis is proposed",
                |w| {
                    w.genesis.clear();
                    concession(w);
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "one genesis is proposed",
                |w| {
                    w.genesis.pop();
                    concession(w);
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "genesis signed by an outsider does not complete the pair",
                |w| {
                    w.genesis[1].author = OUTSIDER;
                    concession(w);
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "duplicate seat is proposed whatever the evidence",
                |w| {
                    w.agreement.players = [A, A];
                    concession(w);
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "genesis only is proposed",
                |_| {},
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "missing claim is proposed",
                |w| w.claim("claim a", A, &win(A), "final"),
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "two concurring claims are a concession",
                concession,
                V1MatchState::Agreed,
                Basis::Concession,
            ),
            (
                "a repeated identical claim stays a concession",
                |w| {
                    concession(w);
                    w.claim("claim a again", A, &win(A), "final");
                },
                V1MatchState::Agreed,
                Basis::Concession,
            ),
            (
                "duplicate claims at different positions are disputed",
                |w| {
                    concession(w);
                    w.claim("claim a fork", A, "draw", "final");
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "disagreeing outcomes are disputed",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.claim("claim b", B, &win(B), "final");
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "disagreeing final states are disputed",
                |w| {
                    w.claim("claim a", A, "draw", "final");
                    w.claim("claim b", B, "draw", "other final");
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "a non-player's claim is ignored",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.claim("claim x", OUTSIDER, &win(A), "final");
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "a claim naming an unseated winner is ignored",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.claim("claim b", B, &win(OUTSIDER), "final");
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "complete witness decision alone is witnessed",
                |w| w.decision("decision", W, &win(A), false),
                V1MatchState::Witnessed,
                Basis::WitnessComplete,
            ),
            (
                "partial witness decision with a matching claim is witnessed",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.decision("decision", W, &win(A), true);
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
            ),
            (
                "partial witness decision against the only claim is disputed",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.decision("decision", W, &win(B), true);
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "a decision from a witness not named in genesis is ignored",
                |w| w.decision("decision", OUTSIDER, &win(A), false),
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "a player cannot issue a witness decision",
                |w| w.decision("decision", A, &win(A), false),
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "a decision on other logic is ignored",
                |w| {
                    w.decision("decision", W, &win(A), false);
                    w.decisions[0].body.logic_hash = h("other logic");
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "a partial decision missing an unseated player is ignored",
                |w| {
                    w.decision("decision", W, &win(A), true);
                    if let WitnessEvidence::Partial { missing_from, .. } =
                        &mut w.decisions[0].body.evidence
                    {
                        *missing_from = OUTSIDER;
                    }
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
            (
                "conflicting witness decisions are disputed",
                |w| {
                    w.decision("decision 1", W, &win(A), false);
                    w.decision("decision 2", W, &win(B), false);
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "a witness contradicting a concession is disputed",
                |w| {
                    concession(w);
                    w.decision("decision", W, &win(B), false);
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "a witness does not resolve disagreeing claims",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.claim("claim b", B, &win(B), "final");
                    w.decision("decision", W, &win(A), false);
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "a witness concurring with a concession keeps the concession",
                |w| {
                    concession(w);
                    w.decision("decision", W, &win(A), false);
                },
                V1MatchState::Agreed,
                Basis::Concession,
            ),
            (
                "a player's objection disputes a concession",
                |w| {
                    concession(w);
                    w.objection("objection", B, h("claim a"));
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "a player's objection disputes a witness decision",
                |w| {
                    w.decision("decision", W, &win(A), false);
                    w.objection("objection", B, h("decision"));
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "an outsider's objection is ignored",
                |w| {
                    concession(w);
                    w.objection("objection", OUTSIDER, h("claim a"));
                },
                V1MatchState::Agreed,
                Basis::Concession,
            ),
            (
                "an objection to something outside this match is ignored",
                |w| {
                    concession(w);
                    w.objection("objection", B, h("another match's claim"));
                },
                V1MatchState::Agreed,
                Basis::Concession,
            ),
            (
                "fork at a claim's frontier voids the match",
                |w| {
                    concession(w);
                    w.forks.push(ForkProof { author: A, seq: 5 });
                },
                V1MatchState::Void,
                Basis::None,
            ),
            (
                "fork before the frontier voids the match",
                |w| {
                    concession(w);
                    w.forks.push(ForkProof { author: B, seq: 2 });
                },
                V1MatchState::Void,
                Basis::None,
            ),
            (
                "fork after every referenced position changes nothing",
                |w| {
                    concession(w);
                    w.forks.push(ForkProof { author: A, seq: 9 });
                },
                V1MatchState::Agreed,
                Basis::Concession,
            ),
            (
                "fork before genesis voids the match",
                |w| {
                    concession(w);
                    w.forks.push(ForkProof { author: B, seq: 0 });
                },
                V1MatchState::Void,
                Basis::None,
            ),
            (
                "a foreign-authored decision on a forked move chain is void",
                |w| {
                    w.decision("decision", W, &win(A), false);
                    w.decisions[0].after = vec![h("move a 4")];
                    w.forks.push(ForkProof { author: A, seq: 4 });
                },
                V1MatchState::Void,
                Basis::None,
            ),
            (
                "a foreign-authored decision on a voided claim is void",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.decision("decision", W, &win(A), false);
                    w.forks.push(ForkProof { author: A, seq: 6 });
                },
                V1MatchState::Void,
                Basis::None,
            ),
            (
                "a fork of the objector's chain at its frontier voids the match",
                |w| {
                    concession(w);
                    w.objection("objection", B, h("claim a"));
                    w.objections[0].body.author_frontier = vec![h("move b 3")];
                    w.forks.push(ForkProof { author: B, seq: 3 });
                },
                V1MatchState::Void,
                Basis::None,
            ),
            (
                "a full-history audit never resolves a dispute",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.claim("claim b", B, &win(B), "final");
                    w.audit(
                        "audit",
                        AuditScope::FullHistory,
                        AuditVerdict::ResultConfirmed,
                    );
                },
                V1MatchState::Disputed,
                Basis::None,
            ),
            (
                "an audit never creates a result",
                |w| {
                    w.audit(
                        "audit",
                        AuditScope::FullHistory,
                        AuditVerdict::ResultConfirmed,
                    );
                },
                V1MatchState::Proposed,
                Basis::None,
            ),
        ];
        for &(name, edit, state, basis) in rows {
            let mut world = World::new();
            edit(&mut world);
            let reduction = world.reduce();
            assert_eq!(reduction.state, state, "{name}");
            assert_eq!(reduction.basis, basis, "{name}");
            assert!(!reduction.finality_proven, "{name}");
            assert_ne!(reduction.state, V1MatchState::Final, "{name}");
        }
    }

    #[test]
    fn rejected_objects_are_reported_as_ignored() {
        let mut world = World::new();
        concession(&mut world);
        world.claim("claim x", OUTSIDER, &win(A), "final");
        world.decision("shopped decision", OUTSIDER, &win(A), false);
        world.objection("outsider objection", OUTSIDER, h("claim a"));
        world.objection("stray objection", B, h("elsewhere"));
        let reduction = world.reduce();
        assert_eq!(reduction.state, V1MatchState::Agreed);
        for name in [
            "claim x",
            "shopped decision",
            "outsider objection",
            "stray objection",
        ] {
            assert!(reduction.ignored.contains(&h(name)), "{name}");
        }
    }

    #[test]
    fn void_objects_are_listed_transitively_across_authors() {
        let mut world = World::new();
        concession(&mut world);
        world.decision("decision", W, &win(A), false);
        world.forks.push(ForkProof { author: A, seq: 6 });
        let reduction = world.reduce();
        assert_eq!(reduction.state, V1MatchState::Void);
        // A's own claim is forked history; the witness's decision names it.
        // B's claim references A's frontier at seq 5, before the fork, so it
        // stands.
        assert_eq!(reduction.void, vec![h("claim a"), h("decision")]);
    }

    #[test]
    fn audits_are_checked_for_closure_without_changing_state() {
        let mut world = World::new();
        concession(&mut world);
        world.audit(
            "clean audit",
            AuditScope::FullHistory,
            AuditVerdict::ResultConfirmed,
        );
        world.audit(
            "dishonest audit",
            AuditScope::Sampled {
                intervals: vec![CheckpointInterval { first: 0, last: 9 }],
            },
            AuditVerdict::ResultConfirmed,
        );
        let reduction = world.reduce();
        assert_eq!(reduction.state, V1MatchState::Agreed);
        assert_eq!(reduction.audits, vec![h("clean audit")]);
        assert!(reduction.ignored.contains(&h("dishonest audit")));

        // A fork at the audited transcript's move voids the audit and the
        // claims that reference that move: the match is void too.
        world.forks.push(ForkProof { author: A, seq: 5 });
        let reduction = world.reduce();
        assert_eq!(reduction.state, V1MatchState::Void);
        assert_eq!(reduction.void_audits, vec![h("clean audit")]);
        assert!(reduction.audits.is_empty());
    }

    #[test]
    fn unknown_references_are_reported_unresolved() {
        let mut world = World::new();
        concession(&mut world);
        world.claims[0].body.frontiers.push(h("unsynced move"));
        let reduction = world.reduce();
        assert_eq!(reduction.state, V1MatchState::Agreed);
        assert_eq!(reduction.unresolved, vec![h("unsynced move")]);
    }

    #[test]
    fn reduction_is_order_independent() {
        let mut world = World::new();
        concession(&mut world);
        world.decision("decision", W, &win(A), false);
        let forward = world.reduce();
        world.claims.reverse();
        world.genesis.reverse();
        let backward = world.reduce();
        assert_eq!(forward.state, backward.state);
        assert_eq!(forward.basis, backward.basis);
    }
}
