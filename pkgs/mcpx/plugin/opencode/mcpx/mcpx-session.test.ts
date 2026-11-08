/**
 * The mcpx_exec tool adapter, against a fake daemon and a cancellable CLI.
 *
 * These tests exercise the host ToolContext.abort signal at the adapter
 * boundary so they catch a signal dropped at any point in either route.
 */

import { afterEach, describe, expect, mock, test } from "bun:test"
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

import { OPERATION_FINGERPRINT } from "./ops.gen.ts"

const anySchema: any = new Proxy(() => {}, {
  get: (_target, prop) => (prop === Symbol.toPrimitive ? () => "" : anySchema),
  apply: () => anySchema,
})
mock.module("@opencode-ai/plugin", () => ({
  tool: Object.assign((def: unknown) => def, { schema: anySchema }),
}))

const REAL_TMP = tmpdir()
const scratch: string[] = []
const tempRoot = (): string => {
  const dir = mkdtempSync(join(REAL_TMP, "mcpx-abort-"))
  scratch.push(dir)
  return dir
}

const savedEnv = { ...process.env }
const servers: Array<{ stop: (force?: boolean) => void }> = []
afterEach(() => {
  for (const key of Object.keys(process.env)) {
    if (!(key in savedEnv)) delete process.env[key]
  }
  Object.assign(process.env, savedEnv)
  for (const server of servers.splice(0)) server.stop(true)
  for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true })
})

const hooksFor = async (options: Record<string, unknown>) => {
  const directory = tempRoot()
  process.env.MCPX_STATE_DIR = directory
  process.env.TMPDIR = tempRoot()
  delete process.env.MCPX_DAEMON_ENDPOINT
  delete process.env.MCPX_ENDPOINT
  delete process.env.MCPX_SOCKET
  const client = {
    tui: { showToast: async () => {} },
    session: { get: async () => { throw new Error("no such session") } },
  }
  const plugin = (await import("../mcpx-session.ts")).default as any
  return plugin({ directory, worktree: directory, project: {}, client }, { headless: true, tools: true, ...options })
}

const waitFor = async (predicate: () => boolean, message: string): Promise<void> => {
  const until = Date.now() + 3000
  while (!predicate() && Date.now() < until) await new Promise((resolve) => setTimeout(resolve, 10))
  if (!predicate()) throw new Error(message)
}

describe("mcpx_exec cancellation", () => {
  test("aborting the host context cancels daemon exec and preserves its session", async () => {
    const directory = tempRoot()
    const socket = join(directory, "daemon.sock")
    let execStarted!: (body: any) => void
    const started = new Promise<any>((resolve) => { execStarted = resolve })
    let requestWasAborted = false
    const server = Bun.serve({
      unix: socket,
      async fetch(req) {
        const path = new URL(req.url).pathname
        if (path === "/v1/health") {
          return Response.json({ status: "ok", operationFingerprint: OPERATION_FINGERPRINT })
        }
        if (path === "/v1/exec") {
          execStarted(await req.json())
          req.signal.addEventListener("abort", () => { requestWasAborted = true }, { once: true })
          await new Promise<void>((resolve) => req.signal.addEventListener("abort", () => resolve(), { once: true }))
          return Response.json({ result: "cancelled", exitCode: 0 })
        }
        return new Response("not found", { status: 404 })
      },
    })
    servers.push(server)

    const hooks = await hooksFor({ socket })
    const controller = new AbortController()
    const execution = hooks.tool.mcpx_exec.execute(
      { source: "await new Promise(() => {})" },
      { sessionID: "ses_abort_test", abort: controller.signal },
    )
    const body = await started
    expect(body.options).toEqual({ output: "structured", session: "ses_abort_test" })
    controller.abort()
    await expect(execution).rejects.toBeDefined()
    await waitFor(() => requestWasAborted, "daemon request did not observe abort")
  })

  test("aborting the host context kills the CLI fallback child", async () => {
    const directory = tempRoot()
    const pidFile = join(directory, "child.pid")
    const childScript = [
      'const fs = require("node:fs");',
      'if (process.argv[1] === "--json" && process.argv[2] === "status") { process.stdout.write(JSON.stringify({running:false})); process.exit(0); }',
      `fs.writeFileSync(${JSON.stringify(pidFile)}, String(process.pid));`,
      'process.stdout.write("started");',
      "setInterval(() => {}, 1000);",
    ].join("\n")
    const hooks = await hooksFor({
      backend: "cli",
      bin: process.execPath,
      binArgs: ["-e", childScript, "--"],
    })
    const controller = new AbortController()
    const execution = hooks.tool.mcpx_exec.execute(
      { source: "await new Promise(() => {})" },
      { sessionID: "ses_cli_abort_test", abort: controller.signal },
    )
    await waitFor(() => existsSync(pidFile), "CLI child did not start")
    const pid = Number(readFileSync(pidFile, "utf8"))
    controller.abort()
    await expect(execution).rejects.toBeDefined()
    await waitFor(() => {
      try {
        process.kill(pid, 0)
        return false
      } catch {
        return true
      }
    }, "CLI child survived its abort")
  })
})
