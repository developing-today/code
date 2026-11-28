//! B16: trusted-relay fan-out bounds — max-hop propagation and
//! breadth × depth resource budgets (#10263, statements S1.54-S1.57 and
//! S1.61; Part B "Query contract").
//!
//! A trusted remote may re-fan-out (S1.54); whether a peer may relay at
//! all is the B31 decision (`fully_trusted` with an explicit relay
//! grant, S1.55). This slice bounds what relaying can cost:
//!
//! - [`ScopedLedger`] is a node's durable local reservation ledger. A
//!   reservation is a vector over [`Dimension`]s, reserved atomically
//!   against every applicable [`Scope`]: the query, the authenticated
//!   requester or origin, the supplying peer, the locally authorized
//!   tenant, and the node aggregate, which every reservation must
//!   include and which stays the final cap. For every scope and
//!   dimension, `held + spent <= limit` always holds and `spent` never
//!   decreases. A reservation with a duplicate scope, a scope or
//!   dimension with no configured limit, or an amount over any cap is
//!   refused with nothing changed.
//! - Locally measured use moves from held to spent as it is recorded,
//!   and may not exceed the reserved amount or name a dimension that was
//!   not reserved. Closing a reservation (cancellation, completion or
//!   expiry) refunds only the unused held amounts; spent work stays
//!   charged. A retry with the same reservation id and the exact same
//!   vector, scopes and expiry returns the same reservation and never
//!   mints more; any other binding for that id is a conflict. A closed
//!   reservation leaves a tombstone until its expiry plus a finite
//!   grace, so it cannot be reopened while the origin request is live.
//! - [`ChildAllocator`] splits the grant a node received among the
//!   children it forwards to. Depth is carried, not shared: each child
//!   gets at most one hop fewer than the grant (S1.56, S1.57), and a
//!   grant with no hops left cannot forward. The other dimensions are
//!   partitioned: the children's allocations, the node's own local use,
//!   and one peer slot per forward together never exceed the grant, so
//!   the whole tree below an origin can contact at most its peer slots
//!   in total, however wide or deep (S1.61). A child's deadline is the
//!   grant's deadline; it is never extended. A retry for a child returns
//!   its exact allocation.
//!
//! Limits are per honest node. Nothing here enforces a network-wide
//! total against Byzantine nodes that fork grants or misreport work.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::{BTreeMap, BTreeSet};

use super::b20_request_identity::RequestId;

/// One reservable resource dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dimension {
    /// Relay depth. In a [`ChildAllocator`] this is carried down the
    /// tree, one fewer per hop, never summed.
    Hops,
    /// Peers that may be contacted (breadth).
    PeerSlots,
    /// Locally metered work units.
    Work,
    /// Bytes.
    Bytes,
}

/// An amount per dimension. A dimension not present is not reserved.
pub type Vector = BTreeMap<Dimension, u64>;

/// An accounting scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// One origin query.
    Query(RequestId),
    /// An authenticated requester or origin.
    Requester(String),
    /// The peer that supplied the request.
    SupplyingPeer(String),
    /// A tenant from local authorization context.
    Tenant(String),
    /// The node aggregate across all tenants and requests.
    Node,
}

/// A reservation's id, stable across retries.
pub type ReservationId = [u8; 32];

/// Why a ledger operation was refused. A refusal changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// The scope list names a scope twice.
    DuplicateScope(Scope),
    /// The scope list does not include the node aggregate.
    MissingNodeScope,
    /// No limit is configured for this scope and dimension.
    NoLimit(Scope, Dimension),
    /// The reservation would take this scope and dimension over its cap.
    OverCap(Scope, Dimension),
    /// The request or reservation has expired.
    Expired,
    /// The id is reserved with a different vector, scope list or expiry.
    BindingConflict,
    /// The reservation is closed and cannot be reopened or used.
    Closed,
    /// No reservation has this id.
    Unknown,
    /// Use was recorded in a dimension that was not reserved.
    OutsideReservation(Dimension),
    /// Use would exceed the amount reserved in this dimension.
    OverReservation(Dimension),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    vector: Vector,
    scopes: Vec<Scope>,
    expires_at: u64,
}

#[derive(Debug, Clone)]
struct Reservation {
    binding: Binding,
    used: Vector,
    closed: bool,
}

/// A node's durable local reservation ledger.
#[derive(Debug, Clone)]
pub struct ScopedLedger {
    limits: BTreeMap<(Scope, Dimension), u64>,
    held: BTreeMap<(Scope, Dimension), u64>,
    spent: BTreeMap<(Scope, Dimension), u64>,
    reservations: BTreeMap<ReservationId, Reservation>,
    grace: u64,
}

fn amount(map: &BTreeMap<(Scope, Dimension), u64>, scope: &Scope, dim: Dimension) -> u64 {
    map.get(&(scope.clone(), dim)).copied().unwrap_or(0)
}

impl ScopedLedger {
    /// A ledger with these caps, keeping tombstones `grace` past expiry.
    pub const fn new(limits: BTreeMap<(Scope, Dimension), u64>, grace: u64) -> Self {
        Self {
            limits,
            held: BTreeMap::new(),
            spent: BTreeMap::new(),
            reservations: BTreeMap::new(),
            grace,
        }
    }

    /// The cap for a scope and dimension, if configured.
    pub fn limit(&self, scope: &Scope, dim: Dimension) -> Option<u64> {
        self.limits.get(&(scope.clone(), dim)).copied()
    }

    /// Reserved and not yet used.
    pub fn held(&self, scope: &Scope, dim: Dimension) -> u64 {
        amount(&self.held, scope, dim)
    }

    /// Used and charged for good.
    pub fn spent(&self, scope: &Scope, dim: Dimension) -> u64 {
        amount(&self.spent, scope, dim)
    }

    /// Reserve `vector` against every scope in `scopes`, atomically.
    pub fn reserve(
        &mut self,
        id: ReservationId,
        vector: Vector,
        scopes: Vec<Scope>,
        expires_at: u64,
        now: u64,
    ) -> Result<Vector, BudgetError> {
        let mut seen = BTreeSet::new();
        for scope in &scopes {
            if !seen.insert(scope) {
                return Err(BudgetError::DuplicateScope(scope.clone()));
            }
        }
        if !seen.contains(&Scope::Node) {
            return Err(BudgetError::MissingNodeScope);
        }
        let binding = Binding {
            vector,
            scopes,
            expires_at,
        };
        if let Some(existing) = self.reservations.get(&id) {
            if existing.binding != binding {
                return Err(BudgetError::BindingConflict);
            }
            if existing.closed {
                return Err(BudgetError::Closed);
            }
            if now >= expires_at {
                return Err(BudgetError::Expired);
            }
            return Ok(binding.vector);
        }
        if now >= expires_at {
            return Err(BudgetError::Expired);
        }
        let over_cap = |this: &Self| -> Result<Option<BudgetError>, BudgetError> {
            for scope in &binding.scopes {
                for (&dim, &want) in &binding.vector {
                    let limit = this
                        .limit(scope, dim)
                        .ok_or_else(|| BudgetError::NoLimit(scope.clone(), dim))?;
                    let used = this.held(scope, dim) + this.spent(scope, dim);
                    if used.checked_add(want).is_none_or(|total| total > limit) {
                        return Ok(Some(BudgetError::OverCap(scope.clone(), dim)));
                    }
                }
            }
            Ok(None)
        };
        if let Some(err) = over_cap(self)? {
            self.close_expired(now);
            if let Some(err) = over_cap(self)? {
                return Err(err);
            }
        }
        for scope in &binding.scopes {
            for (&dim, &want) in &binding.vector {
                *self.held.entry((scope.clone(), dim)).or_default() += want;
            }
        }
        let vector = binding.vector.clone();
        self.reservations.insert(
            id,
            Reservation {
                binding,
                used: Vector::new(),
                closed: false,
            },
        );
        Ok(vector)
    }

    /// Record locally measured use against an open reservation; it is
    /// charged to every scope at once.
    pub fn record_use(
        &mut self,
        id: &ReservationId,
        delta: &Vector,
        now: u64,
    ) -> Result<(), BudgetError> {
        let r = self.reservations.get(id).ok_or(BudgetError::Unknown)?;
        if r.closed {
            return Err(BudgetError::Closed);
        }
        if now >= r.binding.expires_at {
            return Err(BudgetError::Expired);
        }
        for (&dim, &more) in delta {
            let reserved = r
                .binding
                .vector
                .get(&dim)
                .copied()
                .ok_or(BudgetError::OutsideReservation(dim))?;
            let used = r.used.get(&dim).copied().unwrap_or(0);
            if used.checked_add(more).is_none_or(|total| total > reserved) {
                return Err(BudgetError::OverReservation(dim));
            }
        }
        let scopes = r.binding.scopes.clone();
        let r = self.reservations.get_mut(id).ok_or(BudgetError::Unknown)?;
        for (&dim, &more) in delta {
            *r.used.entry(dim).or_default() += more;
            for scope in &scopes {
                let key = (scope.clone(), dim);
                *self.held.entry(key.clone()).or_default() -= more;
                *self.spent.entry(key).or_default() += more;
            }
        }
        Ok(())
    }

    /// Close a reservation, refunding its unused held amounts. Closing
    /// it again refunds nothing.
    pub fn close(&mut self, id: &ReservationId) -> Result<Vector, BudgetError> {
        let r = self.reservations.get_mut(id).ok_or(BudgetError::Unknown)?;
        if r.closed {
            return Ok(Vector::new());
        }
        r.closed = true;
        let mut refund = Vector::new();
        for (&dim, &reserved) in &r.binding.vector {
            let unused = reserved - r.used.get(&dim).copied().unwrap_or(0);
            refund.insert(dim, unused);
            for scope in &r.binding.scopes {
                *self.held.entry((scope.clone(), dim)).or_default() -= unused;
            }
        }
        Ok(refund)
    }

    /// Close every open reservation whose expiry has passed.
    pub fn close_expired(&mut self, now: u64) -> Vec<ReservationId> {
        let due: Vec<ReservationId> = self
            .reservations
            .iter()
            .filter(|(_, r)| !r.closed && now >= r.binding.expires_at)
            .map(|(id, _)| *id)
            .collect();
        for id in &due {
            let _ = self.close(id);
        }
        due
    }

    /// Drop tombstones whose expiry plus grace has passed; returns how
    /// many were dropped. Open reservations are never dropped.
    pub fn prune(&mut self, now: u64) -> usize {
        let before = self.reservations.len();
        let grace = self.grace;
        self.reservations
            .retain(|_, r| !r.closed || now < r.binding.expires_at.saturating_add(grace));
        before - self.reservations.len()
    }

    /// Whether a reservation is open.
    pub fn is_open(&self, id: &ReservationId) -> bool {
        self.reservations.get(id).is_some_and(|r| !r.closed)
    }
}

/// The budget a node received for one request: its relay grant's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// The request.
    pub request_id: RequestId,
    /// Amounts; `Hops` is the remaining depth.
    pub vector: Vector,
    /// The request's deadline, never extended.
    pub deadline: u64,
}

/// Why a child allocation or local use was refused. A refusal changes
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocError {
    /// The deadline has passed.
    Expired,
    /// The grant has no hops left, so it cannot forward.
    NoHopsLeft,
    /// The child asked for at least as many hops as the grant has.
    TooDeep,
    /// The amount names a dimension the grant does not carry.
    OutsideGrant(Dimension),
    /// Not enough of this dimension is left unallocated.
    Exhausted(Dimension),
    /// The child already has a different allocation.
    ChildConflict,
}

/// Splits a received grant among forwarded children and local use.
#[derive(Debug, Clone)]
pub struct ChildAllocator {
    grant: Grant,
    local: Vector,
    children: BTreeMap<String, Vector>,
}

impl ChildAllocator {
    /// Start allocating from `grant`.
    pub const fn new(grant: Grant) -> Self {
        Self {
            grant,
            local: Vector::new(),
            children: BTreeMap::new(),
        }
    }

    fn committed(&self, dim: Dimension) -> u64 {
        let children: u64 = self.children.values().filter_map(|v| v.get(&dim)).sum();
        let forwards = if dim == Dimension::PeerSlots {
            self.children.len() as u64
        } else {
            0
        };
        self.local.get(&dim).copied().unwrap_or(0) + children + forwards
    }

    /// What is left unallocated, per carried dimension except hops.
    pub fn remaining(&self) -> Vector {
        self.grant
            .vector
            .iter()
            .filter(|(dim, _)| **dim != Dimension::Hops)
            .map(|(&dim, &total)| (dim, total - self.committed(dim)))
            .collect()
    }

    fn check(&self, extra: &Vector) -> Result<(), AllocError> {
        for (&dim, &want) in extra {
            if dim == Dimension::Hops {
                continue;
            }
            let total = self
                .grant
                .vector
                .get(&dim)
                .copied()
                .ok_or(AllocError::OutsideGrant(dim))?;
            if self
                .committed(dim)
                .checked_add(want)
                .is_none_or(|sum| sum > total)
            {
                return Err(AllocError::Exhausted(dim));
            }
        }
        Ok(())
    }

    /// Allocate `vector` to `child`, which costs one extra peer slot for
    /// the forward itself. The child's `Hops` is its depth and must be
    /// below the grant's.
    pub fn allocate(&mut self, child: &str, vector: Vector, now: u64) -> Result<Grant, AllocError> {
        if now >= self.grant.deadline {
            return Err(AllocError::Expired);
        }
        if let Some(existing) = self.children.get(child) {
            return if *existing == vector {
                Ok(self.child_grant(vector))
            } else {
                Err(AllocError::ChildConflict)
            };
        }
        let hops = self
            .grant
            .vector
            .get(&Dimension::Hops)
            .copied()
            .unwrap_or(0);
        if hops == 0 {
            return Err(AllocError::NoHopsLeft);
        }
        if vector.get(&Dimension::Hops).copied().unwrap_or(0) >= hops {
            return Err(AllocError::TooDeep);
        }
        let mut need = vector.clone();
        let slots = need.entry(Dimension::PeerSlots).or_default();
        *slots = slots
            .checked_add(1)
            .ok_or(AllocError::Exhausted(Dimension::PeerSlots))?;
        self.check(&need)?;
        self.children.insert(child.to_owned(), vector.clone());
        Ok(self.child_grant(vector))
    }

    const fn child_grant(&self, vector: Vector) -> Grant {
        Grant {
            request_id: self.grant.request_id,
            vector,
            deadline: self.grant.deadline,
        }
    }

    /// Charge locally measured use against what is left.
    pub fn consume(&mut self, delta: &Vector, now: u64) -> Result<(), AllocError> {
        if now >= self.grant.deadline {
            return Err(AllocError::Expired);
        }
        if let Some(dim) = delta.keys().find(|d| **d == Dimension::Hops) {
            return Err(AllocError::OutsideGrant(*dim));
        }
        self.check(delta)?;
        for (&dim, &more) in delta {
            *self.local.entry(dim).or_default() += more;
        }
        Ok(())
    }

    /// The children allocated so far.
    pub const fn children(&self) -> &BTreeMap<String, Vector> {
        &self.children
    }
}
