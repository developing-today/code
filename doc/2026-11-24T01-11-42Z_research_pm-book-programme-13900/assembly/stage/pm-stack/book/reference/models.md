# Model dependencies

This chapter states the PM stack's model surface exactly as the survey's model census records it. Every entry follows the same shape: what it is, its inputs and shape, its defaults where the survey gives them, its behaviour, and its failure shapes where the survey records them. Census method: greps over `pm-stack/` and `pkgs/pm-*` for embedding, LLM, and model-call patterns, ground truth the laptop clone at `4d937b3`, 2026-11-23.

## The headline fact: no LLM or embedding calls in the PM core

The aggregator, the console, dispatch, and the projector make **no LLM calls and no embedding calls of their own**. No embedding model, embedding endpoint, or vector store is referenced in `pm-stack/aggregator/src/`, `pm-stack/console/src/`, `pm-stack/dispatch/src/`, `pm-stack/projector/src/`, or `pkgs/pm-*/`. The entries below record each component's position, then the two real model surfaces in and around the stack: the voice line's Workers AI inference, and the Atlas embedding dependency.

## The aggregator

- **What it is:** The dashboard service; its model position is measurement, not inference.
- **Behaviour:** No call to `api.openai.com`, `api.anthropic.com`, Workers AI, or a chat-completions endpoint exists in `aggregator/src/`. Grep hits for provider API patterns are quota accounting for Workers AI neurons (`src/saas-extract.ts`, `sources/fixtures.ts`) — measuring a bill, not calling a model. Its adapters (`claude-adapter.ts`, `clinepass-adapter.ts`, `token-adapter.ts`) call provider usage endpoints (for example `https://claude.ai/api/organizations/&lt;uuid&gt;/usage`) to read quota windows — no inference.
- **Search:** `/search` is a title and linked-reference search over the aggregator's own projections (`src/search-view.ts`), not a vector search.
- **Failure shapes:** None recorded in the survey.

## The console

- **What it is:** The owner app, local server and edge Worker; its model position is none.
- **Behaviour:** No LLM call exists in `console/src/` or `console/edge/src/`. Its summarisation-shaped surfaces (overview, questions) are projections of stored data.
- **Fixtures, not calls:** Test and demo fixtures name models as data — `console/scripts/demo-state.ts` uses `model: "gpt-5.2-codex"` and `"workhorse"` as session labels; `console/test/edge.test.ts` uses `model: "muse"`. These strings are fixtures, not dependencies.

## Dispatch

- **What it is:** The intake service; it makes no inference of its own, and it *places* inference.
- **Inputs:** A caller-supplied model hint, in `"provider/model"` form or as `{providerID, id}` (`dispatch/src/validate.ts`), passed to the OpenCode server when a session is created (`dispatch/src/opencode.ts`). The README's worked example names `opencode-go/gpt-5.6-luna`.
- **Behaviour:** The placement guard (`dispatch/src/placement-guard.ts`) maps explicit providers to quota pools — `opencode-go.drewrypope`, `opencode-go.popesdrew`, `opencode-go.developing-today-0`, `opencode-go.0x111dev`, plus ClinePass model hints — and pins the bare `opencode-go` provider to the active OpenCode Console account at launch.
- **Failure shapes:** The guard fails pre-commit on a walled pool.

## The projector

- **What it is:** The linking and projection service; its model position is none.
- **Behaviour:** No LLM or embedding call exists in `projector/src/`. The word "embedders" in `projector/src/server.ts` comments means embedding applications — hosts that embed the projector library — not vector embedders.

## Voice-line inference (Workers AI)

- **What it is:** The only in-tree inference calls in `pm-stack/`: the `voice-line` Worker's speech models, in `pm-stack/voice-line-worker/src/session.ts`.
- **Shape:** Speech-to-text is `STT_MODEL = "@cf/openai/whisper-large-v3-turbo"`; text-to-speech is `TTS_MODEL = "@cf/myshell-ai/melotts"`. Both run through `env.AI.run(...)`.
- **Behaviour:** Push-to-talk runs over a WebSocket in a Durable Object: speech in, transcription, answer, speech out.
- **The answer is not an LLM:** Between transcription and speech, `matchIntent(transcript)` and `buildAnswer(intent, snapshot)` run over a Turso queue snapshot in the same file — template answers for recognised intents, for example `queue_state`.
- **Failure shapes:** None recorded in the survey.

## The Atlas embedding dependency

- **What it is:** The pinned embedding space of Atlas (`pkgs/atlas-search/`), an adjacent system, not a PM-stack dependency. No PM component consumes Atlas embeddings in the code surveyed; the PM connection is documentary only (`pm-stack/lineage-manifest-2026-11-12.md`, line 329, records the open design question #1518 on code and config embeddings). Any claim that the PM dashboard uses embeddings is UNVERIFIED and not supported by code.
- **Production model:** `Snowflake/snowflake-arctic-embed-m-v2.0`, 768 dimensions. The pin agrees in three places: `pkgs/atlas-search/worker/src/space.ts` (`EMBED_MODEL`, `EMBED_DIMS = 768`, #6788), `pkgs/atlas-search/atlas_search.py` (`MODEL`, at full 768 dimensions), and the Rust staging aggregator's mandated identity `EMBEDDING_MODEL_ID` (`pkgs/atlas-search/crates/atlas-core/src/shards.rs`), used by `aggregate.rs` at promotion; every unit database's manifest is stamped `embedding_model`.
- **Behaviour:** `validateEmbed` in the Worker refuses any `/v1/embed` response naming another model or another width.
- **Dormant fallback (Python reference only):** `BAAI/bge-small-en-v1.5` through fastembed, in a 384-dimension table `vec_bge_small_384_f32`. The Atlas book calls the Python reference the only surface that still offers the dormant bge-small fallback embedder. The production Worker and the Rust aggregator do not offer it.
- **Overrides:** `ATLAS_SEARCH_MODEL` and `ATLAS_VEC_TABLE` exist in the Python tool only.
- **Full coverage:** The Atlas book, published at [https://developing-today.github.io/code/atlas/](https://developing-today.github.io/code/atlas/).

## Models the stack measures and routes (not dependencies)

- **The AA and Cost Explorer and quota system are about models:** `aggregator/src/aa.ts` reads the deployed OpenCode config's provider catalogs and `experimental.model_routes` (route names include `workhorse`, `speed`, `specialist`, `summit`, `small`), computes route leaf sets, and joins Artificial Analysis measurements, plan seats, and per-model price bands. These are data subjects, not calls.
- **A ready-for-agent default:** Every repository in `pm-stack/ready-for-agent/config.json` sets `defaultModel: "opencode-go/kimi-k3.1-pro"` — a model default in another app's configuration, not a call from this repo.
- **Session provenance:** Session records (`model.ts` `SessionRecord`, the console's `Work.sessions()`) carry the session's model or route as a string, for example `workhorse` — provenance data about fleet work, not a PM-stack dependency.
- **Provider services named by quota adapters** — Claude, ClinePass, OpenCode Go/Zen, Open Design, and Mistral as a planned exporter adopter — are account and usage integrations, not inference dependencies of the PM stack.
- **A naming trap, recorded so the book does not repeat it:** Historical load-test data in `aggregator/docs/aa-load-2026-11-12/gen_load.py` names synthetic families "Claude Sonnet 5.5" and "Claude Opus 5.5" (slugs `claude-sonnet-5-5`, `claude-opus-5-5`) — a generated load fixture, not a dependency and not a real model name to cite.
- **A non-dependency in the port registry:** OpenClaw's embedding server (llama-server, port 18791 in `pm-stack/PORTS.md`) is a laptop service recorded in the registry; no PM code references it.

Sources: PM-SURVEY model census (`pm-stack/aggregator/src/`, `pm-stack/console/src/`, `pm-stack/dispatch/src/`, `pm-stack/projector/src/`, `pm-stack/voice-line-worker/src/session.ts`, `pkgs/atlas-search/`), survey file `models.md`, census date 2026-11-23.
