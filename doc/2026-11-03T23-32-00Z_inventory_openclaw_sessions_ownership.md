# Inventory: OpenClaw sessions and ownership (2026-11-03)

Issue: #2302 (parent #2023, closed). Compiled 2026-11-03T23:32Z by queue
task 8905 (lane code-j) from **read-only metadata only**:

- `~/.openclaw/agents/main/agent/openclaw-agent.sqlite` — `session_nodes`
  structural columns (key, label, display name, category, status,
  created_via, timestamps). Transcript content (`entry_json`) was **not**
  read.
- `~/.openclaw/state/openclaw.sqlite` — `cron_jobs` and `acp_sessions`
  metadata.
- `~/.openclaw/openclaw.json` — key-level inspection only (channel names,
  plugin entry names, memory slot, gateway mode/port, ACP allowlist, MCP
  server names). No credential values were read or are recorded here.
- `~/.openclaw/workspace/FLEET.md` and `CHANNELS.md` — stated role and
  channel documentation, used as context and checked against the observed
  configuration.
- `systemctl --user is-active openclaw-gateway` → `active`.

Observed facts and inference are kept separate: anything not directly in
the metadata above is marked _(inference)_ or **unknown**.

## 1. Sessions (`session_nodes`, agent `main`)

17 session nodes, **all non-archived** (0 archived). Status counts:
`done` 14, `failed` 1, status NULL 2. `created_via` counts: `run` 11,
`operator` 3, `channel` 1, `cron` 1, NULL 1. **No node carries status
`running`** — see the limitation in §5.

| Session key (truncated)                   | Name / label                          | Status | Via      | Created (UTC)          | Last activity (UTC) | Apparent responsibility                                                                                     |
| ----------------------------------------- | ------------------------------------- | ------ | -------- | ---------------------- | ------------------- | ----------------------------------------------------------------------------------------------------------- |
| `agent:main:main`                         | —                                     | failed | run      | 2026-10-26 22:54       | 2026-10-31 21:08    | The main session node. Current live-main status **unknown** (§5).                                           |
| `agent:main:main:heartbeat`               | —                                     | done   | —        | —                      | —                   | Heartbeat bookkeeping node for the `heartbeat-main` job (§4).                                               |
| `agent:main:slack:direct:<redacted>`      | —                                     | done   | channel  | 2026-11-03 15:37       | —                   | A Slack direct-message session (user id redacted).                                                          |
| `agent:main:cron:90183259-…`              | Automation: Memory Dreaming Promotion | —      | cron     | 2026-10-27 08:00       | —                   | Session node for the promotion cron job (§4).                                                               |
| `agent:main:acp-bridge:5b5899aa-…`        | —                                     | done   | operator | 2026-10-26 23:24       | 2026-10-26 23:25    | ACP bridge session; unnamed, purpose beyond the bridge role **unknown**.                                    |
| `agent:main:acp-bridge:49f9b9cc-…`        | —                                     | done   | operator | 2026-10-26 23:14       | 2026-10-26 23:14    | ACP bridge session; unnamed, purpose **unknown**.                                                           |
| `agent:main:acp-bridge:28314078-…`        | —                                     | done   | operator | 2026-10-26 23:04       | 2026-10-26 23:07    | ACP bridge session; unnamed, purpose **unknown**.                                                           |
| `agent:main:smoke-20261029`               | smoke-20261029                        | done   | run      | 2026-10-29 07:30       | 2026-10-29 07:32    | One-off smoke test _(inference from the name)_.                                                             |
| `agent:main:cmp-f1` … `agent:main:cmp-f5` | cmp-f1 … cmp-f5                       | done   | run      | 2026-10-29 09:01–09:07 | within ~1 min each  | Five short comparison runs _(inference from names/timing)_; what was compared is **unknown** from metadata. |
| `agent:main:r2-probe`                     | r2-probe                              | done   | run      | 2026-10-29 09:53       | 2026-10-29 09:56    | One-off probe _(inference from the name)_.                                                                  |
| `agent:main:probe-r2-model`               | probe-r2-model                        | —      | run      | 2026-10-29 09:49       | —                   | One-off model probe _(inference from the name)_; no terminal status recorded.                               |
| `agent:main:r2-dispatch-probe`            | r2-dispatch-probe                     | done   | run      | 2026-10-29 09:56       | 2026-10-29 09:57    | One-off dispatch probe _(inference from the name)_.                                                         |
| `agent:main:spawnprobe-2229`              | spawnprobe-2229                       | done   | run      | 2026-10-29 10:16       | 2026-10-29 10:16    | One-off spawn probe _(inference from the name)_.                                                            |

Ownership: `session_nodes` records owner-actor columns, but no node in
this inventory is given a named human owner beyond the roles above, and
this inventory does not record actor IDs. The standing role assignment
(FLEET.md) is: **OpenClaw is the local, always-on node — chat channels,
monitoring, quick questions, and delegating coding to OpenCode via
ACP.** No per-session delegation to any other named agent appears in the
metadata.

## 2. ACP sessions (`acp_sessions`)

Exactly **1** ACP session: backend `acpx`, agent `opencode`, mode
`persistent`, state `idle`, working directory
`~/.openclaw/workspace`, last activity 2026-10-28 15:50 UTC.

ACP configuration (`openclaw.json`): enabled, `allowedAgents` =
`opencode`, `claude`, `codex`, `agy`. Only the OpenCode session above
exists in `acp_sessions`; the other allowed agents have no recorded ACP
session in this inventory. This is an observation about recorded
sessions, not a wiring gap: `home/openclaw/default.nix` configures ACP
command overrides for all four agents. Claude and Codex are also
available through their native runtime plugins; ACP is their
background-spawn path.

## 3. Channels, plugins, MCP

- **Channels (config):** exactly one channel is configured —
  `slack`, `enabled: true`. Gateway: mode `local`, port `18789`,
  service `active`.
- **Doc-vs-observed drift:** CHANNELS.md still describes Slack as
  "pending Drew's one-time setup". The observed configuration
  (channel enabled, plus the 2026-11-03 Slack direct session node in
  §1) indicates the setup progressed past what CHANNELS.md records.
  This inventory did not send any message, so end-to-end Slack delivery
  is **not verified** — only configuration and session metadata are.
- **Plugin entries (22):** `acpx`, `brave`, `codex`, `copilot`,
  `deepseek`, `diffs`, `discord`, `duckduckgo`, `exa`, `firecrawl`,
  `lobster`, `matrix`, `memory-lancedb`, `meta`, `opencode`,
  `perplexity`, `searxng`, `signal`, `slack`, `tavily`, `tokenjuice`,
  `whatsapp`. A loaded plugin is not an enabled channel: of the
  channel-capable plugins (discord, matrix, signal, whatsapp, slack),
  only Slack has a `channels` entry.
- **MCP servers (direct wiring in `openclaw.json`):** `codedb`,
  `context7`, `desktop-commander`, `fff`. (FLEET.md additionally
  describes the fleet-wide mcpx gateway; that is fleet infrastructure,
  not this node's own session inventory.)

## 4. Recurring jobs (`cron_jobs`)

2 jobs, both `enabled = 1`, owner agent `main`:

| Job                       | Schedule                       | Description (as recorded)                                                                                                                                                                                   |
| ------------------------- | ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Memory Dreaming Promotion | cron `0 3 * * *` (daily 03:00) | `[managed-by=memory-core.short-term-promotion]` Promote weighted short-term recalls into MEMORY.md (limit=10, minScore=0.750, minRecallCount=3, minUniqueQueries=3, recencyHalfLifeDays=14, maxAgeDays=30). |
| heartbeat-main            | every 1,800,000 ms (30 min)    | —                                                                                                                                                                                                           |

**Memory mismatch (reported, not changed):** the promotion job's
description says it is managed by `memory-core`, but the active memory
plugin is `memory-lancedb` — `plugins.slots.memory = "memory-lancedb"`,
`memory-lancedb` is a configured plugin entry, and there is **no**
`memory-core` plugin entry in `openclaw.json`. Either the job
description is stale or the job expects a plugin that is no longer the
configured memory slot; resolving that is a configuration decision, and
this task's scope is read-only, so no change was made.

## 5. Limitations

- `session_nodes.status` holds no `running` row at read time, and the
  main node's recorded status is `failed` (last activity 2026-10-31).
  Whether a main session is _currently_ live cannot be settled from
  this metadata alone; the gateway service itself is `active`. The
  active-main claim is therefore recorded as **unknown**, per #2302's
  instruction not to infer it.
- Probe/comparison sessions (`cmp-*`, `r2-*`, `spawnprobe-*`,
  `smoke-*`) are unnamed beyond their session names; their purposes are
  inference from names and timing only.
- Session responsibilities were not read from transcripts; nothing in
  this doc quotes or summarises any conversation content.
