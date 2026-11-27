# content-throttle (hatch#13)

The remaining phases of the fleet content-creation throttle, on top of
the phase-1 creation ledger (`tools/content-ledger/`, hatch#3).
Design: developing-today/code#2013 (record PR code#2226); tracking
record code#2227; this repo's issue: dezren39/hatch#13.

## What is here

- `content_throttle.py` — the shared throttle wrapper (spec item 2),
  the circuit breaker (item 4), and the gap detector (item 6).
  Budgets, intact from the design: 45/min and 300/hour account-wide,
  ≥ 1 s between POSTs, ≤ 8 in flight, per-lane 50/hour sustained with
  idle share reclaimable, 75/hour held for priority-10 breakage work,
  bulk programs ≤ 20 issues per 30 minutes.
- `../gh-post` routes every post through the throttle gate and adds
  `gh-post note` — the comment-reduction change (item 5): one live
  note per task target, edited in place, instead of one comment per
  checkpoint. Target ≤ 2 comments per task (claim + live note).
- `test_content_throttle.py` — stub-`gh` tests, no network.

## Rollout state (read this before flipping anything)

- **Throttle mode: `warn`** — the wrapper computes what it _would_
  delay and appends it to `~/.local/state/fleet/content-throttle-warn.jsonl`,
  but delays nothing. The warn week started when the state DB was
  first initialised; `content_throttle.py mode` prints the mode and
  the warn-since date.
- **After one week**, review the warn log (what would have been
  delayed, which constraint bound, whether any of it was work that
  should not have waited), then flip:
  `python3 tools/content-throttle/content_throttle.py mode enforce`.
  In enforce mode a caller whose wait exceeds the max wait (default
  30 min) is _visibly refused_ (exit 6) — never silently dropped.
- **The circuit breaker is live in both modes.** It reacts to GitHub's
  own 403/429 refusals: honour `retry-after` (one retry), else 60 s
  then exponential ×2 over 5 attempts; 5 consecutive failures open the
  circuit for 15 minutes and callers get a park-and-resume brief.
  This is deliberate: GitHub warns that continuing while limited can
  ban the integration, so the breaker is not part of the warn-only
  staging.
- **Success measure after enforcement** (code#2227): no creation 403
  for 14 days, account-wide hourly total under 300, queue throughput
  unchanged.

## Gap detector → fleet monitor

Spec item 6 belongs in the fleet monitor. There is no monitor in this
repo; the integration surface is:

```
python3 tools/content-throttle/content_throttle.py gap-check
```

Exit 0 = no gap. Exit 2 = alert: queued filing work exists (stuck
ledger reservations, waiting throttle slots, prepared outbox entries)
and no creation has been seen for > 5 minutes. Alerts are also
appended to `~/.local/state/fleet/content-throttle-alerts.jsonl`
(circuit openings land there too). The monitor should run it on its
cycle and alert on exit 2.

## Failure behaviour

A throttle or ledger failure refuses the post _before_ any API call
(exit 5) — never a silent fall-back to unthrottled filing. That is the
phase-1 rule applied to pacing, and it is a deliberate availability
coupling: if the ledger is down, fleet filing stops, loudly.
