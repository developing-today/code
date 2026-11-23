# mcpx + opencode plan (phase-1 findings incorporated 2026-09-30; enable in phase 2 only)

## What mcpx is

Go MCP proxy/gateway living in `dezren39/nix` at `pkgs/mcpx/`. A persistent
**daemon** owns pools of MCP servers; the `mcpx` CLI (`ls`, `search`, `run`,
`exec`, `servers`, `config`, `daemon`, …) is a thin client. The daemon
auto-spawns by default (`daemon.autostart=true`), so the CLI alone is enough to
get it running — systemd is for boot persistence and restart, not for first
launch.

## Import recipe (verified — research clone /tmp/dn-nix)

```nix
inputs.dezren39-nix.url = "github:dezren39/nix";
# → inputs.dezren39-nix.packages.x86_64-linux.mcpx
```

- `flake.nix:385`: `mcpx = pkgs.callPackage ./pkgs/mcpx/package.nix { inherit bun-bin; };`
  exposed via `packages` at `:424`; also `apps.<system>.mcpx` at `:494-497`.
- There is **no `nixosModules` output** in that flake (darwin-only hosts), so there
  is no module to import — package only.
- Build: `buildGoModule`, subpackage `cmd/mcpx`, vendorHash pinned; wrapped with
  `makeBinaryWrapper` pinning deno, bun-bin, nodejs, git on PATH.

## How it runs

- Daemon: `mcpx daemon` (foreground). Network mode:
  `mcpx daemon --address 127.0.0.1 --port 8899`.
- Transport: **unix socket + local HTTP API**. State dir resolution:
  `$MCPX_STATE_DIR` → `$XDG_STATE_HOME/mcpx` → `~/.local/state/mcpx`; socket
  beside it, mode `0600`, state dir `0700`. Per-user daemon is the designed model.
- Config: JSON/JSONC. Search order: `$MCPX_CONFIG` → walk up from cwd
  (`.config/mcpx/config.json`, `.mcpx.json`, `.mcpx/config.json`, `.mcp.json`) →
  `$XDG_CONFIG_HOME/mcpx/config.json` → `~/.config/mcpx/config.json` →
  `~/.mcpx.json` → `/etc/mcpx/config.json`. Files merge, nearest wins per server
  name; `"disabled": true` drops an inherited server.
- Minimal example:
  ```jsonc
  { "mcpServers": {
      "a": { "command": "x", "args": ["1"] },                          // stdio
      "b": { "url": "https://example.com/mcp",
             "auth": { "type": "bearer", "token": "${VAR}" } }          // http
  } }
  ```
  `${VAR}` expansion keeps secrets out of the file; unset vars are reported up
  front and `Describe()` never prints literals.
- Servers: `mcpx servers add <name> -- <cmd> …` writes config; the daemon
  hot-reloads within its reap interval without restarting other servers. Pooled
  stdio child processes or HTTP clients, idle reaping.

## lootbin vs lootbox — OPEN QUESTION

The string "lootbin" appears **nowhere** in `dezren39/nix`, `dezren39/hatch`, or
GitHub code search scoped to owner `dezren39`. The repo's only MCP-serving host
is **lootbox** — Drew's Mac:
- launchd user agent `lootbox` (`configuration.nix:718-755`, `KeepAlive`,
  `RunAtLoad`), running a Deno/Hono server on port 9420.
- Server list in `lootbox.config.json` (`mcpServers`): `codebase-memory`,
  `codedb`, `fff-nix`, `fff-worktree`, `fff`, `chrome-devtools`, `context7`.
  Each spawned as a child stdio process by the lootbox server.
- lootbox ≠ mcpx. Lootbox is a separate Deno app (`pkgs/lootbox-update`); mcpx
  is the Go gateway.

**For Drew:** did you mean lootbox? And is parity "mcpx here with a similar
server list", or do you also want lootbox-the-app ported to this box?

## opencode plugin (`pkgs/mcpx/plugin/opencode/`)

A **session-awareness plugin, not a full client**: tells mcpx which opencode
session is active so MCP servers get leased per session (two agents → separate
processes instead of one corrupted shared instance). Everything else (server
listing, exec, logs, daemon discovery) goes over the daemon's unix socket — no
`mcpx` binary needed on PATH.
Install (per `plugin/opencode/README.md`): copy `mcpx-session.ts` + `mcpx/` dir
→ `~/.config/opencode/plugin/`; skills → `~/.config/opencode/skills/`.

## systemd unit (DRAFT — phase 2)

```ini
# /etc/systemd/system/mcpx.service — WRITTEN BY init.sh EVERY BOOT (/etc is ephemeral)
[Unit]
Description=mcpx MCP gateway daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=hatch
ExecStart=/home/hatch/.nix-profile/bin/mcpx daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Binary path: profile merge landed (b2af047) — `~/.nix-profile` symlinks to
`/nix/var/nix/profiles/per-user/root/profile`, so this is correct once mcpx is
installed via `nix profile`. Since
`daemon.autostart=true`, the CLI would spawn the daemon even without this unit;
the unit buys boot persistence + restart-on-failure + `systemctl` visibility.

## init.sh hook (DRAFT — follows TOOLS.md systemd pattern)

```bash
install_mcpx_service() {
  # /etc is ephemeral: rewrite the unit file on every boot, then enable --now
  cat > /etc/systemd/system/mcpx.service <<'EOF'
<unit contents>
EOF
  systemctl daemon-reload
  systemctl enable --now mcpx.service
}
```

Verification (phase 2): `systemctl stop mcpx` → re-run hook →
`systemctl is-active mcpx` must be active. Recycle check: init.sh runs on boot
via the sandbox-boot-init cron.

## opencode install plan (phase 2 — AFTER B/C/D lands; profile is being merged)

1. `nix profile install nixpkgs#opencode` (NOT in phase 1 — read-only period).
2. Smoke test: `opencode --version`, then try a free model.
3. If free models need interactive auth (`opencode auth login` → browser):
   record it, move on, add `REMIND DREW` to TODO.md. Do not fake it.
4. Install the mcpx opencode plugin (paths above); verify opencode lists mcpx tools.

## Push-back plan (phase 2)

- Service file → `dezren39/nix` on a branch (never force-push main), open PR.
- Feedback file (what we learned / what broke) → same repo, branch/PR.
- Anything broken upstream → GitHub issue → fix → PR. Chain PRs ahead of main.
- Push denied → stay on local branch, keep moving, add `REMIND DREW` to TODO.md.
