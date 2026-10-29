# Examples

Four scripts you can run as they are. Each one calls the same deterministic
fixture server, prints something short, and has its output checked into
[`examples/`](../examples) beside it. `go test ./examples/` runs every script
and compares stdout with that file, so an example that stops being true fails
the build instead of sitting here looking plausible.

They are deliberately small. `tools` is on `globalThis`, so none of them
imports anything, and the first one is barely longer than the command that
runs it.

Run them from `pkgs/mcpx`. The first run writes `mcpx-client.ts` and its
declarations beside the script, which is how `mcpx run` works for any file;
the `.gitignore` in that directory keeps them out of git.

## Call one tool

The whole shape: top-level `await`, one argument object, print the result.
`demo.echo` answers with a text block, which arrives as a string.

```ts
// Call one tool and print what it returns.
//
// Nothing is imported: `tools` is already on globalThis.

const echoed = await tools.demo.echo({ message: "hello from a script" });

console.log("demo.echo:", String(echoed));
```

    mcpx run examples/call-a-tool.ts

```
demo.echo: hello from a script
```

## Find a tool from inside the script

`search()` and `describe()` read the generated client, so looking something up
costs no round trip to the server. This one lists the namespace, reads one
tool's signature, then calls two tools — one returning text, one returning
`structuredContent`, which arrives already parsed.

```ts
// Find out what a namespace offers from inside the script, then call two of
// the tools it lists.
//
// `search()` and `describe()` read the generated client, so they cost no
// round trip to the server.

console.log("tools:", Object.keys(tools.demo).join(", "));
console.log(describe("demo.echo").trim());

const echoed = await tools.demo.echo({ message: "two tools" });
const structured = await tools.demo.structured();

console.log("echo:", String(echoed));
console.log("structured:", JSON.stringify(structured));
```

    mcpx run examples/discover.ts

```
tools: boom, chatty, echo, fancy_name, open, slow, state, structured, wipe
demo.echo({ message })
      Echo a message back.
echo: two tools
structured: {"n":42}
```

## State that survives between calls

`open` records a value in the server process and `state` reads it back. The
daemon keeps one process for the session, so a later call sees what an earlier
one did — no local variable involved. The script prints only whether its own
two values are there, because the full list also holds whatever an earlier run
recorded.

`log()` is on `globalThis` too, and writes to the durable log on stderr. The
expected output below is stdout, which is what the test compares.

```ts
// `open` records a value in the server process and `state` reads it back, so
// a later call in the same script can see what an earlier one did.
//
// log() writes to the durable log on stderr; console.log is the script's
// stdout, which is what the expected output below holds.

log("recording two values");

console.log(String(await tools.demo.open({ value: "alpha" })));
console.log(String(await tools.demo.open({ value: "beta" })));

const state = await tools.demo.state();

console.log("alpha recorded:", state.seen.includes("alpha"));
console.log("beta recorded:", state.seen.includes("beta"));
```

    mcpx run examples/stateful.ts

```
opened alpha
opened beta
alpha recorded: true
beta recorded: true
```

## A recipe with parameters

The header declares the holes and the body uses `@name`, which is the whole
recipe convention. `mcpx run` refuses a file like this — it cannot fill the
holes — so it is saved and run with values instead. `times` has a default and
is not passed here.

```ts
// Echo a message a few times through the demo server.
//
// The header declares the holes; `mcpx recipes run` fills them. A plain
// `mcpx run` refuses this file, because @message would reach the runtime
// unfilled.
//
// @param message:string   what to echo
// @param times:number = 2   how many times

for (let i = 0; i < @times; i++) {
  console.log(String(await tools.demo.echo({ message: @message })));
}
```

    mcpx recipes save recipe-echo examples/recipe-echo.ts
    mcpx recipes run recipe-echo message="piped through a recipe"

The `mcpx: recipe recipe-echo` line goes to stderr; the two echoed lines are
stdout:

```
piped through a recipe
piped through a recipe
```

---

Every one of these is executed by `go test ./...` on each change, against
the same fixture server, so the output above is what the scripts print today
rather than what they printed when the page was written. For why the fixture
is a fake and not a real server, see [the FAQ](./faq.md#examples).
