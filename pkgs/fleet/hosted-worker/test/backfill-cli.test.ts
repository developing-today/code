// TS CLI backfill verb tests (#14312). Oracles: the operator
// contract (#3352) — the verbs' whole command line is declared
// and parsed strictly, so an unknown flag, a missing value, or a
// malformed repo refuses (invalid_cli) before any service call —
// and the wiring: `backfill` posts its scope to the backfill
// route, `backfill runs` GETs the recorded runs (the CLI's one
// read verb), and `divergence` posts its scope to the divergence
// route, each rendering the service's document JSON-first.

import assert from "node:assert/strict";
import { test } from "node:test";
import { parseBackfillRunsArgs, parseRepoScopeArgs } from "../src/backfill-cli.ts";
import { runCli, type CliDeps } from "../src/cli.ts";
import { FailClosed } from "../src/errors.ts";
import { InMemorySessionStateStore } from "../src/session-resolution.ts";

function assertInvalidCli(fn: () => unknown): void {
  assert.throws(fn, (e: unknown) => e instanceof FailClosed && e.code === "invalid_cli");
}

test("parseRepoScopeArgs: no flags is the configured scope (empty list)", () => {
  assert.deepEqual(parseRepoScopeArgs([], "backfill"), []);
});

test("parseRepoScopeArgs: repeated --repo collects the scope in order", () => {
  assert.deepEqual(parseRepoScopeArgs(["--repo", "developing-today/code", "--repo", "dezren39/hatch"], "divergence"), [
    "developing-today/code",
    "dezren39/hatch",
  ]);
});

test("parseRepoScopeArgs: unknown flags, missing values and malformed repos refuse", () => {
  assertInvalidCli(() => parseRepoScopeArgs(["--all"], "backfill"));
  assertInvalidCli(() => parseRepoScopeArgs(["--repo"], "backfill"));
  assertInvalidCli(() => parseRepoScopeArgs(["--repo", "--repo", "a/b"], "backfill"));
  assertInvalidCli(() => parseRepoScopeArgs(["--repo", "no-slash"], "divergence"));
  assertInvalidCli(() => parseRepoScopeArgs(["a/b"], "divergence"));
});

test("parseBackfillRunsArgs: takes no arguments at all", () => {
  parseBackfillRunsArgs([]);
  assertInvalidCli(() => parseBackfillRunsArgs(["--repo", "a/b"]));
});

// --- Wiring: the verbs reach the routes with the parsed scope.

interface RecordedCall {
  method: string;
  path: string;
  body: string;
}

function makeDeps(calls: RecordedCall[]): CliDeps {
  return {
    serviceUrl: "https://fleet-hosted.test",
    token: "caller-token",
    fetch: async (req: Request) => {
      calls.push({ method: req.method, path: new URL(req.url).pathname, body: await req.text() });
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    },
    env: () => null,
    store: new InMemorySessionStateStore(),
    readFile: () => {
      throw new Error("no file reads in these tests");
    },
    hostname: () => "test-host",
    cwd: () => "/tmp",
  };
}

test("backfill verb posts the named scope to the backfill route", async () => {
  const calls: RecordedCall[] = [];
  const out = await runCli(["backfill", "--repo", "developing-today/code"], makeDeps(calls));
  assert.equal(out.exitCode, 0, out.stderr);
  assert.deepEqual(calls, [
    { method: "POST", path: "/v1/ingestion/backfill", body: JSON.stringify({ repos: ["developing-today/code"] }) },
  ]);
  assert.equal(out.stdout, `${JSON.stringify({ ok: true }, null, 2)}\n`);
});

test("backfill verb with no --repo posts an empty document (the configured scope)", async () => {
  const calls: RecordedCall[] = [];
  const out = await runCli(["backfill"], makeDeps(calls));
  assert.equal(out.exitCode, 0, out.stderr);
  assert.deepEqual(calls, [{ method: "POST", path: "/v1/ingestion/backfill", body: "{}" }]);
});

test("backfill runs GETs the recorded runs", async () => {
  const calls: RecordedCall[] = [];
  const out = await runCli(["backfill", "runs"], makeDeps(calls));
  assert.equal(out.exitCode, 0, out.stderr);
  assert.deepEqual(calls, [{ method: "GET", path: "/v1/ingestion/backfill/runs", body: "" }]);
});

test("divergence verb posts the named scope to the divergence route", async () => {
  const calls: RecordedCall[] = [];
  const out = await runCli(["divergence", "--repo", "developing-today/code"], makeDeps(calls));
  assert.equal(out.exitCode, 0, out.stderr);
  assert.deepEqual(calls, [
    { method: "POST", path: "/v1/ingestion/divergence", body: JSON.stringify({ repos: ["developing-today/code"] }) },
  ]);
});

test("a malformed scope refuses before any service call", async () => {
  const calls: RecordedCall[] = [];
  const out = await runCli(["backfill", "--repo", "no-slash"], makeDeps(calls));
  assert.equal(out.exitCode, 1);
  assert.equal((JSON.parse(out.stderr) as { error: string }).error, "invalid_cli");
  assert.deepEqual(calls, []);
});
