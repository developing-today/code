# Script-side generate: the API contract

> The one remainder of #1925 that is not implemented: a script-callable
> generation surface. This page is the contract the implementation packet
> builds against. It answers the six questions #14183 names, and records
> the readiness verdict at the end.

## Status

Contract only. Nothing here is implemented: there is no `generate` global
in the script client today (`internal/codegen/emit.go`), no route, no
setting. The implementation packet is a separate piece of work once this
contract is settled.

## What it is not

- **Not a prompt hook.** Daemon-side prompt hooks (`internal/execsvc`,
  `docs/exec.md`) already ask a model about a run, with effects `advise`
  (implemented) and `propose` (implemented), while `ask`, `apply` and `run`
  are refused at validation. Hooks observe runs; this surface lets a script
  ask for text in the middle of its own work.
- **Not `sampling.answerLocally`.** That setting (slice 3 of #1925,
  `internal/daemon/sampling_local.go`) answers *upstream servers'*
  sampling requests with the configured provider. This surface is the other
  direction: the script is the asker.
- **Not `internal/scripting`.** That package is mcpx's own script
  synthesis, validation and repair (used by the consumer/intent path), not
  a surface scripts call.

## 1. Call surface and name, as seen from a script

One launcher-installed global, beside `log`, `emit`, `artifact` and
`call`:

```ts
const res = await generate({
  prompt: "summarise these rows in one sentence",
  system: "you answer in lowercase",
  model: "summaries",
  maxTokens: 200,
  timeout: "20s",
  stop: ["\n\n"],
});
if (res.ok) {
  emit({ summary: res.text, via: res.model });
} else {
  log.info("generate unavailable: {error}", res);
}
```

`generate` is async, takes a single options object, and returns a
discriminated result (section 5) — it never throws for a model-side
outcome. Throwing stays reserved for programmer errors the script cannot
branch on (see section 6). A generated ambient declaration
(`GlobalDeclarations` in `internal/codegen/emit.go`) carries it the way
`log`, `emit` and `artifact` are carried today, so editors complete it
with no import.

Placement follows the one-implementation rule (`docs/exec.md`): the
global is installed by the launcher for local runs and by `execsvc` for
daemon runs, over the same generated client. `mcpx exec` snippets, named
scripts (`mcpx run`), and remote runs all see the same function, because
there is one place that installs globals. Parity is structural, not
tested-in.

## 2. Which script languages can call it

Every language the runner harnesses — today TypeScript/JavaScript
(deno, bun, node, per `ExecOptions.runtime`) and Python (the harnessed
client, #1917), with Roc following its codegen target
(`internal/codegen/roc.go`). The surface is one generated-client function
per target, added alongside the existing per-language globals, so a new
harnessed language gets `generate` by implementing the same client
function — the contract does not bless one language's spelling over
another's, only the request/response shapes in sections 4 and 5, which
are JSON on the wire either way.

## 3. Provider and model selection, and how a model profile is chosen

Selection reuses the model layer exactly as it stands (`internal/model`,
settings in `internal/settings/model.go`); no new selection machinery.

- `model.provider`: `auto` | `sampling` | `plugin` | `openai` |
  `anthropic` | `google` | `none` (default). `none` means no operation
  calls a model — the behaviour mcpx has always had.
- `model.default`: the model id used when the call names none, in the
  `provider/model` namespace (`openai/gpt-5`, `ollama/qwen3:8b`). The
  prefix chooses the provider and is stripped before it reaches that
  provider's API. Empty means no default: a call that names no model is
  `unavailable` (section 6), never sent under an invented id.
- `model.profiles`: named ordered fallback chains —
  `{"summaries": {"models": ["openai/gpt-5", "anthropic/claude-sonnet"],
  "budget": 2048, "timeout": "30s"}}`. A call's `model` field may name a
  profile or a `provider/model` id; models are tried in order, an
  unconfigured or failed provider is skipped, a profile budget caps the
  per-call token ask, and the profile timeout bounds each provider
  attempt. An HTTP 402 stops the chain at that provider rather than
  spending through a different account (`PaymentRequiredError`).

Resolution order for one call:

1. The call's `model` field; empty falls back to `model.default`.
2. If the name matches a profile (case-insensitive), the profile chain
   runs.
3. Otherwise the id splits on the first `/` into provider and model
   (`SplitModelID`); a bare id belongs to the routed provider.
4. `auto` prefers a configured provider (an `apiKeyEnv` naming a set
   variable, or an explicit `baseURL`), then the sampling path, then
   fails cleanly — auto never scans the environment for known key names,
   and never parks a request on a sampling question nobody may answer
   ahead of an explicitly configured endpoint.

`model.provider=plugin` resolves only where the daemon installed the
plugin answerer path; elsewhere it is `unavailable`, not a silent
fallback. The scope guard from #1925 binds the surface, not each call:
no mcpx operation may call a model on a path where a deterministic answer
exists — a script calling `generate` is the caller's explicit choice, so
the guard is satisfied once, at the surface, and each call is honoured.

The sampling backend asks the run's *driver* through the broker: the
script spends the driver's model, not the daemon's. A driver that never
declared the sampling capability gets `unavailable`, and the daemon must
never answer its own script's sampling question — reentrancy through the
broker is a loop, and loops are refused, not retried.

## 4. Request and response shape

The script options object maps 1:1 onto `model.Request`:

| script field | `model.Request` | notes |
| --- | --- | --- |
| `prompt` (required) | `Messages: [{Role: "user", Content}]` | Non-empty; empty is `rejected`. |
| `system` | `Messages: [{Role: "system", Content}, …]` first | Placeholder substitution is the caller's job; the layer sends text as given. |
| `model` | `Model` | Profile name or `provider/model` id; empty means `model.default`. |
| `maxTokens` | `MaxTokens` | Nonpositive means unset; section 5 caps it. |
| `stop` | `Stop` | Honoured where the backend supports it, ignored elsewhere. |
| `timeout` | call context deadline | Section 5 precedence. |
| `schema` (optional) | `Schema` | JSON Schema the answer must conform to; passed to backends with structured output, ignored by the rest. |

The response is the discriminated union:

```ts
type GenerateResult =
  | { ok: true; text: string; model: string; provider: string }
  | { ok: false; error: string; errorKind: "unavailable" | "timeout" | "rejected" | "provider" };
```

`model`/`provider` name what actually answered, as the backend reports
it — they may differ from the requested id (the profile router fills
them in when the backend leaves them blank). `text` is the model's reply
verbatim: no fence-stripping (that belongs to script synthesis, not to
generation), no trimming beyond what the backend already did.

## 5. Budgets and timeouts

One rule, already implemented for prompt hooks (`ModelTokenCap` in
`internal/execsvc/prompt_hooks.go`, resolved identically by the daemon
and the CLI): the per-call ask, capped by every applicable ceiling.

- The per-call ask is the call's `maxTokens`; nonpositive is *unset*,
  never a value — an unset ask with no ceiling in force leaves the
  provider's own limit in place, which must never be how a configured
  budget ends up.
- Ceilings, tightest wins: the selected profile's `budget`, then
  `model.budget` when positive (0 means no layer-imposed ceiling, so the
  per-call ask stands).
- The call's `timeout`, when set and positive, bounds that call;
  otherwise the selected profile's `timeout`; otherwise `model.timeout`.
  Expiry is `timeout`, never a partial answer.

How the existing knobs map:

- `prompt.maxTokens` is the prompt-hook per-call ask. Script calls bring
  their own ask, so it does not bound them; the `ModelTokenCap` *rule* is
  shared, the *knob* is not.
- `prompt.sampleTimeout` bounds a sampling question a generation raises;
  it applies to script calls only through the sampling backend, as the
  question's life, exactly as it does for hooks today.
- `prompt.catalogBudget` does not apply: it bounds the tool catalog sent
  to the model with a prompt hook, and a script call sends no catalog.
- `model.budget` / `model.timeout` are the layer-wide bounds and apply to
  every script call through the layer.

## 6. Error semantics

A model-side outcome is a value (`ok: false`), never a throw — the
prompt-hook precedent is that model unavailability must not fail work,
and a script that cannot branch on `unavailable` will wrap every call in
try/catch and learn nothing. Only programmer errors throw: a missing or
non-object argument, an unreadable `schema`, values that fail validation
before any provider is touched.

`errorKind` values:

- `unavailable` — `model.ErrUnavailable`: none configured
  (`model.provider=none`, the default), the named backend has no path
  installed (sampling/plugin with no answerer), a named provider is not
  configured, a profile is empty, or no default exists and the call named
  no model. Also the sampling loop refusal in section 3. The `error`
  text names the reason, so a misconfigured provider is
  distinguishable from none configured.
- `timeout` — the call, profile, or layer deadline expired; the provider
  is treated as unavailable for that call and the caller falls back to
  its deterministic path with a note, the same shape as a hook frame's
  `"advice unavailable: …"`.
- `rejected` — the request itself is invalid: empty `prompt`, unknown
  profile name, malformed model id, negative budget values. Nothing was
  sent anywhere.
- `provider` — reached but failed: HTTP errors (402 included, with the
  profile chain stopped), unreachable endpoints, empty output, invalid
  JSON. Retrying the identical call is expected to fail identically
  until something outside changes.

No partial application: a failed call changes nothing (there is nothing
to change — generation has no side effects), charges nothing mcpx-side
(spend is the provider's affair), and leaves the run continuing
deterministically.

## Readiness verdict

The implementation packet is **ready to write** with two decisions to
ratify, both recorded above as contract rather than fact:

1. The discriminated-result shape over throwing for model-side outcomes
   (section 6). If the packet prefers `ToolError` throws for parity with
   tool calls, it must say why the hook precedent does not apply.
2. The knob mapping in section 5 — script calls share the `ModelTokenCap`
   rule but not the `prompt.*` knobs. If the packet wants one shared
   per-call setting, it must rename and migrate the hook knob rather
   than overloading it.

Open question that is genuinely open: whether the sampling backend needs
a per-call opt-out flag for scripts running under drivers that answer
sampling interactively (a script loop asking a human-backed driver once
per row is a footgun). The contract permits the backend; the packet
should decide whether the first cut includes it or refuses
`provider=sampling` from scripts until the loop guard has a test.

Refs #1925, #14183.
