import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  artifactIdFor,
  buildTriagePacket,
  extractMetadataPayload,
  openMetadataStore,
  renderMetadataBlock,
  runMetadataCheck,
  sourceUrlFor,
  storeOutcome,
  upsertMetadataBlock,
  type EpicSlugMap,
  type IngestionItem,
  type MetadataCheckContext,
} from "../src/metadata-ingestion.ts";
import { createPmMetadataStore, getPmMetadata } from "../src/metadata-store.ts";

const EPICS: EpicSlugMap = new Map([
  ["pm-board", { repo: "developing-today/code", number: 9060 }],
  ["atlas", { repo: "developing-today/code", number: 8983 }],
]);

const CONTEXT: MetadataCheckContext = { epicSlugs: EPICS, ingestedBy: "test", now: "2026-11-23T08:00:00Z" };

function item(overrides: Partial<IngestionItem> = {}): IngestionItem {
  return {
    repo: "developing-today/code",
    number: 9999,
    kind: "issue",
    title: "A block-less item",
    body: "Just a description.\n",
    labels: ["kind/feature", "primary-parent/pm-board"],
    state: "OPEN",
    author: "plain-gh-user",
    createdAt: "2026-11-20T00:00:00Z",
    updatedAt: "2026-11-21T00:00:00Z",
    ...overrides,
  };
}

describe("dual-ingestion block plumbing", () => {
  test("extracts the canonical JSON payload and ignores ordinary comments", () => {
    expect(extractMetadataPayload(null)).toEqual({ found: false, malformed: false });
    expect(extractMetadataPayload("no block <!-- agent-note -->")).toEqual({ found: false, malformed: false });
    const payload = extractMetadataPayload(`text\n${renderMetadataBlock({ pm_meta: 1, links: [] })}\ntail`);
    expect(payload).toEqual({ found: true, json: JSON.stringify({ pm_meta: 1, links: [] }, null, 2) });
  });

  test("markers without a JSON fence are malformed, not silently absent", () => {
    expect(extractMetadataPayload("<!-- pm-metadata:start -->oops<!-- pm-metadata:end -->")).toEqual({
      found: false,
      malformed: true,
    });
  });

  test("a title containing triple backticks round-trips through render/extract/parse", () => {
    const title = "Document ```json examples";
    const metadata = {
      pm_meta: 1,
      direct_parent: { repo: "developing-today/code", number: 9060, kind: "epic" },
      links: [] as unknown[],
      title,
    };
    const body = `description\n\n${renderMetadataBlock(metadata)}\n`;
    const payload = extractMetadataPayload(body);
    expect(payload.found).toBe(true);
    if (!payload.found) return;
    expect((JSON.parse(payload.json) as { title: string }).title).toBe(title);
    // The next check reads its own block as current, not generated or triage.
    const outcome = runMetadataCheck(item({ body, title, labels: ["kind/feature"] }), CONTEXT);
    expect(outcome.kind).toBe("current");
    if (outcome.kind !== "current") return;
    expect(outcome.record.parsed.extras.title).toBe(title);
  });

  test("upsertMetadataBlock replaces in place and appends idempotently", () => {
    const metadata = { pm_meta: 1, links: [] as unknown[] };
    const once = upsertMetadataBlock("body\n", metadata);
    expect(once).toContain("<!-- pm-metadata:start -->");
    expect(upsertMetadataBlock(once, metadata)).toBe(once);
    expect(upsertMetadataBlock(null, metadata)).toBe(`${renderMetadataBlock(metadata)}\n`);
  });
});

describe("runMetadataCheck: the one check both watchers share", () => {
  test("a parseable block is current: trusted as content, stored verbatim", () => {
    const block = renderMetadataBlock({
      pm_meta: 1,
      direct_parent: { repo: "developing-today/code", number: 9060, kind: "epic" },
      links: [],
      provenance: { actor: "fleet" },
    });
    const outcome = runMetadataCheck(item({ body: `description\n\n${block}\n` }), CONTEXT);
    expect(outcome.kind).toBe("current");
    if (outcome.kind !== "current") return;
    expect(outcome.record.artifactId).toBe("github:developing-today/code#issue/9999");
    expect(outcome.record.sourceUrl).toBe(sourceUrlFor(item()));
    expect(outcome.record.parsed.metadata.direct_parent).toEqual({
      repo: "developing-today/code",
      number: 9060,
      kind: "epic",
    });
  });

  test("a block-less item with a resolvable parent is generated with provenance", () => {
    const outcome = runMetadataCheck(item(), CONTEXT);
    expect(outcome.kind).toBe("generated");
    if (outcome.kind !== "generated") return;
    expect(outcome.replacedInvalidBlock).toBe(false);
    expect(outcome.record.parsed.indexes.directParentNumber).toBe(9060);
    expect(outcome.body).toContain("<!-- pm-metadata:start -->");
    expect(outcome.body.startsWith("Just a description.")).toBe(true);
    // The generated row round-trips through the store.
    const db = new Database(":memory:");
    try {
      createPmMetadataStore(db);
      storeOutcome(db, outcome);
      const row = getPmMetadata(db, artifactIdFor(item()));
      expect(row?.indexes.directParentNumber).toBe(9060);
      expect(row?.metadata).toMatchObject({ pm_meta: 1 });
    } finally {
      db.close();
    }
  });

  test("an unresolvable item is forwarded for triage, never half-described", () => {
    const outcome = runMetadataCheck(item({ labels: ["kind/feature"] }), CONTEXT);
    expect(outcome.kind).toBe("needs-triage");
    if (outcome.kind !== "needs-triage") return;
    expect(outcome.evidence.reasons).toHaveLength(1);
    expect(outcome.evidence.reasons[0]).toMatch(/no resolvable parent/);
    expect(outcome.evidence.sourceUrl).toBe("https://github.com/developing-today/code/issues/9999");
    // Triage outcomes store nothing: the queue packet is their trail.
    const db = new Database(":memory:");
    try {
      createPmMetadataStore(db);
      storeOutcome(db, outcome);
      expect(getPmMetadata(db, artifactIdFor(item()))).toBeNull();
    } finally {
      db.close();
    }
  });

  test("distinct slugs resolving to different epics are ambiguous triage", () => {
    const outcome = runMetadataCheck(item({ labels: ["primary-parent/pm-board", "epic/atlas"] }), CONTEXT);
    expect(outcome.kind).toBe("needs-triage");
    if (outcome.kind !== "needs-triage") return;
    expect(outcome.evidence.reasons[0]).toMatch(/ambiguous program/);
  });

  test("a native parent takes precedence over a stale label parent", () => {
    const outcome = runMetadataCheck(
      item({
        labels: ["primary-parent/atlas"],
        nativeParent: { repo: "developing-today/code", number: 9123, kind: "issue" },
      }),
      CONTEXT,
    );
    expect(outcome.kind).toBe("generated");
    if (outcome.kind !== "generated") return;
    expect(outcome.record.parsed.indexes.directParentNumber).toBe(9123);
  });

  test("an epic cannot resolve itself as its direct parent", () => {
    const outcome = runMetadataCheck(item({ number: 9060, labels: ["kind/epic", "primary-parent/pm-board"] }), CONTEXT);
    expect(outcome.kind).toBe("needs-triage");
    if (outcome.kind !== "needs-triage") return;
    expect(outcome.evidence.reasons[0]).toMatch(/no resolvable parent/);
  });

  test("an unparseable block is replaced by ascertainment, flagged on the outcome", () => {
    const outcome = runMetadataCheck(
      item({ body: "<!-- pm-metadata:start -->```json\n{broken\n```<!-- pm-metadata:end -->" }),
      CONTEXT,
    );
    expect(outcome.kind).toBe("generated");
    if (outcome.kind !== "generated") return;
    expect(outcome.replacedInvalidBlock).toBe(true);
    // One block in the result: the corrupt original is gone.
    expect(outcome.body.match(/pm-metadata:start/g)).toHaveLength(1);
    expect(outcome.record.parsed.metadata).toMatchObject({
      ascertained: expect.objectContaining({ replaced_invalid_block: true }),
    } as object);
  });

  test("an unparseable block with no resolvable parent is triage with the invalid-block reason", () => {
    const outcome = runMetadataCheck(
      item({
        labels: ["kind/feature"],
        body: "<!-- pm-metadata:start -->no fence<!-- pm-metadata:end -->",
      }),
      CONTEXT,
    );
    expect(outcome.kind).toBe("needs-triage");
    if (outcome.kind !== "needs-triage") return;
    expect(outcome.evidence.reasons[0]).toMatch(/no JSON payload/);
    expect(outcome.evidence.ascertained.invalidBlock).toBe(true);
  });

  test("a merged PR is a valid ingestion target, not a miss", () => {
    const outcome = runMetadataCheck(
      item({
        kind: "pr",
        number: 12371,
        state: "MERGED",
        title: "feat(pm): add versioned metadata store foundation",
      }),
      CONTEXT,
    );
    expect(outcome.kind).toBe("generated");
    if (outcome.kind !== "generated") return;
    expect(outcome.record.itemKind).toBe("pr");
    expect(outcome.record.artifactId).toBe("github:developing-today/code#pr/12371");
    expect(outcome.record.sourceUrl).toBe("https://github.com/developing-today/code/pull/12371");
  });
});

describe("buildTriagePacket: the broker's required fields, pinned", () => {
  test("the forward packet passes repo-queue payload validation", () => {
    const outcome = runMetadataCheck(item({ labels: ["kind/feature"] }), CONTEXT);
    expect(outcome.kind).toBe("needs-triage");
    if (outcome.kind !== "needs-triage") return;
    const packet = buildTriagePacket(outcome.evidence, "test", "2026-11-23T08:00:00Z");
    expect(packet.dedupe_key).toBe("repo-health:v1:triage:developing-today/code:issue:9999");
    const payload = packet.payload as Record<string, unknown>;
    expect(payload.schema).toBe("repo-task/v1");
    expect(payload.kind).toBe("triage");
    for (const field of ["problem", "priority_reason"]) {
      expect(typeof payload[field]).toBe("string");
      expect((payload[field] as string).length).toBeGreaterThan(0);
    }
    for (const field of ["evidence", "acceptance", "tests", "context_refs"]) {
      expect(Array.isArray(payload[field])).toBe(true);
      expect((payload[field] as unknown[]).length).toBeGreaterThan(0);
    }
    const scope = payload.scope as { include: unknown[]; exclude: unknown[] };
    expect(Array.isArray(scope.include)).toBe(true);
    expect(scope.exclude.length).toBeGreaterThan(0);
    const risk = payload.risk as { level: string; flags: unknown[] };
    expect(["low", "medium", "high"]).toContain(risk.level);
    expect(Array.isArray(risk.flags)).toBe(true);
    for (const field of ["timebox_minutes", "max_attempts"]) {
      expect(Number.isInteger(payload[field])).toBe(true);
      expect(payload[field] as number).toBeGreaterThan(0);
    }
  });
});

describe("openMetadataStore: clean-environment boot (#13293)", () => {
  test("creates a missing parent directory before opening", async () => {
    const root = await mkdtemp(join(tmpdir(), "pm-metadata-store-"));
    try {
      const path = join(root, "state", "pm-aggregator", "pm-metadata.sqlite");
      const db = openMetadataStore(path, (p) => new Database(p));
      expect(
        db.query("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'pm_metadata_items'").get(),
      ).toEqual({ name: "pm_metadata_items" });
      db.close();
      expect(existsSync(path)).toBe(true);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("an unhealable parent still fails closed", async () => {
    const root = await mkdtemp(join(tmpdir(), "pm-metadata-store-"));
    try {
      const blocker = join(root, "blocker");
      await writeFile(blocker, "not a directory");
      expect(() => openMetadataStore(join(blocker, "pm-metadata.sqlite"), (p) => new Database(p))).toThrow();
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
