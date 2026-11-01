# TOOLS.md — this machine (the laptop, hostname `amd`)

This file is managed by Nix (source: `home/openclaw/` in the developing-today/code repo). Edit it there, not in the workspace.

## The fleet on this laptop (all systemd _user_ services; linger is on, so they start at boot)

| Service                | What                                                 | Address                                    |
| ---------------------- | ---------------------------------------------------- | ------------------------------------------ |
| `openclaw-gateway`     | This gateway                                         | tailnet, port 18789 (Control UI + WebChat) |
| `openchamber`          | OpenChamber server (GUI over OpenCode)               | 127.0.0.1:3000, socket-activated           |
| `mcpx`                 | Standing MCP gateway daemon                          | config `~/.config/mcpx/config.json`        |
| `clef-proxy`           | Provider proxy                                       | 127.0.0.1:18742                            |
| `agy-proxy`            | Antigravity proxy (Gemini/Claude models)             | 127.0.0.1:18743                            |
| `codex-proxy`          | ChatGPT/Codex plan proxy (OpenAI — final reserve)    | 127.0.0.1:18744                            |
| `antigravity-ls-proxy` | Antigravity language-server proxy                    | 127.0.0.1:18745                            |
| `design-code-proxy`    | Open Design AMR plan proxy (DeepSeek/Haiku/Luna/GLM) | 127.0.0.1:18746                            |
| `muse-proxy`           | Meta High Usage plan proxy                           | 127.0.0.1:18747                            |
| `commandcode-proxy`    | CommandCode plan proxy (DeepSeek/Kimi/GLM/Qwen)      | 127.0.0.1:18748                            |
| `claude-code-proxy`    | Claude Code proxy                                    | 127.0.0.1:18749                            |

- Check one: `systemctl --user status <name>` · logs: `journalctl --user -u <name> -f`
- OpenCode (the engine under OpenChamber) keeps its live managed-server port in `~/.config/openchamber/managed-opencode/state.json` — it changes on restart; never hardcode it.
- OpenCode config: `~/code/.opencode/opencode.jsonc` (symlinked from the repo). Its provider/proxy wiring is the fleet's model-routing source of truth.

## Coding work = OpenCode (the OpenChamber engine)

For real coding, spawn an ACP session instead of improvising: `/acp spawn opencode`, with cwd `~/code` for repo work. `opencode` is the default and currently the only ACP harness (the acpx startup check initializes every allowed agent; the others resolve to npx-fetched adapters that hang on this laptop's flaky npmjs route). Claude/Codex are available through their native runtime plugins instead. It uses the same OpenCode build, config, auth, route pools and quota discipline as OpenChamber sessions.

## Repos (the fleet of four — see FLEET.md)

- `developing-today/code` — all new code lands here; this laptop's NixOS flake lives at its root (`~/code`).
- `developing-today/opencode` — the OpenCode fork. `developing-today/openchamber` — OpenChamber. `dezren39/nix` — dezren39's nix config.
- Local checkouts: `~/code` (code), `~/git/openchamber-fork`, plus worktrees under `~/work/`.

## Nix rules on this machine

- The system flake is `~/code/flake.nix`; host `amd`. Apply user-level changes with the Home Manager activation package from that flake; full `sudo nixos-rebuild switch --flake ~/code#amd` is Drew's to run (sudo needs his password).
- Never scatter code in $HOME — code goes in developing-today/code (private/sensitive material: dezren39/hatch-personal).
- Secrets are referenced **by name only** (Bitwarden Secrets Manager / sops under `/run/secrets/agent-auth/`). Never write a secret value into a file, commit, chat, or log. This gateway's own token lives at `/run/secrets/openclaw/gateway-token` (0600, not in git, not in the Nix store).
