//! Delegation (P2.5, design §7): grants between accounts inside a group, the chain search that
//! decides whether an account may act at a causal position, the three removal modes, and vetoes.
//!
//! A grant is an event by a grantor who holds the scope there. An account may exercise a scope
//! only through a chain of grants that starts at a root. The roots are the accounts a group's
//! create named, its creator included. A manager added by someone else has membership, but no
//! authority until a grant reaches it. This is S2's choice: the owner is the only root.
//!
//! Each link attenuates: a grantor passes on no more than it holds. A link carries authority at a
//! position when its grantor was valid where it was made, the grant is not voided by a concurrent
//! removal of its grantor, no veto in the position's past cuts it, and the group's mode lets it
//! stand there (design §7.3).
//!
//! The mode decides a removed member's grants. Under `keep_prior` they stand. Under `cascade` they
//! stand only while the grantor still holds the scope. Under `transfer` the removal's successor
//! reissues them, from the removal on, if it holds the scope then and now.

use std::collections::{BTreeMap, BTreeSet};

use p2panda_core::Hash;

use super::fold::{Decision, Effect, Engine, FoldError, Judged, Refusal, access_role, causal_past};
use super::policy::Delegation;
use super::{Body, Delegate, Event, Key, MemberOp, Role};

/// A grant that took effect: `from` gave `to` the scope `scope` in `group`, as event `id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Grant {
    pub(super) id: Hash,
    pub(super) group: Hash,
    pub(super) from: Key,
    pub(super) to: Key,
    pub(super) scope: Role,
    pub(super) redelegate: bool,
}

/// A removal that took effect. Its successor takes over the removed member's grants under
/// `transfer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Removal {
    pub(super) id: Hash,
    pub(super) group: Hash,
    pub(super) member: Key,
    pub(super) successor: Option<Key>,
    /// The removal's `after`. A successor is judged there, not at the removal itself: the removal's
    /// own position already contains the reissue, so judging it there would recurse.
    pub(super) after: Vec<Hash>,
}

/// The account a removal event removes, if the event is one.
const fn removed_member(event: &Event) -> Option<Key> {
    match &event.body {
        Body::Member(MemberOp::Remove { member, .. }) => Some(*member),
        _ => None,
    }
}

/// One link of a chain at a position: `from` may give `scope` to `to`. A reissued link is a grant
/// from a successor. It stands for the removed member's grant `origin`.
#[derive(Clone, Debug)]
struct Link {
    origin: Hash,
    from: Key,
    to: Key,
    scope: Role,
    redelegate: bool,
    reissued: bool,
}

impl Engine {
    /// Judges a grant. The author must be an active member who holds the scope at the event's
    /// position, and must reach it through a chain that can be passed on.
    pub(super) fn delegate(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        event: &Event,
        delegate: Delegate,
    ) -> Result<Judged, FoldError> {
        let group = match self.acting_in(event) {
            Ok(group) => group,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if delegate.scope == Role::Admin {
            return Ok(Err(Refusal::BadScope));
        }
        if delegate.grantee == event.author {
            return Ok(Err(Refusal::BadGrantee));
        }
        if !self.holds(group, event.author, delegate.scope, &event.after)? {
            return Ok(Err(Refusal::Attenuated));
        }
        if !self.authorized(
            by_id,
            group,
            event.author,
            delegate.scope,
            &event.after,
            true,
        )? {
            return Ok(Err(Refusal::NoChain));
        }
        Ok(Ok(Decision {
            group,
            action: None,
            effect: Effect::Delegation(delegate),
        }))
    }

    /// Judges a veto. The author must hold Manage and a chain, and the grant must be one the veto
    /// has seen.
    pub(super) fn veto(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        event: &Event,
        grant: Hash,
    ) -> Result<Judged, FoldError> {
        let group = match self.acting_in(event) {
            Ok(group) => group,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let seen = causal_past(by_id, &event.after);
        let known = self
            .grants
            .get(&grant)
            .is_some_and(|target| target.group == group)
            && seen.contains(&grant);
        if !known {
            return Ok(Err(Refusal::UnknownGrant));
        }
        if !self.holds(group, event.author, Role::Manage, &event.after)? {
            return Ok(Err(Refusal::NotManager));
        }
        if !self.authorized(
            by_id,
            group,
            event.author,
            Role::Manage,
            &event.after,
            false,
        )? {
            return Ok(Err(Refusal::NoChain));
        }
        Ok(Ok(Decision {
            group,
            action: None,
            effect: Effect::Veto(grant),
        }))
    }

    /// Judges a manager's membership change, which the membership layer would otherwise accept. A
    /// manager needs a chain from an owner. Removing oneself is leaving, and needs none.
    pub(super) fn manage_chain(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        event: &Event,
        op: MemberOp,
        decision: Decision,
    ) -> Result<Judged, FoldError> {
        if let MemberOp::Remove { member, .. } = op
            && member == event.author
        {
            return Ok(Ok(decision));
        }
        let manages = self.authorized(
            by_id,
            decision.group,
            event.author,
            Role::Manage,
            &event.after,
            false,
        )?;
        if manages {
            Ok(Ok(decision))
        } else {
            Ok(Err(Refusal::NoChain))
        }
    }

    /// The grants in force at the head of the events: every grant that its own link carries when
    /// every event is in the past. Each is reported once, under its own event ID.
    pub(super) fn in_force(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<BTreeSet<Hash>, FoldError> {
        let head: Vec<Hash> = by_id.keys().copied().collect();
        let groups: Vec<Hash> = self.groups.keys().copied().collect();
        let mut in_force = BTreeSet::new();
        for group in groups {
            for link in self.links(by_id, group, &head)? {
                if !link.reissued {
                    in_force.insert(link.origin);
                }
            }
        }
        Ok(in_force)
    }

    /// Whether `actor` may exercise `scope` in `group` at the causal position `after` (design
    /// §7.2). A root that holds the scope there is authorized. Otherwise a chain of links leads
    /// from a root holding the scope to the actor, every link attenuating. With `need_redelegate`,
    /// the last link into the actor must allow the actor to grant on. The actor's own membership
    /// is not checked here: a grant may reach an account that is not a member.
    pub(super) fn authorized(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        group: Hash,
        actor: Key,
        scope: Role,
        after: &[Hash],
        need_redelegate: bool,
    ) -> Result<bool, FoldError> {
        let Some(record) = self.groups.get(&group) else {
            return Ok(false);
        };
        let roots = record.roots.clone();
        if roots.contains(&actor) && self.holds(group, actor, scope, after)? {
            return Ok(true);
        }
        let links = self.links(by_id, group, after)?;
        let mut reached = BTreeSet::new();
        for root in roots {
            if self.holds(group, root, scope, after)? {
                reached.insert(root);
            }
        }
        let mut stack: Vec<Key> = reached.iter().copied().collect();
        while let Some(holder) = stack.pop() {
            for link in links
                .iter()
                .filter(|link| link.from == holder && link.scope >= scope)
            {
                if link.to == actor && (!need_redelegate || link.redelegate) {
                    return Ok(true);
                }
                if link.redelegate && reached.insert(link.to) {
                    stack.push(link.to);
                }
            }
        }
        Ok(false)
    }

    /// Whether `who` holds `scope` in `group` at the causal position `after`, by membership.
    fn holds(
        &mut self,
        group: Hash,
        who: Key,
        scope: Role,
        after: &[Hash],
    ) -> Result<bool, FoldError> {
        let access = self.membership.access_at_position(after, group, who)?;
        Ok(access.is_some_and(|access| access_role(&access) >= scope))
    }

    /// The links that carry authority at `after`, in the group's mode (design §7.3).
    fn links(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        group: Hash,
        after: &[Hash],
    ) -> Result<Vec<Link>, FoldError> {
        let Some(mode) = self
            .groups
            .get(&group)
            .map(|record| record.policy.delegation_mode())
        else {
            return Ok(Vec::new());
        };
        let past = causal_past(by_id, after);
        let grants: Vec<Grant> = self
            .grants
            .values()
            .filter(|grant| {
                grant.group == group && past.contains(&grant.id) && self.settled.contains(&grant.id)
            })
            .cloned()
            .collect();
        let removals: Vec<Removal> = self
            .removals
            .values()
            .filter(|removal| {
                removal.group == group
                    && past.contains(&removal.id)
                    && self.settled.contains(&removal.id)
            })
            .cloned()
            .collect();
        let mut links = Vec::new();
        for grant in grants {
            if self.void(by_id, &grant) || self.vetoed(grant.id, &past) {
                continue;
            }
            let removed: Vec<&Removal> = removals
                .iter()
                .filter(|removal| removal.member == grant.from)
                .collect();
            if mode == Delegation::Transfer && !removed.is_empty() {
                // The removed grantor's own link is superseded. Each successor the removals name
                // reissues it from the removal on, if it held the scope then and holds it here.
                for removal in removed {
                    let Some(successor) = removal.successor else {
                        continue;
                    };
                    if !causal_past(by_id, &[removal.id]).contains(&grant.id) {
                        continue;
                    }
                    let holds_now = self.holds(group, successor, grant.scope, after)?;
                    if holds_now
                        && self.authorized(
                            by_id,
                            group,
                            successor,
                            grant.scope,
                            &removal.after,
                            true,
                        )?
                    {
                        links.push(Link {
                            origin: grant.id,
                            from: successor,
                            to: grant.to,
                            scope: grant.scope,
                            redelegate: grant.redelegate,
                            reissued: true,
                        });
                    }
                }
                continue;
            }
            if mode == Delegation::Cascade && !self.holds(group, grant.from, grant.scope, after)? {
                continue;
            }
            links.push(Link {
                origin: grant.id,
                from: grant.from,
                to: grant.to,
                scope: grant.scope,
                redelegate: grant.redelegate,
                reissued: false,
            });
        }
        Ok(links)
    }

    /// Whether a settled removal of the grantor is concurrent with the grant: neither is in the
    /// other's causal past. Strong remove voids such a grant (design §6.2 and §7.3). The removals
    /// are read from the events, not from the round's state: a concurrent removal may be judged
    /// after the grant in canonical order.
    fn void(&self, by_id: &BTreeMap<Hash, &Event>, grant: &Grant) -> bool {
        let past_grant = causal_past(by_id, &[grant.id]);
        self.settled
            .iter()
            .filter_map(|id| by_id.get(id).copied())
            .filter(|event| {
                event.group == Some(grant.group) && removed_member(event) == Some(grant.from)
            })
            .any(|removal| {
                !past_grant.contains(&removal.id)
                    && !causal_past(by_id, &[removal.id]).contains(&grant.id)
            })
    }

    /// Whether a veto of the grant is in `past`.
    fn vetoed(&self, grant: Hash, past: &BTreeSet<Hash>) -> bool {
        self.vetoes.iter().any(|(veto, target)| {
            *target == grant && past.contains(veto) && self.settled.contains(veto)
        })
    }
}
