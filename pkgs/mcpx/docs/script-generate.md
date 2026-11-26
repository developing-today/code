# Script-side generate: the API contract

> The one remainder of #1925 that is not implemented: a script-callable
> generation surface. This page is the contract the implementation packet
> builds against. It answers the six questions #14183 names, and records
> the readiness verdict at the end.

## Status

Contract only, settled by lane oc-code-07 for #14183. There is no public
script `generate` global or route today. The existing `model.*` settings
are reused; no new setting is proposed. This replaces the competing
drafts in #14459 and #14472, not the existing model layer.

## What it is not

- **Not a prompt hook.** Daemon-side prompt hooks (`internal/execsvc`,
  `docs/exec.md`) already ask a model about a run, with effects `advise`
  (implemented) and `propose` (implemented), while `ask`, `apply` and `run`
  are refused at validation. Hooks observe runs; this surface lets a script
  ask for text in the middle of its own work.
- **Not `sampling.answerLocally`.** That setting (slice 3 of #1925,
  `internal/daemon/sampling_local.go`) answers _upstream servers'_
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
discriminated result (section 4) — it never throws for a model-side
outcome. Only a missing/non-object argument throws (see section 6).
A generated ambient declaration
(`GlobalDeclarations` in `internal/codegen/emit.go`) carries it the way
`log`, `emit` and `artifact` are carried today, so editors complete it
with no import.

Placement follows the generated-client installer (`installGlobals` in
`internal/codegen/emit.go`), not a synthetic `tools.model` MCP namespace.
The implementation must add one daemon handler and client transport for
it, used by local and daemon execution alike; local launchers must not
make direct provider calls. Local snippets, named scripts and remote runs
must be acceptance-tested against the same handler.

The handler must validate run identity/cancellation and use the daemon's
configured model policy. Existing tool visibility checks do not authorize
this non-tool operation, and `script.permissions` supplies runtime flags,
not provider/model grants. This contract does not invent a namespace or
model allow-list that mcpx does not have. `model.provider=none` refuses
every call, including one with an explicit model/profile. Model selection
never supplies credentials or changes daemon configuration. The generated
client is a convenience, not an authorization boundary or a claim that
arbitrary network-capable scripts are sandboxed.

## 2. Which script languages can call it

The runner has TypeScript/JavaScript, Python and Roc generated clients
(`internal/codegen/language.go`). First implementation slice: only
TypeScript/JavaScript under Deno, Bun and Node. Python and Roc bindings
are explicitly deferred, not promised by TypeScript code generation;
they must later implement this same JSON request/result contract with
language-appropriate async calls. Until then their generated declarations
must not advertise `generate`. Arbitrary shebang interpreters receive no
supported generation binding.

## 3. Provider and model selection, and how a model profile is chosen

Reuse the existing model layer (`internal/model`, settings in
`internal/settings/model.go`), adding request-aware provider resolution
where necessary. Resolve `provider/model` ids against configured endpoints
before invoking a backend; never merely strip a mismatching provider
prefix and send the model to another provider's endpoint. This routing
guarantee is an implementation requirement, not a claim that the current
single-provider resolver already enforces it.

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

1. First refuse `model.provider=none`. Otherwise use the call's `model`
   field; empty falls back to `model.default`.
2. If the name matches a profile (case-insensitive), the profile chain
   runs.
3. Otherwise the id splits on the first `/` into provider and model
   (`SplitModelID`); a bare id belongs to the routed provider.
4. For a bare id, `auto` selects a configured provider (an `apiKeyEnv`
   naming a set variable, or an explicit `baseURL`), otherwise returns
   `unavailable`. It never scans for known key names or installs a
   sampling fallback in this slice.

The scope guard from #1925 binds the surface, not each call:
no mcpx operation may call a model on a path where a deterministic answer
exists — a script calling `generate` is the caller's explicit choice, so
the guard is satisfied once, at the surface, and each call is honoured.

For this first slice the sampling and plugin paths are disabled for
script calls: resolve without either answerer installed. Explicit
selection of either returns `unavailable`; profile/auto selection must
not install them as fallbacks. This avoids interactive sampling per row
and recursive self-sampling. Enabling either later needs its own tested
capability, cancellation and loop contract. Direct configured provider
calls and existing direct-provider profile chains remain in scope.

## 4. Request and response shape

The options type is:

```ts
type GenerateOptions = {
  prompt: string;
  system?: string;
  model?: string;
  maxTokens?: number; // finite integer; nonpositive means unset
  stop?: string[];
  timeout?: string; // positive Go duration, e.g. "20s"; omitted means unset
  schema?: Record<string, unknown>; // JSON-serializable schema object
};
```

Unknown fields return `rejected`, rather than silently ignoring typos.
The options map to the model request and call context as follows:

| script field        | `model.Request`                                  | notes                                                                                                                                                          |
| ------------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `prompt` (required) | `Messages: [{Role: "user", Content}]`            | Non-empty; empty is `rejected`.                                                                                                                                |
| `system`            | `Messages: [{Role: "system", Content}, …]` first | Placeholder substitution is the caller's job; the layer sends text as given.                                                                                   |
| `model`             | `Model`                                          | Profile name or `provider/model` id; empty means `model.default`.                                                                                              |
| `maxTokens`         | `MaxTokens`                                      | Nonpositive means unset; section 5 caps it.                                                                                                                    |
| `stop`              | `Stop`                                           | Honoured where the backend supports it, ignored elsewhere.                                                                                                     |
| `timeout`           | call context deadline                            | Section 5 precedence.                                                                                                                                          |
| `schema` (optional) | `Schema`                                         | JSON Schema hint passed to backends that support structured output; ignored elsewhere. No cross-provider conformance guarantee; validate output in the caller. |

The response is the discriminated union:

```ts
type GenerateResult = { ok: true; text: string; model: string; provider: string } | { ok: false; error: string; errorKind: "unavailable" | "timeout" | "rejected" | "provider" };
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

- The per-call ask is the call's `maxTokens`; nonpositive is _unset_,
  never a value — an unset ask with no ceiling in force leaves the
  provider's own limit in place, which must never be how a configured
  budget ends up.
- Ceilings, tightest wins: the selected profile's `budget`, then
  `model.budget` when positive (0 means no layer-imposed ceiling, so the
  per-call ask stands).
- Deadlines take the earliest of the remaining run deadline, positive
  call `timeout`, and positive `model.timeout` (default 90s). A positive
  profile timeout additionally bounds each attempt; attempts share the
  overall deadline rather than restarting it. Neither a call nor a
  profile can extend the layer/run deadline. Expiry or run cancellation
  is `timeout`, never a partial answer. No automatic client retries;
  configured profile fallback remains within that deadline.

How the existing knobs map:

- `prompt.maxTokens` (default 1500) supplies the ask for existing
  consumer/script synthesis and prompt hooks. Explicit script calls
  bring their own ask, so it does not bound them; the `ModelTokenCap`
  _rule_ is shared, the _knob_ is not.
- `prompt.sampleTimeout` (default 90s) is the broker sampling timeout
  fallback when `model.timeout` is nonpositive. It has no effect on this
  direct-provider slice; sampling is refused as described above.
- `prompt.catalogBudget` (default 1200) bounds the catalog used in
  consumer script synthesis (`internal/daemon/consumer.go`), not a
  general generation token budget. A script call sends no automatic
  catalog, so this knob does not apply.
- `model.budget` / `model.timeout` are the layer-wide bounds and apply to
  every script call through the layer.

## 6. Error semantics

A model-side outcome is a value (`ok: false`), never a throw — the
prompt-hook precedent is that model unavailability must not fail work,
and a script that cannot branch on `unavailable` will wrap every call in
try/catch and learn nothing. Only a missing or non-object options argument
throws `TypeError`. All field-validation failures, including malformed
schema, return `ok:false, errorKind:"rejected"` before contacting a provider.

`errorKind` values:

- `unavailable` — `model.ErrUnavailable`: none configured
  (`model.provider=none`, the default), the named backend has no path
  installed (sampling/plugin with no answerer), a named provider is not
  configured, a profile is empty, or no default exists and the call named
  no model. Also the disabled sampling/plugin paths in section 3. The `error`
  text names the reason, so a misconfigured provider is
  distinguishable from none configured.
- `timeout` — the call, profile, or layer deadline expired; the provider
  is treated as unavailable for that call and the caller falls back to
  its own fallback if it chooses. No fallback script code is run implicitly.
- `rejected` — the request itself is invalid: empty `prompt`, malformed
  model id, invalid timeout, or malformed optional fields. Nothing was
  sent anywhere.
- `provider` — reached but failed: HTTP errors (402 included, with the
  profile chain stopped), unreachable endpoints, empty output, or an
  invalid provider protocol response. A generated answer need not be
  JSON. No client retry is automatic; transient errors may succeed on a
  later caller-initiated attempt. Failure text must not expose credentials
  or raw provider payloads. Client-to-daemon transport failures use
  `provider` too (or `timeout` on deadline/cancellation); they do not throw.

The implementation must preserve typed profile-attempt failures rather
than classify from error text. On exhaustion, an expired overall deadline
returns `timeout`; otherwise the last attempted provider failure determines
`timeout` or `provider`. Return `unavailable` when no configured backend
could be attempted. Successful fallback still returns success, and HTTP
402 still stops immediately. The existing profile router wraps exhausted
chains in `ErrUnavailable`; preserving these causes is required work, not
existing behavior.

No generated text is returned on failure and no tool is executed by
`generate`. Provider requests can incur charges even if a timeout or
transport error prevents delivery. A model-side failure leaves the script
able to choose its own fallback; it does not guarantee deterministic model
output or undo provider-side work.

## Readiness verdict

The implementation packet is **ready**, scoped to TypeScript/JavaScript
and configured direct providers. No ratification or owner decision is
needed. The three draft conflicts are settled:

1. Use the generated `generate` global, not `tools.model`: globals are
   already installed, whereas tool namespaces are registry-backed and do
   not supply imaginary model permissions.
2. Return discriminated results for model outcomes, intentionally unlike
   `ToolError` throws from tool calls. This optional model operation uses
   the hook precedent of a recoverable unavailable outcome. Throw only
   for the local invocation/programmer errors listed above.
3. Reuse the token-cap rule and `model.*` policy, not the consumer's
   `prompt.*` asks/catalog knobs. Deadlines cannot be extended by calls.

Implementation done checks: transport and shared local/daemon behavior;
request/result declarations; default/explicit model and profile choice;
provider-qualified models never reaching another provider's endpoint;
typed profile-exhaustion classification;
disabled providers refusing calls; profile 402 stopping fallback; cap
properties (a positive ceiling is never exceeded, including unset asks);
earliest-deadline/cancellation behavior; programmer throws versus all four
model result kinds; and absence of unsupported Python/Roc bindings.
Use fake providers and a controllable clock, not paid live inference, as
the test oracle. `ask`/`run` prompt effects remain out of scope.

Refs #1925, #14183.
