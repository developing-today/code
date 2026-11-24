# Install

> **Prerequisites, before the table:** a JavaScript runtime — deno
> (recommended; it is the only runtime where typechecking works), bun,
> or node 22.6 or later — unless you install with nix or the container,
> which bring their own. `git` is recommended (without it, the `repo`
> and `worktree` scopes degrade to the working directory). macOS or
> Linux; on Windows, use WSL. When you are done: `mcpx doctor`.

---

## Channels

The [0.1.0 release](https://github.com/developing-today/code/releases/tag/mcpx/v0.1.0)
contains archives for Linux x86-64, Linux ARM64 and macOS ARM64, plus
`SHA256SUMS`. Pick the channel that fits how you operate the daemon.

| Channel            | Command                                                                                                                         | Status                                                                                                                                                                                                                                                                                                                                                                                     |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| nix                | `nix run github:developing-today/code#mcpx -- ls` to try it; `nix profile install github:developing-today/code#mcpx` to keep it | **Available** — the flake builds `pkgs/mcpx/package.nix` and exposes it as an app; the wrapper puts `git`, `deno`, `bun` and `node` on `PATH`, so a nix install is complete with no other prerequisites                                                                                                                                                                                    |
| container          | From the repository root, run `nix build .#packages.x86_64-linux.mcpx-container && docker load < result`.                       | Build the image locally with Bun as its runtime. See [the container chapter](./container.md) for image contents.                                                                                                                                                                                                                                                                           |
| just the plugin    | copy `pkgs/mcpx/plugin/opencode/` into `~/.config/opencode/plugin/` (the plugin README's "Install")                             | **Available** as a manual copy — and it is not an install of mcpx: the plugin needs **no mcpx binary on its machine**, because it finds a daemon over the unix socket or an `endpoint`; it still needs a daemon running somewhere, and as copied it exposes no tools and injects no guidance until you set `tools: true` / `MCPX_PLUGIN_TOOLS=1` (and `instructions: true` for the prompt) |
| shell installer    | Download the script and run the [versioned install](#install-a-release) below.                                                  | Downloads the matching release archive, verifies its checksum and installs the binary. Supply Deno, Bun or Node for `run` and `exec`.                                                                                                                                                                                                                                                      |
| agent prompt       | a prompt you paste to your agent, with the `llms.txt` URL                                                                       | Not yet                                                                                                                                                                                                                                                                                                                                                                                    |
| pwsh (install.ps1) | `irm <short>/install.ps1 \| iex`                                                                                                | **Blocked** — mcpx does not build for Windows today; WSL is the Windows answer                                                                                                                                                                                                                                                                                                             |
| rpm / deb          | `dnf install ./mcpx-<version>.rpm`                                                                                              | Not yet                                                                                                                                                                                                                                                                                                                                                                                    |
| flatpak            | —                                                                                                                               | Not recommended (decided in the release design, #1881)                                                                                                                                                                                                                                                                                                                                     |
| pkg                | —                                                                                                                               | Not yet — and still an open question: macOS `.pkg`, FreeBSD `pkg`, or Homebrew                                                                                                                                                                                                                                                                                                             |
| docker (published) | `docker run --rm ghcr.io/developing-today/code/mcpx:0.1.0 --version`                                                            | The [registry package](https://github.com/orgs/developing-today/packages/container/code%2Fmcpx) has the `0.1.0` and `latest` tags. The [publish run](https://github.com/developing-today/code/actions/runs/43886854966) built the image and pushed both tags.                                                                                                                              |

Nix caveats, stated plainly: the flake is a large monorepo flake, so
installing one package from it fetches and evaluates a lot (the cost
from a cold machine is unmeasured — an open question on #1878), and
`x86_64-darwin` is excluded; aarch64-darwin, x86_64-linux and
aarch64-linux are the supported systems.

## Install a release

Download the [installer at this revision](https://github.com/developing-today/code/blob/d24c19c6e0654f2466fc98bc86e3dacb786f5067/pkgs/mcpx/scripts/install.sh).
Run the following commands in a temporary directory. The first selects the script,
the second selects the release, and the last reads back the installed version.

```sh
curl -fsSLo install-mcpx.sh https://raw.githubusercontent.com/developing-today/code/d24c19c6e0654f2466fc98bc86e3dacb786f5067/pkgs/mcpx/scripts/install.sh
MCPX_VERSION=0.1.0 sh ./install-mcpx.sh
"$HOME/.local/bin/mcpx" --version
```

For a per-host binary, pass `--system` to the script. It requests elevated
privileges for the copy into `/usr/local/bin` if needed. Set `MCPX_PREFIX`
to select another installation directory. Omit `MCPX_VERSION` to let the
script look for the latest stable MCPX release.

## Install as

Choose how the daemon runs once the binary is installed.

| Mode                                       | What to do                                                                                                                                                                                                                                            | Status                                                                                                                                                                             |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Just run the binary                        | Nothing. Any command that needs the daemon starts one (`daemon.autostart`, default true), and an auto-started daemon exits after 4 hours idle (`autostart.idleExit`)                                                                                  | Shipped                                                                                                                                                                            |
| Per-user                                   | Binary in `~/.local/bin`. Configuration in `~/.config/mcpx/config.json`.                                                                                                                                                                              | The installer uses this binary directory by default.                                                                                                                               |
| Per-host                                   | Binary in `/usr/local/bin`. Configuration in `/etc/mcpx/config.json`, the last location on the search path.                                                                                                                                           | The installer accepts `--system`.                                                                                                                                                  |
| Local host (a daemon other machines reach) | `mcpx daemon --address <ip> --port <n>` on that host; clients set `daemon.endpoint` / `MCPX_DAEMON_ENDPOINT` — a client with an endpoint set never autostarts a local daemon. Remember: the API is unauthenticated, the network is the access control | Shipped                                                                                                                                                                            |
| Persistent daemon                          | `mcpx daemon` (foreground) or `mcpx daemon --detached`; a deliberately started daemon's `daemon.idleExit` defaults to `0` — it never idle-exits                                                                                                       | Shipped                                                                                                                                                                            |
| System service                             | Import the appropriate [MCPX Nix module](https://github.com/developing-today/code/tree/d24c19c6e0654f2466fc98bc86e3dacb786f5067/pkgs/mcpx/modules) and enable `services.mcpx`.                                                                        | The NixOS module provides a systemd user unit. Home Manager supports a systemd user unit on Linux and a launchd user agent on macOS. The shell installer installs the binary only. |
| Fallback: no daemon                        | Set `daemon.inline` true: servers run inside each command process. Slower, nothing pooled, nothing shared                                                                                                                                             | Shipped, last resort                                                                                                                                                               |

## Verify

```sh
mcpx doctor
```

`doctor` checks the runtime, git, the configuration chain, whether
every configured server's command is actually installed, and the
daemon — and each failed check says what to do about it.

Then: [the quick start](./quick-start.md).
