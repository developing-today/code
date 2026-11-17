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

`src/watchdog-escalation.ts` — the **rung-4 hand-off** (#6033): plans the
deduped queue-task + incident-issue effects behind injected effectors,
records the refs on the incident, plans recovery (comment + close +
complete), and renders the NEEDS DREW surfacing line (immediate when
human-blocked, after 24 h unresolved otherwise).

`src/adapters.ts` + `adapters/` — the **adapter loader**. An adapter is a
per-provider JSON file (origins allowlist, CDP port, producer document
path and/or gatherer heartbeat key, cookie names — names only, login
hints, class/rung map overrides, thresholds, owning repo). The loader
validates structurally and reports defects per file instead of throwing:
a broken adapter degrades to "that source is unobserved", never to a
crashed tick. `adapters/fixture.json` is the synthetic adapter the
acceptance tests and demos run on; real provider adapters are follow-ons
(Mistral's is the openchamber-side follow-on named in #6032).

`src/watchdog.ts` — the **oneshot entry point**. One tick: load adapters,
read the state file back through `coerceHealth`, observe each source
(producer document + gatherer heartbeat + a live CDP liveness probe),
classify — including the classes the producer cannot see itself (missing
document, stale read while the browser lives, dead CDP endpoint) — call
`decide`, execute the chosen rung, run rung-4 escalation/recovery effects,
write the state file back atomically (tmp+rename), and append one line per
source to `quota-watchdog-history.jsonl`. Nothing decision-relevant lives
in process memory (#5928). Rung executors:

- rung 2 — CDP session re-validation: `/json/version` + `/json/list`, an
  allowlist origin open, adapter cookies present **by name** via the CDP
  websocket (values never enter the process); a dead endpoint gets one
  bounded supervised restart of the adapter's browser unit;
- rung 3 — re-login driver: opens the adapter's login URL (allowlist
  origins only), stops dead at a human-only challenge marker and marks
  the incident human-blocked via `recordRungOutcome`, otherwise confirms
  the session by cookie presence.

Browser repairs run only with `QUOTA_WATCHDOG_LIVE_REPAIRS=1` and the
rung-4 production effectors (fleet queue enqueue / `gh`) only with
`QUOTA_WATCHDOG_LIVE=1`; without them the tick journals its plans as dry
runs and creates nothing. The systemd unit sets both.

`producers/producer-template.mjs` — the forkable producer scaffold
distilled from the retired Meta producer: timeout-bounded CDP plumbing,
tab ownership (a producer closes only tabs it created), failure-class
exit codes with a written failure document, atomic `latest.json` +
`history.jsonl`. Registering a fork with the gatherer means adding its
document to the aggregator's file-export inputs (`FILE_EXPORTS` in the
pm-aggregator package); the watchdog watches the document directly either
way. `test/fixtures/fixture-producer.mjs` is the scenario-driven stub the
acceptance tests use in place of a browser producer.

`../systemd/pm-quota-watchdog.service` + `.timer` — the oneshot + timer,
on the gatherer's 15-minute rhythm offset to `*:3/15` so each tick judges
fresh producer documents. Installed on the laptop as user units; the
service runs this directory as deployed under `~/pm-stack/exporters/`
with adapters from `~/.config/quota-watchdog/adapters/`.

## Tests

`test/watchdog-state.test.ts` names its oracles: PBT invariants over
generated tick sequences (attempt caps, log bound, monotonic failure
counts, cooldown honored, no secret survives), an MBT reference model of
the ladder stepped in lockstep with the implementation, and the four
acceptance fixtures (broken-source walk, rate-limited cooldown, human
challenge, quiet-when-healthy).

`test/watchdog.test.ts` covers the entry point end to end against a
scratch tree: the same four acceptance fixtures driven through `runTick`
(the ladder walk escalates exactly once and dedupes on later ticks;
recovery comments/closes/completes the hand-off), cross-invocation
persistence (every tick re-reads the state file from disk — the #5928
oracle), the classification of missing-document / stale-read / dead-CDP
observations, and the adapter loader's defect handling.

Run them:

```bash
cd pm-stack/aggregator && bun test ../exporters/test/
```
