// TS CLI sync verb tests (#12591). Oracle: design §3.2/§7.2 —
// the CLI builds the same request documents the REST door
// validates (one validation path: the parsers run the built
// shape through src/sync.ts's own parsers) and renders the
// response JSON-first.

import assert from "node:assert/strict";
import { test } from "node:test";
import { FailClosed } from "../src/errors.ts";
import { parseFlowSyncCliArgs, parseProjectSyncCliArgs, renderSyncResponse } from "../src/sync-cli.ts";

function refusal(fn: () => unknown): FailClosed {
  try {
    fn();
  } catch (e) {
    assert.ok(e instanceof FailClosed, `expected FailClosed, got ${String(e)}`);
    return e;
  }
  assert.fail("expected a FailClosed refusal");
}

test("sync project args build the door's request document, ops in argv order", () => {
  const request = parseProjectSyncCliArgs([
    "--owner",
    "developing-today",
    "--project",
    "3",
    "--add-issue",
    "developing-today/code#42",
    "--set-status",
    "developing-today/code#7=Done",
    "--set-item-status",
    "PVTI_fixture_9=In Review",
  ]);
  assert.deepEqual(request, {
    project: { owner: "developing-today", number: 3 },
    ops: [
      {
        op: "add-issue",
        repo: "developing-today/code",
        number: 42,
        kind: "issue",
        status: "Todo",
        contentId: null,
      },
      {
        op: "set-status",
        itemId: null,
        issue: { repo: "developing-today/code", number: 7 },
        field: "Status",
        to: "Done",
      },
      {
        op: "set-status",
        itemId: "PVTI_fixture_9",
        issue: null,
        field: "Status",
        to: "In Review",
      },
    ],
  });
});

test("--add-status names the initial status every add carries", () => {
  const request = parseProjectSyncCliArgs([
    "--owner",
    "o",
    "--project",
    "1",
    "--add-issue",
    "o/r#5",
    "--add-status",
    "In Progress",
  ]);
  const first = request.ops[0];
  assert.equal(first.op, "add-issue");
  if (first.op === "add-issue") assert.equal(first.status, "In Progress");
});

test("sync project arg refusals are named 400s", () => {
  assert.equal(refusal(() => parseProjectSyncCliArgs(["--owner", "o"])).code, "invalid_sync");
  assert.equal(refusal(() => parseProjectSyncCliArgs(["--owner", "o", "--project", "1"])).code, "invalid_sync");
  assert.equal(
    refusal(() => parseProjectSyncCliArgs(["--owner", "o", "--project", "1", "--set-status", "o/r#5"])).code,
    "invalid_sync",
  );
  assert.equal(
    refusal(() => parseProjectSyncCliArgs(["--owner", "o", "--project", "1", "--add-issue", "o/r5"])).code,
    "invalid_sync",
  );
  assert.equal(
    refusal(() => parseProjectSyncCliArgs(["--owner", "o", "--project", "1", "--bogus", "x"])).code,
    "invalid_sync",
  );
  // A shape the CLI builds but the door refuses refuses identically
  // (one validation path): a non-numeric project number.
  assert.equal(
    refusal(() => parseProjectSyncCliArgs(["--owner", "o", "--project", "x", "--add-issue", "o/r#5"])).code,
    "invalid_body",
  );
});

test("sync flow args build the door's request document", () => {
  const request = parseFlowSyncCliArgs([
    "--flow",
    "verdict",
    "--repo",
    "developing-today/code",
    "--number",
    "7",
    "--marker",
    "stewardship-cycle-566",
    "--body",
    "Stewardship verdict: accept.",
    "--add-label",
    "review/accepted",
    "--remove-label",
    "review/pending",
  ]);
  assert.deepEqual(request, {
    flow: "verdict",
    repo: "developing-today/code",
    number: 7,
    kind: "issue",
    marker: "stewardship-cycle-566",
    body: "Stewardship verdict: accept.",
    labels: { add: ["review/accepted"], remove: ["review/pending"] },
  });
});

test("sync flow requires its named fields and refuses unknown flags", () => {
  assert.equal(refusal(() => parseFlowSyncCliArgs(["--flow", "verdict", "--repo", "o/r"])).code, "invalid_sync");
  assert.equal(
    refusal(() =>
      parseFlowSyncCliArgs([
        "--flow",
        "verdict",
        "--repo",
        "o/r",
        "--number",
        "7",
        "--marker",
        "m1",
        "--body",
        "b",
        "--bogus",
        "x",
      ]),
    ).code,
    "invalid_sync",
  );
});

test("render is the response document, JSON-first with a trailing newline", () => {
  const out = renderSyncResponse({
    flow: "tend",
    repo: "o/r",
    kind: "issue",
    number: 1,
    binding_id: "a".repeat(32),
    comment: { outcome: "posted", url: "https://x/1" },
    labels: null,
    retires: [],
  });
  assert.ok(out.endsWith("}\n"));
  assert.equal(JSON.parse(out).comment.outcome, "posted");
});
