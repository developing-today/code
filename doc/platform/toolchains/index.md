# Toolchains — Setup-Cycle Index

Draft status: DRAFT, generated 2026-10-21 from `~/TOOLS.md` plus live version checks in the Muse sandbox. Not yet reviewed, not committed anywhere.
Rule for every doc in this folder: secrets are referenced by **Bitwarden item NAME only — never by value**. If a doc ever needs a value, it says where to fetch it, not what it is.

Every doc follows the same cycle: **Download → Install → Gather secrets → Configure → Test → Known quirks & what has NOT worked.**

| Doc | Tool | Verified in sandbox 2026-10-21 |
|---|---|---|
| [nix.md](nix.md) | Nix 2.35.2 (single-user, symlinked store) | `nix (Nix) 2.35.2` |
| [git.md](git.md) | Git + GitHub CLI (`gh`) | git 2.43.0; gh 2.101.0, logged in as `dezren39` |
| [bitwarden-bws.md](bitwarden-bws.md) | Bitwarden Secrets Manager CLI (`bws`) | bws 2.1.0 |
| [cloudflare-wrangler.md](cloudflare-wrangler.md) | Cloudflare API / `cf` first, Wrangler fallback | cf 1.0.0-beta.20; wrangler 4.147.0 |
| [onepassword-op.md](onepassword-op.md) | 1Password CLI (`op`) | op 2.41.0 |
| [bun.md](bun.md) | Bun | bun 1.4.2 |
| [uv.md](uv.md) | uv (Python tooling) | uv 0.12.16 |
| [go.md](go.md) | Go | go 1.25.1 linux/amd64 |
| [rust.md](rust.md) | Rust via rustup | toolchain stable; cargo 1.98.1 (with env set, see rust.md) |
| [tailscale.md](tailscale.md) | Tailscale (platform wrapper, not upstream CLI) | `tailscale status` → Connected |
| [chromium-playwright.md](chromium-playwright.md) | Real browser (managed Chromium) + Playwright status | No local chromium/playwright binary found — see doc |
| [muse-cli.md](muse-cli.md) | Muse CLI | **NOT found on PATH in this sandbox on 2026-10-21** — see doc |

Cross-cutting sandbox facts that affect almost every tool (from TOOLS.md):

- `/home/hatch` (btrfs) is the only filesystem that survives a sandbox recycle. The overlay (`/`, `/usr/local/bin`, apt installs) is wiped. Anything that must persist lives under `~/tools/` or another `~/` path, and `~/tools/init.sh` (cron `sandbox-boot-init`, every 5 min) restores the ephemeral pieces each boot.
- `/tmp` is a 512M tmpfs — too small for some builds (see go.md) and **not shared with systemd services after a recycle** (see nix.md / services note).
- Outbound HTTPS from scripts must go through the egress proxy. The session `HTTPS_PROXY` URL carries a **rotating per-session credential — never bake it into files**. Services use the unauthenticated in-sandbox form `http://hatch-egress-proxy:3128` plus `NODE_EXTRA_CA_CERTS=/run/hatch/egress-tls/ca-bundle.pem`.
- Python `urllib.request` honours the proxy env vars; `http.client` does not (direct TLS fails with SSL WRONG_VERSION_NUMBER).
- The exec tool's `stdin` parameter conflicts with shell heredocs (`<<'EOF'` wins, stdin is silently dropped). When a secret must be fed via stdin, use `python3 -c '...'` with no heredoc.
- Any unknown command is resolved by `~/tools/dispatcher.sh` (omnibin-backed `program@version` support via `/bin/bash` wrapper + `BASH_ENV=~/tools/shell-env.sh`). Exact-version misses fail loudly with exit 127 — never substituted.
- [binary-backup.md](binary-backup.md) — standalone binaries to back up as-is (incl. bws), generated 2026-10-21
