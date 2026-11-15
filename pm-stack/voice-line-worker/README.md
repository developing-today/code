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

Deploy needs (account-side, one time):

1. A Turso database for the ledger; set its URL in `wrangler.toml`
   (`TURSO_URL` var) — the schema is created idempotently at runtime.
2. `wrangler secret put TURSO_TOKEN` and
   `wrangler secret put STATUS_INGEST_TOKEN`.
3. `bun run deploy`, then push a first queue snapshot:
   `curl -X POST <worker>/ingest-status -H "Authorization: Bearer $STATUS_INGEST_TOKEN" -d '{"queued":0,"claimed":0,"parked":0,"blocked":0}'`

## Protocol

Client → server: binary PCM16LE frames; text control frames
`{"type":"start"}` (reset buffer) and `{"type":"stop"}` (process).
Server → client: JSON events `ready`, `transcript` (text + seconds
submitted/raw), `answer` (intent + text), then one binary MP3 frame
(the spoken answer), then `done` (session id + ledger seconds), or
`error`.
