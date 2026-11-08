# PM-stack chaos drills — schedule and interlock (#1283, epic #1184)

The PM stack's failure modes are silent by default: a dead collector,
an aged feed, or a down component service have all presented as
"everything is fine" at least once. The drills in this directory make
the failure deliberate, on a cadence, and verify the honesty signals
actually fire — the degraded banner / health state, and the watchdog
path — instead of trusting that they would.

All drills are **laptop-only** (host `amd`): the collector, the
component services, and the aggregator are laptop user units since
#3362/#3363. A drill refuses to run on any other host.

## The drill set

| Drill | Script | Fault injected | Named signals verified | Result file |
| --- | --- | --- | --- | --- |
| kill-collector | `scripts/kill-collector-drill.sh` | `fleet-feed-collector.timer` stopped | feed age crosses `FEED_STALE_MS` (5 min, #1148) in `/api/health`; restoration = feed fresh again | `kill-collector-result.json` |
| age-feed | `scripts/age-feed-drill.sh` | feed `generatedAt` aged while the collector runs | feed reported stale in `/api/health` despite a live collector; restoration = fresh feed | `age-feed-result.json` |
| drop-service | `scripts/drop-service-drill.sh` | one component user unit stopped (default `pm-projector.service`) | the unit's `/api/health` services entry reports `ok=false` **and** the top-level `degraded` flag is true; restoration = entry `ok=true` again | `drop-service-result.json` |

Result files live in the drill state dir
(`~/.local/state/pm-drills/` on the laptop). A passed drill posts its
result as a comment on #1283. A drill whose expected signal does
**not** fire files a new issue under epic #1184 with the captured
health evidence — a silent honesty signal is a finding, not a drill
error.

## Schedule

Weekly rotation, one drill per week, each in its own quiet window so
two drills never contend for the stack in the same window:

| Drill | Window (laptop local) | Unit |
| --- | --- | --- |
| kill-collector | Sunday 04:17 | (slot reserved; laptop-hosted timer not yet installed) |
| age-feed | Wednesday 03:43 ± 15 min | `systemd/age-feed-drill.timer` |
| drop-service | Friday 04:07 ± 15 min | `systemd/drop-service-drill.timer` |

The systemd units in `systemd/` are the schedule of record; they are
installed deliberately on the laptop, never by a drill itself. Any
drill can also be run on demand by invoking its script directly on
the laptop — the interlocks below make a manual run and a scheduled
run mutually exclusive.

## Interlocks (every drill, before anything is touched)

A drill that refuses to start touches **nothing** — the #2865 rule
(the kill-collector drill originally restored, i.e. started services,
on refusal paths; its restore trap is now gated on the fault actually
having been injected, and the drop-service drill is built the same
way from the start):

1. **One drill at a time.** The drop-service drill takes the shared
   lock `pm-drill.lock` in the state dir, its own `drop-service.lock`,
   and try-locks the legacy per-drill lock files
   (`kill-collector.lock`, `age-feed.lock`). The two older drills
   predate the shared lock and take only their own lock files, so
   until they are migrated to also take `pm-drill.lock`, the shared
   lock alone cannot exclude them — the try-lock checks bridge that
   gap. Migration for the older drills: take `pm-drill.lock` first,
   then their own lock, in that order everywhere (the drop-service
   drill's order), so no lock-ordering deadlock is possible.
2. **No drills during a real incident.** If the incident flag
   (`~/.local/state/pm-drills/incident.flag` by default) exists,
   every drill refuses. Raise the flag before working a real PM-stack
   incident; clear it when the incident closes.
3. **A fault that is already present is an incident, not a drill.**
   Each drill verifies its target is healthy at start (collector
   timer active and feed fresh; feed current; target unit active and
   its health entry up) and refuses otherwise.
4. **Restoration is armed before the fault, and always runs.** Once
   a drill has injected its fault, a trap on EXIT/INT/TERM restores
   everything it touched, and the drill then confirms restoration
   with a fresh `/api/health` read before reporting. A drill whose
   restoration cannot be confirmed reports `failed` even if its
   signal fired.

## Changing the set

- New drill: mirror an existing script's structure (host guard,
  interlocks above, gated restore trap, JSON result, pass-comment /
  finding-issue reporting), add its row here, and give it a window
  no other drill uses.
- Never run two faults at once, and never point a drill at the
  aggregator itself — it is the observer every drill verifies
  through.
