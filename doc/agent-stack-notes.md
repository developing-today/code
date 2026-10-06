# Agent stack: opencode, OpenChamber, t3code, Antigravity, Jules

Findings from configuring these tools together, with emphasis on what can be
done declaratively, what must be imperative, and the sharp edges that cost time.
Everything here was verified on this machine rather than taken from docs.

---

## 1. opencode: `plugin` vs `plugins` — the one that silently breaks things

opencode v1 and v2 disagree about the config key, and OpenChamber rewrites it.

Verified with `opencode debug config` against both builds:

| config key  | opencode 1.18.19 | opencode 2.0.23                 |
| ----------- | ---------------- | ------------------------------- |
| `"plugin"`  | loads            | loads (normalises to `plugins`) |
| `"plugins"` | **ignored**      | loads                           |

So `plugin` is the only spelling that works on both. The same applies to
`agent` vs `agents`.

The trap: **OpenChamber rewrites `plugin` → `plugins` on startup.** Its own
source says so (`server/lib/opencode/plugins.js`):

> OpenCode 2 reads `plugins` ... and still decodes the v1 `plugin` array.
> OpenChamber reads both and always writes `plugins` with v2 entries.

That is correct for OpenChamber, which runs opencode 2.x. It silently stops all
plugins loading for a v1 CLI. There is no error; `/models` and plugin features
just quietly do nothing.

Mitigation in this repo: `systemd.user.paths.opencode-config-normalize` watches
the config and converts it back. See `pkgs/opencode-config-normalize/`.

Check with:

```sh
opencode debug config | jq '.plugin | length'
```

Also note the canonical schema at <https://opencode.ai/config.json> lists
`plugin` and `agent` but **not** `plugins`, `agents` or `warming` — OpenChamber
writes keys that are not in opencode's published schema.

## 2. opencode config: scope and precedence

- Global config lives at `~/.config/opencode/opencode.json[c]`.
  **On this machine `~/.config/opencode` is a symlink to `<repo>/.opencode`**,
  so "global" and "project" are literally the same file.
- Precedence (later wins): remote `.well-known/opencode` → global → `OPENCODE_CONFIG`
  → project → `.opencode/` dirs → `OPENCODE_CONFIG_CONTENT` → managed → MDM.
- Configs **merge**, they do not replace. `plugin` arrays effectively union
  across global and project.
- Plugin dirs are plural: `plugins/`, `agents/`, `commands/`, `skills/`,
  `tools/`, `themes/`. Singular forms work for backwards compatibility.
- npm plugins are auto-installed by Bun into `~/.cache/opencode/node_modules/`.

`OPENCODE_CONFIG=/path/to/file` points a single process at a different config —
useful to give OpenChamber its own, if the rewrite ever becomes intolerable.

## 3. opencode providers

opencode sources its catalogue from **models.dev** (226 providers). A provider
does not need a plugin if it is in that catalogue — just name it in `provider`.

Useful ids found:

| id                        | npm                               | env                                                                      |
| ------------------------- | --------------------------------- | ------------------------------------------------------------------------ |
| `google`                  | `@ai-sdk/google`                  | `GEMINI_API_KEY` / `GOOGLE_API_KEY` / `GOOGLE_GENERATIVE_AI_API_KEY`     |
| `google-vertex`           | `@ai-sdk/google-vertex`           | `GOOGLE_VERTEX_PROJECT`, `_LOCATION`, `GOOGLE_APPLICATION_CREDENTIALS`   |
| `google-vertex-anthropic` | `@ai-sdk/google-vertex/anthropic` | same                                                                     |
| `cloudflare-workers-ai`   | `@ai-sdk/openai-compatible`       | `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_API_KEY`                            |
| `cloudflare-ai-gateway`   | `ai-gateway-provider`             | `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_GATEWAY_ID`, `CLOUDFLARE_API_TOKEN` |

Notes:

- There is **no** `gemini` or `google-generative-ai` id. `google` is the direct
  Gemini API.
- Cloudflare Workers AI exposes an OpenAI-compatible endpoint at
  `https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1`, so
  `@ai-sdk/openai-compatible` works as a drop-in for any similar endpoint.
- Some Workers AI models are `tool_call: false` in the catalogue
  (`llama-3.1-8b`, `qwen2.5-coder-32b`, `qwq-32b`). They appear in the picker
  but cannot drive an agent loop.
- Context windows are **per provider, not per model**. `openai/gpt-6-luna` is
  1,050,000 across essentially every provider in models.dev, but
  `databricks/databricks-gpt-5-6-luna` is 400,000 — a real Databricks platform
  limit, not stale metadata. If a context window looks wrong, check which
  provider the model resolved through before blaming the catalogue. The local
  copy is `~/.cache/opencode/models.json`.
- Knobs: per-provider `whitelist` / `blacklist`, top-level
  `disabled_providers` / `enabled_providers` (disabled wins).

Custom provider shape, for anything OpenAI-shaped not in the catalogue:

```jsonc
"provider": {
  "my-provider": {
    "npm": "@ai-sdk/openai-compatible",
    "name": "My Provider",
    "options": { "baseURL": "https://example.com/v1", "apiKey": "{env:MY_KEY}" },
    "models": { "model-id-from-GET-/v1/models": {} }
  }
}
```

## 4. Agent services that are not providers

Jules is an **agent service**, not a chat-completions endpoint — it cannot be an
opencode `provider` and is absent from models.dev. The supported integration is
MCP. Same reasoning applies to any "run a task on my repo" service.

MCP config shape (`McpLocalConfig` requires `type` + `command`):

```jsonc
"mcp": {
  "jules": {
    "type": "local",
    "command": ["npx", "-y", "@google/jules-mcp"],
    "enabled": true,
    "environment": { "JULES_API_KEY": "{env:JULES_API_KEY}" }
  }
}
```

`McpRemoteConfig` takes `type` + `url`, plus `headers` and `oauth`.

## 5. t3code: headless and declarative

t3 is an **orchestrator over other coding agents**, not a peer of opencode. Its
provider drivers (from `apps/server/src/provider/Drivers` on `pingdotgg/t3code`
main): `opencode`, `codex`, `claudeAgent`, `antigravity`, `cursor`, `grok`,
plus `PiDriver` and `AcpRegistryDriver` on main.

Note `claudeAgent` is camelCase — `CLAUDE_DRIVER_KIND = ProviderDriverKind.make("claudeAgent")`.
A `[a-z-]+` grep will miss it.

### Headless

```sh
t3 serve --tailscale-serve --no-browser    # server, no browser, prints pairing info
t3 pair                                    # QR code for phone
t3 auth pairing|session                    # headless auth control plane
t3 service install|status                  # background service (prefer a systemd unit)
```

`--tailscale-serve` is first-class: it configures Tailscale Serve to expose the
backend over HTTPS on the tailnet. No tunnel, no reverse proxy, no public
exposure.

### Declarative provider config — yes, this works

State lives under `$T3CODE_HOME/userdata/`. Settings are
`$T3CODE_HOME/userdata/settings.json`, created on demand. **A hand-written
settings.json is read and preserved** — verified by writing one and restarting
with no parse warnings. Upstream anticipates this ("Bitbucket tokens
hand-edited into settings.json").

```jsonc
{
  "providerInstances": {
    "opencode": { "driver": "opencode", "displayName": "opencode v2", "enabled": true },
    "opencode-v1": { "driver": "opencode", "displayName": "opencode v1", "enabled": true, "config": { "binaryPath": "/nix/store/...-opencode-1.18.19/bin/opencode" } },
    "opencode-v2": { "driver": "opencode", "displayName": "opencode v2", "enabled": true, "config": { "binaryPath": "/nix/store/...-opencode-2.0.23/bin/opencode" } },
    "codex": { "driver": "codex", "enabled": true },
    "claudeAgent": { "driver": "claudeAgent", "enabled": true },
    "antigravity": { "driver": "antigravity", "enabled": true },
  },
}
```

`ProviderInstanceConfig = { driver, displayName?, accentColor?, environment?, enabled?, config? }`.
The default instance id for a driver is the driver kind itself.

### Model lists per instance

Each instance's `config` also takes a **`customModels`** list — the direct
analogue of opencode's `provider.<id>.models`:

```js
CustomModelSetting = Union([
  String,
  Struct({
    slug: TrimmedNonEmptyString,
    name: optional(TrimmedNonEmptyString),
    capabilities: optional(ModelCapabilities),
  }),
]);
customModels: optionalKey(ArraySchema(CustomModelSetting));
```

So either a bare slug string or `{slug, name?, capabilities?}`. Confirmed wired
for the `opencode`, `codex` and `grok` settings. The seed script merges rather
than overwrites, so hand-added `customModels` survive a rebuild.

`binaryPath` is settable for `opencode`, `codex`, `claude` and `grok`
(`makeBinaryPathSetting(...)`) but **not** `antigravity`, which resolves its
binary another way. The built-in default model constant is `gpt-6-astra`.

Drivers whose credentials are absent are better seeded `enabled: false` — an
enabled instance with no auth shows up as a permanently unhealthy provider
rather than an obvious misconfiguration. `pkgs/t3code-seed-providers/seed.sh`
gates `grok` on `~/.grok/auth.json` / `$GROK_AUTH` / `$GROK_API_KEY` for this
reason.

There is also a REST surface at `/api/provider` (GET/POST/PUT/PATCH/DELETE) if
you would rather drive it over HTTP.

Other state: `userdata/state.sqlite`, `userdata/keybindings.json`,
`userdata/model-manifest.json`, `caches/<driver>.json` per driver.

### PATH matters for systemd

t3 discovers agents on `PATH`, and a systemd **user unit does not inherit the
login shell's PATH**. Without an explicit `Environment=PATH=...` every provider
shows as unavailable with no stated cause. See `home/common/default.nix`.

## 6. Version compatibility between the clients

| Client      | opencode requirement                                                                      |
| ----------- | ----------------------------------------------------------------------------------------- |
| t3code      | `MINIMUM_OPENCODE_VERSION = "1.14.19"`, no maximum; classifies `major >= 2 ? "v2" : "v1"` |
| OpenChamber | `>= 2.0.20` **and** `major === 2` exactly — a future 3.x is rejected                      |

Both opencode generations coexist here: the default `opencode` on PATH
matches v2 (2.0.23, required by OpenChamber), while v1 (1.18.19) remains
accessible as `opencode-v1`. OpenChamber needs 2.0.20+ specifically because
that release added `GET /api/credential`.

OpenChamber reads the host's `~/.config/opencode` via `os.homedir()`. The
`SPACE_HOME = '/home/space'` constant is container-internal, for its sandboxed
"spaces" feature — not a host path.

## 7. Secrets

Do **not** put secrets in `home.sessionVariables` or systemd `Environment=` —
both bake values into world-readable `/nix/store` paths.

Pattern used here: value in a `0600` file outside the repo, read at shell
startup, so only the path is ever in the store.

```nix
programs.bash.initExtra = ''
  if [ -r "$HOME/.config/jules/api-key" ]; then
    export JULES_API_KEY="$(< "$HOME/.config/jules/api-key")"
  fi
'';
```

`sops-nix` is the better long-term home; note `.sops.yaml` currently only lists
an age key for `laptop-framework`, so a new secret would need this host's key
added before it could decrypt.

## 8. Packaging notes for this class of tool

- **Not every npm CLI needs `buildNpmPackage`.** Vercel's published tarball is
  fully esbuild-bundled and runs with no `node_modules` at all; `command-code`,
  `wrangler`, `jules-fleet` and `jules-merge` genuinely need their deps. Test
  with `node dist/<entry> --version` against the bare tarball first.
- npm tarballs ship no lockfile. Generate one from the published
  `package.json` with devDependencies stripped, then vendor it and re-strip in
  `postPatch` so lock and manifest agree.
- `command-code` leaves five unpublished internal packages in
  `devDependencies` (`@commandcode/{harness,providers,remote,shared,tui}`) that
  404 on the registry. npm resolves the full tree even under `--omit=dev`, so
  lock generation is impossible until they are removed.
- Google ships CLIs as `storage.googleapis.com` / `dl.google.com` tarballs
  containing dynamically linked Go binaries. `fetchurl` + `autoPatchelfHook`,
  modelled on nixpkgs' `antigravity-cli`. The `jules` binary additionally
  bundles `go-keyring` (needs a DBus secret service for `jules login`) and
  `minio/selfupdate` (always fails against a read-only store; no flag disables it).
- Watch for binary-name collisions in `buildEnv`: `honeycomb-refinery` ships a
  bare `convert` that collides with ImageMagick and fails the whole
  home-manager profile.
- **Prefer upstream prebuilt release artifacts over overriding `src`.** Bumping
  `src` on a Go/Rust package invalidates `vendorHash`/`cargoHash` and forces a
  full source build with no cache hits. See `pkgs/latest-cli/`.
- **An npm `bin` pointing at a `.js` file does not imply Node.** `grok-dev`
  (superagent-ai/grok-cli) fails under Node three ways that all look like
  separate packaging bugs: `ERR_IMPORT_ATTRIBUTE_MISSING` on a bare
  `import ... from "../package.json"`, then `ERR_MODULE_NOT_FOUND` on
  extensionless relative imports, then `ERR_UNSUPPORTED_ESM_URL_SCHEME` on a
  `bun:` import. Only the third is unpatchable, and it reveals the real fact:
  it is a **Bun** application. Running it under Bun makes all three moot with no
  source edits. Treat escalating ESM errors as one question about runtime, not
  three bugs — I patched the first two before finding that out.
- Watch for npm name drift: that repo is `grok-cli` but publishes as `grok-dev`;
  the packages actually named `grok-cli` and `grok` are unrelated projects.
- Unmet peer deps break `buildNpmPackage` with `ENOTCACHED` on a package npm
  tried to fetch at build time. Generate the lock _and_ build with
  `--legacy-peer-deps`.
- `nix build` exiting 0 does not mean the thing works. Several bugs here were
  only visible by inspecting the generated artifact: an empty systemd `PATH`,
  the wrong `opencode` build on `PATH`, OpenChamber's missing `sdk/dist`.

## 9. Things that do not exist (checked, so nobody re-checks)

- No Honeycomb CLI. `honeyvent` and `honeytrigger` are archived upstream; there
  is no `hny` (the npm package of that name is an unrelated serialization
  format). `honeymarker` is all that remains. Honeycomb's direction is MCP +
  Terraform provider.
- No Cloudflare opencode plugin needed — both Cloudflare providers are built in.
- No Jules driver in t3code, and none in `vibe-kanban` (whose executors are
  `acp`, `amp`, `claude`, `codex`, `copilot`, `cursor`, `droid`, `gemini`,
  `opencode`, `qwen`).
- No official Jules VS Code/JetBrains plugin, Slack/Discord integration, or GCP
  surface. Jules is Google Labs, not GCP.
- Manus has no Linux client at all; its "My Computer" local-access feature is
  macOS/Windows only. Custom MCP servers are the only way to give it local
  access, and self-hosting saves nothing on credits.

## 10. Imperative steps that cannot be declared

```sh
gemini extensions install https://github.com/gemini-cli-extensions/jules
gh secret set JULES_API_KEY            # for .github/workflows/jules.yml
jules login                            # or use JULES_API_KEY to skip the keyring
```

t3 provider instances _can_ now be declared via `settings.json` (§5) — the UI is
no longer the only route.
