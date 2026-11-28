// Tests for the CDP wedge probe (#10630).
//
// Oracles, named per the testing doctrine (AGENTS.md):
//   - Wire oracle: cdpPageResponsive runs against a real local
//     websocket server speaking just enough CDP. A renderer that
//     answers Runtime.evaluate reads responsive; one that accepts
//     the socket and never replies (the hung-renderer shape) reads
//     unresponsive within the timeout; one that replies with a CDP
//     error reads unresponsive with the error named; a dead
//     endpoint reads unresponsive instead of throwing.
//   - Target-choice oracle: pickAllowlistTarget picks the page-type
//     allowlist target with a debugger websocket and nothing else —
//     background pages, workers, off-origin pages, and pages with
//     no debugger websocket never count as "the page is open".

import { describe, expect, test } from "bun:test";
import { cdpPageResponsive, pickAllowlistTarget } from "../src/watchdog.ts";

function fakePage(mode: "answer" | "silent" | "error") {
  const server = Bun.serve({
    port: 0,
    fetch(req, srv) {
      if (srv.upgrade(req)) return undefined;
      return new Response("not a websocket", { status: 400 });
    },
    websocket: {
      message(ws, message) {
        const msg = JSON.parse(String(message)) as { id?: number; method?: string };
        if (msg.method !== "Runtime.evaluate") return;
        if (mode === "answer") {
          ws.send(JSON.stringify({ id: msg.id, result: { result: { type: "number", value: 1 } } }));
        } else if (mode === "error") {
          ws.send(JSON.stringify({ id: msg.id, error: { message: "Cannot find context" } }));
        }
        // "silent": the hung renderer — socket open, never replies.
      },
    },
  });
  return { url: `ws://127.0.0.1:${server.port}/devtools/page/fake`, stop: () => server.stop(true) };
}

describe("wedge probe — cdpPageResponsive (wire oracle)", () => {
  test("a renderer that answers Runtime.evaluate is responsive", async () => {
    const page = fakePage("answer");
    try {
      const r = await cdpPageResponsive(page.url, 2000);
      expect(r.ok).toBe(true);
      expect(r.detail).toContain("answered");
    } finally {
      page.stop();
    }
  });

  test("a renderer that never answers is unresponsive within the timeout", async () => {
    const page = fakePage("silent");
    try {
      const r = await cdpPageResponsive(page.url, 300);
      expect(r.ok).toBe(false);
      expect(r.detail).toContain("unresponsive");
    } finally {
      page.stop();
    }
  });

  test("a CDP error reply is unresponsive and names the error", async () => {
    const page = fakePage("error");
    try {
      const r = await cdpPageResponsive(page.url, 2000);
      expect(r.ok).toBe(false);
      expect(r.detail).toContain("Cannot find context");
    } finally {
      page.stop();
    }
  });

  test("a dead websocket endpoint is unresponsive, not a throw", async () => {
    const r = await cdpPageResponsive("ws://127.0.0.1:1/devtools/page/none", 1000);
    expect(r.ok).toBe(false);
  });
});

describe("wedge probe — pickAllowlistTarget (target-choice oracle)", () => {
  const origins = ["https://fixture.invalid"];

  test("picks the page target on an allowlist origin with a debugger websocket", () => {
    const t = pickAllowlistTarget(
      [
        { type: "page", url: "https://other.invalid/", webSocketDebuggerUrl: "ws://x/1" },
        { type: "page", url: "https://fixture.invalid/app", webSocketDebuggerUrl: "ws://x/2" },
      ],
      origins,
    );
    expect(t?.webSocketDebuggerUrl).toBe("ws://x/2");
  });

  test("background pages and workers on the origin do not count", () => {
    const t = pickAllowlistTarget(
      [
        { type: "background_page", url: "https://fixture.invalid/bg", webSocketDebuggerUrl: "ws://x/1" },
        { type: "service_worker", url: "https://fixture.invalid/sw", webSocketDebuggerUrl: "ws://x/2" },
      ],
      origins,
    );
    expect(t).toBeNull();
  });

  test("a page without a debugger websocket does not count", () => {
    expect(pickAllowlistTarget([{ type: "page", url: "https://fixture.invalid/" }], origins)).toBeNull();
  });

  test("no targets at all is null", () => {
    expect(pickAllowlistTarget([], origins)).toBeNull();
  });
});
