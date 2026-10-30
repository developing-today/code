// Run the (possibly edited) home-page example in the page (#1876 stage 2).
//
// The pieces, in order:
//   1. the fake daemon is installed (fake-daemon.js) so the generated
//      client's POST /v1/call lands in the in-page mocks;
//   2. the real generated client's `tools` object (client.browser.js —
//      client.ts after the build-time strip, see generate-client.sh) is
//      put on globalThis, along with the client's standard script
//      helpers (log, emit, call, ...) — the same surface `mcpx run`
//      gives a script through installGlobals();
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

/**
 * The standard script globals the real runner installs before importing
 * a script (internal/runner/runner.go: installGlobals() from the
 * generated client). `tools` is handled separately by the caller; these
 * are the rest of the named helpers. Namespace objects stay behind
 * `tools.*` — the flattening installGlobals also does (a bare `demo`
 * global per namespace) is deliberately not reproduced on a docs page.
 */
export const SCRIPT_GLOBALS = [
  "log",
  "emit",
  "call",
  "batch",
  "readResource",
  "artifact",
  "ToolError",
  "paths",
  "here",
  "hereDir",
  "releaseConsole",
  "captureFrames",
  "errorFrames",
  "search",
  "transport",
  "describe",
];

/**
 * Install the client's script helpers on globalThis for one run, the
 * installGlobals way (a name already present is left alone), and return
 * a restore function that removes exactly what was installed.
 */
export function installScriptGlobals(client) {
  const installed = [];
  for (const name of SCRIPT_GLOBALS) {
    if (client?.[name] === undefined) continue;
    if (name in globalThis) continue;
    globalThis[name] = client[name];
    installed.push(name);
  }
  return () => {
    for (const name of installed) delete globalThis[name];
  };
}

// The client's log/emit records reach the page as console.error lines
// prefixed with this sentinel (client.ts: LOG_SENTINEL + JSON, through
// writeStderr's browser fallback). The sentinel must match the client's.
const RECORD_SENTINEL = "\u001emcpx\u001e";

/** Render one client record line for the output pane, or null if the
 *  line is not a client record. */
export function formatRecordLine(line) {
  if (typeof line !== "string" || !line.startsWith(RECORD_SENTINEL)) return null;
  let record;
  try {
    record = JSON.parse(line.slice(RECORD_SENTINEL.length));
  } catch {
    return null;
  }
  if (record.kind === "result") return format(record.value);
  if (record.kind === "log") {
    const attrs = record.attrs ?? {};
    const { args, ...rest } = attrs;
    const parts = [String(record.msg ?? "")];
    for (const a of args ?? []) parts.push(format(a));
    const keys = Object.keys(rest);
    if (keys.length > 0) {
      parts.push(keys.map((k) => `${k}=${format(rest[k])}`).join(" "));
    }
    return parts.filter((x) => x !== "").join(" ");
  }
  return null;
}

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

export function runExample(source, client, onLine) {
  const run = runTail.then(() => runExampleOnce(source, client, onLine));
  runTail = run.catch(() => {});
  return run;
}

async function runExampleOnce(source, client, onLine) {
  const restoreFetch = installFakeDaemon();
  const hadTools = "tools" in globalThis;
  const prevTools = globalThis.tools;
  const prevLog = console.log;
  const prevError = console.error;
  globalThis.tools = client?.tools;
  const restoreGlobals = installScriptGlobals(client);
  console.log = (...args) => onLine(args.map(format).join(" "));
  // The client's log/emit helpers write their records through
  // console.error (see formatRecordLine); surface those in the pane
  // and let genuine errors through to the page's own console.
  console.error = (...args) => {
    const rendered = args.length === 1 ? formatRecordLine(args[0]) : null;
    if (rendered !== null) onLine(rendered);
    else prevError(...args);
  };
  try {
    const code = stripTypes(source);
    const url = URL.createObjectURL(
      new Blob([code], { type: "text/javascript" }),
    );
    try {
      const mod = await import(/* @vite-ignore */ url);
      // Mirror the real runner (internal/runner/runner.go): a default
      // export is the program's main. Invoke it with the (empty) argv
      // array and print a non-undefined return value — an edit shaped
      // as `export default () => ...` must behave in the simulation
      // exactly as the copied command behaves against a real daemon.
      const entry = mod?.default;
      if (typeof entry === "function") {
        const value = await entry([]);
        if (value !== undefined) {
          console.log(
            typeof value === "string" ? value : JSON.stringify(value, null, 2),
          );
        }
      }
    } finally {
      URL.revokeObjectURL(url);
    }
  } catch (e) {
    onLine("error: " + format(e));
  } finally {
    console.log = prevLog;
    console.error = prevError;
    if (hadTools) globalThis.tools = prevTools;
    else delete globalThis.tools;
    restoreGlobals();
    restoreFetch();
  }
}
