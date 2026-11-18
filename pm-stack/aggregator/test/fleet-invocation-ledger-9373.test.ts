import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readFleetInvocationLedger } from "../src/analytics-store.ts";

const dirs: string[] = [];
afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

describe("fleet invocation ledger (#9373)", () => {
  test("counts valid versioned records in the seven-day window", () => {
    const dir = mkdtempSync(join(tmpdir(), "fleet-ledger-"));
    dirs.push(dir);
    const file = join(dir, "invocations.jsonl");
    const now = Date.parse("2026-11-18T12:00:00Z");
    writeFileSync(file, [
      JSON.stringify({ schema: "fleet-invocation/v1", at_unix_ms: now - 1_000, command: "queue", exit_code: 0 }),
      JSON.stringify({ schema: "fleet-invocation/v1", at_unix_ms: now - 2_000, command: "queue", exit_code: 2 }),
      JSON.stringify({ schema: "fleet-invocation/v1", at_unix_ms: now - 8 * 86_400_000, command: "secrets", exit_code: 0 }),
      "{torn",
    ].join("\n"));
    expect(readFleetInvocationLedger(file, now)).toEqual({
      readable: true,
      total: 2,
      byCommand: [{ subcommand: "queue", count: 2 }],
    });
  });

  test("distinguishes a readable empty ledger from a missing ledger", () => {
    const dir = mkdtempSync(join(tmpdir(), "fleet-ledger-"));
    dirs.push(dir);
    const file = join(dir, "empty.jsonl");
    writeFileSync(file, "");
    expect(readFleetInvocationLedger(file, Date.now())).toEqual({ readable: true, total: 0, byCommand: [] });
    expect(readFleetInvocationLedger(join(dir, "missing.jsonl"), Date.now())).toEqual({ readable: false, total: 0, byCommand: [] });
  });
});
