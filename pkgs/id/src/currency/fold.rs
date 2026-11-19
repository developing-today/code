//! Deterministic graph-native currency fold (#5522 phase 2, design #2043).
//!
//! [`graph`] defines the signed currency contracts; this module folds a set
//! of those events into balances. The fold is a pure function of the event
//! set: events are applied in causal order (backlink plus `after` frontier),
//! with a deterministic tie-break, so every node that has seen the same
//! events derives the same state — including the same verdicts.
//!
//! What the fold guarantees, per design #2043 invariants 1–3:
//!
//! - **Identity.** A currency is its semantic [`CurrencyId`] (issuer plus
//!   canonical definition hash); balances key on its compact hash, never on
//!   names, registry IDs, or ledger IDs.
//! - **Cap.** Counted supply never exceeds the definition's finite cap.
//!   Issuance past the cap is void at its causal position. The design's
//!   supply-account exception (Decisions Q2) has no representation in the
//!   signed v1 bodies — no field marks an issuance as supply-account — so
//!   the fold counts every issuance; giving the exception a contract marker
//!   is follow-up design work, recorded on #5522, not silently invented here.
//! - **Causal validity.** A transfer or burn that overspends the signer's
//!   balance *at its causal position* is void ([`InvalidReason::InsufficientFunds`]).
//!   The fold does not claim double-spends are prevented: on a fork, two
//!   spends can both be signed, and the fold deterministically voids the one
//!   that is causally later. Balances are reproducible folds, exactly as the
//!   design states.
//! - **Pending.** An event whose causal ancestry (backlink or `after`) is
//!   absent from the input, or whose body was not supplied, is
//!   [`FoldOutcome::Pending`] and moves no value. Dependents of a pending
//!   event are pending too: they are never evaluated against a state that
//!   lacks their ancestor, so a spend funded by a body-less transfer waits
//!   rather than being voided. Folding again once the body arrives decides
//!   the whole pending subtree.
//! - **Body binding.** Bodies travel separately from the signed headers, so
//!   a supplied body is used only if its BLAKE3 hash equals the header's
//!   signed `payload_hash`. A body that does not match is not this event's
//!   body: it is ignored and the event is [`FoldOutcome::Pending`], exactly
//!   as if no body had been supplied. It is not voided, because a forged
//!   body must not be able to void an honestly signed event; the genuine
//!   body can still arrive and decide it. A currency event whose header
//!   commits to no body at all can never be decided and is
//!   [`InvalidReason::InvalidBody`].
//!
//! - **Issuer rotation** (design #2804 §3.3, §3.6). Issuance authority
//!   follows the issuer's generation chain, folded from the
//!   `issuer.rotation` events in the input with
//!   [`fold_rotation_chain`]. Each generation key may issue only inside its
//!   own interval: causally after the rotation that installed it
//!   ([`InvalidReason::UnlinkedSuccessor`] otherwise) and not causally after
//!   the rotation that retired it ([`InvalidReason::RetiredGeneration`]).
//!   Issuance whose position against a rotation the graph cannot settle,
//!   because the two are causally concurrent or the chain forked, is
//!   [`FoldOutcome::Disputed`]: kept as evidence, counted in
//!   [`CurrencyFold::disputed`], and never in balances or supply. A
//!   currency with no rotation event folds exactly as before. Currency
//!   definitions commit no recovery key yet (#2251 owns that field), so the
//!   chain is folded with no recovery commitment and a compromise recovery
//!   is never authoritative here (§3.4).
//!
//! What the fold deliberately does not do:
//!
//! - **Anchored currencies.** A definition bound to a ledger
//!   ([`CurrencyBinding::Ledger`]) registers, but its value events are
//!   refused ([`InvalidReason::AnchoredBinding`]): anchored balance reads
//!   come from the ledger, not from graph intent (invariant 3).
//! - **Conversion, receipts, grants.** Later design phases with their own
//!   contracts; not folded here.
//! - **Signature verification.** Events arrive as [`Event`] values whose
//!   headers the chain layer has already authenticated; the fold trusts the
//!   header's signer and kind and validates everything else itself.

use std::collections::{BTreeMap, BTreeSet};

use p2panda_core::Hash;

use crate::chain::Event;
use crate::currency::IssuerId;
use crate::currency::graph::{
    BURN_KIND, CurrencyBinding, CurrencyDefinition, CurrencyId, DEFINE_KIND, ISSUE_KIND,
    TRANSFER_KIND, TokenBurn, TokenIssue, TokenTransfer,
};
use crate::issuer_rotation::{
    ChainStatus, GenerationChain, IssuerRotation, author_id, fold_rotation_chain,
};

/// Why a currency event was voided by the fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidReason {
    /// Body bytes failed the contract's strict decode/validation, or the
    /// signed header commits to no body.
    InvalidBody,
    /// A second definition for an already-registered semantic currency.
    DuplicateDefinition,
    /// The named currency has no definition in the folded set.
    UnknownCurrency,
    /// Issuance signed by someone other than the currency's issuer.
    NotIssuer,
    /// Issuance would push counted supply past the definition's cap.
    OverCap,
    /// Transfer or burn exceeds the signer's balance at its causal position.
    InsufficientFunds,
    /// Value event for a ledger-bound currency; balances are the ledger's.
    AnchoredBinding,
    /// Balance or supply arithmetic overflowed `u128`.
    Overflow,
    /// Issuance signed by a generation key causally after the rotation
    /// that retired that generation (design #2804 §3.3).
    RetiredGeneration,
    /// Issuance signed by a rotated-in key that does not causally follow
    /// the rotation installing it, so that rotation does not authorise it
    /// (design #2804 §3.2 rule 4).
    UnlinkedSuccessor,
}

/// Why an issuance is disputed rather than settled (design #2804 §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisputeReason {
    /// Signed by a generation key that a rotation retired, but neither
    /// before nor after that rotation in the causal graph: whether it fell
    /// inside the generation's interval cannot be decided.
    ConcurrentWithRotation,
    /// The issuer's generation chain forked (two rotations claim the same
    /// predecessor and neither settles), and this issuance is not causally
    /// before the fork.
    ForkedChain,
}

/// The fold's verdict for one input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldOutcome {
    /// The event applied; value or a definition moved as signed.
    Applied,
    /// The event is void: present, causally placed, and refused.
    Invalid(InvalidReason),
    /// Ancestry or body is missing (a body whose hash does not match the
    /// header's signed `payload_hash` counts as missing), or an ancestor is
    /// itself pending; the event is not yet decidable.
    Pending,
    /// Not a currency event; it only participates in causality.
    Ignored,
    /// Issuance whose validity the observed graph cannot settle. It moves
    /// no value; its amount is reported by [`CurrencyFold::disputed`].
    Disputed(DisputeReason),
}

/// The folded state: definitions, balances, counted supply, and verdicts.
#[derive(Clone, Debug, Default)]
pub struct CurrencyFold {
    definitions: BTreeMap<Hash, (CurrencyId, CurrencyDefinition)>,
    balances: BTreeMap<(Hash, IssuerId), u128>,
    supply: BTreeMap<Hash, u128>,
    issued: BTreeMap<Hash, u128>,
    burned: BTreeMap<Hash, u128>,
    frontier: BTreeMap<IssuerId, (u64, Hash)>,
    outcomes: BTreeMap<Hash, FoldOutcome>,
    disputed: BTreeMap<Hash, u128>,
    generations: BTreeMap<Hash, GenerationChain>,
}

impl CurrencyFold {
    /// The registered definition for a currency's compact hash, if any.
    #[must_use]
    pub fn definition(&self, currency: &Hash) -> Option<&(CurrencyId, CurrencyDefinition)> {
        self.definitions.get(currency)
    }

    /// An account's folded balance in a currency (0 when never funded).
    #[must_use]
    pub fn balance(&self, currency: &Hash, account: &IssuerId) -> u128 {
        self.balances
            .get(&(*currency, *account))
            .copied()
            .unwrap_or(0)
    }

    /// Counted (issued minus burned) supply of a currency.
    #[must_use]
    pub fn supply(&self, currency: &Hash) -> u128 {
        self.supply.get(currency).copied().unwrap_or(0)
    }

    /// Total applied issuance of a currency (never reduced by burns).
    #[must_use]
    pub fn issued(&self, currency: &Hash) -> u128 {
        self.issued.get(currency).copied().unwrap_or(0)
    }

    /// Total applied burns of a currency.
    #[must_use]
    pub fn burned(&self, currency: &Hash) -> u128 {
        self.burned.get(currency).copied().unwrap_or(0)
    }

    /// Every holder of a currency with a non-zero balance, by key.
    #[must_use]
    pub fn holders(&self, currency: &Hash) -> Vec<(IssuerId, u128)> {
        self.balances
            .range((*currency, [0; 32])..=(*currency, [0xff; 32]))
            .filter(|(_, balance)| **balance > 0)
            .map(|((_, holder), balance)| (*holder, *balance))
            .collect()
    }

    /// The decided frontier: for each author, the highest sequence number
    /// and event id the fold reached a final verdict on. Pending events are
    /// not covered. This is the "as of" a snapshot names.
    #[must_use]
    pub const fn frontier(&self) -> &BTreeMap<IssuerId, (u64, Hash)> {
        &self.frontier
    }

    /// Total disputed issuance of a currency: issuance the observed graph
    /// cannot place inside its generation's interval. Never in supply.
    #[must_use]
    pub fn disputed(&self, currency: &Hash) -> u128 {
        self.disputed.get(currency).copied().unwrap_or(0)
    }

    /// The issuer's generation chain for a currency, when the input held
    /// at least one rotation event for it. `None` means the generation-0
    /// key is the only issuance key.
    #[must_use]
    pub fn generation_chain(&self, currency: &Hash) -> Option<&GenerationChain> {
        self.generations.get(currency)
    }

    /// The fold's verdict for an event id, if the event was in the input.
    #[must_use]
    pub fn outcome(&self, event: &Hash) -> Option<FoldOutcome> {
        self.outcomes.get(event).copied()
    }

    fn credit(&mut self, currency: &Hash, account: &IssuerId, amount: u128) -> Result<(), ()> {
        let entry = self.balances.entry((*currency, *account)).or_insert(0);
        *entry = entry.checked_add(amount).ok_or(())?;
        Ok(())
    }

    fn debit(&mut self, currency: &Hash, account: &IssuerId, amount: u128) -> Result<(), ()> {
        let entry = self.balances.entry((*currency, *account)).or_insert(0);
        *entry = entry.checked_sub(amount).ok_or(())?;
        Ok(())
    }
}

/// Fold `events` into currency state.
///
/// `bodies` maps event IDs to their body bytes (event headers commit to body
/// hashes; bodies travel separately). A body is used only when its hash
/// equals the header's signed `payload_hash`; currency events without a
/// matching body are [`FoldOutcome::Pending`], and so is every event that
/// causally depends on a pending event. Duplicate event IDs are folded once.
/// The result depends only on the event set and the matching bodies, never
/// on the order of `events`.
#[must_use]
pub fn fold_events(events: &[Event], bodies: &BTreeMap<Hash, Vec<u8>>) -> CurrencyFold {
    let mut by_id: BTreeMap<Hash, &Event> = BTreeMap::new();
    for event in events {
        by_id.entry(event.id).or_insert(event);
    }

    let graph = Graph::new(&by_id);
    let mut fold = CurrencyFold::default();
    // Events with a final verdict (Applied, Invalid, Ignored). Pending events
    // are never decided, so nothing that depends on them is evaluated.
    let mut decided: BTreeSet<Hash> = BTreeSet::new();
    let mut pending: BTreeSet<Hash> = BTreeSet::new();

    // Fixpoint over causal readiness: an event is decidable once every
    // event it causally depends on (backlink + `after` frontier) is decided.
    loop {
        let ready: Vec<Hash> = by_id
            .iter()
            .filter(|(id, event)| {
                !decided.contains(id)
                    && !pending.contains(id)
                    && dependencies(event).iter().all(|dep| decided.contains(dep))
            })
            .map(|(id, _)| *id)
            .collect();
        if ready.is_empty() {
            break;
        }
        // Deterministic tie-break among causally-ready events: signer, then
        // sequence, then event id. Two folds of the same set agree exactly.
        let mut ordered: Vec<&Event> = ready.iter().map(|id| by_id[id]).collect();
        ordered.sort_by(|a, b| {
            a.header
                .verifying_key
                .as_bytes()
                .cmp(b.header.verifying_key.as_bytes())
                .then_with(|| a.header.seq_num.cmp(&b.header.seq_num))
                .then_with(|| a.id.as_bytes().cmp(b.id.as_bytes()))
        });
        for event in ordered {
            let outcome = apply(&mut fold, &graph, event, bodies.get(&event.id));
            fold.outcomes.insert(event.id, outcome);
            if outcome == FoldOutcome::Pending {
                pending.insert(event.id);
            } else {
                decided.insert(event.id);
                let seq = u64::from(event.header.seq_num);
                let head = fold
                    .frontier
                    .entry(signer(event))
                    .or_insert((seq, event.id));
                // On a fork (two events at one sequence number) the larger
                // id is the head, so the frontier does not depend on order.
                if (seq, event.id.as_bytes()) > (head.0, head.1.as_bytes()) {
                    *head = (seq, event.id);
                }
            }
        }
    }

    // Anything undecided is waiting on ancestry outside the input set, or on
    // a pending ancestor (missing or mismatched body).
    for id in by_id.keys() {
        fold.outcomes.entry(*id).or_insert(FoldOutcome::Pending);
    }
    for (compact, (id, _)) in &fold.definitions {
        if let Some(chain) = graph.chains.get(&(id.issuer(), *compact)) {
            fold.generations.insert(*compact, chain.clone());
        }
    }
    fold
}

/// One rotation event as [`fold_rotation_chain`] takes it: verified author,
/// event id, and the rotation its header carries.
type Rotated = (IssuerId, Hash, IssuerRotation);

/// The causal graph of the input and the issuer generation chains folded
/// from its rotation events. Both are facts of the input set alone, so
/// classifying an issuance against them never depends on fold order.
struct Graph<'a> {
    by_id: &'a BTreeMap<Hash, &'a Event>,
    /// Generation chains keyed by (generation-0 issuer, currency compact
    /// hash), only for currencies that have a rotation event.
    chains: BTreeMap<(IssuerId, Hash), GenerationChain>,
}

impl<'a> Graph<'a> {
    fn new(by_id: &'a BTreeMap<Hash, &'a Event>) -> Self {
        let mut rotations: BTreeMap<(IssuerId, Hash), Vec<Rotated>> = BTreeMap::new();
        // The chain layer admits a rotation extension only on the
        // `issuer.rotation` kind, so the extension alone marks the event.
        for event in by_id.values() {
            if let Some(rotation) = &event.header.extensions.rotation {
                rotations
                    .entry((rotation.issuer_id, rotation.currency_id))
                    .or_default()
                    .push((author_id(event), event.id, rotation.clone()));
            }
        }
        // No currency definition commits a recovery key yet (#2251), so no
        // recovery transition is authoritative (design #2804 §3.4).
        let chains = rotations
            .into_iter()
            .map(|((issuer, currency), list)| {
                let chain = fold_rotation_chain(&issuer, &currency, None, &list);
                ((issuer, currency), chain)
            })
            .collect();
        Self { by_id, chains }
    }

    /// Whether `ancestor` is a strict causal ancestor of `event` through
    /// backlinks and `after` links among the input events.
    fn precedes(&self, ancestor: &Hash, event: &Hash) -> bool {
        if ancestor == event {
            return false;
        }
        let mut seen: BTreeSet<Hash> = BTreeSet::new();
        let mut stack: Vec<Hash> = vec![*event];
        while let Some(id) = stack.pop() {
            let Some(current) = self.by_id.get(&id) else {
                continue;
            };
            for dep in dependencies(current) {
                if dep == *ancestor {
                    return true;
                }
                if seen.insert(dep) {
                    stack.push(dep);
                }
            }
        }
        false
    }

    /// Whether `event`, signed by `signer`, may issue under the issuer's
    /// generation chain for this currency. `Ok` when the currency has no
    /// rotation and `signer` is the generation-0 key, as before rotation
    /// existed.
    fn issuance_authority(
        &self,
        issuer: &IssuerId,
        currency: &Hash,
        signer: &IssuerId,
        event: &Hash,
    ) -> Result<(), FoldOutcome> {
        let Some(chain) = self.chains.get(&(*issuer, *currency)) else {
            return if signer == issuer {
                Ok(())
            } else {
                Err(FoldOutcome::Invalid(InvalidReason::NotIssuer))
            };
        };
        // A key may hold more than one generation (A to B and back to A);
        // the issuance stands if any of its intervals holds it, and the
        // most favourable verdict wins otherwise.
        let mut verdict = FoldOutcome::Invalid(InvalidReason::NotIssuer);
        for (generation, key) in chain.keys.iter().enumerate() {
            if key != signer {
                continue;
            }
            match self.generation_interval(chain, generation, event) {
                Ok(()) => return Ok(()),
                Err(found) => verdict = stronger(verdict, found),
            }
        }
        // A key named only by one of the competing rotations of a forked
        // chain holds no generation, but its activity after that rotation
        // is a disputed branch, not a stranger's issuance (§3.2 rule 5).
        if let ChainStatus::Conflicted { candidates, .. } = &chain.status
            && candidates.iter().any(|candidate| {
                self.successor_of(candidate) == Some(*signer) && self.precedes(candidate, event)
            })
        {
            verdict = stronger(verdict, FoldOutcome::Disputed(DisputeReason::ForkedChain));
        }
        Err(verdict)
    }

    /// The successor key a rotation event names.
    fn successor_of(&self, rotation_event: &Hash) -> Option<IssuerId> {
        self.by_id
            .get(rotation_event)
            .and_then(|event| event.header.extensions.rotation.as_ref())
            .map(|rotation| rotation.successor_key)
    }

    /// Whether `event` falls inside `generation`'s interval (§3.3).
    fn generation_interval(
        &self,
        chain: &GenerationChain,
        generation: usize,
        event: &Hash,
    ) -> Result<(), FoldOutcome> {
        if let Some(start) = generation
            .checked_sub(1)
            .and_then(|g| chain.transitions.get(g))
            && !self.precedes(start, event)
        {
            return Err(FoldOutcome::Invalid(InvalidReason::UnlinkedSuccessor));
        }
        if let Some(end) = chain.transitions.get(generation) {
            if self.precedes(end, event) {
                return Err(FoldOutcome::Invalid(InvalidReason::RetiredGeneration));
            }
            if self.precedes(event, end) {
                return Ok(());
            }
            return Err(FoldOutcome::Disputed(DisputeReason::ConcurrentWithRotation));
        }
        // The active generation. When the chain forked out of it, only
        // issuance causally before every competing rotation is settled.
        if let ChainStatus::Conflicted { candidates, .. } = &chain.status
            && !candidates
                .iter()
                .all(|candidate| self.precedes(event, candidate))
        {
            return Err(FoldOutcome::Disputed(DisputeReason::ForkedChain));
        }
        Ok(())
    }
}

/// Of two refusals for one issuance, the one closest to standing: a
/// dispute over a voiding, a specific voiding over [`InvalidReason::NotIssuer`],
/// and otherwise the first found.
const fn stronger(current: FoldOutcome, found: FoldOutcome) -> FoldOutcome {
    match (current, found) {
        (_, FoldOutcome::Disputed(_)) | (FoldOutcome::Invalid(InvalidReason::NotIssuer), _) => {
            found
        }
        _ => current,
    }
}

/// The event IDs an event causally depends on: its author's previous event
/// (backlink) and its asserted `after` frontier.
fn dependencies(event: &Event) -> Vec<Hash> {
    let mut deps: Vec<Hash> = Vec::new();
    if let Some(backlink) = event.header.backlink {
        deps.push(backlink);
    }
    deps.extend(event.header.extensions.after.iter().copied());
    deps
}

fn signer(event: &Event) -> IssuerId {
    *event.header.verifying_key.as_bytes()
}

fn apply(
    fold: &mut CurrencyFold,
    graph: &Graph<'_>,
    event: &Event,
    body: Option<&Vec<u8>>,
) -> FoldOutcome {
    let kind = event.header.extensions.kind.as_str();
    if !matches!(kind, DEFINE_KIND | ISSUE_KIND | TRANSFER_KIND | BURN_KIND) {
        return FoldOutcome::Ignored;
    }
    let body = match verified_body(event, body) {
        Ok(body) => body,
        Err(outcome) => return outcome,
    };
    match kind {
        DEFINE_KIND => apply_define(fold, event, body),
        ISSUE_KIND => apply_issue(fold, graph, event, body),
        TRANSFER_KIND => apply_transfer(fold, event, body),
        BURN_KIND => apply_burn(fold, event, body),
        _ => FoldOutcome::Ignored,
    }
}

/// The supplied body, but only if it is the one the signed header commits
/// to. A header that commits to no body can never be satisfied, so the event
/// is void; a missing body, or one whose hash differs from the signed
/// `payload_hash` (forged or corrupted in transit), leaves the event pending
/// until the genuine body is supplied.
fn verified_body<'a>(event: &Event, body: Option<&'a Vec<u8>>) -> Result<&'a [u8], FoldOutcome> {
    let Some(committed) = event.header.payload_hash else {
        return Err(FoldOutcome::Invalid(InvalidReason::InvalidBody));
    };
    match body {
        Some(body) if Hash::digest(body) == committed => Ok(body),
        _ => Err(FoldOutcome::Pending),
    }
}

fn apply_define(fold: &mut CurrencyFold, event: &Event, body: &[u8]) -> FoldOutcome {
    let Ok(definition) = CurrencyDefinition::from_body(body) else {
        return FoldOutcome::Invalid(InvalidReason::InvalidBody);
    };
    let Ok(id) = CurrencyId::from_definition(signer(event), &definition) else {
        return FoldOutcome::Invalid(InvalidReason::InvalidBody);
    };
    let compact = id.compact_hash();
    if fold.definitions.contains_key(&compact) {
        return FoldOutcome::Invalid(InvalidReason::DuplicateDefinition);
    }
    fold.definitions.insert(compact, (id, definition));
    fold.supply.insert(compact, 0);
    fold.issued.insert(compact, 0);
    fold.burned.insert(compact, 0);
    FoldOutcome::Applied
}

/// The registered definition for a value event, with the anchored-binding
/// refusal applied uniformly to issue/transfer/burn.
fn value_target(
    fold: &CurrencyFold,
    currency: &Hash,
) -> Result<(CurrencyId, CurrencyDefinition), FoldOutcome> {
    let Some((id, definition)) = fold.definitions.get(currency) else {
        return Err(FoldOutcome::Invalid(InvalidReason::UnknownCurrency));
    };
    if matches!(definition.binding, CurrencyBinding::Ledger { .. }) {
        return Err(FoldOutcome::Invalid(InvalidReason::AnchoredBinding));
    }
    Ok((*id, definition.clone()))
}

fn apply_issue(
    fold: &mut CurrencyFold,
    graph: &Graph<'_>,
    event: &Event,
    body: &[u8],
) -> FoldOutcome {
    let Ok(issue) = TokenIssue::from_body(body) else {
        return FoldOutcome::Invalid(InvalidReason::InvalidBody);
    };
    let (id, definition) = match value_target(fold, &issue.currency) {
        Ok(pair) => pair,
        Err(outcome) => return outcome,
    };
    match graph.issuance_authority(&id.issuer(), &issue.currency, &signer(event), &event.id) {
        Ok(()) => {}
        Err(FoldOutcome::Disputed(reason)) => {
            let disputed = fold.disputed(&issue.currency).saturating_add(issue.amount);
            fold.disputed.insert(issue.currency, disputed);
            return FoldOutcome::Disputed(reason);
        }
        Err(outcome) => return outcome,
    }
    let supply = fold.supply(&issue.currency);
    match supply.checked_add(issue.amount) {
        Some(next) if next <= definition.supply.capped => {
            let Some(issued) = fold.issued(&issue.currency).checked_add(issue.amount) else {
                return FoldOutcome::Invalid(InvalidReason::Overflow);
            };
            if fold
                .credit(&issue.currency, &issue.to, issue.amount)
                .is_err()
            {
                return FoldOutcome::Invalid(InvalidReason::Overflow);
            }
            fold.supply.insert(issue.currency, next);
            fold.issued.insert(issue.currency, issued);
            FoldOutcome::Applied
        }
        _ => FoldOutcome::Invalid(InvalidReason::OverCap),
    }
}

fn apply_transfer(fold: &mut CurrencyFold, event: &Event, body: &[u8]) -> FoldOutcome {
    let Ok(transfer) = TokenTransfer::from_body(body) else {
        return FoldOutcome::Invalid(InvalidReason::InvalidBody);
    };
    if let Err(outcome) = value_target(fold, &transfer.currency) {
        return outcome;
    }
    let from = signer(event);
    if fold.balance(&transfer.currency, &from) < transfer.amount {
        return FoldOutcome::Invalid(InvalidReason::InsufficientFunds);
    }
    if fold
        .debit(&transfer.currency, &from, transfer.amount)
        .is_err()
    {
        return FoldOutcome::Invalid(InvalidReason::InsufficientFunds);
    }
    if fold
        .credit(&transfer.currency, &transfer.to, transfer.amount)
        .is_err()
    {
        return FoldOutcome::Invalid(InvalidReason::Overflow);
    }
    FoldOutcome::Applied
}

fn apply_burn(fold: &mut CurrencyFold, event: &Event, body: &[u8]) -> FoldOutcome {
    let Ok(burn) = TokenBurn::from_body(body) else {
        return FoldOutcome::Invalid(InvalidReason::InvalidBody);
    };
    if let Err(outcome) = value_target(fold, &burn.currency) {
        return outcome;
    }
    let from = signer(event);
    if fold.balance(&burn.currency, &from) < burn.amount {
        return FoldOutcome::Invalid(InvalidReason::InsufficientFunds);
    }
    if fold.debit(&burn.currency, &from, burn.amount).is_err() {
        return FoldOutcome::Invalid(InvalidReason::InsufficientFunds);
    }
    let supply = fold.supply(&burn.currency);
    fold.supply
        .insert(burn.currency, supply.saturating_sub(burn.amount));
    // Burns never exceed what was issued, so this cannot saturate.
    let burned = fold.burned(&burn.currency).saturating_add(burn.amount);
    fold.burned.insert(burn.currency, burned);
    FoldOutcome::Applied
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::currency::graph::{
        CurrencyDefinition, SupplyCap, sign_burn, sign_definition, sign_issue, sign_transfer,
    };
    use p2panda_core::SigningKey;

    fn issuer_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn alice_key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    fn bob_key() -> SigningKey {
        SigningKey::from_bytes(&[11; 32])
    }

    fn definition(cap: u128) -> CurrencyDefinition {
        CurrencyDefinition {
            name: "Tokens".to_owned(),
            symbol: "TST".to_owned(),
            scale: 2,
            supply: SupplyCap { capped: cap },
            binding: CurrencyBinding::Graph,
            memo: None,
        }
    }

    /// A small causal world: define -> issue 100 to Alice.
    struct World {
        currency: CurrencyId,
        bodies: BTreeMap<Hash, Vec<u8>>,
        define: Event,
        issue: Event,
    }

    fn world(cap: u128, amount: u128) -> World {
        let issuer = issuer_key();
        let def = definition(cap);
        let (currency, define) = sign_definition(&issuer, 0, None, &def, vec![]).unwrap();
        let issue_body = TokenIssue {
            currency: currency.compact_hash(),
            amount,
            to: crate::currency::issuer_id(&alice_key()),
            memo: None,
        };
        let issue = sign_issue(
            &issuer,
            1,
            Some(define.id),
            &currency,
            &issue_body,
            vec![define.id],
        )
        .unwrap();
        let mut bodies = BTreeMap::new();
        bodies.insert(define.id, def.body().unwrap());
        bodies.insert(issue.id, issue_body.body().unwrap());
        World {
            currency,
            bodies,
            define,
            issue,
        }
    }

    fn transfer(
        world: &World,
        key: &SigningKey,
        seq: u32,
        backlink: Option<Hash>,
        after: Vec<Hash>,
        to: &SigningKey,
        amount: u128,
    ) -> (Event, TokenTransfer) {
        let body = TokenTransfer {
            currency: world.currency.compact_hash(),
            amount,
            to: crate::currency::issuer_id(to),
            seq_ref: u64::from(seq),
            memo: None,
        };
        let event = sign_transfer(key, seq, backlink, &body, after).unwrap();
        (event, body)
    }

    #[test]
    fn causal_funding_chain_applies_in_any_input_order() {
        let world = world(1_000, 100);
        let alice = alice_key();
        let (t1, t1_body) = transfer(
            &world,
            &alice,
            0,
            None,
            vec![world.issue.id],
            &bob_key(),
            40,
        );
        let mut bodies = world.bodies.clone();
        bodies.insert(t1.id, t1_body.body().unwrap());
        // Reversed input order must not matter.
        let fold = fold_events(
            &[t1.clone(), world.issue.clone(), world.define.clone()],
            &bodies,
        );
        let compact = world.currency.compact_hash();
        assert_eq!(fold.outcome(&t1.id), Some(FoldOutcome::Applied));
        assert_eq!(
            fold.balance(&compact, &crate::currency::issuer_id(&alice_key())),
            60
        );
        assert_eq!(
            fold.balance(&compact, &crate::currency::issuer_id(&bob_key())),
            40
        );
        assert_eq!(fold.supply(&compact), 100);
    }

    #[test]
    fn over_cap_issuance_is_void_and_supply_stays_at_cap() {
        let issuer = issuer_key();
        let world = world(100, 100);
        let extra_body = TokenIssue {
            currency: world.currency.compact_hash(),
            amount: 1,
            to: crate::currency::issuer_id(&alice_key()),
            memo: None,
        };
        let extra = sign_issue(
            &issuer,
            2,
            Some(world.issue.id),
            &world.currency,
            &extra_body,
            vec![world.issue.id],
        )
        .unwrap();
        let mut bodies = world.bodies.clone();
        bodies.insert(extra.id, extra_body.body().unwrap());
        let fold = fold_events(
            &[world.define.clone(), world.issue.clone(), extra.clone()],
            &bodies,
        );
        assert_eq!(
            fold.outcome(&extra.id),
            Some(FoldOutcome::Invalid(InvalidReason::OverCap))
        );
        assert_eq!(fold.supply(&world.currency.compact_hash()), 100);
    }

    #[test]
    fn forked_double_spend_voids_the_causally_later_spend() {
        let world = world(1_000, 50);
        let alice = alice_key();
        // Two spends of the full balance on a fork: neither causally follows
        // the other, so the deterministic tie-break decides. Both sign over
        // the same funding; exactly one may apply.
        let (s1, s1_body) = transfer(
            &world,
            &alice,
            0,
            None,
            vec![world.issue.id],
            &bob_key(),
            50,
        );
        let (s2, s2_body) = transfer(
            &world,
            &alice,
            0,
            None,
            vec![world.issue.id],
            &issuer_key(),
            50,
        );
        // sign_transfer with the same key/seq yields the same header apart
        // from the body hash — the two events are distinct spends.
        assert_ne!(s1.id, s2.id);
        let mut bodies = world.bodies.clone();
        bodies.insert(s1.id, s1_body.body().unwrap());
        bodies.insert(s2.id, s2_body.body().unwrap());
        let fold = fold_events(
            &[
                world.define.clone(),
                world.issue.clone(),
                s1.clone(),
                s2.clone(),
            ],
            &bodies,
        );
        let applied = [s1.id, s2.id]
            .iter()
            .filter(|id| fold.outcome(id) == Some(FoldOutcome::Applied))
            .count();
        let voided = [s1.id, s2.id]
            .iter()
            .filter(|id| {
                fold.outcome(id) == Some(FoldOutcome::Invalid(InvalidReason::InsufficientFunds))
            })
            .count();
        assert_eq!(applied, 1, "exactly one forked spend applies");
        assert_eq!(voided, 1, "the other is void for insufficient funds");
        assert_eq!(fold.supply(&world.currency.compact_hash()), 50);
    }

    #[test]
    fn missing_ancestry_is_pending_and_moves_no_value() {
        let world = world(1_000, 100);
        // Fold only the transfer's world minus the issue: the transfer names
        // the issue in `after`, which is absent, so it stays pending.
        let alice = alice_key();
        let (t1, t1_body) = transfer(
            &world,
            &alice,
            0,
            None,
            vec![world.issue.id],
            &bob_key(),
            10,
        );
        let mut bodies = world.bodies.clone();
        bodies.remove(&world.issue.id);
        bodies.insert(t1.id, t1_body.body().unwrap());
        let fold = fold_events(&[world.define.clone(), t1.clone()], &bodies);
        assert_eq!(fold.outcome(&t1.id), Some(FoldOutcome::Pending));
        assert_eq!(
            fold.balance(
                &world.currency.compact_hash(),
                &crate::currency::issuer_id(&bob_key())
            ),
            0
        );
    }

    #[test]
    fn burn_reduces_balance_and_counted_supply() {
        let world = world(1_000, 80);
        let alice = alice_key();
        let burn_body = TokenBurn {
            currency: world.currency.compact_hash(),
            amount: 30,
            seq_ref: 0,
        };
        let burn = sign_burn(&alice, 0, None, &burn_body, vec![world.issue.id]).unwrap();
        let mut bodies = world.bodies.clone();
        bodies.insert(burn.id, burn_body.body().unwrap());
        let fold = fold_events(
            &[world.define.clone(), world.issue.clone(), burn.clone()],
            &bodies,
        );
        let compact = world.currency.compact_hash();
        assert_eq!(fold.outcome(&burn.id), Some(FoldOutcome::Applied));
        assert_eq!(
            fold.balance(&compact, &crate::currency::issuer_id(&alice_key())),
            50
        );
        assert_eq!(fold.supply(&compact), 50);
    }

    #[test]
    fn unknown_currency_events_are_void() {
        let world = world(1_000, 10);
        let alice = alice_key();
        let unknown_body = TokenIssue {
            currency: Hash::digest(b"no such currency"),
            amount: 5,
            to: crate::currency::issuer_id(&alice),
            memo: None,
        };
        let unknown = sign_transfer(
            &alice,
            0,
            None,
            &TokenTransfer {
                currency: unknown_body.currency,
                amount: 5,
                to: unknown_body.to,
                seq_ref: 0,
                memo: None,
            },
            vec![],
        )
        .unwrap();
        let mut bodies = world.bodies.clone();
        bodies.insert(
            unknown.id,
            TokenTransfer {
                currency: unknown_body.currency,
                amount: 5,
                to: unknown_body.to,
                seq_ref: 0,
                memo: None,
            }
            .body()
            .unwrap(),
        );
        let fold = fold_events(
            &[world.define.clone(), world.issue, unknown.clone()],
            &bodies,
        );
        assert_eq!(
            fold.outcome(&unknown.id),
            Some(FoldOutcome::Invalid(InvalidReason::UnknownCurrency))
        );
    }

    /// Alice (funded with 100) pays Bob 40 in `t1`; Bob, funded only by
    /// `t1`, pays the issuer 30 in `t2`.
    fn funded_chain(world: &World) -> (Event, TokenTransfer, Event, TokenTransfer) {
        let (t1, t1_body) = transfer(
            world,
            &alice_key(),
            0,
            None,
            vec![world.issue.id],
            &bob_key(),
            40,
        );
        let (t2, t2_body) = transfer(world, &bob_key(), 0, None, vec![t1.id], &issuer_key(), 30);
        (t1, t1_body, t2, t2_body)
    }

    #[test]
    fn missing_body_keeps_dependents_pending_until_the_body_arrives() {
        let world = world(1_000, 100);
        let (t1, t1_body, t2, t2_body) = funded_chain(&world);
        let events = [
            world.define.clone(),
            world.issue.clone(),
            t1.clone(),
            t2.clone(),
        ];
        let compact = world.currency.compact_hash();
        let alice = crate::currency::issuer_id(&alice_key());
        let bob = crate::currency::issuer_id(&bob_key());
        let issuer = crate::currency::issuer_id(&issuer_key());

        // t1's body has not arrived: t1 is pending, and t2 — funded only by
        // t1 — must wait too rather than be voided for insufficient funds.
        let mut bodies = world.bodies;
        bodies.insert(t2.id, t2_body.body().unwrap());
        let fold = fold_events(&events, &bodies);
        assert_eq!(fold.outcome(&t1.id), Some(FoldOutcome::Pending));
        assert_eq!(fold.outcome(&t2.id), Some(FoldOutcome::Pending));
        assert_eq!(fold.balance(&compact, &alice), 100);
        assert_eq!(fold.balance(&compact, &bob), 0);
        assert_eq!(fold.balance(&compact, &issuer), 0);

        // Once the body arrives, the whole pending subtree is decided.
        bodies.insert(t1.id, t1_body.body().unwrap());
        let fold = fold_events(&events, &bodies);
        assert_eq!(fold.outcome(&t1.id), Some(FoldOutcome::Applied));
        assert_eq!(fold.outcome(&t2.id), Some(FoldOutcome::Applied));
        assert_eq!(fold.balance(&compact, &alice), 60);
        assert_eq!(fold.balance(&compact, &bob), 10);
        assert_eq!(fold.balance(&compact, &issuer), 30);
        assert_eq!(fold.supply(&compact), 100);
    }

    #[test]
    fn body_not_matching_the_signed_hash_is_ignored_and_pending() {
        let world = world(1_000, 100);
        let (t1, t1_body, t2, t2_body) = funded_chain(&world);
        let events = [
            world.define.clone(),
            world.issue.clone(),
            t1.clone(),
            t2.clone(),
        ];
        let compact = world.currency.compact_hash();
        let alice = crate::currency::issuer_id(&alice_key());
        let bob = crate::currency::issuer_id(&bob_key());
        let mallory = crate::currency::issuer_id(&SigningKey::from_bytes(&[13; 32]));

        // A well-formed body that redirects and inflates t1. It decodes
        // fine, but it is not the body t1's signed header commits to.
        let forged = TokenTransfer {
            to: mallory,
            amount: 100,
            ..t1_body.clone()
        };
        let mut bodies = world.bodies.clone();
        bodies.insert(t1.id, forged.body().unwrap());
        bodies.insert(t2.id, t2_body.body().unwrap());
        let fold = fold_events(&events, &bodies);
        assert_eq!(fold.outcome(&t1.id), Some(FoldOutcome::Pending));
        assert_eq!(fold.outcome(&t2.id), Some(FoldOutcome::Pending));
        assert_eq!(fold.balance(&compact, &alice), 100);
        assert_eq!(fold.balance(&compact, &mallory), 0);
        assert_eq!(fold.balance(&compact, &bob), 0);

        // A forged issuance body cannot mint to someone else either.
        let mut forged_issue = world.bodies.clone();
        forged_issue.insert(
            world.issue.id,
            TokenIssue {
                currency: compact,
                amount: 100,
                to: mallory,
                memo: None,
            }
            .body()
            .unwrap(),
        );
        let fold = fold_events(&[world.define.clone(), world.issue.clone()], &forged_issue);
        assert_eq!(fold.outcome(&world.issue.id), Some(FoldOutcome::Pending));
        assert_eq!(fold.balance(&compact, &mallory), 0);
        assert_eq!(fold.supply(&compact), 0);

        // The genuine body still decides the event the forgery could not.
        bodies.insert(t1.id, t1_body.body().unwrap());
        let fold = fold_events(&events, &bodies);
        assert_eq!(fold.outcome(&t1.id), Some(FoldOutcome::Applied));
        assert_eq!(fold.outcome(&t2.id), Some(FoldOutcome::Applied));
        assert_eq!(fold.balance(&compact, &mallory), 0);
        assert_eq!(fold.balance(&compact, &bob), 10);
    }

    #[test]
    fn currency_event_committing_to_no_body_is_invalid() {
        let world = world(1_000, 100);
        let (t1, t1_body, _, _) = funded_chain(&world);
        // Same signer and extension as t1, but the header commits to no body.
        let bodiless =
            crate::chain::sign(&alice_key(), 0, None, None, t1.header.extensions).unwrap();
        assert_eq!(bodiless.header.payload_hash, None);
        let mut bodies = world.bodies.clone();
        bodies.insert(bodiless.id, t1_body.body().unwrap());
        let fold = fold_events(
            &[world.define.clone(), world.issue.clone(), bodiless.clone()],
            &bodies,
        );
        assert_eq!(
            fold.outcome(&bodiless.id),
            Some(FoldOutcome::Invalid(InvalidReason::InvalidBody))
        );
        assert_eq!(
            fold.balance(
                &world.currency.compact_hash(),
                &crate::currency::issuer_id(&bob_key())
            ),
            0
        );
    }

    // ---- issuer rotation (design #2804 §3.3) ----

    fn successor_key() -> SigningKey {
        SigningKey::from_bytes(&[13; 32])
    }

    fn rival_key() -> SigningKey {
        SigningKey::from_bytes(&[15; 32])
    }

    /// A routine rotation of `world`'s currency, signed by `by` at `seq`
    /// on `backlink`, installing `to` as generation `from + 1`.
    fn rotate(
        world: &World,
        by: &SigningKey,
        seq: u32,
        backlink: Option<Hash>,
        from: u64,
        previous: Option<Hash>,
        to: &SigningKey,
    ) -> Event {
        let mut rotation = IssuerRotation::unsigned(
            crate::issuer_rotation::RotationMode::Routine,
            world.currency.issuer(),
            world.currency.compact_hash(),
            from,
            previous,
            crate::currency::issuer_id(to),
            None,
        );
        rotation.prove_successor(to).unwrap();
        crate::issuer_rotation::sign_rotation(
            by,
            seq,
            backlink,
            &rotation,
            backlink.into_iter().collect(),
        )
        .unwrap()
    }

    /// An issuance of `amount` to Alice signed by any generation key.
    fn issue_by(
        world: &World,
        key: &SigningKey,
        seq: u32,
        backlink: Option<Hash>,
        after: Vec<Hash>,
        amount: u128,
    ) -> (Event, Vec<u8>) {
        let body = TokenIssue {
            currency: world.currency.compact_hash(),
            amount,
            to: crate::currency::issuer_id(&alice_key()),
            memo: None,
        };
        let event = crate::currency::graph::sign_generation_issue(
            key,
            seq,
            backlink,
            &world.currency,
            &body,
            after,
        )
        .unwrap();
        (event, body.body().unwrap())
    }

    /// Folds `events` in the given order and reversed, asserting both
    /// folds agree on every verdict, and returns the first.
    fn fold_both_ways(events: &[Event], bodies: &BTreeMap<Hash, Vec<u8>>) -> CurrencyFold {
        let fold = fold_events(events, bodies);
        let mut reversed = events.to_vec();
        reversed.reverse();
        let back = fold_events(&reversed, bodies);
        for event in events {
            assert_eq!(fold.outcome(&event.id), back.outcome(&event.id));
        }
        fold
    }

    #[test]
    fn a_routine_rotation_hands_issuance_to_the_successor() {
        let mut world = world(1_000, 100);
        let (old, new) = (issuer_key(), successor_key());
        let rotation = rotate(&world, &old, 2, Some(world.issue.id), 0, None, &new);
        let (by_new, by_new_body) = issue_by(&world, &new, 0, None, vec![rotation.id], 50);
        let (by_old, by_old_body) = issue_by(&world, &old, 3, Some(rotation.id), vec![], 7);
        world.bodies.insert(by_new.id, by_new_body);
        world.bodies.insert(by_old.id, by_old_body);
        let events = vec![
            world.define.clone(),
            world.issue.clone(),
            rotation.clone(),
            by_new.clone(),
            by_old.clone(),
        ];
        let fold = fold_both_ways(&events, &world.bodies);
        let c = world.currency.compact_hash();

        // History before the rotation stands; the successor issues after
        // it; the retired key cannot issue past its own rotation.
        assert_eq!(fold.outcome(&world.issue.id), Some(FoldOutcome::Applied));
        assert_eq!(fold.outcome(&by_new.id), Some(FoldOutcome::Applied));
        assert_eq!(
            fold.outcome(&by_old.id),
            Some(FoldOutcome::Invalid(InvalidReason::RetiredGeneration))
        );
        assert_eq!(fold.supply(&c), 150);
        assert_eq!(fold.disputed(&c), 0);
        let chain = fold.generation_chain(&c).unwrap();
        assert_eq!(
            chain.keys,
            vec![world.currency.issuer(), crate::currency::issuer_id(&new)]
        );
        assert_eq!(chain.transitions, vec![rotation.id]);
    }

    #[test]
    fn a_successor_that_does_not_follow_its_rotation_is_not_authorised() {
        let mut world = world(1_000, 100);
        let (old, new) = (issuer_key(), successor_key());
        let rotation = rotate(&world, &old, 2, Some(world.issue.id), 0, None, &new);
        // Signed by the successor but linked only to the definition.
        let (early, early_body) = issue_by(&world, &new, 0, None, vec![world.define.id], 50);
        world.bodies.insert(early.id, early_body);
        let events = vec![
            world.define.clone(),
            world.issue.clone(),
            rotation,
            early.clone(),
        ];
        let fold = fold_both_ways(&events, &world.bodies);
        assert_eq!(
            fold.outcome(&early.id),
            Some(FoldOutcome::Invalid(InvalidReason::UnlinkedSuccessor))
        );
        assert_eq!(fold.supply(&world.currency.compact_hash()), 100);
    }

    #[test]
    fn issuance_concurrent_with_a_rotation_is_disputed_not_settled() {
        let mut world = world(1_000, 100);
        let (old, new) = (issuer_key(), successor_key());
        let rotation = rotate(&world, &old, 2, Some(world.issue.id), 0, None, &new);
        // The old key forks its chain at the rotation's sequence number:
        // neither before nor after the rotation.
        let (forked, forked_body) = issue_by(&world, &old, 2, Some(world.issue.id), vec![], 40);
        world.bodies.insert(forked.id, forked_body);
        let events = vec![
            world.define.clone(),
            world.issue.clone(),
            rotation,
            forked.clone(),
        ];
        let fold = fold_both_ways(&events, &world.bodies);
        let c = world.currency.compact_hash();
        assert_eq!(
            fold.outcome(&forked.id),
            Some(FoldOutcome::Disputed(DisputeReason::ConcurrentWithRotation))
        );
        assert_eq!(fold.disputed(&c), 40);
        assert_eq!(fold.supply(&c), 100);
        assert_eq!(
            fold.balance(&c, &crate::currency::issuer_id(&alice_key())),
            100
        );
    }

    #[test]
    fn a_forked_chain_disputes_both_branches_and_keeps_history() {
        let mut world = world(1_000, 100);
        let (old, new, rival) = (issuer_key(), successor_key(), rival_key());
        // The old key's chain forks at sequence 2: an issuance `pre` that
        // the rotation to `new` follows, and the rotation to `rival`, which
        // is concurrent with `pre`.
        let (pre, pre_body) = issue_by(&world, &old, 2, Some(world.issue.id), vec![], 3);
        let to_new = rotate(&world, &old, 3, Some(pre.id), 0, None, &new);
        let to_rival = rotate(&world, &old, 2, Some(world.issue.id), 0, None, &rival);
        let (by_new, by_new_body) = issue_by(&world, &new, 0, None, vec![to_new.id], 5);
        let (by_rival, by_rival_body) = issue_by(&world, &rival, 0, None, vec![to_rival.id], 6);
        let (by_old, by_old_body) = issue_by(&world, &old, 4, Some(to_new.id), vec![], 7);
        // A rival successor's issuance before its own rotation is no branch.
        let (unlinked, unlinked_body) = issue_by(&world, &rival, 0, None, vec![world.define.id], 9);
        // A stranger who no rotation names stays a stranger.
        let (stranger, stranger_body) = issue_by(&world, &bob_key(), 0, None, vec![to_new.id], 8);
        for (event, body) in [
            (&pre, pre_body),
            (&unlinked, unlinked_body),
            (&by_new, by_new_body),
            (&by_rival, by_rival_body),
            (&by_old, by_old_body),
            (&stranger, stranger_body),
        ] {
            world.bodies.insert(event.id, body);
        }
        let events = vec![
            world.define.clone(),
            world.issue.clone(),
            pre.clone(),
            to_new,
            to_rival,
            unlinked.clone(),
            by_new.clone(),
            by_rival.clone(),
            by_old.clone(),
            stranger.clone(),
        ];
        let fold = fold_both_ways(&events, &world.bodies);
        let c = world.currency.compact_hash();
        let forked = Some(FoldOutcome::Disputed(DisputeReason::ForkedChain));
        assert_eq!(fold.outcome(&world.issue.id), Some(FoldOutcome::Applied));
        // `pre` precedes one competing rotation but not the other.
        assert_eq!(fold.outcome(&pre.id), forked);
        assert_eq!(
            fold.outcome(&unlinked.id),
            Some(FoldOutcome::Invalid(InvalidReason::NotIssuer))
        );
        assert_eq!(fold.outcome(&by_new.id), forked);
        assert_eq!(fold.outcome(&by_rival.id), forked);
        assert_eq!(fold.outcome(&by_old.id), forked);
        assert_eq!(
            fold.outcome(&stranger.id),
            Some(FoldOutcome::Invalid(InvalidReason::NotIssuer))
        );
        assert_eq!(fold.supply(&c), 100);
        assert_eq!(fold.disputed(&c), 21);
        assert!(matches!(
            fold.generation_chain(&c).unwrap().status,
            ChainStatus::Conflicted { generation: 1, .. }
        ));
    }

    #[test]
    fn a_key_rotated_back_in_issues_again_in_its_new_interval() {
        let mut world = world(1_000, 100);
        let (old, new) = (issuer_key(), successor_key());
        let first = rotate(&world, &old, 2, Some(world.issue.id), 0, None, &new);
        let back = {
            let mut rotation = IssuerRotation::unsigned(
                crate::issuer_rotation::RotationMode::Routine,
                world.currency.issuer(),
                world.currency.compact_hash(),
                1,
                Some(first.id),
                world.currency.issuer(),
                None,
            );
            rotation.prove_successor(&old).unwrap();
            crate::issuer_rotation::sign_rotation(&new, 0, None, &rotation, vec![first.id]).unwrap()
        };
        let (again, again_body) = issue_by(&world, &old, 3, Some(first.id), vec![back.id], 9);
        let (late, late_body) = issue_by(&world, &new, 1, Some(back.id), vec![], 4);
        world.bodies.insert(again.id, again_body);
        world.bodies.insert(late.id, late_body);
        let events = vec![
            world.define.clone(),
            world.issue.clone(),
            first,
            back,
            again.clone(),
            late.clone(),
        ];
        let fold = fold_both_ways(&events, &world.bodies);
        assert_eq!(fold.outcome(&again.id), Some(FoldOutcome::Applied));
        assert_eq!(
            fold.outcome(&late.id),
            Some(FoldOutcome::Invalid(InvalidReason::RetiredGeneration))
        );
        assert_eq!(fold.supply(&world.currency.compact_hash()), 109);
    }

    #[test]
    fn without_a_rotation_only_the_definition_key_issues() {
        let mut world = world(1_000, 100);
        let (stranger, body) = issue_by(&world, &bob_key(), 0, None, vec![world.define.id], 5);
        world.bodies.insert(stranger.id, body);
        let events = vec![world.define.clone(), world.issue.clone(), stranger.clone()];
        let fold = fold_both_ways(&events, &world.bodies);
        assert_eq!(
            fold.outcome(&stranger.id),
            Some(FoldOutcome::Invalid(InvalidReason::NotIssuer))
        );
        assert_eq!(fold.supply(&world.currency.compact_hash()), 100);
    }

    #[test]
    fn a_rotation_of_another_currency_changes_nothing_here() {
        let mut world = world(1_000, 100);
        let (old, new) = (issuer_key(), successor_key());
        let mut rotation = IssuerRotation::unsigned(
            crate::issuer_rotation::RotationMode::Routine,
            world.currency.issuer(),
            Hash::digest(b"some other currency"),
            0,
            None,
            crate::currency::issuer_id(&new),
            None,
        );
        rotation.prove_successor(&new).unwrap();
        let elsewhere =
            crate::issuer_rotation::sign_rotation(&old, 2, Some(world.issue.id), &rotation, vec![])
                .unwrap();
        let (more, more_body) = issue_by(&world, &old, 3, Some(elsewhere.id), vec![], 11);
        world.bodies.insert(more.id, more_body);
        let events = vec![
            world.define.clone(),
            world.issue.clone(),
            elsewhere,
            more.clone(),
        ];
        let fold = fold_both_ways(&events, &world.bodies);
        let c = world.currency.compact_hash();
        assert_eq!(fold.outcome(&more.id), Some(FoldOutcome::Applied));
        assert!(fold.generation_chain(&c).is_none());
        assert_eq!(fold.supply(&c), 111);
    }
}
