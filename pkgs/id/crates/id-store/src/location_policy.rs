//! Scope authorization applied to location queries before candidate
//! counts and projection (slice A9115-privacy-filter, issue #10271;
//! parent #9115; design `part-a-addressing/versioned-design.md`
//! section "#9115 — Existence acknowledgements, location harvester,
//! privacy", privacy rule + A-9115.1/A-9115.2).
//!
//! The per-scope [`LocationIndex`] harvests every valid visible
//! location in a complete signed sidecar (S3.29) — the index itself
//! never omits. Privacy is therefore a query-time property, and
//! this module is the query-time gate: given the caller's
//! authenticated authorization (the #9083 tenant/scope context,
//! consumed here, never defined), it decides which harvested
//! entries a query may see *before* any candidate count is
//! computed, any ranking happens, or any projection is materialized
//! for ranking or caching. Every output of this module — counts,
//! candidate lists, single-entry lookups, projections, existence
//! acknowledgements — is a function of the authorized visible
//! subset only: an entry the caller is not authorized for is
//! indistinguishable from an absent one, so counts cannot leak the
//! existence of unauthorized locations.
//!
//! Visibility (design privacy rule: default visibility is private
//! within the caller-authorized namespace/tenant; public
//! enumeration is prohibited unless explicitly configured):
//!
//! - [`AccessClass::Public`] entries are visible to a caller
//!   holding the explicitly configured public grant, and to the
//!   scope's own authorized caller (a public location in one's own
//!   scope is at least as visible as a scope-private one);
//! - [`AccessClass::Scope`] entries are visible only to the caller
//!   authorized for the index's exact namespace + tenant scope;
//! - [`AccessClass::StoredUnverified`] entries are visible only to
//!   the scope-authorized caller that also holds the unverified
//!   grant: a `stored_unverified` observation is an assertion about
//!   unverified storage, never proof of accepted or validated
//!   content (A-9115.1), so seeing one is its own grant, not a side
//!   effect of scope membership;
//! - an entry created by a tombstone before any assertion carries
//!   no access class; it names a location retired inside the scope
//!   and is scope-private, like [`AccessClass::Scope`].
//!
//! Authorization filters *which* entries are visible, never their
//! states: a visible entry is returned with its state
//! (`fresh | stale | deleted | unverified`), provenance and lineage
//! intact (A-9115.2). Existence acknowledgements follow A-9115.1:
//! internally [`AckResult::VerifiedPresent`],
//! [`AckResult::NotPresent`] and [`AckResult::Indeterminate`] are
//! distinct results; the coalesced external form
//! ([`ExternalAck`]) folds everything but a verified presence into
//! one answer, and an unauthorized location acknowledges exactly
//! like an absent one.
//!
//! The crate-root export belongs to slice 9110-02; until that lands
//! the integration test includes this module (and `location_index`)
//! by path.

use id_core::id_shape::ContentId;
use id_core::location_ref::TypedLocator;

use crate::location_index::{
    AccessClass, LocationEntry, LocationIndex, LocationIndexError, LocationState, MAX_LABEL_BYTES,
};

/// A caller's authenticated authorization for location queries
/// against one per-scope index: the namespace + tenant scope the
/// caller is authorized for (the #9083 context), plus the
/// explicitly configured grants that widen visibility beyond that
/// scope.
///
/// An authorization with an empty scope (see
/// [`ScopeAuthorization::anonymous`]) never covers a real index
/// scope: index scope labels are non-empty by construction, so the
/// empty scope matches nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeAuthorization {
    namespace_id: String,
    tenant_scope: String,
    public_visible: bool,
    unverified_visible: bool,
}

impl ScopeAuthorization {
    /// The default authorization: private within the caller's own
    /// namespace + tenant scope. That scope's `Scope` and
    /// `StoredUnverified` entries are visible, and its `Public`
    /// entries are visible (at least as visible as scope-private
    /// ones). No cross-scope visibility and no public-enumeration
    /// grant is implied.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError::Field`] for an empty or over-long
    /// scope label, mirroring the index's own scope validation.
    pub fn for_scope(
        namespace_id: impl Into<String>,
        tenant_scope: impl Into<String>,
    ) -> Result<Self, LocationIndexError> {
        let namespace_id = namespace_id.into();
        check_label("namespace", &namespace_id)?;
        let tenant_scope = tenant_scope.into();
        check_label("tenant_scope", &tenant_scope)?;
        Ok(Self {
            namespace_id,
            tenant_scope,
            public_visible: false,
            unverified_visible: true,
        })
    }

    /// An authorization holding no scope and no grants: nothing is
    /// visible to it until a grant is explicitly configured. This
    /// is the starting point for a public reader — pair it with
    /// [`ScopeAuthorization::with_public_visible`].
    #[must_use]
    pub const fn anonymous() -> Self {
        Self {
            namespace_id: String::new(),
            tenant_scope: String::new(),
            public_visible: false,
            unverified_visible: false,
        }
    }

    /// Sets the explicitly configured public grant: whether
    /// [`AccessClass::Public`] entries are visible to this caller
    /// outside its own scope. Public enumeration is prohibited
    /// unless this is set (design privacy rule).
    #[must_use]
    pub const fn with_public_visible(mut self, visible: bool) -> Self {
        self.public_visible = visible;
        self
    }

    /// Sets the unverified grant: whether
    /// [`AccessClass::StoredUnverified`] entries are visible to
    /// this (scope-authorized) caller. A `stored_unverified`
    /// observation is never proof of content (A-9115.1), so this
    /// grant is separate from scope membership.
    #[must_use]
    pub const fn with_unverified_visible(mut self, visible: bool) -> Self {
        self.unverified_visible = visible;
        self
    }

    /// The namespace this authorization is scoped to (empty for an
    /// anonymous authorization).
    #[must_use]
    pub fn namespace_id(&self) -> &str {
        &self.namespace_id
    }

    /// The tenant scope this authorization is scoped to (empty for
    /// an anonymous authorization).
    #[must_use]
    pub fn tenant_scope(&self) -> &str {
        &self.tenant_scope
    }

    /// Whether the explicitly configured public grant is held.
    #[must_use]
    pub const fn public_visible(&self) -> bool {
        self.public_visible
    }

    /// Whether the unverified grant is held.
    #[must_use]
    pub const fn unverified_visible(&self) -> bool {
        self.unverified_visible
    }

    /// Whether this authorization covers the index's own scope
    /// exactly (namespace and tenant scope both equal).
    fn covers_scope(&self, index: &LocationIndex) -> bool {
        self.namespace_id == index.namespace_id() && self.tenant_scope == index.tenant_scope()
    }

    /// Whether one harvested entry is visible under this
    /// authorization. This predicate is the whole privacy rule:
    /// every query output below is computed from the entries it
    /// accepts, and from nothing else.
    #[must_use]
    pub fn is_visible(&self, index: &LocationIndex, entry: &LocationEntry) -> bool {
        let scope = self.covers_scope(index);
        match entry.access_class {
            Some(AccessClass::Public) => self.public_visible || scope,
            Some(AccessClass::Scope) => scope,
            Some(AccessClass::StoredUnverified) => scope && self.unverified_visible,
            // A tombstone-created entry asserts no class; it names
            // a location retired inside the scope and is private to
            // that scope.
            None => scope,
        }
    }

    /// The authorized view over one index: all counts, candidates
    /// and projections are taken through it, so authorization is
    /// applied before any of them is computed.
    #[must_use]
    pub const fn view<'a>(&'a self, index: &'a LocationIndex) -> AuthorizedLocations<'a> {
        AuthorizedLocations {
            index,
            authorization: self,
        }
    }
}

/// The authorized view over one [`LocationIndex`]: the only query
/// surface the privacy rule allows ranking, caching and result
/// code to consume. Constructed by [`ScopeAuthorization::view`].
#[derive(Debug)]
pub struct AuthorizedLocations<'a> {
    index: &'a LocationIndex,
    authorization: &'a ScopeAuthorization,
}

impl<'a> AuthorizedLocations<'a> {
    /// The visible locations of one content, in locator order,
    /// each exposing its state, provenance and lineage. Locations
    /// the caller is not authorized for are absent, exactly as if
    /// they had never been harvested.
    #[must_use]
    pub fn candidates(&self, content_id: &ContentId) -> Vec<&'a LocationEntry> {
        self.index
            .locations_for(content_id)
            .into_iter()
            .filter(|entry| self.authorization.is_visible(self.index, entry))
            .collect()
    }

    /// The number of visible locations of one content. Computed
    /// from the authorized visible subset only, after filtering —
    /// never from the harvested count — so it cannot leak how many
    /// unauthorized locations exist.
    #[must_use]
    pub fn candidate_count(&self, content_id: &ContentId) -> usize {
        self.candidates(content_id).len()
    }

    /// The visible entry for one content at one location, if any.
    /// An unauthorized entry returns `None`, indistinguishable from
    /// an absent one.
    #[must_use]
    pub fn entry(
        &self,
        content_id: &ContentId,
        locator: &TypedLocator,
    ) -> Option<&'a LocationEntry> {
        self.index
            .entry(content_id, locator)
            .filter(|entry| self.authorization.is_visible(self.index, entry))
    }

    /// Every visible entry, in key order.
    pub fn visible_entries(&self) -> impl Iterator<Item = &'a LocationEntry> {
        self.index
            .entries()
            .filter(|entry| self.authorization.is_visible(self.index, entry))
    }

    /// The number of visible entries in the index, in every state.
    #[must_use]
    pub fn visible_len(&self) -> usize {
        self.visible_entries().count()
    }

    /// Whether no entry is visible to this caller.
    #[must_use]
    pub fn is_visibly_empty(&self) -> bool {
        self.visible_len() == 0
    }

    /// The materialized authorized projection: the visible entries,
    /// cloned, in key order. This — never the raw index — is what
    /// ranking and caching may hold: it contains no unauthorized
    /// locator values and its length is the authorized count.
    #[must_use]
    pub fn projection(&self) -> Vec<LocationEntry> {
        self.visible_entries().cloned().collect()
    }

    /// The internal existence acknowledgement for one location
    /// (A-9115.1): a visible `fresh` entry is
    /// [`AckResult::VerifiedPresent`]; a visible `unverified` entry
    /// is [`AckResult::Indeterminate`] (storage was observed,
    /// content was not verified — never proof of presence); a
    /// visible `stale` or `deleted` entry, an absent location and
    /// an unauthorized location are all [`AckResult::NotPresent`],
    /// indistinguishably.
    #[must_use]
    pub fn acknowledge(&self, content_id: &ContentId, locator: &TypedLocator) -> AckResult {
        match self.entry(content_id, locator) {
            None => AckResult::NotPresent,
            Some(entry) => match entry.state {
                LocationState::Fresh => AckResult::VerifiedPresent,
                LocationState::Unverified => AckResult::Indeterminate,
                LocationState::Stale | LocationState::Deleted => AckResult::NotPresent,
            },
        }
    }

    /// The coalesced external existence acknowledgement (A-9115.1:
    /// policy may coalesce the internal results externally):
    /// [`ExternalAck::VerifiedPresent`] only for a verified
    /// presence, [`ExternalAck::NotVerifiedPresent`] for every
    /// other internal result, so an external caller cannot
    /// distinguish unverified storage, absence and lack of
    /// authorization.
    #[must_use]
    pub fn acknowledge_external(
        &self,
        content_id: &ContentId,
        locator: &TypedLocator,
    ) -> ExternalAck {
        match self.acknowledge(content_id, locator) {
            AckResult::VerifiedPresent => ExternalAck::VerifiedPresent,
            AckResult::NotPresent | AckResult::Indeterminate => ExternalAck::NotVerifiedPresent,
        }
    }
}

/// The internal existence acknowledgement for one location
/// (A-9115.1). `NotPresent` and `Indeterminate` are separate
/// internal results; only the external form coalesces them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckResult {
    /// A visible, validated location is present (`verified_present`).
    VerifiedPresent,
    /// No visible location is present: it is absent, stale,
    /// deleted, or not authorized for this caller — the policy
    /// layer does not distinguish these.
    NotPresent,
    /// A visible `stored_unverified` observation exists: storage
    /// was asserted, content was never verified. Not proof of
    /// presence.
    Indeterminate,
}

/// The coalesced external existence acknowledgement (A-9115.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAck {
    /// The location is verifiably present for this caller.
    VerifiedPresent,
    /// Anything else: absent, stale, deleted, unverified, or not
    /// authorized — one answer for all of them.
    NotVerifiedPresent,
}

/// Scope-label validation, mirroring the location index's own rule
/// (non-empty, at most [`MAX_LABEL_BYTES`] bytes).
fn check_label(field: &'static str, label: &str) -> Result<(), LocationIndexError> {
    if label.is_empty() {
        return Err(LocationIndexError::Field {
            field,
            problem: "must not be empty",
        });
    }
    if label.len() > MAX_LABEL_BYTES {
        return Err(LocationIndexError::Field {
            field,
            problem: "is too long",
        });
    }
    Ok(())
}
