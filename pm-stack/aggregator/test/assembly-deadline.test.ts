import { describe, expect, it } from "bun:test";
import { mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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

  describe("last-good across a restart (#10383)", () => {
    const stalled = () => new Promise<Response>(() => {});

    it("serves the previous process's last-good, marked stale with its age", async () => {
      const dir = mkdtempSync(join(tmpdir(), "last-good-"));
      let clock = 1_000_000;
      const before = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir, now: () => clock });
      for (const path of ["/program", "/api/program.json"]) {
        const fresh = await before(new Request(`http://localhost${path}`), async () =>
          path.startsWith("/api/")
            ? Response.json({ projects: ["id"] })
            : new Response("<html><body><h1>Program</h1></body></html>", {
                headers: { "content-type": "text/html; charset=utf-8" },
              }),
        );
        expect(fresh.status).toBe(200);
      }
      expect(readdirSync(dir).filter((f) => f.endsWith(".json"))).toHaveLength(2);
      expect(readdirSync(dir).some((f) => f.includes(".tmp-"))).toBe(false);

      // A restart 50 minutes later: past the in-memory 30-minute cap,
      // inside the disk copy's own cap.
      clock += 50 * 60_000;
      const after = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir, now: () => clock });
      const page = await after(new Request("http://localhost/program"), stalled);
      expect(page.status).toBe(200);
      expect(page.headers.get("x-aggregator-stale")).toBe("true");
      const html = await page.text();
      expect(html).toContain("<h1>Program</h1>");
      expect(html).toContain("saved before the dashboard restarted, 50 min ago");

      const api = await after(new Request("http://localhost/api/program.json"), stalled);
      expect(api.status).toBe(200);
      expect(JSON.parse(await api.text())).toMatchObject({ projects: ["id"], stale: true });
    });

    it("still answers 503 when the disk copy is too old or missing", async () => {
      const dir = mkdtempSync(join(tmpdir(), "last-good-"));
      let clock = 1_000_000;
      const before = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir, now: () => clock });
      await before(new Request("http://localhost/api/program.json"), async () => Response.json({ projects: [] }));
      clock += 7 * 60 * 60_000;
      const after = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir, now: () => clock });
      expect((await after(new Request("http://localhost/api/program.json"), stalled)).status).toBe(503);
      expect((await after(new Request("http://localhost/api/board.json"), stalled)).status).toBe(503);
    });

    it("ignores an unreadable or mismatched disk copy", async () => {
      const dir = mkdtempSync(join(tmpdir(), "last-good-"));
      const before = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir });
      await before(new Request("http://localhost/program"), async () => new Response("<body>ok</body>"));
      const [file] = readdirSync(dir);
      writeFileSync(join(dir, file), "{not json");
      const after = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir });
      expect((await after(new Request("http://localhost/program"), stalled)).status).toBe(503);
    });

    it("serves the disk copy after the cold grace, not the full deadline, on a cold first request", async () => {
      const dir = mkdtempSync(join(tmpdir(), "last-good-"));
      const before = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir });
      await before(new Request("http://localhost/api/program.json"), async () => Response.json({ projects: ["id"] }));

      // Oracle: with a 60 s assembly deadline, a cold request whose
      // assembly never settles must still answer with the disk copy in
      // roughly the cold grace (50 ms here), not after 60 s.
      const after = createDashboardResponseDeadline({ timeoutMs: 60_000, coldStaleGraceMs: 50, persistDir: dir });
      const start = performance.now();
      const page = await after(new Request("http://localhost/api/program.json"), stalled);
      const elapsed = performance.now() - start;
      expect(elapsed).toBeLessThan(5_000);
      expect(page.status).toBe(200);
      expect(page.headers.get("x-aggregator-stale")).toBe("true");
      const body = JSON.parse(await page.text());
      expect(body).toMatchObject({ projects: ["id"], stale: true });
      expect(body.staleReason).toContain("still running");
      expect(body.staleReason).toContain("saved before the dashboard restarted");
    });

    it("prefers a fresh assembly that finishes inside the cold grace over the disk copy", async () => {
      const dir = mkdtempSync(join(tmpdir(), "last-good-"));
      const before = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir });
      await before(new Request("http://localhost/api/program.json"), async () => Response.json({ v: "old" }));

      // Oracle: a fast fresh assembly on the cold first request is
      // served fresh and unmarked — the disk copy is only the fallback
      // for an assembly that outlives the grace.
      const after = createDashboardResponseDeadline({ timeoutMs: 60_000, coldStaleGraceMs: 500, persistDir: dir });
      const fresh = await after(new Request("http://localhost/api/program.json"), async () => Response.json({ v: "new" }));
      expect(fresh.status).toBe(200);
      expect(fresh.headers.get("x-aggregator-stale")).toBeNull();
      expect(await fresh.json()).toEqual({ v: "new" });
    });

    it("seeds the in-memory last-good from a cold request's late assembly", async () => {
      const dir = mkdtempSync(join(tmpdir(), "last-good-"));
      const before = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir });
      await before(new Request("http://localhost/api/program.json"), async () => Response.json({ v: "old" }));

      const after = createDashboardResponseDeadline({ timeoutMs: 100, coldStaleGraceMs: 20, persistDir: dir });
      const cold = await after(new Request("http://localhost/api/program.json"), async () => {
        await Bun.sleep(60);
        return Response.json({ v: "late" });
      });
      expect(JSON.parse(await cold.text())).toMatchObject({ v: "old", stale: true });
      // The background assembly settles and seeds memory; the next
      // stalled request is served the late response as ordinary
      // in-memory stale (no pre-restart wording).
      await Bun.sleep(80);
      const stale = await after(new Request("http://localhost/api/program.json"), stalled);
      const body = JSON.parse(await stale.text());
      expect(body).toMatchObject({ v: "late", stale: true });
      expect(body.staleReason).not.toContain("restarted");
    });

    it("prefers a fresh in-process response over the disk copy", async () => {
      const dir = mkdtempSync(join(tmpdir(), "last-good-"));
      const before = createDashboardResponseDeadline({ timeoutMs: 10, persistDir: dir });
      await before(new Request("http://localhost/api/program.json"), async () => Response.json({ v: "old" }));
      // The seeding call below must return via its assembly, not via its
      // deadline: seeding finishes with the disk persist (mkdir + write +
      // rename), which under CI load can outlive a 10 ms deadline. If it
      // does, the shared run is still in flight when the stalled call
      // arrives, and that call joins the run (#7065) and receives the
      // fresh response unmarked — correct behaviour, but the premise of
      // this test (a stale serve) never happens and staleReason is
      // absent. A 250 ms deadline gives the persist tail ample headroom;
      // the stalled call still outlives it and takes the fallback.
      const after = createDashboardResponseDeadline({ timeoutMs: 250, persistDir: dir });
      await after(new Request("http://localhost/api/program.json"), async () => Response.json({ v: "new" }));
      const stale = await after(new Request("http://localhost/api/program.json"), stalled);
      const body = JSON.parse(await stale.text());
      expect(body.v).toBe("new");
      expect(body.staleReason).not.toContain("restarted");
    });
  });
});
