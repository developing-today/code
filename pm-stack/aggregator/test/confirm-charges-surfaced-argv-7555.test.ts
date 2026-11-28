import { test, expect } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #7555: confirm-charges-surfaced.ts takes exactly one positional
// argument. Extra or flag-like tokens used to be silently ignored — a
// second feed path exited 0 while that feed's delivered charges were
// never confirmed, so they would re-alert forever. Each case runs the
// real script as a subprocess against an isolated surfaced-state path,
// exactly like push-fleet-feed.sh invokes it (the #3681 suite pattern).
const script = join(import.meta.dir, "../scripts/confirm-charges-surfaced.ts");
const valid = { normalized: "acmeai", amountUsd: 12.5, at: "2026-10-30" };
const validKey = "acmeai|12.50|2026-10-30";
const other = { normalized: "otherco", amountUsd: 99, at: "2026-10-31" };

async function runArgv(argv: string[]) {
  const dir = await mkdtemp(join(tmpdir(), "confirm-surfaced-argv-7555-"));
  const statePath = join(dir, "state.json");
  try {
    const proc = Bun.spawn([process.execPath, script, ...argv], {
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
    return { code, stderr, surfaced, dir };
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

async function runFeeds(feeds: unknown[], extraArgs: string[] = []) {
  const dir = await mkdtemp(join(tmpdir(), "confirm-surfaced-argv-7555-"));
  try {
    const feedPaths: string[] = [];
    for (const [i, feed] of feeds.entries()) {
      const feedPath = join(dir, `feed${i}.json`);
      await writeFile(feedPath, JSON.stringify(feed));
      feedPaths.push(feedPath);
    }
    // Delegate the spawn to runArgv by way of a state path inside dir:
    // runArgv uses its own temp dir for state, so instead spawn here.
    const statePath = join(dir, "state.json");
    const proc = Bun.spawn([process.execPath, script, ...feedPaths, ...extraArgs], {
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

test("control: the single documented argument confirms as before", async () => {
  const r = await runFeeds([{ chargesAttention: [valid] }]);
  expect(r.code).toBe(0);
  expect(r.surfaced).toEqual([validKey]);
}, 30_000);

test("extra unknown argument: usage error exit 2, nothing confirmed", async () => {
  const r = await runFeeds([{ chargesAttention: [valid] }], ["--bogus"]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("--bogus");
  expect(r.surfaced).toBeNull();
}, 30_000);

test("second feed path: usage error exit 2, neither feed confirmed", async () => {
  const r = await runFeeds([{ chargesAttention: [valid] }, { chargesAttention: [other] }]);
  expect(r.code).toBe(2);
  expect(r.stderr).toContain("unexpected argument");
  expect(r.surfaced).toBeNull();
}, 30_000);

test("flag-like invocation: --feed <path> is a usage error, feed never read", async () => {
  const dir = await mkdtemp(join(tmpdir(), "confirm-surfaced-argv-7555-"));
  try {
    const feedPath = join(dir, "feed.json");
    await writeFile(feedPath, JSON.stringify({ chargesAttention: [valid] }));
    const r = await runArgv(["--feed", feedPath]);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain("--feed");
    expect(r.surfaced).toBeNull();
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}, 30_000);
