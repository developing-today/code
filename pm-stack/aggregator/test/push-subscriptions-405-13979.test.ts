import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";

// #13979 sentinel for #4983: GET /api/push/subscriptions was removed by
// #9939 (POST upsert + DELETE by endpoint remain). A future change that
// re-adds the enumerating GET must trip the suite instead of silently
// restoring the endpoint-enumeration surface.
let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;

beforeAll(async () => {
  port = 6200 + Math.floor(Math.random() * 800);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_FRESHNESS_PORT: String(port + 1000),
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_STORE_BACKEND: "file",
    },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
}, 60_000);

afterAll(() => {
  proc?.kill();
});

describe("GET /api/push/subscriptions stays gone (#13979 / #4983)", () => {
  test("GET returns 405 with no endpoint/p256dh/auth material", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/api/push/subscriptions`);
    expect(res.status).toBe(405);
    const raw = await res.text();
    expect(raw).not.toContain("p256dh");
    expect(raw).not.toContain('"auth"');
    expect(raw).not.toContain("endpoint");
    const body = JSON.parse(raw) as { error?: unknown };
    expect(body.error).toBe("method not allowed");
  });

  test("other non-writing methods are also 405, not an enumeration", async () => {
    for (const method of ["PUT", "PATCH"]) {
      const res = await fetch(`http://127.0.0.1:${port}/api/push/subscriptions`, { method });
      expect(res.status).toBe(405);
      const raw = await res.text();
      expect(raw).not.toContain("p256dh");
      expect(raw).not.toContain('"auth"');
      expect(raw).not.toContain("endpoint");
    }
  });
});
