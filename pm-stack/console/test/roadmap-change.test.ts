// Roadmap changes made by the platform itself (#13497, second half).
//
// Oracles: GitHub is a recorder, so what is asked of it is compared
// exactly. The pull-request oracle is the acceptance test of the
// issue: across every call the path makes, the only file written is
// the roadmap file — one contents write, on the branch the pull
// request is opened from — so the generated diff touches only that
// file. The file edited is in the shape of the real files in
// planning/roadmaps/, and what an edit did is read back with the
// console's own parseRoadmap, not by re-implementing it here.
import { describe, expect, test } from "bun:test";
import { readConfig } from "../src/config.ts";
import { Ops, type Run } from "../src/ops.ts";
import { CAPABILITIES, parseRoadmap, Plan } from "../src/plan.ts";
import { applyRoadmapChange, openRoadmapChangePr, roadmapChangeRefusal } from "../src/roadmap-change.ts";
import { Work } from "../src/work.ts";

const CFG = readConfig({ HOME: "/h" });
const REPO = "developing-today/code";
const START = Date.parse("2026-11-24T07:00:00Z");
const FLEET = CAPABILITIES.find((c) => c.id === "fleet") as (typeof CAPABILITIES)[number];
const FILE = "planning/roadmaps/fleet.md";

const SAMPLE = `# Roadmap: Fleet

Written from the programme's epics.

## Now

| Title | Status | Intent | Links |
| --- | --- | --- | --- |
| Alpha item | in-progress | The first thing. | #101 |
| Beta item | planned | The second thing. | #102 |

## Next

| Title | Status | Intent | Links |
| --- | --- | --- | --- |
| Gamma item | planned | The third thing. | #103 |

## Later

| Title | Status | Intent | Links |
| --- | --- | --- | --- |
| Delta item | shaped | The fourth thing. | #104 |
`;

const b64 = (s: string) => Buffer.from(s, "utf8").toString("base64");
const unb64 = (s: string) => Buffer.from(s, "base64").toString("utf8");

interface Call {
  method: string;
  path: string;
  body?: unknown;
}

/** GitHub and the queue, standing in: every request kept, answered by
 * `answer`. The clock is fixed, so a branch name is known exactly. */
function world(answer: (c: Call) => unknown) {
  const calls: Call[] = [];
  const run: Run = async (argv, o) => {
    if (argv[1] === "queue" && argv[2] === "enqueue") return { code: 0, stdout: "", stderr: "" };
    if (argv[1] !== "gh" || argv[2] !== "api") throw new Error(`unexpected command ${argv.join(" ")}`);
    const rest = argv.slice(3).filter((a) => a !== "--input" && a !== "-");
    const at = rest.indexOf("-X");
    const method = at >= 0 ? rest[at + 1] : "GET";
    const path = rest.find((a) => a.startsWith("repos/") || a.startsWith("search/")) ?? "";
    const call: Call = { method, path, ...(o?.stdin ? { body: JSON.parse(o.stdin) } : {}) };
    calls.push(call);
    return { code: 0, stdout: JSON.stringify(answer(call) ?? {}), stderr: "" };
  };
  const clock = () => new Date(START);
  const ops = new Ops(CFG, run);
  const work = new Work(ops, undefined, clock);
  return { plan: new Plan(ops, work, clock), ops, calls };
}

/** The repository as the path meets it: the fleet roadmap on main,
 * and a pull request that opens as #777. */
const repoAnswer = (c: Call): unknown => {
  if (c.method === "GET" && c.path === `repos/${REPO}/contents/${FILE}`) return { content: b64(SAMPLE), sha: "sha-file" };
  if (c.method === "GET" && c.path === `repos/${REPO}/git/ref/heads/main`) return { object: { sha: "sha-main" } };
  if (c.method === "POST" && c.path === `repos/${REPO}/pulls`)
    return { number: 777, html_url: `https://github.com/${REPO}/pull/777` };
  if (c.method === "POST" && c.path === `repos/${REPO}/issues`)
    return { number: 4242, html_url: `https://github.com/${REPO}/issues/4242`, title: (c.body as { title: string }).title };
  return {};
};

describe("applyRoadmapChange", () => {
  test("add puts a row in the section asked for, and touches nothing else", () => {
    const out = applyRoadmapChange(SAMPLE, {
      action: "add",
      title: "Epsilon item",
      to: "next",
      intent: "The fifth thing.",
      issue: 105,
    });
    const rows = parseRoadmap(out);
    expect(rows.find((r) => r.title === "Epsilon item")).toMatchObject({
      status: "planned",
      intent: "The fifth thing.",
      issues: [105],
      phase: "next",
      section: "Next",
    });
    // Apart from the one inserted line, the file is byte for byte itself.
    const added = out.split("\n").findIndex((l) => l.includes("Epsilon item"));
    const lines = out.split("\n");
    lines.splice(added, 1);
    expect(lines.join("\n")).toBe(SAMPLE);
  });

  test("move takes the row, word for word, to the other section", () => {
    const out = applyRoadmapChange(SAMPLE, { action: "move", row: "Gamma item", to: "now" });
    expect(parseRoadmap(out).find((r) => r.title === "Gamma item")?.phase).toBe("now");
    expect(out.split("\n").length).toBe(SAMPLE.split("\n").length);
    expect([...out.split("\n")].sort()).toEqual([...SAMPLE.split("\n")].sort());
  });

  test("built marks the row shipped where it sits", () => {
    const out = applyRoadmapChange(SAMPLE, { action: "built", row: "Alpha item" });
    expect(parseRoadmap(out).find((r) => r.title === "Alpha item")).toMatchObject({ status: "shipped", phase: "built" });
  });

  test("drop removes the row and no other", () => {
    const out = applyRoadmapChange(SAMPLE, { action: "drop", row: "Beta item" });
    expect(parseRoadmap(out).map((r) => r.title)).toEqual(["Alpha item", "Gamma item", "Delta item"]);
  });

  test("rewrite retitles the row in place", () => {
    const out = applyRoadmapChange(SAMPLE, { action: "rewrite", row: "Delta item", title: "Delta item, renamed" });
    const rows = parseRoadmap(out);
    expect(rows.map((r) => r.title)).toContain("Delta item, renamed");
    expect(rows.find((r) => r.title === "Delta item, renamed")?.phase).toBe("later");
  });

  test("what cannot be made mechanically is refused, with the reason", () => {
    expect(roadmapChangeRefusal({ action: "add-under", row: "Alpha item", title: "Sub" })).toContain("no row of its own");
    expect(roadmapChangeRefusal({ action: "rewrite", row: "Alpha item" })).toContain("retitle");
    expect(roadmapChangeRefusal({ action: "add", title: "Epsilon item" })).toBeNull();
    expect(() => applyRoadmapChange(SAMPLE, { action: "drop", row: "No such row" })).toThrow(/no roadmap row is titled/);
    expect(() => applyRoadmapChange(SAMPLE, { action: "add", title: "X", to: "built" })).toThrow(/no Done section/);
  });
});

describe("openRoadmapChangePr", () => {
  test("the pull request it opens touches only the roadmap file", async () => {
    const w = world(repoAnswer);
    const made = await openRoadmapChangePr(
      w.ops,
      REPO,
      FLEET,
      { action: "add", title: "Epsilon item", to: "next", intent: "The fifth thing.", issue: 105 },
      { said: 'add an item "Epsilon item"', words: "add epsilon please", issue: 4242, now: new Date(START) },
    );
    expect(made.url).toBe(`https://github.com/${REPO}/pull/777`);
    expect(made.file).toBe(FILE);
    // One branch, made from main's head.
    const refs = w.calls.filter((c) => c.method === "POST" && c.path === `repos/${REPO}/git/refs`);
    expect(refs.length).toBe(1);
    expect(refs[0].body).toEqual({ ref: `refs/heads/${made.branch}`, sha: "sha-main" });
    // One write, to the roadmap file, on that branch, over main's copy.
    const writes = w.calls.filter((c) => c.method === "PUT");
    expect(writes.length).toBe(1);
    expect(writes[0].path).toBe(`repos/${REPO}/contents/${FILE}`);
    const put = writes[0].body as { branch: string; sha: string; content: string; message: string };
    expect(put.branch).toBe(made.branch);
    expect(put.sha).toBe("sha-file");
    // No other contents path is written, or even read, anywhere in the path.
    expect(new Set(w.calls.filter((c) => c.path.includes("/contents/")).map((c) => c.path))).toEqual(
      new Set([`repos/${REPO}/contents/${FILE}`]),
    );
    // What was written is the file with the change in it.
    const rows = parseRoadmap(unb64(put.content));
    expect(rows.find((r) => r.title === "Epsilon item")?.phase).toBe("next");
    // The pull request is from that branch to main, and says what it holds.
    const pulls = w.calls.filter((c) => c.method === "POST" && c.path === `repos/${REPO}/pulls`);
    expect(pulls.length).toBe(1);
    const pr = pulls[0].body as { head: string; base: string; body: string };
    expect(pr.head).toBe(made.branch);
    expect(pr.base).toBe("main");
    expect(pr.body).toContain("#4242");
    expect(pr.body).toContain(`changes only \`${FILE}\``);
  });

  test("a change the file refuses opens nothing", async () => {
    const w = world(repoAnswer);
    await expect(
      openRoadmapChangePr(
        w.ops,
        REPO,
        FLEET,
        { action: "drop", row: "No such row" },
        { said: "drop", words: "x", issue: 1, now: new Date(START) },
      ),
    ).rejects.toThrow(/no roadmap row is titled/);
    expect(w.calls.filter((c) => c.method !== "GET")).toEqual([]);
  });
});

describe("Plan.intake: a change asked for in the app", () => {
  test("is filed as an issue, and made as a pull request against the roadmap file", async () => {
    const w = world(repoAnswer);
    const got = await w.plan.intake({
      kind: "change",
      area: "fleet",
      text: "add epsilon please",
      change: { action: "add", to: "next", title: "Epsilon item" },
    });
    expect(got.issue).toBe(4242);
    expect(got.queued).toBe(true);
    expect(got.pr).toBe(`https://github.com/${REPO}/pull/777`);
    // The issue hears about the pull request.
    const comment = w.calls.find((c) => c.method === "POST" && c.path === `repos/${REPO}/issues/4242/comments`);
    expect((comment?.body as { body: string }).body).toContain(`https://github.com/${REPO}/pull/777`);
    // And the only file written on the way is the roadmap file.
    const writes = w.calls.filter((c) => c.method === "PUT");
    expect(writes.map((c) => c.path)).toEqual([`repos/${REPO}/contents/${FILE}`]);
  });

  test("a change that cannot be made mechanically is still filed, and the answer says why not", async () => {
    const w = world(repoAnswer);
    const got = await w.plan.intake({
      kind: "change",
      area: "fleet",
      text: "drop the imaginary one",
      about: { row: "No such row" },
      change: { action: "drop" },
    });
    expect(got.issue).toBe(4242);
    expect(got.queued).toBe(true);
    expect(got.pr).toBeUndefined();
    expect(got.note).toContain("not made as a pull request");
    expect(got.note).toContain("no roadmap row is titled");
    expect(w.calls.filter((c) => c.method === "PUT")).toEqual([]);
  });
});
