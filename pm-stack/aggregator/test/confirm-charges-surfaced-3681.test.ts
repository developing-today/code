import { test, expect } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #3681: confirm-charges-surfaced.ts runs AFTER delivery, so a malformed
// chargesAttention element must never crash confirmation (delivered
// charges would re-alert forever) or persist a bogus key. Each case runs
// the real script as a subprocess against an isolated surfaced-state
// path, exactly like push-fleet-feed.sh invokes it.
const script = join(import.meta.dir, "../scripts/confirm-charges-surfaced.ts");
const valid = { normalized: "acmeai", amountUsd: 12.5, at: "2026-10-30" };
const validKey = "acmeai|12.50|2026-10-30";

async function runConfirm(feed: unknown) {
  const dir = await mkdtemp(join(tmpdir(), "confirm-surfaced-3681-"));
  const feedPath = join(dir, "feed.json");
  const statePath = join(dir, "state.json");
  try {
    await writeFile(feedPath, typeof feed === "string" ? feed : JSON.stringify(feed));
    const proc = Bun.spawn([process.execPath, script, feedPath], {
      env: { ...process.env, CHARGES_SURFACED_STATE_PATH: statePath },
      stdout: "pipe",
      stderr: "pipe",
    });
    const stderr = await new Response(proc.stderr).text();
    const code = await proc.exited;
    let surfaced: string[] | null = null;
    try {
      surfaced = JSON.parse(await readFile(statePath, "utf8")).surfaced;
    } catch {
      surfaced = null; // no state file written
    }
    return { code, stderr, surfaced };
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

test("valid control: well-formed feed confirms exactly as before", async () => {
  const r = await runConfirm({ chargesAttention: [valid] });
  expect(r.code).toBe(0);
  expect(r.surfaced).toEqual([validKey]);
}, 30_000);

test("null element: dropped loudly, no crash, no key persisted", async () => {
  const r = await runConfirm({ chargesAttention: [null] });
  expect(r.code).toBe(0);
  expect(r.stderr).toContain("dropped 1 malformed");
  expect(r.surfaced ?? []).toEqual([]);
}, 30_000);

test("string amount: dropped loudly, no key persisted", async () => {
  const r = await runConfirm({ chargesAttention: [{ ...valid, amountUsd: "12.50" }] });
  expect(r.code).toBe(0);
  expect(r.stderr).toContain("dropped 1 malformed");
  expect(r.surfaced ?? []).toEqual([]);
}, 30_000);

test("mixed valid + invalid: the valid sibling is still confirmed", async () => {
  const r = await runConfirm({ chargesAttention: [valid, null] });
  expect(r.code).toBe(0);
  expect(r.stderr).toContain("dropped 1 malformed");
  expect(r.surfaced).toEqual([validKey]);
}, 30_000);

test("top-level null: clear invalid-feed error, never a raw TypeError", async () => {
  const r = await runConfirm("null");
  expect(r.code).not.toBe(0);
  expect(r.stderr).toContain("invalid feed");
  expect(r.stderr).not.toContain("TypeError");
}, 30_000);

test("top-level array: rejected instead of silently succeeding", async () => {
  const r = await runConfirm([]);
  expect(r.code).not.toBe(0);
  expect(r.stderr).toContain("invalid feed");
  expect(r.stderr).not.toContain("TypeError");
}, 30_000);

test("missing normalized: dropped, and no undefined|… key can enter state", async () => {
  const { normalized: _omit, ...noNormalized } = valid;
  const r = await runConfirm({ chargesAttention: [noNormalized] });
  expect(r.code).toBe(0);
  expect(r.stderr).toContain("dropped 1 malformed");
  expect(r.surfaced ?? []).toEqual([]);
  expect((r.surfaced ?? []).some((k) => k.startsWith("undefined|"))).toBe(false);
}, 30_000);
