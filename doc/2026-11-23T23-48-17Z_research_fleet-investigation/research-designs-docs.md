# Fleet program investigation — Research: designs & docs

- **Scope:** `doc/` archive fleet entries, `pkgs/fleet/doc/` runbooks, the DOC-01
  contract, `pkgs/fleet/README.md` structure, `doc/README.md` index structure,
  fleet book build plumbing, and fleet charter/roadmap docs.
- **Source checkout:** laptop checkout `~/git/code` in `developing-today/code`,
  read read-only via `~/tools/laptop-ssh.sh`. No files in that checkout were
  edited, no worktrees created, no comments posted.
- **Grounding:** every path, title and quotation below was read in that
  checkout in this pass. Anything not read directly is labelled
  **[unverified]**.
- **Archive size check:** `find doc | wc -l` = **2,901** lines including the
  `doc` root itself; `find doc -type f -o -type d | wc -l` = **2,898**.
  Top level of `doc/` alone: 563 entries (`ls doc/ | wc -l`). This matches the
  brief's “~2,900 paths”.

---

## 1. Fleet entries in the `doc/` archive

### 1.1 How the list was derived

```sh
find doc -maxdepth 1 | sort | grep -i -E "fleet|queue|broker|placement|dispatch|secrets|bridge|quota|agent"
find doc | grep -i -E "fleet|queue|broker|placement|dispatch|secret|bridge|quota|agent|lifecycle" | sort
```

The second (recursive) grep is dominated by two large packages —
`2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/` (~180 paths:
`census/`, `components/`, `contracts/`, `decisions/`, `issues/bodies/`,
`research/`, `reviews/`, `sources/`, `validation/`) and
`2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/sources/` (11 files) —
plus false positives inside unrelated spikes (e.g. `legacy-secret-key.toml`
under a groups/delegation spike). The top-level list is the useful census.

### 1.2 Top-level `doc/` entries matching the brief's terms (complete, 28)

Dated folders/files:

| Path in `doc/` | Kind in name |
|---|---|
| `2026-10-29T11-27-00Z_investigation_bridge_semantics_hosted_native/` | investigation — bridge |
| `2026-11-03T15-10-56Z_design_task-placement-framework-v1/` (+ `placement-policy.yaml`) | design — placement |
| `2026-11-04T11-45-48Z_design_fleet_multicall_binary/` | design — fleet |
| `2026-11-08T06-40-00Z_design_pm-stack_quota-throughput-contract.md` | design — quota (file, not folder) |
| `2026-11-08T13-55-51Z_design_agent_note_placement/` | design — agent-note placement |
| `2026-11-09_design_id6300_agents_in_worlds_v1.md` | design — agents (ID program, not fleet binary) |
| `2026-11-09T05-00-00Z_feature_mqtt_device_bridge_id_device_v1/` | feature — bridge (ID device) |
| `2026-11-10_design_id6301_agents_as_citizens_v2_outline.md` | design — agents (ID) |
| `2026-11-10_design_queue_task_data_model_3424.md` | design — queue |
| `2026-11-10T20-00-00Z_design_agent_note_placement.md` | design — agent-note placement (file) |
| `2026-11-11_design_id8579_queue_schema_wipe_test_audit.md` | design/audit — queue schema |
| `2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/` (`note.md` + `sources/`) | design — fleet CA / credentials / secrets |
| `2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/` | design — PM-stack + fleet orchestration |
| `2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md` | design — agents (ID) |
| `2026-11-14T18-05-00Z_design_fleet_quota_admission/` | design — fleet quota |
| `2026-11-16_design_id6303_hosted_agent_services_v4_outline.md` | design — hosted agent services (ID) |
| `2026-11-16T09-20-00Z_inventory_legacy_agent_notes_7718/` (`manifest.json`, `manifest.md`) | inventory — agent notes |
| `2026-11-20T13-35-00Z_design_id10662_hosted_agent_service_listing/` | design — hosted agent service |
| `2026-11-21T11-30-00Z_design_fleet-agent-front-door/` | design — fleet agent |
| `2026-11-22T14-28-45Z_design_fleet-tool-workers-edition/` | design — fleet tool, Workers edition |
| `2026-11-22T22-35-00Z_design_quota-plane-stage-3/` | design — quota plane |
| `2026-11-23_design_id1067-account-quotas.md` | design — account quotas (ID F12.10) |
| `2026-11-23T05-00-00Z_design_bridge_v1_implementation/` | design — bridge v1 implementation |

Undated / non-dated top-level entries:

| Path | Notes |
|---|---|
| `doc/agents/` | Directory — see §1.4 |
| `doc/agent-stack-notes.md` | Agent stack notes (opencode/OpenChamber/etc.) |
| `doc/fleet/` | Directory — see §1.4 |
| `doc/fleet-execution-handoff.md` | Fleet command execution / handoff |
| `doc/fleet-machine-registry.yaml` | Machine registry data |
| `doc/fleet-schedule-registry.md` | Schedule registry |
| `doc/placement-intake/` | `fleet-place.reference`, `fleet-surfaces.reference.json`, `placement.md` |

Related but outside the name grep, found in the recursive pass and relevant
to the coordinator: `doc/2026-10-30T11-55-00Z_design_grant_lifecycle.md`
(lifecycle), `doc/decisions/FDR-0004-manual-only-queue-items.md` and
`FDR-0005-fleet-run-identity-transaction-log.md` (queue / fleet-run),
`doc/living/one-queue-model.md`, `doc/living/quota-full-utilization-burn-plan.md`,
`doc/living/fleet-roles-minder-catcher-advocate.md`,
`doc/living/fleet-sweep-review-job-family.md`,
`doc/living/opencode-2-fleet-migration.md`,
`doc/living/saas-service-id-atlas-fleet.md`.

**Caution on names:** `*_agent_note_placement*` and `*_agents_in_worlds / as_citizens / shared_agent_worlds*`
are **not** fleet-binary placement. The former is about where agent *notes*
live (see §1.3 #15); the latter are ID-program world designs. Name-grep alone
over-counts “fleet placement”.

### 1.3 The ~15 most significant — title + one-line gist

Heads read: first ~30 lines of the main `.md` in each folder/file.

1. **`2026-11-04T11-45-48Z_design_fleet_multicall_binary/2026-11-04T11-45-48Z_design_fleet_multicall_binary.md`**
   — *“One `fleet` multicall binary: search, queue, dispatch, proxy”*
   — Design record 2026-11-04, “Step 0 of epic developing-today/code#5412 —
   Drew's 2026-11-04 ruling to consolidate the fleet's tools into ONE
   multicall binary, `fleet`”; the founding design `pkgs/fleet/README.md`
   itself cites in its first paragraph. Design only, ends with a go/no-go
   decision list (§8).

2. **`2026-11-03T15-10-56Z_design_task-placement-framework-v1/…md`** (+ `placement-policy.yaml`)
   — *“Design: task-placement framework v1 — decision rules and a shared-pool efficiency model”*
   — Issue #2024 (epic #2055); decision rules for where tasks run, with a
   v1.1 amendment 2026-11-22 adding the machine dimension; states the POOL
   MAP wins over this doc on any disagreement.

3. **`2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`** — `main.md`:
   *“PM Stack & Fleet orchestration — package guide”* (package #9196)
   — The largest fleet design package in the archive: two coordinated tracks
   (A: priority/urgency/PM visibility/dispatch #9199–#9204; B: work-item
   lifecycle/hydration/dependencies/journal/leases/outbox #9206–#9209),
   shared `architecture.md` (“Keep one existing Fleet queue/broker as the
   claim and canonical work-state authority”), 8 locally-adopted FDRs
   (`decisions/FDR-PMO-001…008`), census, crosswalk, and ~100 issue bodies
   including `issues/bodies/DOC-01.md` — the source package DOC-01 came from.

4. **`2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/note.md`**
   — *“Fleet CA, credentials & secrets — v0.3 design mill: intake note”*
   — Program epic #9155, domain starters #9156–#9167; canonical in-repo home
   of the v0.3 intake, with `sources/` holding the governing spec verbatim
   (`fleet_architecture_spec_v0_3.md`, §§0–49 + Appendix A),
   `SECRETS_BACKEND_RESEARCH_2026-11-06.md`, decision review, handoffs and
   `SHA256SUMS`; precedence and binding constraints (e.g. ML-DSA-44 baseline)
   are stated in `note.md`.

5. **`2026-11-21T11-30-00Z_design_fleet-agent-front-door/…md`**
   — *“Fleet agent front door: one way in for every agent”*
   — Date 2026-11-21, “design, implemented in the same change”; code
   `pkgs/fleet/crates/fleet-agent` (`fleet agent`, `fleet shell`), rests on
   FDR-0013; one onboarding/task-intake ladder for every agent kind, with
   `fleet` as the only binary an agent talks to.

6. **`2026-11-14T18-05-00Z_design_fleet_quota_admission/…md`**
   — *“Fleet quota admission: protect capacity before launching work”*
   — “Status: implementation proposal, not deployed policy. Tracks: #2732.”
   Pre-launch quota admission projection (stopped / walled / pace_hold /
   unknown / eligible), complementing the reactive breaker in #9950; all
   windows bind, unknown is never treated as zero/available.

7. **`2026-11-22T22-35-00Z_design_quota-plane-stage-3/…md`**
   — *“Design (quota plane, Stage 3): predictive, self-tuning quota operations”*
   — Issue #1177, under epic #1149 and Stage-3 epic #12694; pace forecasting
   driving placement, pool re-weighting **proposals as PRs, never silent
   config edits**, anomaly detection, weekly design review.

8. **`2026-11-08T06-40-00Z_design_pm-stack_quota-throughput-contract.md`**
   — *“pm-stack quota/throughput design contract (#851, #3933)”*
   — The joined model the pm-stack aggregator renders: one `QuotaView`,
   pools/windows, burn rate + active sessions, burn history, and
   `quota.choices` guidance (`hasHeadroom`, `burnBeforeReset`, `doNotStart`).

9. **`2026-11-10_design_queue_task_data_model_3424.md`**
   — *“Design brief: durable repo-queue task data model”*
   — Design-mill intake brief for #3424 (not the design itself): six pieces
   including absolute `not_before` for parks and blockers as typed links
   (to tasks/PRs/issues) rather than text; companion claim-side issue #3418.

10. **`2026-11-11_design_id8579_queue_schema_wipe_test_audit.md`**
    — *“Wipe-test audit of the fleet-queue database schema”*
    — Issue #8579, parent design #8575; audits the 9 tables of the
    fleet-queue Turso store (schema of record in
    `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`) and classifies every
    field as Coordination / Durable truth / GitHub mirror cache.

11. **`2026-11-22T14-28-45Z_design_fleet-tool-workers-edition/…md`**
    — *“Design (component C): fleet tool, Workers/TypeScript equivalent — hosted Metadata stamping/query, session provenance, sync tools”*
    — Issue #9081, component C of hosted-editions epic #9078; hosted/native
    parity-or-deliberate-divergence for the fleet tool, both tiers tier 1.

12. **`2026-10-29T11-27-00Z_investigation_bridge_semantics_hosted_native/…md`**
    — *“Investigation: Bridge Semantics — Hosted (HTTPS/WS) Edition Syncing with Native Iroh Nodes”*
    — Issue #2058 (ID wave-1). Verdict: “The bridge is a sync peer, not a
    translator and not a proxy.” Fleet-relevant because `fleet bridge` is
    the client domain; the semantics ruling itself is ID-side.

13. **`2026-11-23T05-00-00Z_design_bridge_v1_implementation/…md`**
    — *“The hosted signed-object bridge v1 — implementation record (#3818)”*
    — Production shape of #12, in `pkgs/id/hosted-worker`; restates the
    sync-peer ruling; the #2276 PoC is the semantic oracle.

14. **`doc/fleet-execution-handoff.md`** (undated)
    — *“Fleet command execution and handoff”*, “Current handoff reference:
    2026-11-22” — `fleet queue dispatch` / `repo-queue dispatch` dry-run
    unless `--dispatch`, runner workflow `.github/workflows/fleet-command.yml`,
    standard image `ghcr.io/dezren39/fleet-build:full`. This is the operational
    companion to `pkgs/fleet/doc/agents/remote-build-and-test.md` (§2).

15. **`2026-11-08T13-55-51Z_design_agent_note_placement/…md`**
    — *“Design: where agent notes live — scratch, memory, repo, Zettelkasten, or ID graph”* (issue #6022)
    — Included to disambiguate: this is **note** placement, not task
    placement. Rule: choose the canonical home by what must remain
    authoritative — ID object / repo document / Zettelkasten / scratch-memory.

### 1.4 Undated fleet homes inside `doc/` — full listings

**`doc/fleet/`** (16 entries + 1 subdir, from `ls`):

- `2026-10-19-reliability-review.md` — *“Reliability review, 2026-10-19”*:
  dated snapshot of one session (what broke/changed), state at commit
  `e509884`; durable mechanisms live in `services.md` / `mcp.md`.
- `agent-events-turso.md` — *“agent.events — Turso sink, coupling verdict,
  and cutover (#5690)”*: companion to the queue Turso cutover.
- `archer-slack/` — `aster-usage-log.md`, `dot-archer-authority.md`,
  `slack-agent-design.md`, `slack-app-manifest.json`,
  `socket-mode-collector.md`. **[gist unverified — directory listed, files not read]**
- `creation-outbox.md` — **[unverified — listed, not read]**
- `generic-containers.md` — **[unverified — listed, not read]**
- `gh-credential-lifecycle.md` — *“fleet gh: credential lifecycle and current
  limitations”*: current behaviour as implemented, source of truth
  `pkgs/fleet/crates/fleet/src/gh.rs` + `fleet-secrets`; closes #10744.
  (Also republished inside the package as `pkgs/fleet/docs/fleet-gh-credentials.md`.)
- `mcp.md` — *“MCP servers”*: every MCP server runs under mcpx; wiring,
  auth, inventory as of its heading date.
- `mcpx-extraction.md`, `mcpx-identity-transport.md`,
  `mcpx-oauth-authorization-server.md`, `mcpx-opencode-plan.md` —
  **[unverified — listed, not read]**
- `queue-fallback-mode.md` — *“Queue fallback mode — GitHub-native claiming
  when the backend is down”*: status ACTIVE whenever the backend is
  unreachable (Drew's order 2026-11-06); tooling `fleet queue fallback`.
- `queue-turso-cutover.md` — *“Queue core on Turso — cutover runbook
  (#6741)”*: “CUTOVER EXECUTED 2026-11-07T05:10Z”; production broker runs
  the Turso backend against `fleet-queue`.
- `repo-universe.yaml`, `restore-procedure.md`, `services.md`
  (*“The long-running services: OpenChamber, OpenCode, mcpx”*, written
  2026-10-19), `turso-databases.md` — services/databases/restore are
  **[listed; only `services.md` head read]**.

**`doc/agents/`** (9 files, from `ls`):

- `fleet-command-channel.md` — *“Central commands: the fleet command
  channel”*: Central can send any fleet agent a message/command via the
  bridge mailboxes; client is `fleet bridge`; short way is `fleet agent next`.
- `gh-cache-daemon.md` — **[unverified — listed, not read]**
- `onboarding-an-agent.md` — *“Onboarding an agent to the fleet”*: every
  agent joins the same way (`fleet agent kinds` / `enroll` / `next`);
  design link to the fleet-agent-front-door record (§1.3 #5).
- `one-queue-claiming-work.md` — *“The one queue: claiming work”* (moved
  verbatim from `AGENTS.md` by task 15116 / #8430): “There is ONE fleet
  queue (one broker, one tasks table)”; default claim is unfiltered.
- `operator-script-argument-contract.md`, `owner-preferences.md`,
  `parallel-writers-and-subsessions.md`, `testing-doctrine.md`,
  `web-retrieval-fallback.md` — **[unverified — listed, not read]**

**`doc/decisions/`** — the Fleet Decision Records (FDR) index home:
`README.md`, `TEMPLATE.md`, `FDR-0001-resume-by-session.md` through
`FDR-0013-no-owner-decision-gates.md`. Heads read for two:
FDR-0004 *“Manual-only queue items: a first-class flag, not a park or a low
priority”* (2026-11-17, accepted) and FDR-0005 *“Fleet init/run identity,
GitHub carriage, and a backed-up transaction log”* (2026-11-17, proposed —
“not adopted by filing”). Full FDR list is from `ls`; other FDR gists
**[unverified]**.

**`doc/living/` fleet-relevant:** `one-queue-model.md` (*“One-queue model —
gap analysis and designation (2026-11-10)”*, status: shipped, PR #8438),
`quota-full-utilization-burn-plan.md` (*“Quota full-utilization program +
burn plan”*, first edition code#2173, built 2026-10-29), plus
`fleet-roles-minder-catcher-advocate.md`, `fleet-sweep-review-job-family.md`,
`opencode-2-fleet-migration.md`, `saas-service-id-atlas-fleet.md`
**[listed, not read]**.

**`doc/placement-intake/placement.md`** — *“Fleet placement: where build jobs
run”*; header warns: “Current operational instructions live in
[doc/fleet-execution-handoff.md]… several status notes below are historical.”

---

## 2. Runbooks & agent docs — `pkgs/fleet/doc/`

### 2.1 Full listing (verified: this is the whole tree)

```
pkgs/fleet/doc/
└── agents/
    ├── documentation.md
    └── remote-build-and-test.md
```

There are no other files under `pkgs/fleet/doc/`. (The package's other docs
live elsewhere: `pkgs/fleet/docs/verbs.md` and
`pkgs/fleet/docs/fleet-gh-credentials.md`, and the book in `pkgs/fleet/book/`
— see §6.)

### 2.2 One-line gist each

- **`pkgs/fleet/doc/agents/documentation.md`** — *“Documentation: the fleet
  book is part of the change”*: the agent-facing statement of DOC-01
  extended to the book (full text in §3.3); also gives build/preview
  commands and notes deployment is combined with the mcpx book's.
- **`pkgs/fleet/doc/agents/remote-build-and-test.md`** — *“Remote builds and
  tests through Fleet”*: the dispatch-first runbook, summarised in §2.3.

Both are cited from `pkgs/fleet/README.md`'s Layout section:
`fleet-bridge` → “Runbook: doc/agents/fleet-command-channel.md” is in the
**repo-root** `doc/agents/` (§1.4), not this folder — do not confuse the two
`doc/agents/` trees; `fleet-agent` → onboarding runbook likewise points to
the root tree, while `fleet-queue` → “Remote build/test dispatch runbook:
doc/agents/remote-build-and-test.md” points to **this** folder (paths in the
README are relative to `pkgs/fleet/`).

### 2.3 `remote-build-and-test.md` — summary (read in full)

**Rule (its heading):** “dispatch first — do not compile on the sandbox or
the laptop.” Builds/tests/CPU-heavy work run on dispatched GitHub Actions
runners; the flow is push → dispatch → track asynchronously.

- **Submit/track:** `fleet queue dispatch --repo <owner/name> --ref <ref>
  --command '<cmd>' --dispatch --json` (dry run without `--dispatch`);
  track with `fleet queue session status/result <HANDLE>`; full log via
  `gh run view <RUN_ID> --repo <RUNNER_REPO> --log`. Keep `handle`,
  `run_id`, `runner_repo`, `portable_id` (`OWNER/REPO#RUN_ID`), `run_url`.
- **Placement:** runner pool chosen by live load — developing-today org
  (20 concurrent jobs) and dezren39 account (40) as stated in the doc;
  GitHub-hosted default, Blacksmith a budgeted fallback (paid minutes need
  `--allow-paid-blacksmith`), macOS via `--platform macos`; caches keyed
  for reuse; `--followup` pins a repeat job to the previous run's host/outputs.
- **Choosing a runner:** omit `--runner` for automatic placement;
  `--container`, `--actions-cache` / `--actions-cache-restore-key`,
  `--push-nix-cache` options.
- **Other targets:** `fleet shell` for laptop / Codespace / local
  (`fleet shell targets`; `--target laptop|codespace|local|actions|auto`;
  `auto` never picks `actions`; `--build` takes a laptop build-lock slot);
  detached laptop jobs via `fleet laptop-agent poll/fetch` — distinct from
  Actions dispatch; `fleet queue dispatch --surface laptop` “currently
  creates a handoff record; it does not execute arbitrary commands”;
  `fleet dispatch launch` submits to pm-dispatch.
- **Portable IDs / local state:** `fp_…` handles in
  `~/.local/state/fleet-place/sessions.json`, mirrored best-effort to the
  queue backend's `placement_sessions` table.
- **Sandbox seats:** static `fleet` build from `scripts/muse/bootstrap.sh`;
  pre-#13122 builds fail with `UnknownIssuer` under the egress proxy — the
  doc's remedy is update the binary or route proxy vars through
  `http://broker:broker@127.0.0.1:3130` (address as printed in the doc).
- **Other routes:** `fleet dispatch-test scan|submit|status` for
  fleet-umbrella submodules; raw `gh workflow run fleet-command.yml` /
  `fleet-run.yml` fallback; “No `queue dispatch` verb? Your binary is
  stale” recovery via the bootstrap / pinned release in
  `scripts/muse/bootstrap.sh`.

---

## 3. The DOC-01 contract

### 3.1 Where it lives

`pkgs/fleet/README.md`, heading (line 277):

> ## Operator and contributor contract (DOC-01, #10728)

Opening lines (paraphrase, with source designs named in the text): the queue
surface's contract with operators — lifecycle relations, authority, error
posture and privacy — sourced from components #9206, #9208, #9209 of the
pm-orchestration design (§1.3 #3); “A machine check ties it to the registry:
`pmo_operator_docs_match_registered_help_inventory`
(`crates/fleet-registry/tests/pmo_operator_docs.rs`).”

Subsections: *The inventory is the registry* / *Designed spellings — not yet
in the registry* / *Dependency semantics — typed edges only* / *Authority and
attribution* / *Privacy posture — private by default* / *Honesty contracts* /
*Contributor contract*.

### 3.2 Operative text — the book-chapter rule (verbatim, short excerpt)

From the “inventory is the registry” subsection of DOC-01 in
`pkgs/fleet/README.md`:

> “[`book/`](book/) (mdBook sources, `book.toml` at this package's root) is
> the complete, self-contained reference for the binary — every domain,
> subcommand, flag, default and operational procedure — and **any change to
> a command, flag, default, capability, surface or procedure updates the
> matching chapter in the same change**.”

And the next sentence records where the rule is recorded for agents:
in `doc/agents/documentation.md` (i.e. `pkgs/fleet/doc/agents/documentation.md`)
and in the book's own “Keeping this book current” chapter; the book builds
with `nix build .#fleet-docs`; `create-missing = false` makes a missing
chapter a build failure, never a stub.

### 3.3 The same rule in the other two recorded places

The book chapter `pkgs/fleet/book/development/keeping-this-book-current.md`
states that the rule “lives in three places, which must agree”:
(1) `pkgs/fleet/README.md` — DOC-01 itself; (2)
`pkgs/fleet/doc/agents/documentation.md`; (3) that chapter itself. If they
disagree, “the README's DOC-01 text wins for command-contract questions and
this chapter wins for book-scope questions”.

- **`pkgs/fleet/doc/agents/documentation.md`, heading “The rule (DOC-01, extended)”:**
  the README contract already requires the registry and
  `pkgs/fleet/docs/verbs.md` to be updated in the same change; the book is
  part of the same contract — any change to a command, subcommand, flag,
  default, exit code, capability, surface-registry entry, broker behaviour
  or operational procedure “**updates the matching book chapter in the same
  PR**”; designed-but-unshipped spellings are never presented as shipped.
- **`keeping-this-book-current.md`, heading “The DOC-01 contract”:** paraphrases
  the contributor paragraph (update `crates/fleet-registry/src/lib.rs` and
  `docs/verbs.md` in the same change; decision record for settled semantics),
  then, under “The extension: the book is part of the same change”:

  > “**Any change to a command, a flag, a default, a capability, a surface,
  > or an operational procedure updates the matching chapter of this book in
  > the same pull request.**”

  It adds: a follow-up issue promising to update the book later “does not
  satisfy the rule”, and enumerates the concrete per-change updates
  (verb/flag → reference chapter + `verbs.md`; new domain → register-a-domain
  + layout; changed default → every chapter stating the old default; changed
  procedure → matching operations chapter; changed book deployment → the
  deploying chapter).
- **Contributor contract (README, DOC-01's last subsection), in substance:**
  keep the command contract together — registry + `docs/verbs.md` updated in
  the same change, including nested leaves and examples; never present a
  designed spelling as shipped (runtime dispatch is the authority); a change
  to any settled semantic recorded there (claim/park/resume, supervision and
  stop authority, cap policy, attribution/logging) requires a decision
  record in the same body of work (FDR index at `doc/decisions/README.md`).

Other in-book DOC-01 references (from `grep -rn "DOC-01" pkgs/ --include="*.md"`):
`book/development/add-a-subcommand.md` step 7, `add-a-flag.md`,
`register-a-domain.md` (registry table + step 8), `layout.md`,
`testing.md` (the DOC-01 machine test). All are in `pkgs/fleet/book/`.

### 3.4 Does a DOC-01-style contract appear elsewhere? (`pkgs/id`, mcpx)

**No — verified by grep.** `grep -rn "DOC-01" pkgs/ --include="*.md"` returns
hits **only** under `pkgs/fleet/` (README, `doc/agents/documentation.md`, and
the book chapters above).

- **`pkgs/id/README.md`:** no `DOC-01`, and no book-currency rule found —
  `grep -n "book\|DOC-01\|documentation" pkgs/id/README.md` returns only its
  `## Documentation` link list (ARCHITECTURE.md, WEB.md, web/README.md,
  docs/, AGENTS.md) plus the new “ID program” subsection (§4.2). **[No
  equivalent contract was found in the parts searched; a full read of
  `pkgs/id/ARCHITECTURE.md` / `AGENTS.md` for an analogous rule was not done
  — unverified beyond `pkgs/id/README.md`.]**
- **`pkgs/mcpx/README.md`:** headings-only scan (`grep -n "^#\|^##\|DOC-01\|book"`)
  shows no DOC-01 heading and no “DOC-01” hit anywhere in `pkgs/` outside
  fleet. mcpx does have its own book plumbing that fleet's mirrors
  (§6), but no DOC-01-style named contract was found in its README.
  **[Full-text search was the `pkgs/`-wide DOC-01 grep; a differently-named
  mcpx documentation rule would not have been caught — unverified.]**

Note the direction of the exemplar relationship in the sources: fleet's
`docs.nix` and `book.toml` **mirror mcpx's** book setup (#1939/#1941), while
DOC-01 itself is fleet-originated (from the #9206/#9208/#9209 DOC-01 slice,
issue #10728).

---

## 4. `pkgs/fleet/README.md` structure

### 4.1 Section outline (headings only, with line numbers from `grep -n "^#"`)

```
1    # fleet — the multicall fleet binary
20   ## Layout
139  ## Session identity in worktrees (#9075)
171  ## Versioning — derived per-app versions (#8566; policy #5790)
199  ### Tag and release procedure (ruled on #5790, 2026-11-06)
235  ## Variants
258  ## The stub contract
277  ## Operator and contributor contract (DOC-01, #10728)
289  ### The inventory is the registry
329  ### Designed spellings — not yet in the registry
349  ### Dependency semantics — typed edges only
358  ### Authority and attribution
367  ### Privacy posture — private by default
389  ### Honesty contracts
409  ### Contributor contract
423  ## Operator reference
438  ## Build and smoke
447  ## Deploy: the two live installs (read this before merging a new verb)
499  ## Location-aware init: provision before the binary flip (#5687, impl #6770)
```

First paragraph (grounding for a pointer): the README opens by naming the
epic (developing-today/code#5412) and citing the founding design note
`../../doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/…md`, then
narrates steps 1–6 (#5429, #5430, #5431, #5432, #5433, #5554, #5604).

`## Operator reference` (line 423) currently has exactly two bullets:
`docs/fleet-gh-credentials.md` (#10744) and the `doc/decisions/` FDR index,
with the same-body-of-work rule restated.

### 4.2 Where a “Fleet program” pointer would fit — mirroring PR #13243

**How PR #13243 did it for ID (verified in git log/show):**

- Merge commit `af119693476` — “Merge pull request #13243 from
  developing-today/lane-ui-13240 — doc: ID program investigation &
  build-out prompt (#13240)”; single content commit `2104be5b15b`
  (2026-11-23 06:07 CST).
- Commit message: “Adds the program prompt … as a dated design record under
  doc/, linked from the doc map's dated-records starting points and from a
  new ID program subsection in pkgs/id/README.md.”
- Diffstat: **3 files, 242 insertions** — the new dated record (236 lines),
  `doc/README.md` (+2), `pkgs/id/README.md` (+4).
- In `pkgs/id/README.md` the addition is a new `### ID program` subsection
  **inside the existing `## Documentation` section, immediately before
  `## License`** (diff hunk at line 412).

**Fleet mirror:** `pkgs/fleet/README.md` has **no `## Documentation` section
and no `## License` section** — its closest analogue is `## Operator
reference` (line 423), a two-bullet link list sitting between DOC-01
(line 277–421) and `## Build and smoke` (line 438). A fleet-program pointer
mirroring #13243's shape would therefore be either:

- a new `### Fleet program` subsection under `## Operator reference`, or
- a new top-level `## Fleet program` section inserted after
  `## Operator reference` and before `## Build and smoke` —

with the same 3-part content as the ID subsection: tracking issue link,
charter = program-prompt dated record under `doc/`, program epics, and
deliverables (fleet book under `/code/fleet/`, prioritised issue map).
**[Recommendation, not a fact in the repo: the coordinator should pick one
of the two placements; no fleet-program section exists yet — verified by
the heading outline above.]**

Path caveat for the mirror: in `pkgs/id/README.md` the charter link is
`docs/2026-11-23T12-06-42Z_design_id_program_investigation_prompt/…` (id's
`docs/` reaches the repo-root `doc/` archive). From `pkgs/fleet/README.md`,
existing links to the archive use `../../doc/…` (e.g. the FDR index link
`../../doc/decisions/README.md` and the founding-design citation
`../../doc/2026-11-04T11-45-48Z_…`). A fleet entry should use the
`../../doc/<dated-folder>/<file>.md` form.

---

## 5. `doc/README.md` index structure

### 5.1 Structure of the file (title: “# Documentation map”)

- Opening paragraph: this page says where each kind of document lives and
  how it changes; placement rules in
  `planning/DOCUMENTATION_ARCHITECTURE.md`, dated-record protocol in
  `doc/DOCUMENTATION_PROTOCOL.md`.
- `## Where things live` — bullets for repository root, `doc/<UTC>_<kind>_<name>/`
  dated records, `doc/<UTC>_research_<slug>/` research packages,
  `doc/references/`, `doc/decisions/` (FDR index), `planning/`, package docs,
  agent skills.
- `## Research packages` — the request catalog; one line per package.
- `## Dated records` — the starting-points list (see §5.2).
- Closing sections: the two pattern-breaking names note, `## Legacy places`,
  `## Legacy notes` (kept verbatim legacy text).

### 5.2 The “dated-records starting points” section — exact format

Heading and lead-in (verbatim):

> ## Dated records
>
> Folders are named `<UTC datetime>_<kind>_<name>`. The kinds in use are
> `feature`, `design`, `architecture`, `phase`, `fix`, `reference` and
> `research`. Run `ls doc/` to list them all. Some starting points:

**The ID entry added by PR #13243** is the **first bullet** of that list.
The diff in commit `2104be5b15b` adds exactly these two lines (entry + blank
line) immediately after “Some starting points:”:

> - [ID program investigation & build-out prompt](2026-11-23T12-06-42Z_design_id_program_investigation_prompt/2026-11-23T12-06-42Z_design_id_program_investigation_prompt.md) — the charter prompt for the ID program tracked in #13240: full investigation and design completion for the ID platform (epics #2056 and #9106), new designs for the obvious next steps, an ID book published under `/code/id/`, and a prioritized map of all ID issues with blockers marked.

Format to mirror for a fleet entry, as shown by this and its neighbours:

- `- [<Title>](<folder>/<same-named>.md) — <one-sentence gist>.`
- Link path is relative to `doc/` (no `doc/` prefix).
- The gist names: what the record is, the tracking issue number, the
  program epics, and the deliverables.
- It is placed at the **top** of the starting-points list (newest program
  first), ahead of the C8 Atlas entry; note the list is **not** strictly
  chronological below that — the Atlas program entry (#13241) sits third,
  after the C8 entry, per the current file.
- Blank line after the entry in the #13243 diff (the list's existing
  spacing is irregular — some entries are blank-line separated, later ones
  are consecutive).

Current top of the list, in order (for placement context): ID program
(#13240) → C8 Atlas ingestion (#9576) → Atlas program (#13241) → Atlas
v2–v4 (#6020) → Partial replication (#5800) → …

---

## 6. The fleet book build plumbing

### 6.1 `pkgs/fleet/book.toml` (read in full)

Header comment: “mdBook configuration for the fleet book (#13165). The book
root is pkgs/fleet itself so the sources stay next to the binary they
document: plain markdown under book/, readable on GitHub exactly as written,
and rendered as a book via the .#fleet-docs flake output
(pkgs/fleet/docs.nix).”

```toml
[book]
title = "fleet"
authors = ["developing-today"]
language = "en"
src = "book"

[build]
build-dir = "rendered"
# Every chapter in SUMMARY.md must exist on disk; never invent stubs.
create-missing = false

[output.html]
git-repository-url = "https://github.com/developing-today/code"
```

Note: the book's own “Deploying this book” chapter says build directory
`book` — `book.toml` says `rendered`. **[Discrepancy in the sources as read;
book.toml is the config of record.]**

### 6.2 `pkgs/fleet/docs.nix` — flake package `fleet-docs`

- Header: “The fleet book (#13165): render pkgs/fleet/book with mdBook into
  static HTML, sandboxed, with the flake-pinned mdbook from nixpkgs and no
  network access. Exposed as the .#fleet-docs flake output… Mirrors
  pkgs/mcpx/docs.nix (#1939, Approach C) minus the mcpx-specific Go
  preprocessors.”
- Derivation: `runCommand "fleet-docs"`; inputs are only `book.toml` + the
  `book/` tree (`lib.fileset.toSource`); links escaping the book tree
  (`../`, `../../`) are rewritten in the copied pages to the repo URL
  `https://github.com/developing-today/code/blob/main/pkgs/fleet/`.
- **`siteUrl` argument (the #1941 pattern):** default `"/"`; baked in via
  `MDBOOK_OUTPUT__HTML__SITE_URL`; the Pages deploy overrides it with
  `"/code/fleet/"` — e.g. `fleetDocs.override { siteUrl = "/code/fleet/"; }`.
- Flake wiring (verified): `lib/default.nix` line 740
  `fleetDocs = pkgs.callPackage ../pkgs/fleet/docs.nix { };` and line 793
  `fleet-docs = fleetDocs;` in the packages set. Build: `nix build .#fleet-docs`.

### 6.3 Book source tree (`pkgs/fleet/book/`, from `ls -R` + `SUMMARY.md`)

`SUMMARY.md` + `index.md` and six chapter groups:

- `getting-started/` — installation, quickstart, shape
- `concepts/` — architecture, queue, placement, secrets, bridge, agents,
  proxy, search, gh, dispatch (10 chapters)
- `reference/` — builtins, queue-core, queue-lifecycle, queue-fallback,
  queue-dispatch, queue-admin, serve, proxy, dispatch, dispatch-test,
  secrets, gh, bridge, agent, shell, search, laptop-agent (17 chapters)
- `tutorials/` — enrol-a-seat, claim-and-complete, dispatch-a-build,
  resolve-a-secret, bridge-messaging, run-the-brokers (6)
- `library/` — crates, queue-client, placement, registry-and-config (4)
- `development/` — layout, building, testing, add-a-subcommand, add-a-flag,
  register-a-domain, release-and-rollout, keeping-this-book-current (8)
- `operations/` — services, health-and-healing, troubleshooting,
  deploying-this-book, faq, glossary (6)

### 6.4 Workflows — who builds, who deploys

- **`.github/workflows/fleet-docs.yml`** (“name: fleet docs”) — **build
  only, never deploys.** On push to `main` + `workflow_dispatch`:
  `nix build .#fleet-docs`, stage `result` → `site`, assert
  `site/index.html`. Its header states why: “A repository has exactly one
  GitHub Pages site and every Pages deployment replaces the whole site, so
  the fleet book is deployed by the mcpx-docs workflow's deploy job, staged
  under fleet/ with siteUrl "/code/fleet/" — a fleet-only deploy from here
  would wipe the mcpx book (and vice versa).”
- **`.github/workflows/mcpx-docs.yml`** (“name: mcpx docs”) — **the combined
  Pages deploy.** Its `deploy` job (main only) builds the mcpx book with
  `siteUrl = "/code/"`, stages it as `site/`, then builds the fleet book:

  ```sh
  nix build --impure --expr \
    '(builtins.getFlake (toString ./.)).packages.${builtins.currentSystem}.fleet-docs.override { siteUrl = "/code/fleet/"; }' \
    --out-link result-fleet-docs
  cp -rL result-fleet-docs site/fleet   # after chmod u+w site; asserts site/fleet/index.html
  ```

  then `configure-pages` + `upload-pages-artifact`; a separate `publish`
  job runs `deploy-pages` (pages concurrency group, one retry — the job
  split is #10956). Resulting URLs: mcpx at
  `https://developing-today.github.io/code/`, fleet at
  `https://developing-today.github.io/code/fleet/`.
- There is **no** `pages.yml` / `*page*` workflow (verified by `ls`).
  The authoritative prose for all of this is the book chapter
  `pkgs/fleet/book/operations/deploying-this-book.md`, which also records
  the **not-yet-wired** custom-domain step (no CNAME anywhere, `cname: null`)
  — do not present a custom domain as live.

### 6.5 How a new book section would be added (for the investigation prompt)

Per the sources above, extending the fleet book (not creating a new one)
means, in one change:

1. Add the chapter file(s) under the fitting group in `pkgs/fleet/book/`
   (e.g. a new `concepts/` or `operations/` chapter), each file starting
   with its exact chapter title as its single `# ` heading, internal links
   relative `.md` only (rules in `keeping-this-book-current.md`).
2. Add the entry to `pkgs/fleet/book/SUMMARY.md` in the same group —
   `create-missing = false` means a SUMMARY entry without a file (or a
   broken link) fails the build by name.
3. If the section documents a new/changed command, flag, default,
   capability, surface or procedure, DOC-01 (§3) also requires the matching
   `pkgs/fleet/docs/verbs.md` / registry updates in the same PR.
4. Verify with `nix build .#fleet-docs` (the exact CI build;
   `fleet-docs.yml` re-runs it on main). No workflow change is needed for a
   new section: the deploy in `mcpx-docs.yml` rebuilds the whole book and
   re-stages it under `site/fleet/` automatically.

---

## 7. Fleet charter / roadmap docs in `pkgs/fleet`

### 7.1 Existence check (verified)

In `pkgs/fleet/` top level, the only Markdown file is **`README.md`**
(`ls pkgs/fleet/*.md` → `README.md` only; alongside `book.toml`,
`Cargo.toml`, `rust-toolchain.toml`). Explicitly:

- **No `ARCHITECTURE.md`** in `pkgs/fleet/` (`ls pkgs/fleet/ARCH*` → no such file).
- **No `ROADMAP.md`** in `pkgs/fleet/` (`ls pkgs/fleet/ROADMAP*` → no such file).
- **No charter file** of any name found; `grep -rn -i "charter\|roadmap"
  pkgs/fleet/README.md pkgs/fleet/book/` returned **no hits**.
- **No single fleet charter exists.** **[Explicit finding, per the brief.]**

### 7.2 What plays the charter/roadmap role instead

- **`pkgs/fleet/README.md`** — the package's long-form document and closest
  thing to a charter: founding epic #5412 + founding design citation,
  step-by-step landing history (steps 1–6), Layout, Variants, the stub
  contract, DOC-01, deploy doctrine. The book's `development/layout.md`
  describes it as “the package's own long-form document, including the
  operator and contributor contract (DOC-01)”.
- **`pkgs/fleet/book/concepts/architecture.md`** — the book's Architecture
  chapter (listed in `SUMMARY.md` under concepts; **[listed, chapter body
  not read in this pass — unverified gist beyond its title/placement]**),
  and `pkgs/fleet/book/index.md` — the book's introduction, whose “Why fleet
  exists” section states the four problems fleet solves once (shared work
  queue; secrets without vault sprawl via the secrets broker; remote build
  placement; agent coordination).
- **`pkgs/fleet/docs/verbs.md`** — the canonical command reference (DOC-01
  surface), and **`pkgs/fleet/docs/fleet-gh-credentials.md`** — the `fleet gh`
  credential lifecycle reference (#10744). These are the only two files in
  `pkgs/fleet/docs/` (verified by `ls`).
- **Repo-root `ROADMAP.md`** — no fleet-specific roadmap file; fleet appears
  only inside two rows pointing elsewhere: “Repo-wide CI and automation” →
  `planning/roadmaps/repo-and-ci.md` (“…and the queue fleet program: lanes
  for every queue, OpenChamber session dispatch (#1233, #1234), pool &
  routing intelligence (#1343, #1340)”) and “The PM stack (Kaneo, Pad,
  Grist, the aggregator, the queue fleet)” → `planning/roadmaps/pm-stack.md`
  (coherence epic #1184). **[The two `planning/roadmaps/*.md` files
  themselves were not read — unverified beyond the root ROADMAP's pointers.]**
- **A charter does exist for the neighbouring PM program, not for fleet:**
  `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/sources/02_charter_v1_1.md`
  is listed among that package's sources, and its companion
  `COMPANION_pm_charter_shared_conventions.md` also appears in the secrets
  package's `sources/` with the note “Shared queue/dispatch conventions
  ONLY; the PM workstream is a separate program.” **[File names/notes
  verified from listings and the secrets `note.md` source table; the charter
  file's body was not read — unverified.]**

### 7.3 Implication for the coordinator

Because there is no fleet charter and no `## Documentation` / programme
section in `pkgs/fleet/README.md`, the fleet investigation prompt will be
creating both artefacts the ID program already had a home for: (a) the
charter/programme record itself (as a new dated record under `doc/`,
§5.2 format), and (b) its two pointers (`doc/README.md` starting-points
bullet + a new fleet-program subsection in `pkgs/fleet/README.md`,
§4.2 placement). The book, by contrast, already exists and is complete in
plumbing (§6) — the prompt should direct executors to **extend**
`pkgs/fleet/book/` under DOC-01, exactly as its own
“Keeping this book current” chapter requires.

---

## Appendix — sources consulted (all in laptop `~/git/code`)

- Listings: `find doc`, `ls doc/`, `ls -R pkgs/fleet/doc/`, `ls pkgs/fleet/`,
  `ls doc/fleet/ doc/agents/ doc/decisions/ doc/living/`,
  `ls .github/workflows/`, `ls -R pkgs/fleet/book`.
- Read (full or head ~30 lines as noted): `pkgs/fleet/README.md`,
  `pkgs/fleet/book.toml`, `pkgs/fleet/docs.nix`,
  `pkgs/fleet/doc/agents/documentation.md`,
  `pkgs/fleet/doc/agents/remote-build-and-test.md` (full),
  `pkgs/fleet/book/development/keeping-this-book-current.md` (full),
  `pkgs/fleet/book/operations/deploying-this-book.md` (full),
  `pkgs/fleet/book/SUMMARY.md`, `pkgs/fleet/book/index.md` (head),
  `doc/README.md` (full), `doc/fleet/*.md` heads, `doc/agents/*.md` heads,
  `doc/decisions/FDR-0004/FDR-0005` heads, `doc/living/*.md` heads,
  `doc/placement-intake/placement.md` head, `doc/fleet-execution-handoff.md`
  head, and the §1.3 dated-record heads.
- Greps: DOC-01 across `pkgs/**/*.md`; charter/roadmap across
  `pkgs/fleet/README.md` + `pkgs/fleet/book/`; headings of
  `pkgs/fleet/README.md`, `pkgs/id/README.md`, `pkgs/mcpx/README.md`;
  `fleet-docs` in `flake.nix` / `lib/default.nix` (hits in `lib/default.nix`
  lines 740, 793).
- Workflows read in full: `.github/workflows/fleet-docs.yml`,
  `.github/workflows/mcpx-docs.yml`.
- Git: `git log --oneline --all --grep=13243`,
  `git show af119693476 --stat`, `git show 2104be5b15b` (stat + diff for
  `doc/README.md` and `pkgs/id/README.md` + prompt head).

## Unverified / gaps (consolidated)

- Gists marked **[unverified]** in §1.4 / §7.2: files listed but not read
  (`doc/fleet/creation-outbox.md`, `generic-containers.md`, mcpx-* files,
  `archer-slack/*`, several `doc/agents/*` and `doc/living/*` files, other
  FDRs, `planning/roadmaps/repo-and-ci.md` and `pm-stack.md`,
  `book/concepts/architecture.md` body, PM charter `02_charter_v1_1.md` body).
- §3.4: the “no DOC-01 analogue” finding is exact for the literal string
  `DOC-01` across `pkgs/**/*.md`; a differently-named documentation-currency
  rule in `pkgs/id/ARCHITECTURE.md`, `pkgs/id/AGENTS.md` or mcpx docs was
  not exhaustively searched.
- §6.1: `book.toml` (`build-dir = "rendered"`) vs the deploying chapter's
  prose (“build directory `book`”) discrepancy is reported as found, not
  resolved.
- No live-system facts (deployed binaries, Pages site state, issue states)
  were checked; this file covers only the repository documents named in the
  brief.
