// Smoke test for the home-page example component (#1876 stages 1-3).
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
//   4. Stage 3 end-to-end: the example runs through the
//      local-daemon transport against a local-daemon test double (a
//      plain HTTP server speaking POST /v1/call with CORS headers,
//      standing in for the reader's daemon), and a dead address
//      produces the clear no-daemon message instead of a raw error.
//      The client here is a stub whose tools.demo.echo makes the one
//      call the real generated client makes (POST /v1/call, see
//      internal/codegen/emit.go); the real client itself is exercised
//      by check 2, and the daemon's CORS behaviour by the Go tests in
//      internal/daemon/cors_test.go.
//
// #1941 (site build) should wire this into the docs CI pipeline.

import { execCommand, scriptCommands } from "./copy-command.js";
import { installFakeDaemon } from "./fake-daemon.js";
import { formatRecordLine, installScriptGlobals } from "./run-example.js";
import { installLocalDaemon } from "./local-daemon.js";
import { stripTypes } from "./strip-types.js";
import { execFileSync } from "node:child_process";
import { createServer } from "node:http";
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

// --- 1d2. strip-types: literals inside a construct do not split it ----------
{
  const greet =
    'function greet(name: string = "world"): string { return name; }\n' +
    "console.log(greet());";
  check(
    "strip: a string default does not split the parameter list",
    stripTypes(greet) ===
      'function greet(name = "world") { return name; }\nconsole.log(greet());',
    stripTypes(greet),
  );
  check(
    "strip: a comment inside a parameter list does not split it either",
    stripTypes("function f(a: number, /* units */ b: string): void {}") ===
      "function f(a, /* units */ b) {}",
    stripTypes("function f(a: number, /* units */ b: string): void {}"),
  );
}

// --- 1d3. strip-types: an assertion never eats its literal operand ----------
{
  check(
    "strip: a cast on a string literal keeps the literal",
    stripTypes('console.log("hello" as string);') ===
      'console.log("hello");',
    stripTypes('console.log("hello" as string);'),
  );
  check(
    "strip: a cast in a declaration keeps the value",
    stripTypes('const message = "hello" as string;') ===
      'const message = "hello";',
    stripTypes('const message = "hello" as string;'),
  );
}

// --- 1d4. strip-types: declarations are removed whole, literals included ---
{
  check(
    "strip: a string-literal union type alias leaves nothing behind",
    stripTypes('type Mode = "fast" | "slow";\nconsole.log("fast");') ===
      '\nconsole.log("fast");',
    stripTypes('type Mode = "fast" | "slow";\nconsole.log("fast");'),
  );
  check(
    "strip: an interface with a literal union member is removed whole",
    stripTypes('interface Cfg { mode: "fast" | "slow" }\nconst x = 1;') ===
      "\nconst x = 1;",
    stripTypes('interface Cfg { mode: "fast" | "slow" }\nconst x = 1;'),
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
  check(
    "run: a log template is interpolated and its attrs consumed",
    formatRecordLine(
      sentinel +
        JSON.stringify({
          kind: "log",
          msg: "fetched {count} pages",
          template: "fetched {count} pages",
          attrs: { count: 3 },
        }),
    ) === "fetched 3 pages",
  );
  check(
    "run: escaped braces render literally, missing keys stay, leftovers append",
    formatRecordLine(
      sentinel +
        JSON.stringify({
          kind: "log",
          msg: "use {{x}} for {missing}",
          template: "use {{x}} for {missing}",
          attrs: { other: 1 },
        }),
    ) === "use {x} for {missing} other=1",
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

    const greetLines = await runSource(
      'function greet(name: string = "world"): string { return name; }\n' +
        "console.log(greet());",
    );
    check(
      "stage2: an edit with a string default parameter runs",
      greetLines.join("\n") === "world",
      greetLines.join("\n"),
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

// --- 4. stage 3: local-daemon transport against a test double -------------
{
  // The test double: the reader's daemon, reduced to the one route
  // the example exercises plus the CORS headers the real daemon
  // answers with for an allow-listed origin (internal/daemon/cors.go).
  const double = createServer((req, res) => {
    const cors = {
      "access-control-allow-origin": req.headers.origin ?? "*",
    };
    if (req.method === "OPTIONS") {
      res.writeHead(204, {
        ...cors,
        "access-control-allow-methods": "POST",
        "access-control-allow-headers": "content-type",
      });
      res.end();
      return;
    }
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      let parsed = {};
      try {
        parsed = JSON.parse(body || "{}");
      } catch {
        /* answered as an unknown call below */
      }
      res.writeHead(200, { "content-type": "application/json", ...cors });
      if (parsed.server === "demo" && parsed.tool === "echo") {
        res.end(
          JSON.stringify({
            result: {
              content: [
                { type: "text", text: String(parsed.args?.message ?? "") },
              ],
            },
          }),
        );
      } else {
        res.end(JSON.stringify({ error: "test double: unknown call" }));
      }
    });
  });
  await new Promise((resolve) => double.listen(0, "127.0.0.1", resolve));
  const daemonBase = `http://127.0.0.1:${double.address().port}`;

  // A stub client whose demo.echo makes exactly the generated
  // client's hot-path call and unwraps the text content the way the
  // real client does.
  const stubClient = {
    tools: {
      demo: {
        echo: async ({ message }) => {
          const resp = await fetch("http://mcpx.invalid/v1/call", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({
              server: "demo",
              tool: "echo",
              args: { message },
            }),
          });
          const out = await resp.json();
          if (out.error) throw new Error(out.error);
          return out.result.content.map((c) => c.text ?? "").join("");
        },
      },
    },
  };

  // runSource mirrors runExampleOnce (which creates a blob: module --
  // a browser-only seam), so the stage-3 checks drive the transport
  // through the same mirror, with installLocalDaemon standing where
  // runExample would install it for transport: "local".
  const example = readFileSync(join(here, "example.ts"), "utf8");
  globalThis.tools = stubClient.tools;
  try {
    const restore = installLocalDaemon(daemonBase);
    try {
      const lines = await runSource(example);
      check(
        "stage3: example runs against the local-daemon test double",
        lines.join("\n") === "demo.echo: hello from a script",
        lines.join("\n"),
      );
    } finally {
      restore();
    }

    const restoreDead = installLocalDaemon("http://127.0.0.1:1");
    try {
      let message = "";
      try {
        await runSource(example);
      } catch (e) {
        message = String(e?.message ?? e);
      }
      check(
        "stage3: no daemon at the address gives the clear message",
        message.includes("no mcpx daemon answered"),
        message,
      );
    } finally {
      restoreDead();
    }
  } finally {
    delete globalThis.tools;
    double.close();
  }
}

process.exit(failures === 0 ? 0 : 1);
