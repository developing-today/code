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
  // A value that is missing, empty, or flag-like is a usage error (#6930):
  // `--corpus --publisher` used to take "--publisher" as the corpus dir and
  // die with a raw ENOENT stack, and `--publisher --corpus` blamed the
  // publisher file for the operator's missing value, instead of giving the
  // usage error the sibling aggregator CLIs give for the same mistake.
  const valueAfter = (i: number): string => {
    const value: string | undefined = rest[i + 1];
    if (value === undefined || value === "" || value.startsWith("--")) usage();
    return value;
  };
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === "--corpus") {
      corpus = valueAfter(i);
      i++;
    } else if (rest[i] === "--publisher") {
      publisher = valueAfter(i);
      i++;
    } else usage();
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

  // A corpus file that is missing, non-JSON, or the wrong shape is a
  // failed check naming the file (#7048), never a crash: this gate exists
  // to report every divergence in one run, and a raw TypeError/SyntaxError
  // aborts the remaining sections and hides their results. Load each file
  // defensively up front (the publisher section below already works this
  // way); a file that fails to load records its FAIL here and its section
  // checks are skipped.
  const loaded = new Map<string, unknown>();
  const loadFailed = new Set<string>();
  for (const name of ["orchestration.json", "dns.json", "projects.json", "registry.json", "quota.json", "feed.json"]) {
    try {
      loaded.set(name, await loadJson(at(name)));
    } catch (err) {
      loadFailed.add(name);
      check(`corpus ${name} parses as JSON`, false, String(err));
    }
  }
  const isPlainObject = (v: unknown): v is Record<string, unknown> =>
    typeof v === "object" && v !== null && !Array.isArray(v);

  if (!loadFailed.has("orchestration.json")) {
    const orchestration = loaded.get("orchestration.json");
    const problem = orchestrationSectionProblem(orchestration);
    check("corpus orchestration passes isOrchestrationSection", problem === null, problem ?? undefined);
    const provenance = isPlainObject(orchestration) ? orchestration.provenance : undefined;
    check(
      "corpus orchestration pins the object provenance form (#1110)",
      isPlainObject(provenance) && Array.isArray(provenance.sources),
    );
  }

  if (!loadFailed.has("dns.json")) {
    const dns = loaded.get("dns.json");
    check("corpus dns passes isDnsSection", isDnsSection(dns));
  }

  if (!loadFailed.has("projects.json")) {
    const projects = loaded.get("projects.json");
    check("corpus projects passes isProjectPoolSection", isProjectPoolSection(projects));
    check(
      "corpus projects: every item passes isProjectPoolItem",
      Array.isArray(projects) && projects.every(isProjectPoolItem),
      Array.isArray(projects) ? undefined : "projects.json is not an array",
    );
  }

  if (!loadFailed.has("registry.json")) {
    const registry = loaded.get("registry.json");
    check(
      "corpus registry: every row passes isSessionRecord",
      Array.isArray(registry) && registry.every(isSessionRecord),
      Array.isArray(registry) ? undefined : "registry.json is not an array",
    );
    check("corpus registry sanitizes with zero drops", sanitizeRegistry(registry).dropped === 0);
  }

  if (!loadFailed.has("quota.json")) {
    const quota = loaded.get("quota.json");
    const quotaObj = isPlainObject(quota) ? quota : null;
    const pools = quotaObj && Array.isArray(quotaObj.pools) ? (quotaObj.pools as unknown[]) : null;
    const burn = quotaObj && Array.isArray(quotaObj.burn) ? (quotaObj.burn as unknown[]) : null;
    check(
      "corpus quota: every pool passes isQuotaPool",
      pools !== null && pools.every(isQuotaPool),
      pools === null ? "quota.json has no pools array" : undefined,
    );
    check(
      "corpus quota: every window passes isQuotaWindow",
      pools !== null &&
        pools.every(
          (p) => isPlainObject(p) && Array.isArray(p.windows) && (p.windows as unknown[]).every(isQuotaWindow),
        ),
      pools === null ? "quota.json has no pools array" : undefined,
    );
    check(
      "corpus quota: every burn segment passes isBurnSegment",
      burn !== null && burn.every(isBurnSegment),
      burn === null ? "quota.json has no burn array" : undefined,
    );
    // sanitizeQuotaView is not total: a quota object missing the fields it
    // dereferences (e.g. choices, feed.ts) throws instead of dropping. A
    // throw here is one more shape failure to report, not a crash (#7048).
    let san: ReturnType<typeof sanitizeQuotaView> | null = null;
    let sanError: string | undefined;
    if (quotaObj) {
      try {
        san = sanitizeQuotaView(quota as Parameters<typeof sanitizeQuotaView>[0]);
      } catch (err) {
        sanError = String(err);
      }
    }
    check(
      "corpus quota sanitizes with zero drops",
      san !== null &&
        san.droppedPools + san.droppedCredentials + san.droppedWindows + san.droppedActiveSessions + san.droppedBurn ===
          0,
      sanError ?? (san === null ? "quota.json is not an object" : undefined),
    );
  }

  if (!loadFailed.has("feed.json")) {
    const feed = loaded.get("feed.json");
    check("corpus feed.json passes isFleetFeed", isFleetFeed(feed));
    for (const name of ["orchestration", "quota", "dns", "registry", "projects"]) {
      check(
        `corpus feed.json carries ${name}.json verbatim`,
        !loadFailed.has(`${name}.json`) &&
          isPlainObject(feed) &&
          JSON.stringify(feed[name]) === JSON.stringify(loaded.get(`${name}.json`)),
        loadFailed.has(`${name}.json`) ? `${name}.json did not load` : undefined,
      );
    }
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
