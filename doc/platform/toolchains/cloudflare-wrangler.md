# Cloudflare — `cf` / API first, Wrangler configured as fallback — Setup Cycle

Verified 2026-10-21: `cf` 1.0.0-beta.20 at `~/tools/bin/cf`; wrangler 4.147.0 at `~/tools/bin/wrangler`. Account: "Developing.today.llc.domains.0@gmail.com's Account" (account id truncated as `234b866b…` in notes).

**Standing rule (Drew's direction, verbatim intent): ward to only ever use `cf` — but make wrangler configured.**
In practice: prefer the Cloudflare API and the `cf` CLI for all routine work. Wrangler stays installed, authenticated, and configured so it is ready when something genuinely needs it (notably Worker secret puts via the helper below) — it is the fallback, not the default.

## Download / Install
- Both tools are npm packages installed with `--prefix` under persistent disk, because npm's default `/usr` prefix is on the wiped overlay:
  - `~/tools/cf/` — package.json pins `cf: ^1.0.0-beta.20`; binary symlinked `~/tools/bin/cf -> ~/tools/cf/node_modules/.bin/cf`
  - `~/tools/wrangler/` — package.json pins `wrangler: ^4.147.0` (installed 2026-10-05); binary symlinked `~/tools/bin/wrangler -> ~/tools/wrangler/node_modules/.bin/wrangler`
- Reinstall after loss: `npm --prefix ~/tools/wrangler install` (and same for `~/tools/cf`), then confirm the symlinks in `~/tools/bin/`.

## Gather secrets
- Tokens live in Bitwarden Secrets Manager (names only):
  - `cloudflare.api-token-day-to-day` — **the routine token**: Workers Scripts/KV/R2/D1/Queues read+write + Workers Tail read, scoped to this account only (minted 2026-10-07).
  - `cloudflare.api-token-ai-inference` — AI Gateway Read+Run + Workers AI Read+Write, account-scoped. Also the token the opencode service fetches fresh from Bitwarden each boot (see index/services notes).
  - `cloudflare.api-token-account-wide` — broad admin copy. **NEVER put this in Worker secrets or env files** — mint scoped tokens from it instead.
  - R2 S3 credentials: `cloudflare.r2-worker-*` (dedicated bucket-scoped pair the nix-cache worker uses) vs `cloudflare.r2-*` (broad pasted admin copy, kept, not used by the worker). R2 S3 rule learned 2026-10-07: S3 Access Key ID = token id, Secret Access Key = SHA-256(token value).
- Pass tokens **transiently** via `CLOUDFLARE_API_TOKEN` environment variable fed from stdin — never stored in files, wrangler config, or chat. Wrappers: `~/tools/cloudflare/wrangler-authed.py` and `~/tools/cloudflare/wrangler-secret.py` (reads `{"token","name","value"}` JSON from stdin, sets the env var transiently, pipes the value to `wrangler secret put` with cwd `~/git/nix-cache`).

## Configure
- No persistent wrangler auth file is kept; auth is per-invocation via the env var above, which is what "wrangler configured" means here: installed, on PATH, project `wrangler` config present in `~/git/nix-cache`, token injectable on demand.
- R2 must be enabled in the Cloudflare Dashboard before any R2 API works — before Drew enabled it (2026-10-05) the API returned error 10042 "Please enable R2 through the Cloudflare Dashboard". After enabling, Workers/KV/D1/Queues APIs all returned 200 with no further dashboard activation.

## Test
```bash
cf --version        # expected: 1.0.0-beta.20
wrangler --version  # expected: 4.147.0 (a proxy-env WARNING line is normal)
# Auth check (token fed transiently, never echoed):
#   wrangler-authed.py / cf accounts list → expected: the account above listed, HTTP 200 on /accounts and /accounts/{id}/workers/scripts
```
Known cosmetic failures that are NOT auth failures: `/user/tokens/verify` returns 401 "Invalid API Token" for this token type while resource calls succeed; `/user` returns 403 (needs user-level auth; expected for API tokens).

## Known quirks & what has NOT worked
- **Cloudflare edge error 1010 blocks non-browser User-Agents:** python-urllib's default UA gets 403 on workers.dev even for no-auth routes, and on models/AI-Gateway hosts. Use a browser UA header for worker/API test requests. This once masqueraded as a "wrong upload secret" for an hour. Bun's fetch passes where urllib fails.
- **`wrangler secret put` stores stdin bytes verbatim — NO trailing newline.** A stray `\n` breaks `timingSafeEqual` checks and SigV4 signatures. `wrangler-secret.py` exists precisely to send the value with no trailing newline.
- The exec `stdin` parameter + heredoc conflict silently emptied the token in three consecutive auth checks (reported "not authenticated" when the token was fine). Use `python3 -c '...'` (no single quotes inside) and let `stdin` carry the secret.
- Python `http.client` ignores proxy env vars and fails direct TLS (SSL WRONG_VERSION_NUMBER) — use `urllib.request`, which honours them.
- Hand-rolled SigV4 DELETE against R2 kept failing (2026-10-07 probe-object cleanup) — use a real S3 client for R2 object cleanup if it matters.
- Presign flow for the nix-cache worker is verified end-to-end: presign 200 → signed R2 URL → PUT 200 (bypasses the 100 MB Worker request-body limit; Worker never touches the bytes).
