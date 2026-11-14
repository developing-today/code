import { describe, expect, it } from "bun:test";
import { BOUNDED_DASHBOARD_PATHS, createDashboardResponseDeadline } from "../src/assembly-deadline.ts";

const routes = [
  "/board",
  "/api/board.json",
  "/api/overview.json",
  "/frag/overview",
  "/program",
  "/api/program.json",
] as const;

describe("dashboard assembly deadline (#5664)", () => {
  it("covers only the six routes whose complete assembly can outlive Bun's request window", () => {
    expect([...BOUNDED_DASHBOARD_PATHS].sort()).toEqual([...routes].sort());
    // #9781: /api/program.json was deliberately excluded while it was
    // fast; steward run 369 measured a 17.83 s unbounded first pass,
    // so the twin is bounded now like its HTML sibling.
    expect(BOUNDED_DASHBOARD_PATHS.has("/api/program.json")).toBe(true);
  });

  it.each(routes)("returns a visible, non-empty bounded response for a stalled %s", async (path) => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 10 });
    const start = performance.now();
    const response = await withinDeadline(
      new Request(`http://localhost${path}`),
      () => new Promise<Response>(() => {}),
    );
    const elapsed = performance.now() - start;
    const body = await response.text();

    expect(elapsed).toBeLessThan(250);
    expect(body.length).toBeGreaterThan(0);
    if (path === "/frag/overview") {
      expect(response.status).toBe(200);
      expect(response.headers.get("content-type")).toContain("text/event-stream");
      expect(body).toContain('role="alert"');
      expect(body).toContain("timed out");
      // The inner patch must not duplicate the #overview-main shell id.
      expect(body).not.toContain('id="overview-main"');
    } else if (path.startsWith("/api/")) {
      expect(response.status).toBe(503);
      expect(response.headers.get("content-type")).toContain("application/json");
      expect(JSON.parse(body)).toMatchObject({ stale: false, timeoutMs: 10 });
      expect(JSON.parse(body).error).toContain("timed out");
    } else {
      expect(response.status).toBe(503);
      expect(response.headers.get("content-type")).toContain("text/html");
      expect(body).toContain('role="alert"');
      expect(body).toContain("timed out");
    }
  });

  it("returns the last successful representation with a visible stale marker when a later assembly stalls", async () => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 10 });
    const cases = [
      {
        path: "/board",
        response: new Response("<html><body><main>board</main></body></html>", {
          headers: { "content-type": "text/html; charset=utf-8" },
        }),
        marker: "Showing the last successful",
      },
      { path: "/api/overview.json", response: Response.json({ overview: "ready" }), marker: '"stale": true' },
      {
        path: "/frag/overview",
        response: new Response("event: patch\\ndata: original\\n\\n", {
          headers: { "content-type": "text/event-stream; charset=utf-8" },
        }),
        marker: "Showing the last successful",
      },
    ] as const;

    for (const item of cases) {
      const request = new Request(`http://localhost${item.path}`);
      const first = await withinDeadline(request, async () => item.response.clone());
      expect(first.status).toBe(200);

      const stale = await withinDeadline(request, () => new Promise<Response>(() => {}));
      const body = await stale.text();
      expect(stale.status).toBe(200);
      expect(stale.headers.get("x-aggregator-stale")).toBe("true");
      expect(body).toContain(item.marker);
    }
  });

  it.each(routes)("uses a late successful %s assembly as the next request's visible stale response", async (path) => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 5 });
    const request = new Request(`http://localhost${path}`);
    const delayedAssembly = async () => {
      await Bun.sleep(30);
      if (path === "/frag/overview") {
        return new Response("event: patch\ndata: ready\n\n", {
          headers: { "content-type": "text/event-stream; charset=utf-8" },
        });
      }
      if (path.startsWith("/api/")) return Response.json({ ready: true });
      return new Response("<html><body><main>ready</main></body></html>", {
        headers: { "content-type": "text/html; charset=utf-8" },
      });
    };

    const first = await withinDeadline(request, delayedAssembly);
    expect(first.status).toBe(path === "/frag/overview" ? 200 : 503);
    await Bun.sleep(35);

    const stale = await withinDeadline(request, () => new Promise<Response>(() => {}));
    const body = await stale.text();
    expect(stale.status).toBe(200);
    expect(stale.headers.get("x-aggregator-stale")).toBe("true");
    if (path.startsWith("/api/")) expect(JSON.parse(body)).toMatchObject({ ready: true, stale: true });
    else expect(body).toContain("Showing the last successful");
  });

  it("preserves a fast response unchanged", async () => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 100 });
    const response = await withinDeadline(new Request("http://localhost/api/board.json"), async () =>
      Response.json({ board: [{ id: "fast" }] }),
    );

    expect(response.status).toBe(200);
    expect(response.headers.get("x-aggregator-stale")).toBeNull();
    expect(await response.json()).toEqual({ board: [{ id: "fast" }] });
  });

  it("coalesces concurrent same-key requests into one assembly (#7065)", async () => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 200 });
    const request = new Request("http://localhost/api/board.json");
    let started = 0;
    let concurrent = 0;
    let maxConcurrent = 0;
    const assemble = async () => {
      started += 1;
      concurrent += 1;
      maxConcurrent = Math.max(maxConcurrent, concurrent);
      await Bun.sleep(30);
      concurrent -= 1;
      return Response.json({ board: [{ id: "shared" }] });
    };

    const responses = await Promise.all([
      withinDeadline(request, assemble),
      withinDeadline(request, assemble),
      withinDeadline(request, assemble),
    ]);

    expect(started).toBe(1);
    expect(maxConcurrent).toBe(1);
    for (const response of responses) {
      expect(response.status).toBe(200);
      expect(await response.json()).toEqual({ board: [{ id: "shared" }] });
    }
  });

  it("joins the still-running assembly after an earlier waiter timed out (#7065)", async () => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 10 });
    const request = new Request("http://localhost/api/board.json");
    let started = 0;
    const assemble = async () => {
      started += 1;
      await Bun.sleep(50);
      return Response.json({ board: [{ id: "late" }] });
    };

    const first = await withinDeadline(request, assemble);
    expect(first.status).toBe(503);
    // The first assembly is still running; this overlapping request must
    // share it (and time out on its own deadline), not start a second run.
    const second = await withinDeadline(request, assemble);
    expect(second.status).toBe(503);
    expect(started).toBe(1);

    // The shared late run seeds the last-good cache exactly once, so a
    // later stalled request is served the shared result as stale.
    await Bun.sleep(60);
    const stale = await withinDeadline(request, () => new Promise<Response>(() => {}));
    expect(stale.status).toBe(200);
    expect(stale.headers.get("x-aggregator-stale")).toBe("true");
    expect(JSON.parse(await stale.text())).toMatchObject({ board: [{ id: "late" }], stale: true });
  });

  it("does not poison the key when the shared assembly rejects (#7065)", async () => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 100 });
    const request = new Request("http://localhost/api/board.json");

    await expect(
      withinDeadline(request, async () => {
        throw new Error("assembly failed");
      }),
    ).rejects.toThrow("assembly failed");

    const recovered = await withinDeadline(request, async () => Response.json({ board: [{ id: "fresh" }] }));
    expect(recovered.status).toBe(200);
    expect(await recovered.json()).toEqual({ board: [{ id: "fresh" }] });
  });

  it("does not use a last-good response for a different filtered URL", async () => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 10 });
    const filtered = new Request("http://localhost/api/board.json?view=mine");
    await withinDeadline(filtered, async () => Response.json({ board: [{ id: "mine" }] }));

    const timedOut = await withinDeadline(filtered, () => new Promise<Response>(() => {}));
    expect(timedOut.status).toBe(200);
    expect(JSON.parse(await timedOut.text())).toMatchObject({ stale: true, board: [{ id: "mine" }] });

    const other = await withinDeadline(
      new Request("http://localhost/api/board.json?view=other"),
      () => new Promise<Response>(() => {}),
    );
    expect(other.status).toBe(503);
    expect(JSON.parse(await other.text()).stale).toBe(false);
  });
});
