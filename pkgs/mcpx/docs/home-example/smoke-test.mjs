// Smoke test for the home-page example component (#1876 stages 1-2).
//
// Run with Node (>= 22, for node:module type stripping of client.ts if the
// browser build has not been generated yet):
//
//   node smoke-test.mjs
//
// Three checks, no browser needed:
//   1. Stage 1 round-trip: execCommand(source) pasted into a real shell
//      delivers the source to `mcpx exec` verbatim — apostrophes, dollar
//      signs, backticks and a line that reads MCPX_EOF included. A stub
//      `mcpx` captures its argument, standing in for the reader's daemon
//      run (the acceptance check "pasted into a terminal, runs against a
//      real local daemon" is this same command with the real binary).
//   2. Stage 2 end-to-end: the fake daemon + the REAL generated client +
//      example.ts produce the example's expected output.
//   3. The strip-only transform runs a typed edit of the example.
//
// #1941 (site build) should wire this into the docs CI pipeline.

import { execCommand, scriptCommands } from "./copy-command.js";
import { installFakeDaemon } from "./fake-daemon.js";
import { stripTypes } from "./strip-types.js";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
let failures = 0;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
  if (!ok) failures++;
};

// --- 1. copy-as-command round-trips through a shell -----------------------
{
  const dir = mkdtempSync(join(tmpdir(), "mcpx-home-example-"));
  const out = join(dir, "captured.txt");
  writeFileSync(
    join(dir, "mcpx"),
    `#!/bin/sh\nprintf '%s' "$2" > ${JSON.stringify(out)}\n`,
    { mode: 0o755 },
  );
  const source =
    `const s = await tools.demo.echo({ message: "it's $HOME \`whoami\`" });\n` +
    `// MCPX_EOF on its own line must not end the heredoc early\n` +
    `MCPX_EOF\nconsole.log(s);`;
  const cmd = execCommand(source);
  execFileSync("bash", ["-c", cmd], {
    env: { ...process.env, PATH: `${dir}:${process.env.PATH}` },
  });
  const captured = readFileSync(out, "utf8");
  // Command substitution strips trailing newlines; the source has none.
  check("stage1: mcpx exec receives the source verbatim", captured === source);

  const script = scriptCommands("home-example", source);
  check(
    "stage1: script form saves then runs the named script",
    script.includes("cat > .mcpx/scripts/home-example.ts") &&
      script.trimEnd().endsWith("mcpx run home-example"),
  );
}

// --- 2+3. fake daemon + real generated client -----------------------------
const clientPath = existsSync(join(here, "client.browser.js"))
  ? join(here, "client.browser.js")
  : join(here, "client.ts"); // Node strips its types on import.

async function runSource(source) {
  const lines = [];
  const prevLog = console.log;
  console.log = (...args) =>
    lines.push(args.map((a) => (typeof a === "string" ? a : String(a))).join(" "));
  try {
    const dir = mkdtempSync(join(tmpdir(), "mcpx-home-run-"));
    const mod = join(dir, "example.mjs");
    writeFileSync(mod, stripTypes(source));
    await import(pathToFileURL(mod).href + `?t=${Date.now()}`);
  } finally {
    console.log = prevLog;
  }
  return lines;
}

{
  const restore = installFakeDaemon();
  try {
    const { tools } = await import(pathToFileURL(clientPath).href);
    globalThis.tools = tools;

    const example = readFileSync(join(here, "example.ts"), "utf8");
    const lines = await runSource(example);
    check(
      "stage2: in-browser example runs end-to-end against the fake daemon",
      lines.join("\n") === "demo.echo: hello from a script",
      lines.join("\n"),
    );

    const typed =
      `interface Opts { message: string }\n` +
      `type Alias = string;\n` +
      `const opts: Opts = { message: "typed edit" };\n` +
      `const echoed: string = await tools.demo.echo(opts);\n` +
      `console.log("demo.echo:", echoed as string);`;
    const typedLines = await runSource(typed);
    check(
      "stage2: strip-only transform runs a typed edit",
      typedLines.join("\n") === "demo.echo: typed edit",
      typedLines.join("\n"),
    );
  } finally {
    delete globalThis.tools;
    restore();
  }
}

process.exit(failures === 0 ? 0 : 1);
