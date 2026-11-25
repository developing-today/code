# MCP servers

Every MCP server on this machine runs under mcpx, the gateway daemon described in
[`services.md`](services.md). This page covers how the servers are wired, how they
authenticate, and an inventory of each one. The inventory states facts as of the
date in its heading; re-run its checks before relying on them.

## How it is wired

- **One list.** `mcpx.config.nix` defines every server and how mcpx pools it.
  `homeUser.nix` (`home.activation.mcpxConfig`) writes it to
  `~/.config/mcpx/config.json` as a real file. See `services.md` for why it is not a
  home-manager link.
- **Absolute commands.** Every stdio server is started by its
  `/run/current-system/sw/bin/...` path. A daemon inherits the PATH of whatever
  started it, and a bare name could fail with "executable file not found" under
  another PATH. A server that runs other programs gets its own `env.PATH`
  (`azure` runs `az`; `kubernetes`' kubeconfig runs `kubelogin`).
- **npm-only servers.** `mcp-remote` and `chrome-devtools-mcp` are built from a
  committed lockfile (`pkgs/npm-mcp-servers`, `importNpmLock`; versions pinned in
  its `package.json`). No package in the lockfile has an install
  script, and `npm rebuild` runs with `--ignore-scripts`. To update, change
  `package.json`, run `npm install --package-lock-only --ignore-scripts` there, and
  rebuild.
- **Prebuilt release servers.** `azure` (`pkgs/azure-mcp`, Microsoft's `azmcp`)
  and `kubernetes` (`pkgs/kubernetes-mcp-server`) are the vendors' per-platform
  binaries, fetched from npm by hash (`hashes.json` in each package says how to
  update). `azmcp` is a self-contained single-file .NET app, so no .NET SDK or
  runtime is installed; nixpkgs has only a 3.0.0 beta of it and no
  kubernetes-mcp-server.
- **Pooling** (`mcpx` block per server):
  - `chrome-devtools`, `chrome-live`, `helium-live`, `safari` and `playwright`
    get one process per agent session (`sharing = "exclusive"`,
    `scope = "session"`); browser state must not be shared.
  - `aws` gets `startTimeout = "180s"`, because `uvx` downloads the proxy on its
    first start.
  - `github` hides `create_repository`, `fork_repository`, `delete_repository`,
    `push_files`, `create_or_update_file`, `delete_file` and `merge_pull_request`;
    `datadog` hides `delete_*`, `execute_code`, `execute_datadog_workflow` and the
    tools that change live monitors, dashboards and workflows (`excludeTools`,
    dezren39/darwin-nix#664). Issue and PR tools stay.
  - Under the default `policy.write=ask`, a tool its server does not mark
    read-only counts as a possible write, and a caller nobody answers (a script,
    an unattended agent) waits `elicit.askTimeout` (45 s) and is refused with
    "may be destructive and was not confirmed". Measured 2026-11-24 from
    `mcpx_exec`: nixos, deepwiki and markitdown calls were refused; fff, codedb,
    codebase-memory, context7, terraform, aws-knowledge and atlassian calls went
    through. Their read-only tools are now declared in `readOnlyTools` (exact
    names), and `nixos.nix` answers in seconds. `azure` has no annotations at all
    and refuses writes itself (`--read-only`), so it gets `writePolicy = "allow"`
    instead.
- **Only the launchd agent starts a daemon** (`daemon.autostart = false`). See
  `services.md`.
- **Smoke test.** `just mcpx-check` lists every server's state and calls one tool
  each on codedb, fff, chrome-devtools, context7, nixos and kubernetes (azure
  needs `az login`, so the recipe prints the command instead).

## Scripts the daemon runs

OpenCode's `mcpx_exec` sends a script to the daemon (`POST /v1/exec` on its socket),
so it runs as the daemon does: as this user, in the agent's `WorkingDirectory`
(`$HOME`), with the agent's environment (`SHELL=/bin/zsh`, the launchd `TMPDIR`)
and the agent's PATH plus mcpx's own git, deno, bun and node. `mcpx exec` and
`mcpx run` from a shell run the script locally instead, in the caller's directory
with the caller's environment (`script.cwd = caller`). Measured 2026-11-24.

The agent's PATH (`services.mcpx.environment.PATH`, `configuration.nix`) used to
end at `/usr/bin:/bin`, so a daemon-run script could not spawn `gh`, `az`,
`kubectl` or `nix` (`entity not found`), and found Apple's `jq` and `git`. It now
appends `/usr/sbin`, `/sbin`, the system and per-user Nix profiles, the default
Nix profile, `/opt/homebrew/bin` and `~/.local/bin`, after `/usr/bin:/bin`, so no
name that resolved before resolves differently. That reaches launchd at the next
`just switch`; `just mcpx-exec-env` prints what a daemon-run script sees.

The HTTP API accepts per-run `options.cwd` and `options.env` too. Tested against
the running daemon without a restart: supplying this worktree as `options.cwd`
and the built plist's PATH as `options.env.PATH` ran as the same user in that
worktree and found `gh`. The native `mcpx_exec` tool only accepts `source` and
does not forward the caller's working directory; no global default can infer a
different directory for each caller. Use `mcpx run` from the intended directory,
`Deno.Command(..., { cwd: "/explicit/path" })`, or the supported HTTP options.
`SHELL=/bin/zsh` is just an environment value: `Deno.Command` executes the named
program directly and does not source interactive/login shell startup files.

`tools.comms` (cross-session messaging) is not available in these scripts,
although `mcpx_discover` and the `mcpx_exec` description say it is: it belongs to
OpenCode's messaging plugin, and the daemon has no such namespace. Use it from
OpenCode's Code Mode (`execute`).

## OAuth for the remote servers

atlassian, context7 and datadog run through `mcp-remote`, which does the OAuth
sign-in in a browser and stores tokens itself:

- **Where tokens live:** `~/.mcp-auth/mcp-remote-v1/`, in files named after an MD5
  hash of the server URL (`<hash>_tokens.json`, `_client_info.json` and so on).
  `v1` is mcp-remote's `CONFIG_STORE_VERSION`. `MCP_REMOTE_CONFIG_DIR` moves the
  base directory.
- **The URL is the identity.** Changing a server's URL, including its query
  string (Datadog's `toolsets` and `omit_tools`), needs a new sign-in.
- **Signing in:** `just mcp-login <name>`, and press Ctrl-C once it reports it is
  connected. Context7's sign-in is optional and only raises its rate limit
  (`just context7-login`).
- **Old tokens:** token files for two earlier Datadog URLs are still in that
  directory. They are expired and unused.

github uses `gh`'s token: the `github-mcp` wrapper in `systemPackages.nix` reads
`gh auth token` from the login keychain at start, so no token is stored in the
repo or the nix store.

The mcpx auth block for github also accepts an identity **failover list**
(#7948, `pkgs/mcpx/internal/mcpauth`): an ordered list of identities, each
named by secret name only -- GitHub App identities (App ID + installation ID
in config; the PEM private key lives in the secret store and mcpx mints
short-lived installation tokens from it) with the single user token last as
the fallback. A call that hits a soft block (HTTP 429, or a 403 carrying
rate-limit markers such as the secondary-limit "submitted too quickly")
rotates to the next identity; a hard block (401, a marker-less 403, 404, 422)
surfaces immediately and never rotates. Rotation state (current identity +
cooldowns) persists in the configured state file across server restarts, and
every rotation is logged with the identity name and the soft-block reason.
The pool resolves the current identity's credential at connection start; since
#8017 the daemon call path also runs each tools/call through the rotator: an
error result carrying a soft block is translated to an APIError, the identity
rotates, the github instance is respawned on the rotated credential, and the
call is retried, so the calling session never sees the soft block. Hard blocks
surface unchanged. The session-facing tool surface is unchanged. This Mac's
`github` server has no auth block (checked 2026-11-24 in `mcpx.config.nix`), so
it runs on the one `gh` token above and never rotates.

aws signs requests (SigV4) using `~/.aws/config` profiles
(with `--skip-auth` so its 8 tools load even when logged out, and `--profile`
populated dynamically from `aws configure list-profiles` so callers can switch
accounts per call via the `aws_profile` tool parameter).

azure and kubernetes use the Azure CLI's login (`az login`; account
Drewry.Pope@Vertexinc.Com on 2026-11-24). `azure` sets
`AZURE_TOKEN_CREDENTIALS=AzureCliCredential`, which tries that credential alone,
so a daemon nobody watches never falls back to opening a browser; it also sets
`AZURE_MCP_COLLECT_TELEMETRY=false`. `kubernetes` reads `~/.kube/config`, whose
two AKS users (corptoolsdev01 in Central US, corptoolsprd01 in East US 2) run
`kubelogin get-token --login azurecli`. Their API servers are private
(corptoolsdev01 resolves to 10.253.52.4), so cluster calls need the VPN: on
2026-11-24, off VPN, `configuration_contexts_list` answered and `namespaces_list`
timed out exactly as `kubectl get ns` did.

## Inventory (2026-10-20)

Tool counts are from `mcpx ls` on 2026-10-20. Credential details come from token
file metadata, `gh auth status`, and `aws configure list-profiles`. No secret
values were read for this page.

| Server              | What it does                                                              | Transport                                                                                                                           | Auth                                                                                                                                                             | Credential (2026-10-20)                                                                                                                                       | Tools | Notable risks                                                                                                                                                                                            |
| ------------------- | ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| atlassian           | Jira, Confluence, Loom                                                    | `mcp-remote` → `mcp.atlassian.com/v2/mcp?tools=all` (`--static-oauth-client-metadata` pinning 20 read/search scopes)                | OAuth                                                                                                                                                            | Present; access token refreshed (8h TTL), 90-day refresh token (`exp` 2027-01-18); scope pinned so `.well-known` changes or timeouts never wipe `tokens.json` | 108   | Read/search only (pinning the 20 granted scopes hides the 82 ungranted write/delete tools so calls never trigger 403 step-up browser re-auth)                                                            |
| aws                 | AWS APIs & docs                                                           | `uvx mcp-proxy-for-aws-cli@1.7.0` → `aws-mcp.us-east-1.api.aws/mcp` (`--skip-auth`, `--profile` from `aws configure list-profiles`) | SigV4, per-call `aws_profile` switching                                                                                                                          | `~/.aws/config` role-switch profiles (`source_profile = mfa`); sign in via `aws login` / `aws-login-auto`                                                     | 8     | `aws___run_script` can execute AWS API calls in whichever profile is passed in `aws_profile` (use `-read-only` profiles for read-only work)                                                              |
| aws-knowledge       | AWS documentation                                                         | HTTP `knowledge-mcp.global.api.aws`                                                                                                 | None                                                                                                                                                             | —                                                                                                                                                             | 5     | —                                                                                                                                                                                                        |
| chrome-devtools     | Headless Chrome, one per session                                          | stdio `chrome-devtools-mcp`                                                                                                         | None                                                                                                                                                             | —                                                                                                                                                             | 27    | `evaluate_script`, `upload_file`                                                                                                                                                                         |
| chrome-live         | Live user Chrome profile (`--autoConnect` via `Local State` toggle)       | stdio `chrome-devtools-mcp --autoConnect`                                                                                           | Chrome per-connection approval prompt                                                                                                                            | `devtools.remote_debugging.user-enabled = true` asserted in `Local State` by `home.activation.browserRemoteDebug`                                             | 27    | Full access to signed-in Chrome tabs (`evaluate_script`, `upload_file`) once user clicks Allow                                                                                                           |
| helium-live         | Helium CDP session via the `helium-cdp` launcher on loopback CDP `:19222` | stdio `chrome-devtools-mcp --browserUrl=http://127.0.0.1:19222`                                                                     | Loopback (`helium-cdp` launcher only; a normally launched Helium serves approval-gated CDP on `:9222` via the `Local State` toggle, the DevToolsActivePort path) | —                                                                                                                                                             | 27    | Full access to the tabs of the Helium session the launcher started (`evaluate_script`, `upload_file`)                                                                                                    |
| codebase-memory     | Code call graph and impact                                                | stdio `codebase-memory-mcp`                                                                                                         | None                                                                                                                                                             | —                                                                                                                                                             | 17    | `delete_project` (local index only)                                                                                                                                                                      |
| codedb              | Code search (symbols, callers)                                            | stdio `codedb mcp`                                                                                                                  | None                                                                                                                                                             | —                                                                                                                                                             | 5     | —                                                                                                                                                                                                        |
| context7            | Library documentation                                                     | `mcp-remote` → `mcp.context7.com/mcp`                                                                                               | OAuth, optional                                                                                                                                                  | Not signed in; runs anonymously at a lower rate limit                                                                                                         | 2     | —                                                                                                                                                                                                        |
| datadog             | Logs, metrics, traces, monitors, dashboards, notebooks, workflows (US1)   | `mcp-remote` → `mcp.datadoghq.com/v1/mcp?toolsets=core,workflows,alerting&omit_tools=delete_datadog_workflow`                       | OAuth                                                                                                                                                            | Present, refreshed that day                                                                                                                                   | 63    | `execute_datadog_workflow` (does whatever the workflow does), `update_datadog_monitor`, `upsert_datadog_dashboard`, `delete_datadog_notebook`; the token carries 173 scopes, including `api_keys_delete` |
| deepwiki            | Wikis for public GitHub repos                                             | HTTP `mcp.deepwiki.com/mcp`                                                                                                         | None                                                                                                                                                             | —                                                                                                                                                             | 3     | —                                                                                                                                                                                                        |
| fff                 | File and content search over `~/git`                                      | stdio `fff-mcp`                                                                                                                     | None                                                                                                                                                             | —                                                                                                                                                             | 3     | —                                                                                                                                                                                                        |
| fff-nix             | Same, over this repo                                                      | stdio `fff-mcp`                                                                                                                     | None                                                                                                                                                             | —                                                                                                                                                             | 3     | —                                                                                                                                                                                                        |
| fff-worktree        | Same, over `~/.local/share/opencode/worktree`                             | stdio `fff-mcp`                                                                                                                     | None                                                                                                                                                             | —                                                                                                                                                             | 3     | —                                                                                                                                                                                                        |
| github              | GitHub                                                                    | stdio `github-mcp` → `github-mcp-server stdio`                                                                                      | `gh` token (account dezren39; scopes `repo`, `workflow`, `gist`, `read:org`, `admin:public_key`, `admin:ssh_signing_key`)                                        | Present                                                                                                                                                       | 43    | `push_files`, `delete_file`, `merge_pull_request`, issue and PR writes (#371). With `workflow` scope a push can change Actions workflows                                                                 |
| markitdown          | Converts a URL or file to Markdown                                        | stdio `markitdown-mcp`                                                                                                              | None                                                                                                                                                             | —                                                                                                                                                             | 1     | Can read local `file:` URIs                                                                                                                                                                              |
| nixos               | Nix packages, options, NixOS wiki                                         | stdio `mcp-nixos`                                                                                                                   | None                                                                                                                                                             | —                                                                                                                                                             | 2     | —                                                                                                                                                                                                        |
| playwright          | Headless browser, one per session                                         | stdio `playwright-mcp`                                                                                                              | None                                                                                                                                                             | —                                                                                                                                                             | 25    | `browser_run_code_unsafe`, `browser_file_upload`                                                                                                                                                         |
| safari              | Live user Safari session (Apple native MCP)                               | stdio `/usr/bin/safaridriver --mcp`                                                                                                 | `safaridriver --enable` (asserted in `postActivation`) + one-time Safari > Settings > Developer > Allow remote automation                                        | —                                                                                                                                                             | 17    | Full access to Safari tabs (`evaluate_javascript`, `page_interactions`)                                                                                                                                  |
| sequential-thinking | Step-by-step reasoning scratchpad                                         | stdio `mcp-server-sequential-thinking`                                                                                              | None                                                                                                                                                             | —                                                                                                                                                             | 1     | —                                                                                                                                                                                                        |
| terraform           | Public Terraform registry lookups                                         | stdio `terraform-mcp-server stdio --toolsets=registry`                                                                              | None                                                                                                                                                             | —                                                                                                                                                             | 9     | —                                                                                                                                                                                                        |

Configured 2026-11-24 (tool counts from the servers' own `tools/list` that day).
Both packages and the full system built successfully; temporary daemon
configuration proved Azure authentication and Kubernetes context discovery,
then the original live config was restored. The definitions below become live
at `just switch` (needs sudo). The offline check
`nix build .#checks.aarch64-darwin.agent-tooling` verifies initialization and
Kubernetes Secret refusal against a fake API without cloud credentials or VPN:

| Server     | What it does                                            | Transport                                                    | Auth                                               | Credential (2026-11-24)                        | Tools | Notable risks                                                                                                                                                                                             |
| ---------- | ------------------------------------------------------- | ------------------------------------------------------------ | -------------------------------------------------- | ---------------------------------------------- | ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| azure      | Azure resources and services (Microsoft's server)       | stdio `azmcp server start --read-only` (3.0.0)               | Azure CLI login only                               | `az login` present (Drewry.Pope@Vertexinc.Com) | 67    | One tool per Azure service, unannotated; the server refuses writes itself (`--read-only`). Reads whatever the signed-in account may read; Key Vault secret reads ask first (the server's own elicitation) |
| kubernetes | Kubernetes API for the AKS clusters in `~/.kube/config` | stdio `kubernetes-mcp-server --config …/config.toml` (0.1.3) | kubelogin (`--login azurecli`) per kubeconfig user | Same `az login`; the clusters need the VPN     | 17    | Read-only toolsets (config, core, helm) and Secrets refused by the server's config; it can still read logs and every other object the account may read, in production too                                 |

`elicit.confirmDestructive` stays off, but that does **not** disable mcpx's
separate default `policy.write=ask`: a possible write with nobody to answer is
refused after the deadline, not run. The old claim here that it proceeds after
waiting was wrong for the installed daemon. Destructive GitHub/Datadog tools
are also hidden (above); the 2026-10-20 inventory is historical, not a current
allowlist. Re-discover the tool surface before using it.

## Other MCP clients: mcpx only (2026-10-22)

Every other MCP client gets one MCP server, mcpx, as
`/run/current-system/sw/bin/mcpx --config ~/.config/mcpx/config.json serve`
(stdio). `mcp-clients.py`, run by home-manager on every switch
(`home.activation.mcpClients`), sets it and changes nothing else in each file.
The first time it changes a file, it keeps a copy in
`~/.local/state/mcp-clients/backup/`. The `--config` matters: without it mcpx
searches upward from the client's working directory, a repository's
`.mcp.json` changes which daemon it looks for, and with autostart off none is
there.

A client sees mcpx's gateway, not the 18 servers' tools by name: about 80
`mcpx_*` tools (`mcpx_search`, `mcpx_types`, `mcpx_call`, `mcpx_exec`, ...).
`mcpx_call {namespace, tool, arguments}` reaches any server; namespaces use `_`
for `-` (`chrome_devtools`). The daemon also serves MCP over HTTP at `/mcp` on
its loopback port, but that port is chosen at random on each start, so the
clients use stdio.

| Client      | Installed      | Config                                            | Before                                                                                               | Now                                                                                                                                                                                                        |
| ----------- | -------------- | ------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Copilot CLI | yes (Homebrew) | `~/.copilot/mcp-config.json`                      | codedb (`~/bin/codedb`), deepwiki, plus its built-in GitHub server                                   | mcpx. The built-in server has no config key; the `copilot` shell alias passes `--disable-builtin-mcps`. Checked: `copilot --model gpt-6-luna -p` called `mcpx_namespaces` through it and got 18 namespaces |
| Zed         | yes            | `~/.config/zed/settings.json` (`context_servers`) | GitHub, Context7 and Postgres extensions (Postgres had no `database_url` and errored at every start) | mcpx. Zed started `mcpx ... serve` itself on the settings change. The three extensions were moved to `~/.local/state/mcp-clients/backup/zed-extensions/`, and Zed unloaded them                            |
| Claude Code | no             | `~/.claude.json` (`mcpServers`)                   | codedb, deepwiki                                                                                     | mcpx                                                                                                                                                                                                       |
| Codex       | no             | `~/.codex/config.toml` (`[mcp_servers.*]`)        | codedb, deepwiki                                                                                     | mcpx                                                                                                                                                                                                       |
| OpenCode    | yes            | none needed                                       | the fork's built-in mcpx plugin                                                                      | unchanged; an `mcp` entry would list the tools twice                                                                                                                                                       |
| OpenChamber | yes            | none                                              | drives OpenCode                                                                                      | unchanged                                                                                                                                                                                                  |

No config was found for VS Code (application data left, app removed), Cursor (a
dangling `/usr/local/bin/cursor` link), Windsurf, Claude Desktop or Gemini CLI.
A client added later needs an entry in `mcp-clients.py`.

## Tool namespaces that are not MCP servers

Agents in OpenCode also see these plugins:

- **`github`:** built into the OpenCode fork; uses the `gh` token. It lists 45 tools
  and hides 33 administrative ones, such as `secret_set`, which the model can turn
  on itself. Its `api` tool accepts any HTTP method. Write actions are allowed by
  default (#371).
- **`opencode` and `mcpx_*`:** built in. The `mcpx_*` tools talk to the mcpx daemon.
- **`tokenscope`:** not a plugin — a command, installed from
  `commands/tokenscope.md` by `./symlink-commands`; it invokes the tokenscope
  tool.
- **`openchamber*`:** four tools, added through OpenChamber's
  `opencode.managed.json`.
