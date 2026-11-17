# The home-page example component

Issue #1876, Approach C, stages 1–3. This directory is the example slot of
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

## Stage 3 — run on my machine (`local-daemon.js`)

The opt-in control in `component.html` (off by default, switched only
by the reader) swaps the fake daemon for the local-daemon transport:
`/v1` requests are rewritten to the reader's daemon address (default
`http://127.0.0.1:7432`, editable in the component) and carried by the
page's real fetch, so the example runs against the reader's own
daemon through the same generated client. The label changes to say
so. A hosted public daemon is rejected in the issue, permanently.

The daemon side is the recorded surface choice on #1876/#1890, taken
because #1890 had not decided its shared-vs-docs-only question: the
daemon already refused browser origins outside
`transport.allowedOrigins` (`internal/daemon/origin.go`); stage 3
adds the missing CORS protocol for those allow-listed origins —
preflight answers and `Access-Control-Allow-Origin` —
(`internal/daemon/cors.go`). No new setting, no hardcoded origin, no
origin allow-listed by default: the reader opts in twice, here and
in their daemon's allow-list.

What a reader needs, and what the in-page error names when it is
missing:

- a running daemon on a **fixed `daemon.port`** (the default is a
  free port only the daemon itself reports, so the page cannot
  discover it);
- this site's origin in **`transport.allowedOrigins`** (without it
  the daemon refuses the page, and its error says exactly that);
- **no bearer token** on the daemon (`transport.bearerToken`): a
  browser tab has nowhere safe to keep one, so the local run is for
  the default loopback trust setup.

Chrome's Local Network Access restrictions on public pages calling
loopback are answered by the same CORS exchange (the preflight is
the permission probe); a browser that still blocks it surfaces the
same no-daemon message.

## Verify

```sh
node smoke-test.mjs   # shell round-trip for stage 1; end-to-end runs for stages 2 and 3
./generate-client.sh  # regenerate client.ts / client.browser.js from the fixture
```

#1941 should wire `smoke-test.mjs` into the docs CI pipeline; until then
it is the recorded smoke check for this component.
