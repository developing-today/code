//! Friendship as a group (design §5.4).
//!
//! A friendship is a group from the friendship template: two members, mutual request, and dissolve
//! on removal. Its ID is derived from the pair and a counter, so two concurrent requests for one
//! pair name the same group and collapse into one object. The counter increments when a friendship
//! dissolves, so re-friending makes a new group.

use p2panda_core::Hash;

use super::Key;

/// Two accounts, smaller key first. A friendship has exactly one pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    low: Key,
    high: Key,
}

impl Pair {
    /// The pair of two accounts, or `None` when they are the same account.
    pub fn of(a: Key, b: Key) -> Option<Self> {
        match a.cmp(&b) {
            std::cmp::Ordering::Less => Some(Self { low: a, high: b }),
            std::cmp::Ordering::Greater => Some(Self { low: b, high: a }),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// Both accounts, smaller first.
    pub const fn members(&self) -> [Key; 2] {
        [self.low, self.high]
    }
}

/// The ID of the friendship group for `pair` at `counter` (design §5.4): BLAKE3 over the label
/// "friendship", the sorted pair, and the counter.
pub fn friendship_id(pair: &Pair, counter: u32) -> Hash {
    let mut bytes = Vec::with_capacity(b"friendship".len() + 64 + 4);
    bytes.extend_from_slice(b"friendship");
    bytes.extend_from_slice(&pair.low);
    bytes.extend_from_slice(&pair.high);
    bytes.extend_from_slice(&counter.to_be_bytes());
    Hash::digest(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_a_pair_is_the_same_in_either_order_and_needs_two_accounts() {
        assert_eq!(Pair::of([1; 32], [2; 32]), Pair::of([2; 32], [1; 32]));
        assert_eq!(Pair::of([1; 32], [1; 32]), None);
    }

    #[test]
    fn test_the_friendship_id_depends_on_the_pair_and_the_counter() {
        let ann_bo = Pair::of([1; 32], [2; 32]).unwrap();
        let ann_cy = Pair::of([1; 32], [3; 32]).unwrap();
        assert_eq!(friendship_id(&ann_bo, 0), friendship_id(&ann_bo, 0));
        assert_ne!(friendship_id(&ann_bo, 0), friendship_id(&ann_bo, 1));
        assert_ne!(friendship_id(&ann_bo, 0), friendship_id(&ann_cy, 0));
    }
}
