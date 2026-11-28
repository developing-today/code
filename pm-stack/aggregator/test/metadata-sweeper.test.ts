import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { describe, expect, test } from "bun:test";

const script = join(import.meta.dir, "../scripts/metadata-sweeper.ts");

async function makeFakeFleet(root: string): Promise<string> {
  const path = join(root, "fake-fleet.sh");
  await writeFile(path, "#!/bin/sh\nprintf '[]\\n'\n");
  await chmod(path, 0o755);
  return path;
}

describe("metadata-sweeper database startup (#13355)", () => {
  test("creates a missing DB parent before scanning", async () => {
    const root = await mkdtemp(join(tmpdir(), "metadata-sweeper-test-"));
    try {
      const dbPath = join(root, "new", "nested", "pm-metadata.sqlite");
      const fakeFleet = await makeFakeFleet(root);
      execFileSync(process.execPath, ["run", script, "--repo", "example/repo", "--db", dbPath], {
        env: { ...process.env, FLEET_BIN: fakeFleet },
        stdio: "pipe",
      });
      expect(existsSync(dbPath)).toBe(true);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("dry-run keeps using memory and does not create the configured DB", async () => {
    const root = await mkdtemp(join(tmpdir(), "metadata-sweeper-test-"));
    try {
      const dbPath = join(root, "new", "nested", "pm-metadata.sqlite");
      const fakeFleet = await makeFakeFleet(root);
      execFileSync(process.execPath, ["run", script, "--repo", "example/repo", "--db", dbPath, "--dry-run"], {
        env: { ...process.env, FLEET_BIN: fakeFleet },
        stdio: "pipe",
      });
      expect(existsSync(dbPath)).toBe(false);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
