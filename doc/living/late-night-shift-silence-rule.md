# Late-night shift silence rule

Living document · Status: shipped (2026-10-25) · Register entry X13 · Tracking epic: developing-today/code#8805

## Purpose

- Stop overnight work stalling when Drew does not reply.
- Drew's words on 2026-10-25 17:39 CDT: "you should just build
  something as long as it's reversible".
- Silence on the nightly priorities list no longer means no work.

## What landed (features, and how each works)

- A standing rule for the late-night shift:
  - No reply to the nightly priorities list does not block work.
  - Muse starts the top-ranked fully-reversible candidates.
  - Those candidates proceed within the $0 cap.
  - Irreversible items wait for morning.
  - Spending items wait for morning.
  - Drew-reserved items wait for morning.
- The rule is encoded in the late-night-shift-priorities job
  body, step 6.
- It is not tracked as a GitHub issue — by design, per the
  register.

## Rough architecture

- Home: the `late-night-shift-priorities` cron job definition,
  step 6 of its body.
- Also recorded in:
  `~/memory/topics/memory-full-text-2026-10-29.md`
- Decision inputs at run time, per the rule itself:
  - the ranked nightly priorities list
  - whether a candidate is fully reversible
  - whether a candidate stays within the $0 cap
  - whether a candidate is Drew-reserved
- Tracking in the register: NEVER-ISSUED.

## Status

- design_status in the register: built-into-running-system.
- implementation_status in the register: shipped.
- Adopted 2026-10-25 and operating.
- Invoked by later overnight runs, for example the 2026-11-05
  run in the wider record.
- Triage in the register: (b) DONE-WITHOUT-ISSUE.

## Next steps — immediate / near-term / long-term

- Immediate: not recorded.
- Near-term: not recorded.
- Long-term: not recorded.

## Open follow-ups

- None recorded in the register entry.

## Future ideas not yet specced

- None recorded in the register entry.

## Links (register entry, related issues/PRs named in the register entry)

- Register entry: X13, in the design register (REGISTER.md).
- Job: late-night-shift-priorities (rule at step 6 of its body).
- Also recorded in: `~/memory/topics/memory-full-text-2026-10-29.md`
- Tracking epic: developing-today/code#8805.
- Tracking in the register: NEVER-ISSUED (no GitHub issue).
