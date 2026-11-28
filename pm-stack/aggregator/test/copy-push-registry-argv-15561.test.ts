// Pinning test for copy-push-registry-to-d1.ts (#15561 slice 2, #3352
// operator-script contract): unknown or incomplete arguments are a usage
// error (exit 2) BEFORE any secret, network, or database I/O.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/copy-push-registry-to-d1.ts");

async function runScript(args: string[]): Promise<{ code: number; stderr: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: { ...process.env, BWS_ACCESS_TOKEN: "invalid-before-parse" },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [code, stderr] = await Promise.all([proc.exited, new Response(proc.stderr).text()]);
  return { code, stderr };
}

describe("copy-push-registry-to-d1 argv", () => {
  test("unknown flag is a usage error", async () => {
    const r = await runScript(["--bogus"]);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain("Usage: copy-push-registry-to-d1.ts");
  });

  test("missing mode is a usage error", async () => {
    const r = await runScript([]);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain("Usage: copy-push-registry-to-d1.ts");
  });

  test("duplicate mode is a usage error", async () => {
    const r = await runScript(["--copy", "--verify-only"]);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain("Usage: copy-push-registry-to-d1.ts");
  });
});
