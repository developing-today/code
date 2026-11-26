# Assessment: `main-ci-failure-fix-trigger` OpenChamber scheduled-task port (#15346)

Assessment of record for developing-today/code#15346, the wave-2
assess-first follow-on for `main-ci-failure-fix-trigger` from the
Muse→OpenCode cron wrapper inventory (#14222, inventory record PR
#15351, row 12). Parent programme: #2174. Compiled 2026-11-26 by the
xaltha lane (Muse sandbox seat).

**Decision: NO-PORT for now.** The gating check fails (gap 2 still
open, gap 4 has no closing record), and assessment question 1 has no
answer that preserves the dedupe guarantee with the state homes that
exist today. The job stays on Muse cron. The conditions that would
flip this decision are listed at the end; all of them are already
tracked by #2174 / #2072, so no new follow-on issue is filed.

## The job, as recorded

`main-ci-failure-fix-trigger` (fleet schedule registry,
`doc/fleet-schedule-registry.md`): interval 10m, enabled, owner
goal:keep-the-four-repos-healthy. Per #15346 and inventory row 12 it
detects new main-branch CI failures across the four fleet repos,
files/reuses the tracking issue, enqueues the fix task, and dispatches
OpenCode fix sessions through the conductor mechanics; it also runs
the CI run-silence probe. It runs on two seats: the primary seat and
Felix's sandbox seat (inventory, live `cron.list` on Felix's seat,
2026-11-25). Its dedupe state lives in the repo-health goal workspace
(#15346 body).

Corroboration from this seat, 2026-11-26: the job is **not** in
xaltha's live `cron.list`, and this seat's copy of the
keep-the-four-repos-healthy goal workspace holds no definition or
state file for it. The job's state is seat-local: each seat that runs
it keeps its own copy.

## Gating check (#2174 monitoring gaps) — verified 2026-11-26

| Gap                                                               | State                       | Evidence                                                                                                                                                                                                                                                                                            |
| ----------------------------------------------------------------- | --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1 — per-run history                                               | **Closed**                  | PR #8437 merged 2026-11-14 (`feat(pm-stack): scheduled-history collector — per-run history for OpenChamber scheduled tasks (#2174 Gap 1)`); suite wired into CI by PR #11011                                                                                                                        |
| 2 — token accounting (OpenCode-side exporter, #2072 work)         | **Still open**              | #2072 is OPEN at check time; no merged exporter PR found. #2174's own pilot record states the position: per-run token capture still flows through OpenCode's own session store, and "the #2072 exporter remains the durable fix"                                                                    |
| 3 — failure alerting / watchdog                                   | **Closed**                  | PR #11158 merged 2026-11-17 (`feat(scheduled-history): Gap 3 watchdog over OpenChamber run history (#2174)`)                                                                                                                                                                                        |
| 4 — server residency (OpenChamber server always-on on the laptop) | **No closing record found** | #2174 is OPEN and its acceptance (gaps 1–4 closed and demonstrated on a pilot) is unmet; a search of issues and PRs found no residency confirmation record. The 2026-10-29 pilot comment lists residency confirmation among the build tasks Drew's direction created, with no completion note since |

#2174's gating, as restated on #15346, requires gaps 1–4 closed
before wave migration proceeds. Gaps 2 and 4 do not meet that bar at
assessment time, so a port is not eligible on gating alone —
independent of the three questions below.

## Question 1 — where does the detection state live on the laptop? (gap 5)

The detection half is `gh` polling plus state-file watermarks: what
has already been seen, per repo and per workflow, so a failure is
detected once. Today those watermarks live in the repo-health goal
workspace **on each seat that runs the job** — sandbox-resident,
seat-local files (see the corroboration above: a seat that does not
run the job has no copy at all).

The dedupe guarantee — one live fix track per failing workflow per
repo — currently rests on the two shared layers underneath the
watermarks: tracking-issue reuse on GitHub and the fleet queue's
dedupe keys (Supabase `agent.tasks`: packets, dedupe keys, lane
claims — #2072's record of the existing pieces; the queue's atomic
claim was demonstrated non-duplicating by the #2174 pilot,
2026-10-29).

A laptop scheduled session has three candidate state homes:

- **A laptop-local file** (the OpenChamber scheduler's own pattern —
  its task state is a laptop JSON holding last-run state). This is
  the wrong home: it adds a third, divergent watermark copy while
  the Muse seats still run the job additively (#2174's rollout is
  additive; the Muse side is not removed first). Divergent
  watermarks mean the same failure is "new" on more than one seat;
  the queue dedupe key still protects the enqueue, but detection —
  including tracking-issue filing, which happens before enqueue —
  is exactly where duplicates would appear.
- **The sandbox goal workspace, reached remotely.** Keeps one home,
  but makes a laptop-resident job depend on a sandbox seat being up
  and reachable — the dependency the port exists to remove, and the
  laptop↔sandbox legs are the fleet's known flaky path (incident
  #15131).
- **Supabase, following the queue pattern** (#2174 gap 5's own
  prescription: move per-job state to Supabase, following the queue
  pattern). This is the only home that preserves the guarantee:
  one watermark store, atomically advanced, shared by every seat
  and surface that runs detection.

**Answer:** the state must live in Supabase (a shared watermark /
dedupe store beside `agent.tasks`), with any local file reduced to
a cache. That store does not exist for this job today. Until it
does, question 1 has no port-preserving answer.

## Question 2 — does the fleet-queue enqueue leg keep its native form?

**Yes — this is the strongest leg.** The queue is already
laptop-reachable and Supabase-backed: the #2174 pilot verified
`repo-queue.py health` runs on the laptop against Supabase, and the
pilot's OpenChamber lanes claimed queue tasks through the atomic DB
claim with no double-claims against the concurrently running Muse
lanes. An OpenChamber scheduled session on the laptop enqueues the
same way the Muse wrapper does, through the same queue client and
the same dedupe keys; nothing about the enqueue leg needs a new
shape. (The queue broker outage, incident #15131, is a transport
incident affecting both forms equally, not a port blocker specific
to this job.)

## Question 3 — the laptop equivalent of the silence-probe leg

The probe leg reads a sandbox checkout of the code repo. A laptop
equivalent exists in shape: the laptop holds its own checkouts —
OpenChamber projects are anchored at laptop paths (the #2174 pilot
record anchors the code queue lane at `/home/user/git/code`) — and
OpenCode sessions are laptop-resident, so a ported probe would read
the laptop checkout instead of a sandbox one.

Two caveats keep this from being a clean yes:

- **Freshness contract.** The sandbox checkout is kept current by
  the sandbox lanes that work in it. A probe reading a stale laptop
  checkout would judge CI silence against the wrong tree. The port
  would need an explicit fetch-before-probe step and a staleness
  bound.
- **Isolation.** The laptop checkout is also a working tree for
  laptop lanes. The probe needs a dedicated read-only clone or
  worktree, not the shared working copy.

**Answer:** laptop equivalent = a dedicated read-only laptop clone
with a fetch-before-probe freshness step. Feasible, but it is new
surface to build and verify, not a rename. (Laptop paths above are
as recorded in the #2174 pilot comment; the laptop was not
reachable from this seat during the assessment — incident #15131 —
so they are recorded-state, not a fresh live read.)

## Decision

**No-port for now; `main-ci-failure-fix-trigger` stays on Muse
cron.** Two independent grounds, either sufficient:

1. **Gating:** #2174 gaps 2 (token accounting, #2072 — still open)
   and 4 (server residency — no closing record) are not closed, and
   #15346's gating requires them verified before a wave-2 port
   proceeds.
2. **Dedupe:** question 1's only guarantee-preserving answer —
   detection watermarks in a shared Supabase store — does not exist
   yet. A port onto laptop-local state would weaken the one-live-
   fix-track guarantee, which #15346 names as the hard constraint:
   an answer that weakens it is a no-port answer.

Questions 2 and 3 are answered favourably (enqueue keeps its native
form; the probe has a feasible laptop equivalent), so the port is
worth revisiting once the blockers clear.

**Flip conditions** — re-run this assessment when all three hold:

- #2072's OpenCode-side exporter is merged (gap 2 closed);
- a server-residency confirmation for the laptop OpenChamber
  deployment is recorded on #2174 (gap 4 closed);
- the detection watermarks/dedupe state for this job live in a
  shared Supabase store (gap 5 applied to this job), demonstrated
  by two concurrent runners producing one fix track for one
  injected failure.

No follow-on port issue is filed: under the no-port branch of
#15346's acceptance, the disposition and reasoning recorded here
and on the issue are the deliverable, and every flip condition is
already tracked by #2174 / #2072. No schedule change is made, so
`doc/fleet-schedule-registry.md` is unchanged.

## Verification

Claims in this record were checked 2026-11-26 from the xaltha seat:
PR #8437 and PR #11158 merge states and dates read via `fleet gh`;
#2072 and #2174 read open with the quoted pilot/exporter positions;
the inventory record read from current `main` (PR #15351, merged
2026-11-26); the registry row read from the repo; this seat's live
`cron.list` and goal-workspace copy inspected directly. Limits:
the primary seat's scheduler, Felix's seat state files, and the
laptop's stores were not reachable from this seat (incident
#15131); statements about them are recorded-state from #15346,
#2174, and the inventory, and are marked as such where they appear.
