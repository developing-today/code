# Server-observed chat history and grantable platform modules

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md), the [capability protocol module](../../pkgs/id/src/world_caps.rs), and the [RocCap record](../2026-10-16T16-00-00Z_feature_roc_cap/2026-10-16T16-00-00Z_feature_roc_cap.md).

## Intent

1. Communication worlds could send chat (`chat.say`) but could not read it, so a
   program could not show logs, summarise, or greet people in context. Add a
   `chat.history` capability request.
2. The platform modules `RocJson`, `RocGrid`, and `RocCap` shipped after the
   catalog did, so `id world caps --grant RocJson` was refused as an unknown
   name: a world could only reach them through `*`. Add them to the catalog.

## Decisions

- **History comes from the retained-events buffer.** `chat.history` scans the
  world's recent events for chat lines and returns the newest `count` of them,
  oldest first, each with `sequence`, `participant`, and `text`. The buffer is
  already bounded by `WorldLimits::retained_events` and snapshot bytes, so a
  request can never see unbounded history.
- **Bounded count.** `count` defaults to 16 and is clamped to
  `MAX_CHAT_HISTORY_LINES` (64). `count: 0` returns an empty list rather than a
  surprise default.
- **History requires the CHAT scope.** `allows_effect` now maps `chat.history`
  to the CHAT scope alongside `chat.say`, so a spectator with `join` and
  `input` cannot cause the program to read chat lines. An ungranted request is
  refused and counted like any other capability.
- **Module names are catalog names.** `RocJson`, `RocGrid`, and `RocCap` join
  `Key` and `Screen` in `CATALOG`, so grants, revocations, and ceilings can
  name them explicitly. They remain compile-time import gates; a runtime
  request for them is still `unknown`.

## Acceptance

- `chat_history_returns_the_newest_lines_oldest_first` covers order, the
  newest-N cut, and `count: 0`.
- `a_granted_chat_history_request_is_served_over_the_ledger` drives the Mock
  program through `chat.say` and then `chat.history` and asserts the ledger's
  allowed count for `chat.history`.
- `scope_names_and_effects_follow_the_scopes` asserts GUEST may read history
  and a spectator may not.
- `grant_names_must_exist` accepts the new names.
- `pf.RocCap` gains `history_request` and `history_lines`; the compile test now
  issues both a `chat.say` and a `chat.history` request and asserts both are
  re-announced through `wants`.
- Full library suite: 1,320 passed, 4 ignored.

## Follow-ups

- Paging with a `since` sequence cursor for longer transcripts.
- A checked-in example that shows a chat client built on `chat.history`.
- Participant display names in history lines once names are stable per seat.
