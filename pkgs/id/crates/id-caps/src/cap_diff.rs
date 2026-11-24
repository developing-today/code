//! The capability-change comparison — #5999 §6's contract, consumed by
//! the change notice of #5959 (implementation issue #12921).
//!
//! The comparison is over user-meaningful capability **classes** (the
//! vocabulary in [`crate::cap_class`]), never raw capability names:
//!
//! ```text
//! classes(S)   = { class(n) : n ∈ S }              // unknown names → `unknown`
//! messages(S)  = maximal( classes(S) \ {cap.none} ) // maximal under ≼
//! N            = messages(requested ∪ platform_bound)
//! trigger      = ∃ c ∈ N : ¬(∃ b ∈ B : c ≼ b)
//! new_classes  = { c ∈ N : ¬(∃ b ∈ B : c ≼ b) }    // exactly what the notice names
//! ```
//!
//! `B` is the accepted baseline — the message set the user last
//! explicitly approved for a world, stored by
//! [`crate::cap_baseline`]. The trigger is a pure function of
//! `(B, N, ≼, table_version)`: no clocks, no history, no heuristics
//! (#5999 §6 givens). Fail-closed throughout (#5999 §7 invariant 4 and
//! 10): an unknown name or class id resolves to the `unknown` class,
//! which nothing covers, and a comparison that cannot be computed at
//! all is a trigger carrying `unknown` ([`uncomputable_diff`]) — the
//! connect stage that decides uncomputability lives in #12921's
//! follow-on slice; this module is the pure core it calls.
//!
//! Optional capabilities are not authority the world holds on connect
//! (#5999 §4 rule 6): names the registry entry marks optional are
//! excluded before classes are computed. Used-set changes never enter
//! the comparison at all (#5999 §5): the declared set is compared, the
//! used set is only shown.

use std::collections::BTreeSet;

use crate::cap_class::{CLASS_FS_READ_ARBITRARY, CLASS_FS_SANDBOX, CLASS_NET_OUT, CLASS_NONE};
use crate::world_caps::{ProviderRegistry, is_sensor_cap};

/// The subsumption order of #5999 §3.1: `subsumes(covering, covered)`
/// is `covered ≼ covering` — accepting `covering` means accepting
/// `covered`.
///
/// The declared facts, and nothing else:
///
/// - `c ≼ c` for every class (reflexivity);
/// - `cap.none ≼ c` for every class `c` — a messageless capability is
///   implied by anything (in practice `cap.none` is dropped from
///   message sets before the comparison; the relation is stated for
///   completeness of the lattice);
/// - `fs.sandbox ≼ fs.read.arbitrary`;
/// - `net.host.<name> ≼ net.out` for every per-host class.
///
/// In particular `unknown` is subsumed by nothing but itself and
/// subsumes nothing but itself (#5999 §2.4), and collapse is never
/// inferred from a shared prefix, a shared `RequiredScope`, or a
/// shared `ProviderKind` (#5999 §3.1, §7 invariant 7): `chat.read`
/// and `chat.write` are incomparable, as are `device.keyboard` and
/// `device.screen`.
#[must_use]
pub fn subsumes(covering: &str, covered: &str) -> bool {
    if covered == covering {
        return true;
    }
    if covered == CLASS_NONE {
        return true;
    }
    match (covered, covering) {
        (CLASS_FS_SANDBOX, CLASS_FS_READ_ARBITRARY) => true,
        (narrow, CLASS_NET_OUT) => {
            narrow.len() > "net.host.".len() && narrow.starts_with("net.host.")
        }
        _ => false,
    }
}

/// Whether any class in `baseline` covers `class` under [`subsumes`]
/// — the `∃ b ∈ B : class ≼ b` of #5999 §6.
#[must_use]
pub fn covered_by(baseline: &BTreeSet<&str>, class: &str) -> bool {
    baseline.iter().any(|b| subsumes(b, class))
}

/// The maximal classes of a class set under the subsumption order.
///
/// A class drops out when a broader class in the same set subsumes
/// it (#5999 §3.2 rule 5). This maximalisation is what implements
/// the collapse rule — within one class, membership is invisible,
/// and a broader accepted class covers a narrower new one.
#[must_use]
pub fn maximal_classes<'a>(classes: &BTreeSet<&'a str>) -> BTreeSet<&'a str> {
    classes
        .iter()
        .copied()
        .filter(|c| !classes.iter().any(|d| d != c && subsumes(d, c)))
        .collect()
}

/// The message set of a declared capability-name set (#5999 §6).
///
/// Each name is mapped to its class through the registry
/// (fail-closed — an unregistered name, or a name whose class
/// assignment is undesigned, resolves to `unknown`), `cap.none` is
/// dropped, and the result is maximalised under the subsumption
/// order.
///
/// `declared` is the set the comparison reads — `requested ∪
/// platform_bound` at the connect stage. A `*` in it is a defect of
/// the registry entry, not a shortcut (#5999 §3.3): it is expanded
/// over the catalog exactly as the ledger expands a `*` grant —
/// every registered name except sensor caps, which always need an
/// explicit entry — and nothing about `*` turns off the trigger.
/// Names in `optional` are excluded before classes are computed
/// (#5999 §4 rule 6).
#[must_use]
pub fn message_set(
    registry: &ProviderRegistry,
    declared: &BTreeSet<String>,
    optional: &BTreeSet<String>,
) -> BTreeSet<&'static str> {
    let mut classes: BTreeSet<&str> = BTreeSet::new();
    if declared.contains("*") {
        for entry in registry.entries() {
            if !is_sensor_cap(entry.name) && !optional.contains(entry.name) {
                classes.insert(registry.class_for_name(entry.name).id);
            }
        }
    }
    for name in declared {
        if name != "*" && !optional.contains(name.as_str()) {
            classes.insert(registry.class_for_name(name).id);
        }
    }
    classes.remove(CLASS_NONE);
    maximal_classes(&classes)
}

/// The outcome of comparing a new message set `N` against the
/// accepted baseline `B` (#5999 §6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapDiff {
    /// The classes in `N` not covered by `B` — exactly what the
    /// notice names, one plain-language line each (#5959 §3.1).
    /// Sorted, so the surface renders them in a stable order.
    pub new_classes: Vec<String>,
}

impl CapDiff {
    /// Whether the comparison triggers the change notice: a class in
    /// `N` is not covered by `B` (#5999 §4 rule 1 — the whole
    /// trigger; removals, within-class moves, messageless and
    /// optional additions, and used-set-only changes are silent).
    #[must_use]
    pub const fn triggers(&self) -> bool {
        !self.new_classes.is_empty()
    }
}

/// Compare `new` (a message set computed by [`message_set`]) against
/// `baseline` (the accepted message set). Pure: the verdict depends
/// only on the two sets and the subsumption order (#5999 §7
/// invariant 1).
#[must_use]
pub fn compare(baseline: &BTreeSet<&str>, new: &BTreeSet<&str>) -> CapDiff {
    CapDiff {
        new_classes: new
            .iter()
            .copied()
            .filter(|c| !covered_by(baseline, c))
            .map(ToOwned::to_owned)
            .collect(),
    }
}

/// The fail-closed diff for a comparison that cannot be computed.
///
/// Missing manifest data, an unparseable class, version skew
/// (#5999 §7 invariant 10, #5959 §3.2): a trigger carrying the single
/// `unknown` class. Silence requires understanding; an unparseable
/// world is a prompting world.
#[must_use]
pub fn uncomputable_diff() -> CapDiff {
    CapDiff {
        new_classes: vec![crate::cap_class::CLASS_UNKNOWN.to_owned()],
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::cap_class::{
        CLASS_CHAT_READ, CLASS_CHAT_WRITE, CLASS_DEVICE_KEYBOARD, CLASS_DEVICE_LOCATION,
        CLASS_DEVICE_SCREEN, CLASS_UNKNOWN, CLASS_WORLD_PRESENCE, CLASS_WORLD_RECORDS,
        CLASS_WORLD_SCHEDULE,
    };
    use crate::world_caps::PROVIDER_REGISTRY;

    fn names(set: &[&str]) -> BTreeSet<String> {
        set.iter().copied().map(ToOwned::to_owned).collect()
    }

    fn classes<'a>(set: &[&'a str]) -> BTreeSet<&'a str> {
        set.iter().copied().collect()
    }

    #[test]
    fn subsumption_declared_facts_and_nothing_else() {
        // Reflexivity, including for `unknown` (idempotence, #5999
        // §7 invariant 8, needs unknown ≼ unknown).
        for class in [
            CLASS_CHAT_READ,
            CLASS_UNKNOWN,
            CLASS_FS_SANDBOX,
            CLASS_NET_OUT,
        ] {
            assert!(subsumes(class, class), "{class}");
        }
        // cap.none is covered by everything.
        assert!(subsumes(CLASS_CHAT_READ, CLASS_NONE));
        assert!(subsumes(CLASS_UNKNOWN, CLASS_NONE));
        // The two declared cross-class facts.
        assert!(subsumes(CLASS_FS_READ_ARBITRARY, CLASS_FS_SANDBOX));
        assert!(!subsumes(CLASS_FS_SANDBOX, CLASS_FS_READ_ARBITRARY));
        assert!(subsumes(CLASS_NET_OUT, "net.host.example.com"));
        assert!(!subsumes("net.host.example.com", CLASS_NET_OUT));
        assert!(!subsumes(CLASS_NET_OUT, "net.host."));
        // Incomparable pairs: no collapse from a shared prefix.
        assert!(!subsumes(CLASS_CHAT_WRITE, CLASS_CHAT_READ));
        assert!(!subsumes(CLASS_CHAT_READ, CLASS_CHAT_WRITE));
        assert!(!subsumes(CLASS_DEVICE_SCREEN, CLASS_DEVICE_KEYBOARD));
        assert!(!subsumes(CLASS_DEVICE_KEYBOARD, CLASS_DEVICE_SCREEN));
        assert!(!subsumes(CLASS_WORLD_PRESENCE, CLASS_CHAT_READ));
        // `unknown` is subsumed by nothing else and subsumes nothing
        // else (#5999 §2.4).
        assert!(!subsumes(CLASS_FS_READ_ARBITRARY, CLASS_UNKNOWN));
        assert!(!subsumes(CLASS_NET_OUT, CLASS_UNKNOWN));
        assert!(!subsumes(CLASS_UNKNOWN, CLASS_CHAT_READ));
    }

    #[test]
    fn maximal_drops_classes_a_broader_class_covers() {
        let set = classes(&[CLASS_FS_SANDBOX, CLASS_FS_READ_ARBITRARY, CLASS_CHAT_READ]);
        assert_eq!(
            maximal_classes(&set),
            classes(&[CLASS_FS_READ_ARBITRARY, CLASS_CHAT_READ])
        );
        // `unknown` is never dropped by another class.
        let set = classes(&[CLASS_UNKNOWN, CLASS_FS_READ_ARBITRARY]);
        assert_eq!(maximal_classes(&set), set);
    }

    #[test]
    fn message_set_maps_names_to_classes_and_collapses_within_a_class() {
        // loc.position and loc.watch are both device.location: the
        // second name adds no message (#5999 §3.2 rule 1).
        let set = message_set(
            &PROVIDER_REGISTRY,
            &names(&["loc.position", "loc.watch", "chat.say"]),
            &BTreeSet::new(),
        );
        assert_eq!(set, classes(&[CLASS_DEVICE_LOCATION, CLASS_CHAT_WRITE]));
    }

    #[test]
    fn message_set_drops_messageless_and_excludes_optional() {
        // time.now is cap.none: messageless additions are silent
        // (#5999 §4 rule 5).
        let set = message_set(
            &PROVIDER_REGISTRY,
            &names(&["time.now", "chat.history"]),
            &BTreeSet::new(),
        );
        assert_eq!(set, classes(&[CLASS_CHAT_READ]));
        // An optional capability produces no class in N (#5999 §4
        // rule 6).
        let set = message_set(
            &PROVIDER_REGISTRY,
            &names(&["chat.say", "chat.history"]),
            &names(&["chat.say"]),
        );
        assert_eq!(set, classes(&[CLASS_CHAT_READ]));
    }

    #[test]
    fn message_set_fails_closed_on_unknown_names() {
        let set = message_set(
            &PROVIDER_REGISTRY,
            &names(&["no.such.capability"]),
            &BTreeSet::new(),
        );
        assert_eq!(set, classes(&[CLASS_UNKNOWN]));
    }

    #[test]
    fn message_set_expands_star_as_the_ledger_would() {
        let set = message_set(&PROVIDER_REGISTRY, &names(&["*"]), &BTreeSet::new());
        // Every non-sensor catalog class is reached, including
        // net.out via http.fetch and `unknown` via the names whose
        // class assignment is still undesigned.
        for expected in [
            CLASS_CHAT_READ,
            CLASS_CHAT_WRITE,
            CLASS_WORLD_PRESENCE,
            CLASS_WORLD_RECORDS,
            CLASS_WORLD_SCHEDULE,
            CLASS_DEVICE_SCREEN,
            CLASS_DEVICE_KEYBOARD,
            CLASS_NET_OUT,
            CLASS_UNKNOWN,
        ] {
            assert!(set.contains(expected), "{expected}");
        }
        // Sensor caps are not reached by `*`: device.location needs
        // an explicit entry, exactly as in the ledger.
        assert!(!set.contains(CLASS_DEVICE_LOCATION));
    }

    #[test]
    fn compare_names_exactly_the_uncovered_classes() {
        let baseline = classes(&[CLASS_CHAT_READ, CLASS_FS_READ_ARBITRARY]);
        let new = classes(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE, CLASS_FS_SANDBOX]);
        let diff = compare(&baseline, &new);
        // fs.sandbox is covered by the accepted fs.read.arbitrary
        // (§3.2 rule 2); only chat.write is new.
        assert!(diff.triggers());
        assert_eq!(diff.new_classes, [CLASS_CHAT_WRITE]);
    }

    #[test]
    fn uncomputable_comparison_triggers_with_unknown() {
        let diff = uncomputable_diff();
        assert!(diff.triggers());
        assert_eq!(diff.new_classes, [CLASS_UNKNOWN]);
    }

    #[test]
    fn table_split_makes_the_split_out_class_new() {
        // #5999 §7 invariant 9, split direction: a baseline recorded
        // when only device.location existed; a newer table splits
        // out device.location.precise, which is new and triggers.
        let baseline = classes(&[CLASS_DEVICE_LOCATION]);
        let new = classes(&[CLASS_DEVICE_LOCATION, "device.location.precise"]);
        let diff = compare(&baseline, &new);
        assert!(diff.triggers());
        assert_eq!(diff.new_classes, ["device.location.precise"]);
    }

    /// The class vocabulary the property tests range over: the
    /// shipped table, one per-host class, and `unknown` (#5999 §7:
    /// the state space is `(B, N, ≼)` over a small class vocabulary,
    /// decidable without a host).
    fn class_vocabulary() -> Vec<&'static str> {
        let mut vocab: Vec<&'static str> =
            crate::cap_class::CAP_CLASSES.iter().map(|c| c.id).collect();
        vocab.push("net.host.example.com");
        vocab
    }

    mod property {
        use super::*;
        use proptest::prelude::*;

        fn class_set() -> impl Strategy<Value = BTreeSet<&'static str>> {
            let vocab = class_vocabulary();
            proptest::collection::btree_set(proptest::sample::select(vocab), 0..=5)
        }

        proptest! {
            /// Oracle: #5999 §7 invariant 1 (class-set determined) —
            /// the verdict depends only on the message sets: maximal-
            /// ising either side never changes whether it triggers,
            /// and maximalising the baseline never changes which
            /// classes are named new.
            #[test]
            fn verdict_depends_only_on_the_message_sets(
                b in class_set(),
                n in class_set(),
            ) {
                let diff = compare(&b, &n);
                let maximal_diff = compare(&maximal_classes(&b), &maximal_classes(&n));
                prop_assert_eq!(diff.triggers(), maximal_diff.triggers());
                prop_assert_eq!(
                    compare(&maximal_classes(&b), &n).new_classes,
                    diff.new_classes
                );
            }

            /// Oracle: #5999 §7 invariant 2 (monotone in acceptance)
            /// — widening the baseline can only remove classes from
            /// the new set, never add one: accepting can only silence.
            #[test]
            fn accepting_more_never_adds_new_classes(
                b in class_set(),
                extra in class_set(),
                n in class_set(),
            ) {
                let widened: BTreeSet<&str> = b.union(&extra).copied().collect();
                let before: BTreeSet<String> =
                    compare(&b, &n).new_classes.into_iter().collect();
                let after: BTreeSet<String> =
                    compare(&widened, &n).new_classes.into_iter().collect();
                prop_assert!(after.is_subset(&before));
            }

            /// Oracle: #5999 §7 invariant 3 (never trigger on
            /// shrink) — a new set contained in the baseline is
            /// always silent.
            #[test]
            fn a_subset_never_triggers(b in class_set(), n in class_set()) {
                let shrunk: BTreeSet<&str> = n.intersection(&b).copied().collect();
                prop_assert!(!compare(&b, &shrunk).triggers());
            }

            /// Oracle: #5999 §7 invariant 4 (fail-closed) — `unknown`
            /// in the new set triggers whenever the baseline does not
            /// itself carry `unknown`, and is named in the diff.
            #[test]
            fn unknown_is_new_unless_the_baseline_carries_it(
                b in class_set(),
                n in class_set(),
            ) {
                let mut with_unknown = n.clone();
                with_unknown.insert(CLASS_UNKNOWN);
                let diff = compare(&b, &with_unknown);
                if b.contains(CLASS_UNKNOWN) {
                    prop_assert!(!diff.new_classes.iter().any(|c| c == CLASS_UNKNOWN));
                } else {
                    prop_assert!(diff.triggers());
                    prop_assert!(diff.new_classes.iter().any(|c| c == CLASS_UNKNOWN));
                }
            }

            /// Oracle: #5999 §7 invariant 5 (set semantics) — the
            /// order the raw lists were built in is irrelevant.
            #[test]
            fn construction_order_is_irrelevant(
                b_list in proptest::collection::vec(
                    proptest::sample::select(class_vocabulary()), 0..=8),
                n_list in proptest::collection::vec(
                    proptest::sample::select(class_vocabulary()), 0..=8),
            ) {
                let b: BTreeSet<&str> = b_list.iter().copied().collect();
                let n: BTreeSet<&str> = n_list.iter().copied().collect();
                let b_rev: BTreeSet<&str> = b_list.iter().rev().copied().collect();
                let n_rev: BTreeSet<&str> = n_list.iter().rev().copied().collect();
                prop_assert_eq!(compare(&b, &n), compare(&b_rev, &n_rev));
            }

            /// Oracle: #5999 §7 invariant 8 (idempotent) — comparing
            /// a state against itself is silent, and accepting the
            /// new set makes re-comparing it silent.
            #[test]
            fn comparing_a_state_against_itself_is_silent(
                b in class_set(),
                n in class_set(),
            ) {
                prop_assert!(!compare(&b, &b).triggers());
                prop_assert!(!compare(&n, &n).triggers());
            }

            /// Oracle: #5999 §7 invariant 9 (table-change safety),
            /// silent direction — a recomputation whose new set is
            /// covered by the baseline recorded under the older
            /// table stays silent; only a class the baseline does
            /// not cover can trigger (the split direction is the
            /// unit test above).
            #[test]
            fn covered_recomputation_stays_silent(
                b in class_set(),
                n in class_set(),
            ) {
                let covered: BTreeSet<&str> = n
                    .iter()
                    .copied()
                    .filter(|c| covered_by(&b, c))
                    .collect();
                prop_assert!(!compare(&b, &covered).triggers());
            }
        }
    }
}
