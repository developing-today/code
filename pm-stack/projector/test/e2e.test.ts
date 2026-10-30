// End-to-end proof: feed a realistic webhook fixture sequence (push, issue,
// issue comment, PR, PR review, PR review comment, cross-repo issue) through
// the real server handler — including HMAC-SHA256 signature verification — and
// assert the resulting link-registry rows and read-model rows.
//
// Run: bun test test/e2e.test.ts
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync, readdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { migrate } from "../src/store";
import { handleRequest, type ServerEnv } from "../src/server";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURES = join(__dirname, "..", "fixtures");

const SECRET = "test-webhook-secret";
const TOKEN = "test-api-token";

let db: Client;
let dir: string;

async function hmacSha256Hex(body: Uint8Array, secret: string): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = new Uint8Array(
    await crypto.subtle.sign(
      "HMAC",
      key,
      body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength) as ArrayBuffer,
    ),
  );
  return Array.from(sig, (b) => b.toString(16).padStart(2, "0")).join("");
}

function webhookReq(body: string, event: string, signature: string, delivery: string): Request {
  return new Request("http://127.0.0.1/github/webhook", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-github-event": event,
      "x-hub-signature-256": `sha256=${signature}`,
      "x-github-delivery": delivery,
    },
    body,
  });
}

const EVENT_BY_FILE: Record<string, string> = {
  "01-push.json": "push",
  "02-issue-opened.json": "issues",
  "03-issue-comment.json": "issue_comment",
  "04-pr-opened.json": "pull_request",
  "05-pr-review.json": "pull_request_review",
  "06-pr-review-comment.json": "pull_request_review_comment",
  "07-openchamber-issue.json": "issues",
};

async function sendFixture(dbx: Client, env: ServerEnv, file: string, delivery: string): Promise<Response> {
  const body = readFileSync(join(FIXTURES, file), "utf8");
  const bytes = new TextEncoder().encode(body);
  const sig = await hmacSha256Hex(bytes, SECRET);
  return handleRequest(webhookReq(body, EVENT_BY_FILE[file], sig, delivery), dbx, env);
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-e2e-"));
  db = createClient({ url: `file:${join(dir, "e2e.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("end-to-end webhook + read model", () => {
  test("fixture sequence applies and produces registry + read-model rows", async () => {
    const env: ServerEnv = {
      GITHUB_WEBHOOK_SECRET: SECRET,
      PM_PROJECTOR_API_TOKEN: TOKEN,
      PM_REPOS: "developing-today/code,developing-today/openchamber",
    };

    const files = readdirSync(FIXTURES).sort();
    expect(files).toEqual([
      "01-push.json",
      "02-issue-opened.json",
      "03-issue-comment.json",
      "04-pr-opened.json",
      "05-pr-review.json",
      "06-pr-review-comment.json",
      "07-openchamber-issue.json",
    ]);

    const responses: Record<string, any> = {};
    for (let i = 0; i < files.length; i++) {
      const res = await sendFixture(db, env, files[i], `delivery-${i + 1}`);
      expect(res.status).toBe(200);
      responses[files[i]] = await res.json();
    }

    // Each webhook applied once.
    expect(responses["01-push.json"].status).toBe("applied");
    expect(responses["02-issue-opened.json"].refs).toEqual(["developing-today/code#754"]);

    // -- items: issue + PR + cross-repo issue -------------------------------
    const items = await db.execute("SELECT ref, kind, state, title, merged FROM pmp_items ORDER BY ref");
    const byRef = new Map(items.rows.map((r: any) => [r.ref, r]));
    expect(byRef.get("developing-today/code#754")).toMatchObject({ kind: "issue", state: "open" });
    const pr = byRef.get("developing-today/code#763");
    expect(pr).toMatchObject({ kind: "pr", state: "open", merged: 0 });
    expect(byRef.has("developing-today/openchamber#66")).toBe(true);

    // -- comments: unified stream incl. PR-interior reviews + line comments --
    const comments = await db.execute(
      "SELECT kind, ref, review_state, path, line FROM pmp_comments ORDER BY created_at",
    );
    const kinds = comments.rows.map((r: any) => r.kind);
    expect(kinds).toContain("issue_comment");
    expect(kinds).toContain("review");
    expect(kinds).toContain("review_comment");
    const review = comments.rows.find((r: any) => r.kind === "review") as any;
    expect(review.review_state).toBe("approved");
    expect(review.ref).toBe("developing-today/code#763");
    const lineComment = comments.rows.find((r: any) => r.kind === "review_comment") as any;
    expect(lineComment.path).toBe("doc/README.md");
    expect(lineComment.line).toBe(12);

    // -- pushes recorded ----------------------------------------------------
    const pushes = await db.execute("SELECT head_sha, ref FROM pmp_push_events");
    expect(pushes.rows.length).toBe(1);
    expect((pushes.rows[0] as any).head_sha).toBe("2222222222222222222222222222222222222222");

    // -- link registry: cross-repo reference (requirement #1) ----------------
    // A roadmap row in Grist links BOTH a code issue and an openchamber issue.
    await handleRequest(
      new Request("http://127.0.0.1/links", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` },
        body: JSON.stringify({
          subject: "grist:czYHYKgG9SHcm9CPwiKEq1/Roadmap/5",
          ref: "developing-today/code#754",
          relation: "implements",
        }),
      }),
      db,
      env,
    );
    await handleRequest(
      new Request("http://127.0.0.1/links", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` },
        body: JSON.stringify({
          subject: "grist:czYHYKgG9SHcm9CPwiKEq1/Roadmap/5",
          ref: "developing-today/openchamber#66",
          relation: "relates",
        }),
      }),
      db,
      env,
    );
    // A Kaneo task tracks the PR (cross-repo: task lives in Kaneo's code project).
    await handleRequest(
      new Request("http://127.0.0.1/links", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${TOKEN}` },
        body: JSON.stringify({ subject: "kaneo:code/CODE-107", ref: "developing-today/code#763", relation: "tracks" }),
      }),
      db,
      env,
    );

    // Unauthorized link write is rejected.
    const bad = await handleRequest(
      new Request("http://127.0.0.1/links", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ subject: "x", ref: "developing-today/code#1", relation: "relates" }),
      }),
      db,
      env,
    );
    expect(bad.status).toBe(401);

    // -- joined read model over HTTP (requirement #1 + #2 in one call) ------
    const subjectRes = await handleRequest(
      new Request("http://127.0.0.1/subjects/grist%3AczYHYKgG9SHcm9CPwiKEq1%2FRoadmap%2F5"),
      db,
      env,
    );
    expect(subjectRes.status).toBe(200);
    const view = (await subjectRes.json()) as any;
    expect(view.subject).toBe("grist:czYHYKgG9SHcm9CPwiKEq1/Roadmap/5");
    expect(view.links.map((l: any) => l.ref).sort()).toEqual([
      "developing-today/code#754",
      "developing-today/openchamber#66",
    ]);
    // The joined view carries live item state.
    const link754 = view.links.find((l: any) => l.ref === "developing-today/code#754");
    expect(link754.item.state).toBe("open");
    expect(link754.item.title).toBe("Kaneo sync trial issue (kept closed/in-Done)");
    // And unified comment activity across the linked items.
    expect(view.activity.some((c: any) => c.kind === "issue_comment")).toBe(true);

    // -- comment read API filtered by refs -----------------------------------
    const commentsRes = await handleRequest(
      new Request("http://127.0.0.1/comments?refs=developing-today/code%23763"),
      db,
      env,
    );
    const prComments = (await commentsRes.json()) as any;
    expect(prComments.comments.map((c: any) => c.kind).sort()).toEqual(["review", "review_comment"]);

    // -- duplicate webhook delivery is idempotent -----------------------------
    const dup = await sendFixture(db, env, "02-issue-opened.json", "delivery-2");
    expect(await dup.json()).toEqual({ status: "duplicate" });
  });
});
