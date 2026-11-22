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

## Deploy (GitHub Actions, staged first)

The `cold-store-rehydrator-upload` workflow builds the lockfile-pinned
bundle, uploads a git-SHA-tagged version using the repository-pinned `cf`
binary, assigns a unique preview alias, and checks the preview endpoints.
It does not create a production deployment or change the existing route.
Review the successful upload run's preview URL and confirm `/health` is
healthy and unauthenticated `/fetch/<sha>` still returns `401` before
running `cold-store-rehydrator-promote`.

Promotion is a separate, manually dispatched workflow. It downloads the
version record from a successful upload run and deploys that version at
100% only after the operator attests that staged parity was reviewed. The
script also refuses versions missing the existing secret binding names;
secret values are never read by the workflow. The D1 and Durable Object
bindings, the existing SQLite `BundleFlight` class, and the original
`workers.dev` route are retained in `cloudflare.config.ts`; a local test
checks the binding/migration fields against `wrangler.toml`.

Both workflows require the protected GitHub environment
`cold-store-rehydrator-production`, with environment secrets
`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`. The token should be
scoped to this Worker and only the Workers Scripts read/edit permissions
needed for versions and deployments. Configure required reviewers on the
environment before enabling the workflow. Jobs are limited to `main`, use
`contents: read` (and `actions: read` only to fetch the successful upload
artifact), and have no pull-request trigger. A direct local invocation
with no arguments also uploads/stages only and requires
`CLOUDFLARE_API_TOKEN` plus `CLOUDFLARE_ACCOUNT_ID` in the environment;
`--deploy-version <id>` is the explicit promotion/rollback operation.

Until `sync-manifest` has run at least once against a manifest containing
verified bundles, every `/fetch/<sha>` is a well-formed 404.
