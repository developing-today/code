# CHANNELS.md — how Drew reaches this OpenClaw

## Live today

- **Control UI / WebChat** — the gateway UI on the tailnet: `http://amd-6.raptor-pumpkinseed.ts.net:18789/` (token auth; token at `/run/secrets/openclaw/gateway-token`). Also the `openclaw` CLI on this laptop (`openclaw agent --message "…"`, `openclaw status`, `openclaw doctor`).
- **ACP coding sessions** from any connected surface: `/acp spawn opencode` (the only ACP harness wired so far; Claude/Codex run via their native runtime plugins).

## Plugins loaded, channels pending Drew's one-time setup

Each of these is packaged and loaded; the channel stays inert until Drew does the one external step. Tracked as GitHub issues in developing-today/code.

- **Slack** (Developing Today workspace) — needs a Slack app (bot + app-level socket token) created in the workspace; tokens filed in Bitwarden by name (`openclaw.slack-bot-token`, `openclaw.slack-app-token`), then referenced from the Nix config. Intended home: a channel alongside Aster, so OpenClaw is a third voice in #alignment-style exchanges.
- **Telegram** — needs a bot from @BotFather + Drew's numeric user ID for `allowFrom`.
- **WhatsApp** — plugin loaded; pairing is a QR scan with Drew's phone (`openclaw channels login --channel whatsapp`).
- **Discord / Matrix / Signal** — plugins loaded; enable per-channel when Drew wants them (bot token / homeserver account / signal-cli link respectively).

## Search & research plugins loaded

Brave, Tavily, Exa, Firecrawl, Perplexity, DuckDuckGo — provider keys (where a plugin needs one) get filed in Bitwarden by name and wired via the gateway environment; DuckDuckGo and the built-in web tools work without keys.
