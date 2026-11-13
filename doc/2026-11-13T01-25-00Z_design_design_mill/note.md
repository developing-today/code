# The design mill — concept, history, and worked examples

**What this doc is:** the concept and the track record. The operating
playbook — launch packet, roles, judge loop, check-in cadence,
anti-patterns — lives in the design-mill skill
(`~/workspace/custom-skills/design-mill/SKILL.md` in hatch-personal) and is
deliberately not duplicated here.

**Filed:** 2026-11-12 (Drew's directive: "document the design mill concept
and several examples from now and October where we made designs of designs
and broke things down"). Every example below was verified against GitHub
records (issue/PR numbers, doc paths, merge dates) on 2026-11-12; anything
that could not be verified is labeled as such.

---

## 1. The concept in one page

Some directives are too large for one design. A single instruction from
Drew routinely spans platforms, data models, transports, and months of
work — and carries dozens of individual requirements, some stated in one
clause each. Written as one design doc, such a directive either stays
vague (and the detail evaporates) or becomes a monolith nobody can execute
against. Worse, the failure mode is silent: a requirement nobody wrote
down is a requirement nobody builds, and nothing reports the loss.

The design mill is the technique we use instead. It treats a large
directive as raw material to be *milled* — ground down, recursively, until
every piece is small enough to build:

```
directive
  → statement inventory     (every sentence extracted, numbered, mapped)
  → starter epic            (one program issue holding framing + staging)
  → proto design            (V1 = what exists today; the shape of the whole)
  → summit elaboration      (ONE owner session expands the proto design:
                             state-of-the-art research, child designs,
                             versioned V2/V3/V4 layers)
  → child designs           (each itself decomposable the same way)
  → implementation slices   (granular issues, individually workable)
  → queue                   (slices become tasks; lanes execute them)
```

The shape is recursive by design: a child design inside one program can
itself be milled (its own inventory, its own slices), and designs of
designs are the normal case, not the exception. The 2026-10-28 ID program
ran investigations whose *synthesis* issues merged them into designs,
which then spawned their own follow-ups — three levels deep before any
code was written.

### Invariants

1. **Nothing is dropped.** Every statement in the directive maps to an
   issue — a child design, an implementation slice, a research item, or
   an explicit parked item with a horizon. "Zero unmapped" is the intake
   bar, and gap passes re-walk the record (including closed designs and
   raw session stores) hunting statements that never got an issue.
2. **One owner per elaboration.** Exactly one summit session owns a
   program's build-out end to end. Many hands may write, but one head
   holds the architecture, the versioning, and the final language.
3. **Designs amend, they don't fork.** New work extends existing designs
   and cross-links them. A program that discovers a sibling design adds
   to it or records an input to it — it does not restate it in parallel.
4. **Versions layer, never replace.** V1 records what exists today;
   V2/V3/V4 layer the fuller vision and the short- and long-term
   directions. A later pass adds designs and next steps; it never
   silently rewrites the earlier ones away.
5. **Research is its own issue class.** State-of-the-art expansion
   (competitors, prior art, best-in-class) happens *before* design, as
   filed, evidence-backed items that sibling designs build on. "Needs
   research" is a filing, not a blocker and not a gap papered over
   inside a design.
6. **Sequencing is a tracked item.** Prioritization — quick results
   first, long-horizon pieces explicitly parked in the plan — is itself
   filed as an issue. "In the plan for 3 months from now" is a valid,
   recorded state; silently dropping is not.
7. **Questions batch; work doesn't stall.** Only Drew can settle some
   questions. Those batch up with options and a recommendation, are
   recorded back onto the issues as constraints once ruled, and never
   block the rest of the mill.

## 2. Mechanics in brief

Muse (the orchestrator) does the intake: reads and analyzes the brief,
files the starter issue, merges the source docs into `main` (worktree +
PR, per house rules), creates the task-queue item, and launches **one
summit session** on the summit route head with the mill's launch packet.
The summit session runs **detached** — it never stalls on a question
(the `question` tool is forbidden in detached work; questions are
recorded with a chosen default and work continues) — and delegates
everything it can spec crisply to **workhorse subsessions**: research
sweeps, inventories, codebase audits, first drafts. Fresh **judge
sessions** review batches of subsession output, fill small gaps, and
spec the rest; a batch returns upward only when worker and judge agree
it is complete (return-on-agreement). The orchestrator checks in every
5–15 minutes: status, questions log, spot-verification of artifacts
against specs, and the forest view — total scope covered vs. the
statement inventory.

Full operational detail: the design-mill skill (path above).

## 3. Worked examples, chronological

### 3.1 2026-10-26 — project structure & visualization (the proto-mill)

**Directive:** every project gets a standard home — description, status
over time, roadmap, and visualization weighted by effort and complexity.

**Cascade:** the spec (written in the lab-cluster goal workspace; no
repo copy survives — honestly noted) produced epic
[#1129](https://github.com/developing-today/code/issues/1129)
(per-project status pages for all projects), which decomposed into
**9 child issues #1120–#1128**: a project-configuration registry design
(#1120), the /program hub implementation (#1121), the
complexity-weighted visualization (#1122 — effort-weighted rollups,
hands queue, blocked weight, dependency depth graph), and six
per-project section build-outs. Artifacts: PR
[#1142](https://github.com/developing-today/code/pull/1142) (hub +
pages), PR [#1082](https://github.com/developing-today/code/pull/1082)
(first roadmap + status record, the pattern made concrete). The cascade
kept going after the epic closed: adoption follow-ons
[#1264](https://github.com/developing-today/code/issues/1264) (mode-label
backfill) and [#1266](https://github.com/developing-today/code/issues/1266)
(prose dependencies converted to native blocked-by edges).

**What it proved:** the directive → spec → epic → children → adoption
shape works and self-extends. This predates the name "design mill" —
the pattern existed before the protocol.

### 3.2 2026-10-28 — the ID program: interfaces on a substrate (the deepest early mill)

**Directive:** Drew's 2026-10-28 ID program directive. Its LIST 6–15
became epic [#2056](https://github.com/developing-today/code/issues/2056)
(ID platform expansion — apps, comms, currency, hosting, boards),
filed 2026-10-29 with **29 children (#2026–#2054)**: 19 parallel
investigations and 10 named syntheses. Every child carried the same
working pattern verbatim: *"parallel investigations, then the named
synthesis issue merges them into a full design… Deliverables: a markdown
design/investigation doc … plus follow-up issues for anything
buildable."* A second wave of ramification issues (#2057–#2066 — world
VMs, currency control planes, key recovery) was filed the same morning.

**The interfaces layer** — many interfaces and app shapes over one ID
substrate — was LIST 6 and its siblings, and it is where "designs of
designs" is most visible:

- **App model:** investigation
  [#2026](https://github.com/developing-today/code/issues/2026) (apps as
  folders; TUI/web interface parity) + investigation
  [#2027](https://github.com/developing-today/code/issues/2027)
  (capability-based specialization across web, ESP32/MQTT, text/audio)
  → synthesis [#2028](https://github.com/developing-today/code/issues/2028)
  (ID core build v1), merged as PR
  [#3909](https://github.com/developing-today/code/pull/3909).
- **Todos:** investigation
  [#2029](https://github.com/developing-today/code/issues/2029) (did-it
  tracker data model) → synthesis
  [#2031](https://github.com/developing-today/code/issues/2031)
  (sticky-notes / did-it tracker, the first ID app), PR
  [#3910](https://github.com/developing-today/code/pull/3910).
- **IRC:** investigation
  [#2049](https://github.com/developing-today/code/issues/2049)
  ("IRC-first chat on ID"; its doc quotes the ask: *"I mostly just want
  IRC", with room to grow toward Discord/Slack/Teams*) → synthesis
  [#2051](https://github.com/developing-today/code/issues/2051) (ID chat
  v1 — "the smallest IRC-shaped channel layer"), PR
  [#3866](https://github.com/developing-today/code/pull/3866), with PoC
  children #2808/#2809 filed from the investigation.
- **phpBB:** investigation
  [#2052](https://github.com/developing-today/code/issues/2052)
  ("boards — a modern phpBB on ID identity: no registration, copyable
  themes"; the body asks to bring back phpBB3, modernised) +
  investigation [#2053](https://github.com/developing-today/code/issues/2053)
  (embedded playable posts) → synthesis
  [#2054](https://github.com/developing-today/code/issues/2054) (ID
  boards v1), PR [#3832](https://github.com/developing-today/code/pull/3832).
- **Real-time games, ticks, agreed winners:** investigation
  [#2037](https://github.com/developing-today/code/issues/2037)
  (match-result attestation — *"A result counts when both agree who won
  (chess-style: the loser says the other won), OR both players send
  their data to a user acting as WITNESS who … decides a winner"*) +
  investigation [#2038](https://github.com/developing-today/code/issues/2038)
  (match audit — replay validation; provable results require a world
  program *"deterministic under replay (fixed tick, seeded RNG…)"*) →
  synthesis [#2039](https://github.com/developing-today/code/issues/2039)
  (match attestation + audit v1), PR
  [#3807](https://github.com/developing-today/code/pull/3807). The
  attestation investigation found its own missing prerequisite and
  filed it: [#2814](https://github.com/developing-today/code/issues/2814)
  (player-signed move capture).

**Cascade counts:** 1 directive → 2 sibling epics (#2055 fleet, #2056
ID) → 29 children + 10 same-morning ramifications → investigations
landed Oct 29–30 → syntheses landed as designs on 2026-11-01/02 → PoC
and slice children filed throughout.

**Honest edges:** feeds were *not* part of the October directive —
board feeds arrive 2026-11-04/05 via
[#5680](https://github.com/developing-today/code/issues/5680) →
[#5862](https://github.com/developing-today/code/issues/5862). The
standalone tick data-model design is also November:
[#5968](https://github.com/developing-today/code/issues/5968) (2026-11-05,
statements S52–S58 of the 2026-11-04 worlds brief — itself a mill
intake, with its own statement inventory). The full text of the
2026-10-28 directive lives in hatch-personal, not in GitHub; the issue
bodies cite it, and the decomposition above is verified from the issues
and merged docs themselves.

### 3.3 2026-11-01 — the crate-split design (a synthesis that became an implementation cascade)

**Directive:** the ID build-time review (issue
[#3195](https://github.com/developing-today/code/issues/3195), itself
ACT-15 of the 2026-10-28 program) scoped crate splitting as a fallback;
Drew elevated it to first-class on 2026-11-01 (umbrella
[#3447](https://github.com/developing-today/code/issues/3447)).

**Cascade:** the synthesis pattern from §3.2 produced the crate-split
design — a 10-crate map derived from the real import graph, ten unclean
seams each with a named resolution, and a staged migration sequence —
merged as PR [#3824](https://github.com/developing-today/code/pull/3824)
(`doc/2026-11-01T16-53-00Z_design_id_crate_split/`). The design then
decomposed into **7 step issues**
([#4403](https://github.com/developing-today/code/issues/4403),
[#4405](https://github.com/developing-today/code/issues/4405),
[#4406](https://github.com/developing-today/code/issues/4406),
[#4407](https://github.com/developing-today/code/issues/4407),
[#4408](https://github.com/developing-today/code/issues/4408),
[#4409](https://github.com/developing-today/code/issues/4409),
[#4411](https://github.com/developing-today/code/issues/4411)), each
step one PR with `main` green and a byte-identical canary. Steps 1–5
landed and closed on 2026-11-11; steps 6–7 remain open and are being
worked under the same design.

**Classification, honestly:** this is the mill *pattern* (one design,
ground into individually workable steps under a single umbrella) but
not a formal mill *run* — no summit session, no queue item; the design
came out of the ID program's investigation→synthesis machinery. It is
included because Drew counts it in the family, and because it shows
the pattern operating inside a program the mill later formalized.

### 3.4 2026-11-06 — the mill gets its name, its machinery, and its first formal runs

On 2026-11-06 Drew set the protocol explicitly: for each large design —
intake by Muse, starter issue, docs merged to main, queue item, then
ONE summit session owns the build-out. Before the first designs
arrived, the machinery itself was tested: the workhorse-subsession
workshop test, filed as
[opencode#1881](https://github.com/developing-today-forks/opencode/issues/1881)
("Workshop test: subsession machinery from a workhorse session —
results and system-prompt suggestions", 2026-11-06). Its findings
became granular children within the hour — including
[opencode#1883](https://github.com/developing-today-forks/opencode/issues/1883),
the `question`-tool stall bug (a detached subsession's question
silently stalls the child *and* its foreground parent, and the retry
machinery re-asks in a loop), which is the empirical origin of the
mill's detached-operation rule. The working pattern was also written
into the repos' agent docs the same day (PR
[#6607](https://github.com/developing-today/code/pull/6607)).

**First formal intakes under the protocol:**

- **Cluster egress** — issue
  [#5030](https://github.com/developing-today/code/issues/5030)
  (egress strategy for 2 Gbps home fiber). Mill intake 2026-11-10:
  brief merged via PR [#8179](https://github.com/developing-today/code/pull/8179)
  (`doc/2026-11-10_design_cluster_egress_5030.md`), build-out enqueued
  as queue task 14649, summit-owned.
- **Queue task data model** — issue
  [#3424](https://github.com/developing-today/code/issues/3424)
  (durable task data model). Intake brief via PR
  [#8180](https://github.com/developing-today/code/pull/8180)
  (`doc/2026-11-10_design_queue_task_data_model_3424.md`, including a
  V1 baseline of already-landed fields — the versioned-design rule
  applied to a data model), build-out enqueued as task 14650.
- **Atlas v2–v4** — issue
  [#6020](https://github.com/developing-today/code/issues/6020)
  (expansive Atlas arc). Design-mill routing recorded on the issue;
  queue task 14861; the summit session owned it end to end and
  delivered PR [#8519](https://github.com/developing-today/code/pull/8519)
  on 2026-11-11: the design plus `research/`, `review/`, and four
  follow-on designs (F01–F04), which themselves filed as fixture
  issues
  [#8525](https://github.com/developing-today/code/issues/8525)–[#8528](https://github.com/developing-today/code/issues/8528).
  Designs → designs → issues, two levels in one run.

The same week shows the versioned-design habit spreading: the ID
phase-14 discovery/registry/federation topic filed as one core issue
([#6277](https://github.com/developing-today/code/issues/6277)) plus
explicit v1/v2/v3/v4 layer issues (#6287–#6290) on 2026-11-05.

### 3.5 2026-11-11 — the design register: the mill turned on its own output

**Directive:** Drew's 2026-11-11 order — review every design and larger
task asked for in the past 60 days, make a comprehensive list, load it
into the PM stack, and *stop dropping asked-for work*.

Epic [#8805](https://github.com/developing-today/code/issues/8805).
The register itself merged via PR
[#8804](https://github.com/developing-today/code/pull/8804):
**110 entries (41 top-level + 69 named sub-designs)** across Atlas, ID
& fleet, PM stack, Cloudflare, Turso, and adjacent infra, plus 131
idea-engine entries. A gap pass (PR
[#8840](https://github.com/developing-today/code/pull/8840)) re-walked
the record and raised it to **117 entries** — the "nothing dropped"
invariant applied retroactively to two months of directives. The
tracking manifest (PR
[#8860](https://github.com/developing-today/code/pull/8860)) recorded
37 tracking issues and 27 living docs filed in the follow-up round;
the register items were boarded on Project #4 with a dedicated status
vocabulary, the 131 ideas on a new Project #5, and 9 still-live items
that had never been issued were filed on the spot (#8806–#8811,
#8838, and two cross-repo items).

**What it taught:** the mill's inventory discipline is also an audit
instrument. Two months of designs-of-designs had produced real work
that existed only in chat transcripts and session stores; the register
run is the reason "gap passes walk the raw record" is an invariant and
not a nicety.

### 3.6 2026-11-12 — four full cycles in one day

By November 12 the intake half of the mill (Muse decomposes, files,
scaffolds, enqueues; a summit session owns the elaboration) ran four
times in a single day:

1. **PM stack hierarchy + genealogy.** Drew's directive: many boards
   in parent/sibling relations, every task's ancestry walkable to the
   top ("bugfix X, part of epic Y, spawned by design Z, under roadmap
   item A, under roadmap priority CC"). Starter epic
   [#9060](https://github.com/developing-today/code/issues/9060);
   design merged via PR [#9064](https://github.com/developing-today/code/pull/9064)
   (`doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy/`,
   16 sections); **12 implementation slices** filed as native
   sub-issues (#9062, #9065–#9071, #9073–#9076).
   *Lesson:* the PM-stack coherence epic #1184 was already at GitHub's
   **100-sub-issue-per-parent cap**, so #9060 could not be natively
   parented under it — the design added the rule that grouping epics
   near the cap get an intermediate grouping epic.
2. **Hosted editions.** PM service, ID service, and the fleet tool as
   workable Cloudflare Workers equivalents, multi-tenant on
   Turso/R2. Starter epic
   [#9078](https://github.com/developing-today/code/issues/9078)
   (native sub-issue of the Turso+Cloudflare epic #8987); scaffold
   merged via PR [#9088](https://github.com/developing-today/code/pull/9088)
   with a seeded **20-statement inventory (S1–S20)** mapped to
   components and a V1→V4 plan; **9 component issues #9079–#9087**;
   elaboration queued as task 15339. A same-day amendment (dual
   tier-1 platforms; the self-hosted tier) was folded in as a tenth
   component (#9087) before the intake even closed.
   *Lesson from the sibling wiring check:* a *stale sandbox binary*
   had made `fleet gh` look like it couldn't rotate tokens — an
   instrument fault nearly recorded as a design flaw (filed and
   closed as [#9057](https://github.com/developing-today/code/issues/9057)).
   Verify the tool before you redesign around it.
3. **Atlas v1.** Search build-out, backup mesh, schema doctrine,
   per-embed classification, continuous ingestion, virtual
   filesystem. Starter
   [#9091](https://github.com/developing-today/code/issues/9091) under
   the Atlas program epic
   [#8983](https://github.com/developing-today/code/issues/8983);
   proto design merged via PR
   [#9102](https://github.com/developing-today/code/pull/9102)
   (`doc/2026-11-12T23-59-00Z_design_atlas_v1_proto/`) whose §J maps
   **every statement of the directive to its issue**; **10 workstream
   issues #9092–#9101**; elaboration queued as task 15341. The intake
   also *amended* two existing designs' slices (#9073/#9074, binding
   them to the shared schema-version doctrine) and recorded Atlas's
   storage questions as inputs — explicitly not decisions — to the
   hosted-editions content-plane design (#9084): the amend-don't-fork
   invariant in action.
4. **Federation & namespaces** (the current instance). Extensible
   primitives, peer trust and signed data, the namespace system,
   triples and user attributes. Starter epic
   [#9106](https://github.com/developing-today/code/issues/9106)
   ("major design iteration (2026-11-12 directive)"), filed the same
   evening with acceptance written in mill vocabulary (statement
   inventory S0–S7, zero unmapped). It decomposed immediately into
   **3 part epics** — #9107 (ID core: namespaces, triples, user
   attributes), #9108 (federation & trust), #9109 (Atlas-facing:
   storage economics, benchmarks, ID search) — plus a cross-cutting
   extensibility design (#9110), a modes/configuration design
   (#9111), and the refine-and-prioritize plan
   ([#9112](https://github.com/developing-today/code/issues/9112)):
   **36 component issues (#9113–#9148)** under the parts, 42
   subordinate issues in total. Its summit elaboration is queued as
   this doc is written.

## 4. When NOT to use the mill

The mill is for directives whose decomposition *is* the work. It is
the wrong tool when:

- **The design fits in one doc a competent reader can hold whole.** A
  single-feature design with a handful of requirements should be one
  design issue, written once — milling it produces process, not
  clarity.
- **The task is a fix.** "Fix it" means fix it now: a bug with a known
  shape gets a fix and, where the fleet's rules call for it, a tracking
  issue — never a starter epic and a queue item.
- **The requirements are actually unknown.** The mill decomposes a
  directive; it does not discover one. A question that needs a spike,
  a prototype, or Drew's ruling first gets that first — the mill will
  faithfully inventory a vague brief into precisely-filed vagueness.
- **The deadline is the design.** If the value is in a decision made
  today, make the decision and file the follow-ups; the full cascade
  can run afterward against the decision as its V1.

## 5. The through-line

October proved the shape (investigations → syntheses → designs →
slices) inside the ID program. November 6 gave it an owner, a queue,
and a launch packet. November 11 turned its inventory discipline into
an audit that recovered two months of dropped work. November 12 ran
it four times in a day, at program scale, and each run left the
shared record — epics, boards, manifests, this doc — more complete
than it found it. The mill's real product is not any single design;
it is the guarantee that a directive of any size ends as a complete,
tracked, buildable tree with nothing silently missing.
