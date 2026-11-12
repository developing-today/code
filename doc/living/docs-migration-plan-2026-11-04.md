# Docs migration plan — hatch-personal / hatch / darwin-nix docs → developing-today/code canonical home (2026-11-04)

> Living document — committed 2026-11-11 from ~/hatch-personal/workspace/scratch/docs-migration-plan-2026-11-04/plan.md (gap-addendum entry G2; program tracked by code#5983 and batches #5976/#5978/#5980/#5981/#5982). The enumeration classified 482 / 475 / 474 tracked files across the three repos. Status: program in progress; this plan moves nothing by itself.

---

# Docs migration plan: hatch-personal / hatch / darwin-nix → developing-today/code

Drew's directive, 2026-11-04 ~22:59–23:00 CST. Plan + enumeration only — this
document does not move anything. Filed as a tracking issue + batch issues in
developing-today/code (numbers in §8).

## 1. What this is

Three repos accumulated documentation and design material that belongs with the
production code, now that the fleet binaries are built and being
productionized:

- **dezren39/hatch-personal** (private; local tree `/home/hatch`) — 482 tracked files
- **dezren39/hatch** (public) — 475 tracked files. Verified: this is a
  near-exact public mirror of hatch-personal's tracked set; the only delta is
  the 7 `repos/*` submodule entries, which exist only in hatch-personal. One
  classification therefore serves both repos; counts differ by those 7 files.
- **dezren39/darwin-nix** — 474 tracked files (the Mac nix-darwin flake repo;
  laptop primary `~/git/nix`).

developing-today/code (6,196 tracked files) becomes the canonical home for
everything that is not strictly about the sandbox environment (hatch repos) or
about making the flake work (darwin-nix).

**Copy, never delete.** hatch-personal and darwin-nix keep their historical
copies in place. This migration changes which copy is *canonical/living*, not
which copies exist. No file is removed from any source repo by this plan, and
the standing delete guardrail applies throughout.

## 2. Classification standard

Every tracked item/cluster gets exactly one classification:

- **MOVE** — canonical home becomes developing-today/code.
- **STAY** — stays canonical where it is, with a one-line reason.
- **STAY-UNTIL-REPLACED** — stays for now; the register in §6 names the missing
  Fleet feature that is its exit condition, and each gap has a GitHub issue.

Drew's tests, applied in order:

1. Is it the init script, the quota script, or sandbox boot/init machinery? → STAY.
2. Is it an update-getting or fallback script standing in for a Fleet feature
   that doesn't exist yet? → STAY-UNTIL-REPLACED (and file the Fleet gap).
3. Is it baked into / part of the system itself — prompts, AGENTS.md, avatars,
   identity/soul files, sandbox-environment notes (TOOLS.md + TOOLS.d/ are the
   archetype)? → STAY.
4. Everything else: **would this make sense to someone running the fleet/code
   without this sandbox?** Yes → MOVE. No → STAY.
5. darwin-nix specialisation: is it specifically about making the NixOS /
   nix-darwin flake work (flake structure, build/switch, module mechanics,
   machine configuration)? → STAY. Documentation about anything else → MOVE.
   **Docs about a topic move; functional flake code/config that merely uses
   the topic stays** (e.g. a module wiring a service is config, not a doc).

Scope note: this migration covers *documentation, designs, and backup
scripts*. Running service code that happens to live in the sandbox tree
(provider proxy implementations, service units) is part of the sandbox system
and stays; where such code is already superseded by a canonical copy in code,
that is recorded, not re-migrated.

## 3. Counts

| Repo | Tracked | MOVE | STAY-UNTIL-REPLACED | STAY |
|---|---|---|---|---|
| hatch-personal | 482 | 110 | 18 | 354 |
| hatch (public mirror) | 475 | 110 | 18 | 347 |
| darwin-nix | 474 | 32 | 0 | 442 |
| **Total (unique, hatch counted once)** | **956** | **142** | **18** | **796** |

Counts are per cluster; the full cluster tables are §4–§5 and the
machine-readable inventory is `inventory.json` beside this file.

## 4. hatch-personal / hatch enumeration

The two repos share one table (hatch = same set minus `repos/*`).

### 4.1 MOVE — backup scripts & backup documentation (batch: backups)

| Path / cluster | Files | Destination in code | Notes |
|---|---|---|---|
| `tools/runtime-db-export/` | 21 | `pkgs/agent-backup/` (runtime DB export machinery) | The Muse runtime-DB backup/export pipeline (schema.sql, spec_*.json, loaders, prune SQL). A database backup system — Drew named this class explicitly. |
| `tools/bws-backup-secrets.sh` | 1 | `pkgs/agent-backup/` | Bitwarden secrets backup. Known hazard (delete-then-create) already tracked elsewhere; migration must not run it. |
| `reports/` (`README.md`, `backup-process-2026-10-21.md`) | 2 | `doc/backups/` | The backup-process report is a design/process doc for the backup system. |
| `library/toolchains/binary-backup.md` | 1 | `doc/backups/` | How binaries are backed up (pairs with the nix binary-backup practice). |
| `library/skills/muse-backup.md` + `archive/small-files/workspace/custom-skills/muse-backup/SKILL.md` | 2 | `doc/backups/` | Backup skill documentation (the live skill itself lives in workspace/custom-skills, outside these repos). |

### 4.2 MOVE — Fleet designs, fleet machinery, task-system schema (batch: fleet)

| Path / cluster | Files | Destination in code | Notes |
|---|---|---|---|
| `tools/fleet-brokers/` | 13 | — (already there) | **Already migrated 2026-11-04** (code#5430): canonical broker source is `pkgs/fleet/crates/fleet-queue/` per its own SUPERSEDED.md. Hatch copy = rollback evidence / historical copy. No action; recorded so the enumeration is complete. |
| `tools/fleet-fetch.py` + `tools/fleet-build-lock` | 2 | `pkgs/fleet/` | Repo-snapshot fetcher writing to Supabase `agent.repo_snapshots`; fleet machinery, not sandbox-specific. |
| `workspace/supabase/muse-dev/migrations/` (0001–0024) | 29 | `pkgs/fleet/` (schema home; exact subdir in batch issue) | The agent/task-queue database schema (agent, repo queue, claims, engineering events). This *is* the "how Muse tasks work" substrate — belongs with the fleet code that runs against it. |
| `workspace/archer-slack/` | 6 | `doc/fleet/` + code with the slack agent | `slack-agent-design.md` is a design doc; puller is its implementation. Not sandbox environment. |
| `notes/architecture/mcpx-opencode-plan.md` | 1 | `doc/fleet/` | Design/plan for mcpx + opencode; mcpx's canonical home is code (`pkgs/mcpx`). |
| `tools/lane-claim.sh`, `tools/lane-handoff.sh`, `tools/repo-task-v1.schema.json`, `tools/enqueue-when-green.sh` | 4 | `pkgs/fleet/` | Pre-fleet-binary queue client artifacts + the repo-task schema. Borderline (see §7) — they document the queue protocol the fleet binary now owns. |

### 4.3 MOVE — database / platform tooling & architecture docs (batch: architecture)

| Path / cluster | Files | Destination in code | Notes |
|---|---|---|---|
| `tools/supabase-exec.py` | 1 | `pkgs/fleet/` or `pkgs/database/` | The fleet-wide Supabase SQL helper. **Dependency note:** the STAY quota-recorder calls it at its hatch path; the batch must repoint quota-recorder (or land the code copy first and shim) — flagged in the batch issue. |
| `tools/turso-exec.py` | 1 | same as above | Turso/libSQL helper used by Atlas + fleet tooling. |
| `tools/cloudflare/` (`wrangler-authed.py`, `wrangler-secret.py`) | 2 | `pkgs/cf/` | Cloudflare tooling. Borderline: it resolves credentials through the sandbox bws route — the *tool* is fleet tooling, the credential route stays documented in TOOLS.d. |
| `library/toolchains/` | 14 | `doc/platform/toolchains/` | go, rust, bun, uv, git, tailscale, bitwarden-bws, onepassword-op, cloudflare-wrangler, chromium-playwright, binary-backup (→ §4.1), index, nix, muse-cli. Toolchain how-tos that make sense without this sandbox. Two flagged borderline in §7 (nix.md, muse-cli.md). |
| `library/plans/` (`ai-plans.md`, `hardware.md`) | 2 | `doc/platform/` | Forward plans incl. the lab/hardware plan — fleet subject matter. |
| `library/inventory-summary.md` | 1 | `doc/platform/` | Borderline (§7). |
| `notes/research/muse/` | 6 | `doc/platform/` | Research into how Muse (the platform) works — mission, sources, report. Drew's "how Muse tasks work" class: general platform knowledge, not sandbox environment. Borderline (§7). |

### 4.4 STAY-UNTIL-REPLACED (register in §6)

| Path / cluster | Files | Exit condition |
|---|---|---|
| `tools/laptop-ssh.sh`, `tools/laptop-ssh-real.sh`, `tools/laptop-proxy-connect.py`, `tools/laptop-agent.py`, `tools/laptop-agent-daemon.py`, `tools/laptop-agent-broker.py`, `tools/board-agent.service`, `tools/install-board-agent-service.sh` | 8 | Fleet grows a built-in fallback connection mode (direct/SSH) for when the fleet proxy path is unavailable. Gap issue filed (§8). |
| `tools/oc-attach`, `tools/oc-attach.py`, `tools/oc-attach.md` | 3 | The attach-first client helper lands — **already tracked as developing-today/code#1357**; no duplicate filed. |
| `tools/proxy-supervisor/` (4) + `tools/proxy-supervisor.service` + `tools/proxy-supervisor.timer` + `tools/install-proxy-supervisor-service.sh` | 7 | Fleet provides built-in supervised local proxy endpoints, so ad-hoc tools don't need a separate supervisor. Gap issue filed (§8). Note: current config has an empty proxy list — the supervisor is dormant machinery today, which strengthens the case for folding it into fleet. |

### 4.5 STAY — the sandbox system itself (354 files; clusters)

| Path / cluster | Files | Reason |
|---|---|---|
| `workspace/avatars/` | 171 | Avatars — explicitly stay ("we have a copy of … the avatars. All of that stays"). |
| `archive/` (install-scripts 5, small-files 31) | 36 | Historical archive; historical copies stay by Drew's ruling. (Live counterparts classified on their own merits above.) |
| `docs/` (Muse platform docs: muse.md, connectors.md, feed.md, devices/, chat/, …) | 29 | The platform's own documentation set, baked into this system. See §7 for the feed.md borderline. |
| `tools/` init family: `init.sh`, `init-ensure.sh`, `install-*.sh` service installers (13), `verify-services.sh` | ~16 | Sandbox boot/init machinery — explicitly stays. |
| `tools/` quota family: `quota-recorder.py`, `quota-recorder/quota-recorder.service`, `quota-recorder/quota-recorder.timer` | 3 | The quota script — explicitly stays. (Its supabase-exec dependency is §4.3.) |
| `tools/` nix survival family: `nix-ensure.sh`, `nix-install.sh`, `nix-installer.sh`, `nix-profile-sync.sh`, `nix-repair.sh`, `nix-restore.sh`, `nix-snapshot.sh`, `install-nix-wrapper.sh`, `install-bash-wrapper.sh`, `taint-shim/` (2), `dispatcher.sh` + `dispatcher/` (2), `omnibin-refresh.sh` | ~14 | Strictly this sandbox's nix/store arrangement (TOOLS.d/nix.md is its doc twin). Update-getting scripts also stay per Drew's rule. |
| `tools/` credential plumbing: `bws-env.sh`, `bws-receive-token.sh`, `bws-store-values.py`, `cred-receiver.py`, `op-env.sh` | 5 | Sandbox-side secret access routes (values by name only; the *routes* are environment). |
| `tools/` provider proxy services: `muse-proxy/` + start/service/install (5), `codex-proxy/` (3), `commandcode-proxy/` (3), `claude-code-proxy/` (3), `agy-proxy/` (3), `provider-usage/` (1), `opencode.service`, `opencode-serve-start.sh`, `install-opencode-service.sh`, `mcpx.service`, `install-mcpx-service.sh`, `install-mcpx-servers.sh`, `claude-login-run.sh` | ~28 | Running sandbox system services (the provider doors). Their forks/canonical sources live in the opencode fork / code already; these are this machine's service wiring, installed by init. |
| `tools/` sandbox utilities: `mem` + `mem.py`, `shell-env.sh`, `restore-workspace-links.sh`, `git-sweep.sh`, `session-count.py`, `rustdesk-gl/` (3) | 8 | Sandbox-environment utilities (memory CLI, boot link repair, home-repo hygiene, GUI launcher for this box). |
| `TOOLS.md` + `TOOLS.d/` (10) | 11 | The archetype STAY: strictly sandbox-environment notes. |
| `notes/architecture/` except mcpx-opencode-plan | 15 | Sandbox architecture: boot-lifecycle, capabilities (measured Linux caps of *this* sandbox), disk/home-dir inventories, nix-persistence, nix-install-flakes, system-profile, systemd, telemetry, program-access, infra-gaps, omnibin + dispatcher-vs-omnibin (this box's program-access design). |
| `library/` system family: `AGENTS.md`, `identity.md`, `how-we-work.md`, `how-files-work.md`, `doc-practices.md`, `README.md`, `submodules.md`, `mise-dotfiles-review.md`, `mise.toml.draft` | 9 | The agent system's own working/identity docs and this machine's dotfiles — part of the system itself. |
| Root + config: `README.md`, `TODO.md`, `.gitignore`, `.gitmodules`, `scripts/link-submodules.sh`, `repos/*` (7, hatch-personal only), `.claude/hooks/` (2), `.codex/config.toml`, `.gemini/settings.json` | ~13 | Repo mechanics and per-tool config for this environment. |

## 5. darwin-nix enumeration

474 tracked files. The rule: flake mechanics stay; documentation about
anything else moves. Functional code/config that *uses* a topic stays —
only documentation *about* the topic moves.

### 5.1 MOVE (32 files) — batch: darwin-nix non-flake docs

| Path / cluster | Files | Why |
|---|---|---|
| `planning/plans/mcpx-extraction.md`, `planning/plans/mcpx-identity-transport.md`, `planning/plans/mcpx-oauth-authorization-server.md` | 3 | mcpx design documents. mcpx's canonical home is code (`pkgs/mcpx`); the extraction plan's own goal was "mcpx moves to code" — the docs follow the code. |
| `docs/mcp.md` | 1 | Documentation about mcpx and its MCP server inventory — about mcpx, not about the flake. |
| `docs/services.md` | 1 | How OpenChamber / OpenCode / mcpx run as services — fleet service documentation. |
| `docs/2026-10-19-reliability-review.md` | 1 | Dated point-in-time review of those same services; travels with services.md. |
| `instructions/work-strategies.md`, `instructions/worktrees.md`, `instructions/recordings.md`, `instructions/archive/subagents.md` | 4 | Agent-workflow documentation ("how Muse tasks/agents work" class) — general, not flake-specific. |
| `planning/README.md`, `planning/designs/README.md`, `planning/plans/README.md`, `planning/templates/design.md`, `planning/templates/plan.md`, `planning/templates/roadmap-item.md` | 6 | The planning/design methodology + templates — a fleet-wide working method that happens to live here. Borderline (§7). |
| `command/tokenscope.md`, `commands/gha-fix.md`, `commands/gha-why.md`, `commands/warp-import.md` | 4 | Agent command definitions — general agent tooling, not flake content. Borderline (§7). |
| `tool-definitions/` (README + 11 tool prompt files) | 12 | Agent tool-definition prompts — general agent-system documentation. Borderline (§7). |

Destination: `doc/platform/` (workflow/methodology) and `doc/fleet/` (mcpx +
services docs), exact placement in the batch issue.

### 5.2 Five-topic findings (Drew's named topics)

| Topic | Documentation found (moves) | Functional config/code that merely references it (stays) |
|---|---|---|
| **mcpx** | `planning/plans/mcpx-*.md` (3), `docs/mcp.md`, `docs/services.md` (mcpx is one of its three services), `docs/2026-10-19-reliability-review.md` (in part); mcpx *sections* inside staying docs: `ROADMAP.md` (its largest area), `ARCHITECTURE.md`, `README.md` | `mcpx.config.nix` (the machine's mcpx configuration — config the flake consumes, **flagged borderline in §7** given Drew's "anything about mcpx" wording), `pkgs/npm-mcp-servers/`, `mcp-clients.py` + `mcp-clients-activation.sh` + tests, `openchamber.nix` references, `.agents/skills/update-opencode/SKILL.md` (a skill for updating *this repo's* pins) |
| **ID project** | **None.** No ID documentation exists in darwin-nix. The only "identity" design is `planning/plans/mcpx-identity-transport.md` — that is mcpx's identity transport, counted under mcpx above. | `pkgs/fleet-brokers/src/**` queue-broker code references the code repo's queue (functional code; canonical fleet source is in code — same already-migrated pattern as hatch's copy). |
| **Atlas** | **None.** Every "atlas" hit is **Atlassian** (false positive): `pkgs/twg-cli/` (Atlassian Teamwork Graph CLI), the `atlassian` MCP server entries in `docs/mcp.md` / `mcpx.config.nix`, `systemPackages.nix` comments. Our Atlas (the search/graph system) is not documented in this repo at all. | n/a |
| **Cloudflare** | **None — zero hits** anywhere in the tree (verified by full-tree grep on the laptop primary). | None. Clean. |
| **Turso** | **None — zero hits** anywhere in the tree (same grep). | None. Clean. |

### 5.3 STAY (442 files; clusters)

| Path / cluster | Files | Reason |
|---|---|---|
| `archive/` (vendored microsoft-foundry + plannotator skill archives) | 207 | Third-party vendored archive, not our documentation and not fleet subject matter. Borderline (§7) — recommended STAY. |
| Root nix modules + flake: `flake.nix`, `flake.lock`, `configuration.nix`, `homeUser.nix`, `homePrograms.nix`, `homePackages.nix`, `systemPackages.nix`, `services.nix`, `nix.settings.nix`, `brews.nix`, `casks.nix`, `masApps.nix`, `gitSettings.nix`, `mcpx.config.nix`, `openchamber.nix`, `opencode-tmp-guard.nix`, `opencode-prune-report.nix`, `treefmt.nix`, `nix.conf`, `.sops.yaml` | ~20 | The flake itself and the machine configuration it builds — the repo's entire purpose. |
| Root maintenance scripts + tests (`*.sh`, `*.test.sh`, `git-maintain-repos`, `git-discover-repos`, `clean`, `adopt`, `s`, `r`, `search.sh`, …) | ~60 | Flake/machine maintenance tooling (build, switch, clean, backup of *this machine's* editor state, opencode DB pruning on this machine). Functional, not documentation. |
| `pkgs/` (adopt, brew-repair, buffer-backup, bun-bin, codedb, flake-tidy, fleet-brokers, laptop-agent, noTunes, npm-mcp-servers, opencode-share, service-events, sidepulse, sidepulse-setup, symlinker, twg-cli, + patches) | 81 | Flake packages — functional code the flake builds. Note: `pkgs/fleet-brokers/src` and `pkgs/laptop-agent/src` duplicate sources whose canonical homes are now code / the laptop tooling; recorded as a finding, not re-migrated here. `pkgs/buffer-backup` is a VS Code extension package, not a backup script. |
| `patches/` (sidepulse stack) | 12 | This repo's patch stack against upstream sidepulse — flake mechanics. |
| `docs/sidepulse-customisations.md`, `docs/sidepulse-leds.md`, `docs/sidepulse-modes.md` | 3 | Documentation of *this repo's* sidepulse patch stack / this machine's hardware. Borderline (§7). |
| `README.md`, `ARCHITECTURE.md`, `ROADMAP.md`, `TODO.md`, `NOTICE`, `LICENSE.*`, `DCO-1.1` | 7 | The repo's own front-door docs. ARCHITECTURE.md is the flake's architecture doc (stay); ROADMAP.md is this repo's roadmap (stay) — both carry mcpx *sections* that should be trimmed to pointers when the mcpx batch lands (§7). |
| `.github/` (incl. `ci-policy.md`, workflows) | 12 | This repo's CI — flake-repo mechanics. |
| `.agents/skills/` (merge-first, nix-config, update-opencode) | 3 | Skills for working on *this flake*. |
| Misc config/dotfiles: `sounds/` (17), `themes/`, `plugins/` (6), `lib/` (3), `.zed/`, `.jules/`, `justfile`, `renovate.json`, `config.toml`, `tui.jsonc`, `opencode-*.json/jsonc`, `vibrant-ink.json`, `vsc-extensions.txt`, `.aerospace.toml`, `.envrc`, `.gitignore` | ~40 | Machine configuration and assets the flake deploys. |

## 6. STAY-UNTIL-REPLACED register

| # | Item(s) | Why it can't move yet | Exit condition (Fleet feature) | Tracking |
|---|---|---|---|---|
| R1 | laptop-ssh family (8 files, §4.4) | It *is* the fallback path to the laptop when the fleet proxy route is down; TOOLS.d/laptop.md documents why stock SSH drops bursts. Removing/relocating it before fleet has an equivalent breaks recovery. | `fleet` gains a built-in fallback connection mode (direct SSH with connection multiplexing) used automatically when the proxy path fails. | New gap issue (§8) |
| R2 | oc-attach family (3 files, §4.4) | The documented attach-first OpenCode client until the fleet-side helper exists. | The attach client helper ships. | **Existing: developing-today/code#1357** (no duplicate filed) |
| R3 | proxy-supervisor family (7 files, §4.4) | Supervises local proxy endpoints that non-fleet tools still dial; currently dormant (empty proxy list) but wired into service checks. | `fleet serve` provides built-in supervised local proxy endpoints with health/drain semantics matching the supervisor's config. | New gap issue (§8) |

## 7. Borderline — needs Drew's eye (classified, not stalled)

1. **`docs/feed.md` (hatch/hatch-personal)** — classified STAY as part of the
   platform's baked-in doc set. But Drew's first example of a MOVE was "how
   the news agent works." No document about *our* news/feed agent exists in
   any of the three repos' tracked files — the feed pipeline's design lives in
   untracked goal workspaces. If Drew means the product Feed doc, it moves;
   if he means our agent's design, there is nothing in these repos to move
   (see §9, Atlas note — same situation).
2. **`mcpx.config.nix` (darwin-nix)** — classified STAY (functional config the
   flake consumes), per the doc-vs-config distinction. Drew's wording was
   "anything about mcpx … should NOT be in darwin-nix at all." If he means
   the config too, the exit is mcpx configuration being generated from code —
   a bigger change than a docs migration; flagged rather than assumed.
3. **`ROADMAP.md` / `ARCHITECTURE.md` mcpx sections (darwin-nix)** — docs
   stay (they are the repo's own), but their mcpx content is the largest
   single block of mcpx prose in the repo. Recommendation: when the mcpx
   batch lands, trim those sections to pointers at the code copies.
4. **darwin-nix `planning/` methodology + `commands/` + `tool-definitions/`**
   (22 files) — classified MOVE (general agent-workflow material). A
   reasonable alternative read is "this repo's own working method" → STAY.
   Drew's "how Muse tasks work" example tips it to MOVE.
5. **darwin-nix `archive/` (207 files)** — classified STAY. It is vendored
   third-party skill documentation (Microsoft Foundry, Plannotator), not our
   docs and not about the flake either. Strictly by the letter of the rule it
   is "documentation not about the flake," but moving vendored archives into
   code would pollute code; recommended STAY as an archive.
6. **`docs/sidepulse-*.md` (darwin-nix, 3)** — classified STAY (docs of this
   repo's own patch stack). If sidepulse is treated as a fleet service like
   mcpx, they move with the services batch instead.
7. **hatch `library/toolchains/nix.md` + `muse-cli.md`** — classified MOVE
   with the toolchain cluster, but both overlap STAY territory (nix.md
   overlaps TOOLS.d/nix.md's subject; muse-cli.md documents the platform
   CLI). If Drew prefers, these two stay and the other 12 move.
8. **hatch queue artifacts** (`lane-claim.sh`, `lane-handoff.sh`,
   `repo-task-v1.schema.json`, `enqueue-when-green.sh`) — classified MOVE as
   fleet-protocol artifacts; they are also still executable plumbing some
   sandbox jobs reference. The batch must grep for live references before
   treating the code copy as canonical.
9. **`library/skills/` (hatch, 4)** — classified MOVE; they document fleet
   tooling (backup, repo census, setup-cycle). They are also agent-system
   documentation, which argues STAY with the prompts. Drew's eye.
10. **`notes/research/muse/` (hatch, 6)** — classified MOVE per the
    "how Muse tasks work" example; it is research *about the platform we run
    on*, which argues STAY. Drew's eye.

## 8. Filed issues (developing-today/code)

- Tracking: [#5983](https://github.com/developing-today/code/issues/5983)
- Batch — backup scripts & backup docs: [#5976](https://github.com/developing-today/code/issues/5976)
- Batch — fleet designs & machinery: [#5978](https://github.com/developing-today/code/issues/5978)
- Batch — architecture & how-it-works docs: [#5980](https://github.com/developing-today/code/issues/5980)
- Batch — Atlas docs sweep: [#5981](https://github.com/developing-today/code/issues/5981)
- Batch — darwin-nix non-flake docs: [#5982](https://github.com/developing-today/code/issues/5982)
- Gap — fleet fallback connection mode (R1): [#5972](https://github.com/developing-today/code/issues/5972)
- Gap — fleet built-in local proxy endpoints (R3): [#5974](https://github.com/developing-today/code/issues/5974)
- Cross-linked from epic #2056 with a brief comment.
- R2's gap already exists: [developing-today/code#1357](https://github.com/developing-today/code/issues/1357)

## 9. Migration mechanics (for the execution issues)

- **Copy-not-delete, always.** Execution = copy into code, commit there via
  the normal PR flow (worktree from origin/main, merge commit, CI on main),
  and leave the source file untouched. The source repos keep historical
  copies permanently; nothing in this plan authorises a deletion anywhere.
- **Pointers:** because sources keep their copies, silent staleness is the
  real risk. Proposal: when a batch lands, add a one-line header to the
  *hatch-personal* copy (`Canonical: developing-today/code/<path> — this copy
  is historical`) for living docs only (not for archive/, not for binaries).
  Scripts that move (runtime-db-export, bws-backup-secrets) get a pointer
  comment at the top of the hatch copy plus a wrapper note in the batch PR.
  darwin-nix docs that move get the same one-line header; staying docs with
  moved sections (ROADMAP/ARCHITECTURE) get section-level pointers.
- **Reference updates:** before each batch declares the code copy canonical,
  grep the sandbox tree, TOOLS.d, goal workspaces, and cron bodies for the
  old path and repoint (cron/job changes only via the cron tools, per
  standing rule). Known load-bearing reference on day one:
  quota-recorder → `tools/supabase-exec.py` (§4.3).
- **Autocommit interaction:** hatch-personal autocommits every minute, so
  pointer-header edits land automatically; expect a small commit burst per
  batch, which is fine. Repo size: code gains ~140 mostly-text files
  (runtime-db-export specs are the largest); negligible. hatch repos do not
  shrink (copies stay) — this plan does not reduce hatch size and doesn't
  claim to.
- **Order:** (1) backups batch (self-contained, highest organisational
  value, includes the DB backup machinery Drew named first); (2) fleet batch
  (migrations + fleet-fetch + designs; fleet-brokers already done);
  (3) architecture batch (toolchains, platform docs, DB helpers — carries
  the supabase-exec dependency work); (4) darwin-nix batch (docs only,
  leaves the flake untouched functionally); (5) Atlas batch (a sweep, see
  below — it is mostly *finding* the docs, not moving repo files).
- **Atlas batch reality check:** there are **no Atlas documents in any of
  the three repos** (hatch: zero filename/content hits in the tracked set;
  darwin-nix: only Atlassian false positives). Atlas design/docs live in
  *untracked* places: `~/workspace/research_notes/`, goal workspaces under
  `~/workspace/goals/`, memory topic files, and code's own `doc/` already
  (code has `pkgs/atlas-search` and dated design dirs). The Atlas batch
  issue therefore enumerates a sweep of those untracked locations into
  code's `doc/`, rather than a repo-to-repo move. The same is true of the
  "news agent" design (goal workspaces). Drew's premise ("it landed there,
  that's fine") holds for the *workspace*; it just isn't in the *git* repos.

## 10. Longer-term direction (recorded, NOT part of this migration)

Per Drew: eventually all of these docs, the Zettelkasten notes, conversation
logs, and notes from various places integrate into **Atlas + the
Zettelkasten graph / ID system**, with classifications and other metadata.
Markdown in code is not going away wholesale, but designs and general docs
may centralise there later. A technical reference / beginner's-guide website
may eventually be backed by objects tagged for that site — the backing could
be a repo, a database, or something undecided. The code repo is the best
home **now**, and consolidating there first makes that later move easier
(one source tree to ingest instead of three-plus-workspaces).

## 11. Surprises found during inventory

1. **dezren39/hatch is a file-for-file public mirror of hatch-personal**
   (minus submodules) — including `tools/` scripts and the Supabase
   migrations. Anything classified MOVE exists in *both*; the public repo
   should get its pointer headers too, or be noted as a stale public copy.
2. **The fallback estate is bigger than one script:** the laptop connection
   has a three-layer fallback family (laptop-ssh.sh multiplexing wrapper,
   laptop-ssh-real.sh, laptop-proxy-connect.py, plus a laptop-side agent
   trio) — all of it pre-fleet machinery still in daily use (register R1).
3. **proxy-supervisor is dormant**: its config's proxy list is empty. It is
   still installed and service-checked every boot.
4. **fleet-brokers was already migrated today** (2026-11-04, code#5430) —
   the pattern this plan generalises (canonical in code, hatch copy kept as
   rollback evidence) is already Drew-approved in practice.
5. **darwin-nix is the Mac's flake** (nix-darwin), and it is *clean* on
   Cloudflare and Turso (zero hits) and carries no ID or Atlas docs; its
   contamination is concentrated in mcpx (3 design plans + mcp.md +
   services.md) and in general agent-workflow docs that drifted in
   (instructions/, tool-definitions/, commands/).
6. **The prompt/identity files at the hatch-personal root (AGENTS.md,
   SOUL.md, MEMORY.md, memory/) are not in the tracked file set** — the
   tracked system docs are `docs/` and `library/`. They stay regardless
   (Drew's ruling), but anyone auditing "what's in the repo" should know
   the tracked set ≠ the on-disk system.
