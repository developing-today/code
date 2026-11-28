// Pinning test for the #3352 argument contract on
// scripts/quota-forecast-backtest.ts (#12696): unknown arguments, a
// stray positional, and a present-but-valueless option are usage
// errors (exit 2) naming the offending argument, before any file is
// read or written; a valid run over a fixture store writes the
// report, the ledger, and the facts.
import { describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/quota-forecast-backtest.ts");

async function runScript(args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], { stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

describe("quota-forecast-backtest argument contract (#3352)", () => {
  test("unknown argument, stray positional, and valueless option all exit 2 naming the token", async () => {
    const bad = await runScript(["--store-dir", "/nonexistent", "--bogus"]);
    expect(bad.code).toBe(2);
    expect(bad.stderr).toContain("--bogus");

    const stray = await runScript(["--store-dir", "/nonexistent", "extra"]);
    expect(stray.code).toBe(2);
    expect(stray.stderr).toContain("extra");

    const valueless = await runScript(["--store-dir"]);
    expect(valueless.code).toBe(2);
    expect(valueless.stderr).toContain("--store-dir");

    const missing = await runScript([]);
    expect(missing.code).toBe(2);
    expect(missing.stderr).toContain("--store-dir");
  });

  test("a valid run over a fixture store writes report, ledger, and facts", async () => {
    const dir = await mkdtemp(join(tmpdir(), "quota-forecast-backtest-"));
    try {
      const store = join(dir, "store");
      await mkdir(store);
      const reset = "2026-11-20T05:00:00.000Z";
      const lines: string[] = [];
      for (let i = 0; i < 10; i++) {
        lines.push(
          JSON.stringify({
            ts: new Date(Date.parse("2026-11-20T00:00:00Z") + i * 1_800_000).toISOString(),
            serviceId: "cline-pass",
            metricId: "5h",
            value: 10 * (i + 1),
            unit: "pct",
            limit: 100,
            resetsAt: reset,
            source: "api",
            observedBy: "test",
          }),
        );
      }
      await writeFile(join(store, "usage-2026-11.jsonl"), lines.join("\n") + "\n");
      const reportOut = join(dir, "report.md");
      const ledgerOut = join(dir, "ledger.json");
      const factsOut = join(dir, "facts.jsonl");
      const run = await runScript([
        "--store-dir",
        store,
        "--report-out",
        reportOut,
        "--ledger-out",
        ledgerOut,
        "--facts-out",
        factsOut,
      ]);
      expect(run.code).toBe(0);
      const report = await readFile(reportOut, "utf8");
      expect(report).toContain("Per-pool calibration");
      const ledger = JSON.parse(await readFile(ledgerOut, "utf8")) as {
        kind: string;
        pools: { pool: string; rollup: { precision: number | null } }[];
      };
      expect(ledger.kind).toBe("quota-forecast-accuracy");
      expect(ledger.pools.find((p) => p.pool === "cline-pass")?.rollup.precision).toBe(1);
      const facts = (await readFile(factsOut, "utf8")).trim().split("\n");
      expect(facts.length).toBe(8);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
