//! Replica fact export, split out of directory.rs for the F11.10
//! production-line guard (#8591). The method stays an inherent
//! Directory method at its historical path; only its home file moved.

use super::validate::pair;
use super::*;

impl Directory {
    /// Directory facts a read-only replica needs for spectator
    /// authorization (F12.13 phase 4): everything
    /// [`WorldCore::effective_scopes`] consults for the given
    /// capability subjects — account ceilings (tiers and group grants) and
    /// the friendship graph — as entries the replica applies to an empty
    /// directory. The caller passes every account appearing in the
    /// replicated journal's capability chains; unknown subjects are skipped
    /// (they read back under the anonymous tier, fail-closed).
    ///
    /// Account secrets never cross: signed-up accounts carry only their
    /// stored digest, keyed accounts only their public ID. Friendships are
    /// re-expressed without envelope IDs — the request/accept pair
    /// validates as a unit, and only current friendships are exported, so
    /// there is nothing to void. Groups export only when the groups a
    /// subject's level flows through form a contiguous `1..=k` prefix (the
    /// journal replays groups in creation order); group founders are the
    /// lowest-ID account already holding admin, so no authority widens.
    ///
    /// # Errors
    ///
    /// Fails when a group the subjects depend on cannot be represented as
    /// such a prefix, so a gappy topology can never silently narrow a
    /// subject's ceiling. Worlds without group grants (like the phase-4
    /// friendship-delegation acceptance case) never hit this.
    pub fn replica_facts_for(&self, subjects: &BTreeSet<String>) -> Result<Vec<DirectoryEntry>> {
        // Groups that grant a subject some level, closed under nesting: a
        // subject's level can flow through a nested group, and `MemberSet`
        // refuses an unknown group.
        let mut groups: BTreeSet<u64> = BTreeSet::new();
        let mut stack: Vec<u64> = self
            .groups
            .keys()
            .filter(|id| subjects.iter().any(|s| self.level_in(s, **id).is_some()))
            .copied()
            .collect();
        while let Some(id) = stack.pop() {
            if groups.insert(id) {
                for ((_, member), _) in self
                    .members
                    .range((id, Member::Group { id: 0 })..=(id, Member::Group { id: u64::MAX }))
                {
                    if let Member::Group { id: nested } = member {
                        stack.push(*nested);
                    }
                }
            }
        }
        // Journal replay creates groups in order, so the export must be the
        // contiguous prefix starting at group 1.
        let ordered: Vec<u64> = groups.iter().copied().collect();
        let contiguous = ordered
            .iter()
            .enumerate()
            .all(|(index, id)| u64::try_from(index + 1).is_ok_and(|expected| expected == *id));
        ensure!(
            contiguous,
            "replica export cannot represent non-contiguous group IDs {ordered:?}"
        );
        // Accounts to reproduce: the subjects that exist, plus founders of
        // exported groups (`GroupCreated` refuses an unknown founder).
        let mut accounts: BTreeSet<String> = subjects
            .iter()
            .filter(|id| self.accounts.contains_key(*id))
            .cloned()
            .collect();
        let mut founders: BTreeMap<u64, String> = BTreeMap::new();
        for id in &ordered {
            // Admin can be inherited through nested-group membership; use
            // the same effective-level calculation as `ensure_admins`.
            let founder = self
                .accounts
                .keys()
                .filter(|account| self.level_in(account.as_str(), *id) == Some(Level::Admin))
                .min()
                .cloned();
            let Some(founder) = founder else {
                anyhow::bail!("replica export cannot represent group {id} with no account admin");
            };
            accounts.insert(founder.clone());
            founders.insert(*id, founder);
        }
        let mut entries = Vec::new();
        let mut sorted_accounts: Vec<&String> = accounts.iter().collect();
        sorted_accounts.sort();
        for id in sorted_accounts {
            let account = &self.accounts[id.as_str()];
            if let Some(digest) = self.digests.get(id.as_str()) {
                entries.push(DirectoryEntry::AccountSignedUp {
                    id: (*id).clone(),
                    name: account.name.clone(),
                    digest: hex_encode(digest),
                });
            } else {
                entries.push(DirectoryEntry::AccountKeyed {
                    id: (*id).clone(),
                    name: account.name.clone(),
                });
            }
            // Ceilings read the verified tier (directly verified or via a
            // confirmed email), so export the verified flag on that basis:
            // the replica reaches the same ceiling either way.
            if self.is_verified(id) {
                entries.push(DirectoryEntry::AccountVerified { id: (*id).clone() });
            }
        }
        // Non-default tiers ride along: ceilings intersect them.
        let defaults = BTreeMap::from([
            (Tier::Anonymous, WorldScopes::GUEST),
            (Tier::Registered, WorldScopes::GUEST),
            (
                Tier::Verified,
                WorldScopes::GUEST.union(WorldScopes::DELEGATE),
            ),
        ]);
        for (tier, scopes) in &self.tiers {
            if defaults.get(tier) != Some(scopes) {
                entries.push(DirectoryEntry::TierSet {
                    tier: *tier,
                    scopes: scopes.bits(),
                });
            }
        }
        for id in &ordered {
            let group = &self.groups[id];
            entries.push(DirectoryEntry::GroupCreated {
                id: *id,
                name: group.name.clone(),
                description: group.description.clone(),
                scopes: group.scopes.bits(),
                founder: founders[id].clone(),
            });
        }
        // All groups exist before any nested `MemberSet` references them.
        // Directory replay refuses a membership naming a not-yet-created
        // group, even when both IDs belong to one contiguous replica prefix.
        // Founder downgrades are collected here and emitted only after
        // every group's memberships exist (see the deferral below).
        let mut founder_downgrades: Vec<DirectoryEntry> = Vec::new();
        for id in &ordered {
            // Subject memberships reproduce subject ceilings; nested links
            // reproduce the paths levels flow through. Other accounts stay
            // out: the founder above already keeps an admin in place.
            let mut memberships: Vec<(Member, Level)> = self
                .members
                .range(
                    (*id, Member::Account { id: String::new() })
                        ..=(*id, Member::Group { id: u64::MAX }),
                )
                .map(|((_, member), level)| (member.clone(), *level))
                .filter(|(member, _)| match member {
                    Member::Account { id } => subjects.contains(id),
                    Member::Group { .. } => true,
                })
                .collect();
            memberships.sort_by(|a, b| a.0.cmp(&b.0));
            for (member, level) in memberships {
                // The founder's admin comes from `GroupCreated` itself.
                if member
                    == (Member::Account {
                        id: founders[id].clone(),
                    })
                    && level == Level::Admin
                {
                    continue;
                }
                let is_founder = member
                    == (Member::Account {
                        id: founders[id].clone(),
                    });
                let entry = DirectoryEntry::MemberSet {
                    group: *id,
                    member,
                    level: Some(level),
                };
                // A founder whose direct level is below `Admin` was chosen
                // because they hold the group's admin through an inherited
                // path. Sorted order emits account memberships before
                // nested-group ones, so the downgrade could land before
                // the nested membership carrying that path exists, leaving
                // the group momentarily admin-less — `checked_mode`
                // rejects it through `ensure_admins` and replica startup
                // fails. Defer founder downgrades until all memberships
                // are in place.
                if is_founder {
                    founder_downgrades.push(entry);
                } else {
                    entries.push(entry);
                }
            }
        }
        entries.extend(founder_downgrades);
        // Current friendships among the exported accounts. Preserve the
        // request ID when one exists: a signed FriendRemove artifact uses
        // it to distinguish this friendship from an older relationship
        // between the same pair after the directory is projected.
        let sorted: Vec<&String> = accounts.iter().collect();
        let mut pairs: Vec<(&String, &String)> = Vec::new();
        for (index, a) in sorted.iter().enumerate() {
            for b in &sorted[index + 1..] {
                if self.are_friends(a, b) {
                    pairs.push((a, b));
                }
            }
        }
        for (a, b) in pairs {
            let request = self.friend_requests.get(&pair(a, b)).cloned();
            entries.push(DirectoryEntry::FriendRequested {
                from: (*a).clone(),
                to: (*b).clone(),
                id: request.clone(),
            });
            entries.push(DirectoryEntry::FriendAccepted {
                from: (*a).clone(),
                to: (*b).clone(),
                request,
                id: None,
            });
        }
        Ok(entries)
    }
}
