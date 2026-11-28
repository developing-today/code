import { test, expect } from "bun:test";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #6926: a --kind that is present but missing its value (trailing, or
// followed by another flag) must be a usage error, exit 2 — never a
// silent fall-through to the daily default. Each case runs the real
// script as a subprocess with the fixture source and an isolated
// digests dir, exactly like the systemd units invoke it (plus
// --no-deliver, which only suppresses the push step).
const script = join(import.meta.dir, "../scripts/generate-digest.ts");

async function runDigest(args: string[]) {
  const dir = await mkdtemp(join(tmpdir(), "digest-kind-6926-"));
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

test("valid control: --kind weekly archives a weekly digest", async () => {
  const r = await runDigest(["--no-deliver", "--kind", "weekly"]);
  expect(r.code).toBe(0);
  expect(r.archived.some((f) => f.startsWith("weekly-"))).toBe(true);
}, 30_000);

test("valid control: no --kind still defaults to a daily digest", async () => {
  const r = await runDigest(["--no-deliver"]);
  expect(r.code).toBe(0);
  expect(r.archived.some((f) => f.startsWith("daily-"))).toBe(true);
}, 30_000);

test("trailing --kind: usage error exit 2, nothing archived", async () => {
  const r = await runDigest(["--no-deliver", "--kind"]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("--kind requires a value");
  expect(r.archived).toEqual([]);
}, 30_000);

test("flag-as-value: --kind --no-deliver is a usage error, nothing archived", async () => {
  const r = await runDigest(["--kind", "--no-deliver"]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("--kind requires a value");
  expect(r.archived).toEqual([]);
}, 30_000);
