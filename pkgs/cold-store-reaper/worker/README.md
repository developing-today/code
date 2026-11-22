# cold-store-rehydrator (Worker)

The read path of the Drive cold store (design section 6): given an object's
sha256, pull its zstd frame out of its Drive bundle by byte range,
decompress it, verify the hash, and return the original bytes. This is the
v1 slice — it serves the object on the same request; filling R2 first and
flipping the manifest state back to `hot` belong to the caller's layer.

## Endpoints

| Route                 | Auth                                         | Result                                                                                                 |
| --------------------- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `GET /health`         | none                                         | `{"ok": true}`                                                                                         |
| `GET /fetch/<sha256>` | `Authorization: Bearer <REHYDRATOR_API_KEY>` | `200` + object bytes; `401` without the key; `404` unknown sha; `502` on any Drive/decode/hash failure |

Successful fetches carry `X-Cold-Store-Sha256`, `X-Cold-Store-Bundle`, and
`Content-Length` headers. Bytes are **never** returned unverified — a sha256
mismatch after decompression is a 502, not a best-effort serve.

## How it works

1. The Worker looks the sha up in the D1 binding **MANIFEST** (database
   `cold-store-manifest`): `objects → members → bundles` yields the Drive
   file id, the frame's offset/length inside the bundle, and the object's
   expected sha256/size. The manifest is kept current by the reaper's
   `cold-store sync-manifest`.
2. The job is handed to the Durable Object **BUNDLE_FLIGHT**, keyed by
   bundle id (`idFromName(bundle_id)`). One instance per bundle
   single-flights concurrent work: identical concurrent member requests
   join one in-flight Drive pull; the Google access token is cached in DO
   storage until ~60 s before expiry.
3. Inside the DO: refresh-token exchange at
   `https://oauth2.googleapis.com/token`, then
   `GET https://www.googleapis.com/drive/v3/files/<id>?alt=media` with
   `Range: bytes=<offset>-<offset+frame_length-1>`. The single frame is
   decompressed with [fzstd](https://www.npmjs.com/package/fzstd)
   (pure-JS, decompress-only) and sha256-verified.

## Layout

```
src/index.js      Worker entry point + BundleFlight Durable Object
src/decoder.js    the one zstd-frame decoder (fzstd) — shared with the proof test
test/run-local-proof.mjs   npm test: packs a real bundle with the Python
                           reaper, decodes a member frame with src/decoder.js,
                           asserts the sha256 (the decoder that ships is the
                           decoder that is tested)
dist/worker.js    esbuild bundle — local build output; the deploy
                  artifact is the flake package build (package.nix)
wrangler.toml     local-development binding configuration
cloudflare.config.ts  deployment configuration, parity-tested against
                      wrangler.toml for the deployed bindings/migration
package.nix       the bundle as a flake package (lockfile-pinned deps)
deploy.sh/.mjs    the deploy app: cf Workers Versions API upload/promote
```

## Build

```sh
npm ci
npm run build     # esbuild -> dist/worker.js (single self-contained file)
npm test          # local frame-decode proof (needs python3 + the zstd CLI)
```

Commit `package.json`, `package-lock.json`, and `src/` together. The
committed `dist/` is a convenience copy for local inspection only —
deploys upload the flake-built bundle, never the committed one.

## Deploy (GitHub Actions, staging gate required)

The `cold-store-rehydrator-upload` workflow builds the lockfile-pinned
bundle and uploads a git-SHA-tagged immutable version with the pinned `cf`
binary. It captures the currently serving deployment before upload and
checks that it is unchanged afterward. Uploading a version does not change
traffic.

Cloudflare does not generate Version URLs for Workers with Durable Objects.
This Worker uses `BUNDLE_FLIGHT`, so the upload workflow does not assign a
preview alias, send requests to a purported preview URL, enable public
preview access, or use production refresh secrets for test traffic. No
independent isolated private staging workflow exists yet. As a result,
`cold-store-rehydrator-promote` fails closed unless given a successful trusted
run of `.github/workflows/cold-store-rehydrator-private-staging.yml` with an
`cold-store-rehydrator-private-staging-evidence` artifact. That evidence must
identify the exact upload run, version ID, and source SHA, report
`result: "passed"`, and assert isolated private staging with no production
secrets or production-resource access. Do not add such a workflow until its
isolation and runtime bindings can be independently verified. Until then,
production promotion is unavailable through Actions.

If private staging is later implemented, promotion also compares the current
serving deployment fingerprint with the exact baseline captured by the
successful upload run. It refuses a stale or malformed artifact and refuses
to promote a version missing any existing secret binding names. Runtime
secret values are never fetched or copied by these workflows; the Worker
continues to use its existing Cloudflare secret bindings. The D1 and Durable
Object bindings, the existing SQLite `BundleFlight` class, the original
`workers.dev` route, and storage are retained unchanged.

Actions require the protected GitHub environment
`cold-store-rehydrator-production`, a required reviewer, the dedicated
Worker-scoped environment secret `COLD_STORE_REHYDRATOR_CF_API_TOKEN`, and
the environment variable `COLD_STORE_REHYDRATOR_CF_ACCOUNT_ID`. The token
needs only Workers Scripts read/edit permissions for this Worker’s version
upload, inspection, and deployment. The secret is available only to the
trusted `main` dispatch job step; no PR workflow has access to it.

For local operator use, `deploy.sh` keeps the existing SOPS account-ID and
Bitwarden token lookup (`cloudflare.api-token-day-to-day`, overridable with
`CSR_CF_TOKEN_SECRET`), or accepts explicit `CLOUDFLARE_ACCOUNT_ID` and
`CLOUDFLARE_API_TOKEN` environment values. Runtime API and Google refresh
secrets are not fetched or copied. Upload is the no-argument operation;
`--deploy-version <id> --expected-baseline <sha256>` is the guarded
promotion/rollback operation and requires the exact current serving baseline.

Until `sync-manifest` has run at least once against a manifest containing
verified bundles, every `/fetch/<sha>` is a well-formed 404.
