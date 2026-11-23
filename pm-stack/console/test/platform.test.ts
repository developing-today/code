// The list of Workers, as the Platform screen reads it.
//
// Oracles: the rules for a row are written out here by hand, one case
// each. That the file in the repository matches the repository is
// checked beside it (pm-stack/platform/workers.test.ts); that the edge
// reads it with no machine is in test/edge.test.ts.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { readConfig } from "../src/config.ts";
import { Ops, type Run } from "../src/ops.ts";
import { workerRow, workersList } from "../src/platform.ts";

const good = {
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
  expect(workerRow(good)).toEqual(good);
  expect(workerRow({ ...good, deploy: "actions", workflow: "bridge-deploy.yml" })).toMatchObject({
    deploy: "actions",
    workflow: "bridge-deploy.yml",
  });
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
    ["a group there is not", { ...good, group: "misc" }],
    ["a state there is not", { ...good, state: "maybe" }],
    ["a way of deploying there is not", { ...good, deploy: "magic" }],
    ["from Actions, with no workflow", { ...good, deploy: "actions", workflow: null }],
    ["by hand, with a workflow", { ...good, workflow: "bridge-deploy.yml" }],
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
  expect([p.checked, p.malformed, p.workers.length]).toEqual(["2026-11-23", 0, 18]);
  // One read of the list, and one of each deploy workflow: not one per Worker.
  expect(asked).toHaveLength(3);
  const byName = new Map(p.workers.map((w) => [w.name, w]));
  expect(byName.get("pm-console-edge")?.run).toEqual({
    commit: "2a96e875037d",
    state: "success",
    at: "2026-11-23T16:32:32Z",
    url: "https://github.com/developing-today/code/actions/runs/45217672084",
  });
  // A deploy that has never run on main has no run to show.
  expect(byName.get("pkarr")?.run).toBeNull();
  expect(byName.get("voice-line")?.run).toBeNull();
  // Its runs cannot be read: that Worker says so, and the rest still show.
  fails.add("repos/developing-today/code/actions/workflows/pm-console-edge-deploy.yml/runs?branch=main&per_page=1");
  const partial = await ops.platform();
  expect(partial.workers).toHaveLength(18);
  expect(partial.workers.find((w) => w.name === "pm-console-edge")?.run).toEqual({
    error: expect.stringContaining("404"),
  });
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
