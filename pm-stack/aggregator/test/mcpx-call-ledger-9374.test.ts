import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "bun:test";
import { readMcpxCallLedger } from "../src/analytics-store.ts";

const dirs: string[] = [];
function ledgerDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "mcpx-call-ledger-"));
  dirs.push(dir);
  return dir;
}
afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

describe("mcpx daemon call ledger (#9374)", () => {
  test("counts only valid records in the last seven days and groups tools", () => {
    const dir = ledgerDir();
    const now = Date.parse("2026-11-18T12:00:00.000Z");
    const day = "2026-11-18";
    writeFileSync(join(dir, `mcpx-${day}.jsonl`), [
      JSON.stringify({ ts: "2026-11-18T11:00:00.000Z", server: "github", tool: "issues", outcome: "success" }),
      JSON.stringify({ ts: "2026-11-18T11:30:00.000Z", server: "github", tool: "issues", outcome: "error" }),
      JSON.stringify({ ts: "2026-11-01T11:00:00.000Z", server: "old", tool: "ignored", outcome: "success" }),
      "{torn",
    ].join("\n"));
    expect(readMcpxCallLedger(dir, now)).toEqual({
      calls: 2,
      errors: 1,
      topTools: [{ name: "github.issues", count: 2 }],
    });
  });

  test("missing ledger directory degrades to zero recorded calls", () => {
    const dir = ledgerDir();
    rmSync(dir, { recursive: true, force: true });
    expect(readMcpxCallLedger(dir, Date.now())).toEqual({ calls: 0, errors: 0, topTools: [] });
  });
});
