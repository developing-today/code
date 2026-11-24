# mcpx — existing designs, docs & the design backlog (research, 2026-11-23)

Sources: laptop checkout `~/git/code` at `4cfceaa8` (2026-11-23), `pkgs/mcpx/docs/`
tree + SUMMARY, `pkgs/mcpx/{ASSESSMENT,TODO,OPENCODE-V2}.md`, GitHub REST issue
reads. Issue numbers are developing-today/code unless prefixed otherwise.

## 1. Where mcpx designs live (three homes, no single register)

1. **The book tree** (`pkgs/mcpx/docs/`) — the public record: 84 SUMMARY chapters,
   plus package-local archives `decisions/` (8 files), `changes/` (8),
   `compare/` (12), `spec/` (13), `audit/` (3), `testing/` (3). Design-shaped
   chapters already in the book: `elicitation.md` (a design), `proposals.md`,
   `ideas.md`, `parity.md`, `protocol.md` (every revision, both directions),
   `compendium.md` (structure + mapping), `declared-but-unread.md` and
   `in-name-only.md` (self-audits of settings that are declared but never read /
   named but not real — the book audits the code, unusually).
2. **Package working docs** — `ASSESSMENT.md` (the lootbox replacement audit,
   with a 2026-10-30 corrections header — claims aged against the code are
   corrected in place), `TODO.md` (the supervisor design + session-resume state,
   written as prose, not issues), `OPENCODE-V2.md`, `TESTING.md`, `AGENTS.md`
   (the agent-facing contract), `agentdocs.go` + `agents-docs/`.
3. **GitHub issues** — the tracker family (#1884–#1987, filed as one design
   pass) and the design-mill passes (#10389–#10405). There is **no mcpx charter
   doc** and no single design register; #1900 (the super tracker, §3) is the
   closest thing and it is an ordering, not a register.

Contrast: the ID program has no charter either, but ID designs funnel into the
repo-root `doc/` archive (~2,900 paths). mcpx has **0** entries there — its
archive is the book tree. Any program-level design series must respect that
split: program records land in `doc/` per fleet convention; mcpx product
designs belong in `pkgs/mcpx/docs/` where the book renders them.

## 2. The tracker family (one design pass, still the backlog's spine)

Filed as a set (source/creator), each a theme with its own sub-designs:

- #1884 TTL renewal — one convention (touch-on-read), not a renew endpoint per resource
- #1885 extensibility — plugins, hooks, tool sources, embedding (tracker)
- #1887 surfaces — reaching the daemon over ssh and the browser, keeping what you reached
- #1888 the book — overview, manual, compendium (tracker)
- #1889 runtimes, languages, dependencies
- #1890 docs site platform — generator, Pages, subdomain, agent markdown
- #1891 the AI layer and search — where a model comes from, everything that would use one
- #1892 comprehensive docs + testing strategy
- #1894 docs level 1 — the 30-second layer
- #1900 **the backlog build-out** — every theme, in one order, with the disagreements
  between them (super tracker)
- #1905 testing: integration tests for every feature, across environments
- #1911 how the JS runtime is chosen; TypeScript handling; automatic validate/compile
- #1915 resolve a script's import graph statically; prepare missing dependencies (in-review)
- #1925 where the model comes from — a provider layer mcpx owns, model profiles,
  life after MCP Sampling's deprecation (**blocked**)
- #1927 checking, refusing, repairing a request — a graded ladder from diagnostics
  to rewrite-and-run
- #1929 lifecycle hooks on exec — prePrompt/postPrompt/onError as one declared list
- #1931 administration by intent — finish the operation surface, then route a
  sentence to a declared operation
- #1940 mcpx × opencode code mode — nested code mode, `?codemode=false`, `mcpx_exec`
- #1950 OAuth authorization for remote servers and for /mcp (61 conformance rows; in-review)
- #1957 smaller conformance gaps — user identity, annotation trust, rate limits,
  cursors, URI encoding, prompt validation, restart, invalidation
- #1983 testing environments as CI matrix entries; #1985 CI workflow skeleton;
  #1987 extended platforms (static artifacts; alpine/scratch/distroless/flatpak/Fedora/Windows)
- #4375 design: read it bounded — one response-read contract for outbound calls
- #7843 conformance: user-identity — how elicitation state binds identity (or refuses to)
- #8242 evaluate in-house deterministic simulation (Antithesis declined)
- #8465/#8469/#8471/#8472/#8473 the settings/CLI coverage burn-downs (see state file §5)
- #8822 dedup (shipped; tracking home), #8809 server-parity logins (Drew-gated)
- #9963 Phase 3 of #1384 (supervised stdio by default), #11147 two unbuilt #1962
  considerations (adapters.allowBare gate — also #13118; cwd parameter for bare tools)
- #10330 CI venue for three scripts suites; #10471 CI over budget (rolling);
  #2686 nix-sandbox checkPhase exclusions; #3016 nightly stress failure (blocked)

## 3. The design-mill passes (fleet-level, mcpx as a component)

- #10389 one fleet surface for OpenCode, OpenChamber, ID, and mcpx (priority/high)
- #10390 pass 1: fleet subcommands for the four tools — pass-through, endpoint +
  credential resolution
- #10393 pass 4: MCP everywhere — fleet MCP via mcpx, namespaces, fan-out, remote graph
- #10394 pass 5: embedding research — ID and mcpx inside fleet
- #10405 supplement: middle-road fleet MCP, ID graph publication, transitive
  capability purity

These are the fleet↔mcpx integration designs; none has landed as of 2026-11-23
(all open, `status/triaged`).

## 4. Book coverage vs the actual surface (gap analysis, to be proven mechanically)

Knowns: ~50 CLI commands (binary `--help`), 252 settings paths, 8 effect
exemptions, ~60 internal packages, one public Go package (`pkg/client`), the
/v1 HTTP API (book has `http-api.md`), the MCP protocol surface (`protocol.md`),
transports socket/systemd + ssh + tsnet.

Suspected gaps (hypotheses for the survey pass, labelled unverified):
- The book predates parts of the settings ratchet: whether the configuration
  chapter enumerates all 252 settings with defaults + provenance, or a subset,
  is unmeasured. The docs-twins check inside `.#mcpx-docs` gates twin currency
  for *some* surfaces only.
- The exemption tail (§5 of the state file) — the book's `declared-but-unread.md`
  covers declared-but-unread settings; whether the 8 effect exemptions are
  documented as a set anywhere public is unmeasured.
- Post-book features: tsnet transport, the takeover handoff, session resume
  (#1414 phases), the Atlas remote server pattern, multi-daemon (`mcpx daemons`),
  the OpenAI-compatible `proxy` — each needs a chapter-presence check.
- Reverse gap: book chapters describing surfaces the code no longer has
  (ASSESSMENT.md's corrections header is the pattern for how this repo handles
  aged claims; the book itself has no equivalent sweep on record).

## 5. Standing design tensions the investigation must engage

1. **Supervisor vs takeover** (#1384/#9963, TODO.md): the takeover handoff is a
   self-described stopgap; SSE sessions can't move. The supervisor design is
   the daemon-lifecycle centrepiece.
2. **Session state across restarts** (#1414): Phases 1–2 landed; Phase 3
   (graceful-shutdown DELETE) open. Cold restart, takeover, and supervisor
   designs interact — they are one lifecycle design, currently three documents.
3. **Identity & conformance** (#1957, #7843, #1950): elicitation state binding
   identity, annotation trust, OAuth for remote servers and for /mcp itself.
   This is where per-client policy lives today — prompts and rate limits
   (proto.* settings), not a capability-ceiling model.
4. **The model layer** (#1891, #1925): mcpx owns a provider layer + profiles;
   MCP Sampling's deprecation forces the question. The `proxy` command and
   `prompt` command both hang off this.
5. **Two searches**: mcpx's semantic tool search (embeddings subsystem,
   `embeddings.wasmPath`, wazero) vs Atlas corpus search (served *through*
   mcpx as the `atlas` remote server). Discovery-inside-mcpx vs corpus search
   is an un-drawn boundary.
6. **Embedding mcpx itself** (#1877, #1885): public Go API (pkg/client exists),
   C ABI, WASM, mcpx-as-its-own-upstream; the harness-free plugin core (#1861).
   Library use is designed but not yet a documented, versioned surface.
7. **Release channels** (#1881 release.yaml + artifacts + install channels;
   #9900 GHCR container channel never published — blocked): the docs promise
   install paths the release pipeline only partly keeps.
8. **Docs platform** (#1890, #1943): the book is live at the repo Pages root;
   the custom subdomain is Drew's pending word; the fleet/atlas/id books now
   share mcpx's deploy — the book's own deployment chapter must absorb that
   fact (it is no longer mcpx's site alone).
