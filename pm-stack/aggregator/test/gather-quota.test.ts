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
        SUPABASE_QUOTA_SINK: "0",
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
        SUPABASE_QUOTA_SINK: "0",
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
        SUPABASE_QUOTA_SINK: "0",
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
        SUPABASE_QUOTA_SINK: "0",
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
    expect(records.some((record: { serviceId: string }) => record.serviceId === "opencode-go.drewrypope")).toBe(
      false,
    );
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

    // A plain-clone lib checkout: manifest + lockfile declare fake-dep,
    // but node_modules was never installed — the #8759 defect shape.
    await mkdir(join(libDir, "quota/credentials"), { recursive: true });
    await mkdir(join(libDir, "quota/providers"), { recursive: true });
    await writeFile(join(checkoutRoot, "package.json"), JSON.stringify({ dependencies: { "fake-dep": "1.0.0" } }), "utf8");
    await writeFile(join(checkoutRoot, "bun.lock"), "", "utf8");
    await writeFile(join(libDir, "quota/index.js"), "export async function listConfiguredQuotaProviders() { return []; }\n", "utf8");
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
        SUPABASE_QUOTA_SINK: "0",
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
        SUPABASE_QUOTA_SINK: "0",
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
