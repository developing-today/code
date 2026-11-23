// Door route-module tests (#12583). Oracle: design §3.1 — one Worker,
// route modules for the five component-C front doors over the shared
// plumbing. In the skeleton every door route exists, refuses the
// unauthenticated (401), and stubs 501 naming its filling issue for an
// authenticated caller; unknown paths are a plain 404.

import assert from "node:assert/strict";
import { test } from "node:test";
import { dispatch } from "../src/router.ts";
import { bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();
const token = await h.sign(h.rpPayload());

const DOOR_ROUTES: { method: string; path: string; fillingIssue: string }[] = [
  { method: "POST", path: "/v1/stamps", fillingIssue: "#12585" },
  { method: "GET", path: "/v1/query", fillingIssue: "#12587" },
  { method: "POST", path: "/v1/query", fillingIssue: "#12587" },
  { method: "POST", path: "/v1/queue", fillingIssue: "#12589" },
  { method: "POST", path: "/v1/ingestion/github", fillingIssue: "#12590" },
  { method: "POST", path: "/v1/sync/project", fillingIssue: "#12591" },
  { method: "POST", path: "/v1/sync/queue-github", fillingIssue: "#12591" },
];

test("every door route refuses an unauthenticated request", async () => {
  for (const r of DOOR_ROUTES) {
    const res = await dispatch(
      new Request(`https://fleet-hosted.test${r.path}`, { method: r.method }),
      h.deps,
    );
    assert.equal(res.status, 401, `${r.method} ${r.path} without a token`);
  }
});

test("every door route stubs 501 naming its filling issue for an authenticated caller", async () => {
  for (const r of DOOR_ROUTES) {
    const res = await dispatch(bearerReq(r.path, token, { method: r.method }), h.deps);
    assert.equal(res.status, 501, `${r.method} ${r.path} with a valid token`);
    const body = (await res.json()) as { error: string; message: string };
    assert.equal(body.error, "not_implemented");
    assert.ok(
      body.message.includes(r.fillingIssue),
      `${r.method} ${r.path} stub must name ${r.fillingIssue}`,
    );
  }
});

test("unknown paths are 404, including a wrong method on a door path", async () => {
  const unknown = await dispatch(bearerReq("/v1/nope", token), h.deps);
  assert.equal(unknown.status, 404);
  const wrongMethod = await dispatch(bearerReq("/v1/stamps", token, { method: "GET" }), h.deps);
  assert.equal(wrongMethod.status, 404);
});
