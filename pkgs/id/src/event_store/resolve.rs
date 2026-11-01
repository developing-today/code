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
//!
//! A voided event stays stored. Its author's later events do not wait on it, and they are judged
//! by the same rules. That is why an honest author never stalls on another author's removal.

use std::collections::{HashMap, HashSet};

use p2panda_auth::group::GroupAction;
use p2panda_core::{Hash, SeqNum};
use redb::ReadableTable;

use super::{StoreError, corrupt, decode_proof, decode_stored, split_position};
use crate::chain::Event;
use crate::membership::{AuthId, CausalEvent, Membership, Role};
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
                (Some(action), false) => Role::Auth {
                    author: node.author,
                    group: node.group,
                    action: action.clone(),
                    originator: None,
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

    Ok(Resolved {
        applied,
        membership,
    })
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
    let mut placed = vec![false; nodes.len()];
    let mut order = Vec::with_capacity(nodes.len());
    let mut progress = true;
    while progress {
        progress = false;
        for (index, node) in nodes.iter().enumerate() {
            if placed[index] {
                continue;
            }
            let ready = node.after.iter().all(|dep| match position.get(dep) {
                Some(&other) => placed[other],
                None => node.excluded,
            });
            if ready {
                placed[index] = true;
                order.push(index);
                progress = true;
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
pub(super) fn roster_of(resolved: &Resolved, group: &Hash) -> Vec<AuthorKey> {
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
