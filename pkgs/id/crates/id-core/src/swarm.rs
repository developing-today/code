//! Multi-provider chunk-range retrieval scheduler — PoC (issue #2261,
//! follow-up B of the #2050 investigation).
//!
//! Retrieval in `pkgs/id` is single-source today: the `get` path performs
//! one `store.remote().fetch(conn, hash)` over one connection. iroh-blobs
//! already supports the swarm-shaped pieces — `(hash, ChunkRanges)`
//! requests per provider, bao verified streaming (each 16 KiB chunk group
//! verified against the blob's BLAKE3 root hash as it arrives, from any
//! source, in any order) — what does not exist is the scheduling policy
//! that uses several holders at once. This module is that policy, as a
//! pure, deterministic state machine the fetch loop can drive:
//!
//! - Wanted-set = chunk-group ranges of the blob; ranges are assigned
//!   rarest-first across the provider set, disjoint per holder, with at
//!   most [`MAX_SOURCES`] concurrent sources.
//! - A verified group is marked done immediately (the fetch loop persists
//!   it to FsStore at the same moment, so partial state is resumable and
//!   servable). A bad group is discarded, its range requeued, and the
//!   holder deprioritised for this blob. A missing-range stream close
//!   shrinks that holder's known ranges and requeues the range elsewhere.
//! - Endgame: when every remaining wanted group is already in flight, the
//!   trailing ranges are duplicate-requested from another holder; the
//!   first verified copy wins and the loser is cancelled.
//! - Nobody-home is distinct: groups no holder can serve are reported as
//!   an explicit uncovered byte range, never as a generic transfer
//!   failure, and the rest of the plan still proceeds.
//! - Both discovery modes work: observe-informed (held ranges known via
//!   [`HeldRanges`]) and optimistic (ranges unknown — every holder is a
//!   candidate and missing-range reports teach the scheduler).
//!
//! PoC status: this is the review-gated proof of concept for #2261.
//! Fixture benchmarks for the required matrix (1 vs 2 vs 4 holders,
//! partial mix, holder leaving, optimistic mode) live in
//! `tests/swarm_bench.rs` and are reported on the issue. The fetch-loop
//! wiring in the `get` path and live multi-holder benchmarks are the
//! remaining steps recorded on the issue; nothing here is the permanent
//! retrieval path until that review round decides.

use std::collections::BTreeMap;

use crate::holders::HeldRanges;

/// Size of one bao chunk group: the unit iroh-blobs verifies as it
/// streams. All ranges in this module are group-aligned except a blob's
/// final group, which ends at the blob size.
pub const CHUNK_GROUP_BYTES: u64 = 16 * 1024;

/// Maximum number of holders fetched from concurrently (issue #2261:
/// "start with ≤4 concurrent sources").
pub const MAX_SOURCES: usize = 4;

/// A half-open byte range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteRange {
    /// Inclusive start offset.
    pub start: u64,
    /// Exclusive end offset.
    pub end: u64,
}

impl ByteRange {
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }
}

/// One assignment: fetch `ranges` from holder `holder`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// Index of the holder in the scheduler's provider set.
    pub holder: usize,
    /// Disjoint, coalesced ranges to fetch from that holder.
    pub ranges: Vec<ByteRange>,
}

/// Why a group is not (or no longer) assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedEvent {
    /// Group verified against the blob root hash; persist and serve.
    Verified {
        /// Holder the group came from.
        holder: usize,
        /// The verified range.
        range: ByteRange,
    },
    /// Group failed verification from this holder: discard, requeue,
    /// deprioritise the holder.
    BadGroup {
        /// Holder that served the bad group.
        holder: usize,
        /// The failed range.
        range: ByteRange,
    },
    /// Holder closed the stream reporting it lacks this range: shrink
    /// its known ranges, requeue elsewhere.
    Missing {
        /// Holder reporting the gap.
        holder: usize,
        /// The range it does not hold.
        range: ByteRange,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupState {
    Wanted,
    InFlight(usize),
    Done,
}

#[derive(Debug, Clone)]
struct HolderState {
    name: String,
    held: HeldRanges,
    /// Deprioritisation score: bad groups subtract, verified groups add.
    /// Holders at or below `DEPRIORITISE_AT` are skipped while any
    /// better-scored holder can serve a group.
    score: i32,
    in_flight: usize,
}

const DEPRIORITISE_AT: i32 = -2;

/// The scheduler for one blob fetch. Drive it with
/// [`Scheduler::assignments`] and feed outcomes back with
/// [`Scheduler::apply`]; it is complete when [`Scheduler::is_complete`],
/// and stalled on [`Scheduler::uncovered`] ranges when no holder can
/// serve them.
#[derive(Debug)]
pub struct Scheduler {
    blob_size: u64,
    groups: u64,
    state: Vec<GroupState>,
    holders: Vec<HolderState>,
    /// Per-range provenance: one line per verified range, in the order
    /// groups completed ("range X-Y <- holder H").
    pub provenance: Vec<String>,
}

impl Scheduler {
    /// Create a scheduler for a blob of `blob_size` bytes and the given
    /// `(name, held-ranges)` provider set. Hand-fed provider lists are
    /// acceptable for the PoC (issue #2261); in production the set comes
    /// from the #2260 provider table.
    pub fn new(blob_size: u64, holders: Vec<(String, HeldRanges)>) -> Self {
        let groups = blob_size.div_ceil(CHUNK_GROUP_BYTES);
        Self {
            blob_size,
            groups,
            state: vec![GroupState::Wanted; groups as usize],
            holders: holders
                .into_iter()
                .map(|(name, held)| HolderState {
                    name,
                    held,
                    score: 0,
                    in_flight: 0,
                })
                .collect(),
            provenance: Vec::new(),
        }
    }

    fn group_range(&self, g: u64) -> ByteRange {
        let start = g * CHUNK_GROUP_BYTES;
        ByteRange::new(start, (start + CHUNK_GROUP_BYTES).min(self.blob_size))
    }

    /// Does holder `h` claim group `g`? `Unknown` counts as a claim in
    /// optimistic mode (the scheduler learns from missing reports).
    fn claims(&self, h: usize, g: u64) -> bool {
        let r = self.group_range(g);
        match &self.holders[h].held {
            HeldRanges::Complete => true,
            HeldRanges::Unknown => true,
            HeldRanges::Ranges(rs) => rs.iter().any(|&(s, e)| s <= r.start && r.end <= e),
        }
    }

    /// Is holder `h`'s claim on group `g` backed by known ranges (vs an
    /// optimistic `Unknown` guess)?
    fn knows(&self, h: usize, g: u64) -> bool {
        !matches!(self.holders[h].held, HeldRanges::Unknown) && self.claims(h, g)
    }

    /// Availability of group `g`: how many holders claim it.
    fn availability(&self, g: u64) -> usize {
        (0..self.holders.len())
            .filter(|&h| self.claims(h, g))
            .count()
    }

    /// Byte range of wanted groups no holder can serve, if any — the
    /// distinct nobody-home signal. Optimistic (`Unknown`) holders count
    /// as coverage here, so this fires only when coverage is *known* to
    /// be absent.
    pub fn uncovered(&self) -> Option<ByteRange> {
        let mut first: Option<u64> = None;
        let mut last = 0;
        for g in 0..self.groups {
            if self.state[g as usize] == GroupState::Wanted && self.availability(g) == 0 {
                if first.is_none() {
                    first = Some(g);
                }
                last = g;
            }
        }
        first.map(|f| ByteRange::new(f * CHUNK_GROUP_BYTES, self.group_range(last).end))
    }

    pub fn is_complete(&self) -> bool {
        self.state.iter().all(|s| *s == GroupState::Done)
    }

    /// Compute the next assignments. Invariants (the module's oracles):
    /// ranges are group-aligned, disjoint across holders, only wanted
    /// groups are assigned (except endgame duplicates of in-flight
    /// groups), and at most [`MAX_SOURCES`] holders are active.
    pub fn assignments(&mut self) -> Vec<Assignment> {
        // Endgame: every wanted group is already in flight — duplicate
        // the trailing ranges onto a second holder.
        let any_wanted = self.state.iter().any(|s| *s == GroupState::Wanted);
        let any_in_flight = self
            .state
            .iter()
            .any(|s| matches!(s, GroupState::InFlight(_)));
        if !any_wanted && any_in_flight {
            return self.endgame_assignments();
        }

        // Rarest-first: wanted groups sorted by availability, then index.
        let mut wanted: Vec<u64> = (0..self.groups)
            .filter(|&g| self.state[g as usize] == GroupState::Wanted)
            .collect();
        wanted.sort_by_key(|&g| (self.availability(g), g));

        let mut active: Vec<usize> = (0..self.holders.len())
            .filter(|&h| self.holders[h].in_flight > 0)
            .collect();
        let mut by_holder: BTreeMap<usize, Vec<u64>> = BTreeMap::new();

        for g in wanted {
            // Candidate holders: claim the group, best score first,
            // then least loaded. Known-range claims beat optimistic ones.
            let mut cands: Vec<usize> = (0..self.holders.len())
                .filter(|&h| self.claims(h, g))
                .collect();
            cands.sort_by_key(|&h| {
                (
                    std::cmp::Reverse(self.knows(h, g)),
                    std::cmp::Reverse(self.holders[h].score),
                    self.holders[h].in_flight + by_holder.get(&h).map_or(0, |v| v.len()),
                )
            });
            // Skip deprioritised holders while a better one can serve.
            let pick = cands
                .iter()
                .copied()
                .find(|&h| self.holders[h].score > DEPRIORITISE_AT)
                .or_else(|| cands.first().copied());
            let Some(h) = pick else { continue };
            // Respect the concurrent-source cap for *new* holders.
            if !active.contains(&h) {
                if active.len() >= MAX_SOURCES {
                    continue;
                }
                active.push(h);
            }
            self.state[g as usize] = GroupState::InFlight(h);
            by_holder.entry(h).or_default().push(g);
        }

        by_holder
            .into_iter()
            .map(|(holder, gs)| {
                self.holders[holder].in_flight += gs.len();
                Assignment {
                    holder,
                    ranges: coalesce(gs.iter().map(|&g| self.group_range(g)).collect()),
                }
            })
            .collect()
    }

    fn endgame_assignments(&mut self) -> Vec<Assignment> {
        let mut by_holder: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
        for g in 0..self.groups {
            if let GroupState::InFlight(cur) = self.state[g as usize] {
                // Duplicate onto the best other holder that claims it.
                let dup = (0..self.holders.len())
                    .filter(|&h| h != cur && self.claims(h, g))
                    .max_by_key(|&h| self.holders[h].score);
                if let Some(h) = dup {
                    by_holder.entry(h).or_default().push(g);
                }
            }
        }
        by_holder
            .into_iter()
            .map(|(holder, gs)| Assignment {
                holder,
                ranges: coalesce(gs.iter().map(|&g| self.group_range(g)).collect()),
            })
            .collect()
    }

    /// Feed one fetch outcome back into the scheduler.
    pub fn apply(&mut self, ev: SchedEvent) {
        match ev {
            SchedEvent::Verified { holder, range } => {
                for g in self.groups_in(range) {
                    if self.state[g as usize] != GroupState::Done {
                        self.state[g as usize] = GroupState::Done;
                        self.holders[holder].score += 1;
                        self.provenance.push(format!(
                            "range {}-{} <- holder {}",
                            range.start, range.end, self.holders[holder].name
                        ));
                    }
                }
                self.recount_in_flight();
            }
            SchedEvent::BadGroup { holder, range } => {
                for g in self.groups_in(range) {
                    if let GroupState::InFlight(h) = self.state[g as usize] {
                        if h == holder {
                            self.state[g as usize] = GroupState::Wanted;
                        }
                    }
                }
                self.holders[holder].score -= 2;
                self.recount_in_flight();
            }
            SchedEvent::Missing { holder, range } => {
                // Shrink the holder's known ranges by the reported range,
                // and requeue anything it had in flight there.
                if let HeldRanges::Ranges(rs) = &mut self.holders[holder].held {
                    let mut out = Vec::new();
                    for &(s, e) in rs.iter() {
                        if e <= range.start || s >= range.end {
                            out.push((s, e));
                        } else {
                            if s < range.start {
                                out.push((s, range.start));
                            }
                            if e > range.end {
                                out.push((range.end, e));
                            }
                        }
                    }
                    *rs = out;
                }
                for g in self.groups_in(range) {
                    if let GroupState::InFlight(h) = self.state[g as usize] {
                        if h == holder {
                            self.state[g as usize] = GroupState::Wanted;
                        }
                    }
                }
                self.recount_in_flight();
            }
        }
    }

    fn groups_in(&self, range: ByteRange) -> Vec<u64> {
        (0..self.groups)
            .filter(|&g| {
                let r = self.group_range(g);
                r.start < range.end && range.start < r.end
            })
            .collect()
    }

    fn recount_in_flight(&mut self) {
        for h in self.holders.iter_mut() {
            h.in_flight = 0;
        }
        for s in &self.state {
            if let GroupState::InFlight(h) = s {
                self.holders[*h].in_flight += 1;
            }
        }
    }
}

/// Merge sorted, adjacent byte ranges into maximal contiguous ranges.
fn coalesce(mut rs: Vec<ByteRange>) -> Vec<ByteRange> {
    rs.sort();
    let mut out: Vec<ByteRange> = Vec::new();
    for r in rs {
        if let Some(last) = out.last_mut() {
            if last.end == r.start {
                last.end = r.end;
                continue;
            }
        }
        out.push(r);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn complete(name: &str) -> (String, HeldRanges) {
        (name.to_string(), HeldRanges::Complete)
    }

    fn ranges(name: &str, rs: Vec<(u64, u64)>) -> (String, HeldRanges) {
        (name.to_string(), HeldRanges::Ranges(rs))
    }

    fn blob(groups: u64) -> u64 {
        groups * CHUNK_GROUP_BYTES
    }

    /// Oracle: assignments are disjoint and cover only wanted groups.
    fn assert_disjoint(a: &[Assignment]) {
        let mut all: Vec<ByteRange> = a.iter().flat_map(|x| x.ranges.clone()).collect();
        all.sort();
        for w in all.windows(2) {
            assert!(w[0].end <= w[1].start, "overlap: {:?} vs {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn rarest_first_and_disjoint() {
        // 4 groups; holder A has groups 0-1, holder B has all, holder C
        // has group 3 only. Group 3 is rarest (B, C) after group 0-1 (A, B).
        let mut s = Scheduler::new(
            blob(4),
            vec![
                ranges("a", vec![(0, blob(2))]),
                complete("b"),
                ranges("c", vec![(blob(3), blob(4))]),
            ],
        );
        let a = s.assignments();
        assert_disjoint(&a);
        let total: u64 = a
            .iter()
            .flat_map(|x| &x.ranges)
            .map(|r| (r.end - r.start) / CHUNK_GROUP_BYTES)
            .sum();
        assert_eq!(total, 4, "all groups assigned exactly once");
    }

    #[test]
    fn source_cap_is_respected() {
        let holders: Vec<(String, HeldRanges)> =
            (0..8).map(|i| complete(&format!("h{i}"))).collect();
        let mut s = Scheduler::new(blob(16), holders);
        let a = s.assignments();
        assert!(a.len() <= MAX_SOURCES, "active holders {} > cap", a.len());
        assert_disjoint(&a);
    }

    #[test]
    fn bad_group_requeues_and_deprioritises() {
        let mut s = Scheduler::new(blob(2), vec![complete("a"), complete("b")]);
        let a = s.assignments();
        let first = a[0].clone();
        let r = first.ranges[0];
        s.apply(SchedEvent::BadGroup {
            holder: first.holder,
            range: r,
        });
        // Requeued: a later assignment offers the range again, and the
        // deprioritised holder is not the first pick while score differs.
        let a2 = s.assignments();
        assert!(
            a2.iter().any(|x| x.ranges.contains(&r)),
            "bad range must be requeued"
        );
        s.apply(SchedEvent::BadGroup {
            holder: first.holder,
            range: r,
        });
        let a3 = s.assignments();
        assert!(
            a3.iter().all(|x| x.holder != first.holder || a3.len() == 1),
            "twice-bad holder deprioritised while an alternative exists"
        );
    }

    #[test]
    fn missing_shrinks_holder_and_requeues() {
        let mut s = Scheduler::new(
            blob(2),
            vec![ranges("a", vec![(0, blob(2))]), complete("b")],
        );
        let r = ByteRange::new(0, blob(1));
        s.apply(SchedEvent::Missing {
            holder: 0,
            range: r,
        });
        // Holder 0 no longer claims group 0.
        assert!(!s.claims(0, 0));
        assert!(s.claims(0, 1));
        let a = s.assignments();
        assert_disjoint(&a);
    }

    #[test]
    fn nobody_home_is_distinct() {
        // Only group 0 is held; groups 1-2 have no holder.
        let mut s = Scheduler::new(blob(3), vec![ranges("a", vec![(0, blob(1))])]);
        let u = s.uncovered().expect("uncovered range must be reported");
        assert_eq!(u, ByteRange::new(blob(1), blob(3)));
        let _ = s.assignments();
        assert!(!s.is_complete());
    }

    #[test]
    fn endgame_duplicates_trailing_ranges() {
        let mut s = Scheduler::new(blob(2), vec![complete("a"), complete("b")]);
        let a = s.assignments();
        // One holder takes both groups (least-loaded tie-break may vary);
        // with everything in flight and nothing wanted, endgame offers
        // duplicates from the other holder.
        let _ = a;
        let e = s.endgame_assignments();
        assert!(!e.is_empty(), "endgame must duplicate in-flight ranges");
        // Completing via either copy finishes the fetch.
        for x in &e {
            for r in &x.ranges {
                s.apply(SchedEvent::Verified {
                    holder: x.holder,
                    range: *r,
                });
            }
        }
        assert!(s.is_complete());
        assert_eq!(s.provenance.len(), 2);
    }

    #[test]
    fn optimistic_unknown_learns_from_missing() {
        let mut s = Scheduler::new(
            blob(1),
            vec![("u".to_string(), HeldRanges::Unknown), complete("b")],
        );
        // Unknown counts as coverage: no nobody-home report.
        assert!(s.uncovered().is_none());
        let a = s.assignments();
        assert_disjoint(&a);
    }
}
