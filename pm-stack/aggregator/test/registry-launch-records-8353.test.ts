// #8353: the collector's registry section masked an unreadable
// launch-records file as absent — the read sat in a bare catch, and the
// section then marked ok/skipped from its other sources alone.
// loadLaunchRecords separates the three states: absent (ENOENT) is the
// normal no-sessions-yet state and stays error-free; a read file yields
// its records (torn lines still tolerated per-line); any other read
// failure surfaces as `error` so the collector can mark the section
// error instead of claiming ok for that source.
// Oracle: the filesystem itself — a path that does not exist, a real
// JSONL file, and a directory where the file should be (a deterministic
// non-ENOENT read failure on every platform).
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { loadLaunchRecords } from "../src/registry.ts";

async function tempDir(): Promise<string> {
  return mkdtemp(join(tmpdir(), "launch-records-8353-"));
}

describe("loadLaunchRecords (#8353)", () => {
  test("missing file is absent: no records, no error", async () => {
    const dir = await tempDir();
    const load = await loadLaunchRecords(join(dir, "claude-launch-records.jsonl"));
    expect(load.records).toEqual([]);
    expect(load.error).toBeNull();
  });

  test("readable file yields its records, torn lines tolerated", async () => {
    const dir = await tempDir();
    const path = join(dir, "claude-launch-records.jsonl");
    await writeFile(
      path,
      [
        JSON.stringify({ id: "ses_launch_ok", label: "Ok", lastSeenAt: "2026-11-10T00:00:00.000Z" }),
        "{ torn final line",
        "",
      ].join("\n"),
      "utf8",
    );
    const load = await loadLaunchRecords(path);
    expect(load.error).toBeNull();
    expect(load.records.map((r) => r.id)).toEqual(["ses_launch_ok"]);
  });

  test("a directory in the file's place is unreadable: error surfaces, no records", async () => {
    const dir = await tempDir();
    const load = await loadLaunchRecords(dir);
    expect(load.records).toEqual([]);
    expect(load.error).not.toBeNull();
  });
});
