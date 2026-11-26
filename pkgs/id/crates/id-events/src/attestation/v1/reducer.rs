//! The v1 attestation reducer (#2039 phases 4 and 5).
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
//!   of this match, audit reports that pass the frozen honest-scope rules
//!   from a key that fits the claimed auditor role (never a seated player).
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
//! - **Witness fallback (phase 5).** Decisions count only from the
//!   genesis-named witness or the fallback pinned in genesis
//!   [`WitnessTerms`]. A fallback decision takes effect only once issuer
//!   receipts show it arrived at least `fallback_delay_secs` after the
//!   issuer first received a claim ([`Reduction::premature`] otherwise); a
//!   primary decision the issuer received no later than the first effective
//!   fallback decision outranks it, and the losers are reported in
//!   [`Reduction::superseded`]. Agreements whose terms fail
//!   [`WitnessTerms::validate`] stay `Proposed`.
//! - **Honest evidence labels.** Complete evidence must name two claims by
//!   different players; partial evidence must not list a claim from the
//!   player it calls missing, and its non-receipt statement must be the
//!   deciding witness's own event. A mislabeled decision is ignored.
//! - **Finality only from receipts.** Only the receipt issuer named in the
//!   terms can sign receipts and closures. A witnessed result becomes
//!   [`V1MatchState::Final`] when an issuer closure statement checks out:
//!   the issuer received the decision at `t0`, and the closure comes no
//!   earlier than each seat's deadline (the objection window from the
//!   issuer serving that seat, capped at `t0 + objection_cap_secs`; the cap
//!   for a seat never served). Once a closure is proven, only objections the
//!   issuer received by the objector's deadline still dispute; late and
//!   never-received objections are reported, not counted. A self-authored
//!   frontier never proves first receipt. With no issuer (an offline match)
//!   a witnessed result stays `Witnessed`, with
//!   [`Finality::NoReceiptIssuer`]; a concession stays `Agreed`.

use std::collections::{HashMap, HashSet};

use p2panda_core::Hash;

use super::audit::AuditReport;
use super::receipt::{Closure, Receipt, ReceiptEvent, WitnessTerms};
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
            seq: u64::from(event.header.seq_num),
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
    /// Witness decisions, from the primary witness or the fallback.
    pub decisions: &'a [Signed<WitnessDecision>],
    /// Objections.
    pub objections: &'a [Signed<Objection>],
    /// Audit reports.
    pub audits: &'a [Signed<AuditReport>],
    /// Receipt-issuer receipts.
    pub receipts: &'a [Signed<Receipt>],
    /// Receipt-issuer closure statements.
    pub closures: &'a [Signed<Closure>],
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
    /// A named witness decided on complete evidence.
    WitnessComplete,
    /// A named witness decided on partial evidence only; procedural, not
    /// replay proof.
    WitnessPartial,
}

/// Whether a witnessed result is protocol-final.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finality {
    /// The state is not a witnessed result.
    NotApplicable,
    /// The agreement names no receipt issuer (an offline match): nothing can
    /// prove when the objection window closed, so the result stays
    /// [`V1MatchState::Witnessed`] and is never final.
    NoReceiptIssuer,
    /// A receipt issuer is named but no closure statement checks out against
    /// its receipts yet.
    Unproven,
    /// This issuer closure statement closed the window: the state is
    /// [`V1MatchState::Final`].
    Proven {
        /// The closure statement's id.
        closure: Hash,
    },
}

/// The reducer's answer, with the evidence it could not use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reduction {
    /// The computed state.
    pub state: V1MatchState,
    /// The basis of a non-disputed, non-void result.
    pub basis: Basis,
    /// Whether the result is protocol-final; `true` exactly when
    /// [`Reduction::finality`] is [`Finality::Proven`].
    pub finality_proven: bool,
    /// Why a witnessed result is or is not final.
    pub finality: Finality,
    /// Admitted match objects voided by fork evidence, in input order.
    pub void: Vec<Hash>,
    /// Admitted audit reports voided by fork evidence. They do not change
    /// the state; a registry must not accept them as a basis.
    pub void_audits: Vec<Hash>,
    /// Admitted audit reports with intact evidence closure.
    pub audits: Vec<Hash>,
    /// Issuer receipts and closures voided by fork evidence. They do not
    /// void the match; they simply prove nothing.
    pub void_receipts: Vec<Hash>,
    /// Witness decisions that lost to another witness under the pinned
    /// fallback rule: a fallback decision when the primary decided in time,
    /// or a primary decision the issuer first received after a fallback
    /// decision took effect.
    pub superseded: Vec<Hash>,
    /// Fallback decisions without receipts proving the primary's delay had
    /// run out. They count for nothing until such receipts appear.
    pub premature: Vec<Hash>,
    /// Objections the issuer first received after the objector's window
    /// closed. Reported only once a closure is proven.
    pub late_objections: Vec<Hash>,
    /// Objections the issuer never received: a self-authored frontier does
    /// not show when an objection was lodged. Reported only once a closure
    /// is proven.
    pub unlodged_objections: Vec<Hash>,
    /// Referenced hashes with no known position or object: the reducer
    /// cannot check them against forks until they are synced.
    pub unresolved: Vec<Hash>,
    /// Objects that name this agreement but failed admission (wrong author,
    /// wrong witness or logic, malformed or mislabeled outcome or evidence,
    /// dishonest audit scope, receipts not from the issuer), and seated
    /// objections whose target is outside this match.
    pub ignored: Vec<Hash>,
}

impl Reduction {
    const fn with_state(state: V1MatchState) -> Self {
        Self {
            state,
            basis: Basis::None,
            finality_proven: false,
            finality: Finality::NotApplicable,
            void: Vec::new(),
            void_audits: Vec::new(),
            audits: Vec::new(),
            void_receipts: Vec::new(),
            superseded: Vec::new(),
            premature: Vec::new(),
            late_objections: Vec::new(),
            unlodged_objections: Vec::new(),
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

/// Whether a decision's evidence label matches the evidence it names, as far
/// as the known objects can tell. Complete evidence must name two claims by
/// different players, never some other known object; partial evidence must
/// not list a claim from the player it says was missing, and its non-receipt
/// statement must be the deciding witness's own event, never a claim or
/// another author's event. References nobody has synced stay unresolved.
fn honest_label(
    decision: &Signed<WitnessDecision>,
    claim_author: &HashMap<Hash, PlayerId>,
    known: &HashMap<Hash, (PlayerId, u64)>,
) -> bool {
    match &decision.body.evidence {
        WitnessEvidence::Complete {
            claim_a, claim_b, ..
        } => {
            let not_a_claim = |id: &Hash| known.contains_key(id) && !claim_author.contains_key(id);
            if not_a_claim(claim_a) || not_a_claim(claim_b) {
                return false;
            }
            !matches!(
                (claim_author.get(claim_a), claim_author.get(claim_b)),
                (Some(a), Some(b)) if a == b
            )
        }
        WitnessEvidence::Partial {
            received,
            missing_from,
            non_receipt_statement,
            ..
        } => {
            !received
                .iter()
                .any(|id| claim_author.get(id) == Some(missing_from))
                && !claim_author.contains_key(non_receipt_statement)
                && known
                    .get(non_receipt_statement)
                    .is_none_or(|(author, _)| *author == decision.author)
        }
    }
}

/// Issuer receipt times. Every query takes the earliest matching receipt.
struct Clock<'e> {
    receipts: Vec<&'e Signed<Receipt>>,
}

impl Clock<'_> {
    fn first(&self, subject: Hash, event: ReceiptEvent) -> Option<u64> {
        self.receipts
            .iter()
            .filter(|object| object.body.subject == subject && object.body.event == event)
            .map(|object| object.body.at_secs)
            .min()
    }

    fn received(&self, subject: Hash, from: PlayerId) -> Option<u64> {
        self.first(subject, ReceiptEvent::Received { from })
    }

    fn served(&self, subject: Hash, to: PlayerId) -> Option<u64> {
        self.first(subject, ReceiptEvent::Served { to })
    }
}

/// The earliest closure statement that checks out against the receipts, with
/// each seat's objection deadline for its decision. A decision's window opens
/// when the issuer first received it (`t0`). A seat the issuer served runs
/// `objection_window_secs` from that service; an unserved seat runs to the
/// cap; neither runs past `t0 + objection_cap_secs`. The closure must come
/// no earlier than the later deadline.
fn prove_closure(
    closures: &[&Signed<Closure>],
    decisions: &[&Signed<WitnessDecision>],
    clock: &Clock<'_>,
    agreement: &GenesisAgreement,
    cap_secs: u64,
) -> Option<(Hash, [u64; 2])> {
    closures
        .iter()
        .filter_map(|closure| {
            let decision = decisions
                .iter()
                .find(|decision| decision.id == closure.body.decision)?;
            let opened = clock.received(decision.id, decision.author)?;
            let limit = opened.saturating_add(cap_secs);
            let deadlines = agreement.players.map(|player| {
                clock.served(decision.id, player).map_or(limit, |served| {
                    served
                        .max(opened)
                        .saturating_add(agreement.objection_window_secs)
                        .min(limit)
                })
            });
            let latest = deadlines[0].max(deadlines[1]);
            (closure.body.closed_at_secs >= latest).then_some((
                closure.body.closed_at_secs,
                closure.id,
                deadlines,
            ))
        })
        .min_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
        })
        .map(|(_, id, deadlines)| (id, deadlines))
}

/// Computes the v1 match state for `agreement` from `evidence`. See the
/// module documentation for the rules.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn reduce(agreement: &GenesisAgreement, evidence: &Evidence<'_>) -> Reduction {
    let [first, second] = agreement.players;
    if first == second {
        return Reduction::with_state(V1MatchState::Proposed);
    }
    let terms = WitnessTerms::of(agreement);
    if terms.validate(agreement).is_err() {
        return Reduction::with_state(V1MatchState::Proposed);
    }
    let Ok(agreement_hash) = agreement.agreement_hash() else {
        return Reduction::with_state(V1MatchState::Proposed);
    };
    let seated = |player: &PlayerId| agreement.players.contains(player);
    let player_hex = agreement.players.map(|player| hex(&player));
    let outcome_ok = |outcome: &str| outcome == "draw" || player_hex.iter().any(|h| h == outcome);
    let forked = |author: &PlayerId, seq: u64| {
        evidence
            .forks
            .iter()
            .any(|proof| proof.author == *author && proof.seq <= seq)
    };
    let mut ignored = Vec::new();

    // Every signed object's position, admitted or not: a reference to a
    // known object is never "unresolved", and mislabel checks need authors.
    let mut known: HashMap<Hash, (PlayerId, u64)> = evidence
        .positions
        .iter()
        .map(|position| (position.id, (position.author, position.seq)))
        .collect();
    {
        let mut note = |id: Hash, author: PlayerId, seq: u64| {
            known.insert(id, (author, seq));
        };
        evidence
            .genesis
            .iter()
            .for_each(|o| note(o.id, o.author, o.seq));
        evidence
            .claims
            .iter()
            .for_each(|o| note(o.id, o.author, o.seq));
        evidence
            .decisions
            .iter()
            .for_each(|o| note(o.id, o.author, o.seq));
        evidence
            .objections
            .iter()
            .for_each(|o| note(o.id, o.author, o.seq));
        evidence
            .audits
            .iter()
            .for_each(|o| note(o.id, o.author, o.seq));
        evidence
            .receipts
            .iter()
            .for_each(|o| note(o.id, o.author, o.seq));
        evidence
            .closures
            .iter()
            .for_each(|o| note(o.id, o.author, o.seq));
    }

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
    let claim_author: HashMap<Hash, PlayerId> = claims
        .iter()
        .map(|object| (object.id, object.author))
        .collect();
    let mut primary = Vec::new();
    let mut fallback = Vec::new();
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
        let well_formed = decision.version == VERSION
            && decision.logic_hash == agreement.logic_hash
            && outcome_ok(&decision.outcome)
            && evidence_ok
            && honest_label(object, &claim_author, &known);
        if well_formed && agreement.witness == Some(object.author) {
            primary.push(object);
        } else if well_formed && terms.fallback == Some(object.author) {
            fallback.push(object);
        } else {
            ignored.push(object.id);
        }
    }
    let mut audits = Vec::new();
    for object in evidence.audits {
        if object.body.agreement != agreement_hash {
            continue;
        }
        if object.body.validate().is_ok()
            && object.body.logic_hash == agreement.logic_hash
            && object.body.match_id == agreement.game_id
            && object.body.role.fits(agreement, &object.author)
        {
            audits.push(object);
        } else {
            ignored.push(object.id);
        }
    }
    let from_issuer = |author: &PlayerId, version: u8| {
        terms.receipt_issuer.as_ref() == Some(author) && version == VERSION
    };
    let mut receipts = Vec::new();
    for object in evidence.receipts {
        if object.body.agreement != agreement_hash {
            continue;
        }
        if from_issuer(&object.author, object.body.version) {
            receipts.push(object);
        } else {
            ignored.push(object.id);
        }
    }
    // Receipt times come only from the issuer's unforked history.
    let clock = Clock {
        receipts: receipts
            .iter()
            .copied()
            .filter(|object| !forked(&object.author, object.seq))
            .collect(),
    };

    // The pinned fallback rule. A fallback decision takes effect only once
    // the issuer received it at least `fallback_delay_secs` after it first
    // received any claim; a primary decision the issuer received no later
    // than the first effective fallback decision outranks every fallback
    // decision. Nobody chooses among witnesses after play.
    let mut superseded = Vec::new();
    let mut premature = Vec::new();
    let opened = claims
        .iter()
        .filter_map(|object| clock.received(object.id, object.author))
        .min();
    let mut timed = Vec::new();
    for object in fallback {
        match (clock.received(object.id, object.author), opened) {
            (Some(at), Some(opened)) if at >= opened.saturating_add(terms.fallback_delay_secs) => {
                timed.push((at, object));
            }
            _ => premature.push(object.id),
        }
    }
    let takeover = timed.iter().map(|(at, _)| *at).min();
    let (in_time, late): (Vec<_>, Vec<_>) = primary.into_iter().partition(|object| {
        takeover.is_none_or(|takeover| {
            clock
                .received(object.id, object.author)
                .is_some_and(|at| at <= takeover)
        })
    });
    superseded.extend(late.iter().map(|object| object.id));
    let decisions: Vec<&Signed<WitnessDecision>> = if in_time.is_empty() {
        timed.into_iter().map(|(_, object)| object).collect()
    } else {
        superseded.extend(timed.iter().map(|(_, object)| object.id));
        in_time
    };

    let mut closures = Vec::new();
    for object in evidence.closures {
        if object.body.agreement != agreement_hash {
            continue;
        }
        let names_decision = decisions
            .iter()
            .any(|decision| decision.id == object.body.decision);
        if from_issuer(&object.author, object.body.version) && names_decision {
            closures.push(object);
        } else {
            ignored.push(object.id);
        }
    }
    let targets: HashSet<Hash> = std::iter::once(agreement_hash)
        .chain(genesis.iter().map(|object| object.id))
        .chain(claims.iter().map(|object| object.id))
        .chain(decisions.iter().map(|object| object.id))
        .chain(audits.iter().map(|object| object.id))
        .chain(closures.iter().map(|object| object.id))
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
    let receipt_start = nodes.len();
    nodes.extend(
        receipts
            .iter()
            .map(|&object| Node::of(object, std::iter::once(object.body.subject))),
    );
    nodes.extend(
        closures
            .iter()
            .map(|&object| Node::of(object, std::iter::once(object.body.decision))),
    );

    let mut positions = known;
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
    let (match_nodes, aux_nodes) = nodes.split_at(audit_start);
    let (audit_nodes, receipt_nodes) = aux_nodes.split_at(receipt_start - audit_start);
    let void_objects: Vec<Hash> = match_nodes
        .iter()
        .filter(|node| void.contains(&node.id))
        .map(|node| node.id)
        .collect();
    let (void_audits, live_audits): (Vec<Hash>, Vec<Hash>) = audit_nodes
        .iter()
        .map(|node| node.id)
        .partition(|id| void.contains(id));
    let void_receipts: Vec<Hash> = receipt_nodes
        .iter()
        .map(|node| node.id)
        .filter(|id| void.contains(id))
        .collect();
    let clock = Clock {
        receipts: clock
            .receipts
            .into_iter()
            .filter(|object| !void.contains(&object.id))
            .collect(),
    };
    closures.retain(|object| !void.contains(&object.id));

    let mut result = Reduction {
        void: void_objects,
        void_audits,
        audits: live_audits,
        void_receipts,
        superseded,
        premature,
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

    // A proven closure ends the objection window: only objections the
    // issuer received by the objector's deadline still count. Without one,
    // every admitted objection counts.
    let proven = terms.receipt_issuer.and_then(|_| {
        prove_closure(
            &closures,
            &decisions,
            &clock,
            agreement,
            terms.objection_cap_secs,
        )
    });
    if let Some((_, deadlines)) = proven {
        objections.retain(|object| {
            let seat = usize::from(object.author == agreement.players[1]);
            match clock.received(object.id, object.author) {
                None => {
                    result.unlodged_objections.push(object.id);
                    false
                }
                Some(at) if at > deadlines[seat] => {
                    result.late_objections.push(object.id);
                    false
                }
                Some(_) => true,
            }
        });
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
            result.finality = match (terms.receipt_issuer, proven) {
                (None, _) => Finality::NoReceiptIssuer,
                (Some(_), None) => Finality::Unproven,
                (Some(_), Some((closure, _))) => {
                    result.state = V1MatchState::Final;
                    result.finality_proven = true;
                    Finality::Proven { closure }
                }
            };
        }
        _ => {}
    }
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Basis, EventPosition, Evidence, Finality, ForkProof, Signed, hex, reduce};
    use crate::attestation::v1::audit::{
        AuditReport, AuditScope, AuditVerdict, AuditorRole, CheckpointInterval,
    };
    use crate::attestation::v1::receipt::{Closure, Receipt, ReceiptEvent, WitnessTerms};
    use crate::attestation::v1::{
        Objection, V1MatchState, VERSION, WitnessDecision, WitnessEvidence,
    };
    use crate::attestation::{GenesisAgreement, PlayerId, ResultClaim};
    use p2panda_core::Hash;

    const A: PlayerId = [1; 32];
    const B: PlayerId = [2; 32];
    const W: PlayerId = [3; 32];
    const OUTSIDER: PlayerId = [4; 32];
    const FALLBACK: PlayerId = [5; 32];
    const ISSUER: PlayerId = [6; 32];

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
            witness_terms: None,
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
        receipts: Vec<Signed<Receipt>>,
        closures: Vec<Signed<Closure>>,
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
                receipts: Vec::new(),
                closures: Vec::new(),
                positions,
                forks: Vec::new(),
            }
        }

        /// A world whose genesis pins a fallback witness, a receipt issuer
        /// and a two-hour objection cap.
        fn with_terms() -> Self {
            let mut world = Self::new();
            world.agreement.witness_terms = Some(WitnessTerms {
                fallback: Some(FALLBACK),
                fallback_delay_secs: 600,
                receipt_issuer: Some(ISSUER),
                objection_cap_secs: 7200,
            });
            world.regenesis();
            world
        }

        /// Re-signs both genesis events over the current agreement.
        fn regenesis(&mut self) {
            for object in &mut self.genesis {
                object.body = self.agreement.clone();
            }
        }

        fn receipt(&mut self, name: &str, subject: &str, event: ReceiptEvent, at_secs: u64) {
            let body = Receipt {
                version: VERSION,
                agreement: self.agreement.agreement_hash().unwrap(),
                subject: h(subject),
                event,
                at_secs,
            };
            let seq = u64::try_from(self.receipts.len() + self.closures.len()).unwrap();
            self.receipts.push(signed(name, ISSUER, seq, &[], body));
        }

        fn received(&mut self, subject: &str, from: PlayerId, at_secs: u64) {
            let name = format!("received {subject}");
            self.receipt(&name, subject, ReceiptEvent::Received { from }, at_secs);
        }

        fn served(&mut self, subject: &str, to: PlayerId, at_secs: u64) {
            let name = format!("served {subject} {}", to[0]);
            self.receipt(&name, subject, ReceiptEvent::Served { to }, at_secs);
        }

        fn close(&mut self, decision: &str, closed_at_secs: u64) {
            let body = Closure {
                version: VERSION,
                agreement: self.agreement.agreement_hash().unwrap(),
                decision: h(decision),
                closed_at_secs,
            };
            let seq = u64::try_from(self.receipts.len() + self.closures.len()).unwrap();
            self.closures
                .push(signed("closure", ISSUER, seq, &[], body));
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
                    receipts: &self.receipts,
                    closures: &self.closures,
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

    /// A partial witness decision the issuer received, served to both seats
    /// and closed at the later seat's deadline (`210 + 3600` for A,
    /// `220 + 3600` for B).
    fn closed(w: &mut World) {
        w.claim("claim a", A, &win(A), "final");
        w.received("claim a", A, 100);
        w.decision("decision", W, &win(A), true);
        w.received("decision", W, 200);
        w.served("decision", A, 210);
        w.served("decision", B, 220);
        w.close("decision", 3820);
    }

    fn objected(w: &mut World, lodged_at: Option<u64>) {
        closed(w);
        w.objection("objection", B, h("decision"));
        if let Some(at) = lodged_at {
            w.received("objection", B, at);
        }
    }

    fn fallback_after(w: &mut World, at: u64) {
        w.claim("claim a", A, &win(A), "final");
        w.received("claim a", A, 100);
        w.decision("fallback", FALLBACK, &win(A), true);
        w.received("fallback", FALLBACK, at);
    }

    type TermsRow = (&'static str, fn(&mut World), V1MatchState, Basis, Finality);

    #[test]
    fn receipt_and_fallback_table() {
        let unproven = Finality::Unproven;
        let proven = Finality::Proven {
            closure: h("closure"),
        };
        let rows: &[TermsRow] = &[
            (
                "a closure at both seats' deadlines is final",
                closed,
                V1MatchState::Final,
                Basis::WitnessPartial,
                proven,
            ),
            (
                "a closure before the slower seat's deadline proves nothing",
                |w| {
                    closed(w);
                    w.closures[0].body.closed_at_secs = 3819;
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "an unserved seat's window runs to the cap",
                |w| {
                    closed(w);
                    w.receipts
                        .retain(|object| object.id != h("served decision 2"));
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a closure at the cap covers an unserved seat",
                |w| {
                    closed(w);
                    w.receipts
                        .retain(|object| object.id != h("served decision 2"));
                    w.closures[0].body.closed_at_secs = 7400;
                },
                V1MatchState::Final,
                Basis::WitnessPartial,
                proven,
            ),
            (
                "the cap bounds a late service",
                |w| {
                    closed(w);
                    w.receipts
                        .retain(|object| object.id != h("served decision 2"));
                    w.served("decision", B, 7000);
                    w.closures[0].body.closed_at_secs = 7400;
                },
                V1MatchState::Final,
                Basis::WitnessPartial,
                proven,
            ),
            (
                "no receipt of the decision means no window opened",
                |w| {
                    closed(w);
                    w.receipts
                        .retain(|object| object.id != h("received decision"));
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a closure signed by the witness is ignored",
                |w| {
                    closed(w);
                    w.closures[0].author = W;
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a closure naming another decision is ignored",
                |w| {
                    closed(w);
                    w.closures[0].body.decision = h("another decision");
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a receipt signed by the witness proves nothing",
                |w| {
                    closed(w);
                    for object in &mut w.receipts {
                        if object.id == h("received decision") {
                            object.author = W;
                        }
                    }
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "forked issuer history proves nothing and voids nothing",
                |w| {
                    closed(w);
                    w.forks.push(ForkProof {
                        author: ISSUER,
                        seq: 0,
                    });
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a timely objection disputes a closed result",
                |w| objected(w, Some(3000)),
                V1MatchState::Disputed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "an objection lodged at the deadline still counts",
                |w| objected(w, Some(3820)),
                V1MatchState::Disputed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "a late objection does not reopen a final result",
                |w| objected(w, Some(3821)),
                V1MatchState::Final,
                Basis::WitnessPartial,
                proven,
            ),
            (
                "an objection's own frontier does not prove it was lodged",
                |w| {
                    objected(w, None);
                    w.objections[0].body.author_frontier = vec![h("move b 5")];
                },
                V1MatchState::Final,
                Basis::WitnessPartial,
                proven,
            ),
            (
                "every objection counts until a closure is proven",
                |w| {
                    objected(w, None);
                    w.closures.clear();
                },
                V1MatchState::Disputed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "a concession is not made final by a closure",
                |w| {
                    closed(w);
                    w.claim("claim b", B, &win(A), "final");
                },
                V1MatchState::Agreed,
                Basis::Concession,
                Finality::NotApplicable,
            ),
            (
                "a fallback decision after the delay is witnessed",
                |w| fallback_after(w, 700),
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a fallback decision can be closed",
                |w| {
                    fallback_after(w, 700);
                    w.close("fallback", 7900);
                },
                V1MatchState::Final,
                Basis::WitnessPartial,
                proven,
            ),
            (
                "a fallback decision before the delay is premature",
                |w| fallback_after(w, 699),
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "a fallback decision the issuer never received is premature",
                |w| {
                    fallback_after(w, 700);
                    w.receipts
                        .retain(|object| object.id != h("received fallback"));
                },
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "a fallback decision with no received claim is premature",
                |w| {
                    fallback_after(w, 700);
                    w.receipts
                        .retain(|object| object.id != h("received claim a"));
                },
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "a primary decision in time outranks a contrary fallback",
                |w| {
                    fallback_after(w, 700);
                    w.decisions[0].body.outcome = win(B);
                    w.decision("decision", W, &win(A), true);
                    w.received("decision", W, 650);
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a primary decision after the fallback took over is superseded",
                |w| {
                    fallback_after(w, 700);
                    w.decision("decision", W, &win(B), true);
                    w.received("decision", W, 800);
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a primary decision the issuer never received loses to the fallback",
                |w| {
                    fallback_after(w, 700);
                    w.decision("decision", W, &win(B), true);
                },
                V1MatchState::Witnessed,
                Basis::WitnessPartial,
                unproven,
            ),
            (
                "a primary decision stands while no fallback has taken effect",
                |w| w.decision("decision", W, &win(A), false),
                V1MatchState::Witnessed,
                Basis::WitnessComplete,
                unproven,
            ),
            (
                "complete evidence naming one player's two claims is ignored",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.claim("claim b", A, &win(A), "final");
                    w.decision("decision", W, &win(A), false);
                },
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "complete evidence naming a move instead of a claim is ignored",
                |w| {
                    w.decision("decision", W, &win(A), false);
                    if let WitnessEvidence::Complete { claim_b, .. } =
                        &mut w.decisions[0].body.evidence
                    {
                        *claim_b = h("move b 5");
                    }
                },
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "partial evidence holding the missing player's claim is ignored",
                |w| {
                    w.claim("claim b", B, &win(A), "final");
                    w.decision("decision", W, &win(A), true);
                    if let WitnessEvidence::Partial { received, .. } =
                        &mut w.decisions[0].body.evidence
                    {
                        *received = vec![h("claim b")];
                    }
                },
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "a non-receipt statement signed by a player is ignored",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.decision("decision", W, &win(A), true);
                    if let WitnessEvidence::Partial {
                        non_receipt_statement,
                        ..
                    } = &mut w.decisions[0].body.evidence
                    {
                        *non_receipt_statement = h("move b 5");
                    }
                },
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "a non-receipt statement that is a claim is ignored",
                |w| {
                    w.claim("claim a", A, &win(A), "final");
                    w.decision("decision", W, &win(A), true);
                    if let WitnessEvidence::Partial {
                        non_receipt_statement,
                        ..
                    } = &mut w.decisions[0].body.evidence
                    {
                        *non_receipt_statement = h("claim a");
                    }
                },
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
            (
                "unusable terms leave the match proposed",
                |w| {
                    closed(w);
                    w.agreement
                        .witness_terms
                        .as_mut()
                        .unwrap()
                        .objection_cap_secs = 0;
                    w.regenesis();
                },
                V1MatchState::Proposed,
                Basis::None,
                Finality::NotApplicable,
            ),
        ];
        for &(name, edit, state, basis, finality) in rows {
            let mut world = World::with_terms();
            edit(&mut world);
            let reduction = world.reduce();
            assert_eq!(reduction.state, state, "{name}");
            assert_eq!(reduction.basis, basis, "{name}");
            assert_eq!(reduction.finality, finality, "{name}");
            assert_eq!(
                reduction.finality_proven,
                matches!(finality, Finality::Proven { .. }),
                "{name}"
            );
        }
    }

    #[test]
    fn an_offline_witnessed_result_is_explicitly_not_final() {
        let mut world = World::new();
        world.decision("decision", W, &win(A), false);
        // Receipts and closures mean nothing without an issuer in genesis.
        world.received("decision", W, 200);
        world.close("decision", 9000);
        let reduction = world.reduce();
        assert_eq!(reduction.state, V1MatchState::Witnessed);
        assert_eq!(reduction.finality, Finality::NoReceiptIssuer);
        assert!(!reduction.finality_proven);
        assert!(reduction.ignored.contains(&h("received decision")));
        assert!(reduction.ignored.contains(&h("closure")));
    }

    #[test]
    fn a_fallback_is_only_a_witness_when_genesis_names_it() {
        let mut world = World::new();
        world.decision("fallback", FALLBACK, &win(A), false);
        let reduction = world.reduce();
        assert_eq!(reduction.state, V1MatchState::Proposed);
        assert!(reduction.ignored.contains(&h("fallback")));
    }

    #[test]
    fn closure_reports_late_and_unlodged_objections() {
        let mut world = World::with_terms();
        closed(&mut world);
        world.objection("late objection", B, h("decision"));
        world.received("late objection", B, 4000);
        world.objection("unlodged objection", A, h("claim a"));
        let reduction = world.reduce();
        assert_eq!(reduction.state, V1MatchState::Final);
        assert_eq!(reduction.late_objections, vec![h("late objection")]);
        assert_eq!(reduction.unlodged_objections, vec![h("unlodged objection")]);
    }

    #[test]
    fn fallback_supersession_and_prematurity_are_reported() {
        let mut world = World::with_terms();
        fallback_after(&mut world, 700);
        world.decision("decision", W, &win(A), true);
        world.received("decision", W, 650);
        let reduction = world.reduce();
        assert_eq!(reduction.superseded, vec![h("fallback")]);
        assert!(reduction.premature.is_empty());

        let mut world = World::with_terms();
        fallback_after(&mut world, 699);
        let reduction = world.reduce();
        assert_eq!(reduction.premature, vec![h("fallback")]);
        assert!(reduction.superseded.is_empty());
    }

    #[test]
    fn forked_issuer_receipts_are_reported_void() {
        let mut world = World::with_terms();
        closed(&mut world);
        world.forks.push(ForkProof {
            author: ISSUER,
            seq: 4,
        });
        let reduction = world.reduce();
        assert!(reduction.void.is_empty());
        assert_eq!(reduction.void_receipts, vec![h("closure")]);
        assert_eq!(reduction.finality, Finality::Unproven);
    }
}
