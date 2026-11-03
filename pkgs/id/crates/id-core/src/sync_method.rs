//! Which sync method a session uses (P3.3).
//!
//! Two methods reconcile two peers' event sets. `LogSync` sends each chain's latest position, so it
//! costs two round trips and control bytes that grow with the chain count. Range sync summarizes
//! sorted ranges of event IDs, so it costs seven to nine round trips and control bytes that grow
//! with the difference. Round 2 measured the crossover at about 3.5 percent of the chain count
//! (explainers section 5, range-config results Q3). The rule:
//!
//! - **`LogSync`** when the estimated difference `D` exceeds 0.035 times the chain count `C`, or
//!   when the peer is not yet bootstrapped (no sync with it has completed).
//! - **Range** otherwise.
//!
//! The initiator chooses, and sends its choice in a [`Request`]. The responder answers with an
//! [`Answer`]: it agrees to run that method, or refuses it. It never chooses for the session.
//!
//! The rule is written against [`SyncStats`], so range sync's index can supply the numbers when
//! it merges. [`SyncSupport`] is the same kind of seam for the answer: range sync adds itself when
//! it merges, and until then a responder refuses range requests.

use std::fmt::Debug;

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

/// The sync methods a session can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMethod {
    /// `LogSync`: each chain's latest position, then the events the other side lacks.
    Log,
    /// Range sync (P3.1): sorted ranges of event IDs, summarized and split.
    Range,
}

/// The `LogSync` crossover, in thousandths: `LogSync` when `1000 * D > CROSSOVER_PER_THOUSAND * C`.
/// 35 is 0.035, measured in round 2. The integer form is exact, so 0.035 has no rounding edge.
pub const CROSSOVER_PER_THOUSAND: u128 = 35;

/// What the initiator knows about one peer. The rule needs these three numbers.
pub trait SyncStats {
    /// The chains (one per author) the initiator holds for the topic. This is `C`.
    fn chain_count(&self) -> u64;

    /// The estimated number of events the two sides disagree on. This is `D`. It is a lower
    /// bound: the peer's new events are unknown until the first exchange. A margin is advisable.
    fn estimated_difference(&self) -> u64;

    /// When the last sync with this peer completed, in Unix milliseconds. `None` means the peer is
    /// not bootstrapped: no sync has ever completed with it.
    fn last_completed_sync_ms(&self) -> Option<u64>;
}

/// The method a session with `stats` uses.
///
/// `LogSync` when the peer is not bootstrapped, or when `D` is above `0.035 * C`. Range otherwise.
/// The comparison is exact: at `D == 0.035 * C` the rule picks range.
pub fn choose(stats: &impl SyncStats) -> SyncMethod {
    if stats.last_completed_sync_ms().is_none() {
        return SyncMethod::Log;
    }
    let chains = u128::from(stats.chain_count());
    let difference = u128::from(stats.estimated_difference());
    if difference * 1000 > chains * CROSSOVER_PER_THOUSAND {
        SyncMethod::Log
    } else {
        SyncMethod::Range
    }
}

/// [`SyncStats`] as plain values, for a caller that already has them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Estimate {
    /// The chains the initiator holds for the topic (`C`).
    pub chains: u64,
    /// The estimated difference (`D`).
    pub difference: u64,
    /// When the last sync with the peer completed, or `None` before the first.
    pub last_completed_sync_ms: Option<u64>,
}

impl SyncStats for Estimate {
    fn chain_count(&self) -> u64 {
        self.chains
    }

    fn estimated_difference(&self) -> u64 {
        self.difference
    }

    fn last_completed_sync_ms(&self) -> Option<u64> {
        self.last_completed_sync_ms
    }
}

/// The methods this node can run. A responder refuses any request for another method.
pub trait SyncSupport: Debug + Send + Sync {
    /// Whether this node runs `method`.
    fn supports(&self, method: SyncMethod) -> bool;
}

/// Runs `LogSync` only. Range sync (P3.1) adds itself here when it merges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogSyncOnly;

/// Runs both methods: `LogSync`, and range sync as a method of the same handshake
/// session (#179, wired in `log_sync::run_agreed`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogAndRange;

impl SyncSupport for LogAndRange {
    fn supports(&self, _method: SyncMethod) -> bool {
        true
    }
}

impl SyncSupport for LogSyncOnly {
    fn supports(&self, method: SyncMethod) -> bool {
        method == SyncMethod::Log
    }
}

/// The initiator's first frame: the topic it syncs, and the method it chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The group hash whose logs are offered.
    pub topic: Hash,
    /// The method the initiator chose with [`choose`].
    pub method: SyncMethod,
}

/// The responder's answer to a [`Request`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    /// The responder runs the method the request named. The method is echoed, so the initiator
    /// can check that both sides agree.
    Agree(SyncMethod),
    /// The responder does not run the method. Nothing else is sent on the stream.
    Refuse,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The estimate at `chains` chains, `difference` events apart, with a completed sync.
    fn synced(chains: u64, difference: u64) -> Estimate {
        Estimate {
            chains,
            difference,
            last_completed_sync_ms: Some(1),
        }
    }

    /// `numerator` per thousand of `chains`, rounded down. The chain counts below divide evenly.
    fn per_thousand(chains: u64, numerator: u64) -> u64 {
        chains * numerator / 1000
    }

    #[test]
    fn test_the_rule_is_exact_at_0_034_0_035_and_0_036_of_the_chains() {
        for chains in [1000_u64, 2000, 10_000] {
            // Range below the crossover, and at it: "exceeds" is strict.
            assert_eq!(
                choose(&synced(chains, per_thousand(chains, 34))),
                SyncMethod::Range,
                "0.034 of {chains} chains"
            );
            assert_eq!(
                choose(&synced(chains, per_thousand(chains, 35))),
                SyncMethod::Range,
                "0.035 of {chains} chains is not above the crossover"
            );
            // LogSync above it.
            assert_eq!(
                choose(&synced(chains, per_thousand(chains, 36))),
                SyncMethod::Log,
                "0.036 of {chains} chains"
            );
            assert_eq!(
                choose(&synced(chains, per_thousand(chains, 35) + 1)),
                SyncMethod::Log,
                "one event above 0.035 of {chains} chains"
            );
        }
    }

    #[test]
    fn test_an_unbootstrapped_peer_is_always_log() {
        let never_synced = Estimate {
            chains: 1000,
            difference: 0,
            last_completed_sync_ms: None,
        };
        assert_eq!(choose(&never_synced), SyncMethod::Log);
    }

    #[test]
    fn test_an_empty_chain_set_is_range_only_with_no_difference() {
        assert_eq!(choose(&synced(0, 0)), SyncMethod::Range);
        assert_eq!(choose(&synced(0, 1)), SyncMethod::Log);
    }

    #[test]
    fn test_the_rule_does_not_overflow_at_the_extremes() {
        assert_eq!(
            choose(&synced(u64::MAX, u64::MAX)),
            SyncMethod::Log,
            "1000 * max is above 35 * max"
        );
        assert_eq!(choose(&synced(u64::MAX, 0)), SyncMethod::Range);
    }

    #[test]
    fn test_the_request_and_answer_round_trip_on_the_wire() {
        let request = Request {
            topic: Hash::digest([7_u8]),
            method: SyncMethod::Range,
        };
        let bytes = postcard::to_allocvec(&request).unwrap();
        assert_eq!(postcard::from_bytes::<Request>(&bytes).unwrap(), request);

        for answer in [Answer::Agree(SyncMethod::Log), Answer::Refuse] {
            let bytes = postcard::to_allocvec(&answer).unwrap();
            assert_eq!(postcard::from_bytes::<Answer>(&bytes).unwrap(), answer);
        }
    }

    #[test]
    fn test_log_sync_only_supports_log() {
        assert!(LogSyncOnly.supports(SyncMethod::Log));
        assert!(!LogSyncOnly.supports(SyncMethod::Range));
    }

    #[test]
    fn test_log_and_range_supports_both() {
        assert!(LogAndRange.supports(SyncMethod::Log));
        assert!(LogAndRange.supports(SyncMethod::Range));
    }
}
