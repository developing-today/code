# Install

> **Prerequisites, before the table:** a JavaScript runtime — deno
> (recommended; it is the only runtime where typechecking works), bun,
> or node 22.6 or later — unless you install with nix or the container,
> which bring their own. `git` is recommended (without it, the `repo`
> and `worktree` scopes degrade to the working directory). macOS or
> Linux; on Windows, use WSL. When you are done: `mcpx doctor`.

---

## Channels

The status column is the honest part of this page. Today, **nix, a
locally built container, and the plugin copy are the only channels that
exist**; everything else is designed but not built (that work is
#1881), and this page will not pretend otherwise.

| Channel | Command | Status |
| --- | --- | --- |
| nix | `nix run github:developing-today/code#mcpx -- ls` to try it; `nix profile install github:developing-today/code#mcpx` to keep it | **Available** — the flake builds `pkgs/mcpx/package.nix` and exposes it as an app; the wrapper puts `git`, `deno`, `bun` and `node` on `PATH`, so a nix install is complete with no other prerequisites |
| container | from the repository root: `docker build -f pkgs/mcpx/Containerfile -t mcpx pkgs/mcpx` (or podman), then run the image — see [container](./container.md) | **Partly** — the Containerfile builds a working image locally (bun as its runtime; deliberately no `git`, see [container](./container.md)); no image is published anywhere yet |
| just the plugin | copy `pkgs/mcpx/plugin/opencode/` into `~/.config/opencode/plugin/` (the plugin README's "Install") | **Available** as a manual copy — and it is not an install of mcpx: the plugin needs **no mcpx binary on its machine**, because it finds a daemon over the unix socket or an `endpoint`; it still needs a daemon running somewhere, and as copied it exposes no tools and injects no guidance until you set `tools: true` / `MCPX_PLUGIN_TOOLS=1` (and `instructions: true` for the prompt) |
| bash installer | per-user: `curl -fsSL <short> \| sh`; system-wide: `curl -fsSL <short> \| sh -s -- --system` (fallback: `curl -fsSL <short> \| sudo sh`) | Not yet — there are no releases for it to download |
| agent prompt | a prompt you paste to your agent, with the `llms.txt` URL | Not yet |
| pwsh (install.ps1) | `irm <short>/install.ps1 \| iex` | **Blocked** — mcpx does not build for Windows today; WSL is the Windows answer |
| rpm / deb | `dnf install ./mcpx-<version>.rpm` | Not yet |
| flatpak | — | Not recommended (decided in the release design, #1881) |
| pkg | — | Not yet — and still an open question: macOS `.pkg`, FreeBSD `pkg`, or Homebrew |
| docker (published) | `docker run … ghcr.io/<owner>/mcpx` | Not yet — local builds only, see the container row |

Nix caveats, stated plainly: the flake is a large monorepo flake, so
installing one package from it fetches and evaluates a lot (the cost
from a cold machine is unmeasured — an open question on #1878), and
`x86_64-darwin` is excluded; aarch64-darwin, x86_64-linux and
aarch64-linux are the supported systems.

The system-wide forms above are deliberate. Never
`sudo $(curl <short> | sh)`: that runs the installer unprivileged and
then executes the installer's *stdout* as root. The `--system` form
lets the script escalate only the copy into `/usr/local/bin`.

## Install as

Installing is only half the choice; the other half is how the daemon
lives on the machine. Every row except the service is shipped today.

| Mode | What to do | Status |
| --- | --- | --- |
| Just run the binary | Nothing. Any command that needs the daemon starts one (`daemon.autostart`, default true), and an auto-started daemon exits after 4 hours idle (`autostart.idleExit`) | Shipped |
| Per-user | Binary in `~/.local/bin`; config at `~/.config/mcpx/config.json` | Config shipped; the binary location arrives with the installer (#1881) |
| Per-host | Binary in `/usr/local/bin` (the installer's `--system`); config at `/etc/mcpx/config.json`, the last stop on the config search path | Config shipped but was undocumented until this page; binary location awaits #1881 |
| Local host (a daemon other machines reach) | `mcpx daemon --address <ip> --port <n>` on that host; clients set `daemon.endpoint` / `MCPX_DAEMON_ENDPOINT` — a client with an endpoint set never autostarts a local daemon. Remember: the API is unauthenticated, the network is the access control | Shipped |
| Persistent daemon | `mcpx daemon` (foreground) or `mcpx daemon --detached`; a deliberately started daemon's `daemon.idleExit` defaults to `0` — it never idle-exits | Shipped |
| System service | A launchd plist or systemd unit running `mcpx daemon`, with `daemon.autostart` false on clients so no stray second daemon starts | **Proposed**, not shipped — until then, the on-demand daemon above is the supported path |
| Fallback: no daemon | Set `daemon.inline` true: servers run inside each command process. Slower, nothing pooled, nothing shared | Shipped, last resort |

## Verify

```sh
mcpx doctor
```

`doctor` checks the runtime, git, the configuration chain, whether
every configured server's command is actually installed, and the
daemon — and each failed check says what to do about it.

Then: [the quick start](./quick-start.md).
