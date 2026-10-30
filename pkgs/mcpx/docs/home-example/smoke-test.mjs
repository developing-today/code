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
import { formatRecordLine, installScriptGlobals } from "./run-example.js";
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

// --- 1b. strip-types is syntax-aware: literals and comments are data -----
{
  check(
    "strip: `as` inside a string literal is preserved",
    stripTypes(`console.log("copy as script");`) ===
      `console.log("copy as script");`,
  );
  check(
    "strip: template text and comments preserved, real casts still stripped",
    stripTypes(
      "const s = `hi ${name as string}`; // keep as is\nconsole.log(s as string);",
    ) === "const s = `hi ${name}`; // keep as is\nconsole.log(s);",
  );
  check(
    "strip: annotation-shaped text inside a string is preserved",
    stripTypes(`const label: string = "value: string = x";`) ===
      `const label = "value: string = x";`,
  );
}

// --- 1c. strip-types: object properties are not parameter annotations ----
{
  const objLine =
    "const expected = 1, actual = 2;\n" +
    "console.log({ first: 0, actual: expected, last: 0 }.actual);";
  check(
    "strip: object property after a comma keeps its value",
    stripTypes(objLine) === objLine,
    stripTypes(objLine),
  );
  check(
    "strip: declarations, arrows and methods still strip",
    stripTypes(
      "function f(a: number, b?: string): void {}\n" +
        "const g = (x: number): number => x;\n" +
        "class C { run(v: string): string { return v; } }",
    ) ===
      "function f(a, b) {}\nconst g = (x) => x;\nclass C { run(v) { return v; } }",
    stripTypes(
      "function f(a: number, b?: string): void {}\n" +
        "const g = (x: number): number => x;\n" +
        "class C { run(v: string): string { return v; } }",
    ),
  );
  check(
    "strip: default-value object literals keep their properties",
    stripTypes("function h(o = { a: 1, b: 2 }, n: number): void {}") ===
      "function h(o = { a: 1, b: 2 }, n) {}",
    stripTypes("function h(o = { a: 1, b: 2 }, n: number): void {}"),
  );
}

// --- 1d. strip-types: regex literals are protected, division is not -------
{
  check(
    "strip: a regex containing annotation-shaped text is preserved",
    stripTypes("console.log(/ as string /.source);") ===
      "console.log(/ as string /.source);",
  );
  check(
    "strip: division still reads as code",
    stripTypes("const x: number = a / b / c;") === "const x = a / b / c;",
  );
  check(
    "strip: a regex after a keyword is preserved, flags included",
    stripTypes("function t(s: string): boolean { return /a: string/i.test(s); }") ===
      "function t(s) { return /a: string/i.test(s); }",
    stripTypes("function t(s: string): boolean { return /a: string/i.test(s); }"),
  );
}

// --- 1e. run-example: script globals + client record lines ------------------
{
  const fakeClient = { log: () => {}, emit: () => {}, describe: () => "" };
  const restore = installScriptGlobals(fakeClient);
  check(
    "run: script globals are installed for the run",
    globalThis.log === fakeClient.log && globalThis.emit === fakeClient.emit,
  );
  restore();
  check(
    "run: script globals are removed afterwards",
    !("log" in globalThis) && !("emit" in globalThis),
  );
  globalThis.log = "page's own";
  const restore2 = installScriptGlobals(fakeClient);
  check(
    "run: a pre-existing global is left alone (installGlobals semantics)",
    globalThis.log === "page's own",
  );
  restore2();
  delete globalThis.log;

  const sentinel = "\u001emcpx\u001e";
  check(
    "run: an emit record renders its value",
    formatRecordLine(sentinel + JSON.stringify({ kind: "result", value: "hi" })) ===
      "hi",
  );
  check(
    "run: a log record renders message and args",
    formatRecordLine(
      sentinel +
        JSON.stringify({ kind: "log", msg: "starting", attrs: { args: [42] } }),
    ) === "starting 42",
  );
  check(
    "run: a non-record console.error line is not a record",
    formatRecordLine("some plain error") === null,
  );
}

// --- 2+3. fake daemon + real generated client -----------------------------
const clientPath = existsSync(join(here, "client.browser.js"))
  ? join(here, "client.browser.js")
  : join(here, "client.ts"); // Node strips its types on import.

async function runSource(source) {
  const lines = [];
  const prevLog = console.log;
  // Braces, deliberately: console.log returns undefined, and a default
  // export that ends in a log call must "return" undefined too — an
  // expression-bodied capture would hand back push()'s count and the
  // runner mirror below would print it as the script's result.
  console.log = (...args) => {
    lines.push(args.map((a) => (typeof a === "string" ? a : String(a))).join(" "));
  };
  try {
    const dir = mkdtempSync(join(tmpdir(), "mcpx-home-run-"));
    const mod = join(dir, "example.mjs");
    writeFileSync(mod, stripTypes(source));
    // Mirror runExampleOnce (run-example.js), which mirrors the real
    // runner: import, then invoke a function default export with an
    // empty argv array and print a non-undefined return value.
    const imported = await import(pathToFileURL(mod).href + `?t=${Date.now()}`);
    if (typeof imported?.default === "function") {
      const value = await imported.default([]);
      if (value !== undefined) {
        console.log(
          typeof value === "string" ? value : JSON.stringify(value, null, 2),
        );
      }
    }
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

    const objLines = await runSource(
      "const expected = 1, actual = 2;\n" +
        "console.log({ first: 0, actual: expected, last: 0 }.actual);",
    );
    check(
      "stage2: object-property example prints the same as the copied command",
      objLines.join("\n") === "1",
      objLines.join("\n"),
    );

    const defLines = await runSource(
      `export default () => console.log("ran via default export");`,
    );
    check(
      "stage2: a default-export edit is invoked like the real runner",
      defLines.join("\n") === "ran via default export",
      defLines.join("\n"),
    );

    const defRetLines = await runSource(`export default () => "returned";`);
    check(
      "stage2: a default export's return value is printed",
      defRetLines.join("\n") === "returned",
      defRetLines.join("\n"),
    );
  } finally {
    delete globalThis.tools;
    restore();
  }
}

process.exit(failures === 0 ? 0 : 1);
