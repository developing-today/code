//! Tags as signed events: observed-remove claims (Phase 5, P5.1; design §9.1, spike S8).
//!
//! A tag is a claim. `tag.set` names a subject (any event ID), a name, and an optional value. It
//! is attributed to the key that signed it. `tag.clear` names the claims it removes. Both are
//! ordinary events: the kind is in the header's [`ChainExt`](crate::chain::ChainExt), and so is the
//! operation, in its `tag` field. The operation is the tag's body (#182). It is signed with the
//! header, so an event carries its whole tag, and sync moves events alone.
//!
//! A clear removes a claim only when the claim is named by the clear and lies in the clear's
//! causal past, which is the set of events its author had seen. A concurrent add the clear did not
//! see survives it. A clear that names a claim its author never saw does nothing to that claim.
//! The past is the transitive closure of each event's `after` hashes and backlink, so the clear's
//! frontier need not name a claim for the clear to remove it.
//!
//! The operation's limits are checked when a header is built and when it is decoded. A value is
//! text inside the header, and the header decoder declares text at most 512 bytes long, so a value
//! is at most [`MAX_VALUE_BYTES`].
//!
//! Two folds give the live claims. [`live_claims`] walks the causal past of each clear. It is the
//! reference. [`compacted_claims`] makes one pass over the parent links and keeps, for each event,
//! only the live claims in its past. Both give the same result (P5.3, #184) — including on a
//! cyclic `parents` graph, where the one-pass fold cannot order events after their parents:
//! [`compacted_claims`] detects a cycle and falls back to the reference computation for the
//! whole input rather than answer by iteration order (#6709). A cycle cannot arise from the
//! store — an event's ID is a hash of bytes that name its parents — so the fallback is a
//! contract guarantee for out-of-contract inputs, not a production path. An applied list
//! may repeat an event ID — a verbatim duplicate, or two entries with the same ID and
//! different contents; both folds keep only the last entry for an ID, so the agreement
//! holds on those inputs too (#4359).
//!
//! The old iroh-docs tag store in [`crate::tags`] is a separate model and is not read here.

use std::collections::{BTreeMap, HashMap, HashSet};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

use crate::waiting::AuthorKey;

/// The kind of a `tag.set` event, as the header's `kind` names it.
pub const SET_KIND: &str = "tag.set";

/// The kind of a `tag.clear` event, as the header's `kind` names it.
pub const CLEAR_KIND: &str = "tag.clear";

/// The most bytes a tag name may have. A name is at least one byte.
pub const MAX_NAME_BYTES: usize = 128;

/// The most bytes a tag value may have. The header decoder declares text at most 512 bytes long.
pub const MAX_VALUE_BYTES: usize = 512;

/// The most claims one `tag.clear` may name. The header decoder declares arrays at most 512 long.
pub const MAX_CLEAR_CLAIMS: usize = 256;

/// What a tag event does. The kind of the event says which form the operation takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagOp {
    /// Adds a claim: the subject carries `name` (and `value`), attributed to the signer.
    Set {
        /// The event the claim is about. Any event ID.
        subject: Hash,
        /// The tag name.
        name: String,
        /// The tag value. `None` for a bare tag.
        value: Option<String>,
    },
    /// Removes the named claims, if the clear's author had seen them.
    Clear {
        /// The IDs of the `tag.set` events this clear removes.
        claims: Vec<Hash>,
    },
}

/// An applied tag event, as the fold reads it: its ID, its signer, and its operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// The event's ID. For a `tag.set`, this is the claim's ID.
    pub id: Hash,
    /// The key that signed the event.
    pub author: AuthorKey,
    /// What the event does.
    pub op: TagOp,
}

/// A claim that is live: a `tag.set` that no applied clear removes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    /// The claim's ID: the `tag.set` event's ID.
    pub id: Hash,
    /// The key that signed the claim.
    pub author: AuthorKey,
    /// The subject the claim is about.
    pub subject: Hash,
    /// The tag name.
    pub name: String,
    /// The tag value, if any.
    pub value: Option<String>,
}

/// Why a tag operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TagError {
    /// The operation is not the one the event's kind names.
    #[error("a {kind:?} event carries the wrong tag operation")]
    KindMismatch {
        /// The kind of the event.
        kind: String,
    },
    /// A field breaks a limit or the NFC rule.
    #[error("{0}")]
    Field(String),
}

/// Whether `kind` is a tag kind, so its event carries a tag operation.
#[must_use]
pub fn is_tag_kind(kind: &str) -> bool {
    kind == SET_KIND || kind == CLEAR_KIND
}

/// Checks a tag operation against the kind of the event that carries it.
///
/// # Errors
///
/// Returns [`TagError::KindMismatch`] when the operation is not the one `kind` names. Returns
/// [`TagError::Field`] for a name or value that breaks its limit or the NFC rule, and for a clear
/// that names more than [`MAX_CLEAR_CLAIMS`] claims or names the same claim twice.
pub fn check_op(kind: &str, op: &TagOp) -> Result<(), TagError> {
    match (kind, op) {
        (SET_KIND, TagOp::Set { name, value, .. }) => {
            check_text("name", name, 1, MAX_NAME_BYTES)?;
            if let Some(text) = value {
                check_text("value", text, 0, MAX_VALUE_BYTES)?;
            }
            Ok(())
        }
        (CLEAR_KIND, TagOp::Clear { claims }) => {
            if claims.len() > MAX_CLEAR_CLAIMS {
                return Err(TagError::Field(format!(
                    "{} claims, the most is {MAX_CLEAR_CLAIMS}",
                    claims.len()
                )));
            }
            let mut seen = HashSet::with_capacity(claims.len());
            if !claims.iter().all(|claim| seen.insert(claim)) {
                return Err(TagError::Field("a clear names a claim twice".to_owned()));
            }
            Ok(())
        }
        _ => Err(TagError::KindMismatch {
            kind: kind.to_owned(),
        }),
    }
}

/// Checks a text field: its byte length is in `min..=max`, and it is NFC.
fn check_text(field: &str, text: &str, min: usize, max: usize) -> Result<(), TagError> {
    let length = text.len();
    if !(min..=max).contains(&length) {
        return Err(TagError::Field(format!(
            "the {field} is {length} bytes, it must be {min} to {max}"
        )));
    }
    if !is_nfc(text) {
        return Err(TagError::Field(format!("the {field} is not NFC")));
    }
    Ok(())
}

/// The events that `start` reaches by following parent links: its own causal past, transitively.
/// `parents` maps each event to the events it names, its `after` hashes and its backlink. The
/// starting events are included.
#[must_use]
pub fn causal_past(start: &[Hash], parents: &HashMap<Hash, Vec<Hash>>) -> HashSet<Hash> {
    let mut past = HashSet::new();
    let mut queue: Vec<Hash> = start.to_vec();
    while let Some(id) = queue.pop() {
        if !past.insert(id) {
            continue;
        }
        if let Some(named) = parents.get(&id) {
            queue.extend(named.iter().copied());
        }
    }
    past
}

/// The live claims, given the applied tag events and the parent links of every event the store
/// holds. A claim is live unless an applied clear names it and has it in its causal past.
/// If the applied list repeats an event ID, only the last entry for that ID is folded —
/// the same normalization [`compacted_claims`] gets from its by-ID map (#4359).
///
/// This walks the causal past of each clear, so it costs O(clears x history). It is the reference
/// for [`compacted_claims`]. The result is sorted by claim ID.
#[must_use]
pub fn live_claims(applied: &[Applied], parents: &HashMap<Hash, Vec<Hash>>) -> Vec<Claim> {
    // Fold only the last entry for each event ID, as `compacted_claims` does through
    // its by-ID map: a repeated ID -- even carrying a different operation -- is one event.
    let mut by_id: BTreeMap<[u8; 32], &Applied> = BTreeMap::new();
    for event in applied {
        by_id.insert(*event.id.as_bytes(), event);
    }
    let mut sets: BTreeMap<[u8; 32], Claim> = BTreeMap::new();
    let mut removed: HashSet<Hash> = HashSet::new();
    for event in by_id.values().copied() {
        if let TagOp::Set {
            subject,
            name,
            value,
        } = &event.op
        {
            sets.insert(
                *event.id.as_bytes(),
                Claim {
                    id: event.id,
                    author: event.author,
                    subject: *subject,
                    name: name.clone(),
                    value: value.clone(),
                },
            );
        }
    }
    for event in by_id.values().copied() {
        let TagOp::Clear { claims } = &event.op else {
            continue;
        };
        // What the clear's author had seen: the causal past of the clear, not counting the clear.
        let seen = parents
            .get(&event.id)
            .map(|named| causal_past(named, parents))
            .unwrap_or_default();
        removed.extend(claims.iter().filter(|claim| seen.contains(claim)));
    }
    sets.into_values()
        .filter(|claim| !removed.contains(&claim.id))
        .collect()
}

/// Whether the `parents` graph contains a directed cycle (a self-loop counts).
/// Iterative three-color depth-first search over every node the graph names —
/// keys and the events they name alike. Deterministic: the answer is a property
/// of the graph, never of iteration order.
fn parents_have_cycle(parents: &HashMap<Hash, Vec<Hash>>) -> bool {
    const UNVISITED: u8 = 0;
    const OPEN: u8 = 1;
    const DONE: u8 = 2;
    let mut color: HashMap<Hash, u8> = HashMap::new();
    let mut nodes: Vec<Hash> = parents.keys().copied().collect();
    for named in parents.values() {
        nodes.extend(named.iter().copied());
    }
    for start in nodes {
        if color.get(&start).copied().unwrap_or(UNVISITED) != UNVISITED {
            continue;
        }
        let mut stack = vec![(start, false)];
        while let Some((id, closing)) = stack.pop() {
            if closing {
                color.insert(id, DONE);
                continue;
            }
            match color.get(&id).copied().unwrap_or(UNVISITED) {
                DONE => continue,
                OPEN => return true,
                _ => {}
            }
            color.insert(id, OPEN);
            stack.push((id, true));
            if let Some(named) = parents.get(&id) {
                for parent in named {
                    match color.get(parent).copied().unwrap_or(UNVISITED) {
                        OPEN => return true,
                        DONE => {}
                        _ => stack.push((*parent, false)),
                    }
                }
            }
        }
    }
    false
}

/// The live claims, folded in one pass over the parent links (P5.3, #184).
///
/// Each event gets the set of live claims in its causal past, including itself. The set of an
/// event is built from its parents' sets, so each event is visited once. A clear removes each
/// claim it names that is in its parents' sets: those are the claims its author had seen. A
/// removed claim is dropped from every set built after it, so a set holds only claims that are
/// still live. Removal is final, so dropping it changes no later decision.
///
/// The result is the same as [`live_claims`], and it is sorted by claim ID. The output is built
/// from the by-ID map, so a repeated event ID contributes one claim — the last entry for that
/// ID — exactly as [`live_claims`] collapses repeats when it inserts into its map.
/// The walk for a clear is over the live claims in its parents' sets, not over its whole past. A clear from an author who
/// had not seen a claim leaves that claim live, as [`live_claims`] does.
///
/// Cyclic `parents` graphs are the one input the one-pass fold cannot order: whichever of a
/// set/clear pair a depth-first walk folds second would decide the result, so the answer
/// would depend on `HashMap` iteration order. A cycle is detected up front and the whole
/// input is folded by [`live_claims`] instead (#6709) — the equivalence is then by
/// construction, and deterministic. Cycles cannot arise from the store (an event ID is a
/// hash of the bytes naming its parents), so this costs one O(V + E) scan per call and the
/// fallback itself never runs on store inputs.
#[must_use]
pub fn compacted_claims(applied: &[Applied], parents: &HashMap<Hash, Vec<Hash>>) -> Vec<Claim> {
    if parents_have_cycle(parents) {
        return live_claims(applied, parents);
    }
    let by_id: HashMap<Hash, &Applied> = applied.iter().map(|event| (event.id, event)).collect();
    // The live claims in each event's causal past, including the event if it is a live claim.
    let mut live_past: HashMap<Hash, HashSet<Hash>> = HashMap::new();
    let mut removed: HashSet<Hash> = HashSet::new();
    let mut open: HashSet<Hash> = HashSet::new();
    for root in parents.keys().copied() {
        // A depth-first walk in post-order: an event is folded after its parents.
        let mut stack = vec![(root, false)];
        while let Some((id, folded)) = stack.pop() {
            if live_past.contains_key(&id) {
                continue;
            }
            let named: &[Hash] = match parents.get(&id) {
                Some(named) => named,
                None => &[],
            };
            if !folded {
                if !open.insert(id) {
                    continue;
                }
                stack.push((id, true));
                stack.extend(
                    named
                        .iter()
                        .filter(|parent| !live_past.contains_key(*parent))
                        .map(|parent| (*parent, false)),
                );
                continue;
            }
            open.remove(&id);
            let mut past: HashSet<Hash> = HashSet::new();
            for parent in named {
                if let Some(claims) = live_past.get(parent) {
                    past.extend(claims.iter().filter(|claim| !removed.contains(*claim)));
                }
            }
            match by_id.get(&id).map(|event| &event.op) {
                Some(TagOp::Clear { claims }) => {
                    removed.extend(claims.iter().filter(|claim| past.contains(*claim)));
                }
                Some(TagOp::Set { .. }) if !removed.contains(&id) => {
                    past.insert(id);
                }
                _ => {}
            }
            live_past.insert(id, past);
        }
    }
    // One claim per event ID: `by_id` keeps the last entry for a repeated ID, as
    // `live_claims` does when it inserts into its map, so duplicates in `applied`
    // never reach the output (#4359).
    let mut live: Vec<Claim> = by_id
        .values()
        .copied()
        .filter(|event| !removed.contains(&event.id))
        .filter_map(|event| match &event.op {
            TagOp::Set {
                subject,
                name,
                value,
            } => Some(Claim {
                id: event.id,
                author: event.author,
                subject: *subject,
                name: name.clone(),
                value: value.clone(),
            }),
            TagOp::Clear { .. } => None,
        })
        .collect();
    live.sort_by_key(|claim| *claim.id.as_bytes());
    live
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn hash(n: u8) -> Hash {
        Hash::from_bytes([n; 32])
    }

    fn set(id: u8, subject: u8, name: &str) -> Applied {
        Applied {
            id: hash(id),
            author: [id; 32],
            op: TagOp::Set {
                subject: hash(subject),
                name: name.to_owned(),
                value: None,
            },
        }
    }

    fn clear(id: u8, claims: &[u8]) -> Applied {
        Applied {
            id: hash(id),
            author: [id; 32],
            op: TagOp::Clear {
                claims: claims.iter().map(|n| hash(*n)).collect(),
            },
        }
    }

    #[test]
    fn test_a_tag_op_is_checked_against_its_kind() {
        let set_op = TagOp::Set {
            subject: hash(1),
            name: "city".to_owned(),
            value: Some(String::new()),
        };
        let clear_op = TagOp::Clear { claims: vec![] };
        assert!(check_op(SET_KIND, &set_op).is_ok());
        assert!(check_op(CLEAR_KIND, &clear_op).is_ok());
        assert!(check_op(CLEAR_KIND, &set_op).is_err());
        assert!(check_op(SET_KIND, &clear_op).is_err());
        assert!(check_op("message", &set_op).is_err());
    }

    #[test]
    fn test_a_name_or_value_outside_its_limits_is_refused() {
        let check = |name: &str, value: Option<String>| {
            check_op(
                SET_KIND,
                &TagOp::Set {
                    subject: hash(1),
                    name: name.to_owned(),
                    value,
                },
            )
        };
        assert!(check("", None).is_err());
        assert!(check(&"n".repeat(MAX_NAME_BYTES + 1), None).is_err());
        assert!(check("a", Some("v".repeat(MAX_VALUE_BYTES))).is_ok());
        assert!(check("a", Some("v".repeat(MAX_VALUE_BYTES + 1))).is_err());
        assert!(check("a", Some("e\u{301}".to_owned())).is_err());
    }

    #[test]
    fn test_a_clear_naming_a_claim_twice_is_refused() {
        let adjacent = TagOp::Clear {
            claims: vec![hash(1), hash(1)],
        };
        assert!(matches!(
            check_op(CLEAR_KIND, &adjacent),
            Err(TagError::Field(_))
        ));
        let non_adjacent = TagOp::Clear {
            claims: vec![hash(1), hash(2), hash(1)],
        };
        assert!(matches!(
            check_op(CLEAR_KIND, &non_adjacent),
            Err(TagError::Field(_))
        ));
    }

    #[test]
    fn test_a_clear_naming_max_distinct_claims_is_accepted() {
        let op = TagOp::Clear {
            claims: (0..MAX_CLEAR_CLAIMS)
                .map(|n| Hash::digest(n.to_be_bytes()))
                .collect(),
        };
        assert!(check_op(CLEAR_KIND, &op).is_ok());
    }

    #[test]
    fn test_a_clear_naming_too_many_claims_is_refused() {
        let op = TagOp::Clear {
            claims: (0..=MAX_CLEAR_CLAIMS)
                .map(|n| Hash::digest(n.to_be_bytes()))
                .collect(),
        };
        assert!(check_op(CLEAR_KIND, &op).is_err());
    }

    #[test]
    fn test_the_compacted_fold_keeps_a_claim_a_clear_did_not_see() {
        // `b` (event 3) names claim 1, but its past is only itself, so claim 1 stays live.
        let applied = [set(1, 9, "city"), clear(3, &[1])];
        let parents: HashMap<Hash, Vec<Hash>> = HashMap::from([(hash(3), vec![])]);
        let live = compacted_claims(&applied, &parents);
        assert_eq!(live.iter().map(|c| c.id).collect::<Vec<_>>(), vec![hash(1)]);
        assert_eq!(live, live_claims(&applied, &parents));
    }

    #[test]
    fn test_the_compacted_fold_removes_a_claim_its_clear_saw_through_another_event() {
        // Claim 1 is in the past of clear 4 through event 2, which names claim 1.
        let applied = [set(1, 9, "city"), set(2, 9, "seen"), clear(4, &[1])];
        let parents: HashMap<Hash, Vec<Hash>> =
            HashMap::from([(hash(2), vec![hash(1)]), (hash(4), vec![hash(2)])]);
        let live = compacted_claims(&applied, &parents);
        assert_eq!(live.iter().map(|c| c.id).collect::<Vec<_>>(), vec![hash(2)]);
        assert_eq!(live, live_claims(&applied, &parents));
    }

    #[test]
    fn test_a_duplicate_applied_set_entry_yields_one_claim() {
        // #4359: a verbatim repeat of a set entry must not emit the claim twice.
        let applied = [set(1, 9, "city"), set(1, 9, "city"), set(2, 9, "seen")];
        let parents: HashMap<Hash, Vec<Hash>> = HashMap::from([(hash(2), vec![hash(1)])]);
        let live = compacted_claims(&applied, &parents);
        assert_eq!(
            live.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![hash(1), hash(2)]
        );
        assert_eq!(live, live_claims(&applied, &parents));
    }

    #[test]
    fn test_same_id_entries_with_different_values_keep_the_last() {
        // #4359: equivocation — two entries share an ID; the last entry wins,
        // as it does in `live_claims`, and only one claim is emitted.
        let with_value = |value: &str| Applied {
            id: hash(1),
            author: [1; 32],
            op: TagOp::Set {
                subject: hash(9),
                name: "city".to_owned(),
                value: Some(value.to_owned()),
            },
        };
        let applied = [with_value("A"), with_value("B")];
        let parents: HashMap<Hash, Vec<Hash>> = HashMap::from([(hash(1), vec![])]);
        let live = compacted_claims(&applied, &parents);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].value, Some("B".to_owned()));
        assert_eq!(live, live_claims(&applied, &parents));
    }

    #[test]
    fn test_the_compacted_fold_agrees_on_the_two_cycle_repro() {
        // #6709: set 1, clear 2 naming 1, parents 1 -> [2], 2 -> [1].
        // The reference removes the claim: 1 is in 2's causal past through
        // the cycle. The compacted fold must agree, deterministically —
        // before the fix its answer flipped with HashMap iteration order.
        let applied = [set(1, 9, "city"), clear(2, &[1])];
        let parents: HashMap<Hash, Vec<Hash>> =
            HashMap::from([(hash(1), vec![hash(2)]), (hash(2), vec![hash(1)])]);
        for _ in 0..16 {
            assert!(compacted_claims(&applied, &parents).is_empty());
            assert_eq!(
                compacted_claims(&applied, &parents),
                live_claims(&applied, &parents)
            );
        }
    }

    #[test]
    fn test_the_compacted_fold_agrees_on_self_loops() {
        // A self-loop on the set, and one on the clear: both are cycles,
        // both folds must agree on each.
        let applied = [set(1, 9, "city"), clear(2, &[1])];
        let set_loop: HashMap<Hash, Vec<Hash>> =
            HashMap::from([(hash(1), vec![hash(1)]), (hash(2), vec![hash(1)])]);
        assert_eq!(
            compacted_claims(&applied, &set_loop),
            live_claims(&applied, &set_loop)
        );
        assert!(compacted_claims(&applied, &set_loop).is_empty());
        let clear_loop: HashMap<Hash, Vec<Hash>> =
            HashMap::from([(hash(1), vec![]), (hash(2), vec![hash(2), hash(1)])]);
        assert_eq!(
            compacted_claims(&applied, &clear_loop),
            live_claims(&applied, &clear_loop)
        );
        assert!(compacted_claims(&applied, &clear_loop).is_empty());
    }

    #[test]
    fn test_the_folds_agree_over_generated_graphs() {
        // Property test with `live_claims` as the oracle (the repo's
        // testing doctrine: every automated test names its oracle).
        // The signed-history proptest in tests/tag_claims.rs can only
        // generate DAGs; this generator builds raw parents graphs —
        // cycles, self-loops, duplicate parent entries, nodes named but
        // never applied — plus applied lists with duplicate IDs, the
        // shapes #4359 and #6709 are about. A fixed seed keeps the run
        // deterministic; 2000 graphs of up to 8 nodes.
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                x
            }
            fn below(&mut self, n: u64) -> u64 {
                self.next() % n
            }
        }
        let mut rng = Rng(0x6709_184_4359);
        for case in 0..2000 {
            let n = 1 + rng.below(8) as u8;
            let mut parents: HashMap<Hash, Vec<Hash>> = HashMap::new();
            for id in 1..=n {
                let edges = (0..rng.below(4))
                    .map(|_| hash(1 + rng.below(u64::from(n)) as u8))
                    .collect::<Vec<_>>();
                parents.insert(hash(id), edges);
            }
            let mut applied: Vec<Applied> = Vec::new();
            for id in 1..=n {
                match rng.below(3) {
                    0 => applied.push(set(id, 9, "city")),
                    1 => {
                        let claims: Vec<u8> = (0..rng.below(3))
                            .map(|_| 1 + rng.below(u64::from(n)) as u8)
                            .collect();
                        applied.push(clear(id, &claims));
                    }
                    _ => {}
                }
                // Occasional verbatim duplicate / same-ID second entry.
                if rng.below(4) == 0 {
                    applied.push(set(id, 9, "city"));
                }
            }
            assert_eq!(
                compacted_claims(&applied, &parents),
                live_claims(&applied, &parents),
                "folds diverged on generated graph case {case} (seed 0x6709_184_4359, n={n})"
            );
        }
    }

    #[test]
    fn test_same_id_entries_with_different_operations_fold_as_the_last() {
        // #4359: a repeated ID carrying a different operation is still one event:
        // the last entry decides, in both folds. Set-then-clear leaves no claim;
        // clear-then-set leaves the claim live.
        let parents: HashMap<Hash, Vec<Hash>> = HashMap::from([(hash(1), vec![])]);
        let set_then_clear = [set(1, 9, "city"), clear(1, &[])];
        assert!(compacted_claims(&set_then_clear, &parents).is_empty());
        assert_eq!(
            compacted_claims(&set_then_clear, &parents),
            live_claims(&set_then_clear, &parents)
        );
        let clear_then_set = [clear(1, &[]), set(1, 9, "city")];
        assert_eq!(compacted_claims(&clear_then_set, &parents).len(), 1);
        assert_eq!(
            compacted_claims(&clear_then_set, &parents),
            live_claims(&clear_then_set, &parents)
        );
    }
}
