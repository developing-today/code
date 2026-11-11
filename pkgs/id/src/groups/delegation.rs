//! Delegation (P2.5, design §7): grants between accounts inside a group, the chain search that
//! decides whether an account may act at a causal position, explicit per-removal grant disposition,
//! and vetoes.
//!
//! A grant is an event by a grantor who holds the scope there. An account may exercise a scope
//! through a chain of grants that starts at a root. The roots are the owners and managers at the
//! action's position (design §7.2): a member holding Manage there is a root, so a manager's own
//! grants and actions are valid without a grant reaching it. A grant reaches the others.
//!
//! Each link attenuates: a grantor passes on no more than it holds. A link carries authority at a
//! position when its grantor was valid where it was made, the grant is not voided by a concurrent
//! removal of its grantor (strong remove only; weak remove stands it, design §6.3), no veto in the
//! position's past cuts it, and the removal's mode lets it stand there (design §7.3).
//!
//! Each removal's `RemoveMode` decides its grantor's earlier grants. `Revoke` ends them;
//! `Transfer` reissues them from a permitted successor; `KeepPrior` retains the original grantor;
//! and `ReassignToGroup` makes the group the root. Strong/weak removal still decides what happens
//! to grants concurrent with the removal.
//! One limit on `KeepPrior` (issue #169, ruling 2026-11-03): a re-add starts clean. A grant an
//! account made or received before its removal does not revive when the account is re-added -
//! at positions whose past contains the re-add, such a grant is no link, and authority flows
//! only from grants issued after the re-add. Positions before the re-add are judged as before:
//! there the grant still stands, as a fact about the past.

use std::collections::{BTreeMap, BTreeSet};

use p2panda_core::Hash;

use super::fold::{Decision, Effect, Engine, FoldError, Judged, Refusal, access_role, causal_past};
use super::policy::{Delegation, RemoveMode, RemoveSuccessor};
use super::{Body, Delegate, Event, Key, MemberOp, Role};

/// A grant that took effect: `from` gave `to` the scope `scope` in `group`, as event `id`.
///
/// A grant with a `target` is a crossing (P2.7, see [`super::worlds`]). It is judged and kept in
/// force like any other grant, but it is never a link in its home world's chains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// The granting event's ID.
    pub id: Hash,
    /// The group the grant is made in.
    pub group: Hash,
    /// The grantor, or `None` when a removal re-anchored the grant at the group.
    pub from: Option<Key>,
    /// The grantee.
    pub to: Key,
    /// The scope granted.
    pub scope: Role,
    /// Whether the grantee may grant the scope on.
    pub redelegate: bool,
    /// Whether the grant remains under its original grantor after that grantor was removed.
    pub retained: bool,
    /// The world the grant names, if it is a crossing.
    pub target: Option<String>,
}

/// A removal that took effect and the disposition selected for the removed member's grants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Removal {
    pub(super) id: Hash,
    pub(super) group: Hash,
    pub(super) member: Key,
    pub(super) mode: RemoveMode,
    pub(super) actor: Key,
    /// The removal's `after`. A successor is judged there, not at the removal itself: the removal's
    /// own position already contains the reissue, so judging it there would recurse.
    pub(super) after: Vec<Hash>,
}

/// The account a removal event removes, if the event is one.
pub(super) const fn removed_member(event: &Event) -> Option<Key> {
    match &event.body {
        Body::Member(MemberOp::Remove { member, .. }) => Some(*member),
        _ => None,
    }
}

/// The account a member-add event adds, if the event is one. A re-add is an add whose causal
/// past contains a removal of the same account.
pub(super) const fn added_member(event: &Event) -> Option<Key> {
    match &event.body {
        Body::Member(MemberOp::Add { member, .. }) => Some(*member),
        _ => None,
    }
}

/// One link of a chain at a position: `from` may give `scope` to `to`. `None` is the group root. A
/// reissued link is a grant from a successor. It stands for the removed member's grant `origin`.
#[derive(Clone, Debug)]
struct Link {
    origin: Hash,
    from: Option<Key>,
    to: Key,
    scope: Role,
    redelegate: bool,
    reissued: bool,
    retained: bool,
    crossing: bool,
}

/// How a grant that remains in force is exposed to callers.
#[derive(Clone, Copy, Debug)]
pub(super) struct GrantVisibility {
    pub(super) from: Option<Key>,
    pub(super) retained: bool,
}

impl Engine {
    /// Judges a grant. The author must be an active member who holds the scope at the event's
    /// position, and must reach it through a chain that can be passed on. A crossing (`target`
    /// set) may carry Write at most and must name a world.
    pub(super) fn delegate(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        event: &Event,
        delegate: Delegate,
        target: Option<&str>,
    ) -> Result<Judged, FoldError> {
        let group = match self.acting_in(event, by_id) {
            Ok(group) => group,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let (author, author_role) = match self.effective_author(by_id, group, event) {
            Some(res) => res,
            None => return Ok(Err(Refusal::NotActive)),
        };
        if delegate.scope == Role::Admin {
            return Ok(Err(Refusal::BadScope));
        }
        if target.is_some_and(|world| world.is_empty() || delegate.scope > Role::Write) {
            return Ok(Err(Refusal::NotCrossable));
        }
        if delegate.grantee == author || delegate.grantee == event.author {
            return Ok(Err(Refusal::BadGrantee));
        }
        if author_role < delegate.scope {
            return Ok(Err(Refusal::Attenuated));
        }
        if !self.holds(group, author, delegate.scope, &event.after)? {
            return Ok(Err(Refusal::Attenuated));
        }
        if !self.authorized(by_id, group, author, delegate.scope, &event.after, true)? {
            return Ok(Err(Refusal::NoChain));
        }
        Ok(Ok(Decision {
            group,
            action: None,
            effect: Effect::Delegation {
                delegate,
                target: target.map(str::to_owned),
            },
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
        let group = match self.acting_in(event, by_id) {
            Ok(group) => group,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let (author, author_role) = match self.effective_author(by_id, group, event) {
            Some(res) => res,
            None => return Ok(Err(Refusal::NotActive)),
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
        if author_role < Role::Manage {
            return Ok(Err(Refusal::NotManager));
        }
        if !self.holds(group, author, Role::Manage, &event.after)? {
            return Ok(Err(Refusal::NotManager));
        }
        if !self.authorized(by_id, group, author, Role::Manage, &event.after, false)? {
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
        let author = match self.effective_author(by_id, decision.group, event) {
            Some((a, _)) => a,
            None => event.author,
        };
        if let MemberOp::Remove { member, .. } = op
            && member == author
        {
            return Ok(Ok(decision));
        }
        let manages = self.authorized(
            by_id,
            decision.group,
            author,
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
    ) -> Result<BTreeMap<Hash, GrantVisibility>, FoldError> {
        let head: Vec<Hash> = by_id.keys().copied().collect();
        let groups: Vec<Hash> = self.groups.keys().copied().collect();
        let mut in_force = BTreeMap::new();
        for group in groups {
            for link in self.links(by_id, group, &head)? {
                if !link.reissued {
                    in_force.insert(
                        link.origin,
                        GrantVisibility {
                            from: link.from,
                            retained: link.retained,
                        },
                    );
                }
            }
        }
        Ok(in_force)
    }

    /// Whether `actor` may exercise `scope` in `group` at the causal position `after` (design
    /// §7.2). The roots are the owners and managers at that position: a root holds Manage there,
    /// so it holds every scope. An actor that is a root is authorized. Otherwise a chain of links
    /// leads from a root to the actor, every link attenuating. With `need_redelegate`, the last
    /// link into the actor must allow the actor to grant on. The actor's own membership is not
    /// checked here: a grant may reach an account that is not a member. Crossings (grants that
    /// name another world) are not links here, so they never extend a chain in this group.
    pub(super) fn authorized(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        group: Hash,
        actor: Key,
        scope: Role,
        after: &[Hash],
        need_redelegate: bool,
    ) -> Result<bool, FoldError> {
        if !self.groups.contains_key(&group) {
            return Ok(false);
        }
        let actor = match self.authorized_device_account(by_id, group, actor, after) {
            Some((account, device_scope)) => {
                if device_scope < scope {
                    return Ok(false);
                }
                account
            }
            None => actor,
        };
        let roots = self.membership.managers_at_position(after, group)?;
        if roots.contains(&actor) {
            return Ok(true);
        }
        let links = self.links(by_id, group, after)?;
        let mut reached: BTreeSet<Key> = roots.iter().copied().collect();
        let mut stack: Vec<Key> = reached.iter().copied().collect();
        // A re-anchored grant is a group root, not a synthetic member or account. It reaches its
        // grantee, and only a redelegatable grant extends the chain.
        for link in links
            .iter()
            .filter(|link| link.from.is_none() && !link.crossing)
        {
            if link.scope >= scope && link.to == actor && (!need_redelegate || link.redelegate) {
                return Ok(true);
            }
            if link.scope >= scope && link.redelegate && reached.insert(link.to) {
                stack.push(link.to);
            }
        }
        while let Some(holder) = stack.pop() {
            for link in links
                .iter()
                .filter(|link| !link.crossing && link.from == Some(holder) && link.scope >= scope)
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
    pub(super) fn holds(
        &mut self,
        group: Hash,
        who: Key,
        scope: Role,
        after: &[Hash],
    ) -> Result<bool, FoldError> {
        let access = self.membership.access_at_position(after, group, who)?;
        Ok(access.is_some_and(|access| access_role(&access) >= scope))
    }

    /// The links that carry authority at `after`, applying each settled removal's explicit mode.
    fn links(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        group: Hash,
        after: &[Hash],
    ) -> Result<Vec<Link>, FoldError> {
        if !self.groups.contains_key(&group) {
            return Ok(Vec::new());
        }
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
            // The removals of the grantor that come after the grant. A grant made after a re-add is
            // not one of them: its grantor was valid where it was made.
            let superseding: Vec<&Removal> = removals
                .iter()
                .filter(|removal| {
                    Some(removal.member) == grant.from
                        && causal_past(by_id, &[removal.id]).contains(&grant.id)
                })
                .collect();
            if superseding.is_empty() {
                // Issue #169: an endpoint removed under KeepPrior and later re-added starts
                // clean, so this grant is no link at a position that has seen the re-add.
                // Only the grantee endpoint can arise here: a grantor removal that contains
                // the grant in its past would supersede the grant, not leave this branch.
                if self.cut_by_readd(by_id, &grant, &removals, &past) {
                    continue;
                }
                // A non-removal role change (a demotion) carries no RemoveMode: under
                // Cascade delegation the link stands only while the grantor still holds
                // the scope at `after`, so demoting the grantor below the grant's scope
                // withdraws the delegated authority (the pre-RemoveMode Cascade check,
                // including for cross-world crossings). Under Transfer delegation the
                // link stands until a removal disposes of it. A group-anchored grant
                // (from: None) has no grantor to demote.
                if self.groups[&group].policy.delegation_mode() == Delegation::Cascade
                    && let Some(grantor) = grant.from
                    && !self.holds(group, grantor, grant.scope, after)?
                {
                    continue;
                }
                links.push(Link {
                    origin: grant.id,
                    from: grant.from,
                    to: grant.to,
                    scope: grant.scope,
                    redelegate: grant.redelegate,
                    reissued: false,
                    retained: false,
                    crossing: grant.target.is_some(),
                });
                continue;
            }

            // Revocation wins if several removals of a re-added grantor apply to the same old
            // grant. Otherwise transfers replace it, group reassignment exposes it as a group
            // root, and keep-prior preserves the original grantor.
            if superseding
                .iter()
                .any(|removal| removal.mode == RemoveMode::Revoke)
            {
                continue;
            }
            let transfers: Vec<(&Removal, RemoveSuccessor)> = superseding
                .iter()
                .filter_map(|removal| match removal.mode {
                    RemoveMode::Transfer(opts) => Some((*removal, opts.successor)),
                    _ => None,
                })
                .collect();
            if !transfers.is_empty() {
                for (removal, successor) in transfers {
                    let successors: Vec<Key> = match successor {
                        RemoveSuccessor::Member(key) | RemoveSuccessor::Designated(key) => {
                            vec![key]
                        }
                        RemoveSuccessor::Admin => self
                            .admins_at_position(by_id, &group, &removal.after)
                            .into_iter()
                            .collect(),
                        RemoveSuccessor::Actor => {
                            // `removal.actor` is the removing event's author key, which
                            // may be an authorized device key rather than the account
                            // itself: membership and `holds` are account-level, so
                            // resolve the device to its account at the removal's
                            // position, or the reissue check fails and the grants are
                            // silently revoked instead of transferred.
                            let actor = self
                                .authorized_device_account(
                                    by_id,
                                    group,
                                    removal.actor,
                                    &removal.after,
                                )
                                .map(|(account, _)| account)
                                .unwrap_or(removal.actor);
                            vec![actor]
                        }
                    };
                    for successor in successors {
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
                                from: Some(successor),
                                to: grant.to,
                                scope: grant.scope,
                                redelegate: grant.redelegate,
                                reissued: true,
                                retained: false,
                                crossing: grant.target.is_some(),
                            });
                        }
                    }
                }
                continue;
            }
            if superseding
                .iter()
                .any(|removal| removal.mode == RemoveMode::ReassignToGroup)
            {
                links.push(Link {
                    origin: grant.id,
                    from: None,
                    to: grant.to,
                    scope: grant.scope,
                    redelegate: grant.redelegate,
                    reissued: false,
                    retained: false,
                    crossing: grant.target.is_some(),
                });
                continue;
            }
            // Issue #169: the retained grant does not revive when an endpoint is re-added.
            if self.cut_by_readd(by_id, &grant, &removals, &past) {
                continue;
            }
            links.push(Link {
                origin: grant.id,
                from: grant.from,
                to: grant.to,
                scope: grant.scope,
                redelegate: grant.redelegate,
                reissued: false,
                retained: true,
                crossing: grant.target.is_some(),
            });
        }
        Ok(links)
    }

    /// Whether `KeepPrior` must cut this grant at this position because an endpoint was
    /// re-added (issue #169): for the grantor or the grantee, a settled `KeepPrior`
    /// removal in `past` has the grant in its causal past, and a settled re-add of the
    /// same account in `past` has that removal in its causal past. The re-add starts the
    /// account clean, so the pre-removal grant is no link at any position that has seen
    /// the re-add. Grants issued after the re-add are untouched: no removal that follows
    /// them contains them in its past together with a later re-add, until the account is
    /// removed and re-added again.
    fn cut_by_readd(
        &self,
        by_id: &BTreeMap<Hash, &Event>,
        grant: &Grant,
        removals: &[Removal],
        past: &BTreeSet<Hash>,
    ) -> bool {
        [grant.from, Some(grant.to)].iter().any(|member| {
            removals
                .iter()
                .filter(|removal| {
                    Some(removal.member) == *member
                        && removal.mode == RemoveMode::KeepPrior
                        && causal_past(by_id, &[removal.id]).contains(&grant.id)
                })
                .any(|removal| {
                    past.iter().any(|id| {
                        self.settled.contains(id)
                            && by_id.get(id).is_some_and(|event| {
                                event.group == Some(grant.group)
                                    && added_member(event) == *member
                                    && causal_past(by_id, &[*id]).contains(&removal.id)
                            })
                    })
                })
        })
    }

    /// Whether a settled removal of the grantor is concurrent with the grant: neither is in the
    /// other's causal past, and the group voids such a grant. Strong remove voids it (design §6.2
    /// and §7.3). Weak remove stands it, and only the authority check decides later events
    /// (design §6.3). The removals are read from the events, not from the round's state: a
    /// concurrent removal may be judged after the grant in canonical order.
    fn void(&self, by_id: &BTreeMap<Hash, &Event>, grant: &Grant) -> bool {
        if !self.strong_removal(grant.group) {
            return false;
        }
        let past_grant = causal_past(by_id, &[grant.id]);
        self.settled
            .iter()
            .filter_map(|id| by_id.get(id).copied())
            .filter(|event| event.group == Some(grant.group) && removed_member(event) == grant.from)
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
