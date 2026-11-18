//! One identity as a recovery relay shows it (#2382).
//!
//! A [`RelayView`] folds an identity's keyset from the keyset records a
//! [`RecoveryRelay`] holds, with recovery rotations judged against the
//! relayed annex and first-seen times, and reports where one of its
//! recovery requests stands. It is what the client flow reads: a
//! successor that has lost every key learns the head to pin from it,
//! guardians and surviving devices find the request in it, and the
//! successor learns from it when the rotation may be made.
//!
//! The relay is trusted for nothing but time. The identity's chain is
//! anchored at the genesis its own key signed (the identity ID is the
//! genesis admin key), and every record verified before the relay kept
//! it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result, bail};

use crate::keyset::{KeysetEntry, KeysetState, Log, fold_with_recovery};
use crate::recovery::{AnnexEntry, AnnexRecord, RecoveryEvidence, Seen};
use crate::recovery_flow::{Progress, progress};
use crate::recovery_relay::RecoveryRelay;

/// Where a recovery request stands, as a relay shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Its rotation has folded: the successor admin key is an admin key
    /// of the identity.
    Completed,
    /// The keyset moved on from the head the request pins, so the
    /// request can never complete (guardian recovery Decision 3).
    Moot,
    /// Still open at the pinned head.
    Open(Progress),
}

/// An identity's keyset and recovery evidence as one relay holds them.
#[derive(Debug)]
pub struct RelayView<'a> {
    identity: String,
    genesis: String,
    log: Log,
    annex: Vec<Seen<'a>>,
    guardian_folds: BTreeMap<String, KeysetState>,
    keyset_first_seen: BTreeMap<String, u64>,
}

impl<'a> RelayView<'a> {
    /// The view of `identity` in `relay`.
    ///
    /// Every other identity whose genesis the relay holds is folded as
    /// a possible guardian, without recovery rotations of its own.
    ///
    /// # Errors
    ///
    /// Fails if the relay holds no genesis for `identity`, or more than
    /// one.
    pub fn new(relay: &'a RecoveryRelay, identity: &str) -> Result<Self> {
        let mut log = Log::default();
        for record in relay.keyset_records() {
            log.insert(record.clone())?;
        }
        let roots = geneses(&log);
        let genesis = match roots.get(identity).map(Vec::as_slice) {
            Some([genesis]) => genesis.clone(),
            Some(_) => bail!("the relay holds more than one keyset genesis for {identity}"),
            None => bail!(
                "the relay holds no keyset for {identity}; a device of that identity must publish it first"
            ),
        };
        let keyset_first_seen = relay.keyset_first_seen();
        let times_only = RecoveryEvidence {
            annex: &[],
            guardian_folds: &BTreeMap::new(),
            keyset_first_seen: &keyset_first_seen,
        };
        let guardian_folds = roots
            .iter()
            .filter(|(id, hashes)| id.as_str() != identity && hashes.len() == 1)
            .filter_map(|(id, hashes)| {
                fold_with_recovery(&log, &hashes[0], &BTreeSet::new(), &times_only)
                    .ok()
                    .map(|state| (id.clone(), state))
            })
            .collect();
        Ok(Self {
            identity: identity.to_owned(),
            genesis,
            log,
            annex: relay.annex(identity),
            guardian_folds,
            keyset_first_seen,
        })
    }

    /// The recovery evidence the relay holds for this identity.
    #[must_use]
    pub fn evidence(&self) -> RecoveryEvidence<'_> {
        RecoveryEvidence {
            annex: &self.annex,
            guardian_folds: &self.guardian_folds,
            keyset_first_seen: &self.keyset_first_seen,
        }
    }

    /// The identity's keyset, folded with recovery.
    ///
    /// # Errors
    ///
    /// Fails if the chain cannot be folded.
    pub fn head(&self) -> Result<KeysetState> {
        fold_with_recovery(&self.log, &self.genesis, &BTreeSet::new(), &self.evidence())
    }

    /// A lamport counter above every record of this identity's chain,
    /// for a record that continues it.
    ///
    /// # Errors
    ///
    /// Fails if a counter in the chain is already at its maximum.
    pub fn next_lamport(&self) -> Result<u64> {
        let mut children: HashMap<&str, Vec<(&str, u64)>> = HashMap::new();
        let mut genesis_lamport = 0;
        for record in self.log.records() {
            if record.hash == self.genesis {
                genesis_lamport = record.lamport;
            }
            children
                .entry(record.prev_hash.as_str())
                .or_default()
                .push((record.hash.as_str(), record.lamport));
        }
        let mut highest = genesis_lamport;
        let mut seen = BTreeSet::from([self.genesis.as_str()]);
        let mut queue = vec![self.genesis.as_str()];
        while let Some(hash) = queue.pop() {
            for &(child, lamport) in children.get(hash).map_or(&[][..], Vec::as_slice) {
                if seen.insert(child) {
                    highest = highest.max(lamport);
                    queue.push(child);
                }
            }
        }
        highest
            .checked_add(1)
            .context("the keyset's lamport counter is exhausted")
    }

    /// The recovery request with this hash, if the relay holds it for
    /// this identity.
    #[must_use]
    pub fn request(&self, request_id: &str) -> Option<&'a AnnexRecord> {
        self.annex.iter().map(|seen| seen.record).find(|record| {
            record.hash == request_id && matches!(record.entry, AnnexEntry::RecoveryRequest { .. })
        })
    }

    /// Where the request `request_id` stands at `now`.
    ///
    /// # Errors
    ///
    /// Fails if the relay does not hold the request for this identity,
    /// or the keyset cannot be folded.
    pub fn status(&self, request_id: &str, now: u64) -> Result<Status> {
        let request = self.request(request_id).with_context(|| {
            format!(
                "the relay holds no recovery request {request_id} for {}",
                self.identity
            )
        })?;
        let AnnexEntry::RecoveryRequest {
            keyset_head_hash,
            successor_admin_key,
            ..
        } = &request.entry
        else {
            bail!("{request_id} is not a recovery request");
        };
        let head = self.head()?;
        if head.is_admin(successor_admin_key) {
            return Ok(Status::Completed);
        }
        if head.head_hash != *keyset_head_hash {
            return Ok(Status::Moot);
        }
        Ok(Status::Open(progress(
            &self.evidence(),
            &head,
            request_id,
            now,
        )?))
    }
}

/// The genesis record hashes in `log`, by the identity each roots.
fn geneses(log: &Log) -> BTreeMap<String, Vec<String>> {
    let mut roots: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for record in log.records() {
        if let KeysetEntry::Genesis { admin_key, .. } = &record.entry
            && record.prev_hash.is_empty()
            && *admin_key == record.author
        {
            roots
                .entry(admin_key.clone())
                .or_default()
                .push(record.hash.clone());
        }
    }
    roots
}
