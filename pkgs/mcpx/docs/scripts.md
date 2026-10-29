# Scripts

<a id="script"></a>

## Script

A script is TypeScript (or JavaScript) that calls tools through
the [generated client](./interfaces.md#client-interfaces): no MCP
boilerplate, no schemas in context, just `tools.<ns>.<fn>(args)`
— or the namespace as a bare identifier inside `mcpx exec`. This
is the page a script writer lives on: the contract a script runs
under, where named scripts live, which runtime runs them, and how
to check one before it runs. The full contract is also stated in
`AGENTS.md`, "Script contract"; this page adds the mechanics
around it.

### The contract

- **stdout is the result; stderr is diagnostics.** Use the `log`
  helper rather than `console.error` for diagnostics:
  `log.info("fetched {count} pages", { count: 3 })` — a message
  may carry `{placeholders}` filled from the attributes, and
  `log(msg)` is `log.info(msg)`.
- **A default export is the entry point.** If a script has one,
  mcpx calls it with argv and prints what it returns. Otherwise
  top-level code runs. `--export <name>` calls a named export
  instead, spreading the arguments into it.
- **`emit()` streams.** Each value passed to `emit()` is written
  as it is produced; streaming and returning combine — the
  streamed values are progress, the return value is the answer.
- **`--json` gives one envelope.** `mcpx --json run <script>`
  returns one document with the result, the logs, the exit code
  and the duration — the form another program should consume.
- **Results arrive unwrapped.** Structured output is already
  parsed, JSON-in-text is already parsed, plain text is a string.
  `.raw` holds the full MCP envelope (that is where image bytes
  live). A tool returning `isError` throws `ToolError`.

```ts
import tools, { log } from "./mcpx-client.ts";

export default async function main(args: string[]) {
  const pages = await tools.chrome_devtools.list_pages({});
  log.info("fetched {count} pages", { count: pages.length });
  return pages;
}
```

Scripts need no imports at all inside `mcpx exec`: `tools`,
`log`, `emit` and every namespace are already globals. Relative
paths in a script resolve against **your** working directory,
not the script's.

### Named scripts

A named script lives at `.mcpx/scripts/<name>.ts`, searched
upward from the working directory, then
`~/.config/mcpx/scripts/`. The nearest one wins.

```sh
mcpx scripts              # list what exists
mcpx run <name> [args…]   # run one
mcpx run ./path/to.ts     # a path is used verbatim
```

`mcpx scripts` lists shadowed scripts too — a nearer script of
the same name hides a farther one, and the listing marks which
is which rather than letting the hidden one surprise you.
Arguments after the name arrive in `Deno.args` on Deno; on every
runtime, a default export is called with argv, which is the
portable way to read them.

### Runtimes

`script.runtime` (`--runtime`) chooses; the default order is
**deno, then bun, then node** — the first in
`script.runtimeOrder` that is installed _and_ can enforce the
permission profile asked for. A runtime that cannot enforce the
profile is skipped under `auto`, never silently weakened.
[runtimes.md](./runtimes.md) is the full page on choosing,
declaring and constraining runtimes; the two limits a script
writer hits first:

- **Node transforms TypeScript; it does not merely strip it.**
  Node is invoked with `--experimental-transform-types`, chosen
  over strip-only mode precisely because strip mode rejects
  non-erasable TypeScript — `enum`, value namespaces, parameter
  properties — that Node itself runs fine once types are
  transformed. An `enum` in a script is guarded by a test
  (`TestNodeRunsNonErasableTypeScript`), not a limitation.
- **Typechecking is Deno-only.** `--typecheck` /
  `script.typecheck` runs the check (via `deno check`) before
  the run and only applies when Deno is the runtime.

### Authority

Scripts run with **your full permissions**: a real runtime, the
real filesystem, the network, subprocesses. `script.permissions`
names narrower permission profiles, but a profile is only as
real as the runtime enforcing it — Deno enforces them, node
enforces some, bun enforces none — and under `auto` a runtime
that cannot enforce the profile asked for is skipped rather
than trusted. The details and the profile table are in
[runtimes.md](./runtimes.md). The short version is the FAQ's
answer to "is it safe?": no — treat a script like a shell
command you are about to paste.

### Where a script runs

`exec.where` is `auto`, `local` or `remote` (`--local`,
`--remote`). Auto runs locally when the daemon is on this
machine and on the daemon when it is not — a person at a
terminal gets a terminal-shaped run, and a configured remote
endpoint gets a remote one. How artifacts (a screenshot a script
produced, say) are delivered from each arrangement is
[exec.md](./exec.md)'s subject.

### Checking before running

- **`mcpx diagnose`** checks a script before it runs — most
  valuably against the daemon's record of what each tool's
  schema _used to be_, so a call broken by a schema change is
  reported as two facts ("the argument became required; you
  wrote none") rather than as a bare failure at run time.
- **Automatic preflight** (`diagnose.preflight`) runs those
  checks on `exec` source before it executes. A diagnostic is
  advice with a name: `--diagnose-preflight=false` runs the
  script anyway.

### Recipes, and onward

A **recipe** is a named script that declares its own holes in
its header:

```
// @param repo:string   which repository
// @param days:number = 30   how old counts as stale
```

`mcpx recipes` lists them; `mcpx recipes run <name> key=value…`
fills each `@name` reference in the body with the value, encoded
as JSON, so a recipe cannot become an injection site. Values not
supplied and without a default are asked for as one form.
`mcpx recipes match '<words>'` ranks recipes against free text
deterministically — a score over names, descriptions, parameters
and the tools called; no model, no cost, the same answer every
time.

Onward from here: `mcpx prompt` turns a request in words into a
script (trying recipes first, and returning the script rather
than running it unless `--run` is given, because generated code
runs with your credentials); artifacts a script produces are
covered in [exec.md](./exec.md); and stateful servers — why two
calls in one invocation share a browser and two invocations may
not — are under "Stateful servers" in `AGENTS.md`.
