//! Delegation (P2.5, design §7): grants between accounts inside a group, the chain search that
//! decides whether an account may act at a causal position, the three removal modes, and vetoes.
//!
//! A grant is an event by a grantor who holds the scope there. An account may exercise a scope
//! through a chain of grants that starts at a root. The roots are the owners and managers at the
//! action's position (design §7.2): a member holding Manage there is a root, so a manager's own
//! grants and actions are valid without a grant reaching it. A grant reaches the others.
//!
//! Each link attenuates: a grantor passes on no more than it holds. A link carries authority at a
//! position when its grantor was valid where it was made, the grant is not voided by a concurrent
//! removal of its grantor (strong remove only; weak remove stands it, design §6.3), no veto in the
//! position's past cuts it, and the group's mode lets it stand there (design §7.3).
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
///
/// A grant with a `target` is a crossing (P2.7, see [`super::worlds`]). It is judged and kept in
/// force like any other grant, but it is never a link in its home world's chains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// The granting event's ID.
    pub id: Hash,
    /// The group the grant is made in.
    pub group: Hash,
    /// The grantor.
    pub from: Key,
    /// The grantee.
    pub to: Key,
    /// The scope granted.
    pub scope: Role,
    /// Whether the grantee may grant the scope on.
    pub redelegate: bool,
    /// The world the grant names, if it is a crossing.
    pub target: Option<String>,
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
pub(super) const fn removed_member(event: &Event) -> Option<Key> {
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
    crossing: bool,
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
            auth_author: None,
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
            auth_author: None,
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
        // Authorize the signer, not the resolved account: for a device signer this is
        // what enforces the device's scope cap (a Read-scoped device of a manager
        // account holds no Manage here, however its account stands).
        let manages = self.authorized(
            by_id,
            decision.group,
            event.author,
            Role::Manage,
            &event.after,
            false,
        )?;
        if !manages {
            // A Manage grant reaches the author but its chain does not stand at this
            // position (cut, vetoed, or its grantor lost the scope): that is a chain
            // failure, not a missing manager role.
            let reached_by_grant = self.grants.values().any(|grant| {
                grant.group == decision.group
                    && (grant.to == author || grant.to == event.author)
                    && grant.scope >= Role::Manage
            });
            return Ok(Err(if reached_by_grant {
                Refusal::NoChain
            } else {
                Refusal::NotManager
            }));
        }
        // The membership layer validates the operation against its author's own
        // access, so a delegated operation is placed under the chain root whose
        // authority it exercises (a device is likewise placed under its account).
        let mut decision = decision;
        if let Some(root) = self.chain_root(by_id, decision.group, event.author, &event.after)? {
            decision.auth_author = Some(root);
        }
        Ok(Ok(decision))
    }

    /// The root whose Manage authority `actor` exercises at `after`: the actor itself
    /// when it is a manager there (a device resolves to its account first, under the
    /// same scope cap as [`Engine::authorized`]), otherwise the root at the head of
    /// the chain of Manage links that reaches it. Deterministic in the event set:
    /// roots and links are traversed in their stored (sorted) order.
    fn chain_root(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        group: Hash,
        actor: Key,
        after: &[Hash],
    ) -> Result<Option<Key>, FoldError> {
        if !self.groups.contains_key(&group) {
            return Ok(None);
        }
        let actor = match self.authorized_device_account(by_id, group, actor, after) {
            Some((account, device_scope)) => {
                if device_scope < Role::Manage {
                    return Ok(None);
                }
                account
            }
            None => actor,
        };
        let roots = self.membership.managers_at_position(after, group)?;
        if roots.contains(&actor) {
            return Ok(Some(actor));
        }
        let links = self.links(by_id, group, after)?;
        let mut origin: BTreeMap<Key, Key> = roots.iter().map(|root| (*root, *root)).collect();
        let mut stack: Vec<Key> = roots.iter().copied().collect();
        while let Some(holder) = stack.pop() {
            let root = origin[&holder];
            for link in links
                .iter()
                .filter(|link| !link.crossing && link.from == holder && link.scope >= Role::Manage)
            {
                if link.to == actor {
                    return Ok(Some(root));
                }
                if link.redelegate && !origin.contains_key(&link.to) {
                    origin.insert(link.to, root);
                    stack.push(link.to);
                }
            }
        }
        Ok(None)
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
        while let Some(holder) = stack.pop() {
            for link in links
                .iter()
                .filter(|link| !link.crossing && link.from == holder && link.scope >= scope)
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
            // The removals of the grantor that come after the grant. A grant made after a re-add is
            // not one of them: its grantor was valid where it was made.
            let superseding: Vec<&Removal> = removals
                .iter()
                .filter(|removal| {
                    removal.member == grant.from
                        && causal_past(by_id, &[removal.id]).contains(&grant.id)
                })
                .collect();
            if mode == Delegation::Transfer && !superseding.is_empty() {
                // The removed grantor's own link is superseded. Each successor the removals name
                // reissues it from the removal on, if it held the scope then and holds it here.
                for removal in superseding {
                    let Some(successor) = removal.successor else {
                        continue;
                    };
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
                            crossing: grant.target.is_some(),
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
                crossing: grant.target.is_some(),
            });
        }
        Ok(links)
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
