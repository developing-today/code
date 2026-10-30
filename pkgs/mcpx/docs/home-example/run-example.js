// Run the (possibly edited) home-page example in the page (#1876 stage 2).
//
// The pieces, in order:
//   1. the fake daemon is installed (fake-daemon.js) so the generated
//      client's POST /v1/call lands in the in-page mocks;
//   2. the real generated client's `tools` object (client.browser.js —
//      client.ts after the build-time strip, see generate-client.sh) is
//      put on globalThis, the same surface `mcpx run` gives a script;
//   3. the reader's source goes through the strip-only transform
//      (strip-types.js) and is imported as a blob module, so top-level
//      await works exactly as it does under mcpx;
//   4. console.log output is captured line by line into the output pane
//      for the duration of the run, then everything is restored.
//
// Errors — a ToolError from a mock, a strip-types refusal, a syntax error
// in an edit — are reported through onLine as `error: ...` lines, the way
// a terminal run would print them, not swallowed.

import { installFakeDaemon } from "./fake-daemon.js";
import { stripTypes } from "./strip-types.js";

function format(value) {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.message;
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

// Runs are serialized on a module-level chain: a run replaces shared page
// globals (fetch, tools, console.log) and restores them at the end, so two
// overlapping runs would restore each other's swaps mid-flight. Callers
// also disable the Run button, but the invariant lives here so no caller
// can break it.
let runTail = Promise.resolve();

export function runExample(source, clientTools, onLine) {
  const run = runTail.then(() => runExampleOnce(source, clientTools, onLine));
  runTail = run.catch(() => {});
  return run;
}

async function runExampleOnce(source, clientTools, onLine) {
  const restoreFetch = installFakeDaemon();
  const hadTools = "tools" in globalThis;
  const prevTools = globalThis.tools;
  const prevLog = console.log;
  globalThis.tools = clientTools;
  console.log = (...args) => onLine(args.map(format).join(" "));
  try {
    const code = stripTypes(source);
    const url = URL.createObjectURL(
      new Blob([code], { type: "text/javascript" }),
    );
    try {
      await import(/* @vite-ignore */ url);
    } finally {
      URL.revokeObjectURL(url);
    }
  } catch (e) {
    onLine("error: " + format(e));
  } finally {
    console.log = prevLog;
    if (hadTools) globalThis.tools = prevTools;
    else delete globalThis.tools;
    restoreFetch();
  }
}
