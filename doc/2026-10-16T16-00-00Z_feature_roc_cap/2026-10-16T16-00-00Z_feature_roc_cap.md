# RocCap: shared request bookkeeping for the capability protocol

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md), the [RocGrid record](../2026-10-16T15-00-00Z_feature_roc_grid/2026-10-16T15-00-00Z_feature_roc_grid.md), and the [capability protocol module](../../pkgs/id/src/world_caps.rs).

## Intent

Every capability-using app re-derived the same bookkeeping: a counter for
request ids, a pending list, matching a `result` frame back to the request that
asked, and the argument documents for `chat.say`, `time.now`, `players.list`,
and `world.info`. `RocCap.roc` is that bookkeeping as a tested library,
exposed by the platform as `pf.RocCap`.

## Decisions

- **Ids come from the session, not the app.** `issue(session, cap)` returns the
  session with one more pending entry plus the `id` to embed. Ids are `r0`,
  `r1`, … in first-issue order, so they are identical under journal replay.
- **The request document is part of the session.** `attach(session, id,
document)` stores it, and `requests(session)` returns the outstanding
  documents for `wants`. This matters after replay: the model must be able to
  re-announce exactly the requests it had in flight, including their arguments,
  not just their ids. `from_pending(counter, documents)` rebuilds a session
  from a snapshot.
- **Routing is by id and capability.** `answered(session, id)` returns the
  capability that issued an id, or `""` when the id is not pending. A result
  from an older model of the same world is routed exactly as the model that
  sent it would have routed it, and unknown ids are ignored instead of
  corrupting the current session.
- **No hidden effects.** The module only builds and matches documents. The app
  decides what to append to `wants` and when to send, so a program can choose
  request-once, request-per-tick, or user-driven flows.
- **Documents are `RocJson` values.** `chat_request`, `now_request`,
  `players_request`, and `world_request` compose with `RocJson.encode` and the
  app's own `wants` document.

## Acceptance

- `the_capability_helper_issues_matching_requests` compiles a small app that
  issues and attaches a `chat.say` request, runs the resulting module in the
  sandbox, and asserts the encoded document has `id: "r0"`, `cap: "chat.say"`,
  and the seed-derived `args.text`; it also asserts `wants` re-announces the
  same document through `RocCap.requests`. It skips when `roc` is unavailable,
  like the other compile tests.
- `lounge` now keeps its whole request lifecycle in `pf.RocCap` and its
  checked-in test passes with the `r0`-based ids.
- The platform still builds every checked-in example, and all 21 sandbox tests
  pass.

## Follow-ups

- A subscription helper (`time.tick`, `players`) belongs in the same module
  once an app needs it.
- Result payload helpers (`players.list` rows, `time.now` values) could join
  the module so apps stop re-reading `value` objects by hand.
