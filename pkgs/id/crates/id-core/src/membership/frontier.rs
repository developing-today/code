//! The auth-frontier mapping (P2.1).
//!
//! An auth event names its causal past through `after`, and that can include events which are
//! not group operations. p2panda-auth orders group operations only by their explicit
//! `dependencies`. So each auth operation needs the maximal group operations of its group in its
//! full causal past, found by walking `after` through the other events. That set is its auth
//! frontier.
//!
//! The index holds every event the membership layer has decided, with its `after` hashes and
//! whether it stands in the auth graph. The frontier of a past is memoised per group, so each
//! event does work only for the part of its past that the index has not seen for that group.
//!
//! Three rules keep the mapping honest (S2 showed each failure):
//!
//! - An `after` hash the index does not hold is an error, never a root. Treating it as a root
//!   drops an add the author had seen, which voids a valid operation.
//! - Only events that stand in the auth graph are named. An event that stands for no graph node
//!   (any non-auth event, or a refused event with no placeholder) is walked through.
//! - The result is minimal: an operation dominated by another in the set is dropped.

use std::collections::{HashMap, HashSet};

use p2panda_core::Hash;
use thiserror::Error;

/// Why the index refused a lookup or an insert.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FrontierError {
    /// An `after` hash the index does not hold. The event must wait for it.
    #[error("event {0} is not in the causal index")]
    UnknownEvent(Hash),
    /// The event is already indexed. Inserting it again would corrupt the memo.
    #[error("event {0} is already in the causal index")]
    AlreadyIndexed(Hash),
}

#[derive(Clone, Debug)]
struct Node {
    after: Vec<Hash>,
    /// The group of this event as a node of the auth graph, if it is one.
    graph_group: Option<Hash>,
}

/// The decided events, with the auth frontier of each past memoised per group.
#[derive(Clone, Debug, Default)]
pub struct CausalIndex {
    nodes: HashMap<Hash, Node>,
    /// `memo[(event, group)]` is the auth frontier of the past of `event` for `group`.
    memo: HashMap<(Hash, Hash), Vec<Hash>>,
}

impl CausalIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the event is indexed (decided, in any way).
    pub fn contains(&self, id: &Hash) -> bool {
        self.nodes.contains_key(id)
    }

    /// Records a decided event. Its `after` hashes must already be indexed, which the caller
    /// checks by holding the event until they are.
    ///
    /// `graph_group` is the group of the event as a node of the auth graph, or `None` when the
    /// event stands for no graph node.
    pub fn insert(
        &mut self,
        id: Hash,
        after: Vec<Hash>,
        graph_group: Option<Hash>,
    ) -> Result<(), FrontierError> {
        if self.nodes.contains_key(&id) {
            return Err(FrontierError::AlreadyIndexed(id));
        }
        if let Some(missing) = after.iter().find(|hash| !self.nodes.contains_key(hash)) {
            return Err(FrontierError::UnknownEvent(*missing));
        }
        self.nodes.insert(id, Node { after, graph_group });
        Ok(())
    }

    /// The auth frontier for `group` of an event whose `after` is `after`: the maximal group
    /// operations in the causal past of that event, sorted. An `after` hash that is itself a group
    /// operation of `group` counts as its own candidate.
    pub fn frontier(&mut self, after: &[Hash], group: Hash) -> Result<Vec<Hash>, FrontierError> {
        self.fill(after, group)?;
        self.combine(after, group)
    }

    /// Memoises the frontier of every event in the past of `roots` for `group`. The walk is
    /// iterative, so a long chain of non-auth events cannot exhaust the stack.
    fn fill(&mut self, roots: &[Hash], group: Hash) -> Result<(), FrontierError> {
        let mut stack: Vec<(Hash, bool)> = roots.iter().map(|id| (*id, false)).collect();
        while let Some((id, expanded)) = stack.pop() {
            if self.memo.contains_key(&(id, group)) {
                continue;
            }
            let after = self
                .nodes
                .get(&id)
                .ok_or(FrontierError::UnknownEvent(id))?
                .after
                .clone();
            if expanded {
                let frontier = self.combine(&after, group)?;
                self.memo.insert((id, group), frontier);
            } else {
                stack.push((id, true));
                for parent in after {
                    if !self.memo.contains_key(&(parent, group)) {
                        stack.push((parent, false));
                    }
                }
            }
        }
        Ok(())
    }

    /// The minimal set of group operations that a past built from `after` contains. Each `after`
    /// hash contributes itself when it is a group operation, and otherwise its own frontier. The
    /// memo must already cover the past.
    fn combine(&self, after: &[Hash], group: Hash) -> Result<Vec<Hash>, FrontierError> {
        let mut candidates = Vec::new();
        for parent in after {
            let node = self
                .nodes
                .get(parent)
                .ok_or(FrontierError::UnknownEvent(*parent))?;
            if node.graph_group == Some(group) {
                candidates.push(*parent);
            } else {
                let frontier = self
                    .memo
                    .get(&(*parent, group))
                    .ok_or(FrontierError::UnknownEvent(*parent))?;
                candidates.extend(frontier.iter().copied());
            }
        }
        candidates.sort();
        candidates.dedup();

        let maximal = candidates
            .iter()
            .filter(|candidate| {
                !candidates
                    .iter()
                    .any(|other| other != *candidate && self.below(**candidate, *other, group))
            })
            .copied()
            .collect();
        Ok(maximal)
    }

    /// Whether `candidate` lies in the causal past of `of`, a group operation. Reads only the
    /// memoised frontiers, walking from `of` through the frontiers of the operations it reaches.
    fn below(&self, candidate: Hash, of: Hash, group: Hash) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![of];
        while let Some(current) = stack.pop() {
            let Some(frontier) = self.memo.get(&(current, group)) else {
                continue;
            };
            for hash in frontier {
                if *hash == candidate {
                    return true;
                }
                if seen.insert(*hash) {
                    stack.push(*hash);
                }
            }
        }
        false
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const G: u8 = 1;
    const H: u8 = 2;

    fn h(n: u8) -> Hash {
        Hash::digest([n])
    }

    /// Indexes an event: `auth` is the group it stands for in the auth graph.
    fn put(index: &mut CausalIndex, id: u8, after: &[u8], auth: Option<u8>) {
        let after: Vec<Hash> = after.iter().map(|n| h(*n)).collect();
        index.insert(h(id), after, auth.map(h)).unwrap();
    }

    #[test]
    fn test_group_operations_over_each_other_name_the_latest() {
        // 10 (group G) is the create. 11 (group G) names 10. 12 (group G) names 11.
        let mut index = CausalIndex::new();
        put(&mut index, 10, &[], Some(G));
        put(&mut index, 11, &[10], Some(G));
        put(&mut index, 12, &[11], Some(G));
        assert_eq!(index.frontier(&[h(12)], h(G)).unwrap(), vec![h(12)]);
        assert_eq!(index.frontier(&[h(11)], h(G)).unwrap(), vec![h(11)]);
    }

    #[test]
    fn test_a_non_auth_hop_is_walked_through() {
        // X (auth) -> D (document) -> R names D. R's frontier is X, not D (S2 mapping case).
        let mut index = CausalIndex::new();
        put(&mut index, 1, &[], Some(G));
        put(&mut index, 2, &[1], None);
        assert_eq!(index.frontier(&[h(2)], h(G)).unwrap(), vec![h(1)]);
    }

    #[test]
    fn test_frontier_is_minimal_across_non_auth_hops() {
        // X1 -> D1 -> X2 -> D2 -> R. X2's frontier is {X1}; R's is {X2}, never {X1, X2}.
        let mut index = CausalIndex::new();
        put(&mut index, 1, &[], Some(G));
        put(&mut index, 2, &[1], None);
        put(&mut index, 3, &[2], Some(G));
        put(&mut index, 4, &[3], None);
        // The frontier of an event is taken over its `after`: X2 names [2], R names [4].
        assert_eq!(index.frontier(&[h(2)], h(G)).unwrap(), vec![h(1)]);
        assert_eq!(index.frontier(&[h(4)], h(G)).unwrap(), vec![h(3)]);
    }

    #[test]
    fn test_concurrent_group_operations_are_both_kept() {
        // Two group operations by different authors, neither in the other's past.
        let mut index = CausalIndex::new();
        put(&mut index, 1, &[], Some(G));
        put(&mut index, 2, &[1], Some(G));
        put(&mut index, 3, &[1], Some(G));
        assert_eq!(
            index.frontier(&[h(2), h(3)], h(G)).unwrap(),
            vec![h(2), h(3)]
        );
    }

    #[test]
    fn test_another_groups_operation_is_walked_through() {
        // X (group G) -> Y (group H) -> R (group G). R's frontier for G is X: Y is another group,
        // but X is in R's past through Y.
        let mut index = CausalIndex::new();
        put(&mut index, 1, &[], Some(G));
        put(&mut index, 2, &[1], Some(H));
        assert_eq!(index.frontier(&[h(2)], h(G)).unwrap(), vec![h(1)]);
        assert_eq!(index.frontier(&[h(2)], h(H)).unwrap(), vec![h(2)]);
    }

    #[test]
    fn test_a_refused_event_with_no_graph_node_is_walked_through() {
        // X refused without a placeholder (graph_group None): R's frontier is X's own frontier.
        let mut index = CausalIndex::new();
        put(&mut index, 1, &[], Some(G));
        put(&mut index, 2, &[1], None);
        put(&mut index, 3, &[2], Some(G));
        assert_eq!(index.frontier(&[h(3)], h(G)).unwrap(), vec![h(3)]);
        assert_eq!(index.frontier(&[h(2)], h(G)).unwrap(), vec![h(1)]);
    }

    #[test]
    fn test_unknown_after_is_an_error_not_a_root() {
        // The event names 99, which the index does not hold. A root would drop X from its past.
        let mut index = CausalIndex::new();
        put(&mut index, 1, &[], Some(G));
        assert_eq!(
            index.frontier(&[h(1), h(99)], h(G)),
            Err(FrontierError::UnknownEvent(h(99)))
        );
    }

    #[test]
    fn test_insert_rejects_an_unknown_after() {
        let mut index = CausalIndex::new();
        assert_eq!(
            index.insert(h(2), vec![h(99)], None),
            Err(FrontierError::UnknownEvent(h(99)))
        );
        assert!(!index.contains(&h(2)));
    }

    #[test]
    fn test_insert_rejects_a_second_copy() {
        let mut index = CausalIndex::new();
        put(&mut index, 1, &[], Some(G));
        assert_eq!(
            index.insert(h(1), vec![], Some(h(G))),
            Err(FrontierError::AlreadyIndexed(h(1)))
        );
    }

    #[test]
    fn test_a_long_non_auth_chain_does_not_exhaust_the_stack() {
        let mut index = CausalIndex::new();
        let mut prev = Hash::digest(b"root");
        index.insert(prev, vec![], Some(h(G))).unwrap();
        for n in 0..20_000u32 {
            let id = Hash::digest(n.to_be_bytes());
            index.insert(id, vec![prev], None).unwrap();
            prev = id;
        }
        assert_eq!(
            index.frontier(&[prev], h(G)).unwrap(),
            vec![Hash::digest(b"root")]
        );
    }
}
