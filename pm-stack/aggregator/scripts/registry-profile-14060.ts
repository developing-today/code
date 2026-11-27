// registry-profile-14060.ts — deterministic cold /sessions first-hit profile.
// Strict CLI contract: --records, --cold-runs, and --warm-runs are the whole
// accepted command line; reject unknown, duplicate, missing, or malformed
// arguments before creating files or starting the aggregator source.

import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { ActionsStore } from "../src/actions.ts";
import { identitiesFromRecords } from "../src/registry.ts";
import { parseRegistryQuery, queryRegistry } from "../src/registry-view.ts";
import { renderSessions } from "../src/render.ts";
import { liveSource } from "../src/sources/live.ts";

const USAGE = "usage: registry-profile-14060.ts --records <n> [--cold-runs <n>] [--warm-runs <n>]";
function usageError(message: string): never {
  console.error(`registry-profile-14060: ${message}`);
  console.error(USAGE);
  process.exit(2);
}

function parseArgs(argv: string[]): { records: number; coldRuns: number; warmRuns: number } {
  const values: Record<string, number> = { "--cold-runs": 1, "--warm-runs": 3 };
  const seen = new Set<string>();
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i]!;
    if (flag !== "--records" && flag !== "--cold-runs" && flag !== "--warm-runs") {
      usageError(`unknown argument ${JSON.stringify(flag)}`);
    }
    if (seen.has(flag)) usageError(`duplicate argument ${flag}`);
    seen.add(flag);
    const raw = argv[++i];
    if (raw === undefined || raw.startsWith("--") || !/^\d+$/.test(raw)) {
      usageError(`${flag} needs a non-negative decimal integer`);
    }
    const value = Number(raw);
    if (!Number.isSafeInteger(value) || value < 1) usageError(`${flag} must be a positive safe integer`);
    values[flag] = value;
  }
  if (!seen.has("--records")) usageError("--records is required");
  if (values["--records"]! > 100_000) usageError("--records must be at most 100000");
  if (values["--cold-runs"]! > 10 || values["--warm-runs"]! > 100) {
    usageError("--cold-runs is capped at 10 and --warm-runs at 100");
  }
  return { records: values["--records"]!, coldRuns: values["--cold-runs"]!, warmRuns: values["--warm-runs"]! };
}

const options = parseArgs(process.argv.slice(2));

function syntheticFeed(count: number) {
  return {
    version: 1,
    generatedAt: "2026-11-27T05:00:00.000Z",
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    registry: Array.from({ length: count }, (_, index) => ({
      id: `synthetic-session-${index}`,
      source: "opencode-sandbox",
      label: `Synthetic session ${index}`,
      agentId: "opencode:synthetic",
      agentLabel: "Synthetic",
      parentId: null,
      status: "complete",
      taskRefs: [],
    })),
  };
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}

async function profileRun(dir: string, runs: number): Promise<Record<string, number>[]> {
  await mkdir(dir, { recursive: true });
  const feedPath = join(dir, "fleet-feed.json");
  const fixture = syntheticFeed(options.records);
  const fixtureStarted = performance.now();
  await writeFile(feedPath, JSON.stringify(fixture));
  const fixtureWriteMs = performance.now() - fixtureStarted;
  let currentStages: Record<string, number> | null = null;
  const source = liveSource(
    loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedPath,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_AGG_OPENCODE_SERVER_STATE: join(dir, "no-opencode-state.json"),
      PM_AGG_OPENCHAMBER_STATE: join(dir, "no-openchamber-state.json"),
      PM_PROJECTOR_URL: "off",
    }),
    {
      onRegistryTiming: (stage, elapsedMs) => {
        if (currentStages) currentStages[stage] = elapsedMs;
      },
    },
  );
  const actionsStore = new ActionsStore(join(dir, "actions.json"));
  const out: Record<string, number>[] = [];
  for (let i = 0; i < runs; i++) {
    const stages: Record<string, number> = { ...(i === 0 ? { fixtureWriteMs } : {}) };
    currentStages = stages;
    const started = performance.now();
    const records = await source.registry();
    const queryStarted = performance.now();
    const view = queryRegistry(records, parseRegistryQuery(new URLSearchParams()));
    stages.registryQueryMs = performance.now() - queryStarted;
    const identityStarted = performance.now();
    const identities = identitiesFromRecords(records);
    stages.identityMapMs = performance.now() - identityStarted;
    const actionsStarted = performance.now();
    const actionsByObject = await actionsStore.byObject();
    stages.actionsLoadMs = performance.now() - actionsStarted;
    const renderStarted = performance.now();
    const html = renderSessions(
      view.records,
      source.provenance(),
      source.provenanceDetail?.(),
      { actionsByObject, identities },
      view,
    );
    stages.sessionsRenderMs = performance.now() - renderStarted;
    stages.sessionsFirstHitMs = performance.now() - started;
    stages.recordsReturned = records.length;
    stages.sessionsHtmlBytes = Buffer.byteLength(html);
    out.push(stages);
  }
  return out;
}

const root = await mkdtemp(join(tmpdir(), "registry-profile-14060-"));
try {
  const cold: Record<string, number>[] = [];
  for (let run = 0; run < options.coldRuns; run++) {
    cold.push(...(await profileRun(join(root, `cold-${run}`), 1)));
  }
  const warm = await profileRun(join(root, "warm"), options.warmRuns + 1);
  warm.shift(); // first invocation seeds the in-process feed and registry snapshots
  const stages = [
    "feedStatMs",
    "feedReadMs",
    "feedParseMs",
    "feedSanitizeMs",
    "laptopMapMs",
    "registryMergeMs",
    "registryReadMs",
    "registryQueryMs",
    "identityMapMs",
    "actionsLoadMs",
    "sessionsRenderMs",
    "sessionsFirstHitMs",
  ];
  const summarize = (samples: Record<string, number>[]) =>
    Object.fromEntries(
      stages.map((stage) => [
        stage,
        {
          samples: samples.map((sample) => Number((sample[stage] ?? 0).toFixed(3))),
          medianMs: Number(median(samples.map((sample) => sample[stage] ?? 0)).toFixed(3)),
        },
      ]),
    );
  console.log(
    JSON.stringify(
      {
        profile: "registry-cold-first-hit-14060",
        records: options.records,
        coldRuns: cold.length,
        warmRuns: warm.length,
        outputContainsRecordContent: false,
        cold: summarize(cold),
        warm: summarize(warm),
      },
      null,
      2,
    ),
  );
} finally {
  await rm(root, { recursive: true, force: true });
}
