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
//! fully determined by its inputs. Ledgers are serializable, and the
//! host keeps them in a [`QuotaLedgerStore`] across sessions: a
//! session opens its ledger from the store and hands it back on
//! close, so leave/rejoin cycles and host restarts never reset a
//! budget.

use std::collections::{BTreeMap, BTreeSet};

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

/// A refused [`QuotaLedgerStore::open`]: the pair's ledger is
/// already checked out by a live handle, so a second open would
/// hand out a divergent copy of the same budget (#14526).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerCheckedOut {
    /// The agent whose ledger is checked out.
    pub agent_key: [u8; 32],
    /// The world whose ledger is checked out.
    pub world: String,
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

/// The host-side home of per-(agent, world) quota ledgers (#8301).
///
/// The policy point owns one ledger per live session; the store owns
/// every ledger across sessions. A session opens its ledger from the
/// store and hands it back on close, so leave/rejoin cycles never
/// reset a budget. The store is serializable through its ledger list
/// (see [`Self::ledgers`] and [`Self::restore`]), so a host restart
/// also keeps every spend: the host snapshots the list alongside its
/// other durable state and restores it before opening sessions.
///
/// Opens are exclusive (#14526): a pair's ledger is checked out to
/// at most one live handle at a time, and a second open while one
/// is outstanding is refused rather than handing out a divergent
/// copy of the same budget.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaLedgerStore {
    ledgers: BTreeMap<([u8; 32], String), AgentQuotaLedger>,
    /// Pairs whose ledger is currently checked out by a live
    /// handle (#14526). A checkout is taken by [`Self::open`] and
    /// released by [`Self::close`]; it is runtime state, not part
    /// of a snapshot -- a restored store starts with nothing
    /// checked out.
    #[serde(default)]
    checked_out: BTreeSet<([u8; 32], String)>,
}

impl QuotaLedgerStore {
    /// An empty store: the first open for an agent grants a fresh
    /// budget under its role.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the ledger for an agent joining a world under `role`,
    /// checking it out until it is handed back with [`Self::close`].
    /// The first open grants the role's budget with zero spend; a
    /// later open reopens the persisted ledger, keeping its spend
    /// and adopting the role's current budget and cadence, so a
    /// role edit moves the budget without resetting spend.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerCheckedOut`] when the pair's ledger is
    /// already checked out by a live handle (#14526): two live
    /// handles would each hold a full budget and silently drop
    /// each other's spend on close, so the second open is refused
    /// instead. The refusal changes nothing; retry after the
    /// outstanding handle closes.
    pub fn open(
        &mut self,
        agent_key: [u8; 32],
        world: impl Into<String>,
        role: impl Into<String>,
        class: QuotaClass,
        now: u64,
    ) -> Result<AgentQuotaLedger, LedgerCheckedOut> {
        let world = world.into();
        let key = (agent_key, world.clone());
        if self.checked_out.contains(&key) {
            return Err(LedgerCheckedOut { agent_key, world });
        }
        let ledger = match self.ledgers.get_mut(&key) {
            Some(ledger) => {
                ledger.reopen(role, class, now);
                ledger.clone()
            }
            None => AgentQuotaLedger::open(agent_key, world, role, class, now),
        };
        self.checked_out.insert(key);
        Ok(ledger)
    }

    /// Hand a session's ledger back to the store, e.g. on leave,
    /// releasing its checkout so the pair can open again. Upserts
    /// by the ledger's own agent and world, so a close after a
    /// role edit keeps the ledger under its new role. Closing a
    /// ledger that was never checked out (e.g. via [`Self::restore`])
    /// simply upserts it.
    pub fn close(&mut self, ledger: AgentQuotaLedger) {
        let key = (*ledger.agent_key(), ledger.world().to_owned());
        self.checked_out.remove(&key);
        self.ledgers.insert(key, ledger);
    }

    /// Every ledger the store holds, for snapshots. The host
    /// serializes this list (each ledger carries its own agent,
    /// world and role) and feeds it back to [`Self::restore`].
    /// A [`BTreeMap`] would not survive JSON: tuple keys are not
    /// string keys.
    #[must_use]
    pub fn ledgers(&self) -> Vec<AgentQuotaLedger> {
        self.ledgers.values().cloned().collect()
    }

    /// Rebuild a store from a snapshotted ledger list, e.g. after a
    /// host restart. Later entries win on duplicate (agent, world)
    /// keys, so a concatenated snapshot still converges.
    #[must_use]
    pub fn restore(ledgers: Vec<AgentQuotaLedger>) -> Self {
        let mut store = Self::new();
        for ledger in ledgers {
            store.close(ledger);
        }
        store
    }

    /// The ledger held for an agent in a world, if any.
    #[must_use]
    pub fn get(&self, agent_key: &[u8; 32], world: &str) -> Option<&AgentQuotaLedger> {
        self.ledgers.get(&(*agent_key, world.to_owned()))
    }

    /// Whether the pair's ledger is currently checked out by a
    /// live handle (#14526).
    #[must_use]
    pub fn is_checked_out(&self, agent_key: &[u8; 32], world: &str) -> bool {
        self.checked_out.contains(&(*agent_key, world.to_owned()))
    }

    /// The spend outstanding for one agent in one world as of `now`,
    /// after replenishment; `None` when the store holds no ledger
    /// for the pair. Read-only: an owner read never advances a
    /// ledger's replenishment clock.
    #[must_use]
    pub fn spend_for_agent(&self, agent_key: &[u8; 32], world: &str, now: u64) -> Option<Quota> {
        self.get(agent_key, world)
            .map(|ledger| ledger.spend_at(now))
    }

    /// Spend summed per role across every ledger held: the owner's
    /// per-role view. Read-only, like [`Self::spend_for_agent`].
    #[must_use]
    pub fn spend_by_role(&self, now: u64) -> BTreeMap<String, Quota> {
        let mut totals: BTreeMap<String, Quota> = BTreeMap::new();
        for ledger in self.ledgers.values() {
            totals
                .entry(ledger.role().to_owned())
                .or_default()
                .add(ledger.spend_at(now));
        }
        totals
    }
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

    #[test]
    fn store_reopen_keeps_spend_across_leave_and_rejoin() {
        let mut store = QuotaLedgerStore::new();
        let mut first = store
            .open(AGENT, "w", "archivist", class(2, None), 0)
            .unwrap();
        first.charge(0, call()).unwrap();
        first.charge(1, call()).unwrap();
        store.close(first);

        let mut second = store
            .open(AGENT, "w", "archivist", class(2, None), 2)
            .unwrap();
        assert_eq!(second.spend_at(2).tool_calls, 2);
        let refusal = second.charge(2, call()).unwrap_err();
        assert_eq!(refusal.axis, "tool_calls");
        assert_eq!(refusal.replenishes_at, None);
    }

    #[test]
    fn store_snapshot_survives_a_host_restart() {
        let mut store = QuotaLedgerStore::new();
        let mut ledger = store
            .open(AGENT, "w", "archivist", class(2, None), 0)
            .unwrap();
        ledger.charge(0, call()).unwrap();
        store.close(ledger);

        let snapshot = serde_json::to_string(&store.ledgers()).unwrap();
        let mut restored = QuotaLedgerStore::restore(serde_json::from_str(&snapshot).unwrap());

        let mut rejoined = restored
            .open(AGENT, "w", "archivist", class(2, None), 1)
            .unwrap();
        // Rejoining never grants a fresh budget: the one spent call
        // is still spent, and one more charge exhausts the budget.
        assert_eq!(rejoined.spend_at(1).tool_calls, 1);
        rejoined.charge(1, call()).unwrap();
        assert!(rejoined.charge(2, call()).is_err());
    }

    #[test]
    fn store_reopen_adopts_an_edited_role_without_resetting_spend() {
        let mut store = QuotaLedgerStore::new();
        let mut first = store
            .open(AGENT, "w", "archivist", class(5, None), 0)
            .unwrap();
        first.charge(0, call()).unwrap();
        first.charge(0, call()).unwrap();
        store.close(first);

        let mut second = store
            .open(AGENT, "w", "senior-archivist", class(2, None), 1)
            .unwrap();
        assert_eq!(second.role(), "senior-archivist");
        assert_eq!(second.budget().tool_calls, 2);
        // Two calls were already spent under the old budget; the
        // tightened budget leaves no room.
        assert!(second.charge(1, call()).is_err());
    }

    #[test]
    fn store_owner_views_read_per_agent_and_per_role() {
        let mut store = QuotaLedgerStore::new();
        let mut a = store
            .open([1; 32], "w", "archivist", class(9, None), 0)
            .unwrap();
        let mut b = store
            .open([2; 32], "w", "archivist", class(9, None), 0)
            .unwrap();
        let mut c = store
            .open([3; 32], "w", "courier", class(9, None), 0)
            .unwrap();
        a.charge(0, call()).unwrap();
        b.charge(0, call()).unwrap();
        b.charge(0, call()).unwrap();
        c.charge(0, call()).unwrap();
        store.close(a);
        store.close(b);
        store.close(c);

        assert_eq!(
            store.spend_for_agent(&[1; 32], "w", 0).unwrap().tool_calls,
            1
        );
        assert_eq!(store.spend_for_agent(&[9; 32], "w", 0), None);
        assert_eq!(store.spend_for_agent(&[1; 32], "other", 0), None);

        let totals = store.spend_by_role(0);
        assert_eq!(totals["archivist"].tool_calls, 3);
        assert_eq!(totals["courier"].tool_calls, 1);
    }

    #[test]
    fn store_owner_reads_never_advance_replenishment() {
        let mut store = QuotaLedgerStore::new();
        let mut ledger = store
            .open(AGENT, "w", "archivist", class(2, Some((60, 1))), 0)
            .unwrap();
        ledger.charge(0, call()).unwrap();
        ledger.charge(0, call()).unwrap();
        store.close(ledger);

        // Reads see the post-replenishment spend but must not move
        // the ledger's clock: the tick is still earned by the next
        // charge, not by the read.
        assert_eq!(
            store.spend_for_agent(&AGENT, "w", 60).unwrap().tool_calls,
            1
        );
        let tick = store.get(&AGENT, "w").unwrap().spend_at(119);
        assert_eq!(tick.tool_calls, 1);

        let mut rejoined = store
            .open(AGENT, "w", "archivist", class(2, Some((60, 1))), 119)
            .unwrap();
        rejoined.charge(119, call()).unwrap();
        assert_eq!(rejoined.spend_at(119).tool_calls, 2);
    }

    #[test]
    fn store_refuses_a_second_open_while_checked_out() {
        // The #14526 repro: two live opens for one (agent, world)
        // each carried a full budget -- 4 charges admitted
        // against a budget of 2 -- and the first handle's spend
        // was lost on the second's close. The second open is now
        // refused, so the budget cannot be doubled by a rejoin.
        let mut store = QuotaLedgerStore::new();
        let mut first = store
            .open(AGENT, "w", "archivist", class(2, None), 0)
            .unwrap();
        first.charge(0, call()).unwrap();
        first.charge(0, call()).unwrap();

        let refusal = store
            .open(AGENT, "w", "archivist", class(2, None), 0)
            .unwrap_err();
        assert_eq!(refusal.agent_key, AGENT);
        assert_eq!(refusal.world, "w");
        assert!(store.is_checked_out(&AGENT, "w"));

        // The refusal changed nothing: after the outstanding
        // handle closes, a rejoin reopens its spend -- the
        // budget is spent, and nothing more is admitted.
        store.close(first);
        assert!(!store.is_checked_out(&AGENT, "w"));
        let mut rejoined = store
            .open(AGENT, "w", "archivist", class(2, None), 1)
            .unwrap();
        assert_eq!(rejoined.spend_at(1).tool_calls, 2);
        assert!(rejoined.charge(1, call()).is_err());
    }

    #[test]
    fn store_checkout_is_per_agent_world_pair() {
        let mut store = QuotaLedgerStore::new();
        let _a = store
            .open(AGENT, "w", "archivist", class(2, None), 0)
            .unwrap();
        // Another world for the same agent, and another agent in
        // the same world, are different pairs: both open fine
        // while the first checkout is outstanding.
        let _b = store
            .open(AGENT, "other", "archivist", class(2, None), 0)
            .unwrap();
        let _c = store
            .open([9; 32], "w", "archivist", class(2, None), 0)
            .unwrap();
        assert!(store.is_checked_out(&AGENT, "w"));
        assert!(store.is_checked_out(&AGENT, "other"));
        assert!(store.is_checked_out(&[9; 32], "w"));
    }
}
