// Metadata query engine (#12587; design §7.1, #9060 §13 + §15) —
// the logic behind the query door (src/routes/query.ts) and the TS
// CLI query verbs (src/query-cli.ts), so both render the same
// results from the same code path.
//
// THE SURFACE. List/filter issues and PRs by Metadata fields —
// direct parent, session, lane, host, queue id, link relation/status
// pairs — served from the metadata DB (#9074's surface, hosted),
// with GitHub as fallback for fields the DB has not yet ingested
// (#9060 §13's rule). Results are scoped to the caller's
// credential-resolved binding_id, like every door: the store is
// asked only for the caller's rows, and a fallback item is
// attributed to a tenancy only by its stamped provenance
// auth_subject — an item no tenancy can be attributed from waits
// for ingestion (#12590) rather than leaking across accounts.
//
// DIVERGENCE (#9060 §15). For every DB result whose current GitHub
// body can be read, the body's Metadata block is compared with the
// DB row. A difference is reported as a finding (the top-level keys
// whose values differ), never silently overwritten — and the BODY
// wins on content: the served `metadata` is the body's block. A
// body whose block was removed or no longer parses reports the
// finding on the synthetic field "pm_meta_block" and serves the DB
// content, there being no body content to win. Each result also
// carries the #12586 stamp verification of the served provenance
// when verifier keys are configured — a provenance hand-edit shows
// up both as a divergence finding and as stamp status "diverged".
//
// THE COLLECTOR'S READ PATH (acceptance). The collector consumes
// this surface as a genealogy input exactly as any caller does:
// authenticate (a service token scoped to the `query` door is the
// collector's shape), then GET /v1/query?<filters> or POST
// /v1/query with the filter as JSON. Results carry the full served
// Metadata for every match, so one filtered read is a genealogy
// slice; walking direct_parent / link targets from a result's
// Metadata is the genealogy traversal. High-volume consumers use
// the direct-Turso read path instead (design §3.3 rule 1: read-only,
// metadata DB only, account-scoped token) — this REST surface is
// the authenticated, divergence-reporting path, and the hosted
// surface adds no divergence semantics beyond §15's.

import { FailClosed } from "./errors.ts";
import type { GitHubItemSource } from "./github.ts";
import { parseMetadataBlock } from "./metadata.ts";
import { verifyStamp, type StampVerification } from "./stamp-sig.ts";
import {
  extractKeys,
  matchesMetadataFilter,
  type MetadataFilter,
  type MetadataRecord,
  type MetadataStore,
} from "./stamping.ts";

export type { MetadataFilter };

/** One query result. `metadata` is the served content (the body's
 * block when a checked body wins, else the DB row's; for a
 * github-source result, the body's block — there is no DB row). */
export interface QueryResultItem {
  repo: string;
  kind: "issue" | "pr";
  number: number;
  url: string;
  metadata: Record<string, unknown>;
  source: "db" | "github";
  /** True when the item's current GitHub body was read for this
   * result (divergence judged / fallback parsed from it). */
  body_checked: boolean;
  /** The §15 divergence finding, or null when a checked body agrees
   * with the DB row, or the result came from GitHub. */
  divergence: { fields: string[] } | null;
  /** verifyStamp over the served provenance (#12586), or null when
   * no verifier keys are configured or the served provenance cannot
   * be canonicalized (which also reports as a divergence). */
  stamp: StampVerification | null;
}

export interface QueryResponse {
  filter: MetadataFilter;
  count: number;
  results: QueryResultItem[];
}

// --- Filter parsing (shared by GET params, POST JSON, and the CLI).

const FILTER_KEYS = new Set([
  "repo",
  "kind",
  "direct_parent",
  "session_id",
  "lane",
  "host",
  "queue_id",
  "link_relation",
  "link_status",
  "link_target",
]);

function invalidQuery(message: string): FailClosed {
  return new FailClosed(400, "invalid_query", message);
}

function filterString(raw: Record<string, unknown>, key: string): string | undefined {
  const value = raw[key];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "string" || value.trim() === "") {
    throw invalidQuery(`${key} must be a non-empty string when present`);
  }
  return value;
}

/** An item reference filter (direct_parent, link_target): either
 * the "owner/name#123" string form or the §12 object form
 * {repo, number} (extra keys, e.g. kind, are the block's own and
 * are not part of the match). */
function filterItemRef(raw: Record<string, unknown>, key: string): { repo: string; number: number } | undefined {
  const value = raw[key];
  if (value === undefined || value === null) return undefined;
  if (typeof value === "string") {
    const match = /^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)#([0-9]+)$/.exec(value.trim());
    if (!match) {
      throw invalidQuery(`${key} must be in owner/name#number form when given as a string`);
    }
    const number = Number(match[2]);
    if (!Number.isSafeInteger(number)) {
      throw invalidQuery(`${key} number must be a safe integer`);
    }
    return { repo: match[1], number };
  }
  if (typeof value === "object" && !Array.isArray(value)) {
    const v = value as Record<string, unknown>;
    if (
      typeof v["repo"] !== "string" ||
      !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(v["repo"]) ||
      typeof v["number"] !== "number" ||
      !Number.isSafeInteger(v["number"])
    ) {
      throw invalidQuery(`${key} must carry repo (owner/name) and an integer number`);
    }
    return { repo: v["repo"], number: v["number"] };
  }
  throw invalidQuery(`${key} must be a string or an object`);
}

/** Parse + validate a raw filter object (POST JSON, CLI flags, or
 * GET params pre-collected into an object). Unknown keys refuse:
 * a misspelled filter that silently matched everything would be a
 * wrong answer, not an empty one. */
export function parseQueryFilter(raw: Record<string, unknown>): MetadataFilter {
  for (const key of Object.keys(raw)) {
    if (!FILTER_KEYS.has(key)) {
      throw invalidQuery(`unknown filter field: ${key}`);
    }
  }
  const filter: MetadataFilter = {};
  const repo = filterString(raw, "repo");
  if (repo !== undefined) {
    if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) {
      throw invalidQuery("repo must be in owner/name form");
    }
    filter.repo = repo;
  }
  const kind = filterString(raw, "kind");
  if (kind !== undefined) {
    if (kind !== "issue" && kind !== "pr") {
      throw invalidQuery('kind must be "issue" or "pr"');
    }
    filter.kind = kind;
  }
  const directParent = filterItemRef(raw, "direct_parent");
  if (directParent !== undefined) filter.directParent = directParent;
  const sessionId = filterString(raw, "session_id");
  if (sessionId !== undefined) filter.sessionId = sessionId;
  const lane = filterString(raw, "lane");
  if (lane !== undefined) filter.lane = lane;
  const host = filterString(raw, "host");
  if (host !== undefined) filter.host = host;
  const queueId = filterString(raw, "queue_id");
  if (queueId !== undefined) filter.queueId = queueId;
  const linkRelation = filterString(raw, "link_relation");
  if (linkRelation !== undefined) filter.linkRelation = linkRelation;
  const linkStatus = filterString(raw, "link_status");
  if (linkStatus !== undefined) filter.linkStatus = linkStatus;
  const linkTarget = filterItemRef(raw, "link_target");
  if (linkTarget !== undefined) filter.linkTarget = linkTarget;
  return filter;
}

// --- Divergence comparison.

/** Deterministic JSON for JSON-shaped values (sorted keys,
 * undefined object values dropped as JSON drops them) — equality
 * only, never a signing input (that is stamp-sig.ts's canonical
 * form, a stricter thing). */
function stableStringify(value: unknown): string {
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value) ?? "null";
  }
  if (Array.isArray(value)) {
    return `[${value.map((v) => stableStringify(v === undefined ? null : v)).join(",")}]`;
  }
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record)
    .filter((k) => record[k] !== undefined)
    .sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${stableStringify(record[k])}`).join(",")}}`;
}

/** The top-level Metadata keys whose values differ between the DB
 * row and the body block (present-on-one-side counts as differing). */
export function divergenceFields(dbMetadata: Record<string, unknown>, bodyBlock: Record<string, unknown>): string[] {
  const keys = new Set([...Object.keys(dbMetadata), ...Object.keys(bodyBlock)]);
  const fields: string[] = [];
  for (const key of keys) {
    const dbHasKey = Object.hasOwn(dbMetadata, key);
    const bodyHasKey = Object.hasOwn(bodyBlock, key);
    if (dbHasKey !== bodyHasKey || (dbHasKey && stableStringify(dbMetadata[key]) !== stableStringify(bodyBlock[key]))) {
      fields.push(key);
    }
  }
  return fields.sort();
}

// --- The query itself.

export interface QueryDeps {
  store: MetadataStore;
  githubItems?: GitHubItemSource;
  /** Imported stamp verifier keys (#12586), or null when the
   * deployment configures none. */
  verifyKeys: ReadonlyMap<string, CryptoKey> | null;
}

async function stampStatus(
  metadata: Record<string, unknown>,
  verifyKeys: ReadonlyMap<string, CryptoKey> | null,
): Promise<StampVerification | null> {
  if (verifyKeys === null) return null;
  const provenance = metadata["provenance"];
  if (typeof provenance !== "object" || provenance === null || Array.isArray(provenance)) {
    return null;
  }
  try {
    return await verifyStamp(provenance as Record<string, unknown>, verifyKeys);
  } catch {
    // An uncanonicalizable provenance cannot verify; the divergence
    // comparison has already reported the field itself.
    return null;
  }
}

function itemKey(repo: string, kind: string, number: number): string {
  return `${repo}#${kind}#${number}`;
}

/** Run one Metadata query for one tenancy. See the header for the
 * fallback, tenancy, and divergence rules this implements. */
export async function runMetadataQuery(
  deps: QueryDeps,
  bindingId: string,
  filter: MetadataFilter,
): Promise<QueryResponse> {
  const rows = await deps.store.list(bindingId, filter);
  const results: QueryResultItem[] = [];
  const seen = new Set<string>();

  for (const row of rows) {
    seen.add(itemKey(row.repo, row.kind, row.number));
    let metadata = row.metadata;
    let bodyChecked = false;
    let divergence: { fields: string[] } | null = null;
    if (deps.githubItems) {
      const item = await deps.githubItems.getItem(row.repo, row.kind, row.number);
      if (item !== null) {
        bodyChecked = true;
        let bodyBlock: Record<string, unknown> | null = null;
        try {
          bodyBlock = parseMetadataBlock(item.body).block;
        } catch {
          bodyBlock = null; // a block that no longer parses is a finding, below
        }
        if (bodyBlock === null) {
          divergence = { fields: ["pm_meta_block"] };
        } else {
          const fields = divergenceFields(row.metadata, bodyBlock);
          if (fields.length > 0) {
            divergence = { fields };
            metadata = bodyBlock; // §15: the body wins on content
          }
        }
      }
    }
    results.push({
      repo: row.repo,
      kind: row.kind,
      number: row.number,
      url: row.url,
      metadata,
      source: "db",
      body_checked: bodyChecked,
      divergence,
      stamp: await stampStatus(metadata, deps.verifyKeys),
    });
  }

  // GitHub fallback (#9060 §13): items in the queried repo whose
  // blocks match the filter but which the DB has not yet ingested.
  // Only runs when a repo is named — GitHub is scanned per repo,
  // never globally — and only attributes an item to the caller when
  // its stamped provenance names the caller's binding.
  if (filter.repo !== undefined && deps.githubItems) {
    const items = await deps.githubItems.listItems(filter.repo);
    for (const item of items) {
      if (seen.has(itemKey(item.repo, item.kind, item.number))) continue;
      let block: Record<string, unknown> | null = null;
      try {
        block = parseMetadataBlock(item.body).block;
      } catch {
        block = null;
      }
      if (block === null) continue; // no parseable block: no Metadata fields to match
      const provenance = block["provenance"];
      const authSubject =
        typeof provenance === "object" && provenance !== null && !Array.isArray(provenance)
          ? (provenance as Record<string, unknown>)["auth_subject"]
          : undefined;
      if (authSubject !== bindingId) continue;
      const candidate: MetadataRecord = {
        repo: item.repo,
        kind: item.kind,
        number: item.number,
        url: item.url,
        bindingId,
        metadata: block,
        extracted: extractKeys(block),
        insertedAt: "",
      };
      if (!matchesMetadataFilter(candidate, filter)) continue;
      results.push({
        repo: item.repo,
        kind: item.kind,
        number: item.number,
        url: item.url,
        metadata: block,
        source: "github",
        body_checked: true,
        divergence: null,
        stamp: await stampStatus(block, deps.verifyKeys),
      });
    }
  }

  results.sort((a, b) => a.repo.localeCompare(b.repo) || a.kind.localeCompare(b.kind) || a.number - b.number);
  return { filter, count: results.length, results };
}
