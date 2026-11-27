import { createHmac } from "node:crypto";
import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { createPmMetadataStore, getPmMetadata, parsePmMetadata, upsertPmMetadata } from "../src/metadata-store.ts";
import { createMetadataWebhookHandler } from "../src/webhook-listener.ts";

const SECRET = "test-webhook-secret";
const NOW = "2026-11-23T10:00:00.000Z";

function signedRequest(payload: unknown, event = "issues", action = "opened", secret = SECRET): Request {
  const base = {
    action,
    repository: { full_name: "developing-today/code" },
    issue: {
      number: 4242,
      title: "Created outside Fleet",
      body: "Description",
      state: "open",
      user: { login: "plain-gh-user" },
      labels: [{ name: "kind/feature" }, { name: "primary-parent/pm-board" }],
      milestone: { title: "Q4", due_on: "2026-12-31T00:00:00Z" },
      created_at: "2026-11-22T12:00:00Z",
      updated_at: "2026-11-23T09:00:00Z",
      closed_at: null,
    },
  };
  const overrides = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
  const rawBody = JSON.stringify({
    ...base,
    ...overrides,
    issue:
      overrides.issue && typeof overrides.issue === "object"
        ? { ...base.issue, ...(overrides.issue as object) }
        : base.issue,
  });
  const signature = createHmac("sha256", secret).update(rawBody).digest("hex");
  return new Request("http://localhost/webhooks/github", {
    method: "POST",
    headers: { "x-github-event": event, "x-hub-signature-256": `sha256=${signature}` },
    body: rawBody,
  });
}

describe("GitHub metadata webhook listener", () => {
  test("signed issue event runs the shared check, stores the row, and writes generated metadata back", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const writes: { number: number; body: string }[] = [];
    const enqueued: Record<string, unknown>[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      resolveNativeParent: async () => null,
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async (packet) => {
        enqueued.push(packet);
      },
      readCurrent: async (item) => item,
      now: () => NOW,
    });

    try {
      const response = await handler(signedRequest({}));
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "generated" });
      expect(getPmMetadata(db, "gh:developing-today/code#issue:4242")?.indexes.directParentNumber).toBe(9060);
      expect(writes).toHaveLength(1);
      expect(writes[0]?.body).toContain("<!-- pm-metadata:start -->");
      expect(enqueued).toHaveLength(0);
    } finally {
      db.close();
    }
  });

  test("invalid signatures are rejected before processing", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    let lookedUpEpics = false;
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      resolveNativeParent: async () => null,
      epicSlugs: async () => {
        lookedUpEpics = true;
        return new Map();
      },
      writeBack: async () => {},
      enqueueTriage: async () => {},
      readCurrent: async (item) => item,
    });

    try {
      const response = await handler(signedRequest({}, "issues", "opened", "wrong-secret"));
      expect(response.status).toBe(401);
      expect(lookedUpEpics).toBe(false);
      expect(getPmMetadata(db, "gh:developing-today/code#issue:4242")).toBeNull();
    } finally {
      db.close();
    }
  });

  test("unresolvable metadata is forwarded to triage and not stored", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const enqueued: Record<string, unknown>[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      resolveNativeParent: async () => null,
      epicSlugs: async () => new Map(),
      writeBack: async () => {},
      enqueueTriage: async (packet) => {
        enqueued.push(packet);
      },
      readCurrent: async (item) => item,
      now: () => NOW,
    });

    try {
      const response = await handler(
        signedRequest({
          issue: { ...({} as Record<string, unknown>), number: 4242, labels: [{ name: "kind/feature" }] },
        }),
      );
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "needs-triage" });
      expect(enqueued).toHaveLength(1);
      expect(enqueued[0]?.dedupe_key).toBe("repo-health:v1:triage:developing-today/code:issue:4242");
      expect(getPmMetadata(db, "gh:developing-today/code#issue:4242")).toBeNull();
    } finally {
      db.close();
    }
  });

  test("unsupported GitHub events are acknowledged without touching the store", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    let checked = false;
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      resolveNativeParent: async () => null,
      epicSlugs: async () => {
        checked = true;
        return new Map();
      },
      writeBack: async () => {},
      enqueueTriage: async () => {},
      readCurrent: async (item) => item,
    });

    try {
      const response = await handler(signedRequest({}, "push", "opened"));
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, ignored: true });
      expect(checked).toBe(false);
    } finally {
      db.close();
    }
  });

  test("pull_request events run the same shared check (retroactive completion path)", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const writes: { number: number; body: string }[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      resolveNativeParent: async () => null,
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async () => {},
      readCurrent: async (item) => item,
      now: () => NOW,
    });

    try {
      const prPayload = {
        action: "closed",
        repository: { full_name: "developing-today/code" },
        pull_request: {
          number: 4243,
          title: "Quickly-merged fix",
          body: "Ships fast",
          state: "closed",
          merged: true,
          merged_at: "2026-11-23T09:30:00Z",
          user: { login: "plain-gh-user" },
          labels: [{ name: "primary-parent/pm-board" }],
          milestone: null,
          created_at: "2026-11-23T09:00:00Z",
          updated_at: "2026-11-23T09:30:00Z",
          closed_at: "2026-11-23T09:30:00Z",
        },
      };
      const rawBody = JSON.stringify(prPayload);
      const signature = createHmac("sha256", SECRET).update(rawBody).digest("hex");
      const request = new Request("http://localhost/webhooks/github", {
        method: "POST",
        headers: { "x-github-event": "pull_request", "x-hub-signature-256": `sha256=${signature}` },
        body: rawBody,
      });
      const response = await handler(request);
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "generated" });
      expect(getPmMetadata(db, "gh:developing-today/code#pr:4243")?.indexes.directParentNumber).toBe(9060);
      expect(writes).toHaveLength(1);
    } finally {
      db.close();
    }
  });

  test("a body edit during the epic lookup is preserved by recomputing from the live item", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const writes: { number: number; body: string }[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      resolveNativeParent: async () => null,
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async () => {},
      // The live item changed while the epic catalog was awaited: the
      // handler must recompute from this data, not the event snapshot.
      readCurrent: async (item) => ({
        ...item,
        body: `${item.body ?? ""}\n\nUser edit during epic lookup.`,
        updatedAt: "2026-11-23T09:05:00Z",
      }),
      now: () => NOW,
    });

    try {
      const response = await handler(signedRequest({}));
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "generated" });
      // Oracle (#16012): the written body derives from the live item, so
      // the concurrent user edit survives instead of being overwritten by
      // the stale event-snapshot body.
      expect(writes).toHaveLength(1);
      expect(writes[0]?.body).toContain("User edit during epic lookup.");
      expect(writes[0]?.body).toContain("<!-- pm-metadata:start -->");
    } finally {
      db.close();
    }
  });

  test("a live item that lost its parent during the lookup is forwarded to triage, not written back", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const writes: { number: number; body: string }[] = [];
    const enqueued: Record<string, unknown>[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      resolveNativeParent: async () => null,
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async (packet) => {
        enqueued.push(packet);
      },
      readCurrent: async (item) => ({ ...item, labels: ["kind/feature"], updatedAt: "2026-11-23T09:05:00Z" }),
      now: () => NOW,
    });

    try {
      const response = await handler(signedRequest({}));
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "needs-triage" });
      expect(writes).toHaveLength(0);
      expect(enqueued).toHaveLength(1);
    } finally {
      db.close();
    }
  });

  test("a vanished live item skips the write-back while the stored row stands", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const writes: { number: number; body: string }[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      resolveNativeParent: async () => null,
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async () => {},
      readCurrent: async () => null,
      now: () => NOW,
    });

    try {
      const response = await handler(signedRequest({}));
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "generated" });
      expect(writes).toHaveLength(0);
      expect(getPmMetadata(db, "gh:developing-today/code#issue:4242")?.indexes.directParentNumber).toBe(9060);
    } finally {
      db.close();
    }
  });

  test("a failed live re-read is a retryable failure with no stale write", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const writes: { number: number; body: string }[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      resolveNativeParent: async () => null,
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async () => {},
      readCurrent: async () => {
        throw new Error("gh view failed");
      },
      now: () => NOW,
    });

    try {
      const response = await handler(signedRequest({}));
      expect(response.status).toBe(500);
      expect(writes).toHaveLength(0);
    } finally {
      db.close();
    }
  });

  test("a stale webhook snapshot does not roll back a newer stored row", async () => {
    const db = new Database(":memory:");
    createPmMetadataStore(db);
    const parsed = parsePmMetadata({
      pm_meta: 1,
      direct_parent: { repo: "developing-today/code", number: 3447, kind: "epic" },
      links: [{ target: { repo: "developing-today/code", number: 123 }, relation: "parent", status: "active" }],
    });
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    upsertPmMetadata(db, {
      artifactId: "gh:developing-today/code#issue:4242",
      repo: "developing-today/code",
      itemNumber: 4242,
      itemKind: "issue",
      sourceUrl: "https://github.com/developing-today/code/issues/4242",
      parsed: parsed.value,
      sourceUpdatedAt: "2026-11-23T09:30:00Z",
    });
    const writes: { number: number; body: string }[] = [];
    const handler = createMetadataWebhookHandler({
      secret: async () => SECRET,
      db,
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      resolveNativeParent: async () => null,
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async () => {},
      readCurrent: async (item) => item,
      now: () => NOW,
    });

    try {
      // The event snapshot (updated_at 09:00) predates the stored row
      // (09:30): the store guard must leave the newer row and links alone.
      const response = await handler(signedRequest({}));
      expect(response.status).toBe(202);
      const stored = getPmMetadata(db, "gh:developing-today/code#issue:4242");
      expect(stored?.sourceUpdatedAt).toBe("2026-11-23T09:30:00Z");
      expect(stored?.indexes.directParentNumber).toBe(3447);
      expect(stored?.indexes.links).toEqual(parsed.value.indexes.links);
    } finally {
      db.close();
    }
  });
});
