import { describe, expect, it } from "vitest";
import { buildRunRow, dedupeKey, skipReason } from "../src/policy";

const failurePayload = {
  action: "completed",
  workflow_run: {
    id: 12345,
    run_attempt: 2,
    name: "CI",
    workflow_id: 777,
    head_branch: "main",
    head_sha: "abc123",
    status: "completed",
    conclusion: "failure",
    actor: { login: "dezren39" },
    html_url: "https://github.com/x/y/actions/runs/12345",
    run_started_at: "2026-11-07T10:00:00Z",
    updated_at: "2026-11-07T10:05:00Z",
  },
  repository: { full_name: "developing-today/code" },
};

describe("v4 ingest policy", () => {
  it("stores completed non-skipped runs only", () => {
    expect(skipReason("completed", "success")).toBeNull();
    expect(skipReason("completed", "failure")).toBeNull();
    expect(skipReason("requested", null)).toBe("not_completed");
    expect(skipReason("completed", "skipped")).toBe("skipped_conclusion");
  });
  it("dedupe key matches the Edge Function format", () => {
    expect(dedupeKey("developing-today/code", 12345, 2, "completed")).toBe(
      "github:developing-today/code:12345:2:completed",
    );
  });
  it("keeps the raw payload only for failures", () => {
    const fail = buildRunRow(failurePayload, "developing-today/code", "workflow_run", "2026-11-07T16:05:00Z");
    expect(fail.row.payload).toContain("workflow_run");
    expect(fail.row.dedupe_key).toBe("github:developing-today/code:12345:2:completed");
    const okPayload = structuredClone(failurePayload) as typeof failurePayload;
    okPayload.workflow_run.conclusion = "success";
    const ok = buildRunRow(okPayload, "developing-today/code", "workflow_run", "2026-11-07T16:05:00Z");
    expect(ok.row.payload).toBe("{}");
  });
});
