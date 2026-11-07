//! Pure writer-set projection and subject discovery (§§4–5 of the writer-set
//! design). The caller supplies the immutable event and membership snapshot;
//! this module never consults live store state.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Bound::{Excluded, Unbounded};

use p2panda_core::Hash;

use crate::groups::Role;
use crate::waiting::AuthorKey;

use super::{Entry, Excludes, Rescind, WriterSet};

/// One-level, frontier-pinned direct membership for a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMembers {
    /// The group being expanded.
    pub group: Hash,
    /// The publishing event's signed causal frontier.
    pub frontier: Vec<Hash>,
    /// The resolution snapshot under which membership was resolved.
    pub resolution: Hash,
    /// Direct key members and their resolved roles. Group-valued members are
    /// deliberately not represented: writer-set expansion does not recurse.
    pub members: BTreeMap<AuthorKey, Role>,
}

impl GroupMembers {
    /// A membership result for one `(group, frontier, resolution)` tuple.
    #[must_use]
    pub fn new(
        group: Hash,
        frontier: Vec<Hash>,
        resolution: Hash,
        members: BTreeMap<AuthorKey, Role>,
    ) -> Self {
        Self {
            group,
            frontier,
            resolution,
            members,
        }
    }
}

/// An expanded entry set. A public admission is represented without
/// materializing every known author; exclusions always take precedence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpandedWriters {
    public: bool,
    authors: BTreeSet<AuthorKey>,
    excludes_public: bool,
    excluded_authors: BTreeSet<AuthorKey>,
}

impl ExpandedWriters {
    /// Whether `author` is admitted after exclusions are applied.
    #[must_use]
    pub fn contains(&self, author: &AuthorKey) -> bool {
        !self.excludes_public
            && (self.public || self.authors.contains(author))
            && !self.excluded_authors.contains(author)
    }

    /// Whether this expansion admits every author before exclusions.
    #[must_use]
    pub const fn is_public(&self) -> bool {
        self.public
    }

    /// Explicitly named authors. A public expansion still requires an
    /// all-authors index lookup by its caller.
    #[must_use]
    pub fn explicit_authors(&self) -> &BTreeSet<AuthorKey> {
        &self.authors
    }
}

/// A projection or discovery input cannot be evaluated as specified.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    /// A referenced event or causal dependency is absent from the snapshot.
    #[error("the pinned snapshot is missing event {0}")]
    MissingEvent(Hash),
    /// No resolved one-level membership was supplied for this tuple.
    #[error("the pinned snapshot is missing group {group} at its frontier under resolution {resolution}")]
    MissingGroupMembers {
        /// The group whose expansion is missing.
        group: Hash,
        /// The resolution snapshot required by the view.
        resolution: Hash,
    },
    /// Two events in the input use the same ID.
    #[error("the pinned snapshot contains duplicate event {0}")]
    DuplicateEvent(Hash),
    /// A selected view or writer-set input does not pass its pinned validity gate.
    #[error("the selected {0} configuration event is not applied under the pinned resolution")]
    Undefined(&'static str),
    /// The event metadata does not agree with the writer-set object it carries.
    #[error("writer-set event {0} does not match its signed author, sequence, or selected set")]
    InvalidWriterSet(Hash),
    /// A set event has the wrong body shape for its event kind.
    #[error("event {0} has a writer-set kind but no matching decoded object")]
    MissingWriterSet(Hash),
    /// A continuation cursor belongs to another snapshot or query.
    #[error("the discovery cursor does not belong to this snapshot and query")]
    StaleCursor,
    /// The discovery budget cannot make progress or is invalid.
    #[error("the discovery budget must allow at least one indexed row")]
    InvalidBudget,
    /// The first row to scan is larger than the byte budget.
    #[error("the next indexed row is larger than the discovery byte budget")]
    BudgetTooSmall,
}

/// The least role a group member must hold, in the project's role order.
fn role_at_least(actual: Role, minimum: Role) -> bool {
    actual >= minimum
}

/// Expands a writer-set's entries at their publishing frontier.
///
/// For `retro`, only admission entries with `include_history` contribute; all
/// exclusions still apply. Group snapshots are direct-member snapshots and are
/// selected by the complete `(group, frontier, resolution)` tuple.
///
/// # Errors
///
/// Returns [`ProjectionError::MissingGroupMembers`] when a group entry has no
/// matching pinned membership snapshot.
pub fn expand_set(
    set: &WriterSet,
    frontier: &[Hash],
    resolution: &Hash,
    groups: &[GroupMembers],
    retro: bool,
) -> Result<ExpandedWriters, ProjectionError> {
    let mut expanded = ExpandedWriters::default();
    add_entries(
        &mut expanded,
        &set.entries,
        frontier,
        resolution,
        groups,
        retro,
        false,
    )?;
    add_entries(
        &mut expanded,
        &set.excludes,
        frontier,
        resolution,
        groups,
        false,
        true,
    )?;
    Ok(expanded)
}

fn add_entries(
    expanded: &mut ExpandedWriters,
    entries: &[Entry],
    frontier: &[Hash],
    resolution: &Hash,
    groups: &[GroupMembers],
    history_only: bool,
    exclusion: bool,
) -> Result<(), ProjectionError> {
    for entry in entries {
        if history_only && !entry.include_history() {
            continue;
        }
        match entry {
            Entry::Key { author, .. } => {
                if exclusion {
                    expanded.excluded_authors.insert(*author);
                } else {
                    expanded.authors.insert(*author);
                }
            }
            Entry::Group {
                id, role_min, ..
            } => {
                let members = groups
                    .iter()
                    .find(|snapshot| {
                        snapshot.group == *id
                            && snapshot.frontier == frontier
                            && snapshot.resolution == *resolution
                    })
                    .ok_or(ProjectionError::MissingGroupMembers {
                        group: *id,
                        resolution: *resolution,
                    })?;
                for (author, role) in &members.members {
                    if role_at_least(*role, *role_min) {
                        if exclusion {
                            expanded.excluded_authors.insert(*author);
                        } else {
                            expanded.authors.insert(*author);
                        }
                    }
                }
            }
            Entry::Public { .. } => {
                if exclusion {
                    expanded.excludes_public = true;
                } else {
                    expanded.public = true;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::idset::{Entry, Excludes, WriterSet};

    fn author(n: u8) -> AuthorKey {
        [n; 32]
    }

    fn hash(n: u8) -> Hash {
        Hash::new([n; 32])
    }

    fn set(entries: Vec<Entry>, excludes: Vec<Entry>) -> WriterSet {
        WriterSet {
            set_id: hash(90),
            version: 3,
            owner: author(1),
            scope: Some("test".to_owned()),
            entries,
            excludes: Excludes(excludes),
        }
    }

    #[test]
    fn test_retro_expansion_uses_only_history_marked_entries_and_exclusions_win() {
        let group = hash(10);
        let frontier = vec![hash(11)];
        let resolution = hash(12);
        let members = GroupMembers::new(
            group,
            frontier.clone(),
            resolution,
            BTreeMap::from([(author(2), Role::Read), (author(3), Role::Write)]),
        );
        let set = set(
            vec![
                Entry::Key {
                    author: author(2),
                    include_history: true,
                },
                Entry::Key {
                    author: author(4),
                    include_history: false,
                },
                Entry::Group {
                    id: group,
                    role_min: Role::Write,
                    include_history: true,
                },
            ],
            vec![Entry::Key {
                author: author(3),
                include_history: false,
            }],
        );

        let expanded = expand_set(&set, &frontier, &resolution, &[members], true).unwrap();

        assert!(expanded.contains(&author(2)));
        assert!(!expanded.contains(&author(3)));
        assert!(!expanded.contains(&author(4)));
    }

    #[test]
    fn test_public_exclusion_admits_nobody() {
        let set = set(
            vec![Entry::Public {
                include_history: true,
            }],
            vec![Entry::Public {
                include_history: false,
            }],
        );

        let expanded = expand_set(&set, &[], &hash(12), &[], false).unwrap();

        assert!(expanded.is_public());
        assert!(!expanded.contains(&author(2)));
    }
}
