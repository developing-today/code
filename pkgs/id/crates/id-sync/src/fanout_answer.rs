//! The federated query fan-out answer envelope: completeness, unanswered
//! peers and per-hop status (#11414).
//!
//! Design: `doc/2026-11-17T15-40-00Z_design_id_federated_query_fanout/` §6.3
//! and §7.1–§7.4. A fan-out answer says what it covers. A remote that is slow
//! or down, or a deadline that cuts the fan-out short, never leaves the
//! requester with an answer that looks complete:
//!
//! - every remote that was asked gets one [`ProvenanceEntry`] whose
//!   [`HopStatus`] explains its answer or its absence;
//! - a peer that answers before the deadline keeps its signed response, and a
//!   peer that has not answered by then is recorded as [`HopStatus::Timeout`];
//! - [`Completeness::Partial`] is the answer whenever an eligible remote did
//!   not answer, and [`Completeness::Complete`] is an explicit claim scoped to
//!   the eligible remotes and the policy at the time of the query, never a
//!   claim of global completeness.
//!
//! Policy exclusions are not absences: a hop recorded
//! [`HopStatus::Ineligible`] (the peer class does not allow query-out, §3.1) or
//! [`HopStatus::NotPermitted`] (a relay the grant does not permit, §4.3) is
//! outside the scope the claim covers, so it is listed in provenance but does
//! not make the answer partial.
//!
//! Compatibility (§8): with no remotes the answer is the v1 local answer. The
//! three new fields are absent from its serialized form, and older readers
//! that ignore unknown fields keep working.
//!
//! Not here: the config keys (#9111), how provenance is shown to the requester
//! (#10249), and what a completeness claim can prove (B30, #10279).

use serde::{Deserialize, Serialize};

/// What one hop of a fan-out did (§6.3). Every value but [`Self::Answered`]
/// and [`Self::Empty`] explains an absence and is not a verdict: a
/// [`Self::Timeout`] does not mean the peer lacks the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HopStatus {
    /// The peer returned at least one result in a signed response.
    Answered,
    /// The peer returned a signed response with no results: absent at that
    /// peer, at its policy and time (§3.3).
    Empty,
    /// The hop was not taken because the relay grant does not permit it.
    NotPermitted,
    /// The peer was skipped because its class does not allow query-out, or it
    /// is disabled or revoked.
    Ineligible,
    /// The peer had not answered when the deadline passed.
    Timeout,
    /// The peer could not be reached.
    Unavailable,
    /// The peer refused the query.
    Refused,
    /// A budget ran out before the peer was asked to completion.
    BudgetExhausted,
    /// The peer's answer arrived but was dropped before it reached the answer.
    ResultEvicted,
}

impl HopStatus {
    /// Whether the peer gave a signed response.
    #[must_use]
    pub const fn is_answer(self) -> bool {
        matches!(self, Self::Answered | Self::Empty)
    }

    /// Whether the hop is outside the scope a completeness claim covers: a
    /// policy exclusion rather than an absence.
    #[must_use]
    pub const fn is_policy_exclusion(self) -> bool {
        matches!(self, Self::Ineligible | Self::NotPermitted)
    }

    /// Whether the hop leaves the answer partial: an eligible remote that did
    /// not answer.
    #[must_use]
    pub const fn leaves_partial(self) -> bool {
        !self.is_answer() && !self.is_policy_exclusion()
    }
}

/// Whether a fan-out answer claims to cover its scope (§7.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    /// Every eligible remote that was asked answered, or the requested limit
    /// was met before the list ran out. Scoped to the eligible remotes and the
    /// policy at the time of the query.
    Complete,
    /// At least one eligible remote did not answer. The safe default.
    Partial,
}

/// One hop that was asked, or the local store at hop 0 (§6.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    /// 0 for the local store, then one per remote in the order they were
    /// planned.
    pub hop_index: u32,
    /// The responder or relay that handled this hop.
    pub peer_id: String,
    /// Peer IDs from the origin to this hop.
    pub path: Vec<String>,
    /// What the hop did.
    pub status: HopStatus,
    /// The responder's signature, when a signed response was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responder_signature: Option<String>,
    /// The relay's forwarding receipt, when a relay handled the hop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_digest: Option<String>,
    /// Unix milliseconds the status was recorded.
    pub generated_at: u64,
}

/// A query answer. With no remotes it is the v1 local answer: only `results`
/// is set, and the other fields are left out of its serialized form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanoutAnswer<R> {
    /// The local results, then each answering remote's results in hop order.
    pub results: Vec<R>,
    /// The completeness claim. `None` only for a local-only answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completeness: Option<Completeness>,
    /// One entry per hop asked, the local store first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<ProvenanceEntry>,
    /// The eligible remotes that did not answer, in hop order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unanswered_peers: Vec<String>,
}

/// Why the collector refused to record an outcome.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FanoutError {
    /// The peer is not a planned remote of this fan-out.
    #[error("peer {0} is not a remote of this fan-out")]
    UnknownPeer(String),
    /// The peer's outcome was already recorded.
    #[error("peer {0} already has a recorded outcome")]
    AlreadyRecorded(String),
    /// A peer was planned twice.
    #[error("peer {0} is already a remote of this fan-out")]
    DuplicatePeer(String),
    /// `answered` and `empty` are recorded with the signed response, through
    /// [`FanoutCollector::record_answer`].
    #[error("status {0:?} needs the signed response; record it with record_answer")]
    StatusNeedsResponse(HopStatus),
}

#[derive(Debug)]
struct Recorded<R> {
    status: HopStatus,
    results: Vec<R>,
    responder_signature: Option<String>,
    receipt_digest: Option<String>,
    at: u64,
}

#[derive(Debug)]
struct Remote<R> {
    peer_id: String,
    path: Vec<String>,
    recorded: Option<Recorded<R>>,
}

/// Collects one fan-out's outcomes against its deadline and builds the answer
/// (§7.1–§7.3).
#[derive(Debug)]
pub struct FanoutCollector<R> {
    local_peer: String,
    local: Vec<R>,
    local_at: u64,
    deadline: u64,
    remotes: Vec<Remote<R>>,
}

impl<R> FanoutCollector<R> {
    /// A fan-out whose local store returned `local` at `local_at`, with a
    /// deadline of `deadline` (unix milliseconds). The deadline is never
    /// extended.
    #[must_use]
    pub const fn new(local_peer: String, local: Vec<R>, local_at: u64, deadline: u64) -> Self {
        Self {
            local_peer,
            local,
            local_at,
            deadline,
            remotes: Vec::new(),
        }
    }

    /// Plans one remote, reached along `path` from the origin. Remotes are
    /// asked and reported in the order they are planned.
    ///
    /// # Errors
    ///
    /// [`FanoutError::DuplicatePeer`] when the peer is already planned.
    pub fn plan(&mut self, peer_id: &str, path: Vec<String>) -> Result<(), FanoutError> {
        if self.remotes.iter().any(|remote| remote.peer_id == peer_id) {
            return Err(FanoutError::DuplicatePeer(peer_id.to_owned()));
        }
        self.remotes.push(Remote {
            peer_id: peer_id.to_owned(),
            path,
            recorded: None,
        });
        Ok(())
    }

    fn slot(&mut self, peer_id: &str) -> Result<&mut Remote<R>, FanoutError> {
        let remote = self
            .remotes
            .iter_mut()
            .find(|remote| remote.peer_id == peer_id)
            .ok_or_else(|| FanoutError::UnknownPeer(peer_id.to_owned()))?;
        if remote.recorded.is_some() {
            return Err(FanoutError::AlreadyRecorded(peer_id.to_owned()));
        }
        Ok(remote)
    }

    /// Records a peer's signed response, received at `at`. A response that
    /// arrives at or after the deadline is not kept: the hop is recorded as
    /// [`HopStatus::Timeout`], since the answer was already due.
    ///
    /// # Errors
    ///
    /// [`FanoutError::UnknownPeer`] or [`FanoutError::AlreadyRecorded`].
    pub fn record_answer(
        &mut self,
        peer_id: &str,
        results: Vec<R>,
        responder_signature: String,
        receipt_digest: Option<String>,
        at: u64,
    ) -> Result<(), FanoutError> {
        let deadline = self.deadline;
        let remote = self.slot(peer_id)?;
        remote.recorded = Some(if at >= deadline {
            Recorded {
                status: HopStatus::Timeout,
                results: Vec::new(),
                responder_signature: None,
                receipt_digest: None,
                at: deadline,
            }
        } else {
            Recorded {
                status: if results.is_empty() {
                    HopStatus::Empty
                } else {
                    HopStatus::Answered
                },
                results,
                responder_signature: Some(responder_signature),
                receipt_digest,
                at,
            }
        });
        Ok(())
    }

    /// Records why a peer gave no answer.
    ///
    /// # Errors
    ///
    /// [`FanoutError::StatusNeedsResponse`] for `answered` or `empty`, and
    /// [`FanoutError::UnknownPeer`] or [`FanoutError::AlreadyRecorded`].
    pub fn record_status(
        &mut self,
        peer_id: &str,
        status: HopStatus,
        at: u64,
    ) -> Result<(), FanoutError> {
        if status.is_answer() {
            return Err(FanoutError::StatusNeedsResponse(status));
        }
        let remote = self.slot(peer_id)?;
        remote.recorded = Some(Recorded {
            status,
            results: Vec::new(),
            responder_signature: None,
            receipt_digest: None,
            at,
        });
        Ok(())
    }

    /// The answer once the list is exhausted or the deadline has passed. A
    /// planned remote with no recorded outcome is recorded as
    /// [`HopStatus::Timeout`] at the deadline.
    #[must_use]
    pub fn finish(self) -> FanoutAnswer<R> {
        self.build(false)
    }

    /// The answer when the requested limit was met before the list ran out
    /// (`stop_at_limit`, §3.3). Remotes never asked are left out: they were
    /// not part of this query, so they neither appear in provenance nor make
    /// the answer partial.
    #[must_use]
    pub fn finish_at_limit(self) -> FanoutAnswer<R> {
        self.build(true)
    }

    fn build(self, at_limit: bool) -> FanoutAnswer<R> {
        let Self {
            local_peer,
            mut local,
            local_at,
            deadline,
            remotes,
        } = self;
        if remotes.is_empty() {
            return FanoutAnswer {
                results: local,
                completeness: None,
                provenance: Vec::new(),
                unanswered_peers: Vec::new(),
            };
        }
        let mut provenance = vec![ProvenanceEntry {
            hop_index: 0,
            peer_id: local_peer,
            path: Vec::new(),
            status: if local.is_empty() {
                HopStatus::Empty
            } else {
                HopStatus::Answered
            },
            responder_signature: None,
            receipt_digest: None,
            generated_at: local_at,
        }];
        let mut unanswered_peers = Vec::new();
        let mut hop_index = 0_u32;
        for remote in remotes {
            let recorded = match remote.recorded {
                Some(recorded) => recorded,
                None if at_limit => continue,
                None => Recorded {
                    status: HopStatus::Timeout,
                    results: Vec::new(),
                    responder_signature: None,
                    receipt_digest: None,
                    at: deadline,
                },
            };
            hop_index = hop_index.saturating_add(1);
            if recorded.status.leaves_partial() {
                unanswered_peers.push(remote.peer_id.clone());
            }
            local.extend(recorded.results);
            provenance.push(ProvenanceEntry {
                hop_index,
                peer_id: remote.peer_id,
                path: remote.path,
                status: recorded.status,
                responder_signature: recorded.responder_signature,
                receipt_digest: recorded.receipt_digest,
                generated_at: recorded.at,
            });
        }
        let completeness = if unanswered_peers.is_empty() {
            Completeness::Complete
        } else {
            Completeness::Partial
        };
        FanoutAnswer {
            results: local,
            completeness: Some(completeness),
            provenance,
            unanswered_peers,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Completeness, FanoutAnswer, FanoutCollector, FanoutError, HopStatus};

    const DEADLINE: u64 = 5_000;

    fn path(hops: &[&str]) -> Vec<String> {
        hops.iter().map(|hop| (*hop).to_owned()).collect()
    }

    fn three_peers() -> FanoutCollector<String> {
        let mut fanout = FanoutCollector::new("me".to_owned(), vec!["l1".to_owned()], 10, DEADLINE);
        fanout.plan("slow", path(&["me", "slow"])).unwrap();
        fanout.plan("down", path(&["me", "down"])).unwrap();
        fanout.plan("good", path(&["me", "relay", "good"])).unwrap();
        fanout
    }

    #[test]
    fn golden_slow_and_down_peers_leave_a_partial_answer() {
        let mut fanout = three_peers();
        fanout
            .record_status("down", HopStatus::Unavailable, 40)
            .unwrap();
        fanout
            .record_answer(
                "good",
                vec!["g1".to_owned(), "g2".to_owned()],
                "sig-good".to_owned(),
                Some("receipt-relay".to_owned()),
                900,
            )
            .unwrap();
        // "slow" never answers before the deadline.
        let answer = fanout.finish();
        let golden = serde_json::json!({
            "results": ["l1", "g1", "g2"],
            "completeness": "partial",
            "provenance": [
                {"hop_index": 0, "peer_id": "me", "path": [], "status": "answered", "generated_at": 10},
                {"hop_index": 1, "peer_id": "slow", "path": ["me", "slow"], "status": "timeout", "generated_at": 5000},
                {"hop_index": 2, "peer_id": "down", "path": ["me", "down"], "status": "unavailable", "generated_at": 40},
                {"hop_index": 3, "peer_id": "good", "path": ["me", "relay", "good"], "status": "answered",
                 "responder_signature": "sig-good", "receipt_digest": "receipt-relay", "generated_at": 900}
            ],
            "unanswered_peers": ["slow", "down"]
        });
        assert_eq!(serde_json::to_value(&answer).unwrap(), golden);
        let round: FanoutAnswer<String> = serde_json::from_value(golden).unwrap();
        assert_eq!(round, answer);
    }

    #[test]
    fn golden_all_peers_answer_in_time_is_complete() {
        let mut fanout = three_peers();
        for (peer, at) in [("slow", 4_999), ("down", 30), ("good", 20)] {
            fanout
                .record_answer(peer, Vec::new(), format!("sig-{peer}"), None, at)
                .unwrap();
        }
        let answer = fanout.finish();
        assert_eq!(answer.completeness, Some(Completeness::Complete));
        assert_eq!(answer.unanswered_peers, [] as [String; 0]);
        assert_eq!(answer.results, ["l1"]);
        assert_eq!(answer.provenance.len(), 4);
        assert!(
            answer.provenance[1..].iter().all(
                |entry| entry.status == HopStatus::Empty && entry.responder_signature.is_some()
            )
        );
    }

    #[test]
    fn an_answer_at_the_deadline_is_a_timeout_and_its_results_are_not_kept() {
        let mut fanout = three_peers();
        fanout
            .record_answer(
                "slow",
                vec!["late".to_owned()],
                "sig".to_owned(),
                None,
                DEADLINE,
            )
            .unwrap();
        let answer = fanout.finish();
        assert!(!answer.results.contains(&"late".to_owned()));
        assert_eq!(answer.provenance[1].status, HopStatus::Timeout);
        assert_eq!(answer.provenance[1].responder_signature, None);
        assert_eq!(answer.unanswered_peers, ["slow", "down", "good"]);
    }

    #[test]
    fn policy_exclusions_are_listed_but_do_not_make_the_answer_partial() {
        let mut fanout = three_peers();
        fanout
            .record_status("slow", HopStatus::Ineligible, 1)
            .unwrap();
        fanout
            .record_status("down", HopStatus::NotPermitted, 2)
            .unwrap();
        fanout
            .record_answer("good", vec!["g".to_owned()], "sig".to_owned(), None, 3)
            .unwrap();
        let answer = fanout.finish();
        assert_eq!(answer.completeness, Some(Completeness::Complete));
        assert_eq!(answer.unanswered_peers, [] as [String; 0]);
        assert_eq!(answer.provenance.len(), 4);
    }

    #[test]
    fn every_other_absence_makes_the_answer_partial() {
        for status in [
            HopStatus::Timeout,
            HopStatus::Unavailable,
            HopStatus::Refused,
            HopStatus::BudgetExhausted,
            HopStatus::ResultEvicted,
        ] {
            let mut fanout =
                FanoutCollector::new("me".to_owned(), Vec::<String>::new(), 0, DEADLINE);
            fanout.plan("p", path(&["me", "p"])).unwrap();
            fanout.record_status("p", status, 1).unwrap();
            let answer = fanout.finish();
            assert_eq!(
                answer.completeness,
                Some(Completeness::Partial),
                "{status:?}"
            );
            assert_eq!(answer.unanswered_peers, ["p"], "{status:?}");
        }
    }

    #[test]
    fn stopping_at_the_limit_leaves_unasked_remotes_out() {
        let mut fanout = three_peers();
        fanout
            .record_answer("slow", vec!["s".to_owned()], "sig".to_owned(), None, 5)
            .unwrap();
        let answer = fanout.finish_at_limit();
        assert_eq!(answer.completeness, Some(Completeness::Complete));
        assert_eq!(answer.provenance.len(), 2);
        assert_eq!(answer.results, ["l1", "s"]);
    }

    #[test]
    fn a_local_only_answer_is_the_v1_answer() {
        let fanout = FanoutCollector::new("me".to_owned(), vec![1_u32, 2], 0, DEADLINE);
        let answer = fanout.finish();
        assert_eq!(answer.completeness, None);
        assert_eq!(
            serde_json::to_value(&answer).unwrap(),
            serde_json::json!({"results": [1, 2]})
        );
        let v1: FanoutAnswer<u32> = serde_json::from_str(r#"{"results":[1,2]}"#).unwrap();
        assert_eq!(v1, answer);
    }

    #[test]
    fn misuse_is_refused() {
        let mut fanout = three_peers();
        assert_eq!(
            fanout.plan("slow", Vec::new()),
            Err(FanoutError::DuplicatePeer("slow".to_owned()))
        );
        assert_eq!(
            fanout.record_status("nobody", HopStatus::Refused, 1),
            Err(FanoutError::UnknownPeer("nobody".to_owned()))
        );
        assert_eq!(
            fanout.record_status("slow", HopStatus::Answered, 1),
            Err(FanoutError::StatusNeedsResponse(HopStatus::Answered))
        );
        fanout.record_status("slow", HopStatus::Refused, 1).unwrap();
        assert_eq!(
            fanout.record_answer("slow", Vec::new(), "sig".to_owned(), None, 2),
            Err(FanoutError::AlreadyRecorded("slow".to_owned()))
        );
    }
}
