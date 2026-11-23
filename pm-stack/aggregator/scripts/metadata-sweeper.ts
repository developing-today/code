#!/usr/bin/env bun
// Interval sweeper for dual ingestion (issue #9076; design §16, item 12).
//
// Re-scans in-scope repos for items the webhook listener missed and runs
// each through the shared Metadata check
// (src/metadata-ingestion.ts — the same function the listener calls).
// Outcomes: block present -> DB refresh; block generatable -> DB insert
// (+ body write-back with --write-back); insufficient -> full-triage
// queue packet (+ enqueue with --forward-triage). Merged PRs are valid
// targets: the scan covers all states, so quickly-merged PRs get their
// Metadata completed retroactively.
//
// The scan is bounded: the default --limit of 50 covers only the newest
// items per kind (issues and PRs listed separately). Older items are
// reached with a larger --limit or with targeted --only <issue|pr:number>
// runs. The epic catalog used for parent resolution is NOT bounded by
// --limit: it is fetched independently (all kind/epic issues), so a
// scanned item resolves its epic label even when that epic sits outside
// the scan window.
//
// Argument contract (#3352): the whole command line is declared here and
// parsed strictly before any subprocess, network, or file write. Unknown
// flags, stray positionals, valueless or empty values, and --opt=value
// vs --opt value mismatches are usage errors (exit 2). --dry-run is
// honoured on every path: no DB write, no body edit, no enqueue.
// --help prints this usage and exits 0.
//
// Usage:
//   bun run scripts/metadata-sweeper.ts --repo <owner/name> [--repo ...]
//     [--db <path>] [--limit <n>] [--triage-queue <name>]
//     [--write-back] [--forward-triage] [--dry-run] [--help]
// Env: METADATA_SWEEP_REPOS (comma-separated default repos),
//   PM_METADATA_DB_PATH (default DB), FLEET_BIN (default `fleet`).
import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { Database } from "bun:sqlite";
import {
  buildTriagePacket,
  collectPrimaryParentViolations,
  itemRefKey,
  openMetadataStore,
  primaryParentLabels,
  runMetadataCheck,
  storeOutcome,
  type EpicSlugMap,
  type IngestionItem,
  type PrimaryParentViolation,
} from "../src/metadata-ingestion.ts";

const USAGE =
  "metadata-sweeper: usage: bun run scripts/metadata-sweeper.ts --repo <owner/name> [--repo ...] " +
  "[--only <issue|pr:number> ...] [--db <path>] [--limit <n>] [--triage-queue <name>] [--write-back] [--forward-triage] [--dry-run] [--help]";

interface Options {
  repos: string[];
  /** Targeted backfill/demo: only these items, fetched individually. */
  only: { kind: "issue" | "pr"; number: number }[];
  dbPath: string;
  limit: number;
  triageQueue: string;
  writeBack: boolean;
  forwardTriage: boolean;
  dryRun: boolean;
}

function usageError(message: string): never {
  console.error(`metadata-sweeper: ${message}\n${USAGE}`);
  process.exit(2);
}

function takeValue(args: string[], index: number, flag: string): string {
  const value: string | undefined = args[index + 1];
  if (value === undefined || value.trim() === "" || value.startsWith("-")) {
    usageError(`${flag} requires a non-empty value (got ${value === undefined ? "no value" : JSON.stringify(value)})`);
  }
  return value;
}

/** Strict argv parse: every token accounted for before anything else runs. */
function parseArgs(argv: string[]): Options {
  const repos: string[] = [];
  const only: { kind: "issue" | "pr"; number: number }[] = [];
  let dbPath = process.env.PM_METADATA_DB_PATH ?? join(homedir(), ".local/state/pm-aggregator/pm-metadata.sqlite");
  let limit = 50;
  let triageQueue = "repo-health-code-pilot";
  let writeBack = false;
  let forwardTriage = false;
  let dryRun = false;
  for (let i = 0; i < argv.length; i++) {
    let arg = argv[i] as string;
    let inline: string | null = null;
    const equals = arg.indexOf("=");
    if (arg.startsWith("--") && equals > 2) {
      inline = arg.slice(equals + 1);
      arg = arg.slice(0, equals);
    }
    const valueOf = (flag: string): string => {
      if (inline !== null) {
        if (inline.trim() === "" || inline.startsWith("-")) usageError(`${flag} requires a non-empty value`);
        return inline;
      }
      const value = takeValue(argv, i, flag);
      i++;
      return value;
    };
    switch (arg) {
      case "--repo":
        repos.push(valueOf("--repo"));
        break;
      case "--only": {
        const raw = valueOf("--only");
        const match = /^(issue|pr):(\d+)$/.exec(raw.trim());
        const number = match?.[2] === undefined ? NaN : Number(match[2]);
        if (!match?.[1] || !Number.isSafeInteger(number) || number < 1) {
          usageError(`--only must be issue:<n> or pr:<n> with a positive safe integer (got ${JSON.stringify(raw)})`);
        }
        only.push({ kind: match[1] as "issue" | "pr", number });
        break;
      }
      case "--db":
        dbPath = valueOf("--db");
        break;
      case "--limit": {
        const raw = valueOf("--limit");
        const parsed = Number(raw);
        if (!Number.isSafeInteger(parsed) || parsed < 1) usageError(`--limit requires a positive integer (got ${JSON.stringify(raw)})`);
        limit = parsed;
        break;
      }
      case "--triage-queue":
        triageQueue = valueOf("--triage-queue");
        break;
      case "--write-back":
        if (inline !== null) usageError("--write-back takes no value");
        writeBack = true;
        break;
      case "--forward-triage":
        if (inline !== null) usageError("--forward-triage takes no value");
        forwardTriage = true;
        break;
      case "--dry-run":
        if (inline !== null) usageError("--dry-run takes no value");
        dryRun = true;
        break;
      case "--help":
        if (inline !== null) usageError("--help takes no value");
        console.log(USAGE);
        process.exit(0);
        break;
      default:
        usageError(`unknown argument '${arg}'`);
        break;
    }
  }
  if (repos.length === 0) {
    const fromEnv = (process.env.METADATA_SWEEP_REPOS ?? "")
      .split(",")
      .map((part) => part.trim())
      .filter(Boolean);
    repos.push(...(fromEnv.length > 0 ? fromEnv : ["developing-today/code"]));
  }
  for (const repo of repos) {
    if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) usageError(`--repo must be <owner/name> (got ${JSON.stringify(repo)})`);
  }
  return { repos, only, dbPath, limit, triageQueue, writeBack, forwardTriage, dryRun };
}

const FLEET_BIN = process.env.FLEET_BIN ?? "fleet";

function runFleet(args: string[], input?: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = execFile(FLEET_BIN, args, { maxBuffer: 64 * 1024 * 1024, timeout: 120_000 }, (err, stdout) =>
      err ? reject(new Error(`fleet ${args[0]} failed: ${(err as Error).message}`)) : resolve(stdout),
    );
    if (input !== undefined && child.stdin) {
      child.stdin.write(input);
      child.stdin.end();
    }
  });
}

interface GhLabel {
  name?: string;
}

interface GhItem {
  number?: number;
  title?: string;
  body?: string | null;
  labels?: (string | GhLabel)[];
  milestone?: { title?: string; dueOn?: string | null } | null;
  state?: string;
  author?: { login?: string } | null;
  createdAt?: string;
  updatedAt?: string;
  closedAt?: string | null;
  mergedAt?: string | null;
}

function labelNames(labels: GhItem["labels"]): string[] {
  return (labels ?? []).map((label) => (typeof label === "string" ? label : (label.name ?? ""))).filter(Boolean);
}

function toIngestionItem(repo: string, kind: "issue" | "pr", raw: GhItem): IngestionItem | null {
  if (!Number.isSafeInteger(raw.number) || (raw.number as number) < 1) return null;
  const state = String(raw.state ?? "OPEN").toUpperCase();
  return {
    repo,
    number: raw.number as number,
    kind,
    title: String(raw.title ?? ""),
    body: raw.body ?? null,
    labels: labelNames(raw.labels),
    milestoneTitle: raw.milestone?.title ?? null,
    milestoneDueOn: raw.milestone?.dueOn ?? null,
    state,
    author: raw.author?.login ?? "unknown",
    createdAt: String(raw.createdAt ?? ""),
    updatedAt: String(raw.updatedAt ?? ""),
    closedAt: raw.closedAt ?? raw.mergedAt ?? null,
  };
}

const ISSUE_FIELDS = "number,title,body,labels,milestone,state,author,createdAt,updatedAt,closedAt";
const PR_FIELDS = `${ISSUE_FIELDS},mergedAt`;

async function listItems(repo: string, kind: "issue" | "pr", limit: number): Promise<IngestionItem[]> {
  const command = kind === "issue" ? "issue" : "pr";
  const stdout = await runFleet([
    "gh",
    command,
    "list",
    "--repo",
    repo,
    "--state",
    "all",
    "--limit",
    String(limit),
    "--json",
    kind === "issue" ? ISSUE_FIELDS : PR_FIELDS,
  ]);
  const parsed: unknown = JSON.parse(stdout);
  if (!Array.isArray(parsed)) throw new Error(`unexpected ${command} list shape for ${repo}`);
  const items: IngestionItem[] = [];
  for (const raw of parsed as GhItem[]) {
    const item = toIngestionItem(repo, kind, raw);
    if (item) items.push(item);
  }
  return items;
}

/** Epic catalog for parent resolution, independent of the bounded item
 * scan: `--limit` scopes how many items are processed, never which
 * epics a label can resolve to. Targeted `--only` runs resolve against
 * the same full catalog, so a scanned PR can resolve an epic label even
 * when that epic sits outside the scan window. */
async function fetchEpicSlugs(repo: string): Promise<EpicSlugMap> {
  const stdout = await runFleet([
    "gh",
    "issue",
    "list",
    "--repo",
    repo,
    "--state",
    "all",
    "--label",
    "kind/epic",
    "--limit",
    "1000",
    "--json",
    "number,labels",
  ]);
  const parsed: unknown = JSON.parse(stdout);
  if (!Array.isArray(parsed)) throw new Error(`unexpected epic list shape for ${repo}`);
  const slugs: EpicSlugMap = new Map();
  for (const row of parsed as { number?: unknown; labels?: (string | GhLabel)[] }[]) {
    if (!Number.isSafeInteger(row.number) || (row.number as number) < 1) continue;
    for (const label of labelNames(row.labels)) {
      const slug = /^epic\/(.+)$/.exec(label)?.[1]?.trim().toLowerCase();
      if (slug && !slugs.has(slug)) slugs.set(slug, { repo, number: row.number as number });
    }
  }
  return slugs;
}

/** Native (sub-issue) parent lookup: the parent when found, null when the
 * item has none, undefined when the lookup itself failed. Callers must
 * not finalize a label-resolved parent on undefined — the next sweep
 * retries — because a failed lookup proves nothing about the item. */
async function fetchNativeParent(
  repo: string,
  number: number,
): Promise<{ repo: string; number: number; kind: string } | null | undefined> {
  let stdout: string;
  try {
    stdout = await runFleet(["gh", "api", `repos/${repo}/issues/${number}/parent`, "--jq", "{number: .number, repo_url: .repository_url}"]);
  } catch {
    return undefined;
  }
  try {
    const parsed = JSON.parse(stdout) as { number?: unknown; repo_url?: unknown };
    if (!Number.isSafeInteger(parsed.number) || typeof parsed.repo_url !== "string") return null;
    const match = /\/repos\/([^/]+\/[^/]+)$/.exec(parsed.repo_url);
    if (!match?.[1]) return null;
    return { repo: match[1], number: parsed.number as number, kind: "sub-issue" };
  } catch {
    return undefined;
  }
}

interface SweepCounts {
  scanned: number;
  current: number;
  generated: number;
  writtenBack: number;
  needsTriage: number;
  forwarded: number;
  primaryParentViolations: number;
  errors: number;
}

/** Ancestor chain of a native parent, as `repo#number` keys, for the
 * primary-parent ancestor rule (design §3). Returns null when the chain
 * could not be read: an unreadable chain never manufactures a finding. */
async function fetchAncestorRefs(
  repo: string,
  parent: { repo: string; number: number },
): Promise<Set<string> | null> {
  const refs = new Set<string>();
  let cursor = { repo: parent.repo, number: parent.number };
  try {
    // GitHub's hierarchy is shallow in practice; 10 bounds a cycle or a
    // pathological chain without silently truncating a realistic one.
    for (let depth = 0; depth < 10; depth++) {
      refs.add(itemRefKey(cursor));
      const stdout = await runFleet([
        "gh",
        "api",
        `repos/${cursor.repo}/issues/${cursor.number}/parent`,
        "--jq",
        "{number: .number, repo_url: .repository_url}",
      ]);
      const parsed = JSON.parse(stdout) as { number?: unknown; repo_url?: unknown };
      if (!Number.isSafeInteger(parsed.number) || typeof parsed.repo_url !== "string") break;
      const match = /\/repos\/([^/]+\/[^/]+)$/.exec(parsed.repo_url);
      if (!match?.[1]) break;
      cursor = { repo: match[1], number: parsed.number as number };
    }
    return refs;
  } catch {
    return null;
  }
}

async function fetchOne(repo: string, kind: "issue" | "pr", number: number): Promise<IngestionItem | null> {
  const command = kind === "issue" ? "issue" : "pr";
  const stdout = await runFleet([
    "gh",
    command,
    "view",
    String(number),
    "--repo",
    repo,
    "--json",
    kind === "issue" ? ISSUE_FIELDS : PR_FIELDS,
  ]);
  return toIngestionItem(repo, kind, JSON.parse(stdout) as GhItem);
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2));
  // Dry runs never touch the configured database file: every path below
  // already skips persistence, and the store lives in memory.
  const db = openMetadataStore(options.dryRun ? ":memory:" : options.dbPath, (path) => new Database(path));
  try {
    const scratch = await mkdtemp(join(tmpdir(), "metadata-sweeper-"));
    const summary: Record<string, SweepCounts & { dbRows: number; violations: PrimaryParentViolation[] }> = {};
    try {
      for (const repo of options.repos) {
        const counts: SweepCounts = {
          scanned: 0,
          current: 0,
          generated: 0,
          writtenBack: 0,
          needsTriage: 0,
          forwarded: 0,
          primaryParentViolations: 0,
          errors: 0,
        };
        const violations: PrimaryParentViolation[] = [];
        try {
          const [issues, prs] = await Promise.all([
            listItems(repo, "issue", options.limit),
            listItems(repo, "pr", options.limit),
          ]);
          // Epic catalog independent of the bounded scan: --limit scopes
          // items processed, never which epics a label may resolve to. On
          // catalog failure fall back to the scanned window and say so —
          // targeted --only runs still resolve against the full catalog
          // when the fetch succeeds.
          let epicSlugs: EpicSlugMap;
          try {
            epicSlugs = await fetchEpicSlugs(repo);
          } catch (error) {
            console.error(`metadata-sweeper: repo ${repo}: epic catalog fetch failed (${(error as Error).message}); falling back to scanned issues`);
            epicSlugs = new Map();
            for (const issue of issues) {
              if (!issue.labels.includes("kind/epic")) continue;
              for (const label of issue.labels) {
                const slug = /^epic\/(.+)$/.exec(label)?.[1]?.trim().toLowerCase();
                if (slug && !epicSlugs.has(slug)) epicSlugs.set(slug, { repo, number: issue.number });
              }
            }
          }
          const scanned = [...issues, ...prs];
          const items: IngestionItem[] = options.only.length > 0 ? [] : scanned;
          for (const entry of options.only) {
            const fetched = await fetchOne(repo, entry.kind, entry.number);
            if (fetched) items.push(fetched);
            else {
              counts.errors++;
              console.error(`metadata-sweeper: ${repo} ${entry.kind}:${entry.number} not found`);
            }
          }
          for (const item of items) {
            counts.scanned++;
            try {
              let checked = item;
              let outcome = runMetadataCheck(item, { epicSlugs, ingestedBy: "metadata-sweeper" });
              // Native parent is authoritative (see resolveAscertainedParent):
              // consult it before finalizing any ascertained outcome, not
              // just needs-triage ones, or a stale label would become the
              // stored direct_parent. PRs skip this — sub-issue parents are
              // an issue concept, so their label path stands. A failed
              // lookup is an error for the next sweep to retry, never a
              // finalized label parent and never triage spam: it proves
              // nothing about the item either way.
              if (item.kind === "issue" && !item.nativeParent && outcome.kind !== "current") {
                const parent = await fetchNativeParent(repo, item.number);
                if (parent) {
                  checked = { ...item, nativeParent: parent };
                  outcome = runMetadataCheck(checked, { epicSlugs, ingestedBy: "metadata-sweeper" });
                } else if (parent === undefined) {
                  counts.errors++;
                  console.error(`metadata-sweeper: ${repo}#${item.number}: native-parent lookup failed; skipping (retry next sweep)`);
                  continue;
                }
              }

              // Primary-parent convention (design §3): validate and report
              // violations in the audit output, never resolve them here.
              let ancestorRefs: Set<string> | undefined;
              if (checked.kind === "issue" && checked.nativeParent && primaryParentLabels(checked.labels).length > 0) {
                const chain = await fetchAncestorRefs(repo, checked.nativeParent);
                if (chain === null) {
                  console.error(
                    `metadata-sweeper: ${repo}#${checked.number}: ancestor chain unread; ancestor rule not checked`,
                  );
                } else {
                  ancestorRefs = chain;
                }
              }
              for (const violation of collectPrimaryParentViolations(checked, epicSlugs, ancestorRefs)) {
                counts.primaryParentViolations++;
                violations.push(violation);
              }
              if (outcome.kind === "needs-triage") {
                counts.needsTriage++;
                if (options.forwardTriage && !options.dryRun) {
                  const packet = buildTriagePacket(outcome.evidence, "metadata-sweeper", new Date().toISOString());
                  const packetPath = join(scratch, `triage-${repo.replace("/", "-")}-${item.number}.json`);
                  await writeFile(packetPath, JSON.stringify(packet, null, 2));
                  await runFleet(["queue", "enqueue", packetPath, "--queue", options.triageQueue]);
                  counts.forwarded++;
                }
                continue;
              }
              if (!options.dryRun) storeOutcome(db, outcome);
              if (outcome.kind === "current") {
                counts.current++;
              } else {
                counts.generated++;
                if (options.writeBack && !options.dryRun) {
                  // Race-narrowing guard, not an atomic update: re-read the
                  // body and only write back when it still matches the text
                  // the check ran on. A changed (or unreadable) body skips
                  // the edit and counts an error for the next sweep.
                  const current = await fetchOne(repo, item.kind, item.number).catch(() => null);
                  if (!current || current.body !== item.body) {
                    counts.errors++;
                    console.error(`metadata-sweeper: ${repo}#${item.number}: body changed since the check; skipping write-back`);
                  } else {
                    const bodyPath = join(scratch, `body-${repo.replace("/", "-")}-${item.number}.md`);
                    await writeFile(bodyPath, outcome.body);
                    const command = item.kind === "issue" ? "issue" : "pr";
                    await runFleet(["gh", command, "edit", String(item.number), "--repo", repo, "--body-file", bodyPath]);
                    counts.writtenBack++;
                  }
                }
              }
            } catch (error) {
              counts.errors++;
              console.error(`metadata-sweeper: ${repo}#${item.number}: ${(error as Error).message}`);
            }
          }
        } catch (error) {
          counts.errors++;
          console.error(`metadata-sweeper: repo ${repo}: ${(error as Error).message}`);
        }
        const row = db.query("SELECT COUNT(*) AS count FROM pm_metadata_items").get() as { count: number };
        summary[repo] = { ...counts, dbRows: Number(row.count), violations };
      }
    } finally {
      await rm(scratch, { recursive: true, force: true });
    }
    console.log(
      JSON.stringify(
        {
          repos: options.repos,
          dryRun: options.dryRun,
          writeBack: options.writeBack,
          forwardTriage: options.forwardTriage,
          summary,
        },
        null,
        2,
      ),
    );
  } finally {
    db.close();
  }
}

main().catch((error) => {
  console.error(`metadata-sweeper: fatal: ${(error as Error).message}`);
  process.exit(1);
});
