# Refusal-triggered failover lane: design sketch

Issue: [#14234](https://github.com/developing-today/code/issues/14234)
(acceptance criterion 3 of [#3150](https://github.com/developing-today/code/issues/3150)).
Companion: [developing-today-forks/opencode#1084](https://github.com/developing-today-forks/opencode/issues/1084) (closed; content-policy rejections never touch route health).
Status: **design sketch, not an authorization to implement.** No code,
config, seating, account, or purchase is part of this record.

Read against: `developing-today-forks/opencode` main `5b851fb9b7`, and
`developing-today/code` main `8bc29d7c`.

## 0. What already exists (so the sketch builds on it)

#3150's triage notes (2026-11-09, 2026-11-17) describe the router as
having no content-policy class. That is still true of the
`bin/opencode-route-proxy.mjs` in this repo, whose `classifyHttp` still
maps everything outside 401/402/403/429 to `ProviderInternal`. The
session router in the fork is where routing decisions are made now, and it already does
most of what #1084 asked for:

| Concern | Where it lives in the fork | Behaviour today |
|---|---|---|
| Classifying a refusal | `packages/ai/src/provider-error.ts` `classifyProviderFailure` | A refusal becomes `ContentPolicyError` (`_tag: "ContentPolicy"`) when the error carries a known code (`CONTENT_POLICY_CODES`), or when a 4xx or mid-stream message matches `CONTENT_POLICY_TEXT`. Protocol adapters add their own codes too: MiniMax 1026/1027, Gemini `SAFETY`/`PROHIBITED_CONTENT` finishes, xAI `respect_moderation`, OpenRouter video. |
| A refusal delivered as a finish | `session/runner/step.ts`, `policyFinish` | A `content-filter` finish with no output is recorded as `ContentPolicy`, never as a success. |
| Route health | `step.ts` (the `!policyFailure` gate on `ModelRoute.failed`), `quotaFailureClassification`, and `model-route-log.ts` | A refusal never counts as a failure, a quota event, a cooldown or a demotion. It is counted separately as `policyRejections`, and any half-open probe claim is released. |
| Same-request failover | `step.ts`, where `policyFailure` joins the failover gate and `policyFinishError` goes to `routing.fallback()` | A refusal with no output yet moves the request to the next member of the same route's pool. It is never retried on the same target. |
| Terminal error | `session/to-session-error.ts` | When the pool runs out, the user sees `provider.content-filter`. |

So the gap #3150 left open is narrow. When **every seated candidate on
the route refused this request on policy grounds**, the request fails
with `provider.content-filter`, and there is no special route to try
once.

## 1. Trigger classification

**Rule:** the lane may fire only on an attempt whose recorded reason is
`ContentPolicy`. It must never fire on a generic 400, 403, 422 or an
unclassified `InvalidRequest`. The classifier above is the single
source of truth, and the lane adds no matching of its own.

Evidence per provider family, all already encoded in the classifier:

| Family | Evidence that counts | Must **not** count |
|---|---|---|
| OpenAI / Azure OpenAI | `content_filter`, `ResponsibleAIPolicyViolation` (inner code), `moderation_blocked`, `cyber_policy`, `refusal`; `invalid_prompt` only when its message says "violating our usage policy" | A bare `invalid_prompt` (Bedrock Mantle reuses it for schema errors); a 403 for region or org access |
| Anthropic | `invalid_request_error` only with refusal text matching `CONTENT_POLICY_TEXT`; a `refusal` stop reason before any output | Any other `invalid_request_error` (schema, size) |
| Google (Gemini, Vertex) | finish `SAFETY`, `PROHIBITED_CONTENT`, `BLOCKLIST`, `IMAGE_SAFETY`; a 400 carrying those codes | A 400 for an invalid argument |
| OpenRouter / Vercel gateway | typed `error_type` content-policy codes; a gateway `[code]` label naming one | `invalid_request` with no policy code or text |
| Alibaba Model Studio | `DataInspectionFailed` | Other 400s |
| MiniMax | 1026, 1027 | Other status codes |

Two tightening changes belong in the implementation slice:

1. **A refusal must be unanimous across distinct providers.** The lane
   fires only when the request's failover walk ended with every attempt
   tagged `ContentPolicy`, and those refusals came from at least two
   distinct provider families. A single provider's filter, or an
   exhausted pool that mixed refusals with other failures, takes the
   normal path. This stops one over-eager filter, or a misclassified
   400, from sending work off-pool.
2. **Text-only matches need a 4xx status or a mid-stream frame.** This
   is already true (`clientScoped`). The lane keeps it as a stated
   invariant with a test, so a 5xx whose body happens to mention a
   policy can never trigger the lane.

## 2. Per-request failover mechanics

**Shape:** the lane is a route head of its own (working name
`opencode-route/refusal-lane`). It is reachable **only** from the
refusal branch. It is never a default, never a role default, never a
member of another route's pool, and never chosen by the resolver for a
fresh request. This is Decision 008 read literally: roles state
requirements, and this route's requirement is "the seated models
refused this request on content-policy grounds".

Mechanics, as hooks into the existing step runner:

1. **Where it hooks.** `step.ts` already calls `routing.fallback()` for
   `policyFailure` and `policyFinishError`. Today, when `fallback()`
   returns nothing, the step falls through to the terminal
   `provider.content-filter`. The new branch sits exactly there: when
   the fallback is empty and the unanimity check in §1 holds, resolve
   the lane's route head once and return `Outcome.Failover` to it.
2. **Once per request.** The lane's attempt carries a marker
   (`overTo: "refusal-lane:<target>"`) in the failover chain. A refusal
   or any failure on the lane ends the request with the original
   `provider.content-filter`, and the user also sees that the
   alternative route declined. The lane's own pool is never walked into
   a second provider for the same request. That keeps the exposure to
   one third party per refused request.
3. **No output started.** The existing `!recorded.outputStarted` gate
   holds. A refusal after partial output never reaches the lane,
   because another model's output must not be appended to a published
   reply.
4. **State preserved.** The failover re-issues the same step with the
   same conversation, tool definitions and pending tool results, as
   every `Outcome.Failover` does today. No summarisation and no
   transcript rewrite happen, so the lane model sees the request
   exactly as the refusing model did.
5. **Capability floor.** The lane's target must declare tool calling
   when the request carries tools. A text-only lane target (Venice
   Uncensored 1.2 until its tool test passes) is skipped for a tool
   request, and the request ends as today. Being skipped is never a
   reason to fall back to a weaker target.
6. **Opt-in, default off.** The lane is configured per Location, with a
   setting such as `experimental.refusal_lane: { route, enabled }` that
   defaults to `enabled: false`. With the setting off, behaviour is
   exactly what ships today.

## 3. Telemetry

- **Rows:** the lane attempt is an ordinary `route_attempt` row. It
  adds two fields: `trigger = 'content-policy'` and the
  `failover_from` target. The refusals that led to it keep
  `error_tag = 'ContentPolicy'`, so the existing `policyRejections`
  counters and the #2795 rollup report them unchanged.
- **Health isolation:** lane rows are excluded from the pool health,
  demotion and quota predicates that judge seated routes. A lane
  target's own failures may bench the lane target, but they never
  touch the refusing pool. The #1084 guarantee therefore holds in both
  directions.
- **What is never logged:** no request or response bodies. The lane
  forces `captureBody` off for its own attempt, whatever
  `captureErrorBodies` says, because the refused content is the
  sensitive part. Allowed fields are the sanitized metadata: time,
  route, provider and model ids, the trigger, the `failover_from`
  target, the outcome, the error tag, token counts, cost, and the
  `credentialKey` digest. Raw credential ids are never logged.
- **Read side:** the rollup endpoint from #2831 can add `trigger` as a
  grouping dimension, so /quota analytics can show how often the lane
  fires, how often it succeeds, and what it costs, without new storage.

## 4. Bounds

**Lawful work only.** The lane exists for legitimate work that
mainstream filters over-refuse: security research, reverse
engineering, mature creative work, and controversial-topic analysis.
It does not widen what the fleet will do. Agent instructions and the
fleet's own policies still apply on the lane exactly as on any route.
A request that is unlawful is not made lawful by a different model.

**Privacy and trust (#3150 Q5).** Refused work is often refused
precisely because it is sensitive. Before any pilot:

- The lane provider's current DPA, retention, training-use and
  subprocessor terms are read and recorded in the pilot record.
  The #3150 ranking (2026-11-18) records the providers' own claims:
  Abliteration says prompts and responses are transient, not stored and
  not used for training, with metadata retained; Venice says it has
  zero data retention. Both claims need checking against the contract
  that actually applies.
- Requests that carry credential material are never forwarded. The
  existing error-body redaction patterns run over the outbound request
  as a gate. A match ends the request as a refusal instead of
  forwarding it.
- Sessions can be marked lane-ineligible: for example, any session
  under a client or employer workspace by default, because their
  information may not leave the seated providers.

**Benchmark plan (gates any seating).** It runs on the two candidates
the #3150 ranking named:

| Candidate | Why it is in the plan |
|---|---|
| Abliteration `abliterated-model-large-flash` | The only hosted candidate with documented tool calling and JSON Schema output (rank 1, conditional) |
| Venice `venice-uncensored-1-2` | Cheapest, with zero retention claimed; tool support unverified (rank 2, text-only until it passes the tool test) |

The task set is drawn from real fleet refusals. A collection run
replays the last 30 days of `ContentPolicy` rows by metadata only, and
the operator supplies sanitized prompts for 10 to 20 of them. The set
adds a multi-step tool task with a malformed tool call injected, a
long-context task above 100k tokens for Abliteration, and a structured
output task. Measures:

- task completion
- tool-call validity rate
- refusal rate on the lane itself
- time to first token and total latency
- cost per task

**Pass bar:** at least 80 % task completion, at least 95 % valid tool
calls, and a lane refusal rate under 10 %.

**Pilot-approval gate (downstream prerequisite, named, not performed).**
Drew's explicit approval is required, after the benchmark and the terms
review, for each of:

1. creating any provider account and adding a payment method
2. seating the lane route head in the deployed config
3. setting `enabled: true` on any Location

None of these are done here.

## 5. Implementation slices (each its own issue, once approved)

1. **Unanimity and invariant tests:** add the distinct-provider
   unanimity check to the policy-exhaustion branch, and tests that a
   5xx never classifies as `ContentPolicy`. This is behaviour-neutral
   with the lane off.
2. **Lane route head and config:** the `refusal_lane` setting (default
   off), the route head that the resolver never offers, and the
   once-per-request failover with the capability floor.
3. **Telemetry fields:** `trigger` and `failover_from` on
   `route_attempt`, health-predicate exclusion, forced no-body capture,
   and a rollup dimension.
4. **Outbound credential gate:** run the redaction patterns as a
   forwarding gate.
5. **Benchmark harness and pilot record:** run under the pilot gate.

Slices 1, 3 and 4 are safe to land before any approval, because they
change nothing while the lane is off. Slices 2 and 5 wait for the
pilot gate.
