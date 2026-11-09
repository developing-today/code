import { test, expect } from "bun:test";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #7097: unknown, misspelled, or duplicated arguments must be usage
// errors (exit 2, nothing archived, no delivery attempt) — never
// silently ignored. Before the fix a typo'd --knd weekly archived a
// daily digest with exit 0, and a typo'd --no-delivre proceeded into
// the delivery path. Each case runs the real script as a subprocess
// with the fixture source and an isolated digests dir, like
// generate-digest-kind-6926.test.ts.
const script = join(import.meta.dir, "../scripts/generate-digest.ts");

async function runDigest(args: string[]) {
  const dir = await mkdtemp(join(tmpdir(), "digest-unknown-7097-"));
  try {
    const proc = Bun.spawn([process.execPath, script, ...args], {
      env: { ...process.env, PM_AGG_SOURCE: "fixtures", PM_AGG_DIGESTS_DIR: dir },
      stdout: "pipe",
      stderr: "pipe",
    });
    const stdout = await new Response(proc.stdout).text();
    const stderr = await new Response(proc.stderr).text();
    const code = await proc.exited;
    const archived = await readdir(dir);
    return { code, stdout, stderr, archived };
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

test("valid control: --kind weekly --no-deliver archives a weekly digest", async () => {
  const r = await runDigest(["--kind", "weekly", "--no-deliver"]);
  expect(r.code).toBe(0);
  expect(r.archived.some((f) => f.startsWith("weekly-"))).toBe(true);
}, 30_000);

test("unknown flag: --bogus is a usage error, nothing archived", async () => {
  const r = await runDigest(["--bogus", "--no-deliver"]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("unknown argument");
  expect(r.archived).toEqual([]);
}, 30_000);

test("typo'd flag: --knd weekly is a usage error, not a daily digest", async () => {
  const r = await runDigest(["--knd", "weekly", "--no-deliver"]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("unknown argument");
  expect(r.archived).toEqual([]);
}, 30_000);

test("typo'd suppressor: --no-delivre is a usage error before any delivery", async () => {
  const r = await runDigest(["--kind", "weekly", "--no-delivre"]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("unknown argument");
  expect(r.archived).toEqual([]);
}, 30_000);

test("duplicate --kind is a usage error, nothing archived", async () => {
  const r = await runDigest(["--kind", "weekly", "--kind", "daily", "--no-deliver"]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("duplicate --kind");
  expect(r.archived).toEqual([]);
}, 30_000);

test("stray positional token is a usage error, nothing archived", async () => {
  const r = await runDigest(["weekly", "--no-deliver"]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("unknown argument");
  expect(r.archived).toEqual([]);
}, 30_000);
