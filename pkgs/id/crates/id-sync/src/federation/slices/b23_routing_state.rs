//! B23: routing-state freshness, periodic validation and detection of
//! peer and topology changes (#10251, statements S1.73-S1.76; design
//! "Resource/security invariants").
//!
//! A server may keep routing state (S1.73), but only as signed,
//! expiring, partial observations, never as a trusted global map:
//!
//! - A [`RouteObservation`] is one observer's statement about its own
//!   links: which peers it reached and which it tried and could not
//!   reach, in one scope, when, and until when the statement holds. An
//!   observer can speak only for its own links, so a claim about two
//!   other peers is not expressible.
//! - Observations arrive only as B32 envelopes: [`RoutingState::ingest`]
//!   runs the B32 gate (signature, B05-style signer binding, scope
//!   authorization, freshness, replay) before it decodes anything, then
//!   refuses a poisoned observation: bytes that do not decode exactly,
//!   an observer other than the signer, another scope, a time other
//!   than the envelope's, an expiry not after the observation or past
//!   the policy's maximum lifetime.
//! - Staleness is expected (S1.75). A newer observation from the same
//!   observer replaces the older one; an older or conflicting one never
//!   replaces a newer one. An observation past its expiry is reported
//!   as stale and contributes nothing to the current topology.
//! - [`RoutingState::topology`] is a [`PartialTopology`]: the links that
//!   current observations vouch for, the observers whose statements went
//!   stale, and the configured peers no current observation covers. It
//!   has no notion of completeness; nothing in it says the graph is
//!   whole.
//! - Periodic validation (S1.74): [`RoutingState::validation_due`] names
//!   the configured peers whose own observation is missing, expired or
//!   older than the revalidation interval, so the caller probes them.
//! - Change detection (S1.76): [`changes`] compares two topologies and
//!   reports peers that appeared, peers that vanished (a current
//!   observer reports them unreachable and none reaches them), peers
//!   that were only lost from sight because the statements about them
//!   went stale, observers that went stale, and links gained and lost.
//!
//! Rollback: [`RoutingMode::Static`] ignores dynamic observations and
//! uses the explicit static links; dynamic state still expires and is
//! pruned.
//!
//! Observation payload (integers big-endian, strings as a u32 length
//! and UTF-8 bytes):
//!
//! ```text
//! "id-sync/b23-route-observation/v1" observer scope observed_at:u64
//! expires_at:u64 link_count:u32 (peer status:u8)*
//! ```
//!
//! Links are sorted by peer with no repeats, never name the observer,
//! and status is 1 (reachable) or 2 (unreachable).
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::{BTreeMap, BTreeSet};

use super::b32_ingest_gate::{Envelope, IngestGate, IngressPath, LogicalTime, RejectReason};

/// Domain prefix of an observation payload.
pub const OBSERVATION_DOMAIN: &[u8] = b"id-sync/b23-route-observation/v1";

/// A peer's identity, as bound in the B32 signer table.
pub type PeerId = String;

/// A directed link: (observer, peer).
pub type Link = (PeerId, PeerId);

/// What an observer saw of one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkStatus {
    /// The observer reached the peer.
    Reachable,
    /// The observer tried and could not reach the peer.
    Unreachable,
}

impl LinkStatus {
    const fn tag(self) -> u8 {
        match self {
            Self::Reachable => 1,
            Self::Unreachable => 2,
        }
    }

    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Reachable),
            2 => Some(Self::Unreachable),
            _ => None,
        }
    }
}

/// One observer's signed statement about its own links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteObservation {
    /// Who observed, and signs.
    pub observer: PeerId,
    /// The scope the observation is about.
    pub scope: String,
    /// When it observed, in logical seconds.
    pub observed_at: LogicalTime,
    /// When the statement stops holding.
    pub expires_at: LogicalTime,
    /// What it saw of each peer.
    pub links: BTreeMap<PeerId, LinkStatus>,
}

fn push_str(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    const fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.rest.len() < n {
            return None;
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4)?.try_into().ok().map(u32::from_be_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8)?.try_into().ok().map(u64::from_be_bytes)
    }

    fn string(&mut self) -> Option<String> {
        let len = usize::try_from(self.u32()?).ok()?;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

impl RouteObservation {
    /// The canonical payload bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = OBSERVATION_DOMAIN.to_vec();
        push_str(&mut out, &self.observer);
        push_str(&mut out, &self.scope);
        out.extend_from_slice(&self.observed_at.to_be_bytes());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out.extend_from_slice(
            &u32::try_from(self.links.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for (peer, status) in &self.links {
            push_str(&mut out, peer);
            out.push(status.tag());
        }
        out
    }

    /// Decode canonical payload bytes, refusing anything else: another
    /// domain, trailing bytes, unsorted or repeated peers, a link to the
    /// observer itself, an unknown status or a count that does not fit.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut reader = Reader {
            rest: bytes.strip_prefix(OBSERVATION_DOMAIN)?,
        };
        let observer = reader.string()?;
        let scope = reader.string()?;
        let observed_at = reader.u64()?;
        let expires_at = reader.u64()?;
        let count = reader.u32()?;
        let mut links = BTreeMap::new();
        let mut previous: Option<String> = None;
        for _ in 0..count {
            let peer = reader.string()?;
            let status = LinkStatus::from_tag(*reader.take(1)?.first()?)?;
            if peer == observer || previous.as_ref().is_some_and(|p| *p >= peer) {
                return None;
            }
            previous = Some(peer.clone());
            links.insert(peer, status);
        }
        if !reader.rest.is_empty() || observer.is_empty() {
            return None;
        }
        Some(Self {
            observer,
            scope,
            observed_at,
            expires_at,
            links,
        })
    }
}

/// Where the topology comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    /// Signed, expiring observations.
    Dynamic,
    /// Only the explicit static links (the rollback).
    Static,
}

/// Local routing policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingPolicy {
    /// Dynamic or static.
    pub mode: RoutingMode,
    /// The longest lifetime an observation may claim.
    pub max_ttl: LogicalTime,
    /// How old a peer's own observation may get before it is due for
    /// validation again.
    pub revalidate_after: LogicalTime,
    /// The peers this server expects to know about.
    pub configured_peers: BTreeSet<PeerId>,
    /// The static links used in [`RoutingMode::Static`].
    pub static_links: BTreeSet<Link>,
}

/// Why an observation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The B32 gate refused the envelope.
    Gate(RejectReason),
    /// The payload is not a canonical observation.
    Malformed,
    /// The observation names an observer other than its signer.
    ObserverMismatch,
    /// The observation is for another scope than its envelope or state.
    ScopeMismatch,
    /// The observation time is not the envelope's issue time.
    TimeMismatch,
    /// The expiry is not after the observation time.
    BadExpiry,
    /// The expiry is further out than the policy allows.
    TtlTooLong,
    /// A newer (or the same) observation from this observer is held.
    Superseded,
    /// A different observation with the same time is held.
    Conflict,
}

/// One scope's routing state.
#[derive(Debug, Clone)]
pub struct RoutingState {
    scope: String,
    gate: IngestGate,
    policy: RoutingPolicy,
    latest: BTreeMap<PeerId, RouteObservation>,
}

/// What current observations say, with the gaps named. It never claims
/// to be the whole graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartialTopology {
    /// Links some current statement says are up.
    pub links: BTreeSet<Link>,
    /// Links some current statement says are down.
    pub down: BTreeSet<Link>,
    /// Observers with a current statement.
    pub current_observers: BTreeSet<PeerId>,
    /// Observers whose latest statement has expired.
    pub stale_observers: BTreeSet<PeerId>,
    /// Configured peers that no current statement mentions.
    pub unobserved: BTreeSet<PeerId>,
}

impl PartialTopology {
    /// Peers known to be up: current observers and the peers their
    /// up links reach.
    pub fn reachable(&self) -> BTreeSet<PeerId> {
        let mut out = self.current_observers.clone();
        out.extend(self.links.iter().flat_map(|(a, b)| [a.clone(), b.clone()]));
        out
    }
}

impl RoutingState {
    /// Routing state for `scope`, admitting through `gate` under `policy`.
    pub const fn new(scope: String, gate: IngestGate, policy: RoutingPolicy) -> Self {
        Self {
            scope,
            gate,
            policy,
            latest: BTreeMap::new(),
        }
    }

    /// Switch between dynamic observations and the static links.
    pub const fn set_mode(&mut self, mode: RoutingMode) {
        self.policy.mode = mode;
    }

    /// The observation held for `observer`, current or stale.
    pub fn observation(&self, observer: &str) -> Option<&RouteObservation> {
        self.latest.get(observer)
    }

    /// Admit one observation envelope at `now`.
    pub fn ingest(
        &mut self,
        path: IngressPath,
        envelope: &Envelope,
        now: LogicalTime,
    ) -> Result<(), Refusal> {
        let admission = self
            .gate
            .admit(path, envelope, now)
            .map_err(Refusal::Gate)?;
        let observation = RouteObservation::decode(&envelope.payload).ok_or(Refusal::Malformed)?;
        if observation.observer != admission.signer {
            return Err(Refusal::ObserverMismatch);
        }
        if observation.scope != admission.scope || observation.scope != self.scope {
            return Err(Refusal::ScopeMismatch);
        }
        if observation.observed_at != envelope.issued_at {
            return Err(Refusal::TimeMismatch);
        }
        if observation.expires_at <= observation.observed_at {
            return Err(Refusal::BadExpiry);
        }
        if observation.expires_at - observation.observed_at > self.policy.max_ttl {
            return Err(Refusal::TtlTooLong);
        }
        if let Some(held) = self.latest.get(&observation.observer) {
            if held.observed_at > observation.observed_at || *held == observation {
                return Err(Refusal::Superseded);
            }
            if held.observed_at == observation.observed_at {
                return Err(Refusal::Conflict);
            }
        }
        self.latest
            .insert(observation.observer.clone(), observation);
        Ok(())
    }

    /// The topology at `now`.
    pub fn topology(&self, now: LogicalTime) -> PartialTopology {
        let mut topology = PartialTopology::default();
        match self.policy.mode {
            RoutingMode::Static => topology.links.clone_from(&self.policy.static_links),
            RoutingMode::Dynamic => {
                for (observer, observation) in &self.latest {
                    if now >= observation.expires_at {
                        topology.stale_observers.insert(observer.clone());
                        continue;
                    }
                    topology.current_observers.insert(observer.clone());
                    for (peer, status) in &observation.links {
                        let link = (observer.clone(), peer.clone());
                        match status {
                            LinkStatus::Reachable => topology.links.insert(link),
                            LinkStatus::Unreachable => topology.down.insert(link),
                        };
                    }
                }
            }
        }
        let mentioned: BTreeSet<&PeerId> = topology
            .links
            .iter()
            .chain(&topology.down)
            .flat_map(|(a, b)| [a, b])
            .chain(&topology.current_observers)
            .collect();
        topology.unobserved = self
            .policy
            .configured_peers
            .iter()
            .filter(|peer| !mentioned.contains(peer))
            .cloned()
            .collect();
        topology
    }

    /// Configured peers due for validation at `now`: no observation of
    /// their own, an expired one, or one older than the revalidation
    /// interval. Static mode validates nothing.
    pub fn validation_due(&self, now: LogicalTime) -> BTreeSet<PeerId> {
        if self.policy.mode == RoutingMode::Static {
            return BTreeSet::new();
        }
        self.policy
            .configured_peers
            .iter()
            .filter(|peer| {
                self.latest.get(*peer).is_none_or(|o| {
                    now >= o.expires_at
                        || now >= o.observed_at.saturating_add(self.policy.revalidate_after)
                })
            })
            .cloned()
            .collect()
    }

    /// Drop every observation expired at `now`; returns how many.
    pub fn prune(&mut self, now: LogicalTime) -> usize {
        let before = self.latest.len();
        self.latest.retain(|_, o| now < o.expires_at);
        before - self.latest.len()
    }
}

/// What changed between two topologies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopologyChange {
    /// Peers up now that were not up before.
    pub appeared: BTreeSet<PeerId>,
    /// Peers up before that a current observer now reports down and no
    /// current observer reaches.
    pub vanished: BTreeSet<PeerId>,
    /// Peers up before that nothing current speaks for any more.
    pub lost_sight: BTreeSet<PeerId>,
    /// Observers whose statements went stale.
    pub went_stale: BTreeSet<PeerId>,
    /// Links up now that were not before.
    pub links_added: BTreeSet<Link>,
    /// Links up before that are not now.
    pub links_lost: BTreeSet<Link>,
}

impl TopologyChange {
    /// Whether nothing changed that affects fan-out.
    pub fn is_empty(&self) -> bool {
        self.appeared.is_empty()
            && self.vanished.is_empty()
            && self.lost_sight.is_empty()
            && self.went_stale.is_empty()
            && self.links_added.is_empty()
            && self.links_lost.is_empty()
    }
}

/// Compare two topologies.
pub fn changes(before: &PartialTopology, after: &PartialTopology) -> TopologyChange {
    let up_before = before.reachable();
    let up_after = after.reachable();
    let reported_down: BTreeSet<&PeerId> = after.down.iter().map(|(_, peer)| peer).collect();
    let gone: BTreeSet<PeerId> = up_before.difference(&up_after).cloned().collect();
    let (vanished, lost_sight) = gone
        .into_iter()
        .partition(|peer| reported_down.contains(peer));
    TopologyChange {
        appeared: up_after.difference(&up_before).cloned().collect(),
        vanished,
        lost_sight,
        went_stale: after
            .stale_observers
            .difference(&before.stale_observers)
            .cloned()
            .collect(),
        links_added: after.links.difference(&before.links).cloned().collect(),
        links_lost: before.links.difference(&after.links).cloned().collect(),
    }
}
