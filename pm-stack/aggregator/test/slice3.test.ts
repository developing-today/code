import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { CommentsStore, validateNewComment } from "../src/comments.ts";
import { dnsSectionFromRegister, parseDnsRegister, summarizeDns } from "../src/dns.ts";
import type { SessionFeedEntry } from "../src/model.ts";
import { PAGES } from "../src/pages.ts";
import { agentIdentityOf, mergeRecords, parseLaunchRecords, recordFromSessionEntry } from "../src/registry.ts";
import { renderBoard, renderDns, renderPages, renderSessions } from "../src/render.ts";
import { FIXTURE_BOARD, FIXTURE_DNS, FIXTURE_ITEMS, FIXTURE_REGISTRY, fixtureSource } from "../src/sources/fixtures.ts";

// --- DNS register parsing (#906 §1, register format from #902) --------------

const REGISTER_MD = `# DNS / Domain Register

Prose with a | pipe | table-ish line that is not the register.

| Domain | Registrar | DNS hosted at | Next renewal | Auto-renew | Renewal price (list) | Prior-year cost | Where it's used | Plan / status | Review? |
|---|---|---|---|---|---|---|---|---|---|
| later.dev | Porkbun | Porkbun | 2027-05-01 | ON | $12.87 | unknown | No public-facing use found | Unused — purpose unknown | REVIEW |
| first.de | Porkbun | Porkbun | 2027-02-09 | ON | $4.07 | unknown | No public-facing use found | Unused — \`0x111\` family (earmark inferred) | REVIEW |
| active.example | Porkbun | **Cloudflare** | 2027-02-24 | ON | $29.35 | unknown | Active, recorded: something real | **Active** — fleet infrastructure | Settled — in use |
| noprice.example | Porkbun | Porkbun | 2028-01-01 | unknown | — | unknown | Parked | Parked — namesake brand (earmark inferred) | REVIEW |
`;

describe("dns register parser", () => {
  test("parses the register table, renewal-sorted, bold cells cleaned", () => {
    const entries = parseDnsRegister(REGISTER_MD);
    expect(entries.length).toBe(4);
    expect(entries[0].domain).toBe("first.de");
    expect(entries[0].priceUsd).toBe(4.07);
    const active = entries.find((e) => e.domain === "active.example");
    expect(active?.dnsHost).toBe("Cloudflare");
    expect(active?.plan).toBe("Active — fleet infrastructure");
    expect(active?.status).toBe("active");
    expect(entries.find((e) => e.domain === "noprice.example")?.priceUsd).toBeNull();
  });

  test("classifies active / earmarked / unused from the plan cell", () => {
    const entries = parseDnsRegister(REGISTER_MD);
    const byStatus = (s: string) => entries.filter((e) => e.status === s).map((e) => e.domain);
    expect(byStatus("active")).toEqual(["active.example"]);
    expect(byStatus("earmarked").sort()).toEqual(["first.de", "noprice.example"]);
    expect(byStatus("unused")).toEqual(["later.dev"]);
  });

  test("section totals + summary views", () => {
    const section = dnsSectionFromRegister(REGISTER_MD, "2026-10-25T00:00:00Z", "test");
    expect(section).not.toBeNull();
    expect(section?.totalAnnualListUsd).toBe(46.29);
    const summary = summarizeDns(section!, new Date("2026-12-28T00:00:00Z"), 45);
    expect(summary.nextRenewal?.domain).toBe("first.de");
    expect(summary.renewingSoon.map((e) => e.domain)).toEqual(["first.de"]);
  });

  test("summary ages renewals by the Chicago day, not UTC (#1684)", () => {
    // 2026-10-31T01:30:00Z is 20:30 CDT on 2026-10-30 in Chicago:
    // the UTC day has already rolled over, the Chicago day has not.
    // With the UTC derivation the domain renewing "today" (Chicago)
    // was skipped by nextRenewal and dropped from renewingSoon.
    const entry = (domain: string, renewal: string) => ({
      domain,
      registrar: "Porkbun",
      dnsHost: "Porkbun",
      renewal,
      autoRenew: "ON",
      priceUsd: 10,
      priorYearCost: "unknown",
      used: "test",
      plan: "Active — test",
      review: "",
      status: "active" as const,
    });
    const section = {
      generatedAt: "2026-10-31T01:30:00Z",
      source: "test",
      totalAnnualListUsd: 20,
      entries: [entry("today.dev", "2026-10-30"), entry("tomorrow.dev", "2026-10-31")],
    };
    const summary = summarizeDns(section, new Date("2026-10-31T01:30:00Z"), 45);
    expect(summary.nextRenewal?.domain).toBe("today.dev");
    expect(summary.renewingSoon.map((e) => e.domain)).toEqual(["today.dev", "tomorrow.dev"]);
  });

  test("summary Chicago day holds across the spring DST boundary (#1684)", () => {
    // 2026-03-08T01:30:00Z is 19:30 CST on 2026-03-07 — the evening
    // before the spring-forward. Chicago "today" is still 03-07,
    // and the 45-day horizon is a calendar count (addDaysYmd), so
    // the cutoff lands on 2026-04-21 regardless of the lost hour.
    const entry = (domain: string, renewal: string) => ({
      domain,
      registrar: "Porkbun",
      dnsHost: "Porkbun",
      renewal,
      autoRenew: "ON",
      priceUsd: 10,
      priorYearCost: "unknown",
      used: "test",
      plan: "Active — test",
      review: "",
      status: "active" as const,
    });
    const section = {
      generatedAt: "2026-03-08T01:30:00Z",
      source: "test",
      totalAnnualListUsd: 30,
      entries: [entry("today.dev", "2026-03-07"), entry("cutoff.dev", "2026-04-21"), entry("beyond.dev", "2026-04-22")],
    };
    const summary = summarizeDns(section, new Date("2026-03-08T01:30:00Z"), 45);
    expect(summary.nextRenewal?.domain).toBe("today.dev");
    expect(summary.renewingSoon.map((e) => e.domain)).toEqual(["today.dev", "cutoff.dev"]);
  });

  test("a file without the register table parses to nothing (fail-soft)", () => {
    expect(parseDnsRegister("# nothing here\n")).toEqual([]);
    expect(dnsSectionFromRegister("# nothing\n", "2026-10-25T00:00:00Z", "test")).toBeNull();
  });
});

// --- Identity registry (#906 §3) ---------------------------------------------

const entry = (over: Partial<SessionFeedEntry>): SessionFeedEntry => ({
  sessionId: "ses_x",
  source: "opencode-sandbox",
  lastActivityAt: "2026-10-25T15:00:00Z",
  ...over,
});

describe("session registry", () => {
  test("records carry identity, parent, tokens, and task refs from the title", () => {
    const rec = recordFromSessionEntry(
      entry({
        parentId: "ses_parent",
        agent: "explore",
        title: "Work on https://github.com/developing-today/code/issues/906 today",
        outcome: null,
        activeToolCalls: 1,
      }),
      1234,
    );
    expect(rec.agentId).toBe("opencode:explore");
    expect(rec.parentId).toBe("ses_parent");
    expect(rec.tokensTotal).toBe(1234);
    expect(rec.taskRefs).toEqual(["developing-today/code#906"]);
    expect(rec.status).toBe("in-progress");
    expect(agentIdentityOf(entry({ source: "jules" })).agentId).toBe("jules");
  });

  test("launch records parse; torn lines are skipped", () => {
    const records = parseLaunchRecords(
      [
        JSON.stringify({ id: "claude-1", label: "Builder", taskRefs: ["developing-today/code#906"] }),
        "{torn",
        JSON.stringify({ id: "claude-2", label: "Reviewer", parentId: "claude-1" }),
      ].join("\n"),
    );
    expect(records.length).toBe(2);
    expect(records[0].source).toBe("claude-code");
    expect(records[1].parentId).toBe("claude-1");
  });

  test("merge keeps the record, maxes cumulative tokens, unions task refs", () => {
    const a = recordFromSessionEntry(entry({ sessionId: "ses_m", title: "t" }), 100);
    const b = { ...a, tokensTotal: 250, taskRefs: ["developing-today/code#1"], lastSeenAt: "2026-10-25T16:00:00Z" };
    const merged = mergeRecords([a], [b]);
    expect(merged.length).toBe(1);
    expect(merged[0].tokensTotal).toBe(250);
    expect(merged[0].taskRefs).toEqual(["developing-today/code#1"]);
    // A later merge with fewer tokens never lowers the cumulative total.
    expect(mergeRecords(merged, [a])[0].tokensTotal).toBe(250);
  });

  test("sparse re-read never wipes stored fields (#1560)", () => {
    const full = recordFromSessionEntry(
      entry({
        sessionId: "ses_full",
        title: "Full title",
        model: "opus-4-1",
        directory: "/home/user/git/code",
        parentId: "ses_parent",
        createdAt: "2026-10-25T14:00:00Z",
        lastActivityAt: "2026-10-25T15:00:00Z",
      }),
      500,
    );
    // A sparse feed entry: no title/model/directory/parentId stated.
    const sparse = recordFromSessionEntry(entry({ sessionId: "ses_full" }));
    expect(sparse.model).toBeUndefined();
    expect(sparse.parentId).toBeNull();
    const m = mergeRecords([full], [sparse])[0];
    expect(m.model).toBe("opus-4-1");
    expect(m.title).toBe("Full title");
    expect(m.directory).toBe("/home/user/git/code");
    expect(m.parentId).toBe("ses_parent");
    // The sparse label is just the session-id fallback; the stored one survives.
    expect(m.label).toBe("Full title");
    // Present incoming values still win: a stated model replaces, a stated parent replaces.
    expect(mergeRecords([full], [{ ...sparse, model: "sonnet" }])[0].model).toBe("sonnet");
    expect(mergeRecords([full], [{ ...sparse, parentId: "ses_new_parent" }])[0].parentId).toBe("ses_new_parent");
  });
});

// --- Comments store (#906 §2) ------------------------------------------------

describe("dashboard comments", () => {
  const dir = mkdtempSync(join(tmpdir(), "agg-comments-"));
  const file = join(dir, "comments.json");
  const drew = { kind: "drew" as const, id: "drew", label: "Drew" };
  const agent = { kind: "agent" as const, id: "ses_a", label: "OpenCode · main (laptop)", sessionId: "ses_a" };

  test("add/list/counts, triage, github result, persistence", async () => {
    const store = new CommentsStore(file);
    const note = await store.add({
      objectRef: "gh:developing-today/code#906",
      kind: "note",
      author: drew,
      body: "a note",
    });
    const msg = await store.add({
      objectRef: "gh:developing-today/code#906",
      kind: "message",
      author: drew,
      body: "please do X",
    });
    const gh = await store.add({
      objectRef: "gh:developing-today/code#906",
      kind: "github",
      author: agent,
      body: "posting this",
      githubRef: "developing-today/code#906",
    });
    expect(note.triageState).toBeUndefined();
    expect(msg.triageState).toBe("new");
    expect(gh.githubState).toBe("pending");
    expect((await store.counts()).get("gh:developing-today/code#906")).toBe(3);
    expect(await store.markTriaged([msg.id])).toBe(1);
    expect(await store.markTriaged([msg.id])).toBe(0); // idempotent
    const posted = await store.markGithubResult(gh.id, { url: "https://github.com/x#issuecomment-1" });
    expect(posted?.githubState).toBe("posted");
    // A fresh store over the same file sees the same state.
    const reloaded = new CommentsStore(file);
    expect((await reloaded.all()).length).toBe(3);
    expect((await reloaded.all()).find((c) => c.id === gh.id)?.githubUrl).toContain("issuecomment-1");
  });

  test("github result reports: empty refused, posted terminal, repeats idempotent (#1686)", async () => {
    const dir2 = mkdtempSync(join(tmpdir(), "agg-comments-result-"));
    const store = new CommentsStore(join(dir2, "comments.json"));
    const addGh = () =>
      store.add({
        objectRef: "gh:developing-today/code#906",
        kind: "github",
        author: agent,
        body: "posting this",
        githubRef: "developing-today/code#906",
      });

    // Empty report on a pending comment: refused, state unchanged.
    const pending = await addGh();
    await expect(store.markGithubResult(pending.id, {})).rejects.toThrow("url or an error");
    expect((await store.all()).find((c) => c.id === pending.id)?.githubState).toBe("pending");

    // A report carrying both url and error: refused, unchanged.
    await expect(store.markGithubResult(pending.id, { url: "https://github.com/x#1", error: "boom" })).rejects.toThrow(
      "both",
    );
    expect((await store.all()).find((c) => c.id === pending.id)?.githubState).toBe("pending");

    // Posted, then: an empty report and a later failure are both
    // refused (posted is terminal); the same url repeats as a no-op;
    // a different url conflicts.
    const posted = await addGh();
    await store.markGithubResult(posted.id, { url: "https://github.com/x#issuecomment-9" });
    await expect(store.markGithubResult(posted.id, {})).rejects.toThrow("url or an error");
    await expect(store.markGithubResult(posted.id, { error: "late failure" })).rejects.toThrow("already posted");
    const repeat = await store.markGithubResult(posted.id, {
      url: "https://github.com/x#issuecomment-9",
    });
    expect(repeat?.githubState).toBe("posted");
    await expect(store.markGithubResult(posted.id, { url: "https://github.com/x#issuecomment-10" })).rejects.toThrow(
      "different url",
    );
    const after = (await store.all()).find((c) => c.id === posted.id);
    expect(after?.githubState).toBe("posted");
    expect(after?.githubUrl).toBe("https://github.com/x#issuecomment-9");

    // Failed is not terminal against success: a retry that lands
    // moves failed -> posted; a repeat failure is a no-op.
    const retried = await addGh();
    await store.markGithubResult(retried.id, { error: "first attempt failed" });
    const again = await store.markGithubResult(retried.id, { error: "second attempt failed" });
    expect(again?.githubState).toBe("failed");
    const landed = await store.markGithubResult(retried.id, {
      url: "https://github.com/x#issuecomment-11",
    });
    expect(landed?.githubState).toBe("posted");
  });

  test("a repeat report after a failed persist still lands the result on disk (#1686)", async () => {
    // Codex P2 on PR 3062: markGithubResult flips the in-memory state
    // before persist() runs. If that write fails, a repeat report hit
    // the idempotent early return and acknowledged success while the
    // on-disk comment stayed pending — after a restart it would post
    // again and duplicate the GitHub comment. The repeat must flush
    // the state to disk before returning.
    const dir2 = mkdtempSync(join(tmpdir(), "agg-comments-flush-"));
    const file2 = join(dir2, "comments.json");
    const store2 = new CommentsStore(file2);
    const addGh2 = () =>
      store2.add({
        objectRef: "gh:developing-today/code#1686",
        kind: "github",
        author: agent,
        body: "posting this",
        githubRef: "developing-today/code#1686",
      });

    // Break the filesystem under the store: the store's directory is
    // replaced by a regular file, so mkdir/write inside persist fail.
    const breakFs = () => {
      renameSync(dir2, `${dir2}-moved`);
      writeFileSync(dir2, "not a directory");
    };
    const fixFs = () => {
      rmSync(dir2);
      renameSync(`${dir2}-moved`, dir2);
    };
    const onDisk = async (id: string) => (await new CommentsStore(file2).all()).find((c) => c.id === id);

    // posted path
    const ghP = await addGh2();
    breakFs();
    await expect(store2.markGithubResult(ghP.id, { url: "https://github.com/x#issuecomment-21" })).rejects.toThrow();
    fixFs();
    expect((await onDisk(ghP.id))?.githubState).toBe("pending"); // the failed write never landed
    const repeatP = await store2.markGithubResult(ghP.id, { url: "https://github.com/x#issuecomment-21" });
    expect(repeatP?.githubState).toBe("posted");
    expect((await onDisk(ghP.id))?.githubState).toBe("posted");
    expect((await onDisk(ghP.id))?.githubUrl).toBe("https://github.com/x#issuecomment-21");

    // failed path: same shape via the failed-state early return
    const ghF = await addGh2();
    breakFs();
    await expect(store2.markGithubResult(ghF.id, { error: "first attempt failed" })).rejects.toThrow();
    fixFs();
    expect((await onDisk(ghF.id))?.githubState).toBe("pending");
    const repeatF = await store2.markGithubResult(ghF.id, { error: "first attempt failed" });
    expect(repeatF?.githubState).toBe("failed");
    expect((await onDisk(ghF.id))?.githubState).toBe("failed");
  });

  test("validation rejects anonymous authors, empty bodies, targetless github comments", () => {
    expect(validateNewComment({ objectRef: "x", kind: "note", body: "hi" }).ok).toBe(false);
    expect(validateNewComment({ objectRef: "x", kind: "note", author: drew, body: "  " }).ok).toBe(false);
    expect(validateNewComment({ objectRef: "x", kind: "github", author: drew, body: "hi" }).ok).toBe(false);
    expect(
      validateNewComment({
        objectRef: "x",
        kind: "github",
        author: drew,
        body: "hi",
        githubRef: "developing-today/code#906",
      }).ok,
    ).toBe(true);
    // Drew's authorship from a raw string author is never accepted.
    expect(validateNewComment({ objectRef: "x", kind: "note", author: "drew", body: "hi" }).ok).toBe(false);
  });
});

// --- Pages manifest + renders --------------------------------------------------

describe("pages manifest + slice-3 renders", () => {
  test("manifest registers dns + sessions; routes unique", () => {
    const routes = PAGES.map((p) => p.route);
    expect(new Set(routes).size).toBe(routes.length);
    expect(PAGES.find((p) => p.id === "dns")?.route).toBe("/dns");
    expect(PAGES.find((p) => p.id === "sessions")?.api).toBe("/api/registry.json");
  });

  test("nav + dropdown render from the manifest", async () => {
    const src = fixtureSource();
    const refs = [...FIXTURE_ITEMS.values()].map((i) => i.ref);
    const html = renderBoard(FIXTURE_BOARD, await src.items(refs), src.provenance());
    expect(html).toContain("Pages ▾");
    expect(html).toContain('value="/dns"');
    expect(html).toContain('value="/sessions"');
    expect(html).toContain("DNS register");
  });

  test("dns page renders the three views + next renewal", () => {
    const html = renderDns(FIXTURE_DNS, fixtureSource().provenance());
    expect(html).toContain("Next renewal");
    expect(html).toContain("1110x.de");
    expect(html).toContain("Unused but earmarked");
    expect(html).toContain("Completely unused");
    expect(html).toContain("grandnag.us");
    expect(html).toContain("security.cab");
  });

  test("dns page without a section says so honestly", () => {
    const html = renderDns(null, fixtureSource().provenance());
    expect(html).toContain("has not reached the dashboard yet");
  });

  test("sessions page renders identities, parents, tokens", () => {
    const html = renderSessions(FIXTURE_REGISTRY, fixtureSource().provenance());
    expect(html).toContain("identity registry");
    expect(html).toContain("Muse");
    expect(html).toContain("claude-2026-10-25-slice2-review");
    expect(html).toContain("tok");
  });

  test("pages page lists the manifest", () => {
    const html = renderPages(fixtureSource().provenance());
    expect(html).toContain("/api/registry.json");
    expect(html).toContain("All pages");
  });
});

// --- OpenCode session-identity tool ---------------------------------------------

describe("opencode session-identity tool", () => {
  test("payload carries session id, parent, agent", async () => {
    mock.module("@opencode-ai/plugin", () => ({ tool: (def: unknown) => def }));
    const mod = await import("../opencode-tools/session-identity.ts");
    const payload = mod.identityPayload(
      { sessionID: "ses_1", agent: "explore", directory: "/work", worktree: "/work" },
      "ses_0",
    );
    expect(payload).toEqual({
      sessionId: "ses_1",
      parentId: "ses_0",
      agent: "explore",
      directory: "/work",
      worktree: "/work",
    });
    const def = mod.default as { execute: (a: object, c: object) => Promise<string> };
    const out = JSON.parse(await def.execute({}, { sessionID: "ses_9", agent: "main" }));
    expect(out.sessionId).toBe("ses_9");
    expect(out.parentId).toBeNull(); // no server env in tests — fail-soft
  });
});

// --- Boot: slice-3 routes over HTTP ----------------------------------------------

let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;
const tmp = mkdtempSync(join(tmpdir(), "agg-slice3-"));
const annotationsFile = join(tmp, "roadmap-annotations.json");

beforeAll(async () => {
  port = 5200 + Math.floor(Math.random() * 800);
  const commentsFile = join(tmp, "comments.json");
  writeFileSync(commentsFile, JSON.stringify({ version: 1, comments: [] }));
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_COMMENTS_FILE: commentsFile,
      PM_AGG_ROADMAP_ANNOTATIONS_FILE: annotationsFile,
      PM_AGG_REGISTRY_FILE: join(tmp, "registry.json"),
    },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("slice-3 routes", () => {
  test("dns + sessions + pages routes serve", async () => {
    const dns = await (await fetch(`http://127.0.0.1:${port}/dns`)).text();
    expect(dns).toContain("DNS / domain register");
    const dnsJson = (await (await fetch(`http://127.0.0.1:${port}/api/dns.json`)).json()) as {
      entries: unknown[];
      totalAnnualListUsd: number;
    };
    expect(dnsJson.entries.length).toBe(4);
    expect(dnsJson.totalAnnualListUsd).toBe(595.62);
    const sessions = await (await fetch(`http://127.0.0.1:${port}/sessions`)).text();
    expect(sessions).toContain("identity registry");
    // window=all: fixture last-activity is a fixed date; the default 7-day window ages it out and fails this test on later run dates.
    const registry = (await (await fetch(`http://127.0.0.1:${port}/api/registry.json?window=all`)).json()) as {
      records: { source: string }[];
      identities: { kind: string }[];
    };
    expect(registry.records.some((r) => r.source === "claude-code")).toBe(true);
    expect(registry.identities.some((i) => i.kind === "drew")).toBe(true);
    const pages = await (await fetch(`http://127.0.0.1:${port}/pages`)).text();
    expect(pages).toContain("All pages");
  });

  test("comments round-trip: post both kinds + github, triage, result", async () => {
    const post = (body: unknown) =>
      fetch(`http://127.0.0.1:${port}/api/comments`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
    const author = { kind: "drew", id: "drew", label: "Drew" };
    const ref = "gh:developing-today/code#906";
    const note = await post({ objectRef: ref, kind: "note", author, body: "a plain note" });
    expect(note.status).toBe(201);
    const msg = await post({ objectRef: ref, kind: "message", author, body: "Muse: please check X" });
    const msgBody = (await msg.json()) as { comment: { id: string } };
    const ghRes = await post({
      objectRef: ref,
      kind: "github",
      author,
      body: "for the issue",
      githubRef: "developing-today/code#906",
    });
    const ghBody = (await ghRes.json()) as { comment: { id: string } };

    const list = (await (await fetch(`http://127.0.0.1:${port}/api/comments.json?kind=message&state=new`)).json()) as {
      comments: unknown[];
      pending: { messages: number; github: number };
    };
    expect(list.comments.length).toBe(1);
    expect(list.pending.messages).toBe(1);
    expect(list.pending.github).toBe(1);

    const triage = (await (
      await fetch(`http://127.0.0.1:${port}/api/comments/triage`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ ids: [msgBody.comment.id] }),
      })
    ).json()) as { triaged: number };
    expect(triage.triaged).toBe(1);

    const result = (await (
      await fetch(`http://127.0.0.1:${port}/api/comments/github-result`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          id: ghBody.comment.id,
          url: "https://github.com/developing-today/code/issues/906#issuecomment-1",
        }),
      })
    ).json()) as { comment: { githubState: string } };
    expect(result.comment.githubState).toBe("posted");

    // Anonymous authorship is refused.
    const anon = await post({ objectRef: ref, kind: "note", body: "no author" });
    expect(anon.status).toBe(400);

    // The discuss page renders the thread + the form.
    const discuss = await (await fetch(`http://127.0.0.1:${port}/discuss/${encodeURIComponent(ref)}`)).text();
    expect(discuss).toContain("a plain note");
    expect(discuss).toContain("message to Muse");
    expect(discuss).toContain("Post as GitHub comment");
  });

  test("item page carries the dashboard comments form + kinds explainer", async () => {
    const board = (await (await fetch(`http://127.0.0.1:${port}/api/board.json`)).json()) as {
      board: { id: string }[];
    };
    const id = encodeURIComponent(board.board[0].id);
    const html = await (await fetch(`http://127.0.0.1:${port}/item/${id}`)).text();
    expect(html).toContain("Dashboard comments");
    expect(html).toContain("cform");
  });

  test("roadmap annotations round-trip through API, timeline, drill-down, and JSON; aliases survive reshuffles", async () => {
    const roadmap = (await (await fetch(`http://127.0.0.1:${port}/api/roadmap.json`)).json()) as {
      rows: { subject: string; annotations: { body: string }[] }[];
    };
    expect(roadmap.rows.length).toBeGreaterThan(1);
    const stableRef = roadmap.rows[0].subject;
    const reshuffledRef = roadmap.rows[1].subject;
    const post = (objectRef: string, body: string) =>
      fetch(`http://127.0.0.1:${port}/api/roadmap/annotations`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          objectRef,
          kind: "decision",
          date: "2026-10-12",
          author: { kind: "drew", id: "drew", label: "Drew" },
          body,
        }),
      });

    expect((await post(stableRef, "Moved after review")).status).toBe(201);
    const alias = await fetch(`http://127.0.0.1:${port}/api/roadmap/annotations/alias`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ aliasRef: reshuffledRef, stableRef }),
    });
    expect(alias.status).toBe(200);
    expect((await post(reshuffledRef, "Follow-up under the new row id")).status).toBe(201);

    const json = (await (await fetch(`http://127.0.0.1:${port}/api/roadmap.json`)).json()) as {
      rows: { subject: string; annotations: { body: string; date: string; kind: string; author: { label: string } }[] }[];
      annotationAliases: Record<string, string>;
    };
    const reshuffledRow = json.rows.find((row) => row.subject === reshuffledRef)!;
    expect(reshuffledRow.annotations.map((annotation) => annotation.body)).toEqual([
      "Moved after review",
      "Follow-up under the new row id",
    ]);
    expect(reshuffledRow.annotations[0]).toMatchObject({ date: "2026-10-12", kind: "decision", author: { label: "Drew" } });
    expect(json.annotationAliases[reshuffledRef]).toBe(stableRef);

    const detail = await (await fetch(`http://127.0.0.1:${port}/item/${encodeURIComponent(reshuffledRef)}`)).text();
    expect(detail).toContain("Roadmap annotations");
    expect(detail).toContain("Follow-up under the new row id");
    expect(detail).toContain("name=\"date\"");
    const timeline = await (await fetch(`http://127.0.0.1:${port}/roadmap`)).text();
    expect(timeline).toContain("Moved after review");
  });
});
