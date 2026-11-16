# Bitwarden Secrets Manager CLI (`bws`) — Setup Cycle

Verified: bws 2.1.0 at `~/tools/bin/bws` (on PATH), set up 2026-10-07. This is the sandbox's primary secrets store. **All other toolchain docs reference secrets by the item names listed here — values never appear in any doc.**

## Download
- From `github.com/bitwarden/sdk-sm` release `bws-v2.1.0`, asset `bws-x86_64-unknown-linux-gnu-2.1.0.zip`.

## Install
- Unzip the binary to `~/tools/bin/bws` (persistent disk, on PATH via `~/tools/shell-env.sh`). No installer, no service.

## Gather secrets (the bootstrap problem)
- bws itself is authenticated by a machine-account access token. Format: `0.<org>.<env>.<client_id>:<client_secret>` — shown **ONCE** at creation in the web vault (Secrets Manager → machine account → access tokens). If it's lost, mint a new one; it cannot be re-displayed.
- Receive a new token (keeps it out of argv, process list, and shell history):
  ```bash
  echo "$TOKEN" | ~/tools/bws-receive-token.sh
  ```
  The script stores it to `~/.config/bitwarden/access-token` (file 600, dir 700) and verifies with `bws secret list`.
- The bws access token itself is deliberately **NOT** stored inside Bitwarden Secrets Manager.

## Configure
- Load the token into a shell (never prints it):
  ```bash
  source ~/tools/bws-env.sh   # exports BWS_ACCESS_TOKEN; prints only its length
  ```
- Optional EU vault override: `~/.config/bitwarden/server-url` → exported as `BWS_SERVER_URL` by bws-env.sh.
- Project in use: "developing today" (id `d370cd10-…`, truncated in TOOLS.md deliberately).

## Item names known to exist (names only — use these in other docs)
- `nix-cache.upload-secret`, `nix-cache.signing-key`
- `cachix.cachix-dhall`
- `github-cli.hosts-yml`
- `ssh.id-ed25519`
- `cloudflare.api-token-account-wide` (admin copy — NEVER put in Worker secrets/env; mint scoped tokens from it)
- `cloudflare.api-token-day-to-day`, `cloudflare.api-token-ai-inference`
- `cloudflare.r2-access-key-id`, `cloudflare.r2-secret-access-key`, `cloudflare.r2-endpoint` (broad admin copy, not used by the worker)
- `cloudflare.r2-worker-api-token`, `cloudflare.r2-worker-access-key-id`, `cloudflare.r2-worker-secret-access-key` (dedicated, bucket-scoped to nix-cache-nars only)
- `google.jules-api-key`
- `porkbun.api-key`, `porkbun.secret-key`
- `meta.api-key`, `muse.sdk-token`
- `sops.age-private-key-sandbox`
- `supabase.*` (access token, db password, service-role/secret/anon/publishable keys, project url/ref/host — see the Supabase connector doc for the full list)

## Test
```bash
source ~/tools/bws-env.sh
bws secret list   # expected: JSON list of secrets in the project; failure = token/permissions problem
```
Backup/round-trip pattern: `~/tools/bws-backup-secrets.sh` — copies on-disk secrets into the project, each one sha256-verified after round-trip; re-runnable and idempotent.

## Known quirks & what has NOT worked
- **`bws secret create` has NO stdin/file input.** `create KEY - PROJECT` stores the literal string `"-"`. Pass values via `"$(cat file)"` — the command text retained by the exec harness then contains only the file path, never the value (it appears only transiently in the bws child's argv). Full pattern in `~/tools/bws-backup-secrets.sh`.
- `secret get <id> -o json` shape has varied (object vs array) — normalize with `jq 'if type=="array" then .[0].value else .value end'`.
- `jq -r` appends a trailing newline — use `jq -j` when hashing values for comparison or hashes will never match.
- **Values starting with `-`** (e.g. `-----BEGIN OPENSSH PRIVATE KEY-----`) are parsed as flags. Put options first, then `--`, then positionals: `bws secret create --note ... -o none -- KEY VALUE PROJECT`. Otherwise clap errors **and echoes the value in the error message** — a leak risk, not just a failure.
- Never combine the exec tool's `stdin` parameter with a shell heredoc when feeding token material — the heredoc wins and stdin is silently dropped (see index.md).
