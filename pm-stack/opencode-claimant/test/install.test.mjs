import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  AGENT, CRON, MODEL_ID, PROVIDER_ID, TASK_ID,
  buildTask, mergeTasks,
} from "../install.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const prompt = await readFile(join(here, "..", "prompt.md"), "utf8");

describe("buildTask", () => {
  test("builds a disabled-by-default OpenChamber scheduled task", () => {
    const task = buildTask(prompt);
    expect(task.id).toBe(TASK_ID);
    expect(task.enabled).toBe(false);
    expect(task.schedule).toEqual({
      kind: "cron", cron: CRON, timezone: "America/Chicago",
    });
    expect(task.execution.providerID).toBe(PROVIDER_ID);
    expect(task.execution.modelID).toBe(MODEL_ID);
    expect(task.execution.agent).toBe(AGENT);
    expect(task.execution.prompt).toBe(prompt);
  });

  test("embeds the cap, routing, and shadow rules from the prompt", () => {
    for (const needle of ["12", "workhorse", "shadow",
                          "opencode-claimant", "poll.py"]) {
      expect(prompt).toContain(needle);
    }
  });

  test("rejects a foreign prompt", () => {
    expect(() => buildTask("unrelated prompt")).toThrow();
  });
});

describe("mergeTasks", () => {
  const other = { id: "other-task", name: "keep me", enabled: true };

  test("inserts without disturbing other tasks", () => {
    const store = { version: 1, scheduledTasks: [other] };
    const merged = mergeTasks(store, buildTask(prompt));
    expect(merged.scheduledTasks).toHaveLength(2);
    expect(merged.scheduledTasks[0]).toEqual(other);
    expect(merged.scheduledTasks[1].id).toBe(TASK_ID);
  });

  test("is idempotent: merging twice deep-equals merging once", () => {
    const store = { version: 1, scheduledTasks: [other] };
    const task = buildTask(prompt);
    const once = mergeTasks(store, task);
    const twice = mergeTasks(once, task);
    expect(twice).toEqual(once);
  });

  test("updates in place instead of duplicating", () => {
    const store = { version: 1, scheduledTasks: [buildTask("old #3026 x")] };
    const merged = mergeTasks(store, buildTask(prompt));
    expect(merged.scheduledTasks).toHaveLength(1);
    expect(merged.scheduledTasks[0].execution.prompt).toBe(prompt);
  });

  test("refuses a store without a scheduledTasks array", () => {
    expect(() => mergeTasks({}, buildTask(prompt))).toThrow();
    expect(() => mergeTasks(null, buildTask(prompt))).toThrow();
  });
});
