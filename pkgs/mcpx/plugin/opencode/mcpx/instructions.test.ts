/**
 * The `instructions` option, against opencode v1's real call shape.
 *
 * v1 invokes `experimental.chat.system.transform` with
 * `output = { system: string[] }` (@opencode-ai/plugin's
 * ExperimentalChatSystemTransform). The handler used to write to
 * `output.parts` -- a key that exists only on v2's
 * session.hook("context") SystemPart array -- so the option injected
 * nothing, silently, on every turn (#1938, register PLG-16). These tests
 * invoke the hook exactly the way v1 does and assert the guidance text
 * lands in `system`.
 *
 * `@opencode-ai/plugin` itself is provided by opencode at runtime and is
 * not a dependency of this package, so the test stubs it: the session
 * module only needs `tool()` (an identity over the definition) and the
 * chainable `tool.schema` builders, neither of which the hook under test
 * touches.
 *
 *   bun test plugin/opencode/mcpx/instructions.test.ts
 */

import { afterEach, describe, expect, mock, test } from "bun:test"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

// A schema builder that answers every chain (string().describe(),
// .optional(), ...) with itself.
const anySchema: any = new Proxy(function () {}, {
  get: (_target, prop) => (prop === Symbol.toPrimitive ? () => "" : anySchema),
  apply: () => anySchema,
})
mock.module("@opencode-ai/plugin", () => ({
  tool: Object.assign((def: unknown) => def, { schema: anySchema }),
}))

// Captured before any test points TMPDIR at a scratch directory:
// tmpdir() reads TMPDIR, so asking it later would aim mkdtemp at a
// directory afterEach has already deleted.
const REAL_TMP = tmpdir()
const scratch: string[] = []
const tempRoot = (): string => {
  const dir = mkdtempSync(join(REAL_TMP, "mcpxt-"))
  scratch.push(dir)
  return dir
}

const savedEnv = { ...process.env }
afterEach(() => {
  process.env = { ...savedEnv }
  for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true })
})

/**
 * Build the plugin's hooks with discovery isolated the way daemon.test.ts
 * isolates it: an empty state directory and a temp TMPDIR, so the boot
 * discovery cannot see (or start) anything real.
 */
const hooksFor = async (options: Record<string, unknown>) => {
  const dir = tempRoot()
  process.env.MCPX_STATE_DIR = dir
  process.env.TMPDIR = tempRoot()
  delete process.env.MCPX_PLUGIN_INSTRUCTIONS
  delete process.env.MCPX_DAEMON_ENDPOINT
  delete process.env.MCPX_ENDPOINT
  delete process.env.MCPX_SOCKET
  const client = {
    tui: { showToast: async () => {} },
    session: { get: async () => { throw new Error("no such session") } },
  }
  const plugin = (await import("../mcpx-session.ts")).default as any
  return plugin(
    { directory: dir, worktree: dir, project: {}, client },
    { headless: true, ...options },
  )
}

describe("instructions option (experimental.chat.system.transform)", () => {
  test("pushes the guidance text onto output.system, v1's shape", async () => {
    const hooks = await hooksFor({ instructions: true })
    const output = { system: [] as string[] }
    await hooks["experimental.chat.system.transform"]({}, output)
    expect(output.system).toHaveLength(1)
    expect(output.system[0]).toContain("MCP servers are reached through `mcpx`")
  })

  test("appends after entries v1 already placed in system", async () => {
    const hooks = await hooksFor({ instructions: true })
    const output = { system: ["existing guidance"] }
    await hooks["experimental.chat.system.transform"]({}, output)
    expect(output.system).toHaveLength(2)
    expect(output.system[0]).toBe("existing guidance")
  })

  test("stays off by default: system is left untouched", async () => {
    const hooks = await hooksFor({})
    const output = { system: [] as string[] }
    await hooks["experimental.chat.system.transform"]({}, output)
    expect(output.system).toHaveLength(0)
  })
})
