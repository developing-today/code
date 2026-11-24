# Decision record: automatic context propagation — every fleet action carries its origin (#10357)

Date: 2026-11-24. Design-slice task 22215 (lane any-n), design-phase acceptance of #10357.
Origin: Drew, 2026-11-14. Goal: any command, session, secret lookup, queue action, or file
transfer in the fleet is traceable to what launched it — automatically, with zero per-call
effort from models. "If models have to stop and think about this every time, it has failed."

Status of this record: decisions marked DECIDED are locked by landed work cited below.
Questions marked DEFERRED name the decider and the default the builders should assume meanwhile.

## 1. The context record (DECIDED — fields per #10357, unchanged)

Captured once at the outermost point available, inherited by everything downstream:

- Muse session id + the internal model name serving it
- Queue task id + task name (when launched from the fleet queue)
- OpenCode session id + parent session id; the OpenChamber scheduled job that originated
  the session, when it came from one
- The triggering message id where the surface exposes it
- PID + a compact ancestor summary (cron → Muse → OpenCode → tool call)
- git HEAD hashes: hatch + hatch-personal repos (and the working repo of the call)
- Host + which user-context is acting

## 2. The five mechanisms, evaluated against the hard constraints

Hard constraints (all mechanisms must honor all four): zero model effort · cheap on the
hot path (capture once, inherit after; a warm fleet call is ~0.4 s — no added round trip) ·
no secrets in context (ids and hashes only; the context is logged widely by design) ·
absence is representable (interactive shells and old callers yield partial records, not errors).

1. **Init sourcing** — DECIDED as the capture pattern (`init source`, not execute), DEFERRED
   on the laptop user-context taxonomy. Sourcing is the only mechanism that satisfies
   zero-model-effort for shells; no alternative exists. What remains is enumerating the
   laptop contexts (Drew-interactive, OpenCode, Claude-driven, systemd units) — build
   issue #14186 owns the enumeration. Against the constraints: passes all four; the
   partial-record rule covers pre-contract shells.
2. **Env contract (`FLEET_CTX_*`)** — DECIDED as the wire format, DEFERRED on the exact
   variable list. A small fixed env set is the only carrier common to laptop-agent exec,
   SSH (AcceptEnv / explicit prefix), brokers, and file transfer. Alternatives considered:
   per-surface bespoke headers (rejected — N surfaces × M carriers, models must remember
   each); a central lookup service (rejected — adds a hot-path round trip). Verified
   2026-11-24: no `FLEET_CTX_*` contract exists anywhere in-repo (only unrelated mentions),
   so this is green-field. Build issue #14188 owns the variable list.
3. **Session hooks (OpenCode/OpenChamber fork)** — DECIDED: extend, don't replace, the
   landed claimant link. The task↔session binding already exists (#9046 events:
   `model_stamp` / `session_bound` / `session_unbound` in
   `pkgs/fleet/crates/fleet-queue`; cross-task inversion merged in PR #10560, which states
   the #10357 design "absorbs" the link with no new store). The hooks add: env injection
   on session start, stamping on tool calls and session end, chain extension (a Muse call
   triggering an OpenCode call extends the record). Build issue #14189 owns the hooks. The
   outstanding task↔session multi-stamp link is absorbed here, not a separate build.
4. **MCPX passthrough** — DECIDED as stated in #10357: headers/env through, env-contract
   fallback where the protocol cannot carry headers. No alternative evaluated — the
   fallback clause already covers the failure mode. Build issue #14190 owns it.
5. **Attribution at the stores** — DECIDED: the context id is the single join key across
   brokers' trip logs, job spools, and the atlas session corpus (`fleet search`). This
   answers design question 2 (below). No new store (per #10560 precedent). Build issue
   #14191 owns the per-store logging.

## 3. The three design questions

**Q1 — canonical record per session (env only? spool file? both?) and who sweeps it.**
DEFERRED. Decider: Drew. Default for builders meanwhile: env-carried record is canonical;
a spool file keyed by context id is a per-surface cache, never the source of truth; each
surface that keeps one owns its sweep (job spool: laptop-agent; trip logs: brokers).
Rationale for the default: a spool file as canonical would add the lookup the hot-path
constraint forbids.

**Q2 — how are related sessions from one origin found later ("everything this task
spawned")?** DECIDED: the context id is the join key, and the existing stores already
join on it — broker trip logs (per-trip logging landed), queue events (#9046) with the
cross-task inversion (PR #10560: `repo-queue models` over the link), job spools (sibling
build 1/5 records the id there), atlas corpus (sibling build 5/5 logs it there). No new
index, no new query surface.

**Q3 — the laptop story for systemd timers.** DEFERRED. Decider: Drew. Default for
builders meanwhile: context is the unit name + last invocation id, recorded by the unit
itself (wrapper or `ExecStart` preamble), partial record acceptable per the absence rule.

## 4. Build split (all filed 2026-11-24 with prove-it-end-to-end tests, triage queued)

- #14186 — 1/5 init sourcing (triage task 22335)
- #14188 — 2/5 env contract (triage task 22337)
- #14189 — 3/5 session hooks (triage task 22339)
- #14191 — 5/5 store attribution (triage task 22344)
- #14190 — 4/5 MCPX passthrough (triage task 22341)

Each split carries its own end-to-end proof (same id visible in ≥2 stores, no model
involvement) and the four hard constraints. Item order is parallel-safe except #14188,
which needs the Q1 default confirmed — it proceeds on the default above unless Drew
overrules on #10357.

## 5. Relations

Parent design: #10357 (this record closes its design-phase acceptance item 1; item 2 is
the five splits above). Prerequisites landed: #9046 (binding history), PR #10560
(claim-link rollup). Programme: epic/fleet-orchestration.
