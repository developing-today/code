import { createHmac } from "node:crypto";
import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { createPmMetadataStore, getPmMetadata } from "../src/metadata-store.ts";
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
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async (packet) => {
        enqueued.push(packet);
      },
      now: () => NOW,
    });

    try {
      const response = await handler(signedRequest({}));
      expect(response.status).toBe(202);
      expect(await response.json()).toMatchObject({ accepted: true, outcome: "generated" });
      expect(getPmMetadata(db, "github:developing-today/code#issue/4242")?.indexes.directParentNumber).toBe(9060);
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
      epicSlugs: async () => {
        lookedUpEpics = true;
        return new Map();
      },
      writeBack: async () => {},
      enqueueTriage: async () => {},
    });

    try {
      const response = await handler(signedRequest({}, "issues", "opened", "wrong-secret"));
      expect(response.status).toBe(401);
      expect(lookedUpEpics).toBe(false);
      expect(getPmMetadata(db, "github:developing-today/code#issue/4242")).toBeNull();
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
      epicSlugs: async () => new Map(),
      writeBack: async () => {},
      enqueueTriage: async (packet) => {
        enqueued.push(packet);
      },
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
      expect(getPmMetadata(db, "github:developing-today/code#issue/4242")).toBeNull();
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
      epicSlugs: async () => {
        checked = true;
        return new Map();
      },
      writeBack: async () => {},
      enqueueTriage: async () => {},
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
      epicSlugs: async () => new Map([["pm-board", { repo: "developing-today/code", number: 9060 }]]),
      writeBack: async (item, body) => {
        writes.push({ number: item.number, body });
      },
      enqueueTriage: async () => {},
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
      expect(getPmMetadata(db, "github:developing-today/code#pr/4243")?.indexes.directParentNumber).toBe(9060);
      expect(writes).toHaveLength(1);
    } finally {
      db.close();
    }
  });
});
