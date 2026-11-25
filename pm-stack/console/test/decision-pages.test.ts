import { expect, test } from "bun:test";
import { decisionPage, type DecisionPagesIo } from "../edge/src/decision-pages.ts";

// Oracle: authenticated decision-events wire contract; no browser credential,
// no invented definitions, no successful empty page on an upstream failure.
const stamp = "2026-11-25T09:00:00.000Z";
const io = (body: unknown, seen: Request[] = []): DecisionPagesIo => ({
  secret: "server-only-secret",
  now: () => Date.parse(stamp) + 60_000,
  read: async (r) => { seen.push(r); return Response.json(body); },
});
const req = (path: string) => new Request(`https://pm.security.cab${path}`);

test("index serves activity with freshness and canonical credential-free links", async () => {
  const seen: Request[] = [];
  const response = await decisionPage(req("/decisions?t=old-token"), io({ sets: [
    { setId: "set/<script>", events: 4, lastTs: stamp, lastType: "submit" },
  ] }, seen));
  expect(response?.status).toBe(200);
  const html = await response!.text();
  expect(html).toContain("D1 decision-events");
  expect(html).toContain(stamp);
  expect(html).toContain("60 seconds");
  expect(html).toContain("https://pm.security.cab/decisions/set%2F%3Cscript%3E");
  expect(html).not.toContain("<script>");
  expect(html).not.toContain("server-only-secret");
  expect(html).not.toContain("old-token");
  expect(seen[0].headers.get("authorization")).toBe("Bearer server-only-secret");
  expect(seen[0].url).toBe("https://decision-events.internal/sets?limit=200");
  expect(response!.headers.get("cache-control")).toBe("no-store");
});

test("set page shows ordered event history, composite pagination, and missing definitions honestly", async () => {
  const seen: Request[] = [];
  const response = await decisionPage(req("/decisions/set-1"), io({ setId: "set-1", events: [
    { eventId: "event-1", ts: stamp, type: "submit", questionRef: "q<1", payload: { token: "never-render-payload" } },
  ], nextAfter: `${stamp}|event-1` }, seen));
  const html = await response!.text();
  expect(html).toContain("Question definitions are not in this event feed");
  expect(html).toContain("q&lt;1");
  expect(html).toContain("after=2026-11-25T09%3A00%3A00.000Z%7Cevent-1");
  expect(html).not.toContain("never-render-payload");
  expect(html).not.toContain("<form");
  expect(seen[0].url).toBe("https://decision-events.internal/sets/set-1/events?limit=200");
});

test("page cursor is forwarded but browser cookies and credentials are not", async () => {
  const seen: Request[] = [];
  await decisionPage(new Request(`https://pm.security.cab/decisions/set-1?after=${encodeURIComponent(stamp + "|event-1")}`, {
    headers: { cookie: "owner=private", authorization: "Bearer browser" },
  }), io({ events: [], nextAfter: null }, seen));
  expect(new URL(seen[0].url).searchParams.get("after")).toBe(stamp + "|event-1");
  expect(seen[0].headers.get("cookie")).toBeNull();
});

test("missing configuration and upstream failures are visible, never successful empty lists", async () => {
  let calls = 0;
  for (const status of [401, 403, 429, 500]) {
    const response = await decisionPage(req("/decisions"), {
      ...io(null), read: async () => { calls++; return new Response("secret failure detail", { status }); },
    });
    expect(response?.status).toBe(503);
    expect(await response!.text()).not.toContain("secret failure detail");
  }
  const response = await decisionPage(req("/decisions"), { ...io(null), secret: undefined,
    read: async () => { throw Error("must not be called"); },
  });
  expect(response?.status).toBe(503);
  expect(calls).toBe(4);
});

test("bad JSON shapes and transport exceptions fail visibly", async () => {
  for (const body of [null, {}, { sets: [{}] }, { sets: [{setId: "x", events: 1, lastTs: "nonsense", lastType: "raise"}] }]) {
    expect((await decisionPage(req("/decisions"), io(body)))?.status).toBe(503);
  }
  expect((await decisionPage(req("/decisions"), { ...io(null), read: async () => { throw Error("down"); } }))?.status).toBe(503);
});

test("unrelated paths and writes are never handled", async () => {
  const never = { ...io(null), read: async () => { throw Error("must not read"); } };
  for (const path of ["/console/", "/decisions/a/submit", "/api/decisions/a/submit"]) {
    expect(await decisionPage(req(path), never)).toBeNull();
  }
  expect(await decisionPage(new Request("https://pm.security.cab/decisions", {method: "POST"}), never)).toBeNull();
});
