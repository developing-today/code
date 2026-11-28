//! Issuer snapshots of a graph-native currency (#10070 item 6; design
//! #2043 invariant 11, wire and root construction from the #2741 spec,
//! `doc/2026-10-30T13-25-00Z_design_graph_token_formats_snapshot_anchoring.md`
//! §1.7 and §3).
//!
//! A snapshot is a signed, frontier-named assertion about one currency,
//! not a trust root: it names the per-author frontier the fold covered,
//! the counted issued and burned totals at that frontier, and a
//! `balance_root` over every non-zero holder balance. Anyone holding the
//! events up to the frontier recomputes all three and compares, so a
//! lying snapshot is provable by recomputation ([`verify_snapshot`]).
//!
//! **Balance root.** Leaves are `BLAKE3(holder ‖ balance_u128_le ‖
//! currency)` over the non-zero holders sorted by key bytes, where
//! `currency` is the compact currency hash, so a root can never be
//! replayed across currencies. Parents are `BLAKE3(0x01 ‖ left ‖
//! right)`, pairing left to right; an odd last node is promoted unchanged;
//! a single leaf is the root; an empty tree's root is `BLAKE3("")`.
//!
//! **Frontier.** One entry per author whose events the fold decided, in
//! ascending key order, with the highest decided sequence number and that
//! event's id. Pending events are not covered, so the frontier is
//! causally closed: everything a covered event depends on is covered too.
//!
//! **Signer.** The issuer key active at the snapshot's frontier signs it:
//! the generation-0 key, or after a rotation the key the issuer's
//! generation chain names, folded from the rotation events the frontier
//! covers (design #2804 §3.3). A retired key may still sign a snapshot of a
//! frontier before its rotation. While the chain is forked no key speaks
//! for the issuer, so no snapshot verifies.
//!
//! **Scope.** Graph-native currencies only, signed by the issuer. The
//! anchored fields of the spec (`ledger_pos`, `ledger_accounts_root`) and
//! the reconciliation join belong to the anchored gateway phase and are
//! not part of this body; an anchored currency has no graph-native
//! snapshot ([`SnapshotError::Anchored`]). Cadence (per batch, on demand)
//! is the caller's choice.

use std::collections::{BTreeMap, BTreeSet};

use p2panda_core::{Hash, SeqNum, SigningKey};
use serde::{Deserialize, Serialize};

use crate::chain::Event;
use crate::currency::fold::{CurrencyFold, FoldOutcome, fold_events};
use crate::currency::graph::{
    BodyError, CurrencyBinding, CurrencyId, TokenError, canonical_body, decode_body, key_bytes,
    sign_body,
};
use crate::currency::{IssuerId, issuer_id};
use crate::issuer_rotation::ChainStatus;

/// Event kind carrying an issuer snapshot.
pub const SNAPSHOT_KIND: &str = "token.snapshot";

/// Domain byte that separates internal nodes from leaves.
const NODE_DOMAIN: u8 = 0x01;

/// One author's covered position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontierEntry {
    /// The author's verifying key.
    #[serde(with = "key_bytes")]
    pub author: IssuerId,
    /// The highest covered sequence number.
    pub seq: u64,
    /// The id of the event at that sequence number.
    pub head: Hash,
}

/// The signed snapshot body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenSnapshot {
    /// Compact currency id.
    pub currency: Hash,
    /// Covered frontier, strictly ascending by author.
    pub frontier: Vec<FrontierEntry>,
    /// Counted issuance at the frontier.
    pub issued: u128,
    /// Counted burns at the frontier.
    pub burned: u128,
    /// Merkle root over non-zero holder balances.
    pub balance_root: Hash,
}

impl TokenSnapshot {
    fn validate(&self) -> Result<(), BodyError> {
        if self.frontier.is_empty() {
            return Err(BodyError::Schema("snapshot frontier is empty".to_owned()));
        }
        if self
            .frontier
            .windows(2)
            .any(|pair| pair[0].author >= pair[1].author)
        {
            return Err(BodyError::Schema(
                "snapshot frontier is not strictly ascending by author".to_owned(),
            ));
        }
        if self.burned > self.issued {
            return Err(BodyError::Schema(
                "snapshot burns exceed issuance".to_owned(),
            ));
        }
        Ok(())
    }

    /// Canonical CBOR body.
    ///
    /// # Errors
    ///
    /// An empty or unsorted frontier, burns above issuance, or an encoding
    /// failure.
    pub fn body(&self) -> Result<Vec<u8>, TokenError> {
        self.validate()?;
        Ok(canonical_body(self)?)
    }

    /// Decode a strict canonical-CBOR body and validate it.
    ///
    /// # Errors
    ///
    /// Any [`BodyError`], or the checks [`TokenSnapshot::body`] makes.
    pub fn from_body(bytes: &[u8]) -> Result<Self, TokenError> {
        let snapshot: Self = decode_body(bytes)?;
        snapshot.validate()?;
        Ok(snapshot)
    }
}

/// The balance root over `holders` (zero balances are skipped; order and
/// duplicates of the input do not matter, the last balance for a holder
/// wins).
#[must_use]
pub fn balance_root(currency: &Hash, holders: &[(IssuerId, u128)]) -> Hash {
    let sorted: BTreeMap<IssuerId, u128> = holders.iter().copied().collect();
    let mut level: Vec<Hash> = sorted
        .iter()
        .filter(|(_, balance)| **balance > 0)
        .map(|(holder, balance)| {
            let mut leaf = Vec::with_capacity(32 + 16 + 32);
            leaf.extend_from_slice(holder);
            leaf.extend_from_slice(&balance.to_le_bytes());
            leaf.extend_from_slice(currency.as_bytes());
            Hash::digest(&leaf)
        })
        .collect();
    if level.is_empty() {
        return Hash::digest([]);
    }
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|pair| match pair {
                [left, right] => {
                    let mut node = Vec::with_capacity(1 + 64);
                    node.push(NODE_DOMAIN);
                    node.extend_from_slice(left.as_bytes());
                    node.extend_from_slice(right.as_bytes());
                    Hash::digest(&node)
                }
                [odd] => *odd,
                _ => unreachable!("chunks(2) yields one or two nodes"),
            })
            .collect();
    }
    level[0]
}

/// Why a snapshot could not be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// The fold has no definition for the currency.
    UnknownCurrency,
    /// The currency is ledger-bound; its balances are the ledger's.
    Anchored,
}

/// Take a snapshot of `currency` at the fold's decided frontier.
///
/// # Errors
///
/// [`SnapshotError`].
pub fn snapshot(fold: &CurrencyFold, currency: &Hash) -> Result<TokenSnapshot, SnapshotError> {
    let (_, definition) = fold
        .definition(currency)
        .ok_or(SnapshotError::UnknownCurrency)?;
    if matches!(definition.binding, CurrencyBinding::Ledger { .. }) {
        return Err(SnapshotError::Anchored);
    }
    Ok(TokenSnapshot {
        currency: *currency,
        frontier: fold
            .frontier()
            .iter()
            .map(|(author, (seq, head))| FrontierEntry {
                author: *author,
                seq: *seq,
                head: *head,
            })
            .collect(),
        issued: fold.issued(currency),
        burned: fold.burned(currency),
        balance_root: balance_root(currency, &fold.holders(currency)),
    })
}

/// Sign a snapshot as the currency's issuer.
///
/// # Errors
///
/// [`TokenError::IssuerMismatch`] for any other key,
/// [`TokenError::CurrencyMismatch`] when the body names another currency,
/// or a body or chain error.
pub fn sign_snapshot(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    currency: &CurrencyId,
    snapshot: &TokenSnapshot,
    after: Vec<Hash>,
) -> Result<Event, TokenError> {
    if issuer_id(key) != currency.issuer() {
        return Err(TokenError::IssuerMismatch);
    }
    if snapshot.currency != currency.compact_hash() {
        return Err(TokenError::CurrencyMismatch);
    }
    let body = snapshot.body()?;
    sign_body(key, seq_num, backlink, &body, SNAPSHOT_KIND, after)
}

/// Sign a snapshot with a rotated-in issuer generation key.
///
/// [`sign_snapshot`] signs only with the generation-0 key. A later
/// generation's key signs here; [`verify_snapshot`] decides whether that
/// key was the active one at the snapshot's frontier.
///
/// # Errors
///
/// [`TokenError::CurrencyMismatch`] when the body names another currency,
/// or a body or chain error.
pub fn sign_generation_snapshot(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    currency: &CurrencyId,
    snapshot: &TokenSnapshot,
    after: Vec<Hash>,
) -> Result<Event, TokenError> {
    if snapshot.currency != currency.compact_hash() {
        return Err(TokenError::CurrencyMismatch);
    }
    let body = snapshot.body()?;
    sign_body(key, seq_num, backlink, &body, SNAPSHOT_KIND, after)
}

/// What recomputing a snapshot found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotFinding {
    /// The event is not a snapshot.
    NotSnapshot,
    /// The body is missing, does not match the signed hash, or is malformed.
    BadBody,
    /// The events within the frontier define no such graph-native currency.
    UnknownCurrency,
    /// The snapshot is signed by a key other than the issuer key active at
    /// its frontier, or the issuer's generation chain is forked there.
    NotIssuer,
    /// A frontier head is not among the supplied events.
    HeadMissing,
    /// The supplied event at a frontier position is not the named head.
    HeadMismatch,
    /// A covered event depends on a supplied event outside the frontier:
    /// the frontier is not causally closed, so it is not a fold frontier.
    NotClosed,
    /// Covered events or their bodies are missing, so the snapshot cannot
    /// be recomputed from what was supplied. Not evidence against it.
    Incomplete,
    /// The recomputed frontier differs: the claimed head is not the one
    /// the fold reaches, as on a fork at that sequence number.
    FrontierMismatch,
    /// The recomputed issuance differs.
    IssuedMismatch,
    /// The recomputed burns differ.
    BurnedMismatch,
    /// The recomputed balance root differs.
    RootMismatch,
}

fn dependencies(event: &Event) -> impl Iterator<Item = &Hash> {
    event
        .header
        .backlink
        .iter()
        .chain(event.header.extensions.after.iter())
}

/// Recompute a signed snapshot from `events` and `bodies` (a superset of
/// what it covers is fine) and report the first disagreement.
///
/// Only events whose author is in the frontier, at or below that
/// author's covered sequence number, are folded.
///
/// # Errors
///
/// The first [`SnapshotFinding`], checked in the order the variants are
/// declared.
pub fn verify_snapshot(
    snapshot_event: &Event,
    snapshot_body: Option<&[u8]>,
    events: &[Event],
    bodies: &BTreeMap<Hash, Vec<u8>>,
) -> Result<TokenSnapshot, SnapshotFinding> {
    if snapshot_event.header.extensions.kind != SNAPSHOT_KIND {
        return Err(SnapshotFinding::NotSnapshot);
    }
    let body = snapshot_body
        .filter(|body| snapshot_event.header.payload_hash == Some(Hash::digest(body)))
        .ok_or(SnapshotFinding::BadBody)?;
    let claimed = TokenSnapshot::from_body(body).map_err(|_body| SnapshotFinding::BadBody)?;

    let bound: BTreeMap<IssuerId, (u64, Hash)> = claimed
        .frontier
        .iter()
        .map(|entry| (entry.author, (entry.seq, entry.head)))
        .collect();
    let all: BTreeMap<Hash, &Event> = events.iter().map(|event| (event.id, event)).collect();
    let covered: Vec<Event> = all
        .values()
        .filter(|event| {
            bound
                .get(event.header.verifying_key.as_bytes())
                .is_some_and(|(seq, _)| u64::from(event.header.seq_num) <= *seq)
        })
        .map(|event| (*event).clone())
        .collect();
    let covered_ids: BTreeSet<Hash> = covered.iter().map(|event| event.id).collect();

    let fold = fold_events(&covered, bodies);
    let Some((currency, definition)) = fold.definition(&claimed.currency) else {
        return Err(SnapshotFinding::UnknownCurrency);
    };
    if matches!(definition.binding, CurrencyBinding::Ledger { .. }) {
        return Err(SnapshotFinding::UnknownCurrency);
    }
    let signer = *snapshot_event.header.verifying_key.as_bytes();
    let speaks = fold.generation_chain(&claimed.currency).map_or_else(
        || signer == currency.issuer(),
        |chain| chain.status == ChainStatus::Settled && chain.is_active_key(&signer),
    );
    if !speaks {
        return Err(SnapshotFinding::NotIssuer);
    }
    for (author, (seq, head)) in &bound {
        let mut at = covered.iter().filter(|event| {
            event.header.verifying_key.as_bytes() == author
                && u64::from(event.header.seq_num) == *seq
        });
        if !at.any(|event| event.id == *head) {
            return Err(
                if all.contains_key(head)
                    || covered.iter().any(|event| {
                        event.header.verifying_key.as_bytes() == author
                            && u64::from(event.header.seq_num) == *seq
                    })
                {
                    SnapshotFinding::HeadMismatch
                } else {
                    SnapshotFinding::HeadMissing
                },
            );
        }
    }
    let pending: Vec<&Event> = covered
        .iter()
        .filter(|event| fold.outcome(&event.id) == Some(FoldOutcome::Pending))
        .collect();
    if pending.iter().any(|event| {
        dependencies(event).any(|dep| all.contains_key(dep) && !covered_ids.contains(dep))
    }) {
        return Err(SnapshotFinding::NotClosed);
    }
    if !pending.is_empty() {
        return Err(SnapshotFinding::Incomplete);
    }
    let recomputed =
        snapshot(&fold, &claimed.currency).map_err(|_unknown| SnapshotFinding::UnknownCurrency)?;
    if recomputed.frontier != claimed.frontier {
        return Err(SnapshotFinding::FrontierMismatch);
    }
    if recomputed.issued != claimed.issued {
        return Err(SnapshotFinding::IssuedMismatch);
    }
    if recomputed.burned != claimed.burned {
        return Err(SnapshotFinding::BurnedMismatch);
    }
    if recomputed.balance_root != claimed.balance_root {
        return Err(SnapshotFinding::RootMismatch);
    }
    Ok(claimed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::currency::graph::{
        CurrencyDefinition, SupplyCap, TokenBurn, TokenIssue, TokenTransfer, sign_burn,
        sign_definition, sign_issue, sign_transfer,
    };

    fn hex(hash: &Hash) -> String {
        hash.as_bytes().iter().fold(String::new(), |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        })
    }

    const fn holder(n: u8) -> IssuerId {
        [n; 32]
    }

    // Golden roots computed independently with Python `blake3` over the
    // construction in the module docs; currency bytes are 0x00..=0x1f.
    #[test]
    fn balance_root_matches_independent_vectors() {
        let currency = Hash::from_bytes({
            let mut b = [0u8; 32];
            for (i, x) in b.iter_mut().enumerate() {
                *x = u8::try_from(i).unwrap();
            }
            b
        });
        let six = [
            (holder(6), 2),
            (holder(1), 5),
            (holder(2), 0),
            (holder(4), u128::MAX),
            (holder(3), 7),
            (holder(5), 1),
        ];
        assert_eq!(
            hex(&balance_root(&currency, &six)),
            "079be5b1feb8a06811fdde3609962c4ec4d5a1eafe6454e7a05c09c462bd60b8"
        );
        assert_eq!(
            hex(&balance_root(&currency, &[(holder(9), 42)])),
            "988f4dc066b29795796b63f4cdf88e849608212d28f1f7c6f026bcfe09737d3a"
        );
        assert_eq!(
            hex(&balance_root(&currency, &[])),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            hex(&balance_root(&currency, &[(holder(2), 0)])),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            hex(&balance_root(&currency, &[(holder(3), 7), (holder(1), 5)])),
            "b1a1c40150129f4c7f1e6d92d535b1878e5f171c46f0f2a3e28e21859c2e0acb"
        );
        // The currency is in every leaf, so a root cannot cross currencies.
        assert_ne!(
            balance_root(&currency, &six),
            balance_root(&Hash::digest(b"other"), &six)
        );
    }

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn definition(binding: CurrencyBinding) -> CurrencyDefinition {
        CurrencyDefinition {
            name: "Tokens".to_owned(),
            symbol: "TST".to_owned(),
            scale: 2,
            supply: SupplyCap { capped: 1_000 },
            binding,
            memo: None,
        }
    }

    /// issuer: define, issue 100 to alice; alice: send 30 to bob;
    /// bob: burn 10.
    struct World {
        currency: CurrencyId,
        events: Vec<Event>,
        bodies: BTreeMap<Hash, Vec<u8>>,
    }

    impl World {
        fn id(&self, n: usize) -> Hash {
            self.events[n].id
        }
    }

    fn world() -> World {
        let (issuer, alice, bob) = (key(7), key(9), key(11));
        let def = definition(CurrencyBinding::Graph);
        let (currency, define) = sign_definition(&issuer, 0, None, &def, vec![]).unwrap();
        let c = currency.compact_hash();
        let issue_body = TokenIssue {
            currency: c,
            amount: 100,
            to: issuer_id(&alice),
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
        let send_body = TokenTransfer {
            currency: c,
            amount: 30,
            to: issuer_id(&bob),
            seq_ref: 0,
            memo: None,
        };
        let send = sign_transfer(&alice, 0, None, &send_body, vec![issue.id]).unwrap();
        let burn_body = TokenBurn {
            currency: c,
            amount: 10,
            seq_ref: 0,
        };
        let burn = sign_burn(&bob, 0, None, &burn_body, vec![send.id]).unwrap();
        let bodies = BTreeMap::from([
            (define.id, def.body().unwrap()),
            (issue.id, issue_body.body().unwrap()),
            (send.id, send_body.body().unwrap()),
            (burn.id, burn_body.body().unwrap()),
        ]);
        World {
            currency,
            events: vec![define, issue, send, burn],
            bodies,
        }
    }

    fn signed(world: &World, snap: &TokenSnapshot) -> (Event, Vec<u8>) {
        let event = sign_snapshot(
            &key(7),
            2,
            Some(world.id(1)),
            &world.currency,
            snap,
            vec![world.id(3)],
        )
        .unwrap();
        (event, snap.body().unwrap())
    }

    fn check(world: &World, snap: &TokenSnapshot) -> Result<TokenSnapshot, SnapshotFinding> {
        let (event, body) = signed(world, snap);
        verify_snapshot(&event, Some(&body), &world.events, &world.bodies)
    }

    #[test]
    fn a_snapshot_names_the_frontier_counts_and_root() {
        let w = world();
        let c = w.currency.compact_hash();
        let fold = fold_events(&w.events, &w.bodies);
        let snap = snapshot(&fold, &c).unwrap();
        assert_eq!(snap.issued, 100);
        assert_eq!(snap.burned, 10);
        assert_eq!(fold.supply(&c), 90);
        let mut holders = vec![(issuer_id(&key(9)), 70), (issuer_id(&key(11)), 20)];
        holders.sort_unstable();
        assert_eq!(fold.holders(&c), holders);
        assert_eq!(
            snap.balance_root,
            balance_root(&c, &[(issuer_id(&key(9)), 70), (issuer_id(&key(11)), 20)])
        );
        let mut want = vec![
            FrontierEntry {
                author: issuer_id(&key(7)),
                seq: 1,
                head: w.id(1),
            },
            FrontierEntry {
                author: issuer_id(&key(9)),
                seq: 0,
                head: w.id(2),
            },
            FrontierEntry {
                author: issuer_id(&key(11)),
                seq: 0,
                head: w.id(3),
            },
        ];
        want.sort_by_key(|e| e.author);
        assert_eq!(snap.frontier, want);

        assert_eq!(check(&w, &snap), Ok(snap.clone()));

        // A snapshot is not a value event: folding it changes nothing.
        let (event, body) = signed(&w, &snap);
        let mut events = w.events.clone();
        events.push(event.clone());
        let mut bodies = w.bodies;
        bodies.insert(event.id, body);
        let after = fold_events(&events, &bodies);
        assert_eq!(after.outcome(&event.id), Some(FoldOutcome::Ignored));
        assert_eq!(after.holders(&c), fold.holders(&c));
    }

    #[test]
    fn pending_events_are_not_covered() {
        let mut w = world();
        let burn = w.id(3);
        w.bodies.remove(&burn);
        let fold = fold_events(&w.events, &w.bodies);
        let c = w.currency.compact_hash();
        let snap = snapshot(&fold, &c).unwrap();
        assert_eq!(snap.burned, 0);
        assert!(
            !snap
                .frontier
                .iter()
                .any(|e| e.author == issuer_id(&key(11)))
        );
        // Without the burn's body, a snapshot claiming it cannot be checked.
        let full = snapshot(&fold_events(&world().events, &world().bodies), &c).unwrap();
        assert_eq!(check(&w, &full), Err(SnapshotFinding::Incomplete));
        // The partial one checks out against the same material.
        assert_eq!(check(&w, &snap), Ok(snap));
    }

    #[test]
    fn a_lying_snapshot_is_found_by_recomputation() {
        let w = world();
        let c = w.currency.compact_hash();
        let honest = snapshot(&fold_events(&w.events, &w.bodies), &c).unwrap();

        let mut s = honest.clone();
        s.issued = 101;
        assert_eq!(check(&w, &s), Err(SnapshotFinding::IssuedMismatch));
        let mut s = honest.clone();
        s.burned = 9;
        assert_eq!(check(&w, &s), Err(SnapshotFinding::BurnedMismatch));
        let mut s = honest.clone();
        s.balance_root = balance_root(&c, &[(issuer_id(&key(9)), 90)]);
        assert_eq!(check(&w, &s), Err(SnapshotFinding::RootMismatch));

        // Leaving bob out folds without his burn: the burn count is a lie.
        let mut s = honest.clone();
        s.frontier.retain(|e| e.author != issuer_id(&key(11)));
        assert_eq!(check(&w, &s), Err(SnapshotFinding::BurnedMismatch));

        // A head that names another event, or no supplied event.
        let mut s = honest.clone();
        let bob = s
            .frontier
            .iter_mut()
            .find(|e| e.author == issuer_id(&key(11)))
            .unwrap();
        bob.head = w.id(2);
        assert_eq!(check(&w, &s), Err(SnapshotFinding::HeadMismatch));
        let mut s = honest.clone();
        let bob = s
            .frontier
            .iter_mut()
            .find(|e| e.author == issuer_id(&key(11)))
            .unwrap();
        bob.seq = 1;
        bob.head = Hash::digest(b"nowhere");
        assert_eq!(check(&w, &s), Err(SnapshotFinding::HeadMissing));
        // A head that names a supplied event at a sequence number where the
        // author has none is still a mismatch, not a missing head.
        let mut s = honest.clone();
        let issuer = s
            .frontier
            .iter_mut()
            .find(|e| e.author == issuer_id(&key(7)))
            .unwrap();
        issuer.seq = 5;
        issuer.head = w.id(1);
        assert_eq!(check(&w, &s), Err(SnapshotFinding::HeadMismatch));

        // Cutting the issuer back to the definition leaves alice's spend
        // depending on a supplied event outside the frontier.
        let mut s = honest;
        let issuer = s
            .frontier
            .iter_mut()
            .find(|e| e.author == issuer_id(&key(7)))
            .unwrap();
        issuer.seq = 0;
        issuer.head = w.id(0);
        assert_eq!(check(&w, &s), Err(SnapshotFinding::NotClosed));
    }

    #[test]
    fn a_dependency_nobody_supplied_is_incomplete_not_unclosed() {
        let full = world();
        let c = full.currency.compact_hash();
        let mut s = snapshot(&fold_events(&full.events, &full.bodies), &c).unwrap();
        let issuer = s
            .frontier
            .iter_mut()
            .find(|e| e.author == issuer_id(&key(7)))
            .unwrap();
        issuer.seq = 0;
        issuer.head = full.id(0);
        // Without the issue at all, alice's spend waits on an event nobody
        // supplied: the material is incomplete, the frontier is not open.
        let mut w = world();
        let issue = w.events.remove(1);
        w.bodies.remove(&issue.id);
        let event = sign_snapshot(&key(7), 2, Some(full.id(1)), &w.currency, &s, vec![]).unwrap();
        let body = s.body().unwrap();
        assert_eq!(
            verify_snapshot(&event, Some(&body), &w.events, &w.bodies),
            Err(SnapshotFinding::Incomplete)
        );
    }

    #[test]
    fn a_fork_takes_the_larger_head_and_a_spent_holder_drops_out() {
        let mut w = world();
        let c = w.currency.compact_hash();
        let rest = TokenBurn {
            currency: c,
            amount: 20,
            seq_ref: 0,
        };
        let fork = sign_burn(&key(11), 0, None, &rest, vec![w.id(2)]).unwrap();
        w.bodies.insert(fork.id, rest.body().unwrap());
        w.events.push(fork.clone());

        let fold = fold_events(&w.events, &w.bodies);
        assert_eq!(fold.outcome(&fork.id), Some(FoldOutcome::Applied));
        assert_eq!(fold.outcome(&w.id(3)), Some(FoldOutcome::Applied));
        // Bob spent everything: he is no longer a holder.
        assert_eq!(fold.holders(&c), vec![(issuer_id(&key(9)), 70)]);

        // Bob forked at sequence 0; the head is the larger id, whatever
        // order the events arrive in.
        let larger = fork.id.max(w.id(3));
        let smaller = fork.id.min(w.id(3));
        let mut reversed = w.events.clone();
        reversed.reverse();
        for events in [&w.events, &reversed] {
            let snap = snapshot(&fold_events(events, &w.bodies), &c).unwrap();
            let bob = snap
                .frontier
                .iter()
                .find(|e| e.author == issuer_id(&key(11)))
                .unwrap();
            assert_eq!((bob.seq, bob.head), (0, larger));
        }

        // Naming the other fork as head passes the head check (an event
        // is there) but not the recomputed frontier.
        let mut s = snapshot(&fold, &c).unwrap();
        s.frontier
            .iter_mut()
            .find(|e| e.author == issuer_id(&key(11)))
            .unwrap()
            .head = smaller;
        let event = sign_snapshot(&key(7), 2, Some(w.id(1)), &w.currency, &s, vec![]).unwrap();
        let body = s.body().unwrap();
        assert_eq!(
            verify_snapshot(&event, Some(&body), &w.events, &w.bodies),
            Err(SnapshotFinding::FrontierMismatch)
        );
    }

    #[test]
    fn only_the_issuer_signs_and_only_a_matching_body_counts() {
        let w = world();
        let c = w.currency.compact_hash();
        let honest = snapshot(&fold_events(&w.events, &w.bodies), &c).unwrap();
        assert!(matches!(
            sign_snapshot(&key(9), 1, Some(w.id(2)), &w.currency, &honest, vec![]),
            Err(TokenError::IssuerMismatch)
        ));
        let mut other = honest.clone();
        other.currency = Hash::digest(b"other");
        assert!(matches!(
            sign_snapshot(&key(7), 2, Some(w.id(1)), &w.currency, &other, vec![]),
            Err(TokenError::CurrencyMismatch)
        ));

        // Alice signs a snapshot body directly: it is not the issuer's.
        let body = honest.body().unwrap();
        let forged = sign_body(&key(9), 1, Some(w.id(2)), &body, SNAPSHOT_KIND, vec![]).unwrap();
        assert_eq!(
            verify_snapshot(&forged, Some(&body), &w.events, &w.bodies),
            Err(SnapshotFinding::NotIssuer)
        );

        let (event, body) = signed(&w, &honest);
        assert_eq!(
            verify_snapshot(&event, None, &w.events, &w.bodies),
            Err(SnapshotFinding::BadBody)
        );
        let other_body = TokenSnapshot {
            issued: 500,
            ..honest.clone()
        }
        .body()
        .unwrap();
        assert_eq!(
            verify_snapshot(&event, Some(&other_body), &w.events, &w.bodies),
            Err(SnapshotFinding::BadBody)
        );
        assert_eq!(
            verify_snapshot(&w.events[1], Some(&body), &w.events, &w.bodies),
            Err(SnapshotFinding::NotSnapshot)
        );
        // A snapshot for a currency the frontier does not define.
        let mut s = honest;
        s.currency = Hash::digest(b"other");
        let body = s.body().unwrap();
        let event = sign_body(&key(7), 2, Some(w.id(1)), &body, SNAPSHOT_KIND, vec![]).unwrap();
        assert_eq!(
            verify_snapshot(&event, Some(&body), &w.events, &w.bodies),
            Err(SnapshotFinding::UnknownCurrency)
        );
    }

    #[test]
    fn bodies_are_canonical_and_well_formed() {
        let w = world();
        let c = w.currency.compact_hash();
        let honest = snapshot(&fold_events(&w.events, &w.bodies), &c).unwrap();
        let body = honest.body().unwrap();
        assert_eq!(TokenSnapshot::from_body(&body).unwrap(), honest);

        let mut s = honest.clone();
        s.frontier.reverse();
        assert!(s.body().is_err());
        let unsorted = canonical_body(&s).unwrap();
        assert!(TokenSnapshot::from_body(&unsorted).is_err());
        let mut s = honest.clone();
        s.frontier.push(s.frontier[2]);
        assert!(s.body().is_err());
        let mut s = honest.clone();
        s.frontier.clear();
        assert!(s.body().is_err());
        let mut s = honest;
        s.burned = s.issued + 1;
        assert!(s.body().is_err());
    }

    #[test]
    fn anchored_and_unknown_currencies_have_no_graph_snapshot() {
        let (currency, define) = sign_definition(
            &key(7),
            0,
            None,
            &definition(CurrencyBinding::Ledger { ledger: 1 }),
            vec![],
        )
        .unwrap();
        let bodies = BTreeMap::from([(
            define.id,
            definition(CurrencyBinding::Ledger { ledger: 1 })
                .body()
                .unwrap(),
        )]);
        let fold = fold_events(std::slice::from_ref(&define), &bodies);
        assert_eq!(
            snapshot(&fold, &currency.compact_hash()),
            Err(SnapshotError::Anchored)
        );
        assert_eq!(
            snapshot(&fold, &Hash::digest(b"none")),
            Err(SnapshotError::UnknownCurrency)
        );

        // A graph snapshot of an anchored currency is refused before the
        // signer is looked at.
        let claimed = TokenSnapshot {
            currency: currency.compact_hash(),
            frontier: vec![FrontierEntry {
                author: issuer_id(&key(7)),
                seq: 0,
                head: define.id,
            }],
            issued: 0,
            burned: 0,
            balance_root: balance_root(&currency.compact_hash(), &[]),
        };
        let body = claimed.body().unwrap();
        let forged = sign_body(&key(9), 0, None, &body, SNAPSHOT_KIND, vec![]).unwrap();
        assert_eq!(
            verify_snapshot(&forged, Some(&body), &[define], &bodies),
            Err(SnapshotFinding::UnknownCurrency)
        );
    }

    /// A routine rotation of the world's issuer from key 7 to `to`,
    /// signed by `by` at `seq` on `backlink`.
    fn rotation(
        w: &World,
        by: u8,
        seq: u32,
        backlink: Hash,
        from: u64,
        previous: Option<Hash>,
        to: u8,
    ) -> Event {
        let mut body = crate::issuer_rotation::IssuerRotation::unsigned(
            crate::issuer_rotation::RotationMode::Routine,
            w.currency.issuer(),
            w.currency.compact_hash(),
            from,
            previous,
            issuer_id(&key(to)),
            None,
        );
        body.prove_successor(&key(to)).unwrap();
        crate::issuer_rotation::sign_rotation(&key(by), seq, Some(backlink), &body, vec![]).unwrap()
    }

    fn verify_as(
        w: &World,
        snap: &TokenSnapshot,
        by: u8,
        seq: u32,
        backlink: Option<Hash>,
        after: Vec<Hash>,
    ) -> Result<TokenSnapshot, SnapshotFinding> {
        let event =
            sign_generation_snapshot(&key(by), seq, backlink, &w.currency, snap, after).unwrap();
        verify_snapshot(&event, Some(&snap.body().unwrap()), &w.events, &w.bodies)
    }

    #[test]
    fn after_a_rotation_the_successor_signs_and_the_old_key_only_the_past() {
        let mut w = world();
        let before = snapshot(
            &fold_events(&w.events, &w.bodies),
            &w.currency.compact_hash(),
        )
        .unwrap();
        let rotated = rotation(&w, 7, 2, w.id(1), 0, None, 13);
        w.events.push(rotated.clone());
        let after = snapshot(
            &fold_events(&w.events, &w.bodies),
            &w.currency.compact_hash(),
        )
        .unwrap();

        // The new key speaks for the frontier that holds its rotation.
        assert_eq!(
            verify_as(&w, &after, 13, 0, None, vec![rotated.id]),
            Ok(after.clone())
        );
        // The old key no longer does.
        assert_eq!(
            verify_as(&w, &after, 7, 3, Some(rotated.id), vec![]),
            Err(SnapshotFinding::NotIssuer)
        );
        // But its snapshot of the frontier before the rotation still stands,
        // and the new key cannot speak for that frontier.
        assert_eq!(
            verify_as(&w, &before, 7, 3, Some(rotated.id), vec![]),
            Ok(before.clone())
        );
        assert_eq!(
            verify_as(&w, &before, 13, 0, None, vec![rotated.id]),
            Err(SnapshotFinding::NotIssuer)
        );
    }

    #[test]
    fn a_forked_issuer_chain_signs_no_snapshot() {
        let mut w = world();
        let one = rotation(&w, 7, 2, w.id(1), 0, None, 13);
        let two = rotation(&w, 7, 2, w.id(1), 0, None, 15);
        w.events.push(one.clone());
        w.events.push(two);
        let snap = snapshot(
            &fold_events(&w.events, &w.bodies),
            &w.currency.compact_hash(),
        )
        .unwrap();
        for (by, seq, backlink) in [(13, 0, None), (7, 3, Some(one.id))] {
            assert_eq!(
                verify_as(&w, &snap, by, seq, backlink, vec![one.id]),
                Err(SnapshotFinding::NotIssuer)
            );
        }
    }
}
