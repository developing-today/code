// Sync door tests (#12591). Oracles: the issue's acceptance and
// design §7.2 — project-reconcile's write set runs end to end
// through the endpoint with the same GitHub end state as the
// script path (an item-add lands the item with its initial
// status; an item-edit lands the Status change), and a retry of
// a landed flow writes nothing twice (the retry-safety rule:
// already-applied state is checked before every write). The
// rotation-level failover case lives in test/github.test.ts.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { AuthDeps } from "../src/auth.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { dispatch } from "../src/router.ts";
import { BINDING_A, BINDING_B, SERVICE_TOKEN, bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();
const token = await h.sign(h.rpPayload());

const OWNER = "developing-today";
const PROJECT_NUMBER = 3;
const REPO = "developing-today/code";

function syncDeps(github: FixtureGitHubWriter): AuthDeps {
  return { ...h.deps, githubProjects: github, githubComments: github };
}

function post(path: string, body: unknown, deps: AuthDeps, tok: string = token): Promise<Response> {
  return dispatch(
    bearerReq(path, tok, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: typeof body === "string" ? body : JSON.stringify(body),
    }),
    deps,
  );
}

/** A board in the state project-reconcile.sh starts a pass from:
 * issue #7 is on the project at Todo; issue #42 exists on GitHub
 * (created outside the service) but is not on the project yet. */
function seededBoard(): FixtureGitHubWriter {
  const github = new FixtureGitHubWriter();
  github.seedItem({ repo: REPO, kind: "issue", number: 7, body: "already tracked" });
  github.seedItem({ repo: REPO, kind: "issue", number: 42, body: "discovered work" });
  github.seedProject({
    owner: OWNER,
    number: PROJECT_NUMBER,
    items: [{ repo: REPO, kind: "issue", number: 7, values: { Status: "Todo" } }],
  });
  return github;
}

const RECONCILE_OPS = {
  project: { owner: OWNER, number: PROJECT_NUMBER },
  ops: [
    { op: "add-issue", repo: REPO, number: 42, status: "Todo" },
    { op: "set-status", issue: `${REPO}#7`, to: "Done" },
  ],
};

test("project-reconcile's write set runs end to end with the script path's end state", async () => {
  const github = seededBoard();
  const res = await post("/v1/sync/project", RECONCILE_OPS, syncDeps(github));
  assert.equal(res.status, 200);
  const doc = (await res.json()) as {
    binding_id: string;
    results: { op: string; outcome: string; from?: string | null; to?: string }[];
    counts: { applied: number; already: number; failed: number };
    retires: string[];
  };
  assert.equal(doc.binding_id, BINDING_A);
  assert.deepEqual(
    doc.results.map((r) => [r.op, r.outcome]),
    [
      ["add-issue", "added"],
      ["set-status", "status_set"],
    ],
  );
  assert.equal(doc.results[1].from, "Todo");
  assert.equal(doc.results[1].to, "Done");
  assert.deepEqual(doc.counts, { applied: 2, already: 0, failed: 0 });
  // The endpoint names the raw-gh call sites it retires.
  assert.ok(doc.retires.some((r) => r.includes("gh project item-add")));
  assert.ok(doc.retires.some((r) => r.includes("gh project item-edit")));

  // The GitHub end state is the script path's: #42 is on the
  // project at Todo (item-add + the add's initial status set),
  // #7's Status is Done (item-edit).
  const state = await github.getProjectState(OWNER, PROJECT_NUMBER);
  const byNumber = new Map(state!.items.map((i) => [i.number, i]));
  assert.equal(byNumber.get(42)?.values["Status"], "Todo");
  assert.equal(byNumber.get(7)?.values["Status"], "Done");
});

test("a full retry of a landed project sync writes nothing twice", async () => {
  const github = seededBoard();
  const deps = syncDeps(github);
  await post("/v1/sync/project", RECONCILE_OPS, deps);
  const before = await github.getProjectState(OWNER, PROJECT_NUMBER);

  const res = await post("/v1/sync/project", RECONCILE_OPS, deps);
  assert.equal(res.status, 200);
  const doc = (await res.json()) as {
    results: { outcome: string }[];
    counts: { applied: number; already: number; failed: number };
  };
  assert.deepEqual(
    doc.results.map((r) => r.outcome),
    ["already_present", "already_set"],
  );
  assert.deepEqual(doc.counts, { applied: 0, already: 2, failed: 0 });
  // Identical end state, and still exactly one item per issue.
  assert.deepEqual(await github.getProjectState(OWNER, PROJECT_NUMBER), before);
});

test("set-status accepts the script's resolved item_id shape", async () => {
  const github = seededBoard();
  const state = await github.getProjectState(OWNER, PROJECT_NUMBER);
  const itemId = state!.items[0].itemId;
  const res = await post(
    "/v1/sync/project",
    {
      project: { owner: OWNER, number: PROJECT_NUMBER },
      ops: [{ op: "set-status", item_id: itemId, field: "Status", from: "Todo", to: "In Review" }],
    },
    syncDeps(github),
  );
  assert.equal(res.status, 200);
  const doc = (await res.json()) as { results: { outcome: string }[] };
  assert.equal(doc.results[0].outcome, "status_set");
  const after = await github.getProjectState(OWNER, PROJECT_NUMBER);
  assert.equal(after!.items[0].values["Status"], "In Review");
});

test("per-op failures are recorded per op and never abort the flow", async () => {
  const github = seededBoard();
  const res = await post(
    "/v1/sync/project",
    {
      project: { owner: OWNER, number: PROJECT_NUMBER },
      ops: [
        { op: "set-status", issue: `${REPO}#999`, to: "Done" },
        { op: "set-status", issue: `${REPO}#7`, to: "Sideways" },
        { op: "add-issue", repo: REPO, number: 12345 },
        { op: "set-status", issue: `${REPO}#7`, to: "In Progress" },
      ],
    },
    syncDeps(github),
  );
  assert.equal(res.status, 200);
  const doc = (await res.json()) as {
    results: { outcome: string; error?: string }[];
    counts: { applied: number; already: number; failed: number };
  };
  assert.deepEqual(
    doc.results.map((r) => [r.outcome, r.error]),
    [
      ["failed", "item_not_in_project"],
      ["failed", "option_not_found"],
      ["failed", "content_not_found"],
      ["status_set", undefined],
    ],
  );
  assert.deepEqual(doc.counts, { applied: 1, already: 0, failed: 3 });
});

test("an invisible project refuses 404 before any op runs", async () => {
  const github = seededBoard();
  const res = await post(
    "/v1/sync/project",
    { project: { owner: OWNER, number: 99 }, ops: [{ op: "add-issue", repo: REPO, number: 42 }] },
    syncDeps(github),
  );
  assert.equal(res.status, 404);
  const doc = (await res.json()) as { error: string };
  assert.equal(doc.error, "project_not_visible");
});

test("malformed project-sync bodies refuse 400 naming the rule", async () => {
  const deps = syncDeps(seededBoard());
  const cases: [string, unknown][] = [
    ["not json", "{"],
    ["unknown op", { project: { owner: OWNER, number: 3 }, ops: [{ op: "move", issue: "x/y#1" }] }],
    [
      "set-status with no item reference",
      { project: { owner: OWNER, number: 3 }, ops: [{ op: "set-status", to: "Done" }] },
    ],
    ["empty ops", { project: { owner: OWNER, number: 3 }, ops: [] }],
    [
      "bad issue reference",
      { project: { owner: OWNER, number: 3 }, ops: [{ op: "set-status", issue: "nope", to: "Done" }] },
    ],
    ["project without number", { project: { owner: OWNER }, ops: [{ op: "add-issue", repo: REPO, number: 1 }] }],
  ];
  for (const [name, body] of cases) {
    const res = await post("/v1/sync/project", body, deps);
    assert.equal(res.status, 400, name);
    const doc = (await res.json()) as { error: string };
    assert.equal(doc.error, "invalid_body", name);
  }
  const tooMany = await post(
    "/v1/sync/project",
    {
      project: { owner: OWNER, number: 3 },
      ops: Array.from({ length: 51 }, () => ({ op: "add-issue", repo: REPO, number: 1 })),
    },
    deps,
  );
  assert.equal(tooMany.status, 400);
});

test("unconfigured surfaces fail closed 503 on both sync routes", async () => {
  for (const path of ["/v1/sync/project", "/v1/sync/queue-github"]) {
    const res = await post(path, {}, h.deps);
    assert.equal(res.status, 503, path);
    const doc = (await res.json()) as { error: string };
    assert.equal(doc.error, "sync_not_configured", path);
  }
});

test("a cross-tenant assertion header is refused 403 on the sync door", async () => {
  const res = await dispatch(
    bearerReq("/v1/sync/project", token, {
      method: "POST",
      headers: { "x-binding-id": BINDING_B, "content-type": "application/json" },
      body: JSON.stringify(RECONCILE_OPS),
    }),
    syncDeps(seededBoard()),
  );
  assert.equal(res.status, 403);
  const doc = (await res.json()) as { error: string };
  assert.equal(doc.error, "cross_tenant_refused");
});

test("a service token scoped to other doors is refused 403 on the sync door", async () => {
  const res = await post("/v1/sync/project", RECONCILE_OPS, syncDeps(seededBoard()), SERVICE_TOKEN);
  assert.equal(res.status, 403);
  const doc = (await res.json()) as { error: string };
  assert.equal(doc.error, "service_token_route_scope");
});

// --- Queue/GitHub flows -------------------------------------------

const VERDICT = {
  flow: "verdict",
  repo: REPO,
  number: 7,
  marker: "stewardship-cycle-566",
  body: "Stewardship verdict: accept.",
  labels: { add: ["review/accepted"], remove: [] },
};

test("a verdict flow posts one marked note and applies its labels", async () => {
  const github = seededBoard();
  const res = await post("/v1/sync/queue-github", VERDICT, syncDeps(github));
  assert.equal(res.status, 200);
  const doc = (await res.json()) as {
    flow: string;
    binding_id: string;
    comment: { outcome: string; url?: string };
    labels: { added: string[] } | null;
    retires: string[];
  };
  assert.equal(doc.flow, "verdict");
  assert.equal(doc.binding_id, BINDING_A);
  assert.equal(doc.comment.outcome, "posted");
  assert.ok(doc.comment.url?.includes("#issuecomment-"));
  assert.deepEqual(doc.labels?.added, ["review/accepted"]);
  assert.ok(doc.retires.some((r) => r.includes("gh issue comment")));

  assert.equal(github.comments.length, 1);
  assert.ok(github.comments[0].body.includes("Stewardship verdict: accept."));
  assert.ok(
    github.comments[0].body.includes("<!-- fleet-sync:verdict:stewardship-cycle-566 -->"),
    "the posted note carries the flow marker",
  );
  assert.deepEqual(github.labelsFor(REPO, 7), ["review/accepted"]);
});

test("a retried flow posts nothing twice but still finishes its labels", async () => {
  const github = seededBoard();
  const deps = syncDeps(github);
  await post("/v1/sync/queue-github", VERDICT, deps);
  // Simulate the first attempt having failed its label flip:
  // clear the labels, then retry the identical flow request.
  await github.editIssueLabels(REPO, 7, [], ["review/accepted"]);
  assert.deepEqual(github.labelsFor(REPO, 7), []);

  const res = await post("/v1/sync/queue-github", VERDICT, deps);
  assert.equal(res.status, 200);
  const doc = (await res.json()) as { comment: { outcome: string } };
  assert.equal(doc.comment.outcome, "already_posted");
  assert.equal(github.comments.length, 1, "no duplicate note");
  assert.deepEqual(github.labelsFor(REPO, 7), ["review/accepted"], "labels finished by the retry");

  // A different marker is a different flow instance: it posts.
  const next = await post("/v1/sync/queue-github", { ...VERDICT, marker: "stewardship-cycle-567" }, deps);
  const nextDoc = (await next.json()) as { comment: { outcome: string } };
  assert.equal(nextDoc.comment.outcome, "posted");
  assert.equal(github.comments.length, 2);
});

test("tend and review flows post under their own marker namespaces", async () => {
  const github = seededBoard();
  const deps = syncDeps(github);
  for (const flow of ["tend", "review"] as const) {
    const res = await post(
      "/v1/sync/queue-github",
      { flow, repo: REPO, number: 7, marker: "round-1", body: `${flow} note` },
      deps,
    );
    assert.equal(res.status, 200, flow);
    const doc = (await res.json()) as { comment: { outcome: string }; labels: unknown };
    assert.equal(doc.comment.outcome, "posted", flow);
    assert.equal(doc.labels, null, "no labels named, none applied");
  }
  // Same marker string, different flows: both posted (the
  // namespace is part of the dedupe key).
  assert.equal(github.comments.length, 2);
});

test("malformed flow bodies refuse 400 naming the rule", async () => {
  const deps = syncDeps(seededBoard());
  const cases: [string, unknown][] = [
    ["unknown flow", { ...VERDICT, flow: "announce" }],
    ["marker with whitespace", { ...VERDICT, marker: "cycle 566" }],
    ["missing body", { flow: "verdict", repo: REPO, number: 7, marker: "m1" }],
    ["bad repo", { ...VERDICT, repo: "no-slash" }],
    ["labels not lists", { ...VERDICT, labels: { add: "review/accepted" } }],
  ];
  for (const [name, body] of cases) {
    const res = await post("/v1/sync/queue-github", body, deps);
    assert.equal(res.status, 400, name);
    const doc = (await res.json()) as { error: string };
    assert.equal(doc.error, "invalid_body", name);
  }
});
