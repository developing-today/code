# Brief supplement — capabilities: per-capability routing, failover, and pools as models

Filed 2026-11-13 (Drew, main chat ~07:10 CT) as a supplement to the pool & effort
design mill (starter issue code#9399). Faithful transcription of Drew's brief,
sectioned. Companion to `brief.md`, `brief-decisions-supplement.md`, and
`brief-decisions-amendment-2.md` in this folder. Cross-links: the own-benchmark
program (quota-percentage benchmark tool goal), code#9373 / code#9374 (durable
invocation ledgers), the PM-stack analytics section (/quota#analytics), the
Fleet Roadmap & Stats Space, opencode#1146 (protocol façade), the time-of-day
routing ticket code#9378, and the OpenClaw picker-cleanup issue filed alongside
this supplement.

## §0 Headline rule: failover is scoped per capability

Update OpenCode to support model capabilities and failover **separately for
each capability set**. Failure for text does not mean failure for audio
transcription. Failure for images or video does not mean failure for text, and
so on — unless there is a known issue that is widespread, such as quota
reached, being unable to even form a route, or possibly auth failures. Those
global failure classes fail the whole leg; capability-local failures fail only
that capability on that leg.

## §1 Capability-gated pools

- Pools support requests that can only be served by specific models. Within
  the pool, models that cannot serve the request are excluded, same as normal,
  as if they were walled.
- Models that don't expose the needed capabilities are skipped, or used only
  during fallback.
- All tool formats are accepted.
- A pool's capabilities are set as an **array list of sets of capabilities**;
  a request is eligible for the pool only if its required capability set is a
  subset of one of the pool's sets.

## §2 Declaration and inheritance

- If a pool doesn't declare capabilities, all unique capability sets of the
  underlying models get pushed up — **unless** an inner pool the pool calls
  sets its own capabilities, in which case the inner declaration governs that
  branch. Example: Luna supports images too, but if the Luna plan pool's
  capabilities are set to just text and it is used inside the small pool for
  an image request, the Luna plan pool is skipped — it is acting as a text
  pool regardless of what is underneath it. The small pool moves on to the
  next item, excludes it from the wait / round-robin until it finds an
  actually eligible option, or errors out. The eligible option may still be a
  Luna plan reached by another path whose capabilities were not limited.
- Whether a capability list may be overridden for a plan inline is an open
  design question; the wrapper pattern may make it unnecessary: "Luna plan
  text" is a pool whose only item is the Luna plan, adding specific
  capabilities (or context limits) on top of another pool. Through wrappers
  we can leave all existing pool plans exactly as they are, and focus instead
  on obtaining and setting appropriate capabilities for all providers and
  models that don't directly provide this information.
- Capability config may live in a config section keyed by specific model
  names, model families, or model versions. Capabilities can be set in every
  pool and within every provider. Design question: one `capabilities` key, or
  two — the capabilities a model/pool **has**, and separately which
  capabilities it **allows** without being forced as its default.

## §3 Capability data and protocol coverage

- All custom providers are updated with all the capabilities they support,
  and the code is updated to support all message types and tools each model
  supports — everything supported by OpenCode or OpenChamber or both, plus
  our own extensions for other capabilities.
- Some options (two-way voice streaming) may not be possible over a queue
  session or specific paths such as OpenCode running in a shell on some
  operating systems.
- Fallbacks for models that only state text support: provide the image at a
  tmp path (or in the worktree tmp dir, which gets ignored). This requires
  testing to ensure the model can actually take an image path and convert
  those bytes into an image on its side.
- Provider interfaces must support all OpenAI, Claude, Cloudflare, and System
  1 protocols and message types, even where not directly supported in
  OpenCode sessions or OpenChamber. Extend to images or video through the
  Kitty protocol in a TUI or over SSH where that helps.
- Where providers differ in how an image is provided (Antigravity vs Muse,
  Muse vs Cloudflare, DeepSeek vs GPT), accept any of those methods in the
  input and automatically translate to the best, most useful protocol for
  each provider-model combination.

## §4 Pools as models (the endpoint surface)

Support using pools **just as a model** — not ACP, not executing out to a
shell. A pool endpoint accepts REST, streaming forms, WebSockets — whatever's
supported — enabling any named pool to serve:

- text-to-speech
- speech-to-text / transcription
- image description
- image or video description
- image or video generation from a description
- turning text into audio
- streaming one way
- streaming two ways
- embeddings and vectors
- decision models (System 1 class; see the decisions supplement)

## §5 Workhorse must serve every modality

Make it so images, text-to-speech, videos, image generation, and maybe video
generation all work in the OpenCode **workhorse** pool (and by construction
the other class pools). Workhorse must be usable for:

- generating images
- describing images
- generating metadata from images, and embeddings
- generating embeddings
- two-way conversations
- taking in a video: using **both** the video and the audio to generate
  subtitles, video summaries, chapter markers, notes, and annotations
- follow-up research using the audio and the image frames combined for a
  complete understanding — not just taking the audio from the video, doing
  speech-to-text, and calling it a day
- speech-to-speech: where models support text-to-speech-to-text two-way
  communication and can skip the text step, send and receive audio streams
  directly, even if OpenCode can't (and maybe OpenClaw can't either)

The endpoint should support all protocols, capabilities, and transport
methods any harness or API could support (Codex, Claude, DeepSeek harness,
etc.).

## §6 Harness-only capabilities

Some providers/models have capabilities not easily exposed directly, normally
used only within their own harness or by an ACP calling their own harness.
For those: build our own wire-protocol equivalent, or temporarily disable /
defer to an ACP method — if it can be done without breaking the direct text
model methods. Always consider the full context of what's available.

## §7 Per-capability cost sanity

With multi-provider plans (decision models, medium-strength models), allowing
direct usage of a certain capability may be inefficient because it's expensive
or uses lots of tokens. A model may be priced fine by itself, yet there is no
circumstance where using it is right, because in every such circumstance the
smarter model is cheaper (usually DeepSeek — or if not smarter, equivalent).
The design must let a pool exclude a model **for a capability** on dominance
grounds without excluding it generally.

## §8 Capability data sourcing: models.dev and identity resolution

- Get new model lists directly from providers when possible, and capability
  data directly from providers. (Extracting from inside harness binaries is
  possible but not the expected main source.)
- Routinely pull the entire models.dev page (or the pages for all model IDs
  we interact with), store it ourselves, and **track all changes to
  models.dev over time** at different sample times.
- Identity resolution: provider-published model IDs (published or
  unpublished-but-available) must be tracked back to their most correct
  models.dev ID and AA benchmark records. Inference fills gaps (e.g.
  Luna-2026-10 counts the same as Luna 6.1, which may differ from Luna 6.0).
- Providers silently remap: one replaces the generic Luna / DeepSeek Flash
  with a new version; another updates the generic but adds a pinned ID for
  the new version (e.g. a 11507-specific ID) while the generic points at the
  older; another adds pinned IDs for both and the generic becomes a pointer.
  All these shapes must resolve correctly.
- A quantized model is treated as an entirely different model. We never use
  secretly quantized models without specific investigation and benchmarking.

## §9 Estimation: trends, priors, and plan hidden variables

- Deals exist through self-hosting, buying extra compute, and batching into
  pools (all inputs and outputs) — multiples of pricing discount for the
  same non-quantized capability at a slower rate but cheaper cost per
  eventual result. This ties into our own testing program: run a variety of
  models (similar to or worse than AA benchmarks suggest) to build trend
  lines and understand how our uses differ from AA benchmarks, including
  where plan pricing has undisclosed differences from expected API pricing.
- Effort-curve prior (recorded observation): the previous version got a test
  for medium effort; it performed slightly worse than the curve from high →
  xhigh → max would have led one to believe. That pattern has been stable
  for three different test versions and two previous family versions, and
  lately across all models from that company's source model creator, with
  the only variation being that certain downstream providers are typically
  slower in order to be cheaper (and rare quantization = a different model).
- Even after factoring plan prices vs dollars spent vs tokens received,
  hidden variables may drive real cost: current usage and server needs, time
  of day, day of week, holidays — whether here, UTC, or in China.
- **Terra Low case (motivating anecdote):** Terra Low was supposed to be
  similar to or better than Luna High, faster and cheaper, but in our short
  test it was similar in speed, similar in capability, and burned through
  quota much faster. Hypotheses the design must be able to test:
  - all our models are too complex for some tasks — we need lower pools or
    automatic pool choice;
  - plans secretly evaluate percentage differently — input, output, or
    context/cached ratios may be weighted differently from the published
    benchmark;
  - a variety of other reasons.
- Estimates for new models without AA results: derive relative speed / score
  / cost estimates from trend data, segments, and comparisons ("slightly
  better than X, this fast, cheaper than Y"), using the history of how cost,
  capability, and speed have worked for a given provider over time — marked
  appropriately as estimates, with **precision window bounding boxes**: up to
  this smart, at least this smart; speed between this and this seconds per
  task; TTFT between this and this; tokens per second range; expectations on
  output/input token amounts or thinking-token growth. We already do some of
  this today; gather and track all available data from models.dev, AA, and
  other anecdotes, user experience, blogs, reviews, and benchmarks besides
  AA.
- Track previous predictions, their reasoning, and the real result, to
  improve error-margin accuracy and tune calibration (systematic
  under/over-estimation per property).

## §10 Measurement surfaces and aggregation

Compare all measurements against our real raw data, distinguishing the
execution surface:

- queued tasks
- direct execution
- direct chats with the model from a phone or a computer
- chats executed directly by Muse (Muse as gopher between Drew and
  OpenChamber/OpenCode)
- scheduled queued tasks vs hook tasks

Statistics are aggregated and rolled up over time to Spaces and other
aggregation methods/timings. Required grouping axes: provider, model family,
model version/variant, specific capability — on a given day, time, or holiday
(including Chinese holidays). Questions the data must answer:

- Does a model perform the same on release day vs 1–6 months later?
- Are there indicators a specific model is favored/cheaper than equivalents
  when plan data is converted to dollars and compared to the API equivalent
  in dollars (sum of a session's token usage — input/output/cache — with or
  without sub-sessions and nested children)?

## §11 Quota machinery for the new surfaces

- Keep track of quotas wherever new things are built. OpenCode's quota
  refresh rate must be configurable per model/provider: e.g. set Workhorse
  to a 1-second refresh generally, and specifically set the Claude provider
  to a minimum refresh interval of 5 seconds; the rest of Workhorse might
  use 1 second while the Claude provider models use 5. Providers get minimum
  and maximum refresh timings; targeting can also be by "providers used in a
  session in the past X hours".
- The Antigravity Claude models might be entirely separate — but we also
  need to target **all uses of a model** (compare all providers serving a
  given model against a new model available from one or two providers that
  may or may not overlap the first set).
- Integer-percent-only providers are especially difficult. Method: find all
  queries after the quota was last pulled until the next pull; if the number
  ticked up, proportionally divide the increase evenly across all tokens
  sent or received since the percentage last changed. Compute at 1%, 5%,
  10%, 15%, 20%, and 25% intervals across multiple quotas, using the ratio
  of input/output/cached provided by the API (and bounding cases: as if all
  usage came from input, or all from output, or all from cached). Track
  session counts and other details. From this, narrow a rough window of API
  cost converted into a percent × 100; compare against plan cost for the
  theoretical value of a model effort at a specific time → an **effective
  multiplier**. Other indicators: number of PRs merged, time until task
  completion, PRs merged per task completion, whatever else can be inferred.
- Use large error bands and track over time to align measurements with actual
  usage. Must distinguish: the latency of the quota endpoint itself (some
  update on intervals somewhere between every 5 and 20 minutes; the tick
  could have happened any time between the last measurement and the current
  one, with variable delay), vs endpoints that provide the correct value
  immediately and rate-limit you for polling too often.
- Sessions whose turns spill across a quota tick must be handled; multiple
  sessions may be mid-output when data is gathered, and backtracking 5
  minutes shows the same. Explore whether certain models are favored more
  than expected, or whether plans treat user input turns vs API rates for
  input/output/cached differently. Look for standard tests that isolate the
  requirements enough to run across many models and effort levels and get
  meaningful data.

## §12 Benchmark corpus

- Gather every individual benchmark that makes up the composite AA score.
- Gather other third-party benchmarks with different results; benchmarks
  that were only for OpenAI or Claude may be online and rerunnable for
  another model.
- Generate predictions for models with incomplete information (§9), keep
  tracking predictions + reasoning + actuals, and tune.

## §13 Proof obligations (end to end)

- Prove OpenClaw can handle attachments and all of the above in and out as
  appropriate.
- Prove everything works end to end from an **Android phone through OpenClaw
  to OpenCode, through a pool into a specific model**, potentially with
  transformations along the way for different protocols.
- Ensure all methods used for speech-to-text or for inputs and outputs in
  the OpenClaw app (which could be supported by one of the models in one of
  the pools) are actually supported.
- Set the effort for OpenCode route pools in OpenClaw, and for OpenCode
  routes.

## §14 OpenClaw picker surface (split out as its own issue, filed alongside)

- Remove the "(OpenCode Route pool)" suffix on every pool — it is clarified
  by being submitted by OpenCode Route. At most, say: **Workhorse pool ·
  Summit pool · Scout pool · Specialist pool · Speed pool · Small pool**
  (Small pool is to be **added** — the other five are already exposed).
- Under each, at most: "Support: low, medium, high" (the efforts supported),
  and pass through any capability sets supported by any of the models,
  appended together and made unique.
- Audit whether our OpenClaw providers are correctly made, and whether we
  should support them at all other than the OpenCode route output. They may
  be useful as fallback methods if confirmed to work, but Claude CLI uses
  the Claude binary directly and ACP Copilot might use the detached server.
  We may need to **hide Copilot and Claude CLI from the model picker**.
- Remove the OpenCode route grouping: list the pools directly without any
  grouping or Codex/Claude details. In settings, Copilot or Claude can be
  made visible again, and provider groupings re-enabled.
