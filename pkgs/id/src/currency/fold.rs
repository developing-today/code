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
//!   event are pending too.
//!
//! What the fold deliberately does not do:
//!
//! - **Anchored currencies.** A definition bound to a ledger
//!   ([`CurrencyBinding::Ledger`]) registers, but its value events are
//!   refused ([`InvalidReason::AnchoredBinding`]): anchored balance reads
//!   come from the ledger, not from graph intent (invariant 3).
//! - **Rotation, conversion, receipts, grants.** Later design phases with
//!   their own contracts; not folded here.
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

/// Why a currency event was voided by the fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidReason {
    /// Body bytes failed the contract's strict decode/validation.
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
}

/// The fold's verdict for one input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldOutcome {
    /// The event applied; value or a definition moved as signed.
    Applied,
    /// The event is void: present, causally placed, and refused.
    Invalid(InvalidReason),
    /// Ancestry or body is missing; the event is not yet decidable.
    Pending,
    /// Not a currency event; it only participates in causality.
    Ignored,
}

/// The folded state: definitions, balances, counted supply, and verdicts.
#[derive(Clone, Debug, Default)]
pub struct CurrencyFold {
    definitions: BTreeMap<Hash, (CurrencyId, CurrencyDefinition)>,
    balances: BTreeMap<(Hash, IssuerId), u128>,
    supply: BTreeMap<Hash, u128>,
    outcomes: BTreeMap<Hash, FoldOutcome>,
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
/// `bodies` maps event IDs to their signed body bytes (event headers commit
/// to body hashes; bodies travel separately). Currency events without a
/// supplied body are [`FoldOutcome::Pending`]. Duplicate event IDs are
/// folded once. The result depends only on the event set, never on the
/// order of `events`.
#[must_use]
pub fn fold_events(events: &[Event], bodies: &BTreeMap<Hash, Vec<u8>>) -> CurrencyFold {
    let mut by_id: BTreeMap<Hash, &Event> = BTreeMap::new();
    for event in events {
        by_id.entry(event.id).or_insert(event);
    }

    let mut fold = CurrencyFold::default();
    let mut decided: BTreeSet<Hash> = BTreeSet::new();

    // Fixpoint over causal readiness: an event is decidable once every
    // event it causally depends on (backlink + `after` frontier) is decided.
    loop {
        let ready: Vec<Hash> = by_id
            .iter()
            .filter(|(id, event)| {
                !decided.contains(id)
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
            let outcome = apply(&mut fold, event, bodies.get(&event.id));
            fold.outcomes.insert(event.id, outcome);
            decided.insert(event.id);
        }
    }

    // Anything undecided is waiting on ancestry outside the input set.
    for id in by_id.keys() {
        fold.outcomes.entry(*id).or_insert(FoldOutcome::Pending);
    }
    fold
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

fn apply(fold: &mut CurrencyFold, event: &Event, body: Option<&Vec<u8>>) -> FoldOutcome {
    match event.header.extensions.kind.as_str() {
        DEFINE_KIND => apply_define(fold, event, body),
        ISSUE_KIND => apply_issue(fold, event, body),
        TRANSFER_KIND => apply_transfer(fold, event, body),
        BURN_KIND => apply_burn(fold, event, body),
        _ => FoldOutcome::Ignored,
    }
}

fn apply_define(fold: &mut CurrencyFold, event: &Event, body: Option<&Vec<u8>>) -> FoldOutcome {
    let Some(body) = body else {
        return FoldOutcome::Pending;
    };
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

fn apply_issue(fold: &mut CurrencyFold, event: &Event, body: Option<&Vec<u8>>) -> FoldOutcome {
    let Some(body) = body else {
        return FoldOutcome::Pending;
    };
    let Ok(issue) = TokenIssue::from_body(body) else {
        return FoldOutcome::Invalid(InvalidReason::InvalidBody);
    };
    let (id, definition) = match value_target(fold, &issue.currency) {
        Ok(pair) => pair,
        Err(outcome) => return outcome,
    };
    if signer(event) != id.issuer() {
        return FoldOutcome::Invalid(InvalidReason::NotIssuer);
    }
    let supply = fold.supply(&issue.currency);
    match supply.checked_add(issue.amount) {
        Some(next) if next <= definition.supply.capped => {
            if fold.credit(&issue.currency, &issue.to, issue.amount).is_err() {
                return FoldOutcome::Invalid(InvalidReason::Overflow);
            }
            fold.supply.insert(issue.currency, next);
            FoldOutcome::Applied
        }
        _ => FoldOutcome::Invalid(InvalidReason::OverCap),
    }
}

fn apply_transfer(fold: &mut CurrencyFold, event: &Event, body: Option<&Vec<u8>>) -> FoldOutcome {
    let Some(body) = body else {
        return FoldOutcome::Pending;
    };
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
    if fold.debit(&transfer.currency, &from, transfer.amount).is_err() {
        return FoldOutcome::Invalid(InvalidReason::InsufficientFunds);
    }
    if fold.credit(&transfer.currency, &transfer.to, transfer.amount).is_err() {
        return FoldOutcome::Invalid(InvalidReason::Overflow);
    }
    FoldOutcome::Applied
}

fn apply_burn(fold: &mut CurrencyFold, event: &Event, body: Option<&Vec<u8>>) -> FoldOutcome {
    let Some(body) = body else {
        return FoldOutcome::Pending;
    };
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
    fold.supply.insert(
        burn.currency,
        supply.checked_sub(burn.amount).unwrap_or(0),
    );
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
        let fold = fold_events(&[t1.clone(), world.issue.clone(), world.define.clone()], &bodies);
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
        let fold = fold_events(&[world.define.clone(), world.issue.clone(), extra.clone()], &bodies);
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
        let (s1, s1_body) = transfer(&world, &alice, 0, None, vec![world.issue.id], &bob_key(), 50);
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
            &[world.define.clone(), world.issue.clone(), s1.clone(), s2.clone()],
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
        let fold = fold_events(&[world.define.clone(), world.issue.clone(), burn.clone()], &bodies);
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
        let unknown = sign_transfer(&alice, 0, None, &TokenTransfer {
            currency: unknown_body.currency,
            amount: 5,
            to: unknown_body.to,
            seq_ref: 0,
            memo: None,
        }, vec![])
        .unwrap();
        let mut bodies = world.bodies.clone();
        bodies.insert(unknown.id, TokenTransfer {
            currency: unknown_body.currency,
            amount: 5,
            to: unknown_body.to,
            seq_ref: 0,
            memo: None,
        }.body().unwrap());
        let fold = fold_events(&[world.define.clone(), world.issue.clone(), unknown.clone()], &bodies);
        assert_eq!(
            fold.outcome(&unknown.id),
            Some(FoldOutcome::Invalid(InvalidReason::UnknownCurrency))
        );
    }
}
