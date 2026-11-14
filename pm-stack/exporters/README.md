# pm-stack/exporters — browser-dependent quota sources (#6032, design #6031)

Producers and the watchdog core for quota sources whose data is only
reachable through a logged-in website (Mistral first; Claude console and
Open Design are named later adopters). The design lives in #6031; the
core implementation is #6032.

## What is here today

`src/watchdog-state.ts` — the **decision layer** of the watchdog core:

- the failure-class vocabulary (`session-expired`, `cookie-missing`,
  `endpoint-changed`, `browser-down`, `cdp-unreachable`, `rate-limited`,
  `unknown`) and the one place a class maps to a repair rung;
- the repair ladder decision (`decide`) with per-rung attempt caps
  (`MAX_ATTEMPTS_PER_RUNG`), a disk-honored rate-limit cooldown, and
  escalation once every rung is spent;
- the persisted per-source health record (`SourceHealth`) with a bounded
  repair log, and `coerceHealth` for reading a state file back without
  trusting it;
- `sanitizeDetail`, which strips credential-shaped text before anything
  reaches the state file or a log.

It is pure: no I/O, no browser, no secrets, no provider names. That is
deliberate — the ladder is the part worth testing exhaustively, and the
testing doctrine (AGENTS.md) asks for property- and model-based coverage
of exactly this kind of state machine.

`test/watchdog-state.test.ts` names its oracles: PBT invariants over
generated tick sequences (attempt caps, log bound, monotonic failure
counts, cooldown honored, no secret survives), an MBT reference model of
the ladder stepped in lockstep with the implementation, and the four
acceptance fixtures (broken-source walk, rate-limited cooldown, human
challenge, quiet-when-healthy).

Run them:

```bash
cd pm-stack/aggregator && bun test ../exporters/test/watchdog-state.test.ts
```

## What is NOT here yet (#6032 remainder)

- `src/watchdog.ts` — the oneshot entry point: read the state file, read
  each source's `~/atlas/<provider>-export/latest.json` plus the gatherer
  heartbeat, classify, call `decide`, run the chosen rung, write the state
  file atomically (tmp+rename), append `quota-watchdog-history.jsonl`.
- `src/adapters.ts` — the adapter loader (origins allowlist, CDP port,
  data-path order, cookie names, login hints, class map, thresholds) and
  `adapters/` itself.
- The repair executors for rungs 2 and 3 (CDP session re-validation,
  browser re-login driver), which stop dead at a human-only challenge and
  hand off to `recordRungOutcome`.
- The rung-4 escalation hand-off module (#6033):  plans the deduped queue-task + incident-issue effects behind injected effectors, records the refs on the incident, plans recovery (comment + close + complete), and renders the NEEDS DREW surfacing line (immediate when human-blocked, after 24 h unresolved otherwise). Production effectors (fleet CLI / gh) are wired by the entry point below.
- Producer scaffolding under `pm-stack/exporters/` from the retired Meta
  producer pattern, and the gatherer `FILE_EXPORTS` registration.
- `pm-stack/systemd/` oneshot + timer units on the gatherer cadence.
