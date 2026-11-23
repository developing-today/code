# Voice line worker — step 1 (#9800)

The basement voice line, first step: talk to the fleet and hear the
answer, end to end, Cloudflare-direct.

- **Push-to-talk WebSocket in a Durable Object** (`src/session.ts`,
  one `VoiceSession` per session id). The page at `/` streams PCM16LE
  16 kHz mono while the button is held and sends `{"type":"stop"}` on
  release.
- **Silence is trimmed before transcription** (`src/vad.ts`):
  energy-based VAD cuts head/tail silence and collapses long internal
  pauses, and _only the trimmed audio_ is submitted — Workers AI
  transcription bills per audio minute submitted, pauses included, so
  silence is never billed and never transcribed. The transcript event
  reports seconds submitted vs seconds recorded.
- **Speech-to-text:** Workers AI Whisper
  (`@cf/openai/whisper-large-v3-turbo`) via the `AI` binding.
- **Speech out:** Workers AI MeloTTS (`@cf/myshell-ai/melotts`) speaks
  the answer back over the same socket (Drew's 2026-11-14 amendment:
  the line must talk back). Outgoing synthesis covers only the answer
  text.
- **One read-only intent** (`src/intent.ts`): queue state. The voice
  line takes no write actions; anything else gets a spoken explanation
  of the one capability. Queue state comes from the newest
  `status_snapshots` row, pushed by the fleet via `POST /ingest-status`
  (Bearer `STATUS_INGEST_TOKEN`) — a status _input_, not a voice write
  action.
- **Session ledger in Turso** (`src/ledger.ts`, libsql HTTP pipeline,
  no SDK): every completed utterance writes a `sessions` row — session
  id, timestamps, transcript, intent, answer, audio seconds in
  (post-trim, as submitted) and out, and the two model ids.
  `audio_seconds_out` is estimated from the answer's word count at a
  standard speaking rate until MP3 frame parsing is worth it.

The OpenCode pooled/failover version of speech (both directions) is
the companion issue #9801 and is deliberately not built here; the
socket and intent shapes do not preclude it (the DO calls the two
models in exactly two places).

## Run / deploy

```sh
bun install
bun test            # VAD + intent suites
bunx tsc --noEmit
bunx wrangler dev   # local: AI binding runs remote against the account
```

The checked-in `wrangler.toml` remains the local-development configuration.
Production changes use the pinned Cloudflare `cf` CLI and separate upload and
promotion workflows. Uploading creates an immutable Workers Version and does
not change traffic. Promotion is an explicit, separately approved operation.
From this directory, on Node.js 24.22+:

```sh
npm run cf:build
```

Run uploads through the manual GitHub Actions workflow rather than locally.
It creates a version tagged with the full source SHA, upload run ID, and
attempt, with the full source SHA in its message. Its receipt binds the UUID
reported by cf to the Worker, an account fingerprint, and the upload attempt;
it is published as an Actions artifact. Upload does not create a deployment
or change the serving split. Check provider Version URL settings separately:
an upload-only operation is not isolated from production resources.

GitHub Actions uploads only from a manually dispatched run on `main`; it runs
the offline tests and build first. Promotion is a different manual workflow,
also limited to `main`, and requires the uploaded Version UUID, its full source
SHA, upload run ID and attempt, and the expected currently serving Version
UUID. Before promotion the workflow checks that the source commit is reachable
from `main`, the candidate
has the exact SHA and upload-run tag plus the full-SHA message, the referenced
upload run succeeded from the expected workflow on `main` at that SHA,
Cloudflare marks it deployable, and the
currently serving deployment is a single 100% version matching the supplied
expected ID. It checks the serving version again immediately before promotion
and verifies that the exact deployment returned by cf is the latest active
deployment and serves the candidate at 100%. The immediate recheck is not an
atomic compare-and-swap against other writers; keep the release window free of
out-of-band deployment changes.

Before enabling either workflow, configure these repository environments
(environment protection and secrets cannot be declared in workflow YAML).
A preflight with only the GitHub workflow read token against the GitHub deployment environment API
halts before the deploy job attaches a missing or insufficiently protected
environment:

1. `voice-line-worker-upload`: allow deployments from `main` only; require an
   independent reviewer, prevent self-review, and disallow administrator
   bypass; add the environment secret
   `CLOUDFLARE_VOICE_LINE_UPLOAD_TOKEN`.
2. `voice-line-worker-production`: allow deployments from `main` only; require
   an independent reviewer, prevent self-review, and disallow administrator
   bypass; add the environment secret
   `CLOUDFLARE_VOICE_LINE_PROMOTION_TOKEN`.
3. Set the non-secret environment variable `CLOUDFLARE_ACCOUNT_ID` in both
   environments.

Create separate Cloudflare API tokens with the Individual Workers Editor role
scoped only to the existing `voice-line` Worker. Store them only as the named
environment secrets above. These workflows do not read generic repository or
organization token secrets. The upload job receives only its environment
secret; the promotion job receives only its own secret after the production
environment approval passes.

No voice call or health request is part of either workflow; promotion verifies
the Cloudflare deployment read-back only.

Deploy needs (account-side, one time):

1. A Turso database for the ledger; set its URL in `wrangler.toml`
   (`TURSO_URL` var) — the schema is created idempotently at runtime.
2. `wrangler secret put TURSO_TOKEN` and
   `wrangler secret put STATUS_INGEST_TOKEN`.
3. Push a first queue snapshot:
   `curl -X POST <worker>/ingest-status -H "Authorization: Bearer $STATUS_INGEST_TOKEN" -d '{"queued":0,"claimed":0,"parked":0,"blocked":0}'`

## Protocol

Client → server: binary PCM16LE frames; text control frames
`{"type":"start"}` (reset buffer) and `{"type":"stop"}` (process).
Server → client: JSON events `ready`, `transcript` (text + seconds
submitted/raw), `answer` (intent + text), then one binary MP3 frame
(the spoken answer), then `done` (session id + ledger seconds), or
`error`.
