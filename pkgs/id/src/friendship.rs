//! Friendship: one-way requests that become a friendship on accept, a cap on
//! friends per account, and a dissolve that ends a friendship on both sides.
//!
//! p2panda-auth has no member cap and no dissolve, so both are ours (S2, friendship).
//! The module is pure. Accounts and operation IDs are plain strings, and each
//! operation arrives already signed. Nothing here reads a clock or the network.
//!
//! The rules:
//!
//! - A request is one-way. It stays open until its recipient accepts or declines
//!   it, or a removal names it. Two accounts have at most one open request, and no
//!   request while they are friends.
//! - An accept makes a friendship if both accounts have fewer than the cap.
//! - A removal names the requests it covers, as the directory's `FriendRemove`
//!   does. Naming a request voids it. Naming the request that formed a friendship
//!   dissolves the friendship on both sides. A dissolved friendship is not revived
//!   by an accept of its request. Only a new request makes a new friendship.
//!
//! Each operation is judged against the state of its own causal past: its `after`
//! set, closed transitively. A later operation cannot change what an earlier one
//! meant. Two things depend on the whole set rather than on arrival. A removal wins
//! over a concurrent accept of the request it names. Where concurrent accepts would
//! exceed the cap, the one that comes first in the canonical order is kept. That
//! order is causal order, with ties going to the smaller ID. So every server that
//! holds the same operations reaches the same friend set.

use std::collections::{BTreeMap, BTreeSet};

/// The most friends one account may have. Our number, not p2panda-auth's.
pub const MAX_FRIENDS: usize = 100;

/// An account ID.
pub type Account = String;

/// An operation ID. A request's ID names the request.
pub type OpId = String;

/// Two accounts, smaller first.
pub type Pair = (Account, Account);

/// What an operation asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// `from` asks `to` to be friends.
    Request {
        /// The sender.
        from: Account,
        /// The recipient.
        to: Account,
    },
    /// The recipient `by` accepts `request`.
    Accept {
        /// The request accepted.
        request: OpId,
        /// The recipient, who must be the request's `to`.
        by: Account,
    },
    /// The recipient `by` declines `request`, which voids it.
    Decline {
        /// The request declined.
        request: OpId,
        /// The recipient, who must be the request's `to`.
        by: Account,
    },
    /// `by` ends its friendship with `other`, and voids the requests it names
    /// between them. Naming the request that formed the friendship dissolves it.
    Remove {
        /// The account removing.
        by: Account,
        /// The other account.
        other: Account,
        /// The requests it covers.
        requests: Vec<OpId>,
    },
}

/// An operation with its ID and the IDs it saw. The caller has checked the signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed {
    /// The operation's ID.
    pub id: OpId,
    /// The IDs of the operations it saw, directly.
    pub after: BTreeSet<OpId>,
    /// What it does.
    pub op: Op,
}

/// Why an operation had no effect.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// A request to oneself.
    #[error("no one befriends themselves")]
    SelfRequest,
    /// A removal of oneself.
    #[error("no one removes themselves")]
    SameAccount,
    /// Only the recipient of a request may accept or decline it.
    #[error("{by} is not the recipient of request {request}")]
    NotRecipient {
        /// The request.
        request: OpId,
        /// The account that tried to answer it.
        by: Account,
    },
    /// The request is not among the operations this one saw.
    #[error("request {request} is not among the operations this one saw")]
    UnknownRequest {
        /// The request.
        request: OpId,
    },
    /// A removal names a request between two other accounts.
    #[error("request {request} is not between these two accounts")]
    NotThisPair {
        /// The request.
        request: OpId,
    },
    /// The request was already accepted, declined or voided.
    #[error("request {request} is no longer open")]
    NotOpen {
        /// The request.
        request: OpId,
    },
    /// A removal that names the request dissolved the friendship it formed.
    #[error("request {request} was dissolved by a removal that names it")]
    Dissolved {
        /// The request.
        request: OpId,
    },
    /// The two accounts are already friends.
    #[error("already friends")]
    AlreadyFriends,
    /// An open request already exists between the two accounts.
    #[error("a request is already open between them")]
    AlreadyOpen,
    /// One account already has the maximum number of friends.
    #[error("{account} already has {cap} friends")]
    AtCap {
        /// The account at the cap.
        account: Account,
        /// The cap that was reached.
        cap: usize,
    },
    /// The removal covers no friendship and no open request.
    #[error("nothing to remove")]
    NothingToRemove,
}

/// What became of one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It took effect.
    Applied,
    /// It had no effect.
    Refused(Refusal),
}

/// The operations are not a usable set.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectError {
    /// Two operations share an ID.
    #[error("operation {0} appears twice")]
    DuplicateId(OpId),
    /// An operation names an ID that is not in the set.
    #[error("operation {op} waits for {dependency}")]
    MissingDependency {
        /// The operation that names the missing one.
        op: OpId,
        /// The missing operation.
        dependency: OpId,
    },
    /// The `after` sets form a cycle.
    #[error("the operations form a cycle")]
    Cycle,
}

/// The friend set that the operations determine, and what became of each one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    friends: BTreeSet<Pair>,
    outcomes: BTreeMap<OpId, Outcome>,
}

impl Projection {
    /// Every friendship, as sorted pairs.
    #[must_use]
    pub fn friendships(&self) -> &BTreeSet<Pair> {
        &self.friends
    }

    /// Whether two accounts are friends.
    #[must_use]
    pub fn are_friends(&self, a: &str, b: &str) -> bool {
        self.friends.contains(&pair(a, b))
    }

    /// The friends of one account, sorted.
    #[must_use]
    pub fn friends_of(&self, account: &str) -> Vec<Account> {
        let mut out: Vec<Account> = self
            .friends
            .iter()
            .filter_map(|(a, b)| {
                if a == account {
                    Some(b.clone())
                } else if b == account {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .collect();
        out.sort();
        out
    }

    /// What became of an operation, if it was in the set.
    #[must_use]
    pub fn outcome(&self, id: &str) -> Option<&Outcome> {
        self.outcomes.get(id)
    }
}

/// Project signed operations to the friend set they determine.
///
/// `ops` may come in any order. The result depends only on the operations and their
/// `after` sets. `cap` is the most friends one account may have. The product passes
/// [`MAX_FRIENDS`].
pub fn project(ops: &[Signed], cap: usize) -> Result<Projection, ProjectError> {
    let by_id = index(ops)?;
    let order = canonical_order(&by_id)?;
    let past = pasts(&order);

    // Each operation is judged against its own causal past, which the canonical order
    // has already settled.
    let mut verdicts: BTreeMap<&str, Option<Refusal>> = BTreeMap::new();
    for &signed in &order {
        let id = signed.id.as_str();
        let verdict = match static_refusal(signed, &by_id, &past[id]) {
            Some(refusal) => Some(refusal),
            None => check(&state_before(id, &order, &past, &verdicts), &signed.op, cap).err(),
        };
        verdicts.insert(id, verdict);
    }

    // A valid removal dissolves the friendship that its named request formed, whenever the
    // removal arrived. So a removal wins over a concurrent accept.
    let mut removed: BTreeSet<&str> = BTreeSet::new();
    for &signed in &order {
        if let (Op::Remove { requests, .. }, None) = (&signed.op, &verdicts[signed.id.as_str()]) {
            removed.extend(requests.iter().map(String::as_str));
        }
    }

    // Accepts that survive are kept in canonical order, while both accounts are under the cap.
    let mut friends: BTreeSet<Pair> = BTreeSet::new();
    let mut kept: BTreeMap<&str, usize> = BTreeMap::new();
    let mut outcomes = BTreeMap::new();
    for &signed in &order {
        let id = signed.id.as_str();
        let refusal = match verdicts[id].clone() {
            Some(refusal) => Some(refusal),
            None => match &signed.op {
                Op::Accept { request, .. } => {
                    if removed.contains(request.as_str()) {
                        Some(Refusal::Dissolved {
                            request: request.clone(),
                        })
                    } else if let Some((from, to)) = requested(request, &by_id, &past[id]) {
                        let p = pair(from, to);
                        let full = [from, to]
                            .into_iter()
                            .find(|account| kept.get(*account).copied().unwrap_or(0) >= cap);
                        if friends.contains(&p) {
                            Some(Refusal::AlreadyFriends)
                        } else if let Some(account) = full {
                            Some(Refusal::AtCap {
                                account: account.to_owned(),
                                cap,
                            })
                        } else {
                            for account in [from, to] {
                                *kept.entry(account).or_insert(0) += 1;
                            }
                            friends.insert(p);
                            None
                        }
                    } else {
                        Some(Refusal::UnknownRequest {
                            request: request.clone(),
                        })
                    }
                }
                _ => None,
            },
        };
        outcomes.insert(
            id.to_owned(),
            refusal.map_or(Outcome::Applied, Outcome::Refused),
        );
    }
    Ok(Projection { friends, outcomes })
}

/// Operations by ID.
type Index<'a> = BTreeMap<&'a str, &'a Signed>;

/// The causal past of each operation, by ID.
type Past<'a> = BTreeMap<&'a str, BTreeSet<&'a str>>;

fn pair(a: &str, b: &str) -> Pair {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

fn index(ops: &[Signed]) -> Result<Index<'_>, ProjectError> {
    let mut by_id = BTreeMap::new();
    for signed in ops {
        if by_id.insert(signed.id.as_str(), signed).is_some() {
            return Err(ProjectError::DuplicateId(signed.id.clone()));
        }
    }
    Ok(by_id)
}

/// A topological order that breaks ties by ID. It depends only on the operations.
fn canonical_order<'a>(by_id: &Index<'a>) -> Result<Vec<&'a Signed>, ProjectError> {
    let mut waiting: BTreeMap<&str, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (&id, &signed) in by_id {
        waiting.insert(id, signed.after.len());
        for dep in &signed.after {
            if !by_id.contains_key(dep.as_str()) {
                return Err(ProjectError::MissingDependency {
                    op: id.to_owned(),
                    dependency: dep.clone(),
                });
            }
            dependents.entry(dep.as_str()).or_default().push(id);
        }
    }
    let mut ready: BTreeSet<&str> = waiting
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut order = Vec::with_capacity(by_id.len());
    while let Some(id) = ready.pop_first() {
        order.push(by_id[id]);
        for &next in dependents.get(id).into_iter().flatten() {
            if let Some(n) = waiting.get_mut(next) {
                *n -= 1;
                if *n == 0 {
                    ready.insert(next);
                }
            }
        }
    }
    if order.len() != by_id.len() {
        return Err(ProjectError::Cycle);
    }
    Ok(order)
}

/// Each operation's causal past. `order` must be topological.
fn pasts<'a>(order: &[&'a Signed]) -> Past<'a> {
    let mut past: Past<'a> = BTreeMap::new();
    for &signed in order {
        let mut seen: BTreeSet<&'a str> = BTreeSet::new();
        for dep in &signed.after {
            seen.insert(dep.as_str());
            if let Some(before) = past.get(dep.as_str()) {
                seen.extend(before.iter().copied());
            }
        }
        past.insert(signed.id.as_str(), seen);
    }
    past
}

/// The state that the valid operations in `id`'s causal past build, in canonical order.
fn state_before(
    id: &str,
    order: &[&Signed],
    past: &Past<'_>,
    verdicts: &BTreeMap<&str, Option<Refusal>>,
) -> State {
    let mut state = State::default();
    let before = &past[id];
    for &signed in order {
        let sid = signed.id.as_str();
        if before.contains(sid) && verdicts.get(sid).is_some_and(Option::is_none) {
            effect(&mut state, signed);
        }
    }
    state
}

/// The sender and recipient of a request in the causal past, if `id` is one.
fn requested<'a>(id: &str, by_id: &Index<'a>, past: &BTreeSet<&str>) -> Option<(&'a str, &'a str)> {
    if !past.contains(id) {
        return None;
    }
    let signed: &'a Signed = *by_id.get(id)?;
    match &signed.op {
        Op::Request { from, to } => Some((from.as_str(), to.as_str())),
        _ => None,
    }
}

/// Refusals that depend only on the operation and its causal past.
fn static_refusal(signed: &Signed, by_id: &Index<'_>, past: &BTreeSet<&str>) -> Option<Refusal> {
    match &signed.op {
        Op::Request { from, to } if from == to => Some(Refusal::SelfRequest),
        Op::Request { .. } => None,
        Op::Accept { request, by } | Op::Decline { request, by } => {
            match requested(request, by_id, past) {
                None => Some(Refusal::UnknownRequest {
                    request: request.clone(),
                }),
                Some((_, to)) if to != by => Some(Refusal::NotRecipient {
                    request: request.clone(),
                    by: by.clone(),
                }),
                Some(_) => None,
            }
        }
        Op::Remove {
            by,
            other,
            requests,
        } => {
            if by == other {
                return Some(Refusal::SameAccount);
            }
            let this = pair(by, other);
            requests
                .iter()
                .find_map(|request| match requested(request, by_id, past) {
                    None => Some(Refusal::UnknownRequest {
                        request: request.clone(),
                    }),
                    Some((from, to)) if pair(from, to) != this => Some(Refusal::NotThisPair {
                        request: request.clone(),
                    }),
                    Some(_) => None,
                })
        }
    }
}

/// The friendship state that operations build.
#[derive(Default)]
struct State {
    /// Every request that took effect: its sender and recipient.
    requests: BTreeMap<OpId, (Account, Account)>,
    /// Requests still open.
    open: BTreeSet<OpId>,
    /// Friendships, each with the request that formed it.
    friends: BTreeMap<Pair, OpId>,
}

impl State {
    fn count(&self, account: &str) -> usize {
        self.friends
            .keys()
            .filter(|(a, b)| a == account || b == account)
            .count()
    }

    fn open_between(&self, a: &str, b: &str) -> bool {
        let p = pair(a, b);
        self.open.iter().any(|id| {
            self.requests
                .get(id)
                .is_some_and(|(from, to)| pair(from, to) == p)
        })
    }
}

/// Whether an operation takes effect in `state`.
fn check(state: &State, op: &Op, cap: usize) -> Result<(), Refusal> {
    match op {
        Op::Request { from, to } => {
            if state.friends.contains_key(&pair(from, to)) {
                Err(Refusal::AlreadyFriends)
            } else if state.open_between(from, to) {
                Err(Refusal::AlreadyOpen)
            } else {
                Ok(())
            }
        }
        Op::Accept { request, .. } => {
            let Some((from, to)) = state
                .requests
                .get(request)
                .filter(|_| state.open.contains(request))
            else {
                return Err(Refusal::NotOpen {
                    request: request.clone(),
                });
            };
            if state.friends.contains_key(&pair(from, to)) {
                return Err(Refusal::AlreadyFriends);
            }
            for account in [from, to] {
                if state.count(account) >= cap {
                    return Err(Refusal::AtCap {
                        account: account.clone(),
                        cap,
                    });
                }
            }
            Ok(())
        }
        Op::Decline { request, .. } => {
            if state.open.contains(request) {
                Ok(())
            } else {
                Err(Refusal::NotOpen {
                    request: request.clone(),
                })
            }
        }
        Op::Remove {
            by,
            other,
            requests,
        } => {
            let p = pair(by, other);
            if requests
                .iter()
                .any(|r| state.open.contains(r) || state.friends.get(&p) == Some(r))
            {
                Ok(())
            } else {
                Err(Refusal::NothingToRemove)
            }
        }
    }
}

/// Apply an operation that took effect.
fn effect(state: &mut State, signed: &Signed) {
    match &signed.op {
        Op::Request { from, to } => {
            state
                .requests
                .insert(signed.id.clone(), (from.clone(), to.clone()));
            state.open.insert(signed.id.clone());
        }
        Op::Accept { request, .. } => {
            if let Some((from, to)) = state.requests.get(request).cloned() {
                state.open.remove(request);
                state.friends.insert(pair(&from, &to), request.clone());
            }
        }
        Op::Decline { request, .. } => {
            state.open.remove(request);
        }
        Op::Remove {
            by,
            other,
            requests,
        } => {
            let p = pair(by, other);
            for request in requests {
                state.open.remove(request);
                if state.friends.get(&p) == Some(request) {
                    state.friends.remove(&p);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The cap for scenarios where it binds. Small, so that each boundary is reached.
    const TEST_CAP: usize = 2;

    const ACCOUNTS: [&str; 5] = ["ann", "bo", "cy", "di", "ed"];

    fn signed(id: &str, after: &[&str], op: Op) -> Signed {
        Signed {
            id: id.to_owned(),
            after: after.iter().map(|d| (*d).to_owned()).collect(),
            op,
        }
    }

    fn request(id: &str, after: &[&str], from: &str, to: &str) -> Signed {
        signed(
            id,
            after,
            Op::Request {
                from: from.to_owned(),
                to: to.to_owned(),
            },
        )
    }

    fn accept(id: &str, after: &[&str], request: &str, by: &str) -> Signed {
        signed(
            id,
            after,
            Op::Accept {
                request: request.to_owned(),
                by: by.to_owned(),
            },
        )
    }

    fn decline(id: &str, after: &[&str], request: &str, by: &str) -> Signed {
        signed(
            id,
            after,
            Op::Decline {
                request: request.to_owned(),
                by: by.to_owned(),
            },
        )
    }

    fn remove(id: &str, after: &[&str], by: &str, other: &str, requests: &[&str]) -> Signed {
        signed(
            id,
            after,
            Op::Remove {
                by: by.to_owned(),
                other: other.to_owned(),
                requests: requests.iter().map(|r| (*r).to_owned()).collect(),
            },
        )
    }

    /// Every order that respects the `after` sets. S2 ran every causal order this way.
    fn linearizations(ops: &[Signed]) -> Vec<Vec<Signed>> {
        fn extend(rest: &mut Vec<Signed>, done: &mut Vec<Signed>, out: &mut Vec<Vec<Signed>>) {
            if rest.is_empty() {
                out.push(done.clone());
                return;
            }
            for i in 0..rest.len() {
                if rest[i]
                    .after
                    .iter()
                    .all(|d| done.iter().any(|x| &x.id == d))
                {
                    let next = rest.remove(i);
                    done.push(next.clone());
                    extend(rest, done, out);
                    done.pop();
                    rest.insert(i, next);
                }
            }
        }
        let mut out = Vec::new();
        extend(&mut ops.to_vec(), &mut Vec::new(), &mut out);
        out
    }

    /// The projection of `ops`, which must be the same for every causal order.
    fn settled(ops: &[Signed], cap: usize) -> Projection {
        let expected = project(ops, cap).unwrap();
        for order in linearizations(ops) {
            assert_eq!(project(&order, cap).unwrap(), expected);
        }
        expected
    }

    fn applied(p: &Projection, id: &str) -> bool {
        p.outcome(id) == Some(&Outcome::Applied)
    }

    #[test]
    fn a_request_is_one_way_and_becomes_a_friendship_on_accept() {
        let open = settled(&[request("r0", &[], "ann", "bo")], TEST_CAP);
        assert!(!open.are_friends("ann", "bo"));
        assert!(applied(&open, "r0"));

        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "bo"),
        ];
        let friends = settled(&ops, TEST_CAP);
        assert!(friends.are_friends("bo", "ann"));
        assert_eq!(friends.friends_of("ann"), ["bo"]);
    }

    #[test]
    fn only_the_recipient_may_accept() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "ann"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert_eq!(
            p.outcome("a0"),
            Some(&Outcome::Refused(Refusal::NotRecipient {
                request: "r0".into(),
                by: "ann".into(),
            }))
        );
        assert!(!p.are_friends("ann", "bo"));
    }

    #[test]
    fn a_declined_request_is_void_and_its_later_accept_is_refused() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            decline("d0", &["r0"], "r0", "bo"),
            accept("a1", &["r0", "d0"], "r0", "bo"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert!(applied(&p, "d0"));
        assert_eq!(
            p.outcome("a1"),
            Some(&Outcome::Refused(Refusal::NotOpen {
                request: "r0".into(),
            }))
        );
        assert!(!p.are_friends("ann", "bo"));
    }

    #[test]
    fn a_request_to_oneself_is_refused() {
        let p = settled(&[request("r0", &[], "ann", "ann")], TEST_CAP);
        assert_eq!(
            p.outcome("r0"),
            Some(&Outcome::Refused(Refusal::SelfRequest))
        );
    }

    #[test]
    fn one_open_request_per_pair_in_either_direction() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            request("r1", &["r0"], "bo", "ann"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert_eq!(
            p.outcome("r1"),
            Some(&Outcome::Refused(Refusal::AlreadyOpen))
        );
    }

    #[test]
    fn no_request_between_friends() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "bo"),
            request("r1", &["a0"], "ann", "bo"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert_eq!(
            p.outcome("r1"),
            Some(&Outcome::Refused(Refusal::AlreadyFriends))
        );
    }

    #[test]
    fn an_accept_below_the_cap_is_applied_and_reaches_it() {
        // cap - 1: ann has one friend, so the second is applied and fills the cap.
        let ops = [
            request("r0", &[], "bo", "ann"),
            accept("a0", &["r0"], "r0", "ann"),
            request("r1", &["a0"], "cy", "ann"),
            accept("a1", &["r1"], "r1", "ann"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert!(applied(&p, "a1"));
        assert_eq!(p.friends_of("ann"), ["bo", "cy"]);
    }

    #[test]
    fn an_accept_at_the_cap_is_refused_with_the_account_named() {
        // cap: ann already has cap friends, so a third accept is refused.
        let ops = [
            request("r0", &[], "bo", "ann"),
            accept("a0", &["r0"], "r0", "ann"),
            request("r1", &["a0"], "cy", "ann"),
            accept("a1", &["r1"], "r1", "ann"),
            request("r2", &["a1"], "di", "ann"),
            accept("a2", &["r2"], "r2", "ann"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert_eq!(
            p.outcome("a2"),
            Some(&Outcome::Refused(Refusal::AtCap {
                account: "ann".into(),
                cap: TEST_CAP,
            }))
        );
        assert_eq!(p.friends_of("ann"), ["bo", "cy"]);
    }

    #[test]
    fn past_the_cap_nothing_more_is_added() {
        // cap + 1: a fourth and a fifth request stay open and are never friends.
        let ops = [
            request("r0", &[], "bo", "ann"),
            accept("a0", &["r0"], "r0", "ann"),
            request("r1", &["a0"], "cy", "ann"),
            accept("a1", &["r1"], "r1", "ann"),
            request("r2", &["a1"], "di", "ann"),
            accept("a2", &["r2"], "r2", "ann"),
            request("r3", &["a2"], "ed", "ann"),
            accept("a3", &["r3"], "r3", "ann"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert!(!applied(&p, "a3"));
        assert_eq!(p.friends_of("ann").len(), TEST_CAP);
    }

    #[test]
    fn the_requester_side_counts_toward_the_cap() {
        // bo is at the cap, so bo cannot make a friend of dan.
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "bo"),
            request("r1", &["a0"], "cy", "bo"),
            accept("a1", &["r1"], "r1", "bo"),
            request("r2", &["a1"], "bo", "di"),
            accept("a2", &["r2"], "r2", "di"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert_eq!(
            p.outcome("a2"),
            Some(&Outcome::Refused(Refusal::AtCap {
                account: "bo".into(),
                cap: TEST_CAP,
            }))
        );
    }

    #[test]
    fn an_accept_refused_at_the_cap_is_not_revived_when_a_friend_leaves() {
        // ann is at the cap with bo and cy. The accept of di is refused. When bo leaves,
        // di must not become a friend through that old accept.
        let ops = [
            request("r0", &[], "bo", "ann"),
            accept("a0", &["r0"], "r0", "ann"),
            request("r1", &["a0"], "cy", "ann"),
            accept("a1", &["r1"], "r1", "ann"),
            request("r2", &["a1"], "di", "ann"),
            accept("a2", &["r2"], "r2", "ann"),
            remove("x", &["a2"], "ann", "bo", &["r0"]),
        ];
        let p = settled(&ops, TEST_CAP);
        assert!(applied(&p, "x"));
        assert!(matches!(
            p.outcome("a2"),
            Some(Outcome::Refused(Refusal::AtCap { .. }))
        ));
        assert_eq!(p.friends_of("ann"), ["cy"]);
    }

    #[test]
    fn a_request_side_accept_refused_at_the_cap_is_not_revived_when_a_friend_leaves() {
        // bo is at the cap with ann and cy. The accept of di, which bo requested, is
        // refused. When ann leaves, di must not become a friend of bo.
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "bo"),
            request("r1", &["a0"], "cy", "bo"),
            accept("a1", &["r1"], "r1", "bo"),
            request("r2", &["a1"], "bo", "di"),
            accept("a2", &["r2"], "r2", "di"),
            remove("x", &["a2"], "bo", "ann", &["r0"]),
        ];
        let p = settled(&ops, TEST_CAP);
        assert!(applied(&p, "x"));
        assert!(matches!(
            p.outcome("a2"),
            Some(Outcome::Refused(Refusal::AtCap { .. }))
        ));
        assert_eq!(p.friends_of("bo"), ["cy"]);
        assert!(p.friends_of("di").is_empty());
    }

    #[test]
    fn a_removal_dissolves_the_friendship_on_both_sides() {
        for (by, other) in [("ann", "bo"), ("bo", "ann")] {
            let ops = [
                request("r0", &[], "ann", "bo"),
                accept("a0", &["r0"], "r0", "bo"),
                remove("x", &["a0"], by, other, &["r0"]),
            ];
            let p = settled(&ops, TEST_CAP);
            assert!(applied(&p, "x"));
            assert!(!p.are_friends("ann", "bo"));
            assert!(p.friends_of("bo").is_empty());
            assert!(p.friends_of("ann").is_empty());
        }
    }

    #[test]
    fn a_removal_voids_a_request_that_was_never_delivered() {
        // The chain: bo accepts after the removal, so the request is already gone.
        let chained = [
            request("r0", &[], "ann", "bo"),
            remove("x", &["r0"], "ann", "bo", &["r0"]),
            accept("a0", &["x"], "r0", "bo"),
        ];
        let p = settled(&chained, TEST_CAP);
        assert_eq!(
            p.outcome("a0"),
            Some(&Outcome::Refused(Refusal::NotOpen {
                request: "r0".into(),
            }))
        );

        // The concurrent case: bo accepted without seeing the removal. The removal wins.
        let concurrent = [
            request("r0", &[], "ann", "bo"),
            remove("x", &["r0"], "ann", "bo", &["r0"]),
            accept("a0", &["r0"], "r0", "bo"),
        ];
        let p = settled(&concurrent, TEST_CAP);
        assert!(applied(&p, "x"));
        assert_eq!(
            p.outcome("a0"),
            Some(&Outcome::Refused(Refusal::Dissolved {
                request: "r0".into(),
            }))
        );
        assert!(!p.are_friends("ann", "bo"));
    }

    #[test]
    fn a_removal_with_nothing_to_remove_is_refused() {
        let p = settled(&[remove("x", &[], "ann", "bo", &[])], TEST_CAP);
        assert_eq!(
            p.outcome("x"),
            Some(&Outcome::Refused(Refusal::NothingToRemove))
        );
    }

    #[test]
    fn a_removal_naming_a_request_between_other_accounts_is_refused() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            request("r1", &[], "cy", "di"),
            remove("x", &["r0", "r1"], "ann", "bo", &["r1"]),
        ];
        let p = settled(&ops, TEST_CAP);
        assert_eq!(
            p.outcome("x"),
            Some(&Outcome::Refused(Refusal::NotThisPair {
                request: "r1".into(),
            }))
        );
    }

    #[test]
    fn an_accept_of_a_request_outside_its_causal_past_is_refused() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &[], "r0", "bo"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert_eq!(
            p.outcome("a0"),
            Some(&Outcome::Refused(Refusal::UnknownRequest {
                request: "r0".into(),
            }))
        );
    }

    #[test]
    fn a_missing_dependency_a_duplicate_and_a_cycle_are_errors() {
        assert_eq!(
            project(&[request("r0", &["ghost"], "ann", "bo")], TEST_CAP).unwrap_err(),
            ProjectError::MissingDependency {
                op: "r0".into(),
                dependency: "ghost".into(),
            }
        );
        assert_eq!(
            project(
                &[
                    request("r0", &[], "ann", "bo"),
                    request("r0", &[], "ann", "cy")
                ],
                TEST_CAP
            )
            .unwrap_err(),
            ProjectError::DuplicateId("r0".into())
        );
        assert_eq!(
            project(
                &[
                    request("a", &["b"], "ann", "bo"),
                    request("b", &["a"], "ann", "cy")
                ],
                TEST_CAP
            )
            .unwrap_err(),
            ProjectError::Cycle
        );
    }

    // ----- S2 friendship findings (spikes/shared/s2-auth, friendship.rs and RESULTS.md) -----

    /// F2: two concurrent adds beyond the cap. Every order keeps the same two friends,
    /// and the later add is refused at the cap.
    #[test]
    fn s2_f2_concurrent_adds_beyond_the_cap_resolve_the_same_in_every_order() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "bo"),
            request("r1", &["a0"], "cy", "ann"),
            request("r2", &["a0"], "di", "ann"),
            accept("a1", &["r1"], "r1", "ann"),
            accept("a2", &["r2"], "r2", "ann"),
        ];
        let p = settled(&ops, TEST_CAP);
        assert_eq!(p.friends_of("ann"), ["bo", "cy"]);
        assert_eq!(
            p.outcome("a2"),
            Some(&Outcome::Refused(Refusal::AtCap {
                account: "ann".into(),
                cap: TEST_CAP,
            }))
        );
    }

    /// F3: the concurrent adds, and a removal of bo. The removal dissolves bo's
    /// friendship, and the freed place goes to the two new friends.
    #[test]
    fn s2_f3_concurrent_adds_and_a_removal_of_one_side_dissolve_in_every_order() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "bo"),
            request("r1", &["a0"], "cy", "ann"),
            request("r2", &["a0"], "di", "ann"),
            accept("a1", &["r1"], "r1", "ann"),
            accept("a2", &["r2"], "r2", "ann"),
            remove("x", &["a0"], "ann", "bo", &["r0"]),
        ];
        let p = settled(&ops, TEST_CAP);
        assert!(applied(&p, "x"));
        assert_eq!(
            p.outcome("a0"),
            Some(&Outcome::Refused(Refusal::Dissolved {
                request: "r0".into(),
            }))
        );
        assert_eq!(p.friends_of("ann"), ["cy", "di"]);
        assert!(p.friends_of("bo").is_empty());
    }

    /// F4: mutual removal. Both removals apply, and the friendship ends once.
    #[test]
    fn s2_f4_mutual_removal_dissolves_once_and_stays_dissolved() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "bo"),
            remove("x", &["a0"], "ann", "bo", &["r0"]),
            remove("y", &["a0"], "bo", "ann", &["r0"]),
        ];
        let p = settled(&ops, TEST_CAP);
        assert!(applied(&p, "x"));
        assert!(applied(&p, "y"));
        assert!(p.friendships().is_empty());
    }

    /// F5: after a dissolve, the same request and its accept do not bring the
    /// friendship back, in any order. Only a new request does.
    #[test]
    fn s2_f5_a_dissolved_friendship_is_not_revived_by_its_own_accept() {
        let concurrent = [
            request("r0", &[], "ann", "bo"),
            remove("x", &["r0"], "ann", "bo", &["r0"]),
            accept("a0", &["r0"], "r0", "bo"),
        ];
        let p = settled(&concurrent, TEST_CAP);
        assert!(p.friendships().is_empty());
        assert_eq!(
            p.outcome("a0"),
            Some(&Outcome::Refused(Refusal::Dissolved {
                request: "r0".into(),
            }))
        );
    }

    /// F5, the other half: a new request after a dissolve is open, and accepting it is
    /// a new friendship. This is the product's rule, and it differs from the S2 group
    /// model, where dissolution was terminal. The owner decides which one holds.
    #[test]
    fn s2_f5_a_new_request_after_a_dissolve_is_the_only_way_back() {
        let ops = [
            request("r0", &[], "ann", "bo"),
            accept("a0", &["r0"], "r0", "bo"),
            remove("x", &["a0"], "ann", "bo", &["r0"]),
            request("r2", &["x"], "ann", "bo"),
        ];
        let open = settled(&ops, TEST_CAP);
        assert!(applied(&open, "r2"));
        assert!(!open.are_friends("ann", "bo"));

        let mut again = ops.to_vec();
        again.push(accept("a3", &["r2"], "r2", "bo"));
        assert!(settled(&again, TEST_CAP).are_friends("ann", "bo"));
    }

    // ----- Properties over random operation sets -----

    fn script() -> impl Strategy<Value = Vec<(u8, u8, u8, u16, u16)>> {
        proptest::collection::vec((0u8..4, 0u8..5, 0u8..5, any::<u16>(), any::<u16>()), 1..=10)
    }

    /// Build a set from a script. Each operation sees a subset of the earlier ones, so every
    /// generated set is a DAG.
    fn build(script: &[(u8, u8, u8, u16, u16)]) -> Vec<Signed> {
        let mut ops: Vec<Signed> = Vec::new();
        for (i, &(kind, a, b, pick, seen)) in script.iter().enumerate() {
            let me = ACCOUNTS[usize::from(a) % ACCOUNTS.len()];
            let other = ACCOUNTS[usize::from(b) % ACCOUNTS.len()];
            let id = format!("{pick:04x}-{i:02}");
            let mut after: BTreeSet<OpId> = BTreeSet::new();
            for (j, earlier) in ops.iter().enumerate() {
                if (seen >> j) & 1 == 1 {
                    after.insert(earlier.id.clone());
                }
            }
            let mut open: Vec<(OpId, Account, Account)> = Vec::new();
            for s in &ops {
                if let Op::Request { from, to } = &s.op {
                    open.push((s.id.clone(), from.clone(), to.clone()));
                }
            }
            let op = match kind {
                1 | 2 if !open.is_empty() => {
                    let (request, _from, to) = open[usize::from(pick) % open.len()].clone();
                    after.insert(request.clone());
                    // A wrong recipient sometimes, so the static refusal is exercised.
                    let by = if seen % 7 == 0 { other.to_owned() } else { to };
                    if kind == 1 {
                        Op::Accept { request, by }
                    } else {
                        Op::Decline { request, by }
                    }
                }
                3 => {
                    let mut named = Vec::new();
                    for (k, (request, from, to)) in open.iter().enumerate() {
                        if pair(from, to) == pair(me, other) && (pick >> (k % 16)) & 1 == 1 {
                            named.push(request.clone());
                        }
                    }
                    after.extend(named.iter().cloned());
                    Op::Remove {
                        by: me.to_owned(),
                        other: other.to_owned(),
                        requests: named,
                    }
                }
                _ => Op::Request {
                    from: me.to_owned(),
                    to: other.to_owned(),
                },
            };
            ops.push(Signed { id, after, op });
        }
        ops
    }

    /// A random order that respects the `after` sets. `prio` picks among the ready operations.
    fn linearize(ops: &[Signed], prio: &[u16]) -> Vec<Signed> {
        let mut rest: Vec<&Signed> = ops.iter().collect();
        let mut done: BTreeSet<&str> = BTreeSet::new();
        let mut out = Vec::new();
        while !rest.is_empty() {
            let ready: Vec<usize> = (0..rest.len())
                .filter(|&i| rest[i].after.iter().all(|d| done.contains(d.as_str())))
                .collect();
            if ready.is_empty() {
                break;
            }
            let next = rest.remove(ready[usize::from(prio[out.len() % prio.len()]) % ready.len()]);
            done.insert(next.id.as_str());
            out.push(next.clone());
        }
        out
    }

    fn requested_pair(ops: &[Signed], request: &str) -> Option<Pair> {
        ops.iter().find_map(|s| match &s.op {
            Op::Request { from, to } if s.id == request => Some(pair(from, to)),
            _ => None,
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn the_cap_is_never_exceeded(script in script(), cap in 1usize..=3) {
            let ops = build(&script);
            let p = project(&ops, cap).unwrap();
            for account in ACCOUNTS {
                let friends = p.friends_of(account).len();
                prop_assert!(friends <= cap, "{account} has {friends} friends, cap {cap}");
            }
        }

        #[test]
        fn the_result_does_not_depend_on_the_order(
            script in script(),
            cap in 1usize..=3,
            prio in proptest::collection::vec(any::<u16>(), 12),
        ) {
            let ops = build(&script);
            let expected = project(&ops, cap).unwrap();
            let shuffled = linearize(&ops, &prio);
            prop_assert_eq!(shuffled.len(), ops.len());
            prop_assert_eq!(project(&shuffled, cap).unwrap(), expected.clone());
            let mut reversed = ops.clone();
            reversed.reverse();
            prop_assert_eq!(project(&reversed, cap).unwrap(), expected);
        }

        #[test]
        fn every_friendship_is_formed_by_an_applied_accept(script in script(), cap in 1usize..=3) {
            let ops = build(&script);
            let p = project(&ops, cap).unwrap();
            for (a, b) in p.friendships() {
                let formed = ops.iter().any(|s| match &s.op {
                    Op::Accept { request, .. } => {
                        applied(&p, &s.id)
                            && requested_pair(&ops, request) == Some((a.clone(), b.clone()))
                    }
                    _ => false,
                });
                prop_assert!(formed, "{a} and {b} are friends with no applied accept");
            }
        }
    }
}
