import { afterAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";

// #1195: the server must fail loud — not silently serve fixtures — when
// PM_AGG_SOURCE is unset or invalid outside explicit dev mode
// (PM_AGG_DEV=1). With the dev switch it boots from fixtures normally.
const procs: Subprocess<"ignore", "pipe", "inherit">[] = [];
afterAll(() => {
  for (const p of procs) p.kill();
});

function serverEnv(extra: Record<string, string | undefined>): Record<string, string | undefined> {
  const { PM_AGG_SOURCE: _source, PM_AGG_DEV: _dev, ...rest } = Bun.env;
  void _source;
  void _dev;
  return { ...rest, ...extra };
}

async function waitForHealth(port: number, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) return true;
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  return false;
}

describe("source gate boot behavior (#1195)", () => {
  test("unset PM_AGG_SOURCE without PM_AGG_DEV refuses to start", async () => {
    const port = 5100 + Math.floor(Math.random() * 1000);
    const proc = spawn({
      cmd: ["bun", "run", "src/server.ts"],
      cwd: import.meta.dir + "/..",
      env: serverEnv({ PM_AGG_PORT: String(port) }),
      stdout: "pipe",
      stderr: "inherit",
    });
    procs.push(proc);
    const exited = await proc.exited;
    expect(exited).not.toBe(0);
    expect(await waitForHealth(port, 1500)).toBe(false);
  }, 15000);

  test("unset PM_AGG_SOURCE with PM_AGG_DEV=1 boots from fixtures", async () => {
    const port = 5100 + Math.floor(Math.random() * 1000);
    const proc = spawn({
      cmd: ["bun", "run", "src/server.ts"],
      cwd: import.meta.dir + "/..",
      env: serverEnv({ PM_AGG_PORT: String(port), PM_AGG_DEV: "1" }),
      stdout: "pipe",
      stderr: "inherit",
    });
    procs.push(proc);
    expect(await waitForHealth(port, 12000)).toBe(true);
    const res = await fetch(`http://127.0.0.1:${port}/api/health`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { status: string; source: { board: string } };
    expect(body.status).toBe("ok");
    expect(body.source.board).toBe("fixtures");
    proc.kill();
  }, 20000);
});
