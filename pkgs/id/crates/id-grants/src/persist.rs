//! A stored form of a [`GrantSet`] (#14790 slice 2b, first part).
//!
//! [`GrantSet::to_snapshot`] writes a versioned [`GrantSetSnapshot`]
//! holding every grant, every revocation and every served
//! `snapshot_once` grant; the host stores it however it likes.
//! [`GrantSet::from_snapshot`] reads one back and re-checks it as if the
//! grants were being issued again: each grant passes
//! [`Grant::validate`], each delegated grant passes the attenuation
//! check against its parent, and every revocation and served snapshot
//! names a grant in the set. So a damaged or hand-edited snapshot cannot
//! load a grant wider than its parent, a public grant with `write`, or a
//! delegation from a grant that was never delegable.
//!
//! Two checks made when a grant was first issued are not repeated on
//! load, because they depend on the moment of issue: whether the parent
//! was live then, and whether the delegator belonged to the parent's
//! group. The delegator is taken to be the child's issuer, a member of
//! the parent's subject. Expiry and revocation are still checked at
//! every authorization, so loading an expired grant never makes it
//! allow anything.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    Duration, Grant, GrantError, GrantId, GrantSet, Requester, Subject, check_attenuation,
};

/// The snapshot format this crate writes and reads.
pub const SNAPSHOT_FORMAT: u32 = 1;

/// The stored form of a [`GrantSet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantSetSnapshot {
    /// The snapshot format; [`SNAPSHOT_FORMAT`] for this crate.
    pub format: u32,
    /// Every grant, parents before their children.
    pub grants: Vec<Grant>,
    /// The revoked grants.
    pub revoked: BTreeSet<GrantId>,
    /// The `snapshot_once` grants whose snapshot was served.
    pub consumed: BTreeSet<GrantId>,
}

/// Why a snapshot did not load. Nothing is loaded when any check fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The snapshot's format is not one this crate reads.
    UnknownFormat(u32),
    /// Two grants share an id.
    DuplicateId(GrantId),
    /// This grant names a parent that is not in the snapshot, or its
    /// parent chain loops.
    UnknownParent(GrantId),
    /// This grant fails the grants model's checks.
    Invalid(GrantId, GrantError),
    /// A revocation or served snapshot names a grant not in the set.
    UnknownId(GrantId),
    /// A served snapshot names a grant that is not `snapshot_once`.
    NotSnapshotOnce(GrantId),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat(format) => write!(f, "unknown grant snapshot format {format}"),
            Self::DuplicateId(id) => write!(f, "grant {id} appears twice"),
            Self::UnknownParent(id) => write!(f, "grant {id} has no loadable parent"),
            Self::Invalid(id, err) => write!(f, "grant {id} is invalid: {err}"),
            Self::UnknownId(id) => write!(f, "{id} names no grant in the snapshot"),
            Self::NotSnapshotOnce(id) => write!(f, "grant {id} is not snapshot_once"),
        }
    }
}

impl std::error::Error for LoadError {}

/// The delegator a stored delegated grant is re-checked against: its
/// issuer, as a member of the parent's group when the parent names one.
fn delegator_for(parent: &Grant, child: &Grant) -> Requester {
    let mut groups = BTreeSet::new();
    if let Subject::Group(group) = &parent.subject {
        groups.insert(group.clone());
    }
    Requester {
        identity: child.issuer.clone(),
        groups,
    }
}

impl GrantSet {
    /// The stored form of this set, parents before their children.
    #[must_use]
    pub fn to_snapshot(&self) -> GrantSetSnapshot {
        let mut ordered: Vec<Grant> = Vec::with_capacity(self.grants.len());
        let mut placed: BTreeSet<&GrantId> = BTreeSet::new();
        // Each pass places every grant whose parent is already placed;
        // a set built through the insert methods always finishes.
        while placed.len() < self.grants.len() {
            let before = placed.len();
            for grant in self.grants.values() {
                if placed.contains(&grant.id) {
                    continue;
                }
                if grant.parent.as_ref().is_none_or(|p| placed.contains(p)) {
                    placed.insert(&grant.id);
                    ordered.push(grant.clone());
                }
            }
            if placed.len() == before {
                break;
            }
        }
        GrantSetSnapshot {
            format: SNAPSHOT_FORMAT,
            grants: ordered,
            revoked: self.revoked.clone(),
            consumed: self.consumed.clone(),
        }
    }

    /// Loads a stored set, re-checking every grant (see the module
    /// docs). Grants may come in any order.
    ///
    /// # Errors
    ///
    /// A [`LoadError`] for the first check that failed; nothing loads.
    pub fn from_snapshot(snapshot: GrantSetSnapshot) -> Result<Self, LoadError> {
        if snapshot.format != SNAPSHOT_FORMAT {
            return Err(LoadError::UnknownFormat(snapshot.format));
        }
        let mut pending: BTreeMap<GrantId, Grant> = BTreeMap::new();
        for grant in snapshot.grants {
            if pending.contains_key(&grant.id) {
                return Err(LoadError::DuplicateId(grant.id));
            }
            pending.insert(grant.id.clone(), grant);
        }
        let mut set = Self::new();
        while !pending.is_empty() {
            let ready: Vec<GrantId> = pending
                .values()
                .filter(|grant| {
                    grant
                        .parent
                        .as_ref()
                        .is_none_or(|p| set.grants.contains_key(p))
                })
                .map(|grant| grant.id.clone())
                .collect();
            if ready.is_empty() {
                // Whatever is left names a missing parent or loops.
                if let Some(stuck) = pending.into_keys().next() {
                    return Err(LoadError::UnknownParent(stuck));
                }
                break;
            }
            for id in ready {
                let Some(grant) = pending.remove(&id) else {
                    continue;
                };
                grant
                    .validate()
                    .map_err(|err| LoadError::Invalid(id.clone(), err))?;
                if let Some(parent_id) = &grant.parent {
                    let parent = set
                        .grants
                        .get(parent_id)
                        .ok_or_else(|| LoadError::UnknownParent(id.clone()))?;
                    check_attenuation(parent, &grant, &delegator_for(parent, &grant))
                        .map_err(|err| LoadError::Invalid(id.clone(), err))?;
                }
                set.grants.insert(id, grant);
            }
        }
        for id in &snapshot.revoked {
            if !set.grants.contains_key(id) {
                return Err(LoadError::UnknownId(id.clone()));
            }
        }
        for id in &snapshot.consumed {
            match set.grants.get(id) {
                None => return Err(LoadError::UnknownId(id.clone())),
                Some(grant) if grant.duration != Duration::SnapshotOnce => {
                    return Err(LoadError::NotSnapshotOnce(id.clone()));
                }
                Some(_) => {}
            }
        }
        set.revoked = snapshot.revoked;
        set.consumed = snapshot.consumed;
        Ok(set)
    }
}
