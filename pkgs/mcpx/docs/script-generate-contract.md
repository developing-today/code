# Script-side generate API contract

Status: design contract for #14183 (split from #1925). Implementation is a
separate packet. Verified 2026-11-24 against `main`.

## Current state (what exists)

- `sampling.answerLocally` is implemented. It answers an upstream MCP
  `sampling/createMessage` with mcpx's own configured model provider
  (`pkgs/mcpx/internal/daemon/sampling_local.go`, wired in
  `internal/daemon/hooks.go`). It is a server-to-mcpx path, not a script call.
- Daemon-side prompt hooks exist. A hook declares `prompt` text with
  `@placeholders`; `effect` `advise` (any phase) and `propose` (outcome phases)
  are implemented (`docs/exec.md`, `internal/execsvc/hooks.go`).
- The prompt effects `ask` and `run` are **not implemented**: `internal/execsvc`
  refuses them with "prompt effect ... is not implemented".
- No script-callable generate function exists. Nothing in the runner exposes
  a model call to a script.

## Questions this contract answers

### 1. Call surface and name, as seen from a script

Proposed: a single function on the mcpx script client, `generate`, reached as
`tools.model.generate(...)` in TypeScript and the equivalent member of the
generated Python client. It lives beside the tool namespaces, not in a new
global, so the same permission model applies: a script can generate only if its
permissions allow the `model` namespace.

It is not `tools.<server>.<tool>`; a model is not an MCP server.

### 2. Which script languages can call it

- TypeScript (the default runtime): yes, first.
- Python (`runner/dispatch.go`, generated Python client installed beside the
  script): yes, second, once the TypeScript shape is proven. The Python client
  is generated, so the signature is defined once in the contract and generated
  for both.
- Shell and other interpreters: no. A shell script has no client and no way to
  enforce the permission profile, so it cannot call `generate`.

### 3. Provider/model selection and model-profile choice

Selection follows the existing `model.*` settings; the script does not pick a
provider by name by default.

- Default: `model.provider` / `model.default` as resolved on the daemon for the
  run (the same resolution `sampling.answerLocally` uses).
- Per call: an optional `model` argument in `provider/model` form. It must be a
  model the run's permission profile allows; otherwise the call is refused, not
  downgraded.
- Profiles: an optional `profile` name from `model.profiles`. A profile sets
  timeout and budget; a `model` argument with no `profile` uses the defaults.
- Precedence: explicit `model` > `profile` > `model.default`.

### 4. Request and response shape

Request (proposed):

```ts
generate({
  prompt: string,          // required, may contain no @placeholders
  system?: string,
  model?: string,          // "provider/model", optional
  profile?: string,        // optional, from model.profiles
  maxTokens?: number,      // capped by prompt.maxTokens and model.budget
  catalog?: boolean,       // include the bounded tool catalog slice
})
```

Response (proposed):

```ts
{
  text: string,
  model: string,           // the provider/model that answered
  tokens: { input: number, output: number },
  truncated: boolean,      // true if maxTokens or the catalog budget cut output
}
```

The response is always the answer text plus which model gave it. The script
never receives raw provider payloads.

### 5. Budgets and timeouts

These are the existing knobs; the contract reuses them and adds none.

| Knob | Default | Scope | Meaning for `generate` |
|---|---|---|---|
| `prompt.catalogBudget` | 1200 | call | approximate token ceiling for the catalog slice sent when `catalog: true` |
| `prompt.maxTokens` | 1500 | call | cap on output tokens for one `generate` call; `maxTokens` in the request may only lower it |
| `prompt.sampleTimeout` | 1m30s | daemon | fallback wait for a generation when `model.timeout` is unset |
| `model.budget` | settings | — | a hard ceiling; `maxTokens` is capped by it (`execsvc.ModelTokenCap`) |

A `generate` call that exceeds its timeout fails with a timeout error and
returns no partial text. It does not retry.

### 6. Error semantics

Errors are typed so a script can branch on them without parsing strings:

- `model.unavailable`: no provider resolved (the same condition as
  `sampling.answerLocally`'s "no model provider is configured").
- `model.refused`: the requested `model` or `profile` is not allowed by the
  run's permissions.
- `model.timeout`: `prompt.sampleTimeout` or `model.timeout` elapsed.
- `model.budget`: the request was capped by `model.budget` and nothing could be
  returned within it.
- `model.provider`: the provider returned an error; the message is the
  provider's, truncated.

A `generate` error is an error in the script, like any thrown exception. It is
caught by the script's own `try`, and an `onError` hook may replace it with a
result as usual. A generation never fails a run silently.

## Verdict

**Ready for an implementation packet, scoped to a TypeScript-only first slice.**
The contract answers all six questions. Three choices were open and are decided
here so the packet does not wait on anyone:

1. `generate` is not allowed from a `before` hook. A model call there has no
   script result to attach to, and `before` hooks cannot change the outcome.
2. `profile` is deferred to slice 2. Slice 1 uses `model.default` and an
   explicit `model` argument.
3. Python is deferred until TypeScript passes. The Python client is generated
   from the same signature, so it follows without a new contract.

Out of scope for slice 1: the `ask` and `run` prompt effects stay not
implemented, and `apply` stays refused, as `docs/exec.md` already states.
