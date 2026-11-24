//! Per-(agent, world) quota ledger (#8301, v2 step 3 of #6301).
//!
//! The policy point's v1 budget lives in one bridge session and resets
//! when the session does. This ledger is keyed by the agent and its
//! world instead, so spend survives leave and rejoin: a rejoin reopens
//! the existing ledger and never grants a fresh budget. The budget is
//! the role's [`QuotaClass`]; replenishment follows the role's cadence.
//!
//! Like [`crate::account_quota`], nothing here reads a clock: every
//! operation takes `now` in seconds from the caller, so the ledger is
//! fully determined by its inputs. The ledger is serializable so the
//! host can persist it; the wiring into the policy point is the next
//! slice.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::mcp_policy::Quota;
use crate::role_config::{QuotaClass, Replenishment};

/// A refused charge: the axis that would be exceeded, and when the
/// ledger next regains budget (`None` for a never-replenishing role).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerExhaustion {
    /// The budget axis the charge would have exceeded.
    pub axis: String,
    /// Unix seconds of the next replenishment tick, when one exists.
    pub replenishes_at: Option<u64>,
}

/// The quota ledger for one agent in one world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuotaLedger {
    agent_key: [u8; 32],
    world: String,
    role: String,
    budget: Quota,
    replenishment: Option<Replenishment>,
    spend: Quota,
    last_replenished_at: u64,
}

impl AgentQuotaLedger {
    /// Open a ledger for an agent joining a world under `role`. The
    /// first open grants the role's budget with zero spend; a later
    /// open of an existing ledger must go through [`Self::reopen`].
    #[must_use]
    pub fn open(
        agent_key: [u8; 32],
        world: impl Into<String>,
        role: impl Into<String>,
        class: QuotaClass,
        now: u64,
    ) -> Self {
        Self {
            agent_key,
            world: world.into(),
            role: role.into(),
            budget: class.budget,
            replenishment: class.replenishment,
            spend: Quota::zero(),
            last_replenished_at: now,
        }
    }

    /// Reopen an existing ledger on rejoin. Spend is kept as it was;
    /// only the role's current budget and cadence are adopted, so a
    /// role edit moves the budget without resetting spend.
    pub fn reopen(&mut self, role: impl Into<String>, class: QuotaClass, now: u64) {
        self.role = role.into();
        self.budget = class.budget;
        self.replenishment = class.replenishment;
        self.refresh(now);
    }

    /// The agent this ledger belongs to.
    #[must_use]
    pub const fn agent_key(&self) -> &[u8; 32] {
        &self.agent_key
    }

    /// The world this ledger belongs to.
    #[must_use]
    pub fn world(&self) -> &str {
        &self.world
    }

    /// The role the budget was granted under.
    #[must_use]
    pub fn role(&self) -> &str {
        &self.role
    }

    /// The budget currently in force.
    #[must_use]
    pub const fn budget(&self) -> Quota {
        self.budget
    }

    /// The spend outstanding after replenishment up to `now`.
    pub fn spend(&mut self, now: u64) -> Quota {
        self.refresh(now);
        self.spend
    }

    /// The spend outstanding as of `now`, read without touching the
    /// ledger: replenishment is applied to a copy.
    #[must_use]
    pub fn spend_at(&self, now: u64) -> Quota {
        let mut copy = self.clone();
        copy.refresh(now);
        copy.spend
    }

    /// Charge one admitted action at `now`. A charge that would exceed
    /// the budget is refused and charges nothing.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerExhaustion`] naming the exhausted axis and the
    /// next replenishment time.
    pub fn charge(&mut self, now: u64, charge: Quota) -> Result<(), LedgerExhaustion> {
        self.refresh(now);
        if let Some(axis) = Quota::exceeded_axis(self.budget, self.spend, charge) {
            return Err(LedgerExhaustion {
                axis: axis.to_owned(),
                replenishes_at: self.next_replenishment(),
            });
        }
        self.spend.add(charge);
        Ok(())
    }

    /// Regain budget for every whole replenishment period elapsed by
    /// `now`. A clock that moves backwards regains nothing.
    fn refresh(&mut self, now: u64) {
        let Some(replenishment) = self.replenishment else {
            return;
        };
        let elapsed = now.saturating_sub(self.last_replenished_at);
        let ticks = elapsed / replenishment.every_secs;
        if ticks == 0 {
            return;
        }
        self.spend = Quota {
            frames: self
                .spend
                .frames
                .saturating_sub(replenishment.amount.frames.saturating_mul(ticks)),
            tool_calls: self
                .spend
                .tool_calls
                .saturating_sub(replenishment.amount.tool_calls.saturating_mul(ticks)),
            program_invocations: self.spend.program_invocations.saturating_sub(
                replenishment
                    .amount
                    .program_invocations
                    .saturating_mul(ticks),
            ),
        };
        self.last_replenished_at = self
            .last_replenished_at
            .saturating_add(ticks.saturating_mul(replenishment.every_secs));
    }

    fn next_replenishment(&self) -> Option<u64> {
        self.replenishment
            .map(|r| self.last_replenished_at.saturating_add(r.every_secs))
    }
}

/// Spend summed per role across ledgers: the owner's per-role view.
/// Spend is read as of `now`, after replenishment.
pub fn spend_by_role(ledgers: &mut [AgentQuotaLedger], now: u64) -> BTreeMap<String, Quota> {
    let mut totals: BTreeMap<String, Quota> = BTreeMap::new();
    for ledger in ledgers {
        let spend = ledger.spend(now);
        totals
            .entry(ledger.role().to_owned())
            .or_default()
            .add(spend);
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: [u8; 32] = [7; 32];

    fn class(budget_calls: u64, every: Option<(u64, u64)>) -> QuotaClass {
        QuotaClass {
            budget: Quota {
                frames: 0,
                tool_calls: budget_calls,
                program_invocations: 0,
            },
            replenishment: every.map(|(every_secs, amount)| Replenishment {
                every_secs,
                amount: Quota {
                    frames: 0,
                    tool_calls: amount,
                    program_invocations: 0,
                },
            }),
        }
    }

    fn call() -> Quota {
        Quota {
            frames: 0,
            tool_calls: 1,
            program_invocations: 0,
        }
    }

    #[test]
    fn rejoin_keeps_spend_and_never_resets_budget() {
        let mut ledger = AgentQuotaLedger::open(AGENT, "w", "archivist", class(2, None), 0);
        ledger.charge(1, call()).unwrap();
        ledger.charge(2, call()).unwrap();

        let saved = serde_json::to_string(&ledger).unwrap();
        let mut rejoined: AgentQuotaLedger = serde_json::from_str(&saved).unwrap();
        rejoined.reopen("archivist", class(2, None), 3);

        let refusal = rejoined.charge(4, call()).unwrap_err();
        assert_eq!(refusal.axis, "tool_calls");
        assert_eq!(refusal.replenishes_at, None);
    }

    #[test]
    fn exhaustion_names_next_replenishment_and_charges_nothing() {
        let mut ledger =
            AgentQuotaLedger::open(AGENT, "w", "archivist", class(1, Some((60, 1))), 100);
        ledger.charge(100, call()).unwrap();
        let before = ledger.spend(100);

        let refusal = ledger.charge(110, call()).unwrap_err();

        assert_eq!(refusal.replenishes_at, Some(160));
        assert_eq!(ledger.spend(110), before);
    }

    #[test]
    fn replenishes_on_the_role_cadence_in_whole_periods() {
        let mut ledger =
            AgentQuotaLedger::open(AGENT, "w", "archivist", class(2, Some((60, 1))), 0);
        ledger.charge(0, call()).unwrap();
        ledger.charge(0, call()).unwrap();
        assert!(ledger.charge(59, call()).is_err());

        ledger.charge(60, call()).unwrap();
        assert_eq!(ledger.spend(60).tool_calls, 2);
        assert_eq!(
            ledger.charge(119, call()).unwrap_err().replenishes_at,
            Some(120)
        );
    }

    #[test]
    fn backwards_clock_regains_nothing() {
        let mut ledger =
            AgentQuotaLedger::open(AGENT, "w", "archivist", class(1, Some((60, 1))), 500);
        ledger.charge(500, call()).unwrap();
        assert!(ledger.charge(100, call()).is_err());
    }

    #[test]
    fn role_edit_moves_budget_but_keeps_spend() {
        let mut ledger = AgentQuotaLedger::open(AGENT, "w", "archivist", class(5, None), 0);
        ledger.charge(0, call()).unwrap();
        ledger.charge(0, call()).unwrap();

        ledger.reopen("archivist", class(2, None), 1);

        assert_eq!(ledger.budget().tool_calls, 2);
        assert!(ledger.charge(2, call()).is_err());
    }

    #[test]
    fn spend_by_role_sums_agents_under_a_role() {
        let mut a = AgentQuotaLedger::open([1; 32], "w", "archivist", class(9, None), 0);
        let mut b = AgentQuotaLedger::open([2; 32], "w", "archivist", class(9, None), 0);
        let mut c = AgentQuotaLedger::open([3; 32], "w", "courier", class(9, None), 0);
        a.charge(0, call()).unwrap();
        b.charge(0, call()).unwrap();
        b.charge(0, call()).unwrap();
        c.charge(0, call()).unwrap();

        let mut ledgers = [a, b, c];
        let totals = spend_by_role(&mut ledgers, 0);

        assert_eq!(totals["archivist"].tool_calls, 3);
        assert_eq!(totals["courier"].tool_calls, 1);
    }
}
