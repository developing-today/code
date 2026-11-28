# mcpx — current state (research, 2026-11-23)

All facts below are from first-hand reads on 2026-11-23: the laptop checkout
`~/git/code` at `4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a` (2026-11-23 06:31 CST) via
`~/tools/laptop-ssh.sh`, the installed binary's own `--help`, and GitHub REST reads.
Anything not verified is labelled as such.

## 1. What mcpx is

- A **Go daemon + CLI that aggregates every MCP server on the machine behind one
  surface** (`pkgs/mcpx/` in developing-today/code; VERSION `0.1.0`). One canonical
  source: the multi-copy dedup is done and tracked as shipped on
  [#8822](https://github.com/developing-today/code/issues/8822) (open as tracking home).
- The model (from `pkgs/mcpx/README.md` and `pkgs/mcpx/AGENTS.md`, verified against the
  binary): a local daemon owns every MCP server process; agents never load tool schemas
  into context up front. The loop is `mcpx ls` → `mcpx search <words>` →
  `mcpx types <namespace>[.<tool>]` → `mcpx run <name>` / `mcpx exec '<code>'`.
  Scripts are TypeScript on a real runtime (deno, else bun, else node) with the full
  runtime (filesystem, network, subprocesses); `log`, `emit`, `tools` and every
  namespace are preloaded on `globalThis`. Named scripts live at `.mcpx/scripts/<name>.ts`
  searched upward, then `~/.config/mcpx/scripts/`.
- Lineage: built as the replacement for the lootbox fork (`pkgs/mcpx/ASSESSMENT.md`,
  corrected 2026-10-30). mcpx now carries nine direct dependencies
  (`docs/dependencies.md`).

## 2. Code layout (pkgs/mcpx/)

- Entry: `cmd/mcpx/main.go`; public Go API: `pkg/client` (the embedding surface —
  see [#1877](https://github.com/developing-today/code/issues/1877), open: public Go API,
  C ABI, WASM, mcpx as its own upstream).
- `internal/` (~60 packages): `adapter`, `api`, `artifacts`, `cache`, `catalog`,
  `cli`, `config`, `conformance`, `daemon`, `dashboard`, `defaults`, `diagnose`,
  `e2e`, `elicit`, `events`, `eventstore`, `execsvc`, `hooks`, `launcher`, `logging`,
  `logstore`, `mcpauth`, `mcpclient`, `mcpheaders`, `mcpserver`, `mcpspec`, `model`,
  `openapi`, `opencode`, `plugin`, `pluginsession`, `policy`, `pool`, `preflight`,
  `probe`, `progspec`, `recipes`, `settings`, `tasks`, and more.
- Packaging: `package.nix`, `container-image.nix`, `real-servers.nix`,
  `go-toolchain.nix`; Nix modules in `pkgs/mcpx/modules/` (`common.nix`,
  `nixos.nix`, `home-manager.nix`, `darwin.nix`).
- Agent skill shims live in-repo: `.opencode/skills/mcpx-{agy,codex,claude,opencode2}`
  and `.agents/skills/mcpx-*` — per-harness adapters over the same daemon.

## 3. CLI surface (installed binary, `mcpx --help`, 2026-11-23)

~50 commands in five groups:

- **Running code**: `embeddings`, `exec`, `feedback`, `prompt` (words in, script back),
  `proxy` (OpenAI-compatible LLM reverse proxy), `recipes`, `run`, `scripts`, `skills`.
- **Finding tools**: `batch`, `call`, `catalog` (token-budgeted), `client` (generated
  TS client), `ls`, `openapi`, `prompts`, `resources`, `schema`, `search` (keyword or
  semantic), `types`.
- **The daemon**: `cache`, `daemon [--detached]`, `daemons` (multi-daemon per machine),
  `refresh`, `reload`, `restart`, `serve` (MCP over stdio for a spawning host),
  `status`, `stdio-proxy` (daemon API over stdin/stdout, for ssh tunnels), `stop`,
  `supervise` (internal), `upgrade` (in-place handover to a new binary).
- **Configuration**: `adapter` (CLI programs declared as tools), `api` (OpenAPI
  services as tools), `completion`, `config`, `help`, `init`, `man`, `plugin`,
  `registry` (find + add public-registry servers), `secrets` (in-memory only),
  `servers` (add/remove without restart), `settings` (read/change any setting,
  from anywhere).
- **Inspection**: `dashboard` (live web dashboard), plus doctor/diagnose/stats/elicit
  surfaces referenced across the settings registry and issues (the help output was
  read to the dashboard line; the inspection tail was not fully captured — verify
  in the survey pass).

## 4. Daemon lifecycle & transports

- Transports (`internal/daemon/`): local (unix socket / systemd —
  `transport_systemd_linux.go`), `transport_ssh.go`, `transport_tsnet.go`
  (tsnet: the daemon as a tailnet node). Client side mirrors this:
  `internal/cli/client.go` builds a tsnet dialer (`daemon.NewTSNetDialer`).
- **Takeover handoff** (today): `mcpx upgrade` / `daemon --takeover` — the old daemon
  starts its successor as its own child (stays in the service cgroup) and passes
  stdio children's pipes over a unix socket. Streamable HTTP upstream sessions
  survive; SSE upstream sessions do not (closed + re-initialised on first call).
  `pkgs/mcpx/TODO.md` names the replacement: **a supervisor that owns each stdio
  child's pipes for the child's whole life** — design issue
  [#1384](https://github.com/developing-today/code/issues/1384), Phase 3
  [#9963](https://github.com/developing-today/code/issues/9963) (supervised stdio by
  default + lifecycle hygiene).
- **Cold-restart session resume**: legacy streamable HTTP upstream sessions resume
  from `upstream-sessions.json` (#1415 Phase 1, #1416 Phase 2 landed; design
  [#1414](https://github.com/developing-today/code/issues/1414); the Phase 3
  graceful-shutdown DELETE question remains open).
- Inline start: when no daemon is running, the CLI starts one inline and polls for
  readiness — the poll interval is the `daemon.inlineStartPoll` setting, read at
  `internal/cli/connect.go:305`. Sibling bounds (`daemon.inlineStartTimeout`,
  `autostart.pollInterval`, takeover/socket-probe timeouts) all have effect probes;
  the poll interval itself does not (see §5).
- Stream scanning: CLI/exec read daemon responses through a stream scanner sized by
  `http.streamBufferInit` / `http.streamBufferMax` (`internal/cli/streamscan.go`,
  `opcmd.go:555`, `client.go:1283`, `exec.go:302`); #11308 fixed the
  init > max line case.

## 5. The config system and the exemption tail

- Settings are a typed registry (`internal/settings/`, ~45 files: `registry.go`,
  `wire.go`, `defaults` in `internal/defaults/defaults.go`). **252 distinct setting
  paths** are declared (`Path:` literals, counted 2026-11-23). Every setting is
  reachable via config file, environment, and `mcpx settings`, with a resolved-value
  provenance view (`mcpx config`).
- **Effect-probe contract** (`internal/settings/effect.go`): every setting must have
  either an **effect probe** (a named e2e test proving the setting changes behaviour —
  `EffectProbes`, 220+ entries) or a **written exemption** (`EffectExemptions`).
  `CheckEffectCoverage` fails the build on any gap, on a probe+exemption duplicate,
  or on an exemption without a reason. The exemption count is ratcheted by
  `EffectExemptionPin` — **it can only shrink, and only in the PR that adds the
  replacing probe**. The pin's comment is a dated shrink log (#8467–#8474 chains,
  2026-11-17 → 2026-11-23).
- **The exemption tail is exactly 8 settings** (pin = 8, verified 2026-11-23):
  | Setting                                                                            | Written reason (abridged)                                                                                      | Probe shape it needs                                                                             |
  | ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
  | `daemon.inlineStartPoll`                                                           | lifecycle/timing: a bound/interval the isolated harness cannot vary deterministically                          | a harness that can observe inline-start readiness timing                                         |
  | `daemon.tsnet.hostname`                                                            | lifecycle/binding: a network identity the harness cannot vary                                                  | a tailnet the harness controls                                                                   |
  | `daemon.tsnetUpTimeout`                                                            | tsnet bring-up needs a tailnet the harness does not have                                                       | same                                                                                             |
  | `http.streamBufferInit`                                                            | allocation-capacity hint only; allocation size is not a stable e2e behaviour oracle                            | an allocation-observing oracle, or a contract decision that the hint is probe-exempt by kind     |
  | `embeddings.wasmPath`                                                              | needs a compiled wasm model fixture the harness does not provide; no embed-exporting model ships with the repo | a small embed-exporting wasm fixture model in-tree                                               |
  | `plugin.instructions`                                                              | read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host                  | JS-host e2e probes (the #8472 pattern that cleared 11 sibling plugin.* exemptions on 2026-11-23) |
  | `plugin.disabledTools`                                                             | same                                                                                                           | same                                                                                             |
  | `protoTasks.cancelGrace`                                                           | task-store cancellation timing; covered by internal/tasks store tests, not a settings with/without probe       | a settings-level cancel-timing probe                                                             |
  | The last three are **flag-held** (Drew's item-specific approvals, recorded         |
  | 2026-11-23): #8472's `plugin.disabledTools` + `plugin.instructions` remainders and |
  | #8473's `protoTasks.cancelGrace` (the last exemption in #8473 — landing it empties |
  | that issue's list). The first five are unheld and unclaimed.                       |
- Coverage burn-down tracking homes (open): #8465 (CLI commands, running group),
  #8469 (settings http-transport family, blocked), #8471 (settings model-ai family,
  parked), #8472 (settings plugin family, in-review), #8473 (settings proto family).
  Also open: #2686 (e2e/settings suites cannot run in the nix build sandbox —
  checkPhase exclusions), #10330 (three scripts suites have no registry-named CI venue).

## 6. The book (exists — the Pages root)

- Sources: `pkgs/mcpx/docs/` — mdBook (`pkgs/mcpx/book.toml`, src = `docs`,
  `create-missing = false`), built by `pkgs/mcpx/docs.nix` as flake output
  `.#mcpx-docs` with a `siteUrl` argument (#1939 Approach C, #1941). A Go
  preprocessor (`docs-history`, stdlib-only) renders release metadata/history.
- Scale: `docs/SUMMARY.md` has **84 chapter entries** (112 lines); **120 markdown
  files, ~287,877 words** under `docs/` including the package-local archives:
  `decisions/` (8), `changes/` (8), `compare/` (12), `spec/` (13), `audit/` (3),
  `testing/` (3), `catalogue/` (1), plus 48 top-level chapters (index, overview,
  quick-start, install, cli, compendium, configuration, modes, consumer, container,
  protocol, http-api, elicitation design, parity, proposals, ideas, declared-but-unread,
  in-name-only, landing checklist, level-2 page map, research-and-findings, …).
- Deploy: `.github/workflows/mcpx-docs.yml` builds `.#mcpx-docs` on push to main and
  deploys the **whole Pages site** — mcpx is the root book at
  https://developing-today.github.io/code/ and the fleet book joins under `/fleet/`
  (staged by the fleet-docs path; #13225, open, fixes staging fleet docs into the
  writable Pages directory). One Pages site per repo: every other book (fleet done,
  atlas #13251 in flight, id planned) joins this deploy under its subpath.
- Docs trackers (open): #1888 (the book — overview/manual/compendium), #1889
  (runtimes/languages/dependencies), #1890 (site platform), #1892 (docs + testing
  strategy), #1894 (docs level 1 — the 30-second layer), #1943 (custom subdomain —
  **blocked, Drew's word**: "apply it" for the mcpx CNAME is his recorded pending
  decision).
- Note: mcpx design records live **package-locally** (`docs/decisions`, `docs/changes`)
  — the repo-root `doc/` archive has **0** mcpx-named entries. The ID/Atlas programs
  put designs in `doc/`; mcpx's own convention is the book tree itself.
- Book-vs-surface gap (to be measured, not assumed): the CLI has ~50 commands and
  252 settings; the book has a compendium + configuration chapters and a
  docs-twins currency check inside the `.#mcpx-docs` build, but no mechanical
  coverage proof of book-vs-binary surface has been run for this program — that
  audit is a deliverable.

## 7. Deployment: the Nix cascade

- Modules ship in `pkgs/mcpx/modules/` but are consumed **through the openchamber
  flake**: `dezren39-nix/flake.nix` re-exports `inputs.openchamber.nixosModules.mcpx`
  / `homeManagerModules.mcpx` / `darwinModules.mcpx` (lines 318–320), and the laptop
  config enables `services.mcpx` in `home/common/default.nix` (line 1219) with the
  daemon's settings in `home/common/mcpx-settings.nix` (`services.mcpx.settings`).
  Per MEMORY (2026-11-04 consolidation): `services.openchamber.enable` cascades to
  opencode + mcpx.
- Fossil of the cascade: the laptop's live `~/.config/mcpx/config.json` sits beside
  `config.json.pre-cascade-20261103` — the pre-consolidation config, kept.
- The daemon runs from the Nix profile (`/etc/profiles/per-user/user/bin/mcpx`).

## 8. Integrations (verified state)

- **Atlas → mcpx: LIVE.** `home/common/mcpx-settings.nix` declares server `"atlas"` →
  `https://atlas-search.developing-today.workers.dev/mcp` (the atlas-search Worker's
  MCP endpoint, POST /mcp), gated per the comment on #8625. This is Drew's 2026-11-22
  order (Atlas MCP as a versioned Cloudflare deployment wired into the laptop's mcpx
  as a remote server) in its landed form. The remainder lives on
  [#8625](https://github.com/developing-today/code/issues/8625) (open, blocked:
  cross-client acceptance — OpenClaw / OpenCode / OpenChamber / Muse lanes).
  Atlas sensitivity ceilings are enforced server-side in the Worker; what mcpx adds
  per-client on top is an open design question for this program.
- **ID ↔ mcpx**: ID ships its own MCP bridge with `mcp_policy::PolicySession` in
  id-caps (#7916 closed; #7917 closed 2026-11-23) — MCP servers sit behind ID's
  capability layer. The mcpx-side questions are open design: ID as an mcpx upstream,
  mcpx as the gateway ID clients reach the world through, and code-mode interop
  ([#1940](https://github.com/developing-today/code/issues/1940): nested code mode,
  `?codemode=false`, what a harness expects of `mcpx_exec`).
- **fleet ↔ mcpx**: design-stage. Design-mill passes #10389 (one fleet surface for
  OpenCode/OpenChamber/ID/mcpx), #10390 (fleet subcommands, pass-through + endpoint/
  credential resolution), #10393 (MCP everywhere — fleet MCP via mcpx, namespaces,
  fan-out, remote graph), #10405 (middle-road fleet MCP, transitive capability
  purity) are all open. `fleet search` (Atlas) is a separate, landed integration.
- **Harness adapters**: the opencode plugin is the deep integration — v1 plugin,
  the v2 rewrite [#1858](https://github.com/developing-today/code/issues/1858) with
  its SET-REMAINDER [#2841](https://github.com/developing-today/code/issues/2841)
  (blocked), `docs/opencode-plugin-v2.md` (verified API facts). A harness-free
  plugin core with adapters for codex / claude code / pi / manus is
  [#1861](https://github.com/developing-today/code/issues/1861) (open). Per-harness
  skills: `mcpx-{agy,codex,claude,opencode2}`.
- **Server parity**: epic #8993's second half — GitHub / Atlassian / Datadog MCP
  server logins at parity ([#8809](https://github.com/developing-today/code/issues/8809),
  blocked, `user/escalated`: Drew-gated OAuth/PAT). The live laptop config already
  aggregates many servers (chrome-devtools, cloudrun, codebase-memory, codedb,
  codex-connectors, deepwiki, desktop-commander, antigravity, … — from
  `~/.config/mcpx/config.json` keys, read 2026-11-23).
- **Embeddings inside mcpx**: the daemon has its own embeddings subsystem
  (`internal/catalog/embeddings.go`): a wazero-based Wasm embedder
  (`embeddings.wasmPath`), remote OpenAI-compatible embeddings (probed), and
  semantic `mcpx search`. This is mcpx's tool-discovery search — a different index
  from Atlas's corpus search; the boundary between them is a design question.
