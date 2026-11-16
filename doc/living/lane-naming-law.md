# Lane naming law

Living document · Status: shipped (2026-11-10) · Register entry F4.9 · Tracking epic: developing-today/code#8805

## Purpose

- Give every fleet lane a short name that is coherent on its own.
- Separate what a lane does (its scope) from where it surfaced.
- End ad-hoc lane names that carried platform or tool labels.
- First asked: 2026-11-10 21:29–21:35 CST, in Drew's rulings that evening.

## What landed (features, and how each works)

- The law: a lane name is ONE scope word plus ONE shard letter.
- Scope words recorded in the register: `code`, `not-code`, `any`.
- The amendment at 21:35 CST set `not-code` instead of `other`.
- The shard letter is fresh and unique within its scope group.
- Multiple letters are allowed for separating lanes, provided the name
  still makes sense besides the shard.
- Platform is metadata, never in the name:
  - surface origin is passed separately as platform metadata.
- Lanes are never labelled "claude":
  - Drew's ruling: "what does claude have to do with anything".
- The scheme was accepted at 21:33 CST: "well that's at least sort of
  coherent", then amended as above.
- The law applies to both surfaces.
- It applies including OpenChamber's 12 identically-named tasks.
- Execution was via the lane registry; there was no dedicated issue.
- The rename was recorded complete: "Lane rename + trims: COMPLETE"
  per the 2026-11-11 record.

## Rough architecture

- The broker lane registry is the design artifact and the store.
- Registry state at final state: 36 rows, 17 enabled.
- Each row carries the lane name built from scope word + shard letter.
- Surface origin travels separately as platform metadata.
- Naming is enforced at registry level, not in lane names themselves.
- Not recorded: any separate naming service or validator beyond the
  registry.

## Status

- design_status in the register: built-into-running-system.
- implementation_status in the register: shipped.
- Final on 2026-11-10; rename and trims complete by 2026-11-11.

## Next steps — immediate / near-term / long-term

- Immediate: resolve the registry anomaly in Open follow-ups below.
- Near-term: not recorded.
- Long-term: not recorded.

## Open follow-ups

- Anomaly at record time: broker rows `openchamber-a` /
  `openchamber-b` reappeared with enabled=1 after deletion.
- That reappearance was being investigated at record time.
- No other follow-ups are recorded for this entry.

## Future ideas not yet specced

- None recorded in the register entry.

## Links (register entry, related issues/PRs named in the register entry)

- Register entry: F4.9, in the design register (REGISTER.md).
- Recorded on: developing-today/code#8097.
- Tracking epic: developing-today/code#8805.
- Executed via: the broker lane registry (no dedicated issue).
