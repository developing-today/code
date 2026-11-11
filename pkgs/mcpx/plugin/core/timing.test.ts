/**
 * Core timing log (oracle: the createTimingLog contract in
 * timing.ts -- every execute.after input becomes one record, in
 * order, handed to the optional sink; failures are first-class
 * rows, not invisible).
 *
 *   bun test plugin/core/timing.test.ts
 */
import { describe, expect, test } from "bun:test";
import { createTimingLog, type ToolTimingRecord } from "./timing.ts";

describe("createTimingLog (oracle: one record per input, sink sees each)", () => {
  test("records completed and error rows in order and sinks them", () => {
    const seen: ToolTimingRecord[] = [];
    const log = createTimingLog((r) => seen.push(r));
    const a = log.hook({ tool: "mcpx_exec", sessionID: "s1", status: "completed" });
    const b = log.hook({ tool: "mcpx_search", sessionID: "s1", status: "error" });
    expect(log.records).toEqual([a, b]);
    expect(seen).toEqual([a, b]);
    expect(b.status).toBe("error");
    expect(typeof a.at).toBe("number");
  });

  test("works without a sink", () => {
    const log = createTimingLog();
    log.hook({ tool: "t", sessionID: "s", status: "completed" });
    expect(log.records.length).toBe(1);
  });
});
