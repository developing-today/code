// Pinning tests for #3723: post-alert's arg() must reject a flag-like
// or missing value with usage + exit 2 naming the flag, BEFORE any
// token is resolved or any fetch is made. The pre-fix parser swallowed
// the next flag as a value, so `--title --body 503` posted a critical
// incident literally titled "--body" and a trailing `--severity`
// silently defaulted to critical.
//
// Subprocess pattern from collect-fleet-feed-args-3509.test.ts: the
// script runs verbatim against a stub intake server (Bun.serve) and a
// stub `bws` binary, so "no token resolved, no fetch made" is asserted
// from the stubs' logs — the oracles — not assumed. The control
// invocations prove the documented call still posts exactly the
// incident it names.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/post-alert.ts");

interface Captured {
  body: string;
  authorization: string | null;
}
const captured: Captured[] = [];
let intake: ReturnType<typeof Bun.serve>;
beforeAll(() => {
  intake = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    fetch: async (req) => {
      captured.push({ body: await req.text(), authorization: req.headers.get("authorization") });
      return new Response(JSON.stringify({ ok: true }), { headers: { "Content-Type": "application/json" } });
    },
  });
});
afterAll(() => intake.stop(true));

const FAKE_BWS = `#!/bin/sh
echo "$@" >> "$FAKE_BWS_LOG"
if [ "$1 $2" = "secret list" ]; then
  echo '[{"id":"id1","key":"pm-aggregator.alerts-intake-token"}]'
  exit 0
fi
if [ "$1 $2" = "secret get" ]; then
  echo '{"value":"test-intake-token"}'
  exit 0
fi
exit 1
`;

async function runScript(args: string[]): Promise<{ code: number; stderr: string; bwsCalls: string }> {
  const home = await mkdtemp(join(tmpdir(), "post-alert-3723-"));
  try {
    const bwsPath = join(home, "fake-bws");
    await writeFile(bwsPath, FAKE_BWS);
    await chmod(bwsPath, 0o755);
    const tokenFile = join(home, "access-token");
    await writeFile(tokenFile, "dummy-access-token\n");
    const bwsLog = join(home, "bws.log");
    const proc = Bun.spawn([process.execPath, script, ...args], {
      cwd: home,
      env: {
        PATH: process.env.PATH ?? "/run/current-system/sw/bin:/usr/bin:/bin",
        HOME: home,
        TMPDIR: home,
        BWS_BIN: bwsPath,
        BWS_ACCESS_TOKEN_FILE: tokenFile,
        FAKE_BWS_LOG: bwsLog,
        ALERTS_INTAKE_URL: `http://127.0.0.1:${intake.port}`,
      },
      stdout: "pipe",
      stderr: "pipe",
    });
    const [, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
    const code = await proc.exited;
    const bwsCalls = await readFile(bwsLog, "utf8").catch(() => "");
    return { code, stderr, bwsCalls };
  } finally {
    await rm(home, { recursive: true, force: true });
  }
}

describe("post-alert argument validation (#3723)", () => {
  test("control: the documented invocation posts exactly the incident it names", async () => {
    const before = captured.length;
    const { code, bwsCalls } = await runScript([
      "--source", "watch",
      "--key", "host1",
      "--severity", "warning",
      "--title", "host down",
      "--body", "503",
    ]);
    expect(code).toBe(0);
    expect(bwsCalls).toContain("secret list");
    expect(captured.length).toBe(before + 1);
    const posted = captured[captured.length - 1];
    expect(posted.authorization).toBe("Bearer test-intake-token");
    expect(JSON.parse(posted.body)).toEqual({
      source: "watch",
      key: "host1",
      severity: "warning",
      title: "host down",
      body: "503",
    });
  });

  test("control: an omitted --severity still defaults to critical", async () => {
    const before = captured.length;
    const { code } = await runScript(["--source", "watch", "--key", "host1", "--title", "host down"]);
    expect(code).toBe(0);
    expect(captured.length).toBe(before + 1);
    expect(JSON.parse(captured[captured.length - 1].body).severity).toBe("critical");
  });

  test("flag-as-value and trailing flags exit 2 naming the flag, with no token resolved and no fetch", async () => {
    const badArgSets: { args: string[]; names: string }[] = [
      { args: ["--source", "watch", "--key", "host1", "--title", "--body", "503"], names: "--title" },
      { args: ["--source", "--key", "host1", "--title", "host down"], names: "--source" },
      { args: ["--source", "watch", "--title", "host down", "--key", "--severity", "info"], names: "--key" },
      { args: ["--source", "watch", "--key", "host1", "--title", "host down", "--severity"], names: "--severity" },
      { args: ["--source", "watch", "--key", "host1", "--title"], names: "--title" },
      { args: ["--source", "watch", "--key", "host1", "--title", "host down", "--body", "--severity", "info"], names: "--body" },
      { args: ["--source", "watch", "--key", "host1", "--severity", "--title", "host down"], names: "--severity" },
      { args: ["--source", "watch", "--key", "host1", "--title", ""], names: "--title" },
    ];
    for (const { args, names } of badArgSets) {
      const before = captured.length;
      const { code, stderr, bwsCalls } = await runScript(args);
      expect(code).toBe(2);
      expect(stderr).toContain(names);
      expect(stderr).toContain("usage: post-alert.ts");
      expect(bwsCalls).toBe(""); // no token resolution happened
      expect(captured.length).toBe(before); // no incident was posted
    }
  });
});
