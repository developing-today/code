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
