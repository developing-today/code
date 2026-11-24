// Pinning tests for #1480 phase 1: the intake harness must never silently
// skip new issues and the watermark must advance only after a successful
// pass — forward only, never backward, never from a corrupt state.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  INTAKE_REPOS,
  advanceWatermark,
  isIntakeIssue,
  loadState,
  saveState,
  selectNewIssues,
  shapeIntakeItem,
} from "../src/intake.ts";

const script = join(import.meta.dir, "../scripts/gh-intake-poll.ts");

async function runScript(args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], { stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

describe("intake watermark guards (#1480)", () => {
  test("loadState rejects non-JSON, missing, and invalid watermarks", () => {
    expect(() => loadState("not json")).toThrow();
    expect(() => loadState("{}")).toThrow();
    expect(() => loadState('{"watermark":"yesterday"}')).toThrow();
    expect(loadState('{"watermark":"2026-11-24T00:00:00Z"}')).toEqual({ watermark: "2026-11-24T00:00:00Z" });
  });

  test("advanceWatermark moves forward only", () => {
    const current = { watermark: "2026-11-24T10:00:00Z" };
    expect(advanceWatermark(current, "2026-11-24T11:00:00Z")).toEqual({ watermark: "2026-11-24T11:00:00Z" });
    expect(() => advanceWatermark(current, "2026-11-24T10:00:00Z")).toThrow();
    expect(() => advanceWatermark(current, "2026-11-24T09:00:00Z")).toThrow();
    expect(() => advanceWatermark(current, "soon")).toThrow();
  });

  test("saveState round-trips through loadState", () => {
    expect(loadState(saveState({ watermark: "2026-11-24T10:00:00Z" }))).toEqual({ watermark: "2026-11-24T10:00:00Z" });
  });
});

describe("intake issue selection (#1480)", () => {
  const mark = "2026-11-24T10:00:00Z";
  const older = { number: 1, title: "old", created_at: "2026-11-24T09:00:00Z", labels: [] };
  const atMark = { number: 2, title: "edge", created_at: mark, labels: [] };
  const newer = { number: 3, title: "new", created_at: "2026-11-24T11:00:00Z", labels: [{ name: "kind/bug" }] };
  const pr = { number: 4, title: "a pr", created_at: "2026-11-24T12:00:00Z", pull_request: {}, labels: [] };

  test("isIntakeIssue skips PRs and malformed rows", () => {
    expect(isIntakeIssue(newer)).toBe(true);
    expect(isIntakeIssue(pr)).toBe(false);
    expect(isIntakeIssue({})).toBe(false);
    expect(isIntakeIssue({ number: 5, created_at: "bogus" })).toBe(false);
  });

  test("selectNewIssues takes strictly-after-watermark, oldest first, PRs excluded", () => {
    const items = selectNewIssues("developing-today/code", [newer, pr, older, atMark], mark);
    expect(items.map((i) => i.ref)).toEqual(["developing-today/code#3"]);
  });

  test("shapeIntakeItem carries the repo-qualified ref and label names", () => {
    const item = shapeIntakeItem("developing-today/code", newer);
    expect(item.ref).toBe("developing-today/code#3");
    expect(item.labels).toEqual(["kind/bug"]);
  });
});

describe("gh-intake-poll CLI guards (#1480)", () => {
  test("unknown subcommand and missing subcommand exit 2 before any network", async () => {
    for (const args of [[], ["frobnicate"]]) {
      const r = await runScript(args);
      expect(r.code).toBe(2);
      expect(r.stderr).toContain("usage:");
    }
  });

  test("poll without a state file refuses a silent epoch", async () => {
    const dir = await mkdtemp(join(tmpdir(), "intake-1480-"));
    try {
      const r = await runScript(["poll", "--state", join(dir, "missing.json")]);
      expect(r.code).toBe(2);
      expect(r.stderr).toContain("--since");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("advance refuses backward moves and writes forward moves", async () => {
    const dir = await mkdtemp(join(tmpdir(), "intake-1480-"));
    try {
      const state = join(dir, "intake-state.json");
      await writeFile(state, saveState({ watermark: "2026-11-24T10:00:00Z" }));
      const back = await runScript(["advance", "--to", "2026-11-24T09:00:00Z", "--state", state]);
      expect(back.code).toBe(2);
      expect(JSON.parse(await readFile(state, "utf8"))).toEqual({ watermark: "2026-11-24T10:00:00Z" });
      const fwd = await runScript(["advance", "--to", "2026-11-24T11:00:00Z", "--state", state]);
      expect(fwd.code).toBe(0);
      expect(JSON.parse(await readFile(state, "utf8"))).toEqual({ watermark: "2026-11-24T11:00:00Z" });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("the four repos are exactly the intake universe", () => {
    expect([...INTAKE_REPOS]).toEqual([
      "developing-today/code",
      "developing-today/opencode",
      "developing-today/openchamber",
      "dezren39/nix",
    ]);
  });
});
