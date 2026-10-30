# mcpx level-2 page map

The map for the level-2 docs: the layer a person reads right after
installing mcpx ("basically a more normal set of docs — a new person
might assume this is all there is and won't be upset"). It assigns
every one of the 30 items in the source notes to a page and a fixed
anchor, records the repo material each item draws on, the issue that
writes it, and which parts come from the generated reference.

- Tracker: #1898. This map is part 1 of #1848; the first seven items'
  content is #1848 part 2, still to write.
- Structure: Approach C from #1848 — about ten grouped pages, each
  item with its own fixed anchor (`page#anchor`), anchors set
  explicitly rather than derived from headings so a renamed heading
  never breaks a link. Totals: 10 pages, 30 anchors — `configuration`
  4, `modes` 3, `interfaces` 2, `cli` 3, `mcp` 3, `binary` 1,
  `scripts` 1, `http-api` 2, `plugins` 6, `faq` 5.
- Navigation order follows the notes. (#1848 open question 3 asks
  whether task order would be better; the notes' order stands until
  that is answered.)
- Writing issues, in this repo's numbering: #1848 (configuration +
  modes), #1851 (interfaces, CLI, MCP, binary, script, /v1),
  #1855 (plugins), #1859 (FAQ). The generated reference is #1843.
  (#1848's body quotes the original dezren39/nix numbering —
  #146/#147/#148/#149 and #144 — for the same issues.)
- **Rule for generated content at level 2.** When a page needs
  reference data, it embeds the relevant slice generated from the
  same source (for example "settings where scope = client"), it does
  not just link to the full reference — linking would break "this is
  all there is". #1843 has to support filtered embeds, not only whole
  pages (#1848 open question 7).

## Verification (2026-10-29, HEAD `5a34fa70`)

Every source path below was re-checked to exist at HEAD. Paths are
relative to `pkgs/mcpx/`. Three corrections to the refs as quoted in
#1848, found by that check:

- The story file is `docs/story.md`; there is no `pkgs/mcpx/story.md`.
- Line numbers quoted in #1848 have drifted (for example the
  `mcpServers` tag is now at `internal/config/config.go:190` and
  `SearchPathFrom` at `internal/config/config.go:713`). This map
  cites files, not lines, so it does not rot the same way.
- `.mcpx/scripts/` does not exist at HEAD (see item 82).

## The map

| Notes line | Item (verbatim) | Page `#anchor` | Source material in the repo | Written by | Generated part (#1843) |
| --- | --- | --- | --- | --- | --- |
| 53 | common config | `configuration#common-config` | `docs/configuration.md` "Where a value can come from", "Scope", "Hot", "Reaching them"; `internal/config/config.go` (search path); `internal/cli/manual.go` entries `config`, `settings`, `init` | #1848 | link to the settings table; no copy |
| 54 | server config | `configuration#server-config` | `internal/config/config.go` (server block); `docs/story.md` "Sharing and scope", "Profiles and aliases", "Remote servers without a shim"; `docs/configuration.md` "Servers"; `internal/cli/manual.go` `servers` | #1848 | open: a JSON Schema for the file (`pool.*` rows are generated) |
| 55 | client config | `configuration#client-config` | `docs/configuration.md` "Scope: who reads it"; registry rows with scope `client` and `call` (`internal/settings/registry.go`) | #1848 | the client- and call-scoped settings, filtered from the registry |
| 56 | env vars | `configuration#env-vars` | `docs/configuration.md` "variable" column; `docs/environment.md`, generated from `settings.ScriptEnv()` (`internal/settings/scriptenv.go`) and held to the setters by a test; the names neither source declares, listed below | #1848 | registry names and the script environment both generated; only the names neither declares are hand-written |
| 57 | client modes | `modes#client-modes` | `docs/story.md` "Running without a daemon", "One daemon, many clients"; `docs/exec.md` "Where a script runs"; `AGENTS.md` "Profiles" | #1848 | — |
| 58 | transport options | `modes#transport-options` | `docs/protocol.md` §5; `docs/story.md` "What each transport costs", "Server-sent events, not WebSockets", "Lossless reconnection", "Remote servers without a shim"; `docs/dependencies.md` "Network" | #1848 | — |
| 59 | server modes | `modes#server-modes` | `docs/story.md` "mcpx speaks MCP", "Sharing and scope"; `docs/protocol.md` §5 | #1848 | — |
| 60 | abstract interfaces | `interfaces#abstract-interfaces` | `internal/api/ops*.go`; `docs/parity.md` | #1851 | the parity table |
| 61 | client interfaces | `interfaces#client-interfaces` | `mcpx client`; `internal/codegen`; `plugin/opencode/mcpx/daemon.ts` (`DaemonClient`) | #1851 | — |
| 62 | cli | `cli#cli` | `AGENTS.md` "The loop"; `cmd/mcpx/main.go` | #1851 | — |
| 63 | cli arguments | `cli#cli-arguments` | `cmd/mcpx/main.go`; `internal/settings/bind.go` | #1851 | per-command registry flags |
| 64 | cli commands | `cli#cli-commands` | `internal/cli/manual.go` `Commands()` | #1851 | yes: the whole command table |
| 65 | mcp | `mcp#mcp` | `docs/story.md` "mcpx speaks MCP"; `docs/protocol.md` §2–§4 | #1851 | — |
| 66 | mcp sdk | `mcp#mcp-sdk` | `docs/dependencies.md`; `go.mod` | #1851 | — |
| 67 | mcp compatibility | `mcp#mcp-compatibility` | `docs/protocol.md` §1, §6; `internal/mcpserver/revisions.go`; `GET /v1/protocol` | #1851 | the revision matrix |
| 68 | binary | `binary#binary` | `docs/dependencies.md`; `package.nix` | #1851 | — |
| 69 | script | `scripts#script` | `AGENTS.md` "Scripts", "Script contract"; `docs/story.md` "The script contract", "Named scripts"; `docs/exec.md` | #1851 | — |
| 70 | /v1 | `http-api#v1` | `GET /v1/openapi.json` (`internal/api/openapi.go`); `docs/parity.md` | #1851 | yes: rendered from the OpenAPI document |
| 71 | /v1 sdk | `http-api#v1-sdk` | `DaemonClient` (`plugin/opencode/mcpx/daemon.ts`); `/v1/openapi.json` | #1851 | — |
| 72 | plugins | `plugins#plugins` | `plugin/opencode/README.md` | #1855 | — |
| 73 | plugin compatibility | `plugins#plugin-compatibility` | `docs/opencode-plugin.md` §1, §5 | #1855 | — |
| 74 | plugin interface & config | `plugins#plugin-interface-and-config` | `plugin/opencode/README.md` "Configuration"; `docs/plugin-discovery.md`; `plugin.*` registry rows | #1855 | `plugin.*` rows |
| 75 | opencode v1 plugin | `plugins#opencode-v1-plugin` | `plugin/opencode/README.md`; `docs/opencode-plugin.md` §2, §3, §6 | #1855 | — |
| 76 | opencode v2 plugin | `plugins#opencode-v2-plugin` | `docs/opencode-plugin.md` §4 | #1855 | — |
| 77 | vibeMcp | `plugins#vibemcp` (provisional; moves once its meaning is known) | nothing in the repo | #1855 | — |
| 78 | "so why'd you get rid of all the mcp and then make one, do you like mcp or not" | `faq#do-you-like-mcp` | `docs/story.md` "# Summary", "mcpx speaks MCP" | #1859 | — |
| 79 | "i mean, opencodemode is right there" | `faq#opencode-code-mode` | `OPENCODE-V2.md` (with its 2026-09-29 corrections); `docs/opencode-plugin.md` §1 | #1859 | — |
| 80 | "is it safe?", no. | `faq#is-it-safe` | `AGENTS.md` "Scripts"; `internal/runner/runner.go` | #1859 | — |
| 81 | "but why would you.." | `faq#why` | `ASSESSMENT.md`; `docs/story.md` opening walk | #1859 | — |
| 82 | "but can i get some examples?" | `faq#examples` | `docs/story.md` walk; `mcpx recipes` (`internal/cli/manual.go`); `Examples` fields in `internal/cli/manual.go`. ⚠ `.mcpx/scripts/`, cited in #1848, does not exist at HEAD — do not cite it as a source until it does | #1859 | per-command examples |

#1855 may split `plugins` into an index plus an `opencode` page. The
anchors stay the same either way.

## Contradiction flags — do not copy these into any page

#1848's Findings name three places where the current docs or
behaviour contradict each other. They are flagged here on the pages
they touch, with their state re-verified at HEAD `5a34fa70`
(2026-10-29):

1. **The file key is `mcpServers`, not `servers`** — STILL TRUE.
   `Config.MCPServers` is `json:"mcpServers"`
   (`internal/config/config.go`), the same object other MCP hosts
   use; "servers" is the CLI command and the `/v1` resource.
   Touches: `configuration#common-config`,
   `configuration#server-config`. Both pages must show both names
   and say which one goes where.
2. **`mcpx init`'s starter config taught a dead `mode` key** —
   RESOLVED at HEAD; do not reintroduce it. When #1848 was written,
   `starterConfig` set `"mode": "shared"` / `"mode": "session"`,
   keys the loader silently drops (`mode` had split into `sharing`
   and `scope`). At HEAD the starter config teaches
   `sharing`/`scope` correctly and a test pins it
   (`TestStarterConfigMeansWhatItSays`, `internal/cli/commands.go`),
   and the top-level help's CONCURRENCY block
   (`internal/cli/root.go`) teaches `sharing`/`scope` too.
   Touches: `configuration#common-config`,
   `configuration#server-config`, `modes#client-modes`,
   `modes#server-modes`. Pages teach `sharing`/`scope`; the `mode`
   key and its `pooled` value must appear nowhere.
3. **The config search path is wider than anything documented** —
   STILL TRUE. `SearchPathFrom` (`internal/config/config.go`):
   `MCPX_CONFIG` alone if set; otherwise `.config/mcpx/config.json`,
   `.mcpx.json`, `.mcpx/config.json` and `.mcp.json` at each
   directory from the working directory up to `/`, then
   `$XDG_CONFIG_HOME/mcpx/config.json`, `~/.config/mcpx/config.json`,
   `~/.mcpx.json` and `/etc/mcpx/config.json`. `.mcp.json` is Claude
   Code's project file, so mcpx picks up a project's Claude Code
   servers — nothing says so in the current docs. Touches:
   `configuration#common-config` (the page must list the path
   exactly, in order).

Two smaller points from #1848 that pages must also get right:

- **Env vars outside the registry** (touches
  `configuration#env-vars`): the registry is not the only declaration
  there is. `settings.ScriptEnv()` (`internal/settings/scriptenv.go`)
  declares the environment mcpx and the plugin hand to a script —
  `MCPX_SESSION`, `MCPX_SESSION_ID`, `MCPX_PARENT_SESSION_ID`,
  `MCPX_EPHEMERAL`, `MCPX_RUN`, `MCPX_INPUT`, `MCPX_ENDPOINT`,
  `MCPX_SOCKET`, `MCPX_ARTIFACTS_LOCAL`, `MCPX_CWD`, `MCPX_PID`,
  `MCPX_ENTRY`, `MCPX_CLIENT`, `MCPX_CONFIG_PATH`,
  `MCPX_SCRIPT_DIRS`, `MCPX_RUNTIME` — and `docs/environment.md` is
  generated from it, with a test holding the two directions together
  (`scriptenv_test.go`). So the caller identity the plugin sets
  (`plugin/opencode/mcpx-session.ts`) and the names the generated
  client reads (`internal/codegen/emit.go`) are already declared, not
  hand-written. What an env-var page must add beyond the two
  generators is the small remainder neither declares — config
  selection (`MCPX_CONFIG`) and harness context — and it should link
  the generated page rather than copy it.
- **Upstream "sse" is not the legacy SSE transport** (touches
  `modes#transport-options`): `POST /v1/servers` accepts
  `transport: stdio|http|sse` (`internal/api/ops_settings.go`), but
  the pool builds only a stdio transport or Streamable HTTP
  (`internal/pool/pool.go`, `internal/mcpclient/http.go`) whose
  replies may arrive as an SSE stream. Separately, the deprecated
  2024-11-05 HTTP+SSE transport does have a client —
  `LegacySSETransport` (`NewLegacySSE`, `internal/mcpclient/sse.go`) —
  so the transport page must document that fallback rather than deny
  it, and say which paths reach it, while still not implying that
  `transport: sse` means an SSE-only server (#1848 open question 5).
