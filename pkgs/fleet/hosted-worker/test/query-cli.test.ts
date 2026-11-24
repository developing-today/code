// TS CLI query verb tests (#12587). Oracles: design §3.2 (the CLI
// owns rendering, JSON first) and §7.1 (the CLI verbs render the
// same results as the REST surface) — so the CLI's flags must build
// exactly the filter the REST door validates, through the same
// parseQueryFilter, and its rendering must be the response document
// itself, unchanged.

import assert from "node:assert/strict";
import { test } from "node:test";
import { FailClosed } from "../src/errors.ts";
import { parseQueryFilter, type QueryResponse } from "../src/query.ts";
import { parseQueryCliArgs, renderQueryResponse } from "../src/query-cli.ts";

test("CLI flags build the same filter as the REST parse", () => {
  const viaCli = parseQueryCliArgs([
    "--repo",
    "developing-today/code",
    "--kind",
    "issue",
    "--direct-parent",
    "developing-today/code#9081",
    "--session",
    "ses_abc",
    "--lane",
    "code-a",
    "--host",
    "amd-6",
    "--queue-id",
    "q-1",
    "--link-relation",
    "parent",
    "--link-status",
    "current",
    "--link-target",
    "developing-today/code#101",
  ]);
  const viaRest = parseQueryFilter({
    repo: "developing-today/code",
    kind: "issue",
    direct_parent: "developing-today/code#9081",
    session_id: "ses_abc",
    lane: "code-a",
    host: "amd-6",
    queue_id: "q-1",
    link_relation: "parent",
    link_status: "current",
    link_target: "developing-today/code#101",
  });
  assert.deepEqual(viaCli, viaRest);
  // No flags is the empty filter (list the caller's items).
  assert.deepEqual(parseQueryCliArgs([]), {});
});

test("unknown flags and missing values refuse, never silently drop", () => {
  for (const argv of [["--bogus", "x"], ["--lane"], ["--lane", "--host", "amd-6"], ["code-a"]]) {
    assert.throws(
      () => parseQueryCliArgs(argv),
      (e: unknown) => e instanceof FailClosed && e.code === "invalid_query",
      argv.join(" "),
    );
  }
  // And a bad value fails in the shared validation, with its code.
  assert.throws(
    () => parseQueryCliArgs(["--kind", "epic"]),
    (e: unknown) => e instanceof FailClosed && e.code === "invalid_query",
  );
});

test("rendering is the response document itself, JSON first", () => {
  const response: QueryResponse = {
    filter: { lane: "code-a" },
    count: 1,
    results: [
      {
        repo: "developing-today/code",
        kind: "issue",
        number: 101,
        url: "https://github.com/developing-today/code/issues/101",
        metadata: { pm_meta: 1 },
        source: "db",
        body_checked: true,
        divergence: null,
        stamp: { status: "unsigned" },
      },
    ],
  };
  const rendered = renderQueryResponse(response);
  assert.ok(rendered.startsWith("{"), "JSON first: the document opens the output");
  assert.ok(rendered.endsWith("}\n"));
  assert.deepEqual(JSON.parse(rendered), response);
});
