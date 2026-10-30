// The mcpx home-page example (#1876, Approach C stage 2).
//
// This is the script the home page shows, lets the reader edit, and runs in
// the page against the fake daemon (fake-daemon.js) through the real
// generated client (client.ts / client.browser.js). It is the same shape as
// examples/call-a-tool.ts: nothing is imported, `tools` is on globalThis,
// one call, print the result.
//
// The `demo` namespace is the deterministic fixture server
// (internal/testsupport/fakemcp) — a mock, and the page says so. Keep this
// script in erasable TypeScript only (no enums, namespaces, or parameter
// properties): the in-page strip-only transform (strip-types.js) has to be
// able to run the reader's edits, same as Node's strip-only mode runs the
// generated client.

const echoed = await tools.demo.echo({ message: "hello from a script" });

console.log("demo.echo:", String(echoed));
