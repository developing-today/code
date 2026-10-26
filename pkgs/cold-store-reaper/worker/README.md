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
dist/worker.js    esbuild bundle — this is what gets deployed
wrangler.toml.example      binding reference (deploy is API-driven, see below)
```

## Build

```sh
npm ci
npm run build     # esbuild -> dist/worker.js (single self-contained file)
npm test          # local frame-decode proof (needs python3 + the zstd CLI)
```

Commit `package.json`, `package-lock.json`, `src/`, and the built `dist/`
together — the operator deploys `dist/worker.js` as-is.

## Deploy (operator, via the Cloudflare REST API)

Wrangler does not deploy this in the current environment; the operator:

1. Uploads `dist/worker.js` as the Worker script (PUT
   `/accounts/<acct>/workers/scripts/cold-store-rehydrator` with the D1 and
   DO bindings from `wrangler.toml.example` in the metadata), including the
   DO migration `new_sqlite_classes = ["BundleFlight"]` on first deploy.
2. Sets the secrets as `secret_text` bindings: `REHYDRATOR_API_KEY`
   (operator-chosen), `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`,
   `GOOGLE_REFRESH_TOKEN` (values from Bitwarden — see the reaper README's
   credential notes; the Google pair/refresh token are the same grant the
   reaper uses).
3. Enables the workers.dev subdomain for the script, or attaches a route.

Until `sync-manifest` has run at least once against a manifest containing
verified bundles, every `/fetch/<sha>` is a well-formed 404.
