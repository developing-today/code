import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { authorOptionsHtml, cardCommentHtml, commentFormHtml, statusRequestHtml } from "../src/render.ts";
import { STANDING_IDENTITIES, type AuthorIdentity } from "../src/registry.ts";
import { FIXTURE_REGISTRY } from "../src/sources/fixtures.ts";

// #1179: list pages were quadratic in the session-registry size --
// every card's author picker inlined one <option> per registry
// session (~700 sessions x ~351 cards on /program = 44 MB of HTML).
// Pickers now render the standing identities only and hydrate the
// full list on demand from /api/author-identities. These tests pin
// that: no session identity is ever inlined as a picker option, and
// the endpoint still serves the full list.

const sessions: AuthorIdentity[] = Array.from({ length: 300 }, (_, i) => ({
  id: `ses_fake${i}`,
  label: `OpenCode · build (laptop) · Fake session ${i} with a reasonably long task title`,
  kind: "agent" as const,
  sessionId: `ses_fake${i}`,
}));
const identities: AuthorIdentity[] = [...STANDING_IDENTITIES.map((i) => ({ ...i })), ...sessions];

describe("author picker options (#1179)", () => {
  test("authorOptionsHtml renders standing identities only", () => {
    const html = authorOptionsHtml(identities);
    expect(html).toContain('<option value="drew|drew|" selected>Drew</option>');
    expect(html).toContain("Muse");
    expect(html).toContain("Orchestrator (orchestration-pass) (agent)");
    expect(html).not.toContain("ses_fake");
    expect(html.match(/<option/g)?.length).toBe(STANDING_IDENTITIES.length);
  });

  test("the actbox picker is marked for on-demand hydration and carries no session options", () => {
    const html = statusRequestHtml("gh:developing-today/code#1179", "developing-today/code#1179", "Todo", {
      identities,
    });
    expect(html).toContain('<select name="author" data-author-picker>');
    expect(html).toContain("Act as");
    expect(html).not.toContain("ses_fake");
  });

  test("comment forms (thread + card box) use the same standing-only picker", () => {
    const thread = commentFormHtml("gh:developing-today/code#1179", { identities });
    const card = cardCommentHtml("gh:developing-today/code#1179", undefined, [], { identities });
    for (const html of [thread, card]) {
      expect(html).toContain('<select name="author" data-author-picker>');
      expect(html).not.toContain("ses_fake");
    }
  });

  test("200 cards with 300 registered sessions stay small (no per-card option duplication)", () => {
    const html = Array.from({ length: 200 }, (_, i) =>
      statusRequestHtml(`gh:developing-today/code#${1000 + i}`, `developing-today/code#${1000 + i}`, "Todo", {
        identities,
      }),
    ).join("");
    // Pre-fix this was ~200 x 300 options ~= 9 MB; standing-only
    // pickers keep it at the forms' own weight.
    expect(new TextEncoder().encode(html).length).toBeLessThan(400_000);
    expect(html.match(/<option/g)?.length).toBeLessThan(200 * (STANDING_IDENTITIES.length + 4) + 10);
  });
});

let proc: Subprocess | undefined;
let base = "";

beforeAll(async () => {
  const dir = await mkdtemp(join(tmpdir(), "pm-author-picker-1179-"));
  const port = 5900 + Math.floor(Math.random() * 800);
  base = `http://127.0.0.1:${port}`;
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: join(import.meta.dir, ".."),
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_FEED_FILE: join(dir, "no-feed.json"),
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
      PM_AGG_ACTIONS_FILE: join(dir, "actions.json"),
      PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
      PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
      PM_AGG_QUOTA_VIEW: join(dir, "no-quota-view.md"),
      PM_AGG_QUOTA_REGISTRY: join(dir, "no-quota-registry.md"),
      PM_PROJECTOR_URL: "off",
      KANEO_ADMIN_EMAIL: "",
      KANEO_ADMIN_PASSWORD: "",
      GRIST_API_KEY: "",
    },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 20_000;
  let up = false;
  while (Date.now() < deadline && !up) {
    try {
      const res = await fetch(`${base}/api/health`);
      up = res.ok;
    } catch {
      /* not up yet */
    }
    if (!up) await Bun.sleep(150);
  }
  expect(up).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("/api/author-identities (#1179)", () => {
  test("serves the full standing + session identity list on demand", async () => {
    const res = await fetch(`${base}/api/author-identities`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { identities: AuthorIdentity[] };
    expect(body.identities.length).toBe(STANDING_IDENTITIES.length + FIXTURE_REGISTRY.length);
    expect(body.identities.some((i) => i.sessionId === "claude-2026-10-25-slice2-review")).toBe(true);
    expect(body.identities[0]?.id).toBe("drew");
  });

  test("/sessions renders pickers standing-only, with the hydration hook in the page script", async () => {
    const res = await fetch(`${base}/sessions`);
    expect(res.status).toBe(200);
    const html = await res.text();
    expect(html).toContain('data-author-picker');
    expect(html).toContain("/api/author-identities");
    // A registry session must never appear as an inlined picker option.
    expect(html).not.toContain('<option value="agent|claude-2026-10-25-slice2-review|');
  });
});
