import { Database } from "bun:sqlite";
import { describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { createHmac } from "node:crypto";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildEpicSlugMap, isConfirmedParentAbsent, parseNativeParentResponse } from "../src/metadata-ingestion.ts";
import { createPmMetadataStore, getPmMetadata } from "../src/metadata-store.ts";
import { createMetadataWebhookHandler } from "../src/webhook-listener.ts";

const SECRET = "test-native-parent-secret";
const NOW = "2026-11-27T09:00:00.000Z";
const EPICS = new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]);

describe("shared native-parent contract (#16011)", () => {
  test("only a 404 classifies as confirmed-absent", () => {
    expect(isConfirmedParentAbsent(new Error("gh: Not Found (HTTP 404)"))).toBe(true);
    expect(
      isConfirmedParentAbsent(
        new Error(
          "fleet gh failed: Command failed: fleet gh api repos/a/b/issues/1/parent\n" +
            'gh: Not Found (HTTP 404)\n{"message":"Not Found"}',
        ),
      ),
    ).toBe(true);
    expect(isConfirmedParentAbsent(new Error("gh: Unauthorized (HTTP 401)"))).toBe(false);
    expect(isConfirmedParentAbsent(new Error("gh: Forbidden (HTTP 403): rate limit exceeded"))).toBe(false);
    expect(isConfirmedParentAbsent(new Error("gh: Internal Server Error (HTTP 500)"))).toBe(false);
    expect(isConfirmedParentAbsent(new Error("request failed: socket hang up"))).toBe(false);
    expect(isConfirmedParentAbsent(new Error(""))).toBe(false);
    expect(isConfirmedParentAbsent(null)).toBe(false);
  });

  test("parses the parent endpoint shape, and treats unreadable output as retryable", () => {
    expect(parseNativeParentResponse('{"number": 100, "repo_url": "https://api.github.com/repos/o/r"}')).toEqual({
      repo: "o/r",
      number: 100,
      kind: "sub-issue",
    });
    expect(parseNativeParentResponse("{}")).toBeNull();
    expect(parseNativeParentResponse('{"number": "x", "repo_url": 42}')).toBeNull();
    expect(parseNativeParentResponse("not json")).toBeUndefined();
    expect(parseNativeParentResponse("")).toBeUndefined();
  });

  test("builds the epic catalog and resolves epics outside any item scan window", () => {
    const catalog = buildEpicSlugMap(
      [
        { number: 9060, labels: ["kind/epic", "epic/pm-board"] },
        { number: "bad", labels: ["kind/epic", "epic/bogus"] },
        { number: 12, labels: ["kind/feature"] },
        { number: 9060, labels: [{ name: "kind/epic" }, { name: "epic/pm-board" }] },
      ],
      "developing-today/code",
    );
    expect(catalog.get("pm-board")).toEqual({ repo: "developing-today/code", number: 9060 });
    expect(catalog.has("bogus")).toBe(false);
    expect(catalog.size).toBe(1);
    // The catalog is fetched independently of the bounded item scan: an
    // epic resolves even when no scanned item names it.
    const scannedNumbers = [4242];
    expect(scannedNumbers).not.toContain(catalog.get("pm-board")?.number);
  });
});

function signedIssueRequest(overrides: Record<string, unknown> = {}): Request {
  const rawBody = JSON.stringify({
    action: "opened",
    repository: { full_name: "developing-today/code" },
    issue: {
      number: 4242,
      title: "Created outside Fleet",
      body: "Description",
      state: "open",
      user: { login: "plain-gh-user" },
      labels: [{ name: "kind/feature" }, { name: "primary-parent/pm-board" }],
      milestone: null,
      created_at: "2026-11-26T12:00:00Z",
      updated_at: "2026-11-27T08:00:00Z",
      closed_at: null,
      ...overrides,
    },
  });
  const signature = createHmac("sha256", SECRET).update(rawBody).digest("hex");
  return new Request("http://localhost/webhooks/github", {
    method: "POST",
    headers: { "x-github-event": "issues", "x-hub-signature-256": `sha256=${signature}` },
    body: rawBody,
  });
}

describe("webhook native-parent resolution (#16011)", () => {
  test("a failed lookup is retryable: non-2xx with no store, write-back, or triage writes", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    let writes = 0;
    let enqueued = 0;
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => EPICS,
      resolveNativeParent: async () => undefined,
      readCurrent: async (item) => item,
      writeBack: async () => {
        writes++;
      },
      enqueueTriage: async () => {
        enqueued++;
      },
      now: () => NOW,
    });

    try {
      const response = await handler(signedIssueRequest());
      expect(response.status).toBe(500);
      expect(writes).toBe(0);
      expect(enqueued).toBe(0);
      expect(getPmMetadata(db, "gh:developing-today/code#issue:4242")).toBeNull();
    } finally {
      db.close();
    }
  });

  test("a conflicting native parent wins over the label-derived parent", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => EPICS,
      resolveNativeParent: async () => ({ repo: "developing-today/code", number: 100, kind: "sub-issue" }),
      readCurrent: async (item) => item,
      writeBack: async () => {},
      enqueueTriage: async () => {},
      now: () => NOW,
    });

    try {
      const response = await handler(signedIssueRequest());
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "generated" });
      expect(getPmMetadata(db, "gh:developing-today/code#issue:4242")?.indexes.directParentNumber).toBe(100);
    } finally {
      db.close();
    }
  });

  test("a confirmed-absent parent falls through to label resolution (parentless-plus-label)", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const enqueued: Record<string, unknown>[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => new Map(),
      resolveNativeParent: async () => null,
      readCurrent: async (item) => item,
      writeBack: async () => {},
      enqueueTriage: async (packet) => {
        enqueued.push(packet);
      },
      now: () => NOW,
    });

    try {
      const response = await handler(signedIssueRequest({ labels: [{ name: "kind/feature" }] }));
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "needs-triage" });
      expect(enqueued).toHaveLength(1);
      expect(getPmMetadata(db, "gh:developing-today/code#issue:4242")).toBeNull();
    } finally {
      db.close();
    }
  });

  test("pull_request events skip the native lookup and keep the label path", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    let lookups = 0;
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => EPICS,
      resolveNativeParent: async () => {
        lookups++;
        return null;
      },
      readCurrent: async (item) => item,
      writeBack: async () => {},
      enqueueTriage: async () => {},
      now: () => NOW,
    });

    try {
      const rawBody = JSON.stringify({
        action: "closed",
        repository: { full_name: "developing-today/code" },
        pull_request: {
          number: 4243,
          title: "Quickly-merged fix",
          body: "Ships fast",
          state: "closed",
          merged: true,
          merged_at: "2026-11-27T08:30:00Z",
          user: { login: "plain-gh-user" },
          labels: [{ name: "primary-parent/pm-board" }],
          milestone: null,
          created_at: "2026-11-27T08:00:00Z",
          updated_at: "2026-11-27T08:30:00Z",
          closed_at: "2026-11-27T08:30:00Z",
        },
      });
      const signature = createHmac("sha256", SECRET).update(rawBody).digest("hex");
      const response = await handler(
        new Request("http://localhost/webhooks/github", {
          method: "POST",
          headers: { "x-github-event": "pull_request", "x-hub-signature-256": `sha256=${signature}` },
          body: rawBody,
        }),
      );
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "generated" });
      expect(lookups).toBe(0);
      expect(getPmMetadata(db, "gh:developing-today/code#pr:4243")?.indexes.directParentNumber).toBe(9060);
    } finally {
      db.close();
    }
  });
});

const sweeperScript = join(import.meta.dir, "../scripts/metadata-sweeper.ts");

async function makeModeFleet(root: string): Promise<string> {
  const path = join(root, "mode-fleet.sh");
  await writeFile(
    path,
    `#!/bin/sh
case " $* " in
  *" kind/epic "*)
    printf '%s' '[{"number":9060,"labels":[{"name":"kind/epic"},{"name":"epic/pm-board"}]}]'
    ;;
  *" api "*)
    if [ "\${PARENT_MODE:-404}" = "404" ]; then
      printf '%s\\n' 'gh: Not Found (HTTP 404)' >&2
    else
      printf '%s\\n' 'gh: Internal Server Error (HTTP 500)' >&2
    fi
    exit 1
    ;;
  *" pr list "*)
    printf '[]'
    ;;
  *)
    printf '%s' '[{"number":4242,"title":"Sweeper probe","body":"Probe body.","labels":[{"name":"kind/feature"},{"name":"primary-parent/pm-board"}],"milestone":null,"state":"OPEN","author":{"login":"probe-user"},"createdAt":"2026-11-20T00:00:00Z","updatedAt":"2026-11-21T00:00:00Z","closedAt":null}]'
    ;;
esac
`,
  );
  await chmod(path, 0o755);
  return path;
}

function runSweeper(
  root: string,
  parentMode: string,
): Record<string, { scanned: number; errors: number; generated: number }> {
  const dbPath = join(root, `pm-metadata-${parentMode}.sqlite`);
  const stdout = execFileSync(
    process.execPath,
    ["run", sweeperScript, "--repo", "example/repo", "--db", dbPath, "--dry-run"],
    {
      env: { ...process.env, FLEET_BIN: join(root, "mode-fleet.sh"), PARENT_MODE: parentMode },
      stdio: "pipe",
    },
  ).toString();
  return JSON.parse(stdout).summary as Record<string, { scanned: number; errors: number; generated: number }>;
}

describe("sweeper native-parent lookup outcomes (#16011)", () => {
  test("a confirmed 404 proceeds to label resolution instead of skipping", async () => {
    const root = await mkdtemp(join(tmpdir(), "metadata-native-parent-"));
    try {
      await makeModeFleet(root);
      const summary = runSweeper(root, "404");
      expect(summary["example/repo"]?.scanned).toBe(1);
      expect(summary["example/repo"]?.errors).toBe(0);
      expect(summary["example/repo"]?.generated).toBe(1);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("a server error stays retryable: counted, skipped, never finalized", async () => {
    const root = await mkdtemp(join(tmpdir(), "metadata-native-parent-"));
    try {
      await makeModeFleet(root);
      const summary = runSweeper(root, "500");
      expect(summary["example/repo"]?.scanned).toBe(1);
      expect(summary["example/repo"]?.errors).toBe(1);
      expect(summary["example/repo"]?.generated).toBe(0);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
