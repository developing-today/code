//! Capability negotiation and downgrade rejection (slice 9111-04,
//! issue #10195; parent #9111; design
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`,
//! section "Failure, recovery, and version rules"; statements S1.118,
//! S7.10, S7.11, S7.13, S7.16).
//!
//! The design rule, in one paragraph: "Port/protocol negotiation is
//! signed and identity-bound. Negotiate highest mutually supported
//! allowlisted version. Reject downgrade that removes a required
//! signature, validation, authorization, replay, tenant, or operation
//! capability. Never fall back silently." This module is the state
//! machine that enforces it for one session with one peer:
//!
//! - The local side is a [`NegotiationPolicy`]: an allowlist of protocol
//!   versions (normally the resolved config's `protocol_versions`), a
//!   security floor version, and the locally supported capabilities,
//!   each classified [`CapabilityClass::Required`] (security/correctness
//!   semantics whose removal is a downgrade) or
//!   [`CapabilityClass::Optional`] (additive; negotiating one is
//!   minor-compatible). [`NegotiationPolicy::from_resolved_config`]
//!   seeds the required set from the config contract's fixed gates
//!   ([`id_config::FIXED_GATE_IDS`]): the gates are mechanism, so a
//!   negotiation that drops one is a downgrade by definition.
//! - The peer's side is a [`PeerOffer`]: the peer identity it claims,
//!   the protocol versions it supports, the capabilities it offers, and
//!   a round number. Offers are evaluated in strict round order; a
//!   stale or replayed round is rejected, and every *evaluated* offer
//!   (well-formed, from the bound peer, in round) consumes its round
//!   whether it is accepted or not.
//! - Selection is deterministic: the highest version in the
//!   intersection of the peer's versions and the local allowlist, and
//!   the locally supported capabilities the peer also offers. The
//!   outcome must reach the floor version and include every required
//!   capability, or the offer is rejected as a downgrade.
//! - A rejected *initial* offer closes the session
//!   ([`NegotiationError::SessionClosed`] thereafter): a failed
//!   negotiation is never resumed at weaker terms — "never fall back
//!   silently". A rejected *renegotiation* offer leaves the established
//!   session untouched: per the design's rollback rule, the parties
//!   continue only with the previously allowed protocol version.
//! - A renegotiation whose outcome would be weaker than the
//!   established session — a lower version, or any established
//!   capability dropped — is a downgrade and is rejected; upgrades
//!   (higher version, superset capabilities) are accepted and become
//!   the new baseline.
//!
//! Every acceptance produces a [`NegotiationTranscript`]: the
//! identity-bound record of what was offered and what was selected.
//! Its [`NegotiationTranscript::canonical_bytes`] are the exact bytes a
//! signature must cover for the negotiation to be "signed" in the
//! design's sense; producing and verifying that signature belongs to
//! the envelope layer (#9110/#9083 contracts), which is why this leaf
//! module pins the bytes and the binding checks but defines no
//! cryptography of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Domain prefix of the canonical negotiation transcript bytes. The
/// prefix is part of the signed bytes, so a transcript signature can
/// never be replayed as a signature over some other record shape.
pub const TRANSCRIPT_DOMAIN: &str = "id-config/negotiation-transcript/v1";

/// A protocol version, `<family>/<major>[.<minor>]` (e.g. `id/2.1`).
///
/// Versions are ordered by `(family, major, minor)` — numerically, so
/// `id/2.10` is newer than `id/2.9` — and two versions from different
/// families are never a match. The canonical string form always
/// carries both components (`id/2` canonicalizes to `id/2.0`), so
/// equality, ordering, and serialization all agree on one form.
/// Parsing is fail-closed: an empty family, a missing or non-numeric
/// component, extra components, or whitespace anywhere rejects the
/// string ([`NegotiationError::MalformedVersion`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    family: String,
    major: u32,
    minor: u32,
}

impl ProtocolVersion {
    /// Parse a protocol version string.
    ///
    /// # Errors
    /// Returns [`NegotiationError::MalformedVersion`] for any string
    /// that is not exactly `<family>/<major>` or
    /// `<family>/<major>.<minor>` with a non-empty, whitespace-free
    /// family and numeric components.
    pub fn parse(raw: &str) -> Result<Self, NegotiationError> {
        let malformed = || NegotiationError::MalformedVersion(raw.to_owned());
        if raw.is_empty() || raw.chars().any(char::is_whitespace) {
            return Err(malformed());
        }
        let (family, rest) = raw.split_once('/').ok_or_else(malformed)?;
        if family.is_empty() || rest.contains('/') {
            return Err(malformed());
        }
        let mut components = rest.split('.');
        let major_raw = components.next().ok_or_else(malformed)?;
        let minor_raw = components.next().unwrap_or("0");
        if components.next().is_some() {
            return Err(malformed());
        }
        let parse_component = |text: &str| -> Result<u32, NegotiationError> {
            if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
                return Err(malformed());
            }
            text.parse::<u32>().map_err(|_| malformed())
        };
        Ok(Self {
            family: family.to_owned(),
            major: parse_component(major_raw)?,
            minor: parse_component(minor_raw)?,
        })
    }

    /// The protocol family (the part before `/`).
    #[must_use]
    pub fn family(&self) -> &str {
        &self.family
    }

    /// The major version component. Changing interpretation, ordering,
    /// or mandatory guarantees is a major version (design version
    /// rules).
    #[must_use]
    pub const fn major(&self) -> u32 {
        self.major
    }

    /// The minor version component. Additive optional capabilities are
    /// minor-compatible.
    #[must_use]
    pub const fn minor(&self) -> u32 {
        self.minor
    }

    /// The canonical string form, `<family>/<major>.<minor>`.
    #[must_use]
    pub fn canonical(&self) -> String {
        format!("{}/{}.{}", self.family, self.major, self.minor)
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.canonical())
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Whether a capability is required security/correctness semantics or
/// an additive optional extra. The classification is local policy: it
/// states what *this* side refuses to negotiate away, regardless of
/// how the peer describes the capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityClass {
    /// Removing this capability weakens security or correctness
    /// semantics; an outcome without it is a downgrade and rejects.
    Required,
    /// Additive capability; selecting or dropping it never weakens the
    /// mandatory semantics (minor-compatible, per the design's
    /// version rules).
    Optional,
}

/// The local side of a negotiation: what this node allows, the lowest
/// version it will settle for, and what it can do.
///
/// The required capability set is exactly the capabilities classified
/// [`CapabilityClass::Required`]; it is never a separate list that
/// could disagree with the classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiationPolicy {
    /// Identity of the local party. Bound into every transcript.
    pub local_identity: String,
    /// Protocol versions this side allows (normally the resolved
    /// config's allowlist). Non-empty; negotiation selects only from
    /// this set's intersection with the peer's offer.
    pub allowlist: BTreeSet<ProtocolVersion>,
    /// Security floor: an outcome below this version is a downgrade,
    /// even if the version is allowlisted. Must itself be allowlisted
    /// — a floor nothing can reach would make the policy a silent
    /// deny-all.
    pub floor: ProtocolVersion,
    /// Locally supported capabilities by name, with their class.
    pub capabilities: BTreeMap<String, CapabilityClass>,
}

impl NegotiationPolicy {
    /// Build and validate a policy from raw parts.
    ///
    /// # Errors
    /// Returns [`NegotiationError::EmptyLocalIdentity`] for an empty
    /// identity, [`NegotiationError::EmptyAllowlist`] for an empty
    /// allowlist, [`NegotiationError::FloorNotAllowlisted`] when the
    /// floor is not in the allowlist, and
    /// [`NegotiationError::EmptyCapabilityName`] for an empty
    /// capability name.
    pub fn new(
        local_identity: &str,
        allowlist: BTreeSet<ProtocolVersion>,
        floor: ProtocolVersion,
        capabilities: BTreeMap<String, CapabilityClass>,
    ) -> Result<Self, NegotiationError> {
        if local_identity.trim().is_empty() {
            return Err(NegotiationError::EmptyLocalIdentity);
        }
        if allowlist.is_empty() {
            return Err(NegotiationError::EmptyAllowlist);
        }
        if !allowlist.contains(&floor) {
            return Err(NegotiationError::FloorNotAllowlisted {
                floor: floor.canonical(),
            });
        }
        if capabilities.keys().any(|name| name.trim().is_empty()) {
            return Err(NegotiationError::EmptyCapabilityName);
        }
        Ok(Self {
            local_identity: local_identity.to_owned(),
            allowlist,
            floor,
            capabilities,
        })
    }

    /// Derive the policy from a resolved configuration (slice 9111-01):
    /// the allowlist is the resolved `protocol_versions`, and the
    /// required capabilities are the config contract's fixed gates —
    /// mechanism that no negotiation may bargain away. Additional
    /// optional capabilities the caller supports are layered on top.
    ///
    /// # Errors
    /// Returns [`NegotiationError::MalformedVersion`] if an allowlist
    /// entry or the floor does not parse,
    /// [`NegotiationError::CapabilityClassConflict`] if an optional
    /// capability names a fixed gate (a gate is required by
    /// definition; relabeling it optional is a policy bug, not a
    /// choice), plus every error [`NegotiationPolicy::new`] returns.
    pub fn from_resolved_config(
        resolved: &id_config::ResolvedConfig,
        local_identity: &str,
        floor: &str,
        optional_capabilities: &[&str],
    ) -> Result<Self, NegotiationError> {
        let mut allowlist = BTreeSet::new();
        for raw in &resolved.protocol_versions {
            allowlist.insert(ProtocolVersion::parse(raw)?);
        }
        let mut capabilities: BTreeMap<String, CapabilityClass> = BTreeMap::new();
        for gate in id_config::FIXED_GATE_IDS {
            capabilities.insert(gate.to_owned(), CapabilityClass::Required);
        }
        for name in optional_capabilities {
            if capabilities.contains_key(*name) {
                return Err(NegotiationError::CapabilityClassConflict(
                    (*name).to_owned(),
                ));
            }
            capabilities.insert((*name).to_owned(), CapabilityClass::Optional);
        }
        Self::new(
            local_identity,
            allowlist,
            ProtocolVersion::parse(floor)?,
            capabilities,
        )
    }

    /// The names of every required capability, in canonical order.
    #[must_use]
    pub fn required_capabilities(&self) -> BTreeSet<String> {
        self.capabilities
            .iter()
            .filter(|(_, class)| **class == CapabilityClass::Required)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

/// One peer offer: everything the peer claims, before any of it is
/// trusted. Versions arrive as raw strings (as they would on the
/// wire) and are parsed during evaluation — a malformed string is an
/// adversarial input, not a parse-time accident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerOffer {
    /// The peer identity this offer claims. The session is bound to
    /// one expected peer; an offer naming anyone else is rejected
    /// ([`NegotiationError::IdentityMismatch`]).
    pub peer_identity: String,
    /// Protocol versions the peer supports, as raw version strings.
    pub versions: Vec<String>,
    /// Capabilities the peer offers to run at the negotiated version.
    pub capabilities: BTreeSet<String>,
    /// The negotiation round this offer belongs to. Rounds are
    /// consumed in strict order starting at 0; see [`Negotiation`].
    pub round: u64,
}

/// The identity-bound record of one accepted negotiation round: what
/// was offered and what was selected, by whom, in which round.
///
/// The transcript is the unit the design's "signed and identity-bound"
/// rule applies to. [`NegotiationTranscript::canonical_bytes`] pins
/// the exact bytes a signature must cover; [`NegotiationTranscript::binds`]
/// re-checks the record against the offer it claims to record, so a
/// transcript whose fields were altered after the fact no longer
/// binds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiationTranscript {
    /// Identity of the local party, from the policy.
    pub local_identity: String,
    /// Identity of the peer, from the offer.
    pub peer_identity: String,
    /// The round this transcript settles.
    pub round: u64,
    /// The versions the peer offered, as raw strings, sorted.
    pub offered_versions: Vec<String>,
    /// The capabilities the peer offered, sorted.
    pub offered_capabilities: Vec<String>,
    /// The version both parties settled on, canonical form.
    pub selected_version: String,
    /// The capabilities both parties settled on, sorted.
    pub selected_capabilities: Vec<String>,
}

impl NegotiationTranscript {
    /// The canonical bytes of this transcript: the domain prefix, a
    /// newline, and the deterministic JSON encoding (struct fields in
    /// declaration order, every list sorted at construction). These
    /// are the bytes a signature over this negotiation must cover.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = TRANSCRIPT_DOMAIN.as_bytes().to_vec();
        bytes.push(b'\n');
        // Serializing this struct cannot fail: every field is a plain
        // string, number, or list of strings. The fallback keeps the
        // function total without a panic path.
        let json = serde_json::to_vec(self).unwrap_or_default();
        bytes.extend_from_slice(&json);
        bytes
    }

    /// Whether this transcript faithfully records `offer`: both
    /// identities, the round, and the complete offered version and
    /// capability sets must match the offer exactly. A transcript
    /// edited after acceptance — a version swapped, a capability
    /// dropped from the record — fails this check.
    #[must_use]
    pub fn binds(&self, local_identity: &str, offer: &PeerOffer) -> bool {
        if self.local_identity != local_identity
            || self.peer_identity != offer.peer_identity
            || self.round != offer.round
        {
            return false;
        }
        let mut offered_versions = offer.versions.clone();
        offered_versions.sort();
        if self.offered_versions != offered_versions {
            return false;
        }
        let offered_capabilities: Vec<String> = offer.capabilities.iter().cloned().collect();
        self.offered_capabilities == offered_capabilities
    }
}

/// A settled negotiation: the version and capability set both parties
/// run, plus the transcript that records how they got there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Established {
    /// The negotiated protocol version.
    pub version: ProtocolVersion,
    /// The negotiated capability set: every required capability, plus
    /// the optional capabilities both sides support.
    pub capabilities: BTreeSet<String>,
    /// The transcript of the round that established this state.
    pub transcript: NegotiationTranscript,
}

/// Everything that can make a policy, an offer, or a negotiation
/// unacceptable. Every variant is a rejection, never a warning and
/// never a silent fallback to weaker terms.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NegotiationError {
    /// A version string is not `<family>/<major>[.<minor>]`.
    #[error("protocol version {0:?} is malformed (expected '<family>/<major>[.<minor>]')")]
    MalformedVersion(String),
    /// The local identity is empty.
    #[error("local identity must not be empty")]
    EmptyLocalIdentity,
    /// The expected peer identity is empty.
    #[error("peer identity must not be empty")]
    EmptyPeerIdentity,
    /// The policy allowlist is empty.
    #[error("negotiation allowlist must not be empty")]
    EmptyAllowlist,
    /// The policy floor is not one of the allowlisted versions.
    #[error("negotiation floor {floor} is not in the allowlist")]
    FloorNotAllowlisted {
        /// The floor that is not allowlisted.
        floor: String,
    },
    /// A capability name is empty.
    #[error("capability name must not be empty")]
    EmptyCapabilityName,
    /// An optional capability tries to relabel a fixed gate.
    #[error("capability {0:?} is a fixed gate and cannot be classified optional")]
    CapabilityClassConflict(String),
    /// The offer names a different peer than the session is bound to.
    #[error("offer is from peer {found:?}, but this session is bound to peer {expected:?}")]
    IdentityMismatch {
        /// The peer the session is bound to.
        expected: String,
        /// The peer the offer claims.
        found: String,
    },
    /// The offer's round is not the round the session expects: the
    /// offer is stale, replayed, or from the future.
    #[error(
        "offer round {found} does not match the expected round {expected} (stale or replayed offer)"
    )]
    StaleRound {
        /// The round the session expects next.
        expected: u64,
        /// The round the offer carries.
        found: u64,
    },
    /// The peer offers no version the local allowlist permits.
    #[error("no commonly supported allowlisted protocol version")]
    NoCommonVersion,
    /// The best mutually supported version is below the security
    /// floor: accepting it would be a downgrade.
    #[error("best common version {best} is below the security floor {floor}: downgrade rejected")]
    VersionBelowFloor {
        /// The best version both sides support.
        best: String,
        /// The local security floor.
        floor: String,
    },
    /// The offer omits a required capability: accepting it would be a
    /// downgrade.
    #[error("offer omits required capability {0:?}: downgrade rejected")]
    RequiredCapabilityMissing(String),
    /// A renegotiation offer would weaken the established session.
    #[error("renegotiation would weaken the established session ({0}): downgrade rejected")]
    SessionDowngrade(String),
    /// The session already rejected an initial offer and is closed.
    #[error("negotiation session is closed: a rejected negotiation is never resumed")]
    SessionClosed,
}

/// The session state. A rejected initial negotiation is terminal; an
/// established session survives rejected renegotiation offers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionState {
    /// No offer has been accepted yet.
    Awaiting,
    /// A negotiation has settled; renegotiation offers are evaluated
    /// against this baseline.
    Established(Established),
    /// An initial offer was rejected. Terminal.
    Rejected,
}

/// One capability negotiation session with one expected peer, under
/// one local policy.
///
/// Round discipline: rounds start at 0 and every evaluated offer
/// consumes exactly one round, accepted or rejected. An offer that is
/// malformed, from the wrong peer, or out of round is rejected
/// *without* being evaluated and does not consume a round — it never
/// reached the negotiation.
#[derive(Debug, Clone)]
pub struct Negotiation {
    policy: NegotiationPolicy,
    expected_peer: String,
    state: SessionState,
    next_round: u64,
}

impl Negotiation {
    /// Open a session bound to `expected_peer` under `policy`.
    ///
    /// # Errors
    /// Returns [`NegotiationError::EmptyPeerIdentity`] for an empty
    /// peer identity.
    pub fn new(policy: NegotiationPolicy, expected_peer: &str) -> Result<Self, NegotiationError> {
        if expected_peer.trim().is_empty() {
            return Err(NegotiationError::EmptyPeerIdentity);
        }
        Ok(Self {
            policy,
            expected_peer: expected_peer.to_owned(),
            state: SessionState::Awaiting,
            next_round: 0,
        })
    }

    /// The round the next evaluated offer must carry.
    #[must_use]
    pub const fn next_round(&self) -> u64 {
        self.next_round
    }

    /// The currently established session, if any.
    #[must_use]
    pub fn established(&self) -> Option<&Established> {
        match &self.state {
            SessionState::Established(established) => Some(established),
            _ => None,
        }
    }

    /// Whether an initial offer was rejected and the session is
    /// closed for good.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        matches!(self.state, SessionState::Rejected)
    }

    /// Evaluate one peer offer.
    ///
    /// On acceptance, returns the new [`Established`] state (which is
    /// also retained as the session baseline). On rejection the error
    /// names exactly which rule fired; the session either closes
    /// (initial offer) or continues on its previously established
    /// terms (renegotiation offer) — it never silently continues at
    /// weaker terms.
    ///
    /// # Errors
    /// Returns [`NegotiationError::SessionClosed`] if the session
    /// already rejected an initial offer,
    /// [`NegotiationError::IdentityMismatch`] if the offer names a
    /// different peer, [`NegotiationError::StaleRound`] if the offer's
    /// round is not the expected round,
    /// [`NegotiationError::MalformedVersion`] if any offered version
    /// string is malformed, [`NegotiationError::NoCommonVersion`] if
    /// no offered version is allowlisted,
    /// [`NegotiationError::VersionBelowFloor`] if the best common
    /// version is below the policy floor,
    /// [`NegotiationError::RequiredCapabilityMissing`] if the offer
    /// omits a required capability, and
    /// [`NegotiationError::SessionDowngrade`] if the offer would
    /// weaken the established session.
    pub fn offer(&mut self, offer: PeerOffer) -> Result<Established, NegotiationError> {
        if self.state == SessionState::Rejected {
            return Err(NegotiationError::SessionClosed);
        }
        if offer.peer_identity != self.expected_peer {
            return Err(NegotiationError::IdentityMismatch {
                expected: self.expected_peer.clone(),
                found: offer.peer_identity,
            });
        }
        if offer.round != self.next_round {
            return Err(NegotiationError::StaleRound {
                expected: self.next_round,
                found: offer.round,
            });
        }
        // The offer is well-formed in shape; parse its versions before
        // evaluation. A malformed string is adversarial input and is
        // rejected without consuming the round.
        let mut offered: BTreeSet<ProtocolVersion> = BTreeSet::new();
        for raw in &offer.versions {
            offered.insert(ProtocolVersion::parse(raw)?);
        }

        // From here the offer is evaluated, so it consumes its round
        // whether it is accepted or rejected.
        self.next_round += 1;

        let previous = self.established().cloned();
        let outcome = self.evaluate(&offered, &offer.capabilities);
        match (outcome, previous) {
            (Ok(candidate), None) => {
                let established = self.settle(candidate, &offer);
                self.state = SessionState::Established(established.clone());
                Ok(established)
            }
            (Ok(candidate), Some(previous)) => {
                check_not_weaker(&previous, &candidate)?;
                let established = self.settle(candidate, &offer);
                self.state = SessionState::Established(established.clone());
                Ok(established)
            }
            (Err(error), None) => {
                self.state = SessionState::Rejected;
                Err(error)
            }
            (Err(error), Some(_)) => Err(error),
        }
    }

    /// The stateless core of selection: highest common allowlisted
    /// version, floor check, required-capability check, capability
    /// intersection. Shared by the initial and renegotiation paths so
    /// the two can never drift apart.
    fn evaluate(
        &self,
        offered: &BTreeSet<ProtocolVersion>,
        offered_capabilities: &BTreeSet<String>,
    ) -> Result<(ProtocolVersion, BTreeSet<String>), NegotiationError> {
        let best = offered
            .intersection(&self.policy.allowlist)
            .max()
            .cloned()
            .ok_or(NegotiationError::NoCommonVersion)?;
        if best < self.policy.floor {
            return Err(NegotiationError::VersionBelowFloor {
                best: best.canonical(),
                floor: self.policy.floor.canonical(),
            });
        }
        for required in self.policy.required_capabilities() {
            if !offered_capabilities.contains(&required) {
                return Err(NegotiationError::RequiredCapabilityMissing(required));
            }
        }
        let selected: BTreeSet<String> = self
            .policy
            .capabilities
            .keys()
            .filter(|name| offered_capabilities.contains(*name))
            .cloned()
            .collect();
        Ok((best, selected))
    }

    /// Record an accepted outcome as an [`Established`] with its
    /// transcript of this round's offer.
    fn settle(
        &self,
        (version, capabilities): (ProtocolVersion, BTreeSet<String>),
        offer: &PeerOffer,
    ) -> Established {
        let mut offered_versions = offer.versions.clone();
        offered_versions.sort();
        let selected_version = version.canonical();
        let selected_capabilities: Vec<String> = capabilities.iter().cloned().collect();
        Established {
            version,
            capabilities,
            transcript: NegotiationTranscript {
                local_identity: self.policy.local_identity.clone(),
                peer_identity: offer.peer_identity.clone(),
                round: offer.round,
                offered_versions,
                offered_capabilities: offer.capabilities.iter().cloned().collect(),
                selected_version,
                selected_capabilities,
            },
        }
    }
}

/// The renegotiation downgrade rule: a new outcome is acceptable only
/// if it is at least as strong as the established baseline on both
/// axes — version and capability set.
fn check_not_weaker(
    previous: &Established,
    candidate: &(ProtocolVersion, BTreeSet<String>),
) -> Result<(), NegotiationError> {
    if candidate.0 < previous.version {
        return Err(NegotiationError::SessionDowngrade(format!(
            "version {} is below the established {}",
            candidate.0.canonical(),
            previous.version.canonical()
        )));
    }
    for established_capability in &previous.capabilities {
        if !candidate.1.contains(established_capability) {
            return Err(NegotiationError::SessionDowngrade(format!(
                "established capability {established_capability:?} would be dropped"
            )));
        }
    }
    Ok(())
}
