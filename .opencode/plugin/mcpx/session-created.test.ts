import { describe, expect, mock, test } from "bun:test";

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

// #2355: the shell.env hook converts the server-supplied session
// created stamp. Out-of-range / non-finite stamps used to throw
// RangeError out of the hook on every full-level shell command.
const envFor = async (created: unknown): Promise<Record<string, string>> => {
  const client = {
    session: {
      get: async () => ({
        data: { id: "ses_1", title: "t", directory: "/tmp", version: "1", time: { created } },
      }),
    },
    tui: { showToast: async () => {} },
  };
  const hooks: any = await (plugin as any)(
    { directory: "/tmp", worktree: "/tmp", project: { id: "p" }, client },
    { headless: true },
  );
  const output: any = { env: {} };
  await hooks["shell.env"]({ cwd: "/tmp", sessionID: "ses_1", callID: "c1" }, output);
  return output.env as Record<string, string>;
};

describe("shell.env session created conversion (#2355)", () => {
  test.each([8640000000000001, 1e16, Number.POSITIVE_INFINITY])(
    "out-of-range created %p: hook resolves, both vars omitted, trace ids still injected",
    async (created) => {
      const e = await envFor(created);
      expect(e.MCPX_SESSION_CREATED).toBeUndefined();
      expect(e.MCPX_SESSION_AGE_MS).toBeUndefined();
      expect(e.MCPX_TRACE_IDS).toBeTruthy();
    },
  );

  test("future-but-valid created: CREATED set, AGE_MS omitted", async () => {
    const created = Date.now() + 3_600_000;
    const e = await envFor(created);
    expect(e.MCPX_SESSION_CREATED).toBe(new Date(created).toISOString());
    expect(e.MCPX_SESSION_AGE_MS).toBeUndefined();
  });

  test("normal past created: CREATED and non-negative AGE_MS, unchanged", async () => {
    const created = Date.now() - 60_000;
    const e = await envFor(created);
    expect(e.MCPX_SESSION_CREATED).toBe(new Date(created).toISOString());
    expect(Number(e.MCPX_SESSION_AGE_MS)).toBeGreaterThanOrEqual(60_000);
  });
});
