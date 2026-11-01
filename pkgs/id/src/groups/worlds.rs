//! Cross-world authority (P2.7, task 03 item 7): each world judges its own actions by its own
//! membership and delegations, and a grant that names another world is the only way one world's
//! authority reaches into another.
//!
//! The design (§7.5 and the two cross-world docs) says the rules "overlap with delegation" and must
//! be reconciled, but it does not say how a grant names a world. These are the choices, taken as
//! the more restrictive reading:
//!
//! - **Per-world judgement.** Each world's events are folded on their own ([`Worlds::fold`]). A
//!   removal or a grant in one world cannot change another world's fold, so a removal in A changes
//!   B only through a grant that names B.
//! - **Local authority.** An action in world B at scope S is allowed by B's own membership, or by a
//!   grant in B's own chains. Grants that name another world do not count here.
//! - **Crossings.** A [`Body::Cross`] grant is made in its home world, where it is judged like any
//!   grant. It names a target world and carries at most Write. It is never a link in its home
//!   world's chains, so it gives nothing there and cannot be passed on. It stands while its grantor
//!   holds the scope at home (cascade), and while its grantee still holds at least that scope there
//!   at the head: removing the grantee ends its reach into the target.
//! - **Markers are local.** A world's [`Markers`] decide two things, and only for that world. The
//!   home world's isolation decides whether its members may make crossings at all. The target's
//!   isolation and cross-world write setting decide whether it accepts a crossing: a read needs
//!   both unisolated, and a write also needs the target's setting, as `world_hub::cross_world_allowed`
//!   says. No world reads another world's marker to judge its own members.
//! - **Scope.** A crossing reaches the target only at the scope it grants or below.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::fold::{FoldError, Projection, fold};
use super::{Event, Grant, Key, Role};

/// Whether a world's directory may be read from other worlds.
///
/// Writes never cross worlds, whatever the setting.
///
/// Lives here (rather than in `crate::world`) so the cross-world authority
/// model — and the `crate::blob_gate` access checks built on it — compile
/// without the `world` feature. `crate::world` re-exports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    /// No cross-world reads or writes, in either direction.
    #[default]
    Isolated,
    /// Readable from other unisolated worlds, and only from them.
    Unisolated,
}

impl Isolation {
    /// The wire word: `isolated` or `unisolated`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Isolated => "isolated",
            Self::Unisolated => "unisolated",
        }
    }
}

/// A world's name, as `WorldCore::world_id` names it.
pub type World = String;

/// A world's isolation markers. The defaults are the defaults of a world with no record: isolated,
/// and no cross-world write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Markers {
    /// Whether the world takes part in cross-world access.
    pub isolation: Isolation,
    /// Whether the world accepts cross-world writes.
    pub cross_write: bool,
}

/// Every world's events, folded world by world, with each world's markers.
#[derive(Debug)]
pub struct Worlds {
    folds: BTreeMap<World, Projection>,
    markers: BTreeMap<World, Markers>,
}

impl Worlds {
    /// No worlds at all: the fold of nothing, for callers that must fail closed
    /// (an empty gate) rather than propagate a fold error.
    pub fn empty() -> Self {
        Self {
            folds: BTreeMap::new(),
            markers: BTreeMap::new(),
        }
    }

    /// Folds each world's events on their own.
    ///
    /// # Errors
    ///
    /// Any error [`fold`] gives for one world.
    pub fn fold(
        events: &BTreeMap<World, Vec<Event>>,
        markers: BTreeMap<World, Markers>,
    ) -> Result<Self, FoldError> {
        let folds = events
            .iter()
            .map(|(world, events)| Ok((world.clone(), fold(events)?)))
            .collect::<Result<_, FoldError>>()?;
        Ok(Self { folds, markers })
    }

    /// The folded state of one world.
    pub fn projection(&self, world: &str) -> Option<&Projection> {
        self.folds.get(world)
    }

    /// Whether `actor` may act in `world` at `scope`: by the world's own authority, or through a
    /// crossing that names the world and is accepted there.
    pub fn may_act(&self, world: &str, actor: Key, scope: Role) -> bool {
        self.holds_here(world, actor, scope)
            || (self.accepts(world, scope)
                && self
                    .crossings_into(world)
                    .into_iter()
                    .any(|grant| grant.to == actor && grant.scope >= scope))
    }

    /// Whether the world's own membership, or a grant in its own chains, gives `actor` the scope.
    pub fn holds_here(&self, world: &str, actor: Key, scope: Role) -> bool {
        self.projection(world).is_some_and(|projection| {
            projection.groups().any(|(_, group)| {
                group.is_active() && group.members.get(&actor).is_some_and(|role| *role >= scope)
            }) || projection
                .grants_in_force()
                .any(|grant| grant.target.is_none() && grant.to == actor && grant.scope >= scope)
        })
    }

    /// Whether `world` accepts a crossing at `scope`: unisolated, and for a write also the write
    /// setting. This is the target's half of `world_hub::cross_world_allowed`.
    fn accepts(&self, world: &str, scope: Role) -> bool {
        let markers = self.markers(world);
        markers.isolation == Isolation::Unisolated && (scope < Role::Write || markers.cross_write)
    }

    /// The crossings that name `world`, in force in their home worlds. A home world that is
    /// isolated makes none. The grantee must hold the crossing's scope at home now.
    fn crossings_into(&self, world: &str) -> Vec<&Grant> {
        self.folds
            .iter()
            .filter(|(home, _)| home.as_str() != world)
            .filter(|(home, _)| self.markers(home).isolation == Isolation::Unisolated)
            .flat_map(|(_, projection)| {
                projection.grants_in_force().filter(|grant| {
                    grant.target.as_deref() == Some(world)
                        && projection.groups().any(|(_, group)| {
                            group.is_active()
                                && group
                                    .members
                                    .get(&grant.to)
                                    .is_some_and(|role| *role >= grant.scope)
                        })
                })
            })
            .collect()
    }

    /// The highest scope each of `keys` may use in `world`. Equal maps mean equal authority.
    pub fn authority(&self, world: &str, keys: &[Key]) -> BTreeMap<Key, Role> {
        const SCOPES: [Role; 5] = [
            Role::Pull,
            Role::Read,
            Role::Write,
            Role::Manage,
            Role::Admin,
        ];
        keys.iter()
            .filter_map(|&key| {
                SCOPES
                    .iter()
                    .rev()
                    .find(|&&scope| self.may_act(world, key, scope))
                    .map(|&scope| (key, scope))
            })
            .collect()
    }

    /// The markers of one world. A world with no record has the defaults.
    pub fn markers(&self, world: &str) -> Markers {
        self.markers.get(world).copied().unwrap_or_default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;

    use p2panda_core::Hash;
    use proptest::prelude::*;

    use super::{Isolation, Markers, Worlds};
    use crate::groups::policy::Policy;
    use crate::groups::{
        Body, Create, Delegate, Event, Key, MemberOp, Outcome, Refusal, Role, Template,
    };
    use Isolation::{Isolated, Unisolated};

    /// The accounts the tests use. Account 1 founds each world and is its Admin.
    const KEYS: [Key; 5] = [[1; 32], [2; 32], [3; 32], [4; 32], [5; 32]];

    fn key(n: u8) -> Key {
        [n; 32]
    }

    fn hid(name: &str) -> Hash {
        Hash::digest(name.as_bytes())
    }

    /// Builds events world by world. Each event names the one before it in its world, so a
    /// world is a chain and the only thing a test varies is the order the events arrive in.
    #[derive(Clone, Default)]
    struct Ledger {
        events: BTreeMap<String, Vec<Event>>,
        last: BTreeMap<String, Hash>,
        groups: BTreeMap<String, String>,
        count: usize,
    }

    impl Ledger {
        fn push(&mut self, world: &str, author: u8, group: Option<&str>, body: Body) -> String {
            self.count += 1;
            let name = format!("{world}{}", self.count);
            let event = Event {
                id: hid(&name),
                author: key(author),
                after: self.last.get(world).copied().into_iter().collect(),
                group: group.map(hid),
                body,
            };
            self.last.insert(world.to_owned(), event.id);
            self.events.entry(world.to_owned()).or_default().push(event);
            name
        }

        /// Founds the world's group. Its creator is Admin.
        fn create(&mut self, world: &str, founder: u8) {
            let body = Body::Create(Create {
                template: Template::Custom,
                policy: Policy::default(),
                members: Vec::new(),
                admins_on_create: false,
            });
            let name = self.push(world, founder, None, body);
            self.groups.insert(world.to_owned(), name);
        }

        fn group(&self, world: &str) -> String {
            self.groups[world].clone()
        }

        fn add(&mut self, world: &str, by: u8, who: u8, role: Role) -> String {
            let group = self.group(world);
            let body = Body::Member(MemberOp::Add {
                member: key(who),
                role,
            });
            self.push(world, by, Some(&group), body)
        }

        fn remove(&mut self, world: &str, by: u8, who: u8) -> String {
            let group = self.group(world);
            let body = Body::Member(MemberOp::Remove {
                member: key(who),
                successor: None,
            });
            self.push(world, by, Some(&group), body)
        }

        fn delegate(
            &mut self,
            world: &str,
            by: u8,
            to: u8,
            scope: Role,
            redelegate: bool,
        ) -> String {
            let group = self.group(world);
            let body = Body::Delegate(Delegate {
                grantee: key(to),
                scope,
                can_redelegate: redelegate,
            });
            self.push(world, by, Some(&group), body)
        }

        /// A grant from `by` in `world` that names `names` as the world it is used in.
        fn cross(
            &mut self,
            world: &str,
            by: u8,
            to: u8,
            scope: Role,
            redelegate: bool,
            names: &str,
        ) -> String {
            let group = self.group(world);
            let body = Body::Cross {
                delegate: Delegate {
                    grantee: key(to),
                    scope,
                    can_redelegate: redelegate,
                },
                world: names.to_owned(),
            };
            self.push(world, by, Some(&group), body)
        }

        fn worlds(&self, markers: &BTreeMap<String, Markers>) -> Worlds {
            Worlds::fold(&self.events, markers.clone()).unwrap()
        }
    }

    /// Both worlds unisolated, each accepting cross-world writes.
    fn open() -> BTreeMap<String, Markers> {
        markers(&[("a", Unisolated, true), ("b", Unisolated, true)])
    }

    fn markers(entries: &[(&str, Isolation, bool)]) -> BTreeMap<String, Markers> {
        entries
            .iter()
            .map(|&(world, isolation, cross_write)| {
                (
                    world.to_owned(),
                    Markers {
                        isolation,
                        cross_write,
                    },
                )
            })
            .collect()
    }

    /// World `a`: founder 1 (Admin), manager 2, writer 3, reader 4. World `b`: founder 1 and writer
    /// 5. Account 3 has authority in `a` only.
    fn two_worlds() -> Ledger {
        let mut ledger = Ledger::default();
        ledger.create("a", 1);
        ledger.add("a", 1, 2, Role::Manage);
        ledger.add("a", 1, 3, Role::Write);
        ledger.add("a", 1, 4, Role::Read);
        ledger.create("b", 1);
        ledger.add("b", 1, 5, Role::Write);
        ledger
    }

    // ----- Cross-world rules (P2.7) -----

    #[test]
    fn test_a_member_whose_authority_is_only_in_another_world_is_refused_here() {
        let worlds = two_worlds().worlds(&open());
        assert!(worlds.may_act("a", key(3), Role::Write));
        assert!(!worlds.may_act("b", key(3), Role::Read));
        assert!(!worlds.may_act("b", key(4), Role::Pull));
    }

    #[test]
    fn test_a_grant_that_names_the_world_lets_the_grantee_act_there_within_its_scope() {
        let mut ledger = two_worlds();
        ledger.cross("a", 2, 3, Role::Write, false, "b");
        let worlds = ledger.worlds(&open());
        assert!(worlds.may_act("b", key(3), Role::Read));
        assert!(worlds.may_act("b", key(3), Role::Write));
        assert!(!worlds.may_act("b", key(3), Role::Manage));
        assert!(!worlds.may_act("b", key(4), Role::Read));
    }

    #[test]
    fn test_a_crossing_needs_both_worlds_unisolated_and_the_target_setting_for_writes() {
        for a in [Isolated, Unisolated] {
            for b in [Isolated, Unisolated] {
                for cross_write in [false, true] {
                    let mut ledger = two_worlds();
                    ledger.cross("a", 2, 3, Role::Write, false, "b");
                    let worlds = ledger.worlds(&markers(&[("a", a, true), ("b", b, cross_write)]));
                    let both = a == Unisolated && b == Unisolated;
                    assert_eq!(
                        worlds.may_act("b", key(3), Role::Read),
                        both,
                        "read with a {a:?}, b {b:?}"
                    );
                    assert_eq!(
                        worlds.may_act("b", key(3), Role::Write),
                        both && cross_write,
                        "write with a {a:?}, b {b:?}, cross_write {cross_write}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_a_removal_in_the_home_world_changes_the_other_world_only_through_the_grant() {
        let mut base = two_worlds();
        base.cross("a", 2, 3, Role::Write, false, "b");
        let keys = KEYS;

        let mut unrelated = base.clone();
        unrelated.remove("a", 1, 4);
        assert_eq!(
            base.worlds(&open()).authority("b", &keys),
            unrelated.worlds(&open()).authority("b", &keys),
            "removing an account with no grant into b must not change b"
        );

        assert!(
            base.worlds(&open()).may_act("b", key(3), Role::Read),
            "the grant names b and is in force before the removal"
        );
        let mut grantor = base.clone();
        grantor.remove("a", 1, 2);
        let after = grantor.worlds(&open());
        assert!(
            !after.may_act("b", key(3), Role::Read),
            "removing the grantor ends the grant that names b"
        );
        assert!(
            after.may_act("b", key(5), Role::Write),
            "b's own members are not touched by a removal in a"
        );
    }

    #[test]
    fn test_removing_the_grantee_at_home_ends_its_crossing() {
        let mut ledger = two_worlds();
        ledger.cross("a", 2, 3, Role::Write, false, "b");
        assert!(ledger.worlds(&open()).may_act("b", key(3), Role::Read));
        ledger.remove("a", 1, 3);
        let worlds = ledger.worlds(&open());
        assert!(!worlds.may_act("b", key(3), Role::Read));
    }

    #[test]
    fn test_isolation_markers_are_local_to_their_own_world() {
        let mut ledger = two_worlds();
        ledger.cross("a", 2, 3, Role::Write, false, "b");

        // Isolating a stops a's grants out. b's own members are unaffected.
        let a_isolated = ledger.worlds(&markers(&[("a", Isolated, true), ("b", Unisolated, true)]));
        assert!(a_isolated.may_act("b", key(5), Role::Write));
        assert!(!a_isolated.may_act("b", key(3), Role::Write));

        // Isolating b stops b from taking grants in. b's own members keep working.
        let b_isolated = ledger.worlds(&markers(&[("a", Unisolated, true), ("b", Isolated, true)]));
        assert!(b_isolated.may_act("b", key(5), Role::Write));
        assert!(!b_isolated.may_act("b", key(3), Role::Read));

        // A marker never changes b's own membership: b's founder and writer keep their scopes.
        let own = BTreeMap::from([(key(1), Role::Admin), (key(5), Role::Write)]);
        let plain = ledger.worlds(&markers(&[("a", Isolated, true), ("b", Isolated, true)]));
        assert_eq!(plain.authority("b", &KEYS), own);
        assert_eq!(a_isolated.authority("b", &KEYS), own);
        assert_eq!(b_isolated.authority("b", &KEYS), own);
    }

    #[test]
    fn test_a_crossing_gives_nothing_in_its_home_world_and_cannot_be_passed_on() {
        let mut ledger = two_worlds();
        ledger.cross("a", 2, 3, Role::Write, true, "b");
        let pass_home = ledger.delegate("a", 3, 4, Role::Read, false);
        let pass_on = ledger.cross("a", 3, 4, Role::Read, false, "b");
        let worlds = ledger.worlds(&open());
        let home = worlds.projection("a").unwrap();
        assert_eq!(
            home.outcome(&hid(&pass_home)),
            Some(&Outcome::Refused(Refusal::NoChain)),
            "a crossing is not a link in the home world's chains"
        );
        assert_eq!(
            home.outcome(&hid(&pass_on)),
            Some(&Outcome::Refused(Refusal::NoChain)),
            "a crossing cannot be passed on into another world"
        );
        assert!(!worlds.may_act("b", key(4), Role::Read));
        assert!(!worlds.may_act("a", key(3), Role::Manage));
    }

    #[test]
    fn test_a_crossing_above_write_is_refused() {
        let mut ledger = two_worlds();
        let name = ledger.cross("a", 2, 3, Role::Manage, false, "b");
        let worlds = ledger.worlds(&open());
        assert_eq!(
            worlds.projection("a").unwrap().outcome(&hid(&name)),
            Some(&Outcome::Refused(Refusal::NotCrossable))
        );
        assert!(!worlds.may_act("b", key(3), Role::Manage));
    }

    // ----- Order independence (P2.7) -----

    /// A deterministic shuffle, so a failing case can be replayed from its seed.
    fn shuffled<T: Clone>(items: &[T], mut seed: u64) -> Vec<T> {
        let mut out = items.to_vec();
        for i in (1..out.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let j = usize::try_from(seed % (i as u64 + 1)).unwrap();
            out.swap(i, j);
        }
        out
    }

    /// Each step is (world, kind, who, scope, to, redelegate). Kinds: add or remove by the founder,
    /// a cross-world grant, or a grant inside the world.
    fn script() -> impl Strategy<Value = Vec<(u8, u8, u8, u8, u8, u8)>> {
        prop::collection::vec((0u8..2, 0u8..4, 2u8..=5, 0u8..4, 2u8..=5, 0u8..2), 1..30)
    }

    fn build(script: &[(u8, u8, u8, u8, u8, u8)]) -> Ledger {
        const ROLES: [Role; 4] = [Role::Pull, Role::Read, Role::Write, Role::Manage];
        let mut ledger = Ledger::default();
        ledger.create("a", 1);
        ledger.create("b", 1);
        for &(world, kind, who, scope, to, redelegate) in script {
            let (world, other) = if world == 0 { ("a", "b") } else { ("b", "a") };
            let role = ROLES[usize::from(scope)];
            let redelegate = redelegate == 1;
            match kind {
                0 => {
                    ledger.add(world, 1, who, role);
                }
                1 => {
                    ledger.remove(world, 1, who);
                }
                2 => {
                    ledger.cross(world, who, to, role, redelegate, other);
                }
                _ => {
                    ledger.delegate(world, who, to, role, redelegate);
                }
            }
        }
        ledger
    }

    fn arrive(events: &BTreeMap<String, Vec<Event>>, seed: u64) -> BTreeMap<String, Vec<Event>> {
        events
            .iter()
            .map(|(world, events)| (world.clone(), shuffled(events, seed.max(1))))
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn prop_the_authority_of_each_world_is_the_same_in_every_order(
            script in script(),
            seed in any::<u64>(),
        ) {
            let ledger = build(&script);
            let expected = ledger.worlds(&open());
            let in_order = arrive(&ledger.events, seed);
            let shuffled_worlds = Worlds::fold(&in_order, open()).unwrap();
            let mut reversed = ledger.events.clone();
            for events in reversed.values_mut() {
                events.reverse();
            }
            let reversed_worlds = Worlds::fold(&reversed, open()).unwrap();
            for world in ["a", "b"] {
                let want = expected.authority(world, &KEYS);
                prop_assert_eq!(&shuffled_worlds.authority(world, &KEYS), &want);
                prop_assert_eq!(&reversed_worlds.authority(world, &KEYS), &want);
            }
        }
    }
}
