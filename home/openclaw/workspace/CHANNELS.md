# CHANNELS.md — how Drew reaches this OpenClaw

## Live today

- **Slack** (Developing Today workspace) — LIVE since 2026-10-26. App `OpenClaw` (A0CD7L3S81W), bot **@openclaw** (U0CC72H7KA7), socket mode. Tokens are sops-managed secrets referenced by name (`openclaw.slack-bot-token`, `openclaw.slack-app-token`). DM policy is allowlist — Drew (U0CBX163G8Z) only. Group policy is allowlist with no channels allowlisted for Shed's own sessions yet; the Nov 2026 #alignment binding-verification posts were made through the gateway/operator path by a fleet queue task, not from a Shed session.
- **Telegram** — LIVE. Bundled core channel (ships inside the gateway package, not the runtime-plugin catalog), configured since 2026-11-07: bot **@shed_openclaw_bot** (Shed), polling mode, DM allowlist Drew only. Token is the sops-managed secret `openclaw.telegram-bot-token`. It went live 2026-11-07 (generation 544), then the pinned 2026.10.7 gateway build regressed its plugin load — nix-openclaw stages `dist-runtime` as a symlink to `dist` and the fs-safe loader rejects symlink path components (code#1029). Fixed in home/openclaw packaging by rebuilding the gateway without the `dist-runtime` symlink so discovery uses the canonical `dist/extensions` tree.
- **Control UI / WebChat** — the gateway UI on the tailnet: `http://amd-6.raptor-pumpkinseed.ts.net:18789/` (token auth; token at `/run/secrets/openclaw/gateway-token`). Also the `openclaw` CLI on this laptop (`openclaw agent --message "…"`, `openclaw status`, `openclaw doctor`).
- **ACP coding sessions** from any connected surface: `/acp spawn opencode` — OpenCode runs through the ACP-to-HTTP adapter onto the shared `opencode serve` at 127.0.0.1:13420; `agy` runs the system antigravity-acp server. Claude/Codex work via their native runtime plugins. (The claude/codex ACP wrapper scripts currently point into a plugin capture that has been garbage-collected; tracked on GitHub.)

## Plugins loaded, channels not configured

Each of these is packaged and loaded; the channel stays inert until it is configured. Tracked as GitHub issues in developing-today/code.

- **WhatsApp** — plugin loaded; pairing is a QR scan with Drew's phone (`openclaw channels login --channel whatsapp`). Not linked.
- **Discord / Matrix / Signal** — plugins loaded, not configured; enable per-channel when Drew wants them (bot token / homeserver account / signal-cli link respectively).

## Search & research plugins loaded

Brave, Tavily, Exa, Firecrawl, Perplexity, DuckDuckGo, SearXNG — provider keys (where a plugin needs one) are filed in Bitwarden by name and wired via the gateway environment; DuckDuckGo, SearXNG and the built-in web tools work without keys.
