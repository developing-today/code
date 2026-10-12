//! Local warnings about a client-made group's size and re-add churn (P4.9, N11 and N34).
//!
//! The server evaluates each client-made group. A warning fires when the group's member count
//! reaches 96, 128 or 1,000, and when its devices are re-added in bursts (a re-add is the N8
//! recovery, so a burst is worth a look). The warnings are local to the server: they go to its log
//! through [`log`], and the same text goes to the CLI status line through [`Warning::message`]. No
//! warning blocks an operation, and no group is refused for its size (decision 9).
//!
//! A warning fires once per crossing. The evaluator keeps each group's high-water mark, so a group
//! that shrinks below a threshold and grows back does not warn again. Churn re-arms only once the
//! window falls to [`CHURN_CLEAR`], so a count that flaps around [`CHURN_THRESHOLD`] warns once.

use std::collections::BTreeMap;

use p2panda_core::Hash;

/// The member counts that warn (N11).
pub const SIZE_THRESHOLDS: [usize; 3] = [96, 128, 1_000];

/// The re-add window, in seconds: one day.
pub const CHURN_WINDOW_SECS: u64 = 24 * 60 * 60;

/// The re-adds in the window that warn (N34).
pub const CHURN_THRESHOLD: usize = 4;

/// The window count at or below which churn re-arms: half the threshold. A count that flaps
/// between the two does not re-arm, so it does not warn twice.
pub const CHURN_CLEAR: usize = 2;

/// One warning about a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Warning {
    /// The group reached `threshold` members, and has `members` now.
    Size {
        /// The threshold crossed.
        threshold: usize,
        /// The member count when it was crossed.
        members: usize,
    },
    /// The group had `readds` device re-adds in the last [`CHURN_WINDOW_SECS`].
    Churn {
        /// The re-adds in the window.
        readds: usize,
    },
}

impl Warning {
    /// The text for the server log and the CLI status line.
    pub fn message(&self) -> String {
        match self {
            Self::Size { threshold, members } => format!(
                "group reached {threshold} members ({members} now): each update is sent to every member, so its cost grows with the group"
            ),
            Self::Churn { readds } => format!(
                "{readds} device re-adds in the last 24 hours: each re-add is a compromise recovery, so check the devices"
            ),
        }
    }
}

/// What the server knows about one group when it evaluates it.
#[derive(Clone, Copy, Debug)]
pub struct Observation<'a> {
    /// The group.
    pub group: Hash,
    /// Whether a client made the group. Groups provisioned from a server config are not warned
    /// about.
    pub client_made: bool,
    /// The member count, from the fold.
    pub members: usize,
    /// When each device was re-added, in Unix seconds, from the group's events.
    pub readds: &'a [u64],
}

/// Tracks each group's warnings between evaluations. It holds no clock: the caller passes `now`.
#[derive(Debug, Default)]
pub struct Evaluator {
    groups: BTreeMap<Hash, Tracked>,
}

/// What an evaluator remembers about one group.
#[derive(Debug)]
struct Tracked {
    /// How many of [`SIZE_THRESHOLDS`] the group has warned for. It never goes down.
    crossed: usize,
    /// Whether churn may warn. It disarms on a warning and re-arms at [`CHURN_CLEAR`].
    churn_armed: bool,
}

impl Default for Tracked {
    fn default() -> Self {
        Self {
            crossed: 0,
            churn_armed: true,
        }
    }
}

impl Evaluator {
    /// Evaluates one group at `now` (Unix seconds). It returns the warnings this observation adds,
    /// and never fails: nothing here blocks an operation.
    pub fn observe(&mut self, observation: &Observation<'_>, now: u64) -> Vec<Warning> {
        if !observation.client_made {
            return Vec::new();
        }
        let tracked = self.groups.entry(observation.group).or_default();
        let mut warnings = Vec::new();

        let reached = SIZE_THRESHOLDS
            .iter()
            .filter(|threshold| **threshold <= observation.members)
            .count();
        for threshold in SIZE_THRESHOLDS.iter().take(reached).skip(tracked.crossed) {
            warnings.push(Warning::Size {
                threshold: *threshold,
                members: observation.members,
            });
        }
        tracked.crossed = tracked.crossed.max(reached);

        let readds = window_count(observation.readds, now);
        if readds <= CHURN_CLEAR {
            tracked.churn_armed = true;
        } else if readds >= CHURN_THRESHOLD && tracked.churn_armed {
            tracked.churn_armed = false;
            warnings.push(Warning::Churn { readds });
        }
        warnings
    }
}

/// The re-adds in the window that ends at `now`. A re-add later than `now` is not counted yet.
fn window_count(readds: &[u64], now: u64) -> usize {
    readds
        .iter()
        .filter(|at| **at <= now && now - **at < CHURN_WINDOW_SECS)
        .count()
}

/// Writes each warning to the server log, against the group it is about.
pub fn log(group: &Hash, warnings: &[Warning]) {
    for warning in warnings {
        tracing::warn!(group = %group.to_hex(), "{}", warning.message());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn group() -> Hash {
        Hash::digest(b"a client-made group")
    }

    fn observe(
        evaluator: &mut Evaluator,
        members: usize,
        readds: &[u64],
        now: u64,
    ) -> Vec<Warning> {
        let observation = Observation {
            group: group(),
            client_made: true,
            members,
            readds,
        };
        evaluator.observe(&observation, now)
    }

    fn size(threshold: usize, members: usize) -> Warning {
        Warning::Size { threshold, members }
    }

    #[test]
    fn test_each_threshold_warns_at_its_count_and_not_one_below() {
        // A new group that starts at `members` has crossed every threshold at or below it.
        for (members, expected) in [
            (95, vec![]),
            (96, vec![size(96, 96)]),
            (127, vec![size(96, 127)]),
            (128, vec![size(96, 128), size(128, 128)]),
            (999, vec![size(96, 999), size(128, 999)]),
            (
                1_000,
                vec![size(96, 1_000), size(128, 1_000), size(1_000, 1_000)],
            ),
        ] {
            let warnings = observe(&mut Evaluator::default(), members, &[], 0);
            assert_eq!(warnings, expected, "at {members} members");
        }
    }

    #[test]
    fn test_a_group_warns_once_per_crossing_as_it_grows() {
        let mut evaluator = Evaluator::default();
        let mut seen = Vec::new();
        for members in [95, 96, 96, 97, 127, 128, 128, 999, 1_000, 1_001] {
            seen.extend(observe(&mut evaluator, members, &[], 0));
        }
        assert_eq!(seen, vec![size(96, 96), size(128, 128), size(1_000, 1_000)]);
    }

    #[test]
    fn test_one_jump_past_several_thresholds_warns_for_each_in_order() {
        let warnings = observe(&mut Evaluator::default(), 1_200, &[], 0);
        assert_eq!(
            warnings,
            vec![size(96, 1_200), size(128, 1_200), size(1_000, 1_200)]
        );
    }

    #[test]
    fn test_flapping_across_a_threshold_warns_once() {
        let mut evaluator = Evaluator::default();
        let mut seen = Vec::new();
        for members in [96, 95, 96, 95, 96, 94, 96] {
            seen.extend(observe(&mut evaluator, members, &[], 0));
        }
        assert_eq!(
            seen,
            vec![size(96, 96)],
            "the shrink and regrow do not repeat"
        );
    }

    #[test]
    fn test_churn_warns_at_the_threshold_and_then_rearms_when_the_window_clears() {
        let mut evaluator = Evaluator::default();
        let mut readds = vec![0, 1, 2, 3];
        assert_eq!(
            observe(&mut evaluator, 10, &readds, 3),
            vec![Warning::Churn { readds: 4 }]
        );

        readds.push(4);
        assert!(
            observe(&mut evaluator, 10, &readds, 4).is_empty(),
            "one warning per burst"
        );

        // Every re-add has aged out of the window, so churn re-arms.
        let later = CHURN_WINDOW_SECS + 4;
        assert!(observe(&mut evaluator, 10, &readds, later).is_empty());

        let burst = later + 10;
        readds.extend([burst, burst + 1, burst + 2, burst + 3]);
        assert_eq!(
            observe(&mut evaluator, 10, &readds, burst + 3),
            vec![Warning::Churn { readds: 4 }],
            "a new burst warns again"
        );
    }

    #[test]
    fn test_churn_flapping_around_the_threshold_warns_once() {
        let mut evaluator = Evaluator::default();
        let mut readds = vec![1, 2, 3, 4];
        let mut seen = observe(&mut evaluator, 10, &readds, 4);

        // Re-add 1 ages out, so the count drops to 3, which is above CHURN_CLEAR and does not
        // re-arm. Then a new re-add brings it back to 4.
        let mut now = CHURN_WINDOW_SECS + 1;
        seen.extend(observe(&mut evaluator, 10, &readds, now));
        readds.push(now);
        seen.extend(observe(&mut evaluator, 10, &readds, now));
        now += 1;
        seen.extend(observe(&mut evaluator, 10, &readds, now));
        readds.push(now);
        seen.extend(observe(&mut evaluator, 10, &readds, now));

        assert_eq!(seen, vec![Warning::Churn { readds: 4 }]);
    }

    #[test]
    fn test_a_group_that_is_not_client_made_is_never_warned_about() {
        let observation = Observation {
            group: group(),
            client_made: false,
            members: 1_000,
            readds: &[1, 2, 3, 4],
        };
        assert!(Evaluator::default().observe(&observation, 4).is_empty());
    }

    #[test]
    fn test_each_group_keeps_its_own_warnings() {
        let mut evaluator = Evaluator::default();
        let other = Hash::digest(b"another group");
        assert_eq!(observe(&mut evaluator, 96, &[], 0), vec![size(96, 96)]);
        let observation = Observation {
            group: other,
            client_made: true,
            members: 96,
            readds: &[],
        };
        assert_eq!(evaluator.observe(&observation, 0), vec![size(96, 96)]);
    }

    #[test]
    fn test_the_message_names_the_threshold_and_the_churn() {
        assert!(size(96, 96).message().contains("96"));
        assert!(
            Warning::Churn { readds: 4 }
                .message()
                .contains("4 device re-adds")
        );
    }

    proptest! {
        /// Over any path of sizes, the warnings a group has earned are the thresholds its largest
        /// size has reached, and the count never falls along the way.
        #[test]
        fn test_a_groups_warning_count_follows_its_largest_size(
            sizes in proptest::collection::vec(0usize..1_500, 1..40),
        ) {
            let mut evaluator = Evaluator::default();
            let mut total = 0;
            let mut peak = 0;
            for members in sizes {
                peak = peak.max(members);
                let warned = observe(&mut evaluator, members, &[], 0).len();
                total += warned;
                let reached = SIZE_THRESHOLDS.iter().filter(|t| **t <= peak).count();
                prop_assert_eq!(total, reached);
            }
        }

        /// A bigger group never has fewer warnings than a smaller one.
        #[test]
        fn test_warnings_are_monotone_in_size(a in 0usize..1_500, b in 0usize..1_500) {
            let (small, large) = (a.min(b), a.max(b));
            let small_count = observe(&mut Evaluator::default(), small, &[], 0).len();
            let large_count = observe(&mut Evaluator::default(), large, &[], 0).len();
            prop_assert!(small_count <= large_count);
        }
    }
}
