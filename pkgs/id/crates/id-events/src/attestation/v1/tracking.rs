//! Rebuildable match tracking view (#2039 phase 7).
//!
//! A tracker is a projection over synced evidence, not an authority over
//! live play (design `doc/2026-11-01_design_id2039.md` §7). It holds an
//! [`EvidenceSet`] fed from any number of sync sources and rebuilds a
//! [`TrackingView`] from it under a named [`ViewConfig`]:
//!
//! - The evidence set is keyed by object id, so the same objects synced from
//!   several sources, in any order or more than once, make the same set.
//!   Fork proofs merge to the earliest forked sequence per author.
//! - [`rebuild`] reads the set in id order and runs the reducer and the
//!   registry handoff for every agreement it holds. Two replicas with the
//!   same evidence and config produce equal views.
//! - Every view names its config (name, version and a hash over the policy)
//!   and its evidence frontier (a hash over every object id and fork proof
//!   it read), so two views can be compared and a differing policy is
//!   visible. The signed objects are only read, never changed.
//! - Counting is the config's choice: the reducer state, the registry
//!   basis and the outcome stay as computed, and each match carries a
//!   [`Count`] that says whether this view counts it and, if not, why.

use std::collections::{BTreeMap, BTreeSet};

use p2panda_core::Hash;

use super::audit::AuditReport;
use super::receipt::{Closure, Receipt};
use super::reducer::{EventPosition, Evidence, ForkProof, Signed, reduce};
use super::registry::{AuditPolicy, CountingBasis, ReportStanding, handoff};
use super::{CountingPolicy, Objection, V1MatchState, WitnessDecision};
use crate::attestation::{GenesisAgreement, PlayerId, ResultClaim};

/// A tracker's named view policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewConfig {
    /// The view's name.
    pub name: String,
    /// The config version; bump it whenever the policy changes.
    pub version: u32,
    /// The players whose matches this view tracks. Empty tracks any.
    pub players: Vec<PlayerId>,
    /// Which states, witnesses and game logic count.
    pub counting: CountingPolicy,
    /// Whether a result witnessed on partial evidence only counts.
    pub count_witness_partial: bool,
    /// Which audit reports this view relies on, and whether it counts
    /// `audit-resolved`.
    pub audit: AuditPolicy,
}

fn len(n: usize) -> [u8; 8] {
    u64::try_from(n).unwrap_or(u64::MAX).to_le_bytes()
}

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&len(bytes.len()));
    out.extend_from_slice(bytes);
}

fn sorted<T: Ord + Clone>(items: &[T]) -> Vec<T> {
    items
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

impl ViewConfig {
    /// A hash over the whole policy. Lists are read as sets, so their order
    /// and duplicates do not change it.
    #[must_use]
    pub fn hash(&self) -> Hash {
        let mut out = b"attestation.view.v1".to_vec();
        put(&mut out, self.name.as_bytes());
        put(&mut out, &self.version.to_le_bytes());
        for player in sorted(&self.players) {
            put(&mut out, &player);
        }
        put(&mut out, b"states");
        let states = self
            .counting
            .accepted_states
            .iter()
            .map(|state| format!("{state:?}"))
            .collect::<Vec<_>>();
        for state in sorted(&states) {
            put(&mut out, state.as_bytes());
        }
        put(&mut out, b"witnesses");
        for witness in sorted(&self.counting.accepted_witnesses) {
            put(&mut out, &witness);
        }
        put(&mut out, b"logic");
        for logic in sorted(&self.counting.accepted_logic_hashes) {
            put(&mut out, logic.as_bytes());
        }
        put(&mut out, &[u8::from(self.count_witness_partial)]);
        put(&mut out, &[u8::from(self.audit.accept_audit_resolved)]);
        put(&mut out, b"auditors");
        for auditor in sorted(&self.audit.accepted_auditors) {
            put(&mut out, &auditor);
        }
        put(&mut out, b"runtimes");
        for runtime in sorted(&self.audit.accepted_runtimes) {
            put(&mut out, runtime.as_bytes());
        }
        Hash::digest(&out)
    }
}

/// Synced, verified evidence, deduplicated by object id.
///
/// The caller verifies each event and decodes its body before ingesting
/// (as for [`Signed::admit`]). An object id commits to its signed bytes, so
/// two objects with one id are the same object; the first copy is kept.
#[derive(Clone, Debug, Default)]
pub struct EvidenceSet {
    genesis: BTreeMap<Hash, Signed<GenesisAgreement>>,
    claims: BTreeMap<Hash, Signed<ResultClaim>>,
    decisions: BTreeMap<Hash, Signed<WitnessDecision>>,
    objections: BTreeMap<Hash, Signed<Objection>>,
    audits: BTreeMap<Hash, Signed<AuditReport>>,
    receipts: BTreeMap<Hash, Signed<Receipt>>,
    closures: BTreeMap<Hash, Signed<Closure>>,
    positions: BTreeMap<Hash, EventPosition>,
    forks: BTreeMap<PlayerId, u64>,
}

fn merge<T: Clone>(into: &mut BTreeMap<Hash, Signed<T>>, objects: &[Signed<T>]) -> usize {
    let before = into.len();
    for object in objects {
        into.entry(object.id).or_insert_with(|| object.clone());
    }
    into.len() - before
}

/// The evidence a view was built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frontier {
    /// How many objects, positions and fork proofs the set held.
    pub objects: usize,
    /// A hash over every id and fork proof, in id order.
    pub hash: Hash,
}

impl EvidenceSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Merges one sync batch and returns how many entries were new. A fork
    /// proof is new when it moves an author's earliest forked sequence down.
    pub fn ingest(&mut self, batch: &Evidence<'_>) -> usize {
        let mut added = merge(&mut self.genesis, batch.genesis)
            + merge(&mut self.claims, batch.claims)
            + merge(&mut self.decisions, batch.decisions)
            + merge(&mut self.objections, batch.objections)
            + merge(&mut self.audits, batch.audits)
            + merge(&mut self.receipts, batch.receipts)
            + merge(&mut self.closures, batch.closures);
        for position in batch.positions {
            if !self.positions.contains_key(&position.id) {
                self.positions.insert(position.id, *position);
                added += 1;
            }
        }
        for fork in batch.forks {
            let seq = self.forks.entry(fork.author).or_insert(u64::MAX);
            if fork.seq < *seq {
                *seq = fork.seq;
                added += 1;
            }
        }
        added
    }

    /// What this set holds, as a comparable frontier.
    #[must_use]
    pub fn frontier(&self) -> Frontier {
        let mut out = b"attestation.frontier.v1".to_vec();
        let mut objects = 0;
        let ids = [
            self.genesis.keys().collect::<Vec<_>>(),
            self.claims.keys().collect(),
            self.decisions.keys().collect(),
            self.objections.keys().collect(),
            self.audits.keys().collect(),
            self.receipts.keys().collect(),
            self.closures.keys().collect(),
            self.positions.keys().collect(),
        ];
        for kind in ids {
            put(&mut out, &len(kind.len()));
            for id in kind {
                out.extend_from_slice(id.as_bytes());
                objects += 1;
            }
        }
        put(&mut out, &len(self.forks.len()));
        for (author, seq) in &self.forks {
            out.extend_from_slice(author);
            out.extend_from_slice(&seq.to_le_bytes());
            objects += 1;
        }
        Frontier {
            objects,
            hash: Hash::digest(&out),
        }
    }
}

/// Why a view does not count a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotCounted {
    /// No result to count: proposed, void, or disputed without an accepted
    /// audit resolution.
    NoResult,
    /// A seated player is outside the view's tracked players.
    UntrackedPlayer,
    /// The view does not count this state.
    State,
    /// The view does not accept the agreement's witness or game logic.
    Agreement,
    /// Witnessed on partial evidence, which this view does not count.
    PartialWitness,
}

/// Whether a view counts a match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Count {
    /// Counted, with this outcome.
    Counted {
        /// The counted outcome.
        outcome: String,
    },
    /// Not counted, and why.
    NotCounted(NotCounted),
}

/// One match as a view reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchEntry {
    /// The agreement hash.
    pub agreement: Hash,
    /// The game id.
    pub game_id: String,
    /// The seated players.
    pub players: [PlayerId; 2],
    /// The reducer's state, unchanged.
    pub state: V1MatchState,
    /// The basis this view's audit policy reads.
    pub basis: CountingBasis,
    /// The reducer's outcome, if there is a result.
    pub outcome: Option<String>,
    /// Whether the result is protocol-final.
    pub finality_proven: bool,
    /// Match objects voided by fork evidence.
    pub void: Vec<Hash>,
    /// Audit reports voided by fork evidence.
    pub void_audits: Vec<Hash>,
    /// Every live audit report's standing under this view's policy.
    pub reports: Vec<ReportStanding>,
    /// This view's count.
    pub count: Count,
}

/// A rebuilt view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackingView {
    /// The view's name.
    pub name: String,
    /// The config version.
    pub version: u32,
    /// [`ViewConfig::hash`] of the config the view was built under.
    pub config: Hash,
    /// The evidence the view was built from.
    pub frontier: Frontier,
    /// Every agreement in the evidence, in agreement-hash order.
    pub matches: Vec<MatchEntry>,
}

impl TrackingView {
    /// The counted matches and their outcomes, in agreement-hash order.
    pub fn counted(&self) -> impl Iterator<Item = (Hash, &str)> {
        self.matches.iter().filter_map(|entry| match &entry.count {
            Count::Counted { outcome } => Some((entry.agreement, outcome.as_str())),
            Count::NotCounted(_) => None,
        })
    }
}

fn count(
    config: &ViewConfig,
    agreement: &GenesisAgreement,
    state: V1MatchState,
    basis: &CountingBasis,
    outcome: Option<&String>,
) -> Count {
    use NotCounted as Why;
    let outcome = match (basis, outcome) {
        (CountingBasis::AuditResolved { outcome, .. }, _) => outcome.clone(),
        (CountingBasis::None, _) | (_, None) => return Count::NotCounted(Why::NoResult),
        (_, Some(outcome)) => outcome.clone(),
    };
    if !config.players.is_empty()
        && !agreement
            .players
            .iter()
            .all(|player| config.players.contains(player))
    {
        return Count::NotCounted(Why::UntrackedPlayer);
    }
    let resolved = matches!(basis, CountingBasis::AuditResolved { .. });
    // An audit-resolved match stays disputed; the registry basis, not the
    // state, is what this view counts it under.
    if !resolved && !config.counting.accepted_states.contains(&state) {
        return Count::NotCounted(Why::State);
    }
    let counting = CountingPolicy {
        accepted_states: vec![state],
        ..config.counting.clone()
    };
    if !counting.counts(agreement, state) {
        return Count::NotCounted(Why::Agreement);
    }
    if matches!(basis, CountingBasis::WitnessPartial) && !config.count_witness_partial {
        return Count::NotCounted(Why::PartialWitness);
    }
    Count::Counted { outcome }
}

/// Rebuilds a view from the evidence set under `config`.
#[must_use]
pub fn rebuild(config: &ViewConfig, set: &EvidenceSet) -> TrackingView {
    let genesis: Vec<_> = set.genesis.values().cloned().collect();
    let claims: Vec<_> = set.claims.values().cloned().collect();
    let decisions: Vec<_> = set.decisions.values().cloned().collect();
    let objections: Vec<_> = set.objections.values().cloned().collect();
    let audits: Vec<_> = set.audits.values().cloned().collect();
    let receipts: Vec<_> = set.receipts.values().cloned().collect();
    let closures: Vec<_> = set.closures.values().cloned().collect();
    let positions: Vec<_> = set.positions.values().copied().collect();
    let forks: Vec<_> = set
        .forks
        .iter()
        .map(|(author, seq)| ForkProof {
            author: *author,
            seq: *seq,
        })
        .collect();
    let evidence = Evidence {
        genesis: &genesis,
        claims: &claims,
        decisions: &decisions,
        objections: &objections,
        audits: &audits,
        receipts: &receipts,
        closures: &closures,
        positions: &positions,
        forks: &forks,
    };

    let mut agreements = BTreeMap::new();
    for object in &genesis {
        if let Ok(hash) = object.body.agreement_hash() {
            agreements.entry(hash).or_insert(&object.body);
        }
    }
    let matches = agreements
        .into_iter()
        .map(|(hash, agreement)| {
            let reduction = reduce(agreement, &evidence);
            let handoff = handoff(&config.audit, agreement, &evidence, &reduction);
            let count = count(
                config,
                agreement,
                reduction.state,
                &handoff.basis,
                reduction.outcome.as_ref(),
            );
            MatchEntry {
                agreement: hash,
                game_id: agreement.game_id.clone(),
                players: agreement.players,
                state: reduction.state,
                basis: handoff.basis,
                outcome: reduction.outcome,
                finality_proven: reduction.finality_proven,
                void: reduction.void,
                void_audits: reduction.void_audits,
                reports: handoff.reports,
                count,
            }
        })
        .collect();
    TrackingView {
        name: config.name.clone(),
        version: config.version,
        config: config.hash(),
        frontier: set.frontier(),
        matches,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Count, EvidenceSet, NotCounted, ViewConfig, rebuild};
    use crate::attestation::v1::audit::{AuditReport, AuditScope, AuditVerdict, AuditorRole};
    use crate::attestation::v1::reducer::{EventPosition, Evidence, ForkProof, Signed};
    use crate::attestation::v1::registry::{AuditPolicy, CountingBasis};
    use crate::attestation::v1::{
        CountingPolicy, Objection, V1MatchState, VERSION, WitnessDecision, WitnessEvidence,
    };
    use crate::attestation::{GenesisAgreement, PlayerId, ResultClaim};
    use p2panda_core::Hash;

    const A: PlayerId = [1; 32];
    const B: PlayerId = [2; 32];
    const W: PlayerId = [3; 32];
    const AUDITOR: PlayerId = [7; 32];
    const RUNTIME: &str = "x86_64-linux";

    fn h(name: &str) -> Hash {
        Hash::digest(name.as_bytes())
    }

    fn hex(player: PlayerId) -> String {
        player.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn agreement(game: &str) -> GenesisAgreement {
        GenesisAgreement {
            game_id: game.to_owned(),
            logic_hash: h("logic"),
            players: [A, B],
            witness: Some(W),
            objection_window_secs: 3600,
            witness_terms: None,
        }
    }

    fn signed<T>(name: &str, author: PlayerId, seq: u64, body: T) -> Signed<T> {
        Signed {
            id: h(name),
            author,
            seq,
            after: Vec::new(),
            body,
        }
    }

    /// The signed objects of one source, owned so a test can slice them.
    #[derive(Default, Clone)]
    struct Batch {
        genesis: Vec<Signed<GenesisAgreement>>,
        claims: Vec<Signed<ResultClaim>>,
        decisions: Vec<Signed<WitnessDecision>>,
        objections: Vec<Signed<Objection>>,
        audits: Vec<Signed<AuditReport>>,
        positions: Vec<EventPosition>,
        forks: Vec<ForkProof>,
    }

    impl Batch {
        fn evidence(&self) -> Evidence<'_> {
            Evidence {
                genesis: &self.genesis,
                claims: &self.claims,
                decisions: &self.decisions,
                objections: &self.objections,
                audits: &self.audits,
                positions: &self.positions,
                forks: &self.forks,
                ..Evidence::default()
            }
        }

        fn genesis(&mut self, game: &str) {
            let agreement = agreement(game);
            self.genesis.push(signed(
                &format!("{game} genesis a"),
                A,
                0,
                agreement.clone(),
            ));
            self.genesis
                .push(signed(&format!("{game} genesis b"), B, 0, agreement));
        }

        fn claim(&mut self, game: &str, author: PlayerId, winner: PlayerId) {
            let seat = if author == A { "a" } else { "b" };
            let body = ResultClaim {
                agreement: agreement(game).agreement_hash().unwrap(),
                outcome: hex(winner),
                final_state_hash: h(&format!("{game} final {}", hex(winner))),
                frontiers: Vec::new(),
            };
            self.claims
                .push(signed(&format!("{game} claim {seat}"), author, 2, body));
        }

        fn decision(&mut self, game: &str, winner: PlayerId, partial: bool) {
            let evidence = if partial {
                WitnessEvidence::Partial {
                    received: vec![h(&format!("{game} claim a"))],
                    missing_from: B,
                    deadline_secs: 3600,
                    non_receipt_statement: h("non-receipt"),
                }
            } else {
                WitnessEvidence::Complete {
                    claim_a: h(&format!("{game} claim a")),
                    claim_b: h(&format!("{game} claim b")),
                    log_hash_a: h("log a"),
                    log_hash_b: h("log b"),
                }
            };
            let body = WitnessDecision {
                version: VERSION,
                agreement: agreement(game).agreement_hash().unwrap(),
                logic_hash: h("logic"),
                outcome: hex(winner),
                evidence,
            };
            self.decisions
                .push(signed(&format!("{game} decision"), W, 0, body));
        }

        fn object(&mut self, game: &str, author: PlayerId) {
            let body = Objection {
                version: VERSION,
                target: h(&format!("{game} decision")),
                grounds: "replay disagrees".to_owned(),
                author_frontier: Vec::new(),
            };
            self.objections
                .push(signed(&format!("{game} objection"), author, 3, body));
        }

        fn confirm(&mut self, game: &str, auditor: PlayerId) {
            let agreement = agreement(game);
            let body = AuditReport {
                version: VERSION,
                role: AuditorRole::Independent,
                match_id: agreement.game_id.clone(),
                agreement: agreement.agreement_hash().unwrap(),
                transcript: h(&format!("{game} transcript")),
                evidence: vec![h(&format!("{game} claim a"))],
                logic_hash: agreement.logic_hash,
                rules: h("rules"),
                runtime: RUNTIME.to_owned(),
                tool_version: "match-audit 1".to_owned(),
                scope: AuditScope::FullHistory,
                verdict: AuditVerdict::ResultConfirmed,
                plausibility: None,
                metadata: Vec::new(),
            };
            self.audits
                .push(signed(&format!("{game} audit"), auditor, 0, body));
            self.positions.push(EventPosition {
                id: h(&format!("{game} transcript")),
                author: A,
                seq: 1,
            });
        }
    }

    /// Three matches: chess conceded to A, go witnessed for B, and shogi
    /// disputed but confirmed for A by a full-history audit.
    fn league() -> Batch {
        let mut batch = Batch::default();
        batch.genesis("chess");
        batch.claim("chess", A, A);
        batch.claim("chess", B, A);
        batch.genesis("go");
        batch.decision("go", B, false);
        batch.genesis("shogi");
        batch.claim("shogi", A, A);
        batch.claim("shogi", B, B);
        batch.confirm("shogi", AUDITOR);
        batch
    }

    fn config() -> ViewConfig {
        ViewConfig {
            name: "ladder".to_owned(),
            version: 1,
            players: Vec::new(),
            counting: CountingPolicy {
                accepted_states: vec![
                    V1MatchState::Agreed,
                    V1MatchState::Witnessed,
                    V1MatchState::Final,
                ],
                accepted_witnesses: vec![W],
                accepted_logic_hashes: Vec::new(),
            },
            count_witness_partial: false,
            audit: AuditPolicy {
                accept_audit_resolved: true,
                accepted_auditors: vec![AUDITOR],
                accepted_runtimes: vec![RUNTIME.to_owned()],
            },
        }
    }

    fn view_of(batches: &[&Batch], config: &ViewConfig) -> super::TrackingView {
        let mut set = EvidenceSet::new();
        for batch in batches {
            set.ingest(&batch.evidence());
        }
        rebuild(config, &set)
    }

    fn count_of(view: &super::TrackingView, game: &str) -> Count {
        view.matches
            .iter()
            .find(|entry| entry.game_id == game)
            .unwrap()
            .count
            .clone()
    }

    fn counted(outcome: PlayerId) -> Count {
        Count::Counted {
            outcome: hex(outcome),
        }
    }

    #[test]
    fn a_view_counts_each_basis_its_config_accepts() {
        let view = view_of(&[&league()], &config());
        assert_eq!(view.matches.len(), 3);
        assert_eq!(count_of(&view, "chess"), counted(A));
        assert_eq!(count_of(&view, "go"), counted(B));
        assert_eq!(count_of(&view, "shogi"), counted(A));
        let shogi = view
            .matches
            .iter()
            .find(|entry| entry.game_id == "shogi")
            .unwrap();
        // Counting never rewrites the state: the dispute stays visible.
        assert_eq!(shogi.state, V1MatchState::Disputed);
        assert!(matches!(shogi.basis, CountingBasis::AuditResolved { .. }));
        assert_eq!(view.counted().count(), 3);
    }

    #[test]
    fn two_replicas_build_the_same_view_from_the_same_evidence() {
        let league = league();
        let first = view_of(&[&league], &config());
        // A second replica gets every object reversed, in two batches, with
        // one object twice.
        let mut reversed = league.clone();
        reversed.genesis.reverse();
        reversed.claims.reverse();
        let mut split = Batch::default();
        split.claims = reversed.claims.split_off(2);
        split.genesis.push(reversed.genesis[0].clone());
        let second = view_of(&[&split, &reversed], &config());
        assert_eq!(first, second);
        assert_eq!(first.frontier, second.frontier);
    }

    #[test]
    fn evidence_from_several_sources_rebuilds_the_same_view() {
        // Player A's device, player B's device and the witness each sync
        // part of the record; together they hold what one gatherer holds.
        let league = league();
        let mut from_a = Batch::default();
        let mut from_b = Batch::default();
        let mut from_w = Batch::default();
        for object in &league.genesis {
            if object.author == A {
                from_a.genesis.push(object.clone());
            } else {
                from_b.genesis.push(object.clone());
            }
        }
        for object in &league.claims {
            if object.author == A {
                from_a.claims.push(object.clone());
            } else {
                from_b.claims.push(object.clone());
            }
            // Each player also relays the other's claim it saw.
            from_a.claims.push(object.clone());
        }
        from_w.decisions = league.decisions.clone();
        from_w.audits = league.audits.clone();
        from_w.positions = league.positions.clone();

        let gathered = view_of(&[&league], &config());
        let synced = view_of(&[&from_w, &from_b, &from_a], &config());
        assert_eq!(gathered, synced);

        // A partial sync holds less, and its frontier says so.
        let partial = view_of(&[&from_a, &from_w], &config());
        assert_ne!(partial.frontier, gathered.frontier);
        assert!(partial.frontier.objects < gathered.frontier.objects);
    }

    #[test]
    fn differing_policies_are_visible_and_leave_the_evidence_alone() {
        let league = league();
        let mut set = EvidenceSet::new();
        set.ingest(&league.evidence());
        let before = set.frontier();

        let ladder = rebuild(&config(), &set);
        let mut strict = config();
        strict.name = "strict".to_owned();
        strict.audit.accept_audit_resolved = false;
        strict.counting.accepted_states = vec![V1MatchState::Agreed];
        let strict_view = rebuild(&strict, &set);

        assert_ne!(ladder.config, strict_view.config);
        assert_eq!(ladder.frontier, strict_view.frontier);
        assert_eq!(set.frontier(), before);
        assert_eq!(count_of(&strict_view, "chess"), counted(A));
        assert_eq!(
            count_of(&strict_view, "go"),
            Count::NotCounted(NotCounted::State)
        );
        assert_eq!(
            count_of(&strict_view, "shogi"),
            Count::NotCounted(NotCounted::NoResult)
        );
        // The states themselves are the same under both policies.
        for (one, other) in ladder.matches.iter().zip(&strict_view.matches) {
            assert_eq!(one.state, other.state);
            assert_eq!(one.outcome, other.outcome);
        }
    }

    #[test]
    fn the_config_hash_reads_lists_as_sets() {
        let mut one = config();
        one.counting.accepted_states.reverse();
        one.audit.accepted_auditors.push(AUDITOR);
        assert_eq!(one.hash(), config().hash());
        let mut other = config();
        other.version = 2;
        assert_ne!(other.hash(), config().hash());
        let mut partial = config();
        partial.count_witness_partial = true;
        assert_ne!(partial.hash(), config().hash());
    }

    #[test]
    fn fork_evidence_voids_the_dependent_counts() {
        let league = league();
        let before = view_of(&[&league], &config());
        assert_eq!(before.counted().count(), 3);

        // Player A equivocated at the sequence of its claims: chess and
        // shogi rest on A's claims, so neither counts any more.
        let mut fork = Batch::default();
        fork.forks.push(ForkProof { author: A, seq: 2 });
        let after = view_of(&[&league, &fork], &config());
        assert_eq!(
            count_of(&after, "chess"),
            Count::NotCounted(NotCounted::NoResult)
        );
        assert_eq!(
            count_of(&after, "shogi"),
            Count::NotCounted(NotCounted::NoResult)
        );
        assert_eq!(count_of(&after, "go"), counted(B));
        let chess = after
            .matches
            .iter()
            .find(|entry| entry.game_id == "chess")
            .unwrap();
        assert_eq!(chess.state, V1MatchState::Void);
        assert!(chess.void.contains(&h("chess claim a")));

        // A fork in the auditor's own chain voids the audit report, so the
        // dispute is no longer audit-resolved, while A's claims stand.
        let mut auditor_fork = Batch::default();
        auditor_fork.forks.push(ForkProof {
            author: AUDITOR,
            seq: 0,
        });
        let after = view_of(&[&league, &auditor_fork], &config());
        let shogi = after
            .matches
            .iter()
            .find(|entry| entry.game_id == "shogi")
            .unwrap();
        assert!(shogi.void_audits.contains(&h("shogi audit")));
        assert_eq!(shogi.basis, CountingBasis::None);
        assert_eq!(shogi.count, Count::NotCounted(NotCounted::NoResult));
        assert_eq!(count_of(&after, "chess"), counted(A));

        // A later, higher fork proof does not undo the earlier one.
        let mut later = Batch::default();
        later.forks.push(ForkProof { author: A, seq: 9 });
        let both = view_of(&[&league, &fork, &later], &config());
        let either = view_of(&[&later, &league, &fork], &config());
        assert_eq!(both, either);
        assert_eq!(
            count_of(&both, "chess"),
            Count::NotCounted(NotCounted::NoResult)
        );
    }

    #[test]
    fn a_view_names_why_it_does_not_count() {
        let mut batch = Batch::default();
        batch.genesis("go");
        batch.decision("go", B, true);
        batch.genesis("chess");
        batch.claim("chess", A, A);
        batch.claim("chess", B, A);
        batch.genesis("draughts");
        batch.decision("draughts", A, false);
        batch.object("draughts", B);

        let view = view_of(&[&batch], &config());
        assert_eq!(
            count_of(&view, "go"),
            Count::NotCounted(NotCounted::PartialWitness)
        );
        assert_eq!(
            count_of(&view, "draughts"),
            Count::NotCounted(NotCounted::NoResult)
        );
        let mut partial = config();
        partial.count_witness_partial = true;
        assert_eq!(count_of(&view_of(&[&batch], &partial), "go"), counted(B));

        let mut other_witness = config();
        other_witness.counting.accepted_witnesses = vec![[9; 32]];
        assert_eq!(
            count_of(&view_of(&[&batch], &other_witness), "chess"),
            Count::NotCounted(NotCounted::Agreement)
        );

        let mut only_a = config();
        only_a.players = vec![A];
        assert_eq!(
            count_of(&view_of(&[&batch], &only_a), "chess"),
            Count::NotCounted(NotCounted::UntrackedPlayer)
        );
    }
}
