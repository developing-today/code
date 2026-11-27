// The list of Turso stores against the code that names them.
//
// Oracles: the source itself. Every store the tracked code addresses
// by its Turso hostname has to be in the list under that name, and the
// list may name nothing the code does not address. Each token the list
// names has to be named in the code too. So a store added or renamed in
// the code without the list being told fails here (in the nightly run of
// the pm-stack tests), and no token value is ever written in the list.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { STORES_FILE, storesList } from "../console/src/stores.ts";

const ROOT = join(import.meta.dir, "..", "..");
const raw = JSON.parse(readFileSync(join(ROOT, STORES_FILE), "utf8"));
const listed = storesList(raw);
const CODE = /\.(ts|mjs|js|py|toml|jsonc?)$/;
const tracked = Bun.spawnSync(["git", "ls-files", "--", "pm-stack"], { cwd: ROOT })
  .stdout.toString()
  .split("\n")
  .filter((file) => CODE.test(file) && !file.includes("node_modules") && file !== STORES_FILE);
const HOST =
  /https:\/\/(?:(?<turso>[a-z0-9-]+)-dezren39\.aws-[a-z0-9-]+\.turso\.io|d1-gateway\.developing-today\.workers\.dev\/db\/(?<gateway>[a-z0-9-]+))/g;
const sources = tracked.map((file) => readFileSync(join(ROOT, file), "utf8"));

test("the list reads whole: every store is well formed, and no name is there twice", () => {
  expect(listed.malformed).toBe(0);
  expect(listed.stores.length).toBeGreaterThan(0);
  expect(listed.checked).toMatch(/^\d{4}-\d{2}-\d{2}$/);
});

test("every store the code addresses is in the list, and nothing in the list is unnamed by the code", () => {
  const named = new Set<string>();
  for (const source of sources) {
    for (const m of source.matchAll(HOST)) {
      const name = m[1] ?? m[2];
      if (name) named.add(name);
    }
  }
  expect(named.size).toBeGreaterThan(0);
  expect([...named].sort()).toEqual(listed.stores.map((s) => s.name).sort());
});

test("each store's address is the one the code gives it", () => {
  const all = sources.join("\n");
  for (const store of listed.stores) {
    expect([store.name, all.includes(store.address)]).toEqual([store.name, true]);
  }
});

test("each token the list names is named by the code, and the list holds no token value", () => {
  const all = sources.join("\n");
  for (const store of listed.stores) {
    if (store.token.startsWith("turso.")) {
      expect([store.name, all.includes(store.token)]).toEqual([store.name, true]);
    }
  }
  expect(readFileSync(join(ROOT, STORES_FILE), "utf8")).not.toMatch(/eyJ|ey[A-Za-z0-9_-]{20,}/);
});
