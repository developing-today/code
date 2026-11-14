# OpenClaw on the laptop (`amd`)

OpenClaw is Drew's always-on, local-first AI gateway — a peer intelligence
source in the fleet next to Muse (Archer) and Aster (Dot). It is installed
**declaratively** through this flake via
[nix-openclaw](https://github.com/openclaw/nix-openclaw) (pinned in
`flake.lock`), as a Home Manager module for user `user`.

- **Service:** systemd user unit `openclaw-gateway` (linger is on, so it
  starts at boot). Logs: `journalctl --user -u openclaw-gateway -f`.
- **Control UI / WebChat:** `http://amd-6.raptor-pumpkinseed.ts.net:18789/`
  (tailnet bind, token auth). Token: `/run/secrets/openclaw/gateway-token`
  (0600, machine-local, never in git or the Nix store).
- **Config:** `programs.openclaw` in `default.nix` here; rendered to
  `~/.openclaw/openclaw.json` at activation. Nix mode is on: change the
  Nix config and re-apply — do not use `openclaw plugins install` etc.
- **Workspace/knowledge pack:** `workspace/*.md` here, materialized into
  `~/.openclaw/workspace/` (AGENTS/SOUL/TOOLS/IDENTITY/USER/HEARTBEAT are
  the bootstrap files; FLEET/QUOTAS/CHANNELS are reference files).
- **Models:** the OpenCode server facade (127.0.0.1:13420) is this
  gateway's only model route: the default is the opencode-route
  workhorse pool, with the specialist and speed pools as fallbacks.
  The local plan proxies this gateway once fronted directly are
  retired — CommandCode (:18748) and Claude Code (:18749) in the
  2026-11-06 proxy-infra decommission (both providers are native in
  the fork since 2026-11-05), Design (:18746) on 2026-11-03 and
  Antigravity (:18743) on 2026-10-30 and Codex (:18744) on
  2026-11-06 (replaced by the fork's native openai provider).
  See `workspace/QUOTAS.md`.
- **Coding:** ACP via the acpx plugin. The default ACP agent is
  `opencode` (spawn with cwd `~/code`) — the same OpenCode
  build/config/auth OpenChamber drives; `~/.acpx/config.json` (managed in
  `default.nix`) points the standalone acpx CLI at the fork binary, and
  `plugins.entries.acpx.config.agents.opencode` does the same for the
  embedded runtime (without that override acpx defaults to
  `npx -y opencode-ai acp` — upstream v1, which rejects the fleet's v2
  config). OpenCode is currently the ONLY allowed ACP agent: the acpx
  startup check initializes every allowed agent, and the other harnesses
  resolve to npx-fetched adapters. Note: predeclared
  `agents.entries.*.runtime` ACP agents are NOT used — nix-openclaw's
  pinned schema types `runtime` as a oneOf of submodules, which rejects
  the `acp` branch (minimal repro verified 2026-10-26); on-demand ACP
  spawn via the top-level `acp` config is unaffected.
- **Plugins:** bundled tool plugins (summarize, gogcli, goplaces,
  sag, camsnap, discrawl, wacrawl, qmd) and runtime
  plugins (acpx, slack, discord, whatsapp, matrix, signal, codex,
  copilot, opencode, deepseek, meta, brave, tavily, exa, firecrawl,
  perplexity, duckduckgo, tokenjuice, memory-lancedb, memory-wiki) are packaged by Nix. Chat channels
  stay inert until their one-time external setup — see
  `workspace/CHANNELS.md`.

## Apply

User-level (no sudo), from a checkout of this repo:

```bash
nix build .#nixosConfigurations.amd.config.home-manager.users.user.home.activationPackage
./result/activate
```

Canonical (Drew, sudo): `sudo nixos-rebuild switch --flake ~/code#amd`.

## Verify

```bash
systemctl --user status openclaw-gateway
openclaw status
openclaw doctor
openclaw agent --message "Who are you, who do you work with, and what is your default model?"
```

## Comment loop watcher (production, #9818)

The `pilot-loop:v1` comment loop piloted on issue #1190 runs in
production as a **zero-token pre-filter**: an OpenClaw cron
**command payload** (shell on the Gateway host, no model call)
polls one designated issue every 2 minutes and wakes an agent
turn only when a new unanswered round appears. Idle polls print
the cron silent token `NO_REPLY` and cost ~0 agent tokens (the
pilot's every-2-minute LLM poll cost 39k–81k tokens per idle
poll and was killed on cost).

- **Designated issue:** developing-today/code#9818. To re-point
  the loop, recreate the cron job with a different issue number
  as the script's argument (or set it as the command argument).
- **Script:** `comment-loop/pilot-loop-prefilter.sh` here is the
  source of truth; the deployed copy runs from
  `~/.local/share/openclaw/comment-loop/pilot-loop-prefilter.sh`
  (state in `~/.local/state/openclaw/comment-loop/`). Tests:
  `comment-loop/pilot-loop-prefilter.test.sh` (stubbed gh /
  openclaw / systemctl; also run in CI's basic tier).
- **Cron job:** `comment-loop-prefilter`, `--every 2m`,
  `--command "<deployed script> 9818"`, `--timeout-seconds 1000`.
  Manage with `openclaw cron list|disable|rm` (the CLI needs
  `OPENCLAW_GATEWAY_TOKEN` sourced from
  `/run/secrets/openclaw/gateway-token` in a bare shell).
- **Wake:** the filter derives answered state from the issue
  itself (a round `R<n>` is answered by any `REPLY: R<n>`
  comment) and rate-limits re-wakes of a still-unanswered round
  to one attempt per 15 minutes. The wake is
  `openclaw agent --agent main --session-key
  agent:main:comment-loop-9818-r<n>-<wake-epoch> --timeout 900`
  — a FRESH session per wake (a persistent watcher session
  accumulates bootstrap + transcripts until turns die on
  context overflow; the round state lives on the issue, not in
  the session). The 900s turn timeout is the production floor
  (the script clamps any configured timeout to >= 600s; the
  pilot's first turn died at the 150s default on a cold route).
- **quota_left:** the filter reads the quota gatherer's snapshot
  (`~/.local/state/pm-aggregator/quota-current.json`, refreshed
  by `quota-gatherer.timer`) and injects a one-line summary —
  tightest measured 5h window across healthy providers, with its
  measurement time — into the wake message; the agent turn
  copies it verbatim into the SESSION-REPORT, so the field is a
  real measured value, never `unknown` while the gatherer runs.
- **Gateway resilience:** the gateway unit is WantedBy
  `default.target` (see the Install block in `default.nix`) and
  runs with `Restart=always`; if the filter still finds the
  gateway inactive it performs the documented start step
  (`systemctl --user start openclaw-gateway.service`) and
  surfaces a non-zero failure in cron run history when that does
  not recover it.

### Verify tool plugins (bundled + custom) — NOT `openclaw plugins list`

`openclaw plugins list` shows **runtime plugins only** (channels, harnesses,
providers, search). Tool plugins — the bundled ones (gogcli, goplaces, qmd,
sag, camsnap, discrawl, wacrawl) and custom ones (the vendored summarize
plugin) — never appear in that list, in either the enabled or the disabled
section. Their absence there is expected, not a registration failure
(this cost a diagnosis cycle once: issue #6347).

A tool plugin is registered when both of these hold in the **deployed**
system:

1. Its package is on the gateway's PATH — the `openclaw-gateway-default`
   wrapper script run by the `openclaw-gateway` unit exports a PATH with
   one store path per tool package (e.g. `...-summarize-0.21.14/bin`).
2. Its skill is in the rendered config's skill load paths:
   `~/.openclaw/openclaw.json` lists
   `~/.local/share/nix-openclaw/skills/default/<name>`, and that directory
   contains the plugin's `SKILL.md`.

Then smoke-test the binary from its store path, e.g. for summarize:
`<store-path>/bin/summarize --version` and
`<store-path>/bin/summarize --extract https://example.com`.
Two gotchas: the binaries are on the gateway's PATH only, not the login
shell's (`which summarize` fails interactively — that is normal); and the
first-ever `--extract` is slow (a minute or more) because the
HTML-to-Markdown step shells out to `uvx --from markitdown[all]`, which
downloads its packages on a cold cache before falling back to readability
extraction — later runs take seconds. A full LLM summarize additionally
needs a provider API key in the gateway's environment; `--extract` does
not.
