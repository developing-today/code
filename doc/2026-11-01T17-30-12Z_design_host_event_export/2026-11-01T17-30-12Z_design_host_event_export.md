# Host-event journal export for world fixtures

**Request:** implement issue [#242](https://github.com/developing-today/code/issues/242) in the `fix/id-242` worktree after the owner’s decision was posted on the issue. This design records that decision before implementation, as required by [`DOCUMENTATION_PROTOCOL.md`](../../docs/DOCUMENTATION_PROTOCOL.md).

## Decision

Host-originated `System` events are exported only through an admin-authorized journal/history path. They are not added to participant event streams, snapshots, `events_after`, or `project_event_for`. Capability results are host/admin-only. Presence, ticks and schedule-fire events are available through the same admin-authorized export, in the order in which the journal accepted them.

The export path must not give a join capability access to host-event payloads. Replaying a fixture must feed the recorded `System` events to the program in their original order relative to committed inputs. Checkpoint compaction must preserve the exportable ordered history window; when older history has been trimmed or cannot be reconstructed, a recording must be marked partial rather than claim completeness.

## Context and rationale

`WorldEventKind::System` already represents host-originated inputs with sequence and participant ID zero (`src/world/mod.rs`). The actor updates the program and then persists accepted host events as `JournalEntry::System` (`src/world/actor.rs`). In contrast, participant event pages and snapshots are projections over sequenced `core.events` (`src/world/core.rs`); making System events visible there would change the participant protocol and would not fit its sequence cursor. The live session also drops events at or behind its committed-sequence cursor (`src/world_session/mod.rs`).

The existing projection redacts chat and install seeds but passes other kinds through unchanged (`src/world/mod.rs`). That catch-all is not a sufficient visibility decision for capability results. Keeping all host events out of participant paths is the conservative boundary; an authorized export allows an operator to record the exact nondeterministic inputs without granting that data to joiners.

The fixture format already has an unsequenced `System` entry and its replay path already delivers one to the program (`src/world_fixture.rs`). The missing part is obtaining the interleaved journal history, not inventing a new fixture event kind.

## Initial implementation plan

1. Add an actor-consistent, admin-authorized export operation. It must validate the host admin token and return committed and host events in journal order; it must not be reachable through a participant capability or the participant event projection.
2. Track the exportable ordered event window from durable journal entries, including restore behavior. Keep it bounded consistently with world history and carry enough completeness information to distinguish a full recording from a trimmed one.
3. Extend checkpoint/compaction state to retain that ordered window, including the positions of host events between committed events. Old checkpoints without host history remain readable but cannot be reported as complete host-event recordings.
4. Update `id world record` and its client/protocol path to use the admin-authorized export for event history while keeping the existing fixture JSONL ordering and System replay representation. Mark trimmed exports incomplete; do not alter participant `events_after` behavior.
5. Add tests for admin authorization and participant non-disclosure, interleaving/order, checkpoint and compaction round trips, and a `territory` fixture that replays to the live records when its host events are included.

## Verification targets

- World-session tests reject missing or incorrect admin credentials and prove join capabilities cannot request/export System event payloads.
- Actor/core tests prove the admin history follows durable append order across inputs, presence, capability results, ticks and schedule fires.
- Store tests prove compaction/restore retains the ordered export window and reports loss as partial rather than silently dropping events.
- Fixture tests exercise `territory` host events interleaved with inputs and compare replayed records with the live program.
- Existing projection tests remain unchanged in intent: no System event is added to any participant serialization path.

## Limits

The export is for administrators, not a public event feed. Retention remains bounded; a history window that no longer reaches the relevant install boundary cannot be represented as a complete recording. This work does not make capability results visible to participants, expand participant event cursors, or promise complete history beyond the persisted/retained window.

---

## 2026-11-01T20-25-40Z Update: checkpoint recovery and bounds

New checkpoints store `open_presence` even when the set is empty. This distinguishes a known-empty set from a legacy checkpoint that has no presence metadata, and is covered by journal version 3. When a legacy checkpoint has complete admin history, restore reconstructs the exact set from that history. When its history is absent or incomplete, the exact set is unknowable: the snapshot may include a participant whose join was trimmed from the retained window.

For that ambiguous legacy case, restore conservatively replays one host-only `players.left` for every participant capability persisted in the checkpoint, including revoked capabilities. This clears stale guest-visible presence; it may also produce a redundant leave for a participant who was not present. Admin history remains incomplete, so the resulting fixture/export is still marked partial. This is a lossy cleanup, not a reconstruction claim. Programs using the `players` presence subscription should treat `left` as removal of that participant from their online set. If the legacy capability inventory exceeds the cleanup bound, restore is refused rather than emit an unbounded synthetic replay; the program snapshot is not activated and the journal is not compacted.

The serialized open-presence list has a stable 4,096-participant limit, independent of the world's configurable active-capability limit. The writer and reader enforce the same limit. If live presence exceeds it (for example, while revoked sessions are still closing), compaction is skipped and the complete append-only journal remains authoritative; no oversized checkpoint is written. Regression tests cover both legacy history shapes (absent and incomplete), known-empty new checkpoints, and a revoked-but-still-connected capability over a lowered participant quota. They verify stale snapshot state is cleared, synthetic cleanup does not mark history complete, and every emitted checkpoint restores.
