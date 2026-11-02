//! The applied state of a store, derived from the events it holds and its fork proofs (P2.3).
//!
//! The store keeps every event that is present: stored, applied or voided, and fork proof headers.
//! Which of them apply is a function of that set, so it does not depend on arrival order. The
//! rules are:
//!
//! 1. **Fork.** An event at or above its author's fork point never applies. This is the fork
//!    rule of Phase 1, and it reaches no other author's events.
//! 2. **Group operation.** A membership operation applies when the membership layer says it takes
//!    effect: accepted at its position, not filtered by the resolver, and its author holds manage
//!    access at its dependencies. The strong-remove rules decide concurrency (see
//!    [`Membership::is_effective`]), so a dependent of a voided operation is voided only when its
//!    author lost authority at its position.
//! 3. **Content.** Any other event applies, unless it names a voided event and its author is not a
//!    member of its group at its position. A group with no create has no authority to lose, so its
//!    content always applies. Content is kept on removal, as the content policy says.
//! 4. **Group policy (Phase 2, #168).** The membership layer alone does not know the group rules
//!    that live in [`crate::groups::fold`]: only an Admin may change a role or remove an Admin, a
//!    create may not list its author or repeat a member, and a manager acts only through a chain
//!    from an owner. So every group operation that can be bridged into the fold's event model is
//!    folded with the rest, and an operation the fold refuses with a voiding reason does not
//!    apply, however the membership layer judged it. Refusals that only wait (a group that does
//!    not exist yet, an author not yet active) or freeze do not change the membership verdict:
//!    the store's set may simply not hold the events the fold would need. The fold's projection
//!    also gives the roster of a group it decided, so a refused operation's target never shows
//!    as a member. Operations the bridge cannot represent (a create or an operation naming a
//!    group as a member, friendship and policy bodies the body codec does not carry) keep the
//!    membership verdict alone; the fold judges them when they travel as group events. Two
//!    reconciliations keep the layers in agreement: an operation the fold refuses with a
//!    voiding reason is fed to the membership layer as a no-op, so its authority changes never
//!    reach the descendants the layer judges; and a create that lists its author below Manage
//!    does not bridge at all — the fold counts the creator as an Admin by construction while
//!    the membership layer honours the listed access, so the two models name the creator's
//!    role differently and the group keeps the membership verdict alone.
//!
//! A voided event stays stored. Its author's later events do not wait on it, and they are judged
//! by the same rules. That is why an honest author never stalls on another author's removal.

use std::collections::{HashMap, HashSet};

use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Hash, SeqNum};
use redb::ReadableTable;

use super::{StoreError, corrupt, decode_proof, decode_stored, split_position};
use crate::chain::Event;
use crate::groups::{
    Body as GroupBody, Create as GroupCreate, Event as GroupEvent, MemberOp, Policy, Projection,
    Role as GroupRole, Template,
};
use crate::membership::{AuthId, CausalEvent, Membership, Role};
use crate::rejection::Reason;
use crate::waiting::AuthorKey;

/// One stored event, with the membership action it carries when it is a group operation.
#[derive(Clone, Debug)]
pub(super) struct Stored {
    pub event: Event,
    pub action: Option<GroupAction<AuthId, ()>>,
}

/// The applied state of a set of events.
pub(super) struct Resolved {
    /// The IDs of the events that apply.
    pub applied: HashSet<Hash>,
    /// The membership layer over the events, or `None` when there is no group operation.
    pub membership: Option<Membership>,
    /// The group fold over the bridged group operations, or `None` when no operation bridged.
    pub projection: Option<Projection>,
}

/// One event, or one fork proof header, as the resolution sees it.
struct Node {
    id: Hash,
    author: AuthorKey,
    group: Option<Hash>,
    after: Vec<Hash>,
    action: Option<GroupAction<AuthId, ()>>,
    /// True for a fork proof header, and for an event at or above its author's fork point. Such a
    /// node never applies.
    excluded: bool,
}

/// The author of an event.
pub(super) fn author_of(event: &Event) -> AuthorKey {
    *event.header.verifying_key.as_bytes()
}

/// Resolves which of `stored` apply, given the fork proofs as `(author, fork point)`.
///
/// # Errors
///
/// Returns [`StoreError::Corrupt`] when an event names a dependency that is not present, or when
/// the membership layer cannot keep its invariants. Neither happens for a store written by this
/// code.
pub(super) fn resolve(
    stored: &[Stored],
    forks: &HashMap<AuthorKey, SeqNum>,
    proof_headers: &[Event],
) -> Result<Resolved, StoreError> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut position: HashMap<Hash, usize> = HashMap::new();
    for item in stored {
        let event = &item.event;
        let author = author_of(event);
        let seq = event.header.seq_num;
        let excluded = forks.get(&author).is_some_and(|point| seq >= *point);
        position.insert(event.id, nodes.len());
        nodes.push(Node {
            id: event.id,
            author,
            group: event.header.extensions.group,
            after: event.header.extensions.after.clone(),
            action: item.action.clone(),
            excluded,
        });
    }
    for header in proof_headers {
        if position.contains_key(&header.id) {
            continue;
        }
        position.insert(header.id, nodes.len());
        nodes.push(Node {
            id: header.id,
            author: author_of(header),
            group: header.header.extensions.group,
            after: header.header.extensions.after.clone(),
            action: None,
            excluded: true,
        });
    }

    let order = causal_order(&nodes, &position)?;

    // The group fold is judged first: it depends only on the stored set, never on the
    // membership layer, and the operations it refuses with a voiding reason are known before
    // the layer is built. Pass three below still removes them from `applied`; feeding them to
    // the layer as no-ops here is what keeps their authority changes from reaching the
    // descendants the layer judges in passes one and two (Codex P1 on PR #3725).
    let projection = fold_projection(&nodes);
    let voided: HashSet<Hash> = match &projection {
        Some(projection) => nodes
            .iter()
            .filter(|node| {
                node.action.is_some()
                    && !node.excluded
                    && projection
                        .outcome(&node.id)
                        .and_then(|outcome| outcome.reason())
                        == Some(Reason::Voided)
            })
            .map(|node| node.id)
            .collect(),
        None => HashSet::new(),
    };

    // Pass one: the membership layer over every node that is not excluded, in causal order. An
    // excluded node stays in the index with the dependencies it can name, so a dependent of a
    // fork proof header still has a causal past.
    let has_actions = nodes.iter().any(|node| node.action.is_some());
    let mut membership = has_actions.then(Membership::new);
    if let Some(layer) = membership.as_mut() {
        for &index in &order {
            let node = &nodes[index];
            let after: Vec<Hash> = node
                .after
                .iter()
                .filter(|dep| position.contains_key(dep))
                .copied()
                .collect();
            let role = match (&node.action, node.excluded) {
                (Some(action), false) if !voided.contains(&node.id) => Role::Auth {
                    author: node.author,
                    group: node.group,
                    action: action.clone(),
                },
                _ => Role::Other,
            };
            layer
                .receive(CausalEvent {
                    id: node.id,
                    after,
                    role,
                })
                .map_err(|err| corrupt(format!("the membership layer: {err}")))?;
        }
    }

    // Pass two: each node's effect, in causal order, so that a dependency is decided first.
    let mut applied = HashSet::new();
    for &index in &order {
        let node = &nodes[index];
        if node.excluded {
            continue;
        }
        let takes_effect = match (&node.action, membership.as_mut()) {
            (Some(_), Some(layer)) => layer.is_effective(&node.id),
            (Some(_), None) => false,
            (None, layer) => content_applies(node, &applied, layer)?,
        };
        if takes_effect {
            applied.insert(node.id);
        }
    }

    // Pass three: the group policy gate (#168). An operation the fold refuses with a voiding
    // reason does not apply, even when the membership layer let it through. The projection is
    // kept on the result: it decides the roster of every group it folded. The layer was built
    // with these operations as no-ops, so this removal is the belt to that brace: it also
    // covers an operation the layer never saw as an action at all.
    for id in &voided {
        applied.remove(id);
    }

    Ok(Resolved {
        applied,
        membership,
        projection,
    })
}

/// The fold over the store's group operations, bridged into the group layer's event model.
///
/// A create becomes a group create under the custom template with the default policy record —
/// the record a signed store body cannot carry yet — and an add, remove, promote or demote
/// becomes the member operation with the same member and access. An operation naming a group
/// as its member has no group-event form, and a create that does not list its author has no
/// agreed form (see [`bridge`]), so both are left out and keep the membership verdict.
/// A bridged event names as its `after` the bridged operations it causally follows, reached
/// through any stored events in between, because the fold requires every named dependency to
/// be in its set. A fold that cannot judge the set (it reports an error) gates nothing: the
/// membership verdict stands rather than the store refusing every event it holds.
fn fold_projection(nodes: &[Node]) -> Option<Projection> {
    let position: HashMap<Hash, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect();
    let mut bridged: HashMap<Hash, GroupEvent> = HashMap::new();
    for node in nodes {
        if node.excluded {
            continue;
        }
        if let Some(event) = bridge(node) {
            bridged.insert(node.id, event);
        }
    }
    if bridged.is_empty() {
        return None;
    }
    let bridged_ids: HashSet<Hash> = bridged.keys().copied().collect();
    let mut events: Vec<GroupEvent> = Vec::with_capacity(bridged.len());
    for node in nodes {
        let Some(mut event) = bridged.remove(&node.id) else {
            continue;
        };
        event.after = bridged_after(node, nodes, &position, &bridged_ids);
        events.push(event);
    }
    crate::groups::fold(&events).ok()
}

/// The bridged operations `node` causally follows: walk its `after` links through every stored
/// node, collecting bridged operations and passing through anything else (content, fork proof
/// headers, operations that did not bridge), so causal reachability between bridged operations
/// is preserved exactly.
fn bridged_after(
    node: &Node,
    nodes: &[Node],
    position: &HashMap<Hash, usize>,
    bridged: &HashSet<Hash>,
) -> Vec<Hash> {
    let mut found: Vec<Hash> = Vec::new();
    let mut seen: HashSet<Hash> = HashSet::new();
    let mut stack: Vec<Hash> = node.after.clone();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if bridged.contains(&id) {
            found.push(id);
            continue;
        }
        if let Some(&index) = position.get(&id) {
            stack.extend(nodes[index].after.iter().copied());
        }
    }
    found
}

/// One stored group operation as a group event, or `None` when it has no group-event form.
fn bridge(node: &Node) -> Option<GroupEvent> {
    let action = node.action.as_ref()?;
    let body = match action {
        GroupAction::Create { initial_members } => {
            if node.group.is_some() {
                return None;
            }
            // The two models name the creator differently: the membership layer knows the
            // creator only when the create lists it, while the fold counts the creator as a
            // member (and Admin) by construction and refuses a create that names it. Bridge
            // only a create that lists its author — the form both models agree on — and drop
            // the author's entry for the fold.
            let mut members = Vec::with_capacity(initial_members.len());
            let mut author_listed = false;
            for (member, access) in initial_members {
                let key = member_key(member)?;
                if key == node.author {
                    // The fold counts the creator as an Admin (Manage) by construction, so a
                    // create listing its author below Manage names two different roles for the
                    // creator in the two models. That mapping cannot be reconciled before
                    // folding, so the create does not bridge and the group keeps the membership
                    // verdict alone (Codex P2 on PR #3725).
                    if !access.is_manage() {
                        return None;
                    }
                    author_listed = true;
                    continue;
                }
                members.push((key, role_of(access)));
            }
            if !author_listed {
                return None;
            }
            GroupBody::Create(GroupCreate {
                template: Template::Custom,
                policy: Policy::default(),
                members,
                admins_on_create: false,
            })
        }
        GroupAction::Add { member, access } => GroupBody::Member(MemberOp::Add {
            member: member_key(member)?,
            role: role_of(access),
        }),
        GroupAction::Remove { member } => GroupBody::Member(MemberOp::Remove {
            member: member_key(member)?,
            successor: None,
        }),
        GroupAction::Promote { member, access } | GroupAction::Demote { member, access } => {
            GroupBody::Member(MemberOp::SetRole {
                member: member_key(member)?,
                role: role_of(access),
            })
        }
    };
    if !matches!(body, GroupBody::Create(_)) && node.group.is_none() {
        return None;
    }
    Some(GroupEvent {
        id: node.id,
        author: node.author,
        after: Vec::new(),
        group: node.group,
        body,
    })
}

/// The account key of a member, or `None` when the member is a group: group events name
/// accounts only, so an operation on a group member cannot bridge.
fn member_key(member: &GroupMember<AuthId>) -> Option<crate::groups::Key> {
    match member {
        GroupMember::Individual(AuthId::Key(key)) => Some(*key),
        GroupMember::Individual(AuthId::Group(_)) | GroupMember::Group(_) => None,
    }
}

/// A group role at an access level. Admin has no access level of its own; it shows as Manage,
/// as it does in the fold.
fn role_of(access: &p2panda_auth::Access<()>) -> GroupRole {
    if access.is_manage() {
        GroupRole::Manage
    } else if access.is_write() {
        GroupRole::Write
    } else if access.is_read() {
        GroupRole::Read
    } else {
        GroupRole::Pull
    }
}

/// Whether content applies. It applies unless it names a voided event and its author is not a
/// member of its group at its position.
fn content_applies(
    node: &Node,
    applied: &HashSet<Hash>,
    membership: Option<&mut Membership>,
) -> Result<bool, StoreError> {
    let names_voided = node.after.iter().any(|dep| !applied.contains(dep));
    let (Some(group), Some(layer), true) = (node.group, membership, names_voided) else {
        return Ok(true);
    };
    if !layer.has_group(group) {
        return Ok(true);
    }
    layer
        .is_member_at(node.author, group, &node.after)
        .map_err(|err| corrupt(format!("the membership layer: {err}")))
}

/// The node indices in an order where every dependency comes first. A node that names an absent
/// dependency is placed only when it is excluded, since its dependencies are then dropped.
fn causal_order(nodes: &[Node], position: &HashMap<Hash, usize>) -> Result<Vec<usize>, StoreError> {
    // Causal order with the canonical tie-break: among the nodes whose dependencies are all
    // placed, the smallest event ID goes first -- the groups fold's rule. The membership
    // layer is fed in this order, so it crowns the same winner for concurrent creates of one
    // group as the fold does, whatever order the events were stored in.
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let mut placed = vec![false; nodes.len()];
    let mut queued = vec![false; nodes.len()];
    let mut order = Vec::with_capacity(nodes.len());
    let mut ready: BinaryHeap<Reverse<(Hash, usize)>> = BinaryHeap::new();
    loop {
        for (index, node) in nodes.iter().enumerate() {
            if placed[index] || queued[index] {
                continue;
            }
            let is_ready = node.after.iter().all(|dep| match position.get(dep) {
                Some(&other) => placed[other],
                None => node.excluded,
            });
            if is_ready {
                queued[index] = true;
                ready.push(Reverse((node.id, index)));
            }
        }
        let Some(Reverse((_, index))) = ready.pop() else {
            break;
        };
        placed[index] = true;
        order.push(index);
    }
    if order.len() != nodes.len() {
        return Err(corrupt(
            "an event names a dependency that is not stored, or the dependencies form a cycle",
        ));
    }
    Ok(order)
}

/// What [`load`] reads: the stored events, each author's lowest fork point, and every fork proof
/// header that has ever been kept.
pub(super) type Loaded = (Vec<Stored>, HashMap<AuthorKey, SeqNum>, Vec<Event>);

/// Reads every stored event with its action, the fork points, and the kept fork proof headers.
pub(super) fn load<T>(
    events: &T,
    actions: &T,
    forks: &T,
    proof_headers: &T,
) -> Result<Loaded, StoreError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    let mut stored = Vec::new();
    for entry in events.iter()? {
        let (_, value) = entry?;
        let event = decode_stored(value.value())?;
        let action = match actions.get(event.id.as_bytes().as_slice())? {
            Some(row) => Some(decode_action(row.value())?),
            None => None,
        };
        stored.push(Stored { event, action });
    }
    // An author has a row at each point it has forked at. Its fork point is the lowest of them.
    let mut fork_points: HashMap<AuthorKey, SeqNum> = HashMap::new();
    for entry in forks.iter()? {
        let (key, value) = entry?;
        let (author, _) = split_position(key.value())?;
        let proof = decode_proof(value.value())?;
        let point = fork_points.entry(author).or_insert(proof.seq);
        *point = (*point).min(proof.seq);
    }
    let mut headers = Vec::new();
    for entry in proof_headers.iter()? {
        let (_, value) = entry?;
        headers.push(decode_stored(value.value())?);
    }
    Ok((stored, fork_points, headers))
}

/// Encodes a group action for its table.
pub(super) fn encode_action(action: &GroupAction<AuthId, ()>) -> Result<Vec<u8>, StoreError> {
    let value = cbor_core::Value::serialized(action)
        .map_err(|err| corrupt(format!("a group action does not encode: {err}")))?;
    Ok(value.encode())
}

/// Decodes a group action from its table.
pub(super) fn decode_action(bytes: &[u8]) -> Result<GroupAction<AuthId, ()>, StoreError> {
    let value = cbor_core::Value::decode(bytes)
        .map_err(|err| corrupt(format!("a group action row is not CBOR: {err}")))?;
    value
        .deserialized()
        .map_err(|err| corrupt(format!("a group action row does not decode: {err}")))
}

/// The members of `group` in a resolution, as keys. Empty when there is no membership layer.
/// A group the fold decided takes its roster from the fold's view, so an operation the fold
/// refused never leaves its target listed as a member.
pub(super) fn roster_of(resolved: &Resolved, group: &Hash) -> Vec<AuthorKey> {
    if let Some(view) = resolved
        .projection
        .as_ref()
        .and_then(|projection| projection.group(group))
    {
        let mut keys: Vec<AuthorKey> = view.members.keys().copied().collect();
        keys.sort_unstable();
        return keys;
    }
    let Some(layer) = resolved.membership.as_ref() else {
        return Vec::new();
    };
    let mut keys: Vec<AuthorKey> = layer
        .members(*group)
        .into_iter()
        .filter_map(|(who, _)| match who {
            AuthId::Key(key) => Some(key),
            AuthId::Group(_) => None,
        })
        .collect();
    keys.sort_unstable();
    keys
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn node(n: u8, after: &[u8]) -> Node {
        Node {
            id: Hash::digest([n]),
            author: [0; 32],
            group: None,
            after: after.iter().map(|a| Hash::digest([*a])).collect(),
            action: None,
            excluded: false,
        }
    }

    /// The order is a function of the node set: permuting the stored positions changes
    /// nothing, and concurrent nodes are ordered by event ID, not by stored position.
    #[test]
    fn test_causal_order_is_canonical_under_permutation() {
        // Events 11 and 12 are concurrent roots; event 13 follows 11.
        let base = [node(11, &[]), node(12, &[]), node(13, &[11])];
        let mut reference: Option<Vec<Hash>> = None;
        for perm in [[0, 1, 2], [2, 1, 0], [1, 2, 0], [2, 0, 1]] {
            let nodes: Vec<Node> = perm
                .iter()
                .map(|&i| Node {
                    id: base[i].id,
                    author: [0; 32],
                    group: None,
                    after: base[i].after.clone(),
                    action: None,
                    excluded: false,
                })
                .collect();
            let position: HashMap<Hash, usize> = nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (node.id, index))
                .collect();
            let order = causal_order(&nodes, &position).unwrap();
            let ids: Vec<Hash> = order.iter().map(|&index| nodes[index].id).collect();
            // Causal order holds: 13 comes after 11.
            assert!(
                ids.iter().position(|id| *id == Hash::digest([11]))
                    < ids.iter().position(|id| *id == Hash::digest([13]))
            );
            match &reference {
                None => reference = Some(ids),
                Some(expected) => assert_eq!(&ids, expected),
            }
        }
        // The concurrent roots come out smallest-ID first.
        let ids = reference.unwrap();
        let smaller_first = (Hash::digest([11]) < Hash::digest([12]))
            == (ids[0] == Hash::digest([11]));
        assert!(smaller_first);
    }
}

