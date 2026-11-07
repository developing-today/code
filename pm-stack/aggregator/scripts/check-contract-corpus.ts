// Pre-publish + corpus check for the publisher -> collector ->
// aggregator contract (#1176). Runs the aggregator's ACTUAL validators
// over the shared corpus (test/contract-corpus/v1/) and, when given a
// publisher file, over that file exactly as the collector gates it —
// the same normalizeOrchestrationSection call scripts/collect-fleet-feed.ts
// makes before embedding the section. The publisher can run this before
// publishing orchestration.json; CI runs it on pm-stack changes.
//
// Usage:
//   bun run contract:check
//   bun run contract:check -- --publisher ~/workspace/.../orchestration.json
//   ORCHESTRATION_PATH=... bun run contract:check
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { isDnsSection } from "../src/dns.ts";
import {
  isBurnSegment,
  isFleetFeed,
  isOrchestrationSection,
  isProjectPoolSection,
  isQuotaPool,
  isQuotaWindow,
  normalizeOrchestrationSection,
  orchestrationSectionProblem,
  sanitizeQuotaView,
  sanitizeRegistry,
} from "../src/feed.ts";
import { isProjectPoolItem } from "../src/projects.ts";
import { isSessionRecord } from "../src/registry.ts";

const SCRIPT_DIR = import.meta.dir;
const DEFAULT_CORPUS = join(SCRIPT_DIR, "..", "test", "contract-corpus", "v1");

function usage(): never {
  console.error("usage: check-contract-corpus.ts [--corpus <dir>] [--publisher <orchestration.json>]");
  process.exit(2);
}

function args(): { corpus: string; publisher: string | null } {
  let corpus = DEFAULT_CORPUS;
  let publisher: string | null = process.env.ORCHESTRATION_PATH ?? null;
  const rest = process.argv.slice(2);
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === "--corpus" && rest[i + 1]) corpus = rest[++i];
    else if (rest[i] === "--publisher" && rest[i + 1]) publisher = rest[++i];
    else usage();
  }
  return { corpus, publisher };
}

const failures: string[] = [];
function check(name: string, ok: boolean, detail?: string): void {
  console.log(`${ok ? "ok" : "FAIL"}  ${name}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures.push(name);
}

async function loadJson(path: string): Promise<unknown> {
  return JSON.parse(await readFile(path, "utf8"));
}

async function main(): Promise<void> {
  const { corpus, publisher } = args();
  const at = (name: string) => join(corpus, name);

  const orchestration = await loadJson(at("orchestration.json"));
  const problem = orchestrationSectionProblem(orchestration);
  check("corpus orchestration passes isOrchestrationSection", problem === null, problem ?? undefined);
  check(
    "corpus orchestration pins the object provenance form (#1110)",
    typeof (orchestration as { provenance: unknown }).provenance === "object" &&
      Array.isArray((orchestration as { provenance: { sources: unknown } }).provenance.sources),
  );

  const dns = await loadJson(at("dns.json"));
  check("corpus dns passes isDnsSection", isDnsSection(dns));

  const projects = (await loadJson(at("projects.json"))) as unknown[];
  check("corpus projects passes isProjectPoolSection", isProjectPoolSection(projects));
  check("corpus projects: every item passes isProjectPoolItem", projects.every(isProjectPoolItem));

  const registry = (await loadJson(at("registry.json"))) as unknown[];
  check("corpus registry: every row passes isSessionRecord", registry.every(isSessionRecord));
  check("corpus registry sanitizes with zero drops", sanitizeRegistry(registry).dropped === 0);

  const quota = (await loadJson(at("quota.json"))) as Parameters<typeof sanitizeQuotaView>[0];
  check("corpus quota: every pool passes isQuotaPool", quota.pools.every(isQuotaPool));
  check(
    "corpus quota: every window passes isQuotaWindow",
    quota.pools.every((p) => p.windows.every(isQuotaWindow)),
  );
  check("corpus quota: every burn segment passes isBurnSegment", quota.burn.every(isBurnSegment));
  const san = sanitizeQuotaView(quota);
  check(
    "corpus quota sanitizes with zero drops",
    san.droppedPools + san.droppedCredentials + san.droppedWindows + san.droppedActiveSessions + san.droppedBurn === 0,
  );

  const feed = (await loadJson(at("feed.json"))) as Record<string, unknown>;
  check("corpus feed.json passes isFleetFeed", isFleetFeed(feed));
  for (const name of ["orchestration", "quota", "dns", "registry", "projects"]) {
    check(
      `corpus feed.json carries ${name}.json verbatim`,
      JSON.stringify(feed[name]) === JSON.stringify(await loadJson(at(`${name}.json`))),
    );
  }

  if (publisher) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(await readFile(publisher, "utf8"));
    } catch (err) {
      check(`publisher file parses (${publisher})`, false, String(err));
      parsed = null;
    }
    if (parsed !== null) {
      const pubProblem = orchestrationSectionProblem(parsed);
      check(`publisher section passes the collector gate (${publisher})`, pubProblem === null, pubProblem ?? undefined);
      check(`publisher section normalizes for the feed (${publisher})`, normalizeOrchestrationSection(parsed) !== null);
    }
  }

  if (failures.length) {
    console.error(`\ncontract check FAILED: ${failures.length} check(s): ${failures.join("; ")}`);
    process.exit(1);
  }
  console.log(`\ncontract check passed${publisher ? ` (including publisher file ${publisher})` : ""}.`);
}

await main();
