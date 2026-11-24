// pm-console demo: the act log the offline check counts on.
//
// Oracle: the file the demo appends to, read back here. A canned action
// is one JSON line (kind, args, time); with the variable unset nothing
// is written, so the demo is unchanged for everyone else.
import { describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { demoDeps } from "../scripts/demo-state.ts";

describe("demo act log", () => {
  test("records a canned action as one JSON line, and writes nothing when unset", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pm-console-demo-act-log-"));
    try {
      const log = join(dir, "acts.jsonl");
      process.env.PM_CONSOLE_DEMO_ACT_LOG = log;
      await demoDeps.actions!["question.answer"].run({
        set: "2026-11-17",
        ledger: "ledger-2026-11-17-7",
        qid: "q7",
        option: "opt-1",
      });
      const lines = readFileSync(log, "utf8").trim().split("\n");
      expect(lines).toHaveLength(1);
      const entry = JSON.parse(lines[0]) as { at: string; kind: string; args: unknown };
      expect(entry.kind).toBe("question.answer");
      expect(entry.args).toEqual({
        set: "2026-11-17",
        ledger: "ledger-2026-11-17-7",
        qid: "q7",
        option: "opt-1",
      });
      expect(typeof entry.at).toBe("string");
      delete process.env.PM_CONSOLE_DEMO_ACT_LOG;
      await demoDeps.actions!["question.answer"].run({
        set: "2026-11-17",
        ledger: "ledger-2026-11-17-7",
        qid: "q7",
        option: "opt-1",
      });
      expect(readFileSync(log, "utf8").trim().split("\n")).toHaveLength(1);
    } finally {
      delete process.env.PM_CONSOLE_DEMO_ACT_LOG;
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
