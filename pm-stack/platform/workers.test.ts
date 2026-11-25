// The list of Workers against the repository it describes.
//
// Oracles: the repository itself. Every wrangler file that is tracked
// has to be in the list under the name it gives and the directory it
// is in, and the list may name nothing that is not there; a workflow
// the list says deploys a Worker has to exist, be startable by hand,
// and work in that Worker's directory. So a Worker added or moved
// without the list being told fails here (in the nightly run of the
// pm-stack tests), and the Platform screen cannot quietly go stale.
import { expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { workersList } from "../console/src/platform.ts";

const ROOT = join(import.meta.dir, "..", "..");
const listed = workersList(JSON.parse(readFileSync(join(import.meta.dir, "workers.json"), "utf8")));
const tracked = Bun.spawnSync(["git", "ls-files", "--", "*wrangler.toml", "*wrangler.jsonc", "*wrangler.json"], {
  cwd: ROOT,
})
  .stdout.toString()
  .split("\n")
  .filter(Boolean);
/** The `name` a wrangler file gives its Worker. */
const nameIn = (file: string) =>
  /^\s*"?name"?\s*[:=]\s*"([^"]+)"/m.exec(readFileSync(join(ROOT, file), "utf8"))?.[1] ?? "";

test("the list reads whole: every entry is well formed, and no name is there twice", () => {
  expect(listed.malformed).toBe(0);
  expect(listed.workers.length).toBeGreaterThan(10);
  expect(listed.checked).toMatch(/^\d{4}-\d{2}-\d{2}$/);
});

test("every Worker in the repository is in the list, under its own name and directory", () => {
  expect(tracked.length).toBeGreaterThan(10);
  const inRepo = new Map<string, string[]>();
  for (const file of tracked) {
    const name = nameIn(file);
    expect([file, name]).not.toEqual([file, ""]);
    inRepo.set(name, [...(inRepo.get(name) ?? []), dirname(file)]);
  }
  const names = listed.workers.map((w) => w.name).sort();
  // Nothing in the repository is missing from the list, and nothing in
  // the list is missing from the repository.
  expect(names).toEqual([...inRepo.keys()].sort());
  for (const w of listed.workers) {
    // Its directory holds its wrangler file (a Worker stood up from a
    // second, empty one keeps that beside it, under the same name).
    expect([w.name, inRepo.get(w.name)]).toEqual([w.name, expect.arrayContaining([w.path])]);
  }
});

test("a Worker said to be deployed from Actions has a workflow that deploys it and can be started by hand", () => {
  const fromActions = listed.workers.filter((w) => w.deploy === "actions");
  expect(fromActions.length).toBeGreaterThan(0);
  for (const w of fromActions) {
    const file = join(ROOT, ".github", "workflows", w.workflow as string);
    expect([w.name, existsSync(file)]).toEqual([w.name, true]);
    const workflow = readFileSync(file, "utf8");
    expect([w.name, workflow.includes("workflow_dispatch")]).toEqual([w.name, true]);
    expect([w.name, workflow.includes(w.path)]).toEqual([w.name, true]);
    // A deploy runs wrangler, uploads a version, or runs the Worker's
    // flake deploy app (`nix run .#<name>-deploy`), which uploads
    // through the Cloudflare API itself (message-bridge does).
    expect([w.name, /wrangler|upload-version|nix run [^\n]*#[\w-]+-deploy/.test(workflow)]).toEqual([w.name, true]);
  }
});

test("what is not deployed has no address, and what is in service has one", () => {
  for (const w of listed.workers) {
    expect([w.name, w.state === "in service", w.address !== null]).toEqual([
      w.name,
      w.address !== null,
      w.address !== null,
    ]);
  }
});
