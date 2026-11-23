// The plan: roadmaps read from where they live, and the one way in for
// what the owner says.
//
// Oracles: GitHub and the fleet's enqueue are recorders, so what is
// asked of them is compared exactly, as requests and packets written out
// here by hand. The roadmap text parsed is in the shape of the real
// files in planning/roadmaps/ (two of their rows are copied). What an
// intake item must become is the routing table of the design record
// (doc/2026-11-23T06-50-00Z_architecture_pm_platform_cloudflare_core),
// restated here and not read from the code under test. A queue task is
// checked against the queue's own schema file (pkgs/fleet).
import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { actions } from "../src/actions.ts";
import { readConfig } from "../src/config.ts";
import { HOLDABLE } from "../src/edge-proto.ts";
import { Docs } from "../src/docs.ts";
import { OpError, Ops, type Run, triagePacket } from "../src/ops.ts";
import {
  CAPABILITIES,
  changeSaid,
  type Later,
  laterPacket,
  notePacket,
  parseRoadmap,
  phaseOf,
  Plan,
} from "../src/plan.ts";
import { type Deps, handle, loaders } from "../src/server.ts";
import { promptPacket, Work } from "../src/work.ts";

const CFG = readConfig({ HOME: "/h" });
const REPO = "developing-today/code";
const START = Date.parse("2026-11-23T07:00:00Z");
const ROOT = `${import.meta.dir}/../../..`;

interface Call {
  method: string;
  path: string;
  q?: string;
  body?: unknown;
}
/** GitHub and the queue, standing in: every request kept, answered by
 * `answer`. Preparing a label is kept apart from the rest, since every
 * intake begins with it. The clock is the test's to move. */
function world(answer: (c: Call) => unknown | { fail: string } = () => ({})) {
  const calls: Call[] = [];
  const labelsMade: Call[] = [];
  const packets: unknown[] = [];
  const state = { enqueueFails: false, now: START };
  const run: Run = async (argv, o) => {
    if (argv[1] === "queue" && argv[2] === "enqueue") {
      packets.push(JSON.parse(readFileSync(argv[3], "utf8")));
      return state.enqueueFails
        ? { code: 1, stdout: "", stderr: "queue is down" }
        : { code: 0, stdout: "", stderr: "" };
    }
    if (argv[1] !== "gh" || argv[2] !== "api") throw new Error(`unexpected command ${argv.join(" ")}`);
    const rest = argv.slice(3).filter((a) => a !== "--input" && a !== "-");
    const at = rest.indexOf("-X");
    const method = at >= 0 ? rest[at + 1] : "GET";
    const path = rest.find((a) => a.startsWith("repos/") || a.startsWith("search/")) ?? "";
    const q = rest.find((a) => a.startsWith("q="))?.slice(2);
    const call: Call = { method, path, ...(q ? { q } : {}), ...(o?.stdin ? { body: JSON.parse(o.stdin) } : {}) };
    (method === "POST" && path === `repos/${REPO}/labels` ? labelsMade : calls).push(call);
    const out = answer(call);
    if (out !== null && typeof out === "object" && "fail" in out) {
      return { code: 1, stdout: "", stderr: String((out as { fail: string }).fail) };
    }
    return { code: 0, stdout: JSON.stringify(out ?? {}), stderr: "" };
  };
  const clock = () => new Date(state.now);
  const ops = new Ops(CFG, run);
  const work = new Work(ops, undefined, clock);
  return { plan: new Plan(ops, work, clock), ops, work, calls, labelsMade, packets, state };
}
const NOW = new Date(START).toISOString();

const issue = (number: number, title: string, labels: string[], more: Record<string, unknown> = {}) => ({
  number,
  title,
  html_url: `https://github.com/${REPO}/issues/${number}`,
  state: "open",
  updated_at: "2026-11-22T10:00:00Z",
  comments: 2,
  labels: labels.map((name) => ({ name })),
  ...more,
});

/** The queue's own schema for a task's payload, applied by hand: what it
 * requires, the values it lists, and its sizes. Not a full validator;
 * enough that a packet the queue would refuse does not pass here. */
const SCHEMA = JSON.parse(readFileSync(`${ROOT}/pkgs/fleet/repo-task-v1.schema.json`, "utf8"));
function queueWouldTake(packet: {
  title: string;
  dedupe_key: string;
  priority: number;
  payload: Record<string, unknown>;
}) {
  const p = packet.payload;
  for (const name of SCHEMA.required) expect(p[name]).toBeDefined();
  expect(p.schema).toBe(SCHEMA.properties.schema.const);
  expect(SCHEMA.properties.kind.enum).toContain(p.kind);
  expect(SCHEMA.properties.repo.enum).toContain(p.repo);
  for (const name of ["evidence", "acceptance", "tests"]) {
    const list = p[name] as string[];
    expect(list.length).toBeGreaterThanOrEqual(1);
    for (const line of list) expect(line.length).toBeGreaterThanOrEqual(1);
  }
  const scope = p.scope as { include: string[]; exclude: string[] };
  expect(Array.isArray(scope.include)).toBe(true);
  expect(scope.exclude.length).toBeGreaterThanOrEqual(1);
  const risk = p.risk as { level: string; flags: string[] };
  expect(SCHEMA.properties.risk.properties.level.enum).toContain(risk.level);
  expect(Array.isArray(risk.flags)).toBe(true);
  expect(p.timebox_minutes as number).toBeLessThanOrEqual(SCHEMA.properties.timebox_minutes.maximum);
  expect(p.max_attempts as number).toBeLessThanOrEqual(SCHEMA.properties.max_attempts.maximum);
  expect((p.issue as { number: number }).number).toBeGreaterThanOrEqual(1);
  const dedupe = p.dedupe as { checked_at: string; basis: string };
  expect(dedupe.checked_at.length).toBeGreaterThan(0);
  expect(dedupe.basis.length).toBeGreaterThan(0);
  expect((p.priority_reason as string).length).toBeGreaterThan(0);
  expect(packet.title.length).toBeLessThanOrEqual(180);
}

describe("a roadmap file", () => {
  // Two rows copied from planning/roadmaps/id.md and pm-stack.md, a row
  // made up for each remaining case, and the id file's second kind of
  // table, which is not a roadmap table.
  const FILE = [
    "# Roadmap: id",
    "",
    "Prose with a | pipe and #999 that is not a row.",
    "",
    "## Now — the path to Phase 12",
    "",
    "| Title                | Status      | Intent                                                    | Links |",
    "| -------------------- | ----------- | --------------------------------------------------------- | ----- |",
    "| Many worlds per node | in-progress | 1,000 idle / 100 active worlds; p99 5,650 ms (#557).      |       |",
    "| Fair scheduling and quotas | planned | A world that spins on fuel cannot raise another's p99. | #1067 |",
    "",
    "## Next — Phase 12: platform flexibility",
    "",
    "| Title | Status | Intent | Links |",
    "| ----- | ------ | ------ | ----- |",
    "| Sync protocol + tags/documents hardening | proposed | The tag index (#183) and the removal rules (#195) are done. | #194 |",
    "| Shipped early | done | It is out. | F12.1 #700 |",
    "",
    "## Later — Phase 13 and long-term ideas (priority/low)",
    "",
    "| Title | Status | Intent | Links |",
    "| ----- | ------ | ------ | ----- |",
    "| Native TUI client | idea | A ratatui TUI \\| on the shell contract. | #1071 |",
    "",
    "## Later — Phase 14",
    "",
    "| Title | Status | Intent | Links |",
    "| ----- | ------ | ------ | ----- |",
    "",
    "## Phase tracker — F-item mapping",
    "",
    "| Phase | Item  | Ref  | Title              |",
    "| ----- | ----- | ---- | ------------------ |",
    "| 11    | F11.1 | #238 | Relay kit location |",
    "",
    "## Done",
    "",
    "| Title | Status | Intent | Links |",
    "| ----- | ------ | ------ | ----- |",
    "| The first thing | | Long ago. | #12 |",
  ].join("\n");

  test("its rows are read by heading, and a row's own status wins for work under way and work finished", () => {
    expect(parseRoadmap(FILE)).toEqual([
      {
        title: "Many worlds per node",
        status: "in-progress",
        intent: "1,000 idle / 100 active worlds; p99 5,650 ms (#557).",
        issues: [557],
        phase: "doing", // under Now, and being worked on
        section: "Now — the path to Phase 12",
      },
      {
        title: "Fair scheduling and quotas",
        status: "planned",
        intent: "A world that spins on fuel cannot raise another's p99.",
        issues: [1067],
        phase: "now",
        section: "Now — the path to Phase 12",
      },
      {
        title: "Sync protocol + tags/documents hardening",
        status: "proposed",
        intent: "The tag index (#183) and the removal rules (#195) are done.",
        issues: [194, 183, 195],
        phase: "next",
        section: "Next — Phase 12: platform flexibility",
      },
      {
        title: "Shipped early",
        status: "done",
        intent: "It is out.",
        issues: [700],
        phase: "built", // under Next, and done
        section: "Next — Phase 12: platform flexibility",
      },
      {
        title: "Native TUI client",
        status: "idea",
        intent: "A ratatui TUI | on the shell contract.", // an escaped bar is part of the cell
        issues: [1071],
        phase: "later",
        section: "Later — Phase 13 and long-term ideas (priority/low)",
      },
      { title: "The first thing", status: "", intent: "Long ago.", issues: [12], phase: "built", section: "Done" },
    ]);
  });

  test("only a roadmap's own tables are read, each under its own heading, and each issue once", () => {
    // Written tight, with no blank lines: a heading ends the table above it.
    const tight = [
      "## Now",
      "| Title | Status | Intent | Links |",
      "| - | - | - | - |",
      "| A | planned | Twice (#557), and developing-today/opencode#77 is another repository's. | #557 #78 |",
      "## Next",
      "| Title | Status |",
      "| - | - |",
      "| B | planned |",
      "",
      "| Phase | Item | Ref | Title |",
      "| - | - | - | - |",
      "| 11 | F11.1 | #238 | Not a roadmap row, though it is under a phase |",
    ].join("\n");
    expect(parseRoadmap(tight).map((r) => [r.title, r.phase, r.issues])).toEqual([
      ["A", "now", [557, 78]],
      ["B", "next", []],
    ]);
  });

  test("text that is not a roadmap gives no rows", () => {
    expect(parseRoadmap("")).toEqual([]);
    expect(parseRoadmap("# Title\n\nNothing here.\n\n| a | b |\n| - | - |\n| 1 | 2 |")).toEqual([]);
    // A table under a heading that is not a phase is not the roadmap's.
    expect(parseRoadmap("## Themes\n\n| Title | Status |\n| - | - |\n| X | planned |")).toEqual([]);
  });

  test("the real files parse to rows in every phase they have", () => {
    for (const cap of CAPABILITIES.filter((c) => c.file)) {
      const rows = parseRoadmap(readFileSync(`${ROOT}/${cap.file}`, "utf8"));
      for (const row of rows) {
        expect(row.title.length).toBeGreaterThan(0);
        expect(["doing", "now", "next", "later", "built"]).toContain(row.phase);
      }
    }
    // The two this was written against have rows now: a parser that
    // read none would pass the loop above.
    const count = (file: string) => parseRoadmap(readFileSync(`${ROOT}/${file}`, "utf8")).length;
    expect(count("planning/roadmaps/id.md")).toBeGreaterThanOrEqual(10);
    expect(count("planning/roadmaps/pm-stack.md")).toBeGreaterThanOrEqual(4);
  });
});

describe("a capability's roadmap", () => {
  const MINE = 'label:"app/pm-stack","epic/pm-stack","epic/pm-stack-coherence","roadmap/pm-stack"';
  const OPEN_Q = `repo:${REPO} is:issue is:open ${MINE} label:"work/in-progress","queue/claimed","queue/in-progress","expedite","priority/critical","now","priority/high","next","meta/epic","kind/tracker"`;
  const BUILT_Q = `repo:${REPO} is:issue is:closed ${MINE} closed:>=2026-11-09`; // two weeks before the clock
  const FILE_PATH = `repos/${REPO}/contents/planning/roadmaps/pm-stack.md`;
  const file =
    "## Now\n\n| Title | Status | Intent | Links |\n| - | - | - | - |\n| Coherence — “one place” | in-progress | One place. | #1184 |\n";
  // GitHub gives the file encoded and wrapped in lines; the dash and the
  // quotes above are more than one byte each.
  const encoded = Buffer.from(file, "utf8")
    .toString("base64")
    .replace(/(.{20})/g, "$1\n");
  const answers = (c: Call) => {
    if (c.path === FILE_PATH) return { content: encoded };
    if (c.q === OPEN_Q) {
      return {
        total_count: 140, // more than were sent
        items: [
          issue(10, "Being built", ["app/pm-stack", "queue/claimed", "now"]),
          issue(11, "Soon", ["now", "next"]),
          issue(12, "After", ["priority/high"]),
          issue(13, "epic: the long road", ["meta/epic"]),
          issue(14, "An epic that is ranked", ["kind/tracker", "expedite"]),
        ],
      };
    }
    if (c.q === BUILT_Q) {
      return { total_count: 208, items: [issue(9, "Done last week", ["app/pm-stack"], { state: "closed" })] };
    }
    return { fail: `unexpected ${c.q ?? c.path}` };
  };

  test("is its file's rows and its issues by where they stand, each issue once", async () => {
    const w = world(answers);
    const got = await w.plan.roadmap("pm-stack");
    expect(got.rows).toEqual([
      {
        title: "Coherence — “one place”",
        status: "in-progress",
        intent: "One place.",
        issues: [1184],
        phase: "doing",
        section: "Now",
      },
    ]);
    const numbers = (list: unknown) => (list as { number: number }[]).map((i) => i.number);
    expect(numbers(got.issues.doing)).toEqual([10]); // claimed beats its rank
    expect(numbers(got.issues.now)).toEqual([11, 14]); // the sooner rank wins; a ranked epic is where its rank puts it
    expect(numbers(got.issues.next)).toEqual([12]);
    expect(numbers(got.issues.later)).toEqual([13]);
    expect(numbers(got.issues.built)).toEqual([9]);
    expect((got.issues.doing as { rank: string; working: boolean; area: string }[])[0]).toMatchObject({
      rank: "now",
      working: true,
      area: "pm-stack",
    });
    // How many there are beyond the ones listed.
    expect(got.more).toEqual({ open: 135, built: 207 });
    expect(got.partial).toBe(false);
    // What was asked of GitHub: the file and two searches, nothing written.
    expect(w.calls.map((c) => c.q ?? c.path).sort()).toEqual([FILE_PATH, OPEN_Q, BUILT_Q].sort());
    expect(w.calls.every((c) => c.method === "GET")).toBe(true);
  });

  test("where an open issue stands is its labels: working, then the sooner rank, then the long term", () => {
    expect(phaseOf(["work/in-progress", "next"])).toBe("doing");
    expect(phaseOf(["queue/in-progress"])).toBe("doing");
    expect(phaseOf(["priority/critical", "priority/high"])).toBe("now");
    expect(phaseOf(["expedite"])).toBe("now");
    expect(phaseOf(["next", "meta/epic"])).toBe("next");
    expect(phaseOf(["meta/epic"])).toBe("later");
    expect(phaseOf([])).toBe("later");
  });

  test("it is read once a minute however often it is looked at, and again when asked or when something changed", async () => {
    const w = world((c) => (c.method === "POST" ? { number: 1, html_url: "u", title: "t" } : answers(c)));
    await w.plan.roadmap("pm-stack");
    await w.plan.roadmap("pm-stack");
    // Another reader over the same repository shares what was read.
    await new Plan(w.ops, w.work, () => new Date(w.state.now)).roadmap("pm-stack");
    expect(w.calls).toHaveLength(3);
    w.state.now += 59_000;
    await w.plan.roadmap("pm-stack");
    expect(w.calls).toHaveLength(3);
    // Pressing refresh asks now.
    await w.plan.roadmap("pm-stack", true);
    expect(w.calls).toHaveLength(6);
    // What was read for one capability is not the answer for another.
    const other = await w.plan.roadmap("atlas");
    expect(other.area).toBe("atlas");
    expect(w.calls).toHaveLength(8); // atlas has no file: its two searches
    w.calls.splice(6);
    // A minute on, it is asked again by itself.
    w.state.now += 60_000;
    await w.plan.roadmap("pm-stack");
    expect(w.calls).toHaveLength(9);
    // Something said through the app may change it: the next look asks.
    await w.plan.intake({ kind: "bug", text: "x" });
    const before = w.calls.length;
    await w.plan.roadmap("pm-stack");
    expect(w.calls.length - before).toBe(3);
    // And a rank changed from any screen is where an issue stands.
    const table = actions(w.ops, w.work);
    // (Here the rank cannot be read, so setting it fails: it may have
    // been half set, and the next look asks all the same.)
    await expect(table["issue.rank"].run({ issue: 11, rank: "none" })).rejects.toThrow();
    const again = w.calls.length;
    await w.plan.roadmap("pm-stack");
    expect(w.calls.length - again).toBe(3);
  });

  test("a capability with no file is its issues, and a part that cannot be read says so alone and is asked again", async () => {
    let fail = true;
    const w = world((c) =>
      c.q?.includes("is:open") && fail ? { fail: "HTTP 403: rate limit" } : { items: [], total_count: 0 },
    );
    const got = await w.plan.roadmap("atlas");
    expect(got.file).toBeNull();
    expect(got.rows).toEqual([]);
    for (const phase of ["doing", "now", "next", "later"] as const) {
      expect((got.issues[phase] as { error: string }).error).toContain("rate limit");
    }
    expect(got.issues.built).toEqual([]);
    expect(got.partial).toBe(true);
    expect(w.calls.some((c) => c.path.includes("/contents/"))).toBe(false);
    // What failed is not what the next minute is answered from.
    fail = false;
    const next = await w.plan.roadmap("atlas");
    expect(next.partial).toBe(false);
    expect(next.issues.now).toEqual([]);
    expect(() => w.plan.roadmap("nonesuch")).toThrow(/area: one of id, fleet, atlas, pm-stack/);
  });

  test("the four the owner named come first, and every roadmap file in the repository has a page", () => {
    expect(CAPABILITIES.slice(0, 4).map((c) => c.id)).toEqual(["id", "fleet", "atlas", "pm-stack"]);
    // A file added to planning/roadmaps/ with no page here fails this.
    const files = CAPABILITIES.map((c) => c.file).filter((f): f is string => f !== null);
    // A roadmap file is one that says so in its first line; the
    // directory also holds a brief that is not one.
    const real = readdirSync(`${ROOT}/planning/roadmaps`)
      .filter((f) => f.endsWith(".md"))
      .map((f) => `planning/roadmaps/${f}`)
      .filter((f) => readFileSync(`${ROOT}/${f}`, "utf8").startsWith("# Roadmap:"));
    expect(real.length).toBeGreaterThanOrEqual(6);
    expect([...files].sort()).toEqual(real.sort());
    expect(new Set(CAPABILITIES.map((c) => c.id)).size).toBe(CAPABILITIES.length);
  });
});

describe("intake: what the owner says, routed", () => {
  const WORDS = "the  Deploy again button does nothing on my phone\n\nit just sits there — “no toast”";
  const filing = (number: number) => (c: Call) =>
    c.method === "POST" && c.path === `repos/${REPO}/issues`
      ? { number, html_url: `https://github.com/${REPO}/issues/${number}`, title: (c.body as { title: string }).title }
      : {};

  test("a bug becomes an issue with his words first, the bug labels, the capability's, and a triage task", async () => {
    const w = world(filing(13001));
    const laters: Later[] = [];
    const got = await w.plan.intake({ id: "c1", kind: "bug", area: "pm-stack", text: WORDS }, (packet, later) => {
      laters.push(later);
      return w.ops.enqueue(packet);
    });
    expect(got).toEqual({
      issue: 13001,
      url: `https://github.com/${REPO}/issues/13001`,
      commented: false,
      queued: true,
    });
    expect(w.calls).toHaveLength(1);
    const made = w.calls[0].body as { title: string; body: string; labels: string[] };
    expect(made.title).toBe("Bug (PM stack): the  Deploy again button does nothing on my phone");
    // Verbatim, before anything of ours: not trimmed inside, not re-quoted.
    expect(made.body.startsWith(`${WORDS}\n\n---\n`)).toBe(true);
    expect(made.body).toContain("Filed from the PM app: a **bug** about **PM stack**.");
    expect(made.body).toContain("The words above the line are the owner's, verbatim.");
    expect(made.body).toContain('<!-- pm-intake {"v":1,"id":"c1","kind":"bug","area":"pm-stack","mode":"issue"} -->');
    // The routing table: bug -> kind/bug, from a person, from the app, the capability's own label, rank now.
    expect(made.labels).toEqual(["kind/bug", "source/human", "owner/intake", "app/pm-stack", "now"]);
    const packet = triagePacket(REPO, CFG.queue, 13001, made.title, "now", NOW);
    expect(w.packets).toEqual([packet]);
    queueWouldTake(packet);
    // What a machine is asked to do when the edge filed the issue itself.
    expect(laters).toEqual([
      { kind: "triage.enqueue", args: { issue: 13001, title: made.title, rank: "now", at: NOW } },
    ]);
    // The app's own label is prepared first, once, with what it means on it.
    expect(w.labelsMade).toEqual([
      {
        method: "POST",
        path: `repos/${REPO}/labels`,
        body: {
          name: "owner/intake",
          color: "0e8a16",
          description: "Said by the owner through the PM app: a note, a bug or a change",
        },
      },
    ]);
    await w.plan.intake({ kind: "bug", text: "again" });
    expect(w.labelsMade).toHaveLength(1);
  });

  test("the label being there already is fine, and not being able to prepare it files nothing", async () => {
    const there = world((c) =>
      c.path === `repos/${REPO}/labels` ? { fail: '{"errors":[{"code":"already_exists"}]}' } : filing(5)(c),
    );
    expect((await there.plan.intake({ kind: "bug", text: "x" })).issue).toBe(5);
    const broken = world((c) => (c.path === `repos/${REPO}/labels` ? { fail: "HTTP 500" } : filing(6)(c)));
    await expect(broken.plan.intake({ kind: "bug", text: "x" })).rejects.toThrow(/owner\/intake label/);
    expect(broken.calls).toEqual([]);
    expect(broken.packets).toEqual([]);
  });

  test("a note about an existing issue is left on that issue, ranks it if asked, and wakes the fleet for it", async () => {
    const w = world((c) =>
      c.path.endsWith("/comments")
        ? { html_url: `https://github.com/${REPO}/issues/557#issuecomment-1` }
        : c.method === "GET"
          ? { labels: [{ name: "next" }] }
          : {},
    );
    const laters: Later[] = [];
    const got = await w.plan.intake(
      { kind: "note", area: "id", text: "do the latency work first", about: { issue: 557 }, rank: "expedite" },
      (packet, later) => {
        laters.push(later);
        return w.ops.enqueue(packet);
      },
    );
    expect(got).toEqual({
      issue: 557,
      url: `https://github.com/${REPO}/issues/557#issuecomment-1`,
      commented: true,
      queued: true,
    });
    // No new issue: the note is where people already look.
    expect(w.calls.some((c) => c.path === `repos/${REPO}/issues`)).toBe(false);
    const comment = w.calls.find((c) => c.path === `repos/${REPO}/issues/557/comments`)?.body as { body: string };
    expect(comment.body.startsWith("do the latency work first\n\n---\n")).toBe(true);
    expect(comment.body).toContain("About: #557");
    expect(comment.body).toContain("Rank asked for: `expedite`");
    expect(comment.body).toContain('"kind":"note","area":"id","mode":"issue"} -->');
    // It is marked as having come in through the app, and its rank is
    // set: the new one added, the old one taken off.
    expect(w.calls.filter((c) => c.method !== "GET").map((c) => [c.method, c.path, c.body ?? null])).toEqual([
      ["POST", `repos/${REPO}/issues/557/comments`, comment],
      ["POST", `repos/${REPO}/issues/557/labels`, { labels: ["owner/intake"] }],
      ["POST", `repos/${REPO}/issues/557/labels`, { labels: ["expedite"] }],
      ["DELETE", `repos/${REPO}/issues/557/labels/next`, null],
    ]);
    const packet = notePacket(REPO, CFG.queue, 557, "do the latency work first", "expedite", NOW);
    expect(w.packets).toEqual([packet]);
    expect(packet.priority).toBe(10);
    expect(packet.dedupe_key).toBe(`repo-health:v1:triage:${REPO}:issue:557`); // the issue's one task
    expect(packet.title).toBe(`Owner's note on ${REPO}#557: do the latency work first`);
    queueWouldTake(packet);
    expect(laters).toEqual([
      {
        kind: "triage.enqueue",
        args: { issue: 557, title: "do the latency work first", rank: "expedite", at: NOW, note: true },
      },
    ]);
  });

  test("a note with no rank asked for leaves the issue's rank alone, and is still looked at soon", async () => {
    const w = world((c) => (c.path.endsWith("/comments") ? { html_url: "u" } : {}));
    await w.plan.intake({ kind: "note", text: "fyi", about: { issue: 557 } });
    expect(w.calls.map((c) => [c.method, c.path])).toEqual([
      ["POST", `repos/${REPO}/issues/557/comments`],
      ["POST", `repos/${REPO}/issues/557/labels`],
    ]);
    expect(w.calls[1].body).toEqual({ labels: ["owner/intake"] });
    expect((w.packets[0] as { priority: number }).priority).toBe(20);
  });

  test("the note stands when what follows it fails, and the answer says what did not happen", async () => {
    const w = world((c) => {
      if (c.path.endsWith("/comments")) return { html_url: "u" };
      if (c.path.endsWith("/issues/9/labels")) return { fail: "HTTP 500: labels are down" };
      if (c.method === "GET") return { fail: "HTTP 502: bad gateway" };
      return {};
    });
    w.state.enqueueFails = true;
    const got = await w.plan.intake({ kind: "note", text: "x", about: { issue: 9 }, rank: "now" });
    expect(got).toMatchObject({ issue: 9, commented: true, queued: false });
    expect(got.note).toContain("its rank was not set");
    expect(got.note).toContain("left, not queued: queue is down");
    // A comment that could not be left is a failure, and nothing follows it.
    const none = world((c) => (c.path.endsWith("/comments") ? { fail: "HTTP 404: no such issue" } : {}));
    await expect(none.plan.intake({ kind: "note", text: "x", about: { issue: 9 } })).rejects.toThrow(/404/);
    expect(none.packets).toEqual([]);
  });

  test("a change to a roadmap row says exactly what was asked, and where the row lives", async () => {
    const w = world(filing(13002));
    await w.plan.intake({
      kind: "change",
      area: "pm-stack",
      text: "this matters more than the projector now",
      about: { row: "Aggregator UI: cross-repo WIP board and roadmap view", phase: "next" },
      change: { action: "move", to: "now" },
    });
    const made = w.calls[0].body as { title: string; body: string; labels: string[] };
    expect(made.title).toBe(
      'PM stack roadmap: move "Aggregator UI: cross-repo WIP board and roadmap view" to "Planned: immediately"',
    );
    expect(made.body).toContain(
      'About the roadmap row: "Aggregator UI: cross-repo WIP board and roadmap view" (Planned: short term) in `planning/roadmaps/pm-stack.md`',
    );
    expect(made.body).toContain('Asked for: move to "Planned: immediately"');
    // No kind of its own: from a person, from the app, the capability's, ranked next.
    expect(made.labels).toEqual(["source/human", "owner/intake", "app/pm-stack", "next"]);
    // The other things that may be asked of a row, in words.
    expect(changeSaid({ action: "add", to: "later", title: "Offline maps" })).toBe(
      'add an item "Offline maps" under "Planned: long term"',
    );
    expect(changeSaid({ action: "add-under", to: null, title: "Tiles" }, "Offline maps")).toBe(
      'add a sub-item "Tiles" under "Offline maps"',
    );
    expect(changeSaid({ action: "built", to: null, title: null }, "Row")).toBe('mark "Row" as built');
    expect(changeSaid({ action: "drop", to: null, title: null }, "Row")).toBe('drop "Row" from the roadmap');
    expect(changeSaid({ action: "rewrite", to: null, title: null }, "Row")).toBe('rewrite "Row"');
    expect(changeSaid({ action: "move", to: "built", title: null }, "Row")).toBe('move "Row" to "Built"');
    // A title given is the title filed, and a new item is named in it otherwise.
    const named = world(filing(1));
    await named.plan.intake({ kind: "change", text: "x", title: "  Mine  ", change: { action: "drop" } });
    expect((named.calls[0].body as { title: string }).title).toBe("Mine");
    const added = world(filing(2));
    await added.plan.intake({
      kind: "change",
      area: "fleet",
      text: "x",
      change: { action: "add", to: "later", title: "Offline maps" },
    });
    expect((added.calls[0].body as { title: string; labels: string[] }).title).toBe(
      'Fleet roadmap: add an item "Offline maps" under "Planned: long term"',
    );
    expect((added.calls[0].body as { labels: string[] }).labels).toEqual([
      "source/human",
      "owner/intake",
      "app/fleet",
      "next",
    ]);
  });

  test("asked for as a session, it is a prompt that starts one with his words first, and carries the routing labels", async () => {
    const w = world(filing(13003));
    const laters: Later[] = [];
    const got = await w.plan.intake(
      {
        kind: "note",
        area: "fleet",
        text: "make lanes pick up owner notes first",
        mode: "session",
        about: { issue: 12236 },
      },
      (packet, later) => {
        laters.push(later);
        return w.ops.enqueue(packet);
      },
    );
    expect(got).toEqual({
      issue: 13003,
      url: `https://github.com/${REPO}/issues/13003`,
      commented: false,
      queued: true,
    });
    const made = w.calls.find((c) => c.method === "POST" && c.path === `repos/${REPO}/issues`)?.body as {
      title: string;
      body: string;
      labels: string[];
    };
    const prompt = "make lanes pick up owner notes first\n\nAbout #12236. (note, Fleet; from the PM app)";
    expect(made.title).toBe("Owner: make lanes pick up owner notes first");
    expect(made.body.startsWith(prompt)).toBe(true);
    expect(made.labels).toEqual(["owner/prompt", "expedite"]);
    expect(w.calls.find((c) => c.path === `repos/${REPO}/issues/13003/labels`)?.body).toEqual({
      labels: ["source/human", "owner/intake", "app/fleet"],
    });
    // The task is the prompt's: top band, and it starts a session on that issue.
    const packet = promptPacket(REPO, CFG.queue, 13003, prompt, NOW);
    expect(w.packets).toEqual([packet]);
    queueWouldTake(packet);
    expect(laters).toEqual([{ kind: "prompt.enqueue", args: { issue: 13003, text: prompt, at: NOW } }]);
    // A change sent as a session says what it was about.
    const change = world(filing(7));
    await change.plan.intake({
      kind: "change",
      area: "id",
      text: "sooner",
      mode: "session",
      about: { row: "Native TUI client", phase: "later" },
      change: { action: "move", to: "next" },
    });
    expect((change.calls[0].body as { body: string }).body).toStartWith(
      'sooner\n\nAbout the roadmap row "Native TUI client" of id. Asked for: move to "Planned: short term". (change, id; from the PM app)',
    );
    // A note about an issue, sent as a session, is a session and not a comment.
    expect(w.calls.some((c) => c.path.endsWith("/comments"))).toBe(false);
  });

  test("when the task cannot be filed the issue still is, and the answer says so", async () => {
    const w = world(filing(13004));
    w.state.enqueueFails = true;
    const got = await w.plan.intake({ kind: "bug", text: "x" });
    expect(got).toMatchObject({ issue: 13004, queued: false });
    expect(got.note).toBe("filed, not queued: queue is down");
    // An enqueue that throws (the edge's, with its store away) is the same.
    const thrown = world(filing(13005));
    const said = await thrown.plan.intake({ kind: "bug", text: "x" }, async () => {
      throw new Error("held store is full");
    });
    expect(said).toMatchObject({ issue: 13005, queued: false, note: "filed, not queued: held store is full" });
    // With no capability named, it carries none; and a rank given is the rank filed.
    const none = world(filing(1));
    await none.plan.intake({ kind: "bug", text: "x", rank: "none" });
    expect((none.calls[0].body as { labels: string[] }).labels).toEqual(["kind/bug", "source/human", "owner/intake"]);
    expect((none.packets[0] as { priority: number }).priority).toBe(30);
    const high = world(filing(2));
    await high.plan.intake({ kind: "note", text: "x", rank: "priority/critical" });
    expect((high.calls[0].body as { labels: string[] }).labels).toEqual([
      "source/human",
      "owner/intake",
      "priority/critical",
    ]);
    expect((high.calls[0].body as { title: string }).title).toBe("Note: x");
  });

  test("a bug about an existing issue is its own report, naming the issue, not a comment on it", async () => {
    const w = world(filing(13006));
    const got = await w.plan.intake({ kind: "bug", text: "this broke again", about: { issue: 12520 } });
    expect(got).toMatchObject({ issue: 13006, commented: false });
    expect(w.calls.map((c) => c.path)).toEqual([`repos/${REPO}/issues`]);
    expect((w.calls[0].body as { body: string }).body).toContain("About: #12520");
  });

  test("a long first line is cut for the title only, never in what was said", async () => {
    const long = `${"word ".repeat(40)}end`;
    const w = world(filing(3));
    await w.plan.intake({ kind: "note", text: long });
    const made = w.calls[0].body as { title: string; body: string };
    expect(made.title.length).toBeLessThanOrEqual(110);
    expect(made.title.endsWith("…")).toBe(true);
    expect(made.body.startsWith(`${long}\n\n---\n`)).toBe(true);
  });

  test("what cannot be routed is refused before GitHub is asked anything", async () => {
    const w = world();
    const bad: Record<string, unknown>[] = [
      { kind: "wish", text: "x" },
      { kind: "bug" },
      { kind: "bug", text: "   " },
      { kind: "bug", text: 7 },
      { kind: "bug", text: "x".repeat(8001) },
      { kind: "bug", text: "x", area: "nonesuch" },
      { kind: "bug", text: "x", rank: "urgent" },
      { kind: "bug", text: "x", mode: "email" },
      { kind: "bug", text: "x", title: "t".repeat(201) },
      { kind: "note", text: "x", about: "557" },
      { kind: "note", text: "x", about: { issue: "12a" } },
      { kind: "note", text: "x", about: { issue: 0 } },
      { kind: "note", text: "x", about: { issue: -3 } },
      { kind: "note", text: "x", about: { row: "r", phase: "someday" } },
      { kind: "note", text: "x", about: { row: "r".repeat(301) } },
      { kind: "note", text: "x", change: { action: "drop" } }, // only a change says what to change
      { kind: "change", text: "x" }, // a change says what to change
      { kind: "change", text: "x", change: "move" },
      { kind: "change", text: "x", change: { action: "teleport" } },
      { kind: "change", text: "x", change: { action: "move" } }, // to where
      { kind: "change", text: "x", change: { action: "move", to: "soonish" } },
      { kind: "change", text: "x", change: { action: "add", title: "" } },
    ];
    for (const args of bad) {
      await expect(w.plan.intake(args)).rejects.toBeInstanceOf(OpError);
    }
    expect(w.calls).toEqual([]);
    expect(w.labelsMade).toEqual([]);
    expect(w.packets).toEqual([]);
  });

  test("it is an action like the others: recorded without his words, and held when it cannot be done now", async () => {
    const w = world(filing(13005));
    const table = actions(w.ops, w.work);
    expect(await table["intake.send"].run({ kind: "bug", area: "id", text: "secret words" })).toMatchObject({
      issue: 13005,
    });
    const kept = table["intake.send"].audit({
      kind: "bug",
      area: "id",
      text: "secret words",
      title: "secret title",
      about: { issue: 5, row: "secret row" },
      rank: "now",
    });
    expect(kept).toEqual({ kind: "bug", area: "id", mode: "issue", rank: "now", about: 5 });
    expect(JSON.stringify(kept)).not.toContain("secret");
    expect(HOLDABLE.has("intake.send")).toBe(true);
  });

  test("the half a machine does for the edge is built again there from checked arguments", async () => {
    const w = world();
    const table = actions(w.ops, w.work);
    expect(HOLDABLE.has("triage.enqueue")).toBe(true);
    expect(await table["triage.enqueue"].run({ issue: 13005, title: "Bug: x", rank: "now", at: NOW })).toEqual({
      issue: 13005,
      queued: true,
    });
    expect(
      await table["triage.enqueue"].run({ issue: 557, title: "first line", rank: "none", at: NOW, note: true }),
    ).toEqual({ issue: 557, queued: true });
    expect(w.packets).toEqual([
      triagePacket(REPO, CFG.queue, 13005, "Bug: x", "now", NOW),
      notePacket(REPO, CFG.queue, 557, "first line", "none", NOW),
    ]);
    expect(laterPacket(REPO, CFG.queue, { issue: 1, title: "t", rank: "next", at: NOW, note: "yes" })).toEqual(
      triagePacket(REPO, CFG.queue, 1, "t", "next", NOW), // only `true` is a note
    );
    for (const args of [
      { issue: 0, title: "t", rank: "now", at: "x" },
      { issue: 1.5, title: "t", rank: "now", at: "x" },
      { issue: "abc", title: "t", rank: "now", at: "x" },
      { issue: 1, title: "", rank: "now", at: "x" },
      { issue: 1, title: "t".repeat(301), rank: "now", at: "x" },
      { issue: 1, title: 5, rank: "now", at: "x" },
      { issue: 1, title: "t", rank: "soon", at: "x" },
      { issue: 1, title: "t", at: "x" },
      { issue: 1, title: "t", rank: "now", at: "" },
      { issue: 1, title: "t", rank: "now" },
    ]) {
      await expect(table["triage.enqueue"].run(args)).rejects.toBeInstanceOf(OpError);
    }
    expect(w.packets).toHaveLength(2);
    // Run late, a failure must be a failure: nobody is waiting to read "not queued".
    w.state.enqueueFails = true;
    await expect(table["triage.enqueue"].run({ issue: 1, title: "t", rank: "now", at: "x" })).rejects.toThrow(
      /queue is down/,
    );
    expect(table["triage.enqueue"].audit({ issue: 1, title: "secret", rank: "now" })).toEqual({
      issue: 1,
      rank: "now",
    });
  });
});

describe("what came in, and the bugs", () => {
  test("the Intake screen is read back from GitHub: what the app filed, as it stands now", async () => {
    const w = world((c) =>
      c.path.startsWith(`repos/${REPO}/issues?labels=owner%2Fintake`)
        ? [
            issue(13001, "Bug (PM stack): the button", [
              "kind/bug",
              "owner/intake",
              "app/pm-stack",
              "now",
              "queue/claimed",
            ]),
            issue(13002, "id roadmap: move it", ["owner/intake", "app/id", "next"], {
              body: 'sooner\n\n---\nFiled…\n<!-- pm-intake {"v":1,"id":null,"kind":"change","area":"id","mode":"issue"} -->',
            }),
            issue(13003, "Owner: make lanes pick up notes", ["owner/prompt", "owner/intake", "app/fleet", "expedite"], {
              state: "closed",
            }),
            issue(13004, "Note: a thought", ["owner/intake"], { body: "a thought\n<!-- pm-intake {not json} -->" }),
            issue(557, "An old issue he left a note on", ["owner/intake", "app/id"], { body: null }),
            { ...issue(13010, "a pull request with the label", ["owner/intake"]), pull_request: {} },
          ]
        : {},
    );
    const got = await w.plan.intakeList();
    expect(got.items.map((i) => [i.number, i.kind, i.area, i.rank, i.state, i.working])).toEqual([
      [13001, "bug", "pm-stack", "now", "open", true],
      [13002, "change", "id", "next", "open", false],
      [13003, "session", "fleet", "expedite", "closed", false],
      [13004, "note", null, "none", "open", false],
      [557, "note", "id", "none", "open", false],
    ]);
    expect(w.calls[0].path).toBe(
      `repos/${REPO}/issues?labels=owner%2Fintake&state=all&sort=updated&direction=desc&per_page=50`,
    );
    // The routing table is shown with it, so the screen says where things go.
    expect(got.routes).toEqual({
      note: { labels: ["source/human", "owner/intake"], rank: "next" },
      bug: { labels: ["kind/bug", "source/human", "owner/intake"], rank: "now" },
      change: { labels: ["source/human", "owner/intake"], rank: "next" },
    });
    // Read once a minute, and again when asked.
    await w.plan.intakeList();
    expect(w.calls).toHaveLength(1);
    await w.plan.intakeList(true);
    expect(w.calls).toHaveLength(2);
  });

  test("a list that could not be read is an error, and is asked for again", async () => {
    let fail = true;
    const w = world(() => (fail ? { fail: "HTTP 502" } : []));
    await expect(w.plan.intakeList()).rejects.toThrow(/502/);
    fail = false;
    expect((await w.plan.intakeList()).items).toEqual([]);
    expect(w.calls).toHaveLength(2);
  });

  test("open bugs are listed soonest first, and the ones fixed lately after", async () => {
    const w = world((c) =>
      c.q?.includes("is:open")
        ? {
            total_count: 4,
            items: [
              issue(3, "c", ["kind/bug"]),
              issue(2, "b", ["kind/bug", "next"]),
              issue(1, "a", ["bug", "expedite"]),
              issue(4, "d", ["kind/bug", "next"]),
            ],
          }
        : { total_count: 516, items: [issue(9, "fixed", ["kind/bug"], { state: "closed" })] },
    );
    const got = await w.plan.bugs();
    expect(got.open.map((b) => [b.number, b.rank, b.kind])).toEqual([
      [1, "expedite", "bug"], // the classic label is a bug too
      [4, "next", "bug"], // same rank: the newer report first
      [2, "next", "bug"],
      [3, "none", "bug"],
    ]);
    expect(got.fixed.map((b) => b.number)).toEqual([9]);
    expect(got.more).toEqual({ open: 0, fixed: 515 });
    expect(w.calls.map((c) => c.q)).toEqual([
      `repo:${REPO} is:issue is:open label:"kind/bug","bug"`,
      `repo:${REPO} is:issue is:closed label:"kind/bug","bug" closed:>=2026-11-09`,
    ]);
    await w.plan.bugs();
    expect(w.calls).toHaveLength(2);
  });
});

describe("over HTTP", () => {
  function served(answer: (c: Call) => unknown, who: "owner" | 401 = "owner") {
    const w = world(answer);
    const audited: Record<string, unknown>[] = [];
    const d: Deps = {
      cfg: CFG,
      ops: w.ops,
      work: w.work,
      plan: w.plan,
      // The documents are the real ones, never loaded here: the plan is
      // in none of them, and sending touches none.
      docs: new Docs(loaders(w.ops, w.work, [])),
      actions: actions(w.ops, w.work, undefined, undefined, w.plan),
      identify: async () => {
        if (who === "owner") return { via: "cloudflare-access", email: "owner@example.com" };
        throw new (await import("../src/auth.ts")).AuthError(401, "sign-in required");
      },
      audit: async (e) => {
        audited.push(e);
      },
    };
    return { d, w, audited };
  }
  const get = (path: string) => new Request(`https://pm.security.cab${path}`, { headers: { host: "pm.security.cab" } });
  const post = (path: string, body: unknown) =>
    new Request(`https://pm.security.cab${path}`, {
      method: "POST",
      body: JSON.stringify(body),
      headers: {
        host: "pm.security.cab",
        "content-type": "application/json",
        "x-pm-console": "1",
        origin: "https://pm.security.cab",
      },
    });

  test("the screens' reads answer as the plan does, behind sign-in, and refresh asks again", async () => {
    const { d, w } = served((c) =>
      c.path.includes("/issues?labels=") ? [] : c.path.includes("/contents/") ? { content: "" } : { items: [] },
    );
    const caps = await (await handle(get("/console/api/roadmaps"), d)).json();
    expect(caps.slice(0, 4)).toEqual([
      { id: "id", title: "id", file: "planning/roadmaps/id.md" },
      { id: "fleet", title: "Fleet", file: null },
      { id: "atlas", title: "Atlas", file: null },
      { id: "pm-stack", title: "PM stack", file: "planning/roadmaps/pm-stack.md" },
    ]);
    expect(w.calls).toHaveLength(0); // the list of capabilities asks GitHub nothing
    const fleet = await handle(get("/console/api/roadmap?area=fleet"), d);
    expect(fleet.status).toBe(200);
    expect(await fleet.json()).toMatchObject({ area: "fleet", title: "Fleet", rows: [], partial: false });
    expect(w.calls).toHaveLength(2);
    await handle(get("/console/api/roadmap?area=fleet"), d);
    expect(w.calls).toHaveLength(2);
    await handle(get("/console/api/roadmap?area=fleet&fresh=1"), d);
    expect(w.calls).toHaveLength(4);
    // Only "1" asks again: anything else in its place is a look like any other.
    await handle(get("/console/api/roadmap?area=fleet&fresh=0"), d);
    expect(w.calls).toHaveLength(4);
    expect((await handle(get("/console/api/roadmap?area=nonesuch"), d)).status).toBe(400);
    expect((await handle(get("/console/api/roadmap"), d)).status).toBe(400);
    expect(await (await handle(get("/console/api/bugs"), d)).json()).toMatchObject({ open: [], fixed: [] });
    expect((await (await handle(get("/console/api/intake"), d)).json()).items).toEqual([]);
    const before = w.calls.length;
    await handle(get("/console/api/bugs?fresh=1"), d);
    await handle(get("/console/api/intake?fresh=1"), d);
    expect(w.calls.length - before).toBe(3);
    for (const path of ["roadmaps", "roadmap?area=id", "bugs", "intake"]) {
      expect((await handle(get(`/console/api/${path}`), served(() => ({}), 401).d)).status).toBe(401);
    }
  });

  test("sending is an action: done, answered with where it went, and recorded without his words", async () => {
    const { d, w, audited } = served((c) =>
      c.method === "POST" && c.path === `repos/${REPO}/issues`
        ? { number: 13020, html_url: "https://github.com/x/y/issues/13020", title: "t" }
        : c.path.includes("/issues?labels=")
          ? []
          : {},
    );
    await handle(get("/console/api/intake"), d);
    const res = await handle(
      post("/console/api/act", {
        kind: "intake.send",
        args: { id: "d1", kind: "bug", area: "atlas", text: "a private thought" },
      }),
      d,
    );
    expect(res.status).toBe(200);
    expect((await res.json()).result).toEqual({
      issue: 13020,
      url: "https://github.com/x/y/issues/13020",
      commented: false,
      queued: true,
    });
    expect(w.packets).toHaveLength(1);
    expect(JSON.stringify(audited)).not.toContain("a private thought");
    expect(audited.some((e) => e.action === "intake.send" && e.ok === true)).toBe(true);
    // What the Intake screen shows is read again after it.
    const before = w.calls.length;
    await handle(get("/console/api/intake"), d);
    expect(w.calls.length - before).toBe(1);
    // A send that cannot be routed is refused as the caller's mistake.
    const bad = await handle(post("/console/api/act", { kind: "intake.send", args: { kind: "wish", text: "x" } }), d);
    expect(bad.status).toBe(400);
  });
});
