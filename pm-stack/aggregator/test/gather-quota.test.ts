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
