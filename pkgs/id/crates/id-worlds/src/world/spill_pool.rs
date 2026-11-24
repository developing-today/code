//! Grantor-side eligibility and deterministic successor selection for spill v2.
//!
//! This policy object is deliberately not an authority: pool membership and
//! replica observations can nominate a successor, but only applying the
//! returned lease actions through the world's serialized append actor grants
//! write authority. In particular, a candidate's local state never advances
//! the world's epoch.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{DO_WRITE_LEASE_HOLDER, WriteLease};

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
}
