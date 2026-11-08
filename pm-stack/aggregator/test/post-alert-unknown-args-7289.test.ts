// Pinning tests for #7289: post-alert must reject unknown, misspelled,
// joined, duplicated, and stray arguments with usage + exit 2 BEFORE
// any token is resolved or any fetch is made. The pre-fix parser only
// probed argv with indexOf, so `--severty warning` silently posted a
// warning incident as critical (the ?? "critical" default), the joined
// `--severity=warning` did the same, and `--bodi 503` silently dropped
// the body — all exit 0.
//
// Subprocess pattern from post-alert-args-3723.test.ts: the script runs
// verbatim against a stub intake server (Bun.serve) and a stub `bws`
// binary, so "no token resolved, no fetch made" is asserted from the
// stubs' logs — the oracles — not assumed.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/post-alert.ts");

interface Captured {
  body: string;
  authorization: unknown;
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
  const home = await mkdtemp(join(tmpdir(), "post-alert-7289-"));
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

describe("post-alert unknown arguments (#7289)", () => {
  test("control: the documented invocation still posts exactly the incident it names", async () => {
    const before = captured.length;
    const { code } = await runScript([
      "--source", "watch",
      "--key", "host1",
      "--severity", "warning",
      "--title", "host down",
      "--body", "503",
    ]);
    expect(code).toBe(0);
    expect(captured.length).toBe(before + 1);
    expect(JSON.parse(captured[captured.length - 1].body)).toEqual({
      source: "watch",
      key: "host1",
      severity: "warning",
      title: "host down",
      body: "503",
    });
  });

  test("unknown, misspelled, joined, duplicate, and stray arguments exit 2 with no token resolved and no fetch", async () => {
    const badArgSets: { args: string[]; names: string }[] = [
      // typo'd --severity used to post the incident as critical
      { args: ["--source", "watch", "--key", "host1", "--severty", "warning", "--title", "host down"], names: "--severty" },
      // joined form used to be invisible to indexOf, same critical fallback
      { args: ["--source", "watch", "--key", "host1", "--severity=warning", "--title", "host down"], names: "--severity=warning" },
      // unknown flag
      { args: ["--source", "watch", "--key", "host1", "--title", "host down", "--bogus", "x"], names: "--bogus" },
      // typo'd --body used to drop the body silently
      { args: ["--source", "watch", "--key", "host1", "--severity", "info", "--title", "host down", "--bodi", "503"], names: "--bodi" },
      // duplicate flag: indexOf silently used the first value
      { args: ["--source", "watch", "--key", "host1", "--severity", "warning", "--severity", "info", "--title", "host down"], names: "--severity" },
      // stray positional
      { args: ["--source", "watch", "--key", "host1", "--title", "host down", "extra"], names: "extra" },
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
