# Design: Atlas semi-automated ingestion operations — source set, usefulness filter, disk/IO + plan-spend guardrails

Issue: [#6019](https://github.com/developing-today/code/issues/6019) ·
Parent: [#6018](https://github.com/developing-today/code/issues/6018) (Atlas ingestion program) ·
Statements S1–S5 of the 2026-11-04 brief.

This is the parent-level operations record for #6019. The specialist
sibling designs under #6018 each settled one slice; none of them owns
the integrated operating picture — which sources are swept, on what
trigger, under which usefulness test, watched by which guardrails,
accounted against which plan envelope, and run by which runbook.
That integration is the whole content of this record. Where a sibling
already decided something, this record cites it and does not restate
or override it; the only new decisions here are the gaps between the
siblings.

## 1. Sibling reconciliation — what is settled, what is still open

Revalidated against GitHub state and current main on 2026-11-09.

| Sibling | Scope | State | What this record takes from it |
|---|---|---|---|
| #6026 content versioning + retention | version identity, lineage, per-class retention defaults, retirement-as-report | **Settled** (issue closed; record on main: `doc/2026-11-08T10-26-00Z_design_atlas_content_versioning_retention/`) | Retention defaults and the rule that retirement is a report before it is a deletion (§4.3 below). Sweeps never delete; they surface retirement reports. |
| #6043 layer separation decision | Turso PITR is DB disaster recovery; corpus retention lives in Atlas | **Settled** (decision record on main: `doc/2026-11-05T15-50-41Z_decision_atlas_pitr_vs_retention/`) | Guardrail accounting (§5) never trades corpus retention against PITR windows; the two budgets are separate lines. |
| #6058 embedding cost ledger | per-batch token/$ ledger schema; reuse-ratio alert | **Settled** (issue closed; record on main: `doc/2026-11-09T01-07-20Z_design_atlas_embedding_cost_ledger/`) | The ledger is the embedding-spend source for §5's report. Its one alert rule (reuse ratio < 0.90 on append-mostly classes, 20-chunk floor) is the only embedding alert; dollar alerts live here, not there. |
| #6060 cross-tier vector comparability | score normalization across embedding models/tiers | **Settled** (issue closed; record on main: `doc/2026-11-08T10-44-00Z_decision_atlas_cross_tier_vector_comparability/`) | Nothing operational; cited so the sweep never treats cross-tier score drift as an ingestion failure. |
| #6061 storage growth model | churn × retention simulation; calibration of v1 retention defaults | **Settled** (issue closed, PR #7654 merged; record on main: `doc/2026-11-08T11-18-02Z_design_atlas_storage_growth_model/`) | The growth projection feeding §5's headroom alerts, including its §6 rule: a projection crossing 70% of the storage envelope within 180 days is surfaced for a human decision. |
| #6028 delta-only ingestion + offset embeds | span-anchored delta protocol; land only changes | **Open** (design record on main: `doc/2026-11-08T10-26-00Z_design_atlas_offset_matching/`; issue open/parked) | The discovery protocol every sweep in §3 assumes. Treated as an in-force input for append-mostly classes; the runbook (§6) must not depend on its open parameters being final. |
| #6045 ingest attestation record | in-toto-shaped per-batch statement binding chunks to sources | **Settled** (issue closed 2026-11-09; record on main: `doc/2026-11-09T05-12-32Z_design_atlas_ingest_attestation/`) | The batch manifest in §6 carries the attestation fields as required: every landed batch emits the #6045 statement binding its chunks to their sources. (Row corrected at merge time — #6045 closed and its record landed after this record was first drafted.) |
| #6057 staging-shard ingest gate | write → classify → dedup → provenance-check → publish | **Open** (in review) | The promotion path in §3: sweeps land in staging only; promotion is the gate's job, on merge-on-close per #5669. |
| #6059 secret-tier local embedder | local embedding for secret text; fleet-hardware evaluation | **Open** (in review) | The routing rule in §2: secret-tier content is never sent to a hosted embedder by a sweep, whether or not #6059 has landed — until it lands, secret-tier classes are held out of automated sweeps entirely (§2, approval column). |

Also in force, not siblings of #6019 but load-bearing: staging tier
with merge-on-close promotion (#5669), scripts migrating into code +
fleet (#5670), content-addressed landing (#3125), the
schema-driven classification manifest and its filter surface
(`doc/2026-11-05T13-03-49Z_design_atlas_manifest_filter_surface/`),
and deletion propagation on source deletion / repo privation
(`doc/2026-11-09T00-42-59Z_design_atlas_deletion_propagation.md`).

## 2. The source set and the per-source trigger / usefulness table

"Semi-automated" means, precisely: discovery, filtering, staging, and
accounting run unattended on the triggers below; the steps in the
**Approval** column stay human. Adding a source class, loosening a
class's inclusion test, a bulk backfill beyond the §6 backfill bound,
and any plan change are always human decisions — the sweep proposes,
a person disposes.

The usefulness test for every class is the question the class must be
able to answer, stated in the issue: *what question would this ever
answer?* A candidate item that cannot be tied to its class's question
is rejected, and the rejection is logged with the same manifest shape
as an acceptance (rejection is data — the unassociated-log principle):
`{batch_id, source_class, source_ref, reason_code, bytes, at}`.
Reason codes are a closed initial set: `no-question` (fails the
class test), `duplicate-content` (content address already landed),
`oversize` (single item over the §4 per-item slot), `secret-tier-held`
(§1, #6059), `superseded-version` (a newer version of the same source
object already landed in this batch).

| Source class | Examples / home | Trigger model | Inclusion test (the question it answers) | Approval gate |
|---|---|---|---|---|
| Boot / reboot records | `~/logs/init.jsonl`, init-status records on each fleet host | **Event-driven**: on boot-completion marker, the host's next sweep picks up the closed boot segment. Hybrid fallback: the §6 sweep also scans for unclosed segments older than 24 h. | The boot reached a terminal init state (success or recorded failure). Answers: *did this host come up clean, and what ran?* Partial/still-open segments are held, not rejected. | None per item. A new host joining the fleet is a new source — human-approved. |
| Session logs | OpenCode session store; Muse session transcripts | **Event-driven**: session end / file close finalizes a session; the sweep stages finalized sessions only. | Session reached a terminal state and contains at least one non-synthesized exchange. Answers: *what was decided/done, and what evidence supports it?* Empty, heartbeat-only, and purely synthetic sessions are `no-question` rejects. | Classification changes (e.g. a session reclassified secret-tier) are human-approved before promotion. |
| Chat logs | fleet chat exports (Slack/Discord-class sources as they are connected) | **Scheduled sweep** (hourly slot, §6) over the export delta since the last landed cursor. | Message is in a channel on the class allowlist and is not a bot/heartbeat post. Answers: *what did the fleet agree, announce, or escalate?* Off-allowlist channels are not read at all — absence, not rejection. | Adding a channel to the allowlist is human-approved. |
| Statistics | quota/statistical reports, provider usage snapshots, fleet health rollups | **Scheduled sweep** aligned to the report's own cadence; a report is ingested when its period closes. | The report is the final version for its period (superseded drafts are `superseded-version` rejects). Answers: *what did the fleet consume/produce in this period, at what cost?* | None per item. A new report type is a new source class — human-approved. |
| Reports | generated briefs, sweep reports, design/decision records produced outside the repo | **Event-driven**: file close in the watched report roots; repo-committed records are the `code-repo` class instead and are never double-ingested. | The report is final (not a `.tmp`/partial write) and not already landed under another class. Answers: *what did this run conclude?* | Bulk backfill of a report root's history is human-approved (§6 bound). |
| Code-repo (existing class, for completeness) | git sources | Existing git ingestion; unchanged by this record. | Per #6028/#6058: `code-repo` reuse expectations differ from append-mostly classes and carry no reuse-alert baseline. | Unchanged. |

Secret-tier rule (from §1): any item classified secret-tier is
`secret-tier-held` — logged, never staged, never sent to a hosted
embedder — until #6059 lands a local embedder route. The hold is a
queue, not a rejection: held items are re-offered automatically once
the route exists.

## 3. How a sweep moves data — delta, staging, promotion, retention

One sweep run, per source class, is a pipeline with four seams owned
by other records; this record only fixes their order and the failure
semantics at each seam:

1. **Discover (delta-only, #6028).** The sweep reads from the class's
   last landed cursor (offset/span state per #6028; a plain
   per-source cursor for non-span classes) and enumerates only new or
   changed source objects. A full re-scan is a backfill (§6), never a
   normal sweep.
2. **Filter (§2).** Each candidate passes the class inclusion test
   and the classification manifest's filter surface. Outcomes are
   accept / reject-with-reason / secret-tier-held, all written to the
   batch manifest.
3. **Land (content-addressed, #3125; staging, #5669/#6057).** Accepted
   items land in the staging shard under their content address.
   Landing is idempotent by construction: re-landing the same content
   address is a no-op, which is what makes sweep retries and overlaps
   safe (§6). The batch is attested per #6045 (record on main: `doc/2026-11-09T05-12-32Z_design_atlas_ingest_attestation/`).
4. **Promote (merge-on-close, #5669; gate, #6057).** Promotion from
   staging to the production atlas happens through the ingest gate —
   classify → dedup → provenance-check → publish — not by the sweep.
   The sweep's responsibility ends at a staged, manifested batch.
   Versioning and newest-resolution are #6026's contract; retention
   retirement is a report the sweep surfaces, never a deletion it
   performs; source deletions propagate per the deletion-propagation
   record, detected on the same sync diff.

## 4. Disk and I/O guardrails

Monitors live **on both hosts**: the source host watches its own free
space and I/O pressure (a sweep must never be the thing that fills
the disk it is reading from), and the ingest host watches staging
space and target write load. Thresholds below are **initial slots**,
chosen conservative; each is amendable by a dated amendment to this
record with the measurement that motivated it — never silently
retuned in code.

Measurement points (every run records all of them in the §5 report):

- `src_free_bytes`, `src_io_wait_pct` (source host, at run start and
  at each file boundary for runs over the large-run slot),
- `stg_free_bytes` (ingest host staging volume),
- `tgt_write_rows`, `tgt_read_rows` (target, from the batch's Turso
  usage delta),
- per-file `bytes_read` / `bytes_staged`.

Guardrail behaviour:

| Condition | Threshold (initial slot) | Behaviour |
|---|---|---|
| Source free space low | `src_free_bytes` < 10% of volume or < 5 GiB, whichever larger | **Pause** the run for that source; emit alert `guardrail/src-space`; resume automatically when free space recovers above the threshold for two consecutive checks (5 min apart). |
| Source free space critical | `src_free_bytes` < 5% or < 2 GiB | **Abort** the run for that source (staged work stays staged; nothing is promoted from a partial batch). Alert `guardrail/src-space-critical`. Human acknowledgement required before the next run for that source. |
| Source I/O pressure | `src_io_wait_pct` > 25% sustained over a run's measurement window | **Throttle**: halve the run's read concurrency / batch size for the remainder of the run; note it in the report. Two consecutive throttled runs for a source alert `guardrail/src-io`. |
| Staging space low | `stg_free_bytes` < projected batch bytes × 3 or < 10 GiB | **Pause before landing** (discovery may finish; landing waits). Alert `guardrail/stg-space`. A paused landing older than 24 h escalates to a human. |
| Per-item oversize | single source object > 256 MiB | `oversize` reject (§2) — logged, never partially read. Splitting an oversize class is a design amendment, not a sweep behaviour. |
| Target write burst | run's `tgt_write_rows` > 2× the trailing 7-day per-run median for the class | Finish the current batch, then **pause the class** and alert `guardrail/tgt-burst` with the report attached; a human resumes the class. Protects the plan envelope (§5) from a runaway source. |

No guardrail ever deletes data, skips the manifest, or promotes a
partial batch. Pause state is per (source class, host) and is itself
recorded in the run report, so a paused fleet is visible, not silent.

## 5. Plan-headroom accounting — the plan is a hard ceiling

The current paid plan is the envelope: **Turso scaler, $24.92/mo
(yearly)** (per #6019; storage, reads/writes, and database-count
quotas as published for the plan). The envelope is consumed as a
constant between plan changes (#6061 §3); **a plan change is a human
decision and a recalibration trigger, never a sweep action.** Growth
that would cross the plan is surfaced to Drew, not silently incurred
— cf. #5468 for what unmonitored growth looks like.

Every sweep run emits one report row per source class, and one
rollup row per run, with these fields:

| Field | Source |
|---|---|
| `run_id`, `batch_id`, `source_class`, `started_at`, `finished_at` | the run |
| `items_seen`, `items_accepted`, `items_rejected`, `items_held` | §2 manifest counts |
| `bytes_read`, `bytes_staged`, `storage_delta_bytes` | §4 measurement points; storage delta reconciled against the target's reported size after promotion |
| `chunks_new`, `chunks_reused`, `tokens_embedded`, `embed_model`, `embed_usd` | the #6058 ledger row for the batch (the ledger is the system of record for embedding spend; this report references, never recomputes) |
| `tgt_read_rows`, `tgt_write_rows`, `db_count_after` | target usage delta for the run |
| `envelope_storage_pct`, `envelope_reads_pct`, `envelope_writes_pct`, `envelope_db_pct` | post-run position against the plan envelope |
| `projected_days_to_ceiling` | #6061 growth model, per envelope line |
| `guardrail_events[]`, `pause_state` | §4 |

Alert rules (the complete set — deliberately small, per #6058's
warning against alert fatigue):

1. **Headroom warning** — any envelope line's projection crosses
   **70% of the envelope within 180 days** (#6061 §6 rule): surfaced
   in the cycle report for a human decision.
2. **Headroom alert** — any envelope line actually reaches **80%**:
   alert `plan/headroom-80` with the per-class contribution table.
   Sweeps continue; nothing is throttled on projection alone.
3. **Ceiling stop** — any envelope line reaches **95%**, or the
   growth model projects a crossing **within 30 days**: automated
   sweeps **pause for the affected classes** (largest contributors
   first, per the contribution table) and alert `plan/ceiling-stop`.
   Resuming, shedding retention (via #6026's retirement report), or
   changing the plan is a human decision. The stop exists so the
   fleet never crosses a paid ceiling silently or by inertia.
4. **Spend anomaly** — a run's `embed_usd` or `tgt_write_rows`
   exceeds 2× its class's trailing 7-day median: covered by
   `guardrail/tgt-burst` (§4) for writes, and reported as
   `plan/spend-anomaly` for embedding spend. There is no per-dollar
   embedding alert (#6058 owns the reuse-ratio alert; this record
   owns envelope position).

Accounting is per-run **and** cumulative: the cycle report (weekly
slot) rolls the run rows into per-source totals — storage, reads,
writes, DB count, embedding $ — against the same envelope lines, so
"which source is eating the plan" is always one table away.

## 6. The recurring sweep runbook

This record designs the runbook; it **creates no schedule** (per
#6019's scope). Cadence slots below are the design's recommendation
to the implementation follow-up.

**Cadence shape.** Event-driven classes (§2) are drained by a
frequent lightweight sweep (15-minute slot) that processes finalized
items only; scheduled classes run in their class slot (hourly for
chats, period-close for statistics). One runner, one lock per
(source class, host): a second runner instance for the same pair
exits immediately and logs `overlap-skip`.

**Idempotence.** A run is identified by `(source_class, host,
cursor_from, cursor_to)`. Landing is content-addressed (§3), the
cursor advances only after the batch manifest is durably written,
and promotion is the gate's separate act. Re-running any run —
after a crash, a pause, or an operator retry — therefore converges
to the same staged state; the worst case is re-discovering items
that reject as `duplicate-content`.

**Overlap and backfill.** Sweeps never overlap themselves (lock
above). A **backfill** — re-offering a source range older than the
cursor — is an explicit operator action with a named range, is
bounded to one source class and one host per backfill run, and
requires human approval when the range exceeds **7 days** of source
history or **10,000 items** (initial slots). Backfill batches are
manifested as `kind=backfill` so accounting can separate them from
steady-state growth.

**Partial failure and recovery.** Failure is per-item and per-batch,
never per-fleet:

- Item failure (unreadable, malformed): item is quarantined in the
  manifest with its error; the batch continues. Three failures of
  the same item across runs move it to a dead-letter list that a
  human reviews; the sweep stops re-offering it.
- Batch failure after landing, before manifest: the next run
  re-discovers the same cursor range; content-addressed landing
  makes the retry a no-op for already-landed items (§3).
- Host failure mid-run: the run's lock expires; staged partial work
  is inert (promotion never sees an unmanifested batch).
- Target failure (Turso unreachable / quota error): the run pauses
  all classes for that target, alerts `runbook/target-down`, and
  retries on the next slot. A quota error is treated as a §5
  ceiling event until a human reads it otherwise.

**Operator actions** (the complete set the runner must expose):
`pause-class`, `resume-class`, `run-once --class --host`,
`backfill --class --host --range`, `dead-letter list|retry|drop`,
`report --run|--cycle`. Every operator action is itself logged as a
statistics-class source item (§2) — operator history is data too.

## 7. Implementation follow-up audit

Audited 2026-11-09 against open issues under #6018. The sibling
designs carry their own implementation follow-ons (#6026/#6061
retention + calibration, #6028 delta protocol, #6045 attestation,
#6057 gate, #6059 local embedder, #6058 ledger) — this record adds
none of those again. The #6058 ledger's implementation is assumed by
§5 as the embedding-spend system of record; if it is not yet built
when the sweep runner lands, the runner emits the ledger rows itself
in the #6058 schema rather than inventing a second one.

Exactly one parent-level gap has no existing ticket: the **sweep
runner + guardrail monitor + run/cycle reporting** itself (the
subject of §§4–6). It is filed as a single implementation issue
linked from #6019 alongside this record. No other new tickets are
created by this task; in particular no duplicate child designs and
no production schedule (creating the schedule is a step inside that
implementation issue, gated on Drew's go-ahead there).

## 8. Non-goals and open questions

Non-goals: product-code changes; changing any #6018 parent decision
or the plan ceiling; re-deciding a sibling's settled scope; a
usefulness classifier beyond the static per-class tests in §2 (a
learned filter is a future amendment with its own evidence bar).

Open questions, recorded not blocking: (a) the exact Turso usage
API fields for `tgt_read_rows`/`tgt_write_rows` deltas per run —
the implementation issue verifies them against the live plan before
relying on §5's percentages; (b) whether chat-export cursors are
stable across export-tool upgrades — the runbook's `duplicate-content`
reject rate is the early-warning signal if they are not; (c) #6024's
zone-policy conclusion may tighten §2's secret-tier hold into a
per-zone switch (deletion-propagation §5.3 already anticipates it).

---

*Record written for task 14270 (queue, repo-health-code-pilot) from
issue #6019's packet and the sibling records cited in §1, all
revalidated fresh on 2026-11-09. Design only.*
