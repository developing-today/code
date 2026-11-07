//! Store-wired writer-set expansion: `expand` / `expand_retro` over the
//! membership layer (writer-set spec §9 item 2; issue #4998, origin #3789).
//!
//! [`super::view::expand_set`] is the pure projection: the caller supplies a
//! pinned membership snapshot and it evaluates the entry logic. This module
//! is the wiring that produces that snapshot from the store's membership
//! layer, pinned to the set version's own frontier:
//!
//! - Group entries resolve through
//!   [`Membership::members_at_position`] — the enumeration counterpart of
//!   `is_member_at` / `access_at_position` — at the set version's pinned
//!   frontier, **never** through the live `Membership::members` roster
//!   (spec §4.5). A membership change after the pinned frontier therefore
//!   cannot change a set version's expansion (spec §D4).
//! - Group-valued members are not followed: expansion flattens exactly one
//!   level (spec §D3/D4), because `members_at_position` returns key members
//!   only.
//! - A member's role is their access at that frontier, mapped the way the
//!   groups fold maps it (`access_role` in `crate::groups::fold`). The
//!   membership layer records Admin as Manage — Admin is a policy role the
//!   groups fold tracks apart from access — so it cannot distinguish an
//!   admin from any other manager at a frontier. A `role_min` of
//!   [`Role::Admin`] is therefore evaluated at [`Role::Manage`], the
//!   highest role the layer can answer for: admins are admitted (they are
//!   managers in the layer), and so are non-admin managers. Answering
//!   Admin exactly needs the fold's admin-at-frontier state plumbed into
//!   expansion, which no caller can supply yet; the limitation is pinned
//!   by a named test beside the spec tests (`test_expand_role_min_admin_*`
//!   in `tests/writer_set_expansion.rs`).
//! - Resolved groups are cached under the §4.5 key `(group, frontier, R)`,
//!   where `R` is the caller's resolution-state identity
//!   ([`SetVersion::resolution`]). Within one resolution state an expansion
//!   is immutable and cacheable forever; across resolution states — a fork
//!   proof invalidating a grant is exactly such a move — the cached entry
//!   is not served and the group is resolved again.
//!
//! The result is authors only: no counting path is enabled here (counting
//! is spec §9 item 4). A set carrying a `Public` entry cannot be
//! materialized as a set of authors — every author is admitted — so it is
//! refused with [`ExpandError::PublicEntry`] rather than silently
//! returning only the explicitly named authors; callers that need the
//! public shape use [`ExpandedWriters`] from [`super::view`] directly.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use p2panda_auth::Access;
use p2panda_core::Hash;

use crate::groups::Role;
use crate::membership::{Membership, MembershipError};
use crate::waiting::AuthorKey;

use super::view::{expand_set, GroupMembers, ProjectionError};
use super::{Entry, WriterSet};

/// One set version, pinned for expansion: the set object, the causal
/// frontier its publishing event was expanded at (spec §D4), and the
/// resolution state `R` the expansion is evaluated under (spec §4.5).
#[derive(Clone, Debug)]
pub struct SetVersion<'a> {
    /// The writer set at this version.
    pub set: &'a WriterSet,
    /// The publishing event's signed causal frontier: the `after` hashes
    /// group membership is judged at.
    pub frontier: Vec<Hash>,
    /// The resolution state's identity. Two expansions that differ only in
    /// this value are different evaluations and are cached apart.
    pub resolution: Hash,
}

/// Why a store-wired expansion could not be evaluated.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExpandError {
    /// The membership layer refused a frontier lookup.
    #[error(transparent)]
    Membership(#[from] MembershipError),
    /// The pure projection refused the pinned snapshot.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    /// The set admits the public, which no `BTreeSet<AuthorKey>` can
    /// materialize. Use `view::expand_set`'s [`ExpandedWriters`] instead.
    ///
    /// [`ExpandedWriters`]: super::view::ExpandedWriters
    #[error(
        "the set has a public entry, so its expansion is every author and cannot be materialized as a set of authors"
    )]
    PublicEntry,
}

/// The §4.5 cache key: `(group, frontier, R)`. The frontier is stored
/// sorted, so two spellings of the same causal position share one entry.
type CacheKey = (Hash, Vec<Hash>, Hash);

/// Expands set versions against one membership layer, caching each group's
/// frontier-pinned resolution under its §4.5 key for the expander's life.
/// One expander serves one resolution epoch of one store: when the store's
/// resolution state moves (a fork proof lands), build a fresh expander
/// over the rebuilt layer and name the new `R` in each [`SetVersion`].
pub struct Expander<'a> {
    membership: &'a mut Membership,
    cache: HashMap<CacheKey, BTreeMap<AuthorKey, Role>>,
}

impl<'a> Expander<'a> {
    /// An expander over `membership` with an empty cache.
    pub fn new(membership: &'a mut Membership) -> Self {
        Self {
            membership,
            cache: HashMap::new(),
        }
    }

    /// The number of `(group, frontier, R)` resolutions currently cached.
    /// Exposed so tests and metrics can pin the §4.5 key's behaviour: a
    /// repeat expansion under the same key adds nothing, and the same
    /// `(group, frontier)` under a new `R` adds a second entry instead of
    /// serving the stale one.
    #[must_use]
    pub fn cache_entries(&self) -> usize {
        self.cache.len()
    }

    /// `expand(set_version) -> BTreeSet<AuthorKey>` (spec §3.1): every
    /// author this set version admits, exclusions applied (spec §D6).
    ///
    /// # Errors
    ///
    /// Returns [`ExpandError::Membership`] when the layer cannot judge the
    /// pinned frontier, [`ExpandError::Projection`] when the pinned
    /// snapshot is incomplete, and [`ExpandError::PublicEntry`] when the
    /// set admits the public.
    pub fn expand(&mut self, version: &SetVersion<'_>) -> Result<BTreeSet<AuthorKey>, ExpandError> {
        self.expand_narrowed(version, false)
    }

    /// `expand_retro(set_version)`: the same expansion narrowed to the
    /// entries carrying `include_history` (spec §3.1/§4.1). The narrowing
    /// is per entry, so one flagged entry never imports another entry's
    /// backlog, and `expand_retro(v) ⊆ expand(v)` always holds.
    ///
    /// # Errors
    ///
    /// The same failures as [`Expander::expand`], except that a public
    /// entry carrying no `include_history` does not contribute to the
    /// retro expansion and so does not refuse it.
    pub fn expand_retro(
        &mut self,
        version: &SetVersion<'_>,
    ) -> Result<BTreeSet<AuthorKey>, ExpandError> {
        self.expand_narrowed(version, true)
    }

    fn expand_narrowed(
        &mut self,
        version: &SetVersion<'_>,
        retro: bool,
    ) -> Result<BTreeSet<AuthorKey>, ExpandError> {
        // A public exclusion admits nobody (spec §D6, the emergency
        // stop): the expansion is the finite empty set even when the
        // admissions are public, so the public-admission refusal below
        // must not fire for that configuration.
        let public_excluded = version
            .set
            .excludes
            .0
            .iter()
            .any(|entry| matches!(entry, Entry::Public { .. }));
        if admits_public(version.set, retro) && !public_excluded {
            return Err(ExpandError::PublicEntry);
        }
        // The membership layer's highest distinguishable role is Manage
        // (Admin is recorded as Manage there), so an Admin threshold is
        // evaluated at Manage for the layer-backed snapshot; see the
        // module doc for the limitation and its follow-up.
        let effective;
        let set = if has_admin_threshold(version.set) {
            effective = clamp_admin_thresholds(version.set);
            &effective
        } else {
            version.set
        };
        let mut groups = Vec::new();
        for group in referenced_groups(set) {
            let members = self.group_members(group, version)?;
            groups.push(GroupMembers::new(
                group,
                version.frontier.clone(),
                version.resolution,
                members,
            ));
        }
        let expanded = expand_set(set, &version.frontier, &version.resolution, &groups, retro)?;
        // `ExpandedWriters` keeps exclusions apart from the admitted set —
        // only `contains` applies them — so the materialized expansion is
        // the admitted authors that survive exclusion (spec §D6), not the
        // raw admitted set.
        Ok(expanded
            .explicit_authors()
            .iter()
            .copied()
            .filter(|author| expanded.contains(author))
            .collect())
    }

    /// The frontier-pinned direct members of `group` under the version's
    /// resolution state, from the cache when this exact §4.5 key was
    /// already resolved.
    fn group_members(
        &mut self,
        group: Hash,
        version: &SetVersion<'_>,
    ) -> Result<BTreeMap<AuthorKey, Role>, ExpandError> {
        let mut frontier = version.frontier.clone();
        frontier.sort_unstable();
        let key = (group, frontier, version.resolution);
        if let Some(members) = self.cache.get(&key) {
            return Ok(members.clone());
        }
        let members: BTreeMap<AuthorKey, Role> = self
            .membership
            .members_at_position(&version.frontier, group)?
            .into_iter()
            .map(|(author, access)| (author, access_role(&access)))
            .collect();
        self.cache.insert(key, members.clone());
        Ok(members)
    }
}

/// Whether the set's admissions (narrowed to history-carrying entries when
/// `retro`) include a public entry. Exclusions never admit, so a public
/// exclusion does not count.
fn admits_public(set: &WriterSet, retro: bool) -> bool {
    set.entries
        .iter()
        .any(|entry| matches!(entry, Entry::Public { .. }) && (!retro || entry.include_history()))
}

/// Every group named by an admission or an exclusion entry, in first-use
/// order. Both lists can carry group entries, and the projection needs a
/// pinned snapshot for each one it evaluates.
fn referenced_groups(set: &WriterSet) -> Vec<Hash> {
    let mut groups = Vec::new();
    for entry in set.entries.iter().chain(set.excludes.0.iter()) {
        if let Entry::Group { id, .. } = entry {
            if !groups.contains(id) {
                groups.push(*id);
            }
        }
    }
    groups
}

/// A member's role as their membership access gives it. Mirrors the groups
/// fold's `access_role`: Admin is recorded apart from the access level and
/// shows as Manage here.
fn access_role(access: &Access<()>) -> Role {
    if access.is_manage() {
        Role::Manage
    } else if access.is_write() {
        Role::Write
    } else if access.is_read() {
        Role::Read
    } else {
        Role::Pull
    }
}

/// Whether any group entry in the set (admissions or exclusions) carries
/// an Admin `role_min` threshold.
fn has_admin_threshold(set: &WriterSet) -> bool {
    set.entries
        .iter()
        .chain(set.excludes.0.iter())
        .any(|entry| {
            matches!(
                entry,
                Entry::Group {
                    role_min: Role::Admin,
                    ..
                }
            )
        })
}

/// A copy of `set` with every Admin `role_min` lowered to Manage, the
/// highest role the membership layer distinguishes at a frontier.
fn clamp_admin_thresholds(set: &WriterSet) -> WriterSet {
    fn clamp(entries: &[Entry]) -> Vec<Entry> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Group {
                    id,
                    role_min: Role::Admin,
                    include_history,
                } => Entry::Group {
                    id: *id,
                    role_min: Role::Manage,
                    include_history: *include_history,
                },
                other => other.clone(),
            })
            .collect()
    }
    WriterSet {
        set_id: set.set_id,
        version: set.version,
        owner: set.owner,
        scope: set.scope.clone(),
        entries: clamp(&set.entries),
        excludes: super::Excludes(clamp(&set.excludes.0)),
    }
}
