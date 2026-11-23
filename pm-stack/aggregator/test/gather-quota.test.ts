import { expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const GATHERER_SCRIPT = fileURLToPath(new URL("../scripts/gather-quota.ts", import.meta.url));

test("an ok-but-null provider result retains the last good quota row", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-null-"));

  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const outboxPath = join(dir, "snapshots.outbox.jsonl");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const libDir = join(dir, "fake-openchamber-lib");
    const measuredAt = "2026-11-06T04:00:00.000Z";
    const providerResult = {
      providerId: "meta",
      providerName: "Meta",
      ok: true,
      configured: true,
      fetchedAt: Date.parse("2026-11-06T04:30:00.000Z"),
      failureClass: "no-data",
      lastSuccessAt: Date.parse("2026-11-06T04:00:00.000Z"),
      usage: { windows: { weekly: { usedPercent: null } } },
    };

    await writeFile(
      currentPath,
      `${JSON.stringify(
        {
          version: 1,
          generatedAt: measuredAt,
          source: "opencode-quota",
          heartbeat: {
            tickAt: measuredAt,
            ok: true,
            durationMs: 1,
            perProvider: { meta: { state: "ok", ms: 1, rows: 1 } },
          },
          providers: [
            {
              providerId: "meta",
              providerName: "Meta",
              serviceId: "meta",
              state: "ok",
              measuredAt,
              source: "opencode-quota",
              windows: [
                {
                  name: "weekly",
                  value: 100,
                  pctOfLimit: 100,
                  valueLabel: null,
                  windowSeconds: 604800,
                  resetsAt: "2026-11-09T04:00:00.000Z",
                  measuredAt,
                  source: "opencode-quota",
                  state: "ok",
                },
              ],
            },
          ],
        },
        null,
        2,
      )}\n`,
      "utf8",
    );

    await writeFile(subcommandPath, `console.log(${JSON.stringify(JSON.stringify([providerResult]))});\n`, "utf8");

    // The pre-#5719 gatherer imports this module directly; the current
    // gatherer uses QUOTA_SUBCOMMAND. Keeping the fixture available at
    // both seams lets the same scenario distinguish the old null-row
    // behavior from current last-known retention.
    await mkdir(join(libDir, "opencode"), { recursive: true });
    await mkdir(join(libDir, "quota"), { recursive: true });
    await writeFile(
      join(libDir, "opencode", "auth.js"),
      `export function configureOpenCodeCredentials() {}\nexport function openCodeCredentialSource() { return { list: async () => [] }; }\nexport function openCodeLocalCredentialSource() { return { list: async () => [] }; }\n`,
      "utf8",
    );
    await writeFile(
      join(libDir, "quota", "index.js"),
      `export async function listConfiguredQuotaProviders() { return ["meta"]; }\nexport async function fetchQuotaForProviderInstances(providerId) { return providerId === "meta" ? ${JSON.stringify([providerResult])} : []; }\n`,
      "utf8",
    );
    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
        OPENCHAMBER_LIB_DIR: libDir,
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error(`gatherer failed: ${result.error?.message ?? result.stderr}\n${result.stdout}`);
    }

    const output = JSON.parse(await readFile(currentPath, "utf8"));
    const retained = output.providers.find((entry: { providerId: string }) => entry.providerId === "_muse");
    // The retained row carries the provider's current id: fleet-built
    // providers were renamed to leading-underscore ids (#1823), so the
    // meta row is providerId "_muse" (serviceId stays "meta").
    expect(retained).toMatchObject({
      providerId: "_muse",
      serviceId: "meta",
      state: "ok",
      measuredAt,
      retainedFromFailedTick: output.generatedAt,
      windows: [
        {
          name: "weekly",
          value: 100,
          pctOfLimit: 100,
          measuredAt,
          state: "ok",
        },
      ],
    });
    expect(output.heartbeat).toMatchObject({
      ok: true,
      perProvider: {
        meta: { state: "error" },
      },
    });
    expect(output.heartbeat.perProvider.meta.error).toContain("no-data");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("a lapsed Meta session retains the last good windows and reports lapsed, not error (#8294)", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-meta-lapsed-"));

  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const outboxPath = join(dir, "snapshots.outbox.jsonl");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const previousAt = "2026-11-06T04:00:00.000Z";
    const lapsedResult = {
      providerId: "meta",
      providerName: "Meta",
      ok: false,
      configured: true,
      fetchedAt: Date.parse("2026-11-06T04:30:00.000Z"),
      failureClass: "unauthorized",
      error: "unauthorized",
      usage: null,
    };

    await writeFile(
      currentPath,
      `${JSON.stringify(
        {
          version: 1,
          generatedAt: previousAt,
          source: "opencode-quota",
          heartbeat: {
            tickAt: previousAt,
            ok: true,
            durationMs: 1,
            perProvider: { meta: { state: "ok", ms: 1, rows: 1 } },
          },
          providers: [
            {
              providerId: "meta",
              providerName: "Meta",
              serviceId: "meta",
              state: "ok",
              measuredAt: previousAt,
              source: "opencode-quota",
              windows: [
                {
                  name: "weekly",
                  value: 42,
                  pctOfLimit: 42,
                  valueLabel: null,
                  windowSeconds: 604800,
                  resetsAt: "2026-11-13T04:00:00.000Z",
                  measuredAt: previousAt,
                  source: "opencode-quota",
                  state: "ok",
                },
              ],
            },
          ],
        },
        null,
        2,
      )}\n`,
      "utf8",
    );
    await writeFile(subcommandPath, `console.log(${JSON.stringify(JSON.stringify([lapsedResult]))});\n`, "utf8");

    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error(`gatherer failed: ${result.error?.message ?? result.stderr}\n${result.stdout}`);
    }

    const output = JSON.parse(await readFile(currentPath, "utf8"));
    // The run classifies as lapsed — distinctly from a provider error.
    expect(output.heartbeat.perProvider.meta.state).toBe("lapsed");
    expect(output.heartbeat.perProvider.meta.error).toContain("re-auth");
    const metaRows = output.providers.filter((entry: { serviceId: string }) => entry.serviceId === "meta");
    // The lapsed row itself is present and names the remedy.
    const lapsedRow = metaRows.find((entry: { state: string }) => entry.state === "lapsed");
    expect(lapsedRow).toBeDefined();
    expect(lapsedRow.error).toContain("re-auth");
    // The last good windows stay visible as a retained stale row
    // instead of the pool dropping to a bare error.
    const retainedRow = metaRows.find((entry: { retainedFromFailedTick?: string }) => entry.retainedFromFailedTick);
    expect(retainedRow).toBeDefined();
    expect(retainedRow.windows.map((w: { name: string; pctOfLimit: number }) => [w.name, w.pctOfLimit])).toEqual([
      ["weekly", 42],
    ]);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("a lapsed Go workspace survives ingest instead of being replaced by last-known retention", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-lapsed-"));

  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const outboxPath = join(dir, "snapshots.outbox.jsonl");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const previousAt = "2026-11-06T04:00:00.000Z";
    const lapsedResult = {
      providerId: "opencode-go",
      providerName: "OpenCode Go",
      ok: false,
      configured: true,
      fetchedAt: Date.parse("2026-11-06T04:30:00.000Z"),
      credentialId: "cred_personal",
      credentialLabel: "drewrypope",
      error: "No active OpenCode Go subscription on the selected Console account",
      usage: null,
    };

    // A real ok read from the previous tick would be the retention
    // candidate if the lapsed run were misclassified as an error.
    await writeFile(
      currentPath,
      `${JSON.stringify(
        {
          version: 1,
          generatedAt: previousAt,
          source: "opencode-quota",
          heartbeat: {
            tickAt: previousAt,
            ok: true,
            durationMs: 1,
            perProvider: { "opencode-go": { state: "ok", ms: 1, rows: 2 } },
          },
          providers: [
            {
              providerId: "opencode-go",
              providerName: "OpenCode Go",
              serviceId: "opencode-go",
              state: "ok",
              measuredAt: previousAt,
              source: "opencode-quota",
              windows: [
                {
                  name: "monthly",
                  value: 62,
                  pctOfLimit: 62,
                  valueLabel: null,
                  windowSeconds: null,
                  resetsAt: "2026-12-03T03:02:00.000Z",
                  measuredAt: previousAt,
                  source: "opencode-quota",
                  state: "ok",
                },
              ],
            },
          ],
        },
        null,
        2,
      )}\n`,
      "utf8",
    );
    await writeFile(subcommandPath, `console.log(${JSON.stringify(JSON.stringify([lapsedResult]))});\n`, "utf8");

    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error(`gatherer failed: ${result.error?.message ?? result.stderr}\n${result.stdout}`);
    }

    const output = JSON.parse(await readFile(currentPath, "utf8"));
    const goRows = output.providers.filter((entry: { serviceId: string }) => entry.serviceId.startsWith("opencode-go"));
    expect(goRows.map((entry: { serviceId: string }) => entry.serviceId).sort()).toEqual([
      "opencode-go",
      "opencode-go.drewrypope",
    ]);
    for (const row of goRows) {
      expect(row.state).toBe("lapsed");
      expect(row.windows).toEqual([]);
      expect(row.retainedFromFailedTick).toBeUndefined();
      expect(row.error).toContain("No active OpenCode Go subscription");
    }
    expect(output.heartbeat.perProvider["opencode-go"]).toMatchObject({ state: "lapsed" });
    // No percentage may be recorded as a fresh observation.
    await expect(readFile(outboxPath, "utf8")).rejects.toThrow();
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("a lapsed Go workspace survives retention when a sibling workspace fails", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-mixed-lapse-"));

  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const outboxPath = join(dir, "snapshots.outbox.jsonl");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const previousAt = "2026-11-06T04:00:00.000Z";
    const windowFor = (credentialLabel: string, credentialId: string) => ({
      providerId: "opencode-go",
      providerName: "OpenCode Go",
      serviceId: "opencode-go",
      credentialId,
      credentialLabel,
      state: "ok",
      measuredAt: previousAt,
      source: "opencode-quota",
      windows: [
        {
          name: "monthly",
          value: credentialLabel === "drewrypope" ? 62 : 35,
          pctOfLimit: credentialLabel === "drewrypope" ? 62 : 35,
          valueLabel: null,
          windowSeconds: null,
          resetsAt: "2026-12-03T03:02:00.000Z",
          measuredAt: previousAt,
          source: "opencode-quota",
          state: "ok",
        },
      ],
    });
    const lapsed = {
      providerId: "opencode-go",
      providerName: "OpenCode Go",
      ok: false,
      configured: true,
      fetchedAt: Date.parse("2026-11-06T04:30:00.000Z"),
      credentialId: "cred_personal",
      credentialLabel: "drewrypope",
      error: "No active OpenCode Go subscription on the selected Console account",
      usage: null,
    };
    const failedSibling = {
      providerId: "opencode-go",
      providerName: "OpenCode Go",
      ok: false,
      configured: true,
      fetchedAt: Date.parse("2026-11-06T04:30:01.000Z"),
      credentialId: "cred_lab",
      credentialLabel: "popesdrew",
      error: "OpenCode Go request timed out",
      usage: null,
    };

    await writeFile(
      currentPath,
      `${JSON.stringify(
        {
          version: 1,
          generatedAt: previousAt,
          source: "opencode-quota",
          heartbeat: {
            tickAt: previousAt,
            ok: true,
            durationMs: 1,
            perProvider: { "opencode-go": { state: "ok", ms: 1, rows: 2 } },
          },
          providers: [windowFor("drewrypope", "cred_personal"), windowFor("popesdrew", "cred_lab")],
        },
        null,
        2,
      )}\n`,
      "utf8",
    );
    await writeFile(
      subcommandPath,
      `console.log(${JSON.stringify(JSON.stringify([lapsed, failedSibling]))});\n`,
      "utf8",
    );

    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error(`gatherer failed: ${result.error?.message ?? result.stderr}\n${result.stdout}`);
    }

    const output = JSON.parse(await readFile(currentPath, "utf8"));
    const personal = output.providers.filter(
      (entry: { serviceId: string }) => entry.serviceId === "opencode-go.drewrypope",
    );
    expect(personal).toHaveLength(1);
    expect(personal[0]).toMatchObject({ state: "lapsed", windows: [] });
    expect(personal[0].error).toContain("No active OpenCode Go subscription");

    const sibling = output.providers.find(
      (entry: { serviceId: string; retainedFromFailedTick?: string }) =>
        entry.serviceId === "opencode-go.popesdrew" && entry.retainedFromFailedTick,
    );
    expect(sibling).toMatchObject({ state: "ok", retainedFromFailedTick: output.generatedAt });
    expect(output.heartbeat.perProvider["opencode-go"]).toMatchObject({
      state: "error",
      error: "OpenCode Go request timed out",
    });
    // The lapsed credential contributes no newly measured percentage.
    const records = (await readFile(outboxPath, "utf8"))
      .trim()
      .split("\n")
      .map((line: string) => JSON.parse(line));
    expect(records.some((record: { serviceId: string }) => record.serviceId === "opencode-go.drewrypope")).toBe(false);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("an uninstalled lib checkout dependency is installed and the OpenDesign keeper runs (#8759)", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-libdeps-"));
  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const outboxPath = join(dir, "snapshots.outbox.jsonl");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const installerPath = join(dir, "install-fake-dep.mjs");
    const checkoutRoot = join(dir, "checkout");
    const libDir = join(checkoutRoot, "packages/web/server/lib");
    const renewalStatePath = join(dir, "opendesign-renewal.json");
    const providerResult = {
      providerId: "meta",
      providerName: "Meta",
      ok: true,
      configured: true,
      fetchedAt: Date.parse("2026-11-06T04:30:00.000Z"),
      failureClass: "no-data",
      lastSuccessAt: Date.parse("2026-11-06T04:00:00.000Z"),
      usage: { windows: { weekly: { usedPercent: null } } },
    };

    // A clean main lib checkout (#5721 gate): manifest + lockfile declare
    // fake-dep, but node_modules was never installed — the #8759 defect shape.
    await mkdir(join(libDir, "quota/credentials"), { recursive: true });
    await mkdir(join(libDir, "quota/providers"), { recursive: true });
    await writeFile(
      join(checkoutRoot, "package.json"),
      JSON.stringify({ dependencies: { "fake-dep": "1.0.0" } }),
      "utf8",
    );
    await writeFile(join(checkoutRoot, "bun.lock"), "", "utf8");
    await writeFile(
      join(libDir, "quota/index.js"),
      "export async function listConfiguredQuotaProviders() { return []; }\n",
      "utf8",
    );
    await writeFile(
      join(libDir, "quota/credentials/providers.js"),
      "import 'fake-dep';\nexport function readManagedCredential() { return null; }\nexport function writeManagedCredential() {}\n",
      "utf8",
    );
    await writeFile(
      join(libDir, "quota/providers/opendesign.js"),
      "export async function fetchOpenDesignQuota() { return {}; }\n",
      "utf8",
    );
    await writeFile(join(checkoutRoot, ".gitignore"), "node_modules/\n", "utf8");
    const git = (...args: string[]) => {
      const r = spawnSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", ...args], {
        cwd: checkoutRoot,
        encoding: "utf8",
      });
      if (r.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${r.stderr}`);
    };
    git("init", "-q", "-b", "main");
    git("add", "-A");
    git("commit", "-q", "-m", "lib checkout");
    // The install step (OPENCHAMBER_LIB_INSTALL_CMD seam; production
    // default is bun install at the checkout root): materialize the
    // declared package under the cwd the gatherer gives it — the root.
    const installerSource = [
      "import { mkdir, writeFile } from 'node:fs/promises';",
      "import { join } from 'node:path';",
      "const pkgDir = join(process.cwd(), 'node_modules/fake-dep');",
      "await mkdir(pkgDir, { recursive: true });",
      "await writeFile(join(pkgDir, 'package.json'), JSON.stringify({ name: 'fake-dep', version: '1.0.0', main: 'index.js', type: 'module' }));",
      "await writeFile(join(pkgDir, 'index.js'), 'export {};');",
      "",
    ].join("\n");
    await writeFile(installerPath, installerSource, "utf8");
    await writeFile(subcommandPath, "console.log(" + JSON.stringify(JSON.stringify([providerResult])) + ");\n", "utf8");

    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
        QUOTA_SUBCOMMAND: process.execPath + " " + subcommandPath,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
        OPENCHAMBER_LIB_DIR: libDir,
        OPENCHAMBER_LIB_INSTALL_CMD: process.execPath + " " + installerPath,
        OPENDESIGN_RENEWAL_STATE_PATH: renewalStatePath,
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error("gatherer failed: " + (result.error?.message ?? result.stderr) + "\n" + result.stdout);
    }
    // Oracle: the missing dependency was recovered explicitly, the
    // keeper ran to a real outcome (no stored credential in the
    // fixture), and no failed-soft line was emitted.
    expect(result.stdout).toContain("installing declared dependencies");
    expect(result.stdout).toContain("opendesign-renewal: status=not-configured");
    expect(result.stdout).not.toContain("failed-soft");
    const renewalState = JSON.parse(await readFile(renewalStatePath, "utf8"));
    expect(renewalState.status).toBe("not-configured");
    const installed = JSON.parse(await readFile(join(checkoutRoot, "node_modules/fake-dep/package.json"), "utf8"));
    expect(installed.name).toBe("fake-dep");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("a folded OpenAI answer under the pre-rename 'codex' spelling keys its heartbeat leg '_openai'", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-openai-hb-"));

  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const outboxPath = join(dir, "snapshots.outbox.jsonl");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const win = (usedPercent: number, windowSeconds: number) => ({
      usedPercent,
      remainingPercent: 100 - usedPercent,
      windowSeconds,
      resetAt: 1794364348000,
    });
    const fetchedAt = Date.parse("2026-11-12T06:30:00.000Z");
    // The deployed fork's shape today: the folded door answers both
    // OpenAI credentials under the pre-rename providerId "codex", and
    // the same two credentials again under their own door ids.
    const results = [
      {
        providerId: "codex",
        providerName: "OpenAI Alternate (Student - popesdrew)",
        ok: true,
        configured: true,
        fetchedAt,
        credentialId: "cred_alt",
        credentialLabel: "OpenAI Alternate (Student - popesdrew)",
        usage: { windows: { "5h": win(42, 18_000), weekly: win(65, 604_800) } },
      },
      {
        providerId: "codex",
        providerName: "OpenAI Main (drewrypope)",
        ok: true,
        configured: true,
        fetchedAt,
        credentialId: "cred_main",
        credentialLabel: "OpenAI Main (drewrypope)",
        usage: { windows: { weekly: win(17, 604_800) } },
      },
      {
        providerId: "_openai.alt",
        providerName: "OpenAI Self-Contained Alt (popesdrew, no Codex)",
        ok: true,
        configured: true,
        fetchedAt,
        credentialId: "cred_alt",
        credentialLabel: "OpenAI Alternate (Student - popesdrew)",
        usage: { windows: { "5h": win(42, 18_000), weekly: win(65, 604_800) } },
      },
      {
        providerId: "_openai.main",
        providerName: "OpenAI Self-Contained (drewrypope, no Codex)",
        ok: true,
        configured: true,
        fetchedAt,
        credentialId: "cred_main",
        credentialLabel: "OpenAI Main (drewrypope)",
        usage: { windows: { weekly: win(17, 604_800) } },
      },
    ];
    await writeFile(subcommandPath, `console.log(${JSON.stringify(JSON.stringify(results))});\n`, "utf8");

    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
        OPENCHAMBER_LIB_DIR: join(dir, "fake-openchamber-lib"),
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error(`gatherer failed: ${result.error?.message ?? result.stderr}\n${result.stdout}`);
    }

    const output = JSON.parse(await readFile(currentPath, "utf8"));
    // The folded pool's heartbeat leg reads under the canonical door
    // id — the id its entries carry and consumers look up — and the
    // pre-rename spelling is gone from the heartbeat. The per-door
    // legs keep their own keys: the rename touches nothing else.
    expect(output.heartbeat.perProvider._openai).toMatchObject({ state: "ok", rows: 2 });
    expect(output.heartbeat.perProvider.codex).toBeUndefined();
    expect(output.heartbeat.perProvider["_openai.main"]).toMatchObject({ state: "ok", rows: 1 });
    expect(output.heartbeat.perProvider["_openai.alt"]).toMatchObject({ state: "ok", rows: 1 });
    // The folded entries themselves file under the two doors, as before.
    const folded = output.providers.filter((e: { providerId: string }) => e.providerId === "_openai");
    expect(folded.map((e: { serviceId: string }) => e.serviceId).sort()).toEqual(["_openai.alt", "_openai.main"]);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("a Claude 429 that clears within the tick lands a fresh row, not a retained one (#8725)", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-claude-retry-"));

  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const outboxPath = join(dir, "snapshots.outbox.jsonl");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const counterPath = join(dir, "stub-counter.txt");

    // The stub 429s the Claude leg on the listing and the explicit
    // ask (invocations 0 and 1); the gatherer's within-tick retry
    // (invocation 2) gets a healthy read.
    const stub = [
      'import { readFileSync, writeFileSync } from "node:fs";',
      "let n = 0;",
      'try { n = parseInt(readFileSync(process.env.STUB_COUNTER_PATH, "utf8"), 10) || 0; } catch {}',
      "writeFileSync(process.env.STUB_COUNTER_PATH, String(n + 1));",
      "const now = Date.now();",
      'const ok = { providerId: "_claude", providerName: "Claude", ok: true, configured: true, fetchedAt: now, usage: { windows: { "5h": { usedPercent: 42, windowSeconds: 18000, resetAt: now + 3600000 } } } };',
      'const limited = { providerId: "_claude", providerName: "Claude", ok: false, configured: true, fetchedAt: now, error: "Rate limited. Retrying soon." };',
      'const result = process.env.STUB_MODE === "always429" ? limited : n >= 2 ? ok : limited;',
      "console.log(JSON.stringify([result]));",
      "",
    ].join("\n");
    await writeFile(subcommandPath, stub, "utf8");

    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        QUOTA_CLAUDE_RETRY_BASE_MS: "1",
        STUB_COUNTER_PATH: counterPath,
        STUB_MODE: "clear429",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
        OPENCHAMBER_LIB_DIR: join(dir, "fake-openchamber-lib"),
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error(`gatherer failed: ${result.error?.message ?? result.stderr}\n${result.stdout}`);
    }
    expect(result.stdout).toContain("_claude rate-limited - retry 1/3");
    expect(result.stdout).toContain("recovered=true");

    const output = JSON.parse(await readFile(currentPath, "utf8"));
    const claude = output.providers.find((entry: { providerId: string }) => entry.providerId === "_claude");
    expect(claude).toMatchObject({ providerId: "_claude", state: "ok" });
    expect(claude.retainedFromFailedTick).toBeUndefined();
    expect(claude.windows[0].pctOfLimit).toBe(42);
    expect(output.heartbeat.perProvider._claude).toMatchObject({ state: "ok" });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("a persistent Claude 429 retains the last good row marked rate_limited, never session-expired (#8725)", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-claude-429-retained-"));

  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const outboxPath = join(dir, "snapshots.outbox.jsonl");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const counterPath = join(dir, "stub-counter.txt");
    const measuredAt = "2026-11-17T10:00:00.000Z";
    const lastGood = {
      providerId: "_claude",
      providerName: "Claude",
      serviceId: "anthropic:claude-pro",
      state: "ok",
      measuredAt,
      source: "opencode-quota",
      windows: [
        {
          name: "5h",
          value: 42,
          pctOfLimit: 42,
          valueLabel: null,
          windowSeconds: 18000,
          resetsAt: "2026-11-17T15:00:00.000Z",
          measuredAt,
          source: "opencode-quota",
          state: "ok",
        },
      ],
    };

    await writeFile(
      currentPath,
      `${JSON.stringify(
        {
          version: 1,
          generatedAt: measuredAt,
          source: "opencode-quota",
          heartbeat: {
            tickAt: measuredAt,
            ok: true,
            durationMs: 1,
            perProvider: { _claude: { state: "ok", ms: 1, rows: 1 } },
          },
          providers: [lastGood],
        },
        null,
        2,
      )}\n`,
      "utf8",
    );
    // Cross-tick memory (#5928) holds the same last-good entry, no
    // cooldown in force: the persistent 429 engages retention.
    await writeFile(
      statePath,
      `${JSON.stringify({ "claudeCrossTick:v1": JSON.stringify({ lastGoodEntry: lastGood, cooldownUntil: 0 }) })}\n`,
      "utf8",
    );

    const stub = [
      'import { readFileSync, writeFileSync } from "node:fs";',
      "let n = 0;",
      'try { n = parseInt(readFileSync(process.env.STUB_COUNTER_PATH, "utf8"), 10) || 0; } catch {}',
      "writeFileSync(process.env.STUB_COUNTER_PATH, String(n + 1));",
      "const now = Date.now();",
      'const limited = { providerId: "_claude", providerName: "Claude", ok: false, configured: true, fetchedAt: now, error: "Rate limited. Retrying soon." };',
      "console.log(JSON.stringify([limited]));",
      "",
    ].join("\n");
    await writeFile(subcommandPath, stub, "utf8");

    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        QUOTA_CLAUDE_RETRY_BASE_MS: "1",
        STUB_COUNTER_PATH: counterPath,
        STUB_MODE: "always429",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
        OPENCHAMBER_LIB_DIR: join(dir, "fake-openchamber-lib"),
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error(`gatherer failed: ${result.error?.message ?? result.stderr}\n${result.stdout}`);
    }
    expect(result.stdout).toContain("recovered=false");

    const output = JSON.parse(await readFile(currentPath, "utf8"));
    const claude = output.providers.find((entry: { providerId: string }) => entry.providerId === "_claude");
    expect(claude).toMatchObject({
      providerId: "_claude",
      state: "ok",
      measuredAt,
      retainedFromFailedTick: output.generatedAt,
      retentionFailureKind: "rate_limited",
    });
    expect(output.heartbeat.perProvider._claude.errorKind).toBe("rate_limited");
    expect(output.heartbeat.perProvider._claude.error).toContain("Rate limited");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("the tick publishes the Stage-3 forecast product beside quota-current.json (#12695)", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gather-quota-forecast-"));

  try {
    const currentPath = join(dir, "quota-current.json");
    const heartbeatPath = join(dir, "quota-gatherer-heartbeat.json");
    const statePath = join(dir, "quota-gatherer-state.json");
    const snapshotsDir = join(dir, "snapshots");
    const subcommandPath = join(dir, "quota-subcommand.mjs");
    const now = Date.now();
    const HOUR = 3_600_000;
    const resetMs = now + 10 * HOUR;
    const resetIso = new Date(resetMs).toISOString();

    // Seed the snapshot store with a clean 10 pts/h burn for
    // cline-pass/weekly in the same reset period: 40/50/60 at
    // 3/2/1 h ago. The tick's own read (70) completes the series.
    await mkdir(snapshotsDir, { recursive: true });
    const month = new Date(now).toISOString().slice(0, 7);
    const seed = [3, 2, 1]
      .map((h) =>
        JSON.stringify({
          ts: new Date(now - h * HOUR).toISOString(),
          serviceId: "cline-pass",
          metricId: "weekly",
          value: 70 - h * 10,
          unit: "pct",
          limit: 100,
          resetsAt: resetIso,
          source: "api",
          observedBy: "quota-gatherer",
        }),
      )
      .join("\n");
    await writeFile(join(snapshotsDir, `usage-${month}.jsonl`), `${seed}\n`, "utf8");

    const providerResult = {
      providerId: "cline-pass",
      providerName: "ClinePass",
      ok: true,
      configured: true,
      fetchedAt: now,
      usage: { windows: { weekly: { usedPercent: 70, windowSeconds: 604_800, resetAt: resetMs } } },
    };
    await writeFile(subcommandPath, `console.log(${JSON.stringify(JSON.stringify([providerResult]))});\n`, "utf8");

    const result = spawnSync(process.execPath, [GATHERER_SCRIPT], {
      cwd: dir,
      encoding: "utf8",
      timeout: 30_000,
      env: {
        PATH: process.env.PATH ?? "",
        HOME: dir,
        QUOTA_CURRENT_PATH: currentPath,
        QUOTA_HEARTBEAT_PATH: heartbeatPath,
        QUOTA_GATHERER_STATE_PATH: statePath,
        QUOTA_SNAPSHOTS_DIR: snapshotsDir,
        QUOTA_SUBCOMMAND: `${process.execPath} ${subcommandPath}`,
        QUOTA_SUBCOMMAND_TIMEOUT_MS: "5000",
        OPENDESIGN_RENEWAL_DISABLED: "1",
        META_RENEWAL_DISABLED: "1",
        TURSO_QUOTA_SINK: "0",
      },
    });

    if (result.error || result.status !== 0) {
      throw new Error(`gatherer failed: ${result.error?.message ?? result.stderr}\n${result.stdout}`);
    }
    expect(result.stdout).toContain("forecast wrote");

    const forecast = JSON.parse(await readFile(join(dir, "quota-forecast.json"), "utf8"));
    expect(forecast.version).toBe(1);
    expect(forecast.source).toBe("gather-quota");
    const pool = forecast.pools["cline-pass"]?.find((p: { serviceId: string }) => p.serviceId === "cline-pass");
    expect(pool).toBeDefined();
    const weekly = pool.windows.find((w: { name: string }) => w.name === "weekly");
    // The oracle's verdict on this series: 10 pts/h from 70% walls
    // in ~3 h, before the reset 10 h out.
    expect(weekly.state).toBe("projected");
    expect(weekly.samples).toBeGreaterThanOrEqual(3);
    expect(weekly.ratePtsPerHour).toBeGreaterThan(5);
    expect(weekly.pctNow).toBe(70);
    expect(weekly.resetsAt).toBe(resetIso);
    expect(weekly.projectedWallAt).toBeGreaterThan(now - 60_000);
    expect(weekly.projectedWallAt).toBeLessThan(resetMs);
    expect(weekly.wallsBeforeReset).toBe(true);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
