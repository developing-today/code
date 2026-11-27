//! The stress workload's event-shape schedule (#16044).
//!
//! Shared between the `world_bench` stress generator (included via
//! `#[path]`, like the other `tests/common` helpers) and the
//! integration oracle tests below, so the tests assert on the exact
//! schedule the generator runs — never on a copy of it.
//!
//! The schedule is a pure function of the stream position: the same
//! (seed, tier) always produces the same stream, and the per-1,000
//! counts always match the declared mix table.

/// The stress event mix, as counts per 1,000 events in the same
/// discipline as the loadgen core's mix tables.
pub const STRESS_MIX_CHAT: u64 = 700;
pub const STRESS_MIX_DOC_EDIT: u64 = 200;
pub const STRESS_MIX_PROGRAM_INSTALL: u64 = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StressShape {
    Chat,
    DocEdit,
    ProgramInstall,
}

impl StressShape {
    pub fn name(self) -> &'static str {
        match self {
            StressShape::Chat => "chat",
            StressShape::DocEdit => "doc_edit",
            StressShape::ProgramInstall => "program_install",
        }
    }
}

pub fn stress_shape_at(index: u64) -> StressShape {
    let at = index % 1_000;
    if at < STRESS_MIX_CHAT {
        StressShape::Chat
    } else if at < STRESS_MIX_CHAT + STRESS_MIX_DOC_EDIT {
        StressShape::DocEdit
    } else {
        StressShape::ProgramInstall
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn counts(len: u64) -> (u64, u64, u64) {
        let (mut chat, mut doc_edits, mut program_installs) = (0, 0, 0);
        for i in 0..len {
            match stress_shape_at(i) {
                StressShape::Chat => chat += 1,
                StressShape::DocEdit => doc_edits += 1,
                StressShape::ProgramInstall => program_installs += 1,
            }
        }
        (chat, doc_edits, program_installs)
    }

    fn has_all_shapes(len: u64) -> bool {
        let (chat, doc_edits, program_installs) = counts(len);
        chat > 0 && doc_edits > 0 && program_installs > 0
    }

    #[test]
    fn mix_per_1000_matches_the_declared_table() {
        // Oracle: the manifest's declared mix table (700 chat /
        // 200 doc edit / 100 program install per 1,000 events).
        assert_eq!(counts(1_000), (700, 200, 100));
        assert_eq!(counts(100), (70, 20, 10));
        // A 20-event quick-tier active world is two 10-event grains:
        // 14 chat / 4 doc edit / 2 program install.
        assert_eq!(counts(20), (14, 4, 2));
    }

    #[test]
    fn every_10_event_window_holds_every_shape() {
        // Oracle: issue #16044 — no fixture window may silently
        // lack doc edits or program installs.
        for start in (0..1_000).step_by(10) {
            let mut seen = [false, false, false];
            for i in start..start + 10 {
                match stress_shape_at(i) {
                    StressShape::Chat => seen[0] = true,
                    StressShape::DocEdit => seen[1] = true,
                    StressShape::ProgramInstall => seen[2] = true,
                }
            }
            assert!(
                seen == [true, true, true],
                "window {start}..{} lacks a declared shape",
                start + 10
            );
        }
    }

    #[test]
    fn quick_tier_fixtures_hold_every_shape() {
        // Oracle: issue #16044 — the quick tier's 200-event hot
        // world and 20-event active worlds must include the
        // declared shapes, not only chat.
        assert!(has_all_shapes(200), "quick hot world lacks a shape");
        assert!(has_all_shapes(20), "quick active world lacks a shape");
    }

    #[test]
    fn schedule_is_a_pure_function_of_position() {
        // Oracle: determinism — the shape at a stream position is
        // fixed, so the same (seed, tier) always yields the same
        // stream.
        for i in [0, 7, 9, 10, 699, 700, 899, 900, 999, 1_000] {
            assert_eq!(stress_shape_at(i), stress_shape_at(i));
        }
        assert_eq!(stress_shape_at(0), stress_shape_at(1_000));
    }
}
