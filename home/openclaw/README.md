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
- **Models:** local plan proxies only — primary DeepSeek V4 Flash via the
  CommandCode proxy (:18748), fallbacks on the Design (:18746) and
  Antigravity (:18743) plans. The Codex/OpenAI proxy (:18744) is the final
  reserve and is deliberately not an automatic fallback. See
  `workspace/QUOTAS.md`.
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
  perplexity, duckduckgo, tokenjuice) are packaged by Nix. Chat channels
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
