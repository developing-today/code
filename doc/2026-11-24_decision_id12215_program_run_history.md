# Decision: program-run history stays a live-only snapshot (no journal change)

- **Issue:** [#12215](https://github.com/developing-today/code/issues/12215) (split from #11960 step 6).
- **Date:** 2026-11-24.
- **Status:** decided — live-only snapshot wins; no durable program-run history in the world journal.
- **Refs:** #11960 (join/revoke times, shipped in #11999, journal v12), #11775 (live membership + program-run snapshot, done),
  #6308 with design `doc/2026-11-18_design_id6308_creator_owner_basics_v1.md` (§§3, 10).

## Decision in brief

Durable program-run history (start, end, outcome) does **not** go in the world journal.
The owner report keeps reporting program runs exactly as it does today
(`pkgs/id/crates/id-worlds/src/world_owner_report.rs`):

- with a running world: process-local guest call counts for one program instance
  (`ProgramRunScope::ProgramInstanceInProcess`, `complete: false`), reset on restart/reinstall;
- without one: `Measured::Unavailable(ProgramRunsNotRecorded)` — never zero, never inferred.

No new entry kind, no journal version bump, no compaction/checkpoint change.

## Why: runs are recomputable, times are not

Steps 1–5 of #11960 journaled `issued_at`/`revoked_at` because a join time is a
**wall-clock observation**: once the moment passes, no replay can recover it.
Program runs are the opposite. The journal already records everything a run is a
function of:

- every input delivered to the program (`Committed` / `System` entries),
- the active program (`ProgramInstalled`: module hash + seed, which "with the
  module and the inputs after this event reproduces the program state exactly").

Run outcomes are therefore recomputable from already-journaled facts; journaling
them would store a cache of a deterministic function, not a new fact. The one
thing replay cannot recover — wall-clock start/end timestamps and fuel burned —
is operational telemetry, and the cost of persisting it durably outweighs its
v1 value (below).

## Why not: cost, bounds, and privacy

1. **Hot-path write amplification.** A per-run journal entry fires on every
   guest `update`, i.e. roughly once per committed input — doubling journal
   appends on the input path. The default ack mode syncs (`fdatasync`) per
   append with a single producer, so there is no batching headroom to absorb
   it; every extra append is latency on input acknowledgement.
2. **Unbounded growth vs. bounded alternatives.** Joins scale with participants;
   runs scale with inputs — potentially millions per world. Durable run history
   would need a retention window, a bounded ring, and checkpoint summarization
   rules before it could land, or the journal becomes an ever-growing trace log.
   Nothing in v1 needs that; the owner report's bounded-sample contract (§5 of
   the #6308 design) points the other way.
3. **Replay and compatibility surface.** A new entry kind means a new
   `JOURNAL_VERSION_*` (next: 16), refusal logic, compaction/checkpoint
   carry-through, fork handling, and property tests — the same full-format
   machinery #11960 just paid for join times, but for telemetry rather than
   for facts the report cannot otherwise answer.
4. **Privacy/retention.** Timing and fuel traces persist across forks (copied
   entries) and replication, outliving the process they describe. #6308 §7
   treats participant and moderation data as sensitive; fine-grained execution
   timing deserves a retention decision first, not a silent append-forever.
5. **Consistency with the adopted design.** #6308 §3: "The design does not add
   a journal entry just to make historical charts"; §10 defers journal growth
   "until evidence from real use". No such evidence has arrived: #11775 shipped
   the live counters and no owner workflow since has required post-restart run
   attribution.

## Sketch if durable history is ever justified (not implemented)

If product evidence arrives (post-restart crash attribution, compute billing —
a stated owner need with a retention policy attached), the journal change needs
its own format design first. Sketch of what that design must contain:

- **Entry kind:** a coarse-grained record, not one entry per guest call —
  e.g. `ProgramRunSummary { instance_id, started_at_ms, ended_at_ms,
update_calls, fuel_total, exit: graceful|trapped|replaced }` written once per
  program-instance lifetime (restart, reinstall, crash), plus optionally
  error-only per-run records. Per-`update` entries are rejected: unbounded and
  hot-path.
- **Bounds:** a bounded ring (e.g. last 128 summaries) plus cumulative counters;
  the checkpoint carries the ring and the counters so compaction never replays
  unbounded run history; the owner report caps its sample at `sample_limit`
  exactly as joins do.
- **Version:** `JOURNAL_VERSION_PROGRAM_RUNS = 16` (current max is
  `JOURNAL_VERSION_NOTIFY = 15`), required by the new entry kind and by any
  checkpoint carrying the ring; older binaries refuse at the header check per
  the issue-#227 contract. `journal_version_required_by` gains the new arm.
- **Replay rule:** summaries are advisory — replay recomputes program state
  from inputs, never from summaries; a corrupt/missing summary degrades the
  report section to unavailable, never the world.
- **Retention:** summaries age out by count _and_ wall-clock window, stated in
  the report's `available_since` the same way join times do; no silent
  append-forever.

## Consequences

- No code change in this slice: the current `ProgramRunsSection`
  (live-or-unavailable) **is** the decided behavior, and #11775's acceptance
  line ("If durable run history is required, stop and file a separate
  journal-format design first") is satisfied by this note answering "not
  required."
- Revisit trigger: a concrete owner workflow that needs run facts across
  restarts, filed with a retention bound. Until then, further run-history
  proposals close against this note.
