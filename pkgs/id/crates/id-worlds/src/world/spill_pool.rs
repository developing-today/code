//! Grantor-side eligibility and deterministic successor selection for spill v2.
//!
//! This policy object is deliberately not an authority: pool membership and
//! replica observations can nominate a successor, but only applying the
//! returned lease actions through the world's serialized append actor grants
//! write authority. In particular, a candidate's local state never advances
//! the world's epoch.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use anyhow::{Context, Result, ensure};

use super::{DO_WRITE_LEASE_HOLDER, JournalEntry, WriteLease};

/// Default continuous reader observation window from design #5787 §2.
pub const READER_OBSERVATION_CYCLES: u8 = 10;

/// Why a holder is being replaced. Death permits selecting another candidate
/// without stickiness; a recovery after a polite return may prefer the prior
/// holder when all higher-priority properties tie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailoverCause {
    /// The previous holder's authority-clock term expired.
    HolderExpired,
    /// Recovery after a non-death transition where stickiness may break ties.
    Recovery,
}

/// The grantor's latest view of one configured pool member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolMember {
    /// Authenticated node identity, as established by the host adapter.
    pub identity: String,
    /// Whether the latest report authenticated as this roster member.
    pub authenticated: bool,
    /// Whether the host can currently reach the node.
    pub reachable: bool,
    /// Whether the host considers this node healthy.
    pub healthy: bool,
    /// Current journal replay watermark reported by the reader.
    pub replay_position: u64,
    /// Authoritative journal tip observed by the grantor.
    pub journal_tip: u64,
    /// Number of worlds this host currently holds.
    pub holder_count: u32,
    /// Last authority renewal-cycle identifier observed for this reader.
    pub last_renewal_cycle: Option<u64>,
    /// Consecutive renewal observations made within the catch-up bound.
    pub tip_observation_cycles: u8,
}

impl PoolMember {
    /// Construct a configured member before any authenticated observations.
    #[must_use]
    pub fn configured(identity: impl Into<String>) -> Self {
        Self {
            identity: identity.into(),
            authenticated: false,
            reachable: false,
            healthy: false,
            replay_position: 0,
            journal_tip: 0,
            holder_count: 0,
            last_renewal_cycle: None,
            tip_observation_cycles: 0,
        }
    }

    /// Record one observation cycle. The reader-join window resets on an
    /// out-of-bound lag, authentication failure, or reachability failure.
    pub fn observe(
        &mut self,
        renewal_cycle: u64,
        authenticated: bool,
        reachable: bool,
        healthy: bool,
        replay_position: u64,
        journal_tip: u64,
        max_catch_up_lag: u64,
        holder_count: u32,
    ) {
        let was_within_bound = self.authenticated
            && self.reachable
            && self.replay_position <= self.journal_tip
            && self.journal_tip.saturating_sub(self.replay_position) <= max_catch_up_lag;
        let within_bound = authenticated
            && reachable
            && replay_position <= journal_tip
            && journal_tip.saturating_sub(replay_position) <= max_catch_up_lag;
        let next_cycle =
            self.last_renewal_cycle.and_then(|last| last.checked_add(1)) == Some(renewal_cycle);
        let same_cycle = self.last_renewal_cycle == Some(renewal_cycle);
        self.tip_observation_cycles = if within_bound {
            if same_cycle {
                self.tip_observation_cycles
            } else if was_within_bound && next_cycle {
                self.tip_observation_cycles
                    .saturating_add(1)
                    .min(READER_OBSERVATION_CYCLES)
            } else {
                1
            }
        } else {
            0
        };
        self.last_renewal_cycle = Some(renewal_cycle);
        self.authenticated = authenticated;
        self.reachable = reachable;
        self.healthy = healthy;
        self.replay_position = replay_position;
        self.journal_tip = journal_tip;
        self.holder_count = holder_count;
    }

    /// Whether this member can be selected for a grant to `journal_tip`.
    #[must_use]
    pub fn eligible(&self, journal_tip: u64, max_catch_up_lag: u64) -> bool {
        self.authenticated
            && self.reachable
            && self.journal_tip == journal_tip
            && self.replay_position <= journal_tip
            && journal_tip.saturating_sub(self.replay_position) <= max_catch_up_lag
            && self.tip_observation_cycles >= READER_OBSERVATION_CYCLES
    }
}

/// Grantor-side roster. Only explicitly configured identities can be
/// observed or selected; observations never grant authority by themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpillPool {
    members: BTreeMap<String, PoolMember>,
}

impl SpillPool {
    /// Add a member to the grantor's roster. Reconfiguration resets its
    /// reader observation window so stale eligibility cannot carry over.
    pub fn configure(&mut self, identity: impl Into<String>) -> &PoolMember {
        let identity = identity.into();
        match self.members.entry(identity.clone()) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.insert(PoolMember::configured(identity));
                entry.into_mut()
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(PoolMember::configured(identity))
            }
        }
    }

    /// Remove an identity from the roster. This does not change the active
    /// lease; an active holder still requires a journaled boundary.
    pub fn remove(&mut self, identity: &str) -> Option<PoolMember> {
        self.members.remove(identity)
    }

    /// Record an authenticated host observation for a roster member.
    pub fn observe(
        &mut self,
        identity: &str,
        renewal_cycle: u64,
        authenticated: bool,
        reachable: bool,
        healthy: bool,
        replay_position: u64,
        journal_tip: u64,
        max_catch_up_lag: u64,
        holder_count: u32,
    ) -> bool {
        let Some(member) = self.members.get_mut(identity) else {
            return false;
        };
        member.observe(
            renewal_cycle,
            authenticated,
            reachable,
            healthy,
            replay_position,
            journal_tip,
            max_catch_up_lag,
            holder_count,
        );
        true
    }

    /// Inspect a configured member.
    #[must_use]
    pub fn member(&self, identity: &str) -> Option<&PoolMember> {
        self.members.get(identity)
    }

    /// Pick a deterministic eligible successor. Ordering follows #5787 §2:
    /// exact tip first, healthy first, fewest held worlds, optional stickiness,
    /// then lexical identity.
    #[must_use]
    pub fn select_successor(
        &self,
        journal_tip: u64,
        max_catch_up_lag: u64,
        previous_holder: Option<&str>,
        cause: FailoverCause,
    ) -> Option<&PoolMember> {
        self.members
            .values()
            .filter(|member| {
                member.eligible(journal_tip, max_catch_up_lag)
                    && (cause != FailoverCause::HolderExpired
                        || Some(member.identity.as_str()) != previous_holder)
            })
            .min_by_key(|member| {
                (
                    member.replay_position != journal_tip,
                    !member.healthy,
                    member.holder_count,
                    cause == FailoverCause::HolderExpired
                        || Some(member.identity.as_str()) != previous_holder,
                    member.identity.as_str(),
                )
            })
    }

    /// Plan a failover after the authority-clock term expires. The plan
    /// includes a revoke whenever a holder still stands and always puts that
    /// fence before the grant. With no eligible pool member, the DO is the
    /// safe fallback. Applying the plan remains the actor's responsibility.
    pub fn plan_failover(
        &self,
        lease: &WriteLease,
        now_ms: u64,
        journal_tip: u64,
        max_catch_up_lag: u64,
    ) -> Result<FailoverPlan, FailoverPlanError> {
        if lease.holder.is_some() && !lease.is_expired(now_ms) {
            return Err(FailoverPlanError::TermStillLive);
        }

        let former_holder = lease.holder.clone();
        let successor = self
            .select_successor(
                journal_tip,
                max_catch_up_lag,
                former_holder.as_deref(),
                FailoverCause::HolderExpired,
            )
            .map_or_else(
                || DO_WRITE_LEASE_HOLDER.to_owned(),
                |member| member.identity.clone(),
            );

        let mut actions = Vec::with_capacity(if former_holder.is_some() { 2 } else { 1 });
        let grant_epoch = if let Some(former_holder) = former_holder {
            let revoke_epoch = lease
                .epoch
                .checked_add(1)
                .ok_or(FailoverPlanError::EpochOverflow)?;
            actions.push(LeaseAction::Revoke {
                former_holder,
                epoch: revoke_epoch,
            });
            revoke_epoch
                .checked_add(1)
                .ok_or(FailoverPlanError::EpochOverflow)?
        } else {
            lease
                .epoch
                .checked_add(1)
                .ok_or(FailoverPlanError::EpochOverflow)?
        };
        actions.push(LeaseAction::Grant {
            holder: successor,
            epoch: grant_epoch,
        });

        Ok(FailoverPlan {
            actions,
            pre_grant_tip: journal_tip,
        })
    }
}

/// An ordered, authority-ready lease transition. A grant after a standing
/// holder must be preceded by a revoke action in the same plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseAction {
    /// Fence the standing holder at this resulting epoch.
    Revoke {
        /// Identity whose lease is being fenced.
        former_holder: String,
        /// Epoch established by the revoke record.
        epoch: u64,
    },
    /// Grant authority after the predecessor fence is journaled.
    Grant {
        /// Authenticated identity selected as the next holder.
        holder: String,
        /// Epoch established by the grant record.
        epoch: u64,
    },
}

/// Deterministic failover decision. `pre_grant_tip` is the journal tip used to
/// assess the candidate. The actor must capture the actual grant position
/// after appending the grant and use that position as the resume barrier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailoverPlan {
    actions: Vec<LeaseAction>,
    pre_grant_tip: u64,
}

impl FailoverPlan {
    /// Ordered authority actions. The list is immutable so callers cannot
    /// accidentally move a grant ahead of its predecessor fence.
    #[must_use]
    pub fn actions(&self) -> &[LeaseAction] {
        &self.actions
    }

    /// Journal tip used to assess the successor before appending failover
    /// actions. This is not the successor's grant barrier.
    #[must_use]
    pub const fn pre_grant_tip(&self) -> u64 {
        self.pre_grant_tip
    }
}

/// Journal reason recorded on a failover revocation (design #5787 §1
/// step 1): the fence exists because the standing holder's term expired
/// on the authority clock.
pub const FAILOVER_REVOKE_REASON: &str = "term expired";

impl FailoverPlan {
    /// Execute the plan against the lease it was planned from: assign
    /// each action a durable lease position (#13711) continuing the
    /// lease's own chain, build the ordered journal entries, and prove
    /// the whole sequence against a scratch copy of the lease state
    /// machine before any entry exists. A plan whose lease has moved
    /// since planning — a different holder, an advanced epoch, a
    /// position already consumed — is refused here, so a grant can
    /// never be journaled ahead of its predecessor's fence and the
    /// epoch chain stays a single per-world monotonic chain (design
    /// #5787 §1 steps 1–3, §2).
    ///
    /// The successor's term is `term_ms` from `now_ms`; a DO self-grant
    /// is unterminated, as at bootstrap. The returned execution's
    /// `grant_position` is the replay barrier of §1 step 4: the
    /// successor must replay through it before it may write.
    ///
    /// # Errors
    ///
    /// The lease state machine's refusal of a stale plan, a zero term
    /// for a native successor, or an epoch/position/expiry overflow.
    pub fn execute(
        &self,
        lease: &WriteLease,
        now_ms: u64,
        term_ms: u64,
    ) -> Result<FailoverExecution> {
        let mut scratch = lease.clone();
        let mut intermediate_lease = lease.clone();
        let mut entries = Vec::with_capacity(self.actions.len());
        let mut successor = None;
        let mut grant_epoch = 0;
        let mut grant_position = 0;
        let mut position = lease.last_position();
        for action in &self.actions {
            position = position
                .checked_add(1)
                .context("failover lease position overflow")?;
            match action {
                LeaseAction::Revoke {
                    former_holder,
                    epoch,
                } => {
                    scratch.apply_revoke(
                        former_holder,
                        *epoch,
                        FAILOVER_REVOKE_REASON,
                        position,
                    )?;
                    entries.push(JournalEntry::LeaseRevoked {
                        former_holder: former_holder.clone(),
                        epoch: *epoch,
                        reason: FAILOVER_REVOKE_REASON.to_owned(),
                        position,
                    });
                    intermediate_lease = scratch.clone();
                }
                LeaseAction::Grant { holder, epoch } => {
                    let expires_at_ms = if holder == DO_WRITE_LEASE_HOLDER {
                        None
                    } else {
                        ensure!(term_ms > 0, "a native failover grant requires a term");
                        Some(
                            now_ms
                                .checked_add(term_ms)
                                .context("failover grant expiry overflow")?,
                        )
                    };
                    scratch.apply_grant(holder, *epoch, expires_at_ms, position)?;
                    entries.push(JournalEntry::LeaseGranted {
                        holder: holder.clone(),
                        epoch: *epoch,
                        expires_at_ms,
                        position,
                    });
                    successor = Some(holder.clone());
                    grant_epoch = *epoch;
                    grant_position = position;
                }
            }
        }
        let successor = successor.context("failover plan carries no grant action")?;
        Ok(FailoverExecution {
            entries,
            intermediate_lease,
            resulting_lease: scratch,
            successor,
            grant_epoch,
            grant_position,
        })
    }
}

/// A failover plan executed against its lease ([`FailoverPlan::execute`]):
/// the journal entries to append, in order, and the lease state after
/// every prefix of them. Appending the entries one at a time and
/// adopting [`FailoverExecution::intermediate_lease`] after the revoke
/// keeps live state equal to the journaled prefix at every step, so a
/// crash between the revoke and the grant leaves the world
/// unavailable — every writer refused `holder-lost` — never
/// split-brained (design #5787 §1, §7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailoverExecution {
    entries: Vec<JournalEntry>,
    intermediate_lease: WriteLease,
    resulting_lease: WriteLease,
    successor: String,
    grant_epoch: u64,
    grant_position: u64,
}

impl FailoverExecution {
    /// The ordered journal entries: the predecessor's fence first,
    /// the successor's grant last, each stamped with its durable
    /// lease position.
    #[must_use]
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// The lease after the fence but before the grant: the state of
    /// the fence-to-grant window, in which no holder stands. For a
    /// plan that needed no fence this is the pre-execution lease.
    #[must_use]
    pub const fn intermediate_lease(&self) -> &WriteLease {
        &self.intermediate_lease
    }

    /// The lease after the full plan: the successor holds at
    /// `grant_epoch`.
    #[must_use]
    pub const fn resulting_lease(&self) -> &WriteLease {
        &self.resulting_lease
    }

    /// The identity the grant names.
    #[must_use]
    pub fn successor(&self) -> &str {
        &self.successor
    }

    /// The epoch the grant establishes.
    #[must_use]
    pub const fn grant_epoch(&self) -> u64 {
        self.grant_epoch
    }

    /// The grant entry's durable lease position: the replay barrier
    /// the successor must reach before it may write (design #5787
    /// §1 step 4).
    #[must_use]
    pub const fn grant_position(&self) -> u64 {
        self.grant_position
    }

    /// Whether a node that has replayed through `replayed_position`
    /// has reached the grant barrier and may resume as the successor.
    #[must_use]
    pub const fn resume_ready(&self, replayed_position: u64) -> bool {
        replayed_position >= self.grant_position
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FailoverPlanError {
    /// The current holder has not expired on the authority clock.
    #[error("write lease term has not expired on the authority clock")]
    TermStillLive,
    /// No next epoch can be represented by the journal format.
    #[error("write lease epoch overflow")]
    EpochOverflow,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::world::{LeaseRejectionReason, LeaseWriteRejection};
    use proptest::prelude::*;

    fn eligible_member(pool: &mut SpillPool, identity: &str, tip: u64, load: u32) {
        pool.configure(identity);
        for cycle in 0..u64::from(READER_OBSERVATION_CYCLES) {
            assert!(pool.observe(identity, cycle, true, true, true, tip, tip, 0, load));
        }
    }

    #[test]
    fn reader_join_requires_ten_distinct_consecutive_renewal_cycles() {
        let mut pool = SpillPool::default();
        pool.configure("reader");
        for cycle in 0..u64::from(READER_OBSERVATION_CYCLES - 1) {
            pool.observe("reader", cycle, true, true, true, 7, 7, 0, 0);
        }
        pool.observe("reader", 8, true, true, true, 7, 7, 0, 0);
        assert!(!pool.member("reader").unwrap().eligible(7, 0));
        pool.observe("reader", 9, true, true, true, 5, 7, 1, 0);
        for cycle in 10..19 {
            pool.observe("reader", cycle, true, true, true, 7, 7, 0, 0);
        }
        assert!(!pool.member("reader").unwrap().eligible(7, 0));
        pool.observe("reader", 19, true, true, true, 7, 7, 0, 0);
        assert!(pool.member("reader").unwrap().eligible(7, 0));
        pool.observe("reader", 21, true, true, true, 7, 7, 0, 0);
        assert!(!pool.member("reader").unwrap().eligible(7, 0));
    }

    #[test]
    fn selection_prefers_tip_health_load_and_then_identity_deterministically() {
        let mut pool = SpillPool::default();
        eligible_member(&mut pool, "lagging", 20, 0);
        eligible_member(&mut pool, "z-healthy", 20, 1);
        eligible_member(&mut pool, "a-healthy", 20, 1);
        eligible_member(&mut pool, "unhealthy", 20, 0);
        pool.observe("lagging", 10, true, true, true, 19, 20, 1, 0);
        pool.observe("unhealthy", 10, true, true, false, 20, 20, 0, 0);
        assert_eq!(
            pool.select_successor(20, 1, None, FailoverCause::HolderExpired)
                .map(|member| member.identity.as_str()),
            Some("a-healthy")
        );
        assert_eq!(
            pool.select_successor(20, 1, Some("z-healthy"), FailoverCause::Recovery)
                .map(|member| member.identity.as_str()),
            Some("z-healthy")
        );
        assert_eq!(
            pool.select_successor(20, 1, Some("z-healthy"), FailoverCause::HolderExpired)
                .map(|member| member.identity.as_str()),
            Some("a-healthy")
        );
    }

    #[test]
    fn failover_fences_before_grant_and_advances_each_epoch() {
        let mut pool = SpillPool::default();
        eligible_member(&mut pool, "successor", 50, 0);
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "dead", 1, Some(30_000), 0)
            .unwrap();

        assert_eq!(
            pool.plan_failover(&lease, 29_999, 50, 0),
            Err(FailoverPlanError::TermStillLive)
        );
        let plan = pool.plan_failover(&lease, 30_000, 50, 0).unwrap();
        assert_eq!(
            plan.actions(),
            vec![
                LeaseAction::Revoke {
                    former_holder: "dead".into(),
                    epoch: 2,
                },
                LeaseAction::Grant {
                    holder: "successor".into(),
                    epoch: 3,
                },
            ]
        );
        assert_eq!(plan.pre_grant_tip(), 50);
        lease.apply_revoke("dead", 2, "term expired", 0).unwrap();
        lease.apply_grant("successor", 3, Some(60_000), 0).unwrap();
        assert_eq!(lease.holder.as_deref(), Some("successor"));
        assert_eq!(lease.epoch, 3);
        assert!(lease.validate_history().is_ok());
    }

    #[test]
    fn no_eligible_successor_falls_back_to_do_after_fencing() {
        let pool = SpillPool::default();
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "dead", 1, Some(1), 0)
            .unwrap();
        assert_eq!(
            pool.plan_failover(&lease, 1, 50, 0).unwrap().actions(),
            vec![
                LeaseAction::Revoke {
                    former_holder: "dead".into(),
                    epoch: 2,
                },
                LeaseAction::Grant {
                    holder: DO_WRITE_LEASE_HOLDER.into(),
                    epoch: 3,
                },
            ]
        );
    }

    #[test]
    fn no_standing_holder_grants_at_the_next_epoch() {
        let pool = SpillPool::default();
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "dead", 1, Some(1), 0)
            .unwrap();
        lease.apply_revoke("dead", 2, "term expired", 0).unwrap();
        assert_eq!(
            pool.plan_failover(&lease, 1, 50, 0).unwrap().actions(),
            vec![LeaseAction::Grant {
                holder: DO_WRITE_LEASE_HOLDER.into(),
                epoch: 3,
            }]
        );
    }

    #[test]
    fn writes_during_the_fence_window_are_retryable_holder_lost() {
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "old", 1, Some(10), 0)
            .unwrap();
        lease.apply_revoke("old", 2, "term expired", 0).unwrap();
        assert_eq!(
            lease.check_writer("old", 1, 10),
            Err(LeaseRejectionReason::HolderLost)
        );
    }

    #[test]
    fn fenced_holder_reconnect_is_rejected_and_auditable() {
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "old", 1, Some(10), 0)
            .unwrap();
        lease.apply_revoke("old", 2, "term expired", 0).unwrap();
        lease.apply_grant("new", 3, Some(30), 0).unwrap();
        assert_eq!(
            lease.check_writer("old", 1, 11),
            Err(LeaseRejectionReason::NotHolder)
        );
        lease
            .apply_rejection(LeaseWriteRejection {
                writer: "old".into(),
                presented_epoch: 1,
                operation_id: "reconnected-command".into(),
                observed_epoch: 3,
                reason: LeaseRejectionReason::NotHolder,
                received_at_ms: 11,
                position: 0,
            })
            .unwrap();
        assert_eq!(lease.rejections().count(), 1);
    }

    #[test]
    fn failover_and_handback_races_have_one_serialized_winner() {
        // Handback linearizes before expiry. The resulting DO lease blocks
        // automatic failover from the old holder.
        let mut handback_first = WriteLease::bootstrap();
        handback_first
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "native", 1, Some(30), 0)
            .unwrap();
        assert!(handback_first.check_writer("native", 1, 29).is_ok());
        handback_first
            .apply_handoff("native", DO_WRITE_LEASE_HOLDER, 2, None, 0)
            .unwrap();
        assert_eq!(
            SpillPool::default().plan_failover(&handback_first, 30, 0, 0),
            Err(FailoverPlanError::TermStillLive)
        );

        // Expiry linearizes first. The old holder is fenced and cannot hand
        // back; automatic recovery uses revoke then grant from that state.
        let mut failover_first = WriteLease::bootstrap();
        failover_first
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "native", 1, Some(30), 0)
            .unwrap();
        assert_eq!(
            failover_first.check_writer("native", 1, 30),
            Err(LeaseRejectionReason::Expired)
        );
        let plan = SpillPool::default()
            .plan_failover(&failover_first, 30, 0, 0)
            .unwrap();
        assert!(matches!(
            plan.actions()[0],
            LeaseAction::Revoke { epoch: 2, .. }
        ));
        assert!(matches!(
            plan.actions()[1],
            LeaseAction::Grant { epoch: 3, .. }
        ));
    }

    #[test]
    fn grant_storm_cannot_regrant_before_the_new_term_expires() {
        let pool = SpillPool::default();
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "first", 1, Some(10), 0)
            .unwrap();
        lease.apply_revoke("first", 2, "term expired", 0).unwrap();
        lease.apply_grant("second", 3, Some(40), 0).unwrap();
        assert_eq!(
            pool.plan_failover(&lease, 39, 0, 0),
            Err(FailoverPlanError::TermStillLive)
        );
        let next = pool.plan_failover(&lease, 40, 0, 0).unwrap();
        assert!(matches!(
            next.actions()[0],
            LeaseAction::Revoke { epoch: 4, .. }
        ));
        assert!(matches!(
            next.actions()[1],
            LeaseAction::Grant { epoch: 5, .. }
        ));
        lease.validate_history().unwrap();
    }

    proptest! {
        #[test]
        fn every_failover_plan_fences_and_advances_one_global_epoch_chain(base_epoch in 1_u64..128) {
            let mut lease = WriteLease::bootstrap();
            let mut holder = DO_WRITE_LEASE_HOLDER.to_owned();
            for epoch in 1..=base_epoch {
                let next_holder = format!("holder-{epoch}");
                lease.apply_handoff(&holder, &next_holder, epoch, Some(1), 0).unwrap();
                holder = next_holder;
            }

            let plan = SpillPool::default().plan_failover(&lease, 1, 0, 0).unwrap();
            prop_assert_eq!(plan.actions().len(), 2);
            let LeaseAction::Revoke { former_holder, epoch: revoke_epoch } = &plan.actions()[0] else {
                prop_assert!(false, "a standing holder must be fenced before grant");
                unreachable!();
            };
            let LeaseAction::Grant { holder: successor, epoch: grant_epoch } = &plan.actions()[1] else {
                prop_assert!(false, "grant must follow the fence");
                unreachable!();
            };
            prop_assert_eq!(former_holder, &holder);
            prop_assert_eq!(*revoke_epoch, base_epoch + 1);
            prop_assert_eq!(*grant_epoch, base_epoch + 2);

            lease.apply_revoke(former_holder, *revoke_epoch, "expired", 0).unwrap();
            lease.apply_grant(successor, *grant_epoch, Some(2), 0).unwrap();
            prop_assert!(lease.validate_history().is_ok());
        }
    }

    fn stamped_expired_lease() -> (SpillPool, WriteLease) {
        let mut pool = SpillPool::default();
        eligible_member(&mut pool, "successor", 50, 0);
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "dead", 1, Some(30_000), 4)
            .unwrap();
        (pool, lease)
    }

    #[test]
    fn execution_journals_fence_then_grant_and_names_the_barrier() {
        // Oracle: design #5787 §7, holder death with a caught-up
        // successor — revoke at e+1, grant at e+2, positions continuing
        // the lease chain, and the grant position is the barrier.
        let (pool, lease) = stamped_expired_lease();
        let plan = pool.plan_failover(&lease, 30_000, 50, 0).unwrap();
        let execution = plan.execute(&lease, 30_000, 30_000).unwrap();
        assert_eq!(
            execution.entries(),
            vec![
                JournalEntry::LeaseRevoked {
                    former_holder: "dead".into(),
                    epoch: 2,
                    reason: FAILOVER_REVOKE_REASON.into(),
                    position: 5,
                },
                JournalEntry::LeaseGranted {
                    holder: "successor".into(),
                    epoch: 3,
                    expires_at_ms: Some(60_000),
                    position: 6,
                },
            ]
            .as_slice()
        );
        assert_eq!(execution.successor(), "successor");
        assert_eq!(execution.grant_epoch(), 3);
        assert_eq!(execution.grant_position(), 6);
        assert!(!execution.resume_ready(5));
        assert!(execution.resume_ready(6));

        // Inside the fence-to-grant window no holder stands: every
        // writer is refused holder-lost, never silently admitted.
        let window = execution.intermediate_lease();
        assert_eq!(window.holder, None);
        assert_eq!(
            window.check_writer("dead", 1, 30_000),
            Err(LeaseRejectionReason::HolderLost)
        );

        let resulting = execution.resulting_lease();
        assert_eq!(resulting.holder.as_deref(), Some("successor"));
        assert_eq!(resulting.epoch, 3);
        assert_eq!(resulting.last_position(), 6);
        resulting.validate_history().unwrap();
        // The fenced holder stays fenced at the new epoch.
        assert_eq!(
            resulting.check_writer("dead", 1, 30_001),
            Err(LeaseRejectionReason::NotHolder)
        );
        assert_eq!(resulting.check_writer("successor", 3, 30_001), Ok(()));
    }

    #[test]
    fn execution_without_an_eligible_successor_grants_the_do_untermed() {
        // Oracle: design #5787 §7, holder death with no eligible
        // successor — the DO takes the grant itself, unterminated,
        // rather than granting a lagging replica.
        let pool = SpillPool::default();
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "dead", 1, Some(1), 0)
            .unwrap();
        let plan = pool.plan_failover(&lease, 1, 50, 0).unwrap();
        let execution = plan.execute(&lease, 1, 30_000).unwrap();
        assert_eq!(execution.successor(), DO_WRITE_LEASE_HOLDER);
        assert!(matches!(
            execution.entries()[1],
            JournalEntry::LeaseGranted {
                expires_at_ms: None,
                ..
            }
        ));
        assert_eq!(execution.resulting_lease().expires_at_ms, None);
    }

    #[test]
    fn execution_without_a_standing_holder_is_a_single_grant() {
        let pool = SpillPool::default();
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "dead", 1, Some(1), 0)
            .unwrap();
        lease.apply_revoke("dead", 2, "term expired", 0).unwrap();
        let plan = pool.plan_failover(&lease, 1, 50, 0).unwrap();
        let execution = plan.execute(&lease, 1, 30_000).unwrap();
        assert_eq!(execution.entries().len(), 1);
        // No fence window opens: the intermediate state is the
        // pre-execution lease, already holder-less.
        assert_eq!(execution.intermediate_lease(), &lease);
        assert_eq!(execution.grant_epoch(), 3);
    }

    #[test]
    fn execution_refuses_a_plan_whose_lease_has_moved() {
        // A plan is bound to the lease it was planned from: if the
        // world moved on (here, recovery already happened by hand),
        // executing the stale plan must fail before any entry exists,
        // so no grant can ever be journaled from it.
        let (pool, lease) = stamped_expired_lease();
        let plan = pool.plan_failover(&lease, 30_000, 50, 0).unwrap();
        let mut moved = lease.clone();
        moved.apply_revoke("dead", 2, "term expired", 5).unwrap();
        moved
            .apply_grant(DO_WRITE_LEASE_HOLDER, 3, None, 6)
            .unwrap();
        assert!(plan.execute(&moved, 30_000, 30_000).is_err());
        assert!(
            plan.execute(&WriteLease::bootstrap(), 30_000, 30_000)
                .is_err()
        );
    }

    #[test]
    fn execution_requires_a_term_for_a_native_successor() {
        let (pool, lease) = stamped_expired_lease();
        let plan = pool.plan_failover(&lease, 30_000, 50, 0).unwrap();
        assert!(plan.execute(&lease, 30_000, 0).is_err());
    }

    #[test]
    fn q2_failover_measurements_pin_the_design_bounds() {
        // Q2 deciding evidence (design #5787 Q2, owned by #7572):
        // measured detection, grant, and replay-to-ready under the
        // standing defaults — 30 s term, 10 s renewal. The oracle is
        // the design's own arithmetic: detection-plus-revoke is
        // bounded by one term, a caught-up successor resumes in
        // grant-plus-one-append time from the fence, and renewal
        // costs one journaled control append per renewal.
        const TERM_MS: u64 = 30_000;
        const RENEWAL_MS: u64 = 10_000;

        // Renewal cost: a live holder renewing every RENEWAL_MS for
        // 300 s of authority time journals exactly 30 control
        // appends — 0.1 appends/s per spilled world — and its epoch
        // never moves.
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "holder", 1, Some(TERM_MS), 1)
            .unwrap();
        let mut renewals = 0_u64;
        let mut position = 1_u64;
        for tick in 1..=30_u64 {
            position += 1;
            lease
                .apply_renew("holder", 1, tick * RENEWAL_MS + TERM_MS, position)
                .unwrap();
            renewals += 1;
        }
        assert_eq!(renewals, 30);
        assert_eq!(lease.epoch, 1);
        assert_eq!(lease.expires_at_ms, Some(300_000 + TERM_MS));

        // Detection: this lease was last stamped at t=0 with a full
        // term, so it expires at TERM_MS. A holder dying
        // `death_offset` after that stamp is fenced exactly at
        // expiry — never one millisecond earlier — so the measured
        // detection latency is TERM_MS - death_offset: worst case
        // exactly one term, when death follows a renewal immediately.
        let mut pool = SpillPool::default();
        eligible_member(&mut pool, "successor", 1, 0);
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "holder", 1, Some(TERM_MS), 1)
            .unwrap();
        for (death_offset, expected_detection) in [
            (0_u64, 30_000_u64),
            (10_000, 20_000),
            (20_000, 10_000),
            (29_999, 1),
        ] {
            assert_eq!(
                pool.plan_failover(&lease, TERM_MS - 1, 1, 0),
                Err(FailoverPlanError::TermStillLive),
                "a death {death_offset} ms after the stamp must not be fenced before expiry"
            );
            assert!(pool.plan_failover(&lease, TERM_MS, 1, 0).is_ok());
            assert_eq!(TERM_MS - death_offset, expected_detection);
        }

        // Grant: executing the plan at expiry journals the fence and
        // the grant as two consecutive appends — the grant lands
        // exactly one append after the fence, at epoch e+2 (the
        // revoke consumes e+1), carrying a fresh full term.
        let plan = pool.plan_failover(&lease, TERM_MS, 1, 0).unwrap();
        let execution = plan.execute(&lease, TERM_MS, TERM_MS).unwrap();
        assert_eq!(execution.successor(), "successor");
        assert_eq!(execution.grant_epoch(), 3);
        assert_eq!(execution.entries().len(), 2);
        let positions: Vec<u64> = execution
            .entries()
            .iter()
            .map(|entry| match entry {
                JournalEntry::LeaseRevoked { position, .. }
                | JournalEntry::LeaseGranted { position, .. } => *position,
                other => panic!("failover journals only lease entries, got {other:?}"),
            })
            .collect();
        assert_eq!(positions, vec![2, 3]);
        assert_eq!(execution.grant_position(), positions[1]);
        assert_eq!(
            execution.resulting_lease().expires_at_ms,
            Some(TERM_MS + TERM_MS)
        );

        // Replay-to-ready: the successor observed at the pre-failover
        // tip (position 1) must replay the two failover entries; one
        // that has already replayed the fence replays one; at the
        // grant position it is ready.
        assert!(!execution.resume_ready(1));
        assert!(!execution.resume_ready(2));
        assert!(execution.resume_ready(3));
        assert_eq!(execution.grant_position() - 1, 2);
    }
}
