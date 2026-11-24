// The list of Workers, as the Platform screen reads it.
//
// Oracles: the rules for a row are written out here by hand, one case
// each. For the properties, a generator builds rows that keep every
// rule (so "this is a good row" is what the generator built, not what
// the parser says), a fault is one change known to break one rule, and
// the list is checked against a model written out here: the first good
// row of each name, in the file's order within a group, the groups in
// the order the screen shows them. That the file in the repository
// matches the repository is checked beside it
// (pm-stack/platform/workers.test.ts); that the edge reads it with no
// machine is in test/edge.test.ts.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { readConfig } from "../src/config.ts";
import { Ops, type Run } from "../src/ops.ts";
import { DEPLOYED, GROUPS, STATES, type WorkerRow, workerRow, workersList } from "../src/platform.ts";
import { letters, pick, rng } from "./support.ts";

const good: Omit<WorkerRow, "runDeploys"> = {
  name: "message-bridge",
  title: "Message bridge",
  group: "fleet",
  path: "pkgs/message-bridge/worker",
  does: "Sends and receives messages.",
  state: "in service",
  deploy: "by hand",
  workflow: null,
  address: "https://bridge.security.cab",
  next: "Deployed from Actions.",
};

test("a row of the list: what it must say, and what cannot both be true of it", () => {
  expect(workerRow(good)).toEqual({ ...good, runDeploys: false });
  expect(workerRow({ ...good, deploy: "actions", workflow: "bridge-deploy.yml" })).toMatchObject({
    deploy: "actions",
    workflow: "bridge-deploy.yml",
    // Unless the list says a run that succeeds is a deploy, it is not taken for one.
    runDeploys: false,
  });
  expect(workerRow({ ...good, deploy: "actions", workflow: "bridge-deploy.yml", runDeploys: true })?.runDeploys).toBe(
    true,
  );
  expect(workerRow({ ...good, state: "not deployed", deploy: "none", address: null })).toMatchObject({
    state: "not deployed",
    address: null,
  });
  for (const [why, bad] of [
    ["no name", { ...good, name: "" }],
    ["a name that is not a Worker's", { ...good, name: "Message Bridge" }],
    ["no title", { ...good, title: " " }],
    ["nothing said of what it does", { ...good, does: undefined }],
    ["nothing said of what is next", { ...good, next: "" }],
    ["a path out of the repository", { ...good, path: "../elsewhere" }],
    ["a path that is not one", { ...good, path: "/etc/passwd" }],
    ["a path that climbs out part-way", { ...good, path: "pkgs/../../elsewhere" }],
    ["a group there is not", { ...good, group: "misc" }],
    ["a state there is not", { ...good, state: "maybe" }],
    ["a way of deploying there is not", { ...good, deploy: "magic" }],
    ["from Actions, with no workflow", { ...good, deploy: "actions", workflow: null }],
    ["by hand, with a workflow", { ...good, workflow: "bridge-deploy.yml" }],
    ["a run that deploys, with no workflow to run", { ...good, runDeploys: true }],
    [
      "a run that deploys, said as something else than yes or no",
      { ...good, deploy: "actions", workflow: "x.yml", runDeploys: "yes" },
    ],
    ["a workflow that is not a file's name", { ...good, deploy: "actions", workflow: "../deploy.sh" }],
    ["in service, deployed by nothing", { ...good, deploy: "none" }],
    ["not deployed, and deployed by hand", { ...good, state: "not deployed", address: null }],
    ["not deployed, with an address", { ...good, state: "not deployed", deploy: "none" }],
    ["an address that is not https", { ...good, address: "http://bridge.security.cab" }],
    ["an address that is a script", { ...good, address: "javascript:alert(1)" }],
    ["not an object", "message-bridge"],
    ["nothing", null],
  ] as const) {
    expect([why, workerRow(bad)]).toEqual([why, null]);
  }
});

test("the list: well-formed rows in the order they are shown, a name once, and a count of what could not be read", () => {
  const list = workersList({
    checked: "2026-11-23",
    workers: [
      { ...good, name: "lab-thing", group: "lab" },
      good,
      { ...good, title: "The same name again" },
      { ...good, name: "the-platform", group: "platform" },
      { ...good, name: "Bad Name" },
      "not a row",
    ],
  });
  expect(list.workers.map((w) => [w.name, w.group, w.title])).toEqual([
    ["the-platform", "platform", "Message bridge"],
    ["message-bridge", "fleet", "Message bridge"],
    ["lab-thing", "lab", "Message bridge"],
  ]);
  expect([list.checked, list.malformed]).toEqual(["2026-11-23", 3]);
  // A file that is not a list is an empty one, not a crash.
  for (const odd of [null, {}, { workers: "all of them" }, [], "text"]) {
    expect([odd, workersList(odd)]).toEqual([odd, { checked: null, workers: [], malformed: 0 }]);
  }
  expect(workersList({ checked: "yesterday", workers: [] }).checked).toBeNull();
});

test("read through the fleet binary: the repository's list, with the last deploy of each Worker that Actions deploys", async () => {
  const file = readFileSync(join(import.meta.dir, "..", "..", "platform", "workers.json"), "utf8");
  const asked: string[] = [];
  const answers: Record<string, () => unknown> = {
    "repos/developing-today/code/contents/pm-stack/platform/workers.json": () => ({
      // As GitHub sends a file: encoded, and wrapped in lines.
      content: (btoa(file).match(/.{1,60}/g) ?? []).join("\n"),
    }),
    "repos/developing-today/code/actions/workflows/pm-console-edge-deploy.yml/runs?branch=main&per_page=1": () => ({
      workflow_runs: [
        {
          head_sha: "2a96e875037d4feb8f97b430f1f1e7ef190fdabf",
          status: "completed",
          conclusion: "success",
          created_at: "2026-11-23T16:32:32Z",
          html_url: "https://github.com/developing-today/code/actions/runs/45217672084",
        },
      ],
    }),
    "repos/developing-today/code/actions/workflows/pkarr-worker-deploy.yml/runs?branch=main&per_page=1": () => ({
      workflow_runs: [],
    }),
  };
  const fails = new Set<string>();
  const run: Run = async (argv) => {
    const path = argv[3];
    asked.push(path);
    if (fails.has(path) || !(path in answers)) return { code: 1, stdout: "", stderr: `HTTP 404: ${path}` };
    return { code: 0, stdout: JSON.stringify(answers[path]()), stderr: "" };
  };
  const ops = new Ops(readConfig({ HOME: "/h" }), run);
  const p = await ops.platform();
  expect([p.checked, p.malformed, p.workers.length]).toEqual(["2026-11-23", 0, 20]);
  // One read of the list, and one of each deploy workflow: not one per Worker.
  expect(asked).toHaveLength(4);
  const byName = new Map(p.workers.map((w) => [w.name, w]));
  expect(byName.get("pm-console-edge")?.run).toEqual({
    commit: "2a96e875037d",
    state: "success",
    at: "2026-11-23T16:32:32Z",
    url: "https://github.com/developing-today/code/actions/runs/45217672084",
  });
  // A deploy that has never run on main has no run to show.
  expect(byName.get("pkarr")?.run).toBeNull();
  // The console's deploy puts what it ran on into service; pkarr's goes in steps.
  expect([byName.get("pm-console-edge")?.runDeploys, byName.get("pkarr")?.runDeploys]).toEqual([true, false]);
  expect(byName.get("voice-line")?.run).toBeNull();
  // Its runs cannot be read: that Worker says so, and the rest still show.
  fails.add("repos/developing-today/code/actions/workflows/pm-console-edge-deploy.yml/runs?branch=main&per_page=1");
  const partial = await ops.platform();
  expect(partial.workers).toHaveLength(20);
  expect(partial.workers.find((w) => w.name === "pm-console-edge")?.run).toEqual({
    error: expect.stringContaining("404"),
  });
  // Two Workers that one workflow deploys: it is asked about once, and both show its run.
  const shared = { ...good, deploy: "actions", workflow: "pm-console-edge-deploy.yml" };
  const two = JSON.stringify({ checked: "2026-11-23", workers: [shared, { ...shared, name: "its-twin" }] });
  const real = answers["repos/developing-today/code/contents/pm-stack/platform/workers.json"];
  fails.clear();
  answers["repos/developing-today/code/contents/pm-stack/platform/workers.json"] = () => ({ content: btoa(two) });
  asked.length = 0;
  const twins = await ops.platform();
  expect(asked).toHaveLength(2);
  expect(twins.workers.map((w) => (w.run as { commit: string }).commit)).toEqual(["2a96e875037d", "2a96e875037d"]);
  answers["repos/developing-today/code/contents/pm-stack/platform/workers.json"] = real;
  // The list itself cannot be read, or is not a list: said as that.
  fails.add("repos/developing-today/code/contents/pm-stack/platform/workers.json");
  await expect(ops.platform()).rejects.toThrow();
  fails.clear();
  answers["repos/developing-today/code/contents/pm-stack/platform/workers.json"] = () => ({
    content: btoa("not json"),
  });
  await expect(ops.platform()).rejects.toThrow("the list of Workers is not valid JSON");
  answers["repos/developing-today/code/contents/pm-stack/platform/workers.json"] = () => ({});
  await expect(ops.platform()).rejects.toThrow("the list of Workers could not be read");
});

// --- properties -----------------------------------------------------

const LOWER = "abcdefghijklmnopqrstuvwxyz";
const DIGIT = "0123456789";
const ALNUM = `${LOWER}${LOWER.toUpperCase()}${DIGIT}`;
/** Words as a person writes them: letters of more than one alphabet,
 * marks and blanks, never blank as a whole. */
const prose = (next: () => number, max: number) =>
  `${pick(next, [...ALNUM])}${letters(next, `${ALNUM}   .,:;-'()/é√日\n`, 0, Math.min(max, 60) - 1)}`;

type Loose = Record<string, unknown>;

/** A row that keeps every rule, by construction. What may be left out
 * is sometimes left out and sometimes said as null. */
function goodRow(next: () => number, name?: string): Loose {
  const state = next() < 0.75 ? "in service" : "not deployed";
  const deploy = state === "not deployed" ? "none" : next() < 0.4 ? "actions" : "by hand";
  const row: Loose = {
    name: name ?? `${pick(next, [...LOWER, ...DIGIT])}${letters(next, `${LOWER}${DIGIT}-`, 0, 62)}`,
    title: prose(next, 80),
    group: pick(next, GROUPS),
    path: Array.from(
      { length: 1 + Math.floor(next() * 4) },
      () => `${pick(next, [...ALNUM])}${letters(next, `${ALNUM}_-`, 0, 12)}${next() < 0.2 ? ".d" : ""}`,
    ).join("/"),
    does: prose(next, 400),
    state,
    deploy,
    next: prose(next, 400),
  };
  if (deploy === "actions") {
    row.workflow = `${pick(next, [...ALNUM])}${letters(next, `${ALNUM}._-`, 0, 30)}.${next() < 0.5 ? "yml" : "yaml"}`;
    if (next() < 0.7) row.runDeploys = next() < 0.5;
  } else {
    if (next() < 0.5) row.workflow = null;
    if (next() < 0.3) row.runDeploys = false;
  }
  if (state === "in service" && next() < 0.7)
    row.address = `https://${letters(next, `${LOWER}${DIGIT}`, 1, 12)}.${pick(next, ["security.cab", "workers.dev"])}${
      next() < 0.4 ? `/${letters(next, `${ALNUM}/_-?=`, 0, 20)}` : ""
    }`;
  else if (next() < 0.5) row.address = null;
  return row;
}

/** What the parser is to say of a good row: the row, with what was
 * left out said. */
const read = (row: Loose) =>
  ({
    name: row.name,
    title: row.title,
    group: row.group,
    path: row.path,
    does: row.does,
    state: row.state,
    deploy: row.deploy,
    workflow: row.workflow ?? null,
    runDeploys: row.runDeploys === true,
    address: row.address ?? null,
    next: row.next,
  }) as WorkerRow;

/** One change to a good row, each known to break one rule whatever
 * the rest of the row says. Null: it does not apply to this row. */
const FAULTS: [string, (row: Loose, next: () => number) => Loose | null][] = [
  ...["name", "title", "group", "path", "does", "state", "deploy", "next"].map(
    (field): [string, (row: Loose) => Loose] => [
      `no ${field}`,
      (row) => Object.fromEntries(Object.entries(row).filter(([key]) => key !== field)),
    ],
  ),
  ...["name", "title", "group", "path", "does", "state", "deploy", "next"].map(
    (field): [string, (row: Loose, next: () => number) => Loose] => [
      `a ${field} that is not text`,
      (row, next) => ({ ...row, [field]: pick<unknown>(next, [null, 7, true, [], {}, [String(row[field])]]) }),
    ],
  ),
  ["an empty name", (row) => ({ ...row, name: "" })],
  [
    "a name with a capital in it",
    (row, next) => ({ ...row, name: `${row.name}`.slice(0, 40) + pick(next, [..."ABXZ"]) }),
  ],
  [
    "a name with something else in it",
    (row, next) => ({ ...row, name: `a${pick(next, [..." _./:é"])}${`${row.name}`.slice(0, 40)}` }),
  ],
  ["a name that starts with a hyphen", (row) => ({ ...row, name: `-${`${row.name}`.slice(0, 40)}` })],
  ["a name longer than a Worker's may be", (_row, next) => ({ ..._row, name: letters(next, LOWER, 64, 90) })],
  ["a blank title", (row, next) => ({ ...row, title: letters(next, " \t\n", 0, 5) })],
  ["a title too long", (row, next) => ({ ...row, title: letters(next, ALNUM, 81, 120) })],
  ["too much said of what it does", (row, next) => ({ ...row, does: letters(next, ALNUM, 401, 450) })],
  ["nothing said of what is next", (row, next) => ({ ...row, next: letters(next, " ", 0, 3) })],
  ["too much said of what is next", (row, next) => ({ ...row, next: letters(next, ALNUM, 401, 450) })],
  ["a path from the root", (row) => ({ ...row, path: `/${row.path}` })],
  [
    "a path that climbs",
    (row, next) => ({
      ...row,
      path: pick(next, [`../${row.path}`, `${row.path}/../x`, `${row.path}/..`, `a..b/${row.path}`]),
    }),
  ],
  ["a path with something else in it", (row, next) => ({ ...row, path: `a${pick(next, [..." :*\\~$"])}${row.path}` })],
  ["a path too long", (_row, next) => ({ ..._row, path: letters(next, ALNUM, 201, 230) })],
  ["a group there is not", (row, next) => ({ ...row, group: pick(next, ["misc", "", "Platform", "fleet "]) })],
  ["a state there is not", (row, next) => ({ ...row, state: pick(next, ["maybe", "", "In service", "deployed"]) })],
  [
    "a way of deploying there is not",
    (row, next) => ({ ...row, deploy: pick(next, ["magic", "", "Actions", "hand"]) }),
  ],
  [
    "a workflow, and not deployed from Actions",
    (row) => (row.deploy === "actions" ? null : { ...row, workflow: "deploy.yml" }),
  ],
  [
    "deployed from Actions, with no workflow",
    (row, next) =>
      row.deploy !== "actions"
        ? null
        : next() < 0.5
          ? { ...row, workflow: null }
          : Object.fromEntries(Object.entries(row).filter(([key]) => key !== "workflow")),
  ],
  [
    "a workflow that is not a workflow file's name",
    (row, next) =>
      row.deploy !== "actions"
        ? null
        : {
            ...row,
            workflow: pick<unknown>(next, [
              "../deploy.yml",
              "deploy.sh",
              ".github/workflows/deploy.yml",
              "",
              "deploy yml",
              7,
              {},
            ]),
          },
  ],
  [
    "a run that deploys, said as something other than yes or no",
    (row, next) => ({ ...row, runDeploys: pick<unknown>(next, ["yes", 1, 0, null, {}, "true"]) }),
  ],
  [
    "a run that deploys, with no workflow to run",
    (row) => (row.deploy === "actions" ? null : { ...row, runDeploys: true }),
  ],
  [
    "in service and not, at once",
    (row) => ({ ...row, state: row.state === "in service" ? "not deployed" : "in service" }),
  ],
  [
    "not deployed, with an address",
    (row) => (row.state === "not deployed" ? { ...row, address: "https://x.security.cab" } : null),
  ],
  [
    "an address that is not an https one",
    (row, next) =>
      row.state !== "in service"
        ? null
        : {
            ...row,
            address: pick<unknown>(next, [
              "http://x.security.cab",
              "javascript:alert(1)",
              "https://x.security.cab/a b",
              "x.security.cab",
              "https://",
              "https://X.security.cab",
              7,
              {},
              "",
            ]),
          },
  ],
];

test("property: a row built to keep every rule is read back as it was written, and one fault refuses it", () => {
  const next = rng(13487);
  const applied = new Map<string, number>();
  for (let i = 0; i < 2000; i++) {
    const row = goodRow(next);
    expect([row, workerRow(row)]).toEqual([row, read(row)]);
    // What was read is itself a row that reads the same.
    expect(workerRow(workerRow(row))).toEqual(read(row));
    for (const [why, fault] of FAULTS) {
      const bad = fault(row, next);
      if (bad === null) continue;
      applied.set(why, (applied.get(why) ?? 0) + 1);
      expect([why, bad, workerRow(bad)]).toEqual([why, bad, null]);
    }
  }
  // The generator does reach every fault, each many times.
  expect(FAULTS.map(([why]) => [why, (applied.get(why) ?? 0) > 100])).toEqual(FAULTS.map(([why]) => [why, true]));
});

test("property: whatever is said for a row, reading it does not fail, and a row that is read keeps the rules", () => {
  const next = rng(20261123);
  // Every field from what a row may say and from what it may not, in
  // every mixture: most are refused, and some are good by chance.
  const any = (good: readonly unknown[]) =>
    next() < 0.8
      ? pick(next, good)
      : pick<unknown>(next, [undefined, null, "", " ", 0, 7, true, [], {}, "misc", "../x", "x y"]);
  let kept = 0;
  let refused = 0;
  for (let i = 0; i < 6000; i++) {
    const model = goodRow(next);
    const said: Loose = {
      name: any([model.name]),
      title: any([model.title]),
      group: any(GROUPS),
      path: any([model.path]),
      does: any([model.does]),
      state: any(STATES),
      deploy: any(DEPLOYED),
      workflow: any([null, undefined, "deploy.yml", "a.yaml"]),
      runDeploys: any([undefined, true, false]),
      address: any([null, undefined, "https://x.security.cab", "https://x.workers.dev/health"]),
      next: any([model.next]),
    };
    const got = workerRow(next() < 0.02 ? pick<unknown>(next, [null, undefined, 7, "row", [], [said], true]) : said);
    if (got === null) {
      refused++;
      continue;
    }
    kept++;
    // Nothing is made up: each thing read is the thing said.
    expect(got).toEqual(read(said));
    // And the rules, written out again here.
    expect(/^[a-z0-9][a-z0-9-]{0,62}$/.test(got.name)).toBe(true);
    expect(got.path.startsWith("/") || got.path.split("/").includes("..") || got.path.includes("..")).toBe(false);
    expect([GROUPS.includes(got.group), STATES.includes(got.state), DEPLOYED.includes(got.deploy)]).toEqual([
      true,
      true,
      true,
    ]);
    expect([got.title.trim() !== "", got.does.trim() !== "", got.next.trim() !== ""]).toEqual([true, true, true]);
    expect(got.workflow !== null).toBe(got.deploy === "actions");
    expect(got.deploy === "none").toBe(got.state === "not deployed");
    if (got.runDeploys) expect(got.workflow).not.toBeNull();
    if (got.state === "not deployed") expect(got.address).toBeNull();
    if (got.address !== null) expect(got.address.startsWith("https://")).toBe(true);
    expect(workerRow(got)).toEqual(got);
  }
  // Both happen often enough for the property to mean something.
  expect([kept > 100, refused > 1000]).toEqual([true, true]);
});

test("property: the list is the first good row of each name, in the file's order within a group, and counts the rest", () => {
  const next = rng(18);
  const names = ["pm-console-edge", "message-bridge", "pkarr", "nix-cache", "atlas", "a", "b-2", "queue-alert"];
  let doubled = 0;
  let outOfOrder = 0;
  for (let i = 0; i < 1500; i++) {
    // A file: good rows (names from a few, so some come twice), rows
    // with one fault, and things that are not rows at all.
    const given: { said: unknown; good: Loose | null }[] = Array.from({ length: Math.floor(next() * 14) }, () => {
      const row = goodRow(next, pick(next, names));
      const roll = next();
      if (roll < 0.6) return { said: row, good: row };
      if (roll < 0.9) {
        for (;;) {
          const bad = pick(next, FAULTS)[1](row, next);
          if (bad !== null) return { said: bad, good: null };
        }
      }
      return { said: pick<unknown>(next, [null, "a row", 7, [], [row], true]), good: null };
    });
    const dated = next() < 0.6;
    const checked = dated
      ? `${letters(next, DIGIT, 4, 4)}-${letters(next, DIGIT, 2, 2)}-${letters(next, DIGIT, 2, 2)}`
      : pick<unknown>(next, [
          undefined,
          null,
          "yesterday",
          20261123,
          "2026-11-23T00:00:00Z",
          " 2026-11-23",
          "2026-11-2",
          {},
        ]);

    // The model.
    const first = new Map<string, WorkerRow>();
    for (const { good } of given) if (good && !first.has(`${good.name}`)) first.set(`${good.name}`, read(good));
    const want = GROUPS.flatMap((group) => [...first.values()].filter((row) => row.group === group));
    if (given.filter(({ good }) => good).length > first.size) doubled++;
    if ([...first.values()].some((row, at) => row !== want[at])) outOfOrder++;

    const list = workersList({ checked, workers: given.map(({ said }) => said) });
    expect(list).toEqual({
      checked: dated ? (checked as string) : null,
      workers: want,
      malformed: given.length - want.length,
    });
    // What it gives is a list that reads as itself, with nothing left over.
    expect(workersList({ checked: list.checked, workers: list.workers })).toEqual({ ...list, malformed: 0 });
    // The same rows in another order are the same Workers, counted the
    // same, group by group (which of two of one name is kept is the
    // file's order, so only the names are compared).
    const turned = given.map(({ said }) => said).sort(() => next() - 0.5);
    const again = workersList({ workers: turned });
    expect([again.malformed, again.workers.map((row) => row.name).sort()]).toEqual([
      list.malformed,
      want.map((row) => row.name).sort(),
    ]);
    expect(again.workers.map((row) => GROUPS.indexOf(row.group))).toEqual(
      again.workers.map((row) => GROUPS.indexOf(row.group)).sort((a, b) => a - b),
    );
  }
  // The generator does make files with a name twice, and files whose
  // rows are not already in the order shown.
  expect([doubled > 100, outOfOrder > 100]).toEqual([true, true]);
  // What is not a file's contents at all is an empty list, whatever it is.
  for (let i = 0; i < 300; i++) {
    const odd = pick<unknown>(next, [
      null,
      undefined,
      7,
      "text",
      true,
      [],
      [goodRow(next)],
      {},
      { workers: "all" },
      { workers: { 0: goodRow(next) } },
      { workers: null },
    ]);
    expect([odd, workersList(odd)]).toEqual([odd, { checked: null, workers: [], malformed: 0 }]);
  }
});
