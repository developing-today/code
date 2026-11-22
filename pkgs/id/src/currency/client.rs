//! Stateful graph-native currency client (#2803, graph tranche).
//!
//! The client owns an in-memory local signed event set and matching bodies. It
//! signs intents, but only the deterministic fold decides what counted in this
//! observed set. This is an experimental API, not a production wallet or
//! payment processor: the current fold is not yet conformant with the spec
//! rules for causal funding, causal cap checks, or unresolved-fork treatment.
//! Outcomes may change as events arrive; retries are not idempotent across
//! restarts; callers must not use balances, supply, snapshots, or `Applied`
//! verdicts for authorization or external effects. Gateway submission and
//! ledger-bound currencies intentionally live elsewhere.

use std::collections::BTreeMap;

use p2panda_core::{Hash, SeqNum, SigningKey};

use crate::chain::Event;
use crate::currency::fold::{CurrencyFold, FoldOutcome, fold_events};
use crate::currency::graph::{
    CurrencyBinding, CurrencyDefinition, CurrencyId, SupplyCap, TokenBurn, TokenError, TokenIssue,
    TokenTransfer, sign_burn, sign_definition, sign_issue, sign_transfer,
};
use crate::currency::snapshot::{self, SnapshotError, TokenSnapshot, sign_snapshot};
use crate::currency::{IssuerId, issuer_id};
use crate::issuer_rotation::ChainStatus;

/// Errors from graph-native client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// A graph-only operation was requested for a ledger-bound currency.
    #[error("currency is not graph-bound")]
    NotGraphBound,
    /// A signing or body-contract operation failed.
    #[error(transparent)]
    Token(#[from] TokenError),
    /// The local graph has not registered the named currency.
    #[error("currency is not present in the local graph")]
    UnknownCurrency,
    /// The caller is not the issuer of the named currency.
    #[error("client key is not authorized for this currency operation")]
    NotIssuer,
    /// A snapshot cannot be made for this local graph state.
    #[error("snapshot is unavailable for this local graph state: {0:?}")]
    Snapshot(SnapshotError),
    /// The local signing chain exhausted its sequence-number range.
    #[error("author sequence number is exhausted")]
    SequenceExhausted,
    /// A chain has multiple observed events at the same highest sequence.
    #[error("cannot author against an unresolved author-chain fork")]
    UnresolvedFork,
}

/// Result of a signed mutation. A graph intent is returned even if the local
/// fold says it is void or pending; those are state outcomes, not signing
/// errors. `Applied` means only that the current in-memory event set folds it
/// as applied; it is not durable acceptance, a receipt, or authorization for
/// an external side effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MutationResult {
    /// Content-addressed signed event ID.
    pub event_id: Hash,
    /// Current local fold verdict for that event.
    pub outcome: FoldOutcome,
}

/// An in-memory graph-native view over a caller-provided signing key.
///
/// This type offers no persistence or retry idempotency. Its balances, supply,
/// snapshots, and mutation outcomes are experimental local projections; the
/// current fold does not yet implement the spec's causal-funding, causal-cap,
/// and unresolved-fork rules. Do not use them for authorization or external
/// effects.
#[derive(Debug)]
pub struct GraphCurrencyClient {
    key: SigningKey,
    events: Vec<Event>,
    bodies: BTreeMap<Hash, Vec<u8>>,
}

impl GraphCurrencyClient {
    /// Start with an empty local graph and the given signing key.
    #[must_use]
    pub fn new(key: SigningKey) -> Self {
        Self {
            key,
            events: Vec::new(),
            bodies: BTreeMap::new(),
        }
    }

    /// Add a previously verified graph event and its body to the client's view.
    ///
    /// Signature and chain verification are the ingest layer's responsibility;
    /// the fold still checks body hashes before using the supplied body.
    pub fn observe(&mut self, event: Event, body: Option<Vec<u8>>) {
        if let Some(body) = body {
            if event.header.payload_hash == Some(Hash::digest(&body)) {
                self.bodies.entry(event.id).or_insert(body);
            }
        }
        if !self.events.iter().any(|held| held.id == event.id) {
            self.events.push(event);
        }
    }

    /// All locally observed events.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Bodies keyed by their signed event IDs, suitable for [`fold_events`].
    #[must_use]
    pub fn bodies(&self) -> &BTreeMap<Hash, Vec<u8>> {
        &self.bodies
    }

    /// The current deterministic view of the local graph.
    #[must_use]
    pub fn fold(&self) -> CurrencyFold {
        fold_events(&self.events, &self.bodies)
    }

    /// Sign and append a currency definition with the selected balance binding.
    ///
    /// This client can construct a ledger-bound definition, but graph-only
    /// operations reject it; executing ledger operations belongs to the
    /// separately gated gateway client.
    pub fn define_currency(
        &mut self,
        name: impl Into<String>,
        symbol: impl Into<String>,
        scale: u8,
        supply: SupplyCap,
        binding: CurrencyBinding,
    ) -> Result<CurrencyId, ClientError> {
        let definition = CurrencyDefinition {
            name: name.into(),
            symbol: symbol.into(),
            scale,
            supply,
            binding,
            memo: None,
        };
        let (currency, event) = sign_definition(
            &self.key,
            self.next_seq()?,
            self.backlink(),
            &definition,
            self.frontier()?,
        )?;
        self.record(event, definition.body()?);
        Ok(currency)
    }

    /// Sign and append an issuance. The issuer check is performed before
    /// signing; cap acceptance is reported by the resulting fold outcome.
    pub fn issue(
        &mut self,
        currency: &CurrencyId,
        to: IssuerId,
        amount: u128,
        memo: Option<String>,
    ) -> Result<MutationResult, ClientError> {
        self.require_graph(currency)?;
        if issuer_id(&self.key) != currency.issuer() {
            return Err(ClientError::NotIssuer);
        }
        let body = TokenIssue {
            currency: currency.compact_hash(),
            amount,
            to,
            memo,
        };
        let encoded = body.body()?;
        let event = sign_issue(
            &self.key,
            self.next_seq()?,
            self.backlink(),
            currency,
            &body,
            self.frontier()?,
        )?;
        Ok(self.record_mutation(event, encoded))
    }

    /// Sign a holder-transfer intent. Insufficient funds are a fold verdict,
    /// not a signing-time failure. This is not a payment receipt and must not
    /// be used to trigger an irreversible external effect.
    pub fn pay(
        &mut self,
        currency: &CurrencyId,
        to: IssuerId,
        amount: u128,
        memo: Option<String>,
    ) -> Result<MutationResult, ClientError> {
        self.transfer(currency, to, amount, memo)
    }

    /// Sign a holder burn intent. The current local fold reports whether the
    /// burn is applied; this is not durable acceptance or an authorization
    /// signal.
    pub fn burn(
        &mut self,
        currency: &CurrencyId,
        amount: u128,
    ) -> Result<MutationResult, ClientError> {
        self.require_graph(currency)?;
        let seq = self.next_seq()?;
        let body = TokenBurn {
            currency: currency.compact_hash(),
            amount,
            seq_ref: u64::from(seq),
        };
        let encoded = body.body()?;
        let event = sign_burn(&self.key, seq, self.backlink(), &body, self.frontier()?)?;
        Ok(self.record_mutation(event, encoded))
    }

    /// Sign an award transfer intent through the same contract as [`Self::pay`].
    /// The memo should carry the cause reference (for example a match ID). This
    /// does not establish causal funding or restart-safe idempotency.
    pub fn award(
        &mut self,
        currency: &CurrencyId,
        to: IssuerId,
        amount: u128,
        memo: String,
    ) -> Result<MutationResult, ClientError> {
        self.transfer(currency, to, amount, Some(memo))
    }

    /// Return the balance in the current local fold, or `UnknownCurrency` if
    /// no definition exists. This is not an authoritative or durable balance.
    pub fn balance(&self, currency: &CurrencyId, of: &IssuerId) -> Result<u128, ClientError> {
        self.require_graph(currency)?;
        let fold = self.fold();
        if fold.definition(&currency.compact_hash()).is_none() {
            return Err(ClientError::UnknownCurrency);
        }
        Ok(fold.balance(&currency.compact_hash(), of))
    }

    /// Return `(issued, burned, counted_supply)` from the current local fold.
    /// This is not an authoritative or durable supply, and the current fold
    /// is not yet conformant with the specified causal-cap and fork rules.
    pub fn supply(&self, currency: &CurrencyId) -> Result<(u128, u128, u128), ClientError> {
        self.require_graph(currency)?;
        let fold = self.fold();
        let id = currency.compact_hash();
        if fold.definition(&id).is_none() {
            return Err(ClientError::UnknownCurrency);
        }
        Ok((fold.issued(&id), fold.burned(&id), fold.supply(&id)))
    }

    /// Create and sign a snapshot at the current decided graph frontier.
    pub fn snapshot(&mut self, currency: &CurrencyId) -> Result<TokenSnapshot, ClientError> {
        self.require_graph(currency)?;
        let fold = self.fold();
        let id = currency.compact_hash();
        let state = snapshot::snapshot(&fold, &id).map_err(ClientError::Snapshot)?;
        let body = state.body()?;
        let active_issuer = match fold.generation_chain(&id) {
            None => currency.issuer(),
            Some(chain) if chain.status == ChainStatus::Settled => *chain.active_key(),
            Some(_) => return Err(ClientError::NotIssuer),
        };
        if issuer_id(&self.key) != active_issuer {
            return Err(ClientError::NotIssuer);
        }
        let seq = self.next_seq()?;
        let backlink = self.backlink();
        let frontier = self.frontier()?;
        let event = if active_issuer == currency.issuer() {
            sign_snapshot(&self.key, seq, backlink, currency, &state, frontier)?
        } else {
            snapshot::sign_generation_snapshot(
                &self.key, seq, backlink, currency, &state, frontier,
            )?
        };
        self.record(event, body);
        Ok(state)
    }

    /// Find applied transfers matching a payment predicate. The caller's
    /// effect/receipt lifecycle is deliberately a host concern, not performed
    /// by this pure graph client.
    pub fn matching_payments(
        &self,
        currency: &CurrencyId,
        min_amount: u128,
        memo_contains: Option<&str>,
    ) -> Result<Vec<Hash>, ClientError> {
        self.require_graph(currency)?;
        let id = currency.compact_hash();
        let fold = self.fold();
        Ok(self
            .events
            .iter()
            .filter(|event| {
                event.header.extensions.kind == super::graph::TRANSFER_KIND
                    && fold.outcome(&event.id) == Some(FoldOutcome::Applied)
                    && self
                        .bodies
                        .get(&event.id)
                        .and_then(|body| TokenTransfer::from_body(body).ok())
                        .is_some_and(|transfer| {
                            transfer.currency == id
                                && transfer.amount >= min_amount
                                && memo_contains.is_none_or(|needle| {
                                    transfer
                                        .memo
                                        .as_deref()
                                        .is_some_and(|memo| memo.contains(needle))
                                })
                        })
            })
            .map(|event| event.id)
            .collect())
    }

    fn transfer(
        &mut self,
        currency: &CurrencyId,
        to: IssuerId,
        amount: u128,
        memo: Option<String>,
    ) -> Result<MutationResult, ClientError> {
        self.require_graph(currency)?;
        let seq = self.next_seq()?;
        let body = TokenTransfer {
            currency: currency.compact_hash(),
            amount,
            to,
            seq_ref: u64::from(seq),
            memo,
        };
        let encoded = body.body()?;
        let event = sign_transfer(&self.key, seq, self.backlink(), &body, self.frontier()?)?;
        Ok(self.record_mutation(event, encoded))
    }

    fn require_graph(&self, currency: &CurrencyId) -> Result<(), ClientError> {
        let fold = self.fold();
        let Some((_, definition)) = fold.definition(&currency.compact_hash()) else {
            return Err(ClientError::UnknownCurrency);
        };
        if definition.binding == CurrencyBinding::Graph {
            Ok(())
        } else {
            Err(ClientError::NotGraphBound)
        }
    }

    fn next_seq(&self) -> Result<SeqNum, ClientError> {
        let author = issuer_id(&self.key);
        let mut by_seq = BTreeMap::<u32, Hash>::new();
        for event in self
            .events
            .iter()
            .filter(|event| event.header.verifying_key.as_bytes() == &author)
        {
            let seq = u32::from(event.header.seq_num);
            if by_seq
                .insert(seq, event.id)
                .is_some_and(|held| held != event.id)
            {
                return Err(ClientError::UnresolvedFork);
            }
        }
        let latest = self
            .events
            .iter()
            .filter(|event| event.header.verifying_key.as_bytes() == &author)
            .max_by_key(|event| event.header.seq_num);
        latest.map_or(Ok(0), |event| {
            u32::from(event.header.seq_num)
                .checked_add(1)
                .ok_or(ClientError::SequenceExhausted)
        })
    }

    fn backlink(&self) -> Option<Hash> {
        let author = issuer_id(&self.key);
        self.events
            .iter()
            .filter(|event| event.header.verifying_key.as_bytes() == &author)
            .max_by_key(|event| event.header.seq_num)
            .map(|event| event.id)
    }

    fn frontier(&self) -> Result<Vec<Hash>, ClientError> {
        let author = issuer_id(&self.key);
        let mut latest = BTreeMap::<IssuerId, &Event>::new();
        let mut seen_seq = BTreeMap::<(IssuerId, u32), Hash>::new();
        for event in &self.events {
            let key: IssuerId = *event.header.verifying_key.as_bytes();
            let seq = u32::from(event.header.seq_num);
            if key != author
                && seen_seq
                    .insert((key, seq), event.id)
                    .is_some_and(|held| held != event.id)
            {
                return Err(ClientError::UnresolvedFork);
            }
            if key == author {
                continue;
            }
            if latest
                .get(&key)
                .is_none_or(|held| held.header.seq_num < event.header.seq_num)
            {
                latest.insert(key, event);
            }
        }
        Ok(latest.values().map(|event| event.id).collect())
    }

    fn record(&mut self, event: Event, body: Vec<u8>) {
        self.bodies.insert(event.id, body);
        self.events.push(event);
    }

    fn record_mutation(&mut self, event: Event, body: Vec<u8>) -> MutationResult {
        let event_id = event.id;
        self.record(event, body);
        MutationResult {
            event_id,
            outcome: self
                .fold()
                .outcome(&event_id)
                .unwrap_or(FoldOutcome::Pending),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn key(byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[byte; 32])
    }

    #[test]
    fn snapshot_signer_tracks_the_active_issuer_generation() {
        use crate::issuer_rotation::{IssuerRotation, RotationMode, sign_rotation};

        let issuer_key = key(7);
        let successor_key = key(8);
        let mut issuer = GraphCurrencyClient::new(issuer_key.clone());
        let currency = issuer
            .define_currency(
                "season",
                "S",
                0,
                SupplyCap { capped: 100 },
                CurrencyBinding::Graph,
            )
            .unwrap();
        let definition_event = issuer.events()[0].clone();

        let mut rotation = IssuerRotation::unsigned(
            RotationMode::Routine,
            issuer_id(&issuer_key),
            currency.compact_hash(),
            0,
            None,
            issuer_id(&successor_key),
            None,
        );
        rotation.prove_successor(&successor_key).unwrap();
        let rotation_event = sign_rotation(
            &issuer_key,
            1,
            Some(definition_event.id),
            &rotation,
            vec![definition_event.id],
        )
        .unwrap();
        issuer.observe(rotation_event, None);

        // The retired generation must not sign a snapshot of the new frontier.
        assert!(matches!(
            issuer.snapshot(&currency),
            Err(ClientError::NotIssuer)
        ));

        let mut successor = GraphCurrencyClient::new(successor_key.clone());
        for event in issuer.events().iter().cloned() {
            successor.observe(event.clone(), issuer.bodies().get(&event.id).cloned());
        }
        successor.snapshot(&currency).unwrap();

        let snapshot_event = successor.events().last().unwrap();
        let snapshot_body = successor.bodies().get(&snapshot_event.id).unwrap();
        assert_eq!(
            snapshot_event.header.verifying_key.as_bytes(),
            issuer_id(&successor_key).as_slice()
        );
        assert!(
            snapshot::verify_snapshot(
                snapshot_event,
                Some(snapshot_body),
                successor.events(),
                successor.bodies(),
            )
            .is_ok()
        );
    }

    #[test]
    fn graph_native_issue_and_pay_example_folds_end_to_end() {
        let mut alice = GraphCurrencyClient::new(key(7));
        let bob_id = issuer_id(&key(9));
        let currency = alice
            .define_currency(
                "alice-credits",
                "AC",
                2,
                SupplyCap { capped: 10_000 },
                CurrencyBinding::Graph,
            )
            .unwrap();
        assert_eq!(
            alice
                .issue(&currency, bob_id, 10_000, Some("welcome".to_owned()))
                .unwrap()
                .outcome,
            FoldOutcome::Applied
        );
        assert_eq!(alice.balance(&currency, &bob_id).unwrap(), 10_000);
        assert_eq!(alice.supply(&currency).unwrap(), (10_000, 0, 10_000));
        assert_eq!(
            alice.matching_payments(&currency, 1, None).unwrap().len(),
            0
        );

        let mut bob = GraphCurrencyClient::new(key(9));
        for event in alice.events().iter().cloned() {
            bob.observe(event.clone(), alice.bodies().get(&event.id).cloned());
        }
        let payment = bob
            .pay(
                &currency,
                issuer_id(&key(7)),
                2_500,
                Some("coffee".to_owned()),
            )
            .unwrap();
        assert_eq!(payment.outcome, FoldOutcome::Applied);
        assert_eq!(bob.balance(&currency, &bob_id).unwrap(), 7_500);
        assert_eq!(bob.balance(&currency, &issuer_id(&key(7))).unwrap(), 2_500);
        assert_eq!(
            bob.matching_payments(&currency, 2_500, Some("coffee"))
                .unwrap(),
            vec![payment.event_id]
        );

        let payment_event = bob.events().last().unwrap().clone();
        alice.observe(
            payment_event.clone(),
            bob.bodies().get(&payment_event.id).cloned(),
        );
        assert_eq!(alice.balance(&currency, &bob_id).unwrap(), 7_500);
        assert_eq!(
            alice.balance(&currency, &issuer_id(&key(7))).unwrap(),
            2_500
        );
        assert_eq!(
            alice
                .matching_payments(&currency, 2_500, Some("coffee"))
                .unwrap(),
            vec![payment.event_id]
        );

        let snapshot = alice.snapshot(&currency).unwrap();
        assert_eq!(snapshot.issued, 10_000);
        assert_eq!(snapshot.frontier.len(), 2);
    }

    #[test]
    fn authoring_refuses_a_fork_at_any_observed_author_sequence() {
        let issuer = key(7);
        let mut client = GraphCurrencyClient::new(issuer);
        let currency = client
            .define_currency(
                "season",
                "S",
                0,
                SupplyCap { capped: 100 },
                CurrencyBinding::Graph,
            )
            .unwrap();
        let fork_definition = CurrencyDefinition {
            name: "other season".to_owned(),
            symbol: "OS".to_owned(),
            scale: 0,
            supply: SupplyCap { capped: 100 },
            binding: CurrencyBinding::Graph,
            memo: None,
        };
        let (_, fork) = sign_definition(&key(7), 0, None, &fork_definition, vec![]).unwrap();
        client.observe(fork.clone(), Some(fork_definition.body().unwrap()));

        assert!(matches!(
            client.issue(&currency, issuer_id(&key(9)), 1, None),
            Err(ClientError::UnresolvedFork)
        ));
    }

    #[test]
    fn frontier_refuses_a_non_local_author_fork() {
        let mut client = GraphCurrencyClient::new(key(7));
        let first = CurrencyDefinition {
            name: "first".to_owned(),
            symbol: "F".to_owned(),
            scale: 0,
            supply: SupplyCap { capped: 100 },
            binding: CurrencyBinding::Graph,
            memo: None,
        };
        let second = CurrencyDefinition {
            name: "second".to_owned(),
            symbol: "S".to_owned(),
            ..first.clone()
        };
        let (_, first_event) = sign_definition(&key(8), 0, None, &first, vec![]).unwrap();
        let (_, second_event) = sign_definition(&key(8), 0, None, &second, vec![]).unwrap();
        client.observe(first_event.clone(), Some(first.body().unwrap()));
        client.observe(second_event.clone(), Some(second.body().unwrap()));

        assert!(matches!(
            client.define_currency(
                "local",
                "L",
                0,
                SupplyCap { capped: 10 },
                CurrencyBinding::Graph,
            ),
            Err(ClientError::UnresolvedFork)
        ));
    }

    #[test]
    fn over_cap_issue_is_visible_as_a_void_intent() {
        let mut alice = GraphCurrencyClient::new(key(7));
        let currency = alice
            .define_currency(
                "season",
                "S",
                0,
                SupplyCap { capped: 4 },
                CurrencyBinding::Graph,
            )
            .unwrap();
        assert_eq!(
            alice
                .issue(&currency, issuer_id(&key(8)), 5, None)
                .unwrap()
                .outcome,
            FoldOutcome::Invalid(crate::currency::fold::InvalidReason::OverCap)
        );
        assert_eq!(alice.supply(&currency).unwrap(), (0, 0, 0));
    }

    #[test]
    fn burn_reduces_holder_balance_and_counted_supply() {
        let mut alice = GraphCurrencyClient::new(key(7));
        let alice_id = issuer_id(&key(7));
        let currency = alice
            .define_currency(
                "season",
                "S",
                0,
                SupplyCap { capped: 100 },
                CurrencyBinding::Graph,
            )
            .unwrap();
        assert_eq!(
            alice.issue(&currency, alice_id, 10, None).unwrap().outcome,
            FoldOutcome::Applied
        );

        assert_eq!(
            alice.burn(&currency, 4).unwrap().outcome,
            FoldOutcome::Applied
        );
        assert_eq!(alice.balance(&currency, &alice_id).unwrap(), 6);
        assert_eq!(alice.supply(&currency).unwrap(), (10, 4, 6));
    }

    #[test]
    fn burn_returns_fold_verdict_for_insufficient_funds_and_rejects_ledger_currency() {
        let mut client = GraphCurrencyClient::new(key(7));
        let graph_currency = client
            .define_currency(
                "season",
                "S",
                0,
                SupplyCap { capped: 100 },
                CurrencyBinding::Graph,
            )
            .unwrap();
        assert_eq!(
            client.burn(&graph_currency, 1).unwrap().outcome,
            FoldOutcome::Invalid(crate::currency::fold::InvalidReason::InsufficientFunds)
        );

        let ledger_currency = client
            .define_currency(
                "anchored",
                "A",
                0,
                SupplyCap { capped: 100 },
                CurrencyBinding::Ledger { ledger: 42 },
            )
            .unwrap();
        assert!(matches!(
            client.burn(&ledger_currency, 1),
            Err(ClientError::NotGraphBound)
        ));
    }

    #[test]
    fn non_issuer_cannot_author_an_issuance() {
        let mut issuer = GraphCurrencyClient::new(key(7));
        let currency = issuer
            .define_currency(
                "season",
                "S",
                0,
                SupplyCap { capped: 100 },
                CurrencyBinding::Graph,
            )
            .unwrap();
        let mut holder = GraphCurrencyClient::new(key(9));
        for (event, body) in issuer.events().iter().cloned().map(|event| {
            let body = issuer.bodies().get(&event.id).cloned();
            (event, body)
        }) {
            holder.observe(event, body);
        }

        assert!(matches!(
            holder.issue(&currency, issuer_id(&key(9)), 1, None),
            Err(ClientError::NotIssuer)
        ));
        assert_eq!(holder.events().len(), 1);
    }

    #[test]
    fn game_prize_pool_awards_and_payment_matching_work_end_to_end() {
        // Oracle: #2041 §9 — issue once to the pool, award the winner with
        // match provenance, and make the applied award discoverable as a payment.
        let mut game = GraphCurrencyClient::new(key(7));
        let pool = issuer_id(&key(7));
        let winner = issuer_id(&key(9));
        let currency = game
            .define_currency(
                "season-coins",
                "SC",
                0,
                SupplyCap { capped: 1_000_000 },
                CurrencyBinding::Graph,
            )
            .unwrap();

        assert_eq!(
            game.issue(&currency, pool, 1_000_000, Some("season pool".to_owned()))
                .unwrap()
                .outcome,
            FoldOutcome::Applied
        );
        let award = game
            .award(&currency, winner, 250, "match:example-42".to_owned())
            .unwrap();

        assert_eq!(award.outcome, FoldOutcome::Applied);
        assert_eq!(game.balance(&currency, &pool).unwrap(), 999_750);
        assert_eq!(game.balance(&currency, &winner).unwrap(), 250);
        assert_eq!(game.supply(&currency).unwrap(), (1_000_000, 0, 1_000_000));
        assert_eq!(
            game.matching_payments(&currency, 250, Some("example-42"))
                .unwrap(),
            vec![award.event_id]
        );
    }

    #[test]
    fn definition_preserves_binding_and_graph_operations_reject_ledger_currency() {
        // Oracle: #2251 §4 — define_currency accepts a binding; graph-only
        // mutation must not pretend to execute a ledger-bound currency.
        let mut client = GraphCurrencyClient::new(key(7));
        let currency = client
            .define_currency(
                "anchored",
                "A",
                2,
                SupplyCap { capped: 1_000 },
                CurrencyBinding::Ledger { ledger: 42 },
            )
            .unwrap();

        assert!(matches!(
            client.issue(&currency, issuer_id(&key(9)), 1, None),
            Err(ClientError::NotGraphBound)
        ));
    }

    #[test]
    fn issue_and_pay_reject_unknown_currency_ids() {
        // Oracle: #2041 §4 — issue and pay report `UnknownCurrency` when the
        // supplied identity has no definition in the client's observed graph.
        let mut client = GraphCurrencyClient::new(key(7));
        let unknown = CurrencyId::from_definition(
            issuer_id(&key(8)),
            &CurrencyDefinition {
                name: "not observed".to_owned(),
                symbol: "NO".to_owned(),
                scale: 0,
                supply: SupplyCap { capped: 100 },
                binding: CurrencyBinding::Graph,
                memo: None,
            },
        )
        .unwrap();

        assert!(matches!(
            client.issue(&unknown, issuer_id(&key(9)), 1, None),
            Err(ClientError::UnknownCurrency)
        ));
        assert!(matches!(
            client.pay(&unknown, issuer_id(&key(9)), 1, None),
            Err(ClientError::UnknownCurrency)
        ));
        assert!(client.events().is_empty());
    }
}
