//! The selective-sharing grants model (#5799 decision record; slice 1 of
//! #14790).
//!
//! A [`Grant`] gives a [`Subject`] a set of [`Operation`]s over a set of
//! [`Scope`]s for one [`Duration`]. This crate is the pure model: the
//! types, the creation-time checks ([`Grant::validate`]), the delegation
//! attenuation check, and the authorization check over a [`GrantSet`]
//! ([`GrantSet::authorize`]). It does no I/O and persists nothing.
//! Composing it with the #5678 read-authority resolver (`id-store`'s
//! `read_authority`) is slice 2 of #14790.
//!
//! What the record fixes, and where this crate holds it:
//!
//! - **Units** are objects and stable collections ([`Scope`]). A
//!   database or table selector is expanded to stable collection ids by
//!   the caller when the grant is created; this crate stores only the
//!   expanded ids, so a grant never acts as a wildcard (invariant 3).
//! - **Operations** are explicit. `subscribe` requires `read`; `write`
//!   and `append` are never implied by anything (invariant 4).
//! - **Duration** is required: `standing`, `expires_at`, or
//!   `snapshot_once`. A grant with no duration does not deserialize, so
//!   omission can never mean "permanent". Expiry is checked on every
//!   authorization (invariant 2). A `snapshot_once` grant is read-only and
//!   serves one initial snapshot ([`GrantSet::consume_snapshot`]).
//! - **Revocation** takes effect at the next [`GrantSet::authorize`] and
//!   cascades to every grant delegated from the revoked one (invariants 5
//!   and 7). It promises future denial only, never take-back of bytes
//!   already copied.
//! - **Delegation** is off by default. A delegated grant must be
//!   attenuated against its parent: no scope the parent lacks, no
//!   operation the parent lacks, no expiry later than the parent's, and
//!   further delegation only where the parent allows re-delegation
//!   (invariant 5).
//! - **Existence privacy.** [`Decision::Deny`] carries nothing, so a
//!   requester cannot tell "no such resource" from "not granted"
//!   (invariant 6).
//! - **Public shares** (#5802 D1, D3). The [`Subject::Public`] audience
//!   matches every requester, anonymous ones included. A public grant may
//!   carry only `read` and `subscribe`, and can never be delegated; both
//!   are refused when the grant is created, not only in a UI. Each
//!   allowed access is covered by one grant on its own, so a public grant
//!   never borrows the scope of a private one (D3.5).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

pub mod persist;
pub mod publication;

pub use persist::{GrantSetSnapshot, LoadError, SNAPSHOT_FORMAT};
pub use publication::{
    ClassLevel, DiscoveryMode, ListingEntry, PublicationRecord, Publications, PublishError,
    PublishRequest, ShareId, ShareManifest, UNLISTED_LABEL, UNPUBLISH_NOTICE,
};

/// A grant's stable identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GrantId(pub String);

impl fmt::Display for GrantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who a grant is for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
    /// One identity, by its key.
    Identity(String),
    /// Every member of a group, by the group's id.
    Group(String),
    /// Everyone, including requesters with no identity (#5802 D1).
    Public,
}

/// What a grant covers: one object, or one stable collection (every
/// object in it at the time of each check, including later additions).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// One object, by its id.
    Object(String),
    /// One stable collection, by its id.
    Collection(String),
}

/// An operation a grant permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Read the current state.
    Read,
    /// Receive ongoing updates. Requires `read`.
    Subscribe,
    /// Change existing data.
    Write,
    /// Add new data without changing existing data.
    Append,
}

impl Operation {
    /// The operation's wire name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Subscribe => "subscribe",
            Self::Write => "write",
            Self::Append => "append",
        }
    }
}

/// How long a grant lasts. Every grant names exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Duration {
    /// Valid until revoked.
    Standing,
    /// Valid through this instant (Unix seconds, UTC), inclusive.
    ExpiresAt(u64),
    /// One initial read snapshot only: no subscription and no later
    /// re-fetch. Retries of that one transfer are the caller's to bound.
    SnapshotOnce,
}

/// Whether a grant's holder may delegate it. Off unless set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delegation {
    /// The holder may issue attenuated child grants.
    #[serde(default)]
    pub may_delegate: bool,
    /// Child grants may themselves be allowed to delegate. Requires
    /// `may_delegate`.
    #[serde(default)]
    pub may_redelegate: bool,
}

/// One grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// The grant's id, unique within a [`GrantSet`].
    pub id: GrantId,
    /// The identity that issued the grant: the owner for a root grant,
    /// the delegating holder for a child grant.
    pub issuer: String,
    /// Who the grant is for.
    pub subject: Subject,
    /// What it covers.
    pub scopes: BTreeSet<Scope>,
    /// What it permits.
    pub operations: BTreeSet<Operation>,
    /// How long it lasts. Required.
    pub duration: Duration,
    /// Whether its holder may delegate it.
    #[serde(default)]
    pub delegation: Delegation,
    /// The grant this one was delegated from, if any.
    #[serde(default)]
    pub parent: Option<GrantId>,
}

/// Why a grant was refused when it was created or inserted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// The grant id, the issuer, a subject key, or a scope id is empty.
    EmptyName,
    /// The grant covers nothing.
    EmptyScope,
    /// The grant permits nothing.
    EmptyOperations,
    /// `subscribe` without `read`.
    SubscribeWithoutRead,
    /// A `snapshot_once` grant permits more than `read`, or may delegate.
    SnapshotOnceNotReadOnly,
    /// A public grant carries `write` or `append` (#5802 D3.1).
    PublicOperation(Operation),
    /// A public grant may be delegated (#5802 D1).
    PublicDelegation,
    /// `may_redelegate` without `may_delegate`.
    RedelegateWithoutDelegate,
    /// The set already holds a grant with this id.
    DuplicateId,
    /// The named parent is not in the set, is revoked or expired, or a
    /// root grant names a parent.
    UnknownParent,
    /// The parent grant does not allow delegation.
    ParentNotDelegable,
    /// The delegating identity is not the parent grant's subject.
    DelegatorNotSubject,
    /// The child's issuer is not the delegating identity.
    IssuerNotDelegator,
    /// The child covers a scope the parent does not.
    WiderScope,
    /// The child permits an operation the parent does not.
    ExtraOperation(Operation),
    /// The child could outlive the parent's expiry.
    LaterExpiry,
    /// The child may delegate, but the parent does not allow
    /// re-delegation.
    RedelegationNotAllowed,
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => f.write_str("a grant id, issuer, subject, or scope id is empty"),
            Self::EmptyScope => f.write_str("the grant covers no object or collection"),
            Self::EmptyOperations => f.write_str("the grant permits no operation"),
            Self::SubscribeWithoutRead => f.write_str("subscribe requires read"),
            Self::SnapshotOnceNotReadOnly => {
                f.write_str("a snapshot_once grant permits read only and cannot be delegated")
            }
            Self::PublicOperation(op) => {
                write!(f, "a public grant cannot carry {}", op.name())
            }
            Self::PublicDelegation => f.write_str("a public grant cannot be delegated"),
            Self::RedelegateWithoutDelegate => f.write_str("may_redelegate requires may_delegate"),
            Self::DuplicateId => f.write_str("a grant with this id already exists"),
            Self::UnknownParent => f.write_str("the parent grant is not known or no longer valid"),
            Self::ParentNotDelegable => f.write_str("the parent grant does not allow delegation"),
            Self::DelegatorNotSubject => {
                f.write_str("the delegating identity is not the parent grant's subject")
            }
            Self::IssuerNotDelegator => {
                f.write_str("the child grant's issuer is not the delegating identity")
            }
            Self::WiderScope => f.write_str("the child grant covers a scope the parent does not"),
            Self::ExtraOperation(op) => {
                write!(
                    f,
                    "the child grant permits {}, which the parent does not",
                    op.name()
                )
            }
            Self::LaterExpiry => f.write_str("the child grant could outlive the parent's expiry"),
            Self::RedelegationNotAllowed => {
                f.write_str("the parent grant does not allow re-delegation")
            }
        }
    }
}

impl std::error::Error for GrantError {}

impl Grant {
    /// The creation-time checks every grant must pass on its own.
    pub fn validate(&self) -> Result<(), GrantError> {
        let empty_subject = match &self.subject {
            Subject::Identity(key) | Subject::Group(key) => key.is_empty(),
            Subject::Public => false,
        };
        let empty_scope = self.scopes.iter().any(|scope| match scope {
            Scope::Object(id) | Scope::Collection(id) => id.is_empty(),
        });
        if self.id.0.is_empty() || self.issuer.is_empty() || empty_subject || empty_scope {
            return Err(GrantError::EmptyName);
        }
        if self.scopes.is_empty() {
            return Err(GrantError::EmptyScope);
        }
        if self.operations.is_empty() {
            return Err(GrantError::EmptyOperations);
        }
        if self.operations.contains(&Operation::Subscribe)
            && !self.operations.contains(&Operation::Read)
        {
            return Err(GrantError::SubscribeWithoutRead);
        }
        if self.delegation.may_redelegate && !self.delegation.may_delegate {
            return Err(GrantError::RedelegateWithoutDelegate);
        }
        if self.subject == Subject::Public {
            if let Some(op) = self
                .operations
                .iter()
                .find(|op| matches!(op, Operation::Write | Operation::Append))
            {
                return Err(GrantError::PublicOperation(*op));
            }
            if self.delegation.may_delegate {
                return Err(GrantError::PublicDelegation);
            }
        }
        if self.duration == Duration::SnapshotOnce
            && (self.operations.iter().any(|op| *op != Operation::Read)
                || self.delegation.may_delegate)
        {
            return Err(GrantError::SnapshotOnceNotReadOnly);
        }
        Ok(())
    }

    /// Whether the grant has passed its expiry at `now` (Unix seconds).
    pub const fn is_expired(&self, now: u64) -> bool {
        match self.duration {
            Duration::ExpiresAt(at) => now > at,
            Duration::Standing | Duration::SnapshotOnce => false,
        }
    }

    /// Whether the grant's subject covers `requester` (`None` is an
    /// anonymous requester, covered only by the public audience).
    pub fn covers_requester(&self, requester: Option<&Requester>) -> bool {
        match (&self.subject, requester) {
            (Subject::Public, _) => true,
            (Subject::Identity(key), Some(r)) => *key == r.identity,
            (Subject::Group(group), Some(r)) => r.groups.contains(group),
            (Subject::Identity(_) | Subject::Group(_), None) => false,
        }
    }

    /// Whether the grant's scope covers the object `object` in the
    /// collection `collection`.
    pub fn covers_resource(&self, object: &str, collection: Option<&str>) -> bool {
        self.scopes.iter().any(|scope| match scope {
            Scope::Object(id) => id == object,
            Scope::Collection(id) => collection == Some(id.as_str()),
        })
    }
}

/// Check that `child` is a valid attenuation of `parent`, delegated by
/// `delegator`.
///
/// A child scope must appear in the parent's scope set as
/// is: an object grant cannot be cut from a collection grant here,
/// because collection membership is not this crate's to know.
pub fn check_attenuation(
    parent: &Grant,
    child: &Grant,
    delegator: &Requester,
) -> Result<(), GrantError> {
    if !parent.delegation.may_delegate {
        return Err(GrantError::ParentNotDelegable);
    }
    if !parent.covers_requester(Some(delegator)) {
        return Err(GrantError::DelegatorNotSubject);
    }
    if child.issuer != delegator.identity {
        return Err(GrantError::IssuerNotDelegator);
    }
    if !child.scopes.is_subset(&parent.scopes) {
        return Err(GrantError::WiderScope);
    }
    if let Some(op) = child.operations.difference(&parent.operations).next() {
        return Err(GrantError::ExtraOperation(*op));
    }
    match (parent.duration, child.duration) {
        (Duration::ExpiresAt(p), Duration::ExpiresAt(c)) if c > p => {
            return Err(GrantError::LaterExpiry);
        }
        (Duration::ExpiresAt(_), Duration::Standing) => return Err(GrantError::LaterExpiry),
        _ => {}
    }
    if child.delegation.may_delegate && !parent.delegation.may_redelegate {
        return Err(GrantError::RedelegationNotAllowed);
    }
    Ok(())
}

/// Who is asking: one identity and the groups it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requester {
    /// The requester's identity key.
    pub identity: String,
    /// The groups the requester belongs to, as the host resolved them.
    #[serde(default)]
    pub groups: BTreeSet<String>,
}

/// One access to authorize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessRequest<'a> {
    /// Who is asking; `None` for a requester with no identity.
    pub requester: Option<&'a Requester>,
    /// The object being accessed.
    pub object: &'a str,
    /// The stable collection the object belongs to, if any.
    pub collection: Option<&'a str>,
    /// The operation requested.
    pub operation: Operation,
    /// The time of the check (Unix seconds, UTC).
    pub now: u64,
}

/// The outcome of an authorization check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Allowed, by this one grant.
    Allow(GrantId),
    /// Refused. Carries nothing, so an unknown resource and an ungranted
    /// one look the same.
    Deny,
}

/// The deepest delegation chain [`GrantSet::authorize`] will follow.
/// Longer chains are treated as invalid rather than walked further.
pub const MAX_DELEGATION_DEPTH: usize = 64;

/// A set of grants with their revocation and snapshot state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantSet {
    grants: BTreeMap<GrantId, Grant>,
    revoked: BTreeSet<GrantId>,
    consumed: BTreeSet<GrantId>,
}

impl GrantSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a root grant, issued by its owner. It must have no parent.
    pub fn insert_root(&mut self, grant: Grant) -> Result<(), GrantError> {
        grant.validate()?;
        if grant.parent.is_some() {
            return Err(GrantError::UnknownParent);
        }
        if self.grants.contains_key(&grant.id) {
            return Err(GrantError::DuplicateId);
        }
        self.grants.insert(grant.id.clone(), grant);
        Ok(())
    }

    /// Insert a grant delegated by `delegator` from the grant named in
    /// `grant.parent`. The parent must be in the set, currently valid at
    /// `now`, and the child must attenuate it.
    pub fn insert_delegated(
        &mut self,
        grant: Grant,
        delegator: &Requester,
        now: u64,
    ) -> Result<(), GrantError> {
        grant.validate()?;
        if self.grants.contains_key(&grant.id) {
            return Err(GrantError::DuplicateId);
        }
        let parent_id = grant.parent.as_ref().ok_or(GrantError::UnknownParent)?;
        let parent = self
            .grants
            .get(parent_id)
            .ok_or(GrantError::UnknownParent)?;
        if !self.lineage_valid(parent, now) {
            return Err(GrantError::UnknownParent);
        }
        check_attenuation(parent, &grant, delegator)?;
        self.grants.insert(grant.id.clone(), grant);
        Ok(())
    }

    /// Revoke a grant. It, and every grant delegated from it, stops
    /// authorizing at the next check. Returns whether the grant was known.
    pub fn revoke(&mut self, id: &GrantId) -> bool {
        if self.grants.contains_key(id) {
            self.revoked.insert(id.clone());
            true
        } else {
            false
        }
    }

    /// Record that a `snapshot_once` grant's one snapshot was served.
    /// Later reads under it are denied. Returns whether the grant is a
    /// known `snapshot_once` grant.
    pub fn consume_snapshot(&mut self, id: &GrantId) -> bool {
        match self.grants.get(id) {
            Some(grant) if grant.duration == Duration::SnapshotOnce => {
                self.consumed.insert(id.clone());
                true
            }
            _ => false,
        }
    }

    /// Look up a grant by id.
    pub fn get(&self, id: &GrantId) -> Option<&Grant> {
        self.grants.get(id)
    }

    /// Every grant in the set, in id order.
    pub fn grants(&self) -> impl Iterator<Item = &Grant> {
        self.grants.values()
    }

    /// The root of `grant`'s delegation chain, when `grant` and every
    /// ancestor is present, unrevoked, and unexpired at `now`; `None`
    /// when any link is missing, revoked, or expired, or the chain is
    /// deeper than [`MAX_DELEGATION_DEPTH`].
    fn valid_root<'s>(&'s self, grant: &'s Grant, now: u64) -> Option<&'s Grant> {
        let mut current = grant;
        for _ in 0..=MAX_DELEGATION_DEPTH {
            if self.revoked.contains(&current.id) || current.is_expired(now) {
                return None;
            }
            match &current.parent {
                None => return Some(current),
                Some(parent) => current = self.grants.get(parent)?,
            }
        }
        None
    }

    /// Whether the grant named `id` is in the set and live at `now`:
    /// it and every ancestor present, unrevoked, and unexpired. Whether
    /// a `snapshot_once` grant's snapshot was served is a separate
    /// question ([`GrantSet::authorize`] answers it per access).
    #[must_use]
    pub fn is_live(&self, id: &GrantId, now: u64) -> bool {
        self.grants
            .get(id)
            .is_some_and(|grant| self.lineage_valid(grant, now))
    }

    /// Whether `grant` and every ancestor is present, unrevoked, and
    /// unexpired at `now`.
    fn lineage_valid(&self, grant: &Grant, now: u64) -> bool {
        self.valid_root(grant, now).is_some()
    }

    /// Authorize one access. Allowed only when a single grant covers the
    /// requester, the resource, and the operation, its whole lineage is
    /// valid at the time of the check, and, for a `snapshot_once` grant,
    /// its snapshot has not been served yet. The lowest-id such grant is
    /// named.
    pub fn authorize(&self, request: &AccessRequest<'_>) -> Decision {
        self.authorize_where(request, |_| true)
    }

    /// Authorize one access as [`GrantSet::authorize`] does, counting
    /// only grants whose delegation chain is rooted in a grant issued
    /// by `root_issuer`.
    ///
    /// A host passes the resource's owner here, so a grant that someone
    /// else issued over that resource, or delegated from such a grant,
    /// never allows anything. Refusal still carries nothing.
    pub fn authorize_rooted_in(&self, request: &AccessRequest<'_>, root_issuer: &str) -> Decision {
        self.authorize_where(request, |root| root.issuer == root_issuer)
    }

    /// The shared check behind [`GrantSet::authorize`] and
    /// [`GrantSet::authorize_rooted_in`]: `root_ok` must accept the
    /// valid root of the allowing grant's chain.
    fn authorize_where(
        &self,
        request: &AccessRequest<'_>,
        root_ok: impl Fn(&Grant) -> bool,
    ) -> Decision {
        self.grants
            .values()
            .find(|grant| {
                grant.covers_requester(request.requester)
                    && grant.covers_resource(request.object, request.collection)
                    && grant.operations.contains(&request.operation)
                    && !(grant.duration == Duration::SnapshotOnce
                        && self.consumed.contains(&grant.id))
                    && self.valid_root(grant, request.now).is_some_and(&root_ok)
            })
            .map_or(Decision::Deny, |grant| Decision::Allow(grant.id.clone()))
    }
}
