import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BreakGlassTokenStore, issueBreakGlassToken } from "../src/break-glass.ts";

// #9608 route wiring: the /api/admin/identity-registry gate accepts a
// presented single-use break-glass token when the caller is not an
// Access-verified owner (the Access/IdP-outage path). Boots the real
// server with the token store pointed at a temp file; a token issued
// in-process (the local CLI's job) must authorize exactly one request.
let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;
let issuedToken = "";

beforeAll(async () => {
  const dir = mkdtempSync(join(tmpdir(), "break-glass-route-"));
  const breakGlassFile = join(dir, "break-glass-tokens.json");
  const store = new BreakGlassTokenStore(breakGlassFile);
  const issued = await issueBreakGlassToken({
    store,
    events: { record: async () => {} },
    ttlMs: 60 * 60 * 1000,
  });
  issuedToken = issued.token;

  port = 5100 + Math.floor(Math.random() * 800);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_FRESHNESS_PORT: String(port + 1000),
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_STORE_BACKEND: "file",
      PM_AGG_BREAK_GLASS_FILE: breakGlassFile,
      PM_AGG_EVENTS_FILE: join(dir, "events.json"),
      PM_AGG_IDENTITY_REGISTRY_FILE: join(dir, "identity-registry.json"),
      PM_AGG_REPORT_RECORDS_FILE: join(dir, "report-records.jsonl"),
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

describe("break-glass token authorizes the identity-registry gate (#9608)", () => {
  test("no token is refused, a valid token works exactly once, replay is refused", async () => {
    const url = `http://127.0.0.1:${port}/api/admin/identity-registry`;

    const none = await fetch(url);
    expect(none.status).toBe(403);

    const bogus = await fetch(url, { headers: { authorization: "Bearer not-a-token" } });
    expect(bogus.status).toBe(403);

    const ok = await fetch(url, { headers: { authorization: `Bearer ${issuedToken}` } });
    expect(ok.status).toBe(200);
    const body = (await ok.json()) as { registry: unknown };
    expect(body.registry).toBeTruthy();

    const replay = await fetch(url, { headers: { authorization: `Bearer ${issuedToken}` } });
    expect(replay.status).toBe(403);
    // The authorized request records an audit event, whose store can
    // take seconds to flush on a loaded machine — allow for that.
  }, 30_000);
});
