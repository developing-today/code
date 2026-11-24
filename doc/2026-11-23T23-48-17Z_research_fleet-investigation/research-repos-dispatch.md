# Fleet — Repos, Fork Sync, Remote Dispatch, Nix Packaging & Deployment Profiles

Research file for the fleet program investigation. Read-only research,
2026-11-23 (America/Chicago). All GitHub work via the `gh` CLI / REST API
only (GitHub connector banned per standing rule). Secrets are named only,
never valued. Anything not directly verified from a named source is
labelled **unverified**.

Method note: every `gh api` call in this environment prints a fleet
gh-cache warning (`no cache daemon answered … resolving in-process`) to
stderr; the JSON payloads quoted below are the in-process REST results.

---

## 1. The repo constellation

### 1.1 `dezren39/fleet` — upstream / umbrella

- Source: `gh api repos/dezren39/fleet`
  → `default_branch: main`, `fork: false`, `parent: null`,
  `pushed_at: 2026-11-23T08:42:18Z`.
- Latest commit: `gh api repos/dezren39/fleet/commits?per_page=1`
  → SHA `c8f4349d476c44c922c3064a363e760a77842942`,
  author date `2026-11-23T08:42:18Z`,
  message “Merge pull request #103 from
  dezren39/docs/placement-dispatch-surfaces — docs: document placement
  dispatch and dispatch-test execution surfaces”.
- Role (from its own workflows and flake, below): the upstream home.
  Fixes land here first (see sync-forks header, §1.3); it hosts the
  canonical copies of the dispatch workflows (`fleet-command.yml`,
  `fleet-run.yml`, `fleet-dispatch-test.yml`, `fleet-build-container.yml`,
  `fleet-build-variant.yml`, `fleet-umbrella-verify.yml`,
  `sync-forks.yml`, `sync-upstream.yml`, `auto-format.yml` — directory
  listing via `gh api repos/dezren39/fleet/contents/.github/workflows`),
  the `fleet-build` container (`containers/fleet-build/`), and the
  fleet-wide nixpkgs pin flake (§4.1). It is also one of the three
  active placement surfaces (`fleet-personal`, §2.2).
- Submodules — **the “12 submodules” figure is stale.**
  Source: `gh api repos/dezren39/fleet/contents/.gitmodules` (raw),
  file size 2,224 bytes. It declares **22 submodules**:

  | path | url repo |
  |---|---|
  | code | developing-today/code |
  | opencode | developing-today-forks/opencode |
  | openchamber | developing-today-forks/openchamber |
  | nix | dezren39/darwin-nix |
  | zettel | developing-today/zettel |
  | hatch | dezren39/hatch |
  | hatch-personal | dezren39/hatch-personal |
  | cairn | dezren39/cairn |
  | dezren39-zettel | dezren39/zettel |
  | org-readme | developing-today/.github |
  | zulip-archive | developing-today/zulip-archive |
  | hardware-doc | developing-today/hardware-doc |
  | openclaw-personal | dezren39/openclaw-personal |
  | cairn-private | dezren39/cairn-private |
  | kaneo | developing-today/kaneo |
  | nix-cache | developing-today/nix-cache |
  | hatch-drawer | dezren39/hatch-drawer |
  | hatch-personal-drawer | dezren39/hatch-personal-drawer |
  | opencode-muse | dezren39/opencode-muse |
  | opencode-auto-continue | developing-today/opencode-auto-continue |
  | dezren39 | dezren39/dezren39 |
  | muse-esp32-screen | dezren39/muse-esp32-screen |

  The standing memory figure “fleet umbrella = 12 submodules
  (2026-11-03)” therefore no longer matches the live `.gitmodules`;
  the constellation has grown to 22 entries. (Whether all 22 are
  initialised in any given checkout was not checked — contents API
  only.)

### 1.2 `developing-today/fleet` — the org fork

- Source: `gh api repos/developing-today/fleet`
  → `default_branch: main`, `fork: true`, `parent: dezren39/fleet`,
  `pushed_at: 2026-11-23T08:43:41Z`.
- Fork state vs upstream:
  `gh api repos/developing-today/fleet/compare/dezren39:main...main`
  → `status: ahead`, `ahead_by: 22`, `behind_by: 0`,
  `total_commits: 22`. The reverse comparison
  (`compare/main...dezren39:main`) returns `status: behind`,
  `ahead_by: 0`, `behind_by: 22` — consistent.
  So at read time the org fork is **22 commits ahead of upstream and
  0 behind**: it carries fork-local deltas (the sync-forks header
  names “developing-today/fleet's fleet-run fork deltas” as the
  standing example) and is missing nothing from upstream.
- It hosts an identical workflow directory listing to upstream
  (same 10 entries, verified by the same contents call), and it is
  the **canonical org runner surface** (`fleet-org`, §2.2) and the
  only surface where Blacksmith is configured (§2.1).
- Its `main` is locked by ruleset 26550095 (§1.3).

### 1.3 The 102-fork sync

- Fork count: `gh api repos/dezren39/fleet` → `forks_count: 102`.
- Workflow: `.github/workflows/sync-forks.yml` in dezren39/fleet
  (raw contents read in full). Header and body establish:
  - Name: “sync-forks (push main out to every fork)”. Trigger:
    `push` to `main` plus `workflow_dispatch`. Job-level guard
    `if: github.repository == 'dezren39/fleet'` — the file is inert
    in every fork that receives it. Runner `ubuntu-24.04`,
    timeout 120 min, `permissions: contents: read`.
  - Doctrine (header, Drew 2026-11-22): fixes land on
    dezren39/fleet main; forks follow. This push-driven workflow is
    the complement to the pre-existing daily pull
    (`sync-upstream.yml`, App-driven, defined in the same tree),
    which stays as backstop.
  - Mechanism: GitHub's merge-upstream API
    (`POST /repos/<fork>/merge-upstream`, branch `main`) for every
    non-archived fork, enumerated with
    `gh api --paginate repos/dezren39/fleet/forks?per_page=100`.
    Archived forks are recorded as skipped, not failed. The API
    fast-forwards a strict-prefix fork and clean-merges otherwise;
    it never force-pushes and never auto-resolves a conflict, so
    fork-local deltas survive a clean merge and true divergences
    land on the failure list.
  - Auth: the classic PAT held in the repo secret named
    `GHCR_ORG_PAT` (value never read or printed). The workflow's own
    `GITHUB_TOKEN` cannot write other repositories. The PAT must
    carry the `workflow` scope in addition to `repo`, because any
    sync whose delta touches `.github/workflows/` is refused
    without it — “proven by the first run, 2026-11-22 — all 102
    forks refused on exactly that gate when this workflow file
    itself landed” (workflow header). Standing context (MEMORY.md,
    not re-verified against the token itself): `GHCR_ORG_PAT` now
    holds the laptop `gh` OAuth token (repo + workflow scopes), set
    from the laptop; the BWS key `github.ghcr-classic-pat` was
    updated to the same token.
  - Failure contract (header, Drew 2026-11-22): attempt **every**
    fork no matter how many fail; the job fails only at the **end**,
    with the failed forks listed in the log and the job summary,
    and **one todo issue filed in developing-today/code** carrying
    the same list. The issue is filed through the REST API
    (`POST repos/developing-today/code/issues`), not `gh issue
    create`, because GraphQL issue creation is a separate
    rate-limit bucket that walled the first run (2026-11-22; the
    list survived only in the job summary that time). A missing PAT
    is a configuration error and fails the run immediately, before
    any fork is touched. If filing the issue itself fails, the job
    still fails with the list in the summary (a `::warning::` is
    emitted).
  - Known caveat recorded in the header: fork mains locked by a
    ruleset whose only bypass is the developing.today App
    (developing-today/fleet, developing-today-forks/fleet) “may
    refuse a PAT sync” — those forks keep converging through the
    App-driven daily pull, and their refusal on the failure list is
    the signal to extend the bypass. See the ruleset state below:
    both rulesets now also carry a RepositoryRole admin bypass.
- Rulesets (REST, read 2026-11-23):
  - `gh api repos/developing-today/fleet/rulesets` → one ruleset:
    id **26550095**, name “main: actions-only (commits happen
    upstream)”, target branch, enforcement active.
    Detail (`…/rulesets/26550095`): conditions
    `refs/heads/main`; rules `creation, update, deletion,
    non_fast_forward`; bypass actors: RepositoryRole id 5
    (`always`) and Integration id **5454422** (`always`) — 5454422
    is the developing.today GitHub App (per MEMORY.md org facts).
  - `gh api repos/developing-today-forks/fleet/rulesets` → one
    ruleset: id **26550098**, same name/target/enforcement.
    Detail (`…/rulesets/26550098`): identical shape — same four
    rules on `refs/heads/main`, bypass actors RepositoryRole 5
    (`always`) and Integration 5454422 (`always`).
  - Interpretation grounded in the two reads: the RepositoryRole-5
    bypass is the admin bypass that lets the PAT route (acting with
    an admin role) sync like everywhere else; MEMORY.md records
    Drew's 2026-11-22 order that developing-today-forks/fleet be
    “no different from the other forks” and that ruleset 26550098
    gained the RepositoryRole admin bypass for exactly that reason,
    with the App pull kept as backstop. **Unverified:** the numeric
    role id 5 = “admin” mapping itself is not spelled out in the
    REST payload; it is inferred from the role name in standing
    context.
- First fully clean run — **verified at run level**:
  `gh api repos/dezren39/fleet/actions/runs/45022131766`
  → name “sync-forks (push main out to every fork)”,
  `status: completed`, `conclusion: success`,
  `event: workflow_dispatch`, `created_at: 2026-11-22T12:24:43Z`,
  `head_sha: 214e5cc291a0d0596a65cf39c217ff1fbbbf7690`.
  Its single job (`…/runs/45022131766/jobs`) “push main to all
  forks” also completed/success. Under the workflow's own failure
  contract, a success conclusion means zero failed forks in that
  run (any failure exits 1 at the end). The literal “102/102”
  tally lives in the run's job summary/log, which was **not**
  downloaded in this pass — labelled **partially unverified** at
  the tally level, verified at the success-conclusion level.
  (The same run id on developing-today/fleet returns 404, as
  expected: the run belongs to dezren39/fleet.)

### 1.4 `developing-today/code` — `pkgs/fleet`, the fleet source home

- The fleet implementation does **not** live in the fleet umbrella
  repos; it lives in the monorepo at `developing-today/code`,
  path `pkgs/fleet` (laptop checkout `~/git/code/pkgs/fleet`, and
  contents API agree). Top level there includes: `crates/`,
  `book/`, `doc/`, `docs/`, `placement/` (a Go module:
  `cmd/ evaluate.go policy.go go.mod …`), `migrations/`, `tests/`,
  `hosted-worker/`, `fleet-surfaces.json` (the placement registry,
  §2.2), `fleet-fetch.py`, `lane-claim.sh`, `lane-handoff.sh`,
  `laptop-agent.py`, `laptop-agent-broker.py`, `turso-exec.py`,
  `smoke.sh`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`.
- Crates — `gh api repos/developing-today/code/contents/pkgs/fleet/crates`
  (13 entries, identical in the laptop checkout):
  `fleet`, `fleet-agent`, `fleet-bridge`, `fleet-config`,
  `fleet-dispatch`, `fleet-dispatch-test`, `fleet-laptop-agent`,
  `fleet-proxy`, `fleet-proxy-proto`, `fleet-queue`,
  `fleet-registry`, `fleet-search`, `fleet-secrets`.
  The placement/dispatch logic cited in §2 lives principally in
  `crates/fleet-queue/src/placement.rs` (+ `placement/adapters.rs`).

---

## 2. Remote build/test dispatch

### 2.1 The `fleet-command.yml` workflow — which repo hosts it

- The workflow file exists, byte-identical in name and role, in
  **all three** fleet copies (dezren39/fleet,
  developing-today/fleet, developing-today-forks/fleet) — it
  propagates through the fork sync. Raw read was from
  dezren39/fleet; the directory listings of the other two match.
- It is **dispatch-only**: `on: workflow_dispatch` +
  `workflow_call`, header comment “Dispatch-only runner used by
  repo-queue dispatch. No PR or main-branch checks gate execution;
  placement selects a surface before this starts.”
- Inputs (dispatch form): `repo` (target owner/repo), `command`,
  `ref` (blank = target default branch), `runner`
  (`github|blacksmith`, default github), `platform`
  (`linux|macos`, default linux), `container_image` (blank =
  standard Fleet build image), `actions_cache_key` (default
  `fleet-deps-v1`), `actions_cache_path`,
  `actions_cache_restore_keys`, `previous_run_id`,
  `previous_run_repo`, `push_nix_cache` (bool), `timeout_minutes`
  (5–120, default 60), `dispatch_id`. Secrets (call form):
  `GHCR_ORG_PAT`, `NIX_CACHE_SIGNING_KEY`, `NIX_CACHE_UPLOAD_URL`
  (names only).
- Jobs: `normalize` (validates platform/runner/timeout) →
  `run_linux` / `run_macos`.
  - Linux runs in a container: default image
    `ghcr.io/dezren39/fleet-build:full`, `options: --user root`,
    on `ubuntu-24.04` — or on `blacksmith-4vcpu-ubuntu-2404` when
    the Blacksmith runner is selected. Steps delegate to the local
    composite action `./.github/actions/fleet-command`.
  - macOS runs on `macos-15`, no container.
  - Guard rails in `normalize`: Blacksmith is refused unless the
    hosting repo is exactly `developing-today/fleet`
    (“Blacksmith is configured only for developing-today/fleet”),
    and Blacksmith + macOS is refused (“Blacksmith is Linux-only;
    select runner=github for macOS”).
- The composite action (`.github/actions/fleet-command/action.yml`,
  dezren39/fleet): checks out the target, prepares the shared
  Rust/Go cache under `$GITHUB_WORKSPACE/.fleet/cache`
  (`CARGO_HOME`, `CARGO_TARGET_DIR`, `RUSTUP_HOME`, `GOPATH`,
  `GOMODCACHE`, `GOCACHE` all pointed there), restores it via
  `actions/cache@v4` with a key of
  `<prefix>-<OS>-<dependency-hash>-<target-ref>`, runs the command,
  publishes reusable output, and — only when `push_nix_cache` is
  true and both `NIX_CACHE_*` secrets are set — signs `./result`
  (`nix store sign`) and `nix copy --to $NIX_CACHE_UPLOAD_URL`,
  logging “pushed $out to the shared Fleet R2 cache”.

### 2.2 How `fleet queue dispatch` targets it — placement surfaces

- Source: laptop checkout `~/git/code/pkgs/fleet/fleet-surfaces.json`
  (registry version 2; the same file ships embedded in the binary —
  see `book/reference/queue-dispatch.md`: resolution order is
  `--registry <path>` / `FLEET_PLACE_REGISTRY` → a home copy → the
  source-tree relative path → the registry embedded in the binary).
- The registry's own comment: “The fleet-command.yml workflow is
  dispatch-only; the fleet-queue placement adapter selects a host
  and provider from live capacity and local session history.”
- Active GitHub-Actions surfaces (all workflow `fleet-command.yml`,
  `selection_priority: 10`, `sticky_queue_threshold: 2`):

  | id | host repo | dispatch as | notes from registry |
  |---|---|---|---|
  | fleet-personal | dezren39/fleet | developing-today App installation 175747567 | Repo secrets include `GHCR_ORG_PAT` and `NIX_CACHE_*` for private checkouts, cross-run artifacts, Nix cache pushes |
  | fleet-org | developing-today/fleet | user token (App installation 174792676 recorded) | “Canonical org Fleet command runner… GitHub-hosted Linux is preferred; this host also supports the Blacksmith fallback” |
  | fleet-forks | developing-today-forks/fleet | user | Public targets only — “Private targets are rejected because this host has no GHCR_ORG_PAT secret” |
  | fleet-org-blacksmith | developing-today/fleet (host_surface: fleet-org) | user | Blacksmith fallback, kind `blacksmith` (§2.3) |

  Plus: `org-run` (developing-today/code, workflow fleet-command.yml,
  status **standby**, priority 20 — retained for manual use; placement
  prefers developing-today/fleet so follow-ups share its cache),
  `codespace-overflow` (kind codespace, status **pending-scope**,
  max_live 3, 180 core-hours/mo budget), and `laptop` (kind laptop,
  status **handoff-only** — “this adapter does not run shell commands
  on the laptop”).
  All Actions surfaces target `dezren39/*`, `developing-today/*`,
  `developing-today-forks/*`.
- Dispatch flow (runbook, §2.4): dry-run unless `--dispatch` is
  present; returns a local `fp_…` handle, `run_id`, `runner_repo`,
  `run_url`, and once correlated a portable id `OWNER/REPO#RUN_ID`.
  Sessions are tracked with `fleet queue session status|result`,
  recorded in `~/.local/state/fleet-place/sessions.json` and
  mirrored best-effort to the queue backend's `placement_sessions`
  table. `--followup <handle|portable-id>` prefers the previous
  run's host while it remains eligible, has a free slot, and its
  queue is below the sticky threshold, and lets the workflow
  download the previous run's `fleet-output` artifact (files under
  `target/fleet-output/`).
- A second, older route exists alongside: `fleet dispatch-test
  scan|submit|status` dispatches `fleet-dispatch-test.yml` for work
  inside a fleet-umbrella submodule, storing jobs in the
  fleet-dispatch Turso store (book `concepts/dispatch.md`). And the
  raw fallback: `gh workflow run fleet-command.yml --repo
  developing-today/fleet -f repo=… -f ref=… -f command=…`, or
  `fleet-run.yml` on dezren39/fleet for the matrix-capable generic
  runner (runbook “Other dispatch routes”).

### 2.3 Org concurrency caps, Blacksmith, macOS

Verified from the registry file itself
(`~/git/code/pkgs/fleet/fleet-surfaces.json`, `owners:` block) and
corroborated in `crates/fleet-queue/src/placement.rs` (owner_cap
reads `concurrent_jobs_cap`; the macOS adapter reads
`concurrent_macos_cap` and the effective macOS cap is
`min(concurrent_jobs_cap, concurrent_macos_cap)` — book
`library/placement.md` / `concepts/placement.md`):

| owner | concurrent_jobs_cap | concurrent_macos_cap | plan (as recorded) | private_minutes_monthly |
|---|---|---|---|---|
| developing-today | **20** | **5** | free-org | 2000 |
| developing-today-forks | **20** | **5** | free-org (assumed — “confirm”, per registry) | 2000 |
| dezren39 | **40** | **5** | pro | 3000 |

So the brief's figures (developing-today 20 / macOS 5; dezren39
40 / macOS 5) are **exact** against the current registry.

Blacksmith, from the same registry + runbook + workflow:
- Surface `fleet-org-blacksmith`: budget `free_minutes_monthly:
  3000`, `paid_cap_minutes: 0`, with the registry's rate note:
  “Blacksmith is orgs-only: 3,000 free min/mo, then PAID per-minute
  (it charges for public-repo minutes past the free tier).
  paid_cap_minutes is the registry knob Drew can raise; 0 = free
  tier only.”
- Runbook rule: “GitHub-hosted runners are the default; Blacksmith
  is a budgeted fallback, never the default, and paid Blacksmith
  minutes require the explicit `--allow-paid-blacksmith` override”
  (dispatch form: `--allow-paid-blacksmith <minutes>`).
- Workflow enforcement (§2.1): Blacksmith only from
  developing-today/fleet, Linux only, runner label
  `blacksmith-4vcpu-ubuntu-2404`.

### 2.4 The R2 Nix cache — the only Worker

- Read path (substituter): the fleet-build Containerfile
  (`containers/fleet-build/Containerfile`, dezren39/fleet) bakes
  into the vscode user's `nix.conf`:
  `extra-substituters = https://nix-cache.developing-today.workers.dev
  https://developing-today.cachix.org` (plus the corresponding
  trusted public keys, names/values of keys not reproduced here
  beyond the substituter URLs). The dispatch-test book page gives
  the same pair as the workflow's baked-in substituters.
  `nix-cache.developing-today.workers.dev` is a Cloudflare Workers
  endpoint fronting the cache.
- Write path: §2.1 — `nix copy --to $NIX_CACHE_UPLOAD_URL` (secret
  named `NIX_CACHE_UPLOAD_URL`; signing with secret named
  `NIX_CACHE_SIGNING_KEY`), described in the action's own log line
  and the placement book as “the shared Fleet R2 cache”
  (`book/tutorials/dispatch-a-build.md`: “resolving ./result,
  signing with the key from the host repo's secret named
  NIX_CACHE_SIGNING_KEY, and uploading to the URL in the secret
  named NIX_CACHE_UPLOAD_URL”).
- “The only Worker”: within everything read for this file, this
  nix-cache Worker is the **only** Cloudflare Worker surface the
  fleet dispatch path references — the other cache leg is Cachix
  (a hosted service, not a fleet Worker), and the other caches are
  GitHub Actions cache. The stronger claim “the only Worker in the
  whole fleet system” is **unverified** from the sources read here
  (no fleet-wide Worker inventory was enumerated).

### 2.5 The runbook — `pkgs/fleet/doc/agents/remote-build-and-test.md` (PR #13128)

- Landing: `gh api repos/developing-today/code/pulls/13128` →
  PR #13128 “docs(fleet): dispatch-first guidance for remote builds
  and tests”, state closed, `merged_at: 2026-11-23T08:41:15Z`,
  merge commit `8f2947dd10af1fe576c8b0b2c73c4f9ae009f5fd`.
- Full text read via contents API. Summary:
  - **The rule: dispatch first — do not compile on the sandbox or
    the laptop.** Builds/tests/CPU-heavy work run on dispatched
    GitHub Actions runners; the sandbox is small and shared, the
    laptop is the fleet's machine of record and already loaded.
    Flow: push the code → `fleet queue dispatch --repo <owner/name>
    --ref <ref> --command '<cmd>' --dispatch --json` → track with
    `fleet queue session status/result <HANDLE>`; full log via
    `gh run view <RUN_ID> --repo <RUNNER_REPO> --log`.
  - Placement chooses the pool by live load (developing-today 20,
    dezren39 40 — §2.3); GitHub-hosted default; Blacksmith budgeted
    fallback, paid minutes only via `--allow-paid-blacksmith`;
    macOS via `--platform macos` only when genuinely needed;
    `--followup` pins a repeat job to the previous host/outputs.
  - Run locally only when dispatch cannot: a NixOS build or
    activation, work needing laptop state or secrets, or a quick
    single-binary check while iterating. “‘The sandbox was closer’
    is not a reason.”
  - `--repo` is the repo to build/test; Fleet chooses the runner
    repo independently (`fleet queue surfaces` prints the registry).
    “The public fork pool cannot build private targets.”
  - Runner choice: omit `--runner` for automatic placement;
    `--runner blacksmith` only intentionally; `--container
    <image>`; `--actions-cache <key>:<path>` /
    `--actions-cache-restore-key`; `--push-nix-cache` to publish.
  - `fleet shell` targets (laptop / Codespace / local / actions):
    `auto` never picks `actions`; an explicit target does not fall
    back; `--build` takes a laptop `fleet-build-lock` slot.
    `fleet queue dispatch --surface laptop` currently creates a
    handoff record only; `fleet dispatch launch` is the separate
    pm-dispatch agent-session route.
  - Portable run ids: `fp_…` handles live in
    `~/.local/state/fleet-place/sessions.json`, mirrored best-effort
    to the broker's `placement_sessions` table; `OWNER/REPO#RUN_ID`
    resolves from any workspace with GitHub access.
  - Sandbox seats: dispatch calls the GitHub API directly from the
    seat and the (current) binary validates TLS against the platform
    root store, so it works under the sandbox's TLS-intercepting
    egress proxy. Binaries predating the code#13122 fix fail with
    `invalid peer certificate: UnknownIssuer` — the remedy is to
    update the binary via `scripts/muse/bootstrap.sh`, or route
    proxy variables through the seat's plain-tunnel relay at
    `127.0.0.1:3130` (address only; no credential reproduced).
  - Other routes + stale-binary recovery: `fleet dispatch-test`,
    raw `gh workflow run` forms (§2.2), and how to obtain a current
    binary from the pinned static release (§4.2).

---

## 3. The fleet-build container gap family — RESEARCH ONLY

Flag-held items: the records below are Drew's decisions to make;
this section reports evidence and status only and proposes no
implementation. All six issues were read in full (body + the
comments cited) via `gh api repos/developing-today/code/issues/<n>`
on 2026-11-23. **All six are OPEN** at read time. They are duplicate /
sibling filings of one underlying family — the `fleet-build:full`
dispatch container (image `ghcr.io/dezren39/fleet-build:full`,
built by `fleet-build-container.yml` in dezren39/fleet from
`containers/fleet-build/`) cannot run the runbook's own example
commands out of the box — filed independently by different lanes on
2026-11-23 as each hit the gaps during verification work.

The container as designed (Containerfile, dezren39/fleet, read in
full): two targets — `base` (devcontainers ubuntu-24.04 + pinned
Nix 2.35.4 single-user for user `vscode`, pinned Go 1.25.14) and
`full` (base + the fleet repos' Nix dev shells realised into /nix
by the publish workflow's per-repo warm-export jobs; required repos
code/opencode/openchamber, optional nix/zettel). Crucially, the
base image **does** write a correct nix config — but only into the
`vscode` user's home:
`experimental-features = nix-command flakes ca-derivations`,
`build-users-group =` (empty), `sandbox = false`, the §2.4
substituters. The fleet-command Linux job, however, runs the
container with `--user root` (§2.1), so dispatched commands do not
run under that user's config/HOME — the shape the evidence below
keeps hitting. (That root-vs-vscode mechanism is the researcher's
reading of the two files together; the issues themselves describe
it as “nix config shaped for a daemon that is not there” /
“shaped for a multi-user daemon shape the container does not
provide” — #13198.)

### 3.1 The stacked `nix develop` defects — #13159, #13168, #13193, #13198

Four filings of the same three stacked defects, each verified by
real dispatches against developing-today/code on 2026-11-23, each
failing **before any test code ran**, while the identical suites
passed in the same dev shells on the laptop's NixOS:

1. **Experimental features off** — `error: experimental Nix feature
   'nix-command' is disabled`. Plain `nix develop` / `nix build`
   fails immediately; no `NIX_CONFIG` in the command context
   enables `nix-command flakes`.
2. **Missing `nixbld` group** — with the flags passed explicitly:
   `error: the group 'nixbld' specified in 'build-users-group'
   does not exist`. The effective nix config names a group the
   image does not create. (Precursor on the org-run venue:
   code#12894, now **closed** — “nix unusable inside
   fleet-build:full legs — missing nixbld group”; #13159 notes the
   org-run venue got its equivalent fix in PR #12908.)
3. **`/homeless-shelter` purity check** — with `build-users-group`
   overridden empty, sandboxing auto-disables and the build dies
   with `error: home directory "/homeless-shelter" exists; please
   remove it to assure purity of builds without sandboxing`.
   #13168's comment update (run 45150189636) records that the
   `rm -rf /homeless-shelter` caller-side prefix is **not** a
   sufficient workaround: builders run with
   `HOME=/homeless-shelter`, an early builder recreates it, and a
   later derivation fails the same purity check ~2 minutes in.
   #13193 records the same conclusion from the other side: moving
   the directory aside and rewriting the passwd entry from inside
   the dispatched command did not clear it.

Per-issue records:

| issue | filed by / during | distinguishing evidence | status |
|---|---|---|---|
| **#13159** “fleet-command dispatch container cannot run nix develop out of the box (experimental features, nixbld group, homeless-shelter purity)” — label `app/fleet` | Lane ON (archer), verifying #3810's test oracle; created 2026-11-23T09:26:48Z | Runs 45142821380 (fleet-forks), 45143247862, 45144080446 — the three defects in sequence; same suite 9/9 green on laptop NixOS. Also records a 4th dispatch auto-selecting the `fleet-org-blacksmith` surface with no `--runner blacksmith` requested (no run materialised, no minutes confirmed burned) — the comment thread explicitly scopes that observation OUT of #13159 and points it at the placement program (#13146). A lane claim comment (Lane QB) records a **HOLD: execution stopped on a platform flag; awaiting Drew's item-specific approval** — the fix venue (dezren39/fleet checkout) was flagged by an execution-risk check and work stopped with nothing modified. | **open** |
| **#13168** “fleet-command container: dispatched nix develop/build commands fail out of the box (3 stacked env defects)” | Lane PJ (archer), verifying PR #13147 (`cargo test -p id-native-shell`) | Runs 45145807796 (dezren39/fleet), 45146728602 (fleet-forks), 45147861730 (dezren39/fleet); workaround shape documented, then corrected by the 45150189636 comment above (defect 3 blocks regardless) | **open** |
| **#13193** “fleet dispatch: nix develop commands fail in the fleet-build container (nix config / build-users-group / homeless-shelter purity)” | Lane OR (archer), verifying the #10191 conformance oracle | Four dispatches (45146910204, 45147692449, 45148497025, 45151466620, 45152850260 cited); notes dispatch works for plain shell/cargo/go commands the same night — only nix dev-shell repos (pkgs/id the standing example) are blocked and fall back to laptop builds | **open** |
| **#13198** “fix(fleet-dispatch): nix develop commands fail in the fleet-command container (nix config shaped for a daemon that is not there)” | Lane PV work for #7916 | Runs 45154835360, 45155700465, 45156461202 — same three layers; the #7916 change itself was verified locally on the laptop and merged as PR #13182; the issue is only about the dispatched route | **open** |

Common acceptance shape across the four: the runbook's boring form
(e.g. `fleet queue dispatch --repo developing-today/code --ref
<main sha> --command 'cd pkgs/id && nix develop --command cargo
test …' --dispatch --json`) succeeds on a GitHub-hosted surface
with no extra nix flags, no caller-side cleanup, in the command.

### 3.2 No bun — and the bun/zod v4 failure behind it — #13189

- **#13189** “fleet-command dispatch container has no bun: JS-repo
  verification cannot run dispatched” — **open**.
  Body evidence: dispatched run 45154877923
  (developing-today-forks/fleet; target
  developing-today-forks/openchamber @ lane-qg-2501) failed in ~8 s
  with `bash: line 1: bun: command not found` (exit 127) before any
  test executed. Bun-based repos (openchamber, opencode, the code
  repo's own JS packages) cannot be verified through dispatch at
  all; lanes fall back to laptop-local runs. (The brief's “Node/bun”
  phrasing: the filed evidence names **bun** specifically; no
  separate Node-missing filing was found in this family —
  **Node's presence/absence in the image is unverified** beyond
  the Containerfile, which installs no Node either: its apt list
  is direnv, jq, ripgrep, fd-find, shellcheck, sqlite3, python3,
  unzip, xz-utils, ca-certificates, curl, git, plus pinned Go and
  Nix.)
- **zod v4 evidence** — baseline comment on #13189 (Lane QG,
  2026-11-23): with bun bootstrapped via the official installer
  inside the container, the suites start but **every test file
  fails at import on pristine main** with
  `TypeError: undefined is not an object (evaluating 'z.string')`
  — described in the comment as “zod v4 under the container's
  curl-latest bun”. Identical failure on the change branch and on
  main (runs 45157517205 on fleet-forks, 45156460667 on
  dezren39/fleet), so it is not a property of any change under
  test. The same suites pass on the laptop's pinned bun
  (210 passed / 1 skipped). The comment's conclusion: the
  container is not a usable oracle for the Bun repos in two
  stacked ways — no bun baked in, and an unpinned bun/zod
  combination that cannot import the suites even once bun is
  installed; a **pinned** bun matching the repos' expected
  toolchain is what the evidence points at. Status: recorded as a
  comment on the open #13189; no separate zod issue was found.

### 3.3 Cargo not on PATH — #13170

- **#13170** “fix(fleet): fleet-command Run step lacks the
  toolchain on PATH — dispatched cargo commands fail 127” —
  **open**.
  Body evidence: run 45150362527 (dezren39/fleet, job
  137609977212), dispatched 2026-11-23 for #13146 verification:
  `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue
  --features client,queue-broker` → `bash: line 1: cargo: command
  not found` (exit 127). The body attributes it to the Run step
  executing with `CARGO_HOME`/`RUSTUP_HOME` pointed at the restored
  dependency cache without adding `$CARGO_HOME/bin` to PATH, and
  notes trivial probe commands pass (shell builtins/git), so the
  gap was invisible in the first dispatch verifications.
  Acceptance: a bare `--command 'cargo --version'` dispatch
  succeeds on each active surface with no caller-side PATH prefix.
- Comment-thread correction (probe runs 45152996653 and
  45153741326, 2026-11-23) — the mechanism is narrower than the
  body states, and the record should carry both:
  - The Run command step executes **inside** the fleet-build
    container with `PATH=/home/vscode/.nix-profile/bin:/usr/local/go/bin:…`
    — Go works out of the box; Rust does not.
  - `/home/vscode/.nix-profile/bin` contains only nix itself.
  - `$CARGO_HOME/bin` **does not exist** in the restored dependency
    cache (the cache's CARGO_HOME holds registry/git data only, no
    shims) — so the caller-side PATH prefix does **not** fix it;
    run 45151395564 with the prefix failed 127 identically (the
    body had reported that same run as the prefixed success — the
    comment supersedes that reading).
  - A rustup toolchain exists in the image only as a nix store
    path (`/nix/store/mn67k9rqg1vkfvdhfim1x1dgvxqd9fai-cargo-1.99.1-x86_64-unknown-linux-gnu`,
    `bin/cargo` under it), not linked from anywhere a command can
    rely on; a store-glob prefix works but is hash-dependent and
    brittle.

### 3.4 Family status summary

| issue | gap | status (2026-11-23, gh) |
|---|---|---|
| #13159 | nix develop: 3 stacked defects (+ Blacksmith auto-placement observation, scoped out to #13146) | open — lane HOLD recorded, awaiting Drew's item-specific approval |
| #13168 | same 3 defects | open |
| #13193 | same 3 defects | open |
| #13198 | same 3 defects | open |
| #13189 | no bun; + zod v4 import failure under curl-latest bun (comment evidence) | open |
| #13170 | cargo not on PATH in the Run step | open |
| #12894 | precursor: nix unusable in fleet-build:full legs on the org-run venue | closed |

No fix, workaround-baking, or implementation is proposed here —
per the brief, these are flag-held and Drew's to decide.

---

## 4. Nix packaging & releases

### 4.1 The fleet nixpkgs pin — the expected rev is stale

- Brief's expected value: `11dd50b…` (the pin recorded in
  MEMORY.md for developing-today/fleet's flake.lock,
  `fleetNixpkgsRev`).
- **Live value — verified twice, it has moved.**
  - `gh api repos/developing-today/fleet/contents/flake.lock` (raw):
    the sole `nixpkgs` node's locked rev is
    **`f9887a8a434bfba06e20dfc77bc8e0628266af84`**
    (owner NixOS, repo nixpkgs).
  - `gh api repos/dezren39/fleet/contents/flake.lock` (raw):
    identical — rev `f9887a8a434bfba06e20dfc77bc8e0628266af84`,
    narHash `sha256-yRzh4uc8uxFv5+5TROEIH1rUL2FF1/cvfc+uq5JGGHY=`.
  - The pin's source of truth, `flake.nix` in developing-today/fleet
    (raw, read in full): description “developing-today fleet
    umbrella — canonical shared pins”; the single input is
    `nixpkgs.url = "github:NixOS/nixpkgs/f9887a8a434bfba06e20dfc77bc8e0628266af84"`,
    and the flake exposes `fleetNixpkgsRev = nixpkgs.rev` “so
    tooling can read the pin without parsing the lock”. Its header
    states the pin is always a nixpkgs-unstable **channel tip**
    revision (the newest revision Hydra has fully evaluated/built
    for the channel, maximising cache.nixos.org hit rate) — “Never
    pin an arbitrary master snapshot here.” Moving the fleet =
    update the rev, `nix flake lock`, merge; each consumer then
    re-locks.
- Consumer contract (same flake.nix header, verbatim shape):
  consumer repos (named there: opencode, openchamber, code) declare

  ```nix
  inputs.fleet.url = "github:dezren39/fleet";
  inputs.nixpkgs.follows = "fleet/nixpkgs";
  ```

  so every repo builds against the same nixpkgs revision and dev
  shells share one store closure. (Per-repo verification that each
  consumer's current lock actually follows this pin today was out
  of scope for this pass — **unverified** per consumer.)

### 4.2 Static musl releases — publish, bootstrap pin, latest

- Publisher: `scripts/muse/publish-fleet.sh` in
  developing-today/code (raw contents read in full).
  - Usage: `scripts/muse/publish-fleet.sh [REV]` (default
    `origin/main`); at most one positional arg, flags rejected
    (exit 2) per the operator-script argument contract.
  - It fetches origin main, resolves the rev, takes
    `short=${rev:0:7}`, and forms the tag **`fleet-static-<short-sha>`**
    (i.e. `fleet-static-$short`).
  - Builds in a detached temporary worktree:
    `nix build <src>#fleet-static`; asserts the resulting
    `bin/fleet` is statically linked / static-pie (`file -L`) and
    that `fleet capabilities` reports an available `bridge` domain
    (the full profile, bridge included).
  - Asset name: **`fleet-x86_64-linux-musl`**, plus a
    `fleet-x86_64-linux-musl.sha256` sidecar. Release target repo:
    `developing-today/code`. If the release already exists, it
    re-uses it when the published sha256 matches, else re-uploads
    with `--clobber`; otherwise `gh release create --prerelease
    --target <rev>` titled “fleet static build <short>”.
  - Prints `tag=` and `sha256=` — “the tag and sha256 to pin in
    scripts/muse/bootstrap.sh”.
- Bootstrap pin: `scripts/muse/bootstrap.sh` in
  developing-today/code (raw contents read; the same pin confirmed
  in the laptop checkout `~/git/code/scripts/muse/bootstrap.sh`,
  35,960 bytes, dated Nov 23 03:16 local):
  - `FLEET_URL` default:
    `https://github.com/developing-today/code/releases/download/fleet-static-b8bbcc0/fleet-x86_64-linux-musl`
    → **current pinned tag: `fleet-static-b8bbcc0`**.
  - `FLEET_SHA256` default:
    `dd8c4b8587cf45a2367cf26a1ca80f0b58da7337d1aa9e180c45de0464ecd400`
    (the sha256 of the downloaded asset file; reproduced here
    because it is a public release checksum printed in a public
    script, not a credential).
  - The script's own header: it rebuilds a fresh Muse sandbox from
    the public release binaries, one BWS access token, and the
    restic backups; its `tools` phase downloads the pinned fleet
    into `~/.local/share/muse-bootstrap/bin` sha256-checked, and
    its `fleet` phase installs it at `~/tools/fleet`. Its `token`
    phase also provisions the three shared broker token files from
    the BWS `fleet.*` names (#12898; names only, §5.3).
- Latest release: `gh api "repos/developing-today/code/releases?per_page=30"`
  filtered to `fleet-static-*` — the newest is
  **`fleet-static-b8bbcc0`**, published `2026-11-23T09:02:38Z`,
  `target_commitish: b8bbcc0c49564ab31db8d58b026b67532db9491e`,
  assets `fleet-x86_64-linux-musl` +
  `fleet-x86_64-linux-musl.sha256`. So the bootstrap pin and the
  latest release **agree** at read time. Predecessors in the same
  listing: `fleet-static-ad0707b` (2026-11-22T04:17:35Z),
  `fleet-static-cb3e966` (2026-11-21T06:28:06Z),
  `fleet-static-7bdd04a` (2026-11-21T06:49:57Z) — note cb3e966 and
  7bdd04a are listed newest-first by the API in that order despite
  7bdd04a's later timestamp; only the head (b8bbcc0) is relied on
  here.

---

## 5. Deployment profiles / generations (laptop)

Read live on the laptop (`amd-6`) via `~/tools/laptop-ssh.sh`
(which delegates to the laptop-agent path) on 2026-11-23. Two
independent sources agree: the profile tree
(`~/.local/share/deploy/profiles/`) and the deploy generation logs
(`~/.local/state/deploy/generations/<target>.jsonl`).

### 5.1 Current generations

| target | live profile → generation | artifact (nix store) | source_rev (generation log) | activated |
|---|---|---|---|---|
| queue-broker | `.generations/queue-broker/5` | `/nix/store/0ymw7b3zl2hxij9npmzal6h5fq2g4cp3-fleet-0.1.0` | `b8bbcc0c49564ab31db8d58b026b67532db9491e` | 2026-11-23T09:08:19Z, result `activated` |
| laptop-agent | `.generations/laptop-agent/3` | `/nix/store/20r9fbb0v81l61gk11vh2fj2dbclif8x-laptop-agent-0.1.0` | `b8bbcc0c49564ab31db8d58b026b67532db9491e` | 2026-11-23T09:11:22Z, result `activated` |
| secrets-broker | `.generations/secrets-broker/2` | `/nix/store/v6g0952xc7w27xa8c2l4a6ga0rnddsp1-fleet-0.1.0` | `working-tree` | 2026-11-23T00:35:37Z, result `activated` |

- The brief's expectation is **confirmed for queue-broker**:
  generation **5**, source rev **b8bbcc0** — the same rev as the
  latest static release and the bootstrap pin (§4.2). laptop-agent
  generation **3** is on the same b8bbcc0 rev (deployed three
  minutes later).
- **secrets-broker generation 2 is the deliberate non-flip**: it
  was activated earlier the same day (00:35Z) from a
  `working-tree` source — not a committed rev — and the b8bbcc0
  rollout at ~09:08–09:11Z flipped queue-broker and laptop-agent
  only. It remains on gen 2 while its siblings are on b8bbcc0.
  Its gen-1 record (2026-11-17) shows the family's first attempt
  failed its health check (HTTP 503, “stopped (no previous
  generation)”) before gen 1 was later activated at 09:10Z that
  day from rev `099672152ee5…`.
- Profile mechanics observed: each profile name in
  `~/.local/share/deploy/profiles/` is a symlink into
  `.generations/<target>/<n>`, whose `current` symlink points at
  the artifact store path; a parallel nix-env profile
  (`<target>.nix-env-profile` → `-2-link`) registers the same
  artifact. Generation directories present: queue-broker 1, 2, 3,
  5; laptop-agent 1, 2, 3; secrets-broker 1, 2.
- One anomaly worth recording exactly as found: there is **no
  queue-broker generation-4 directory** under
  `.generations/queue-broker/`, and generation 3's `current`
  symlink points at a (now-absent) nested path
  `…/profiles/queue-broker/.generations/queue-broker/4/current`.
  The generation log explains it: gen 4 (artifact
  `/nix/store/wlnm1dwmwz8ljxzcqrysnr8chqwmahrh-fleet-0.1.0`,
  source_rev `8f07c02ffd1393176536f7a0ecdcf200661b6ac5`,
  2026-11-23T02:08:00Z) carries the note “record repaired
  2026-11-23 by the fleet-rollout coordinator: generation 4 was
  flipped via the PR #12570 profile mechanism outside deploy.py
  logging; artifact and source rev verified first-hand from the
  live profile before the generation-5 deploy”. So gen 4 was a
  real flip done outside deploy.py, later repaired into the log,
  and its directory has since been superseded/collected —
  **why the directory itself is absent is unverified** beyond the
  log note.
- Earlier history in the same logs: all three targets' gen 1/2
  (2026-11-17) were built from source_rev
  `099672152ee5ca56ca9a4faac7ca950f5da6114e` (fleet artifact
  `vn6vam1491lrahby7ib9rq2a291abb58-fleet-0.1.0` shared by
  queue-broker and secrets-broker; laptop-agent had its own
  `57zi2mrsb6sy6d654cb12173qw32wjj4-laptop-agent-0.1.0`), each with
  a deploy → rollback-to-gen-1 → redeploy pattern on 2026-11-17.

### 5.2 deploy.py + `--registry` — the reliable route

- Source: `pkgs/deploy/deploy.py` in developing-today/code
  (laptop checkout `~/git/code/pkgs/deploy/`; siblings
  `default.nix`, `services.nix`, `restore.py`, `route-proxy.nix`,
  `quota-recorder.py`, and an argv/property test suite). Its
  docstring (read in full) describes the deployment-architecture
  CLI (issue #5766 slices 1–3; #5770 slice 4):
  - The **registry** is JSON baked into the package at build time
    (the Nix expression renders the service definitions, replacing
    each derivation with its store path, without depending on it:
    building the CLI realises no target package; a deploy builds
    its target's package from the derivation the registry
    records — code#12738). **`--registry PATH`** (or
    `$DEPLOY_REGISTRY`) points the CLI at another registry file —
    documented there as the route the flake checks use for the
    malformed-definition path, and the operator route for
    deploying against a non-baked registry.
  - Validation is the oracle for “well-formed definition”: exactly
    the §3 fields (plus optional `healthTimeoutSeconds` and the
    slice-4 `timer`), each of the right type/range; any violation
    is reported by field and exits non-zero — “a registry that
    cannot be validated must never be enumerated as if it were
    trustworthy”.
  - Activation (systemd + sandbox classes): install the next
    generation into the per-service profile (a deploy-managed
    profile whose generation directories hold a `current` symlink
    at the artifact; store-path artifacts are additionally
    registered with `nix-env --set` on a parallel nix profile —
    exactly the two link layers observed in §5.1), write the unit
    pointing at the profile's `current` link (never a bare store
    path, “so a generation flip is a profile repoint, not a unit
    rewrite”), daemon-reload, restart only that unit, health
    check, and **flip back to the previous generation (and restart
    on it) on any failure**. Rollback is the same machinery pointed
    at the previous distinct generation. The generation log is one
    append-only JSONL per target under the state dir (generation
    id, source rev, artifact, timestamp, actor) — the logs quoted
    in §5.1.
  - Sandbox class (§4.4 of the design, slice 4): the per-service
    profile lives in the persistent home
    (`~/.local/share/deploy/profiles` by default — /etc does not
    survive a sandbox recycle, home does), rendered units are
    stored inside the generation directory, and `deploy restore`
    re-links every manifest target's current generation after a
    recycle with no registry needed; `pkgs/deploy/restore.py` is
    the same contract as a standalone stdlib script. (The laptop
    profiles in §5.1 live under that same home-based root.)
  - The launchd real path “still refuses explicitly: it cannot be
    exercised from the Linux fleet and stays a rendering/planning
    stub.”
  - Why “reliable route” is the right summary: the gen-4 episode
    (§5.1) is the counterexample — a flip done via the PR #12570
    profile mechanism **outside** deploy.py left no log record and
    a dangling generation-3 pointer, and had to be repaired into
    the log by hand; flips done through deploy.py are validated,
    health-checked, auto-rolled-back on failure, and logged.
    (That last sentence is the researcher's synthesis of the
    docstring + the gen-4 log note, labelled as such.)

### 5.3 Related provisioning fact (names only)

`bootstrap.sh`'s token phase (§4.2) fetches the three shared
broker client tokens from BWS under the names
`fleet.secrets-broker-token`, `fleet.queue-broker-token`,
`fleet.laptop-agent-token` into `~/tools/` at mode 0600 — the
distribution route decided 2026-11-22 for non-muse seats
(code#12898). The older BWS name `secrets-broker.token` is a
different value and is never the client token (MEMORY.md). No
values were read in this pass.

---

## 6. Unverified / discrepancy list

1. **Submodule count**: brief/memory say 12; live `.gitmodules`
   declares **22** (§1.1). The 22 figure is verified; when the
   count changed was not traced.
2. **fleetNixpkgs pin**: brief expects `11dd50b…`; live flake.lock
   + flake.nix in both fleet repos say
   **`f9887a8a434bfba06e20dfc77bc8e0628266af84`** (§4.1). Verified
   current; the `11dd50b` value is stale.
3. **“102/102” tally** for run 45022131766: run + job success
   verified via REST; the literal tally in the job summary/log
   was not downloaded (§1.3).
4. **Ruleset role id 5 = admin**: payload verified; the id→name
   mapping rests on standing context, not the payload (§1.3).
5. **“R2 Nix cache is the only Worker”**: verified as the only
   Worker surface on the dispatch path read here; no fleet-wide
   Worker inventory was enumerated (§2.4).
6. **Node in the fleet-build image**: the bun gap is filed
   (#13189); Node's absence is inferred from the Containerfile's
   install list, not from a filed issue or a probe run (§3.2).
7. **Per-consumer follows verification**: the consumer contract is
   verified from the fleet flake's header; each consumer repo's
   current lock was not individually re-read (§4.1).
8. **Queue-broker gen-4 directory absence**: the flip and its
   out-of-band mechanism are documented in the generation log;
   why no directory remains is unverified (§5.1).
9. **GHCR_ORG_PAT holding the laptop gh OAuth token**: asserted
   by the workflow header only as “the classic PAT in this repo's
   GHCR_ORG_PAT secret” + the workflow-scope requirement; the
   token-identity detail comes from MEMORY.md standing context,
   not from any read of the secret (values never read) (§1.3).

## Sources index

- gh REST (2026-11-23): repos dezren39/fleet, developing-today/fleet,
  developing-today-forks/fleet, developing-today/code; contents
  `.gitmodules`, `flake.lock`, `flake.nix`,
  `.github/workflows/{sync-forks,fleet-command,fleet-build-container}.yml`,
  `.github/actions/fleet-command/action.yml`,
  `containers/fleet-build/Containerfile`,
  `pkgs/fleet/crates` listing, `pkgs/fleet/doc/agents/remote-build-and-test.md`,
  `pkgs/fleet/book/concepts/dispatch.md`,
  `scripts/muse/{publish-fleet,bootstrap}.sh`; compare
  `dezren39:main...main` on developing-today/fleet; rulesets
  26550095 / 26550098; actions run 45022131766 (+ jobs);
  releases filtered `fleet-static-*`; PR #13128; issues
  #13159 #13168 #13193 #13189 #13198 #13170 #12894 (+ comments on
  #13159 #13168 #13170 #13189).
- Laptop (`~/tools/laptop-ssh.sh`, 2026-11-23): `~/git/code/pkgs/fleet/`
  (crates listing, `fleet-surfaces.json`, `placement.rs` cap reads,
  book placement/dispatch pages), `~/git/code/pkgs/deploy/deploy.py`
  docstring, `~/git/code/scripts/muse/bootstrap.sh` pin line,
  `~/.local/share/deploy/profiles/` tree + `.generations/`,
  `~/.local/state/deploy/generations/{queue-broker,laptop-agent,secrets-broker}.jsonl`.
- Standing context: `~/MEMORY.md` / `~/AGENTS.md` (fork-sync
  rulings, GHCR_ORG_PAT identity, ruleset-bypass order, BWS token
  names) — used only where flagged above.
