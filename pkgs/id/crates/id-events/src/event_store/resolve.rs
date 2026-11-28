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
//!    membership verdict alone; the fold judges them when they travel as group events. Each
//!    unbridged shape carries a tracked disposition ruled on issue #6980, with the
//!    per-shape reasoning recorded there. Two
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
use crate::groups::{
    Body as GroupBody, Create as GroupCreate, DeviceOp, Event as GroupEvent, MemberOp, Policy,
    Projection, Role as GroupRole, Template,
};
use id_core::membership::{AuthId, CausalEvent, Membership, Role};
use id_core::rejection::Reason;
use id_core::waiting::AuthorKey;
use id_store::chain::Event;
use id_store::chat::CHAT_MESSAGE_KIND;

/// One stored event, with the membership action it carries when it is a group operation.
#[derive(Clone, Debug)]
pub(super) struct Stored {
    pub event: Event,
    pub action: Option<GroupAction<AuthId, ()>>,
    /// The device operation the event's `group.event` body carries, with the group the body
    /// names, when one was recorded at ingest (issue #5656). A device operation has no
    /// membership action: the body codec does not carry it, so it reaches the fold only
    /// through this channel.
    pub device: Option<(DeviceOp, Option<Hash>)>,
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
    /// The device operation the event's body carries, with the group the body names.
    device: Option<(DeviceOp, Option<Hash>)>,
    /// True for a fork proof header, and for an event at or above its author's fork point. Such a
    /// node never applies.
    excluded: bool,
    /// True for a `chat.message.v1` event: chat content is membership-gated at its causal
    /// position (see [`content_applies`]).
    chat: bool,
    /// The author's own previous event, from the header backlink. A chat event's
    /// membership position includes it: a first message may name its group only here.
    backlink: Option<Hash>,
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
            device: item.device.clone(),
            excluded,
            chat: event.header.extensions.kind == CHAT_MESSAGE_KIND,
            backlink: event.header.backlink,
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
            device: None,
            excluded: true,
            chat: header.header.extensions.kind == CHAT_MESSAGE_KIND,
            backlink: header.header.backlink,
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
            // The layer's causal graph is the full one: an event also follows its
            // author's previous event (the header backlink), not only the frontier
            // its `after` names. Without the backlink edges, a position reached
            // only through an author's own chain — a chat message whose past runs
            // through the author's non-chat events, say — cannot be judged, and
            // membership at that position reads as absent (Codex P1 on PR #3965).
            let after: Vec<Hash> = node
                .after
                .iter()
                .chain(node.backlink.iter())
                .filter(|dep| position.contains_key(dep))
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
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
/// agreed form (see [`bridge`]), so both are left out and keep the membership verdict --
/// by ruling, not by omission: each unbridged shape's disposition is recorded on
/// issue #6980.
/// A bridged event names as its `after` the bridged operations it causally follows, reached
/// through any stored events in between, because the fold requires every named dependency to
/// be in its set. A fold that cannot judge the set (it reports an error) gates nothing: the
/// membership verdict stands rather than the store refusing every event it holds.
fn fold_projection(nodes: &[Node]) -> Option<Projection> {
    match fold_projection_checked(nodes) {
        Ok(projection) => projection,
        Err(error) => {
            // #6979: a fold error is NOT an empty bridged set. The empty set
            // (no operation bridged) returns Ok(None) and stays silent; a set
            // that bridged and then failed to fold must be observable,
            // because the policy gate in `resolve` is skipped for it and the
            // membership verdict stands in its place. Surface the error at
            // error level, then keep the documented fallback.
            tracing::error!(
                "groups fold failed for a bridged set; the group policy gate \
                 is skipped and the membership verdict stands: {error}"
            );
            None
        }
    }
}

/// The fold over the bridged operations, with the fold's error kept:
/// `Ok(None)` when no operation bridged (the empty bridged set),
/// `Ok(Some(_))` when the fold judged the set, and `Err(_)` when the bridged
/// set could not be folded.
///
/// Reachability of the error through [`resolve`]: `resolve` runs
/// [`causal_order`] over the stored nodes before calling
/// [`fold_projection`], and that rejects dependency cycles and absent
/// dependencies in the stored graph; the bridged map dedupes event IDs, and
/// [`bridged_after`] only names bridged events that are in the set. The pure
/// set-shape `FoldError` cases (cycle, missing dependency, duplicate ID)
/// are therefore not known to reach the fold through `resolve` today; they
/// ARE reachable at this function's own boundary, because bridged `after`
/// links are a contraction of the stored graph — pass-through nodes
/// (excluded ones included) can make a bridged event name itself, as the
/// regression test demonstrates. The membership-internal and unsettled
/// cases are excluded by no check `resolve` runs. In every case the error
/// must not be silently conflated with the empty bridged set.
fn fold_projection_checked(nodes: &[Node]) -> Result<Option<Projection>, crate::groups::FoldError> {
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
        return Ok(None);
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
    crate::groups::fold(&events).map(Some)
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
    // A device operation bridges from its recorded body, not from a membership action: the
    // group the body names is the group the operation was made in, and the header's group
    // stands when the body names none (issue #5656).
    if let Some((op, body_group)) = &node.device {
        let group = body_group.or(node.group)?;
        return Some(GroupEvent {
            id: node.id,
            author: node.author,
            after: Vec::new(),
            group: Some(group),
            body: GroupBody::Device(*op),
        });
    }
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
                    // verdict alone (Codex P2 on PR #3725; disposition ruled on
                    // issue #6980).
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
            mode: crate::groups::RemoveMode::Revoke,
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
/// accounts only, so an operation on a group member cannot bridge. Group members are
/// judged by the membership layer alone, by the ruling recorded on issue #6980: the
/// fold's roster names accounts, and a group member written as its ID bytes would
/// conflate group IDs with account keys.
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
/// member of its group at its position. A chat message is judged by the same membership rule
/// whether or not it names a voided event (Codex P1 on PR #3965): the local `send_chat` check
/// only governs cooperative senders, so a `chat.message.v1` event an outsider signed and
/// delivered through ingest or sync must not apply — and so never reach a member's timeline —
/// unless its author was a member of the group at the event's causal position. That position
/// includes the author's backlink, because a member's first message in a group may name the
/// group only through it.
fn content_applies(
    node: &Node,
    applied: &HashSet<Hash>,
    membership: Option<&mut Membership>,
) -> Result<bool, StoreError> {
    let names_voided = node.after.iter().any(|dep| !applied.contains(dep));
    let gated = names_voided || node.chat;
    let (Some(group), Some(layer), true) = (node.group, membership, gated) else {
        return Ok(true);
    };
    if !layer.has_group(group) {
        return Ok(true);
    }
    if node.chat {
        let mut position = node.after.clone();
        if let Some(backlink) = node.backlink
            && !position.contains(&backlink)
        {
            position.push(backlink);
        }
        return layer
            .is_member_at(node.author, group, &position)
            .map_err(|err| corrupt(format!("the membership layer: {err}")));
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

    // Dependency counts and child adjacency lists (as in the groups fold) enqueue each node
    // exactly when its last dependency is placed, so ordering a history is linear in its
    // size instead of rescanning every node after each placement.
    let mut waiting = vec![0usize; nodes.len()];
    let mut blocked = vec![false; nodes.len()];
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut ready: BinaryHeap<Reverse<(Hash, usize)>> = BinaryHeap::new();
    for (index, node) in nodes.iter().enumerate() {
        let after: HashSet<&Hash> = node.after.iter().collect();
        let mut dependencies: HashSet<usize> = HashSet::new();
        for dep in &after {
            match position.get(dep) {
                Some(&other) => {
                    dependencies.insert(other);
                }
                // An absent dependency is dropped only for an excluded node; any other
                // node naming one can never be placed, as before.
                None if !node.excluded => blocked[index] = true,
                None => {}
            }
        }
        waiting[index] = dependencies.len();
        for other in dependencies {
            children.entry(other).or_default().push(index);
        }
        if waiting[index] == 0 && !blocked[index] {
            ready.push(Reverse((node.id, index)));
        }
    }
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(Reverse((_, index))) = ready.pop() {
        order.push(index);
        for child in children.remove(&index).unwrap_or_default() {
            waiting[child] -= 1;
            if waiting[child] == 0 && !blocked[child] {
                ready.push(Reverse((nodes[child].id, child)));
            }
        }
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
    device_ops: &T,
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
        let device = match device_ops.get(event.id.as_bytes().as_slice())? {
            Some(row) => Some(decode_device_op(row.value())?),
            None => None,
        };
        stored.push(Stored {
            event,
            action,
            device,
        });
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

/// Encodes a device operation and the group its body names for its table, as the CBOR
/// array `[op, group]`, where `group` is the group's bytes or null. Mirrors
/// [`encode_action`]; the group travels with the op because the bridged fold event names
/// the group the operation was made in, which the event header alone may not carry.
pub(super) fn encode_device_op(op: &DeviceOp, group: Option<&Hash>) -> Result<Vec<u8>, StoreError> {
    let op_value = cbor_core::Value::serialized(op)
        .map_err(|err| corrupt(format!("a device operation does not encode: {err}")))?;
    let group_value = match group {
        Some(group) => cbor_core::Value::ByteString(group.as_bytes().to_vec().into()),
        None => cbor_core::Value::null(),
    };
    Ok(cbor_core::Value::Array(vec![op_value, group_value]).encode())
}

/// Decodes a device operation and the group its body names from its table.
pub(super) fn decode_device_op(bytes: &[u8]) -> Result<(DeviceOp, Option<Hash>), StoreError> {
    let value = cbor_core::Value::decode(bytes)
        .map_err(|err| corrupt(format!("a device operation row is not CBOR: {err}")))?;
    let cbor_core::Value::Array(items) = value else {
        return Err(corrupt("a device operation row is not an array".to_owned()));
    };
    let [op_value, group_value] = items.try_into().map_err(|_| {
        corrupt("a device operation row does not hold an op and a group".to_owned())
    })?;
    let op: DeviceOp = op_value
        .deserialized()
        .map_err(|err| corrupt(format!("a device operation row does not decode: {err}")))?;
    let group = match group_value {
        value if value.encode() == cbor_core::Value::null().encode() => None,
        cbor_core::Value::ByteString(bytes) => {
            let bytes: [u8; 32] = bytes.as_ref().try_into().map_err(|_| {
                corrupt("a device operation row's group is not 32 bytes".to_owned())
            })?;
            Some(Hash::from_bytes(bytes))
        }
        _ => {
            return Err(corrupt(
                "a device operation row's group is neither bytes nor null".to_owned(),
            ));
        }
    };
    Ok((op, group))
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
            device: None,
            excluded: false,
            chat: false,
            backlink: None,
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
                    device: None,
                    excluded: false,
                    chat: false,
                    backlink: None,
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
        let smaller_first =
            (Hash::digest([11]) < Hash::digest([12])) == (ids[0] == Hash::digest([11]));
        assert!(smaller_first);
    }

    /// A long chain stored newest-first comes out oldest-first: each node is enqueued by
    /// its parent's placement through the adjacency lists, with no rescan.
    #[test]
    fn test_causal_order_places_a_long_chain_in_order() {
        let nodes: Vec<Node> = (0..64u8)
            .rev()
            .map(|n| {
                let after: Vec<u8> = if n == 0 { Vec::new() } else { vec![n - 1] };
                node(n, &after)
            })
            .collect();
        let position: HashMap<Hash, usize> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();
        let order = causal_order(&nodes, &position).unwrap();
        let ids: Vec<Hash> = order.iter().map(|&index| nodes[index].id).collect();
        let expected: Vec<Hash> = (0..64u8).map(|n| Hash::digest([n])).collect();
        assert_eq!(ids, expected);
    }

    /// A bridged create whose only causal path back to a bridged event runs
    /// through a pass-through node that names the create itself: the bridged
    /// contraction makes the create name itself as its own dependency, and
    /// the fold refuses the set with a cycle (#6979 fixture).
    fn self_naming_bridged_set() -> Vec<Node> {
        let author: AuthorKey = [9; 32];
        let create = Node {
            id: Hash::digest([41]),
            author,
            group: None,
            after: vec![Hash::digest([42])],
            action: Some(GroupAction::Create {
                initial_members: vec![(
                    GroupMember::Individual(AuthId::Key(author)),
                    p2panda_auth::Access::manage(),
                )],
            }),
            device: None,
            excluded: false,
            chat: false,
            backlink: None,
        };
        let pass_through = Node {
            id: Hash::digest([42]),
            author,
            group: None,
            after: vec![Hash::digest([41])],
            action: None,
            device: None,
            excluded: true,
            chat: false,
            backlink: None,
        };
        vec![create, pass_through]
    }

    /// #6979: an empty bridged set, a foldable bridged set, and a bridged set
    /// the fold refuses are three distinguishable outcomes at the checked
    /// boundary — the error is not folded into the empty-set `None`.
    #[test]
    fn test_fold_projection_checked_distinguishes_error_from_empty() {
        // No operation bridges: the empty bridged set.
        let empty = [node(1, &[]), node(2, &[1])];
        assert_eq!(fold_projection_checked(&empty), Ok(None));

        // A lone bridged create folds.
        let author: AuthorKey = [9; 32];
        let create = Node {
            id: Hash::digest([43]),
            author,
            group: None,
            after: Vec::new(),
            action: Some(GroupAction::Create {
                initial_members: vec![(
                    GroupMember::Individual(AuthId::Key(author)),
                    p2panda_auth::Access::manage(),
                )],
            }),
            device: None,
            excluded: false,
            chat: false,
            backlink: None,
        };
        assert!(matches!(fold_projection_checked(&[create]), Ok(Some(_))));

        // The self-naming bridged set makes the fold report its error.
        assert_eq!(
            fold_projection_checked(&self_naming_bridged_set()),
            Err(crate::groups::FoldError::Cycle)
        );
    }

    #[derive(Clone, Default)]
    struct BufferWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn captured_fold_log(nodes: &[Node]) -> String {
        use tracing_subscriber::layer::SubscriberExt;
        let writer = BufferWriter::default();
        let buffer = writer.0.clone();
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .without_time()
            .with_writer(writer);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            let _ = fold_projection(nodes);
        });
        String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
    }

    /// #6979: the observable signal. A fold error logs at error level naming
    /// the failure; the empty bridged set — the case the error used to be
    /// conflated with — logs nothing.
    #[test]
    fn test_fold_projection_logs_fold_errors_but_not_the_empty_set() {
        let log = captured_fold_log(&self_naming_bridged_set());
        assert!(
            log.contains("groups fold failed") && log.contains("cycle"),
            "fold error must be surfaced in the log, got: {log}"
        );

        let empty = [node(1, &[]), node(2, &[1])];
        let log = captured_fold_log(&empty);
        assert!(
            !log.contains("groups fold failed"),
            "the empty bridged set must stay silent, got: {log}"
        );
    }
}
