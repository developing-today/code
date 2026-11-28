// Pinning test for #6967: compact-usage-snapshots must reject unknown or
// misspelled arguments with a usage error (exit 2) BEFORE the store is
// read or written — a typo'd --month used to fall back to the default
// 13-month retention, so an --apply run compacted (and deleted) raw
// months a longer requested retention meant to keep, exiting 0; an
// unknown flag was silently ignored. The token after --months is its
// value slot and is judged by the #3285 validation, never re-interpreted
// as an unknown argument. Valid invocations are unchanged (see also the
// #3285 --months pinning test).
import { describe, expect, test } from "bun:test";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/compact-usage-snapshots.ts");

async function runScript(storeDir: string, args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: { ...process.env, PM_AGG_SNAPSHOTS_DIR: storeDir },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

function rawRecord(month: string, day: string, value: number): string {
  return (
    JSON.stringify({
      ts: month + "-" + day + "T12:00:00Z",
      serviceId: "svc-a",
      metricId: "weekly",
      value,
      unit: "pct",
      limit: 100,
      source: "manual",
      observedBy: "test",
    }) + String.fromCharCode(10)
  );
}

describe("compact-usage-snapshots unknown arguments (#6967)", () => {
  test("unknown or misspelled arguments exit 2, name the token, and leave the store untouched", async () => {
    const dir = await mkdtemp(join(tmpdir(), "compact-argv-6967-"));
    try {
      // Two raw months: one inside any plausible 24-month window from a
      // 2026+ run date, one far outside it. Pre-fix, the typo'd run
      // compacted BOTH under the 13-month default; post-fix it must
      // refuse before touching either.
      const recentName = "usage-2025-06.jsonl";
      const oldName = "usage-2024-01.jsonl";
      const recentRaw = rawRecord("2025-06", "15", 10) + rawRecord("2025-06", "16", 20);
      const oldRaw = rawRecord("2024-01", "15", 30);
      await writeFile(join(dir, recentName), recentRaw);
      await writeFile(join(dir, oldName), oldRaw);

      const badArgSets: { args: string[]; token: string }[] = [
        { args: ["--apply", "--month", "24"], token: "--month" },
        { args: ["--month", "24"], token: "--month" },
        { args: ["--bogus"], token: "--bogus" },
        { args: ["--apply", "--bogus", "--months", "24"], token: "--bogus" },
        { args: ["--aply"], token: "--aply" },
        { args: ["stray-positional"], token: "stray-positional" },
        { args: ["--months", "24", "extra"], token: "extra" },
      ];
      for (const { args, token } of badArgSets) {
        const { code, stderr } = await runScript(dir, args);
        expect(code, "args " + JSON.stringify(args)).toBe(2);
        expect(stderr).toContain("unknown argument '" + token + "'");
        expect((await readdir(dir)).sort()).toEqual([oldName, recentName]);
        expect(await readFile(join(dir, recentName), "utf8")).toBe(recentRaw);
        expect(await readFile(join(dir, oldName), "utf8")).toBe(oldRaw);
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 30000);

  test("valid invocations unchanged: --months value judged by #3285, dry-run and --apply still work", async () => {
    const dir = await mkdtemp(join(tmpdir(), "compact-argv-6967-ok-"));
    try {
      const oldName = "usage-2020-01.jsonl";
      await writeFile(join(dir, oldName), rawRecord("2020-01", "15", 42));

      // The --months value slot is consumed, not flagged unknown: a bad
      // value reaches the existing #3285 usage error instead.
      const badValue = await runScript(dir, ["--months", "0"]);
      expect(badValue.code).toBe(2);
      expect(badValue.stderr).toContain("invalid --months '0'");
      expect(badValue.stderr).not.toContain("unknown argument");

      // Dry-run with an explicit valid retention reports and changes nothing.
      const dry = await runScript(dir, ["--months", "24"]);
      expect(dry.code).toBe(0);
      expect(await readdir(dir)).toEqual([oldName]);

      // --apply with the same retention compacts the old month as before.
      const applied = await runScript(dir, ["--apply", "--months", "24"]);
      expect(applied.code).toBe(0);
      expect(await readdir(dir)).toEqual(["usage-2020-01.rollup.jsonl"]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 30000);
});
