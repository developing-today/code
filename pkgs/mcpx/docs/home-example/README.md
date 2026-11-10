# The home-page example component

Issue #1876, Approach C, stages 1–2. This directory is the example slot of
the mcpx home page, landed in the docs source because the page itself
(#1874) and the site build/deploy (#1941) have not landed yet — when they
do, the generator includes `component.html` where the example goes and
runs `generate-client.sh` as part of the build. Nothing here is a parallel
site.

## What a reader gets

- **Stage 1 — copy-as-command** (`copy-command.js`). Every docs example
  can be copied two ways: *Copy as mcpx exec* (a quoted-heredoc command
  that runs the snippet against the reader's real daemon; see the file
  header for why the quoting has to be a heredoc) and *Copy as script*
  (the source itself, plus `scriptCommands()` for the save-and-`mcpx run`
  two-liner).
- **Stage 2 — run it in the page** (`component.html`, `run-example.js`).
  The example is editable; Run executes it in the tab. The label says
  **"Simulated — runs in your browser"**, because it is one:
  - the script runs through a strip-only TypeScript transform
    (`strip-types.js`) and is imported as a blob module;
  - it talks to the **real generated client** — `client.ts`, emitted by
    `mcpx client --ns demo` against the deterministic fixture server
    (`internal/testsupport/fakemcp`), and `client.browser.js`, that same
    file after Node's strip-only transform. Neither is hand-written;
    regenerate both with `./generate-client.sh`;
  - the client's one hot route, `POST /v1/call`, is answered by a fake
    daemon (`fake-daemon.js`) from in-page mocks (`mocks.js`). `/v1/batch`,
    `/v1/resource` and `/v1/artifacts` are not simulated and answer with
    an honest error.
- **No-JS fallback**: the static highlighted example, unchanged.

## Fidelity gaps (also stated on the page)

- The only namespace is the mock `demo` fixture; its deterministic tools
  are mirrored, the rest return an error naming the limitation. No real
  MCP servers, no launcher placeholders, no `Deno.*`/`node:fs`.
- Mocks are computed from the same rules as the fixture where the fixture
  is deterministic (echo, open/state/wipe, structured, boom, fancy-name).

## Stage 3 is not here

Running against the reader's own daemon (a `daemon.cors` allow-list
setting, a fixed `daemon.port`, Chrome Local Network Access handling) is
excluded — a separate later follow-on. A hosted public daemon is rejected
in the issue, permanently. This component makes no daemon changes.

## Verify

```sh
node smoke-test.mjs   # shell round-trip for stage 1; end-to-end run for stage 2
./generate-client.sh  # regenerate client.ts / client.browser.js from the fixture
```

#1941 should wire `smoke-test.mjs` into the docs CI pipeline; until then
it is the recorded smoke check for this component.
