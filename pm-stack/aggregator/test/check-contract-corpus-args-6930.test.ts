// Pinning test for #6930: check-contract-corpus must treat a flag taken
// as the --corpus/--publisher value as a usage error (exit 2), raised
// before any file is read. Pre-fix, `--corpus --publisher` took
// "--publisher" as the corpus dir and died with a raw ENOENT stack
// (exit 1), and `--publisher --corpus` reported a bogus publisher parse
// failure (exit 1) against a valid publisher file contract.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/check-contract-corpus.ts");
const corpus = join(import.meta.dir, "contract-corpus", "v1");

async function runScript(args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: { PATH: process.env.PATH ?? "/run/current-system/sw/bin:/usr/bin:/bin" },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

describe("check-contract-corpus argument validation (#6930)", () => {
  test("flag-like, missing, and empty values are usage errors (exit 2) before any file read", async () => {
    const badArgSets: string[][] = [
      ["--corpus", "--publisher"],
      ["--publisher", "--corpus"],
      ["--corpus", "--corpus", corpus],
      ["--corpus"],
      ["--publisher"],
      ["--corpus", ""],
      ["--publisher", ""],
    ];
    for (const args of badArgSets) {
      const { code, stderr, stdout } = await runScript(args);
      expect(code, `args ${JSON.stringify(args)}`).toBe(2);
      expect(stderr, `args ${JSON.stringify(args)}`).toContain("usage: check-contract-corpus.ts");
      expect(stderr, `args ${JSON.stringify(args)}`).not.toContain("ENOENT");
      expect(stdout, `args ${JSON.stringify(args)}`).not.toContain("publisher file parses");
      expect(stdout, `args ${JSON.stringify(args)}`).not.toContain("contract check");
    }
  }, 60000);

  test("valid forms still pass: no args, and --corpus + --publisher together", async () => {
    const plain = await runScript([]);
    expect(plain.code).toBe(0);
    expect(plain.stdout).toContain("contract check passed");

    const both = await runScript(["--corpus", corpus, "--publisher", join(corpus, "orchestration.json")]);
    expect(both.code).toBe(0);
    expect(both.stdout).toContain("publisher section passes the collector gate");
    expect(both.stdout).toContain("contract check passed (including publisher file");
  }, 60000);
});
