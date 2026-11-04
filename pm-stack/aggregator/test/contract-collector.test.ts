// Hermetic collector-output contract tests (#1176): the collector
// (scripts/collect-fleet-feed.ts) embeds each publisher output only
// after it passes a structural gate, and the aggregator serves it
// through the same validators. These tests replay the collector's
// exact gate calls — normalizeOrchestrationSection,
// dnsSectionFromRegisterJson, extractQuotaPools + isQuotaPool,
// recordFromSessionEntry / parseLaunchRecords, parseProjectItems via
// the real fetchProjectPool with an injected stub fetch — over
// scrubbed publisher captures in test/contract-corpus/v1/publisher/.
// No live credentials, no network: fetchProjectPool takes its fetch,
// and every other input is a checked-in file.
import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { dnsSectionFromRegisterJson, isDnsSection } from "../src/dns.ts";
import { assembleCollectorFeed } from "../src/feed-contract.ts";
import {
  isFleetFeed,
  isOrchestrationSection,
  isQuotaPool,
  isQuotaWindow,
  normalizeOrchestrationSection,
  orchestrationSectionProblem,
} from "../src/feed.ts";
import type { SessionFeedEntry } from "../src/model.ts";
import { fetchProjectPool, isProjectPoolItem, parseProjectItems, resolveEpicRefs } from "../src/projects.ts";
import { extractQuotaPools } from "../src/quota-extract.ts";
import { computeChoices } from "../src/quota-guidance.ts";
import { isSessionRecord, parseLaunchRecords, recordFromSessionEntry } from "../src/registry.ts";

const CORPUS = join(import.meta.dir, "contract-corpus", "v1");
const PUBLISHER = join(CORPUS, "publisher");

async function publisherText(name: string): Promise<string> {
  return readFile(join(PUBLISHER, name), "utf8");
}

describe("collector orchestration gate (publisher file -> feed section)", () => {
  test("the published section embeds: the collector's normalize call accepts it", async () => {
    const parsed: unknown = JSON.parse(await readFile(join(CORPUS, "orchestration.json"), "utf8"));
    expect(orchestrationSectionProblem(parsed)).toBeNull();
    const section = normalizeOrchestrationSection(parsed);
    expect(section?.runId).toBe("orch-2026-10-27T03:14:56Z");
    expect(isOrchestrationSection(section)).toBe(true);
  });

  test("a drifted publisher file is omitted with the failing check named", async () => {
    const parsed: unknown = JSON.parse(await readFile(join(CORPUS, "orchestration.json"), "utf8"));
    const drifted = { ...(parsed as Record<string, unknown>), provenance: 42 };
    expect(normalizeOrchestrationSection(drifted)).toBeNull();
    expect(orchestrationSectionProblem(drifted)).toBe("provenance");
  });
});

describe("collector DNS gate (register.json -> feed section)", () => {
  test("the published register assembles to the corpus section through the collector's call", async () => {
    const corpusDns = JSON.parse(await readFile(join(CORPUS, "dns.json"), "utf8"));
    const section = dnsSectionFromRegisterJson(
      await publisherText("register.json"),
      "2026-10-27T03:10:00Z",
      "register file (#902 pipeline) via fleet feed collector",
      corpusDns.entries,
    );
    expect(section).not.toBeNull();
    expect(isDnsSection(section)).toBe(true);
    expect(section).toEqual(corpusDns);
  });

  test("an empty register omits the section instead of publishing a shapeless one", () => {
    expect(dnsSectionFromRegisterJson(JSON.stringify({ domains: [] }), "2026-10-27T03:10:00Z", "test")).toBeNull();
  });
});

describe("collector quota gate (quota-view.md -> feed pools)", () => {
  const NOW = new Date("2026-10-25T15:00:00Z");

  test("the published view extracts pools that pass the actual pool/window gates", async () => {
    const pools = extractQuotaPools(await publisherText("quota-view.md"), NOW);
    expect(pools.map((p) => p.id)).toContain("anthropic:claude-pro");
    expect(pools.map((p) => p.id)).toContain("cline-pass");
    for (const pool of pools) expect(isQuotaPool(pool)).toBe(true);
    for (const pool of pools) for (const window of pool.windows) expect(isQuotaWindow(window)).toBe(true);
    expect(pools.find((p) => p.id === "anthropic:claude-pro")?.windows[0]?.percent).toBe(68);
  });
});

describe("collector registry gates (sessions + launch records -> feed records)", () => {
  test("session entries map to records the aggregator gate accepts", async () => {
    const entries = JSON.parse(await publisherText("sessions.json")) as SessionFeedEntry[];
    const records = entries.map((e) => recordFromSessionEntry(e));
    expect(records).toHaveLength(2);
    for (const record of records) expect(isSessionRecord(record)).toBe(true);
    expect(records[0]?.taskRefs).toContain("developing-today/code#1176");
    expect(records[1]?.taskRefs).toContain("developing-today/code#1110");
  });

  test("launch records parse to records the aggregator gate accepts; torn lines drop", async () => {
    const jsonl = `${await publisherText("launches.jsonl")}\nnot json\n`;
    const records = parseLaunchRecords(jsonl);
    expect(records).toHaveLength(2);
    for (const record of records) expect(isSessionRecord(record)).toBe(true);
  });
});

describe("collector projects gate (GraphQL -> feed pool, stubbed fetch)", () => {
  test("the published GraphQL response parses to the corpus pool through the real fetch path", async () => {
    const body = await publisherText("projects-graphql.json");
    const stubFetch = (async () => new Response(body)) as unknown as typeof fetch;
    const pool = await fetchProjectPool("contract-corpus-no-credential", stubFetch);
    for (const item of pool) expect(isProjectPoolItem(item)).toBe(true);
    const corpus = JSON.parse(await readFile(join(CORPUS, "projects.json"), "utf8")) as Array<{ id: string }>;
    expect(pool.map((item) => item.id).sort()).toEqual(
      corpus
        .map((item) => item.id)
        .filter((id) => id !== "PVTI_contract_3")
        .sort(),
    );
    expect(pool).toEqual(resolveEpicRefs(parseProjectItems(JSON.parse(body))));
  });

  test("unlinked drafts never reach the pool", async () => {
    const body = JSON.parse(await publisherText("projects-graphql.json")) as {
      data: { node: { items: { nodes: unknown[] } } };
    };
    body.data.node.items.nodes.push({ id: "PVTI_draft", type: "DRAFT_ISSUE", content: { title: "draft" } });
    const pool = parseProjectItems(body);
    expect(pool.find((item) => item.id === "PVTI_draft")).toBeUndefined();
  });
});

describe("collector output assembly (all five sections, hermetic)", () => {
  test("the real section adapters assemble a feed accepted by the real aggregator gate", async () => {
    const corpusDns = JSON.parse(await readFile(join(CORPUS, "dns.json"), "utf8"));
    const [orchestrationRaw, quotaText, registerJson, sessionJson, projectJson] = await Promise.all([
      readFile(join(CORPUS, "orchestration.json"), "utf8"),
      publisherText("quota-view.md"),
      publisherText("register.json"),
      publisherText("sessions.json"),
      publisherText("projects-graphql.json"),
    ]);
    const orchestration = normalizeOrchestrationSection(JSON.parse(orchestrationRaw));
    const pools = extractQuotaPools(quotaText, new Date("2026-10-25T15:00:00Z"));
    const dns = dnsSectionFromRegisterJson(
      registerJson,
      "2026-10-27T03:10:00Z",
      "register file (#902 pipeline) via fleet feed collector",
      corpusDns.entries,
    );
    const registry = [
      ...(JSON.parse(sessionJson) as SessionFeedEntry[]).map((entry) => recordFromSessionEntry(entry)),
      ...parseLaunchRecords(await publisherText("launches.jsonl")),
    ];
    const stubFetch = (async () => new Response(projectJson)) as unknown as typeof fetch;
    const projects = await fetchProjectPool("contract-corpus-no-credential", stubFetch);
    const feed = assembleCollectorFeed({
      generatedAt: "2026-10-27T03:20:00Z",
      quota: { pools, burn: [], choices: computeChoices(pools, Date.parse("2026-10-27T03:20:00Z")) },
      attention: [],
      ...(orchestration ? { orchestration } : {}),
      ...(dns ? { dns } : {}),
      registry,
      projects,
    });

    expect(isFleetFeed(feed)).toBe(true);
    expect(isOrchestrationSection(feed.orchestration)).toBe(true);
    expect(isDnsSection(feed.dns)).toBe(true);
    expect(feed.registry?.every(isSessionRecord)).toBe(true);
    expect(feed.projects?.every(isProjectPoolItem)).toBe(true);
    expect(feed.quota.pools.every(isQuotaPool)).toBe(true);
    expect(feed.quota.pools.every((pool) => pool.windows.every(isQuotaWindow))).toBe(true);
  });
});
