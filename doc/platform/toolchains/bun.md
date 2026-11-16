# Bun — Setup Cycle

Verified 2026-10-21: bun 1.4.2. `bunx` in `~/tools/bin/` is a symlink to the base-image bun at `/opt/hatch-image/bin/bun`.

## Download / Install
- Bun ships in the sandbox base image (`/opt/hatch-image/bin/bun`) — no separate install step was recorded in TOOLS.md, and no version pin is managed locally. If the base image changes, the version changes with it; re-verify with the Test step after any recycle-related surprise.
- Exposure: `~/tools/bin/bunx -> /opt/hatch-image/bin/bun`. Project-level JS work also goes through the tools installed under `~/tools/cf` and `~/tools/wrangler` (npm `--prefix` installs on persistent disk — see cloudflare-wrangler.md).

## Gather secrets
- Bun itself needs no secrets. Services/processes *run* under Bun may need the egress/CA setup below, and tokens fetched by name from Bitwarden (never values in configs).

## Configure
- **Systemd services inherit NONE of the session env** — no proxy, no CA bundle, no tokens. Anything Bun-based running as a service needs its env file to carry:
  - Unauthenticated in-sandbox proxy: `http://hatch-egress-proxy:3128` (the session `HTTPS_PROXY` carries a rotating per-session credential — never bake that form into files)
  - `NODE_EXTRA_CA_CERTS=/run/hatch/egress-tls/ca-bundle.pem`
  - With those two, Bun fetch works (verified: models.dev returned 200, 2026-10-20).
- This pattern is implemented for the opencode service in `~/tools/secrets.d/opencode-serve.env` (dir 700, file 600, regenerated every boot by `~/tools/install-opencode-service.sh`, which also fetches the Cloudflare inference token fresh from Bitwarden by name: `cloudflare.api-token-ai-inference`).

## Test
```bash
bun --version   # expected: 1.4.2
```

## Known quirks & what has NOT worked
- Cloudflare-fronted hosts 403 non-browser User-Agents (error 1010 class) for python urllib, but **Bun's fetch passed** those same hosts (2026-10-20) — when a python check fails and Bun succeeds, suspect the UA, not the token.
- `opencode run` (CLI) hangs on this box under every tested configuration, while the opencode *server* (same binary) answers fine via HTTP — unrelated to Bun itself, but recorded here because Bun is the runtime in that stack: drive opencode through its server (`opencode serve --pure --port 4096 --hostname 127.0.0.1`), always with an explicit `?directory=<project dir>` (a session defaulted to /home/hatch stalls on snapshot/git work over the giant home worktree).
- `--pure` is required for the opencode server for now: with plugins enabled, session creation works but the message loop never starts (mcpx bridge plugin is the suspect).
- `/tmp` is NOT shared with systemd services after a recycle — use `~/tmp/<dir>` for service-visible working directories.
