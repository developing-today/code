// Regression tests for #13355: the metadata-sweeper opened its database
// with `new Database(dbPath)` directly, so in a clean environment (no
// ~/.local/state/pm-aggregator yet) it crashed with SQLITE_CANTOPEN
// before scanning anything — bun:sqlite creates the file but not its
// parents. The server path was fixed the same way in #13293
// (openMetadataStore); the sweeper bypassed that function. The oracle is
// the sweeper's own exit code and the database file: with --db under a
// missing nested directory the run must proceed past the DB open (exit 0,
// file created), an unhealable parent must still fail closed, and a
// dry run must still touch no file.
//
// The sweeper's fleet calls are stubbed through FLEET_BIN: every list
// the scan makes answers `[]`, so the run is hermetic — no network, no
// GitHub — and the only thing under test is the DB open.
import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/metadata-sweeper.ts");

async function runSweeper(args: string[], fleetBin: string): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: { ...process.env, FLEET_BIN: fleetBin },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

async function makeFleetStub(dir: string): Promise<string> {
  const stub = join(dir, "fleet-stub.sh");
  await writeFile(stub, "#!/bin/sh\necho '[]'\n", { mode: 0o755 });
  return stub;
}

describe("metadata-sweeper DB parent directory (#13355)", () => {
  test("a --db under a missing nested directory proceeds past the DB open", async () => {
    const dir = await mkdtemp(join(tmpdir(), "sweeper-db-parent-13355-"));
    try {
      const fleetBin = await makeFleetStub(dir);
      const dbPath = join(dir, "state", "pm-aggregator", "pm-metadata.sqlite");
      const { code, stderr, stdout } = await runSweeper(["--repo", "developing-today/code", "--db", dbPath], fleetBin);
      expect(stderr).not.toContain("unable to open database file");
      expect(code).toBe(0);
      expect(existsSync(dbPath)).toBe(true);
      const summary = JSON.parse(stdout) as { summary: Record<string, { dbRows: number }> };
      expect(summary.summary["developing-today/code"]?.dbRows).toBe(0);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("an unhealable parent still fails closed", async () => {
    const dir = await mkdtemp(join(tmpdir(), "sweeper-db-parent-13355-"));
    try {
      const fleetBin = await makeFleetStub(dir);
      const blocker = join(dir, "blocker");
      await writeFile(blocker, "not a directory");
      const { code, stderr } = await runSweeper(
        ["--repo", "developing-today/code", "--db", join(blocker, "pm-metadata.sqlite")],
        fleetBin,
      );
      // Fails closed either way: on the old code the open threw
      // SQLITE_CANTOPEN; with the fix the mkdir throws EEXIST first.
      // Either way the run must die, not silently proceed.
      expect(code).toBe(1);
      expect(stderr).toContain("metadata-sweeper: fatal:");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("a dry run still touches no file", async () => {
    const dir = await mkdtemp(join(tmpdir(), "sweeper-db-parent-13355-"));
    try {
      const fleetBin = await makeFleetStub(dir);
      const dbPath = join(dir, "state", "pm-aggregator", "pm-metadata.sqlite");
      const { code } = await runSweeper(
        ["--repo", "developing-today/code", "--db", dbPath, "--dry-run"],
        fleetBin,
      );
      expect(code).toBe(0);
      expect(existsSync(dbPath)).toBe(false);
      expect(existsSync(join(dir, "state"))).toBe(false);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
