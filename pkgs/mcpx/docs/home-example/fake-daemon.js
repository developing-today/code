// The fake daemon for the home-page simulation (#1876, Approach C stage 2).
//
// The generated mcpx client talks to a daemon over plain HTTP and its hot
// path is one route, POST /v1/call (internal/codegen/emit.go). In a browser
// there is no daemon — so the page patches globalThis.fetch, answers the
// /v1 routes from the in-page mocks (mocks.js), and lets every other
// request through untouched. The client is the real one, emitted by
// `mcpx client` against the fixture servers (see generate-client.sh); only
// the transport's other end is simulated, and the page is labelled that
// way ("simulated — runs in your browser").
//
// Routes the client can touch:
//   POST /v1/call       -> answered from mocks
//   POST /v1/batch      -> not simulated: an honest error, not a fake result
//   GET  /v1/resource   -> not simulated: honest error
//   GET  /v1/artifacts  -> not simulated: honest error
//
// installFakeDaemon() returns a restore function. Installing twice is a
// no-op that returns the existing restore.

import { callMock } from "./mocks.js";

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const NOT_SIMULATED = (route) =>
  json(
    {
      error:
        `${route} is not part of the browser simulation; ` +
        `the demo answers /v1/call only`,
    },
    501,
  );

export function installFakeDaemon() {
  if (globalThis.__mcpxFakeDaemon) return globalThis.__mcpxFakeDaemon.restore;

  const original = globalThis.fetch.bind(globalThis);

  const patched = async (input, init) => {
    const raw = typeof input === "string" ? input : input.url;
    let path;
    try {
      path = new URL(raw, "http://mcpx.invalid").pathname;
    } catch {
      return original(input, init);
    }

    if (path.endsWith("/v1/call")) {
      let req;
      try {
        req = JSON.parse(init?.body ?? "{}");
      } catch {
        return json({ error: "fake daemon: /v1/call body is not JSON" }, 400);
      }
      const out = callMock(req.server, req.tool, req.args ?? {});
      return json(out);
    }
    if (path.endsWith("/v1/batch")) return NOT_SIMULATED("/v1/batch");
    if (path.endsWith("/v1/resource")) return NOT_SIMULATED("/v1/resource");
    if (path.endsWith("/v1/artifacts")) return NOT_SIMULATED("/v1/artifacts");

    return original(input, init);
  };

  const restore = () => {
    if (globalThis.fetch === patched) globalThis.fetch = original;
    delete globalThis.__mcpxFakeDaemon;
  };

  globalThis.fetch = patched;
  globalThis.__mcpxFakeDaemon = { restore };
  return restore;
}
