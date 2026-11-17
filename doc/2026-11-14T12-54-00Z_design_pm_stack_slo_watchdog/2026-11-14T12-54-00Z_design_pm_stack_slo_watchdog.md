# PM-stack SLOs + self-healing watchdog — design note (#1260)

- Status: design record for issue #1260, part of epic #1184 (PM stack
  coherence). Design only — no implementation in this PR.
- Builds on: #1148 (staleness watchdog — CLOSED; it surfaces dead/stale
  stack inputs as NEEDS ORCHESTRATOR instead of silent decay),
  #6031 (browser-dependent quota watchdog — open design), and the
  deployed stack shape in `pm-stack/README.md` (Aggregator :4870,
  pm-dispatch :4879, ready-for-agent :6056, Pad :7777, Grist :8484,
  pm-tunnel, the comment projector, and the feed collectors).
- What this adds over #1148: #1148 detects and reports. This design
  declares per-facet SLOs with error budgets, and lets the watchdog
  _remediate_ the known failure classes itself inside hard guardrails,
  escalating to Drew only when remediation fails or budget burns.

## 1. SLO definitions per facet

Windows are rolling 30 days unless stated. A facet is "bad" for a
one-minute sample when its signal is past threshold; error budget is
the allowed bad minutes in the window, and budget remaining is
displayed as a percentage plus a burn rate (bad minutes in the last
hour ÷ budget ÷ window share). The health page shows, per facet:
current signal value, SLO target, budget remaining %, 1 h burn rate,
and state (ok / burning / exhausted).

| Facet                   | Signal                                                                        | SLO target                                           | Budget (30 d)                                                              |
| ----------------------- | ----------------------------------------------------------------------------- | ---------------------------------------------------- | -------------------------------------------------------------------------- |
| Feed age                | age of newest fleet/activity feed item the Aggregator serves                  | p95 age ≤ 10 min (7 d window) and no sample > 30 min | 0.5% of minutes (≈216 min) above 10 min; any sample > 30 min counts double |
| Quota read age          | age of the newest quota snapshot the dashboard renders                        | ≤ 75 min at p99 (hourly refresh + slack)             | 1% of minutes (≈432 min)                                                   |
| Orchestration freshness | age of the orchestration state (assignments, queue snapshot) behind the pages | ≤ 90 min at p99                                      | 1% of minutes (≈432 min)                                                   |
| Aggregator up           | HTTP 200 on the Aggregator health route via tailnet                           | 99.5% of minutes                                     | ≈216 min down                                                              |
| Intake (pm-dispatch) up | HTTP 200 on the dispatch health route, loopback + tunnel                      | 99.5% of minutes                                     | ≈216 min down                                                              |
| Projector up            | projector heartbeat / last successful projection age ≤ 15 min                 | 99.5% of minutes                                     | ≈216 min                                                                   |
| Pad / Grist up          | container healthy + HTTP 200                                                  | 99% of minutes (stateful, see §2)                    | ≈432 min down                                                              |
| Tunnel up               | pm-tunnel connected + public intake route answers                             | 99.9% of minutes                                     | ≈43 min down                                                               |

Budget computation is deliberately boring: the watchdog samples each
signal once per minute into an append-only SLO journal; remaining =
1 − (bad minutes in window ÷ budget minutes). No interpolation —
missed samples count as bad for `up` facets and as unknown (excluded,
but flagged) for age facets.

## 2. Remediation catalogue

Only idempotent, reversible actions on stateless producers are
auto-fixable. Every action carries the same frame: preconditions
checked first (unit exists; no deploy in progress — the deploy lock /
unit-file mtime is untouched for 10 min; the facet is actually bad),
then the action, then §3 verification.

| Failure                                                      | Action                                 | Attempt cap    | Cooldown |
| ------------------------------------------------------------ | -------------------------------------- | -------------- | -------- |
| Collector stopped/stale feed                                 | restart the collector user unit        | 3 per incident | 15 min   |
| Feed not pushed (collector fine, age grows)                  | re-run the feed push producer once     | 3 per incident | 15 min   |
| Aggregator down or wedged (port answers, health route fails) | restart `pm-aggregator` user unit      | 3 per incident | 20 min   |
| pm-dispatch down                                             | restart `pm-dispatch` user unit        | 3 per incident | 20 min   |
| Projector stalled (heartbeat stale, unit active)             | restart the projector unit             | 2 per incident | 20 min   |
| Tunnel disconnected                                          | restart `pm-tunnel` user unit          | 2 per incident | 30 min   |
| Quota read stale (producer job failed)                       | re-run the quota refresh producer once | 2 per incident | 30 min   |

Guardrails applying to the whole catalogue:

- **Global caps:** at most 6 remediation actions per facet per day
  and 15 across the stack per day; beyond that the watchdog only
  escalates. A per-facet circuit breaker opens after 3 exhausted
  incidents in 24 h (action clearly not fixing the cause) and stays
  open 24 h.
- **One action in flight:** never remediate two facets at once except
  the correlated case in §4; a restart waits for its §3 verdict
  before any further action on any facet.
- **Stateful services are NOT in the catalogue:** Pad and Grist
  (containers with data) are never restarted automatically — the
  watchdog escalates with diagnostics instead. Same for any database.
- **No blind retries:** an action that fails verification is not
  repeated inside its cooldown even if the cap is not yet hit.

## 3. Verification after remediation

The watchdog proves the fix before standing down, using the facet's
own SLO signal — not the action's exit code:

- Restart actions: health route returns 200 (or the projector
  heartbeat advances) within a 5-minute verification window.
- Re-push / refresh actions: the relevant age signal _decreases_ —
  the newest served item / snapshot timestamp is newer than the
  pre-action value — within 10 minutes.
- A verified fix closes the incident; an unverified one consumes an
  attempt and starts the cooldown clock.

**Logging:** every detection, action, and verdict is appended to the
watchdog journal (append-only JSONL kept with the Aggregator's data,
one line per event: facet, signal value, action, attempt n, verdict,
timestamps) and mirrored as an event in the actions feed the stack
already maintains for agent actions — the same surface #1148 reports
into, so a human reviewing NEEDS ORCHESTRATOR sees detections,
remediations, and escalations in one place.

## 4. Escalation

Two channels: **page Drew** (push, per the PWA/push design) and
**NEEDS ORCHESTRATOR** (journal + feed event, no push).

Escalate to NEEDS ORCHESTRATOR (no page) when: an incident is
detected and remediation is in progress, an action fails verification
but attempts remain, or a non-user-facing facet (projector, quota)
burns at ≤ 2× rate.

Page Drew when any of these holds:

- Remediation is exhausted (cap or circuit breaker) and the facet is
  still bad.
- A user-facing facet (Aggregator, Intake, Tunnel) is down and the
  first verification fails — Drew hears about user-facing outages
  after one failed fix, not three.
- Two or more facets are bad simultaneously (correlated failure —
  host, network, or tunnel; per-facet remediation is the wrong tool).
- A stateful service (Pad, Grist, any database) is down — always a
  page, never an auto-action.
- **Budget effect:** when a facet's budget remaining drops below
  25%, its page threshold tightens to "first failed verification"
  regardless of facet class; when budget is exhausted, the watchdog
  pages on _detection_ and performs at most one remediation attempt
  before standing down — a burning budget means the stack has used
  up its silent-failure allowance for the month.

## 5. Safety — the never-do list, and watchdog self-failure

The watchdog must never:

- write data: no database mutations, no edits to issues/PRs/pages
  beyond its own journal, feed events, and escalation comments;
- deploy: no `git pull`, no nix rebuild/switch, no unit-file or
  config changes (re-rendering config is a deploy);
- touch credentials: no secret reads beyond its own scoped token, no
  rotation, no permission changes;
- restart or signal stateful services (Pad/Grist containers, any
  database) or kill processes outside the catalogue's named units;
- act during a deploy (deploy lock present) or outside the laptop
  host it is scoped to;
- remediate the same incident silently past the caps in §2 — caps
  are code, not guidance.

**Its own failure:** the watchdog writes a heartbeat line to its
journal every run (5-minute cadence). A separate, deliberately
minimal checker — the existing page-health-push pattern, which
already watches page health every 5 minutes — reads the heartbeat;
two missed cycles raise NEEDS ORCHESTRATOR, three page Drew (a dead
watchdog is itself a user-facing reliability outage). The watchdog's
journal is append-only so a wedged watchdog cannot rewrite its
history, and the checker shares no code with the watchdog beyond
reading that file.

## 6. Open questions needing Drew (explicit)

1. **Page channel:** is the page target the PWA/push design, or the
   existing page-health-push → chat path until the PWA lands?
2. **SLO ratification:** the targets and budgets in §1 are proposals
   from current cadences — ratify, or set different numbers (in
   particular the Tunnel's 99.9% and the Feed's 10-minute p95).
3. **Tunnel restarts:** a pm-tunnel restart briefly drops the public
   intake route — acceptable inside the §2 caps, or should tunnel
   remediation page-first?
4. **Stateful restarts:** §2 escalates Pad/Grist always. Confirm no
   exceptions (e.g. a Grist container that is up but wedged)?
5. **Journal retention:** 90 days proposed for the SLO/remediation
   journal — confirm, since budget audits may want a full quarter.

## 7. Implementation follow-ons (filed under epic #1184)

1. SLO sampler + journal + health-page budget display (§1).
2. Remediation executor for the §2 catalogue (caps, cooldowns,
   circuit breakers, deploy-lock precondition).
3. Post-remediation verification + journal/feed mirroring (§3).
4. Escalation router: NEEDS ORCHESTRATOR vs page, with the §4
   budget-aware thresholds.
5. Watchdog heartbeat + self-failure checker (§5).
