// lane-scratch: code-c /tmp/lane-code-c-20261116-0412 task 16482
import { describe, expect, test } from "bun:test";
import { parseQueueSnapshotResult } from "../src/ledger";

// Oracle: the live Turso HTTP pipeline v2 response for
//   SELECT payload, at FROM status_snapshots WHERE kind='queue_state'
// captured 2026-11-16 during the #9800 deploy proof (task 16482).
// Rows are bare arrays of typed values — the original code read
// `rows[0].values` (an object shape the API does not return), which
// resolved to Array.prototype.values and threw
// "function is not iterable" in the Durable Object.
const LIVE_SHAPE = {
  results: [
    {
      type: "ok",
      response: {
        type: "execute",
        result: {
          cols: [
            { name: "payload", decltype: "TEXT" },
            { name: "at", decltype: "TEXT" },
          ],
          rows: [
            [
              { type: "text", value: '{"queued":116,"claimed":10,"parked":18,"blocked":552}' },
              { type: "text", value: "2026-11-16T10:23:47.659Z" },
            ],
          ],
        },
      },
    },
    { type: "ok", response: { type: "close" } },
  ],
} as const;

describe("parseQueueSnapshotResult", () => {
  test("parses the live v2 row shape", () => {
    const snap = parseQueueSnapshotResult(LIVE_SHAPE as never);
    expect(snap).toEqual({
      queued: 116,
      claimed: 10,
      parked: 18,
      blocked: 552,
      at: "2026-11-16T10:23:47.659Z",
    });
  });

  test("no rows -> null", () => {
    const empty = {
      results: [
        { type: "ok", response: { type: "execute", result: { cols: [], rows: [] } } },
      ],
    };
    expect(parseQueueSnapshotResult(empty as never)).toBeNull();
  });
});
