import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SCRIPT = join(import.meta.dir, "..", "scripts", "sync-decision-events.ts");

describe("sync-decision-events CLI --outbox-dir", () => {
  test("exits non-zero when --outbox-dir does not exist", async () => {
    const missing = join(tmpdir(), `missing-outbox-${Date.now()}-${Math.random()}`);
    const proc = Bun.spawn(["bun", "run", SCRIPT, "--outbox-dir", missing, "--dry-run"], {
      stderr: "pipe",
      stdout: "pipe",
    });
    const code = await proc.exited;
    const stderr = await new Response(proc.stderr).text();
    expect(code).toBe(1);
    expect(stderr).toContain("cannot read --outbox-dir");
  });

  test("exits non-zero when --outbox-dir is a regular file", async () => {
    const dir = mkdtempSync(join(tmpdir(), "outbox-file-"));
    try {
      const file = join(dir, "not-a-dir");
      writeFileSync(file, "hello");
      const proc = Bun.spawn(["bun", "run", SCRIPT, "--outbox-dir", file, "--dry-run"], {
        stderr: "pipe",
        stdout: "pipe",
      });
      const code = await proc.exited;
      const stderr = await new Response(proc.stderr).text();
      expect(code).toBe(1);
      expect(stderr).toContain("cannot read --outbox-dir");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
