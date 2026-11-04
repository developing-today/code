// Shared contract corpus tests (#1176): the checked-in corpus under
// test/contract-corpus/v1/ carries scrubbed captures of real published
// orchestration, quota, DNS, registry, and projects sections (see the
// corpus manifest.json for capture provenance). These tests run the
// SAME actual aggregator validators the live source and loadFeed use
// over that corpus — no duplicated handwritten checks — so a
// #1110-class publisher/collector shape drift fails here instead of
// shipping silently as missing feed sections.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { todayFromSection } from "../src/derive.ts";
import { isDnsSection, summarizeDns } from "../src/dns.ts";
import {
  diagnoseOrchestrationSection,
  isBurnSegment,
  isFleetFeed,
  isOrchestrationSection,
  isProjectPoolSection,
  isQuotaPool,
  isQuotaWindow,
  loadFeed,
  normalizeOrchestrationSection,
  orchestrationSectionProblem,
  poolFromFeed,
  sanitizeQuotaView,
  sanitizeRegistry,
} from "../src/feed.ts";
import { isProjectPoolItem, resolveEpicRefs } from "../src/projects.ts";
import { isSessionRecord } from "../src/registry.ts";

const CORPUS = join(import.meta.dir, "contract-corpus", "v1");

async function loadJson(name: string): Promise<unknown> {
  return JSON.parse(await readFile(join(CORPUS, name), "utf8"));
}

describe("contract corpus manifest (v1)", () => {
  test("every listed fixture exists and parses", async () => {
    const manifest = (await loadJson("manifest.json")) as {
      contractVersion: number;
      files: Record<string, unknown>;
      producerInputs: string[];
    };
    expect(manifest.contractVersion).toBe(1);
    for (const name of [...Object.keys(manifest.files), "feed.json"]) {
      await expect(readFile(join(CORPUS, name), "utf8")).resolves.toContain("{");
    }
    for (const name of manifest.producerInputs) {
      expect((await readFile(join(CORPUS, name), "utf8")).length).toBeGreaterThan(0);
    }
  });
});

describe("contract corpus: orchestration section (incl. the #1110 object-provenance case)", () => {
  test("the corpus section passes the actual aggregator gate", async () => {
    const section = await loadJson("orchestration.json");
    expect(orchestrationSectionProblem(section)).toBeNull();
    expect(isOrchestrationSection(section)).toBe(true);
    const normalized = normalizeOrchestrationSection(section);
    expect(normalized?.runId).toBe("orch-2026-10-27T03:14:56Z");
    expect(() => todayFromSection(normalized!)).not.toThrow();
  });

  test("the corpus pins the publisher's object provenance form (the #1110 contract)", async () => {
    const section = (await loadJson("orchestration.json")) as { provenance: unknown };
    expect(typeof section.provenance).toBe("object");
    expect((section.provenance as { sources: unknown }).sources).toHaveLength(3);
    expect(diagnoseOrchestrationSection(section, Date.parse("2026-10-27T03:20:00Z")).state).toBe("live");
  });

  test("drift probes: a string-provenance section normalizes, a poisoned one names its check", async () => {
    const live = (await loadJson("orchestration.json")) as Record<string, unknown>;
    const legacy = { ...live, provenance: "orchestration-pass run orch-2026-10-27T03:14:56Z" };
    expect(isOrchestrationSection(legacy)).toBe(true);
    expect(normalizeOrchestrationSection(legacy)?.provenance).toEqual({
      sources: ["orchestration-pass run orch-2026-10-27T03:14:56Z"],
    });
    for (const provenance of [42, {}, { sources: "fleet feed" }, { sources: ["ok", 7] }]) {
      const bad = { ...live, provenance };
      expect(isOrchestrationSection(bad)).toBe(false);
      expect(orchestrationSectionProblem(bad)).toBe("provenance");
      expect(normalizeOrchestrationSection(bad)).toBeNull();
    }
  });
});

describe("contract corpus: DNS section", () => {
  test("the corpus section passes the actual aggregator gate with all three statuses", async () => {
    const section = (await loadJson("dns.json")) as {
      totalAnnualListUsd: number;
      entries: Array<{ priceUsd: number | null; status: string }>;
    };
    expect(isDnsSection(section)).toBe(true);
    const sum = Math.round(section.entries.reduce((acc, e) => acc + (e.priceUsd ?? 0), 0) * 100) / 100;
    expect(section.totalAnnualListUsd).toBe(sum);
    expect(section.entries.map((e) => e.status).sort()).toEqual(["active", "earmarked", "unused"]);
    expect(summarizeDns(section as never, new Date("2026-12-28T00:00:00Z")).nextRenewal?.domain).toBe("alpha.example");
  });
});

describe("contract corpus: projects section", () => {
  test("the corpus pool passes the shared pool-item gate item by item", async () => {
    const pool = (await loadJson("projects.json")) as unknown[];
    expect(isProjectPoolSection(pool)).toBe(true);
    expect(pool).toHaveLength(2);
    for (const item of pool) expect(isProjectPoolItem(item)).toBe(true);
    expect(poolFromFeed({ projects: pool } as never)).toHaveLength(2);
  });

  test("epic linkage resolves through the actual resolver", async () => {
    const pool = (await loadJson("projects.json")) as Array<Record<string, unknown>>;
    const resolved = resolveEpicRefs(pool.filter((p) => p["contentKind"] === "issue") as never[]);
    expect(resolved.find((p) => p.ref === "example-owner/example-repo#2402")?.epicRef).toBe(
      "example-owner/example-repo#2401",
    );
  });
});

describe("contract corpus: registry section", () => {
  test("the corpus records pass the actual session-record gate with zero drops", async () => {
    const records = (await loadJson("registry.json")) as unknown[];
    for (const record of records) expect(isSessionRecord(record)).toBe(true);
    const san = sanitizeRegistry(records);
    expect(san.dropped).toBe(0);
    expect(san.rows).toHaveLength(2);
  });
});

describe("contract corpus: quota section", () => {
  test("the corpus view sanitizes with zero drops under the actual quota gates", async () => {
    const quota = (await loadJson("quota.json")) as {
      pools: unknown[];
      burn: unknown[];
      choices: { hasHeadroom: string[]; burnBeforeReset: string[]; doNotStart: string[] };
    };
    for (const pool of quota.pools) expect(isQuotaPool(pool)).toBe(true);
    for (const pool of quota.pools) {
      for (const window of (pool as { windows: unknown[] }).windows) expect(isQuotaWindow(window)).toBe(true);
    }
    for (const segment of quota.burn) expect(isBurnSegment(segment)).toBe(true);
    const san = sanitizeQuotaView(quota as never);
    for (const count of [
      san.droppedPools,
      san.droppedCredentials,
      san.droppedWindows,
      san.droppedActiveSessions,
      san.droppedBurn,
      san.droppedPoolFreshness,
      san.droppedBurnTokensPerMin,
    ]) {
      expect(count).toBe(0);
    }
    expect(san.view.pools).toHaveLength(2);
  });
});

describe("contract corpus: whole-feed assembly", () => {
  test("feed.json carries the section fixtures verbatim and passes the feed gate", async () => {
    const feed = (await loadJson("feed.json")) as Record<string, unknown>;
    for (const name of ["orchestration", "dns", "registry", "projects"] as const) {
      expect(feed[name]).toEqual(await loadJson(`${name}.json`));
    }
    expect(feed["quota"]).toEqual(await loadJson("quota.json"));
    expect(isFleetFeed(feed)).toBe(true);
  });

  test("the aggregator's actual loadFeed serves the corpus feed with zero drops", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-contract-"));
    const path = join(dir, "fleet-feed.json");
    await writeFile(path, JSON.stringify(await loadJson("feed.json")));
    const loaded = await loadFeed(path);
    expect(loaded).not.toBeNull();
    expect(loaded?.orchestration?.runId).toBe("orch-2026-10-27T03:14:56Z");
    expect(loaded?.dns?.entries).toHaveLength(3);
    expect(loaded?.projects).toHaveLength(2);
    expect(loaded?.registry).toHaveLength(2);
    expect(loaded?.quota.pools).toHaveLength(2);
    expect(loaded?.quotaDropped).toEqual({
      pools: 0,
      credentials: 0,
      windows: 0,
      activeSessions: 0,
      burn: 0,
      poolFreshness: 0,
      burnTokensPerMin: 0,
    });
  });
});
