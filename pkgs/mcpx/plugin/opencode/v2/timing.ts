/**
 * Tool timing from tool.hook("execute.after") (design doc §2.1). Unlike v1,
 * the hook input is a status union — { status: "completed", result } or
 * { status: "error", ... } — so failures are recorded as first-class rows
 * instead of being invisible. Records are kept in memory for the session and
 * handed to an optional sink (the caller decides where timings go).
 */
export type ToolTimingRecord = {
  tool: string;
  sessionID: string;
  status: "completed" | "error";
  at: number;
};

export type ExecuteAfterInput = {
  tool: string;
  sessionID: string;
  status: "completed" | "error";
};

export function createTimingLog(sink?: (record: ToolTimingRecord) => void): {
  hook: (input: ExecuteAfterInput) => ToolTimingRecord;
  records: ToolTimingRecord[];
} {
  const records: ToolTimingRecord[] = [];
  return {
    records,
    hook(input) {
      const record: ToolTimingRecord = {
        tool: input.tool,
        sessionID: input.sessionID,
        status: input.status,
        at: Date.now(),
      };
      records.push(record);
      sink?.(record);
      return record;
    },
  };
}
