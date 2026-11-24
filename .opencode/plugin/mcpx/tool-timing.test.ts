/**
 * plugin.toolTiming, observed through the real tool.execute.after
 * hook (#8472).
 *
 * The toolTiming setting — MCPX_PLUGIN_TOOL_TIMING, off by default —
 * decides whether the plugin host reports opencode's own tool calls
 * into mcpx's durable log: when on, the "tool.execute.after" hook in
 * mcpx-session.ts sends one `harness.tool` record per tool call to
 * the daemon (DaemonClient.record, POST /v1/log), so one
 * `mcpx stats` covers harness calls too; when off, the hook returns
 * before anything is sent. The choice lives entirely inside this
 * plugin host, so this suite is where its effect can be observed.
 * The Go effect probe for the setting
 * (TestPluginToolTimingRecordsToolCallsFromTheEnvironment in
 * internal/e2e) runs this file. The cases set the setting the way a
 * user would — MCPX_PLUGIN_TOOL_TIMING in the process environment
 * before the plugin loads — point discovery at a stub daemon that
 * answers /v1/health with the plugin's own operation fingerprint
 * (the daemon.test.ts fake's shape) and captures /v1/log posts, run
 * the ordinary caller (the tool.execute.after hook opencode invokes
 * for every finished tool call), and assert the observable
 * with/without difference the effect guard (#1971) demands: with
 * timing on, the record arrives carrying the call's tool, session,
 * call id and title; with timing off or unset, the daemon is
 * reachable (its health endpoint was hit) yet no record is posted.
 *
 *   bun test plugin/opencode/mcpx/tool-timing.test.ts
 */
import { afterEach, describe, expect, mock, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { OPERATION_FINGERPRINT } from "../../core/ops.gen.ts";

// The plugin package has no installed node_modules in the repo
// checkout; only the `tool` helper is needed to build the hooks.
const schemaStub: any = new Proxy(function () {}, {
  get: () => schemaStub,
  apply: () => schemaStub,
});
mock.module("@opencode-ai/plugin", () => ({
  tool: Object.assign((x: unknown) => x, { schema: schemaStub }),
}));
// @ts-ignore: the session plugin exports only its default (loader glob).
const plugin = (await import("../mcpx-session.ts")).default;

// Captured before any test runs: tmpdir() reads TMPDIR, and the
// environment restore below must not strand it.
const REAL_TMP = tmpdir();
const scratch: string[] = [];
const stoppers: Array<() => void> = [];
const savedEnv = { ...process.env };
afterEach(() => {
  // Restore in place, on the original process.env object (the
  // env.test.ts pattern): replacing process.env wholesale does not
  // restore the native environment other code reads.
  for (const key of Object.keys(process.env)) {
    if (!(key in savedEnv)) delete process.env[key];
  }
  Object.assign(process.env, savedEnv);
  while (stoppers.length) stoppers.pop()!();
  for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true });
});

type Captured = { health: number; logs: Array<Record<string, unknown>> };

// A stub daemon close enough for discovery and DaemonClient: health
// with the plugin's own operation fingerprint, an empty status, and
// a /v1/log sink that captures the records posted to it.
const stubDaemon = (): { endpoint: string; captured: Captured } => {
  const captured: Captured = { health: 0, logs: [] };
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/v1/health") {
        captured.health += 1;
        return Response.json({
          status: "ok",
          version: "test",
          pid: 1,
          operationFingerprint: OPERATION_FINGERPRINT,
        });
      }
      if (url.pathname === "/v1/status") {
        return Response.json({
          version: "test",
          operationFingerprint: OPERATION_FINGERPRINT,
          pid: 1,
          servers: [],
        });
      }
      if (url.pathname === "/v1/log" && req.method === "POST") {
        captured.logs.push((await req.json()) as Record<string, unknown>);
        return Response.json({ ok: true });
      }
      return new Response("not found", { status: 404 });
    },
  });
  stoppers.push(() => server.stop(true));
  return { endpoint: `http://127.0.0.1:${server.port}`, captured };
};

// toolAfter loads the plugin with MCPX_PLUGIN_TOOL_TIMING set to
// `timing` (deleted from the environment when undefined), pointed at
// the stub daemon, and runs one tool.execute.after invocation the
// way opencode would for a finished tool call.
const toolAfter = async (timing: string | undefined) => {
  if (timing === undefined) delete process.env.MCPX_PLUGIN_TOOL_TIMING;
  else process.env.MCPX_PLUGIN_TOOL_TIMING = timing;
  const { endpoint, captured } = stubDaemon();
  process.env.MCPX_DAEMON_ENDPOINT = endpoint;
  const dir = mkdtempSync(join(REAL_TMP, "mcpx-tool-timing-"));
  scratch.push(dir);
  const client = {
    session: {
      get: async () => ({
        data: {
          id: "ses_1",
          title: "a title",
          directory: dir,
          version: "1.2.3",
          time: { created: Date.now() - 60_000 },
        },
      }),
    },
    tui: { showToast: async () => {} },
  };
  const hooks: any = await (plugin as any)(
    { directory: dir, worktree: dir, project: { id: "p1" }, client },
    { headless: true },
  );
  await hooks["tool.execute.after"](
    { tool: "ju_tool", sessionID: "ses_1", callID: "call_1" },
    { title: "ran ju_tool" },
  );
  return captured;
};

describe("plugin.toolTiming (MCPX_PLUGIN_TOOL_TIMING)", () => {
  test("on: the finished tool call is recorded to the daemon's log", async () => {
    const captured = await toolAfter("1");
    expect(captured.health).toBeGreaterThanOrEqual(1);
    expect(captured.logs).toEqual([
      {
        event: "harness.tool",
        tool: "ju_tool",
        session: "ses_1",
        call: "call_1",
        title: "ran ju_tool",
      },
    ]);
  });

  test("unset: the daemon is reachable but nothing is recorded — off by default", async () => {
    const captured = await toolAfter(undefined);
    expect(captured.health).toBeGreaterThanOrEqual(1);
    expect(captured.logs).toEqual([]);
  });

  test("explicitly false: nothing is recorded either", async () => {
    const captured = await toolAfter("false");
    expect(captured.health).toBeGreaterThanOrEqual(1);
    expect(captured.logs).toEqual([]);
  });
});
