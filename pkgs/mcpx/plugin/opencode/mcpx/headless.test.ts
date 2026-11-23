/**
 * plugin.headless, observed through the real boot toast (#8472).
 *
 * The headless setting — MCPX_PLUGIN_HEADLESS, or the plugin's
 * headless option — decides whether the session plugin may put a
 * toast on the user's screen. mcpx-session.ts reads it once at load
 * (option, then environment, then a process-shape default: a server
 * running on the main thread has no TUI attached) and the toast
 * helper returns before touching the client when it is set, because
 * `showToast` succeeds whether or not anyone is watching — an
 * unbounded toast in CI is a silent leak, not a visible one.
 *
 * The cases make the boot toast fire on purpose: two fake daemons
 * on real unix sockets both match the directory and neither has a
 * resolve route, so discovery ends at rung 6 — a guess, announced
 * with a warning toast (the daemon.test.ts arrangement). The setting
 * is then the only thing that differs, and the stub client's
 * showToast calls are the observable: with headless set, none; with
 * it cleared, the warning arrives. The Go effect probe for the
 * setting (TestPluginHeadlessSuppressesToastsFromTheEnvironment in
 * internal/e2e) runs this file.
 *
 *   bun test plugin/opencode/mcpx/headless.test.ts
 */
import { afterEach, describe, expect, mock, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { OPERATION_FINGERPRINT } from "../../core/ops.gen.ts";
import type { DaemonInfo } from "../../core/daemon.ts";

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
const stopAll: Array<() => void> = [];
const savedEnv = { ...process.env };
afterEach(() => {
  while (stopAll.length) stopAll.pop()!();
  // Restore in place, on the original process.env object (the
  // env.test.ts pattern): replacing process.env wholesale does not
  // restore the native environment other code reads.
  for (const key of Object.keys(process.env)) {
    if (!(key in savedEnv)) delete process.env[key];
  }
  Object.assign(process.env, savedEnv);
  for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true });
});

const tempRoot = (): string => {
  const dir = mkdtempSync(join(REAL_TMP, "mcpxt-headless-"));
  scratch.push(dir);
  return dir;
};

/** A fake daemon on a real unix socket: health and status, no resolve route. */
const serveFake = (socket: string, config: string, servers: number): void => {
  const server = Bun.serve({
    unix: socket,
    fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/v1/health") {
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
          socket,
          config,
          servers: Array.from({ length: servers }, (_, i) => ({
            name: `s${i}`,
            live: 1,
            instances: [{ key: `ses-${i}` }],
          })),
        });
      }
      return new Response("not found", { status: 404 });
    },
  });
  stopAll.push(() => server.stop(true));
};

const writeInfo = (state: string, key: string, info: DaemonInfo): void => {
  writeFileSync(join(state, `daemon-${key}.json`), JSON.stringify(info, null, 2));
};

type ToastCall = { body?: { title?: string; message?: string; variant?: string } };

/**
 * Load the session plugin over a state directory holding two live
 * daemons that both match the session directory, so boot discovery
 * is ambiguous and the boot warning toast is due. `setEnv` runs
 * before the load (the setting is read from the environment at load
 * time); `options` are the plugin options. Returns every showToast
 * call the stub client received.
 */
const loadWithAmbiguousBoot = async (
  setEnv: () => void,
  options: Record<string, unknown>,
): Promise<ToastCall[]> => {
  const root = tempRoot();
  const state = join(root, "state");
  mkdirSync(state);
  const proj = join(root, "proj");
  const dir = join(proj, "sub");
  mkdirSync(dir, { recursive: true });

  const mine = join(state, "d-mine.sock");
  const other = join(state, "d-other.sock");
  serveFake(mine, join(proj, ".mcpx.json"), 1);
  serveFake(other, join(root, "elsewhere", ".mcpx.json"), 9);
  writeInfo(state, "mine", {
    socket: mine,
    configPath: join(proj, ".mcpx.json"),
    startedAt: new Date(Date.now() - 86_400_000).toISOString(),
  });
  writeInfo(state, "other", {
    socket: other,
    configPath: join(root, "elsewhere", ".mcpx.json"),
    startedAt: new Date().toISOString(),
  });

  // Isolate discovery: this state directory and temp root only, no
  // named endpoint, and no binary for the ladder's CLI rung to ask.
  process.env.MCPX_STATE_DIR = state;
  process.env.TMPDIR = root;
  delete process.env.MCPX_DAEMON_ENDPOINT;
  delete process.env.MCPX_ENDPOINT;
  delete process.env.MCPX_SOCKET;
  process.env.MCPX_PLUGIN_BIN = "";
  setEnv();

  const toasts: ToastCall[] = [];
  const client = {
    session: {
      get: async () => ({ data: { id: "ses_1", directory: dir, time: { created: Date.now() } } }),
    },
    tui: {
      showToast: async (arg: ToastCall) => {
        toasts.push(arg);
      },
    },
  };
  await (plugin as any)({ directory: dir, worktree: proj, project: { id: "p1" }, client }, options);
  // The boot toast is fire-and-forget; let it land before counting.
  await new Promise((resolve) => setTimeout(resolve, 25));
  return toasts;
};

describe("plugin.headless and the boot toast", () => {
  test("MCPX_PLUGIN_HEADLESS=1 suppresses the boot warning toast", async () => {
    const toasts = await loadWithAmbiguousBoot(() => {
      process.env.MCPX_PLUGIN_HEADLESS = "1";
    }, {});
    expect(toasts).toHaveLength(0);
  });

  test("the headless option suppresses the boot warning toast", async () => {
    const toasts = await loadWithAmbiguousBoot(() => {
      delete process.env.MCPX_PLUGIN_HEADLESS;
    }, { headless: true });
    expect(toasts).toHaveLength(0);
  });

  test("headless cleared by option lets the boot warning toast through", async () => {
    const toasts = await loadWithAmbiguousBoot(() => {
      delete process.env.MCPX_PLUGIN_HEADLESS;
    }, { headless: false });
    expect(toasts.length).toBeGreaterThanOrEqual(1);
    expect(toasts[0].body?.message).toContain("2 daemons");
  });

  test("MCPX_PLUGIN_HEADLESS=0 lets the boot warning toast through", async () => {
    const toasts = await loadWithAmbiguousBoot(() => {
      process.env.MCPX_PLUGIN_HEADLESS = "0";
    }, {});
    expect(toasts.length).toBeGreaterThanOrEqual(1);
    expect(toasts[0].body?.message).toContain("2 daemons");
  });

  test("unset everywhere defaults to headless in this process shape", async () => {
    // No option and no environment: the default is the process
    // shape, and a test runner's server thread has no TUI attached,
    // so the default reads headless and the toast stays down. This
    // is the CI case the setting exists for.
    const toasts = await loadWithAmbiguousBoot(() => {
      delete process.env.MCPX_PLUGIN_HEADLESS;
    }, {});
    expect(toasts).toHaveLength(0);
  });
});
