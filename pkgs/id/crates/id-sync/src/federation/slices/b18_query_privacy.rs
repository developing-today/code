//! B18: what a federated query discloses, to whom, and for how long
//! (#10264, statement S1.63; Part B "Resource/security invariants",
//! fan-out design #9144).
//!
//! Fan-out leaks query interest: every remote that is asked learns what
//! was asked. S1.63 makes that a named surface that each scope either
//! accepts or mitigates explicitly, and this slice is where that choice
//! is configured and enforced:
//!
//! - [`PrivacyPolicy`] maps a query scope to a [`ScopePolicy`]. A scope
//!   is either [`ScopePolicy::LocalOnly`] (privacy-sensitive: nothing
//!   leaves this node, the rollback) or [`ScopePolicy::Disclose`], which
//!   names the exact remotes that may receive the query, the exact
//!   [`Field`]s they receive, how long they are asked to keep it, whether
//!   they may relay it, and the [`Decision`] the operator took. A scope
//!   with no entry is local-only too: disclosure is opt-in.
//! - The decision has to be true. [`Decision::Accepted`] means every
//!   field goes out; [`Decision::Mitigated`] means at least one optional
//!   field is withheld. A policy that labels a disclosure wrongly is
//!   refused when it is built, so an audit can rely on the label.
//! - [`release`] turns a query and the remotes a planner (B15) wants to
//!   ask into a [`Release`]: one [`Disclosure`] per authorized remote,
//!   carrying exactly the configured fields, and a reason for every
//!   remote that gets nothing. A query that has expired releases nothing.
//! - [`Gate`] sits in front of the transport. It sends a disclosure only
//!   to a remote that was released it, and only byte for byte the
//!   disclosure that was released, and it records every send, so the
//!   recipient set and the disclosed fields are observable after the
//!   fact.
//!
//! Terms and scope always go out with a disclosure: a remote cannot
//! answer without them. Result limit, deadline and the origin's identity
//! are optional. Withholding the origin is only possible for direct
//! asks, because a relay must verify the origin's signature (B17), so a
//! policy that allows relaying must disclose the origin.
//!
//! Retention is a request to the remote, not a guarantee: the remote is
//! told to keep the query until `retain_until`, which is the configured
//! retention from the time of release, never past the query's own
//! expiry and never above [`MAX_RETENTION_SECS`]. Whether remotes honour
//! it is a matter for receipts (B19) and reputation, not this slice.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::{BTreeMap, BTreeSet};

use super::b20_request_identity::{OriginKey, PeerKey, RequestId};

/// The longest a remote may be asked to keep a query (30 days).
pub const MAX_RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

/// One part of a query that a remote may learn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// What is being looked for. Always disclosed.
    Terms,
    /// The scope the query runs under. Always disclosed.
    Scope,
    /// How many results the requester wants.
    Limit,
    /// When the query expires.
    Deadline,
    /// Who asked.
    Origin,
}

impl Field {
    /// Every field, in order.
    pub const ALL: [Self; 5] = [
        Self::Terms,
        Self::Scope,
        Self::Limit,
        Self::Deadline,
        Self::Origin,
    ];
    /// The fields every disclosure carries.
    pub const REQUIRED: [Self; 2] = [Self::Terms, Self::Scope];
}

/// What the operator decided about a scope's disclosure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Remotes learn every field.
    Accepted,
    /// At least one optional field is withheld.
    Mitigated,
}

/// How a scope may be disclosed to remotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disclose {
    /// The only remotes that may receive the query.
    pub recipients: BTreeSet<PeerKey>,
    /// The fields they receive.
    pub fields: BTreeSet<Field>,
    /// How long a remote is asked to keep the query, in seconds.
    pub retention_secs: u64,
    /// Whether a recipient may relay the query further (B17).
    pub relay: bool,
    /// The operator's explicit decision.
    pub decision: Decision,
}

/// A scope's privacy policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopePolicy {
    /// Privacy-sensitive: no remote fan-out; the local answer stands.
    LocalOnly,
    /// Remote fan-out with the stated disclosure.
    Disclose(Disclose),
}

/// Why a privacy policy was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// A disclosing scope names no recipient.
    NoRecipients { scope: Vec<u8> },
    /// A disclosing scope leaves out terms or scope.
    MissingRequired { scope: Vec<u8>, field: Field },
    /// A disclosing scope asks remotes to keep the query for no time.
    ZeroRetention { scope: Vec<u8> },
    /// A disclosing scope asks remotes to keep the query too long.
    RetentionAboveCeiling { scope: Vec<u8> },
    /// Relaying is allowed but the origin is withheld.
    RelayNeedsOrigin { scope: Vec<u8> },
    /// The decision does not describe the fields: accepted while
    /// withholding, or mitigated while disclosing everything.
    DecisionMismatch { scope: Vec<u8> },
}

/// Per-scope disclosure policy. Unlisted scopes are local-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivacyPolicy {
    scopes: BTreeMap<Vec<u8>, ScopePolicy>,
}

fn check(scope: &[u8], disclose: &Disclose) -> Result<(), PolicyError> {
    let scope = scope.to_vec();
    if disclose.recipients.is_empty() {
        return Err(PolicyError::NoRecipients { scope });
    }
    for field in Field::REQUIRED {
        if !disclose.fields.contains(&field) {
            return Err(PolicyError::MissingRequired { scope, field });
        }
    }
    if disclose.retention_secs == 0 {
        return Err(PolicyError::ZeroRetention { scope });
    }
    if disclose.retention_secs > MAX_RETENTION_SECS {
        return Err(PolicyError::RetentionAboveCeiling { scope });
    }
    if disclose.relay && !disclose.fields.contains(&Field::Origin) {
        return Err(PolicyError::RelayNeedsOrigin { scope });
    }
    let withholds = Field::ALL.iter().any(|f| !disclose.fields.contains(f));
    let truthful = match disclose.decision {
        Decision::Accepted => !withholds,
        Decision::Mitigated => withholds,
    };
    if !truthful {
        return Err(PolicyError::DecisionMismatch { scope });
    }
    Ok(())
}

impl PrivacyPolicy {
    /// Build a policy, refusing the first scope that is misconfigured.
    ///
    /// # Errors
    ///
    /// Any [`PolicyError`], for the first offending scope in scope order.
    pub fn new(scopes: BTreeMap<Vec<u8>, ScopePolicy>) -> Result<Self, PolicyError> {
        for (scope, policy) in &scopes {
            if let ScopePolicy::Disclose(disclose) = policy {
                check(scope, disclose)?;
            }
        }
        Ok(Self { scopes })
    }

    /// The policy for a scope, if one is configured.
    #[must_use]
    pub fn get(&self, scope: &[u8]) -> Option<&ScopePolicy> {
        self.scopes.get(scope)
    }
}

/// A query as the requester holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// The B20 request id.
    pub request_id: RequestId,
    /// Who asked.
    pub origin: OriginKey,
    /// The scope the query runs under.
    pub scope: Vec<u8>,
    /// What is being looked for.
    pub terms: Vec<u8>,
    /// How many results are wanted, if bounded.
    pub limit: Option<u32>,
    /// When the query expires.
    pub expires_at: u64,
}

/// What one remote receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disclosure {
    /// The B20 request id, so answers and duplicates can be matched.
    pub request_id: RequestId,
    /// The scope.
    pub scope: Vec<u8>,
    /// The terms.
    pub terms: Vec<u8>,
    /// The result limit, when disclosed and the query has one.
    pub limit: Option<u32>,
    /// The expiry, when disclosed.
    pub deadline: Option<u64>,
    /// The origin, when disclosed.
    pub origin: Option<OriginKey>,
    /// Until when the remote is asked to keep the query.
    pub retain_until: u64,
    /// Whether the remote may relay it.
    pub relay: bool,
}

impl Disclosure {
    /// The fields this disclosure carries.
    #[must_use]
    pub fn fields(&self) -> BTreeSet<Field> {
        let mut out: BTreeSet<Field> = Field::REQUIRED.into_iter().collect();
        if self.limit.is_some() {
            out.insert(Field::Limit);
        }
        if self.deadline.is_some() {
            out.insert(Field::Deadline);
        }
        if self.origin.is_some() {
            out.insert(Field::Origin);
        }
        out
    }
}

/// Why a remote receives nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withheld {
    /// The scope is configured local-only.
    LocalOnly,
    /// The scope has no policy, so it is local-only.
    Unconfigured,
    /// The scope discloses, but not to this remote.
    NotAuthorized,
}

/// Why nothing could be released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseError {
    /// The query has expired.
    Expired,
}

/// What a query may disclose, remote by remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The B20 request id.
    pub request_id: RequestId,
    /// The query's expiry; nothing is sent at or after it.
    pub expires_at: u64,
    /// The decision behind the release, when the scope discloses.
    pub decision: Option<Decision>,
    /// The disclosure for each authorized remote.
    pub sends: BTreeMap<PeerKey, Disclosure>,
    /// Every other candidate and why it gets nothing.
    pub withheld: BTreeMap<PeerKey, Withheld>,
}

impl Release {
    /// The remotes that may receive the query.
    #[must_use]
    pub fn recipients(&self) -> BTreeSet<PeerKey> {
        self.sends.keys().copied().collect()
    }
}

/// Decide what `query` discloses to each of `candidates` at `now`.
///
/// A remote listed more than once is considered once.
///
/// # Errors
///
/// [`ReleaseError::Expired`] at or after the query's expiry.
pub fn release(
    policy: &PrivacyPolicy,
    query: &Query,
    candidates: &[PeerKey],
    now: u64,
) -> Result<Release, ReleaseError> {
    if now >= query.expires_at {
        return Err(ReleaseError::Expired);
    }
    let mut out = Release {
        request_id: query.request_id,
        expires_at: query.expires_at,
        decision: None,
        sends: BTreeMap::new(),
        withheld: BTreeMap::new(),
    };
    let disclose = match policy.get(&query.scope) {
        None => {
            out.withheld = candidates
                .iter()
                .map(|p| (*p, Withheld::Unconfigured))
                .collect();
            return Ok(out);
        }
        Some(ScopePolicy::LocalOnly) => {
            out.withheld = candidates
                .iter()
                .map(|p| (*p, Withheld::LocalOnly))
                .collect();
            return Ok(out);
        }
        Some(ScopePolicy::Disclose(disclose)) => disclose,
    };
    out.decision = Some(disclose.decision);
    let has = |field| disclose.fields.contains(&field);
    let disclosure = Disclosure {
        request_id: query.request_id,
        scope: query.scope.clone(),
        terms: query.terms.clone(),
        limit: query.limit.filter(|_| has(Field::Limit)),
        deadline: has(Field::Deadline).then_some(query.expires_at),
        origin: has(Field::Origin).then_some(query.origin),
        retain_until: now
            .saturating_add(disclose.retention_secs)
            .min(query.expires_at),
        relay: disclose.relay,
    };
    for peer in candidates {
        if disclose.recipients.contains(peer) {
            out.sends.insert(*peer, disclosure.clone());
        } else {
            out.withheld.insert(*peer, Withheld::NotAuthorized);
        }
    }
    Ok(out)
}

/// Why the gate refused a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateError {
    /// Nothing was released to this remote for this request.
    NotReleased,
    /// The disclosure is not the one that was released.
    Mismatch,
    /// The query has expired.
    Expired,
}

/// One send the gate let through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// Who received it.
    pub peer: PeerKey,
    /// The request.
    pub request_id: RequestId,
    /// What it carried.
    pub fields: BTreeSet<Field>,
}

/// The only way a query leaves this node.
#[derive(Debug, Clone, Default)]
pub struct Gate {
    released: BTreeMap<(RequestId, PeerKey), (Disclosure, u64)>,
    sent: Vec<Sent>,
}

impl Gate {
    /// An empty gate.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allow the sends in `release`. A later release for the same
    /// request and remote replaces the earlier one.
    pub fn admit(&mut self, release: &Release) {
        for (peer, disclosure) in &release.sends {
            self.released.insert(
                (release.request_id, *peer),
                (disclosure.clone(), release.expires_at),
            );
        }
    }

    /// Send `disclosure` to `peer`, if exactly that was released.
    ///
    /// # Errors
    ///
    /// [`GateError::NotReleased`], [`GateError::Mismatch`] or
    /// [`GateError::Expired`]; nothing is recorded.
    pub fn send(
        &mut self,
        peer: PeerKey,
        disclosure: &Disclosure,
        now: u64,
    ) -> Result<(), GateError> {
        let (released, expires_at) = self
            .released
            .get(&(disclosure.request_id, peer))
            .ok_or(GateError::NotReleased)?;
        if now >= *expires_at {
            return Err(GateError::Expired);
        }
        if released != disclosure {
            return Err(GateError::Mismatch);
        }
        self.sent.push(Sent {
            peer,
            request_id: disclosure.request_id,
            fields: disclosure.fields(),
        });
        Ok(())
    }

    /// Forget releases whose query has expired at `now`. The send log is
    /// kept.
    pub fn prune(&mut self, now: u64) {
        self.released.retain(|_, (_, expires_at)| now < *expires_at);
    }

    /// How many releases are held.
    #[must_use]
    pub fn held(&self) -> usize {
        self.released.len()
    }

    /// Every send so far, in order.
    #[must_use]
    pub fn sent(&self) -> &[Sent] {
        &self.sent
    }
}
