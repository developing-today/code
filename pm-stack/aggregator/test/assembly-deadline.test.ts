import { describe, expect, it } from "bun:test";
import { BOUNDED_DASHBOARD_PATHS, createDashboardResponseDeadline } from "../src/assembly-deadline.ts";

const routes = ["/board", "/api/board.json", "/api/overview.json", "/frag/overview", "/program"] as const;

describe("dashboard assembly deadline (#5664)", () => {
  it("covers only the five routes whose complete assembly can outlive Bun's request window", () => {
    expect([...BOUNDED_DASHBOARD_PATHS].sort()).toEqual([...routes].sort());
    expect(BOUNDED_DASHBOARD_PATHS.has("/api/program.json")).toBe(false);
  });

  it.each(routes)("returns a visible, non-empty bounded response for a stalled %s", async (path) => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 10 });
    const start = performance.now();
    const response = await withinDeadline(new Request(`http://localhost${path}`), () => new Promise<Response>(() => {}));
    const elapsed = performance.now() - start;
    const body = await response.text();

    expect(elapsed).toBeLessThan(250);
    expect(body.length).toBeGreaterThan(0);
    if (path === "/frag/overview") {
      expect(response.status).toBe(200);
      expect(response.headers.get("content-type")).toContain("text/event-stream");
      expect(body).toContain("role=\"alert\"");
      expect(body).toContain("timed out");
    } else if (path.startsWith("/api/")) {
      expect(response.status).toBe(503);
      expect(response.headers.get("content-type")).toContain("application/json");
      expect(JSON.parse(body)).toMatchObject({ stale: false, timeoutMs: 10 });
      expect(JSON.parse(body).error).toContain("timed out");
    } else {
      expect(response.status).toBe(503);
      expect(response.headers.get("content-type")).toContain("text/html");
      expect(body).toContain("role=\"alert\"");
      expect(body).toContain("timed out");
    }
  });

  it("returns the last successful representation with a visible stale marker when a later assembly stalls", async () => {
    const withinDeadline = createDashboardResponseDeadline({ timeoutMs: 10 });
    const cases = [
      { path: "/board", response: new Response("<html><body><main>board</main></body></html>", { headers: { "content-type": "text/html; charset=utf-8" } }), marker: "Showing the last successful" },
      { path: "/api/overview.json", response: Response.json({ overview: "ready" }), marker: '"stale": true' },
      { path: "/frag/overview", response: new Response("event: patch\\ndata: original\\n\\n", { headers: { "content-type": "text/event-stream; charset=utf-8" } }), marker: "Showing the last successful" },
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
    const response = await withinDeadline(
      new Request("http://localhost/api/board.json"),
      async () => Response.json({ board: [{ id: "fast" }] }),
    );

    expect(response.status).toBe(200);
    expect(response.headers.get("x-aggregator-stale")).toBeNull();
    expect(await response.json()).toEqual({ board: [{ id: "fast" }] });
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
