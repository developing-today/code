# Hosted worlds & always-on hosting — v1 design (#6283)

- **Issue:** #6283 (Phase 14 core topic #6276; principles from #1240)
- **Status:** v1 first-iteration design, filed by queue task 16847 (lane code-d)
- **Entry artifact:** first-pass research findings on #6276 (task 14341,
  lane code-a, comment 6871664756, 2026-11-14) — the entry condition on
  #6283. This design takes each of its four findings and records
  accept/reject below (§2). Entry condition **checked and met** before
  this design was finalized: the findings exist on #6276, the design-order
  prerequisites #1061 (provider registry) and #1067 (scheduling/quotas)
  are CLOSED with landed designs, and the ladder siblings #6284 (v2),
  #6285 (v3), #6286 (v4) are OPEN and out of scope here.
- **Scope of v1:** exactly one node hosting persistent worlds with the
  same `id` binary and the same journal format as a local host: a world
  survives zero connected clients, restarts and recovers from its
  journal, and an operator can list, start, stop, and inspect hosted
  worlds. First deployment target is the basement cluster. Custody and
  lifecycle depth is v2 (#6284), many-node operation is v3 (#6285),
  public-server operation is v4 (#6286) — all explicitly out of scope
  and gated behind this design.

## 1. Governing principles (#1240) — how each one holds

1. **Local-first always.** The hosted world is the same object as the
   local one: same binary, same `journal.jsonl`, same replay result.
   Hosting adds a supervisor and an operator surface around the world;
   it changes nothing inside the world. A world developed and run
   locally is hosted by copying its directory to the host — a transfer,
   not a port — and can be copied back at any time.
2. **Hostable by anyone at any scale.** v1 is deliberately single-node:
   one binary, one data root, no external services (no database, no
   queue, no separate control plane). Anyone with a small server runs
   the identical host an operator runs on the basement cluster;
   scale differences live in v3/v4 and in backend-only elements (§7),
   never in the world or its journal.
3. **P2P behavior identical to hosted behavior.** Clients reach a hosted
   world over the same iroh transport and the same frames as a
   P2P-hosted world. Lifecycle transitions are authority-frame actions
   evaluated by the world (§4), not by the hosting layer, so a wake,
   pause, or membership check resolves identically wherever the world
   is hosted. The host never grants authority the world would not
   grant locally.
4. **Our hosted instance is the public backstop, never a requirement.**
   Nothing in v1 requires the public instance to exist: a world hosted
   on a home server is fully usable without it, and the split from
   #6276 — a public main endpoint with deeper backend pieces as
   separate iroh/id servers behind private tunnels — is a deployment
   topology (§3), not a dependency. If the public instance is down or
   never contacted, every v1 flow still works.

## 2. Disposition of the #6276 findings (accept / reject)

| #   | Findings constraint                                                                                                                                                                                                                                                        | Disposition                                                             |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| F1  | Exactly three operator-visible lifecycle states — live, asleep, archived — with transitions as authority-frame actions (Phase 10 roles / F12.3); wake is not an authorization bypass; zero clients never archives or deletes                                               | **Accept** — §4                                                         |
| F2  | Journal custody: encryption at rest is an operator/deployment property, not a journal-format change; snapshot/export is a signed, self-describing journal segment set; backup is only real with restore drills (drills owned by v2 #6284)                                  | **Accept** — §5; drill cadence deferred to #6284 as the finding assigns |
| F3  | Basement cluster is the first target; what breaks first, in order: egress on wake bursts, storage growth from un-archived asleep worlds, power/thermal — not CPU; capacity is counted per-state                                                                            | **Accept** — §3 (target), §9 (break-first order + v1 mitigations)       |
| F4  | Parity checklist: journal format + replay result, program ceilings, membership/authority evaluation, name resolution, content addressing identical; quotas/storage tiers, abuse controls, jurisdiction partitioning, public-endpoint rate limits legitimately backend-only | **Accept** — §7, as a conformance test in the breakdown (packet P6)     |
| —   | (no findings constraint rejected)                                                                                                                                                                                                                                          | The design's own open choices are confined to §10                       |

## 3. Shape of the system

```
operator ── id-cli (list/start/stop/inspect, §6) ──▶ host control socket
                                                        │
clients ── iroh (same frames as P2P) ──▶ ┌──────────────▼───────────────┐
                                         │  id binary, host mode        │
                                         │  supervisor (§4)             │
                                         │   ├─ world A: live (process) │
                                         │   ├─ world B: asleep (dir)   │
                                         │   └─ world C: archived (dir) │
                                         └──────────────┬───────────────┘
                              data root: <root>/<world>/journal.jsonl
                                           <root>/<world>/modules/<blake3>.wasm
```

- **Host mode is a mode of the existing `id` binary** (`pkgs/id`), not a
  new binary and not a wrapper product. The supervisor is the only new
  long-lived component: it owns the set of world directories under one
  data root, tracks each world's lifecycle state, starts and stops
  world processes, and answers the operator surface. World internals —
  `id-worlds`' `FileJournal` (`crates/id-worlds/src/world_store.rs`:
  append-only `journal.jsonl`, fsynced per append, content-addressed
  `modules/`), replay, programs, ceilings — are untouched.
- **A world is its directory.** State at rest is exactly the local
  layout. Moving a world between a laptop, a home server, and the
  basement cluster is copying the directory while the world is asleep
  or archived; there is no export/import format distinct from the
  journal itself in v1 (signed snapshot/export sets are custody, §5,
  and their drill discipline is v2).
- **Deployment topology (Drew's split, from #6276).** The basement
  cluster runs: one public main endpoint (the host above, reachable by
  clients), and deeper backend pieces as separate iroh/id servers
  behind private tunnels with limited trusted access. v1 specifies the
  topology and which pieces are public vs tunnel-only; operating the
  public endpoint as a service for others is v4 (#6286).
- **No new infrastructure.** No database, scheduler, queue, or second
  binary is introduced. The host's own state (which worlds exist, their
  lifecycle states, their data-root paths) is a small host file beside
  the data root, rebuilt from the directories it describes — the
  journals remain the only authority on world contents.

## 4. World lifecycle at world scope (F1)

Connection-scope idle handling already exists
(doc/2026-10-11T04-48-13Z_feature_takeover_idle_connections); v1 lifts
it to world scope with exactly three operator-visible states:

| State        | Meaning                                                                                 | Cost while in state               |
| ------------ | --------------------------------------------------------------------------------------- | --------------------------------- |
| **live**     | World process running, accepting connections                                            | CPU/RAM of one world process      |
| **asleep**   | No process; journal + modules at rest; wakes on authenticated connect or operator start | Disk only                         |
| **archived** | At rest; no automatic wake — explicit operator restore to asleep first                  | Disk only, colder accounting (§9) |

Rules:

- **Transitions are authority-frame actions** (Phase 10 roles / F12.3),
  evaluated by the world/host against the same authority data a local
  host uses: owner and operator may wake, pause (live → asleep), and
  archive; restore (archived → asleep) is operator-only.
- **Wake is not an authorization bypass.** A member's connect to an
  asleep world is a wake _request_: the supervisor starts the world,
  and membership/authority is enforced by the world before it serves —
  exactly as if the world had been live. A non-member's connect wakes
  nothing they can use; the world may return to asleep.
- **Zero connected clients is the sleep trigger, never a delete or
  archive trigger.** When a live world's last client disconnects, the
  supervisor pauses it to asleep after the operator-configured idle
  window (§10). Sleep is the zero-client steady state; archiving is
  always an explicit operator act.
- **State is inspectable** (§6): every state change is recorded in the
  host file with time, actor (operator identity or `wake-on-connect`),
  and prior state, so `inspect` can answer "why is this world asleep"
  without reading the journal.

## 5. Journal restart and recovery (F2)

Recovery is the mechanism `id-worlds` already implements — a world is
restored by rebuilding its core from the journal — operated as a
hosting discipline:

- **Restart of the host** (crash, power loss, upgrade): the supervisor
  reads the host file, returns archived worlds to archived and asleep
  worlds to asleep untouched, and restarts only the worlds recorded
  live — each by journal replay from its own directory. A world that
  was mid-append at the crash recovers to the last fsynced entry:
  `FileJournal` fsyncs per append, so the loss window is the single
  in-flight append, whose absence replay tolerates by construction
  (append-only, one `JournalEntry` per line; a torn final line is
  discarded, never partially applied — this property is pinned by a
  test in packet P3).
- **Restart of one world** (operator stop/start, crash of a world
  process): same replay path, scoped to that world's directory; other
  worlds are undisturbed.
- **Journal format compatibility** is governed by the existing journal
  version discipline and #227 (journal rollback compatibility, done):
  the host refuses to open a world whose journal version it cannot
  replay, names the version in `inspect`, and never rewrites a journal
  it cannot read. Same binary + same journal format is the parity
  invariant (§7); a host upgrade that changes replay results is a
  parity bug, not a migration.
- **Snapshot/export** for custody is a signed, self-describing journal
  segment set, building on F12.13 journal replication (#806, CLOSED —
  completed 2026-11-14; its signed stream is a landed input, §8).
  Encryption at rest is the deployment's property (disk/volume
  encryption chosen by the operator), never a journal-format change,
  so an exported or copied world stays readable by any host. Restore
  drills — the cadence and the restore-into-scratch-world check that
  make a snapshot a backup rather than a hope — are owned by v2
  (#6284), as the research assigns; v1 delivers the snapshot/export
  shape and one documented manual restore procedure (packet P3).

## 6. Operator basics (list / start / stop / inspect)

The operator surface is four `id-cli` subcommands against the host's
local control socket (no network listener; the socket is
filesystem-permissioned to the operator). All output is derived from
the host file + journal headers — the CLI reports facts, never
guesses:

- **`host list`** — every world under the data root: name/id, lifecycle
  state, journal size + entry count, last state-change time and actor.
  This is the per-state census §9 capacity accounting reads.
- **`host start <world>`** — asleep → live (replay, then serve);
  archived worlds refuse with the restore step named.
- **`host stop <world>`** — live → asleep: drain connections, final
  fsync, record the transition. `host archive <world>` /
  `host restore <world>` move between asleep and archived.
- **`host inspect <world>`** — the world's record: state and history,
  journal version + head entry, module count, data-root path, quota
  posture inherited from the #1067 design (storage tier, ceilings),
  and the last recovery result if the world was replayed after a
  crash. Inspect never opens the world for serving.

## 7. Hosted / local parity (F4)

The v1 invariant, stated as a checklist and enforced by packet P6 as a
conformance test (same journal in, same replay result out, on a local
host and on the hosted node):

**Must be identical:** journal format and replay result; program
ceilings; membership/authority evaluation (including lifecycle
transitions, §4); name resolution outcomes; content addressing
(`modules/<blake3>.wasm`).

**Legitimately backend-only** (never world-visible as behavior
differences): quotas and storage tiers (per the #1067 design); abuse
controls and public-endpoint rate limits; jurisdiction partitioning
(residency: one hosted world per jurisdiction, fail-closed on
mismatch — doc/2026-11-01_investigation_shared_world_residency.md);
encryption-at-rest choices (§5).

The self-hosting interop stance
(doc/2026-11-08T09-12-27Z_design_id-self-hosting-interop-stance) is a
recorded input: anyone runs this same binary at any scale, and nothing
in the hosted path may depend on our instance (principle 4).

## 8. Dependencies — named, not assumed

| Dependency                                                                         | State (fresh, 2026-11-16)         | How v1 treats it                                                                                                                                                                                                                                                                                  |
| ---------------------------------------------------------------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #805 — F12.11 delta and binary frames (screen patches, resume tokens)              | **OPEN**                          | **Not assumed.** Wake-burst resync (§9) is bounded by resume tokens/delta frames when #805 lands; until then v1 wakes with full resync and names the egress cost as the known v1 limitation. No v1 packet blocks on #805; P6 records resync bytes so the improvement is measurable when it lands. |
| #806 — F12.13 journal replication (read replicas over a signed stream)             | **CLOSED** (completed 2026-11-14) | Landed input: §5's signed snapshot/export segment set builds on its signed stream. Replication-based backup and read replicas are **not** v1 scope — they belong to custody depth in v2 (#6284).                                                                                                  |
| #1061 — F12.4 provider registry design                                             | CLOSED (design landed)            | Input only: if a hosted world registers providers, it does so under that design; v1 adds no registry behavior.                                                                                                                                                                                    |
| #1067 — F12.10 fair scheduling / quotas design                                     | CLOSED (design landed)            | Input: §6/§9 quota posture (storage tiers, wake fairness) follows that design; v1 implements only the host-side accounting hooks (packet P1), not a new quota system.                                                                                                                             |
| #227 — journal rollback compatibility                                              | Done (per #6276)                  | §5's version discipline builds on it.                                                                                                                                                                                                                                                             |
| #6284 / #6285 / #6286 — v2 custody & lifecycle, v3 many nodes, v4 public operation | OPEN                              | Successors, not dependencies: v1 must not pre-build their scope, and their designs consume this document.                                                                                                                                                                                         |

## 9. What breaks first (F3) — and the v1 mitigation for each

In the research's expected order, on the basement cluster:

1. **Egress bandwidth on wake bursts** — many clients resyncing when a
   popular world wakes. v1 mitigation: wake is serialized per world by
   the supervisor (one replay, then clients attach), idle windows (§10)
   keep brief disconnects from cycling worlds, and P6 measures full-
   resync bytes per wake so the #805 improvement is quantified when it
   lands. This is v1's named scaling limit, not a surprise.
2. **Storage growth from un-archived asleep worlds** — sleeping worlds
   cost disk forever. v1 mitigation: `host list` is a per-state census
   (live/asleep/archived counts and bytes), storage tiers from #1067
   apply per state, and archiving is one operator command (§6). v1
   never auto-archives (F1); it makes the growth visible instead.
3. **Power/thermal** — an always-on small cluster's real ceiling.
   v1 mitigation: asleep worlds cost no CPU; the host runs as one
   supervised process per node under the node's existing service
   management (packet P5), so power events are ordinary host restarts
   and §5 recovery is the answer, not new machinery.

## 10. Open choices — decided here with defaults

1. **Supervisor placement.** In-binary host mode (chosen) over an
   external supervisor daemon: one binary is the principle-2
   requirement, and a second daemon would be a second thing for every
   self-hoster to run. The node's service manager (systemd/NixOS,
   packet P5) supervises the one host process.
2. **Idle window before sleep.** Operator-configured per host,
   default 15 minutes after the last client disconnects. Never 0
   (a disconnect/reconnect flap must not cycle a world) and never
   infinite by default (an always-live world is an explicit operator
   pin, recorded in `inspect`).
3. **Wake-on-connect.** On, for members, per §4 — with membership
   enforced before serving. Operators may disable it per world
   (operator-start-only), recorded in the host file.
4. **Snapshot cadence.** Manual in v1 (operator command, §5 shape);
   scheduled snapshots and restore drills are v2 (#6284) scope, as
   the research assigns custody depth there.

## 11. Implementation breakdown

Filed as issues and linked from #6283 in the same pass as this design;
each carries its own acceptance criteria. Packets are ordered — P1
first, the rest build on it — but P3/P4 may proceed in parallel once
P1's host file format is fixed.

- **P1 — Host mode in the `id` binary.** Supervisor over a data root:
  world directory registry (host file), live/asleep/archived state
  tracking, start/stop by journal replay, quota accounting hooks per
  #1067. (§3, §4, §5)
- **P2 — Lifecycle transitions as authority-frame actions.**
  Wake/pause/archive/restore evaluated under Phase 10 roles / F12.3;
  wake-on-connect with membership enforced before serving; idle-window
  sleep; per-world operator pins. (§4, §10)
- **P3 — Restart/recovery hardening + snapshot/export.** Crash-case
  test matrix (host kill mid-append, world-process kill, power-loss
  simulation), torn-final-line tolerance pinned by test, signed
  snapshot/export segment set per #806's stream, documented manual
  restore procedure. (§5)
- **P4 — Operator CLI.** `id-cli host list|start|stop|archive|restore|inspect`
  over the local control socket, output derived from host file +
  journal headers. (§6)
- **P5 — Basement-cluster deployment pack.** Service definition
  (systemd/NixOS) for the single-node host, data-root layout, the
  public-endpoint vs private-tunnel split of §3, operator runbook for
  start/stop/inspect/recovery. (§3, §9)
- **P6 — Parity conformance.** The §7 checklist as tests: identical
  journal → identical replay result, ceilings, authority outcomes, and
  content addressing, local vs hosted; wake-resync byte measurement
  recorded for the #805 follow-on. (§7, §9)

## 12. Out of scope (explicit)

Implementing any of P1–P6 (this task is design + breakdown only);
assuming an outcome for open #805; changing the Phase 14 principles
from #1240; custody/drill cadence (#6284), multi-node hosting (#6285),
and public-server operation for others (#6286).
